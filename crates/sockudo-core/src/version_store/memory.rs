use super::append_storage::{AppendRunPlan, AppendRunRef, StoredVersionPayload, without_data};
use super::store::VersionStore;
use super::types::*;
use crate::error::{Error, Result};
use crate::history::now_ms;
use crate::versioned_messages::{
    MessageAction, MessageSerial, VersionSerial, ensure_same_chain,
    validate_replay_continuity_iter, validate_version_chain,
};
use async_trait::async_trait;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
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
    messages: BTreeMap<String, VersionChain>,
    open_stream_count: usize,
    replay: BTreeMap<u64, Arc<MemoryVersionEntry>>,
    // Parallel map: `delivery_serial -> server-side append time (ms)`.
    // Used by `purge_before` for TTL eviction without touching read paths.
    created_at: BTreeMap<u64, i64>,
}

/// One retained version. A compact entry keeps the complete original
/// operation but not the accumulated data, which is the prefix of its
/// chain's run snapshot of `run.data_len` bytes.
struct MemoryVersionEntry {
    record: StoredVersionRecord,
    run: Option<AppendRunRef>,
}

/// Accumulated data shared by consecutive appends of one message.
#[derive(Clone)]
struct AppendRun {
    snapshot: String,
    head: VersionSerial,
    // Retained entries stored in this run; the snapshot is freed at zero.
    refs: usize,
}

// All indexes and replay are updated under the existing store write lock. No
// second lock is acquired. Retained records are immutable; only purge rebuilds
// indexes. Import order is independent of version-serial order.
#[derive(Clone, Default)]
struct VersionChain {
    entries: Vec<Arc<MemoryVersionEntry>>,
    versions: HashSet<VersionSerial>,
    operations: HashMap<String, usize>,
    latest: Option<usize>,
    append_count: usize,
    runs: HashMap<VersionSerial, AppendRun>,
}

impl VersionChain {
    fn latest(&self) -> Option<&Arc<MemoryVersionEntry>> {
        self.latest.map(|index| &self.entries[index])
    }

    /// The public full-state record of a retained entry.
    fn materialize(&self, entry: &MemoryVersionEntry) -> Result<StoredVersionRecord> {
        let Some(run) = entry.run.as_ref() else {
            return Ok(entry.record.clone());
        };
        StoredVersionPayload::Compact {
            run: run.clone(),
            record: entry.record.clone(),
        }
        .into_record(self.runs.get(&run.run).map(|state| state.snapshot.as_str()))
    }

    fn materialize_latest(&self) -> Result<StoredVersionRecord> {
        let latest = self
            .latest()
            .ok_or_else(|| Error::InvalidMessageFormat("version chain must not be empty".into()))?;
        self.materialize(latest)
    }

    fn validate_incoming(&self, record: &StoredVersionRecord) -> Result<()> {
        // The full-chain validator remains available for import/repair callers.
        // Every retained predecessor was validated when inserted. Comparing its
        // chain identity plus indexed uniqueness is sufficient for one new row.
        if let Some(first) = self.entries.first() {
            ensure_same_chain(&first.record.message, &record.message)?;
        }
        if self.versions.contains(record.version_serial()) {
            return Err(Error::InvalidMessageFormat(format!(
                "duplicate version_serial {} in version chain",
                record.version_serial().as_str()
            )));
        }
        Ok(())
    }

    /// Store `record` under `plan`, updating its run snapshot first. Every
    /// check precedes the first mutation, so a failure leaves the chain as is.
    fn commit(
        &mut self,
        record: &StoredVersionRecord,
        plan: &AppendRunPlan,
    ) -> Result<Arc<MemoryVersionEntry>> {
        let conflict = |reason: &str| {
            Error::Internal(format!(
                "append run for version {} of message {} {reason}",
                record.version_serial().as_str(),
                record.message_serial().as_str()
            ))
        };
        let write = plan.snapshot_write(record);
        match plan {
            AppendRunPlan::Full => {}
            AppendRunPlan::Start { run } => {
                let snapshot = write.ok_or_else(|| conflict("has no accumulated data"))?;
                if self.runs.contains_key(&run.run) {
                    return Err(conflict("already exists"));
                }
                self.runs.insert(
                    run.run.clone(),
                    AppendRun {
                        snapshot: snapshot.to_owned(),
                        head: record.version_serial().clone(),
                        refs: 1,
                    },
                );
            }
            AppendRunPlan::Extend {
                run,
                expected_head,
                expected_len,
            } => {
                let fragment = write.ok_or_else(|| conflict("has no fragment"))?;
                let state = self
                    .runs
                    .get_mut(&run.run)
                    .ok_or_else(|| conflict("is missing"))?;
                if &state.head != expected_head || state.snapshot.len() as u64 != *expected_len {
                    return Err(conflict("head moved"));
                }
                state.snapshot.push_str(fragment);
                state.head = record.version_serial().clone();
                state.refs += 1;
            }
        }
        let entry = Arc::new(match plan.run() {
            None => MemoryVersionEntry {
                record: record.clone(),
                run: None,
            },
            Some(run) => MemoryVersionEntry {
                record: without_data(record),
                run: Some(run.clone()),
            },
        });
        self.index(Arc::clone(&entry));
        Ok(entry)
    }

