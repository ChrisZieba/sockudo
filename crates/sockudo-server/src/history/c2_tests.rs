//! Live append-storage checks against every durable version store. Requires
//! the isolated Compose project in `audits/performance-2026-09-05/c2`.
use super::*;
use sockudo_core::message_envelope::{MessageContent, MessageEnvelope, PublishIdempotencyMetadata};
use sockudo_core::options::{DatabaseConnection, VersionStoreDriver};
use sockudo_core::version_store::append_storage::StoredVersionPayload;
use sockudo_core::version_store::*;
use sockudo_core::versioned_messages::*;
use sockudo_protocol::messages::MessageData;

const APP: &str = "c2t";
const CHANNEL: &str = "ai:room";

fn serial(n: u64) -> VersionSerial {
    VersionSerial::new(format!("ver:{n:020}")).unwrap()
}

fn meta(n: u64) -> VersionMetadata {
    VersionMetadata {
        serial: serial(n),
        client_id: Some("agent".into()),
        timestamp_ms: 1_700_000_000_000 + n as i64,
        description: Some(format!("op {n}")),
        metadata: Some(sonic_rs::json!({"n": n})),
    }
}

fn fragment(n: u64) -> String {
    let mut out = format!("<{n}:");
    for i in 0..(n % 7) {
        out.push(if i % 2 == 0 { '\u{e9}' } else { '\u{1F642}' });
    }
    out.push('>');
    out
}

fn create(message: &str) -> StoredVersionRecord {
    StoredVersionRecord {
        app_id: APP.into(),
        channel: CHANNEL.into(),
        original_client_id: Some("agent".into()),
        envelope: Some(MessageEnvelope {
            message_id: Some(format!("{message}:client")),
            name: Some("ai.response".into()),
            data: Some(MessageContent::Text("seed".into())),
            publisher_client_id: Some("agent".into()),
            published_at_ms: Some(1),
            ..MessageEnvelope::default()
        }),
        message: VersionedMessage::new_create(
            MessageSerial::new(message).unwrap(),
            meta(0),
            match message {
                "msg:a" => 1,
                _ => 2,
            },
            0,
            Some("ai.response".into()),
            Some(MessageData::String("seed".into())),
            None,
        ),
    }
}

fn json(record: &StoredVersionRecord) -> Vec<u8> {
    sonic_rs::to_vec(record).unwrap()
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
    let (versioned, history, db) = configs(backend, prefix);
    let store = create_version_store(&versioned, &history, &db, &DatabasePooling::default())
        .await
        .unwrap();
    if std::env::var_os("C2_CHUNKED").is_some() {
        store.set_append_storage_enabled(true).await.unwrap();
    }
    store
}

fn prefix() -> String {
    format!("c2t{}", &uuid::Uuid::new_v4().simple().to_string()[..12])
}

fn backends() -> Vec<String> {
    std::env::var("C2_BACKENDS")
        .unwrap_or_else(|_| "postgres,mysql,dynamodb,scylladb,surrealdb".into())
        .split(',')
        .map(str::to_string)
        .collect()
}

fn mutation_request(
    message: &str,
    current: &StoredVersionRecord,
    n: u64,
    mutation: VersionMutation,
) -> VersionMutationRequest {
    VersionMutationRequest {
        app_id: APP.into(),
        channel: CHANNEL.into(),
        message_serial: MessageSerial::new(message).unwrap(),
        expected: VersionPrecondition::from_record(current),
        version: meta(n),
        mutation,
        idempotency: n.is_multiple_of(4).then(|| PublishIdempotencyMetadata {
            cache_key: format!("{message}-op-{n}"),
            payload_fingerprint: format!("fingerprint-{n}"),
        }),
        limits: VersionMutationLimits::default(),
    }
}

async fn applied(store: &dyn VersionStore, request: VersionMutationRequest) -> StoredVersionRecord {
    match store.compare_and_apply(request).await.unwrap() {
        VersionMutationResult::Applied { record, .. } => record,
        other => panic!("mutation not applied: {other:?}"),
    }
}

async fn versions(
    store: &dyn VersionStore,
    message: &str,
    direction: VersionStoreDirection,
    limit: usize,
) -> Vec<StoredVersionRecord> {
    let mut cursor = None;
    let mut items = Vec::new();
    loop {
        let page = store
            .get_versions(VersionStoreReadRequest {
                app_id: APP.into(),
                channel: CHANNEL.into(),
                message_serial: MessageSerial::new(message).unwrap(),
                direction,
                limit,
                cursor,
            })
            .await
            .unwrap();
        items.extend(page.items);
        cursor = page.next_cursor;
        if cursor.is_none() {
            return items;
        }
    }
}

async fn replay(store: &dyn VersionStore) -> Vec<StoredVersionRecord> {
    let mut after = 0;
    let mut items = Vec::new();
    loop {
        let page = store
            .replay_after(VersionReplayRequest {
                app_id: APP.into(),
                channel: CHANNEL.into(),
                after_delivery_serial: after,
                limit: 23,
            })
            .await
            .unwrap();
        let Some(last) = page.last() else {
            return items;
        };
        after = last.delivery_serial();
        items.extend(page);
    }
}

