//! Storage representation for accumulated append versions.
//!
//! Every committed append keeps its complete original operation in its own
//! version entry: fragment, version metadata, extras, envelope facts, the
//! four serial identities and any idempotency receipt. Only the accumulated
//! `data` string is factored out of the entry.
//!
//! Consecutive appends of one logical message form an *append run*. The run
//! owns a single snapshot holding the accumulated string at its head. Because
//! an append only concatenates, the accumulated data of every version in the
//! run is exactly the snapshot prefix of that version's recorded byte length.
//! Retained bytes therefore grow with the sum of fragments plus one snapshot
//! per run, instead of one complete accumulated string per version.
//!
//! Reconstruction is fail-closed: a missing snapshot, a length outside it, a
//! prefix that is not on a character boundary or does not end with the
//! entry's own fragment is an error, never a partial record.
//!
//! Compact payloads start with a discriminator field that older releases do
//! not recognize, so an older reader rejects them instead of decoding a
//! record without data. Records that are not eligible appends, imports and
//! everything written before this representation remain self-contained.
use super::types::StoredVersionRecord;
use crate::error::{Error, Result};
use crate::message_envelope::{MessageContent, MessageEnvelope};
use crate::versioned_messages::{MessageAction, MessageSerial, VersionSerial, VersionedMessage};
use serde::{Deserialize, Serialize};
use sockudo_protocol::messages::MessageData;
use std::collections::{BTreeSet, HashMap};

/// Current compact append payload format.
pub const APPEND_STORAGE_FORMAT: u8 = 1;
/// Chunked format; format 1 remains readable.
pub const CHUNKED_APPEND_STORAGE_FORMAT: u8 = 2;
/// Maximum bytes in one stored chunk, independent of UTF-8 boundaries.
pub const CHUNK_BYTES: usize = 4096;

/// Serialized compact payloads begin with this exact field.
const COMPACT_PREFIX: &[u8] = br#"{"sockudo_append_storage":"#;

/// Location of an append version's accumulated data.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppendRunRef {
    /// Unique incarnation of a chunked run. Absent on format-1 snapshots.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub generation: Option<String>,
    /// Version serial of the run's first append; unique within its message.
    pub run: VersionSerial,
    /// UTF-8 byte length of the accumulated data at this version.
    pub data_len: u64,
}

#[derive(Serialize)]
struct CompactPayloadRef<'a> {
    sockudo_append_storage: u8,
    run: &'a VersionSerial,
    data_len: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    generation: Option<&'a str>,
    record: CompactRecordRef<'a>,
}

// Abbreviate only internal format-2 field names. Aliases keep earlier chunked
// rows readable; public records and legacy format-1 serialization are unchanged.
#[derive(Serialize, Deserialize)]
#[serde(remote = "StoredVersionRecord")]
struct CompactRecord {
    #[serde(rename = "a", alias = "app_id")]
    app_id: String,
    #[serde(rename = "c", alias = "channel")]
    channel: String,
    #[serde(rename = "o", alias = "original_client_id")]
    original_client_id: Option<String>,
    #[serde(rename = "e", alias = "envelope")]
    envelope: Option<MessageEnvelope>,
    #[serde(rename = "m", alias = "message")]
    message: VersionedMessage,
}