    fn insert_full(&mut self, record: StoredVersionRecord) -> Arc<MemoryVersionEntry> {
        let entry = Arc::new(MemoryVersionEntry { record, run: None });
        self.index(Arc::clone(&entry));
        entry
    }

    fn index(&mut self, entry: Arc<MemoryVersionEntry>) {
        let index = self.entries.len();
        let record = &entry.record;
        if self
            .latest()
            .is_none_or(|latest| record.version_serial() > latest.record.version_serial())
        {
            self.latest = Some(index);
        }
        self.versions.insert(record.version_serial().clone());
        if let Some(operation) = record
            .envelope
            .as_ref()
            .and_then(|envelope| envelope.idempotency.as_ref())
        {
            // Imports historically permit repeated operation keys; preserve the
            // first inserted matching receipt, including after partial purge.
            self.operations
                .entry(operation.cache_key.clone())
                .or_insert(index);
        }
        self.append_count += usize::from(record.message.action == MessageAction::Append);
        self.entries.push(entry);
    }

    fn remove(&mut self, version: &VersionSerial) {
        let entries = std::mem::take(&mut self.entries);
        self.versions.clear();
        self.operations.clear();
        self.latest = None;
        self.append_count = 0;
        for entry in entries {
            if entry.record.version_serial() != version {
                self.index(entry);
                continue;
            }
            if let Some(run) = entry.run.as_ref()
                && let Some(state) = self.runs.get_mut(&run.run)
            {
                state.refs -= 1;
                if state.refs == 0 {
                    self.runs.remove(&run.run);
                }
            }
        }
    }
}

