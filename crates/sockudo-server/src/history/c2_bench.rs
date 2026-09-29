//! C2 diagnostic harness for durable version stores. Diagnostic-only and
//! identical in the baseline and candidate builds: it uses the production
//! factory and public `VersionStore` API, and measures retained storage
//! through generic catalog/table scans so new companion tables are counted
//! without the harness knowing their names.
//!
//! Requires the isolated Compose project in
//! `audits/performance-2026-09-05/c2/compose.yaml`. Select work with
//! `C2_BACKENDS`, `C2_APPENDS`, `C2_FRAGMENTS` (comma-separated).
use super::*;
use sockudo_core::message_envelope::{MessageContent, MessageEnvelope};
use sockudo_core::options::{DatabaseConnection, VersionStoreDriver};
use sockudo_core::version_store::*;
use sockudo_core::versioned_messages::*;
use sockudo_protocol::messages::MessageData;
use std::hint::black_box;
use std::time::Instant;

const APP: &str = "c2";
const CHANNEL: &str = "ai:room";

struct Lcg(u64);
impl Lcg {
    fn next(&mut self) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        self.0 >> 33
    }
}

fn fragment(rng: &mut Lcg, bytes: usize) -> String {
    let mut out = String::with_capacity(bytes);
    while out.len() < bytes {
        let left = bytes - out.len();
        match rng.next() % 16 {
            0 if left >= 4 => out.push('\u{1F642}'),
            1 | 2 if left >= 2 => out.push('\u{e9}'),
            n => out.push((b'a' + (n as u8 % 26)) as char),
        }
    }
    out
}

fn version(n: u64) -> VersionMetadata {
    VersionMetadata {
        serial: VersionSerial::new(format!("ver:{n:020}")).unwrap(),
        client_id: Some("agent".into()),
        timestamp_ms: 1_700_000_000_000 + n as i64,
        description: None,
        metadata: None,
    }
}

fn create_record() -> StoredVersionRecord {
    let envelope = MessageEnvelope {
        message_id: Some("client-message-1".into()),
        name: Some("ai.response".into()),
        data: Some(MessageContent::Text(String::new())),
        publisher_client_id: Some("agent".into()),
        published_at_ms: Some(1_700_000_000_000),
        ..MessageEnvelope::default()
    };
    StoredVersionRecord {
        app_id: APP.into(),
        channel: CHANNEL.into(),
        original_client_id: Some("agent".into()),
        envelope: Some(envelope),
        message: VersionedMessage::new_create(
            MessageSerial::new("msg:1").unwrap(),
            version(0),
            1,
            0,
            Some("ai.response".into()),
            Some(MessageData::String(String::new())),
            None,
        ),
    }
}

fn digest(state: &mut u64, record: &StoredVersionRecord) {
    let mut record = record.clone();
    if let Some(envelope) = record.envelope.as_mut() {
        envelope.stream_id = None;
    }
    for byte in sonic_rs::to_vec(&record).unwrap() {
        *state ^= u64::from(byte);
        *state = state.wrapping_mul(0x100000001b3);
    }
}

fn report(label: &str, name: &str, mut samples: Vec<u64>) {
    samples.sort_unstable();
    let pct = |p: usize| samples[(samples.len() - 1) * p / 100];
    println!(
        "{label},latency,{name},{},{},{},{},{}",
        samples.len(),
        pct(50),
        pct(95),
        pct(99),
        samples[samples.len() - 1]
    );
}

// Accepted benchmark limits: transport encoding and fixed transaction metadata
// are counted explicitly; accumulated state must still fit one bounded tail.
fn wire_budget(backend: &str, fragment: usize, index: usize) -> u64 {
    let fragment = fragment as u64;
    match backend {
        "postgres" | "mysql" => fragment + 1024 + if index < 2 { 6 * 1024 } else { 3 * 1024 },
        "dynamodb" => 4 * fragment.div_ceil(3) + 4 * 4096u64.div_ceil(3) + 10 * 1024,
        "scylladb" => fragment + 4096 + 8 * 1024,
        "surrealdb" => fragment + 4096 + 5 * 1024,
        _ => unreachable!(),
    }
}

fn configs(
    backend: &str,
    prefix: &str,
) -> (VersionedMessagesConfig, HistoryConfig, DatabaseConfig) {
    let mut versioned = VersionedMessagesConfig {
        enabled: true,
        retention_window_seconds: 3600,
        ..VersionedMessagesConfig::default()
    };
    let mut history = HistoryConfig::default();
    let mut db = DatabaseConfig::default();
    match backend {
        "postgres" => {
            versioned.driver = VersionStoreDriver::Postgres;
            history.postgres.table_prefix = prefix.into();
            db.postgres = DatabaseConnection {
                host: "127.0.0.1".into(),
                port: 25471,
                username: "c2".into(),
                password: "c2-local-only".into(),
                database: "c2".into(),
                ..Default::default()
            };
        }
        "mysql" => {
            versioned.driver = VersionStoreDriver::Mysql;
            history.mysql.table_prefix = prefix.into();
            db.mysql = DatabaseConnection {
                host: "127.0.0.1".into(),
                port: 25472,
                username: "root".into(),
                password: "c2-local-only".into(),
                database: "c2".into(),
                ..Default::default()
            };
        }
        "dynamodb" => {
            versioned.driver = VersionStoreDriver::DynamoDb;
            history.dynamodb.table_prefix = prefix.into();
            db.dynamodb.endpoint_url = Some("http://127.0.0.1:25473".into());
            db.dynamodb.aws_access_key_id = Some("c2".into());
            db.dynamodb.aws_secret_access_key = Some("c2-local-only".into());
        }
        "scylladb" => {
            versioned.driver = VersionStoreDriver::ScyllaDb;
            history.scylladb.table_prefix = prefix.into();
            db.scylladb.nodes = vec!["127.0.0.1:25474".into()];
            db.scylladb.keyspace = "c2".into();
            db.scylladb.replication_factor = 1;
        }
        "surrealdb" => {
            versioned.driver = VersionStoreDriver::SurrealDb;
            history.surrealdb.table_prefix = prefix.into();
            db.surrealdb.url = "ws://127.0.0.1:25475".into();
            db.surrealdb.namespace = "c2".into();
            db.surrealdb.database = "c2".into();
            db.surrealdb.password = "c2-local-only".into();
        }
        other => panic!("unknown backend {other}"),
    }
    (versioned, history, db)
}

async fn open(backend: &str, prefix: &str) -> Arc<dyn VersionStore + Send + Sync> {
    open_at(backend, prefix, None).await
}

