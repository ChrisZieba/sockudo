use super::append_runs::{FORMAT_FENCE, WRITE_SEEDS, seed_writes};
use super::mutation::{MutationState, mutation_arguments};
use super::*;
use sockudo_core::version_store::append_storage::{AppendRunPlan, AppendRunRef};
use sockudo_core::versioned_messages::VersionSerial;

#[cfg(feature = "versioned-messages")]
#[async_trait::async_trait]
impl VersionStore for SurrealVersionStore {
    async fn ensure_stream_id(&self, app_id: &str, channel: &str) -> Result<String> {
        Ok(format!("{app_id}/{channel}"))
    }

    async fn reserve_delivery_position(
        &self,
        app_id: &str,
        channel: &str,
    ) -> Result<VersionWriteReservation> {
        let block = self.reserve_delivery_positions(app_id, channel, 1).await?;
        Ok(VersionWriteReservation {
            stream_id: block.stream_id,
            delivery_serial: block.start_delivery_serial,
        })
    }

    async fn reserve_delivery_positions(
        &self,
        app_id: &str,
        channel: &str,
        block_size: u64,
    ) -> Result<VersionWriteReservationBlock> {
        if block_size == 0 {
            return Err(Error::InvalidMessageFormat(
                "version delivery reservation block size must be greater than 0".to_string(),
            ));
        }
        let block_size_i64 = i64::try_from(block_size).map_err(|_| {
            Error::InvalidMessageFormat(
                "version delivery reservation block size is too large".to_string(),
            )
        })?;
        let record_id = deterministic_key([app_id, channel].into_iter());
        let stream_id = format!("{app_id}/{channel}");
        loop {
            let existing: Option<StoredVersionStreamRec> = self
                .db
                .select((self.tables.streams.clone(), record_id.clone()))
                .await
                .map_err(|e| {
                    Error::Internal(format!("Failed to fetch SurrealDB version stream: {e}"))
                })?;

            if let Some(existing) = existing {
                let now_ms = sockudo_core::history::now_ms();
                let mut response = self
                    .db
                    .query("UPDATE ONLY type::record($table, $id) SET next_delivery_serial = $next, updated_at_ms = $now WHERE next_delivery_serial = $expected RETURN AFTER")
                    .bind(("table", self.tables.streams.clone()))
                    .bind(("id", record_id.clone()))
                    .bind(("next", existing.next_delivery_serial + block_size_i64))
                    .bind(("now", now_ms))
                    .bind(("expected", existing.next_delivery_serial))
                    .await
                    .map_err(|e| Error::Internal(format!("Failed to advance SurrealDB version delivery serial: {e}")))?;
                let updated: Option<StoredVersionStreamRec> =
                    response.take(0usize).map_err(|e| {
                        Error::Internal(format!(
                            "Failed to decode SurrealDB version serial advancement: {e}"
                        ))
                    })?;
                if updated.is_some() {
                    return Ok(VersionWriteReservationBlock {
                        stream_id: existing.stream_id,
                        start_delivery_serial: existing.next_delivery_serial as u64,
                        len: block_size,
                    });
                }
                continue;
            }

            let now_ms = sockudo_core::history::now_ms();
            let stream = StoredVersionStreamRec {
                app_id: app_id.to_string(),
                channel: channel.to_string(),
                stream_id: stream_id.clone(),
                next_delivery_serial: block_size_i64.saturating_add(1),
                oldest_delivery_serial: None,
                newest_delivery_serial: None,
                open_stream_count: 0,
                updated_at_ms: now_ms,
            };
            let create_result: Result<Option<StoredVersionStreamRec>> = self
                .db
                .create((self.tables.streams.clone(), record_id.clone()))
                .content(stream.clone())
                .await
                .map_err(|e| {
                    Error::Internal(format!("Failed to create SurrealDB version stream: {e}"))
                });
            match create_result {
                Ok(Some(_)) | Ok(None) => {
                    return Ok(VersionWriteReservationBlock {
                        stream_id,
                        start_delivery_serial: 1,
                        len: block_size,
                    });
                }
                Err(err) => {
                    let err_text = err.to_string();
                    if err_text.contains("already exists")
                        || err_text.contains("already been created")
                        || err_text.contains("Database record")
                    {
                        continue;
                    }
                    return Err(err);
                }
            }
        }
    }