#[derive(Serialize)]
#[serde(untagged)]
enum CompactRecordRef<'a> {
    Legacy(&'a StoredVersionRecord),
    Chunked(#[serde(with = "CompactRecord")] &'a StoredVersionRecord),
}

#[derive(Deserialize)]
struct CompactPayload {
    sockudo_append_storage: u8,
    run: VersionSerial,
    data_len: u64,
    #[serde(default)]
    generation: Option<String>,
    #[serde(with = "CompactRecord")]
    record: StoredVersionRecord,
}

/// A decoded version entry.
#[derive(Debug, Clone)]
pub enum StoredVersionPayload {
    /// Self-contained record.
    Full(StoredVersionRecord),
    /// Append whose accumulated data is a prefix of its run snapshot. The
    /// record's `message.data` and `envelope.data` are `None`.
    Compact {
        run: AppendRunRef,
        record: StoredVersionRecord,
    },
}

/// Accumulated run snapshots needed to expand a batch of payloads.
pub type AppendRunSnapshots = HashMap<(MessageSerial, VersionSerial), String>;

impl StoredVersionPayload {
    /// Decode a stored entry written by this or any earlier release.
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        if !is_compact(bytes) {
            return sonic_rs::from_slice::<StoredVersionRecord>(bytes)
                .map(Self::Full)
                .map_err(|_| Error::Internal("failed to decode version record".to_string()));
        }
        let payload: CompactPayload = sonic_rs::from_slice(bytes).map_err(|_| {
            // Parser diagnostics may contain source payload fragments.
            Error::Internal("failed to decode compact append version".to_string())
        })?;
        if !matches!(
            payload.sockudo_append_storage,
            APPEND_STORAGE_FORMAT | CHUNKED_APPEND_STORAGE_FORMAT
        ) || (payload.sockudo_append_storage == CHUNKED_APPEND_STORAGE_FORMAT
            && payload
                .generation
                .as_ref()
                .is_none_or(|s| uuid::Uuid::parse_str(s).is_err()))
            || (payload.sockudo_append_storage == APPEND_STORAGE_FORMAT
                && payload.generation.is_some())
        {
            return Err(Error::Internal(format!(
                "unsupported append storage format {}",
                payload.sockudo_append_storage
            )));
        }
        let record = payload.record;
        let valid = record.message.action == MessageAction::Append
            && record.message.data.is_none()
            && record.message.append_fragment.is_some()
            && record
                .envelope
                .as_ref()
                .is_none_or(|envelope| envelope.data.is_none());
        if !valid {
            return Err(Error::Internal(format!(
                "malformed compact append version {} for message {}",
                record.version_serial().as_str(),
                record.message_serial().as_str()
            )));
        }
        Ok(Self::Compact {
            run: AppendRunRef {
                generation: payload.generation,
                run: payload.run,
                data_len: payload.data_len,
            },
            record,
        })
    }

    /// The stored record; a compact record has no accumulated data.
    #[must_use]
    pub fn record(&self) -> &StoredVersionRecord {
        match self {
            Self::Full(record) | Self::Compact { record, .. } => record,
        }
    }

    /// The append run this entry's data lives in, if compact.
    #[must_use]
    pub fn run(&self) -> Option<&AppendRunRef> {
        match self {
            Self::Full(_) => None,
            Self::Compact { run, .. } => Some(run),
        }
    }

    /// Serialize for storage.
    pub fn encode(&self) -> Result<Vec<u8>> {
        match self {
            Self::Full(record) => encode_full(record),
            Self::Compact { run, record } => sonic_rs::to_vec(&CompactPayloadRef {
                sockudo_append_storage: if run.generation.is_some() {
                    CHUNKED_APPEND_STORAGE_FORMAT
                } else {
                    APPEND_STORAGE_FORMAT
                },
                generation: run.generation.as_deref(),
                run: &run.run,
                data_len: run.data_len,
                record: if run.generation.is_some() {
                    CompactRecordRef::Chunked(record)
                } else {
                    CompactRecordRef::Legacy(record)
                },
            })
            .map_err(|e| Error::Internal(format!("failed to encode compact append version: {e}"))),
        }
    }

    /// Materialize the public full-state record. `snapshot` is the run's
    /// accumulated data and is required for compact entries.
    pub fn into_record(self, snapshot: Option<&str>) -> Result<StoredVersionRecord> {
        match self {
            Self::Full(record) => Ok(record),
            Self::Compact { run, mut record } => {
                let Some(snapshot) = snapshot else {
                    return Err(Error::Internal(format!(
                        "append storage run {} for message {} is missing",
                        run.run.as_str(),
                        record.message_serial().as_str()
                    )));
                };
                let data = prefix(&run, &record, snapshot)?;
                if let Some(envelope) = record.envelope.as_mut() {
                    envelope.data = Some(MessageContent::Text(data.to_owned()));
                }
                record.message.data = Some(MessageData::String(data.to_owned()));
                Ok(record)
            }
        }
    }
}

