use super::*;
use sockudo_core::version_store::append_storage::AppendRunPlan;
use surrealdb::types::{Bytes, SurrealValue, Value};

/// Only definite aborted writes are retried. Connection/time-out errors have
/// ambiguous commit outcomes and must not replay an operation without a receipt.
pub(super) fn is_write_conflict(error: &surrealdb::Error) -> bool {
    use surrealdb::types::{ErrorDetails, QueryError};
    let message = error.message();
    matches!(error.details(), ErrorDetails::Query(Some(QueryError::TransactionConflict)))
        || error.is_already_exists()
        || message.contains("version_conflict")
        // SurrealDB 3.0 servers do not send the newer structured error kind.
        || (message.contains("Database index")
            && message.contains("_delivery_unique_v2")
            && message.contains("already contains"))
        || (message.starts_with("Database record")
            && (message.contains("already exists") || message.contains("already been created")))
        || message.contains("transaction can be retried")
}

/// An explicit transaction can put `NotExecuted` placeholders before its
/// actual failure. Inspect every statement so that rollback does not hide the
/// conflict which caused it (notably on SurrealDB 3.2).
pub(super) fn check_transaction(
    mut response: surrealdb::IndexedResults,
) -> std::result::Result<(), surrealdb::Error> {
    let mut errors: Vec<_> = response.take_errors().into_iter().collect();
    errors.sort_by_key(|(index, _)| *index);
    let mut placeholder = None;
    for (_, error) in errors {
        if error.query_details() == Some(&surrealdb::types::QueryError::NotExecuted)
            || error.message() == "The query was not executed due to a failed transaction"
        {
            placeholder = Some(error);
        } else {
            return Err(error);
        }
    }
    placeholder.map_or(Ok(()), Err)
}

impl SurrealVersionStore {
    /// Immutable function ABI. Any change to arguments, schema assumptions or
    /// function semantics requires a new suffix; old nodes may still call v1.
    /// Version 2 requires the unique delivery index.
    fn mutation_function(&self) -> String {
        format!("{}_mutate_v2", self.tables.entries)
    }

    pub(super) async fn ensure_mutation_function(&self) -> Result<()> {
        let sql = include_str!("mutation_v2.surql")
            .replace("__KEY_FN__", &format!("{}_key_v1", self.tables.entries))
            .replace("__MUTATE_FN__", &self.mutation_function())
            .replace("__FORMAT__", &self.tables.format)
            .replace("__STREAMS__", &self.tables.streams)
            .replace("__MESSAGES__", &self.tables.messages)
            .replace("__ENTRIES__", &self.tables.entries)
            .replace("__RUNS__", &self.tables.runs)
            .replace("__CHUNKS__", &self.tables.chunks)
            .replace("__RECEIPTS__", &self.tables.receipts);
        self.db
            .query(sql)
            .await
            .and_then(|response| response.check())
            .map_err(|e| {
                Error::Internal(format!(
                    "failed to initialize version mutation function: {e}"
                ))
            })?;
        Ok(())
    }

    /// Keep the existing explicit transaction boundary; only the immutable
    /// statement body lives on the server instead of crossing the wire again.
    pub(super) async fn commit_mutation_function(
        &self,
        args: Vec<Value>,
    ) -> std::result::Result<(), surrealdb::Error> {
        let response = self
            .db
            .query(format!(
                "BEGIN TRANSACTION; RETURN fn::{}($a); COMMIT TRANSACTION;",
                self.mutation_function()
            ))
            .bind(("a", args))
            .await?;
        check_transaction(response)
    }
}

