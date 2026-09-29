use super::*;
use sockudo_core::version_store::append_storage::{
    AppendRunPlan, AppendRunRef, AppendRunSnapshots, CHUNK_BYTES, StoredVersionPayload,
    encode_full, expand_payloads, is_compact, snapshot_from_bytes,
};
use sockudo_core::versioned_messages::VersionSerial;
use std::collections::BTreeMap;

/// One accumulated snapshot per append run; see
/// `sockudo_core::version_store::append_storage`. The snapshot is stored as
/// a string value rather than a byte vector, which SurrealDB would persist as
/// an array of numbers.
#[cfg(feature = "versioned-messages")]
#[derive(Debug, Clone, Serialize, Deserialize, SurrealValue)]
pub(super) struct StoredVersionAppendRunRec {
    pub(super) app_id: String,
    pub(super) channel: String,
    pub(super) message_serial: String,
    pub(super) run_version_serial: String,
    pub(super) head_version_serial: String,
    #[serde(default)]
    pub(super) data: String,
    #[serde(default)]
    pub(super) generation: Option<String>,
    pub(super) data_len: i64,
    /// Referenced by an idempotency receipt; receipts are never purged, so
    /// neither is a snapshot a duplicate replay reconstructs from.
    pub(super) pinned: bool,
    pub(super) updated_at_ms: i64,
}

#[cfg(feature = "versioned-messages")]
#[derive(Debug, Clone, Deserialize, SurrealValue)]
struct PayloadWithId {
    id: surrealdb::types::RecordId,
    payload_bytes: StoredPayloadBytes,
}

#[derive(Debug, Clone, Serialize, Deserialize, SurrealValue)]
pub(super) struct StoredAppendFormat {
    pub(super) enabled: bool,
    pub(super) epoch: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, SurrealValue)]
pub(super) struct StoredAppendChunk {
    pub(super) chunk_index: i64,
    pub(super) data_bytes: surrealdb::types::Bytes,
}

#[derive(Debug, Clone, Serialize, Deserialize, SurrealValue)]
pub(super) struct AppendChunkMutation {
    pub(super) id: String,
    pub(super) chunk_index: i64,
    pub(super) data_bytes: surrealdb::types::Bytes,
}

#[derive(Debug, Clone, Serialize, Deserialize, SurrealValue)]
pub(super) struct AppendSeed {
    pub(super) run_id: String,
    pub(super) run: StoredVersionAppendRunRec,
    pub(super) chunks: Vec<AppendChunkMutation>,
}

pub(super) const WRITE_SEEDS: &str = "FOR $seed IN $seeds { UPSERT type::record($run_table, $seed.run_id) CONTENT $seed.run; FOR $chunk IN $seed.chunks { UPSERT type::record($chunk_table, $chunk.id) SET run_id = $seed.run_id, generation = $seed.run.generation, chunk_index = $chunk.chunk_index, data_bytes = $chunk.data_bytes; }; };";
pub(super) const FORMAT_FENCE: &str = "LET $format_write = UPDATE ONLY type::record($format_table, 'format') SET fence += 1 WHERE epoch = $format_epoch AND enabled = $format_enabled RETURN AFTER; IF $format_write = NONE { THROW 'version_conflict'; };";

pub(super) fn seed_writes(
    record: &StoredVersionRecord,
    plan: &AppendRunPlan,
    now_ms: i64,
) -> Result<Vec<AppendSeed>> {
    let Some(run) = plan.run() else {
        return Ok(Vec::new());
    };
    let generation = run.generation.as_deref().unwrap_or_default();
    let run_id = storage_run_id(
        &record.app_id,
        &record.channel,
        record.message_serial().as_str(),
        run.run.as_str(),
        run.generation.as_deref(),
    );
    let chunks = plan
        .chunk_writes(record)?
        .into_iter()
        .map(|chunk| AppendChunkMutation {
            id: deterministic_key(
                [
                    record.app_id.as_str(),
                    record.channel.as_str(),
                    record.message_serial().as_str(),
                    generation,
                    &chunk.index.to_string(),
                ]
                .into_iter(),
            ),
            chunk_index: chunk.index as i64,
            data_bytes: chunk.bytes.into(),
        })
        .collect();
    Ok(vec![AppendSeed {
        run_id,
        chunks,
        run: StoredVersionAppendRunRec {
            app_id: record.app_id.clone(),
            channel: record.channel.clone(),
            message_serial: record.message_serial().as_str().to_string(),
            run_version_serial: run.run.as_str().to_string(),
            head_version_serial: record.version_serial().as_str().to_string(),
            data: String::new(),
            generation: run.generation.clone(),
            data_len: run.data_len as i64,
            pinned: false,
            updated_at_ms: now_ms,
        },
    }])
}

