use super::store::VersionStore;
use super::types::*;
use crate::error::{Error, Result};
use crate::history::now_ms;
use crate::versioned_messages::{
    MessageAction, MessageSerial, validate_replay_continuity_iter, validate_version_chain,
    validate_version_chain_iter,
};
use async_trait::async_trait;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use tokio::sync::RwLock;

#[derive(Clone, Default)]
pub struct MemoryVersionStore {
    channels: Arc<RwLock<BTreeMap<String, MemoryVersionChannel>>>,
}

#[derive(Clone)]
struct MemoryVersionChannel {
    stream_id: String,
    next_delivery_serial: u64,
    messages: BTreeMap<String, Vec<Arc<StoredVersionRecord>>>,
    replay: BTreeMap<u64, Arc<StoredVersionRecord>>,
    // Parallel map: `delivery_serial -> server-side append time (ms)`.
    // Used by `purge_before` for TTL eviction without touching read paths.
    created_at: BTreeMap<u64, i64>,
}

impl Default for MemoryVersionChannel {
    fn default() -> Self {
        Self {
            stream_id: uuid::Uuid::new_v4().to_string(),
            next_delivery_serial: 1,
            messages: BTreeMap::new(),
            replay: BTreeMap::new(),
            created_at: BTreeMap::new(),
        }
    }
}

impl MemoryVersionStore {
    pub fn new() -> Self {
        Self::default()
    }

    fn channel_key(app_id: &str, channel: &str) -> String {
        format!("{app_id}\0{channel}")
    }

    fn is_terminal(record: &StoredVersionRecord) -> bool {
        matches!(
            record
                .message
                .extras
                .as_ref()
                .and_then(|extras| extras.ai_transport_headers())
                .and_then(|headers| headers.status()),
            Some("complete" | "cancelled")
        )
    }
}

#[async_trait]
impl VersionStore for MemoryVersionStore {
    async fn ensure_stream_id(&self, app_id: &str, channel: &str) -> Result<String> {
        let key = Self::channel_key(app_id, channel);
        let mut channels = self.channels.write().await;
        Ok(channels.entry(key).or_default().stream_id.clone())
    }

    async fn reserve_delivery_position(
        &self,
        app_id: &str,
        channel: &str,
    ) -> Result<VersionWriteReservation> {
        let key = Self::channel_key(app_id, channel);
        let mut channels = self.channels.write().await;
        let channel_state = channels.entry(key).or_default();
        let reservation = VersionWriteReservation {
            stream_id: channel_state.stream_id.clone(),
            delivery_serial: channel_state.next_delivery_serial,
        };
        channel_state.next_delivery_serial = channel_state.next_delivery_serial.saturating_add(1);
        Ok(reservation)
    }

    async fn reserve_delivery_positions(
        &self,
        app_id: &str,
        channel: &str,
        block_size: u64,
    ) -> Result<VersionWriteReservationBlock> {
        VersionWriteReservationBlock::validate(block_size)?;
        let key = Self::channel_key(app_id, channel);
        let mut channels = self.channels.write().await;
        let channel_state = channels.entry(key).or_default();
        let block = VersionWriteReservationBlock {
            stream_id: channel_state.stream_id.clone(),
            start_delivery_serial: channel_state.next_delivery_serial,
            len: block_size,
        };
        channel_state.next_delivery_serial = channel_state
            .next_delivery_serial
            .saturating_add(block_size);
        Ok(block)
    }

    async fn append_version(&self, record: StoredVersionRecord) -> Result<()> {
        let key = Self::channel_key(&record.app_id, &record.channel);
        let mut channels = self.channels.write().await;
        let channel_state = channels.entry(key).or_default();

        if let Some(existing) = channel_state.replay.get(&record.delivery_serial()) {
            return Err(Error::InvalidMessageFormat(format!(
                "duplicate delivery_serial {} in version replay log for {}:{} (existing message_serial {}, incoming {})",
                record.delivery_serial(),
                record.app_id,
                record.channel,
                existing.message_serial().as_str(),
                record.message_serial().as_str()
            )));
        }

        let message_serial = record.message_serial().as_str().to_owned();
        if let Some(chain) = channel_state.messages.get(&message_serial) {
            validate_version_chain_iter(
                chain
                    .iter()
                    .map(|entry| &entry.message)
                    .chain(std::iter::once(&record.message)),
            )?;
        } else {
            validate_version_chain(std::slice::from_ref(&record.message))?;
        }

        let record = Arc::new(record);
        channel_state
            .messages
            .entry(message_serial)
            .or_default()
            .push(Arc::clone(&record));
        channel_state
            .created_at
            .insert(record.delivery_serial(), now_ms());
        channel_state
            .replay
            .insert(record.delivery_serial(), Arc::clone(&record));
        channel_state.next_delivery_serial = channel_state
            .next_delivery_serial
            .max(record.delivery_serial().saturating_add(1));

        Ok(())
    }

