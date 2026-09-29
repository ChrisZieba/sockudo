#[cfg(test)]
mod tests;

use super::*;
use sockudo_core::version_store::append_storage::{
    AppendRunPlan, AppendRunRef, AppendRunSnapshots, StoredVersionPayload, encode_full,
    expand_payloads, is_compact, snapshot_from_bytes,
};
use sockudo_core::versioned_messages::{MessageSerial, VersionSerial};
use std::collections::{BTreeMap, HashMap};

// Keep PostgreSQL tail tuples below the TOAST threshold; the reserved page
// space permits HOT updates and prompt in-page reclamation.
const CHUNK_BYTES: usize = 1024;

/// Head of an append run as read inside a mutation transaction.
pub(super) struct AppendRunHead {
    pub(super) head: VersionSerial,
    pub(super) data_len: u64,
    pub(super) generation: Option<String>,
}

/// A page of stored entries with the run snapshots they need, read by one
/// statement so the page and its snapshots come from the same snapshot.
pub(super) struct MaterializedPage {
    pub(super) records: Vec<StoredVersionRecord>,
    /// The run each record is stored in, if compact.
    pub(super) runs: Vec<Option<AppendRunRef>>,
    /// Head of each loaded run, keyed like the snapshots.
    pub(super) heads: std::collections::HashMap<(MessageSerial, VersionSerial), AppendRunHead>,
}

impl MysqlVersionStore {
    /// One accumulated snapshot per append run; see
    /// `sockudo_core::version_store::append_storage`.
    pub(super) fn append_runs_table(&self) -> String {
        format!("{}_runs", self.tables.version_entries)
    }

    pub(super) fn append_chunks_table(&self) -> String {
        format!("{}_chunks", self.tables.version_entries)
    }

    fn append_format_table(&self) -> String {
        format!("{}_format", self.tables.version_entries)
    }

    pub(super) async fn ensure_append_runs(&self) -> Result<()> {
        let runs = self.append_runs_table();
        let sql = format!(
            "CREATE TABLE IF NOT EXISTS `{runs}` (app_id VARCHAR(255) {MYSQL_ASCII_IDENTIFIER_CHARSET} NOT NULL, channel VARCHAR(255) {MYSQL_ASCII_IDENTIFIER_CHARSET} NOT NULL, message_serial VARCHAR({MAX_VERSIONED_SERIAL_LENGTH}) NOT NULL, run_version_serial VARCHAR({MAX_VERSIONED_SERIAL_LENGTH}) NOT NULL, head_version_serial VARCHAR({MAX_VERSIONED_SERIAL_LENGTH}) NOT NULL, data_bytes LONGBLOB NOT NULL, data_len BIGINT NOT NULL, created_at_ms BIGINT NOT NULL, updated_at_ms BIGINT NOT NULL, PRIMARY KEY (app_id, channel, message_serial, run_version_serial), INDEX `{runs}_updated_at_idx` (updated_at_ms)) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci"
        );
        sqlx::query(sqlx::AssertSqlSafe(sql.as_str()))
            .execute(&self.pool)
            .await
            .map_err(|e| Error::Internal(format!("failed to initialize append run table: {e}")))?;
        self.ensure_column(&runs, "generation", "VARCHAR(36) NULL")
            .await?;
        let chunks = self.append_chunks_table();
        let marker = self.append_format_table();
        for sql in [
            format!(
                "CREATE TABLE IF NOT EXISTS `{chunks}` (app_id VARCHAR(255) {MYSQL_ASCII_IDENTIFIER_CHARSET} NOT NULL, channel VARCHAR(255) {MYSQL_ASCII_IDENTIFIER_CHARSET} NOT NULL, message_serial VARCHAR({MAX_VERSIONED_SERIAL_LENGTH}) NOT NULL, run_version_serial VARCHAR({MAX_VERSIONED_SERIAL_LENGTH}) NOT NULL, generation VARCHAR(36) NOT NULL, chunk_index BIGINT NOT NULL, data_bytes BLOB NOT NULL, PRIMARY KEY (app_id, channel, message_serial, run_version_serial, generation, chunk_index), FOREIGN KEY (app_id, channel, message_serial, run_version_serial) REFERENCES `{runs}` (app_id, channel, message_serial, run_version_serial) ON DELETE CASCADE) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci"
            ),
            format!(
                "CREATE TABLE IF NOT EXISTS `{marker}` (singleton BOOLEAN PRIMARY KEY DEFAULT TRUE, enabled BOOLEAN NOT NULL DEFAULT FALSE, CHECK (singleton = TRUE)) ENGINE=InnoDB"
            ),
            format!("INSERT IGNORE INTO `{marker}` (singleton, enabled) VALUES (TRUE, FALSE)"),
        ] {
            sqlx::query(sqlx::AssertSqlSafe(sql))
                .execute(&self.pool)
                .await
                .map_err(|e| Error::Internal(format!("failed to initialize append chunks: {e}")))?;
        }
        self.ensure_column(&self.tables.version_entries, "append_len", "BIGINT NULL")
            .await?;
        // Denormalized run id of compact entries, so readers join the
        // snapshot in the entry's own statement. A trailing nullable column
        // is added instantly by MySQL 8.
        self.ensure_column(
            &self.tables.version_entries,
            "append_run",
            &format!("VARCHAR({MAX_VERSIONED_SERIAL_LENGTH}) NULL"),
        )
        .await
    }

