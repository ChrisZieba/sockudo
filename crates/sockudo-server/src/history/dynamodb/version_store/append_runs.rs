use super::*;
use aws_sdk_dynamodb::types::KeysAndAttributes;
use sockudo_core::version_store::append_storage::{
    AppendRunPlan, AppendRunRef, AppendRunSnapshots, CHUNK_BYTES, StoredVersionPayload,
    encode_full, is_compact, snapshot_from_bytes,
};
use sockudo_core::versioned_messages::{MessageSerial, VersionSerial};
use std::collections::BTreeSet;

/// BatchGetItem accepts at most 100 keys per request.
const BATCH_GET_LIMIT: usize = 100;

impl DynamoDbVersionStore {
    pub(super) fn append_manifest_key(record: &StoredVersionRecord, run: &AppendRunRef) -> String {
        let mut key = Self::append_run_key(record.message_serial().as_str(), run.run.as_str());
        if let Some(generation) = &run.generation {
            key.push(' ');
            key.push_str(generation);
        }
        key
    }

    pub(super) fn seed_manifest_write(
        &self,
        record: &StoredVersionRecord,
        plan: &AppendRunPlan,
    ) -> Result<Option<TransactWriteItem>> {
        let Some(mut write) =
            self.append_run_write(record, plan, true, sockudo_core::history::now_ms())?
        else {
            return Ok(None);
        };
        if let Some(put) = write.put.as_mut() {
            put.item
                .insert("append_pending".to_string(), AttributeValue::Bool(true));
            put.item
                .insert("append_ready".to_string(), AttributeValue::Bool(false));
            put.condition_expression =
                Some("attribute_not_exists(message_version_key)".to_string());
        }
        Ok(Some(write))
    }

    pub(super) fn seed_stage_guard(
        &self,
        record: &StoredVersionRecord,
        plan: &AppendRunPlan,
    ) -> Result<Option<TransactWriteItem>> {
        let Some(run) = plan.run() else {
            return Ok(None);
        };
        let check = aws_sdk_dynamodb::types::ConditionCheck::builder().table_name(&self.tables.version_entries)
            .key("app_channel", Self::attr_s(&Self::app_channel_key(&record.app_id, &record.channel)))
            .key("message_version_key", Self::attr_s(&Self::append_manifest_key(record, run)))
            .condition_expression("append_pending = :yes AND append_ready = :no AND attribute_not_exists(garbage_collecting)")
            .expression_attribute_values(":yes", AttributeValue::Bool(true))
            .expression_attribute_values(":no", AttributeValue::Bool(false))
            .build().map_err(|e| Error::Internal(format!("failed to build append seed stage guard: {e}")))?;
        Ok(Some(
            TransactWriteItem::builder().condition_check(check).build(),
        ))
    }

    pub(super) fn seed_activation_write(
        &self,
        record: &StoredVersionRecord,
        plan: &AppendRunPlan,
    ) -> Result<Option<TransactWriteItem>> {
        let Some(run) = plan.run() else {
            return Ok(None);
        };
        let update = Update::builder().table_name(&self.tables.version_entries)
            .key("app_channel", Self::attr_s(&Self::app_channel_key(&record.app_id, &record.channel)))
            .key("message_version_key", Self::attr_s(&Self::append_manifest_key(record, run)))
            .update_expression("SET append_pending = :no")
            .condition_expression("append_pending = :yes AND append_ready = :yes AND head_version_serial = :head AND attribute_not_exists(garbage_collecting)")
            .expression_attribute_values(":yes", AttributeValue::Bool(true))
            .expression_attribute_values(":no", AttributeValue::Bool(false))
            .expression_attribute_values(":head", Self::attr_s(record.version_serial().as_str()))
            .build().map_err(|e| Error::Internal(format!("failed to build append seed activation: {e}")))?;
        Ok(Some(TransactWriteItem::builder().update(update).build()))
    }