fn prefix<'a>(
    run: &AppendRunRef,
    record: &StoredVersionRecord,
    snapshot: &'a str,
) -> Result<&'a str> {
    let invalid = |reason: &str| {
        Error::Internal(format!(
            "append storage run {} cannot reconstruct version {} of message {}: {reason}",
            run.run.as_str(),
            record.version_serial().as_str(),
            record.message_serial().as_str()
        ))
    };
    let len = usize::try_from(run.data_len).map_err(|_| invalid("length overflow"))?;
    // `get` rejects both out-of-range lengths and non-boundary splits.
    let data = snapshot
        .get(..len)
        .ok_or_else(|| invalid("length is outside the snapshot"))?;
    let fragment = record
        .message
        .append_fragment
        .as_deref()
        .ok_or_else(|| invalid("fragment is missing"))?;
    if !data.ends_with(fragment) {
        return Err(invalid("snapshot does not contain the fragment"));
    }
    Ok(data)
}

/// Whether stored entry bytes are a compact append payload.
#[must_use]
pub fn is_compact(bytes: &[u8]) -> bool {
    bytes.starts_with(COMPACT_PREFIX)
}

/// Serialize a self-contained record exactly as earlier releases did.
pub fn encode_full(record: &StoredVersionRecord) -> Result<Vec<u8>> {
    sonic_rs::to_vec(record)
        .map_err(|e| Error::Internal(format!("failed to serialize version record: {e}")))
}

/// Validate and convert stored snapshot bytes.
pub fn snapshot_from_bytes(bytes: Vec<u8>) -> Result<String> {
    String::from_utf8(bytes)
        .map_err(|_| Error::Internal("append storage snapshot is not valid UTF-8".to_string()))
}

/// Distinct `(message, run)` snapshots needed by `payloads`.
#[must_use]
pub fn required_runs<'a>(
    payloads: impl IntoIterator<Item = &'a StoredVersionPayload>,
) -> BTreeSet<(MessageSerial, VersionSerial)> {
    payloads
        .into_iter()
        .filter_map(|payload| {
            payload
                .run()
                .map(|run| (payload.record().message_serial().clone(), run.run.clone()))
        })
        .collect()
}

/// Materialize every payload, failing closed when a snapshot is missing.
pub fn expand_payloads(
    payloads: Vec<StoredVersionPayload>,
    snapshots: &AppendRunSnapshots,
) -> Result<Vec<StoredVersionRecord>> {
    payloads
        .into_iter()
        .map(|payload| {
            let snapshot = payload.run().and_then(|run| {
                snapshots
                    .get(&(payload.record().message_serial().clone(), run.run.clone()))
                    .map(String::as_str)
            });
            payload.into_record(snapshot)
        })
        .collect()
}

/// The accumulated data and fragment of an append that can be represented
/// as a run snapshot prefix.
fn compactable(record: &StoredVersionRecord) -> Option<(&str, &str)> {
    if record.message.action != MessageAction::Append {
        return None;
    }
    let Some(MessageData::String(data)) = record.message.data.as_ref() else {
        return None;
    };
    let fragment = record.message.append_fragment.as_deref()?;
    if !data.ends_with(fragment) {
        return None;
    }
    let envelope_matches = record.envelope.as_ref().is_none_or(
        |envelope| matches!(&envelope.data, Some(MessageContent::Text(text)) if text == data),
    );
    envelope_matches.then_some((data.as_str(), fragment))
}