    /// A shared row lock fences every append commit against marker changes.
    pub(super) async fn append_storage_enabled(
        &self,
        tx: &mut sqlx::MySqlConnection,
    ) -> Result<bool> {
        let sql = format!(
            "SELECT enabled FROM `{}` WHERE singleton = TRUE FOR SHARE",
            self.append_format_table()
        );
        sqlx::query_scalar(sqlx::AssertSqlSafe(sql))
            .fetch_one(tx)
            .await
            .map_err(|e| Error::Internal(format!("failed to lock append storage marker: {e}")))
    }

    pub(super) async fn update_append_storage_marker(&self, enabled: bool) -> Result<()> {
        let mut tx = self.pool.begin().await.map_err(|e| {
            Error::Internal(format!("failed to begin append format maintenance: {e}"))
        })?;
        let sql = format!(
            "SELECT enabled FROM `{}` WHERE singleton = TRUE FOR UPDATE",
            self.append_format_table()
        );
        let current: bool = sqlx::query_scalar(sqlx::AssertSqlSafe(sql))
            .fetch_one(&mut *tx)
            .await
            .map_err(|e| {
                Error::Internal(format!("failed to fence append format maintenance: {e}"))
            })?;
        if enabled && !current {
            self.seed_existing_latest(&mut tx).await?;
        }
        let mut query = sqlx::QueryBuilder::<sqlx::MySql>::new(format!(
            "UPDATE `{}` SET enabled = ",
            self.append_format_table()
        ));
        query.push_bind(enabled).push(" WHERE singleton = TRUE");
        query
            .build()
            .execute(&mut *tx)
            .await
            .map_err(|e| Error::Internal(format!("failed to update append storage marker: {e}")))?;
        tx.commit().await.map_err(|e| {
            Error::Internal(format!("failed to commit append format maintenance: {e}"))
        })
    }