    pub(super) async fn stage_seed(&self, record: &StoredVersionRecord) -> Result<AppendRunPlan> {
        let plan = AppendRunPlan::for_seed_record(record);
        let Some(manifest) = self.seed_manifest_write(record, &plan)? else {
            return Ok(plan);
        };
        // Publish the discoverable pending manifest before any chunk. Every
        // stage transaction is fenced against the GC tombstone; abandoned
        // stages are eligible for bounded background cleanup after five minutes.
        self.client
            .transact_write_items()
            .transact_items(manifest)
            .send()
            .await
            .map_err(|e| Error::Internal(format!("failed to stage append manifest: {e}")))?;
        let writes = self.append_chunk_writes(record, &plan)?;
        for batch in writes.chunks(99) {
            let mut transaction = self
                .client
                .transact_write_items()
                .set_transact_items(Some(batch.to_vec()));
            if let Some(guard) = self.seed_stage_guard(record, &plan)? {
                transaction = transaction.transact_items(guard);
            }
            transaction
                .send()
                .await
                .map_err(|e| Error::Internal(format!("failed to stage append chunks: {e}")))?;
        }
        let Some(run) = plan.run() else {
            return Ok(plan);
        };
        self.client.update_item().table_name(&self.tables.version_entries)
            .key("app_channel", Self::attr_s(&Self::app_channel_key(&record.app_id, &record.channel)))
            .key("message_version_key", Self::attr_s(&Self::append_manifest_key(record, run)))
            .update_expression("SET append_ready = :yes")
            .condition_expression("append_pending = :yes AND append_ready = :no AND attribute_not_exists(garbage_collecting)")
            .expression_attribute_values(":yes", AttributeValue::Bool(true))
            .expression_attribute_values(":no", AttributeValue::Bool(false))
            .send().await.map_err(|e| Error::Internal(format!("failed to finish append seed: {e}")))?;
        Ok(plan)
    }

    pub(super) fn seed_attributes(
        item: &mut HashMap<String, AttributeValue>,
        record: &StoredVersionRecord,
        plan: &AppendRunPlan,
    ) {
        if let Some(run) = plan.run() {
            item.insert(
                "latest_append_run".to_string(),
                Self::attr_s(run.run.as_str()),
            );
            item.insert("latest_append_len".to_string(), Self::attr_n(run.data_len));
            item.insert(
                "latest_append_head".to_string(),
                Self::attr_s(record.version_serial().as_str()),
            );
            item.insert(
                "latest_append_generation".to_string(),
                Self::attr_s(run.generation.as_deref().unwrap_or_default()),
            );
            item.insert(
                "latest_append_pinned".to_string(),
                AttributeValue::Bool(true),
            );
        }
    }

    pub(super) async fn seed_latest_states(&self) -> Result<()> {
        let mut start = None;
        loop {
            let page = self
                .client
                .scan()
                .table_name(&self.tables.version_messages)
                .limit(100)
                .consistent_read(true)
                .set_exclusive_start_key(start)
                .send()
                .await
                .map_err(|e| Error::Internal(format!("failed to scan append seeds: {e}")))?;
            for item in page.items() {
                let app_channel = Self::item_str(item, "app_channel")
                    .ok_or_else(|| Error::Internal("append seed partition missing".to_string()))?;
                let Some(record) = self.materialize_latest_item(&app_channel, item).await? else {
                    continue;
                };
                let plan = self.stage_seed(&record).await?;
                let Some(run) = plan.run() else {
                    continue;
                };
                let update = Update::builder().table_name(&self.tables.version_messages)
                    .key("app_channel", Self::attr_s(&app_channel)).key("message_serial", Self::attr_s(record.message_serial().as_str()))
                    .update_expression("SET latest_append_run = :run, latest_append_len = :len, latest_append_head = :head, latest_append_generation = :generation, latest_append_pinned = :pinned")
                    .condition_expression("latest_version_serial = :head AND latest_delivery_serial = :delivery")
                    .expression_attribute_values(":run", Self::attr_s(run.run.as_str()))
                    .expression_attribute_values(":len", Self::attr_n(run.data_len))
                    .expression_attribute_values(":head", Self::attr_s(record.version_serial().as_str()))
                    .expression_attribute_values(":generation", Self::attr_s(run.generation.as_deref().unwrap_or_default()))
                    .expression_attribute_values(":pinned", AttributeValue::Bool(true))
                    .expression_attribute_values(":delivery", Self::attr_n(record.delivery_serial()))
                    .build().map_err(|e| Error::Internal(format!("failed to build append seed publication: {e}")))?;
                let mut transaction = self
                    .client
                    .transact_write_items()
                    .transact_items(TransactWriteItem::builder().update(update).build());
                if let Some(activation) = self.seed_activation_write(&record, &plan)? {
                    transaction = transaction.transact_items(activation);
                }
                transaction
                    .send()
                    .await
                    .map_err(|e| Error::Internal(format!("failed to publish append seed: {e}")))?;
            }
            start = page.last_evaluated_key().cloned();
            if start.is_none() {
                break;
            }
        }
        Ok(())
    }