/// Positional arguments keep transport metadata bounded and avoid serializing
/// the same identities into entry, latest-state, run and receipt arguments.
/// The order is the immutable `mutation_v2.surql` ABI.
pub(super) fn mutation_arguments(
    record: &StoredVersionRecord,
    payload: Vec<u8>,
    current: &StoredVersionRecord,
    stream: &StoredVersionStreamRec,
    format: &append_runs::StoredAppendFormat,
    plan: &AppendRunPlan,
    state: MutationState<'_>,
) -> Result<Vec<Value>> {
    let native = payload.starts_with(br#"{"sockudo_append_storage":2,"#);
    let run = plan.run().map(|run| {
        vec![
            run.run.as_str().to_string().into_value(),
            run.generation.clone().into_value(),
            (run.data_len as i64).into_value(),
            i64::from(matches!(plan, AppendRunPlan::Start { .. })).into_value(),
            match plan {
                AppendRunPlan::Extend { expected_len, .. } => (*expected_len as i64).into_value(),
                _ => 0i64.into_value(),
            },
            state.run_pinned.into_value(),
        ]
    });
    let chunks: Vec<Vec<Value>> = plan
        .chunk_writes(record)?
        .into_iter()
        .map(|chunk| {
            vec![
                (chunk.index as i64).into_value(),
                Bytes::from(chunk.bytes).into_value(),
            ]
        })
        .collect();
    Ok(vec![
        record.app_id.clone().into_value(),
        record.channel.clone().into_value(),
        record.message_serial().as_str().to_string().into_value(),
        record.version_serial().as_str().to_string().into_value(),
        (record.delivery_serial() as i64).into_value(),
        Bytes::from(payload).into_value(),
        state.now_ms.into_value(),
        current.version_serial().as_str().to_string().into_value(),
        stream.next_delivery_serial.into_value(),
        stream.open_stream_count.into_value(),
        state.next_open.into_value(),
        state.next_append.into_value(),
        record.is_open_ai_stream().into_value(),
        format.epoch.into_value(),
        format.enabled.into_value(),
        run.into_value(),
        chunks.into_value(),
        state
            .receipt
            .map(|receipt| {
                vec![
                    receipt.cache_key.clone(),
                    receipt.payload_fingerprint.clone(),
                ]
            })
            .into_value(),
        native.into_value(),
    ])
}

pub(super) struct MutationState<'a> {
    pub receipt: Option<&'a sockudo_core::message_envelope::PublishIdempotencyMetadata>,
    pub now_ms: i64,
    pub next_open: i64,
    pub next_append: i64,
    pub run_pinned: bool,
}

#[cfg(test)]
mod tests {
    use super::*;
    use sockudo_core::version_store::append_storage::{AppendRunRef, encode_full};
    use sockudo_core::version_store::{
        VersionCreateLimits, VersionMutation, VersionMutationLimits, VersionPrecondition,
    };
    use sockudo_core::versioned_messages::{
        MessageAppend, VersionMetadata, VersionSerial, VersionedMessage,
    };
    use sockudo_protocol::messages::MessageData;

    #[test]
    fn retries_only_definite_write_conflicts() {
        use surrealdb::types::QueryError;
        assert!(is_write_conflict(&surrealdb::Error::query(
            "conflict".into(),
            Some(QueryError::TransactionConflict)
        )));
        assert!(is_write_conflict(&surrealdb::Error::internal(
            "Database index `entries_delivery_unique_v2` already contains position".into()
        )));
        assert!(!is_write_conflict(&surrealdb::Error::internal(
            "request timed out".into()
        )));
        assert!(!is_write_conflict(&surrealdb::Error::internal(
            "connection closed".into()
        )));
        assert!(!is_write_conflict(&surrealdb::Error::internal(
            "Database index `unrelated` already contains value".into()
        )));
    }