pub(super) fn append_run_id(
    app_id: &str,
    channel: &str,
    message_serial: &str,
    run: &str,
) -> String {
    deterministic_key([app_id, channel, message_serial, run].into_iter())
}

pub(super) fn storage_run_id(
    app_id: &str,
    channel: &str,
    message: &str,
    run: &str,
    generation: Option<&str>,
) -> String {
    match generation {
        Some(generation) => {
            deterministic_key([app_id, channel, message, run, generation].into_iter())
        }
        None => append_run_id(app_id, channel, message, run),
    }
}

/// Chunks can split a Unicode scalar; validate UTF-8 only after joining the
/// requested byte prefix, and reject missing/reordered/oversized chunks.
fn assemble_prefix(chunks: Vec<StoredAppendChunk>, data_len: u64) -> Result<String> {
    let count = data_len.div_ceil(CHUNK_BYTES as u64);
    if chunks.len() as u64 != count {
        return Err(Error::Internal(
            "append storage chunk is missing".to_string(),
        ));
    }
    let len = usize::try_from(data_len)
        .map_err(|_| Error::Internal("append prefix length overflow".to_string()))?;
    let mut bytes = Vec::new();
    for (index, chunk) in chunks.into_iter().enumerate() {
        if chunk.chunk_index != index as i64
            || chunk.data_bytes.len() > CHUNK_BYTES
            || (index as u64 + 1 < count && chunk.data_bytes.len() != CHUNK_BYTES)
        {
            return Err(Error::Internal("invalid append storage chunk".to_string()));
        }
        bytes.extend_from_slice(&chunk.data_bytes);
    }
    if bytes.len() < len {
        return Err(Error::Internal(
            "append storage prefix is incomplete".to_string(),
        ));
    }
    bytes.truncate(len);
    snapshot_from_bytes(bytes)
}

#[cfg(feature = "versioned-messages")]
impl SurrealVersionStore {
    async fn load_append_run(
        &self,
        app_id: &str,
        channel: &str,
        message_serial: &str,
        run: &str,
    ) -> Result<Option<StoredVersionAppendRunRec>> {
        self.db
            .select((
                self.tables.runs.clone(),
                append_run_id(app_id, channel, message_serial, run),
            ))
            .await
            .map_err(|e| Error::Internal(format!("failed to read append run: {e}")))
    }

    pub(super) async fn append_format(&self) -> Result<StoredAppendFormat> {
        self.db
            .select::<Option<StoredAppendFormat>>((self.tables.format.clone(), "format"))
            .await
            .map_err(|e| Error::Internal(format!("failed to read append storage format: {e}")))?
            .ok_or_else(|| Error::Internal("append storage format marker is missing".to_string()))
    }