async fn open_at(
    backend: &str,
    prefix: &str,
    port: Option<u16>,
) -> Arc<dyn VersionStore + Send + Sync> {
    let (versioned, history, mut db) = configs(backend, prefix);
    if let Some(port) = port {
        match backend {
            "postgres" => db.postgres.port = port,
            "mysql" => db.mysql.port = port,
            "dynamodb" => db.dynamodb.endpoint_url = Some(format!("http://127.0.0.1:{port}")),
            "scylladb" => db.scylladb.nodes = vec![format!("127.0.0.1:{port}")],
            "surrealdb" => db.surrealdb.url = format!("ws://127.0.0.1:{port}"),
            _ => unreachable!(),
        }
    }
    create_version_store(&versioned, &history, &db, &DatabasePooling::default())
        .await
        .unwrap()
}

async fn full_digest(store: &dyn VersionStore) -> (u64, usize, u64, u64) {
    let serial = MessageSerial::new("msg:1").unwrap();
    let mut cursor = None;
    let mut count = 0;
    let mut bytes = 0;
    let mut versions = 0xcbf29ce484222325u64;
    loop {
        let page = store
            .get_versions(VersionStoreReadRequest {
                app_id: APP.into(),
                channel: CHANNEL.into(),
                message_serial: serial.clone(),
                direction: VersionStoreDirection::OldestFirst,
                limit: 100,
                cursor,
            })
            .await
            .unwrap();
        for record in &page.items {
            count += 1;
            bytes += record.data_bytes().unwrap();
            digest(&mut versions, record);
        }
        cursor = page.next_cursor;
        if cursor.is_none() {
            break;
        }
    }
    let mut after = 0;
    let mut replay = 0xcbf29ce484222325u64;
    loop {
        let items = store
            .replay_after(VersionReplayRequest {
                app_id: APP.into(),
                channel: CHANNEL.into(),
                after_delivery_serial: after,
                limit: 100,
            })
            .await
            .unwrap();
        let Some(last) = items.last() else {
            break;
        };
        after = last.delivery_serial();
        for record in &items {
            digest(&mut replay, record);
        }
    }
    (count, bytes, versions, replay)
}

async fn run_case(backend: &str, appends: u64, fragment_bytes: usize, rep: u32) {
    assert!(
        !(std::env::var_os("C2_WIRE_BYTES").is_some()
            && std::env::var_os("C2_WRITE_METRICS").is_some()),
        "wire and database-volume instrumentation require separate diagnostic runs"
    );
    let prefix = format!("c2b{}", &uuid::Uuid::new_v4().simple().to_string()[..12]);
    let label = format!("{backend},{appends},{fragment_bytes},{rep}");
    // Opt-in volume-only runs: a proxy adds scheduling work and therefore must
    // never be used to claim an uninstrumented latency result.
    let wire = if std::env::var_os("C2_WIRE_BYTES").is_some() {
        let port = match backend {
            "postgres" => 25471,
            "mysql" => 25472,
            "dynamodb" => 25473,
            "scylladb" => 25474,
            "surrealdb" => 25475,
            _ => unreachable!(),
        };
        Some(c2_wire_meter::WireMeter::start(port).await.unwrap())
    } else {
        None
    };
    let store = open_at(
        backend,
        &prefix,
        wire.as_ref().map(|meter| meter.local_port()),
    )
    .await;
    if std::env::var_os("C2_CHUNKED").is_some() {
        store.set_append_storage_enabled(true).await.unwrap();
    }
    let serial = MessageSerial::new("msg:1").unwrap();
    let mut rng = Lcg(0xC2 ^ appends ^ ((fragment_bytes as u64) << 20));
    let fragments: Vec<String> = (0..appends)
        .map(|_| fragment(&mut rng, fragment_bytes))
        .collect();

    let VersionCreateResult::Applied { record, .. } = store
        .commit_create(VersionCreateRequest {
            record: create_record(),
            limits: VersionCreateLimits::default(),
        })
        .await
        .unwrap()
    else {
        panic!("create was not applied");
    };
    let mut expected = VersionPrecondition::from_record(&record);
    let mut write_ns = Vec::with_capacity(appends as usize);
    let mut wire_samples = Vec::with_capacity(appends as usize);
    let enforce_wire = std::env::var_os("C2_WIRE_BUDGETS").is_some();
    assert!(
        !enforce_wire || wire.is_some(),
        "wire budgets require C2_WIRE_BYTES"
    );
    let counters_before = write_counters(backend).await;
    let wire_before = wire
        .as_ref()
        .map(|meter| (meter.sent_bytes(), meter.received_bytes()));
    let started_all = Instant::now();
    for (index, data_fragment) in fragments.iter().enumerate() {
        let wire_start = wire.as_ref().map(|meter| meter.snapshot());
        let started = Instant::now();
        let outcome = store
            .compare_and_apply(VersionMutationRequest {
                app_id: APP.into(),
                channel: CHANNEL.into(),
                message_serial: serial.clone(),
                expected: expected.clone(),
                version: version(index as u64 + 1),
                mutation: VersionMutation::Append(MessageAppend {
                    data_fragment: data_fragment.clone(),
                    extras: None,
                }),
                idempotency: None,
                limits: VersionMutationLimits::default(),
            })
            .await;
        write_ns.push(started.elapsed().as_nanos() as u64);
        if let (Some(meter), Some(before)) = (&wire, wire_start) {
            let delta = meter.snapshot().since(before);
            assert_eq!(delta.failed_connections, 0, "wire meter forwarding failed");
            assert!(delta.client_bytes > 0, "driver bypassed wire meter");
            if enforce_wire {
                let limit = wire_budget(backend, fragment_bytes, index);
                assert!(
                    delta.client_bytes <= limit,
                    "{backend} append {} sent {} bytes, exceeding accepted wire limit {limit}",
                    index + 1,
                    delta.client_bytes
                );
            }
            wire_samples.push(delta.client_bytes);
        }
        match outcome {
            Ok(VersionMutationResult::Applied { record, .. }) => {
                expected = VersionPrecondition::from_record(&record);
            }
            other => {
                println!(
                    "{label},write_failed,append,{},{}",
                    index + 1,
                    match other {
                        Ok(result) => format!("{result:?}").chars().take(160).collect::<String>(),
                        Err(error) => error.to_string().chars().take(160).collect(),
                    }
                    .replace(',', ";")
                );
                assert!(!enforce_wire, "wire-budget workload did not complete");
                return;
            }
        }
    }
    println!(
        "{label},writes,total_ms,{}",
        started_all.elapsed().as_millis()
    );

    if let (Some(meter), Some((sent, received))) = (&wire, wire_before) {
        assert_eq!(meter.snapshot().failed_connections, 0);
        for (name, delta) in [
            (
                "client_socket_bytes",
                meter.sent_bytes().saturating_sub(sent),
            ),
            (
                "server_socket_bytes",
                meter.received_bytes().saturating_sub(received),
            ),
        ] {
            println!(
                "{label},write_volume,{name},{delta},per_append,{}",
                delta / appends.max(1)
            );
        }
        let steady_max = wire_samples
            .iter()
            .skip(2)
            .copied()
            .max()
            .unwrap_or_default();
        println!(
            "{label},write_volume,client_socket_bytes_steady_max,{steady_max},per_append,{steady_max}"
        );
        if enforce_wire {
            println!("{label},wire_budget,passed,appends,{appends}");
        }
        wire_samples.sort_unstable();
        if !wire_samples.is_empty() {
            for (name, value) in [
                (
                    "client_socket_bytes_p50",
                    wire_samples[(wire_samples.len() - 1) / 2],
                ),
                (
                    "client_socket_bytes_max",
                    wire_samples[wire_samples.len() - 1],
                ),
            ] {
                println!("{label},write_volume,{name},{value},per_append,{value}");
            }
        }
    }
    let counters_after = write_counters(backend).await;
    for (name, after) in counters_after {
        if let Some((_, before)) = counters_before.iter().find(|(key, _)| *key == name) {
            let delta = after.saturating_sub(*before);
            println!(
                "{label},write_volume,{name},{delta},per_append,{}",
                delta / appends.max(1)
            );
        }
    }
    let (versions, data_bytes, versions_digest, replay_digest) = full_digest(store.as_ref()).await;
    println!(
        "{label},equivalence,versions,{versions},data_bytes,{data_bytes},versions_digest,{versions_digest:016x},replay_digest,{replay_digest:016x}"
    );
    // A fresh store instance reads only persisted state (restart/reconstruction).
    let reopened = open(backend, &prefix).await;
    let (_, _, reopened_versions, reopened_replay) = full_digest(reopened.as_ref()).await;
    println!(
        "{label},restart,versions_digest,{reopened_versions:016x},replay_digest,{reopened_replay:016x}"
    );

    let mut rng = Lcg(0x5eed);
    let mut latest_ns = Vec::new();
    let mut random_ns = Vec::new();
    let mut page_ns = Vec::new();
    let mut replay_ns = Vec::new();
    for _ in 0..3 {
        black_box(store.get_latest(APP, CHANNEL, &serial).await.unwrap());
    }
    for _ in 0..51 {
        let started = Instant::now();
        black_box(store.get_latest(APP, CHANNEL, &serial).await.unwrap());
        latest_ns.push(started.elapsed().as_nanos() as u64);
    }
    for _ in 0..51 {
        let target = 1 + rng.next() % appends;
        let started = Instant::now();
        let page = store
            .get_versions(VersionStoreReadRequest {
                app_id: APP.into(),
                channel: CHANNEL.into(),
                message_serial: serial.clone(),
                direction: VersionStoreDirection::OldestFirst,
                limit: 1,
                cursor: Some(VersionStoreCursor {
                    version: 1,
                    version_serial: version(target - 1).serial,
                    direction: VersionStoreDirection::OldestFirst,
                }),
            })
            .await
            .unwrap();
        random_ns.push(started.elapsed().as_nanos() as u64);
        assert_eq!(page.items[0].version_serial(), &version(target).serial);
    }
    for _ in 0..21 {
        let start = rng.next() % appends;
        let started = Instant::now();
        black_box(
            store
                .get_versions(VersionStoreReadRequest {
                    app_id: APP.into(),
                    channel: CHANNEL.into(),
                    message_serial: serial.clone(),
                    direction: VersionStoreDirection::NewestFirst,
                    limit: 100,
                    cursor: Some(VersionStoreCursor {
                        version: 1,
                        version_serial: version(start + 1).serial,
                        direction: VersionStoreDirection::NewestFirst,
                    }),
                })
                .await
                .unwrap(),
        );
        page_ns.push(started.elapsed().as_nanos() as u64);
    }
    for _ in 0..21 {
        let after = rng.next() % appends;
        let started = Instant::now();
        black_box(
            store
                .replay_after(VersionReplayRequest {
                    app_id: APP.into(),
                    channel: CHANNEL.into(),
                    after_delivery_serial: after,
                    limit: 100,
                })
                .await
                .unwrap(),
        );
        replay_ns.push(started.elapsed().as_nanos() as u64);
    }
    report(&label, "append_ns", write_ns);
    report(&label, "get_latest_ns", latest_ns);
    report(&label, "random_version_read_ns", random_ns);
    report(&label, "page100_read_ns", page_ns);
    report(&label, "replay100_read_ns", replay_ns);
    for (metric, value) in retained(backend, &prefix).await {
        println!("{label},retained,{metric},{value}");
    }
    drop((store, reopened));
    cleanup(backend, &prefix).await;
}

