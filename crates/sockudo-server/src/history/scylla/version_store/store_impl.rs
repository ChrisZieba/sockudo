use super::*;
use sockudo_core::version_store::append_storage::{
    AppendRunPlan, AppendRunRef, StoredVersionPayload, encode_full,
};
use sockudo_core::versioned_messages::VersionSerial;

#[cfg(feature = "versioned-messages")]
#[async_trait::async_trait]
impl VersionStore for ScyllaVersionStore {
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
        let select_q = format!(
            "SELECT next_delivery_serial FROM {} WHERE app_id = ? AND channel = ?",
            self.tables.version_streams_fq()
        );
        let insert_q = format!(
            "INSERT INTO {} (app_id, channel, next_delivery_serial, migration_state, updated_at_ms) VALUES (?, ?, ?, 'native_only', ?) IF NOT EXISTS",
            self.tables.version_streams_fq()
        );
        let update_q = format!(
            "UPDATE {} SET next_delivery_serial = ?, updated_at_ms = ? WHERE app_id = ? AND channel = ? IF next_delivery_serial = ?",
            self.tables.version_streams_fq()
        );

        loop {
            let rows = self
                .session
                .query_unpaged(select_q.as_str(), (app_id, channel))
                .await
                .map_err(|e| {
                    Error::Internal(format!("Failed to read ScyllaDB version stream: {e}"))
                })?
                .into_rows_result()
                .map_err(|e| {
                    Error::Internal(format!("Failed to decode ScyllaDB version stream: {e}"))
                })?;

            if let Some(row) = rows.maybe_first_row::<(i64,)>().map_err(|e| {
                Error::Internal(format!("Failed to deserialize version stream: {e}"))
            })? {
                let current = row.0 as u64;
                let now_ms = sockudo_core::history::now_ms();
                let mut stmt = Statement::new(update_q.clone());
                stmt.set_serial_consistency(Some(SerialConsistency::LocalSerial));
                let result = self
                    .session
                    .query_unpaged(
                        stmt,
                        (
                            (current as i64).saturating_add(block_size_i64),
                            now_ms,
                            app_id,
                            channel,
                            current as i64,
                        ),
                    )
                    .await
                    .map_err(|e| map_scylla_lwt_error("advance version delivery serial", e))?;
                if version_lwt_applied(result)? {
                    return Ok(VersionWriteReservationBlock {
                        stream_id: format!("{}/{}", app_id, channel),
                        start_delivery_serial: current,
                        len: block_size,
                    });
                }
                continue;
            }

            let now_ms = sockudo_core::history::now_ms();
            let mut stmt = Statement::new(insert_q.clone());
            stmt.set_serial_consistency(Some(SerialConsistency::LocalSerial));
            let result = self
                .session
                .query_unpaged(
                    stmt,
                    (app_id, channel, block_size_i64.saturating_add(1), now_ms),
                )
                .await
                .map_err(|e| map_scylla_lwt_error("create version stream row", e))?;
            if version_lwt_applied(result)? {
                return Ok(VersionWriteReservationBlock {
                    stream_id: format!("{}/{}", app_id, channel),
                    start_delivery_serial: 1,
                    len: block_size,
                });
            }
        }
    }

    async fn append_version(&self, record: StoredVersionRecord) -> Result<()> {
        self.write_projections(&record, &encode_full(&record)?)
            .await
    }

    async fn commit_create(&self, request: VersionCreateRequest) -> Result<VersionCreateResult> {
        use scylla::statement::batch::{Batch, BatchType};

        if let Some(limit) = request.limits.max_accumulated_message_bytes
            && request.record.data_bytes()? > limit
        {
            return Ok(VersionCreateResult::Rejected(
                VersionCreateRejection::AccumulatedMessageBytes { limit },
            ));
        }
        let commits = self.tables.version_commits_fq();
        let initialize = format!(
            "INSERT INTO {commits} (app_id, channel, commit_key, next_delivery_serial, open_stream_count, created_at_ms) VALUES (?, ?, 's', 1, 0, ?) IF NOT EXISTS"
        );
        let mut initialize_statement = Statement::new(initialize);
        initialize_statement.set_serial_consistency(Some(SerialConsistency::LocalSerial));
        let _ = self
            .prepared
            .execute_unpaged(
                initialize_statement,
                (
                    &request.record.app_id,
                    &request.record.channel,
                    sockudo_core::history::now_ms(),
                ),
            )
            .await
            .map_err(|e| map_scylla_lwt_error("initialize atomic version stream", e))?;
        let stream_q = format!(
            "SELECT next_delivery_serial, open_stream_count FROM {commits} WHERE app_id = ? AND channel = ? AND commit_key = 's'"
        );
        let stream_rows = self
            .prepared
            .execute_unpaged(stream_q, (&request.record.app_id, &request.record.channel))
            .await
            .map_err(|e| Error::Internal(format!("Failed to read atomic version stream: {e}")))?
            .into_rows_result()
            .map_err(|e| Error::Internal(format!("Failed to decode atomic version stream: {e}")))?;
        let (next_delivery, open_count) = stream_rows
            .single_row::<(i64, i64)>()
            .map_err(|e| Error::Internal(format!("Failed to deserialize version stream: {e}")))?;
        if request.record.is_open_ai_stream()
            && let Some(limit) = request.limits.max_open_streaming_messages_per_channel
            && open_count as usize >= limit
        {
            return Ok(VersionCreateResult::Rejected(
                VersionCreateRejection::OpenStreamingMessages { limit },
            ));
        }
        let message_key = message_commit_key(request.record.message_serial().as_str());
        let existing_q = format!(
            "SELECT payload_bytes FROM {commits} WHERE app_id = ? AND channel = ? AND commit_key = ?"
        );
        let existing = self
            .prepared
            .execute_unpaged(
                existing_q,
                (
                    &request.record.app_id,
                    &request.record.channel,
                    &message_key,
                ),
            )
            .await
            .map_err(|e| Error::Internal(format!("Failed to check atomic create target: {e}")))?
            .into_rows_result()
            .map_err(|e| Error::Internal(format!("Failed to decode atomic create target: {e}")))?;
        if let Some((payload,)) = existing
            .maybe_first_row::<(Vec<u8>,)>()
            .map_err(|e| Error::Internal(format!("Failed to deserialize create target: {e}")))?
        {
            let current = self
                .materialize_payloads(
                    &request.record.app_id,
                    &request.record.channel,
                    vec![payload],
                )
                .await?
                .pop();
            return Ok(VersionCreateResult::Conflict { current });
        }
        let stream_id = format!("{}/{}", request.record.app_id, request.record.channel);
        let record = request
            .record
            .with_delivery_position(&stream_id, next_delivery as u64);
        let payload = sonic_rs::to_vec(&record)
            .map_err(|e| Error::Internal(format!("Failed to serialize create record: {e}")))?;
        let version_key = version_commit_key(
            record.message_serial().as_str(),
            record.version_serial().as_str(),
        );
        let delivery_key = delivery_commit_key(record.delivery_serial());
        let next_open = open_count + i64::from(record.is_open_ai_stream());
        let now_ms = sockudo_core::history::now_ms();
        let seed = if self.append_storage_enabled().await? {
            self.stage_seed(&record).await?
        } else {
            AppendRunPlan::Full
        };
        let mut batch = Batch::new(BatchType::Logged);
        batch.set_serial_consistency(Some(SerialConsistency::LocalSerial));
        batch.append_statement(Statement::new(format!(
            "UPDATE {commits} SET next_delivery_serial = ?, open_stream_count = ? WHERE app_id = ? AND channel = ? AND commit_key = 's' IF next_delivery_serial = ? AND open_stream_count = ?"
        )));
        batch.append_statement(Statement::new(format!(
            "INSERT INTO {commits} (app_id, channel, commit_key, payload_bytes, latest_version_serial, latest_delivery_serial, history_serial, action, append_count, is_open_stream, created_at_ms, append_run, append_len, append_head, append_generation) VALUES (?, ?, ?, ?, ?, ?, ?, ?, 0, ?, ?, ?, ?, ?, ?) IF NOT EXISTS"
        )));
        batch.append_statement(Statement::new(format!(
            "INSERT INTO {commits} (app_id, channel, commit_key, payload_bytes, latest_version_serial, latest_delivery_serial, history_serial, action, created_at_ms) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?) IF NOT EXISTS"
        )));
        batch.append_statement(Statement::new(format!(
            "INSERT INTO {commits} (app_id, channel, commit_key, payload_bytes, latest_version_serial, latest_delivery_serial, history_serial, action, created_at_ms) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?) IF NOT EXISTS"
        )));
        let result = self
            .prepared
            .batch(
                &batch,
                (
                    (
                        next_delivery + 1,
                        next_open,
                        &record.app_id,
                        &record.channel,
                        next_delivery,
                        open_count,
                    ),
                    (
                        &record.app_id,
                        &record.channel,
                        &message_key,
                        payload.as_slice(),
                        record.version_serial().as_str(),
                        record.delivery_serial() as i64,
                        record.history_serial() as i64,
                        record.message.action.as_str(),
                        record.is_open_ai_stream(),
                        now_ms,
                        seed.run().map(|run| run.run.as_str()),
                        seed.run().map(|run| run.data_len as i64),
                        seed.run().map(|_| record.version_serial().as_str()),
                        seed.run().and_then(|run| run.generation.as_deref()),
                    ),
                    (
                        &record.app_id,
                        &record.channel,
                        &version_key,
                        payload.as_slice(),
                        record.version_serial().as_str(),
                        record.delivery_serial() as i64,
                        record.history_serial() as i64,
                        record.message.action.as_str(),
                        now_ms,
                    ),
                    (
                        &record.app_id,
                        &record.channel,
                        &delivery_key,
                        payload.as_slice(),
                        record.version_serial().as_str(),
                        record.delivery_serial() as i64,
                        record.history_serial() as i64,
                        record.message.action.as_str(),
                        now_ms,
                    ),
                ),
            )
            .await
            .map_err(|e| map_scylla_lwt_error("commit atomic version create", e))?;
        if !version_batch_applied(result)? {
            return Ok(VersionCreateResult::Conflict {
                current: self
                    .get_latest(&record.app_id, &record.channel, record.message_serial())
                    .await?,
            });
        }
        if let Err(error) = self.write_projections(&record, &payload).await {
            tracing::warn!(error = %error, "failed to refresh ScyllaDB version create projections");
        }
        Ok(VersionCreateResult::Applied { record, stream_id })
    }

    async fn compare_and_apply(
        &self,
        request: VersionMutationRequest,
    ) -> Result<VersionMutationResult> {
        use ::scylla::statement::batch::{Batch, BatchType};
        use ::scylla::value::CqlValue;

        #[cfg(test)]
        let wire_reads = crate::history::c2_wire_meter::phase_snapshot();
        let commits = self.tables.version_commits_fq();
        if let Some(operation) = request.idempotency.as_ref() {
            let operation_key = operation_commit_key(&operation.cache_key);
            let receipt_q = format!(
                "SELECT payload_bytes, latest_version_serial FROM {commits} WHERE app_id = ? AND channel = ? AND commit_key = ?"
            );
            let receipt = self
                .prepared
                .execute_unpaged(
                    receipt_q,
                    (&request.app_id, &request.channel, &operation_key),
                )
                .await
                .map_err(|e| Error::Internal(format!("Failed to read mutation receipt: {e}")))?
                .into_rows_result()
                .map_err(|e| Error::Internal(format!("Failed to decode mutation receipt: {e}")))?;
            if let Some((payload, fingerprint)) = receipt
                .maybe_first_row::<(Vec<u8>, String)>()
                .map_err(|e| Error::Internal(format!("Failed to deserialize receipt: {e}")))?
            {
                if fingerprint != operation.payload_fingerprint {
                    return Err(Error::IdempotencyConflict);
                }
                let record = self
                    .materialize_payloads(&request.app_id, &request.channel, vec![payload])
                    .await?
                    .pop()
                    .ok_or_else(|| Error::Internal("mutation receipt is empty".to_string()))?;
                return Ok(VersionMutationResult::Duplicate {
                    record,
                    stream_id: format!("{}/{}", request.app_id, request.channel),
                });
            }
        }
        let stream_q = format!(
            "SELECT next_delivery_serial, open_stream_count FROM {commits} WHERE app_id = ? AND channel = ? AND commit_key = 's'"
        );
        let stream = self
            .prepared
            .execute_unpaged(stream_q, (&request.app_id, &request.channel))
            .await
            .map_err(|e| Error::Internal(format!("Failed to read atomic version stream: {e}")))?
            .into_rows_result()
            .map_err(|e| Error::Internal(format!("Failed to decode atomic version stream: {e}")))?;
        let Some((next_delivery, open_count)) = stream
            .maybe_first_row::<(i64, i64)>()
            .map_err(|e| Error::Internal(format!("Failed to deserialize version stream: {e}")))?
        else {
            return Ok(VersionMutationResult::Conflict { current: None });
        };
        let message_key = message_commit_key(request.message_serial.as_str());
        let message_q = format!(
            "SELECT payload_bytes, latest_version_serial, latest_delivery_serial, append_count, is_open_stream, append_run, append_len, append_head, append_generation FROM {commits} WHERE app_id = ? AND channel = ? AND commit_key = ?"
        );
        let message = self
            .prepared
            .execute_unpaged(message_q, (&request.app_id, &request.channel, &message_key))
            .await
            .map_err(|e| Error::Internal(format!("Failed to read mutation predecessor: {e}")))?
            .into_rows_result()
            .map_err(|e| Error::Internal(format!("Failed to decode mutation predecessor: {e}")))?;
        let Some((
            current_payload,
            expected_version,
            expected_delivery,
            append_count,
            was_open,
            latest_run,
            latest_run_len,
            latest_run_head,
            latest_run_generation,
        )) = message
            .maybe_first_row::<(
                Vec<u8>,
                String,
                i64,
                i64,
                bool,
                Option<String>,
                Option<i64>,
                Option<String>,
                Option<String>,
            )>()
            .map_err(|e| {
                Error::Internal(format!("Failed to deserialize mutation predecessor: {e}"))
            })?
        else {
            return Ok(VersionMutationResult::Conflict { current: None });
        };
        let compact_predecessor = StoredVersionPayload::decode(&current_payload)?
            .run()
            .cloned();
        // Decode every supported format before applying the mutation.
        let current = self
            .materialize_payloads(&request.app_id, &request.channel, vec![current_payload])
            .await?
            .pop()
            .ok_or_else(|| Error::Internal("mutation predecessor is empty".to_string()))?;
        // The run pointer is written with the latest-state row, naming the
        // version it describes; older releases never update it, so it is used
        // only while it still names this predecessor. The batch re-checks the
        // run head.
        let mut predecessor_run = None;
        if let (Some(run), Some(data_len), Some(head)) = (
            latest_run,
            latest_run_len.and_then(|value| u64::try_from(value).ok()),
            latest_run_head,
        ) && head == current.version_serial().as_str()
            && current.data_bytes()? as u64 == data_len
        {
            predecessor_run = Some(AppendRunRef {
                run: VersionSerial::new(run)?,
                data_len,
                generation: latest_run_generation,
            });
        }
        if compact_predecessor.is_some()
            && predecessor_run
                .as_ref()
                .is_none_or(|run| run.generation.is_none())
        {
            predecessor_run = compact_predecessor;
        }
        let delivery_serial = (next_delivery as u64).max(current.delivery_serial() + 1);
        let stream_id = format!("{}/{}", request.app_id, request.channel);
        let outcome =
            request.apply_to(&current, &stream_id, delivery_serial, append_count as usize)?;
        let VersionMutationResult::Applied { record, .. } = outcome else {
            return Ok(outcome);
        };
        let opens = !was_open && record.is_open_ai_stream();
        let closes = was_open && !record.is_open_ai_stream();
        if opens
            && let Some(limit) = request.limits.max_open_streaming_messages_per_channel
            && open_count as usize >= limit
        {
            return Ok(VersionMutationResult::Rejected(
                VersionMutationRejection::OpenStreamingMessages { limit },
            ));
        }
        let next_open = open_count + i64::from(opens) - i64::from(closes);
        let next_append = append_count
            + i64::from(matches!(
                request.mutation,
                sockudo_core::version_store::VersionMutation::Append(_)
            ));
        let enabled = self.append_storage_enabled().await?;
        let plan = if enabled {
            AppendRunPlan::for_record_chunked(
                current.version_serial(),
                predecessor_run.as_ref(),
                &record,
            )
        } else {
            AppendRunPlan::Full
        };
        let payload = plan.encode(&record)?;
        let seed_staged = enabled && matches!(plan, AppendRunPlan::Full);
        let plan = if seed_staged {
            self.stage_seed(&record).await?
        } else {
            plan
        };
        let latest_payload = payload.clone();
        let version_key = version_commit_key(
            record.message_serial().as_str(),
            record.version_serial().as_str(),
        );
        let delivery_key = delivery_commit_key(delivery_serial);
        let now_ms = sockudo_core::history::now_ms();
        let text = |value: &str| Some(CqlValue::Text(value.to_string()));
        let int = |value: i64| Some(CqlValue::BigInt(value));
        let app = text(&record.app_id);
        let channel = text(&record.channel);
        let mut batch = Batch::new(BatchType::Logged);
        batch.set_serial_consistency(Some(SerialConsistency::LocalSerial));
        let mut values: Vec<Vec<Option<CqlValue>>> = Vec::with_capacity(6);
        batch.append_statement(Statement::new(format!(
            "UPDATE {commits} SET next_delivery_serial = ?, open_stream_count = ? WHERE app_id = ? AND channel = ? AND commit_key = 's' IF next_delivery_serial = ? AND open_stream_count = ?"
        )));
        values.push(vec![
            int(delivery_serial as i64 + 1),
            int(next_open),
            app.clone(),
            channel.clone(),
            int(next_delivery),
            int(open_count),
        ]);
        batch.append_statement(Statement::new(format!(
            "UPDATE {commits} SET payload_bytes = ?, latest_version_serial = ?, latest_delivery_serial = ?, action = ?, append_count = ?, is_open_stream = ?, created_at_ms = ?, append_run = ?, append_len = ?, append_head = ?, append_generation = ? WHERE app_id = ? AND channel = ? AND commit_key = ? IF latest_version_serial = ? AND latest_delivery_serial = ?"
        )));
        values.push(vec![
            Some(CqlValue::Blob(latest_payload)),
            text(record.version_serial().as_str()),
            int(delivery_serial as i64),
            text(record.message.action.as_str()),
            int(next_append),
            Some(CqlValue::Boolean(record.is_open_ai_stream())),
            int(now_ms),
            plan.run().and_then(|run| text(run.run.as_str())),
            plan.run().and_then(|run| int(run.data_len as i64)),
            plan.run()
                .and_then(|_| text(record.version_serial().as_str())),
            plan.run()
                .and_then(|run| run.generation.as_deref().and_then(text)),
            app.clone(),
            channel.clone(),
            text(&message_key),
            text(&expected_version),
            int(expected_delivery),
        ]);
        for key in [&version_key, &delivery_key] {
            batch.append_statement(Statement::new(format!(
                "INSERT INTO {commits} (app_id, channel, commit_key, payload_bytes, latest_version_serial, latest_delivery_serial, history_serial, action, created_at_ms) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?) IF NOT EXISTS"
            )));
            values.push(vec![
                app.clone(),
                channel.clone(),
                text(key),
                Some(CqlValue::Blob(payload.clone())),
                text(record.version_serial().as_str()),
                int(delivery_serial as i64),
                int(record.history_serial() as i64),
                text(record.message.action.as_str()),
                int(now_ms),
            ]);
        }
        if let (Some(run), Some(snapshot)) = (plan.run(), plan.snapshot_after(&record))
            && !seed_staged
        {
            let run_key = Self::run_manifest_key(record.message_serial().as_str(), run);
            let snapshot = if run.generation.is_some() {
                ""
            } else {
                snapshot
            };
            // The run row holds only the conditional head and length for v2.
            match &plan {
                AppendRunPlan::Extend {
                    expected_head,
                    expected_len,
                    ..
                } => {
                    batch.append_statement(Statement::new(format!(
                        "UPDATE {commits} SET payload_bytes = ?, latest_version_serial = ?, append_len = ?, created_at_ms = ? WHERE app_id = ? AND channel = ? AND commit_key = ? IF latest_version_serial = ? AND append_len = ?"
                    )));
                    values.push(vec![
                        Some(CqlValue::Blob(snapshot.as_bytes().to_vec())),
                        text(record.version_serial().as_str()),
                        int(run.data_len as i64),
                        int(now_ms),
                        app.clone(),
                        channel.clone(),
                        text(&run_key),
                        text(expected_head.as_str()),
                        int(*expected_len as i64),
                    ]);
                }
                _ => {
                    // A run id is its first version serial, which is greater
                    // than every retained version of the message, so an
                    // existing row with this key is an unreferenced leftover.
                    batch.append_statement(Statement::new(format!(
                        "INSERT INTO {commits} (app_id, channel, commit_key, payload_bytes, latest_version_serial, append_len, created_at_ms) VALUES (?, ?, ?, ?, ?, ?, ?)"
                    )));
                    values.push(vec![
                        app.clone(),
                        channel.clone(),
                        text(&run_key),
                        Some(CqlValue::Blob(snapshot.as_bytes().to_vec())),
                        text(record.version_serial().as_str()),
                        int(run.data_len as i64),
                        int(now_ms),
                    ]);
                }
            }
        }
        if let Some(run) = plan.run().filter(|run| run.generation.is_some())
            && !seed_staged
        {
            for chunk in plan.chunk_writes(&record)? {
                batch.append_statement(Statement::new(format!(
                    "INSERT INTO {commits} (app_id, channel, commit_key, payload_bytes) VALUES (?, ?, ?, ?)"
                )));
                values.push(vec![
                    app.clone(),
                    channel.clone(),
                    text(&Self::append_chunk_key(
                        record.message_serial().as_str(),
                        run,
                        chunk.index,
                    )),
                    Some(CqlValue::Blob(chunk.bytes)),
                ]);
            }
        }
        if let Some(operation) = request.idempotency.as_ref() {
            batch.append_statement(Statement::new(format!(
                "INSERT INTO {commits} (app_id, channel, commit_key, payload_bytes, latest_version_serial, created_at_ms) VALUES (?, ?, ?, ?, ?, ?) IF NOT EXISTS"
            )));
            values.push(vec![
                app.clone(),
                channel.clone(),
                text(&operation_commit_key(&operation.cache_key)),
                Some(CqlValue::Blob(payload.clone())),
                text(&operation.payload_fingerprint),
                int(now_ms),
            ]);
        }
        #[cfg(test)]
        let write_measurement = Self::measure_batch_before(&values);
        #[cfg(test)]
        crate::history::c2_wire_meter::report_phase("scylla_reads_and_plan", wire_reads);
        #[cfg(test)]
        let wire_batch = crate::history::c2_wire_meter::phase_snapshot();
        let result = self
            .prepared
            .batch(&batch, values)
            .await
            .map_err(|e| map_scylla_lwt_error("commit atomic version mutation", e))?;
        #[cfg(test)]
        crate::history::c2_wire_meter::report_phase("scylla_atomic_batch", wire_batch);
        if !version_batch_applied(result)? {
            return Ok(VersionMutationResult::Conflict {
                current: self
                    .get_latest(&request.app_id, &request.channel, &request.message_serial)
                    .await?,
            });
        }
        #[cfg(test)]
        Self::measure_batch_after(write_measurement);
        if let (Some(run), Some(snapshot)) = (plan.run(), plan.snapshot_after(&record)) {
            self.append_cache.insert(
                &record.app_id,
                &record.channel,
                record.message_serial(),
                run,
                snapshot.to_owned(),
            );
        }
        #[cfg(test)]
        let wire_projections = crate::history::c2_wire_meter::phase_snapshot();
        if let Err(error) = self.write_projections(&record, &payload).await {
            tracing::warn!(error = %error, "failed to refresh ScyllaDB version mutation projections");
        }
        #[cfg(test)]
        crate::history::c2_wire_meter::report_phase("scylla_projections", wire_projections);
        Ok(VersionMutationResult::Applied { record, stream_id })
    }

    async fn get_latest(
        &self,
        app_id: &str,
        channel: &str,
        message_serial: &sockudo_core::versioned_messages::MessageSerial,
    ) -> Result<Option<StoredVersionRecord>> {
        let commit_q = format!(
            "SELECT payload_bytes FROM {} WHERE app_id = ? AND channel = ? AND commit_key = ?",
            self.tables.version_commits_fq()
        );
        let commit_rows = self
            .session
            .query_unpaged(
                commit_q,
                (app_id, channel, message_commit_key(message_serial.as_str())),
            )
            .await
            .map_err(|e| Error::Internal(format!("Failed to query atomic latest version: {e}")))?
            .into_rows_result()
            .map_err(|e| Error::Internal(format!("Failed to decode atomic latest version: {e}")))?;
        if let Some((payload,)) = commit_rows
            .maybe_first_row::<(Vec<u8>,)>()
            .map_err(|e| Error::Internal(format!("Failed to deserialize atomic latest: {e}")))?
        {
            return Ok(self
                .materialize_payloads(app_id, channel, vec![payload])
                .await?
                .pop());
        }
        // version_entries_by_message is clustered by version_serial DESC — LIMIT 1 gives the latest.
        let sql = format!(
            "SELECT payload_bytes FROM {} WHERE app_id = ? AND channel = ? AND message_serial = ? LIMIT 1",
            self.tables.version_entries_by_message_fq()
        );
        let rows = self
            .session
            .query_unpaged(sql.as_str(), (app_id, channel, message_serial.as_str()))
            .await
            .map_err(|e| Error::Internal(format!("Failed to query latest version: {e}")))?
            .into_rows_result()
            .map_err(|e| Error::Internal(format!("Failed to decode latest version: {e}")))?;

        let Some(row) = rows
            .maybe_first_row::<(Vec<u8>,)>()
            .map_err(|e| Error::Internal(format!("Failed to deserialize latest version: {e}")))?
        else {
            return Ok(None);
        };

        Ok(self
            .materialize_payloads(app_id, channel, vec![row.0])
            .await?
            .pop())
    }

    async fn get_versions(&self, request: VersionStoreReadRequest) -> Result<VersionStorePage> {
        request.validate()?;
        let prefix = format!("v:{}:", request.message_serial.as_str());
        let upper = format!("{prefix}\u{10ffff}");
        let order = match request.direction {
            VersionStoreDirection::NewestFirst => "DESC",
            VersionStoreDirection::OldestFirst => "ASC",
        };
        // Both bounds stay inside this message's `v:` keys; a cursor replaces
        // the bound on the side the page is moving toward.
        let (lower_op, lower, upper) = match (request.cursor.as_ref(), request.direction) {
            (None, _) => (">=", prefix.clone(), upper),
            (Some(cursor), VersionStoreDirection::OldestFirst) => (
                ">",
                version_commit_key(
                    request.message_serial.as_str(),
                    cursor.version_serial.as_str(),
                ),
                upper,
            ),
            (Some(cursor), VersionStoreDirection::NewestFirst) => (
                ">=",
                prefix.clone(),
                version_commit_key(
                    request.message_serial.as_str(),
                    cursor.version_serial.as_str(),
                ),
            ),
        };
        let commit_sql = format!(
            "SELECT payload_bytes FROM {} WHERE app_id = ? AND channel = ? AND commit_key {lower_op} ? AND commit_key < ? ORDER BY commit_key {order} LIMIT ?",
            self.tables.version_commits_fq()
        );
        let commit_rows = self
            .session
            .query_unpaged(
                commit_sql,
                (
                    &request.app_id,
                    &request.channel,
                    lower,
                    upper,
                    (request.limit + 1) as i32,
                ),
            )
            .await
            .map_err(|e| Error::Internal(format!("Failed to query atomic versions: {e}")))?
            .into_rows_result()
            .map_err(|e| Error::Internal(format!("Failed to decode atomic versions: {e}")))?;
        let atomic_payloads = commit_rows
            .rows::<(Vec<u8>,)>()
            .map_err(|e| Error::Internal(format!("Failed to stream atomic versions: {e}")))?
            .map(|row| row.map(|value| value.0))
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(|e| Error::Internal(format!("Failed to collect atomic versions: {e}")))?;
        if !atomic_payloads.is_empty() {
            let has_more = atomic_payloads.len() > request.limit;
            let items = self
                .materialize_payloads(
                    &request.app_id,
                    &request.channel,
                    atomic_payloads.into_iter().take(request.limit).collect(),
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
            return Ok(VersionStorePage {
                items,
                next_cursor,
                has_more,
            });
        }
        // version_entries_by_message is clustered by version_serial DESC.
        // For NewestFirst: just read in natural order (DESC). For OldestFirst: use CLUSTERING ORDER.
        // Scylla doesn't support changing order per-query, but we can ORDER BY explicitly.
        let fetch_limit = (request.limit + 1) as i32;

        let rows = if let Some(cursor) = &request.cursor {
            let (op, order) = match request.direction {
                VersionStoreDirection::NewestFirst => ("<", "DESC"),
                VersionStoreDirection::OldestFirst => (">", "ASC"),
            };
            let sql = format!(
                "SELECT payload_bytes FROM {} WHERE app_id = ? AND channel = ? AND message_serial = ? AND version_serial {} ? ORDER BY version_serial {} LIMIT ?",
                self.tables.version_entries_by_message_fq(),
                op,
                order
            );
            self.session
                .query_unpaged(
                    sql.as_str(),
                    (
                        &request.app_id,
                        &request.channel,
                        request.message_serial.as_str(),
                        cursor.version_serial.as_str(),
                        fetch_limit,
                    ),
                )
                .await
                .map_err(|e| Error::Internal(format!("Failed to query version history: {e}")))?
                .into_rows_result()
                .map_err(|e| Error::Internal(format!("Failed to decode version history: {e}")))?
        } else {
            let order = match request.direction {
                VersionStoreDirection::NewestFirst => "DESC",
                VersionStoreDirection::OldestFirst => "ASC",
            };
            let sql = format!(
                "SELECT payload_bytes FROM {} WHERE app_id = ? AND channel = ? AND message_serial = ? ORDER BY version_serial {} LIMIT ?",
                self.tables.version_entries_by_message_fq(),
                order
            );
            self.session
                .query_unpaged(
                    sql.as_str(),
                    (
                        &request.app_id,
                        &request.channel,
                        request.message_serial.as_str(),
                        fetch_limit,
                    ),
                )
                .await
                .map_err(|e| Error::Internal(format!("Failed to query version history: {e}")))?
                .into_rows_result()
                .map_err(|e| Error::Internal(format!("Failed to decode version history: {e}")))?
        };

        let raw: Vec<Vec<u8>> = rows
            .rows::<(Vec<u8>,)>()
            .map_err(|e| Error::Internal(format!("Failed to stream version rows: {e}")))?
            .map(|r| r.map(|row| row.0))
            .collect::<std::result::Result<_, _>>()
            .map_err(|e| Error::Internal(format!("Failed to collect version rows: {e}")))?;

        let has_more = raw.len() > request.limit;
        let items = self
            .materialize_payloads(
                &request.app_id,
                &request.channel,
                raw.into_iter().take(request.limit).collect(),
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
        let after_key = delivery_commit_key(request.after_delivery_serial);
        let commit_sql = format!(
            "SELECT payload_bytes FROM {} WHERE app_id = ? AND channel = ? AND commit_key > ? AND commit_key < 'e:' ORDER BY commit_key ASC LIMIT ?",
            self.tables.version_commits_fq()
        );
        let commit_rows = self
            .session
            .query_unpaged(
                commit_sql,
                (
                    &request.app_id,
                    &request.channel,
                    after_key,
                    request.limit as i32,
                ),
            )
            .await
            .map_err(|e| Error::Internal(format!("Failed to replay atomic versions: {e}")))?
            .into_rows_result()
            .map_err(|e| Error::Internal(format!("Failed to decode atomic replay: {e}")))?;
        let atomic = commit_rows
            .rows::<(Vec<u8>,)>()
            .map_err(|e| Error::Internal(format!("Failed to stream atomic replay: {e}")))?
            .map(|row| {
                row.map(|(payload,)| payload)
                    .map_err(|e| Error::Internal(format!("Failed to read atomic replay: {e}")))
            })
            .collect::<Result<Vec<Vec<u8>>>>()?;
        if !atomic.is_empty() {
            return self
                .materialize_payloads(&request.app_id, &request.channel, atomic)
                .await;
        }
        let sql = format!(
            "SELECT payload_bytes FROM {} WHERE app_id = ? AND channel = ? AND delivery_serial > ? LIMIT ?",
            self.tables.version_entries_by_delivery_fq()
        );
        let rows = self
            .session
            .query_unpaged(
                sql.as_str(),
                (
                    &request.app_id,
                    &request.channel,
                    request.after_delivery_serial as i64,
                    request.limit as i32,
                ),
            )
            .await
            .map_err(|e| Error::Internal(format!("Failed to replay version entries: {e}")))?
            .into_rows_result()
            .map_err(|e| Error::Internal(format!("Failed to decode replay rows: {e}")))?;

        let payloads = rows
            .rows::<(Vec<u8>,)>()
            .map_err(|e| Error::Internal(format!("Failed to stream replay rows: {e}")))?
            .map(|r| {
                r.map(|(bytes,)| bytes)
                    .map_err(|e| Error::Internal(format!("Failed to collect replay row: {e}")))
            })
            .collect::<Result<Vec<Vec<u8>>>>()?;
        self.materialize_payloads(&request.app_id, &request.channel, payloads)
            .await
    }

    async fn latest_by_history(
        &self,
        app_id: &str,
        channel: &str,
    ) -> Result<Vec<StoredVersionRecord>> {
        let commit_q = format!(
            "SELECT payload_bytes, history_serial FROM {} WHERE app_id = ? AND channel = ? AND commit_key >= 'm:' AND commit_key < 'n:'",
            self.tables.version_commits_fq()
        );
        let commit_rows = self
            .session
            .query_unpaged(commit_q, (app_id, channel))
            .await
            .map_err(|e| Error::Internal(format!("Failed to query atomic messages: {e}")))?
            .into_rows_result()
            .map_err(|e| Error::Internal(format!("Failed to decode atomic messages: {e}")))?;
        let mut atomic = commit_rows
            .rows::<(Vec<u8>, i64)>()
            .map_err(|e| Error::Internal(format!("Failed to stream atomic messages: {e}")))?
            .map(|row| {
                row.map_err(|e| Error::Internal(format!("Failed to read atomic message: {e}")))
            })
            .collect::<Result<Vec<(Vec<u8>, i64)>>>()?;
        if !atomic.is_empty() {
            atomic.sort_by_key(|value| value.1);
            return self
                .materialize_payloads(
                    app_id,
                    channel,
                    atomic.into_iter().map(|value| value.0).collect(),
                )
                .await;
        }
        // Read version_messages ordered by history_serial, then fetch each entry individually.
        let msg_q = format!(
            "SELECT message_serial, latest_version_serial, history_serial FROM {} WHERE app_id = ? AND channel = ?",
            self.tables.version_messages_fq()
        );
        let msg_rows = self
            .session
            .query_unpaged(msg_q.as_str(), (app_id, channel))
            .await
            .map_err(|e| Error::Internal(format!("Failed to query version messages: {e}")))?
            .into_rows_result()
            .map_err(|e| Error::Internal(format!("Failed to decode version messages: {e}")))?;

        let mut msgs: Vec<(String, String, i64)> = msg_rows
            .rows::<(String, String, i64)>()
            .map_err(|e| Error::Internal(format!("Failed to stream version message rows: {e}")))?
            .map(|r| r.map_err(|e| Error::Internal(format!("Failed to collect msg row: {e}"))))
            .collect::<Result<Vec<_>>>()?;

        // Sort by history_serial ascending.
        msgs.sort_by_key(|(_, _, hs)| *hs);

        let entry_q = format!(
            "SELECT payload_bytes FROM {} WHERE app_id = ? AND channel = ? AND message_serial = ? AND version_serial = ?",
            self.tables.version_entries_by_message_fq()
        );
        let mut result = Vec::with_capacity(msgs.len());
        for (message_serial, latest_version_serial, _history_serial) in msgs {
            let rows = self
                .session
                .query_unpaged(
                    entry_q.as_str(),
                    (
                        app_id,
                        channel,
                        message_serial.as_str(),
                        latest_version_serial.as_str(),
                    ),
                )
                .await
                .map_err(|e| Error::Internal(format!("Failed to fetch version entry: {e}")))?
                .into_rows_result()
                .map_err(|e| Error::Internal(format!("Failed to decode version entry: {e}")))?;

            if let Some((bytes,)) = rows
                .maybe_first_row::<(Vec<u8>,)>()
                .map_err(|e| Error::Internal(format!("Failed to deserialize entry: {e}")))?
            {
                result.push(bytes);
            }
        }
        self.materialize_payloads(app_id, channel, result).await
    }

    async fn set_append_storage_enabled(&self, enabled: bool) -> Result<()> {
        if enabled && !self.append_storage_enabled().await? {
            self.seed_latest_states().await?;
        }
        // Scylla LWT batches are partition-scoped. Maintenance must drain
        // writers before changing this store-level marker.
        self.session
            .query_unpaged(
                format!(
                    "INSERT INTO {}_format (marker, enabled) VALUES ('append', ?)",
                    self.tables.version_commits_fq()
                ),
                (enabled,),
            )
            .await
            .map_err(|e| Error::Internal(format!("failed to update append storage marker: {e}")))?;
        Ok(())
    }

    async fn materialize_append_storage(&self, batch_size: usize) -> Result<u64> {
        self.materialize_compact_rows(batch_size).await
    }

    async fn stream_state(&self, app_id: &str, channel: &str) -> Result<VersionStreamState> {
        let commit_sql = format!(
            "SELECT next_delivery_serial FROM {} WHERE app_id = ? AND channel = ? AND commit_key = 's'",
            self.tables.version_commits_fq()
        );
        let commit_rows = self
            .session
            .query_unpaged(commit_sql, (app_id, channel))
            .await
            .map_err(|e| Error::Internal(format!("Failed to read atomic stream state: {e}")))?
            .into_rows_result()
            .map_err(|e| Error::Internal(format!("Failed to decode atomic stream state: {e}")))?;
        if let Some((next_serial,)) = commit_rows
            .maybe_first_row::<(i64,)>()
            .map_err(|e| Error::Internal(format!("Failed to deserialize atomic stream: {e}")))?
        {
            let bounds_sql = format!(
                "SELECT latest_delivery_serial FROM {} WHERE app_id = ? AND channel = ? AND commit_key >= 'd:' AND commit_key < 'e:'",
                self.tables.version_commits_fq()
            );
            let bounds = self
                .session
                .query_unpaged(bounds_sql, (app_id, channel))
                .await
                .map_err(|e| Error::Internal(format!("Failed to read delivery bounds: {e}")))?
                .into_rows_result()
                .map_err(|e| Error::Internal(format!("Failed to decode delivery bounds: {e}")))?;
            let serials = bounds
                .rows::<(i64,)>()
                .map_err(|e| Error::Internal(format!("Failed to stream delivery bounds: {e}")))?
                .collect::<std::result::Result<Vec<_>, _>>()
                .map_err(|e| Error::Internal(format!("Failed to collect delivery bounds: {e}")))?;
            return Ok(VersionStreamState {
                stream_id: Some(format!("{app_id}/{channel}")),
                next_delivery_serial: Some(next_serial as u64),
                oldest_available_delivery_serial: serials.first().map(|value| value.0 as u64),
                newest_available_delivery_serial: serials.last().map(|value| value.0 as u64),
            });
        }
        let sql = format!(
            "SELECT next_delivery_serial, oldest_available_delivery_serial, newest_available_delivery_serial FROM {} WHERE app_id = ? AND channel = ?",
            self.tables.version_streams_fq()
        );
        let rows = self
            .session
            .query_unpaged(sql.as_str(), (app_id, channel))
            .await
            .map_err(|e| Error::Internal(format!("Failed to read version stream state: {e}")))?
            .into_rows_result()
            .map_err(|e| Error::Internal(format!("Failed to decode version stream state: {e}")))?;

        let Some((next_serial, oldest, newest)) = rows
            .maybe_first_row::<(Option<i64>, Option<i64>, Option<i64>)>()
            .map_err(|e| {
                Error::Internal(format!("Failed to deserialize version stream state: {e}"))
            })?
        else {
            return Ok(VersionStreamState::default());
        };

        Ok(VersionStreamState {
            stream_id: Some(format!("{}/{}", app_id, channel)),
            next_delivery_serial: next_serial.map(|v| v as u64),
            oldest_available_delivery_serial: oldest.map(|v| v as u64),
            newest_available_delivery_serial: newest.map(|v| v as u64),
        })
    }
}

#[cfg(feature = "versioned-messages")]
impl ScyllaVersionStore {
    /// Write the legacy lookup projections and message pointer for an
    /// already-encoded payload (full for imports, compact for runs).
    async fn write_projections(&self, record: &StoredVersionRecord, payload: &[u8]) -> Result<()> {
        let now_ms = sockudo_core::history::now_ms();
        let payload_size = payload.len() as i64;

        // Write to both entry tables for the two query access patterns.
        let insert_by_msg = format!(
            "INSERT INTO {} (app_id, channel, message_serial, version_serial, delivery_serial, history_serial, action, client_id, description, event_name, payload_bytes, payload_size_bytes, version_timestamp_ms, created_at_ms) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?) IF NOT EXISTS{}",
            self.tables.version_entries_by_message_fq(),
            self.ttl_suffix(),
        );
        self.prepared
            .execute_unpaged(
                insert_by_msg.as_str(),
                (
                    &record.app_id,
                    &record.channel,
                    record.message_serial().as_str(),
                    record.version_serial().as_str(),
                    record.delivery_serial() as i64,
                    record.history_serial() as i64,
                    record.message.action.as_str(),
                    record.original_client_id.as_deref(),
                    record.message.version.description.as_deref(),
                    record.message.name.as_deref(),
                    payload,
                    payload_size,
                    record.message.version.timestamp_ms,
                    now_ms,
                ),
            )
            .await
            .map_err(|e| {
                Error::Internal(format!("Failed to insert version entry (by-message): {e}"))
            })?;
        #[cfg(test)]
        Self::measure_projection_entry(record, payload);

        let insert_by_delivery = format!(
            "INSERT INTO {} (app_id, channel, delivery_serial, message_serial, version_serial, history_serial, action, client_id, description, event_name, payload_bytes, payload_size_bytes, version_timestamp_ms, created_at_ms) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?) IF NOT EXISTS{}",
            self.tables.version_entries_by_delivery_fq(),
            self.ttl_suffix(),
        );
        self.prepared
            .execute_unpaged(
                insert_by_delivery.as_str(),
                (
                    &record.app_id,
                    &record.channel,
                    record.delivery_serial() as i64,
                    record.message_serial().as_str(),
                    record.version_serial().as_str(),
                    record.history_serial() as i64,
                    record.message.action.as_str(),
                    record.original_client_id.as_deref(),
                    record.message.version.description.as_deref(),
                    record.message.name.as_deref(),
                    payload,
                    payload_size,
                    record.message.version.timestamp_ms,
                    now_ms,
                ),
            )
            .await
            .map_err(|e| {
                Error::Internal(format!("Failed to insert version entry (by-delivery): {e}"))
            })?;
        #[cfg(test)]
        Self::measure_projection_entry(record, payload);

        // Upsert version_messages. ScyllaDB has no conditional upsert like SQL; use a LWT
        // to only advance if the new version_serial is greater than the stored one.
        let select_msg_q = format!(
            "SELECT latest_version_serial FROM {} WHERE app_id = ? AND channel = ? AND message_serial = ?",
            self.tables.version_messages_fq()
        );
        let insert_msg_q = format!(
            "INSERT INTO {} (app_id, channel, message_serial, history_serial, original_client_id, latest_version_serial, latest_delivery_serial, latest_action, created_at_ms, updated_at_ms) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?) IF NOT EXISTS{}",
            self.tables.version_messages_fq(),
            self.ttl_suffix(),
        );
        let update_msg_q = format!(
            "UPDATE {} {}SET latest_version_serial = ?, latest_delivery_serial = ?, latest_action = ?, updated_at_ms = ? WHERE app_id = ? AND channel = ? AND message_serial = ? IF latest_version_serial < ?",
            self.tables.version_messages_fq(),
            self.update_ttl_clause(),
        );

        let existing = self
            .prepared
            .execute_unpaged(
                select_msg_q.as_str(),
                (
                    &record.app_id,
                    &record.channel,
                    record.message_serial().as_str(),
                ),
            )
            .await
            .map_err(|e| Error::Internal(format!("Failed to read version message row: {e}")))?
            .into_rows_result()
            .map_err(|e| Error::Internal(format!("Failed to decode version message row: {e}")))?;

        if let Some(row) = existing
            .maybe_first_row::<(Option<String>,)>()
            .map_err(|e| Error::Internal(format!("Failed to deserialize version message: {e}")))?
        {
            let current_serial = row.0.unwrap_or_default();
            if record.version_serial().as_str() > current_serial.as_str() {
                let mut stmt = Statement::new(update_msg_q.clone());
                stmt.set_serial_consistency(Some(SerialConsistency::LocalSerial));
                self.prepared
                    .execute_unpaged(
                        stmt,
                        (
                            record.version_serial().as_str(),
                            record.delivery_serial() as i64,
                            record.message.action.as_str(),
                            now_ms,
                            &record.app_id,
                            &record.channel,
                            record.message_serial().as_str(),
                            record.version_serial().as_str(),
                        ),
                    )
                    .await
                    .map_err(|e| {
                        Error::Internal(format!("Failed to update version message: {e}"))
                    })?;
                #[cfg(test)]
                Self::measure_projection_latest(record);
            }
        } else {
            let mut stmt = Statement::new(insert_msg_q.clone());
            stmt.set_serial_consistency(Some(SerialConsistency::LocalSerial));
            self.prepared
                .execute_unpaged(
                    stmt,
                    (
                        &record.app_id,
                        &record.channel,
                        record.message_serial().as_str(),
                        record.history_serial() as i64,
                        record.original_client_id.as_deref(),
                        record.version_serial().as_str(),
                        record.delivery_serial() as i64,
                        record.message.action.as_str(),
                        now_ms,
                        now_ms,
                    ),
                )
                .await
                .map_err(|e| {
                    Error::Internal(format!("Failed to insert version message row: {e}"))
                })?;
        }

        // Update stream delivery window (best-effort, non-LWT).
        let update_stream = format!(
            "UPDATE {} SET updated_at_ms = ? WHERE app_id = ? AND channel = ?",
            self.tables.version_streams_fq()
        );
        self.prepared
            .execute_unpaged(
                update_stream.as_str(),
                (now_ms, &record.app_id, &record.channel),
            )
            .await
            .map_err(|e| {
                Error::Internal(format!("Failed to update version stream timestamp: {e}"))
            })?;

        Ok(())
    }
}