    async fn append_version(&self, record: StoredVersionRecord) -> Result<()> {
        let format = self.append_format().await?;
        let plan = if format.enabled {
            AppendRunPlan::for_seed_record(&record)
        } else {
            AppendRunPlan::Full
        };
        let now_ms = sockudo_core::history::now_ms();
        let payload_bytes = sockudo_core::version_store::append_storage::encode_full(&record)?;
        let entry_id = deterministic_key(
            [
                record.app_id.as_str(),
                record.channel.as_str(),
                record.message_serial().as_str(),
                record.version_serial().as_str(),
            ]
            .into_iter(),
        );
        let message_id = deterministic_key(
            [
                record.app_id.as_str(),
                record.channel.as_str(),
                record.message_serial().as_str(),
            ]
            .into_iter(),
        );
        let stream_id =
            deterministic_key([record.app_id.as_str(), record.channel.as_str()].into_iter());
        let entry = StoredVersionEntryRec {
            app_id: record.app_id.clone(),
            channel: record.channel.clone(),
            message_serial: record.message_serial().as_str().to_string(),
            version_serial: record.version_serial().as_str().to_string(),
            delivery_serial: record.delivery_serial() as i64,
            payload_bytes: payload_bytes.clone().into(),
            created_at_ms: now_ms,
        };
        let message = StoredVersionMessageRec {
            app_id: record.app_id.clone(),
            channel: record.channel.clone(),
            message_serial: record.message_serial().as_str().to_string(),
            latest_version_serial: record.version_serial().as_str().to_string(),
            latest_entry_key: entry_id.clone(),
            history_serial: record.history_serial() as i64,
            latest_payload_bytes: payload_bytes.clone().into(),
            append_count: i64::from(
                record.message.action == sockudo_core::versioned_messages::MessageAction::Append,
            ),
            is_open_stream: record.is_open_ai_stream(),
            updated_at_ms: now_ms,
            latest_append_run: plan.run().map(|run| run.run.as_str().to_string()),
            latest_append_len: plan.run().map(|run| run.data_len as i64),
            latest_append_head: plan
                .run()
                .map(|_| record.version_serial().as_str().to_string()),
            latest_append_pinned: plan.run().map(|_| false),
            latest_append_generation: plan.run().and_then(|run| run.generation.clone()),
        };
        // The imported entry stays full. Its seed and latest pointer become
        // visible atomically, so the first following append only rewrites a tail.
        self.db.query(format!(
            "BEGIN TRANSACTION; {FORMAT_FENCE} IF (SELECT * FROM ONLY type::record($entry_table, $entry_id)) = NONE {{ CREATE ONLY type::record($entry_table, $entry_id) CONTENT $entry_content; }}; LET $existing = SELECT * FROM ONLY type::record($message_table, $message_id); IF $existing = NONE {{ CREATE ONLY type::record($message_table, $message_id) CONTENT $message_content; {WRITE_SEEDS} }} ELSE IF $existing.latest_version_serial < $version {{ UPDATE ONLY type::record($message_table, $message_id) SET latest_version_serial = $version, latest_entry_key = $entry_id, latest_payload_bytes = $payload, updated_at_ms = $now, latest_append_run = $run, latest_append_len = $len, latest_append_head = $head, latest_append_pinned = $pinned, latest_append_generation = $generation WHERE latest_version_serial < $version; {WRITE_SEEDS} }}; UPDATE ONLY type::record($stream_table, $stream_id) SET oldest_delivery_serial = IF oldest_delivery_serial IS NONE OR $delivery < oldest_delivery_serial THEN $delivery ELSE oldest_delivery_serial END, newest_delivery_serial = IF newest_delivery_serial IS NONE OR $delivery > newest_delivery_serial THEN $delivery ELSE newest_delivery_serial END, updated_at_ms = $now; COMMIT TRANSACTION;"
        ))
            .bind(("format_table", self.tables.format.clone())).bind(("format_epoch", format.epoch)).bind(("format_enabled", format.enabled))
            .bind(("entry_table", self.tables.entries.clone())).bind(("entry_id", entry_id)).bind(("entry_content", entry))
            .bind(("message_table", self.tables.messages.clone())).bind(("message_id", message_id))
            .bind(("run", message.latest_append_run.clone())).bind(("len", message.latest_append_len))
            .bind(("head", message.latest_append_head.clone())).bind(("pinned", message.latest_append_pinned))
            .bind(("generation", message.latest_append_generation.clone())).bind(("message_content", message))
            .bind(("version", record.version_serial().as_str().to_string())).bind(("payload", payload_bytes))
            .bind(("run_table", self.tables.runs.clone())).bind(("chunk_table", self.tables.chunks.clone()))
            .bind(("seeds", seed_writes(&record, &plan, now_ms)?))
            .bind(("stream_table", self.tables.streams.clone())).bind(("stream_id", stream_id))
            .bind(("delivery", record.delivery_serial() as i64)).bind(("now", now_ms))
            .await.and_then(|response| response.check())
            .map_err(|e| Error::Internal(format!("failed to import version and append seed: {e}")))?;
        Ok(())
    }