    pub(super) const FORMAT_MARKER_KEY: &'static str = "__append_storage_format__";

    pub(super) async fn append_storage_epoch(&self) -> Result<Option<String>> {
        let result = self
            .client
            .get_item()
            .table_name(&self.tables.version_streams)
            .key("app_channel", Self::attr_s(Self::FORMAT_MARKER_KEY))
            .consistent_read(true)
            .send()
            .await
            .map_err(|e| Error::Internal(format!("failed to read append storage marker: {e}")))?;
        Ok(result
            .item
            .as_ref()
            .filter(|item| item.get("enabled") == Some(&AttributeValue::Bool(true)))
            .and_then(|item| Self::item_str(item, "epoch")))
    }

    pub(super) fn append_chunk_key(message: &str, run: &AppendRunRef, index: u64) -> String {
        format!(
            "__append_chunk__ {message} {} {} {index:020}",
            run.run.as_str(),
            run.generation.as_deref().unwrap_or_default()
        )
    }

    async fn load_payload_runs(
        &self,
        app_channel: &str,
        payloads: &[StoredVersionPayload],
    ) -> Result<AppendRunSnapshots> {
        let legacy = payloads
            .iter()
            .filter_map(|payload| {
                payload
                    .run()
                    .filter(|run| run.generation.is_none())
                    .map(|run| (payload.record().message_serial().clone(), run.run.clone()))
            })
            .collect();
        let mut snapshots = self.load_append_runs(app_channel, &legacy).await?;
        let mut needed: HashMap<
            (MessageSerial, VersionSerial),
            (&StoredVersionRecord, AppendRunRef),
        > = HashMap::new();
        for payload in payloads {
            if let Some(run) = payload.run().filter(|run| run.generation.is_some()) {
                let key = (payload.record().message_serial().clone(), run.run.clone());
                let entry = needed.entry(key).or_insert((payload.record(), run.clone()));
                if entry.1.generation != run.generation {
                    return Err(Error::Internal(
                        "conflicting append run generations".to_string(),
                    ));
                }
                entry.1.data_len = entry.1.data_len.max(run.data_len);
            }
        }
        for (key, (record, run)) in needed {
            if let Some(snapshot) = self.append_cache.get(
                &record.app_id,
                &record.channel,
                record.message_serial(),
                &run,
            ) {
                snapshots.insert(key, snapshot);
                continue;
            }
            let count = run.data_len.div_ceil(CHUNK_BYTES as u64);
            let mut chunks = HashMap::new();
            for first in (0..count).step_by(BATCH_GET_LIMIT) {
                let keys = (first..count.min(first + BATCH_GET_LIMIT as u64))
                    .map(|index| {
                        let chunk_key =
                            Self::append_chunk_key(record.message_serial().as_str(), &run, index);
                        (chunk_key, index)
                    })
                    .collect::<HashMap<_, _>>();
                let mut pending = keys
                    .keys()
                    .map(|key| {
                        HashMap::from([
                            ("app_channel".to_string(), Self::attr_s(app_channel)),
                            ("message_version_key".to_string(), Self::attr_s(key)),
                        ])
                    })
                    .collect::<Vec<_>>();
                for _ in 0..8 {
                    if pending.is_empty() {
                        break;
                    }
                    let request = KeysAndAttributes::builder()
                        .set_keys(Some(std::mem::take(&mut pending)))
                        .consistent_read(true)
                        .projection_expression("message_version_key, payload_bytes")
                        .build()
                        .map_err(|e| {
                            Error::Internal(format!("failed to build append chunk read: {e}"))
                        })?;
                    let response = self
                        .client
                        .batch_get_item()
                        .request_items(&self.tables.version_entries, request)
                        .send()
                        .await
                        .map_err(|e| {
                            Error::Internal(format!("failed to read append chunks: {e}"))
                        })?;
                    for item in response
                        .responses()
                        .and_then(|tables| tables.get(&self.tables.version_entries))
                        .into_iter()
                        .flatten()
                    {
                        if let Some(index) = Self::item_str(item, "message_version_key")
                            .and_then(|key| keys.get(&key).copied())
                        {
                            let bytes =
                                Self::item_bytes(item, "payload_bytes").ok_or_else(|| {
                                    Error::Internal("append chunk is missing data".to_string())
                                })?;
                            chunks.insert(index, bytes);
                        }
                    }
                    if let Some(unprocessed) = response
                        .unprocessed_keys()
                        .and_then(|tables| tables.get(&self.tables.version_entries))
                    {
                        pending = unprocessed.keys().to_vec();
                    }
                }
                if !pending.is_empty() {
                    return Err(Error::Internal(
                        "append chunk read retries exhausted".to_string(),
                    ));
                }
            }
            let mut bytes = Vec::new();
            for index in 0..count {
                let chunk = chunks
                    .remove(&index)
                    .ok_or_else(|| Error::Internal("append chunk is missing".to_string()))?;
                let required =
                    (run.data_len - index * CHUNK_BYTES as u64).min(CHUNK_BYTES as u64) as usize;
                if chunk.len() < required || chunk.len() > CHUNK_BYTES {
                    return Err(Error::Internal("invalid append chunk length".to_string()));
                }
                bytes.extend_from_slice(&chunk[..required]);
            }
            let snapshot = snapshot_from_bytes(bytes)?;
            self.append_cache.insert(
                &record.app_id,
                &record.channel,
                record.message_serial(),
                &run,
                snapshot.clone(),
            );
            snapshots.insert(key, snapshot);
        }
        Ok(snapshots)
    }

