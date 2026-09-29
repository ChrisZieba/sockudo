use super::*;
use sockudo_core::version_store::append_storage::{
    AppendRunPlan, AppendRunRef, StoredVersionPayload, encode_full,
};
use sockudo_core::versioned_messages::VersionSerial;

#[cfg(feature = "versioned-messages")]
#[async_trait::async_trait]
impl VersionStore for DynamoDbVersionStore {
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
        let app_channel = Self::app_channel_key(app_id, channel);
        loop {
            let existing = self
                .client
                .get_item()
                .table_name(&self.tables.version_streams)
                .key("app_channel", Self::attr_s(&app_channel))
                .send()
                .await
                .map_err(|e| {
                    Error::Internal(format!("Failed to read version stream from DynamoDB: {e}"))
                })?
                .item;

            let now_ms = sockudo_core::history::now_ms();

            if let Some(item) = existing {
                let current = Self::item_num(&item, "next_delivery_serial").unwrap_or(1) as u64;
                let next = current.saturating_add(block_size);
                let result = self
                    .client
                    .update_item()
                    .table_name(&self.tables.version_streams)
                    .key("app_channel", Self::attr_s(&app_channel))
                    .update_expression("SET next_delivery_serial = :next, updated_at_ms = :now")
                    .condition_expression("next_delivery_serial = :expected")
                    .expression_attribute_values(":next", Self::attr_n(next))
                    .expression_attribute_values(":expected", Self::attr_n(current))
                    .expression_attribute_values(":now", Self::attr_n(now_ms))
                    .send()
                    .await;
                match result {
                    Ok(_) => {
                        return Ok(VersionWriteReservationBlock {
                            stream_id: format!("{}/{}", app_id, channel),
                            start_delivery_serial: current,
                            len: block_size,
                        });
                    }
                    Err(e)
                        if e.as_service_error()
                            .is_some_and(|error| error.is_conditional_check_failed_exception()) =>
                    {
                        continue;
                    }
                    Err(e) => {
                        return Err(Error::Internal(format!(
                            "Failed to advance DynamoDB version delivery serial: {e}"
                        )));
                    }
                }
            } else {
                let mut new_item = HashMap::new();
                new_item.insert("app_channel".to_string(), Self::attr_s(&app_channel));
                new_item.insert("app_id".to_string(), Self::attr_s(app_id));
                new_item.insert("channel".to_string(), Self::attr_s(channel));
                new_item.insert(
                    "next_delivery_serial".to_string(),
                    Self::attr_n(block_size.saturating_add(1)),
                );
                new_item.insert("migration_state".to_string(), Self::attr_s("native_only"));
                new_item.insert("updated_at_ms".to_string(), Self::attr_n(now_ms));

                let create_result = self
                    .client
                    .put_item()
                    .table_name(&self.tables.version_streams)
                    .set_item(Some(new_item))
                    .condition_expression("attribute_not_exists(app_channel)")
                    .send()
                    .await;
                match create_result {
                    Ok(_) => {
                        return Ok(VersionWriteReservationBlock {
                            stream_id: format!("{}/{}", app_id, channel),
                            start_delivery_serial: 1,
                            len: block_size,
                        });
                    }
                    Err(e)
                        if e.as_service_error()
                            .is_some_and(|error| error.is_conditional_check_failed_exception()) =>
                    {
                        continue;
                    }
                    Err(e) => {
                        return Err(Error::Internal(format!(
                            "Failed to create DynamoDB version stream row: {e}"
                        )));
                    }
                }
            }
        }
    }

    async fn append_version(&self, record: StoredVersionRecord) -> Result<()> {
        let now_ms = sockudo_core::history::now_ms();
        let payload = sonic_rs::to_vec(&record)
            .map_err(|e| Error::Internal(format!("Failed to serialize version record: {e}")))?;
        let app_channel = Self::app_channel_key(&record.app_id, &record.channel);
        let app_channel_message = Self::app_channel_message_key(
            &record.app_id,
            &record.channel,
            record.message_serial().as_str(),
        );
        let message_version_key = Self::message_version_key(
            record.message_serial().as_str(),
            record.version_serial().as_str(),
        );

        // Write the entry (idempotent via condition).
        let mut entry_item = HashMap::new();
        entry_item.insert("app_channel".to_string(), Self::attr_s(&app_channel));
        entry_item.insert(
            "message_version_key".to_string(),
            Self::attr_s(&message_version_key),
        );
        entry_item.insert(
            "app_channel_message".to_string(),
            Self::attr_s(&app_channel_message),
        );
        entry_item.insert("app_id".to_string(), Self::attr_s(&record.app_id));
        entry_item.insert("channel".to_string(), Self::attr_s(&record.channel));
        entry_item.insert(
            "message_serial".to_string(),
            Self::attr_s(record.message_serial().as_str()),
        );
        entry_item.insert(
            "version_serial".to_string(),
            Self::attr_s(record.version_serial().as_str()),
        );
        entry_item.insert(
            "delivery_serial".to_string(),
            Self::attr_n(record.delivery_serial()),
        );
        entry_item.insert(
            "history_serial".to_string(),
            Self::attr_n(record.history_serial()),
        );
        entry_item.insert(
            "action".to_string(),
            Self::attr_s(record.message.action.as_str()),
        );
        entry_item.insert("payload_bytes".to_string(), Self::attr_b(payload.clone()));
        entry_item.insert("created_at_ms".to_string(), Self::attr_n(now_ms));
        if let Some(expires_at) = self.expires_at_value() {
            entry_item.insert(Self::EXPIRES_AT_ATTR.to_string(), expires_at);
        }

        let put_result = self
            .client
            .put_item()
            .table_name(&self.tables.version_entries)
            .set_item(Some(entry_item))
            .condition_expression("attribute_not_exists(message_version_key)")
            .send()
            .await;
        if let Err(e) = put_result
            && !e
                .as_service_error()
                .is_some_and(|error| error.is_conditional_check_failed_exception())
        {
            return Err(Error::Internal(format!(
                "Failed to write version entry to DynamoDB: {e}"
            )));
        }
        // Duplicate version entry — idempotent, continue.

        // Advance version_messages if this version_serial is greater.
        let (update_expr, expires_value) = if let Some(expires) = self.expires_at_value() {
            (
                "SET latest_version_serial = :vs, latest_delivery_serial = :ds, latest_action = :action, latest_payload_bytes = :payload, is_open_stream = :is_open, append_count = if_not_exists(append_count, :zero) + :append_increment, updated_at_ms = :now, history_serial = :hs, original_client_id = :oc, created_at_ms = if_not_exists(created_at_ms, :now), expires_at = :exp REMOVE latest_append_run, latest_append_len, latest_append_head, latest_append_pinned, latest_append_generation",
                Some(expires),
            )
        } else {
            (
                "SET latest_version_serial = :vs, latest_delivery_serial = :ds, latest_action = :action, latest_payload_bytes = :payload, is_open_stream = :is_open, append_count = if_not_exists(append_count, :zero) + :append_increment, updated_at_ms = :now, history_serial = :hs, original_client_id = :oc, created_at_ms = if_not_exists(created_at_ms, :now) REMOVE latest_append_run, latest_append_len, latest_append_head, latest_append_pinned, latest_append_generation",
                None,
            )
        };
        let seed = if self.append_storage_epoch().await?.is_some() {
            self.stage_seed(&record).await?
        } else {
            AppendRunPlan::Full
        };
        let update_expr = if seed.run().is_some() {
            update_expr
                .split(" REMOVE ")
                .next()
                .unwrap_or(update_expr)
                .to_string()
                + ", latest_append_run = :seed_run, latest_append_len = :seed_len, latest_append_head = :vs, latest_append_generation = :seed_generation, latest_append_pinned = :seed_pinned"
        } else {
            update_expr.to_string()
        };
        let mut update_builder = Update::builder()
            .table_name(&self.tables.version_messages)
            .key("app_channel", Self::attr_s(&app_channel))
            .key(
                "message_serial",
                Self::attr_s(record.message_serial().as_str()),
            )
            .update_expression(update_expr)
            .condition_expression(
                "attribute_not_exists(latest_version_serial) OR latest_version_serial < :vs",
            )
            .expression_attribute_values(":vs", Self::attr_s(record.version_serial().as_str()))
            .expression_attribute_values(":ds", Self::attr_n(record.delivery_serial()))
            .expression_attribute_values(":action", Self::attr_s(record.message.action.as_str()))
            .expression_attribute_values(":payload", Self::attr_b(payload.clone()))
            .expression_attribute_values(
                ":is_open",
                AttributeValue::Bool(record.is_open_ai_stream()),
            )
            .expression_attribute_values(":zero", Self::attr_n(0))
            .expression_attribute_values(
                ":append_increment",
                Self::attr_n(usize::from(
                    record.message.action
                        == sockudo_core::versioned_messages::MessageAction::Append,
                )),
            )
            .expression_attribute_values(":now", Self::attr_n(now_ms))
            .expression_attribute_values(":hs", Self::attr_n(record.history_serial()))
            .expression_attribute_values(
                ":oc",
                record
                    .original_client_id
                    .as_deref()
                    .map(Self::attr_s)
                    .unwrap_or(AttributeValue::Null(true)),
            );
        if let Some(run) = seed.run() {
            update_builder = update_builder
                .expression_attribute_values(":seed_run", Self::attr_s(run.run.as_str()))
                .expression_attribute_values(":seed_len", Self::attr_n(run.data_len))
                .expression_attribute_values(
                    ":seed_generation",
                    Self::attr_s(run.generation.as_deref().unwrap_or_default()),
                )
                .expression_attribute_values(":seed_pinned", AttributeValue::Bool(true));
        }
        if let Some(expires) = expires_value {
            update_builder = update_builder.expression_attribute_values(":exp", expires);
        }
        let update = update_builder
            .build()
            .map_err(|e| Error::Internal(format!("failed to build imported latest state: {e}")))?;
        let mut transaction = self
            .client
            .transact_write_items()
            .transact_items(TransactWriteItem::builder().update(update).build());
        if let Some(activation) = self.seed_activation_write(&record, &seed)? {
            transaction = transaction.transact_items(activation);
        }
        if let Err(error) = transaction.send().await {
            if error
                .as_service_error()
                .is_some_and(|error| error.is_transaction_canceled_exception())
            {
                let latest = self
                    .get_latest(&record.app_id, &record.channel, record.message_serial())
                    .await?;
                if latest
                    .as_ref()
                    .is_some_and(|latest| latest.version_serial() >= record.version_serial())
                {
                    tracing::debug!(error = %error, "imported latest state is already superseded");
                    return Ok(());
                }
            }
            return Err(Error::Internal(format!(
                "failed to publish imported latest state: {error}"
            )));
        }

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
        let app_channel = Self::app_channel_key(&request.record.app_id, &request.record.channel);
        let stream_item = self
            .client
            .get_item()
            .table_name(&self.tables.version_streams)
            .key("app_channel", Self::attr_s(&app_channel))
            .consistent_read(true)
            .send()
            .await
            .map_err(|e| Error::Internal(format!("Failed to read version stream: {e}")))?
            .item;
        let next_delivery = stream_item
            .as_ref()
            .and_then(|item| Self::item_num(item, "next_delivery_serial"))
            .unwrap_or(1) as u64;
        let open_count = stream_item
            .as_ref()
            .and_then(|item| Self::item_num(item, "open_stream_count"))
            .unwrap_or(0) as usize;
        if request.record.is_open_ai_stream()
            && let Some(limit) = request.limits.max_open_streaming_messages_per_channel
            && open_count >= limit
        {
            return Ok(VersionCreateResult::Rejected(
                VersionCreateRejection::OpenStreamingMessages { limit },
            ));
        }
        let message_key = request.record.message_serial().as_str().to_string();
        let existing = self
            .client
            .get_item()
            .table_name(&self.tables.version_messages)
            .key("app_channel", Self::attr_s(&app_channel))
            .key("message_serial", Self::attr_s(&message_key))
            .consistent_read(true)
            .send()
            .await
            .map_err(|e| Error::Internal(format!("Failed to read create target: {e}")))?
            .item;
        if existing.is_some() {
            let current = self
                .get_latest(
                    &request.record.app_id,
                    &request.record.channel,
                    request.record.message_serial(),
                )
                .await?
                .ok_or_else(|| {
                    Error::Internal("Existing message has no readable version entry".to_string())
                })?;
            return Ok(VersionCreateResult::Conflict {
                current: Some(current),
            });
        }

        let stream_id = format!("{}/{}", request.record.app_id, request.record.channel);
        let record = request
            .record
            .with_delivery_position(&stream_id, next_delivery);
        let payload = sonic_rs::to_vec(&record)
            .map_err(|e| Error::Internal(format!("Failed to serialize create record: {e}")))?;
        let now_ms = sockudo_core::history::now_ms();
        let next_open = open_count + usize::from(record.is_open_ai_stream());
        let stream_write = if stream_item.is_some() {
            let update = Update::builder()
                .table_name(&self.tables.version_streams)
                .key("app_channel", Self::attr_s(&app_channel))
                .update_expression("SET next_delivery_serial = :next, open_stream_count = :new_open, updated_at_ms = :now, oldest_available_delivery_serial = if_not_exists(oldest_available_delivery_serial, :delivery), newest_available_delivery_serial = :delivery")
                .condition_expression("next_delivery_serial = :expected AND (attribute_not_exists(open_stream_count) OR open_stream_count = :open)")
                .expression_attribute_values(":next", Self::attr_n(next_delivery + 1))
                .expression_attribute_values(":expected", Self::attr_n(next_delivery))
                .expression_attribute_values(":open", Self::attr_n(open_count))
                .expression_attribute_values(":new_open", Self::attr_n(next_open))
                .expression_attribute_values(":delivery", Self::attr_n(next_delivery))
                .expression_attribute_values(":now", Self::attr_n(now_ms))
                .build()
                .map_err(|e| Error::Internal(format!("Failed to build stream update: {e}")))?;
            TransactWriteItem::builder().update(update).build()
        } else {
            let mut item = HashMap::new();
            item.insert("app_channel".to_string(), Self::attr_s(&app_channel));
            item.insert("app_id".to_string(), Self::attr_s(&record.app_id));
            item.insert("channel".to_string(), Self::attr_s(&record.channel));
            item.insert("next_delivery_serial".to_string(), Self::attr_n(2));
            item.insert("open_stream_count".to_string(), Self::attr_n(next_open));
            item.insert(
                "oldest_available_delivery_serial".to_string(),
                Self::attr_n(1),
            );
            item.insert(
                "newest_available_delivery_serial".to_string(),
                Self::attr_n(1),
            );
            item.insert("migration_state".to_string(), Self::attr_s("native_only"));
            item.insert("updated_at_ms".to_string(), Self::attr_n(now_ms));
            let put = Put::builder()
                .table_name(&self.tables.version_streams)
                .set_item(Some(item))
                .condition_expression("attribute_not_exists(app_channel)")
                .build()
                .map_err(|e| Error::Internal(format!("Failed to build stream create: {e}")))?;
            TransactWriteItem::builder().put(put).build()
        };
        let entry_put = Put::builder()
            .table_name(&self.tables.version_entries)
            .set_item(Some(self.entry_item(
                &record,
                encode_full(&record)?,
                None,
            )?))
            .condition_expression("attribute_not_exists(message_version_key)")
            .build()
            .map_err(|e| Error::Internal(format!("Failed to build create entry: {e}")))?;
        let mut message_item = HashMap::new();
        message_item.insert("app_channel".to_string(), Self::attr_s(&app_channel));
        message_item.insert("message_serial".to_string(), Self::attr_s(&message_key));
        message_item.insert(
            "latest_version_serial".to_string(),
            Self::attr_s(record.version_serial().as_str()),
        );
        message_item.insert(
            "latest_delivery_serial".to_string(),
            Self::attr_n(next_delivery),
        );
        message_item.insert(
            "latest_action".to_string(),
            Self::attr_s(record.message.action.as_str()),
        );
        message_item.insert("latest_payload_bytes".to_string(), Self::attr_b(payload));
        message_item.insert("append_count".to_string(), Self::attr_n(0));
        message_item.insert(
            "is_open_stream".to_string(),
            AttributeValue::Bool(record.is_open_ai_stream()),
        );
        message_item.insert(
            "history_serial".to_string(),
            Self::attr_n(record.history_serial()),
        );
        message_item.insert("created_at_ms".to_string(), Self::attr_n(now_ms));
        message_item.insert("updated_at_ms".to_string(), Self::attr_n(now_ms));
        let create_epoch = self.append_storage_epoch().await?;
        let seed = if create_epoch.is_some() {
            AppendRunPlan::for_seed_record(&record)
        } else {
            AppendRunPlan::Full
        };
        Self::seed_attributes(&mut message_item, &record, &seed);
        Self::validate_item_size(&message_item)?;
        let message_put = Put::builder()
            .table_name(&self.tables.version_messages)
            .set_item(Some(message_item))
            .condition_expression("attribute_not_exists(message_serial)")
            .build()
            .map_err(|e| Error::Internal(format!("Failed to build message create: {e}")))?;
        let mut transaction = self
            .client
            .transact_write_items()
            .transact_items(stream_write)
            .transact_items(TransactWriteItem::builder().put(entry_put).build())
            .transact_items(TransactWriteItem::builder().put(message_put).build());
        if let Some(activation) = self.seed_activation_write(&record, &seed)? {
            transaction = transaction.transact_items(activation);
        }
        if let Some(epoch) = create_epoch.as_deref() {
            transaction = transaction.transact_items(self.append_marker_guard(epoch)?);
            self.stage_seed_plan(&record, seed).await?;
        }
        let result = transaction.send().await;
        match result {
            Ok(_) => Ok(VersionCreateResult::Applied { record, stream_id }),
            Err(error)
                if error
                    .as_service_error()
                    .is_some_and(|error| error.is_transaction_canceled_exception()) =>
            {
                let current = self
                    .get_latest(&record.app_id, &record.channel, record.message_serial())
                    .await?;
                if let Some(current) = current {
                    Ok(VersionCreateResult::Conflict {
                        current: Some(current),
                    })
                } else {
                    Ok(VersionCreateResult::Conflict { current: None })
                }
            }
            Err(error) => Err(Error::Internal(format!(
                "Failed to transact version create: {error}"
            ))),
        }
    }

    async fn compare_and_apply(
        &self,
        request: VersionMutationRequest,
    ) -> Result<VersionMutationResult> {
        let app_channel = Self::app_channel_key(&request.app_id, &request.channel);
        if let Some(operation) = request.idempotency.as_ref() {
            let receipt_key = Self::operation_receipt_key(&operation.cache_key);
            if let Some(item) = self
                .client
                .get_item()
                .table_name(&self.tables.version_entries)
                .key("app_channel", Self::attr_s(&app_channel))
                .key("message_version_key", Self::attr_s(&receipt_key))
                .consistent_read(true)
                .send()
                .await
                .map_err(|e| Error::Internal(format!("Failed to read operation receipt: {e}")))?
                .item
            {
                let fingerprint = Self::item_str(&item, "operation_fingerprint");
                if fingerprint.as_deref() != Some(operation.payload_fingerprint.as_str()) {
                    return Err(Error::IdempotencyConflict);
                }
                let record = self
                    .materialize_items(&app_channel, std::slice::from_ref(&item))
                    .await?
                    .pop()
                    .flatten()
                    .ok_or_else(|| Error::Internal("Receipt payload is missing".to_string()))?;
                return Ok(VersionMutationResult::Duplicate {
                    record,
                    stream_id: format!("{}/{}", request.app_id, request.channel),
                });
            }
        }
        let stream_item = self
            .client
            .get_item()
            .table_name(&self.tables.version_streams)
            .key("app_channel", Self::attr_s(&app_channel))
            .consistent_read(true)
            .send()
            .await
            .map_err(|e| Error::Internal(format!("Failed to read version stream: {e}")))?
            .item;
        let Some(stream_item) = stream_item else {
            return Ok(VersionMutationResult::Conflict { current: None });
        };
        let next_delivery =
            Self::item_num(&stream_item, "next_delivery_serial").unwrap_or(1) as u64;
        let open_count = Self::item_num(&stream_item, "open_stream_count").unwrap_or(0) as usize;
        let message_item = self
            .client
            .get_item()
            .table_name(&self.tables.version_messages)
            .key("app_channel", Self::attr_s(&app_channel))
            .key(
                "message_serial",
                Self::attr_s(request.message_serial.as_str()),
            )
            .consistent_read(true)
            .send()
            .await
            .map_err(|e| Error::Internal(format!("Failed to read mutation predecessor: {e}")))?
            .item;
        let Some(message_item) = message_item else {
            return Ok(VersionMutationResult::Conflict { current: None });
        };
        let current = self
            .materialize_latest_item(&app_channel, &message_item)
            .await?
            .ok_or_else(|| Error::Internal("message has no readable latest state".to_string()))?;
        let append_count = Self::item_num(&message_item, "append_count").unwrap_or(0) as usize;
        // The run pointer is written with the latest-state record, naming the
        // version it describes; older releases never update it, so it is used
        // only while it still names this predecessor. The commit re-checks
        // the run head.
        let mut predecessor_run = None;
        let mut run_pinned = false;
        if let (Some(run), Some(data_len), Some(head)) = (
            Self::item_str(&message_item, "latest_append_run"),
            Self::item_num(&message_item, "latest_append_len")
                .and_then(|value| u64::try_from(value).ok()),
            Self::item_str(&message_item, "latest_append_head"),
        ) && head == current.version_serial().as_str()
            && current.data_bytes()? as u64 == data_len
        {
            run_pinned = message_item
                .get("latest_append_pinned")
                .and_then(|value| value.as_bool().ok())
                .copied()
                .unwrap_or(false);
            predecessor_run = Some(AppendRunRef {
                run: VersionSerial::new(run)?,
                data_len,
                generation: Self::item_str(&message_item, "latest_append_generation"),
            });
        }
        if let Some(bytes) = message_item
            .get("latest_payload_bytes")
            .and_then(|value| value.as_b().ok())
            && predecessor_run
                .as_ref()
                .is_none_or(|run| run.generation.is_none())
            && let Some(run) = StoredVersionPayload::decode(bytes.as_ref())?.run()
        {
            predecessor_run = Some(run.clone());
        }
        let delivery_serial = next_delivery.max(current.delivery_serial().saturating_add(1));
        let stream_id = format!("{}/{}", request.app_id, request.channel);
        let outcome = request.apply_to(&current, &stream_id, delivery_serial, append_count)?;
        let VersionMutationResult::Applied { record, .. } = outcome else {
            return Ok(outcome);
        };
        let opens = !current.is_open_ai_stream() && record.is_open_ai_stream();
        let closes = current.is_open_ai_stream() && !record.is_open_ai_stream();
        if opens
            && let Some(limit) = request.limits.max_open_streaming_messages_per_channel
            && open_count >= limit
        {
            return Ok(VersionMutationResult::Rejected(
                VersionMutationRejection::OpenStreamingMessages { limit },
            ));
        }
        let next_open = open_count
            .saturating_add(usize::from(opens))
            .saturating_sub(usize::from(closes));
        let next_append_count = append_count
            + usize::from(matches!(
                request.mutation,
                sockudo_core::version_store::VersionMutation::Append(_)
            ));
        let now_ms = sockudo_core::history::now_ms();
        let format_epoch = self.append_storage_epoch().await?;
        let plan = if format_epoch.is_some() {
            AppendRunPlan::for_record_chunked(
                current.version_serial(),
                predecessor_run.as_ref(),
                &record,
            )
        } else {
            AppendRunPlan::Full
        };
        let payload = plan.encode(&record)?;
        let full_seed = format_epoch.is_some() && matches!(plan, AppendRunPlan::Full);
        let large_start = matches!(plan, AppendRunPlan::Start { .. })
            && self.append_chunk_writes(&record, &plan)?.len()
                + 5
                + usize::from(request.idempotency.is_some())
                > 100;
        let seed_staged = full_seed || large_start;
        let plan = if full_seed {
            AppendRunPlan::for_seed_record(&record)
        } else {
            plan
        };
        let latest_payload = payload.clone();
        let run_pinned = run_pinned
            || plan.run().is_some_and(|run| run.generation.is_some())
            || (request.idempotency.is_some() && plan.run().is_some());
        let stream_update = Update::builder()
            .table_name(&self.tables.version_streams)
            .key("app_channel", Self::attr_s(&app_channel))
            .update_expression("SET next_delivery_serial = :next, open_stream_count = :new_open, newest_available_delivery_serial = :delivery, updated_at_ms = :now")
            .condition_expression("next_delivery_serial = :expected AND (attribute_not_exists(open_stream_count) OR open_stream_count = :open)")
            .expression_attribute_values(":next", Self::attr_n(delivery_serial + 1))
            .expression_attribute_values(":expected", Self::attr_n(next_delivery))
            .expression_attribute_values(":open", Self::attr_n(open_count))
            .expression_attribute_values(":new_open", Self::attr_n(next_open))
            .expression_attribute_values(":delivery", Self::attr_n(delivery_serial))
            .expression_attribute_values(":now", Self::attr_n(now_ms))
            .build()
            .map_err(|e| Error::Internal(format!("Failed to build stream mutation: {e}")))?;
        let entry_put = Put::builder()
            .table_name(&self.tables.version_entries)
            .set_item(Some(self.entry_item(
                &record,
                payload.clone(),
                request.idempotency.as_ref(),
            )?))
            .condition_expression("attribute_not_exists(message_version_key)")
            .build()
            .map_err(|e| Error::Internal(format!("Failed to build mutation entry: {e}")))?;
        let message_update = Update::builder()
            .table_name(&self.tables.version_messages)
            .key("app_channel", Self::attr_s(&app_channel))
            .key(
                "message_serial",
                Self::attr_s(request.message_serial.as_str()),
            )
            .update_expression(if plan.run().is_some() {
                "SET latest_version_serial = :next_vs, latest_delivery_serial = :next_ds, latest_action = :action, latest_payload_bytes = :payload, append_count = :append_count, is_open_stream = :is_open, updated_at_ms = :now, latest_append_run = :run, latest_append_len = :run_len, latest_append_head = :next_vs, latest_append_pinned = :run_pinned, latest_append_generation = :run_generation"
            } else {
                "SET latest_version_serial = :next_vs, latest_delivery_serial = :next_ds, latest_action = :action, latest_payload_bytes = :payload, append_count = :append_count, is_open_stream = :is_open, updated_at_ms = :now REMOVE latest_append_run, latest_append_len, latest_append_head, latest_append_pinned, latest_append_generation"
            })
            .condition_expression("latest_version_serial = :expected_vs AND latest_delivery_serial = :expected_ds")
            .expression_attribute_values(":next_vs", Self::attr_s(record.version_serial().as_str()))
            .expression_attribute_values(":next_ds", Self::attr_n(delivery_serial))
            .expression_attribute_values(":action", Self::attr_s(record.message.action.as_str()))
            .expression_attribute_values(":payload", Self::attr_b(latest_payload))
            .expression_attribute_values(":append_count", Self::attr_n(next_append_count))
            .expression_attribute_values(":is_open", AttributeValue::Bool(record.is_open_ai_stream()))
            .expression_attribute_values(":now", Self::attr_n(now_ms))
            .expression_attribute_values(":expected_vs", Self::attr_s(current.version_serial().as_str()))
            .expression_attribute_values(":expected_ds", Self::attr_n(current.delivery_serial()));
        let message_update = if let Some(run) = plan.run() {
            message_update
                .expression_attribute_values(":run", Self::attr_s(run.run.as_str()))
                .expression_attribute_values(
                    ":run_generation",
                    Self::attr_s(run.generation.as_deref().unwrap_or_default()),
                )
                .expression_attribute_values(":run_len", Self::attr_n(run.data_len))
                .expression_attribute_values(":run_pinned", AttributeValue::Bool(run_pinned))
        } else {
            message_update
        }
        .build()
        .map_err(|e| Error::Internal(format!("Failed to build message mutation: {e}")))?;
        // Validate the exact post-update latest item before publishing any
        // state. Attribute names count toward the service item limit too.
        let mut next_message_item = message_item.clone();
        for (field, binding) in [
            ("latest_version_serial", ":next_vs"),
            ("latest_delivery_serial", ":next_ds"),
            ("latest_action", ":action"),
            ("latest_payload_bytes", ":payload"),
            ("append_count", ":append_count"),
            ("is_open_stream", ":is_open"),
            ("updated_at_ms", ":now"),
            ("latest_append_run", ":run"),
            ("latest_append_len", ":run_len"),
            ("latest_append_head", ":next_vs"),
            ("latest_append_pinned", ":run_pinned"),
            ("latest_append_generation", ":run_generation"),
        ] {
            if field.starts_with("latest_append_") && plan.run().is_none() {
                next_message_item.remove(field);
            } else if let Some(value) = message_update
                .expression_attribute_values()
                .and_then(|values| values.get(binding))
            {
                next_message_item.insert(field.to_string(), value.clone());
            }
        }
        Self::validate_item_size(&next_message_item)?;
        let mut transaction = self
            .client
            .transact_write_items()
            .transact_items(TransactWriteItem::builder().update(stream_update).build())
            .transact_items(TransactWriteItem::builder().put(entry_put).build())
            .transact_items(TransactWriteItem::builder().update(message_update).build());
        if let Some(epoch) = format_epoch.as_deref() {
            transaction = transaction.transact_items(self.append_marker_guard(epoch)?);
        }
        let mut staged_token = None;
        let mut staged_chunks = None;
        if seed_staged {
            if let Some(activation) = self.seed_activation_write(&record, &plan)? {
                transaction = transaction.transact_items(activation);
            }
        } else {
            let writes = self.append_chunk_writes(&record, &plan)?;
            let staged = transaction
                .as_input()
                .get_transact_items()
                .as_ref()
                .map_or(0, Vec::len)
                + writes.len()
                + usize::from(plan.run().is_some())
                + usize::from(request.idempotency.is_some())
                > 100;
            if staged {
                staged_token = Some(uuid::Uuid::new_v4().to_string());
                staged_chunks = Some(writes);
            } else {
                for write in writes {
                    transaction = transaction.transact_items(write);
                }
            }
            if let Some(mut run_write) =
                self.append_run_write(&record, &plan, run_pinned, now_ms)?
            {
                if let Some(token) = staged_token.as_deref() {
                    Self::fence_staged_publication(&mut run_write, token)?;
                }
                transaction = transaction.transact_items(run_write);
            }
        }
        if let Some(operation) = request.idempotency.as_ref() {
            let mut receipt = HashMap::new();
            receipt.insert("app_channel".to_string(), Self::attr_s(&app_channel));
            receipt.insert(
                "message_version_key".to_string(),
                Self::attr_s(&Self::operation_receipt_key(&operation.cache_key)),
            );
            receipt.insert(
                "operation_fingerprint".to_string(),
                Self::attr_s(&operation.payload_fingerprint),
            );
            receipt.insert("payload_bytes".to_string(), Self::attr_b(payload));
            Self::validate_item_size(&receipt)?;
            let receipt_put = Put::builder()
                .table_name(&self.tables.version_entries)
                .set_item(Some(receipt))
                .condition_expression("attribute_not_exists(message_version_key)")
                .build()
                .map_err(|e| Error::Internal(format!("Failed to build operation receipt: {e}")))?;
            transaction =
                transaction.transact_items(TransactWriteItem::builder().put(receipt_put).build());
        }
        // The available chunk budget depends on whether this operation has
        // a receipt and a format fence; do not reserve unused transaction slots.
        if transaction
            .as_input()
            .get_transact_items()
            .as_ref()
            .is_some_and(|items| items.len() > 100)
        {
            return Err(Error::Configuration(
                "append exceeds DynamoDB's 100-item atomic transaction limit; reduce the append fragment size".to_string(),
            ));
        }
        #[cfg(test)]
        let write_measurement = self
            .measure_write_before(
                transaction
                    .as_input()
                    .get_transact_items()
                    .as_deref()
                    .unwrap_or_default(),
            )
            .await?;
        // All publication items and their size checks are complete before
        // staging creates any durable state or claims a lease.
        if seed_staged {
            self.stage_seed_plan(&record, plan.clone()).await?;
        }
        if let (Some(writes), Some(token)) = (staged_chunks.as_deref(), staged_token.as_deref()) {
            let epoch = format_epoch.as_deref().ok_or_else(|| {
                Error::Internal("append staging requires enabled chunked storage".to_string())
            })?;
            if self
                .stage_append_chunks_with_token(&record, &plan, epoch, writes, token)
                .await?
                .is_none()
            {
                return Ok(VersionMutationResult::Conflict {
                    current: self
                        .get_latest(&request.app_id, &request.channel, &request.message_serial)
                        .await?,
                });
            }
        }
        let result = transaction.send().await;
        if let Err(_error) = &result
            && let Some(token) = staged_token.as_deref()
        {
            tracing::debug!("staged append publication failed");
            self.release_append_stage(&record, &plan, token).await;
        }
        match result {
            Ok(_) => {
                #[cfg(test)]
                self.measure_write_after(write_measurement).await?;

                if let (Some(run), Some(snapshot)) = (plan.run(), plan.snapshot_after(&record)) {
                    self.append_cache.insert(
                        &record.app_id,
                        &record.channel,
                        record.message_serial(),
                        run,
                        snapshot.to_owned(),
                    );
                }
                Ok(VersionMutationResult::Applied { record, stream_id })
            }
            Err(error)
                if error
                    .as_service_error()
                    .is_some_and(|error| error.is_transaction_canceled_exception()) =>
            {
                Ok(VersionMutationResult::Conflict {
                    current: self
                        .get_latest(&request.app_id, &request.channel, &request.message_serial)
                        .await?,
                })
            }
            Err(error) => Err(Error::Internal(format!(
                "Failed to transact version mutation: {error}"
            ))),
        }
    }

    async fn get_latest(
        &self,
        app_id: &str,
        channel: &str,
        message_serial: &sockudo_core::versioned_messages::MessageSerial,
    ) -> Result<Option<StoredVersionRecord>> {
        let app_channel = Self::app_channel_key(app_id, channel);
        if let Some(item) = self
            .client
            .get_item()
            .table_name(&self.tables.version_messages)
            .key("app_channel", Self::attr_s(&app_channel))
            .key("message_serial", Self::attr_s(message_serial.as_str()))
            .consistent_read(true)
            .send()
            .await
            .map_err(|e| Error::Internal(format!("Failed to read latest message: {e}")))?
            .item
            && let Some(record) = self.materialize_latest_item(&app_channel, &item).await?
        {
            return Ok(Some(record));
        }
        let app_channel_message =
            Self::app_channel_message_key(app_id, channel, message_serial.as_str());
        // Query the message GSI sorted by version_serial DESC, limit 1.
        let result = self
            .client
            .query()
            .table_name(&self.tables.version_entries)
            .index_name(&self.tables.version_entries_message_index)
            .key_condition_expression("app_channel_message = :acm")
            .expression_attribute_values(":acm", Self::attr_s(&app_channel_message))
            .scan_index_forward(false)
            .limit(1)
            .send()
            .await
            .map_err(|e| Error::Internal(format!("Failed to query latest version: {e}")))?;

        let items = result.items();
        if items.is_empty() {
            return Ok(None);
        }
        Ok(self
            .materialize_items(&app_channel, &items[..1])
            .await?
            .pop()
            .flatten())
    }

    async fn get_versions(&self, request: VersionStoreReadRequest) -> Result<VersionStorePage> {
        request.validate()?;
        let app_channel_message = Self::app_channel_message_key(
            &request.app_id,
            &request.channel,
            request.message_serial.as_str(),
        );
        let scan_forward = matches!(request.direction, VersionStoreDirection::OldestFirst);
        let fetch_limit = request
            .limit
            .checked_add(1)
            .and_then(|limit| i32::try_from(limit).ok())
            .ok_or_else(|| {
                Error::InvalidMessageFormat(
                    "version history limit exceeds DynamoDB query capacity".to_string(),
                )
            })?;

        let mut query = self
            .client
            .query()
            .table_name(&self.tables.version_entries)
            .index_name(&self.tables.version_entries_message_index)
            .key_condition_expression(if request.cursor.is_some() {
                match request.direction {
                    VersionStoreDirection::NewestFirst => {
                        "app_channel_message = :acm AND version_serial < :cursor_vs"
                    }
                    VersionStoreDirection::OldestFirst => {
                        "app_channel_message = :acm AND version_serial > :cursor_vs"
                    }
                }
            } else {
                "app_channel_message = :acm"
            })
            .expression_attribute_values(":acm", Self::attr_s(&app_channel_message))
            .scan_index_forward(scan_forward)
            .limit(fetch_limit);

        if let Some(cursor) = &request.cursor {
            query = query.expression_attribute_values(
                ":cursor_vs",
                Self::attr_s(cursor.version_serial.as_str()),
            );
        }

        // DynamoDB stops each Query at 1 MiB even when its item limit has
        // not been reached. Bound work by scanned rows, preserving the public
        // cursor's progress through expired records as well as live records.
        let app_channel = Self::app_channel_key(&request.app_id, &request.channel);
        let now_secs = sockudo_core::history::now_ms() / 1000;
        let mut scanned = Vec::new();
        let mut start = None;
        // A service page normally contributes at least one row (no filter).
        // This additional cap also bounds concurrent-delete empty pages.
        for _ in 0..=request.limit {
            let result = query
                .clone()
                .limit((request.limit + 1 - scanned.len()) as i32)
                .set_exclusive_start_key(start)
                .send()
                .await
                .map_err(|e| Error::Internal(format!("failed to query version history: {e}")))?;
            scanned.extend_from_slice(result.items());
            start = result
                .last_evaluated_key()
                .cloned()
                .filter(|key| !key.is_empty());
            if scanned.len() > request.limit || start.is_none() {
                break;
            }
        }
        let has_more = scanned.len() > request.limit || start.is_some();
        scanned.truncate(request.limit);
        let last_scanned = scanned
            .last()
            .and_then(|item| Self::item_str(item, "version_serial"))
            .or_else(|| {
                start
                    .as_ref()
                    .and_then(|key| Self::item_str(key, "version_serial"))
            })
            .map(VersionSerial::new)
            .transpose()?;
        if has_more && last_scanned.is_none() {
            return Err(Error::Internal(
                "version history page has no continuation serial".to_string(),
            ));
        }
        let live_items = scanned
            .into_iter()
            .filter(|item| {
                Self::item_num(item, Self::EXPIRES_AT_ATTR).is_none_or(|expires| expires > now_secs)
            })
            .collect::<Vec<_>>();
        let items = self
            .materialize_items(&app_channel, &live_items)
            .await?
            .into_iter()
            .flatten()
            .collect();

        let next_cursor = if has_more {
            last_scanned.map(|version_serial| VersionStoreCursor {
                version: 1,
                version_serial,
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
        let app_channel = Self::app_channel_key(&request.app_id, &request.channel);
        let fetch_limit = i32::try_from(request.limit).map_err(|_| {
            Error::InvalidMessageFormat(
                "version replay limit exceeds DynamoDB query capacity".to_string(),
            )
        })?;
        let query = self
            .client
            .query()
            .table_name(&self.tables.version_entries)
            .index_name(&self.tables.version_entries_delivery_index)
            .key_condition_expression("app_channel = :ac AND delivery_serial > :after")
            .expression_attribute_values(":ac", Self::attr_s(&app_channel))
            .expression_attribute_values(":after", Self::attr_n(request.after_delivery_serial))
            .scan_index_forward(true)
            .limit(fetch_limit);
        let mut scanned = Vec::new();
        let mut start = None;
        let now_secs = sockudo_core::history::now_ms() / 1000;
        for _ in 0..request.limit {
            let page = query
                .clone()
                .limit((request.limit - scanned.len()) as i32)
                .set_exclusive_start_key(start)
                .send()
                .await
                .map_err(|e| Error::Internal(format!("failed to replay version entries: {e}")))?;
            scanned.extend_from_slice(page.items());
            start = page
                .last_evaluated_key()
                .cloned()
                .filter(|key| !key.is_empty());
            if scanned.len() == request.limit || start.is_none() {
                break;
            }
        }
        if scanned.len() < request.limit && start.is_some() {
            return Err(Error::Internal(
                "version replay could not complete within its page budget".to_string(),
            ));
        }
        let live = scanned
            .into_iter()
            .filter(|item| {
                Self::item_num(item, Self::EXPIRES_AT_ATTR).is_none_or(|expires| expires > now_secs)
            })
            .collect::<Vec<_>>();
        // Expired entries leave delivery gaps; caller continuity checks reject
        // them exactly as if asynchronous TTL deletion had already completed.
        Ok(self
            .materialize_items(&app_channel, &live)
            .await?
            .into_iter()
            .flatten()
            .collect())
    }

    async fn latest_by_history(
        &self,
        app_id: &str,
        channel: &str,
    ) -> Result<Vec<StoredVersionRecord>> {
        let app_channel = Self::app_channel_key(app_id, channel);
        // This API returns all messages. Page through the base table while
        // retaining only identities, rather than complete latest payloads.
        let query = self
            .client
            .query()
            .table_name(&self.tables.version_messages)
            .key_condition_expression("app_channel = :ac")
            .expression_attribute_values(":ac", Self::attr_s(&app_channel))
            .projection_expression("message_serial, latest_version_serial, history_serial")
            .limit(100);
        let mut msgs = Vec::new();
        let mut start = None;
        loop {
            let page = query
                .clone()
                .set_exclusive_start_key(start.clone())
                .send()
                .await
                .map_err(|e| Error::Internal(format!("failed to query version messages: {e}")))?;
            msgs.extend(page.items().iter().filter_map(|item| {
                Some((
                    Self::item_str(item, "message_serial")?,
                    Self::item_str(item, "latest_version_serial")?,
                    Self::item_num(item, "history_serial")?,
                ))
            }));
            let next = page
                .last_evaluated_key()
                .cloned()
                .filter(|key| !key.is_empty());
            if next.is_none() {
                break;
            }
            if next == start {
                return Err(Error::Internal(
                    "version messages query did not advance".to_string(),
                ));
            }
            start = next;
        }

        msgs.sort_by_key(|(_, _, hs)| *hs);

        let mut result = Vec::with_capacity(msgs.len());
        for (message_serial, latest_version_serial, _hs) in msgs {
            let app_channel_message =
                Self::app_channel_message_key(app_id, channel, &message_serial);
            let entry_result = self
                .client
                .query()
                .table_name(&self.tables.version_entries)
                .index_name(&self.tables.version_entries_message_index)
                .key_condition_expression("app_channel_message = :acm AND version_serial = :vs")
                .expression_attribute_values(":acm", Self::attr_s(&app_channel_message))
                .expression_attribute_values(":vs", Self::attr_s(&latest_version_serial))
                .limit(1)
                .send()
                .await
                .map_err(|e| Error::Internal(format!("Failed to fetch version entry: {e}")))?;

            if let Some(record) = self
                .materialize_items(
                    &app_channel,
                    &entry_result.items()[..entry_result.items().len().min(1)],
                )
                .await?
                .pop()
                .flatten()
            {
                result.push(record);
            }
        }
        Ok(result)
    }

    async fn purge_before(&self, _before_ms: i64, batch_size: usize) -> Result<(u64, bool)> {
        self.purge_append_chunks(batch_size).await
    }

    async fn set_append_storage_enabled(&self, enabled: bool) -> Result<()> {
        if enabled && self.append_storage_epoch().await?.is_none() {
            self.seed_latest_states().await?;
        }
        self.client
            .put_item()
            .table_name(&self.tables.version_streams)
            .item("app_channel", Self::attr_s(Self::FORMAT_MARKER_KEY))
            .item("enabled", AttributeValue::Bool(enabled))
            .item("epoch", Self::attr_s(&uuid::Uuid::new_v4().to_string()))
            .send()
            .await
            .map_err(|e| Error::Internal(format!("failed to update append storage marker: {e}")))?;
        Ok(())
    }

    async fn validate_append_storage_rollback(&self, batch_size: usize) -> Result<()> {
        self.preflight_append_materialization(batch_size).await
    }

    async fn materialize_append_storage(&self, batch_size: usize) -> Result<u64> {
        self.materialize_compact_items(batch_size).await
    }

    async fn stream_state(&self, app_id: &str, channel: &str) -> Result<VersionStreamState> {
        let app_channel = Self::app_channel_key(app_id, channel);
        let result = self
            .client
            .get_item()
            .table_name(&self.tables.version_streams)
            .key("app_channel", Self::attr_s(&app_channel))
            .send()
            .await
            .map_err(|e| {
                Error::Internal(format!("Failed to read DynamoDB version stream state: {e}"))
            })?;

        let Some(item) = result.item else {
            return Ok(VersionStreamState::default());
        };

        Ok(VersionStreamState {
            stream_id: Some(format!("{}/{}", app_id, channel)),
            next_delivery_serial: Self::item_num(&item, "next_delivery_serial").map(|v| v as u64),
            oldest_available_delivery_serial: Self::item_num(
                &item,
                "oldest_available_delivery_serial",
            )
            .map(|v| v as u64),
            newest_available_delivery_serial: Self::item_num(
                &item,
                "newest_available_delivery_serial",
            )
            .map(|v| v as u64),
        })
    }
}
