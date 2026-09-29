//! Opt-in volume diagnostics, compiled only into tests. Strong pre/post reads
//! make these volume-only samples unsuitable for latency comparisons. Counters
//! cover successful compare_and_apply publication transactions only. They
//! exclude staged chunk batches, staging lease claims/releases, seed writes,
//! maintenance and create. Therefore these are not total write-volume counters
//! for large staged appends; the transport proxy observes all sent bytes.
//! Item sizing follows AWS's approximate number representation and excludes
//! per-item storage overhead, GSI writes, replication and physical I/O.
use super::*;
use std::sync::atomic::{AtomicU64, Ordering};

static ATTEMPTS: AtomicU64 = AtomicU64::new(0);
static COMMITS: AtomicU64 = AtomicU64::new(0);
static ITEM_BYTES: AtomicU64 = AtomicU64::new(0);
static PAYLOAD_BYTES: AtomicU64 = AtomicU64::new(0);
static WCU_UNITS: AtomicU64 = AtomicU64::new(0);

pub(super) struct WriteMeasurement(Vec<(String, HashMap<String, AttributeValue>, u64)>);

fn number_bytes(number: &str) -> u64 {
    let mantissa = number.split(['e', 'E']).next().unwrap_or(number);
    let digits = mantissa
        .bytes()
        .filter(u8::is_ascii_digit)
        .collect::<Vec<_>>();
    let significant = digits
        .split(|byte| *byte != b'0')
        .next()
        .map_or(0, <[u8]>::len);
    let trimmed = &digits[significant..];
    let count = trimmed
        .iter()
        .rposition(|digit| *digit != b'0')
        .map_or(1, |index| index + 1);
    count.div_ceil(2) as u64 + 1
}

fn value_bytes(value: &AttributeValue) -> u64 {
    match value {
        AttributeValue::S(value) => value.len() as u64,
        AttributeValue::N(value) => number_bytes(value),
        AttributeValue::B(value) => value.as_ref().len() as u64,
        AttributeValue::Bool(_) | AttributeValue::Null(_) => 1,
        AttributeValue::L(values) => {
            3 + values
                .iter()
                .map(|value| 1 + value_bytes(value))
                .sum::<u64>()
        }
        AttributeValue::M(values) => 3 + values.len() as u64 + item_bytes(values),
        AttributeValue::Ss(values) => values.iter().map(|value| value.len() as u64).sum(),
        AttributeValue::Ns(values) => values.iter().map(|value| number_bytes(value)).sum(),
        AttributeValue::Bs(values) => values.iter().map(|value| value.as_ref().len() as u64).sum(),
        _ => 0,
    }
}

fn item_bytes(item: &HashMap<String, AttributeValue>) -> u64 {
    item.iter()
        .map(|(key, value)| key.len() as u64 + value_bytes(value))
        .sum()
}

impl DynamoDbVersionStore {
    /// Cumulative diagnostic counters; the benchmark takes before/after deltas.
    pub(crate) fn benchmark_write_counters() -> Vec<(String, u64)> {
        if std::env::var_os("C2_WRITE_METRICS").is_none() {
            return Vec::new();
        }
        [
            ("dynamodb_mutation_attempts", &ATTEMPTS),
            ("dynamodb_committed_mutations", &COMMITS),
            ("dynamodb_base_table_write_item_bytes_estimate", &ITEM_BYTES),
            ("dynamodb_committed_payload_attribute_bytes", &PAYLOAD_BYTES),
            ("dynamodb_transactional_base_table_wcu_estimate", &WCU_UNITS),
        ]
        .into_iter()
        .map(|(name, counter)| (name.to_string(), counter.load(Ordering::Relaxed)))
        .collect()
    }

    pub(super) async fn measure_write_before(
        &self,
        writes: &[TransactWriteItem],
    ) -> Result<Option<WriteMeasurement>> {
        if std::env::var_os("C2_WRITE_METRICS").is_none() {
            return Ok(None);
        }
        ATTEMPTS.fetch_add(1, Ordering::Relaxed);
        let mut measured = Vec::new();
        for write in writes {
            let (table, key) = if let Some(update) = write.update.as_ref() {
                (update.table_name.clone(), update.key.clone())
            } else if let Some(put) = write.put.as_ref() {
                let mut keys = vec!["app_channel"];
                if put.table_name == self.tables.version_entries {
                    keys.push("message_version_key");
                }
                if put.table_name == self.tables.version_messages {
                    keys.push("message_serial");
                }
                let key = keys
                    .into_iter()
                    .map(|key| {
                        put.item
                            .get(key)
                            .cloned()
                            .map(|value| (key.to_string(), value))
                            .ok_or_else(|| {
                                Error::Internal("diagnostic write item has no key".to_string())
                            })
                    })
                    .collect::<Result<HashMap<_, _>>>()?;
                (put.table_name.clone(), key)
            } else {
                continue;
            };
            let item = self
                .client
                .get_item()
                .table_name(&table)
                .set_key(Some(key.clone()))
                .consistent_read(true)
                .send()
                .await
                .map_err(|e| Error::Internal(format!("failed to measure pre-write item: {e}")))?
                .item;
            measured.push((table, key, item.as_ref().map_or(0, item_bytes)));
        }
        Ok(Some(WriteMeasurement(measured)))
    }

    pub(super) async fn measure_write_after(
        &self,
        measurement: Option<WriteMeasurement>,
    ) -> Result<()> {
        let Some(WriteMeasurement(items)) = measurement else {
            return Ok(());
        };
        let (mut bytes, mut payload_bytes, mut units) = (0, 0, 0);
        for (table, key, before) in items {
            let item = self
                .client
                .get_item()
                .table_name(table)
                .set_key(Some(key))
                .consistent_read(true)
                .send()
                .await
                .map_err(|e| {
                    Error::Internal(format!("failed to measure committed write item: {e}"))
                })?
                .item
                .ok_or_else(|| {
                    Error::Internal("diagnostic committed item is missing".to_string())
                })?;
            let charged = before.max(item_bytes(&item));
            bytes += charged;
            // Transactional base-table writes consume two units per KiB,
            // based on the larger before/after item; this excludes GSI costs.
            units += 2 * charged.div_ceil(1024);
            payload_bytes += ["payload_bytes", "latest_payload_bytes"]
                .iter()
                .filter_map(|key| item.get(*key))
                .map(value_bytes)
                .sum::<u64>();
        }
        ITEM_BYTES.fetch_add(bytes, Ordering::Relaxed);
        PAYLOAD_BYTES.fetch_add(payload_bytes, Ordering::Relaxed);
        WCU_UNITS.fetch_add(units, Ordering::Relaxed);
        COMMITS.fetch_add(1, Ordering::Relaxed);
        Ok(())
    }
}

#[test]
fn item_size_estimate_counts_binary_without_base64_expansion() {
    assert_eq!(number_bytes("1000"), 2);
    assert_eq!(number_bytes("123456"), 4);
    let item = HashMap::from([
        (
            "data".to_string(),
            AttributeValue::B(Blob::new(vec![0; 4096])),
        ),
        ("n".to_string(), AttributeValue::N("1234".to_string())),
    ]);
    assert_eq!(item_bytes(&item), 4104);
}