impl Default for MemoryVersionChannel {
    fn default() -> Self {
        Self {
            stream_id: uuid::Uuid::new_v4().to_string(),
            next_delivery_serial: 1,
            messages: BTreeMap::new(),
            open_stream_count: 0,
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

/// Retained append storage, for tests.
#[cfg(test)]
#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct AppendStorageStats {
    pub compact_entries: usize,
    pub full_entries: usize,
    pub runs: usize,
    pub snapshot_bytes: usize,
    /// Accumulated data bytes held by retained entries themselves.
    pub entry_data_bytes: usize,
}

#[cfg(test)]
impl MemoryVersionStore {
    pub(crate) async fn append_storage_stats(&self) -> AppendStorageStats {
        let channels = self.channels.read().await;
        let mut stats = AppendStorageStats::default();
        for chain in channels
            .values()
            .flat_map(|channel| channel.messages.values())
        {
            for entry in &chain.entries {
                if entry.run.is_some() {
                    stats.compact_entries += 1;
                } else {
                    stats.full_entries += 1;
                }
                stats.entry_data_bytes += entry.record.data_bytes().unwrap_or_default();
            }
            stats.runs += chain.runs.len();
            stats.snapshot_bytes += chain
                .runs
                .values()
                .map(|run| run.snapshot.len())
                .sum::<usize>();
        }
        stats
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
                existing.record.message_serial().as_str(),
                record.message_serial().as_str()
            )));
        }

        let message_serial = record.message_serial().as_str().to_owned();
        if let Some(chain) = channel_state.messages.get(&message_serial) {
            chain.validate_incoming(&record)?;
        } else {
            validate_version_chain(std::slice::from_ref(&record.message))?;
        }

        let was_open = channel_state
            .messages
            .get(&message_serial)
            .and_then(VersionChain::latest)
            .is_some_and(|entry| entry.record.is_open_ai_stream());
        // Imports carry arbitrary predecessors and stay self-contained.
        let delivery_serial = record.delivery_serial();
        let entry = channel_state
            .messages
            .entry(message_serial.clone())
            .or_default()
            .insert_full(record);
        let is_open = channel_state.messages[&message_serial]
            .latest()
            .is_some_and(|entry| entry.record.is_open_ai_stream());
        channel_state.open_stream_count =
            channel_state.open_stream_count - usize::from(was_open) + usize::from(is_open);
        channel_state.created_at.insert(delivery_serial, now_ms());
        channel_state.replay.insert(delivery_serial, entry);
        channel_state.next_delivery_serial = channel_state
            .next_delivery_serial
            .max(delivery_serial.saturating_add(1));

        Ok(())
    }

    async fn commit_create(&self, request: VersionCreateRequest) -> Result<VersionCreateResult> {
        let key = Self::channel_key(&request.record.app_id, &request.record.channel);
        let mut channels = self.channels.write().await;
        let channel_state = channels.entry(key).or_default();

        if let Some(chain) = channel_state
            .messages
            .get(request.record.message_serial().as_str())
            .filter(|chain| chain.latest().is_some())
        {
            return Ok(VersionCreateResult::Conflict {
                current: Some(chain.materialize_latest()?),
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
            let open = channel_state.open_stream_count;
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
        let mut chain = VersionChain::default();
        let entry = chain.commit(&record, &AppendRunPlan::Full)?;
        channel_state.open_stream_count += usize::from(record.is_open_ai_stream());
        channel_state
            .messages
            .insert(record.message_serial().as_str().to_string(), chain);
        channel_state.created_at.insert(delivery_serial, now_ms());
        channel_state.replay.insert(delivery_serial, entry);
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
            && let Some(existing) = chain
                .operations
                .get(&incoming.cache_key)
                .map(|&index| &chain.entries[index])
        {
            let existing_idempotency = existing
                .record
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
                record: chain.materialize(existing)?,
                stream_id: channel_state.stream_id.clone(),
            });
        }

        let latest = chain.latest().ok_or_else(|| {
            Error::InvalidMessageFormat("version chain must not be empty".to_string())
        })?;
        if !request.expected.matches(&latest.record) {
            return Ok(VersionMutationResult::Conflict {
                current: Some(chain.materialize(latest)?),
            });
        }
        // Extend only a run whose head is still exactly this predecessor.
        let predecessor_run = latest
            .run
            .as_ref()
            .filter(|run| {
                chain.runs.get(&run.run).is_some_and(|state| {
                    &state.head == latest.record.version_serial()
                        && state.snapshot.len() as u64 == run.data_len
                })
            })
            .cloned();
        let current = chain.materialize(latest)?;

        if matches!(request.mutation, VersionMutation::Append(_)) {
            if request.limits.reject_append_after_terminal && Self::is_terminal(&current) {
                return Ok(VersionMutationResult::Rejected(
                    VersionMutationRejection::TerminalMessage,
                ));
            }
            if let Some(limit) = request.limits.max_appends_per_message {
                let append_count = chain.append_count;
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
        let plan =
            AppendRunPlan::for_record(current.version_serial(), predecessor_run.as_ref(), &record);
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
            let open = channel_state.open_stream_count;
            if open >= limit {
                return Ok(VersionMutationResult::Rejected(
                    VersionMutationRejection::OpenStreamingMessages { limit },
                ));
            }
        }

        chain.validate_incoming(&record)?;
        if channel_state.replay.contains_key(&delivery_serial) {
            return Err(Error::InvalidMessageFormat(format!(
                "duplicate delivery_serial {delivery_serial} in version replay log"
            )));
        }

        let entry = channel_state
            .messages
            .get_mut(request.message_serial.as_str())
            .ok_or_else(|| {
                Error::Internal("version chain disappeared during mutation".to_string())
            })?
            .commit(&record, &plan)?;
        channel_state.open_stream_count = channel_state.open_stream_count
            - usize::from(current.is_open_ai_stream())
            + usize::from(record.is_open_ai_stream());
        channel_state.created_at.insert(delivery_serial, now_ms());
        channel_state.replay.insert(delivery_serial, entry);
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

        chain.materialize_latest().map(Some)
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
                    .materialize_latest()
                    .map(|record| (message_serial.clone(), record))
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

        let mut items = chain.entries.iter().collect::<Vec<_>>();
        items.sort_by(|left, right| {
            left.record
                .version_serial()
                .cmp(right.record.version_serial())
        });
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
                            item.record.version_serial() < &cursor.version_serial
                        }
                        VersionStoreDirection::OldestFirst => {
                            item.record.version_serial() > &cursor.version_serial
                        }
                    })
            })
            .take(request.limit + 1)
            .collect::<Vec<_>>();

        let has_more = filtered.len() > request.limit;
        let items = filtered
            .into_iter()
            .take(request.limit)
            .map(|item| chain.materialize(item))
            .collect::<Result<Vec<_>>>()?;
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
            stored_items.iter().map(|entry| &entry.record.message),
            request.after_delivery_serial,
        )?;

        stored_items
            .into_iter()
            .map(|entry| match entry.run {
                None => Ok(entry.record.clone()),
                Some(_) => channel_state
                    .messages
                    .get(entry.record.message_serial().as_str())
                    .ok_or_else(|| {
                        Error::Internal(format!(
                            "version chain for message {} is missing its replay entry",
                            entry.record.message_serial().as_str()
                        ))
                    })?
                    .materialize(entry),
            })
            .collect()
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
            .filter(|chain| chain.latest().is_some())
            .map(VersionChain::materialize_latest)
            .collect::<Result<Vec<_>>>()?;

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
                let message_key = record.record.message_serial().as_str().to_string();
                if let Some(chain) = state.messages.get_mut(&message_key) {
                    let was_open = chain
                        .latest()
                        .is_some_and(|entry| entry.record.is_open_ai_stream());
                    chain.remove(record.record.version_serial());
                    let is_open = chain
                        .latest()
                        .is_some_and(|entry| entry.record.is_open_ai_stream());
                    state.open_stream_count =
                        state.open_stream_count - usize::from(was_open) + usize::from(is_open);
                    if chain.entries.is_empty() {
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