    pub(super) async fn materialize_latest_item(
        &self,
        app_channel: &str,
        item: &HashMap<String, AttributeValue>,
    ) -> Result<Option<StoredVersionRecord>> {
        let Some(payload) = item.get("latest_payload_bytes") else {
            return Ok(None);
        };
        let mut entry = item.clone();
        entry.insert("payload_bytes".to_string(), payload.clone());
        Ok(self
            .materialize_items(app_channel, &[entry])
            .await?
            .pop()
            .flatten())
    }

    pub(super) fn append_chunk_writes(
        &self,
        record: &StoredVersionRecord,
        plan: &AppendRunPlan,
    ) -> Result<Vec<TransactWriteItem>> {
        let Some(run) = plan.run().filter(|run| run.generation.is_some()) else {
            return Ok(Vec::new());
        };
        let chunks = plan.chunk_writes(record)?;
        chunks
            .into_iter()
            .map(|chunk| {
                // Chunks are pinned: extending a run must not let an earlier sealed
                // chunk expire before a later entry or a never-expiring receipt.
                let item = HashMap::from([
                    (
                        "app_channel".to_string(),
                        Self::attr_s(&Self::app_channel_key(&record.app_id, &record.channel)),
                    ),
                    (
                        "message_version_key".to_string(),
                        Self::attr_s(&Self::append_chunk_key(
                            record.message_serial().as_str(),
                            run,
                            chunk.index,
                        )),
                    ),
                    ("payload_bytes".to_string(), Self::attr_b(chunk.bytes)),
                ]);
                let put = Put::builder()
                    .table_name(&self.tables.version_entries)
                    .set_item(Some(item))
                    .build()
                    .map_err(|e| {
                        Error::Internal(format!("failed to build append chunk write: {e}"))
                    })?;
                Ok(TransactWriteItem::builder().put(put).build())
            })
            .collect()
    }

    /// Run snapshots live in the entries table under a key no entry or
    /// receipt can produce: serials never contain whitespace. They carry no
    /// GSI key attributes, so the delivery and message indexes skip them.
    pub(super) fn append_run_key(message_serial: &str, run: &str) -> String {
        format!("__append_run__ {message_serial} {run}")
    }

    fn item_bytes(item: &HashMap<String, AttributeValue>, key: &str) -> Option<Vec<u8>> {
        item.get(key)
            .and_then(|value| value.as_b().ok())
            .map(|value| value.as_ref().to_vec())
    }

