//! Opt-in, test-only append diagnostics. These count application CQL values,
//! not Scylla commitlog, SSTable, replication, Paxos, or transport bytes. The
//! atomic batch counters include bound keys/preconditions and CQL value-length
//! framing. Projection counters include both historical copies and the latest
//! pointer update. No production fields or code are emitted.
use super::*;
use scylla::value::CqlValue;
use std::sync::atomic::{AtomicU64, Ordering};

static ATTEMPTS: AtomicU64 = AtomicU64::new(0);
static COMMITS: AtomicU64 = AtomicU64::new(0);
static BATCH_VALUES: AtomicU64 = AtomicU64::new(0);
static BATCH_BLOBS: AtomicU64 = AtomicU64::new(0);
static PROJECTION_VALUES: AtomicU64 = AtomicU64::new(0);
static PROJECTION_BLOBS: AtomicU64 = AtomicU64::new(0);

pub(super) struct WriteMeasurement {
    bound_bytes: u64,
    blob_bytes: u64,
}

fn enabled() -> bool {
    std::env::var_os("C2_WRITE_METRICS").is_some()
}

fn value_bytes(value: &CqlValue) -> u64 {
    match value {
        CqlValue::Blob(value) => value.len() as u64,
        CqlValue::Text(value) | CqlValue::Ascii(value) => value.len() as u64,
        CqlValue::BigInt(_) => 8,
        CqlValue::Boolean(_) => 1,
        _ => panic!("unsupported append diagnostic CQL value type"),
    }
}

impl ScyllaVersionStore {
    pub(crate) fn benchmark_write_counters() -> Vec<(String, u64)> {
        if !enabled() {
            return Vec::new();
        }
        [
            ("scylla_atomic_mutation_attempts", &ATTEMPTS),
            ("scylla_committed_atomic_mutations", &COMMITS),
            ("scylla_atomic_batch_bound_value_bytes", &BATCH_VALUES),
            ("scylla_atomic_mutation_blob_bytes", &BATCH_BLOBS),
            ("scylla_projection_bound_value_bytes", &PROJECTION_VALUES),
            ("scylla_projection_blob_bytes", &PROJECTION_BLOBS),
        ]
        .into_iter()
        .map(|(name, counter)| (name.to_string(), counter.load(Ordering::Relaxed)))
        .collect()
    }

    pub(super) fn measure_batch_before(
        values: &[Vec<Option<CqlValue>>],
    ) -> Option<WriteMeasurement> {
        if !enabled() {
            return None;
        }
        ATTEMPTS.fetch_add(1, Ordering::Relaxed);
        let bound_bytes = values
            .iter()
            .map(|row| {
                2 + row
                    .iter()
                    .map(|value| 4 + value.as_ref().map_or(0, value_bytes))
                    .sum::<u64>()
            })
            .sum();
        let blob_bytes = values
            .iter()
            .flatten()
            .filter_map(|value| match value {
                Some(CqlValue::Blob(bytes)) => Some(bytes.len() as u64),
                _ => None,
            })
            .sum();
        Some(WriteMeasurement {
            bound_bytes,
            blob_bytes,
        })
    }

    pub(super) fn measure_batch_after(measurement: Option<WriteMeasurement>) {
        if let Some(measurement) = measurement {
            BATCH_VALUES.fetch_add(measurement.bound_bytes, Ordering::Relaxed);
            BATCH_BLOBS.fetch_add(measurement.blob_bytes, Ordering::Relaxed);
            COMMITS.fetch_add(1, Ordering::Relaxed);
        }
    }

    pub(super) fn measure_projection_entry(record: &StoredVersionRecord, payload: &[u8]) {
        if !enabled() {
            return;
        }
        let texts = [
            &record.app_id,
            &record.channel,
            record.message_serial().as_str(),
            record.version_serial().as_str(),
            record.message.action.as_str(),
        ];
        let optional = [
            record.original_client_id.as_deref(),
            record.message.version.description.as_deref(),
            record.message.name.as_deref(),
        ];
        let bytes = 2
            + 14 * 4
            + 5 * 8
            + payload.len() as u64
            + texts.iter().map(|value| value.len() as u64).sum::<u64>()
            + optional
                .into_iter()
                .flatten()
                .map(|value| value.len() as u64)
                .sum::<u64>();
        PROJECTION_VALUES.fetch_add(bytes, Ordering::Relaxed);
        PROJECTION_BLOBS.fetch_add(payload.len() as u64, Ordering::Relaxed);
    }

    pub(super) fn measure_projection_latest(record: &StoredVersionRecord) {
        if !enabled() {
            return;
        }
        // The benchmark creates its message before sampling, so each append
        // takes the eight-parameter UPDATE of the existing legacy pointer.
        let texts = [
            record.version_serial().as_str(),
            record.message.action.as_str(),
            &record.app_id,
            &record.channel,
            record.message_serial().as_str(),
            record.version_serial().as_str(),
        ];
        let bytes = 2 + 8 * 4 + 2 * 8 + texts.iter().map(|value| value.len() as u64).sum::<u64>();
        PROJECTION_VALUES.fetch_add(bytes, Ordering::Relaxed);
    }
}

#[test]
fn cql_diagnostic_sizes_binary_and_multibyte_text() {
    assert_eq!(value_bytes(&CqlValue::Blob(vec![0; 4096])), 4096);
    assert_eq!(value_bytes(&CqlValue::Text("🙂é".to_string())), 6);
}