    async fn commit_create(&self, request: VersionCreateRequest) -> Result<VersionCreateResult> {
        if let Some(limit) = request.limits.max_accumulated_message_bytes
            && request.record.data_bytes()? > limit
        {
            return Ok(VersionCreateResult::Rejected(
                VersionCreateRejection::AccumulatedMessageBytes { limit },
            ));
        }
        let stream_record_id = deterministic_key(
            [
                request.record.app_id.as_str(),
                request.record.channel.as_str(),
            ]
            .into_iter(),
        );
        let existing_stream: Option<StoredVersionStreamRec> = self
            .db
            .select((self.tables.streams.clone(), stream_record_id.clone()))
            .await
            .map_err(|e| Error::Internal(format!("Failed to read version stream: {e}")))?;
        let next_delivery = existing_stream
            .as_ref()
            .map_or(1, |stream| stream.next_delivery_serial) as u64;
        let open_count = existing_stream
            .as_ref()
            .map_or(0, |stream| stream.open_stream_count) as usize;
        if request.record.is_open_ai_stream()
            && let Some(limit) = request.limits.max_open_streaming_messages_per_channel
            && open_count >= limit
        {
            return Ok(VersionCreateResult::Rejected(
                VersionCreateRejection::OpenStreamingMessages { limit },
            ));
        }
        let message_id = deterministic_key(
            [
                request.record.app_id.as_str(),
                request.record.channel.as_str(),
                request.record.message_serial().as_str(),
            ]
            .into_iter(),
        );
        if let Some(existing) = self
            .get_latest(
                &request.record.app_id,
                &request.record.channel,
                request.record.message_serial(),
            )
            .await?
        {
            return Ok(VersionCreateResult::Conflict {
                current: Some(existing),
            });
        }
        let stream_id = format!("{}/{}", request.record.app_id, request.record.channel);
        let record = request
            .record
            .with_delivery_position(&stream_id, next_delivery);
        let format = self.append_format().await?;
        let plan = if format.enabled {
            AppendRunPlan::for_seed_record(&record)
        } else {
            AppendRunPlan::Full
        };
        let payload_bytes = sonic_rs::to_vec(&record)
            .map_err(|e| Error::Internal(format!("Failed to serialize create record: {e}")))?;
        let now_ms = sockudo_core::history::now_ms();
        let entry_id = deterministic_key(
            [
                record.app_id.as_str(),
                record.channel.as_str(),
                record.message_serial().as_str(),
                record.version_serial().as_str(),
            ]
            .into_iter(),
        );
        let entry = StoredVersionEntryRec {
            app_id: record.app_id.clone(),
            channel: record.channel.clone(),
            message_serial: record.message_serial().as_str().to_string(),
            version_serial: record.version_serial().as_str().to_string(),
            delivery_serial: next_delivery as i64,
            payload_bytes: payload_bytes.clone().into(),
            created_at_ms: now_ms,
        };
        let message = StoredVersionMessageRec {
            app_id: record.app_id.clone(),
            channel: record.channel.clone(),
            message_serial: record.message_serial().as_str().to_string(),
            latest_version_serial: record.version_serial().as_str().to_string(),
            latest_entry_key: entry_id.clone(),
            history_serial: record.history_serial() as i64,
            latest_payload_bytes: payload_bytes.into(),
            append_count: 0,
            is_open_stream: record.is_open_ai_stream(),
            updated_at_ms: now_ms,
            latest_append_run: plan.run().map(|run| run.run.as_str().to_string()),
            latest_append_len: plan.run().map(|run| run.data_len as i64),
            latest_append_head: plan
                .run()
                .map(|_| record.version_serial().as_str().to_string()),
            latest_append_pinned: plan.run().map(|_| false),
            latest_append_generation: plan.run().and_then(|run| run.generation.clone()),
        };
        let next_open = open_count + usize::from(record.is_open_ai_stream());
        let (stream_statement, stream_content) = if let Some(stream) = existing_stream {
            (
                "LET $stream_write = UPDATE ONLY type::record($stream_table, $stream_id) SET next_delivery_serial = $next_delivery, open_stream_count = $next_open, oldest_delivery_serial = IF oldest_delivery_serial IS NONE THEN $delivery ELSE oldest_delivery_serial END, newest_delivery_serial = $delivery, updated_at_ms = $now WHERE next_delivery_serial = $expected_delivery AND open_stream_count = $expected_open RETURN AFTER; IF $stream_write = NONE { THROW 'version_conflict'; };",
                Some(stream),
            )
        } else {
            (
                "LET $stream_write = CREATE ONLY type::record($stream_table, $stream_id) CONTENT $stream_content;",
                None,
            )
        };
        let stream = stream_content.unwrap_or(StoredVersionStreamRec {
            app_id: record.app_id.clone(),
            channel: record.channel.clone(),
            stream_id: stream_id.clone(),
            next_delivery_serial: 2,
            oldest_delivery_serial: Some(1),
            newest_delivery_serial: Some(1),
            open_stream_count: next_open as i64,
            updated_at_ms: now_ms,
        });
        let sql = format!(
            "BEGIN TRANSACTION; {FORMAT_FENCE} {stream_statement} LET $message_write = CREATE ONLY type::record($message_table, $message_id) CONTENT $message_content; LET $entry_write = CREATE ONLY type::record($entry_table, $entry_id) CONTENT $entry_content; {WRITE_SEEDS} COMMIT TRANSACTION;"
        );
        let response = self
            .db
            .query(sql)
            .bind(("format_table", self.tables.format.clone()))
            .bind(("format_epoch", format.epoch))
            .bind(("format_enabled", format.enabled))
            .bind(("run_table", self.tables.runs.clone()))
            .bind(("chunk_table", self.tables.chunks.clone()))
            .bind(("seeds", seed_writes(&record, &plan, now_ms)?))
            .bind(("stream_table", self.tables.streams.clone()))
            .bind(("stream_id", stream_record_id))
            .bind(("stream_content", stream))
            .bind(("next_delivery", (next_delivery + 1) as i64))
            .bind(("next_open", next_open as i64))
            .bind(("delivery", next_delivery as i64))
            .bind(("now", now_ms))
            .bind(("expected_delivery", next_delivery as i64))
            .bind(("expected_open", open_count as i64))
            .bind(("message_table", self.tables.messages.clone()))
            .bind(("message_id", message_id))
            .bind(("message_content", message))
            .bind(("entry_table", self.tables.entries.clone()))
            .bind(("entry_id", entry_id))
            .bind(("entry_content", entry))
            .await;
        match response.and_then(mutation::check_transaction) {
            Ok(_) => Ok(VersionCreateResult::Applied { record, stream_id }),
            Err(error) if mutation::is_write_conflict(&error) => {
                Ok(VersionCreateResult::Conflict {
                    current: self
                        .get_latest(&record.app_id, &record.channel, record.message_serial())
                        .await?,
                })
            }
            Err(error) => Err(Error::Internal(format!(
                "Failed to transact SurrealDB version create: {error}"
            ))),
        }
    }

