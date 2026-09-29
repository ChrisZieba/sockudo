mod append_runs;
mod mutation;
mod payload_bytes;
mod store_impl;

use super::*;
use payload_bytes::StoredPayloadBytes;

// =================== SurrealVersionStore ===================

#[cfg(feature = "versioned-messages")]
#[derive(Debug, Clone, Serialize, Deserialize, SurrealValue)]
struct StoredVersionStreamRec {
    app_id: String,
    channel: String,
    stream_id: String,
    next_delivery_serial: i64,
    oldest_delivery_serial: Option<i64>,
    newest_delivery_serial: Option<i64>,
    #[serde(default)]
    open_stream_count: i64,
    updated_at_ms: i64,
}

#[cfg(feature = "versioned-messages")]
#[derive(Debug, Clone, Serialize, Deserialize, SurrealValue)]
struct StoredVersionMessageRec {
    app_id: String,
    channel: String,
    message_serial: String,
    latest_version_serial: String,
    latest_entry_key: String,
    history_serial: i64,
    #[serde(default)]
    latest_payload_bytes: StoredPayloadBytes,
    #[serde(default)]
    append_count: i64,
    #[serde(default)]
    is_open_stream: bool,
    updated_at_ms: i64,
    /// Run of the latest version when it is a compact append. Only a hint:
    /// older releases never clear it, so the run head is always re-checked.
    #[serde(default)]
    latest_append_run: Option<String>,
    #[serde(default)]
    latest_append_len: Option<i64>,
    /// Version the pointer describes; it is used only while this still
    /// names the latest version.
    #[serde(default)]
    latest_append_head: Option<String>,
    #[serde(default)]
    latest_append_pinned: Option<bool>,
    #[serde(default)]
    latest_append_generation: Option<String>,
}

#[cfg(feature = "versioned-messages")]
#[derive(Debug, Clone, Serialize, Deserialize, SurrealValue)]
struct StoredVersionEntryRec {
    app_id: String,
    channel: String,
    message_serial: String,
    version_serial: String,
    delivery_serial: i64,
    payload_bytes: StoredPayloadBytes,
    // Server-side append time (ms since epoch). Indexed for the purge worker
    // which deletes rows older than the retention cutoff in batched queries.
    created_at_ms: i64,
}

#[cfg(feature = "versioned-messages")]
#[derive(Debug, Clone, Serialize, Deserialize, SurrealValue)]
struct StoredVersionReceiptRec {
    operation_fingerprint: String,
    payload_bytes: StoredPayloadBytes,
    created_at_ms: i64,
}

#[cfg(feature = "versioned-messages")]
#[derive(Debug, Clone, Deserialize, SurrealValue)]
struct VersionLatestKeyRow {
    latest_entry_key: String,
}

#[cfg(feature = "versioned-messages")]
#[derive(Debug, Clone, Deserialize, SurrealValue)]
struct VersionPayloadRow {
    payload_bytes: StoredPayloadBytes,
}

#[cfg(feature = "versioned-messages")]
#[derive(Clone)]
struct VersionStoreTables {
    streams: String,
    messages: String,
    entries: String,
    receipts: String,
    runs: String,
    chunks: String,
    format: String,
}

#[cfg(feature = "versioned-messages")]
pub struct SurrealVersionStore {
    db: Surreal<Any>,
    tables: VersionStoreTables,
    append_cache: sockudo_core::version_store::append_storage::AppendSnapshotCache,
}