    async fn commit_create(&self, request: VersionCreateRequest) -> Result<VersionCreateResult> {
        let key = Self::channel_key(&request.record.app_id, &request.record.channel);
        let mut channels = self.channels.write().await;
        let channel_state = channels.entry(key).or_default();

        if let Some(current) = channel_state
            .messages
            .get(request.record.message_serial().as_str())
            .and_then(|chain| {
                chain
                    .iter()
                    .max_by(|left, right| left.version_serial().cmp(right.version_serial()))
            })
        {
            return Ok(VersionCreateResult::Conflict {
                current: Some(current.as_ref().clone()),
            });
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
            let open = channel_state
                .messages
                .values()
                .filter_map(|chain| {
                    chain
                        .iter()
                        .max_by(|left, right| left.version_serial().cmp(right.version_serial()))
                })
                .filter(|record| record.is_open_ai_stream())
                .count();
            if open >= limit {
                return Ok(VersionCreateResult::Rejected(
                    VersionCreateRejection::OpenStreamingMessages { limit },
                ));
            }
        }

        let delivery_serial = channel_state.next_delivery_serial;
        let record = request
            .record
            .with_delivery_position(&channel_state.stream_id, delivery_serial);
        validate_version_chain(std::slice::from_ref(&record.message))?;
        if channel_state.replay.contains_key(&delivery_serial) {
            return Err(Error::InvalidMessageFormat(format!(
                "duplicate delivery_serial {delivery_serial} in version replay log"
            )));
        }
        let stored_record = Arc::new(record.clone());
        channel_state.messages.insert(
            record.message_serial().as_str().to_string(),
            vec![Arc::clone(&stored_record)],
        );
        channel_state.created_at.insert(delivery_serial, now_ms());
        channel_state.replay.insert(delivery_serial, stored_record);
        channel_state.next_delivery_serial = delivery_serial.saturating_add(1);

        Ok(VersionCreateResult::Applied {
            record,
            stream_id: channel_state.stream_id.clone(),
        })
    }

    async fn compare_and_apply(
        &self,
        request: VersionMutationRequest,
    ) -> Result<VersionMutationResult> {
        let key = Self::channel_key(&request.app_id, &request.channel);
        let mut channels = self.channels.write().await;
        let Some(channel_state) = channels.get_mut(&key) else {
            return Ok(VersionMutationResult::Conflict { current: None });
        };
        let Some(chain) = channel_state.messages.get(request.message_serial.as_str()) else {
            return Ok(VersionMutationResult::Conflict { current: None });
        };

        if let Some(incoming) = request.idempotency.as_ref()
            && let Some(existing) = chain.iter().find(|record| {
                record
                    .envelope
                    .as_ref()
                    .and_then(|envelope| envelope.idempotency.as_ref())
                    .is_some_and(|operation| operation.cache_key == incoming.cache_key)
            })
        {
            let existing_idempotency = existing
                .envelope
                .as_ref()
                .and_then(|envelope| envelope.idempotency.as_ref())
                .ok_or_else(|| {
                    Error::Internal(
                        "matched mutation idempotency record disappeared during lookup".to_string(),
                    )
                })?;
            if existing_idempotency.payload_fingerprint != incoming.payload_fingerprint {
                return Err(Error::IdempotencyConflict);
            }
            return Ok(VersionMutationResult::Duplicate {
                record: existing.as_ref().clone(),
                stream_id: channel_state.stream_id.clone(),
            });
        }

        let current = chain
            .iter()
            .max_by(|left, right| left.version_serial().cmp(right.version_serial()))
            .ok_or_else(|| {
                Error::InvalidMessageFormat("version chain must not be empty".to_string())
            })?;
        if !request.expected.matches(current) {
            return Ok(VersionMutationResult::Conflict {
                current: Some(current.as_ref().clone()),
            });
        }

        if matches!(request.mutation, VersionMutation::Append(_)) {
            if request.limits.reject_append_after_terminal && Self::is_terminal(current) {
                return Ok(VersionMutationResult::Rejected(
                    VersionMutationRejection::TerminalMessage,
                ));
            }
            if let Some(limit) = request.limits.max_appends_per_message {
                let append_count = chain
                    .iter()
                    .filter(|record| record.message.action == MessageAction::Append)
                    .count();
                if append_count >= limit {
                    return Ok(VersionMutationResult::Rejected(
                        VersionMutationRejection::AppendCount { limit },
                    ));
                }
            }
        }

        let delivery_serial = channel_state
            .next_delivery_serial
            .max(current.delivery_serial().saturating_add(1));
        let record = current.apply_mutation(&request, &channel_state.stream_id, delivery_serial)?;
        if let Some(limit) = request.limits.max_accumulated_message_bytes
            && record.data_bytes()? > limit
        {
            return Ok(VersionMutationResult::Rejected(
                VersionMutationRejection::AccumulatedMessageBytes { limit },
            ));
        }
        if !current.is_open_ai_stream()
            && record.is_open_ai_stream()
            && let Some(limit) = request.limits.max_open_streaming_messages_per_channel
        {
            let open = channel_state
                .messages
                .values()
                .filter_map(|entries| {
                    entries
                        .iter()
                        .max_by(|left, right| left.version_serial().cmp(right.version_serial()))
                })
                .filter(|entry| entry.is_open_ai_stream())
                .count();
            if open >= limit {
                return Ok(VersionMutationResult::Rejected(
                    VersionMutationRejection::OpenStreamingMessages { limit },
                ));
            }
        }

        validate_version_chain_iter(
            chain
                .iter()
                .map(|entry| &entry.message)
                .chain(std::iter::once(&record.message)),
        )?;
        if channel_state.replay.contains_key(&delivery_serial) {
            return Err(Error::InvalidMessageFormat(format!(
                "duplicate delivery_serial {delivery_serial} in version replay log"
            )));
        }

        let stored_record = Arc::new(record.clone());
        channel_state
            .messages
            .get_mut(request.message_serial.as_str())
            .ok_or_else(|| {
                Error::Internal("version chain disappeared during mutation".to_string())
            })?
            .push(Arc::clone(&stored_record));
        channel_state.created_at.insert(delivery_serial, now_ms());
        channel_state.replay.insert(delivery_serial, stored_record);
        channel_state.next_delivery_serial = delivery_serial.saturating_add(1);

        Ok(VersionMutationResult::Applied {
            record,
            stream_id: channel_state.stream_id.clone(),
        })
    }