    /// Strongly consistent run snapshots for one app/channel.
    pub(super) async fn load_append_runs(
        &self,
        app_channel: &str,
        runs: &BTreeSet<(MessageSerial, VersionSerial)>,
    ) -> Result<AppendRunSnapshots> {
        let mut snapshots = AppendRunSnapshots::new();
        let runs = runs.iter().collect::<Vec<_>>();
        for chunk in runs.chunks(BATCH_GET_LIMIT) {
            let mut pending = chunk
                .iter()
                .map(|(message, run)| {
                    HashMap::from([
                        ("app_channel".to_string(), Self::attr_s(app_channel)),
                        (
                            "message_version_key".to_string(),
                            Self::attr_s(&Self::append_run_key(message.as_str(), run.as_str())),
                        ),
                    ])
                })
                .collect::<Vec<_>>();
            while !pending.is_empty() {
                let request = KeysAndAttributes::builder()
                    .set_keys(Some(std::mem::take(&mut pending)))
                    .consistent_read(true)
                    .projection_expression("message_version_key, payload_bytes")
                    .build()
                    .map_err(|e| {
                        Error::Internal(format!("failed to build append run read: {e}"))
                    })?;
                let response = self
                    .client
                    .batch_get_item()
                    .request_items(&self.tables.version_entries, request)
                    .send()
                    .await
                    .map_err(|e| Error::Internal(format!("failed to read append runs: {e}")))?;
                for item in response
                    .responses()
                    .and_then(|tables| tables.get(&self.tables.version_entries))
                    .into_iter()
                    .flatten()
                {
                    let key = Self::item_str(item, "message_version_key").unwrap_or_default();
                    let Some((message, run)) = chunk.iter().find(|(message, run)| {
                        key == Self::append_run_key(message.as_str(), run.as_str())
                    }) else {
                        continue;
                    };
                    let bytes = Self::item_bytes(item, "payload_bytes").ok_or_else(|| {
                        Error::Internal("append run snapshot is missing".to_string())
                    })?;
                    snapshots.insert((message.clone(), run.clone()), snapshot_from_bytes(bytes)?);
                }
                if let Some(unprocessed) = response
                    .unprocessed_keys()
                    .and_then(|tables| tables.get(&self.tables.version_entries))
                {
                    pending = unprocessed.keys().to_vec();
                }
            }
        }
        Ok(snapshots)
    }

    /// Decode entry or receipt items of one app/channel into full-state
    /// records. An expired compact entry whose run TTL already removed its
    /// snapshot is omitted (`None`) exactly as if TTL had removed the entry;
    /// every other missing snapshot fails closed.
    pub(super) async fn materialize_items(
        &self,
        app_channel: &str,
        items: &[HashMap<String, AttributeValue>],
    ) -> Result<Vec<Option<StoredVersionRecord>>> {
        let payloads = items
            .iter()
            .map(|item| {
                let bytes = Self::item_bytes(item, "payload_bytes").ok_or_else(|| {
                    Error::Internal("Missing payload_bytes in version entry".to_string())
                })?;
                StoredVersionPayload::decode(&bytes)
            })
            .collect::<Result<Vec<_>>>()?;
        let snapshots = self.load_payload_runs(app_channel, &payloads).await?;
        let now_secs = sockudo_core::history::now_ms() / 1000;
        payloads
            .into_iter()
            .zip(items)
            .map(|(payload, item)| {
                let snapshot = payload.run().and_then(|run| {
                    snapshots
                        .get(&(payload.record().message_serial().clone(), run.run.clone()))
                        .map(String::as_str)
                });
                let expired = Self::item_num(item, Self::EXPIRES_AT_ATTR)
                    .is_some_and(|expires_at| expires_at <= now_secs);
                if payload.run().is_some() && snapshot.is_none() && expired {
                    return Ok(None);
                }
                payload.into_record(snapshot).map(Some)
            })
            .collect()
    }

