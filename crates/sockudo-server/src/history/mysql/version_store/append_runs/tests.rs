use super::*;
use sockudo_core::version_store::{VersionMutation, VersionMutationLimits, VersionPrecondition};
use sockudo_core::versioned_messages::{MessageAppend, VersionMetadata, VersionedMessage};
use sockudo_protocol::messages::MessageData;

fn metadata(n: u64) -> VersionMetadata {
    VersionMetadata {
        serial: VersionSerial::new(format!("ver:{n:020}")).unwrap(),
        client_id: None,
        timestamp_ms: n as i64,
        description: None,
        metadata: None,
    }
}

fn request(current: &StoredVersionRecord, n: u64, fragment: String) -> VersionMutationRequest {
    VersionMutationRequest {
        app_id: "chunks".into(),
        channel: "room".into(),
        message_serial: current.message_serial().clone(),
        expected: VersionPrecondition::from_record(current),
        version: metadata(n),
        mutation: VersionMutation::Append(MessageAppend {
            data_fragment: fragment,
            extras: None,
        }),
        idempotency: None,
        limits: VersionMutationLimits::default(),
    }
}

#[tokio::test]
#[ignore = "requires isolated C2 mysql service"]
async fn chunked_sql_rollout_abort_and_purge() {
    let prefix = format!("c2sql{}", &uuid::Uuid::new_v4().simple().to_string()[..10]);
    let config = DatabaseConnection {
        host: "127.0.0.1".into(),
        port: 25472,
        username: "root".into(),
        password: "c2-local-only".into(),
        database: "c2".into(),
        ..Default::default()
    };
    let store = MysqlVersionStore::new(&config, &DatabasePooling::default(), &prefix)
        .await
        .unwrap();
    let other = MysqlVersionStore::new(&config, &DatabasePooling::default(), &prefix)
        .await
        .unwrap();
    let create = StoredVersionRecord {
        app_id: "chunks".into(),
        channel: "room".into(),
        original_client_id: None,
        envelope: None,
        message: VersionedMessage::new_create(
            MessageSerial::new("msg:chunks").unwrap(),
            metadata(0),
            1,
            0,
            None,
            Some(MessageData::String(String::new())),
            None,
        ),
    };
    let VersionCreateResult::Applied {
        record: mut current,
        ..
    } = store
        .commit_create(VersionCreateRequest {
            record: create,
            limits: Default::default(),
        })
        .await
        .unwrap()
    else {
        panic!("create rejected");
    };
    // Fresh stores are compatible with old readers until the explicit marker transition.
    let VersionMutationResult::Applied { record, .. } = store
        .compare_and_apply(request(&current, 1, "x".into()))
        .await
        .unwrap()
    else {
        panic!("append rejected");
    };
    current = record;
    let raw_sql = format!(
        "SELECT payload_bytes FROM `{}` WHERE version_serial = ?",
        store.tables.version_entries
    );
    let raw: Vec<u8> = sqlx::query_scalar(sqlx::AssertSqlSafe(raw_sql.as_str()))
        .bind(current.version_serial().as_str())
        .fetch_one(&store.pool)
        .await
        .unwrap();
    assert!(matches!(
        StoredVersionPayload::decode(&raw).unwrap(),
        StoredVersionPayload::Full(_)
    ));
    // Reproduce the exact format-1 collision: the latest row is the first
    // and only append in its run. Activation must replace it atomically.
    let legacy = AppendRunPlan::for_record(&metadata(0).serial, None, &current);
    let legacy_payload = legacy.encode(&current).unwrap();
    let mut tx = store.pool.begin().await.unwrap();
    let mut insert = sqlx::QueryBuilder::<sqlx::MySql>::new(format!(
        "INSERT INTO `{}` (app_id, channel, message_serial, run_version_serial, head_version_serial, data_bytes, data_len, created_at_ms, updated_at_ms) VALUES (",
        store.append_runs_table()
    ));
    insert
        .push_bind("chunks")
        .push(", ")
        .push_bind("room")
        .push(", ")
        .push_bind(current.message_serial().as_str())
        .push(", ")
        .push_bind(current.version_serial().as_str())
        .push(", ")
        .push_bind(current.version_serial().as_str())
        .push(", ")
        .push_bind(b"x".as_slice())
        .push(", 1, 1, 1)");
    insert.build().execute(&mut *tx).await.unwrap();
    let mut update = sqlx::QueryBuilder::<sqlx::MySql>::new(format!(
        "UPDATE `{}` SET payload_bytes = ",
        store.tables.version_entries
    ));
    update
        .push_bind(&legacy_payload)
        .push(", append_run = ")
        .push_bind(current.version_serial().as_str())
        .push(" WHERE version_serial = ")
        .push_bind(current.version_serial().as_str());
    update.build().execute(&mut *tx).await.unwrap();
    tx.commit().await.unwrap();
    assert_eq!(
        other
            .get_latest("chunks", "room", current.message_serial())
            .await
            .unwrap()
            .unwrap()
            .message
            .data,
        current.message.data
    );
    store.set_append_storage_enabled(true).await.unwrap();
    let mut expected = vec![current.clone()];
    // The first chunk ends in the middle of a four-byte character; subsequent
    // fragments span several chunks and finish exactly at a chunk boundary.
    for (n, fragment) in [
        (2, format!("{}🙂", "a".repeat(CHUNK_BYTES - 3))),
        (3, "é".repeat(CHUNK_BYTES)),
        (4, "b".repeat(CHUNK_BYTES - 2)),
    ] {
        let VersionMutationResult::Applied { record, .. } = other
            .compare_and_apply(request(&current, n, fragment))
            .await
            .unwrap()
        else {
            panic!("append rejected");
        };
        current = record;
        expected.push(current.clone());
    }
    let raw: Vec<u8> = sqlx::query_scalar(sqlx::AssertSqlSafe(raw_sql.as_str()))
        .bind(current.version_serial().as_str())
        .fetch_one(&store.pool)
        .await
        .unwrap();
    let payload = StoredVersionPayload::decode(&raw).unwrap();
    let run = payload.run().unwrap();
    assert!(run.generation.is_some());
    assert_eq!(
        run.run,
        metadata(1).serial,
        "first append must extend the maintenance seed"
    );
    assert_eq!(run.data_len as usize % CHUNK_BYTES, 0);
    for direction in [
        VersionStoreDirection::OldestFirst,
        VersionStoreDirection::NewestFirst,
    ] {
        let page = store
            .get_versions(VersionStoreReadRequest {
                app_id: "chunks".into(),
                channel: "room".into(),
                message_serial: current.message_serial().clone(),
                direction,
                limit: 20,
                cursor: None,
            })
            .await
            .unwrap();
        for original in &expected {
            let returned = page
                .items
                .iter()
                .find(|item| item.version_serial() == original.version_serial())
                .unwrap();
            assert_eq!(
                sonic_rs::to_vec(returned).unwrap(),
                sonic_rs::to_vec(original).unwrap()
            );
        }
    }
    // An aborted metadata/tail write is invisible to another store instance.
    let VersionMutationResult::Applied {
        record: aborted, ..
    } = request(&current, 5, "abort🙂".into())
        .apply_to(&current, "chunks/room", current.delivery_serial() + 1, 0)
        .unwrap()
    else {
        panic!("apply rejected");
    };
    let plan = AppendRunPlan::for_record_chunked(current.version_serial(), Some(run), &aborted);
    let mut tx = store.pool.begin().await.unwrap();
    store
        .write_append_run(&mut tx, &aborted, &plan, sockudo_core::history::now_ms())
        .await
        .unwrap();
    tx.rollback().await.unwrap();
    assert_eq!(
        sonic_rs::to_vec(
            &other
                .get_latest("chunks", "room", current.message_serial())
                .await
                .unwrap()
                .unwrap()
        )
        .unwrap(),
        sonic_rs::to_vec(&current).unwrap()
    );
    // A marker update cannot pass an in-flight compact-writing transaction.
    let mut tx = store.pool.begin().await.unwrap();
    assert!(store.append_storage_enabled(&mut tx).await.unwrap());
    assert!(
        tokio::time::timeout(
            std::time::Duration::from_millis(50),
            other.set_append_storage_enabled(false)
        )
        .await
        .is_err()
    );
    tx.rollback().await.unwrap();
    other.set_append_storage_enabled(false).await.unwrap();
    assert_eq!(store.materialize_append_storage(2).await.unwrap(), 3);
    assert_eq!(store.materialize_append_storage(2).await.unwrap(), 0);
    for original in &expected {
        let raw: Vec<u8> = sqlx::query_scalar(sqlx::AssertSqlSafe(raw_sql.as_str()))
            .bind(original.version_serial().as_str())
            .fetch_one(&store.pool)
            .await
            .unwrap();
        assert_eq!(
            sonic_rs::from_slice::<StoredVersionRecord>(&raw)
                .unwrap()
                .message
                .data,
            original.message.data
        );
    }
    // Imported full records also seed their data before any subsequent append.
    store.set_append_storage_enabled(true).await.unwrap();
    let imported = StoredVersionRecord {
        app_id: "chunks".into(),
        channel: "room".into(),
        original_client_id: None,
        envelope: None,
        message: VersionedMessage::new_create(
            MessageSerial::new("msg:imported").unwrap(),
            metadata(20),
            2,
            100,
            None,
            Some(MessageData::String("z".repeat(CHUNK_BYTES * 300 + 9))),
            None,
        ),
    };
    store.append_version(imported.clone()).await.unwrap();
    let VersionMutationResult::Applied {
        record: imported_append,
        ..
    } = store
        .compare_and_apply(request(&imported, 21, "tail".into()))
        .await
        .unwrap()
    else {
        panic!("import append rejected");
    };
    let raw: Vec<u8> = sqlx::query_scalar(sqlx::AssertSqlSafe(raw_sql.as_str()))
        .bind(imported_append.version_serial().as_str())
        .fetch_one(&store.pool)
        .await
        .unwrap();
    let imported_payload = StoredVersionPayload::decode(&raw).unwrap();
    assert_eq!(
        imported_payload.run().unwrap().run,
        imported.version_serial().clone()
    );
    assert_eq!(
        other
            .get_latest("chunks", "room", imported.message_serial())
            .await
            .unwrap()
            .unwrap()
            .message
            .data,
        imported_append.message.data
    );
    loop {
        let (_, more) = store.purge_before(i64::MAX, 100).await.unwrap();
        if !more {
            break;
        }
    }
    let sql = format!("SELECT COUNT(*) FROM `{}`", store.append_chunks_table());
    let count: i64 = sqlx::query_scalar(sqlx::AssertSqlSafe(sql))
        .fetch_one(&store.pool)
        .await
        .unwrap();
    assert_eq!(count, 0);
    crate::history::c2_bench::cleanup("mysql", &prefix).await;
}