    async fn get_latest(
        &self,
        app_id: &str,
        channel: &str,
        message_serial: &MessageSerial,
    ) -> Result<Option<StoredVersionRecord>> {
        let key = Self::channel_key(app_id, channel);
        let channels = self.channels.read().await;
        let Some(channel_state) = channels.get(&key) else {
            return Ok(None);
        };
        let Some(chain) = channel_state.messages.get(message_serial.as_str()) else {
            return Ok(None);
        };

        let latest = chain
            .iter()
            .max_by(|left, right| left.version_serial().cmp(right.version_serial()))
            .ok_or_else(|| Error::InvalidMessageFormat("version chain must not be empty".into()))?;

        Ok(Some(latest.as_ref().clone()))
    }

    async fn get_latest_batch(
        &self,
        app_id: &str,
        channel: &str,
        message_serials: &[MessageSerial],
    ) -> Result<BTreeMap<MessageSerial, StoredVersionRecord>> {
        if message_serials.is_empty() {
            return Ok(BTreeMap::new());
        }

        let key = Self::channel_key(app_id, channel);
        let channels = self.channels.read().await;
        let Some(channel_state) = channels.get(&key) else {
            return Ok(BTreeMap::new());
        };
        let requested = message_serials.iter().collect::<BTreeSet<_>>();
        requested
            .into_iter()
            .filter_map(|message_serial| {
                channel_state
                    .messages
                    .get(message_serial.as_str())
                    .map(|chain| (message_serial, chain))
            })
            .map(|(message_serial, chain)| {
                chain
                    .iter()
                    .max_by(|left, right| left.version_serial().cmp(right.version_serial()))
                    .map(|record| (message_serial.clone(), record.as_ref().clone()))
                    .ok_or_else(|| {
                        Error::InvalidMessageFormat("version chain must not be empty".into())
                    })
            })
            .collect()
    }