    async fn compare_and_apply(
        &self,
        request: VersionMutationRequest,
    ) -> Result<VersionMutationResult> {
        for attempt in 0..8 {
            #[cfg(test)]
            let wire_before = crate::history::c2_wire_meter::phase_snapshot();
            let stream_record_id =
                deterministic_key([request.app_id.as_str(), request.channel.as_str()].into_iter());
            let Some(stream): Option<StoredVersionStreamRec> = self
                .db
                .select((self.tables.streams.clone(), stream_record_id.clone()))
                .await
                .map_err(|e| Error::Internal(format!("Failed to read version stream: {e}")))?
            else {
                return Ok(VersionMutationResult::Conflict { current: None });
            };
            #[cfg(test)]
            crate::history::c2_wire_meter::report_phase("surrealdb.stream", wire_before);
            if let Some(operation) = request.idempotency.as_ref() {
                let receipt_id = deterministic_key(
                    [
                        request.app_id.as_str(),
                        request.channel.as_str(),
                        operation.cache_key.as_str(),
                    ]
                    .into_iter(),
                );
                if let Some(receipt) = self
                    .db
                    .select::<Option<StoredVersionReceiptRec>>((
                        self.tables.receipts.clone(),
                        receipt_id,
                    ))
                    .await
                    .map_err(|e| Error::Internal(format!("Failed to read mutation receipt: {e}")))?
                {
                    if receipt.operation_fingerprint != operation.payload_fingerprint {
                        return Err(Error::IdempotencyConflict);
                    }
                    let record = self
                        .materialize_payloads(
                            &request.app_id,
                            &request.channel,
                            vec![receipt.payload_bytes],
                        )
                        .await?
                        .pop()
                        .ok_or_else(|| Error::Internal("mutation receipt is empty".to_string()))?;
                    return Ok(VersionMutationResult::Duplicate {
                        record,
                        stream_id: stream.stream_id,
                    });
                }
            }
            #[cfg(test)]
            let wire_before = crate::history::c2_wire_meter::phase_snapshot();
            let message_id = deterministic_key(
                [
                    request.app_id.as_str(),
                    request.channel.as_str(),
                    request.message_serial.as_str(),
                ]
                .into_iter(),
            );
            let Some(mut message): Option<StoredVersionMessageRec> = self
                .db
                .select((self.tables.messages.clone(), message_id.clone()))
                .await
                .map_err(|e| {
                    Error::Internal(format!("Failed to read mutation predecessor: {e}"))
                })?
            else {
                return Ok(VersionMutationResult::Conflict { current: None });
            };
            #[cfg(test)]
            crate::history::c2_wire_meter::report_phase("surrealdb.message", wire_before);
            #[cfg(test)]
            let wire_before = crate::history::c2_wire_meter::phase_snapshot();
            let format = self.append_format().await?;
            #[cfg(test)]
            crate::history::c2_wire_meter::report_phase("surrealdb.marker", wire_before);

            #[cfg(test)]
            let wire_before = crate::history::c2_wire_meter::phase_snapshot();
            let current = if message.latest_payload_bytes.is_empty() {
                self.get_latest(&request.app_id, &request.channel, &request.message_serial)
                    .await?
                    .ok_or_else(|| Error::Internal("Latest version entry is missing".to_string()))?
            } else {
                self.materialize_payloads(
                    &request.app_id,
                    &request.channel,
                    vec![std::mem::take(&mut message.latest_payload_bytes)],
                )
                .await?
                .pop()
                .ok_or_else(|| Error::Internal("mutation predecessor is empty".to_string()))?
            };
            #[cfg(test)]
            crate::history::c2_wire_meter::report_phase("surrealdb.materialize", wire_before);
            // The run pointer is written with the latest-state record, naming the
            // version it describes; older releases never update it, so it is used
            // only while it still names this predecessor. The transaction
            // re-checks the run head.
            let mut predecessor_run = None;
            let mut run_pinned = false;
            if let (Some(run), Some(data_len), Some(head)) = (
                message.latest_append_run.as_deref(),
                message
                    .latest_append_len
                    .and_then(|value| u64::try_from(value).ok()),
                message.latest_append_head.as_deref(),
            ) && head == current.version_serial().as_str()
                && current.data_bytes()? as u64 == data_len
            {
                run_pinned = message.latest_append_pinned.unwrap_or(false);
                predecessor_run = Some(AppendRunRef {
                    run: VersionSerial::new(run)?,
                    data_len,
                    generation: message.latest_append_generation.clone(),
                });
            }
            let delivery_serial = (stream.next_delivery_serial as u64)
                .max(current.delivery_serial().saturating_add(1));
            let outcome = request.apply_to(
                &current,
                &stream.stream_id,
                delivery_serial,
                message.append_count as usize,
            )?;
            let VersionMutationResult::Applied { record, .. } = outcome else {
                return Ok(outcome);
            };
            let opens = !current.is_open_ai_stream() && record.is_open_ai_stream();
            let closes = current.is_open_ai_stream() && !record.is_open_ai_stream();
            if opens
                && let Some(limit) = request.limits.max_open_streaming_messages_per_channel
                && stream.open_stream_count as usize >= limit
            {
                return Ok(VersionMutationResult::Rejected(
                    VersionMutationRejection::OpenStreamingMessages { limit },
                ));
            }
            let payload_plan = if format.enabled {
                AppendRunPlan::for_record_chunked(
                    current.version_serial(),
                    predecessor_run.as_ref(),
                    &record,
                )
            } else {
                AppendRunPlan::Full
            };
            // The explicit store marker is the rollout gate. Compact latest state
            // is never written while older nodes can still participate.
            let payload_bytes = payload_plan.encode(&record)?;
            let receipt_references_run =
                request.idempotency.is_some() && payload_plan.run().is_some();
            let plan = if format.enabled && matches!(payload_plan, AppendRunPlan::Full) {
                AppendRunPlan::for_seed_record(&record)
            } else {
                payload_plan
            };
            let run_pinned = (matches!(plan, AppendRunPlan::Extend { .. }) && run_pinned)
                || receipt_references_run;
            let args = mutation_arguments(
                &record,
                payload_bytes,
                &current,
                &stream,
                &format,
                &plan,
                MutationState {
                    now_ms: sockudo_core::history::now_ms(),
                    next_open: stream.open_stream_count + i64::from(opens) - i64::from(closes),
                    next_append: message.append_count
                        + i64::from(matches!(
                            request.mutation,
                            sockudo_core::version_store::VersionMutation::Append(_)
                        )),
                    run_pinned,
                    receipt: request.idempotency.as_ref(),
                },
            )?;
            #[cfg(test)]
            let wire_before = crate::history::c2_wire_meter::phase_snapshot();
            let committed = self.commit_mutation_function(args).await;
            #[cfg(test)]
            crate::history::c2_wire_meter::report_phase("surrealdb.commit", wire_before);
            match committed {
                Ok(_) => {
                    if let (Some(run), Some(snapshot)) = (plan.run(), plan.snapshot_after(&record))
                    {
                        self.append_cache.insert(
                            &record.app_id,
                            &record.channel,
                            record.message_serial(),
                            run,
                            snapshot.to_string(),
                        );
                    }
                    return Ok(VersionMutationResult::Applied {
                        record,
                        stream_id: stream.stream_id,
                    });
                }
                Err(error) if mutation::is_write_conflict(&error) => {
                    // Retry the complete read/plan/transaction. Keep the caller's
                    // precondition and operation identity: stale requests must
                    // conflict, while channel contention can safely retry.
                    // Index errors can contain record values; omit their text.
                    tracing::debug!(
                        attempt_count = attempt + 1,
                        "surreal version transaction conflicted"
                    );
                    tokio::task::yield_now().await;
                }
                Err(error) => {
                    return Err(Error::Internal(format!(
                        "Failed to transact SurrealDB version mutation: {error}"
                    )));
                }
            }
        }
        Ok(VersionMutationResult::Conflict {
            current: self
                .get_latest(&request.app_id, &request.channel, &request.message_serial)
                .await?,
        })
    }

