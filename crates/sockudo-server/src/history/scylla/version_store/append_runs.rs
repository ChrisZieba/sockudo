use super::*;
use futures_util::StreamExt;
use sockudo_core::version_store::append_storage::{
    AppendRunPlan, AppendRunRef, AppendRunSnapshots, CHUNK_BYTES, StoredVersionPayload,
    encode_full, expand_payloads, is_compact, snapshot_from_bytes,
};
use sockudo_core::versioned_messages::{MessageSerial, VersionSerial};
use std::collections::{BTreeSet, HashMap};

/// Run snapshots live in the commit partition. Serials never contain
/// whitespace, so this key cannot collide with another message's run.
pub(super) fn append_run_commit_key(message_serial: &str, run: &str) -> String {
    format!("r:{message_serial} {run}")
}

impl ScyllaVersionStore {
    pub(super) async fn stage_seed(&self, record: &StoredVersionRecord) -> Result<AppendRunPlan> {
        let plan = AppendRunPlan::for_seed_record(record);
        let Some(run) = plan.run() else {
            return Ok(plan);
        };
        let commits = self.tables.version_commits_fq();
        for chunk in plan.chunk_writes(record)? {
            self.session.query_unpaged(format!("INSERT INTO {commits} (app_id, channel, commit_key, payload_bytes) VALUES (?, ?, ?, ?)"),
                (&record.app_id, &record.channel, Self::append_chunk_key(record.message_serial().as_str(), run, chunk.index), chunk.bytes)).await
                .map_err(|e| Error::Internal(format!("failed to stage append chunk: {e}")))?;
        }
        self.session.query_unpaged(format!("INSERT INTO {commits} (app_id, channel, commit_key, latest_version_serial, append_len) VALUES (?, ?, ?, ?, ?)"),
            (&record.app_id, &record.channel, Self::run_manifest_key(record.message_serial().as_str(), run), record.version_serial().as_str(), run.data_len as i64)).await
            .map_err(|e| Error::Internal(format!("failed to stage append seed: {e}")))?;
        Ok(plan)
    }

    pub(super) fn run_manifest_key(message: &str, run: &AppendRunRef) -> String {
        let mut key = append_run_commit_key(message, run.run.as_str());
        if let Some(generation) = &run.generation {
            key.push(' ');
            key.push_str(generation);
        }
        key
    }

    pub(super) async fn seed_latest_states(&self) -> Result<()> {
        let commits = self.tables.version_commits_fq();
        let mut statement = Statement::new(format!(
            "SELECT app_id, channel, commit_key, payload_bytes FROM {commits}"
        ));
        statement.set_page_size(100);
        let mut rows = self
            .session
            .query_iter(statement, ())
            .await
            .map_err(|e| Error::Internal(format!("failed to scan append seeds: {e}")))?
            .rows_stream::<(String, String, String, Option<Vec<u8>>)>()
            .map_err(|e| Error::Internal(format!("failed to decode append seeds: {e}")))?;
        while let Some(row) = rows.next().await {
            let (app_id, channel, key, payload) =
                row.map_err(|e| Error::Internal(format!("failed to read append seed: {e}")))?;
            if !key.starts_with("m:") {
                continue;
            }
            let Some(payload) = payload else {
                continue;
            };
            let Some(record) = self
                .materialize_payloads(&app_id, &channel, vec![payload])
                .await?
                .pop()
            else {
                continue;
            };
            let seed = self.stage_seed(&record).await?;
            if let Some(run) = seed.run() {
                let result = self.session.query_unpaged(format!("UPDATE {commits} SET append_run = ?, append_len = ?, append_head = ?, append_generation = ? WHERE app_id = ? AND channel = ? AND commit_key = ? IF latest_version_serial = ? AND latest_delivery_serial = ?"),
                    (run.run.as_str(), run.data_len as i64, record.version_serial().as_str(), run.generation.as_deref(), &app_id, &channel, &key, record.version_serial().as_str(), record.delivery_serial() as i64)).await
                    .map_err(|e| Error::Internal(format!("failed to publish append seed: {e}")))?;
                if !version_batch_applied(result)? {
                    return Err(Error::Internal(
                        "latest state changed during append seed maintenance".to_string(),
                    ));
                }
            }
        }
        Ok(())
    }

    pub(super) async fn append_storage_enabled(&self) -> Result<bool> {
        let result = self
            .prepared
            .execute_unpaged(
                format!(
                    "SELECT enabled FROM {}_format WHERE marker = 'append'",
                    self.tables.version_commits_fq()
                ),
                (),
            )
            .await
            .map_err(|e| Error::Internal(format!("failed to read append storage marker: {e}")))?
            .into_rows_result()
            .map_err(|e| Error::Internal(format!("failed to decode append storage marker: {e}")))?;
        Ok(result
            .maybe_first_row::<(bool,)>()
            .map_err(|e| Error::Internal(format!("failed to decode append storage marker: {e}")))?
            .is_some_and(|row| row.0))
    }