    async fn get_versions(&self, request: VersionStoreReadRequest) -> Result<VersionStorePage> {
        request.validate()?;
        let key = Self::channel_key(&request.app_id, &request.channel);
        let channels = self.channels.read().await;
        let Some(channel_state) = channels.get(&key) else {
            return Ok(VersionStorePage {
                items: Vec::new(),
                next_cursor: None,
                has_more: false,
            });
        };
        let Some(chain) = channel_state.messages.get(request.message_serial.as_str()) else {
            return Ok(VersionStorePage {
                items: Vec::new(),
                next_cursor: None,
                has_more: false,
            });
        };

        let mut items = chain.iter().collect::<Vec<_>>();
        items.sort_by(|left, right| left.version_serial().cmp(right.version_serial()));
        if matches!(request.direction, VersionStoreDirection::NewestFirst) {
            items.reverse();
        }

        let filtered = items
            .into_iter()
            .filter(|item| {
                request
                    .cursor
                    .as_ref()
                    .is_none_or(|cursor| match request.direction {
                        VersionStoreDirection::NewestFirst => {
                            item.version_serial() < &cursor.version_serial
                        }
                        VersionStoreDirection::OldestFirst => {
                            item.version_serial() > &cursor.version_serial
                        }
                    })
            })
            .take(request.limit + 1)
            .collect::<Vec<_>>();

        let has_more = filtered.len() > request.limit;
        let items = filtered
            .into_iter()
            .take(request.limit)
            .map(|item| item.as_ref().clone())
            .collect::<Vec<_>>();
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
        let key = Self::channel_key(&request.app_id, &request.channel);
        let channels = self.channels.read().await;
        let Some(channel_state) = channels.get(&key) else {
            return Ok(Vec::new());
        };

        let stored_items = channel_state
            .replay
            .range((request.after_delivery_serial.saturating_add(1))..)
            .map(|(_, value)| value)
            .take(request.limit)
            .collect::<Vec<_>>();

        validate_replay_continuity_iter(
            stored_items.iter().map(|entry| &entry.message),
            request.after_delivery_serial,
        )?;

        Ok(stored_items
            .into_iter()
            .map(|item| item.as_ref().clone())
            .collect())
    }

    async fn latest_by_history(
        &self,
        app_id: &str,
        channel: &str,
    ) -> Result<Vec<StoredVersionRecord>> {
        let key = Self::channel_key(app_id, channel);
        let channels = self.channels.read().await;
        let Some(channel_state) = channels.get(&key) else {
            return Ok(Vec::new());
        };

        let mut latest = channel_state
            .messages
            .values()
            .filter_map(|chain| {
                chain
                    .iter()
                    .max_by(|left, right| left.version_serial().cmp(right.version_serial()))
                    .map(|record| record.as_ref().clone())
            })
            .collect::<Vec<_>>();

        latest.sort_by_key(StoredVersionRecord::history_serial);
        Ok(latest)
    }

    async fn stream_state(&self, app_id: &str, channel: &str) -> Result<VersionStreamState> {
        let key = Self::channel_key(app_id, channel);
        let channels = self.channels.read().await;
        let Some(channel_state) = channels.get(&key) else {
            return Ok(VersionStreamState::default());
        };

        Ok(VersionStreamState {
            stream_id: Some(channel_state.stream_id.clone()),
            next_delivery_serial: Some(channel_state.next_delivery_serial),
            oldest_available_delivery_serial: channel_state
                .replay
                .first_key_value()
                .map(|(k, _)| *k),
            newest_available_delivery_serial: channel_state
                .replay
                .last_key_value()
                .map(|(k, _)| *k),
        })
    }

    async fn purge_before(&self, before_ms: i64, batch_size: usize) -> Result<(u64, bool)> {
        if batch_size == 0 {
            return Ok((0, false));
        }
        let mut channels = self.channels.write().await;
        let mut deleted: u64 = 0;
        let mut has_more = false;

        for state in channels.values_mut() {
            let remaining = batch_size.saturating_sub(deleted as usize);
            if remaining == 0 {
                has_more = true;
                break;
            }

            let mut to_remove: Vec<u64> = Vec::new();
            for (&delivery_serial, &created_ms) in state.created_at.iter() {
                if created_ms >= before_ms {
                    break;
                }
                if to_remove.len() >= remaining {
                    has_more = true;
                    break;
                }
                to_remove.push(delivery_serial);
            }

            for delivery_serial in to_remove {
                state.created_at.remove(&delivery_serial);
                let Some(record) = state.replay.remove(&delivery_serial) else {
                    continue;
                };
                let message_key = record.message_serial().as_str().to_string();
                if let Some(chain) = state.messages.get_mut(&message_key) {
                    chain.retain(|entry| entry.version_serial() != record.version_serial());
                    if chain.is_empty() {
                        state.messages.remove(&message_key);
                    }
                }
                deleted += 1;
            }

            if !has_more
                && state
                    .created_at
                    .iter()
                    .next()
                    .is_some_and(|(_, &ts)| ts < before_ms)
            {
                has_more = true;
            }
        }

        Ok((deleted, has_more))
    }
}