    /// The transactional write of `record`'s run snapshot. A run referenced
    /// by an idempotency receipt is pinned: receipts are never expired, so
    /// neither is the snapshot a duplicate replay reconstructs from.
    /// Otherwise its TTL is refreshed past every entry stored in it.
    pub(super) fn append_run_write(
        &self,
        record: &StoredVersionRecord,
        plan: &AppendRunPlan,
        pinned: bool,
        now_ms: i64,
    ) -> Result<Option<TransactWriteItem>> {
        let (Some(run), Some(record_data)) = (plan.run(), plan.snapshot_after(record)) else {
            return Ok(None);
        };
        let app_channel = Self::app_channel_key(&record.app_id, &record.channel);
        let mut key = Self::append_run_key(record.message_serial().as_str(), run.run.as_str());
        if let Some(generation) = &run.generation {
            key.push(' ');
            key.push_str(generation);
        }
        let pinned = pinned || run.generation.is_some();
        let expires = (!pinned && run.generation.is_none())
            .then(|| self.expires_at_value())
            .flatten();
        let record_data = if run.generation.is_some() {
            ""
        } else {
            record_data
        };
        let write = match plan {
            AppendRunPlan::Full => return Ok(None),
            AppendRunPlan::Start { .. } => {
                let mut item = HashMap::from([
                    ("app_channel".to_string(), Self::attr_s(&app_channel)),
                    ("message_version_key".to_string(), Self::attr_s(&key)),
                    (
                        "payload_bytes".to_string(),
                        Self::attr_b(record_data.as_bytes().to_vec()),
                    ),
                    (
                        "head_version_serial".to_string(),
                        Self::attr_s(record.version_serial().as_str()),
                    ),
                    ("data_len".to_string(), Self::attr_n(run.data_len)),
                    (
                        "append_run_pinned".to_string(),
                        AttributeValue::Bool(pinned),
                    ),
                    ("created_at_ms".to_string(), Self::attr_n(now_ms)),
                    ("updated_at_ms".to_string(), Self::attr_n(now_ms)),
                ]);
                if let Some(generation) = run.generation.as_ref() {
                    item.insert("append_generation".to_string(), Self::attr_s(generation));
                }
                if let Some(expires) = expires {
                    item.insert(Self::EXPIRES_AT_ATTR.to_string(), expires);
                }
                // A run id is its first version serial, which is greater than
                // every retained version of the message, so an existing item
                // with this key is an unreferenced leftover and is replaced.
                let put = Put::builder()
                    .table_name(&self.tables.version_entries)
                    .set_item(Some(item))
                    .build()
                    .map_err(|e| Error::Internal(format!("failed to build append run: {e}")))?;
                TransactWriteItem::builder().put(put).build()
            }
            AppendRunPlan::Extend {
                expected_head,
                expected_len,
                ..
            } => {
                // DynamoDB cannot append to a binary attribute; the snapshot
                // is rewritten, but only this one copy per run is retained.
                let expression = if expires.is_some() {
                    "SET payload_bytes = :data, head_version_serial = :head, data_len = :len, append_run_pinned = :pinned, updated_at_ms = :now, append_pending = :pending, expires_at = :exp"
                } else {
                    "SET payload_bytes = :data, head_version_serial = :head, data_len = :len, append_run_pinned = :pinned, updated_at_ms = :now, append_pending = :pending REMOVE expires_at"
                };
                let mut update = Update::builder()
                    .table_name(&self.tables.version_entries)
                    .key("app_channel", Self::attr_s(&app_channel))
                    .key("message_version_key", Self::attr_s(&key))
                    .update_expression(expression)
                    .condition_expression(
                        "head_version_serial = :expected_head AND data_len = :expected_len AND attribute_not_exists(garbage_collecting)",
                    )
                    .expression_attribute_values(
                        ":data",
                        Self::attr_b(record_data.as_bytes().to_vec()),
                    )
                    .expression_attribute_values(
                        ":head",
                        Self::attr_s(record.version_serial().as_str()),
                    )
                    .expression_attribute_values(":len", Self::attr_n(run.data_len))
                    .expression_attribute_values(":pinned", AttributeValue::Bool(pinned))
                    .expression_attribute_values(":pending", AttributeValue::Bool(false))
                    .expression_attribute_values(":now", Self::attr_n(now_ms))
                    .expression_attribute_values(
                        ":expected_head",
                        Self::attr_s(expected_head.as_str()),
                    )
                    .expression_attribute_values(":expected_len", Self::attr_n(*expected_len));
                if let Some(expires) = expires {
                    update = update.expression_attribute_values(":exp", expires);
                }
                let update = update
                    .build()
                    .map_err(|e| Error::Internal(format!("failed to build append run: {e}")))?;
                TransactWriteItem::builder().update(update).build()
            }
        };
        Ok(Some(write))
    }