/// How to persist a newly applied version.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AppendRunPlan {
    /// Persist the self-contained record.
    Full,
    /// Begin run `run.run` whose snapshot is the version's accumulated data.
    Start { run: AppendRunRef },
    /// Append the version's fragment to run `run.run`, whose head must still
    /// be `expected_head` with a snapshot of `expected_len` bytes.
    Extend {
        run: AppendRunRef,
        expected_head: VersionSerial,
        expected_len: u64,
    },
}

impl AppendRunPlan {
    /// Plan the storage of `record`, derived from the predecessor with serial
    /// `predecessor_serial`. `predecessor_run` is the predecessor's run when
    /// it is stored compactly; backends must verify that the predecessor is
    /// still that run's head in the same atomic commit.
    #[must_use]
    pub fn for_record(
        predecessor_serial: &VersionSerial,
        predecessor_run: Option<&AppendRunRef>,
        record: &StoredVersionRecord,
    ) -> Self {
        let Some((data, fragment)) = compactable(record) else {
            return Self::Full;
        };
        let data_len = data.len() as u64;
        match predecessor_run {
            Some(previous)
                if previous.data_len.checked_add(fragment.len() as u64) == Some(data_len) =>
            {
                Self::Extend {
                    run: AppendRunRef {
                        generation: previous.generation.clone(),
                        run: previous.run.clone(),
                        data_len,
                    },
                    expected_head: predecessor_serial.clone(),
                    expected_len: previous.data_len,
                }
            }
            _ => Self::Start {
                run: AppendRunRef {
                    generation: None,
                    run: record.version_serial().clone(),
                    data_len,
                },
            },
        }
    }

    /// Seed immutable starting data during maintenance or a non-append write.
    /// Backends keep the public entry full and publish the seed pointer only
    /// atomically with that entry. The first subsequent append extends it.
    #[must_use]
    pub fn for_seed_record(record: &StoredVersionRecord) -> Self {
        let Some(MessageData::String(data)) = record.message.data.as_ref() else {
            return Self::Full;
        };
        Self::Start {
            run: AppendRunRef {
                generation: Some(uuid::Uuid::new_v4().to_string()),
                run: record.version_serial().clone(),
                data_len: data.len() as u64,
            },
        }
    }

    /// Plan a chunked run, starting a new incarnation after a full or format-1 row.
    #[must_use]
    pub fn for_record_chunked(
        predecessor_serial: &VersionSerial,
        predecessor_run: Option<&AppendRunRef>,
        record: &StoredVersionRecord,
    ) -> Self {
        let mut plan = Self::for_record(
            predecessor_serial,
            predecessor_run.filter(|run| run.generation.is_some()),
            record,
        );
        if let Self::Start { run } = &mut plan {
            run.generation = Some(uuid::Uuid::new_v4().to_string());
        }
        plan
    }

    /// Return only the changed tail and newly appended chunks. Chunk boundaries
    /// are byte boundaries; readers validate UTF-8 only after joining the prefix.
    /// A new run currently bootstraps the predecessor's complete data once.
    pub fn chunk_writes(&self, record: &StoredVersionRecord) -> Result<Vec<AppendChunkWrite>> {
        self.chunk_writes_sized(record, CHUNK_BYTES)
    }