/// Drop every object created under this case's unique prefix so in-memory
/// emulators do not accumulate earlier cases.
pub(super) async fn cleanup(backend: &str, prefix: &str) {
    let owned = |name: &String| name.starts_with(&format!("{prefix}_"));
    match backend {
        #[cfg(feature = "postgres")]
        "postgres" => {
            let pool = sqlx::PgPool::connect("postgres://c2:c2-local-only@127.0.0.1:25471/c2")
                .await
                .unwrap();
            let tables: Vec<String> = sqlx::query_scalar(
                "SELECT table_name::text FROM information_schema.tables WHERE table_schema = 'public'",
            )
            .fetch_all(&pool)
            .await
            .unwrap();
            for table in tables.iter().filter(|name| owned(name)) {
                let sql = format!("DROP TABLE IF EXISTS \"{table}\" CASCADE");
                sqlx::query(sqlx::AssertSqlSafe(sql.as_str()))
                    .execute(&pool)
                    .await
                    .unwrap();
            }
        }
        #[cfg(feature = "mysql")]
        "mysql" => {
            let pool = sqlx::MySqlPool::connect("mysql://root:c2-local-only@127.0.0.1:25472/c2")
                .await
                .unwrap();
            let mut tables: Vec<String> = sqlx::query_scalar(
                "SELECT CAST(table_name AS CHAR) FROM information_schema.tables WHERE table_schema = 'c2'",
            )
            .fetch_all(&pool)
            .await
            .unwrap();
            tables.sort_by_key(|name| !name.ends_with("_chunks"));
            for table in tables.iter().filter(|name| owned(name)) {
                let sql = format!("DROP TABLE IF EXISTS `{table}`");
                sqlx::query(sqlx::AssertSqlSafe(sql.as_str()))
                    .execute(&pool)
                    .await
                    .unwrap();
            }
        }
        #[cfg(feature = "dynamodb")]
        "dynamodb" => {
            let config = aws_config::from_env()
                .region(aws_sdk_dynamodb::config::Region::new("us-east-1"))
                .endpoint_url("http://127.0.0.1:25473")
                .credentials_provider(aws_sdk_dynamodb::config::Credentials::new(
                    "c2",
                    "c2-local-only",
                    None,
                    None,
                    "static",
                ))
                .load()
                .await;
            let client = aws_sdk_dynamodb::Client::new(&config);
            let tables = client.list_tables().send().await.unwrap();
            for table in tables.table_names().iter().filter(|name| owned(name)) {
                client
                    .delete_table()
                    .table_name(table)
                    .send()
                    .await
                    .unwrap();
            }
        }
        #[cfg(feature = "scylladb")]
        "scylladb" => {
            let session = ::scylla::client::session_builder::SessionBuilder::new()
                .known_node("127.0.0.1:25474")
                .build()
                .await
                .unwrap();
            let tables: Vec<String> = session
                .query_unpaged(
                    "SELECT table_name FROM system_schema.tables WHERE keyspace_name = 'c2'",
                    (),
                )
                .await
                .unwrap()
                .into_rows_result()
                .unwrap()
                .rows::<(String,)>()
                .unwrap()
                .map(|row| row.unwrap().0)
                .collect();
            for table in tables.iter().filter(|name| owned(name)) {
                session
                    .query_unpaged(format!("DROP TABLE IF EXISTS c2.{table}"), ())
                    .await
                    .unwrap();
            }
        }
        #[cfg(feature = "surrealdb")]
        "surrealdb" => {
            let db = surrealdb::engine::any::connect("ws://127.0.0.1:25475")
                .await
                .unwrap();
            db.signin(surrealdb::opt::auth::Root {
                username: "root".into(),
                password: "c2-local-only".into(),
            })
            .await
            .unwrap();
            db.use_ns("c2").use_db("c2").await.unwrap();
            for suffix in ["key_v1", "mutate_v1"] {
                db.query(format!(
                    "REMOVE FUNCTION IF EXISTS fn::{prefix}_version_entries_{suffix}"
                ))
                .await
                .unwrap()
                .check()
                .unwrap();
            }
            let mut info = db.query("INFO FOR DB").await.unwrap();
            let info: Option<surrealdb_types::Value> = info.take(0).unwrap();
            let tables: Vec<String> = info
                .map(surrealdb_types::Value::into_json_value)
                .as_ref()
                .and_then(|value| value.get("tables"))
                .and_then(|value| value.as_object())
                .map(|map| map.keys().cloned().collect::<Vec<_>>())
                .unwrap_or_default();
            for table in tables.iter().filter(|name| owned(name)) {
                db.query(format!("REMOVE TABLE IF EXISTS {table}"))
                    .await
                    .unwrap()
                    .check()
                    .unwrap();
            }
        }
        _ => {}
    }
}