/// Two interleaved messages; appends are broken by an update and a delete
/// so each message has several runs; every fourth mutation is idempotent.
async fn write_workload(store: &dyn VersionStore) -> Vec<StoredVersionRecord> {
    let mut committed = Vec::new();
    let mut latest = std::collections::HashMap::new();
    for message in ["msg:a", "msg:b"] {
        let VersionCreateResult::Applied { record, .. } = store
            .commit_create(VersionCreateRequest {
                record: create(message),
                limits: VersionCreateLimits::default(),
            })
            .await
            .unwrap()
        else {
            panic!("create not applied");
        };
        committed.push(record.clone());
        latest.insert(message, record);
    }
    for n in 1..=90u64 {
        let message = if n % 3 == 0 { "msg:b" } else { "msg:a" };
        let current = latest[message].clone();
        let mutation = match n {
            30 => VersionMutation::Update(MessageFieldDelta {
                name: FieldPatch::Replace("renamed".into()),
                ..MessageFieldDelta::default()
            }),
            61 => VersionMutation::Delete(MessageFieldDelta {
                data: FieldPatch::Replace(MessageData::String("tomb".into())),
                ..MessageFieldDelta::default()
            }),
            _ => VersionMutation::Append(MessageAppend {
                data_fragment: fragment(n),
                extras: None,
            }),
        };
        let record = applied(store, mutation_request(message, &current, n, mutation)).await;
        committed.push(record.clone());
        latest.insert(message, record);
    }
    committed
}

async fn assert_reads(store: &dyn VersionStore, committed: &[StoredVersionRecord], label: &str) {
    for message in ["msg:a", "msg:b"] {
        let expected: Vec<_> = committed
            .iter()
            .filter(|record| record.message_serial().as_str() == message)
            .map(json)
            .collect();
        for limit in [1, 7, 100] {
            let oldest = versions(store, message, VersionStoreDirection::OldestFirst, limit).await;
            assert_eq!(
                oldest.iter().map(json).collect::<Vec<_>>(),
                expected,
                "{label}"
            );
            let mut newest =
                versions(store, message, VersionStoreDirection::NewestFirst, limit).await;
            newest.reverse();
            assert_eq!(
                newest.iter().map(json).collect::<Vec<_>>(),
                expected,
                "{label}"
            );
        }
        let latest = store
            .get_latest(APP, CHANNEL, &MessageSerial::new(message).unwrap())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(Some(json(&latest)), expected.last().cloned(), "{label}");
    }
    assert_eq!(
        replay(store).await.iter().map(json).collect::<Vec<_>>(),
        committed.iter().map(json).collect::<Vec<_>>(),
        "{label}"
    );
    let batch = store
        .get_latest_batch(
            APP,
            CHANNEL,
            &[
                MessageSerial::new("msg:a").unwrap(),
                MessageSerial::new("msg:b").unwrap(),
            ],
        )
        .await
        .unwrap();
    assert_eq!(batch.len(), 2, "{label}");
}

async fn raw_append_payload(backend: &str, prefix: &str) -> Vec<u8> {
    let version = serial(1);
    match backend {
        "postgres" => {
            let pool = sqlx::PgPool::connect("postgres://c2:c2-local-only@127.0.0.1:25471/c2")
                .await
                .unwrap();
            let sql = format!(
                "SELECT payload_bytes FROM {prefix}_version_entries WHERE message_serial = 'msg:a' AND version_serial = $1"
            );
            sqlx::query_scalar(sqlx::AssertSqlSafe(sql.as_str()))
                .bind(version.as_str())
                .fetch_one(&pool)
                .await
                .unwrap()
        }
        "mysql" => {
            let pool = sqlx::MySqlPool::connect("mysql://root:c2-local-only@127.0.0.1:25472/c2")
                .await
                .unwrap();
            let sql = format!(
                "SELECT payload_bytes FROM `{prefix}_version_entries` WHERE message_serial = 'msg:a' AND version_serial = ?"
            );
            sqlx::query_scalar(sqlx::AssertSqlSafe(sql.as_str()))
                .bind(version.as_str())
                .fetch_one(&pool)
                .await
                .unwrap()
        }
        "dynamodb" => {
            let client = dynamo_client().await;
            client
                .get_item()
                .table_name(format!("{prefix}_version_entries"))
                .key(
                    "app_channel",
                    aws_sdk_dynamodb::types::AttributeValue::S(format!("{APP}#{CHANNEL}")),
                )
                .key(
                    "message_version_key",
                    aws_sdk_dynamodb::types::AttributeValue::S(format!(
                        "msg:a#{}",
                        version.as_str()
                    )),
                )
                .send()
                .await
                .unwrap()
                .item
                .unwrap()["payload_bytes"]
                .as_b()
                .unwrap()
                .as_ref()
                .to_vec()
        }
        "scylladb" => {
            let session = ::scylla::client::session_builder::SessionBuilder::new()
                .known_node("127.0.0.1:25474")
                .build()
                .await
                .unwrap();
            let (payload,): (Vec<u8>,) = session
                .query_unpaged(
                    format!(
                        "SELECT payload_bytes FROM c2.{prefix}_version_commits WHERE app_id = ? AND channel = ? AND commit_key = ?"
                    ),
                    (APP, CHANNEL, format!("v:msg:a:{}", version.as_str())),
                )
                .await
                .unwrap()
                .into_rows_result()
                .unwrap()
                .single_row()
                .unwrap();
            payload
        }
        "surrealdb" => {
            let db = surreal_client().await;
            let mut response = db
                .query(format!(
                    "SELECT VALUE payload_bytes FROM {prefix}_version_entries WHERE message_serial = 'msg:a' AND version_serial = $version"
                ))
                .bind(("version", version.as_str().to_string()))
                .await
                .unwrap();
            let payloads: Vec<surrealdb::types::Value> = response.take(0).unwrap();
            match payloads.into_iter().next().unwrap() {
                surrealdb::types::Value::Bytes(bytes) => bytes.to_vec(),
                value => <Vec<u8> as surrealdb::types::SurrealValue>::from_value(value).unwrap(),
            }
        }
        other => panic!("unknown backend {other}"),
    }
}

