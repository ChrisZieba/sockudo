use super::*;
use sockudo_core::version_store::append_storage::AppendRunPlan;

#[async_trait::async_trait]
impl VersionStore for PostgresVersionStore {
    async fn ensure_stream_id(&self, app_id: &str, channel: &str) -> Result<String> {
        Ok(format!("{app_id}/{channel}"))
    }

    async fn reserve_delivery_position(
        &self,
        app_id: &str,
        channel: &str,
    ) -> Result<VersionWriteReservation> {
        let now_ms = sockudo_core::history::now_ms();
        let sql = format!(
            r#"
            INSERT INTO {t} (app_id, channel, next_delivery_serial, updated_at_ms)
            VALUES ($1, $2, 2, $3)
            ON CONFLICT (app_id, channel) DO UPDATE SET
                next_delivery_serial = {t}.next_delivery_serial + 1,
                updated_at_ms = EXCLUDED.updated_at_ms
            RETURNING next_delivery_serial - 1 AS reserved_serial
            "#,
            t = self.tables.version_streams
        );
        let row = sqlx::query(sqlx::AssertSqlSafe(sql.as_str()))
            .bind(app_id)
            .bind(channel)
            .bind(now_ms)
            .fetch_one(&self.pool)
            .await
            .map_err(|e| {
                Error::Internal(format!("Failed to reserve version delivery position: {e}"))
            })?;

        Ok(VersionWriteReservation {
            stream_id: format!("{}/{}", app_id, channel),
            delivery_serial: row.get::<i64, _>("reserved_serial") as u64,
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
        let now_ms = sockudo_core::history::now_ms();
        let initial_next = block_size_i64.saturating_add(1);
        let sql = format!(
            r#"
            INSERT INTO {t} (app_id, channel, next_delivery_serial, updated_at_ms)
            VALUES ($1, $2, $3, $4)
            ON CONFLICT (app_id, channel) DO UPDATE SET
                next_delivery_serial = {t}.next_delivery_serial + $5,
                updated_at_ms = EXCLUDED.updated_at_ms
            RETURNING next_delivery_serial - $5 AS reserved_serial
            "#,
            t = self.tables.version_streams
        );
        let row = sqlx::query(sqlx::AssertSqlSafe(sql.as_str()))
            .bind(app_id)
            .bind(channel)
            .bind(initial_next)
            .bind(now_ms)
            .bind(block_size_i64)
            .fetch_one(&self.pool)
            .await
            .map_err(|e| {
                Error::Internal(format!(
                    "Failed to reserve version delivery position block: {e}"
                ))
            })?;

        Ok(VersionWriteReservationBlock {
            stream_id: format!("{}/{}", app_id, channel),
            start_delivery_serial: row.get::<i64, _>("reserved_serial") as u64,
            len: block_size,
        })
    }

    async fn append_version(&self, record: StoredVersionRecord) -> Result<()> {
        let mut tx = self.pool.begin().await.map_err(|e| {
            Error::Internal(format!("failed to begin version import transaction: {e}"))
        })?;
        let append_enabled = self.append_storage_enabled(&mut tx).await?;
        let now_ms = sockudo_core::history::now_ms();
        let payload = sonic_rs::to_vec(&record)
            .map_err(|e| Error::Internal(format!("Failed to serialize version record: {e}")))?;
        let payload_size = payload.len() as i64;

        let insert_entry = format!(
            r#"
            INSERT INTO {t} (
                app_id, channel, message_serial, version_serial, delivery_serial, history_serial,
                action, client_id, description, event_name,
                payload_bytes, payload_size_bytes, version_timestamp_ms, created_at_ms
            ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14)
            ON CONFLICT (app_id, channel, message_serial, version_serial) DO NOTHING
            "#,
            t = self.tables.version_entries
        );
        let inserted = sqlx::query(sqlx::AssertSqlSafe(insert_entry.as_str()))
            .bind(&record.app_id)
            .bind(&record.channel)
            .bind(record.message_serial().as_str())
            .bind(record.version_serial().as_str())
            .bind(record.delivery_serial() as i64)
            .bind(record.history_serial() as i64)
            .bind(record.message.action.as_str())
            .bind(record.original_client_id.as_deref())
            .bind(record.message.version.description.as_deref())
            .bind(record.message.name.as_deref())
            .bind(payload.as_slice())
            .bind(payload_size)
            .bind(record.message.version.timestamp_ms)
            .bind(now_ms)
            .execute(&mut *tx)
            .await
            .map_err(|e| Error::Internal(format!("Failed to insert version entry: {e}")))?;

        // Upsert version_messages: only advance if the incoming version_serial is lexicographically greater.
        let upsert_msg = format!(
            r#"
            INSERT INTO {t} (
                app_id, channel, message_serial, history_serial, original_client_id,
                latest_version_serial, latest_delivery_serial, latest_action,
                created_at_ms, updated_at_ms
            ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $9)
            ON CONFLICT (app_id, channel, message_serial) DO UPDATE SET
                latest_version_serial = EXCLUDED.latest_version_serial,
                latest_delivery_serial = EXCLUDED.latest_delivery_serial,
                latest_action = EXCLUDED.latest_action,
                updated_at_ms = EXCLUDED.updated_at_ms
            WHERE {t}.latest_version_serial < EXCLUDED.latest_version_serial
            "#,
            t = self.tables.version_messages
        );
        sqlx::query(sqlx::AssertSqlSafe(upsert_msg.as_str()))
            .bind(&record.app_id)
            .bind(&record.channel)
            .bind(record.message_serial().as_str())
            .bind(record.history_serial() as i64)
            .bind(record.original_client_id.as_deref())
            .bind(record.version_serial().as_str())
            .bind(record.delivery_serial() as i64)
            .bind(record.message.action.as_str())
            .bind(now_ms)
            .execute(&mut *tx)
            .await
            .map_err(|e| Error::Internal(format!("Failed to upsert version message: {e}")))?;

        // Update stream delivery window.
        let update_stream = format!(
            r#"
            UPDATE {t} SET
                oldest_available_delivery_serial = CASE
                    WHEN oldest_available_delivery_serial IS NULL OR $3 < oldest_available_delivery_serial
                    THEN $3 ELSE oldest_available_delivery_serial END,
                newest_available_delivery_serial = CASE
                    WHEN newest_available_delivery_serial IS NULL OR $3 > newest_available_delivery_serial
                    THEN $3 ELSE newest_available_delivery_serial END,
                updated_at_ms = $4
            WHERE app_id = $1 AND channel = $2
            "#,
            t = self.tables.version_streams
        );
        sqlx::query(sqlx::AssertSqlSafe(update_stream.as_str()))
            .bind(&record.app_id)
            .bind(&record.channel)
            .bind(record.delivery_serial() as i64)
            .bind(now_ms)
            .execute(&mut *tx)
            .await
            .map_err(|e| Error::Internal(format!("Failed to update version stream window: {e}")))?;

        if append_enabled && inserted.rows_affected() != 0 {
            self.seed_full_record(&mut tx, &record).await?;
        }
        tx.commit()
            .await
            .map_err(|e| Error::Internal(format!("failed to commit version import: {e}")))?;
        Ok(())
    }

    async fn commit_create(&self, request: VersionCreateRequest) -> Result<VersionCreateResult> {
        let mut tx = self.pool.begin().await.map_err(|e| {
            Error::Internal(format!("Failed to begin version create transaction: {e}"))
        })?;
        let append_enabled = self.append_storage_enabled(&mut tx).await?;
        let now_ms = sockudo_core::history::now_ms();
        let stream_id = format!("{}/{}", request.record.app_id, request.record.channel);
        let ensure_stream = format!(
            "INSERT INTO {} (app_id, channel, next_delivery_serial, updated_at_ms) VALUES ($1, $2, 1, $3) ON CONFLICT (app_id, channel) DO NOTHING",
            self.tables.version_streams
        );
        sqlx::query(sqlx::AssertSqlSafe(ensure_stream.as_str()))
            .bind(&request.record.app_id)
            .bind(&request.record.channel)
            .bind(now_ms)
            .execute(&mut *tx)
            .await
            .map_err(|e| Error::Internal(format!("Failed to initialize version stream: {e}")))?;
        let lock_stream = format!(
            "SELECT next_delivery_serial FROM {} WHERE app_id = $1 AND channel = $2 FOR UPDATE",
            self.tables.version_streams
        );
        let stream = sqlx::query(sqlx::AssertSqlSafe(lock_stream.as_str()))
            .bind(&request.record.app_id)
            .bind(&request.record.channel)
            .fetch_one(&mut *tx)
            .await
            .map_err(|e| Error::Internal(format!("Failed to lock version stream: {e}")))?;

        let latest_sql = format!(
            "SELECT payload_bytes FROM {} WHERE app_id = $1 AND channel = $2 AND message_serial = $3 ORDER BY version_serial DESC LIMIT 1",
            self.tables.version_entries
        );
        if let Some(row) = sqlx::query(sqlx::AssertSqlSafe(latest_sql.as_str()))
            .bind(&request.record.app_id)
            .bind(&request.record.channel)
            .bind(request.record.message_serial().as_str())
            .fetch_optional(&mut *tx)
            .await
            .map_err(|e| Error::Internal(format!("Failed to check version create target: {e}")))?
        {
            let payload: Vec<u8> = row.get("payload_bytes");
            let current = self
                .materialize_payloads(
                    &mut *tx,
                    &request.record.app_id,
                    &request.record.channel,
                    vec![payload],
                )
                .await?
                .pop();
            return Ok(VersionCreateResult::Conflict { current });
        }

        if let Some(limit) = request.limits.max_accumulated_message_bytes
            && request.record.data_bytes()? > limit
        {
            return Ok(VersionCreateResult::Rejected(
                VersionCreateRejection::AccumulatedMessageBytes { limit },
            ));
        }
        if request.record.is_open_ai_stream()
            && let Some(limit) = request.limits.max_open_streaming_messages_per_channel
        {
            let count_sql = format!(
                "SELECT COUNT(*) AS count FROM {} WHERE app_id = $1 AND channel = $2 AND is_open_stream = TRUE",
                self.tables.version_messages
            );
            let count = sqlx::query(sqlx::AssertSqlSafe(count_sql.as_str()))
                .bind(&request.record.app_id)
                .bind(&request.record.channel)
                .fetch_one(&mut *tx)
                .await
                .map_err(|e| Error::Internal(format!("Failed to count open streams: {e}")))?
                .get::<i64, _>("count") as usize;
            if count >= limit {
                return Ok(VersionCreateResult::Rejected(
                    VersionCreateRejection::OpenStreamingMessages { limit },
                ));
            }
        }

        let delivery_serial = stream.get::<i64, _>("next_delivery_serial") as u64;
        let record = request
            .record
            .with_delivery_position(&stream_id, delivery_serial);
        let payload = sonic_rs::to_vec(&record)
            .map_err(|e| Error::Internal(format!("Failed to serialize version record: {e}")))?;
        let insert_entry = format!(
            "INSERT INTO {} (app_id, channel, message_serial, version_serial, delivery_serial, history_serial, action, client_id, description, event_name, payload_bytes, payload_size_bytes, version_timestamp_ms, created_at_ms, operation_key, operation_fingerprint) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16)",
            self.tables.version_entries
        );
        sqlx::query(sqlx::AssertSqlSafe(insert_entry.as_str()))
            .bind(&record.app_id)
            .bind(&record.channel)
            .bind(record.message_serial().as_str())
            .bind(record.version_serial().as_str())
            .bind(delivery_serial as i64)
            .bind(record.history_serial() as i64)
            .bind(record.message.action.as_str())
            .bind(record.message.version.client_id.as_deref())
            .bind(record.message.version.description.as_deref())
            .bind(record.message.name.as_deref())
            .bind(payload.as_slice())
            .bind(payload.len() as i64)
            .bind(record.message.version.timestamp_ms)
            .bind(now_ms)
            .bind(None::<&str>)
            .bind(None::<&str>)
            .execute(&mut *tx)
            .await
            .map_err(|e| Error::Internal(format!("Failed to insert create version: {e}")))?;
        if append_enabled {
            self.seed_full_record(&mut tx, &record).await?;
        }
        let insert_message = format!(
            "INSERT INTO {} (app_id, channel, message_serial, history_serial, original_client_id, latest_version_serial, latest_delivery_serial, latest_action, is_open_stream, created_at_ms, updated_at_ms) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$10)",
            self.tables.version_messages
        );
        sqlx::query(sqlx::AssertSqlSafe(insert_message.as_str()))
            .bind(&record.app_id)
            .bind(&record.channel)
            .bind(record.message_serial().as_str())
            .bind(record.history_serial() as i64)
            .bind(record.original_client_id.as_deref())
            .bind(record.version_serial().as_str())
            .bind(delivery_serial as i64)
            .bind(record.message.action.as_str())
            .bind(record.is_open_ai_stream())
            .bind(now_ms)
            .execute(&mut *tx)
            .await
            .map_err(|e| Error::Internal(format!("Failed to insert version message: {e}")))?;
        let update_stream = format!(
            "UPDATE {} SET next_delivery_serial = $3, oldest_available_delivery_serial = COALESCE(oldest_available_delivery_serial, $4), newest_available_delivery_serial = $4, updated_at_ms = $5 WHERE app_id = $1 AND channel = $2",
            self.tables.version_streams
        );
        sqlx::query(sqlx::AssertSqlSafe(update_stream.as_str()))
            .bind(&record.app_id)
            .bind(&record.channel)
            .bind((delivery_serial + 1) as i64)
            .bind(delivery_serial as i64)
            .bind(now_ms)
            .execute(&mut *tx)
            .await
            .map_err(|e| Error::Internal(format!("Failed to commit version stream: {e}")))?;
        tx.commit().await.map_err(|e| {
            Error::Internal(format!("Failed to commit version create transaction: {e}"))
        })?;
        Ok(VersionCreateResult::Applied { record, stream_id })
    }

    async fn compare_and_apply(
        &self,
        request: VersionMutationRequest,
    ) -> Result<VersionMutationResult> {
        #[cfg(test)]
        let wire_before = crate::history::c2_wire_meter::phase_snapshot();
        let mut tx = self.pool.begin().await.map_err(|e| {
            Error::Internal(format!("Failed to begin version mutation transaction: {e}"))
        })?;
        let append_enabled = self.append_storage_enabled(&mut tx).await?;
        #[cfg(test)]
        crate::history::c2_wire_meter::report_phase("postgres.begin_marker", wire_before);
        #[cfg(test)]
        let wire_before = crate::history::c2_wire_meter::phase_snapshot();

        let stream_id = format!("{}/{}", request.app_id, request.channel);
        let lock_stream = format!(
            "SELECT next_delivery_serial FROM {} WHERE app_id = $1 AND channel = $2 FOR UPDATE",
            self.tables.version_streams
        );
        let Some(stream) = sqlx::query(sqlx::AssertSqlSafe(lock_stream.as_str()))
            .bind(&request.app_id)
            .bind(&request.channel)
            .fetch_optional(&mut *tx)
            .await
            .map_err(|e| Error::Internal(format!("Failed to lock version stream: {e}")))?
        else {
            return Ok(VersionMutationResult::Conflict { current: None });
        };

        #[cfg(test)]
        crate::history::c2_wire_meter::report_phase("postgres.stream_lock", wire_before);
        if let Some(operation) = request.idempotency.as_ref() {
            let operation_sql = format!(
                "SELECT payload_bytes, operation_fingerprint FROM {} WHERE app_id = $1 AND channel = $2 AND operation_key = $3 LIMIT 1",
                self.tables.version_entries
            );
            if let Some(row) = sqlx::query(sqlx::AssertSqlSafe(operation_sql.as_str()))
                .bind(&request.app_id)
                .bind(&request.channel)
                .bind(&operation.cache_key)
                .fetch_optional(&mut *tx)
                .await
                .map_err(|e| Error::Internal(format!("Failed to read mutation receipt: {e}")))?
            {
                let fingerprint: Option<String> = row.get("operation_fingerprint");
                if fingerprint.as_deref() != Some(operation.payload_fingerprint.as_str()) {
                    return Err(Error::IdempotencyConflict);
                }
                let payload: Vec<u8> = row.get("payload_bytes");
                let record = self
                    .materialize_payloads(
                        &mut *tx,
                        &request.app_id,
                        &request.channel,
                        vec![payload],
                    )
                    .await?
                    .pop()
                    .ok_or_else(|| Error::Internal("mutation receipt is empty".to_string()))?;
                return Ok(VersionMutationResult::Duplicate { record, stream_id });
            }
        }

        // The predecessor and its run snapshot come from one statement. The
        // stream lock serializes mutations; the snapshot write below re-checks
        // the head, so a concurrent purge can only abort this commit.
        #[cfg(test)]
        let wire_before = crate::history::c2_wire_meter::phase_snapshot();
        let latest_sql = self.with_append_runs(&format!(
            "SELECT app_id, channel, message_serial, append_run, append_len, payload_bytes, 1::bigint AS ord FROM {} WHERE app_id = $1 AND channel = $2 AND message_serial = $3 ORDER BY version_serial DESC LIMIT 1",
            self.tables.version_entries
        ));
        let rows = sqlx::query(sqlx::AssertSqlSafe(latest_sql.as_str()))
            .bind(&request.app_id)
            .bind(&request.channel)
            .bind(request.message_serial.as_str())
            .fetch_all(&mut *tx)
            .await
            .map_err(|e| Error::Internal(format!("Failed to read mutation predecessor: {e}")))?;
        #[cfg(test)]
        crate::history::c2_wire_meter::report_phase("postgres.latest", wire_before);
        let mut page = Self::materialize_page(rows)?;
        let (Some(current), Some(run)) = (page.records.pop(), page.runs.pop()) else {
            return Ok(VersionMutationResult::Conflict { current: None });
        };
        // Extend only when the predecessor is still the run head.
        let predecessor_run = run.filter(|run| {
            page.heads
                .get(&(request.message_serial.clone(), run.run.clone()))
                .is_some_and(|head| {
                    &head.head == current.version_serial()
                        && head.data_len == run.data_len
                        && head.generation == run.generation
                })
        });
        // The database maintains this retained-row count in the entry's own
        // transaction, including legacy imports and retention deletes.
        let append_count = if matches!(
            request.mutation,
            sockudo_core::version_store::VersionMutation::Append(_)
        ) && request.limits.max_appends_per_message.is_some()
        {
            let sql = format!(
                "SELECT append_count FROM {}_ac WHERE app_id = $1 AND channel = $2 AND message_serial = $3",
                self.tables.version_entries
            );
            let count = sqlx::query_scalar::<_, i64>(sqlx::AssertSqlSafe(sql.as_str()))
                .bind(&request.app_id)
                .bind(&request.channel)
                .bind(request.message_serial.as_str())
                .fetch_optional(&mut *tx)
                .await
                .map_err(|e| Error::Internal(format!("failed to read message append count: {e}")))?
                .unwrap_or(0);
            usize::try_from(count)
                .map_err(|_| Error::Internal("invalid retained append count".into()))?
        } else {
            0
        };
        let delivery_serial = (stream.get::<i64, _>("next_delivery_serial") as u64)
            .max(current.delivery_serial().saturating_add(1));
        let outcome = request.apply_to(&current, &stream_id, delivery_serial, append_count)?;
        let VersionMutationResult::Applied { record, .. } = outcome else {
            return Ok(outcome);
        };
        if !current.is_open_ai_stream()
            && record.is_open_ai_stream()
            && let Some(limit) = request.limits.max_open_streaming_messages_per_channel
        {
            let count_sql = format!(
                "SELECT COUNT(*) AS count FROM {} WHERE app_id = $1 AND channel = $2 AND is_open_stream = TRUE",
                self.tables.version_messages
            );
            let count = sqlx::query(sqlx::AssertSqlSafe(count_sql.as_str()))
                .bind(&request.app_id)
                .bind(&request.channel)
                .fetch_one(&mut *tx)
                .await
                .map_err(|e| Error::Internal(format!("Failed to count open streams: {e}")))?
                .get::<i64, _>("count") as usize;
            if count >= limit {
                return Ok(VersionMutationResult::Rejected(
                    sockudo_core::version_store::VersionMutationRejection::OpenStreamingMessages {
                        limit,
                    },
                ));
            }
        }
        let plan = if append_enabled {
            AppendRunPlan::for_record_chunked(
                current.version_serial(),
                predecessor_run.as_ref(),
                &record,
            )
        } else {
            AppendRunPlan::Full
        };
        let payload = plan.encode(&record)?;
        let operation_key = request
            .idempotency
            .as_ref()
            .map(|value| value.cache_key.as_str());
        let operation_fingerprint = request
            .idempotency
            .as_ref()
            .map(|value| value.payload_fingerprint.as_str());
        let now_ms = sockudo_core::history::now_ms();
        #[cfg(test)]
        let wire_before = crate::history::c2_wire_meter::phase_snapshot();
        let insert_entry = format!(
            "INSERT INTO {} (app_id, channel, message_serial, version_serial, delivery_serial, history_serial, action, client_id, description, event_name, payload_bytes, payload_size_bytes, version_timestamp_ms, created_at_ms, operation_key, operation_fingerprint, append_run, append_len) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17,$18)",
            self.tables.version_entries
        );
        sqlx::query(sqlx::AssertSqlSafe(insert_entry.as_str()))
            .bind(&record.app_id)
            .bind(&record.channel)
            .bind(record.message_serial().as_str())
            .bind(record.version_serial().as_str())
            .bind(delivery_serial as i64)
            .bind(record.history_serial() as i64)
            .bind(record.message.action.as_str())
            .bind(record.message.version.client_id.as_deref())
            .bind(record.message.version.description.as_deref())
            .bind(record.message.name.as_deref())
            .bind(payload.as_slice())
            .bind(payload.len() as i64)
            .bind(record.message.version.timestamp_ms)
            .bind(now_ms)
            .bind(operation_key)
            .bind(operation_fingerprint)
            .bind(plan.run().map(|run| run.run.as_str()))
            .bind(plan.run().map(|run| run.data_len as i64))
            .execute(&mut *tx)
            .await
            .map_err(|e| Error::Internal(format!("Failed to insert mutation version: {e}")))?;
        #[cfg(test)]
        crate::history::c2_wire_meter::report_phase("postgres.entry", wire_before);
        #[cfg(test)]
        let wire_before = crate::history::c2_wire_meter::phase_snapshot();
        self.write_append_run(&mut tx, &record, &plan, now_ms)
            .await?;
        if append_enabled && matches!(plan, AppendRunPlan::Full) {
            self.seed_full_record(&mut tx, &record).await?;
        }
        #[cfg(test)]
        crate::history::c2_wire_meter::report_phase("postgres.run_chunks", wire_before);
        #[cfg(test)]
        let wire_before = crate::history::c2_wire_meter::phase_snapshot();
        let update_message = format!(
            "UPDATE {} SET latest_version_serial = $4, latest_delivery_serial = $5, latest_action = $6, is_open_stream = $7, updated_at_ms = $8 WHERE app_id = $1 AND channel = $2 AND message_serial = $3",
            self.tables.version_messages
        );
        sqlx::query(sqlx::AssertSqlSafe(update_message.as_str()))
            .bind(&record.app_id)
            .bind(&record.channel)
            .bind(record.message_serial().as_str())
            .bind(record.version_serial().as_str())
            .bind(delivery_serial as i64)
            .bind(record.message.action.as_str())
            .bind(record.is_open_ai_stream())
            .bind(now_ms)
            .execute(&mut *tx)
            .await
            .map_err(|e| Error::Internal(format!("Failed to advance version message: {e}")))?;
        #[cfg(test)]
        crate::history::c2_wire_meter::report_phase("postgres.message", wire_before);
        #[cfg(test)]
        let wire_before = crate::history::c2_wire_meter::phase_snapshot();
        let update_stream = format!(
            "UPDATE {} SET next_delivery_serial = $3, oldest_available_delivery_serial = COALESCE(oldest_available_delivery_serial, $4), newest_available_delivery_serial = GREATEST(COALESCE(newest_available_delivery_serial, $4), $4), updated_at_ms = $5 WHERE app_id = $1 AND channel = $2",
            self.tables.version_streams
        );
        sqlx::query(sqlx::AssertSqlSafe(update_stream.as_str()))
            .bind(&record.app_id)
            .bind(&record.channel)
            .bind((delivery_serial + 1) as i64)
            .bind(delivery_serial as i64)
            .bind(now_ms)
            .execute(&mut *tx)
            .await
            .map_err(|e| Error::Internal(format!("Failed to advance version stream: {e}")))?;
        #[cfg(test)]
        crate::history::c2_wire_meter::report_phase("postgres.stream", wire_before);
        #[cfg(test)]
        let wire_before = crate::history::c2_wire_meter::phase_snapshot();
        tx.commit().await.map_err(|e| {
            Error::Internal(format!(
                "Failed to commit version mutation transaction: {e}"
            ))
        })?;
        #[cfg(test)]
        crate::history::c2_wire_meter::report_phase("postgres.commit", wire_before);
        Ok(VersionMutationResult::Applied { record, stream_id })
    }

    async fn get_latest(
        &self,
        app_id: &str,
        channel: &str,
        message_serial: &sockudo_core::versioned_messages::MessageSerial,
    ) -> Result<Option<StoredVersionRecord>> {
        let sql = self.with_append_runs(&format!(
            "SELECT app_id, channel, message_serial, append_run, append_len, payload_bytes, 1::bigint AS ord FROM {} WHERE app_id = $1 AND channel = $2 AND message_serial = $3 ORDER BY version_serial DESC LIMIT 1",
            self.tables.version_entries
        ));
        let rows = sqlx::query(sqlx::AssertSqlSafe(sql.as_str()))
            .bind(app_id)
            .bind(channel)
            .bind(message_serial.as_str())
            .fetch_all(&self.pool)
            .await
            .map_err(|e| Error::Internal(format!("Failed to query latest version: {e}")))?;
        Ok(Self::materialize_page(rows)?.records.pop())
    }

    async fn get_versions(&self, request: VersionStoreReadRequest) -> Result<VersionStorePage> {
        request.validate()?;
        let fetch_limit = (request.limit + 1) as i64;

        let (order_dir, cursor_op) = match request.direction {
            VersionStoreDirection::NewestFirst => ("DESC", "<"),
            VersionStoreDirection::OldestFirst => ("ASC", ">"),
        };
        // The page is limited before numbering, so the window never scans
        // more than the requested rows.
        let page = |filter: &str, limit: &str| {
            self.with_append_runs(&format!(
                "SELECT *, ROW_NUMBER() OVER (ORDER BY version_serial {order_dir}) AS ord FROM (SELECT app_id, channel, message_serial, version_serial, append_run, append_len, payload_bytes FROM {} WHERE app_id = $1 AND channel = $2 AND message_serial = $3{filter} ORDER BY version_serial {order_dir} LIMIT {limit}) limited",
                self.tables.version_entries
            ))
        };

        let rows = if let Some(cursor) = &request.cursor {
            let sql = page(&format!(" AND version_serial {cursor_op} $4"), "$5");
            sqlx::query(sqlx::AssertSqlSafe(sql.as_str()))
                .bind(&request.app_id)
                .bind(&request.channel)
                .bind(request.message_serial.as_str())
                .bind(cursor.version_serial.as_str())
                .bind(fetch_limit)
                .fetch_all(&self.pool)
                .await
                .map_err(|e| Error::Internal(format!("Failed to query version history: {e}")))?
        } else {
            let sql = page("", "$4");
            sqlx::query(sqlx::AssertSqlSafe(sql.as_str()))
                .bind(&request.app_id)
                .bind(&request.channel)
                .bind(request.message_serial.as_str())
                .bind(fetch_limit)
                .fetch_all(&self.pool)
                .await
                .map_err(|e| Error::Internal(format!("Failed to query version history: {e}")))?
        };

        let mut items = Self::materialize_page(rows)?.records;
        let has_more = items.len() > request.limit;
        items.truncate(request.limit);

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
        let sql = self.with_append_runs(&format!(
            "SELECT *, ROW_NUMBER() OVER (ORDER BY delivery_serial ASC) AS ord FROM (SELECT app_id, channel, message_serial, delivery_serial, append_run, append_len, payload_bytes FROM {} WHERE app_id = $1 AND channel = $2 AND delivery_serial > $3 ORDER BY delivery_serial ASC LIMIT $4) limited",
            self.tables.version_entries
        ));
        let rows = sqlx::query(sqlx::AssertSqlSafe(sql.as_str()))
            .bind(&request.app_id)
            .bind(&request.channel)
            .bind(request.after_delivery_serial as i64)
            .bind(request.limit as i64)
            .fetch_all(&self.pool)
            .await
            .map_err(|e| Error::Internal(format!("Failed to replay version entries: {e}")))?;
        Ok(Self::materialize_page(rows)?.records)
    }

    async fn latest_by_history(
        &self,
        app_id: &str,
        channel: &str,
    ) -> Result<Vec<StoredVersionRecord>> {
        let sql = self.with_append_runs(&format!(
            r#"
            SELECT ve.app_id, ve.channel, ve.message_serial, ve.append_run, ve.append_len, ve.payload_bytes,
                ROW_NUMBER() OVER (ORDER BY vm.history_serial ASC) AS ord
            FROM {vm} vm
            JOIN {ve} ve ON ve.app_id = vm.app_id
                AND ve.channel = vm.channel
                AND ve.message_serial = vm.message_serial
                AND ve.version_serial = vm.latest_version_serial
            WHERE vm.app_id = $1 AND vm.channel = $2
            "#,
            vm = self.tables.version_messages,
            ve = self.tables.version_entries
        ));
        let rows = sqlx::query(sqlx::AssertSqlSafe(sql.as_str()))
            .bind(app_id)
            .bind(channel)
            .fetch_all(&self.pool)
            .await
            .map_err(|e| Error::Internal(format!("Failed to query latest by history: {e}")))?;
        Ok(Self::materialize_page(rows)?.records)
    }

    async fn stream_state(&self, app_id: &str, channel: &str) -> Result<VersionStreamState> {
        let sql = format!(
            "SELECT next_delivery_serial, oldest_available_delivery_serial, newest_available_delivery_serial FROM {} WHERE app_id = $1 AND channel = $2",
            self.tables.version_streams
        );
        let row = sqlx::query(sqlx::AssertSqlSafe(sql.as_str()))
            .bind(app_id)
            .bind(channel)
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| Error::Internal(format!("Failed to read version stream state: {e}")))?;

        match row {
            None => Ok(VersionStreamState::default()),
            Some(row) => Ok(VersionStreamState {
                stream_id: Some(format!("{}/{}", app_id, channel)),
                next_delivery_serial: Some(row.get::<i64, _>("next_delivery_serial") as u64),
                oldest_available_delivery_serial: row
                    .try_get::<Option<i64>, _>("oldest_available_delivery_serial")
                    .unwrap_or(None)
                    .map(|v| v as u64),
                newest_available_delivery_serial: row
                    .try_get::<Option<i64>, _>("newest_available_delivery_serial")
                    .unwrap_or(None)
                    .map(|v| v as u64),
            }),
        }
    }

    async fn purge_before(&self, before_ms: i64, batch_size: usize) -> Result<(u64, bool)> {
        if batch_size == 0 {
            return Ok((0, false));
        }
        let limit = batch_size as i64;

        let entries_sql = format!(
            "DELETE FROM {0} WHERE ctid IN (SELECT ctid FROM {0} WHERE created_at_ms < $1 ORDER BY created_at_ms ASC LIMIT $2)",
            self.tables.version_entries
        );
        let entries_deleted = sqlx::query(sqlx::AssertSqlSafe(entries_sql.as_str()))
            .bind(before_ms)
            .bind(limit)
            .execute(&self.pool)
            .await
            .map_err(|e| Error::Internal(format!("Failed to purge version entries: {e}")))?
            .rows_affected();

        let messages_sql = format!(
            "DELETE FROM {0} WHERE ctid IN (SELECT ctid FROM {0} WHERE updated_at_ms < $1 ORDER BY updated_at_ms ASC LIMIT $2)",
            self.tables.version_messages
        );
        let messages_deleted = sqlx::query(sqlx::AssertSqlSafe(messages_sql.as_str()))
            .bind(before_ms)
            .bind(limit)
            .execute(&self.pool)
            .await
            .map_err(|e| Error::Internal(format!("Failed to purge version messages: {e}")))?
            .rows_affected();

        // Runs go last: a run outlives every retained entry inside it.
        let runs_deleted = self.purge_append_runs(before_ms, limit).await?;

        let deleted = entries_deleted + messages_deleted + runs_deleted;
        let has_more = entries_deleted as i64 == limit
            || messages_deleted as i64 == limit
            || runs_deleted as i64 == limit;
        Ok((deleted, has_more))
    }

    async fn set_append_storage_enabled(&self, enabled: bool) -> Result<()> {
        self.update_append_storage_marker(enabled).await
    }

    async fn materialize_append_storage(&self, batch_size: usize) -> Result<u64> {
        self.update_append_storage_marker(false).await?;
        self.materialize_compact_entries(batch_size).await
    }
}