    async fn get_latest(
        &self,
        app_id: &str,
        channel: &str,
        message_serial: &MessageSerial,
    ) -> Result<Option<StoredVersionRecord>> {
        let msg_id = deterministic_key([app_id, channel, message_serial.as_str()].into_iter());
        let msg_record: Option<StoredVersionMessageRec> = self
            .db
            .select((self.tables.messages.clone(), msg_id))
            .await
            .map_err(|e| {
                Error::Internal(format!("Failed to fetch SurrealDB version message: {e}"))
            })?;

        let Some(msg) = msg_record else {
            return Ok(None);
        };

        if !msg.latest_payload_bytes.is_empty() {
            return Ok(self
                .materialize_payloads(app_id, channel, vec![msg.latest_payload_bytes])
                .await?
                .pop());
        }

        let entry: Option<StoredVersionEntryRec> = self
            .db
            .select((self.tables.entries.clone(), msg.latest_entry_key))
            .await
            .map_err(|e| {
                Error::Internal(format!("Failed to fetch SurrealDB version entry: {e}"))
            })?;

        let Some(entry) = entry else {
            return Ok(None);
        };

        Ok(self
            .materialize_payloads(app_id, channel, vec![entry.payload_bytes])
            .await?
            .pop())
    }

    async fn get_versions(&self, request: VersionStoreReadRequest) -> Result<VersionStorePage> {
        request.validate()?;

        let order = match request.direction {
            VersionStoreDirection::NewestFirst => "DESC",
            VersionStoreDirection::OldestFirst => "ASC",
        };

        let mut clauses = vec![
            "app_id = $app_id".to_string(),
            "channel = $channel".to_string(),
            "message_serial = $message_serial".to_string(),
        ];
        let mut cursor_serial_bind: Option<String> = None;

        if let Some(cursor) = request.cursor.as_ref() {
            clauses.push(match request.direction {
                VersionStoreDirection::NewestFirst => "version_serial < $cursor_serial".to_string(),
                VersionStoreDirection::OldestFirst => "version_serial > $cursor_serial".to_string(),
            });
            cursor_serial_bind = Some(cursor.version_serial.as_str().to_string());
        }

        let sql = format!(
            "SELECT version_serial, payload_bytes FROM {} WHERE {} ORDER BY version_serial {} LIMIT {}",
            self.tables.entries,
            clauses.join(" AND "),
            order,
            request.limit + 1
        );

        let mut query = self
            .db
            .query(sql)
            .bind(("app_id", request.app_id.clone()))
            .bind(("channel", request.channel.clone()))
            .bind((
                "message_serial",
                request.message_serial.as_str().to_string(),
            ));

        if let Some(serial) = cursor_serial_bind {
            query = query.bind(("cursor_serial", serial));
        }

        let mut response = query.await.map_err(|e| {
            Error::Internal(format!("Failed to query SurrealDB version history: {e}"))
        })?;
        let rows: Vec<VersionPayloadRow> = response.take(0usize).map_err(|e| {
            Error::Internal(format!("Failed to decode SurrealDB version history: {e}"))
        })?;

        let has_more = rows.len() > request.limit;
        let items = self
            .materialize_payloads(
                &request.app_id,
                &request.channel,
                rows.into_iter()
                    .take(request.limit)
                    .map(|row| row.payload_bytes)
                    .collect(),
            )
            .await?;

        let next_cursor = if has_more {
            items.last().map(|item| VersionStoreCursor {
                version: 1,
                version_serial: item.version_serial().clone(),
                direction: request.direction,
            })
        } else {
            None
        };

        Ok(VersionStorePage {
            items,
            next_cursor,
            has_more,
        })
    }