async fn dynamo_client() -> aws_sdk_dynamodb::Client {
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
    aws_sdk_dynamodb::Client::new(&config)
}

async fn surreal_client() -> surrealdb::Surreal<surrealdb::engine::any::Any> {
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
    db
}

/// Pinned flags of the prefix's remaining run snapshots.
async fn run_rows(backend: &str, prefix: &str) -> Vec<bool> {
    match backend {
        "postgres" => {
            let pool = sqlx::PgPool::connect("postgres://c2:c2-local-only@127.0.0.1:25471/c2")
                .await
                .unwrap();
            let sql = format!("SELECT COUNT(*) FROM {prefix}_version_entries_runs");
            let count: i64 = sqlx::query_scalar(sqlx::AssertSqlSafe(sql.as_str()))
                .fetch_one(&pool)
                .await
                .unwrap();
            vec![false; count as usize]
        }
        "mysql" => {
            let pool = sqlx::MySqlPool::connect("mysql://root:c2-local-only@127.0.0.1:25472/c2")
                .await
                .unwrap();
            let sql = format!("SELECT COUNT(*) FROM `{prefix}_version_entries_runs`");
            let count: i64 = sqlx::query_scalar(sqlx::AssertSqlSafe(sql.as_str()))
                .fetch_one(&pool)
                .await
                .unwrap();
            vec![false; count as usize]
        }
        "surrealdb" => {
            let db = surreal_client().await;
            let mut response = db
                .query(format!(
                    "SELECT VALUE pinned FROM {prefix}_version_append_runs"
                ))
                .await
                .unwrap();
            response.take(0).unwrap()
        }
        other => panic!("run rows are not inspected for {other}"),
    }
}