async fn retained(backend: &str, prefix: &str) -> Vec<(String, u64)> {
    match backend {
        #[cfg(feature = "postgres")]
        "postgres" => retained_postgres(prefix).await,
        #[cfg(feature = "mysql")]
        "mysql" => retained_mysql(prefix).await,
        #[cfg(feature = "dynamodb")]
        "dynamodb" => retained_dynamodb(prefix).await,
        #[cfg(feature = "scylladb")]
        "scylladb" => retained_scylla(prefix).await,
        #[cfg(feature = "surrealdb")]
        "surrealdb" => retained_surreal(prefix).await,
        _ => Vec::new(),
    }
}

#[cfg(feature = "postgres")]
async fn retained_postgres(prefix: &str) -> Vec<(String, u64)> {
    use sqlx::Row;
    let pool = sqlx::PgPool::connect("postgres://c2:c2-local-only@127.0.0.1:25471/c2")
        .await
        .unwrap();
    let tables: Vec<String> = sqlx::query_scalar(
        "SELECT table_name::text FROM information_schema.tables WHERE table_schema = 'public' ORDER BY 1",
    )
    .fetch_all(&pool)
    .await
    .unwrap()
    .into_iter()
    .filter(|name: &String| name.starts_with(&format!("{prefix}_")))
    .collect();
    let mut out = Vec::new();
    let (mut logical, mut rows, mut physical, mut vacuumed) = (0u64, 0u64, 0u64, 0u64);
    for table in &tables {
        let columns = sqlx::query(
            "SELECT column_name::text AS name, data_type::text AS kind FROM information_schema.columns WHERE table_name = $1",
        )
        .bind(table)
        .fetch_all(&pool)
        .await
        .unwrap();
        let mut table_logical = 0u64;
        for column in columns {
            let name: String = column.get("name");
            let kind: String = column.get("kind");
            let expr = match kind.as_str() {
                "bytea" | "text" | "character varying" => format!("octet_length(\"{name}\")"),
                "jsonb" | "json" => format!("octet_length(\"{name}\"::text)"),
                "bigint" => format!("CASE WHEN \"{name}\" IS NULL THEN 0 ELSE 8 END"),
                "integer" => format!("CASE WHEN \"{name}\" IS NULL THEN 0 ELSE 4 END"),
                "boolean" => format!("CASE WHEN \"{name}\" IS NULL THEN 0 ELSE 1 END"),
                _ => continue,
            };
            let sql = format!("SELECT COALESCE(SUM({expr}), 0)::bigint FROM \"{table}\"");
            let value: i64 = sqlx::query_scalar(sqlx::AssertSqlSafe(sql.as_str()))
                .fetch_one(&pool)
                .await
                .unwrap();
            table_logical += value as u64;
        }
        let sql = format!("SELECT COUNT(*)::bigint FROM \"{table}\"");
        let count: i64 = sqlx::query_scalar(sqlx::AssertSqlSafe(sql.as_str()))
            .fetch_one(&pool)
            .await
            .unwrap();
        let size: i64 = sqlx::query_scalar("SELECT pg_total_relation_size($1::regclass)")
            .bind(format!("\"{table}\""))
            .fetch_one(&pool)
            .await
            .unwrap();
        let dead: i64 = sqlx::query_scalar(
            "SELECT COALESCE(n_dead_tup, 0) FROM pg_stat_all_tables WHERE relid = $1::regclass",
        )
        .bind(format!("\"{table}\""))
        .fetch_one(&pool)
        .await
        .unwrap();
        out.push((
            format!(
                "table{}_dead_tuples_before_vacuum",
                table.trim_start_matches(prefix)
            ),
            dead as u64,
        ));
        let sql = format!("VACUUM FULL \"{table}\"");
        sqlx::query(sqlx::AssertSqlSafe(sql.as_str()))
            .execute(&pool)
            .await
            .unwrap();
        let after: i64 = sqlx::query_scalar("SELECT pg_total_relation_size($1::regclass)")
            .bind(format!("\"{table}\""))
            .fetch_one(&pool)
            .await
            .unwrap();
        let short = table.trim_start_matches(prefix);
        out.push((format!("table{short}_rows"), count as u64));
        out.push((format!("table{short}_logical_bytes"), table_logical));
        out.push((format!("table{short}_physical_bytes"), size as u64));
        logical += table_logical;
        rows += count as u64;
        physical += size as u64;
        vacuumed += after as u64;
    }
    out.push(("total_rows".into(), rows));
    out.push(("total_logical_bytes".into(), logical));
    out.push(("total_physical_bytes".into(), physical));
    out.push(("total_physical_after_vacuum_full_bytes".into(), vacuumed));
    out
}