    /// Maintenance runs with writers stopped. Keep the marker off until every
    /// current aggregate has a seed; a failed/repeated enable remains readable.
    pub(super) async fn seed_latest_records(&self, format: &StoredAppendFormat) -> Result<()> {
        let mut start = 0i64;
        loop {
            let mut response = self
                .db
                .query(format!(
                    "SELECT * FROM {} ORDER BY id LIMIT 100 START $start",
                    self.tables.messages
                ))
                .bind(("start", start))
                .await
                .map_err(|e| {
                    Error::Internal(format!("failed to scan append seed candidates: {e}"))
                })?;
            let messages: Vec<StoredVersionMessageRec> = response.take(0usize).map_err(|e| {
                Error::Internal(format!("failed to decode append seed candidates: {e}"))
            })?;
            let count = messages.len();
            for message in messages {
                if message.latest_append_generation.is_some()
                    && message.latest_append_head.as_deref()
                        == Some(message.latest_version_serial.as_str())
                {
                    continue;
                }
                let record = if message.latest_payload_bytes.is_empty() {
                    self.get_latest(
                        &message.app_id,
                        &message.channel,
                        &MessageSerial::new(&message.message_serial)?,
                    )
                    .await?
                    .ok_or_else(|| {
                        Error::Internal("append seed predecessor is missing".to_string())
                    })?
                } else {
                    self.materialize_payloads(
                        &message.app_id,
                        &message.channel,
                        vec![message.latest_payload_bytes.clone()],
                    )
                    .await?
                    .pop()
                    .ok_or_else(|| {
                        Error::Internal("append seed predecessor is empty".to_string())
                    })?
                };
                let plan = AppendRunPlan::for_seed_record(&record);
                let Some(run) = plan.run() else {
                    continue;
                };
                let message_id = deterministic_key(
                    [
                        message.app_id.as_str(),
                        message.channel.as_str(),
                        message.message_serial.as_str(),
                    ]
                    .into_iter(),
                );
                self.db.query(format!("BEGIN TRANSACTION; {FORMAT_FENCE} LET $seeded = UPDATE ONLY type::record($message_table, $message_id) SET latest_append_run = $run, latest_append_len = $len, latest_append_head = $head, latest_append_pinned = false, latest_append_generation = $generation WHERE latest_version_serial = $head RETURN AFTER; IF $seeded = NONE {{ THROW 'version_conflict'; }}; {WRITE_SEEDS} COMMIT TRANSACTION;"))
                    .bind(("format_table", self.tables.format.clone())).bind(("format_epoch", format.epoch)).bind(("format_enabled", format.enabled))
                    .bind(("message_table", self.tables.messages.clone())).bind(("message_id", message_id))
                    .bind(("run", run.run.as_str().to_string())).bind(("len", run.data_len as i64))
                    .bind(("head", record.version_serial().as_str().to_string())).bind(("generation", run.generation.clone()))
                    .bind(("run_table", self.tables.runs.clone())).bind(("chunk_table", self.tables.chunks.clone()))
                    .bind(("seeds", seed_writes(&record, &plan, sockudo_core::history::now_ms())?))
                    .await.and_then(|response| response.check())
                    .map_err(|e| Error::Internal(format!("failed to seed append predecessor: {e}")))?;
            }
            if count < 100 {
                break;
            }
            start += count as i64;
        }
        Ok(())
    }

    /// Cache hits cover only immutable prefixes, keyed by the persisted run
    /// generation. A short cached prefix never serves a newer remote append.
    pub(super) async fn materialize_payloads(
        &self,
        app_id: &str,
        channel: &str,
        payloads: Vec<StoredPayloadBytes>,
    ) -> Result<Vec<StoredVersionRecord>> {
        let payloads = payloads
            .iter()
            .map(|bytes| StoredVersionPayload::decode(bytes))
            .collect::<Result<Vec<_>>>()?;
        let mut needed = BTreeMap::<(MessageSerial, VersionSerial), AppendRunRef>::new();
        for payload in &payloads {
            if let Some(run) = payload.run() {
                let key = (payload.record().message_serial().clone(), run.run.clone());
                if let Some(previous) = needed.get(&key)
                    && previous.generation != run.generation
                {
                    return Err(Error::Internal(
                        "append run generation mismatch".to_string(),
                    ));
                }
                needed
                    .entry(key)
                    .and_modify(|previous| {
                        previous.data_len = previous.data_len.max(run.data_len);
                    })
                    .or_insert_with(|| run.clone());
            }
        }
        let mut snapshots = AppendRunSnapshots::new();
        let mut admissions = Vec::new();
        for ((message, serial), run) in needed {
            if let Some(snapshot) = self.append_cache.get(app_id, channel, &message, &run) {
                snapshots.insert((message, serial), snapshot);
                continue;
            }
            let snapshot = if let Some(generation) = run.generation.as_deref() {
                let run_id = storage_run_id(
                    app_id,
                    channel,
                    message.as_str(),
                    serial.as_str(),
                    Some(generation),
                );
                let count = run.data_len.div_ceil(CHUNK_BYTES as u64);
                let mut response = self.db.query(format!(
                    "SELECT chunk_index, data_bytes FROM {} WHERE run_id = $run_id AND generation = $generation AND chunk_index < $count ORDER BY chunk_index ASC",
                    self.tables.chunks,
                ))
                .bind(("run_id", run_id))
                .bind(("generation", generation.to_string()))
                .bind(("count", count as i64))
                .await
                .map_err(|e| Error::Internal(format!("failed to read append chunks: {e}")))?;
                let chunks: Vec<StoredAppendChunk> = response
                    .take(0usize)
                    .map_err(|e| Error::Internal(format!("failed to decode append chunks: {e}")))?;
                assemble_prefix(chunks, run.data_len)?
            } else {
                let record = self
                    .load_append_run(app_id, channel, message.as_str(), serial.as_str())
                    .await?
                    .ok_or_else(|| Error::Internal("append storage run is missing".to_string()))?;
                record.data
            };
            // Reconstruction performs the length, UTF-8 and fragment checks
            // even for cache hits.
            admissions.push((message.clone(), run));
            snapshots.insert((message, serial), snapshot);
        }
        let records = expand_payloads(payloads, &snapshots)?;
        for (message, run) in admissions {
            if let Some(snapshot) = snapshots.get(&(message.clone(), run.run.clone())) {
                self.append_cache
                    .insert(app_id, channel, &message, &run, snapshot.clone());
            }
        }
        Ok(records)
    }