    /// Use a smaller, backend-fixed physical chunk size (for inline SQL rows).
    pub fn chunk_writes_sized(
        &self,
        record: &StoredVersionRecord,
        chunk_bytes: usize,
    ) -> Result<Vec<AppendChunkWrite>> {
        if chunk_bytes == 0 || chunk_bytes > CHUNK_BYTES {
            return Err(Error::Internal(
                "invalid append storage chunk size".to_string(),
            ));
        }
        let Some(data) = self.snapshot_after(record) else {
            return Ok(Vec::new());
        };
        if self.run().is_none_or(|run| run.generation.is_none()) {
            return Err(Error::Internal("chunk write requires format 2".to_string()));
        }
        let start = match self {
            Self::Extend { expected_len, .. } => usize::try_from(*expected_len)
                .map_err(|_| Error::Internal("append length overflow".to_string()))?,
            _ => 0,
        };
        if start > data.len() {
            return Err(Error::Internal(
                "append predecessor exceeds data length".to_string(),
            ));
        }
        if start == data.len() && start != 0 {
            return Ok(Vec::new());
        }
        let first = start / chunk_bytes;
        Ok(data.as_bytes()[first * chunk_bytes..]
            .chunks(chunk_bytes)
            .enumerate()
            .map(|(index, bytes)| AppendChunkWrite {
                index: (first + index) as u64,
                bytes: bytes.to_vec(),
            })
            .collect())
    }

    /// The run the record is stored in, if any.
    #[must_use]
    pub fn run(&self) -> Option<&AppendRunRef> {
        match self {
            Self::Full => None,
            Self::Start { run } | Self::Extend { run, .. } => Some(run),
        }
    }

    /// Bytes to write into the run snapshot: the whole accumulated data for
    /// a new run, only the fragment when extending.
    #[must_use]
    pub fn snapshot_write<'a>(&self, record: &'a StoredVersionRecord) -> Option<&'a str> {
        let (data, fragment) = compactable(record)?;
        match self {
            Self::Full => None,
            Self::Start { .. } => Some(data),
            Self::Extend { .. } => Some(fragment),
        }
    }

    /// The complete run snapshot after committing `record`, for backends
    /// that cannot append to a stored value.
    #[must_use]
    pub fn snapshot_after<'a>(&self, record: &'a StoredVersionRecord) -> Option<&'a str> {
        self.run()?;
        match record.message.data.as_ref()? {
            MessageData::String(data) => Some(data.as_str()),
            _ => None,
        }
    }

    /// The stored payload for `record` under this plan.
    #[must_use]
    pub fn payload(&self, record: &StoredVersionRecord) -> StoredVersionPayload {
        match self.run() {
            None => StoredVersionPayload::Full(record.clone()),
            Some(run) => StoredVersionPayload::Compact {
                run: run.clone(),
                record: without_data(record),
            },
        }
    }

    /// Serialize `record` under this plan without cloning its data.
    pub fn encode(&self, record: &StoredVersionRecord) -> Result<Vec<u8>> {
        match self.run() {
            None => encode_full(record),
            Some(run) => StoredVersionPayload::Compact {
                run: run.clone(),
                record: without_data(record),
            }
            .encode(),
        }
    }
}

/// Copy a record without its accumulated data. Fields are listed
/// exhaustively so a new field cannot be silently dropped.
#[must_use]
pub fn without_data(record: &StoredVersionRecord) -> StoredVersionRecord {
    let message = &record.message;
    StoredVersionRecord {
        app_id: record.app_id.clone(),
        channel: record.channel.clone(),
        original_client_id: record.original_client_id.clone(),
        envelope: record.envelope.as_ref().map(|envelope| MessageEnvelope {
            message_id: envelope.message_id.clone(),
            acknowledgement_id: envelope.acknowledgement_id.clone(),
            name: envelope.name.clone(),
            data: None,
            encoding: envelope.encoding.clone(),
            publisher_client_id: envelope.publisher_client_id.clone(),
            publisher_socket_id: envelope.publisher_socket_id.clone(),
            publisher_connection_id: envelope.publisher_connection_id.clone(),
            published_at_ms: envelope.published_at_ms,
            extras: envelope.extras.clone(),
            stream_id: envelope.stream_id.clone(),
            history_serial: envelope.history_serial,
            delivery_serial: envelope.delivery_serial,
            action: envelope.action,
            message_serial: envelope.message_serial.clone(),
            version: envelope.version.clone(),
            idempotency: envelope.idempotency.clone(),
        }),
        message: VersionedMessage {
            action: message.action,
            identity: message.identity.clone(),
            replay_position: message.replay_position.clone(),
            version: message.version.clone(),
            name: message.name.clone(),
            data: None,
            extras: message.extras.clone(),
            append_fragment: message.append_fragment.clone(),
        },
    }
}