    async fn replay_after(
        &self,
        request: VersionReplayRequest,
    ) -> Result<Vec<StoredVersionRecord>> {
        request.validate()?;

        let mut response = self
            .db
            .query(format!(
                "SELECT delivery_serial, payload_bytes FROM {} WHERE app_id = $app_id AND channel = $channel AND delivery_serial > $after ORDER BY delivery_serial ASC LIMIT {}",
                self.tables.entries, request.limit
            ))
            .bind(("app_id", request.app_id.clone()))
            .bind(("channel", request.channel.clone()))
            .bind(("after", request.after_delivery_serial as i64))
            .await
            .map_err(|e| Error::Internal(format!("Failed to query SurrealDB version replay: {e}")))?;

        let rows: Vec<VersionPayloadRow> = response.take(0usize).map_err(|e| {
            Error::Internal(format!(
                "Failed to decode SurrealDB version replay rows: {e}"
            ))
        })?;

        self.materialize_payloads(
            &request.app_id,
            &request.channel,
            rows.into_iter().map(|row| row.payload_bytes).collect(),
        )
        .await
    }

    async fn latest_by_history(
        &self,
        app_id: &str,
        channel: &str,
    ) -> Result<Vec<StoredVersionRecord>> {
        let mut response = self
            .db
            .query(format!(
                "SELECT latest_entry_key, history_serial FROM {} WHERE app_id = $app_id AND channel = $channel ORDER BY history_serial ASC",
                self.tables.messages
            ))
            .bind(("app_id", app_id.to_string()))
            .bind(("channel", channel.to_string()))
            .await
            .map_err(|e| {
                Error::Internal(format!(
                    "Failed to query SurrealDB version messages for latest_by_history: {e}"
                ))
            })?;

        let msg_rows: Vec<VersionLatestKeyRow> = response.take(0usize).map_err(|e| {
            Error::Internal(format!(
                "Failed to decode SurrealDB version message rows in latest_by_history: {e}"
            ))
        })?;

        let mut result = Vec::with_capacity(msg_rows.len());
        for row in msg_rows {
            let entry: Option<StoredVersionEntryRec> = self
                .db
                .select((self.tables.entries.clone(), row.latest_entry_key))
                .await
                .map_err(|e| {
                    Error::Internal(format!(
                        "Failed to fetch SurrealDB version entry in latest_by_history: {e}"
                    ))
                })?;
            if let Some(entry) = entry {
                result.push(entry.payload_bytes);
            }
        }
        self.materialize_payloads(app_id, channel, result).await
    }