    /// Seed all existing latest data before activation. The exclusive marker
    /// lock fences upgraded writers; the maintenance command drains old nodes.
    async fn seed_existing_latest(&self, tx: &mut sqlx::MySqlConnection) -> Result<()> {
        let mut after: Option<(String, String, String)> = None;
        loop {
            let mut keys = sqlx::QueryBuilder::<sqlx::MySql>::new(format!(
                "SELECT app_id, channel, message_serial FROM `{}`",
                self.tables.version_messages
            ));
            if let Some((app, channel, message)) = &after {
                keys.push(" WHERE (app_id, channel, message_serial) > (")
                    .push_bind(app)
                    .push(", ")
                    .push_bind(channel)
                    .push(", ")
                    .push_bind(message)
                    .push(")");
            }
            keys.push(" ORDER BY app_id, channel, message_serial LIMIT 128");
            let rows = keys
                .build()
                .fetch_all(&mut *tx)
                .await
                .map_err(|e| Error::Internal(format!("failed to scan append seeds: {e}")))?;
            if rows.is_empty() {
                break;
            }
            for row in &rows {
                let app: String = row.get("app_id");
                let channel: String = row.get("channel");
                let message: String = row.get("message_serial");
                let mut page = sqlx::QueryBuilder::<sqlx::MySql>::new(format!(
                    "SELECT app_id, channel, message_serial, append_run, append_len, payload_bytes, CAST(1 AS SIGNED) AS ord FROM `{}` WHERE app_id = ",
                    self.tables.version_entries
                ));
                page.push_bind(&app)
                    .push(" AND channel = ")
                    .push_bind(&channel)
                    .push(" AND message_serial = ")
                    .push_bind(&message)
                    .push(" ORDER BY version_serial DESC LIMIT 1");
                let sql = self.with_append_runs(page.sql().as_ref());
                use sqlx::Execute;
                let arguments = page.build().take_arguments().map_err(|e| {
                    Error::Internal(format!("failed to encode append seed query: {e}"))
                })?;
                let records =
                    sqlx::query_with(sqlx::AssertSqlSafe(sql), arguments.unwrap_or_default())
                        .fetch_all(&mut *tx)
                        .await
                        .map_err(|e| Error::Internal(format!("failed to read append seed: {e}")))?;
                let mut materialized = Self::materialize_page(records)?;
                // Existing format-2 heads already have a bounded tail.
                if materialized
                    .runs
                    .last()
                    .and_then(Option::as_ref)
                    .is_some_and(|run| run.generation.is_some())
                {
                    continue;
                }
                if let Some(record) = materialized.records.pop() {
                    self.seed_full_record(tx, &record).await?;
                }
            }
            let last = rows
                .last()
                .ok_or_else(|| Error::Internal("append seed scan is empty".into()))?;
            after = Some((
                last.get("app_id"),
                last.get("channel"),
                last.get("message_serial"),
            ));
            if rows.len() < 128 {
                break;
            }
        }
        Ok(())
    }

    /// Non-append writes may persist their new data once as a future append
    /// seed. The entry itself stays a legacy-readable full-state record.
    pub(super) async fn seed_full_record(
        &self,
        tx: &mut sqlx::MySqlConnection,
        record: &StoredVersionRecord,
    ) -> Result<()> {
        let plan = AppendRunPlan::for_seed_record(record);
        let Some(run) = plan.run() else {
            return Ok(());
        };
        self.write_append_run(tx, record, &plan, sockudo_core::history::now_ms())
            .await?;
        let payload = encode_full(record)?;
        let mut query = sqlx::QueryBuilder::<sqlx::MySql>::new(format!(
            "UPDATE `{}` SET payload_bytes = ",
            self.tables.version_entries
        ));
        query
            .push_bind(payload.as_slice())
            .push(", payload_size_bytes = ")
            .push_bind(payload.len() as i64)
            .push(", append_run = ")
            .push_bind(run.run.as_str())
            // A full record needs only seed metadata on reads, no chunk bytes.
            .push(", append_len = 0 WHERE app_id = ")
            .push_bind(record.app_id.as_str())
            .push(" AND channel = ")
            .push_bind(record.channel.as_str())
            .push(" AND message_serial = ")
            .push_bind(record.message_serial().as_str())
            .push(" AND version_serial = ")
            .push_bind(record.version_serial().as_str());
        query
            .build()
            .execute(tx)
            .await
            .map_err(|e| Error::Internal(format!("failed to store append seed pointer: {e}")))?;
        Ok(())
    }