    /// Rewrite compact entries and receipts as self-contained records.
    pub(super) async fn materialize_compact_items(&self, batch_size: usize) -> Result<u64> {
        if self.append_storage_epoch().await?.is_some() {
            return Err(Error::Configuration(
                "disable append storage and drain writers before materialization".to_string(),
            ));
        }
        let limit = i32::try_from(batch_size.max(1)).unwrap_or(i32::MAX);
        let mut start = None;
        let mut rewritten = 0;
        loop {
            let page = self
                .client
                .scan()
                .consistent_read(true)
                .table_name(&self.tables.version_entries)
                .limit(limit)
                .set_exclusive_start_key(start)
                .send()
                .await
                .map_err(|e| Error::Internal(format!("failed to scan version entries: {e}")))?;
            for item in page.items() {
                let key = Self::item_str(item, "message_version_key").unwrap_or_default();
                if key.starts_with("__append_run__ ") || key.starts_with("__append_chunk__ ") {
                    continue;
                }
                let Some(payload) = Self::item_bytes(item, "payload_bytes") else {
                    continue;
                };
                if !is_compact(&payload) {
                    continue;
                }
                let app_channel = Self::item_str(item, "app_channel").unwrap_or_default();
                // An expired entry whose snapshot TTL already removed is gone.
                let Some(record) = self
                    .materialize_items(&app_channel, std::slice::from_ref(item))
                    .await?
                    .pop()
                    .flatten()
                else {
                    continue;
                };
                let result = self
                    .client
                    .update_item()
                    .table_name(&self.tables.version_entries)
                    .key("app_channel", Self::attr_s(&app_channel))
                    .key("message_version_key", Self::attr_s(&key))
                    .update_expression("SET payload_bytes = :full")
                    .condition_expression("payload_bytes = :compact")
                    .expression_attribute_values(":full", Self::attr_b(encode_full(&record)?))
                    .expression_attribute_values(":compact", Self::attr_b(payload))
                    .send()
                    .await;
                match result {
                    Ok(_) => rewritten += 1,
                    Err(e)
                        if e.as_service_error()
                            .is_some_and(|error| error.is_conditional_check_failed_exception()) =>
                    {
                        tracing::debug!(error = %e, "append entry changed during materialization");
                    }
                    Err(e) => {
                        return Err(Error::Internal(format!(
                            "failed to rewrite compact entry: {e}"
                        )));
                    }
                }
            }
            start = page.last_evaluated_key().cloned();
            if start.is_none() {
                break;
            }
        }
        let mut start = None;
        loop {
            let page = self
                .client
                .scan()
                .consistent_read(true)
                .table_name(&self.tables.version_messages)
                .limit(limit)
                .set_exclusive_start_key(start)
                .send()
                .await
                .map_err(|e| {
                    Error::Internal(format!("failed to scan latest append states: {e}"))
                })?;
            for item in page.items() {
                if Self::item_bytes(item, "latest_payload_bytes")
                    .is_some_and(|bytes| !is_compact(&bytes))
                    && item.contains_key("latest_append_run")
                {
                    let app_channel = Self::item_str(item, "app_channel").ok_or_else(|| {
                        Error::Internal("latest state partition missing".to_string())
                    })?;
                    let message = Self::item_str(item, "message_serial").ok_or_else(|| {
                        Error::Internal("latest state message missing".to_string())
                    })?;
                    self.client.update_item().table_name(&self.tables.version_messages)
                        .key("app_channel", Self::attr_s(&app_channel)).key("message_serial", Self::attr_s(&message))
                        .update_expression("REMOVE latest_append_run, latest_append_len, latest_append_head, latest_append_pinned, latest_append_generation")
                        .send().await.map_err(|e| Error::Internal(format!("failed to clear append seed pointer: {e}")))?;
                }
                let Some(payload) = Self::item_bytes(item, "latest_payload_bytes")
                    .filter(|bytes| is_compact(bytes))
                else {
                    continue;
                };
                let app_channel = Self::item_str(item, "app_channel").ok_or_else(|| {
                    Error::Internal("latest state partition is missing".to_string())
                })?;
                let message = Self::item_str(item, "message_serial").ok_or_else(|| {
                    Error::Internal("latest state message is missing".to_string())
                })?;
                let Some(record) = self.materialize_latest_item(&app_channel, item).await? else {
                    continue;
                };
                let result = self.client.update_item().table_name(&self.tables.version_messages)
                    .key("app_channel", Self::attr_s(&app_channel)).key("message_serial", Self::attr_s(&message))
                    .update_expression("SET latest_payload_bytes = :full REMOVE latest_append_run, latest_append_len, latest_append_head, latest_append_pinned, latest_append_generation")
                    .condition_expression("latest_payload_bytes = :compact")
                    .expression_attribute_values(":full", Self::attr_b(encode_full(&record)?))
                    .expression_attribute_values(":compact", Self::attr_b(payload)).send().await;
                match result {
                    Ok(_) => rewritten += 1,
                    Err(error)
                        if error
                            .as_service_error()
                            .is_some_and(|error| error.is_conditional_check_failed_exception()) =>
                    {
                        tracing::debug!(error = %error, "latest append state changed during materialization");
                    }
                    Err(error) => {
                        return Err(Error::Internal(format!(
                            "failed to materialize latest append state: {error}"
                        )));
                    }
                }
            }
            start = page.last_evaluated_key().cloned();
            if start.is_none() {
                break;
            }
        }
        // Strongly verify both base tables after all rewrites. If a writer
        // was not drained, or a conditional rewrite lost a race, preserve all
        // chunks instead of leaving a remaining compact record unreadable.
        for (table, payload_field, latest) in [
            (&self.tables.version_entries, "payload_bytes", false),
            (&self.tables.version_messages, "latest_payload_bytes", true),
        ] {
            let mut start = None;
            loop {
                let page = self
                    .client
                    .scan()
                    .table_name(table)
                    .consistent_read(true)
                    .limit(limit)
                    .set_exclusive_start_key(start)
                    .send()
                    .await
                    .map_err(|e| {
                        Error::Internal(format!("failed to verify append materialization: {e}"))
                    })?;
                for item in page.items() {
                    let key = Self::item_str(item, "message_version_key").unwrap_or_default();
                    if !latest
                        && (key.starts_with("__append_run__ ")
                            || key.starts_with("__append_chunk__ "))
                    {
                        continue;
                    }
                    if Self::item_bytes(item, payload_field).is_some_and(|bytes| is_compact(&bytes))
                        || (latest && item.contains_key("latest_append_run"))
                    {
                        return Err(Error::Internal(
                            "append references remain after materialization; chunks retained"
                                .to_string(),
                        ));
                    }
                }
                start = page.last_evaluated_key().cloned();
                if start.is_none() {
                    break;
                }
            }
        }
        // Every entry, receipt and latest state is full now. With writers
        // drained for maintenance, no reference can be introduced during GC.
        let mut start = None;
        loop {
            let page = self
                .client
                .scan()
                .consistent_read(true)
                .table_name(&self.tables.version_entries)
                .limit(limit)
                .set_exclusive_start_key(start)
                .send()
                .await
                .map_err(|e| {
                    Error::Internal(format!("failed to scan obsolete append chunks: {e}"))
                })?;
            for item in page.items() {
                let Some(key) = Self::item_str(item, "message_version_key").filter(|key| {
                    key.starts_with("__append_run__ ") || key.starts_with("__append_chunk__ ")
                }) else {
                    continue;
                };
                let app_channel = Self::item_str(item, "app_channel")
                    .ok_or_else(|| Error::Internal("append partition is missing".to_string()))?;
                self.client
                    .delete_item()
                    .table_name(&self.tables.version_entries)
                    .key("app_channel", Self::attr_s(&app_channel))
                    .key("message_version_key", Self::attr_s(&key))
                    .send()
                    .await
                    .map_err(|e| {
                        Error::Internal(format!("failed to delete obsolete append chunk: {e}"))
                    })?;
            }
            start = page.last_evaluated_key().cloned();
            if start.is_none() {
                break;
            }
        }
        Ok(rewritten)
    }
}
