//! Bounded GC for pinned chunks. A durable tombstone fences extensions before
//! any chunk is removed and makes interrupted deletion resumable.
use super::*;
use sockudo_core::version_store::append_storage::{StoredVersionPayload, is_compact};
use std::sync::atomic::Ordering;

type Key = HashMap<String, AttributeValue>;

#[derive(Default)]
pub(super) struct AppendGcCursor {
    scan: Option<Key>,
    candidate: Option<Candidate>,
    pending: std::collections::VecDeque<Candidate>,
}

struct Candidate {
    partition: String,
    manifest: String,
    message: String,
    run: String,
    generation: String,
    head: String,
    pending_seed: bool,
    position: Option<Key>,
    phase: Phase,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Phase {
    References,
    Latest,
    Fence,
    Chunks,
}

impl DynamoDbVersionStore {
    pub(super) async fn purge_append_chunks(&self, batch_size: usize) -> Result<(u64, bool)> {
        if self
            .append_gc_running
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return Ok((0, false));
        }
        struct Running<'a>(&'a std::sync::atomic::AtomicBool);
        impl Drop for Running<'_> {
            fn drop(&mut self) {
                self.0.store(false, Ordering::Release);
            }
        }
        let _running = Running(&self.append_gc_running);
        let mut cursor = std::mem::take(
            &mut *self
                .append_gc
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner),
        );
        let result = self
            .purge_append_chunk_page(&mut cursor, batch_size.clamp(1, 100))
            .await;
        *self
            .append_gc
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = cursor;
        result
    }

    async fn purge_append_chunk_page(
        &self,
        cursor: &mut AppendGcCursor,
        limit: usize,
    ) -> Result<(u64, bool)> {
        if cursor.candidate.is_none() {
            cursor.candidate = cursor.pending.pop_front();
        }
        let Some(candidate) = cursor.candidate.as_mut() else {
            let page = self.client.scan().table_name(&self.tables.version_entries)
                .limit(limit as i32).consistent_read(true).set_exclusive_start_key(cursor.scan.take())
                .projection_expression("app_channel, message_version_key, head_version_serial, append_generation, append_pending, garbage_collecting, created_at_ms")
                .send().await.map_err(|e| Error::Internal(format!("failed to scan append garbage: {e}")))?;
            cursor.scan = page.last_evaluated_key().cloned();
            for item in page.items() {
                let Some(generation) = Self::item_str(item, "append_generation") else {
                    continue;
                };
                let pending_seed = item.get("append_pending") == Some(&AttributeValue::Bool(true));
                let deleting = item.get("garbage_collecting") == Some(&AttributeValue::Bool(true));
                if pending_seed
                    && !deleting
                    && Self::item_num(item, "created_at_ms").is_none_or(|created| {
                        created > sockudo_core::history::now_ms().saturating_sub(5 * 60 * 1000)
                    })
                {
                    continue;
                }
                let Some(manifest) = Self::item_str(item, "message_version_key") else {
                    continue;
                };
                let fields = manifest.split_whitespace().collect::<Vec<_>>();
                if fields.len() != 4 || fields[0] != "__append_run__" || fields[3] != generation {
                    continue;
                }
                let Some(partition) = Self::item_str(item, "app_channel") else {
                    continue;
                };
                let Some(head) = Self::item_str(item, "head_version_serial") else {
                    continue;
                };
                cursor.pending.push_back(Candidate {
                    partition,
                    message: fields[1].to_string(),
                    run: fields[2].to_string(),
                    manifest,
                    generation,
                    head,
                    pending_seed,
                    position: None,
                    phase: if deleting {
                        Phase::Chunks
                    } else {
                        Phase::References
                    },
                });
            }
            cursor.candidate = cursor.pending.pop_front();
            return Ok((
                0,
                cursor.candidate.is_some() || cursor.scan.is_some() || !cursor.pending.is_empty(),
            ));
        };
        match candidate.phase {
            Phase::References => {
                let page = self
                    .client
                    .query()
                    .table_name(&self.tables.version_entries)
                    .key_condition_expression("app_channel = :partition")
                    .expression_attribute_values(":partition", Self::attr_s(&candidate.partition))
                    .projection_expression("message_version_key, payload_bytes")
                    .consistent_read(true)
                    .limit(limit as i32)
                    .set_exclusive_start_key(candidate.position.take())
                    .send()
                    .await
                    .map_err(|e| {
                        Error::Internal(format!("failed to inspect append references: {e}"))
                    })?;
                for item in page.items() {
                    let key = Self::item_str(item, "message_version_key").unwrap_or_default();
                    if key.starts_with("__append_run__ ") || key.starts_with("__append_chunk__ ") {
                        continue;
                    }
                    let Some(bytes) = item
                        .get("payload_bytes")
                        .and_then(|value| value.as_b().ok())
                    else {
                        continue;
                    };
                    if Self::references_candidate(bytes.as_ref(), candidate)? {
                        cursor.candidate = None;
                        return Ok((0, cursor.scan.is_some() || !cursor.pending.is_empty()));
                    }
                }
                candidate.position = page.last_evaluated_key().cloned();
                if candidate.position.is_none() {
                    candidate.phase = Phase::Latest;
                }
            }
            Phase::Latest => {
                let response = self
                    .client
                    .get_item()
                    .table_name(&self.tables.version_messages)
                    .key("app_channel", Self::attr_s(&candidate.partition))
                    .key("message_serial", Self::attr_s(&candidate.message))
                    .consistent_read(true)
                    .send()
                    .await
                    .map_err(|e| {
                        Error::Internal(format!("failed to inspect latest append reference: {e}"))
                    })?;
                if let Some(item) = response.item {
                    let seeded = Self::item_str(&item, "latest_append_run").as_deref()
                        == Some(candidate.run.as_str())
                        && Self::item_str(&item, "latest_append_generation").as_deref()
                            == Some(candidate.generation.as_str());
                    let payload = item
                        .get("latest_payload_bytes")
                        .and_then(|value| value.as_b().ok());
                    let references = match payload {
                        Some(bytes) => Self::references_candidate(bytes.as_ref(), candidate)?,
                        None => false,
                    };
                    if seeded || references {
                        cursor.candidate = None;
                        return Ok((0, cursor.scan.is_some() || !cursor.pending.is_empty()));
                    }
                }
                candidate.phase = Phase::Fence;
            }
            Phase::Fence => {
                let result = self.client.update_item().table_name(&self.tables.version_entries)
                    .key("app_channel", Self::attr_s(&candidate.partition)).key("message_version_key", Self::attr_s(&candidate.manifest))
                    .update_expression("SET garbage_collecting = :yes")
                    .condition_expression(if candidate.pending_seed {
                        "head_version_serial = :head AND append_generation = :generation AND append_pending = :pending"
                    } else {
                        "head_version_serial = :head AND append_generation = :generation AND (attribute_not_exists(append_pending) OR append_pending = :pending)"
                    })
                    .expression_attribute_values(":yes", AttributeValue::Bool(true)).expression_attribute_values(":pending", AttributeValue::Bool(candidate.pending_seed))
                    .expression_attribute_values(":head", Self::attr_s(&candidate.head)).expression_attribute_values(":generation", Self::attr_s(&candidate.generation))
                    .send().await;
                match result {
                    Ok(_) => candidate.phase = Phase::Chunks,
                    Err(error)
                        if error
                            .as_service_error()
                            .is_some_and(|error| error.is_conditional_check_failed_exception()) =>
                    {
                        tracing::debug!(error = %error, "append run changed during garbage collection");
                        cursor.candidate = None;
                        return Ok((0, cursor.scan.is_some() || !cursor.pending.is_empty()));
                    }
                    Err(error) => {
                        return Err(Error::Internal(format!(
                            "failed to fence append garbage: {error}"
                        )));
                    }
                }
            }
            Phase::Chunks => {
                let prefix = format!(
                    "__append_chunk__ {} {} {} ",
                    candidate.message, candidate.run, candidate.generation
                );
                let page = self
                    .client
                    .query()
                    .table_name(&self.tables.version_entries)
                    .key_condition_expression(
                        "app_channel = :partition AND begins_with(message_version_key, :prefix)",
                    )
                    .expression_attribute_values(":partition", Self::attr_s(&candidate.partition))
                    .expression_attribute_values(":prefix", Self::attr_s(&prefix))
                    .projection_expression("message_version_key")
                    .consistent_read(true)
                    .limit(limit as i32)
                    .set_exclusive_start_key(candidate.position.take())
                    .send()
                    .await
                    .map_err(|e| {
                        Error::Internal(format!("failed to read obsolete append chunks: {e}"))
                    })?;
                let mut deleted = 0;
                for item in page.items() {
                    let key = Self::item_str(item, "message_version_key").ok_or_else(|| {
                        Error::Internal("obsolete append chunk key missing".to_string())
                    })?;
                    self.client
                        .delete_item()
                        .table_name(&self.tables.version_entries)
                        .key("app_channel", Self::attr_s(&candidate.partition))
                        .key("message_version_key", Self::attr_s(&key))
                        .send()
                        .await
                        .map_err(|e| {
                            Error::Internal(format!("failed to delete append chunk: {e}"))
                        })?;
                    deleted += 1;
                }
                candidate.position = page.last_evaluated_key().cloned();
                if candidate.position.is_none() {
                    self.client
                        .delete_item()
                        .table_name(&self.tables.version_entries)
                        .key("app_channel", Self::attr_s(&candidate.partition))
                        .key("message_version_key", Self::attr_s(&candidate.manifest))
                        .send()
                        .await
                        .map_err(|e| {
                            Error::Internal(format!(
                                "failed to remove append garbage manifest: {e}"
                            ))
                        })?;
                    cursor.candidate = None;
                }
                return Ok((
                    deleted,
                    cursor.candidate.is_some()
                        || (cursor.scan.is_some() || !cursor.pending.is_empty()),
                ));
            }
        }
        Ok((0, true))
    }

    fn references_candidate(bytes: &[u8], candidate: &Candidate) -> Result<bool> {
        if !is_compact(bytes) {
            return Ok(false);
        }
        let payload = StoredVersionPayload::decode(bytes)?;
        Ok(
            payload.record().message_serial().as_str() == candidate.message
                && payload.run().is_some_and(|run| {
                    run.run.as_str() == candidate.run
                        && run.generation.as_deref() == Some(candidate.generation.as_str())
                }),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sockudo_core::message_envelope::PublishIdempotencyMetadata;
    use sockudo_core::version_store::{
        VersionCreateLimits, VersionMutation, VersionMutationLimits, VersionPrecondition,
    };
    use sockudo_core::versioned_messages::{
        FieldPatch, MessageAppend, MessageFieldDelta, MessageSerial, VersionMetadata,
        VersionSerial, VersionedMessage,
    };
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

    async fn sweep(store: &DynamoDbVersionStore) -> u64 {
        let mut removed = 0;
        for _ in 0..1000 {
            let (count, more) = store.purge_before(i64::MAX, 2).await.unwrap();
            assert!(count <= 2, "one GC step exceeded its deletion budget");
            removed += count;
            if !more {
                return removed;
            }
        }
        panic!("bounded append GC did not finish");
    }

    #[tokio::test]
    #[ignore = "requires isolated DynamoDB Local on port 25473"]
    async fn c2_dynamodb_chunk_gc_preserves_receipts_and_reclaims_unreferenced_runs() {
        let settings = DynamoDbSettings {
            endpoint_url: Some("http://127.0.0.1:25473".to_string()),
            aws_access_key_id: Some("c2".to_string()),
            aws_secret_access_key: Some("c2-local-only".to_string()),
            ..Default::default()
        };
        let prefix = format!("c2gc{}", uuid::Uuid::new_v4().simple());
        let store = DynamoDbVersionStore::new(&settings, &prefix, 3600)
            .await
            .unwrap();
        store.set_append_storage_enabled(true).await.unwrap();
        for (message, receipt) in [
            ("msg:expired", false),
            ("msg:receipt", true),
            ("__append_customer", false),
        ] {
            let record = StoredVersionRecord {
                app_id: "gc".to_string(),
                channel: "room".to_string(),
                original_client_id: None,
                envelope: None,
                message: VersionedMessage::new_create(
                    MessageSerial::new(message).unwrap(),
                    metadata(0),
                    1,
                    0,
                    None,
                    Some(MessageData::String("seed".repeat(2048))),
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
            let result = store
                .compare_and_apply(VersionMutationRequest {
                    app_id: "gc".to_string(),
                    channel: "room".to_string(),
                    message_serial: record.message_serial().clone(),
                    expected: VersionPrecondition::from_record(&record),
                    version: metadata(1),
                    mutation: VersionMutation::Append(MessageAppend {
                        data_fragment: "tail🙂".to_string(),
                        extras: None,
                    }),
                    idempotency: receipt.then(|| PublishIdempotencyMetadata {
                        cache_key: "keep".to_string(),
                        payload_fingerprint: "fingerprint".to_string(),
                    }),
                    limits: VersionMutationLimits::default(),
                })
                .await
                .unwrap();
            let VersionMutationResult::Applied { record, .. } = result else {
                panic!("append failed");
            };
            if message == "__append_customer" {
                // This valid public serial resembles internal namespaces. An
                // update moves latest to another run, leaving history as the
                // only reference to its original append chunks.
                let result = store
                    .compare_and_apply(VersionMutationRequest {
                        app_id: "gc".to_string(),
                        channel: "room".to_string(),
                        message_serial: record.message_serial().clone(),
                        expected: VersionPrecondition::from_record(&record),
                        version: metadata(2),
                        mutation: VersionMutation::Update(MessageFieldDelta {
                            data: FieldPatch::Replace(MessageData::String(
                                "replacement".to_string(),
                            )),
                            ..Default::default()
                        }),
                        idempotency: None,
                        limits: VersionMutationLimits::default(),
                    })
                    .await
                    .unwrap();
                assert!(matches!(result, VersionMutationResult::Applied { .. }));
            }
        }
        assert_eq!(sweep(&store).await, 0, "live references pin their chunks");
        let partition = DynamoDbVersionStore::app_channel_key("gc", "room");
        let page = store
            .client
            .query()
            .table_name(&store.tables.version_entries)
            .key_condition_expression("app_channel = :partition")
            .expression_attribute_values(":partition", DynamoDbVersionStore::attr_s(&partition))
            .consistent_read(true)
            .send()
            .await
            .unwrap();
        for item in page.items() {
            let key = DynamoDbVersionStore::item_str(item, "message_version_key").unwrap();
            if key.starts_with("msg:") {
                store
                    .client
                    .delete_item()
                    .table_name(&store.tables.version_entries)
                    .key("app_channel", DynamoDbVersionStore::attr_s(&partition))
                    .key("message_version_key", DynamoDbVersionStore::attr_s(&key))
                    .send()
                    .await
                    .unwrap();
            }
        }
        for message in ["msg:expired", "msg:receipt"] {
            store
                .client
                .delete_item()
                .table_name(&store.tables.version_messages)
                .key("app_channel", DynamoDbVersionStore::attr_s(&partition))
                .key("message_serial", DynamoDbVersionStore::attr_s(message))
                .send()
                .await
                .unwrap();
        }
        // Interrupt after the persistent fence but before deletion, then
        // recover with a fresh process-local cursor.
        let mut fenced = false;
        for _ in 0..1000 {
            let (removed, _) = store.purge_before(i64::MAX, 2).await.unwrap();
            assert_eq!(removed, 0);
            fenced = store
                .append_gc
                .lock()
                .unwrap()
                .candidate
                .as_ref()
                .is_some_and(|candidate| matches!(candidate.phase, Phase::Chunks));
            if fenced {
                break;
            }
        }
        assert!(
            fenced,
            "garbage collector did not durably fence the expired run"
        );
        drop(store);
        let store = DynamoDbVersionStore::new(&settings, &prefix, 3600)
            .await
            .unwrap();
        assert_eq!(
            sweep(&store).await,
            3,
            "unreferenced three-chunk run is reclaimed after restart"
        );
        let page = store
            .client
            .query()
            .table_name(&store.tables.version_entries)
            .key_condition_expression("app_channel = :partition")
            .expression_attribute_values(":partition", DynamoDbVersionStore::attr_s(&partition))
            .consistent_read(true)
            .send()
            .await
            .unwrap();
        let keys = page
            .items()
            .iter()
            .map(|item| DynamoDbVersionStore::item_str(item, "message_version_key").unwrap())
            .collect::<Vec<_>>();
        assert!(!keys.iter().any(|key| key.contains("msg:expired")));
        assert_eq!(
            keys.iter()
                .filter(|key| key.starts_with("__append_chunk__ msg:receipt "))
                .count(),
            3
        );
        let receipt = page
            .items()
            .iter()
            .filter(|item| {
                DynamoDbVersionStore::item_str(item, "message_version_key")
                    .is_some_and(|key| key.starts_with("__operation__"))
            })
            .cloned()
            .collect::<Vec<_>>();
        assert!(store.materialize_items(&partition, &receipt).await.unwrap()[0].is_some());
        let history = store
            .get_versions(VersionStoreReadRequest {
                app_id: "gc".to_string(),
                channel: "room".to_string(),
                message_serial: MessageSerial::new("__append_customer").unwrap(),
                direction: VersionStoreDirection::OldestFirst,
                cursor: None,
                limit: 10,
            })
            .await
            .unwrap();
        assert_eq!(history.items.len(), 3);
        assert_eq!(
            history.items[1].message.data,
            Some(MessageData::String("seed".repeat(2048) + "tail🙂"))
        );
        store.set_append_storage_enabled(false).await.unwrap();
        assert!(store.materialize_append_storage(1).await.unwrap() > 0);
        assert_eq!(store.materialize_append_storage(1).await.unwrap(), 0);
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
    async fn reach_phase(store: &DynamoDbVersionStore, target: Phase) {
        for _ in 0..1000 {
            assert_eq!(store.purge_before(i64::MAX, 2).await.unwrap().0, 0);
            if store
                .append_gc
                .lock()
                .unwrap()
                .candidate
                .as_ref()
                .is_some_and(|candidate| candidate.phase == target)
            {
                return;
            }
        }
        panic!("garbage collection did not reach expected phase");
    }

    #[tokio::test]
    #[ignore = "requires isolated DynamoDB Local on port 25473"]
    async fn c2_dynamodb_pending_seed_gc_fences_staging_and_publication() {
        use sockudo_core::version_store::append_storage::{AppendRunPlan, encode_full};
        let settings = DynamoDbSettings {
            endpoint_url: Some("http://127.0.0.1:25473".to_string()),
            aws_access_key_id: Some("c2".to_string()),
            aws_secret_access_key: Some("c2-local-only".to_string()),
            ..Default::default()
        };
        let prefix = format!("c2pending{}", uuid::Uuid::new_v4().simple());
        let store = DynamoDbVersionStore::new(&settings, &prefix, 3600)
            .await
            .unwrap();
        let record = |message: &str| StoredVersionRecord {
            app_id: "pending".to_string(),
            channel: "room".to_string(),
            original_client_id: None,
            envelope: None,
            message: VersionedMessage::new_create(
                MessageSerial::new(message).unwrap(),
                metadata(0),
                1,
                1,
                None,
                Some(MessageData::String("x".repeat(8193))),
                None,
            ),
        };
        let partition = DynamoDbVersionStore::app_channel_key("pending", "room");
        for (name, completed, publish_during_gc) in [
            ("manifest-only", false, false),
            ("published-race", true, true),
            ("aborted-ready", true, false),
        ] {
            let record = record(name);
            let plan = if completed {
                store.stage_seed(&record).await.unwrap()
            } else {
                let plan = AppendRunPlan::for_seed_record(&record);
                store
                    .client
                    .transact_write_items()
                    .transact_items(store.seed_manifest_write(&record, &plan).unwrap().unwrap())
                    .send()
                    .await
                    .unwrap();
                plan
            };
            let run = plan.run().unwrap();
            let manifest = DynamoDbVersionStore::append_manifest_key(&record, run);
            assert_eq!(
                sweep(&store).await,
                0,
                "fresh staging must not be collected"
            );
            store
                .client
                .update_item()
                .table_name(&store.tables.version_entries)
                .key("app_channel", DynamoDbVersionStore::attr_s(&partition))
                .key(
                    "message_version_key",
                    DynamoDbVersionStore::attr_s(&manifest),
                )
                .update_expression("SET created_at_ms = :old")
                .expression_attribute_values(":old", DynamoDbVersionStore::attr_n(0))
                .send()
                .await
                .unwrap();
            reach_phase(
                &store,
                if publish_during_gc {
                    Phase::Fence
                } else {
                    Phase::Chunks
                },
            )
            .await;
            let mut latest = HashMap::from([
                (
                    "app_channel".to_string(),
                    DynamoDbVersionStore::attr_s(&partition),
                ),
                (
                    "message_serial".to_string(),
                    DynamoDbVersionStore::attr_s(record.message_serial().as_str()),
                ),
                (
                    "latest_payload_bytes".to_string(),
                    DynamoDbVersionStore::attr_b(encode_full(&record).unwrap()),
                ),
                (
                    "latest_version_serial".to_string(),
                    DynamoDbVersionStore::attr_s(record.version_serial().as_str()),
                ),
                (
                    "latest_delivery_serial".to_string(),
                    DynamoDbVersionStore::attr_n(record.delivery_serial()),
                ),
            ]);
            DynamoDbVersionStore::seed_attributes(&mut latest, &record, &plan);
            let put = Put::builder()
                .table_name(&store.tables.version_messages)
                .set_item(Some(latest))
                .build()
                .unwrap();
            let publish = store
                .client
                .transact_write_items()
                .transact_items(TransactWriteItem::builder().put(put).build())
                .transact_items(
                    store
                        .seed_activation_write(&record, &plan)
                        .unwrap()
                        .unwrap(),
                )
                .send()
                .await;
            if publish_during_gc {
                publish.unwrap();
                assert_eq!(
                    sweep(&store).await,
                    0,
                    "stale pending-state fence cannot collect an activated seed at the same head"
                );
                let state = store
                    .client
                    .get_item()
                    .table_name(&store.tables.version_entries)
                    .key("app_channel", DynamoDbVersionStore::attr_s(&partition))
                    .key(
                        "message_version_key",
                        DynamoDbVersionStore::attr_s(&manifest),
                    )
                    .consistent_read(true)
                    .send()
                    .await
                    .unwrap()
                    .item
                    .unwrap();
                assert!(!state.contains_key("garbage_collecting"));
                store
                    .client
                    .delete_item()
                    .table_name(&store.tables.version_messages)
                    .key("app_channel", DynamoDbVersionStore::attr_s(&partition))
                    .key(
                        "message_serial",
                        DynamoDbVersionStore::attr_s(record.message_serial().as_str()),
                    )
                    .send()
                    .await
                    .unwrap();
            } else {
                assert!(
                    publish.is_err(),
                    "tombstone must reject stale latest publication"
                );
                let writes = store.append_chunk_writes(&record, &plan).unwrap();
                let stage = store
                    .client
                    .transact_write_items()
                    .transact_items(writes[0].clone())
                    .transact_items(store.seed_stage_guard(&record, &plan).unwrap().unwrap())
                    .send()
                    .await;
                assert!(stage.is_err(), "tombstone must reject stale chunk staging");
            }
            assert_eq!(sweep(&store).await, if completed { 3 } else { 0 });
            let remaining = store
                .client
                .query()
                .table_name(&store.tables.version_entries)
                .key_condition_expression("app_channel = :partition")
                .expression_attribute_values(":partition", DynamoDbVersionStore::attr_s(&partition))
                .consistent_read(true)
                .send()
                .await
                .unwrap();
            assert!(
                remaining.items().is_empty(),
                "abandoned seed chunks and manifest must be reclaimed"
            );
        }
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