#[cfg(feature = "mysql")]
async fn retained_mysql(prefix: &str) -> Vec<(String, u64)> {
    use sqlx::Row;
    let pool = sqlx::MySqlPool::connect("mysql://root:c2-local-only@127.0.0.1:25472/c2")
        .await
        .unwrap();
    let mut conn = pool.acquire().await.unwrap();
    sqlx::query("SET SESSION information_schema_stats_expiry = 0")
        .execute(&mut *conn)
        .await
        .unwrap();
    let tables: Vec<String> = sqlx::query_scalar(
        "SELECT CAST(table_name AS CHAR) FROM information_schema.tables WHERE table_schema = 'c2' ORDER BY 1",
    )
    .fetch_all(&mut *conn)
    .await
    .unwrap()
    .into_iter()
    .filter(|name: &String| name.starts_with(&format!("{prefix}_")))
    .collect();
    let mut out = Vec::new();
    let (mut logical, mut rows, mut physical) = (0u64, 0u64, 0u64);
    for table in &tables {
        let columns = sqlx::query(
            "SELECT CAST(column_name AS CHAR) AS name, CAST(data_type AS CHAR) AS kind FROM information_schema.columns WHERE table_schema = 'c2' AND table_name = ?",
        )
        .bind(table)
        .fetch_all(&mut *conn)
        .await
        .unwrap();
        let mut table_logical = 0u64;
        for column in columns {
            let name: String = column.get("name");
            let kind: String = column.get("kind");
            let expr = match kind.as_str() {
                "blob" | "longblob" | "mediumblob" | "text" | "longtext" | "mediumtext"
                | "varchar" | "varbinary" | "json" => format!("OCTET_LENGTH(`{name}`)"),
                "bigint" => format!("IF(`{name}` IS NULL, 0, 8)"),
                "int" => format!("IF(`{name}` IS NULL, 0, 4)"),
                "tinyint" => format!("IF(`{name}` IS NULL, 0, 1)"),
                _ => continue,
            };
            let sql = format!("SELECT CAST(COALESCE(SUM({expr}), 0) AS SIGNED) FROM `{table}`");
            let value: i64 = sqlx::query_scalar(sqlx::AssertSqlSafe(sql.as_str()))
                .fetch_one(&mut *conn)
                .await
                .unwrap();
            table_logical += value as u64;
        }
        let sql = format!("SELECT COUNT(*) FROM `{table}`");
        let count: i64 = sqlx::query_scalar(sqlx::AssertSqlSafe(sql.as_str()))
            .fetch_one(&mut *conn)
            .await
            .unwrap();
        let sql = format!("ANALYZE TABLE `{table}`");
        sqlx::query(sqlx::AssertSqlSafe(sql.as_str()))
            .execute(&mut *conn)
            .await
            .unwrap();
        let size: i64 = sqlx::query_scalar(
            "SELECT CAST(data_length + index_length AS SIGNED) FROM information_schema.tables WHERE table_schema = 'c2' AND table_name = ?",
        )
        .bind(table)
        .fetch_one(&mut *conn)
        .await
        .unwrap();
        let short = table.trim_start_matches(prefix);
        out.push((format!("table{short}_rows"), count as u64));
        out.push((format!("table{short}_logical_bytes"), table_logical));
        out.push((format!("table{short}_physical_bytes"), size as u64));
        logical += table_logical;
        rows += count as u64;
        physical += size as u64;
    }
    out.push(("total_rows".into(), rows));
    out.push(("total_logical_bytes".into(), logical));
    out.push(("total_physical_bytes".into(), physical));
    out
}

#[cfg(feature = "dynamodb")]
fn dynamo_size(value: &aws_sdk_dynamodb::types::AttributeValue) -> u64 {
    use aws_sdk_dynamodb::types::AttributeValue;
    match value {
        AttributeValue::S(s) | AttributeValue::N(s) => s.len() as u64,
        AttributeValue::B(b) => b.as_ref().len() as u64,
        AttributeValue::Bool(_) | AttributeValue::Null(_) => 1,
        AttributeValue::L(items) => 3 + items.iter().map(dynamo_size).sum::<u64>(),
        AttributeValue::M(map) => {
            3 + map
                .iter()
                .map(|(key, value)| key.len() as u64 + dynamo_size(value))
                .sum::<u64>()
        }
        AttributeValue::Ss(items) | AttributeValue::Ns(items) => {
            items.iter().map(|s| s.len() as u64).sum()
        }
        AttributeValue::Bs(items) => items.iter().map(|b| b.as_ref().len() as u64).sum(),
        _ => 0,
    }
}

#[cfg(feature = "dynamodb")]
async fn retained_dynamodb(prefix: &str) -> Vec<(String, u64)> {
    let config = aws_config::from_env()
        .region(aws_sdk_dynamodb::config::Region::new("us-east-1"))
        .endpoint_url("http://127.0.0.1:25473")
        .credentials_provider(aws_sdk_dynamodb::config::Credentials::new(
            "c2",
            "c2-local-only",
            None,
            None,
            "static",
        ))
        .load()
        .await;
    let client = aws_sdk_dynamodb::Client::new(&config);
    let tables: Vec<String> = client
        .list_tables()
        .send()
        .await
        .unwrap()
        .table_names()
        .iter()
        .filter(|name| name.starts_with(&format!("{prefix}_")))
        .cloned()
        .collect();
    let mut out = Vec::new();
    let (mut logical, mut rows, mut gsi) = (0u64, 0u64, 0u64);
    for table in &tables {
        let (mut table_bytes, mut table_rows, mut table_gsi) = (0u64, 0u64, 0u64);
        let mut start = None;
        loop {
            let page = client
                .scan()
                .table_name(table)
                .set_exclusive_start_key(start)
                .send()
                .await
                .unwrap();
            for item in page.items() {
                let size: u64 = item
                    .iter()
                    .map(|(key, value)| key.len() as u64 + dynamo_size(value))
                    .sum();
                table_bytes += size;
                table_rows += 1;
                // Entries GSIs project ALL attributes for items carrying their keys.
                table_gsi += size
                    * (u64::from(item.contains_key("delivery_serial"))
                        + u64::from(item.contains_key("app_channel_message")));
            }
            start = page.last_evaluated_key().cloned();
            if start.is_none() {
                break;
            }
        }
        let short = table.trim_start_matches(prefix);
        out.push((format!("table{short}_rows"), table_rows));
        out.push((format!("table{short}_logical_bytes"), table_bytes));
        out.push((format!("table{short}_gsi_projected_bytes"), table_gsi));
        logical += table_bytes;
        rows += table_rows;
        gsi += table_gsi;
    }
    out.push(("total_rows".into(), rows));
    out.push(("total_logical_bytes".into(), logical));
    out.push(("total_gsi_projected_bytes".into(), gsi));
    out
}