    pub(super) fn append_chunk_key(message: &str, run: &AppendRunRef, index: u64) -> String {
        format!(
            "c:{message} {} {} {index:020}",
            run.run.as_str(),
            run.generation.as_deref().unwrap_or_default()
        )
    }

    async fn load_payload_runs(
        &self,
        app_id: &str,
        channel: &str,
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
        let mut snapshots = self.load_append_runs(app_id, channel, &legacy).await?;
        let mut needed: HashMap<(MessageSerial, VersionSerial), AppendRunRef> = HashMap::new();
        for payload in payloads {
            if let Some(run) = payload.run().filter(|run| run.generation.is_some()) {
                let entry = needed
                    .entry((payload.record().message_serial().clone(), run.run.clone()))
                    .or_insert_with(|| run.clone());
                if entry.generation != run.generation {
                    return Err(Error::Internal(
                        "conflicting append run generations".to_string(),
                    ));
                }
                entry.data_len = entry.data_len.max(run.data_len);
            }
        }
        for (key, run) in needed {
            if let Some(snapshot) = self.append_cache.get(app_id, channel, &key.0, &run) {
                snapshots.insert(key, snapshot);
                continue;
            }
            let count = run.data_len.div_ceil(CHUNK_BYTES as u64);
            let mut chunks = HashMap::new();
            for first in (0..count).step_by(100) {
                let keys = (first..count.min(first + 100))
                    .map(|index| (Self::append_chunk_key(key.0.as_str(), &run, index), index))
                    .collect::<HashMap<_, _>>();
                let rows = self.prepared.execute_unpaged(format!("SELECT commit_key, payload_bytes FROM {} WHERE app_id = ? AND channel = ? AND commit_key IN ?", self.tables.version_commits_fq()), (app_id, channel, keys.keys().cloned().collect::<Vec<_>>()))
                    .await.map_err(|e| Error::Internal(format!("failed to read append chunks: {e}")))?
                    .into_rows_result().map_err(|e| Error::Internal(format!("failed to decode append chunks: {e}")))?;
                for row in rows
                    .rows::<(String, Vec<u8>)>()
                    .map_err(|e| Error::Internal(format!("failed to read append chunks: {e}")))?
                {
                    let (chunk_key, bytes) = row.map_err(|e| {
                        Error::Internal(format!("failed to read append chunk: {e}"))
                    })?;
                    if let Some(index) = keys.get(&chunk_key) {
                        chunks.insert(*index, bytes);
                    }
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
            self.append_cache
                .insert(app_id, channel, &key.0, &run, snapshot.clone());
            snapshots.insert(key, snapshot);
        }
        Ok(snapshots)
    }

    /// Add the run-pointer columns to commit tables created before them.
    pub(super) async fn ensure_append_run_columns(&self) -> Result<()> {
        self.session.query_unpaged(format!("CREATE TABLE IF NOT EXISTS {}_format (marker text PRIMARY KEY, enabled boolean)", self.tables.version_commits_fq()), ())
            .await.map_err(|e| Error::Internal(format!("failed to create append storage marker: {e}")))?;
        let rows = self
            .session
            .query_unpaged(
                "SELECT column_name FROM system_schema.columns WHERE keyspace_name = ? AND table_name = ?",
                (&self.tables.keyspace, &self.tables.version_commits),
            )
            .await
            .map_err(|e| Error::Internal(format!("failed to inspect version commit columns: {e}")))?
            .into_rows_result()
            .map_err(|e| Error::Internal(format!("failed to decode version commit columns: {e}")))?;
        let columns = rows
            .rows::<(String,)>()
            .map_err(|e| Error::Internal(format!("failed to read version commit columns: {e}")))?
            .map(|row| row.map(|value| value.0))
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(|e| Error::Internal(format!("failed to read version commit columns: {e}")))?;
        for (column, kind) in [
            ("append_run", "text"),
            ("append_len", "bigint"),
            ("append_head", "text"),
            ("append_generation", "text"),
        ] {
            if columns.iter().any(|existing| existing == column) {
                continue;
            }
            let sql = format!(
                "ALTER TABLE {} ADD {column} {kind}",
                self.tables.version_commits_fq()
            );
            if let Err(error) = self.session.query_unpaged(sql, ()).await {
                // A concurrent installer may have added it first.
                let message = error.to_string();
                if !message.contains("conflicts with an existing column")
                    && !message.contains("already exists")
                {
                    return Err(Error::Internal(format!(
                        "failed to add version commit column: {error}"
                    )));
                }
            }
        }
        Ok(())
    }

    /// Run snapshots of one app/channel partition.
    pub(super) async fn load_append_runs(
        &self,
        app_id: &str,
        channel: &str,
        runs: &BTreeSet<(MessageSerial, VersionSerial)>,
    ) -> Result<AppendRunSnapshots> {
        if runs.is_empty() {
            return Ok(AppendRunSnapshots::new());
        }
        let keys = runs
            .iter()
            .map(|(message, run)| append_run_commit_key(message.as_str(), run.as_str()))
            .collect::<Vec<_>>();
        let sql = format!(
            "SELECT commit_key, payload_bytes FROM {} WHERE app_id = ? AND channel = ? AND commit_key IN ?",
            self.tables.version_commits_fq()
        );
        let rows = self
            .prepared
            .execute_unpaged(sql, (app_id, channel, &keys))
            .await
            .map_err(|e| Error::Internal(format!("failed to read append runs: {e}")))?
            .into_rows_result()
            .map_err(|e| Error::Internal(format!("failed to decode append runs: {e}")))?;
        let mut snapshots = AppendRunSnapshots::new();
        for row in rows
            .rows::<(String, Option<Vec<u8>>)>()
            .map_err(|e| Error::Internal(format!("failed to read append runs: {e}")))?
        {
            let (key, payload) =
                row.map_err(|e| Error::Internal(format!("failed to read append run: {e}")))?;
            let Some((message, run)) = runs.iter().find(|(message, run)| {
                key == append_run_commit_key(message.as_str(), run.as_str())
            }) else {
                continue;
            };
            let payload = payload
                .ok_or_else(|| Error::Internal("append run snapshot is missing".to_string()))?;
            snapshots.insert(
                (message.clone(), run.clone()),
                snapshot_from_bytes(payload)?,
            );
        }
        Ok(snapshots)
    }

    /// Decode stored payloads of one app/channel into full-state records.
    pub(super) async fn materialize_payloads(
        &self,
        app_id: &str,
        channel: &str,
        payloads: Vec<Vec<u8>>,
    ) -> Result<Vec<StoredVersionRecord>> {
        let payloads = payloads
            .iter()
            .map(|bytes| StoredVersionPayload::decode(bytes))
            .collect::<Result<Vec<_>>>()?;
        let snapshots = self.load_payload_runs(app_id, channel, &payloads).await?;
        expand_payloads(payloads, &snapshots)
    }

    /// Rewrite compact commit rows (versions, deliveries, receipts) and the
    /// legacy projections as self-contained records. Legacy projection rows
    /// keep their remaining TTL.
    pub(super) async fn materialize_compact_rows(&self, batch_size: usize) -> Result<u64> {
        if self.append_storage_enabled().await? {
            return Err(Error::Configuration(
                "disable append storage and drain writers before materialization".to_string(),
            ));
        }
        let page_size = i32::try_from(batch_size.max(1)).unwrap_or(i32::MAX);
        let mut rewritten = 0;
        let commits = self.tables.version_commits_fq();
        let mut statement = Statement::new(format!(
            "SELECT app_id, channel, commit_key, payload_bytes FROM {commits}"
        ));
        statement.set_page_size(page_size);
        let mut rows = self
            .session
            .query_iter(statement, ())
            .await
            .map_err(|e| Error::Internal(format!("failed to scan version commits: {e}")))?
            .rows_stream::<(String, String, String, Option<Vec<u8>>)>()
            .map_err(|e| Error::Internal(format!("failed to scan version commits: {e}")))?;
        while let Some(row) = rows.next().await {
            let (app_id, channel, key, payload) =
                row.map_err(|e| Error::Internal(format!("failed to read version commit: {e}")))?;
            if key.starts_with("m:") && payload.as_ref().is_some_and(|bytes| !is_compact(bytes)) {
                self.session.query_unpaged(format!("UPDATE {commits} SET append_run = null, append_len = null, append_head = null, append_generation = null WHERE app_id = ? AND channel = ? AND commit_key = ?"), (&app_id, &channel, &key)).await
                    .map_err(|e| Error::Internal(format!("failed to clear append seed pointer: {e}")))?;
            }
            // Run snapshots are raw data, not encoded version records.
            let Some(payload) = payload.filter(|payload| {
                ["v:", "d:", "o:", "m:"]
                    .iter()
                    .any(|prefix| key.starts_with(prefix))
                    && is_compact(payload)
            }) else {
                continue;
            };
            let record = self
                .materialize_payloads(&app_id, &channel, vec![payload])
                .await?
                .pop()
                .ok_or_else(|| Error::Internal("compact commit row is empty".to_string()))?;
            self.session
                .query_unpaged(
                    format!(
                        "UPDATE {commits} SET payload_bytes = ?, append_run = null, append_len = null, append_head = null, append_generation = null WHERE app_id = ? AND channel = ? AND commit_key = ?"
                    ),
                    (encode_full(&record)?, &app_id, &channel, &key),
                )
                .await
                .map_err(|e| Error::Internal(format!("failed to rewrite compact commit row: {e}")))?;
            rewritten += 1;
        }
        for (table, key_columns) in [
            (
                self.tables.version_entries_by_message_fq(),
                "message_serial, version_serial",
            ),
            (
                self.tables.version_entries_by_delivery_fq(),
                "delivery_serial",
            ),
        ] {
            rewritten += self
                .materialize_projection(&table, key_columns, page_size)
                .await?;
        }
        let mut statement =
            Statement::new(format!("SELECT app_id, channel, commit_key FROM {commits}"));
        statement.set_page_size(page_size);
        let mut rows = self
            .session
            .query_iter(statement, ())
            .await
            .map_err(|e| Error::Internal(format!("failed to scan obsolete append chunks: {e}")))?
            .rows_stream::<(String, String, String)>()
            .map_err(|e| {
                Error::Internal(format!("failed to decode obsolete append chunks: {e}"))
            })?;
        // Writers are drained and every entry, receipt, latest state and
        // legacy projection has been materialized before removing any chunk.
        while let Some(row) = rows.next().await {
            let (app_id, channel, key) = row.map_err(|e| {
                Error::Internal(format!("failed to read obsolete append chunk: {e}"))
            })?;
            if key.starts_with("r:") || key.starts_with("c:") {
                self.session.query_unpaged(format!("DELETE FROM {commits} WHERE app_id = ? AND channel = ? AND commit_key = ?"), (app_id, channel, key)).await
                    .map_err(|e| Error::Internal(format!("failed to delete obsolete append chunk: {e}")))?;
            }
        }
        Ok(rewritten)
    }

    async fn materialize_projection(
        &self,
        table: &str,
        key_columns: &str,
        page_size: i32,
    ) -> Result<u64> {
        use ::scylla::value::CqlValue;
        let mut statement = Statement::new(format!(
            "SELECT app_id, channel, {key_columns}, payload_bytes, TTL(payload_bytes) FROM {table}"
        ));
        statement.set_page_size(page_size);
        let mut rows = self
            .session
            .query_iter(statement, ())
            .await
            .map_err(|e| Error::Internal(format!("failed to scan version projection: {e}")))?
            .rows_stream::<::scylla::value::Row>()
            .map_err(|e| Error::Internal(format!("failed to scan version projection: {e}")))?;
        let keys = key_columns.split(", ").collect::<Vec<_>>();
        let mut rewritten = 0;
        while let Some(row) = rows.next().await {
            let mut columns = row
                .map_err(|e| Error::Internal(format!("failed to read version projection: {e}")))?
                .columns;
            let ttl = columns.pop().flatten();
            let Some(Some(CqlValue::Blob(payload))) = columns.pop() else {
                continue;
            };
            if !is_compact(&payload) {
                continue;
            }
            let (Some(Some(CqlValue::Text(app_id))), Some(Some(CqlValue::Text(channel)))) =
                (columns.first().cloned(), columns.get(1).cloned())
            else {
                continue;
            };
            let record = self
                .materialize_payloads(&app_id, &channel, vec![payload])
                .await?
                .pop()
                .ok_or_else(|| Error::Internal("compact projection row is empty".to_string()))?;
            let predicate = std::iter::once("app_id = ?")
                .chain(std::iter::once("channel = ?"))
                .map(str::to_string)
                .chain(keys.iter().map(|column| format!("{column} = ?")))
                .collect::<Vec<_>>()
                .join(" AND ");
            let ttl = match ttl {
                Some(CqlValue::Int(seconds)) if seconds > 0 => format!("USING TTL {seconds} "),
                _ => String::new(),
            };
            let mut values = vec![Some(CqlValue::Blob(encode_full(&record)?))];
            values.extend(columns);
            self.session
                .query_unpaged(
                    format!("UPDATE {table} {ttl}SET payload_bytes = ? WHERE {predicate}"),
                    values,
                )
                .await
                .map_err(|e| {
                    Error::Internal(format!("failed to rewrite compact projection row: {e}"))
                })?;
            rewritten += 1;
        }
        Ok(rewritten)
    }
}