    async fn set_append_storage_enabled(&self, enabled: bool) -> Result<()> {
        let format = self.append_format().await?;
        if enabled && !format.enabled {
            self.seed_latest_records(&format).await?;
        }
        self.db.query("BEGIN TRANSACTION; LET $changed = UPDATE ONLY type::record($table, 'format') SET enabled = $enabled, epoch += 1 WHERE epoch = $epoch RETURN AFTER; IF $changed = NONE { THROW 'append_format_changed'; }; COMMIT TRANSACTION;")
            .bind(("table", self.tables.format.clone())).bind(("enabled", enabled)).bind(("epoch", format.epoch))
            .await.and_then(|response| response.check())
            .map_err(|e| Error::Internal(format!("failed to change append storage format: {e}")))?;
        Ok(())
    }

    async fn materialize_append_storage(&self, batch_size: usize) -> Result<u64> {
        self.materialize_compact_records(batch_size).await
    }

    async fn stream_state(&self, app_id: &str, channel: &str) -> Result<VersionStreamState> {
        let record_id = deterministic_key([app_id, channel].into_iter());
        let record: Option<StoredVersionStreamRec> = self
            .db
            .select((self.tables.streams.clone(), record_id))
            .await
            .map_err(|e| {
                Error::Internal(format!(
                    "Failed to fetch SurrealDB version stream state: {e}"
                ))
            })?;

        let Some(stream) = record else {
            return Ok(VersionStreamState::default());
        };

        Ok(VersionStreamState {
            stream_id: Some(stream.stream_id),
            next_delivery_serial: Some(stream.next_delivery_serial as u64),
            oldest_available_delivery_serial: stream.oldest_delivery_serial.map(|v| v as u64),
            newest_available_delivery_serial: stream.newest_delivery_serial.map(|v| v as u64),
        })
    }