    /// Wrap `page_sql`, which must select `app_id, channel, message_serial,
    /// append_run, payload_bytes, ord` from version entries, so the same
    /// statement returns every snapshot the page needs exactly once.
    pub(super) fn with_append_runs(&self, page_sql: &str) -> String {
        format!(
            "WITH page AS ({page_sql}), needed AS (SELECT app_id, channel, message_serial, append_run, MAX(append_len) AS prefix_len FROM page WHERE append_run IS NOT NULL GROUP BY app_id, channel, message_serial, append_run), selected_runs AS (SELECT r.*, n.prefix_len FROM `{runs}` r JOIN needed n ON r.app_id = n.app_id AND r.channel = n.channel AND r.message_serial = n.message_serial AND r.run_version_serial = n.append_run) \
             SELECT 0 AS part, CAST(ord AS SIGNED) AS ord, message_serial, append_run AS run, payload_bytes AS bytes, NULL AS head, NULL AS len, NULL AS generation FROM page \
             UNION ALL \
             SELECT 1, CAST(0 AS SIGNED), r.message_serial, r.run_version_serial, r.data_bytes, r.head_version_serial, r.data_len, r.generation FROM selected_runs r \
             UNION ALL \
             SELECT 2, c.chunk_index, c.message_serial, c.run_version_serial, c.data_bytes, NULL, NULL, c.generation FROM `{chunks}` c JOIN selected_runs r ON c.app_id = r.app_id AND c.channel = r.channel AND c.message_serial = r.message_serial AND c.run_version_serial = r.run_version_serial AND c.generation = r.generation WHERE c.chunk_index * {chunk_bytes} < COALESCE(r.prefix_len, r.data_len)",
            runs = self.append_runs_table(),
            chunks = self.append_chunks_table(),
            chunk_bytes = CHUNK_BYTES
        )
    }

    /// Decode the rows of a [`Self::with_append_runs`] statement in page order.
    pub(super) fn materialize_page(rows: Vec<sqlx::mysql::MySqlRow>) -> Result<MaterializedPage> {
        let mut entries = Vec::new();
        let mut snapshots = AppendRunSnapshots::new();
        let mut heads = HashMap::new();
        let mut chunks: HashMap<_, BTreeMap<i64, Vec<u8>>> = HashMap::new();
        for row in rows {
            let bytes: Vec<u8> = row.get("bytes");
            let part = row.get::<i64, _>("part");
            if part == 0 {
                entries.push((
                    row.get::<i64, _>("ord"),
                    bytes,
                    row.get::<Option<String>, _>("run"),
                ));
                continue;
            }
            let key = (
                MessageSerial::new(row.get::<String, _>("message_serial"))?,
                VersionSerial::new(row.get::<String, _>("run"))?,
            );
            if part == 2 {
                chunks.entry(key).or_default().insert(row.get("ord"), bytes);
                continue;
            }
            let generation: Option<String> = row.get("generation");
            if generation.is_none() {
                snapshots.insert(key.clone(), snapshot_from_bytes(bytes)?);
            }
            heads.insert(
                key,
                AppendRunHead {
                    head: VersionSerial::new(row.get::<String, _>("head"))?,
                    data_len: u64::try_from(row.get::<i64, _>("len"))
                        .map_err(|_| Error::Internal("invalid append run length".to_string()))?,
                    generation,
                },
            );
        }
        entries.sort_by_key(|(ord, _, _)| *ord);
        let payloads = entries
            .iter()
            .map(|(_, bytes, _)| StoredVersionPayload::decode(bytes))
            .collect::<Result<Vec<_>>>()?;
        let mut prefixes: HashMap<_, u64> = HashMap::new();
        for payload in &payloads {
            if let Some(run) = payload.run() {
                let key = (payload.record().message_serial().clone(), run.run.clone());
                let head = heads
                    .get(&key)
                    .ok_or_else(|| Error::Internal("append run is missing".into()))?;
                if head.generation != run.generation || run.data_len > head.data_len {
                    return Err(Error::Internal(
                        "append run generation or length mismatch".into(),
                    ));
                }
                if run.generation.is_some() {
                    prefixes
                        .entry(key)
                        .and_modify(|len| *len = (*len).max(run.data_len))
                        .or_insert(run.data_len);
                }
            }
        }
        for (key, len) in prefixes {
            let len = usize::try_from(len)
                .map_err(|_| Error::Internal("append prefix length overflow".into()))?;
            let mut bytes = Vec::new();
            for (expected, (index, chunk)) in chunks
                .remove(&key)
                .unwrap_or_default()
                .into_iter()
                .enumerate()
            {
                if index != expected as i64 || chunk.len() > CHUNK_BYTES || chunk.is_empty() {
                    return Err(Error::Internal("append chunk sequence is invalid".into()));
                }
                let needed = len.saturating_sub(bytes.len());
                if chunk.len() < needed.min(CHUNK_BYTES) {
                    return Err(Error::Internal("append chunk is truncated".into()));
                }
                bytes.extend_from_slice(&chunk[..chunk.len().min(needed)]);
            }
            if bytes.len() != len {
                return Err(Error::Internal("append chunks are missing".into()));
            }
            snapshots.insert(key, snapshot_from_bytes(bytes)?);
        }
        let runs = payloads
            .iter()
            .zip(&entries)
            .map(|(payload, (_, _, seed))| {
                if let Some(run) = payload.run() {
                    return Some(run.clone());
                }
                let serial = VersionSerial::new(seed.as_ref()?.clone()).ok()?;
                let head =
                    heads.get(&(payload.record().message_serial().clone(), serial.clone()))?;
                (head.generation.is_some() && &head.head == payload.record().version_serial()).then(
                    || AppendRunRef {
                        run: serial,
                        data_len: head.data_len,
                        generation: head.generation.clone(),
                    },
                )
            })
            .collect();
        Ok(MaterializedPage {
            records: expand_payloads(payloads, &snapshots)?,
            runs,
            heads,
        })
    }