    /// Purge a run and its chunks in one transaction, after re-checking that
    /// no retained entry or latest-state record can still reference it.
    pub(super) async fn purge_append_runs(
        &self,
        before_ms: i64,
        limit: i64,
    ) -> Result<(u64, bool)> {
        let mut response = self
            .db
            .query(format!(
                "SELECT * FROM {} WHERE updated_at_ms < $cutoff AND pinned = false LIMIT $limit",
                self.tables.runs,
            ))
            .bind(("cutoff", before_ms))
            .bind(("limit", limit))
            .await
            .map_err(|e| Error::Internal(format!("failed to select expired append runs: {e}")))?;
        let runs: Vec<StoredVersionAppendRunRec> = response
            .take(0usize)
            .map_err(|e| Error::Internal(format!("failed to decode expired append runs: {e}")))?;
        let more = runs.len() as i64 == limit;
        let mut deleted = 0;
        for run in runs {
            let run_id = storage_run_id(
                &run.app_id,
                &run.channel,
                &run.message_serial,
                &run.run_version_serial,
                run.generation.as_deref(),
            );
            let mut response = self.db.query(format!(
                "BEGIN TRANSACTION; UPDATE ONLY type::record($format_table, 'format') SET fence += 1; LET $deleted = DELETE type::record($run_table, $run_id) WHERE updated_at_ms < $cutoff AND pinned = false AND array::len((SELECT VALUE id FROM {entries} WHERE app_id = $app_id AND channel = $channel AND message_serial = $message AND version_serial >= $run_start AND version_serial <= $run_head LIMIT 1)) = 0 AND array::len((SELECT VALUE id FROM {messages} WHERE app_id = $app_id AND channel = $channel AND message_serial = $message AND latest_append_run = $run_start LIMIT 1)) = 0 RETURN BEFORE; IF array::len($deleted) > 0 {{ DELETE {chunks} WHERE run_id = $run_id; }}; RETURN array::len($deleted); COMMIT TRANSACTION;",
                entries = self.tables.entries, messages = self.tables.messages, chunks = self.tables.chunks,
            ))
                .bind(("format_table", self.tables.format.clone()))
                .bind(("run_table", self.tables.runs.clone()))
                .bind(("run_id", run_id))
                .bind(("cutoff", before_ms))
                .bind(("app_id", run.app_id))
                .bind(("channel", run.channel))
                .bind(("message", run.message_serial))
                .bind(("run_start", run.run_version_serial))
                .bind(("run_head", run.head_version_serial))
                .await.and_then(|response| response.check())
                .map_err(|e| Error::Internal(format!("failed to delete expired append run: {e}")))?;
            let count: Option<u64> = response.take(4usize).map_err(|e| {
                Error::Internal(format!("failed to decode append purge count: {e}"))
            })?;
            deleted += count.unwrap_or(0);
        }
        Ok((deleted, more))
    }