/// One bounded chunk mutation to include in the entry's atomic commit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppendChunkWrite {
    /// Zero-based byte-chunk index within the run.
    pub index: u64,
    /// At most [`CHUNK_BYTES`] bytes; may split a UTF-8 character.
    pub bytes: Vec<u8>,
}

/// Bounded, process-local cache of validated immutable run prefixes. Every key
/// includes the run incarnation; extensions never invalidate older prefixes.
/// Format-1 runs are intentionally never cached (their IDs may be replaced).
#[derive(Debug)]
pub struct AppendSnapshotCache {
    state: parking_lot::Mutex<SnapshotCacheState>,
    max_bytes: usize,
}

#[derive(Debug, Default)]
struct SnapshotCacheState {
    entries: std::collections::VecDeque<(String, String)>,
    bytes: usize,
}

impl Default for AppendSnapshotCache {
    fn default() -> Self {
        Self::new(8 * 1024 * 1024)
    }
}

impl AppendSnapshotCache {
    /// Bound includes key and value bytes plus conservative entry overhead.
    #[must_use]
    pub fn new(max_bytes: usize) -> Self {
        Self {
            state: parking_lot::Mutex::new(SnapshotCacheState::default()),
            max_bytes,
        }
    }

    fn key(
        app: &str,
        channel: &str,
        message: &MessageSerial,
        run: &AppendRunRef,
    ) -> Option<String> {
        // Length framing prevents tenant/channel/key ambiguities.
        let generation = run.generation.as_ref()?;
        Some(format!(
            "{}:{}{}:{}{}:{}{}:{}{}:{}",
            app.len(),
            app,
            channel.len(),
            channel,
            message.as_str().len(),
            message.as_str(),
            run.run.as_str().len(),
            run.run.as_str(),
            generation.len(),
            generation
        ))
    }

    /// Returns only a sufficiently long prefix. Callers must still perform the
    /// normal length, UTF-8 boundary and entry-fragment validation.
    #[must_use]
    pub fn get(
        &self,
        app: &str,
        channel: &str,
        message: &MessageSerial,
        run: &AppendRunRef,
    ) -> Option<String> {
        let key = Self::key(app, channel, message, run)?;
        let len = usize::try_from(run.data_len).ok()?;
        let state = self.state.lock();
        let (_, data) = state.entries.iter().find(|(k, _)| *k == key)?;
        data.get(..len).map(str::to_owned)
    }

    /// Admit a validated prefix; oversized snapshots are never cached.
    pub fn insert(
        &self,
        app: &str,
        channel: &str,
        message: &MessageSerial,
        run: &AppendRunRef,
        data: String,
    ) {
        let Some(key) = Self::key(app, channel, message, run) else {
            return;
        };
        let cost = key
            .capacity()
            .saturating_add(data.capacity())
            .saturating_add(128);
        if cost > self.max_bytes {
            return;
        }
        let mut state = self.state.lock();
        if let Some(index) = state.entries.iter().position(|(k, _)| *k == key) {
            if state.entries[index].1.len() >= data.len() {
                return;
            }
            let (old_key, old_data) = state.entries.remove(index).expect("located cache entry");
            state.bytes -= old_key.capacity() + old_data.capacity() + 128;
        }
        while state.bytes > self.max_bytes - cost {
            if let Some((key, data)) = state.entries.pop_front() {
                state.bytes -= key.capacity() + data.capacity() + 128;
            } else {
                break;
            }
        }
        state.bytes += cost;
        state.entries.push_back((key, data));
    }
}