    async fn purge_before(&self, before_ms: i64, batch_size: usize) -> Result<(u64, bool)> {
        if batch_size == 0 {
            return Ok((0, false));
        }
        let limit = batch_size as i64;

        // SurrealDB has no LIMIT clause on DELETE, so we select a batch of
        // record ids and delete them. Two round-trips per table but each is
        // bounded by `limit`, which is what the caller asked for.
        let purge_table = |table: &str, ts_field: &'static str| {
            let db = self.db.clone();
            let table = table.to_string();
            async move {
                let select_sql =
                    format!("SELECT VALUE id FROM {table} WHERE {ts_field} < $cutoff LIMIT $limit");
                let mut response = db
                    .query(select_sql)
                    .bind(("cutoff", before_ms))
                    .bind(("limit", limit))
                    .await
                    .map_err(|e| {
                        Error::Internal(format!(
                            "Failed to select expired rows in SurrealDB {table}: {e}"
                        ))
                    })?;
                let ids: Vec<surrealdb::types::RecordId> = response.take(0usize).map_err(|e| {
                    Error::Internal(format!(
                        "Failed to decode expired row ids in SurrealDB {table}: {e}"
                    ))
                })?;
                let len = ids.len() as u64;
                if ids.is_empty() {
                    return Ok::<(u64, bool), Error>((0, false));
                }
                db.query("DELETE $ids")
                    .bind(("ids", ids))
                    .await
                    .map_err(|e| {
                        Error::Internal(format!(
                            "Failed to delete expired rows in SurrealDB {table}: {e}"
                        ))
                    })?;
                Ok((len, len as i64 == limit))
            }
        };

        let (entries_deleted, entries_more) =
            purge_table(&self.tables.entries, "created_at_ms").await?;
        let (messages_deleted, messages_more) =
            purge_table(&self.tables.messages, "updated_at_ms").await?;

        // Runs go last: a run outlives every retained entry inside it.
        let (runs_deleted, runs_more) = self.purge_append_runs(before_ms, limit).await?;

        Ok((
            entries_deleted + messages_deleted + runs_deleted,
            entries_more || messages_more || runs_more,
        ))
    }
}
