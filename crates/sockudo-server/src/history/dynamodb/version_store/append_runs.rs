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
        self.stage_seed_plan(record, AppendRunPlan::for_seed_record(record))
            .await
    }

    pub(super) async fn stage_seed_plan(
        &self,
        record: &StoredVersionRecord,
        plan: AppendRunPlan,
    ) -> Result<AppendRunPlan> {
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

    /// Stage only changed chunks behind a fenced manifest lease. Every batch
    /// checks the committed head as well as the token: an older writer need not
    /// understand leases, because its atomic head update invalidates our stage.
    /// Staged tails preserve the committed prefix; unpublished future chunks
    /// are invisible and are overwritten before a later writer publishes them.
    #[cfg(test)]
    pub(super) async fn stage_append_chunks(
        &self,
        record: &StoredVersionRecord,
        plan: &AppendRunPlan,
        epoch: &str,
        writes: &[TransactWriteItem],
    ) -> Result<Option<String>> {
        let token = uuid::Uuid::new_v4().to_string();
        self.stage_append_chunks_with_token(record, plan, epoch, writes, &token)
            .await
    }

    pub(super) async fn stage_append_chunks_with_token(
        &self,
        record: &StoredVersionRecord,
        plan: &AppendRunPlan,
        epoch: &str,
        writes: &[TransactWriteItem],
        token: &str,
    ) -> Result<Option<String>> {
        let AppendRunPlan::Extend {
            run,
            expected_head,
            expected_len,
        } = plan
        else {
            return Err(Error::Internal(
                "append staging requires an existing run".to_string(),
            ));
        };
        let partition = Self::app_channel_key(&record.app_id, &record.channel);
        let key = Self::append_manifest_key(record, run);
        let now = sockudo_core::history::now_ms();
        let claim = self.client.update_item().table_name(&self.tables.version_entries)
            .key("app_channel", Self::attr_s(&partition)).key("message_version_key", Self::attr_s(&key))
            .update_expression("SET staging_token = :token, staging_head = :head, staging_deadline_ms = :deadline")
            .condition_expression("head_version_serial = :head AND data_len = :len AND append_generation = :generation AND attribute_not_exists(garbage_collecting) AND (attribute_not_exists(staging_token) OR staging_head <> :head OR staging_deadline_ms <= :now)")
            .expression_attribute_values(":token", Self::attr_s(token))
            .expression_attribute_values(":head", Self::attr_s(expected_head.as_str()))
            .expression_attribute_values(":len", Self::attr_n(*expected_len))
            .expression_attribute_values(":generation", Self::attr_s(run.generation.as_deref().unwrap_or_default()))
            .expression_attribute_values(":now", Self::attr_n(now))
            .expression_attribute_values(":deadline", Self::attr_n(now.saturating_add(5 * 60 * 1000)))
            .send().await;
        // AWS errors can contain response items; never log their Display.
        match claim {
            Ok(_) => {}
            Err(error)
                if error
                    .as_service_error()
                    .is_some_and(|e| e.is_conditional_check_failed_exception()) =>
            {
                tracing::debug!("append staging claim conflicted");
                return Ok(None);
            }
            Err(_error) => {
                tracing::warn!("append staging claim failed");
                // A transport error may follow an accepted claim.
                self.release_append_stage(record, plan, token).await;
                return Err(Error::Internal(
                    "failed to claim append staging".to_string(),
                ));
            }
        }
        for batch in writes.chunks(98) {
            let guard = aws_sdk_dynamodb::types::ConditionCheck::builder()
                .table_name(&self.tables.version_entries)
                .key("app_channel", Self::attr_s(&partition)).key("message_version_key", Self::attr_s(&key))
                .condition_expression("staging_token = :token AND head_version_serial = :head AND data_len = :len AND append_generation = :generation AND attribute_not_exists(garbage_collecting)")
                .expression_attribute_values(":token", Self::attr_s(token))
                .expression_attribute_values(":head", Self::attr_s(expected_head.as_str()))
                .expression_attribute_values(":len", Self::attr_n(*expected_len))
                .expression_attribute_values(":generation", Self::attr_s(run.generation.as_deref().unwrap_or_default()))
                .build().map_err(|e| Error::Internal(format!("failed to build append staging guard: {e}")))?;
            let result = self
                .client
                .transact_write_items()
                .set_transact_items(Some(batch.to_vec()))
                .transact_items(TransactWriteItem::builder().condition_check(guard).build())
                .transact_items(self.append_marker_guard(epoch)?)
                .send()
                .await;
            match result {
                Ok(_) => {}
                Err(error)
                    if error
                        .as_service_error()
                        .is_some_and(|e| e.is_transaction_canceled_exception()) =>
                {
                    tracing::debug!("append staging batch conflicted");
                    self.release_append_stage(record, plan, token).await;
                    return Ok(None);
                }
                Err(_error) => {
                    tracing::warn!("append staging batch failed");
                    self.release_append_stage(record, plan, token).await;
                    return Err(Error::Internal("failed to stage append batch".to_string()));
                }
            }
        }
        Ok(Some(token.to_string()))
    }

    pub(super) async fn release_append_stage(
        &self,
        record: &StoredVersionRecord,
        plan: &AppendRunPlan,
        token: &str,
    ) {
        let Some(run) = plan.run() else {
            return;
        };
        let result = self
            .client
            .update_item()
            .table_name(&self.tables.version_entries)
            .key(
                "app_channel",
                Self::attr_s(&Self::app_channel_key(&record.app_id, &record.channel)),
            )
            .key(
                "message_version_key",
                Self::attr_s(&Self::append_manifest_key(record, run)),
            )
            .update_expression("REMOVE staging_token, staging_head, staging_deadline_ms")
            .condition_expression("staging_token = :token")
            .expression_attribute_values(":token", Self::attr_s(token))
            .send()
            .await;
        // AWS error text may include response items; keep this diagnostic content-free.
        if let Err(error) = result {
            if error
                .as_service_error()
                .is_some_and(|e| e.is_conditional_check_failed_exception())
            {
                tracing::debug!("append staging lease already changed");
            } else {
                tracing::warn!("append staging lease release failed");
            }
        }
    }

    pub(super) fn append_marker_guard(&self, epoch: &str) -> Result<TransactWriteItem> {
        let check = aws_sdk_dynamodb::types::ConditionCheck::builder()
            .table_name(&self.tables.version_streams)
            .key("app_channel", Self::attr_s(Self::FORMAT_MARKER_KEY))
            .condition_expression("epoch = :epoch AND enabled = :enabled")
            .expression_attribute_values(":epoch", Self::attr_s(epoch))
            .expression_attribute_values(":enabled", AttributeValue::Bool(true))
            .build()
            .map_err(|e| Error::Internal(format!("failed to build append marker fence: {e}")))?;
        Ok(TransactWriteItem::builder().condition_check(check).build())
    }

    pub(super) fn fence_staged_publication(
        write: &mut TransactWriteItem,
        token: &str,
    ) -> Result<()> {
        let update = write.update.as_mut().ok_or_else(|| {
            Error::Internal("staged append publication is not an update".to_string())
        })?;
        let condition = update.condition_expression.as_mut().ok_or_else(|| {
            Error::Internal("staged append publication has no condition".to_string())
        })?;
        condition.push_str(" AND staging_token = :staging_token");
        update
            .expression_attribute_values
            .get_or_insert_default()
            .insert(":staging_token".to_string(), Self::attr_s(token));
        // Chunked runs are pinned, so their update already removes expires_at.
        update
            .update_expression
            .push_str(", staging_token, staging_head, staging_deadline_ms");
        Ok(())
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

    /// Read every prospective legacy item before changing any stored state.
    /// DynamoDB's 400 KiB item limit includes attribute names and binary bytes,
    /// rather than the base64-expanded transport representation.
    pub(crate) async fn preflight_append_materialization(&self, batch_size: usize) -> Result<()> {
        for (table, field, latest) in [
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
                    .limit(batch_size.clamp(1, 100) as i32)
                    .set_exclusive_start_key(start)
                    .send()
                    .await
                    .map_err(|e| {
                        Error::Internal(format!("failed to preflight append rollback: {e}"))
                    })?;
                for item in page.items() {
                    if !latest
                        && Self::item_num(item, Self::EXPIRES_AT_ATTR).is_some_and(|expires| {
                            expires <= sockudo_core::history::now_ms() / 1000
                        })
                    {
                        continue;
                    }
                    let key = Self::item_str(item, "message_version_key").unwrap_or_default();
                    if !latest
                        && (key.starts_with("__append_run__ ")
                            || key.starts_with("__append_chunk__ "))
                    {
                        continue;
                    }
                    let Some(payload) =
                        Self::item_bytes(item, field).filter(|bytes| is_compact(bytes))
                    else {
                        continue;
                    };
                    let app_channel = Self::item_str(item, "app_channel").ok_or_else(|| {
                        Error::Internal("append rollback partition missing".to_string())
                    })?;
                    let record = if latest {
                        self.materialize_latest_item(&app_channel, item).await?
                    } else {
                        self.materialize_items(&app_channel, std::slice::from_ref(item))
                            .await?
                            .pop()
                            .flatten()
                    };
                    let Some(record) = record else {
                        continue;
                    };
                    let mut legacy = item.clone();
                    legacy.insert(field.to_string(), Self::attr_b(encode_full(&record)?));
                    if latest {
                        for key in [
                            "latest_append_run",
                            "latest_append_len",
                            "latest_append_head",
                            "latest_append_pinned",
                            "latest_append_generation",
                        ] {
                            legacy.remove(key);
                        }
                    }
                    // Drop the source buffer before checking the reconstructed item.
                    drop(payload);
                    Self::validate_item_size(&legacy)?;
                }
                start = page
                    .last_evaluated_key()
                    .cloned()
                    .filter(|key| !key.is_empty());
                if start.is_none() {
                    break;
                }
            }
        }
        Ok(())
    }

    pub(super) fn validate_item_size(item: &HashMap<String, AttributeValue>) -> Result<()> {
        fn value_size(value: &AttributeValue) -> Result<usize> {
            Ok(match value {
                AttributeValue::S(value) => value.len(),
                AttributeValue::B(value) => value.as_ref().len(),
                // 38 significant digits plus sign/exponent allowance: a
                // conservative upper bound avoids underestimating numbers.
                AttributeValue::N(_) => 22,
                AttributeValue::Bool(_) | AttributeValue::Null(_) => 1,
                AttributeValue::L(values) => {
                    values.iter().try_fold(3, |size, value| -> Result<usize> {
                        Ok(size + 1 + value_size(value)?)
                    })?
                }
                AttributeValue::M(values) => 3 + values.len() + item_size(values)?,
                AttributeValue::Ss(values) => values.iter().map(String::len).sum(),
                AttributeValue::Bs(values) => values.iter().map(|v| v.as_ref().len()).sum(),
                AttributeValue::Ns(values) => 22 * values.len(),
                _ => {
                    return Err(Error::Internal(
                        "unsupported append rollback attribute".to_string(),
                    ));
                }
            })
        }
        fn item_size(item: &HashMap<String, AttributeValue>) -> Result<usize> {
            item.iter().try_fold(0, |size, (key, value)| {
                Ok(size + key.len() + value_size(value)?)
            })
        }
        if item_size(item)? > 400 * 1024 {
            return Err(Error::Configuration(
                "encoded item exceeds DynamoDB's 400 KiB item limit".to_string(),
            ));
        }
        Ok(())
    }

    /// Rewrite compact entries and receipts as self-contained records.
    pub(super) async fn materialize_compact_items(&self, batch_size: usize) -> Result<u64> {
        if self.append_storage_epoch().await?.is_some() {
            return Err(Error::Configuration(
                "disable append storage and drain writers before materialization".to_string(),
            ));
        }
        self.preflight_append_materialization(batch_size).await?;
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
                if let Some(expires) = Self::item_num(item, Self::EXPIRES_AT_ATTR)
                    .filter(|expires| *expires <= sockudo_core::history::now_ms() / 1000)
                {
                    // TTL deletion is asynchronous. Expired normal entries need
                    // no legacy representation and may have lost their run.
                    // Receipts/latest states have no TTL and never enter here.
                    let result = self
                        .client
                        .delete_item()
                        .table_name(&self.tables.version_entries)
                        .key(
                            "app_channel",
                            item.get("app_channel").cloned().ok_or_else(|| {
                                Error::Internal(
                                    "expired append entry partition missing".to_string(),
                                )
                            })?,
                        )
                        .key("message_version_key", Self::attr_s(&key))
                        .condition_expression("expires_at = :expires AND payload_bytes = :payload")
                        .expression_attribute_values(":expires", Self::attr_n(expires))
                        .expression_attribute_values(":payload", Self::attr_b(payload))
                        .send()
                        .await;
                    match result {
                        Ok(_) => {}
                        Err(error)
                            if error
                                .as_service_error()
                                .is_some_and(|e| e.is_conditional_check_failed_exception()) =>
                        {
                            tracing::debug!("expired append entry changed during rollback");
                        }
                        Err(_error) => {
                            tracing::warn!("expired append entry cleanup failed");
                            return Err(Error::Internal(
                                "failed to clean expired append rollback entry".to_string(),
                            ));
                        }
                    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use sockudo_core::versioned_messages::{VersionMetadata, VersionedMessage};
    use sockudo_protocol::messages::MessageData;

    #[test]
    fn rollback_item_limit_counts_attribute_names_and_raw_binary() {
        let item = HashMap::from([(
            "payload".to_string(),
            AttributeValue::B(Blob::new(vec![0; 400 * 1024 - 7])),
        )]);
        assert!(DynamoDbVersionStore::validate_item_size(&item).is_ok());
        let mut too_large = item;
        too_large.insert("flag".to_string(), AttributeValue::Bool(false));
        assert!(DynamoDbVersionStore::validate_item_size(&too_large).is_err());
    }

    #[tokio::test]
    #[ignore = "requires isolated DynamoDB Local on port 25473"]
    async fn version_history_crosses_service_pages_and_skips_expired_rows() {
        let settings = DynamoDbSettings {
            endpoint_url: Some("http://127.0.0.1:25473".to_string()),
            aws_access_key_id: Some("c2".to_string()),
            aws_secret_access_key: Some("c2-local-only".to_string()),
            ..Default::default()
        };
        let prefix = format!("c2pages{}", uuid::Uuid::new_v4().simple());
        let store = DynamoDbVersionStore::new(&settings, &prefix, 0)
            .await
            .unwrap();
        let message_serial = MessageSerial::new("msg:pages").unwrap();
        for n in 0..10 {
            let record = StoredVersionRecord {
                app_id: "pages".to_string(),
                channel: "room".to_string(),
                original_client_id: None,
                envelope: None,
                message: VersionedMessage::new_create(
                    message_serial.clone(),
                    VersionMetadata {
                        serial: VersionSerial::new(format!("ver:{n:020}")).unwrap(),
                        client_id: None,
                        timestamp_ms: n,
                        description: None,
                        metadata: None,
                    },
                    1,
                    n as u64 + 1,
                    None,
                    Some(MessageData::String("x".repeat(190_000))),
                    None,
                ),
            };
            let mut item = store
                .entry_item(&record, encode_full(&record).unwrap(), None)
                .unwrap();
            if n == 0 || n == 9 {
                item.insert(
                    DynamoDbVersionStore::EXPIRES_AT_ATTR.to_string(),
                    DynamoDbVersionStore::attr_n(1),
                );
            }
            store
                .client
                .put_item()
                .table_name(&store.tables.version_entries)
                .set_item(Some(item))
                .send()
                .await
                .unwrap();
        }
        for direction in [
            VersionStoreDirection::OldestFirst,
            VersionStoreDirection::NewestFirst,
        ] {
            let request = VersionStoreReadRequest {
                app_id: "pages".to_string(),
                channel: "room".to_string(),
                message_serial: message_serial.clone(),
                direction,
                cursor: None,
                limit: 7,
            };
            let page = store.get_versions(request.clone()).await.unwrap();
            assert_eq!(page.items.len(), 6);
            assert!(page.has_more);
            let cursor = page.next_cursor.unwrap();
            let last = store
                .get_versions(VersionStoreReadRequest {
                    cursor: Some(cursor),
                    ..request
                })
                .await
                .unwrap();
            assert_eq!(last.items.len(), 2);
            assert!(!last.has_more);
            let serials = page
                .items
                .iter()
                .chain(&last.items)
                .map(|record| record.version_serial().as_str().to_string())
                .collect::<Vec<_>>();
            let mut expected = (1..9).map(|n| format!("ver:{n:020}")).collect::<Vec<_>>();
            if matches!(direction, VersionStoreDirection::NewestFirst) {
                expected.reverse();
            }
            assert_eq!(serials, expected);
        }
        let replay = store
            .replay_after(VersionReplayRequest {
                app_id: "pages".to_string(),
                channel: "room".to_string(),
                after_delivery_serial: 0,
                limit: 10,
            })
            .await
            .unwrap();
        assert_eq!(
            replay
                .iter()
                .map(StoredVersionRecord::delivery_serial)
                .collect::<Vec<_>>(),
            (2..=9).collect::<Vec<_>>()
        );
        let mut requested = Vec::new();
        for n in 0..7 {
            let serial = MessageSerial::new(format!("msg:batch{n}")).unwrap();
            let record = StoredVersionRecord {
                app_id: "pages".to_string(),
                channel: "batch".to_string(),
                original_client_id: None,
                envelope: None,
                message: VersionedMessage::new_create(
                    serial.clone(),
                    VersionMetadata {
                        serial: VersionSerial::new("ver:0").unwrap(),
                        client_id: None,
                        timestamp_ms: 0,
                        description: None,
                        metadata: None,
                    },
                    n + 1,
                    n + 1,
                    None,
                    Some(MessageData::String("x".repeat(190_000))),
                    None,
                ),
            };
            assert!(matches!(
                store
                    .commit_create(VersionCreateRequest {
                        record,
                        limits: Default::default()
                    })
                    .await
                    .unwrap(),
                VersionCreateResult::Applied { .. }
            ));
            requested.push(serial);
        }
        let batch = store
            .get_latest_batch("pages", "batch", &requested)
            .await
            .unwrap();
        assert_eq!(batch.len(), requested.len());
        assert!(requested.iter().all(|serial| batch.contains_key(serial)));
        for n in 1..9 {
            store
                .client
                .update_item()
                .table_name(&store.tables.version_entries)
                .key(
                    "app_channel",
                    DynamoDbVersionStore::attr_s(&DynamoDbVersionStore::app_channel_key(
                        "pages", "room",
                    )),
                )
                .key(
                    "message_version_key",
                    DynamoDbVersionStore::attr_s(&DynamoDbVersionStore::message_version_key(
                        message_serial.as_str(),
                        &format!("ver:{n:020}"),
                    )),
                )
                .update_expression("SET expires_at = :expired")
                .expression_attribute_values(":expired", DynamoDbVersionStore::attr_n(1))
                .send()
                .await
                .unwrap();
        }
        let request = VersionStoreReadRequest {
            app_id: "pages".to_string(),
            channel: "room".to_string(),
            message_serial,
            direction: VersionStoreDirection::OldestFirst,
            cursor: None,
            limit: 7,
        };
        let expired = store.get_versions(request.clone()).await.unwrap();
        assert!(expired.items.is_empty());
        assert!(expired.has_more);
        let end = store
            .get_versions(VersionStoreReadRequest {
                cursor: expired.next_cursor,
                ..request
            })
            .await
            .unwrap();
        assert!(end.items.is_empty());
        assert!(!end.has_more);
        for table in [
            &store.tables.version_entries,
            &store.tables.version_messages,
            &store.tables.version_streams,
        ] {
            store
                .client
                .delete_table()
                .table_name(table)
                .send()
                .await
                .unwrap();
        }
    }
    #[tokio::test]
    #[ignore = "requires isolated DynamoDB Local on port 25473"]
    async fn rollback_preflight_rejects_oversized_legacy_items_before_rewrites() {
        use sockudo_core::version_store::{
            VersionCreateLimits, VersionMutation, VersionMutationLimits, VersionPrecondition,
        };
        use sockudo_core::versioned_messages::MessageAppend;
        let settings = DynamoDbSettings {
            endpoint_url: Some("http://127.0.0.1:25473".to_string()),
            aws_access_key_id: Some("c2".to_string()),
            aws_secret_access_key: Some("c2-local-only".to_string()),
            ..Default::default()
        };
        let prefix = format!("c2preflight{}", uuid::Uuid::new_v4().simple());
        let store = DynamoDbVersionStore::new(&settings, &prefix, 0)
            .await
            .unwrap();
        store.set_append_storage_enabled(true).await.unwrap();
        let metadata = |n| VersionMetadata {
            serial: VersionSerial::new(format!("ver:{n:020}")).unwrap(),
            client_id: None,
            timestamp_ms: n,
            description: None,
            metadata: None,
        };
        let record = StoredVersionRecord {
            app_id: "preflight".to_string(),
            channel: "room".to_string(),
            original_client_id: None,
            envelope: None,
            message: VersionedMessage::new_create(
                MessageSerial::new("msg:large").unwrap(),
                metadata(0),
                1,
                1,
                None,
                Some(MessageData::String("x".repeat(200_000))),
                None,
            ),
        };
        let mut oversized_create = record.clone();
        oversized_create.message.data = Some(MessageData::String("x".repeat(400 * 1024)));
        assert!(matches!(
            store
                .commit_create(VersionCreateRequest {
                    record: oversized_create,
                    limits: VersionCreateLimits::default()
                })
                .await,
            Err(Error::Configuration(_))
        ));
        assert!(
            store
                .client
                .scan()
                .table_name(&store.tables.version_entries)
                .consistent_read(true)
                .send()
                .await
                .unwrap()
                .items()
                .is_empty(),
            "oversized create must not stage chunks before validation"
        );
        let VersionCreateResult::Applied { mut record, .. } = store
            .commit_create(VersionCreateRequest {
                record,
                limits: VersionCreateLimits::default(),
            })
            .await
            .unwrap()
        else {
            panic!("create failed");
        };
        for n in 1..=2 {
            let result = store
                .compare_and_apply(VersionMutationRequest {
                    app_id: record.app_id.clone(),
                    channel: record.channel.clone(),
                    message_serial: record.message_serial().clone(),
                    expected: VersionPrecondition::from_record(&record),
                    version: metadata(n),
                    mutation: VersionMutation::Append(MessageAppend {
                        // 95 changed chunks fit when there is no receipt; the old
                        // fixed94 cap incorrectly rejected this valid transaction.
                        data_fragment: "y".repeat(if n == 1 {
                            94 * CHUNK_BYTES + 1
                        } else {
                            93 * CHUNK_BYTES + 1
                        }),
                        extras: None,
                    }),
                    idempotency: (n == 2).then(|| {
                        sockudo_core::message_envelope::PublishIdempotencyMetadata {
                            cache_key: "boundary-receipt".to_string(),
                            payload_fingerprint: "boundary".to_string(),
                        }
                    }),
                    limits: VersionMutationLimits::default(),
                })
                .await
                .unwrap();
            let VersionMutationResult::Applied { record: next, .. } = result else {
                panic!("append failed");
            };
            record = next;
        }
        let oversized_transaction = store
            .compare_and_apply(VersionMutationRequest {
                app_id: record.app_id.clone(),
                channel: record.channel.clone(),
                message_serial: record.message_serial().clone(),
                expected: VersionPrecondition::from_record(&record),
                version: metadata(3),
                mutation: VersionMutation::Append(MessageAppend {
                    // 99 changed chunks require two stage transactions (98 + 1).
                    data_fragment: "z".repeat(98 * CHUNK_BYTES + 1),
                    extras: None,
                }),
                idempotency: Some(sockudo_core::message_envelope::PublishIdempotencyMetadata {
                    cache_key: "too-many".to_string(),
                    payload_fingerprint: "too-many".to_string(),
                }),
                limits: VersionMutationLimits::default(),
            })
            .await;
        let VersionMutationResult::Applied { record: next, .. } = oversized_transaction.unwrap()
        else {
            panic!("staged append failed");
        };
        record = next;
        let reader = DynamoDbVersionStore::new(&settings, &prefix, 0)
            .await
            .unwrap();
        assert_eq!(
            reader
                .get_latest(&record.app_id, &record.channel, record.message_serial())
                .await
                .unwrap()
                .unwrap()
                .message
                .data,
            record.message.data
        );
        let duplicate = reader
            .compare_and_apply(VersionMutationRequest {
                app_id: record.app_id.clone(),
                channel: record.channel.clone(),
                message_serial: record.message_serial().clone(),
                expected: VersionPrecondition::from_record(&record),
                version: metadata(4),
                mutation: VersionMutation::Append(MessageAppend {
                    data_fragment: "ignored duplicate".to_string(),
                    extras: None,
                }),
                idempotency: Some(sockudo_core::message_envelope::PublishIdempotencyMetadata {
                    cache_key: "too-many".to_string(),
                    payload_fingerprint: "too-many".to_string(),
                }),
                limits: VersionMutationLimits::default(),
            })
            .await
            .unwrap();
        let VersionMutationResult::Duplicate {
            record: duplicate, ..
        } = duplicate
        else {
            panic!("staged receipt was not preserved");
        };
        assert_eq!(duplicate.message.data, record.message.data);
        assert!(store.preflight_append_materialization(1).await.is_err());
        assert!(store.append_storage_epoch().await.unwrap().is_some());
        store.set_append_storage_enabled(false).await.unwrap();
        assert!(store.materialize_append_storage(1).await.is_err());
        let partition = DynamoDbVersionStore::app_channel_key(&record.app_id, &record.channel);
        for key in [
            DynamoDbVersionStore::message_version_key(
                record.message_serial().as_str(),
                &format!("ver:{:020}", 1),
            ),
            DynamoDbVersionStore::message_version_key(
                record.message_serial().as_str(),
                &format!("ver:{:020}", 2),
            ),
            DynamoDbVersionStore::operation_receipt_key("boundary-receipt"),
        ] {
            let item = store
                .client
                .get_item()
                .table_name(&store.tables.version_entries)
                .key("app_channel", DynamoDbVersionStore::attr_s(&partition))
                .key("message_version_key", DynamoDbVersionStore::attr_s(&key))
                .consistent_read(true)
                .send()
                .await
                .unwrap()
                .item
                .unwrap();
            assert!(
                is_compact(&DynamoDbVersionStore::item_bytes(&item, "payload_bytes").unwrap()),
                "failed preflight must not rewrite any compact entry or receipt"
            );
        }
        assert_eq!(
            store
                .get_latest(&record.app_id, &record.channel, record.message_serial())
                .await
                .unwrap()
                .unwrap()
                .message
                .data,
            record.message.data
        );
        for table in [
            &store.tables.version_entries,
            &store.tables.version_messages,
            &store.tables.version_streams,
        ] {
            store
                .client
                .delete_table()
                .table_name(table)
                .send()
                .await
                .unwrap();
        }
    }
    #[tokio::test]
    #[ignore = "requires isolated DynamoDB Local on port 25473"]
    async fn staged_append_takeover_old_writer_and_marker_fences_preserve_committed_prefix() {
        use sockudo_core::version_store::{
            VersionCreateLimits, VersionMutation, VersionMutationLimits, VersionPrecondition,
        };
        use sockudo_core::versioned_messages::MessageAppend;
        let settings = DynamoDbSettings {
            endpoint_url: Some("http://127.0.0.1:25473".to_string()),
            aws_access_key_id: Some("c2".to_string()),
            aws_secret_access_key: Some("c2-local-only".to_string()),
            ..Default::default()
        };
        let prefix = format!("c2stage{}", uuid::Uuid::new_v4().simple());
        let store = DynamoDbVersionStore::new(&settings, &prefix, 0)
            .await
            .unwrap();
        store.set_append_storage_enabled(true).await.unwrap();
        let metadata = |n| VersionMetadata {
            serial: VersionSerial::new(format!("ver:{n:020}")).unwrap(),
            client_id: None,
            timestamp_ms: n,
            description: None,
            metadata: None,
        };
        let seed = "original🙂".repeat(1000);
        let record = StoredVersionRecord {
            app_id: "staging".to_string(),
            channel: "room".to_string(),
            original_client_id: None,
            envelope: None,
            message: VersionedMessage::new_create(
                MessageSerial::new("msg:staged").unwrap(),
                metadata(0),
                1,
                1,
                None,
                Some(MessageData::String(seed.clone())),
                None,
            ),
        };
        let VersionCreateResult::Applied { record, .. } = store
            .commit_create(VersionCreateRequest {
                record,
                limits: VersionCreateLimits::default(),
            })
            .await
            .unwrap()
        else {
            panic!("create failed");
        };
        let request = |current: &StoredVersionRecord, fragment: &str, n| VersionMutationRequest {
            app_id: current.app_id.clone(),
            channel: current.channel.clone(),
            message_serial: current.message_serial().clone(),
            expected: VersionPrecondition::from_record(current),
            version: metadata(n),
            mutation: VersionMutation::Append(MessageAppend {
                data_fragment: fragment.to_string(),
                extras: None,
            }),
            idempotency: None,
            limits: VersionMutationLimits::default(),
        };
        let partition = DynamoDbVersionStore::app_channel_key(&record.app_id, &record.channel);
        let latest = store
            .client
            .get_item()
            .table_name(&store.tables.version_messages)
            .key("app_channel", DynamoDbVersionStore::attr_s(&partition))
            .key(
                "message_serial",
                DynamoDbVersionStore::attr_s(record.message_serial().as_str()),
            )
            .consistent_read(true)
            .send()
            .await
            .unwrap()
            .item
            .unwrap();
        let predecessor = AppendRunRef {
            run: record.version_serial().clone(),
            data_len: seed.len() as u64,
            generation: DynamoDbVersionStore::item_str(&latest, "latest_append_generation"),
        };
        let candidate = |fragment: &str| {
            let VersionMutationResult::Applied { record: next, .. } = request(&record, fragment, 1)
                .apply_to(&record, "staging/room", 2, 0)
                .unwrap()
            else {
                panic!("candidate failed");
            };
            let plan = AppendRunPlan::for_record_chunked(
                record.version_serial(),
                Some(&predecessor),
                &next,
            );
            (next, plan)
        };
        let first_fragment = "a".repeat(10_000);
        let second_fragment = "b".repeat(10_000);
        let (first, first_plan) = candidate(&first_fragment);
        let (second, second_plan) = candidate(&second_fragment);
        let epoch = store.append_storage_epoch().await.unwrap().unwrap();
        let first_token = store
            .stage_append_chunks(
                &first,
                &first_plan,
                &epoch,
                &store.append_chunk_writes(&first, &first_plan).unwrap(),
            )
            .await
            .unwrap()
            .unwrap();
        // A fresh reader must not see the unpublished suffix or torn UTF-8 tail.
        let reader = DynamoDbVersionStore::new(&settings, &prefix, 0)
            .await
            .unwrap();
        assert_eq!(
            reader
                .get_latest(&record.app_id, &record.channel, record.message_serial())
                .await
                .unwrap()
                .unwrap()
                .message
                .data,
            record.message.data
        );
        assert!(
            store
                .stage_append_chunks(
                    &second,
                    &second_plan,
                    &epoch,
                    &store.append_chunk_writes(&second, &second_plan).unwrap()
                )
                .await
                .unwrap()
                .is_none()
        );
        store
            .client
            .update_item()
            .table_name(&store.tables.version_entries)
            .key("app_channel", DynamoDbVersionStore::attr_s(&partition))
            .key(
                "message_version_key",
                DynamoDbVersionStore::attr_s(&DynamoDbVersionStore::append_manifest_key(
                    &first,
                    first_plan.run().unwrap(),
                )),
            )
            .update_expression("SET staging_deadline_ms = :expired")
            .expression_attribute_values(":expired", DynamoDbVersionStore::attr_n(0))
            .send()
            .await
            .unwrap();
        let second_token = store
            .stage_append_chunks(
                &second,
                &second_plan,
                &epoch,
                &store.append_chunk_writes(&second, &second_plan).unwrap(),
            )
            .await
            .unwrap()
            .unwrap();
        assert_ne!(first_token, second_token);
        let mut stale = store
            .append_run_write(&first, &first_plan, true, 1)
            .unwrap()
            .unwrap();
        DynamoDbVersionStore::fence_staged_publication(&mut stale, &first_token).unwrap();
        let error = store
            .client
            .transact_write_items()
            .transact_items(stale)
            .send()
            .await
            .unwrap_err();
        assert!(
            error
                .as_service_error()
                .unwrap()
                .is_transaction_canceled_exception()
        );
        // The ordinary atomic path intentionally ignores leases, just like an
        // older format2 binary. Its head CAS fences every staged batch/commit.
        let VersionMutationResult::Applied {
            record: committed, ..
        } = store
            .compare_and_apply(request(&record, "committed", 1))
            .await
            .unwrap()
        else {
            panic!("ordinary writer failed");
        };
        let mut stale = store
            .append_run_write(&second, &second_plan, true, 1)
            .unwrap()
            .unwrap();
        DynamoDbVersionStore::fence_staged_publication(&mut stale, &second_token).unwrap();
        let error = store
            .client
            .transact_write_items()
            .transact_items(stale)
            .send()
            .await
            .unwrap_err();
        assert!(
            error
                .as_service_error()
                .unwrap()
                .is_transaction_canceled_exception()
        );
        assert!(
            store
                .stage_append_chunks(
                    &second,
                    &second_plan,
                    &epoch,
                    &store.append_chunk_writes(&second, &second_plan).unwrap()
                )
                .await
                .unwrap()
                .is_none()
        );
        let reader = DynamoDbVersionStore::new(&settings, &prefix, 0)
            .await
            .unwrap();
        assert_eq!(
            reader
                .get_latest(&record.app_id, &record.channel, record.message_serial())
                .await
                .unwrap()
                .unwrap()
                .message
                .data,
            Some(MessageData::String(seed + "committed"))
        );
        let mut previous = second_plan.run().unwrap().clone();
        previous.data_len = committed.data_bytes().unwrap() as u64;
        let VersionMutationResult::Applied {
            record: candidate, ..
        } = request(&committed, "never-visible", 2)
            .apply_to(&committed, "staging/room", 3, 1)
            .unwrap()
        else {
            panic!("candidate failed");
        };
        let plan = AppendRunPlan::for_record_chunked(
            committed.version_serial(),
            Some(&previous),
            &candidate,
        );
        store.set_append_storage_enabled(false).await.unwrap();
        assert!(
            store
                .stage_append_chunks(
                    &candidate,
                    &plan,
                    &epoch,
                    &store.append_chunk_writes(&candidate, &plan).unwrap()
                )
                .await
                .unwrap()
                .is_none()
        );
        // Simulate a TTL-delayed expired compact entry whose chunks are gone.
        let missing_run = AppendRunPlan::for_seed_record(&candidate);
        let mut expired = store
            .entry_item(&candidate, missing_run.encode(&candidate).unwrap(), None)
            .unwrap();
        expired.insert(
            DynamoDbVersionStore::EXPIRES_AT_ATTR.to_string(),
            DynamoDbVersionStore::attr_n(1),
        );
        store
            .client
            .put_item()
            .table_name(&store.tables.version_entries)
            .set_item(Some(expired))
            .send()
            .await
            .unwrap();
        store.preflight_append_materialization(1).await.unwrap();
        assert!(store.materialize_append_storage(1).await.unwrap() > 0);
        assert_eq!(
            reader
                .get_latest(&record.app_id, &record.channel, record.message_serial())
                .await
                .unwrap()
                .unwrap()
                .message
                .data,
            committed.message.data
        );
        for table in [
            &store.tables.version_entries,
            &store.tables.version_messages,
            &store.tables.version_streams,
        ] {
            store
                .client
                .delete_table()
                .table_name(table)
                .send()
                .await
                .unwrap();
        }
    }
}