#[cfg(feature = "versioned-messages")]
pub async fn create_surreal_version_store(
    db_config: &SurrealDbSettings,
    table_prefix: &str,
) -> Result<Arc<dyn VersionStore + Send + Sync>> {
    validate_identifier(
        &format!("{table_prefix}_version_streams"),
        "version streams table",
    )?;
    validate_identifier(
        &format!("{table_prefix}_version_receipts"),
        "version receipts table",
    )?;
    validate_identifier(
        &format!("{table_prefix}_version_messages"),
        "version messages table",
    )?;
    validate_identifier(
        &format!("{table_prefix}_version_entries"),
        "version entries table",
    )?;
    validate_identifier(
        &format!("{table_prefix}_version_append_runs"),
        "version append runs table",
    )?;

    let db = connect(db_config.url.as_str()).await.map_err(|e| {
        Error::Internal(format!(
            "Failed to connect to SurrealDB for version store: {e}"
        ))
    })?;
    db.signin(Root {
        username: db_config.username.clone(),
        password: db_config.password.clone(),
    })
    .await
    .map_err(|e| {
        Error::Internal(format!(
            "Failed to authenticate SurrealDB version store: {e}"
        ))
    })?;
    db.use_ns(db_config.namespace.as_str())
        .use_db(db_config.database.as_str())
        .await
        .map_err(|e| {
            Error::Internal(format!(
                "Failed to select SurrealDB namespace for version store: {e}"
            ))
        })?;

    let tables = VersionStoreTables {
        streams: format!("{table_prefix}_version_streams"),
        messages: format!("{table_prefix}_version_messages"),
        entries: format!("{table_prefix}_version_entries"),
        receipts: format!("{table_prefix}_version_receipts"),
        runs: format!("{table_prefix}_version_append_runs"),
        chunks: format!("{table_prefix}_version_append_chunks"),
        format: format!("{table_prefix}_version_append_format"),
    };

    let query = format!(
        "DEFINE TABLE IF NOT EXISTS {} SCHEMALESS;\
         DEFINE TABLE IF NOT EXISTS {} SCHEMALESS;\
         DEFINE TABLE IF NOT EXISTS {} SCHEMALESS;\
         DEFINE TABLE IF NOT EXISTS {} SCHEMALESS;\
         DEFINE INDEX IF NOT EXISTS {}_app_idx ON TABLE {} FIELDS app_id;\
         DEFINE INDEX IF NOT EXISTS {}_message_idx ON TABLE {} FIELDS app_id, channel, message_serial;\
         DEFINE INDEX IF NOT EXISTS {}_history_idx ON TABLE {} FIELDS app_id, channel, history_serial;\
         DEFINE INDEX IF NOT EXISTS {}_updated_at_idx ON TABLE {} FIELDS updated_at_ms;\
         DEFINE INDEX IF NOT EXISTS {}_message_idx ON TABLE {} FIELDS app_id, channel, message_serial, version_serial;\
         DEFINE INDEX IF NOT EXISTS {}_delivery_idx ON TABLE {} FIELDS app_id, channel, delivery_serial;\
         DEFINE INDEX IF NOT EXISTS {}_created_at_idx ON TABLE {} FIELDS created_at_ms;\
         DEFINE TABLE IF NOT EXISTS {} SCHEMALESS;\
         DEFINE INDEX IF NOT EXISTS {}_updated_at_idx ON TABLE {} FIELDS updated_at_ms;",
        tables.streams,
        tables.messages,
        tables.entries,
        tables.receipts,
        tables.streams,
        tables.streams,
        tables.messages,
        tables.messages,
        tables.messages,
        tables.messages,
        tables.messages,
        tables.messages,
        tables.entries,
        tables.entries,
        tables.entries,
        tables.entries,
        tables.entries,
        tables.entries,
        tables.runs,
        tables.runs,
        tables.runs,
    );
    db.query(query).await.map_err(|e| {
        Error::Internal(format!(
            "Failed to initialize SurrealDB version store schema: {e}"
        ))
    })?;

    db.query(format!(
        "DEFINE TABLE IF NOT EXISTS {chunks} SCHEMALESS; DEFINE INDEX IF NOT EXISTS {chunks}_run_idx ON TABLE {chunks} FIELDS run_id; DEFINE TABLE IF NOT EXISTS {format} SCHEMALESS; UPSERT ONLY type::record('{format}', 'format') SET enabled = enabled ?? false, epoch = epoch ?? 0, fence = fence ?? 0;",
        chunks = tables.chunks,
        format = tables.format,
    ))
    .await
    .and_then(|response| response.check())
    .map_err(|e| Error::Internal(format!("failed to initialize append storage format: {e}")))?;

    let store = SurrealVersionStore {
        db,
        tables,
        append_cache: sockudo_core::version_store::append_storage::AppendSnapshotCache::default(),
    };
    store.ensure_mutation_function().await?;
    Ok(Arc::new(store))
}