#[cfg(feature = "scylladb")]
fn cql_size(value: &::scylla::value::CqlValue) -> u64 {
    use ::scylla::value::CqlValue;
    match value {
        CqlValue::Blob(bytes) => bytes.len() as u64,
        CqlValue::Text(text) | CqlValue::Ascii(text) => text.len() as u64,
        CqlValue::BigInt(_) | CqlValue::Double(_) | CqlValue::Timestamp(_) => 8,
        CqlValue::Int(_) | CqlValue::Float(_) => 4,
        CqlValue::Boolean(_) => 1,
        CqlValue::List(items) | CqlValue::Set(items) => items.iter().map(cql_size).sum(),
        CqlValue::Map(items) => items.iter().map(|(k, v)| cql_size(k) + cql_size(v)).sum(),
        _ => 0,
    }
}

#[cfg(feature = "scylladb")]
async fn retained_scylla(prefix: &str) -> Vec<(String, u64)> {
    use futures_util::StreamExt;
    let session = ::scylla::client::session_builder::SessionBuilder::new()
        .known_node("127.0.0.1:25474")
        .build()
        .await
        .unwrap();
    let tables: Vec<String> = session
        .query_unpaged(
            "SELECT table_name FROM system_schema.tables WHERE keyspace_name = 'c2'",
            (),
        )
        .await
        .unwrap()
        .into_rows_result()
        .unwrap()
        .rows::<(String,)>()
        .unwrap()
        .map(|row| row.unwrap().0)
        .filter(|name| name.starts_with(&format!("{prefix}_")))
        .collect();
    let mut out = Vec::new();
    let (mut logical, mut rows) = (0u64, 0u64);
    for table in &tables {
        let mut stream = session
            .query_iter(format!("SELECT * FROM c2.{table}"), ())
            .await
            .unwrap()
            .rows_stream::<::scylla::value::Row>()
            .unwrap();
        let (mut table_bytes, mut table_rows) = (0u64, 0u64);
        while let Some(row) = stream.next().await {
            let row = row.unwrap();
            table_rows += 1;
            table_bytes += row.columns.iter().flatten().map(cql_size).sum::<u64>();
        }
        let short = table.trim_start_matches(prefix);
        out.push((format!("table{short}_rows"), table_rows));
        out.push((format!("table{short}_logical_bytes"), table_bytes));
        logical += table_bytes;
        rows += table_rows;
    }
    out.push(("total_rows".into(), rows));
    out.push(("total_logical_bytes".into(), logical));
    out
}

#[cfg(feature = "surrealdb")]
fn surreal_size(value: &surrealdb_types::Value) -> u64 {
    use surrealdb_types::Value;
    match value {
        Value::String(text) => text.len() as u64,
        Value::Bytes(bytes) => bytes.len() as u64,
        // Byte vectors are persisted as arrays of numbers; count one unit each.
        Value::Number(_) | Value::Bool(_) => 1,
        Value::Array(items) => items.iter().map(surreal_size).sum(),
        Value::Object(map) => map
            .iter()
            .map(|(key, value)| key.len() as u64 + surreal_size(value))
            .sum(),
        _ => 0,
    }
}

#[cfg(feature = "surrealdb")]
async fn retained_surreal(prefix: &str) -> Vec<(String, u64)> {
    let db = surrealdb::engine::any::connect("ws://127.0.0.1:25475")
        .await
        .unwrap();
    db.signin(surrealdb::opt::auth::Root {
        username: "root".into(),
        password: "c2-local-only".into(),
    })
    .await
    .unwrap();
    db.use_ns("c2").use_db("c2").await.unwrap();
    let mut info = db.query("INFO FOR DB").await.unwrap();
    let info: Option<surrealdb_types::Value> = info.take(0).unwrap();
    let json = info.map(surrealdb_types::Value::into_json_value);
    let tables: Vec<String> = json
        .as_ref()
        .and_then(|value| value.get("tables"))
        .and_then(|value| value.as_object())
        .map(|map| map.keys().cloned().collect::<Vec<_>>())
        .unwrap_or_default()
        .into_iter()
        .filter(|name: &String| name.starts_with(&format!("{prefix}_")))
        .collect();
    let mut out = Vec::new();
    let (mut logical, mut rows, mut encoded) = (0u64, 0u64, 0u64);
    for table in &tables {
        let mut response = db.query(format!("SELECT * FROM {table}")).await.unwrap();
        let items: Vec<surrealdb_types::Value> = response.take(0).unwrap();
        let table_rows = items.len() as u64;
        let table_bytes: u64 = items.iter().map(surreal_size).sum();
        let table_encoded: u64 = items
            .into_iter()
            .map(|item| serde_json::to_vec(&item.into_json_value()).unwrap().len() as u64)
            .sum();
        let short = table.trim_start_matches(prefix);
        out.push((format!("table{short}_rows"), table_rows));
        out.push((format!("table{short}_logical_bytes"), table_bytes));
        out.push((format!("table{short}_json_bytes"), table_encoded));
        logical += table_bytes;
        rows += table_rows;
        encoded += table_encoded;
    }
    out.push(("total_rows".into(), rows));
    out.push(("total_logical_bytes".into(), logical));
    out.push(("total_json_bytes".into(), encoded));
    out
}