/// Remove every append run snapshot of the prefix behind the store's back.
async fn drop_runs(backend: &str, prefix: &str) {
    match backend {
        "postgres" => {
            let pool = sqlx::PgPool::connect("postgres://c2:c2-local-only@127.0.0.1:25471/c2")
                .await
                .unwrap();
            let sql = format!("DELETE FROM {prefix}_version_entries_runs");
            sqlx::query(sqlx::AssertSqlSafe(sql.as_str()))
                .execute(&pool)
                .await
                .unwrap();
        }
        "mysql" => {
            let pool = sqlx::MySqlPool::connect("mysql://root:c2-local-only@127.0.0.1:25472/c2")
                .await
                .unwrap();
            let sql = format!("DELETE FROM `{prefix}_version_entries_runs`");
            sqlx::query(sqlx::AssertSqlSafe(sql.as_str()))
                .execute(&pool)
                .await
                .unwrap();
        }
        "dynamodb" => {
            let client = dynamo_client().await;
            let table = format!("{prefix}_version_entries");
            let items = client.scan().table_name(&table).send().await.unwrap();
            for item in items.items() {
                let key = item["message_version_key"].as_s().unwrap();
                if key.starts_with("__append_run__ ") || key.starts_with("__append_chunk__ ") {
                    client
                        .delete_item()
                        .table_name(&table)
                        .key("app_channel", item["app_channel"].clone())
                        .key("message_version_key", item["message_version_key"].clone())
                        .send()
                        .await
                        .unwrap();
                }
            }
        }
        "scylladb" => {
            let session = ::scylla::client::session_builder::SessionBuilder::new()
                .known_node("127.0.0.1:25474")
                .build()
                .await
                .unwrap();
            let keys: Vec<String> = session
                .query_unpaged(
                    format!(
                        "SELECT commit_key FROM c2.{prefix}_version_commits WHERE app_id = ? AND channel = ? AND commit_key >= 'c:' AND commit_key < 's'"
                    ),
                    (APP, CHANNEL),
                )
                .await
                .unwrap()
                .into_rows_result()
                .unwrap()
                .rows::<(String,)>()
                .unwrap()
                .map(|row| row.unwrap().0)
                .collect();
            for key in keys {
                if !key.starts_with("c:") && !key.starts_with("r:") {
                    continue;
                }
                session
                    .query_unpaged(
                        format!(
                            "DELETE FROM c2.{prefix}_version_commits WHERE app_id = ? AND channel = ? AND commit_key = ?"
                        ),
                        (APP, CHANNEL, key),
                    )
                    .await
                    .unwrap();
            }
        }
        "surrealdb" => {
            let db = surreal_client().await;
            db.query(format!(
                "DELETE {prefix}_version_append_runs; DELETE {prefix}_version_append_chunks"
            ))
            .await
            .unwrap()
            .check()
            .unwrap();
        }
        other => panic!("unknown backend {other}"),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires isolated C2 Compose services on ports 25471-25475"]
async fn c2_live_append_storage_contract() {
    for backend in backends() {
        let prefix = prefix();
        let store = open(&backend, &prefix).await;
        let committed = write_workload(store.as_ref()).await;
        assert_reads(store.as_ref(), &committed, &backend).await;

        // Restart: a fresh store reconstructs from persisted rows alone.
        let reopened = open(&backend, &prefix).await;
        assert_reads(reopened.as_ref(), &committed, &format!("{backend} restart")).await;

        // Every original operation is stored; only the aggregate is shared.
        let payload = raw_append_payload(&backend, &prefix).await;
        let StoredVersionPayload::Compact { run, record } =
            StoredVersionPayload::decode(&payload).unwrap()
        else {
            panic!("{backend}: append entries must be stored compactly");
        };
        assert_eq!(
            record.message.append_fragment.as_deref(),
            Some("<1:\u{e9}>")
        );
        assert_eq!(run.data_len, ("seed".len() + "<1:\u{e9}>".len()) as u64);
        assert!(
            sonic_rs::from_slice::<StoredVersionRecord>(&payload).is_err(),
            "{backend}: older releases must reject compact entries"
        );

        // Idempotent replays of compact appends return the original record,
        // including after restart.
        for n in [4u64, 8, 88] {
            let original = committed
                .iter()
                .find(|record| record.version_serial() == &serial(n))
                .unwrap();
            let message = original.message_serial().as_str().to_string();
            let predecessor = committed
                .iter()
                .filter(|record| record.message_serial().as_str() == message)
                .take_while(|record| record.version_serial() < &serial(n))
                .last()
                .unwrap();
            let request = mutation_request(
                &message,
                predecessor,
                n,
                VersionMutation::Append(MessageAppend {
                    data_fragment: fragment(n),
                    extras: None,
                }),
            );
            let VersionMutationResult::Duplicate { record, .. } =
                reopened.compare_and_apply(request).await.unwrap()
            else {
                panic!("{backend}: receipt {n} must replay");
            };
            assert_eq!(json(&record), json(original), "{backend} receipt {n}");
        }

        // A snapshot that disappeared behind the store's back fails closed.
        drop_runs(&backend, &prefix).await;
        let fresh = open(&backend, &prefix).await;
        let failed = fresh
            .get_versions(VersionStoreReadRequest {
                app_id: APP.into(),
                channel: CHANNEL.into(),
                message_serial: MessageSerial::new("msg:a").unwrap(),
                direction: VersionStoreDirection::OldestFirst,
                limit: 100,
                cursor: None,
            })
            .await;
        assert!(failed.is_err(), "{backend}: missing snapshot must not read");
        assert!(
            fresh
                .replay_after(VersionReplayRequest {
                    app_id: APP.into(),
                    channel: CHANNEL.into(),
                    after_delivery_serial: 0,
                    limit: 100,
                })
                .await
                .is_err(),
            "{backend}: missing snapshot must not replay"
        );
        println!("{backend},contract,ok");
        super::c2_bench::cleanup(&backend, &prefix).await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires isolated C2 Compose services on ports 25471-25475"]
async fn c2_live_concurrent_appends_keep_each_fragment_once() {
    for backend in backends() {
        let prefix = prefix();
        let first = open(&backend, &prefix).await;
        let second = open(&backend, &prefix).await;
        let VersionCreateResult::Applied { .. } = first
            .commit_create(VersionCreateRequest {
                record: create("msg:a"),
                limits: VersionCreateLimits::default(),
            })
            .await
            .unwrap()
        else {
            panic!("create not applied");
        };
        let counter = Arc::new(std::sync::atomic::AtomicU64::new(1));
        let mut tasks = Vec::new();
        for (worker, store) in [first.clone(), second.clone()].into_iter().enumerate() {
            let counter = Arc::clone(&counter);
            tasks.push(tokio::spawn(async move {
                for index in 0..20 {
                    let data_fragment = format!("[{worker}.{index}]");
                    loop {
                        let current = store
                            .get_latest(APP, CHANNEL, &MessageSerial::new("msg:a").unwrap())
                            .await
                            .unwrap()
                            .unwrap();
                        let n = counter.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                        let request = VersionMutationRequest {
                            idempotency: None,
                            ..mutation_request(
                                "msg:a",
                                &current,
                                n,
                                VersionMutation::Append(MessageAppend {
                                    data_fragment: data_fragment.clone(),
                                    extras: None,
                                }),
                            )
                        };
                        match store.compare_and_apply(request).await {
                            Ok(VersionMutationResult::Applied { .. }) => break,
                            Ok(VersionMutationResult::Conflict { .. }) => continue,
                            // A lost optimistic race may surface as a backend
                            // conflict error; the operation is retried.
                            Err(_) => continue,
                            Ok(other) => panic!("unexpected outcome {other:?}"),
                        }
                    }
                }
            }));
        }
        for task in tasks {
            task.await.unwrap();
        }
        let history = versions(
            first.as_ref(),
            "msg:a",
            VersionStoreDirection::OldestFirst,
            100,
        )
        .await;
        assert_eq!(history.len(), 41, "{backend}");
        let mut expected = String::from("seed");
        for record in &history[1..] {
            expected.push_str(record.message.append_fragment.as_deref().unwrap());
            assert_eq!(
                record
                    .message
                    .data
                    .clone()
                    .and_then(MessageData::into_string)
                    .as_deref(),
                Some(expected.as_str()),
                "{backend}"
            );
        }
        for worker in 0..2 {
            for index in 0..20 {
                assert_eq!(expected.matches(&format!("[{worker}.{index}]")).count(), 1);
            }
        }
        println!("{backend},concurrency,ok");
        super::c2_bench::cleanup(&backend, &prefix).await;
    }
}

#[tokio::test]
#[ignore = "requires isolated C2 SurrealDB service on port 25475"]
async fn c2_surreal_delivery_uniqueness_rejects_forks_and_corrupt_upgrade() {
    let prefix = prefix();
    let _store = open("surrealdb", &prefix).await;
    let db = surreal_client().await;
    let entries = format!("{prefix}_version_entries");
    let insert = |id: &str| {
        format!("CREATE {entries}:{id} SET app_id = 'test', channel = 'room', delivery_serial = 1;")
    };
    db.query(insert("first")).await.unwrap().check().unwrap();
    let error = db
        .query(insert("second"))
        .await
        .unwrap()
        .check()
        .unwrap_err();
    assert!(error.message().contains("_delivery_unique_v2"));
    assert!(error.message().contains("already contains"));
    // Legacy nodes classify the stable token as a CAS conflict too.
    assert!(error.message().contains("version_conflict"));
    // Simulate a legacy store with an already committed fork. Startup must not
    // swallow the index build failure and allow more writes.
    db.query(format!(
        "REMOVE INDEX {entries}_version_conflict_delivery_unique_v2 ON TABLE {entries};"
    ))
    .await
    .unwrap()
    .check()
    .unwrap();
    db.query(insert("second")).await.unwrap().check().unwrap();
    let (versioned, history, database) = configs("surrealdb", &prefix);
    // Repeating startup also verifies a failed index build did not leave a
    // partial index definition that IF NOT EXISTS would silently accept.
    for _ in 0..2 {
        let result =
            create_version_store(&versioned, &history, &database, &DatabasePooling::default())
                .await;
        match result {
            Err(error) => assert!(
                error
                    .to_string()
                    .contains("unique version delivery positions")
            ),
            Ok(_) => panic!("corrupt history was accepted"),
        }
    }
    super::c2_bench::cleanup("surrealdb", &prefix).await;
}

/// Strict regression: independently connected writers must never acknowledge
/// two versions at one channel position, even when they mutate different messages.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires isolated C2 SurrealDB service on port 25475"]
async fn c2_surreal_concurrent_receipts_and_channel_positions() {
    for compact in [false, true] {
        for same_message in [false, true] {
            let prefix = prefix();
            let first = open("surrealdb", &prefix).await;
            let second = open("surrealdb", &prefix).await;
            first.set_append_storage_enabled(compact).await.unwrap();
            for message in if same_message {
                vec!["msg:a"]
            } else {
                vec!["msg:a", "msg:b"]
            } {
                assert!(matches!(
                    first
                        .commit_create(VersionCreateRequest {
                            record: create(message),
                            limits: VersionCreateLimits::default(),
                        })
                        .await
                        .unwrap(),
                    VersionCreateResult::Applied { .. }
                ));
            }
            let barrier = Arc::new(tokio::sync::Barrier::new(2));
            let counter = Arc::new(std::sync::atomic::AtomicU64::new(1));
            let mut tasks = Vec::new();
            for (worker, store) in [first.clone(), second].into_iter().enumerate() {
                let barrier = barrier.clone();
                let counter = counter.clone();
                tasks.push(tokio::spawn(async move {
                    let message = if same_message || worker == 0 {
                        "msg:a"
                    } else {
                        "msg:b"
                    };
                    let mut applied_requests = Vec::new();
                    for index in 0..40 {
                        let fragment = format!("[{worker}.{index}]");
                        // Synchronize independent connections before each round.
                        tokio::time::timeout(std::time::Duration::from_secs(10), barrier.wait())
                            .await
                            .expect("concurrent writer failed before barrier");
                        let mut committed = false;
                        for _ in 0..100 {
                            let current = store
                                .get_latest(APP, CHANNEL, &MessageSerial::new(message).unwrap())
                                .await
                                .unwrap()
                                .unwrap();
                            let n = counter.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                            let mut request = mutation_request(
                                message,
                                &current,
                                n,
                                VersionMutation::Append(MessageAppend {
                                    data_fragment: fragment.clone(),
                                    extras: None,
                                }),
                            );
                            request.idempotency = Some(PublishIdempotencyMetadata {
                                cache_key: format!("race-{worker}-{index}"),
                                payload_fingerprint: format!("race-fingerprint-{worker}-{index}"),
                            });
                            match store.compare_and_apply(request.clone()).await.unwrap() {
                                VersionMutationResult::Applied { record, .. } => {
                                    applied_requests.push((request, record));
                                    committed = true;
                                    break;
                                }
                                VersionMutationResult::Conflict { .. } => {
                                    tokio::task::yield_now().await
                                }
                                other => panic!("unexpected outcome {other:?}"),
                            }
                        }
                        assert!(committed, "bounded caller retries exhausted");
                    }
                    applied_requests
                }));
            }
            let mut acknowledged = Vec::new();
            for task in tasks {
                acknowledged.extend(
                    tokio::time::timeout(std::time::Duration::from_secs(30), task)
                        .await
                        .expect("concurrent mutation task timed out")
                        .unwrap(),
                );
            }
            // A new connection also removes local-cache explanations for success.
            let restarted = open("surrealdb", &prefix).await;
            let mut records = Vec::new();
            for message in if same_message {
                vec!["msg:a"]
            } else {
                vec!["msg:a", "msg:b"]
            } {
                let history = versions(
                    restarted.as_ref(),
                    message,
                    VersionStoreDirection::OldestFirst,
                    11,
                )
                .await;
                assert_eq!(history.len(), if same_message { 81 } else { 41 });
                let mut aggregate = String::from("seed");
                let history_serial = history[0].message.identity.history_serial;
                for record in &history[1..] {
                    aggregate.push_str(record.message.append_fragment.as_deref().unwrap());
                    assert_eq!(
                        record
                            .message
                            .data
                            .clone()
                            .and_then(MessageData::into_string)
                            .as_deref(),
                        Some(aggregate.as_str())
                    );
                    assert_eq!(record.message.identity.history_serial, history_serial);
                    assert_eq!(record.message_serial().as_str(), message);
                }
                assert_eq!(
                    json(
                        &restarted
                            .get_latest(APP, CHANNEL, &MessageSerial::new(message).unwrap())
                            .await
                            .unwrap()
                            .unwrap()
                    ),
                    json(history.last().unwrap())
                );
                records.extend(history);
            }
            records.sort_by_key(StoredVersionRecord::delivery_serial);
            for (index, record) in records.iter().enumerate() {
                assert_eq!(record.delivery_serial(), index as u64 + 1);
            }
            for (request, expected) in acknowledged {
                assert_eq!(
                    records
                        .iter()
                        .filter(
                            |record| record.message_serial() == expected.message_serial()
                                && record.version_serial() == expected.version_serial()
                        )
                        .count(),
                    1
                );
                let VersionMutationResult::Duplicate { record, .. } =
                    restarted.compare_and_apply(request).await.unwrap()
                else {
                    panic!("receipt was not retained");
                };
                assert_eq!(json(&record), json(&expected));
            }
            let replayed = replay(restarted.as_ref()).await;
            assert_eq!(
                replayed.iter().map(json).collect::<Vec<_>>(),
                records.iter().map(json).collect::<Vec<_>>()
            );
            super::c2_bench::cleanup("surrealdb", &prefix).await;
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires isolated C2 Compose services on ports 25471-25475"]
async fn c2_live_purge_keeps_runs_until_their_entries_expire() {
    for backend in backends()
        .into_iter()
        .filter(|backend| matches!(backend.as_str(), "postgres" | "mysql" | "surrealdb"))
    {
        let prefix = prefix();
        let store = open(&backend, &prefix).await;
        let committed = write_workload(store.as_ref()).await;
        // Partial purge: the oldest entries go, retained versions still read.
        let (deleted, _) = store.purge_before(i64::MAX, 40).await.unwrap();
        assert!(deleted >= 40, "{backend}");
        let retained = replay(store.as_ref()).await;
        assert!(!retained.is_empty(), "{backend}");
        for record in &retained {
            let original = committed
                .iter()
                .find(|candidate| {
                    candidate.version_serial() == record.version_serial()
                        && candidate.message_serial() == record.message_serial()
                })
                .unwrap();
            assert_eq!(json(record), json(original), "{backend}");
        }
        // Full purge removes entries and then every unpinned run.
        loop {
            let (_, has_more) = store.purge_before(i64::MAX, 1000).await.unwrap();
            if !has_more {
                break;
            }
        }
        assert!(replay(store.as_ref()).await.is_empty(), "{backend}");
        let remaining = run_rows(&backend, &prefix).await;
        // Surreal keeps runs pinned by never-purged receipts.
        if backend == "surrealdb" {
            assert!(
                remaining.iter().all(|pinned| *pinned),
                "{backend}: {remaining:?}"
            );
        } else {
            assert!(remaining.is_empty(), "{backend}: {remaining:?}");
        }
        println!("{backend},purge,ok,remaining_runs,{}", remaining.len());
        super::c2_bench::cleanup(&backend, &prefix).await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires isolated C2 Compose DynamoDB on port 25473"]
async fn c2_live_dynamodb_run_ttl_covers_entries_and_pins_receipts() {
    let prefix = prefix();
    let store = open("dynamodb", &prefix).await;
    write_workload(store.as_ref()).await;
    let client = dynamo_client().await;
    let table = format!("{prefix}_version_entries");
    let items = client.scan().table_name(&table).send().await.unwrap();
    let mut max_entry_expiry = 0i64;
    let mut runs = Vec::new();
    for item in items.items() {
        let key = item["message_version_key"].as_s().unwrap();
        let expiry = item
            .get("expires_at")
            .and_then(|value| value.as_n().ok())
            .map(|value| value.parse::<i64>().unwrap());
        if key.starts_with("__append_chunk__ ") {
            assert!(expiry.is_none(), "{key}: chunks must remain pinned");
            continue;
        }
        if key.starts_with("__append_run__ ") {
            let pinned = *item["append_run_pinned"].as_bool().unwrap();
            runs.push((key.clone(), pinned, expiry));
        } else if !key.starts_with("__operation__") {
            max_entry_expiry = max_entry_expiry.max(expiry.unwrap());
        }
    }
    assert!(!runs.is_empty());
    for (key, pinned, expiry) in &runs {
        if *pinned {
            assert!(expiry.is_none(), "{key}: pinned runs never expire");
        } else {
            assert!(
                expiry.unwrap() >= max_entry_expiry - 1,
                "{key}: run expires before entries"
            );
        }
    }
    assert!(runs.iter().any(|(_, pinned, _)| *pinned));
    println!("dynamodb,ttl,ok,runs,{}", runs.len());
    super::c2_bench::cleanup("dynamodb", &prefix).await;
}

/// Rollback step of the mixed-release check: rewrite the compact rows a
/// candidate wrote under `C2_MIXED_PREFIX` so the older release can read them.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires isolated C2 Compose services and C2_MIXED_PREFIX"]
async fn c2_mixed_version_materialize() {
    let prefix = std::env::var("C2_MIXED_PREFIX").unwrap();
    for backend in backends() {
        let store = open(&backend, &prefix).await;
        store.set_append_storage_enabled(false).await.unwrap();
        let rewritten = store.materialize_append_storage(7).await.unwrap();
        // Idempotent: a second pass finds nothing left to rewrite.
        let again = store.materialize_append_storage(7).await.unwrap();
        println!("{backend},materialize,rewritten,{rewritten},second_pass,{again}");
        assert!(rewritten > 0, "{backend}: nothing was materialized");
        assert_eq!(again, 0, "{backend}");
    }
}

/// Diagnostic: race two SurrealDB writers and dump raw entries and run
/// records whenever a reconstruction fails.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "diagnostic; requires the isolated C2 SurrealDB"]
async fn c2_surreal_race_dump() {
    let trials: u32 = std::env::var("C2_TRIALS")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(50);
    for trial in 0..trials {
        let prefix = prefix();
        let stores = [
            open("surrealdb", &prefix).await,
            open("surrealdb", &prefix).await,
        ];
        let VersionCreateResult::Applied { .. } = stores[0]
            .commit_create(VersionCreateRequest {
                record: create("msg:a"),
                limits: VersionCreateLimits::default(),
            })
            .await
            .unwrap()
        else {
            panic!("create not applied");
        };
        let counter = Arc::new(std::sync::atomic::AtomicU64::new(1));
        let mut tasks = Vec::new();
        for (worker, store) in stores.iter().cloned().enumerate() {
            let counter = Arc::clone(&counter);
            tasks.push(tokio::spawn(async move {
                for index in 0..20 {
                    loop {
                        let Ok(Some(current)) = store
                            .get_latest(APP, CHANNEL, &MessageSerial::new("msg:a").unwrap())
                            .await
                        else {
                            continue;
                        };
                        let n = counter.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                        let request = VersionMutationRequest {
                            idempotency: None,
                            ..mutation_request(
                                "msg:a",
                                &current,
                                n,
                                VersionMutation::Append(MessageAppend {
                                    data_fragment: format!("[{worker}.{index}]"),
                                    extras: None,
                                }),
                            )
                        };
                        if let Ok(VersionMutationResult::Applied { .. }) =
                            store.compare_and_apply(request).await
                        {
                            break;
                        }
                    }
                }
            }));
        }
        for task in tasks {
            task.await.unwrap();
        }
        let failed = stores[0]
            .get_versions(VersionStoreReadRequest {
                app_id: APP.into(),
                channel: CHANNEL.into(),
                message_serial: MessageSerial::new("msg:a").unwrap(),
                direction: VersionStoreDirection::OldestFirst,
                limit: 100,
                cursor: None,
            })
            .await
            .err();
        if let Some(error) = failed {
            println!("trial {trial} failed: {error}");
            let db = surreal_client().await;
            let mut response = db
                .query(format!(
                    "SELECT version_serial, delivery_serial, created_at_ms, payload_bytes FROM {prefix}_version_entries ORDER BY version_serial"
                ))
                .await
                .unwrap();
            let rows: Vec<surrealdb_types::Value> = response.take(0).unwrap();
            for row in rows {
                let json = row.into_json_value();
                let bytes: Vec<u8> = json["payload_bytes"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|value| value.as_u64().unwrap() as u8)
                    .collect();
                let summary = match StoredVersionPayload::decode(&bytes).unwrap() {
                    StoredVersionPayload::Full(_) => "full".to_string(),
                    StoredVersionPayload::Compact { run, record } => format!(
                        "run={} len={} frag={} expected={}",
                        run.run.as_str(),
                        run.data_len,
                        record.message.append_fragment.unwrap_or_default(),
                        record.message.replay_position.delivery_serial
                    ),
                };
                println!(
                    "  entry {} delivery={} created={} {summary}",
                    json["version_serial"], json["delivery_serial"], json["created_at_ms"]
                );
            }
            let mut response = db
                .query(format!("SELECT * FROM {prefix}_version_append_runs"))
                .await
                .unwrap();
            let runs: Vec<surrealdb_types::Value> = response.take(0).unwrap();
            for run in runs {
                let json = run.into_json_value();
                println!(
                    "  run head={} len={} data={}",
                    json["head_version_serial"], json["data_len"], json["data"]
                );
            }
            let mut response = db
                .query(format!("SELECT * FROM {prefix}_version_streams"))
                .await
                .unwrap();
            let streams: Vec<surrealdb_types::Value> = response.take(0).unwrap();
            println!(
                "  streams {:?}",
                streams
                    .into_iter()
                    .map(|v| v.into_json_value())
                    .collect::<Vec<_>>()
            );
            return;
        }
        super::c2_bench::cleanup("surrealdb", &prefix).await;
    }
    println!("no failure in {trials} trials");
}

/// Format off -> on -> off -> materialize across independent store instances.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires isolated C2 Compose services on ports 25471-25475"]
async fn c2_chunked_rollout_boundaries_and_materialization() {
    for backend in backends() {
        let prefix = prefix();
        let store = open(&backend, &prefix).await;
        store.set_append_storage_enabled(false).await.unwrap();
        let second = open(&backend, &prefix).await;
        // Re-opening must not change the persisted marker (run without C2_CHUNKED).
        let mut seed = create("msg:a");
        let initial_data = "é".repeat(8192);
        seed.message.data = Some(MessageData::String(initial_data.clone()));
        seed.envelope.as_mut().unwrap().data = Some(MessageContent::Text(initial_data));
        let VersionCreateResult::Applied {
            record: mut current,
            ..
        } = store
            .commit_create(VersionCreateRequest {
                record: seed,
                limits: VersionCreateLimits::default(),
            })
            .await
            .unwrap()
        else {
            panic!("create not applied");
        };
        let mut committed = vec![current.clone()];
        let fragments = [
            "a".to_owned(),
            "x".repeat(4089),
            "🙂".to_owned(),
            "é".repeat(4096),
            "z".repeat(4096),
            "tail".to_owned(),
        ];
        for (index, fragment) in fragments.into_iter().enumerate() {
            if index == 1 {
                store.set_append_storage_enabled(true).await.unwrap();
            }
            if index == 5 {
                store.set_append_storage_enabled(false).await.unwrap();
            }
            let n = index as u64 + 1;
            let request = mutation_request(
                "msg:a",
                &current,
                n,
                VersionMutation::Append(MessageAppend {
                    data_fragment: fragment,
                    extras: None,
                }),
            );
            current = applied(second.as_ref(), request).await;
            committed.push(current.clone());
        }
        for direction in [
            VersionStoreDirection::OldestFirst,
            VersionStoreDirection::NewestFirst,
        ] {
            let rows = versions(second.as_ref(), "msg:a", direction, 2).await;
            let mut expected = committed.iter().map(json).collect::<Vec<_>>();
            if direction == VersionStoreDirection::NewestFirst {
                expected.reverse();
            }
            assert_eq!(
                rows.iter().map(json).collect::<Vec<_>>(),
                expected,
                "{backend}"
            );
        }
        assert_eq!(
            json(
                &second
                    .get_latest(APP, CHANNEL, current.message_serial())
                    .await
                    .unwrap()
                    .unwrap()
            ),
            json(&current)
        );
        let count = store.materialize_append_storage(2).await.unwrap();
        assert!(count > 0, "{backend}");
        assert_eq!(
            store.materialize_append_storage(2).await.unwrap(),
            0,
            "{backend}"
        );
        let reopened = open(&backend, &prefix).await;
        assert_eq!(
            versions(
                reopened.as_ref(),
                "msg:a",
                VersionStoreDirection::OldestFirst,
                2
            )
            .await
            .iter()
            .map(json)
            .collect::<Vec<_>>(),
            committed.iter().map(json).collect::<Vec<_>>(),
            "{backend}"
        );
        println!("{backend},chunked_rollout,ok,rewritten,{count}");
        super::c2_bench::cleanup(&backend, &prefix).await;
    }
}