    /// Rewrite compact entries and receipts as self-contained records.
    pub(super) async fn materialize_compact_records(&self, batch_size: usize) -> Result<u64> {
        let format = self.append_format().await?;
        if format.enabled {
            return Err(Error::InvalidMessageFormat(
                "disable append storage before materialization".to_string(),
            ));
        }
        let limit = i64::try_from(batch_size.max(1)).unwrap_or(i64::MAX);
        let mut rewritten = 0;
        for (table, column) in [
            (self.tables.entries.clone(), "payload_bytes"),
            (self.tables.receipts.clone(), "payload_bytes"),
            (self.tables.messages.clone(), "latest_payload_bytes"),
        ] {
            let mut start = 0i64;
            loop {
                let mut response = self
                    .db
                    .query(format!(
                        "SELECT id, {column} AS payload_bytes FROM {table} ORDER BY id LIMIT $limit START $start"
                    ))
                    .bind(("limit", limit))
                    .bind(("start", start))
                    .await
                    .map_err(|e| Error::Internal(format!("failed to scan {table}: {e}")))?;
                let rows: Vec<PayloadWithId> = response
                    .take(0usize)
                    .map_err(|e| Error::Internal(format!("failed to decode {table}: {e}")))?;
                let count = rows.len() as i64;
                for row in rows {
                    if !is_compact(&row.payload_bytes) {
                        continue;
                    }
                    let (app_id, channel) = {
                        let decoded = StoredVersionPayload::decode(&row.payload_bytes)?;
                        (
                            decoded.record().app_id.clone(),
                            decoded.record().channel.clone(),
                        )
                    };
                    let record = self
                        .materialize_payloads(&app_id, &channel, vec![row.payload_bytes.clone()])
                        .await?
                        .pop()
                        .ok_or_else(|| Error::Internal("compact record is empty".to_string()))?;
                    self.db
                        .query(format!("BEGIN TRANSACTION; LET $fence = UPDATE ONLY type::record($format_table, 'format') SET fence += 1 WHERE enabled = false AND epoch = $epoch RETURN AFTER; IF $fence = NONE {{ THROW 'append_format_changed'; }}; UPDATE $id SET {column} = $payload WHERE {column} = $previous; COMMIT TRANSACTION;"))
                        .bind(("format_table", self.tables.format.clone()))
                        .bind(("epoch", format.epoch))
                        .bind(("previous", row.payload_bytes))
                        .bind(("id", row.id))
                        .bind(("payload", encode_full(&record)?))
                        .await
                        .and_then(|response| response.check())
                        .map_err(|e| {
                            Error::Internal(format!("failed to rewrite compact record: {e}"))
                        })?;
                    rewritten += 1;
                }
                if count < limit {
                    break;
                }
                start += count;
            }
        }
        Ok(rewritten)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chunks(data: &[u8]) -> Vec<StoredAppendChunk> {
        data.chunks(CHUNK_BYTES)
            .enumerate()
            .map(|(index, bytes)| StoredAppendChunk {
                chunk_index: index as i64,
                data_bytes: bytes.to_vec().into(),
            })
            .collect()
    }

    #[test]
    fn chunk_transport_uses_native_bytes() {
        let chunk = AppendChunkMutation {
            id: "chunk".into(),
            chunk_index: 0,
            data_bytes: vec![0, 127, 255].into(),
        };
        let value = chunk.into_value();
        let surrealdb::types::Value::Object(object) = &value else {
            panic!("chunk must be an object");
        };
        assert!(matches!(
            object.get("data_bytes"),
            Some(surrealdb::types::Value::Bytes(_))
        ));
        let decoded = AppendChunkMutation::from_value(value).unwrap();
        assert_eq!(decoded.data_bytes.as_ref(), &[0, 127, 255]);
    }

    #[test]
    fn chunk_prefix_rejoins_split_multibyte_and_ignores_later_append() {
        let expected = format!("{}🦀", "a".repeat(CHUNK_BYTES - 1));
        let later = format!("{expected}later");
        assert_eq!(
            assemble_prefix(chunks(later.as_bytes()), expected.len() as u64).unwrap(),
            expected
        );
        assert!(assemble_prefix(chunks(later.as_bytes()), CHUNK_BYTES as u64 + 1).is_err());
    }

    #[test]
    fn chunk_prefix_exact_boundary_and_empty() {
        let data = "a".repeat(CHUNK_BYTES);
        assert_eq!(
            assemble_prefix(chunks(data.as_bytes()), data.len() as u64).unwrap(),
            data
        );
        assert_eq!(assemble_prefix(Vec::new(), 0).unwrap(), "");
    }

    #[test]
    fn chunk_prefix_rejects_missing_short_reordered_and_oversized_chunks() {
        let data = vec![b'a'; CHUNK_BYTES + 2];
        let mut missing = chunks(&data);
        missing.pop();
        assert!(assemble_prefix(missing, data.len() as u64).is_err());
        let mut short = chunks(&data);
        short[0].data_bytes = short[0].data_bytes[..CHUNK_BYTES - 1].to_vec().into();
        assert!(assemble_prefix(short, data.len() as u64).is_err());
        let mut reordered = chunks(&data);
        reordered.swap(0, 1);
        assert!(assemble_prefix(reordered, data.len() as u64).is_err());
        let mut oversized = chunks(&data);
        oversized[0].data_bytes = vec![b'a'; CHUNK_BYTES + 1].into();
        assert!(assemble_prefix(oversized, data.len() as u64).is_err());
    }
    #[tokio::test]
    #[ignore = "requires isolated C2 SurrealDB service on port 25475"]
    async fn surreal_import_seed_preserves_full_record_and_first_append() {
        use sockudo_core::version_store::{
            VersionMutation, VersionMutationLimits, VersionPrecondition,
        };
        use sockudo_core::versioned_messages::{VersionMetadata, VersionedMessage};
        use sockudo_protocol::messages::MessageData;

        let config = SurrealDbSettings {
            url: "ws://127.0.0.1:25475".into(),
            namespace: "c2".into(),
            database: "c2".into(),
            password: "c2-local-only".into(),
            ..Default::default()
        };
        let prefix = format!("c2seed{}", uuid::Uuid::new_v4().simple());
        let store = create_surreal_version_store(&config, &prefix)
            .await
            .unwrap();
        store.set_append_storage_enabled(true).await.unwrap();
        let position = store
            .reserve_delivery_position("app", "channel")
            .await
            .unwrap();
        let meta = |n| VersionMetadata {
            serial: VersionSerial::new(format!("ver:{n:020}")).unwrap(),
            client_id: None,
            timestamp_ms: n,
            description: None,
            metadata: None,
        };
        let imported = StoredVersionRecord {
            app_id: "app".into(),
            channel: "channel".into(),
            original_client_id: None,
            envelope: None,
            message: VersionedMessage::new_create(
                MessageSerial::new("msg:import").unwrap(),
                meta(0),
                1,
                position.delivery_serial,
                None,
                Some(MessageData::String("a".repeat(CHUNK_BYTES * 3 - 1))),
                None,
            ),
        };
        store.append_version(imported.clone()).await.unwrap();
        store.append_version(imported.clone()).await.unwrap();
        assert_eq!(
            sonic_rs::to_vec(
                &store
                    .get_latest("app", "channel", imported.message_serial())
                    .await
                    .unwrap()
                    .unwrap()
            )
            .unwrap(),
            sonic_rs::to_vec(&imported).unwrap()
        );
        let result = store
            .compare_and_apply(VersionMutationRequest {
                app_id: "app".into(),
                channel: "channel".into(),
                message_serial: imported.message_serial().clone(),
                expected: VersionPrecondition::from_record(&imported),
                version: meta(1),
                mutation: VersionMutation::Append(
                    sockudo_core::versioned_messages::MessageAppend {
                        data_fragment: "🦀".into(),
                        extras: None,
                    },
                ),
                idempotency: None,
                limits: VersionMutationLimits::default(),
            })
            .await
            .unwrap();
        let VersionMutationResult::Applied { record, .. } = result else {
            panic!("append did not apply: {result:?}");
        };
        let db = connect(config.url.as_str()).await.unwrap();
        db.signin(Root {
            username: config.username.clone(),
            password: config.password.clone(),
        })
        .await
        .unwrap();
        db.use_ns(&config.namespace)
            .use_db(&config.database)
            .await
            .unwrap();
        let message: Option<StoredVersionMessageRec> = db
            .select((
                format!("{prefix}_version_messages"),
                deterministic_key(["app", "channel", "msg:import"].into_iter()),
            ))
            .await
            .unwrap();
        let payload = StoredVersionPayload::decode(&message.unwrap().latest_payload_bytes).unwrap();
        assert_eq!(
            payload.run().unwrap().run,
            *imported.version_serial(),
            "append must extend the seed rather than copy the base into a new run"
        );
        assert!(payload.run().unwrap().generation.is_some());
        assert_eq!(
            sonic_rs::to_vec(
                &store
                    .get_latest("app", "channel", record.message_serial())
                    .await
                    .unwrap()
                    .unwrap()
            )
            .unwrap(),
            sonic_rs::to_vec(&record).unwrap()
        );
        for suffix in [
            "streams",
            "messages",
            "entries",
            "receipts",
            "append_runs",
            "append_chunks",
            "append_format",
        ] {
            db.query(format!("REMOVE TABLE {prefix}_version_{suffix}"))
                .await
                .unwrap()
                .check()
                .unwrap();
        }
    }
}