    /// Read supplied receipts and their exact chunk prefixes in one statement.
    pub(super) async fn materialize_payloads<'c, E>(
        &self,
        executor: E,
        app_id: &str,
        channel: &str,
        payloads: Vec<Vec<u8>>,
    ) -> Result<Vec<StoredVersionRecord>>
    where
        E: sqlx::Executor<'c, Database = sqlx::MySql>,
    {
        if payloads.is_empty() {
            return Ok(Vec::new());
        }
        let decoded = payloads
            .iter()
            .map(|bytes| StoredVersionPayload::decode(bytes))
            .collect::<Result<Vec<_>>>()?;
        let mut page = sqlx::QueryBuilder::<sqlx::MySql>::new("");
        for (index, (payload, bytes)) in decoded.iter().zip(&payloads).enumerate() {
            if index != 0 {
                page.push(" UNION ALL ");
            }
            page.push("SELECT ")
                .push_bind(app_id)
                .push(" AS app_id, ")
                .push_bind(channel)
                .push(" AS channel, ")
                .push_bind(payload.record().message_serial().as_str())
                .push(" AS message_serial, ")
                .push_bind(payload.run().map(|run| run.run.as_str()))
                .push(" AS append_run, ")
                .push_bind(payload.run().map(|run| run.data_len as i64))
                .push(" AS append_len, ")
                .push_bind(bytes.as_slice())
                .push(" AS payload_bytes, ")
                .push_bind(index as i64)
                .push(" AS ord");
        }
        let sql = self.with_append_runs(page.sql().as_ref());
        use sqlx::Execute;
        let arguments = page
            .build()
            .take_arguments()
            .map_err(|e| Error::Internal(format!("failed to encode append receipt query: {e}")))?;
        let rows = sqlx::query_with(sqlx::AssertSqlSafe(sql), arguments.unwrap_or_default())
            .fetch_all(executor)
            .await
            .map_err(|e| Error::Internal(format!("failed to read append receipt chunks: {e}")))?;
        Ok(Self::materialize_page(rows)?.records)
    }

    /// Rewrite only the bounded tail chunk and insert new fragment chunks.
    /// Run metadata and all chunks commit atomically with the version entry.
    pub(super) async fn write_append_run(
        &self,
        tx: &mut sqlx::MySqlConnection,
        record: &StoredVersionRecord,
        plan: &AppendRunPlan,
        now_ms: i64,
    ) -> Result<()> {
        let Some(run) = plan.run() else {
            return Ok(());
        };
        let generation = run
            .generation
            .as_deref()
            .ok_or_else(|| Error::Internal("new append run has no generation".into()))?;
        let mut query = sqlx::QueryBuilder::<sqlx::MySql>::new("");
        match plan {
            AppendRunPlan::Full => return Ok(()),
            AppendRunPlan::Start { .. } => {
                query.push(format!("INSERT INTO `{}` (app_id, channel, message_serial, run_version_serial, head_version_serial, data_bytes, data_len, created_at_ms, updated_at_ms, generation) VALUES (", self.append_runs_table()));
                query.push_bind(record.app_id.as_str()).push(", ")
                    .push_bind(record.channel.as_str()).push(", ")
                    .push_bind(record.message_serial().as_str()).push(", ")
                    .push_bind(run.run.as_str()).push(", ")
                    .push_bind(record.version_serial().as_str()).push(", ")
                    .push_bind(&[] as &[u8]).push(", ")
                    .push_bind(run.data_len as i64).push(", ")
                    .push_bind(now_ms).push(", ").push_bind(now_ms).push(", ")
                    .push_bind(generation).push(") ON DUPLICATE KEY UPDATE head_version_serial = VALUES(head_version_serial), data_bytes = VALUES(data_bytes), data_len = VALUES(data_len), created_at_ms = VALUES(created_at_ms), updated_at_ms = VALUES(updated_at_ms), generation = VALUES(generation)");
            }
            AppendRunPlan::Extend {
                expected_head,
                expected_len,
                ..
            } => {
                query.push(format!(
                    "UPDATE `{}` SET data_len = ",
                    self.append_runs_table()
                ));
                query
                    .push_bind(run.data_len as i64)
                    .push(", head_version_serial = ")
                    .push_bind(record.version_serial().as_str())
                    .push(", updated_at_ms = ")
                    .push_bind(now_ms)
                    .push(" WHERE app_id = ")
                    .push_bind(record.app_id.as_str())
                    .push(" AND channel = ")
                    .push_bind(record.channel.as_str())
                    .push(" AND message_serial = ")
                    .push_bind(record.message_serial().as_str())
                    .push(" AND run_version_serial = ")
                    .push_bind(run.run.as_str())
                    .push(" AND generation = ")
                    .push_bind(generation)
                    .push(" AND head_version_serial = ")
                    .push_bind(expected_head.as_str())
                    .push(" AND data_len = ")
                    .push_bind(*expected_len as i64);
            }
        }
        let result =
            query.build().execute(&mut *tx).await.map_err(|e| {
                Error::Internal(format!("failed to write append run metadata: {e}"))
            })?;
        let expected = if matches!(plan, AppendRunPlan::Start { .. }) {
            1..=2
        } else {
            1..=1
        };
        if !expected.contains(&result.rows_affected()) {
            return Err(Error::Internal("append run did not match its head".into()));
        }
        if matches!(plan, AppendRunPlan::Start { .. }) {
            // A replaced leftover run must not leave chunks in the new generation.
            let mut delete = sqlx::QueryBuilder::<sqlx::MySql>::new(format!(
                "DELETE FROM `{}` WHERE app_id = ",
                self.append_chunks_table()
            ));
            delete
                .push_bind(record.app_id.as_str())
                .push(" AND channel = ")
                .push_bind(record.channel.as_str())
                .push(" AND message_serial = ")
                .push_bind(record.message_serial().as_str())
                .push(" AND run_version_serial = ")
                .push_bind(run.run.as_str());
            delete.build().execute(&mut *tx).await.map_err(|e| {
                Error::Internal(format!("failed to remove leftover append chunks: {e}"))
            })?;
        }
        let writes = plan.chunk_writes_sized(record, CHUNK_BYTES)?;
        // Bound SQL bind counts even for large imports or append fragments.
        for write_batch in writes.chunks(256) {
            let mut chunks = sqlx::QueryBuilder::<sqlx::MySql>::new(format!(
                "INSERT INTO `{}` (app_id, channel, message_serial, run_version_serial, generation, chunk_index, data_bytes) ",
                self.append_chunks_table()
            ));
            chunks.push_values(write_batch, |mut row, write| {
                row.push_bind(record.app_id.as_str())
                    .push_bind(record.channel.as_str())
                    .push_bind(record.message_serial().as_str())
                    .push_bind(run.run.as_str())
                    .push_bind(generation)
                    .push_bind(write.index as i64)
                    .push_bind(write.bytes.as_slice());
            });
            chunks.push(" ON DUPLICATE KEY UPDATE data_bytes = VALUES(data_bytes)");
            chunks
                .build()
                .execute(&mut *tx)
                .await
                .map_err(|e| Error::Internal(format!("failed to write append chunks: {e}")))?;
        }
        Ok(())
    }

    /// Delete runs no retained entry of their message still falls inside.
    pub(super) async fn purge_append_runs(&self, before_ms: i64, limit: i64) -> Result<u64> {
        let sql = format!(
            "DELETE FROM `{runs}` WHERE updated_at_ms < ? AND NOT EXISTS (SELECT 1 FROM `{entries}` e WHERE e.app_id = `{runs}`.app_id AND e.channel = `{runs}`.channel AND e.message_serial = `{runs}`.message_serial AND e.version_serial >= `{runs}`.run_version_serial AND e.version_serial <= `{runs}`.head_version_serial) ORDER BY updated_at_ms ASC LIMIT ?",
            runs = self.append_runs_table(),
            entries = self.tables.version_entries
        );
        sqlx::query(sqlx::AssertSqlSafe(sql.as_str()))
            .bind(before_ms)
            .bind(limit)
            .execute(&self.pool)
            .await
            .map(|result| result.rows_affected())
            .map_err(|e| Error::Internal(format!("failed to purge append runs: {e}")))
    }

    /// Rewrite compact entries as self-contained records, walking the entry
    /// primary key in bounded pages. Entries double as receipts here.
    pub(super) async fn materialize_compact_entries(&self, batch_size: usize) -> Result<u64> {
        let limit = i64::try_from(batch_size.max(1))
            .map_err(|_| Error::InvalidMessageFormat("batch size is too large".to_string()))?;
        let entries = &self.tables.version_entries;
        let mut after: Option<(String, String, String, String)> = None;
        let mut rewritten = 0;
        loop {
            let rows = match &after {
                None => {
                    let sql = format!(
                        "SELECT app_id, channel, message_serial, version_serial, payload_bytes FROM `{entries}` ORDER BY app_id, channel, message_serial, version_serial LIMIT ?"
                    );
                    sqlx::query(sqlx::AssertSqlSafe(sql))
                        .bind(limit)
                        .fetch_all(&self.pool)
                        .await
                }
                Some((app_id, channel, message_serial, version_serial)) => {
                    let sql = format!(
                        "SELECT app_id, channel, message_serial, version_serial, payload_bytes FROM `{entries}` WHERE (app_id, channel, message_serial, version_serial) > (?, ?, ?, ?) ORDER BY app_id, channel, message_serial, version_serial LIMIT ?"
                    );
                    sqlx::query(sqlx::AssertSqlSafe(sql))
                        .bind(app_id)
                        .bind(channel)
                        .bind(message_serial)
                        .bind(version_serial)
                        .bind(limit)
                        .fetch_all(&self.pool)
                        .await
                }
            }
            .map_err(|e| Error::Internal(format!("failed to scan version entries: {e}")))?;
            let Some(last) = rows.last() else {
                break;
            };
            after = Some((
                last.get("app_id"),
                last.get("channel"),
                last.get("message_serial"),
                last.get("version_serial"),
            ));
            for row in &rows {
                let payload: Vec<u8> = row.get("payload_bytes");
                if !is_compact(&payload) {
                    continue;
                }
                let app_id: String = row.get("app_id");
                let channel: String = row.get("channel");
                let record = self
                    .materialize_payloads(&self.pool, &app_id, &channel, vec![payload.clone()])
                    .await?
                    .pop()
                    .ok_or_else(|| Error::Internal("compact entry is empty".to_string()))?;
                let full = encode_full(&record)?;
                let sql = format!(
                    "UPDATE `{entries}` SET payload_bytes = ?, payload_size_bytes = ?, append_run = NULL WHERE app_id = ? AND channel = ? AND message_serial = ? AND version_serial = ? AND payload_bytes = ?"
                );
                rewritten += sqlx::query(sqlx::AssertSqlSafe(sql))
                    .bind(full.as_slice())
                    .bind(full.len() as i64)
                    .bind(&app_id)
                    .bind(&channel)
                    .bind(row.get::<String, _>("message_serial"))
                    .bind(row.get::<String, _>("version_serial"))
                    .bind(payload.as_slice())
                    .execute(&self.pool)
                    .await
                    .map_err(|e| Error::Internal(format!("failed to rewrite compact entry: {e}")))?
                    .rows_affected();
            }
            if (rows.len() as i64) < limit {
                break;
            }
        }
        Ok(rewritten)
    }
}