    #[tokio::test]
    #[ignore = "requires isolated C2 SurrealDB service on port 25475"]
    async fn surreal_function_abort_rolls_back_stream_entry_and_utf8_keys() {
        let settings = SurrealDbSettings {
            url: "ws://127.0.0.1:25475".into(),
            namespace: "c2".into(),
            database: "c2".into(),
            password: "c2-local-only".into(),
            ..Default::default()
        };
        let prefix = format!("c2fn{}", uuid::Uuid::new_v4().simple());
        let _initialized = create_surreal_version_store(&settings, &prefix)
            .await
            .unwrap();
        let db = connect(settings.url.as_str()).await.unwrap();
        db.signin(Root {
            username: settings.username,
            password: settings.password,
        })
        .await
        .unwrap();
        db.use_ns(settings.namespace)
            .use_db(settings.database)
            .await
            .unwrap();
        let store = SurrealVersionStore {
            db,
            tables: VersionStoreTables {
                streams: format!("{prefix}_version_streams"),
                messages: format!("{prefix}_version_messages"),
                entries: format!("{prefix}_version_entries"),
                receipts: format!("{prefix}_version_receipts"),
                runs: format!("{prefix}_version_append_runs"),
                chunks: format!("{prefix}_version_append_chunks"),
                format: format!("{prefix}_version_append_format"),
            },
            append_cache: Default::default(),
        };
        store.set_append_storage_enabled(true).await.unwrap();
        let metadata = |n| VersionMetadata {
            serial: VersionSerial::new(format!("ver:{n:020}")).unwrap(),
            client_id: None,
            timestamp_ms: n,
            description: None,
            metadata: None,
        };
        let initial = StoredVersionRecord {
            app_id: "app:é".into(),
            channel: "channel:寧".into(),
            original_client_id: None,
            envelope: None,
            message: VersionedMessage::new_create(
                MessageSerial::new("msg:🦀").unwrap(),
                metadata(0),
                1,
                0,
                None,
                Some(MessageData::String("seed".into())),
                None,
            ),
        };
        let VersionCreateResult::Applied {
            record: current,
            stream_id,
        } = store
            .commit_create(VersionCreateRequest {
                record: initial,
                limits: VersionCreateLimits::default(),
            })
            .await
            .unwrap()
        else {
            panic!("create failed");
        };
        let stream_key =
            deterministic_key([current.app_id.as_str(), current.channel.as_str()].into_iter());
        let message_key = deterministic_key(
            [
                current.app_id.as_str(),
                current.channel.as_str(),
                current.message_serial().as_str(),
            ]
            .into_iter(),
        );
        let stream: StoredVersionStreamRec = store
            .db
            .select::<Option<StoredVersionStreamRec>>((
                store.tables.streams.clone(),
                stream_key.clone(),
            ))
            .await
            .unwrap()
            .unwrap();
        let message: StoredVersionMessageRec = store
            .db
            .select::<Option<StoredVersionMessageRec>>((store.tables.messages.clone(), message_key))
            .await
            .unwrap()
            .unwrap();
        let request = VersionMutationRequest {
            app_id: current.app_id.clone(),
            channel: current.channel.clone(),
            message_serial: current.message_serial().clone(),
            expected: VersionPrecondition::from_record(&current),
            version: metadata(1),
            mutation: VersionMutation::Append(MessageAppend {
                data_fragment: "tail".into(),
                extras: None,
            }),
            idempotency: None,
            limits: VersionMutationLimits::default(),
        };
        let VersionMutationResult::Applied { record, .. } = request
            .apply_to(&current, &stream_id, stream.next_delivery_serial as u64, 0)
            .unwrap()
        else {
            panic!("apply failed");
        };
        let predecessor = AppendRunRef {
            run: VersionSerial::new(message.latest_append_run.unwrap()).unwrap(),
            data_len: message.latest_append_len.unwrap() as u64,
            generation: message.latest_append_generation,
        };
        let mut plan = AppendRunPlan::for_record_chunked(
            current.version_serial(),
            Some(&predecessor),
            &record,
        );
        let AppendRunPlan::Extend { expected_len, .. } = &mut plan else {
            panic!("seed was not extended");
        };
        *expected_len += 1; // Run guard executes after the stream, message and entry writes.
        let format = store.append_format().await.unwrap();
        let args = mutation_arguments(
            &record,
            plan.encode(&record).unwrap(),
            &current,
            &stream,
            &format,
            &plan,
            MutationState {
                now_ms: sockudo_core::history::now_ms(),
                next_open: 0,
                next_append: 1,
                run_pinned: false,
                receipt: None,
            },
        )
        .unwrap();
        let failure = store.commit_mutation_function(args).await.unwrap_err();
        assert!(failure.to_string().contains("version_conflict"));
        let after = store
            .get_latest(&current.app_id, &current.channel, current.message_serial())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(encode_full(&after).unwrap(), encode_full(&current).unwrap());
        let after_stream: StoredVersionStreamRec = store
            .db
            .select::<Option<StoredVersionStreamRec>>((store.tables.streams.clone(), stream_key))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            after_stream.next_delivery_serial,
            stream.next_delivery_serial
        );
        let mut entries = store
            .db
            .query(format!(
                "SELECT VALUE version_serial FROM {}",
                store.tables.entries
            ))
            .await
            .unwrap();
        let entries: Vec<String> = entries.take(0usize).unwrap();
        assert_eq!(entries, [current.version_serial().as_str()]);
        // The regular path must address the same UTF-8 keys generated by Rust.
        let VersionMutationResult::Applied {
            record: applied, ..
        } = store.compare_and_apply(request).await.unwrap()
        else {
            panic!("append failed");
        };
        assert_eq!(
            encode_full(&applied).unwrap(),
            encode_full(&record).unwrap()
        );
        store.set_append_storage_enabled(false).await.unwrap();
        assert!(store.materialize_append_storage(100).await.unwrap() > 0);
        assert_eq!(store.materialize_append_storage(100).await.unwrap(), 0);
        for table in [
            &store.tables.streams,
            &store.tables.messages,
            &store.tables.entries,
            &store.tables.receipts,
            &store.tables.runs,
            &store.tables.chunks,
            &store.tables.format,
        ] {
            store
                .db
                .query(format!("REMOVE TABLE {table}"))
                .await
                .unwrap()
                .check()
                .unwrap();
        }
        for suffix in ["key_v1", "mutate_v2"] {
            store
                .db
                .query(format!(
                    "REMOVE FUNCTION fn::{}_{}",
                    store.tables.entries, suffix
                ))
                .await
                .unwrap()
                .check()
                .unwrap();
        }
    }
}