fn env_list<T: std::str::FromStr>(name: &str, default: &str) -> Vec<T> {
    std::env::var(name)
        .unwrap_or_else(|_| default.to_string())
        .split(',')
        .filter(|value| !value.is_empty())
        .map(|value| {
            value
                .parse()
                .unwrap_or_else(|_| panic!("invalid {name} value {value}"))
        })
        .collect()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires isolated C2 Compose services on ports 25471-25475"]
async fn c2_durable_append_storage() {
    let backends: Vec<String> =
        env_list("C2_BACKENDS", "postgres,mysql,dynamodb,scylladb,surrealdb");
    let appends: Vec<u64> = env_list("C2_APPENDS", "128,512,2000");
    let fragments: Vec<usize> = env_list("C2_FRAGMENTS", "16,64,256");
    let reps: u32 = std::env::var("C2_REPS")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(1);
    println!("backend,appends,fragment_bytes,rep,kind,metric,...");
    for rep in 1..=reps {
        for backend in &backends {
            for &count in &appends {
                for &fragment_bytes in &fragments {
                    run_case(backend, count, fragment_bytes, rep).await;
                }
            }
        }
    }
}

/// Canonical full-state records for `appends` appends with deterministic
/// fragments, computed with the unchanged mutation model rather than a store.
fn reference_chain(appends: u64, fragment_bytes: usize) -> Vec<StoredVersionRecord> {
    let stream_id = format!("{APP}/{CHANNEL}");
    let mut rng = Lcg(0xC2 ^ appends ^ ((fragment_bytes as u64) << 20));
    let mut chain = vec![create_record().with_delivery_position(&stream_id, 1)];
    for n in 1..=appends {
        let current = chain.last().unwrap();
        let request = mixed_request(current, n, fragment(&mut rng, fragment_bytes));
        chain.push(current.apply_mutation(&request, &stream_id, n + 1).unwrap());
    }
    chain
}

/// Every fifth append carries an idempotency receipt.
fn mixed_request(current: &StoredVersionRecord, n: u64, data: String) -> VersionMutationRequest {
    VersionMutationRequest {
        app_id: APP.into(),
        channel: CHANNEL.into(),
        message_serial: MessageSerial::new("msg:1").unwrap(),
        expected: VersionPrecondition::from_record(current),
        version: version(n),
        mutation: VersionMutation::Append(MessageAppend {
            data_fragment: data,
            extras: None,
        }),
        idempotency: n.is_multiple_of(5).then(|| {
            sockudo_core::message_envelope::PublishIdempotencyMetadata {
                cache_key: format!("c2-op-{n}"),
                payload_fingerprint: format!("c2-fingerprint-{n}"),
            }
        }),
        limits: VersionMutationLimits::default(),
    }
}

fn chain_digest(records: &[StoredVersionRecord]) -> u64 {
    let mut state = 0xcbf29ce484222325u64;
    for record in records {
        digest(&mut state, record);
    }
    state
}

/// Mixed-release upgrade/rollback check. The same file runs in the
/// pre-change and candidate builds:
/// `legacy_write` (old) → `continue` (new) → `legacy_read` (old).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires isolated C2 Compose services and C2_MIXED_STEP/C2_MIXED_PREFIX"]
async fn c2_mixed_version() {
    let step = std::env::var("C2_MIXED_STEP").unwrap();
    let prefix = std::env::var("C2_MIXED_PREFIX").unwrap();
    let backends: Vec<String> =
        env_list("C2_BACKENDS", "postgres,mysql,dynamodb,scylladb,surrealdb");
    const LEGACY: u64 = 40;
    const TOTAL: u64 = 80;
    const BYTES: usize = 48;
    let reference = reference_chain(TOTAL, BYTES);
    let mut rng = Lcg(0xC2 ^ TOTAL ^ ((BYTES as u64) << 20));
    let fragments: Vec<String> = (0..TOTAL).map(|_| fragment(&mut rng, BYTES)).collect();
    for backend in &backends {
        let store = open(backend, &prefix).await;
        if step == "continue" && std::env::var_os("C2_CHUNKED").is_some() {
            store.set_append_storage_enabled(true).await.unwrap();
        }
        let label = format!("{backend},{step}");
        match step.as_str() {
            "legacy_write" | "continue" => {
                let (start, end) = if step == "legacy_write" {
                    let VersionCreateResult::Applied { .. } = store
                        .commit_create(VersionCreateRequest {
                            record: create_record(),
                            limits: VersionCreateLimits::default(),
                        })
                        .await
                        .unwrap()
                    else {
                        panic!("create was not applied");
                    };
                    (1, LEGACY)
                } else {
                    (LEGACY + 1, TOTAL)
                };
                for n in start..=end {
                    let current = store
                        .get_latest(APP, CHANNEL, &MessageSerial::new("msg:1").unwrap())
                        .await
                        .unwrap()
                        .unwrap();
                    let request = mixed_request(&current, n, fragments[n as usize - 1].clone());
                    match store.compare_and_apply(request).await.unwrap() {
                        VersionMutationResult::Applied { .. } => {}
                        other => panic!("{label}: append {n} not applied: {other:?}"),
                    }
                }
                let committed = end as usize + 1;
                let (versions, _, versions_digest, replay_digest) =
                    full_digest(store.as_ref()).await;
                let expected = chain_digest(&reference[..committed]);
                println!(
                    "{label},versions,{versions},digest_matches_reference,{},replay_matches_reference,{}",
                    versions_digest == expected,
                    replay_digest == expected
                );
                assert_eq!(versions as usize, committed);
                assert_eq!(versions_digest, expected);
                assert_eq!(replay_digest, expected);
                if step == "continue" {
                    // Replaying a receipt written by the older release, and one
                    // written by this release, returns the original version.
                    for n in [LEGACY / 5 * 5, TOTAL] {
                        let request = mixed_request(
                            &reference[n as usize - 1],
                            n,
                            fragments[n as usize - 1].clone(),
                        );
                        let VersionMutationResult::Duplicate { record, .. } =
                            store.compare_and_apply(request).await.unwrap()
                        else {
                            panic!("{label}: receipt {n} was not a duplicate");
                        };
                        assert_eq!(
                            chain_digest(&[record]),
                            chain_digest(&reference[n as usize..=n as usize])
                        );
                        println!("{label},duplicate_receipt,{n},matches_reference,true");
                    }
                    let reopened = open(backend, &prefix).await;
                    let (_, _, reopened_digest, _) = full_digest(reopened.as_ref()).await;
                    assert_eq!(reopened_digest, expected);
                    println!("{label},restart_matches_reference,true");
                }
            }
            "legacy_read" => {
                let serial = MessageSerial::new("msg:1").unwrap();
                let latest = store.get_latest(APP, CHANNEL, &serial).await;
                let latest_matches =
                    latest
                        .as_ref()
                        .ok()
                        .and_then(|record| record.as_ref())
                        .map(|record| {
                            chain_digest(std::slice::from_ref(record))
                                == chain_digest(&reference[TOTAL as usize..])
                        });
                let page = store
                    .get_versions(VersionStoreReadRequest {
                        app_id: APP.into(),
                        channel: CHANNEL.into(),
                        message_serial: serial.clone(),
                        direction: VersionStoreDirection::NewestFirst,
                        limit: 10,
                        cursor: None,
                    })
                    .await;
                let replay = store
                    .replay_after(VersionReplayRequest {
                        app_id: APP.into(),
                        channel: CHANNEL.into(),
                        after_delivery_serial: 0,
                        limit: 100,
                    })
                    .await;
                let outcome = |ok: bool, error: Option<String>| {
                    if ok {
                        "ok".to_string()
                    } else {
                        format!(
                            "error:{}",
                            error
                                .unwrap_or_default()
                                .chars()
                                .take(120)
                                .collect::<String>()
                                .replace(',', ";")
                        )
                    }
                };
                println!(
                    "{label},get_latest,{},latest_matches_reference,{latest_matches:?},newest_page,{},replay,{}",
                    outcome(
                        latest.is_ok(),
                        latest.as_ref().err().map(ToString::to_string)
                    ),
                    outcome(page.is_ok(), page.as_ref().err().map(ToString::to_string)),
                    outcome(
                        replay.is_ok(),
                        replay.as_ref().err().map(ToString::to_string)
                    ),
                );
            }
            "cleanup" => {
                drop(store);
                cleanup(backend, &prefix).await;
            }
            other => panic!("unknown C2_MIXED_STEP {other}"),
        }
    }
}

/// Two writers race appends on one message through separate store
/// instances. Reports whether every committed version extends its
/// predecessor by exactly its own fragment and each fragment appears once.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires isolated C2 Compose services on ports 25471-25475"]
async fn c2_concurrency_probe() {
    let backends: Vec<String> =
        env_list("C2_BACKENDS", "postgres,mysql,dynamodb,scylladb,surrealdb");
    let trials: u32 = std::env::var("C2_TRIALS")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(10);
    for backend in &backends {
        for trial in 0..trials {
            let prefix = format!("c2c{}", &uuid::Uuid::new_v4().simple().to_string()[..12]);
            let stores = [open(backend, &prefix).await, open(backend, &prefix).await];
            if std::env::var_os("C2_CHUNKED").is_some() {
                stores[0].set_append_storage_enabled(true).await.unwrap();
            }
            let VersionCreateResult::Applied { .. } = stores[0]
                .commit_create(VersionCreateRequest {
                    record: create_record(),
                    limits: VersionCreateLimits::default(),
                })
                .await
                .unwrap()
            else {
                panic!("create was not applied");
            };
            let counter = Arc::new(std::sync::atomic::AtomicU64::new(1));
            let outcomes = Arc::new(std::sync::Mutex::new((0u64, 0u64)));
            let error_kinds = Arc::new(std::sync::Mutex::new(std::collections::BTreeMap::<
                String,
                u64,
            >::new()));
            let mut tasks = Vec::new();
            for (worker, store) in stores.iter().cloned().enumerate() {
                let counter = Arc::clone(&counter);
                let outcomes = Arc::clone(&outcomes);
                let error_kinds = Arc::clone(&error_kinds);
                tasks.push(tokio::spawn(async move {
                    let serial = MessageSerial::new("msg:1").unwrap();
                    for index in 0..20 {
                        let data_fragment = format!("[{worker}.{index}]");
                        for _ in 0..1000 {
                            let Ok(Some(current)) = store.get_latest(APP, CHANNEL, &serial).await
                            else {
                                outcomes.lock().unwrap().1 += 1;
                                continue;
                            };
                            let n = counter.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                            let request = VersionMutationRequest {
                                app_id: APP.into(),
                                channel: CHANNEL.into(),
                                message_serial: serial.clone(),
                                expected: VersionPrecondition::from_record(&current),
                                version: version(n),
                                mutation: VersionMutation::Append(MessageAppend {
                                    data_fragment: data_fragment.clone(),
                                    extras: None,
                                }),
                                idempotency: None,
                                limits: VersionMutationLimits::default(),
                            };
                            match store.compare_and_apply(request).await {
                                Ok(VersionMutationResult::Applied { .. }) => break,
                                Ok(_) => outcomes.lock().unwrap().0 += 1,
                                Err(error) => {
                                    outcomes.lock().unwrap().1 += 1;
                                    let kind: String = error
                                        .to_string()
                                        .chars()
                                        .filter(|c| !c.is_ascii_digit())
                                        .take(400)
                                        .collect();
                                    *error_kinds.lock().unwrap().entry(kind).or_default() += 1;
                                }
                            }
                        }
                    }
                }));
            }
            for task in tasks {
                task.await.unwrap();
            }
            let (conflicts, errors) = *outcomes.lock().unwrap();
            let mut cursor = None;
            let mut records = Vec::new();
            let mut read_error = None;
            loop {
                match stores[0]
                    .get_versions(VersionStoreReadRequest {
                        app_id: APP.into(),
                        channel: CHANNEL.into(),
                        message_serial: MessageSerial::new("msg:1").unwrap(),
                        direction: VersionStoreDirection::OldestFirst,
                        limit: 100,
                        cursor,
                    })
                    .await
                {
                    Ok(page) => {
                        records.extend(page.items);
                        cursor = page.next_cursor;
                        if cursor.is_none() {
                            break;
                        }
                    }
                    Err(error) => {
                        read_error = Some(error.to_string().chars().take(100).collect::<String>());
                        break;
                    }
                }
            }
            let data = |record: &StoredVersionRecord| {
                record
                    .message
                    .data
                    .clone()
                    .and_then(MessageData::into_string)
                    .unwrap_or_default()
            };
            let chain_ok = records.windows(2).all(|pair| {
                data(&pair[1])
                    == format!(
                        "{}{}",
                        data(&pair[0]),
                        pair[1]
                            .message
                            .append_fragment
                            .as_deref()
                            .unwrap_or_default()
                    )
            });
            let latest = records.last().map(data).unwrap_or_default();
            let fragments_once = (0..2).all(|worker| {
                (0..20).all(|index| latest.matches(&format!("[{worker}.{index}]")).count() == 1)
            });
            println!(
                "{backend},trial,{trial},versions,{},chain_ok,{chain_ok},fragments_once,{fragments_once},conflicts,{conflicts},errors,{errors},read_error,{}",
                records.len(),
                read_error.unwrap_or_default().replace(',', ";")
            );
            for (kind, count) in error_kinds.lock().unwrap().iter() {
                println!(
                    "{backend},trial,{trial},error_kind,{count},{}",
                    kind.replace(',', ";")
                );
            }
            drop(stores);
            cleanup(backend, &prefix).await;
        }
    }
}

/// Global database counters sampled outside the timed append loop. An isolated
/// container is required: unrelated database work would contaminate deltas.
async fn write_counters(backend: &str) -> Vec<(String, u64)> {
    match backend {
        #[cfg(feature = "dynamodb")]
        "dynamodb" => super::dynamodb::DynamoDbVersionStore::benchmark_write_counters(),
        #[cfg(feature = "scylladb")]
        "scylladb" => super::scylla::ScyllaVersionStore::benchmark_write_counters(),
        #[cfg(feature = "postgres")]
        "postgres" => {
            let pool = sqlx::PgPool::connect("postgres://c2:c2-local-only@127.0.0.1:25471/c2")
                .await
                .unwrap();
            let bytes: i64 = sqlx::query_scalar(
                "SELECT pg_wal_lsn_diff(pg_current_wal_insert_lsn(), '0/0')::bigint",
            )
            .fetch_one(&pool)
            .await
            .unwrap();
            vec![("wal_bytes".into(), bytes as u64)]
        }
        #[cfg(feature = "mysql")]
        "mysql" => {
            use sqlx::Row;
            let pool = sqlx::MySqlPool::connect("mysql://root:c2-local-only@127.0.0.1:25472/c2")
                .await
                .unwrap();
            let rows = sqlx::query("SHOW GLOBAL STATUS WHERE Variable_name IN ('Innodb_os_log_written', 'Innodb_data_written')")
                .fetch_all(&pool).await.unwrap();
            let mut out = rows
                .into_iter()
                .map(|row| {
                    (
                        row.get::<String, _>(0),
                        row.get::<String, _>(1).parse().unwrap(),
                    )
                })
                .collect::<Vec<_>>();
            let undo: Option<i64> = sqlx::query_scalar("SELECT COUNT FROM information_schema.INNODB_METRICS WHERE NAME = 'trx_rseg_history_len'")
                .fetch_optional(&pool).await.unwrap();
            if let Some(value) = undo {
                out.push(("undo_history_length".into(), value as u64));
            }
            out
        }
        _ => Vec::new(),
    }
}
