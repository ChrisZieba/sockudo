//! C2 diagnostic: retained bytes, write/read CPU and reconstruction latency for
//! one long append stream in the memory version store.
//!
//! Uses only the public `VersionStore` API so the same file runs unchanged
//! against the pre-change baseline and the candidate. One case per process
//! keeps peak RSS attributable:
//!
//! ```text
//! cargo run -p sockudo-core --example c2_append_storage --release -- <appends> <fragment_bytes>
//! ```
use sockudo_core::message_envelope::{MessageContent, MessageEnvelope};
use sockudo_core::version_store::*;
use sockudo_core::versioned_messages::*;
use sockudo_protocol::messages::MessageData;
use std::alloc::{GlobalAlloc, Layout, System};
use std::hint::black_box;
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU64, Ordering};
use std::time::Instant;

struct CountAlloc;
static TRACK: AtomicBool = AtomicBool::new(false);
static CALLS: AtomicU64 = AtomicU64::new(0);
static BYTES: AtomicU64 = AtomicU64::new(0);
static LIVE: AtomicI64 = AtomicI64::new(0);
// SAFETY: every operation forwards its original valid arguments to System.
unsafe impl GlobalAlloc for CountAlloc {
    unsafe fn alloc(&self, l: Layout) -> *mut u8 {
        let p = unsafe { System.alloc(l) };
        if !p.is_null() {
            LIVE.fetch_add(l.size() as i64, Ordering::Relaxed);
            if TRACK.load(Ordering::Relaxed) {
                CALLS.fetch_add(1, Ordering::Relaxed);
                BYTES.fetch_add(l.size() as u64, Ordering::Relaxed);
            }
        }
        p
    }
    unsafe fn dealloc(&self, p: *mut u8, l: Layout) {
        unsafe { System.dealloc(p, l) };
        LIVE.fetch_sub(l.size() as i64, Ordering::Relaxed);
    }
    unsafe fn realloc(&self, p: *mut u8, l: Layout, n: usize) -> *mut u8 {
        let q = unsafe { System.realloc(p, l, n) };
        if !q.is_null() {
            LIVE.fetch_add(n as i64 - l.size() as i64, Ordering::Relaxed);
            if TRACK.load(Ordering::Relaxed) {
                CALLS.fetch_add(1, Ordering::Relaxed);
                BYTES.fetch_add(n as u64, Ordering::Relaxed);
            }
        }
        q
    }
}
#[global_allocator]
static ALLOC: CountAlloc = CountAlloc;

const APP: &str = "c2";
const CHANNEL: &str = "ai:room";

struct Lcg(u64);
impl Lcg {
    fn next(&mut self) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        self.0 >> 33
    }
}

/// Exactly `bytes` of UTF-8 mixing ASCII with 2- and 4-byte characters.
fn fragment(rng: &mut Lcg, bytes: usize) -> String {
    let mut out = String::with_capacity(bytes);
    while out.len() < bytes {
        let left = bytes - out.len();
        match rng.next() % 16 {
            0 if left >= 4 => out.push('\u{1F642}'),
            1 | 2 if left >= 2 => out.push('\u{e9}'),
            n => out.push((b'a' + (n as u8 % 26)) as char),
        }
    }
    out
}

fn version(n: u64) -> VersionMetadata {
    VersionMetadata {
        serial: VersionSerial::new(format!("ver:{n:020}")).unwrap(),
        client_id: Some("agent".into()),
        timestamp_ms: 1_700_000_000_000 + n as i64,
        description: None,
        metadata: None,
    }
}

fn create_record() -> StoredVersionRecord {
    let envelope = MessageEnvelope {
        message_id: Some("client-message-1".into()),
        name: Some("ai.response".into()),
        data: Some(MessageContent::Text(String::new())),
        publisher_client_id: Some("agent".into()),
        published_at_ms: Some(1_700_000_000_000),
        ..MessageEnvelope::default()
    };
    StoredVersionRecord {
        app_id: APP.into(),
        channel: CHANNEL.into(),
        original_client_id: Some("agent".into()),
        envelope: Some(envelope),
        message: VersionedMessage::new_create(
            MessageSerial::new("msg:1").unwrap(),
            version(0),
            1,
            0,
            Some("ai.response".into()),
            Some(MessageData::String(String::new())),
            None,
        ),
    }
}

fn pct(sorted: &[u64], p: usize) -> u64 {
    sorted[(sorted.len() - 1) * p / 100]
}

fn report(name: &str, mut samples: Vec<u64>) {
    samples.sort_unstable();
    println!(
        "latency,{name},{},{},{},{},{}",
        samples.len(),
        pct(&samples, 50),
        pct(&samples, 95),
        pct(&samples, 99),
        samples[samples.len() - 1]
    );
}

/// FNV-1a over the canonical public representation (random stream IDs removed).
fn digest(state: &mut u64, record: &StoredVersionRecord) {
    let mut record = record.clone();
    if let Some(envelope) = record.envelope.as_mut() {
        envelope.stream_id = None;
    }
    for byte in sonic_rs::to_vec(&record).unwrap() {
        *state ^= u64::from(byte);
        *state = state.wrapping_mul(0x100000001b3);
    }
}

fn rusage() -> (f64, f64, i64) {
    // SAFETY: getrusage writes a fully initialized struct for RUSAGE_SELF.
    let mut usage = unsafe { std::mem::zeroed::<libc_rusage::Rusage>() };
    unsafe { libc_rusage::getrusage(0, &mut usage) };
    let secs = |t: libc_rusage::Timeval| t.tv_sec as f64 + f64::from(t.tv_usec) / 1e6;
    (secs(usage.ru_utime), secs(usage.ru_stime), usage.ru_maxrss)
}

/// Minimal 64-bit `getrusage` binding (the crate has no libc dependency).
/// `ru_maxrss` is bytes on macOS and KiB on Linux.
mod libc_rusage {
    #[repr(C)]
    #[derive(Clone, Copy)]
    pub struct Timeval {
        pub tv_sec: i64,
        #[cfg(target_os = "macos")]
        pub tv_usec: i32,
        #[cfg(target_os = "macos")]
        pub _pad: i32,
        #[cfg(not(target_os = "macos"))]
        pub tv_usec: i32,
        #[cfg(not(target_os = "macos"))]
        pub _high: i32,
    }
    #[repr(C)]
    pub struct Rusage {
        pub ru_utime: Timeval,
        pub ru_stime: Timeval,
        pub ru_maxrss: i64,
        pub rest: [i64; 13],
    }
    unsafe extern "C" {
        pub fn getrusage(who: i32, usage: *mut Rusage) -> i32;
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let appends: u64 = args.get(1).map_or(2000, |v| v.parse().unwrap());
    let fragment_bytes: usize = args.get(2).map_or(64, |v| v.parse().unwrap());
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let mut rng = Lcg(0xC2 ^ appends ^ ((fragment_bytes as u64) << 20));
    let fragments: Vec<String> = (0..appends)
        .map(|_| fragment(&mut rng, fragment_bytes))
        .collect();
    let serial = MessageSerial::new("msg:1").unwrap();

    let live_before = LIVE.load(Ordering::SeqCst);
    let store = MemoryVersionStore::new();
    let (cpu_user_0, cpu_sys_0, _) = rusage();
    let mut write_ns = Vec::with_capacity(appends as usize);
    let mut writes_alloc_calls = 0;
    let mut writes_alloc_bytes = 0;
    rt.block_on(async {
        let VersionCreateResult::Applied { record, .. } = store
            .commit_create(VersionCreateRequest {
                record: create_record(),
                limits: VersionCreateLimits::default(),
            })
            .await
            .unwrap()
        else {
            panic!("create was not applied");
        };
        let mut expected = VersionPrecondition::from_record(&record);
        drop(record);
        for (index, data_fragment) in fragments.iter().enumerate() {
            let request = VersionMutationRequest {
                app_id: APP.into(),
                channel: CHANNEL.into(),
                message_serial: serial.clone(),
                expected: expected.clone(),
                version: version(index as u64 + 1),
                mutation: VersionMutation::Append(MessageAppend {
                    data_fragment: data_fragment.clone(),
                    extras: None,
                }),
                idempotency: None,
                limits: VersionMutationLimits::default(),
            };
            CALLS.store(0, Ordering::SeqCst);
            BYTES.store(0, Ordering::SeqCst);
            TRACK.store(true, Ordering::SeqCst);
            let started = Instant::now();
            let outcome = store.compare_and_apply(request).await.unwrap();
            write_ns.push(started.elapsed().as_nanos() as u64);
            TRACK.store(false, Ordering::SeqCst);
            writes_alloc_calls += CALLS.load(Ordering::SeqCst);
            writes_alloc_bytes += BYTES.load(Ordering::SeqCst);
            let VersionMutationResult::Applied { record, .. } = outcome else {
                panic!("append was not applied");
            };
            expected = VersionPrecondition::from_record(&record);
        }
    });
    let (cpu_user_1, cpu_sys_1, _) = rusage();
    let live_retained = LIVE.load(Ordering::SeqCst) - live_before;

    // Public-representation equivalence: every version, oldest first.
    let (versions, data_bytes, versions_digest) = rt.block_on(async {
        let mut cursor = None;
        let mut count = 0u64;
        let mut bytes = 0usize;
        let mut state = 0xcbf29ce484222325u64;
        loop {
            let page = store
                .get_versions(VersionStoreReadRequest {
                    app_id: APP.into(),
                    channel: CHANNEL.into(),
                    message_serial: serial.clone(),
                    direction: VersionStoreDirection::OldestFirst,
                    limit: 100,
                    cursor,
                })
                .await
                .unwrap();
            for record in &page.items {
                count += 1;
                bytes += record.data_bytes().unwrap();
                digest(&mut state, record);
            }
            cursor = page.next_cursor;
            if cursor.is_none() {
                break (count, bytes, state);
            }
        }
    });
    let replay_digest = rt.block_on(async {
        let mut after = 0;
        let mut state = 0xcbf29ce484222325u64;
        loop {
            let items = store
                .replay_after(VersionReplayRequest {
                    app_id: APP.into(),
                    channel: CHANNEL.into(),
                    after_delivery_serial: after,
                    limit: 100,
                })
                .await
                .unwrap();
            let Some(last) = items.last() else {
                break state;
            };
            after = last.delivery_serial();
            for record in &items {
                digest(&mut state, record);
            }
        }
    });

    let (cpu_user_2, cpu_sys_2, _) = rusage();
    let mut rng = Lcg(0x5eed);
    let mut latest_ns = Vec::new();
    let mut random_ns = Vec::new();
    let mut random_alloc_bytes = 0u64;
    let mut page_ns = Vec::new();
    let mut replay_ns = Vec::new();
    rt.block_on(async {
        for _ in 0..5 {
            black_box(store.get_latest(APP, CHANNEL, &serial).await.unwrap());
        }
        for _ in 0..201 {
            let started = Instant::now();
            black_box(store.get_latest(APP, CHANNEL, &serial).await.unwrap());
            latest_ns.push(started.elapsed().as_nanos() as u64);
        }
        for _ in 0..201 {
            // Exactly one historical version: the one after a random cursor.
            let target = 1 + rng.next() % appends;
            let request = VersionStoreReadRequest {
                app_id: APP.into(),
                channel: CHANNEL.into(),
                message_serial: serial.clone(),
                direction: VersionStoreDirection::OldestFirst,
                limit: 1,
                cursor: Some(VersionStoreCursor {
                    version: 1,
                    version_serial: version(target - 1).serial,
                    direction: VersionStoreDirection::OldestFirst,
                }),
            };
            BYTES.store(0, Ordering::SeqCst);
            TRACK.store(true, Ordering::SeqCst);
            let started = Instant::now();
            let page = store.get_versions(request).await.unwrap();
            random_ns.push(started.elapsed().as_nanos() as u64);
            TRACK.store(false, Ordering::SeqCst);
            random_alloc_bytes += BYTES.load(Ordering::SeqCst);
            assert_eq!(page.items[0].version_serial(), &version(target).serial);
            black_box(page);
        }
        for _ in 0..101 {
            let start = rng.next() % appends;
            let request = VersionStoreReadRequest {
                app_id: APP.into(),
                channel: CHANNEL.into(),
                message_serial: serial.clone(),
                direction: VersionStoreDirection::NewestFirst,
                limit: 100,
                cursor: Some(VersionStoreCursor {
                    version: 1,
                    version_serial: version(start + 1).serial,
                    direction: VersionStoreDirection::NewestFirst,
                }),
            };
            let started = Instant::now();
            black_box(store.get_versions(request).await.unwrap());
            page_ns.push(started.elapsed().as_nanos() as u64);
        }
        for _ in 0..101 {
            let after = rng.next() % appends;
            let started = Instant::now();
            black_box(
                store
                    .replay_after(VersionReplayRequest {
                        app_id: APP.into(),
                        channel: CHANNEL.into(),
                        after_delivery_serial: after,
                        limit: 100,
                    })
                    .await
                    .unwrap(),
            );
            replay_ns.push(started.elapsed().as_nanos() as u64);
        }
    });
    let (cpu_user_3, cpu_sys_3, max_rss) = rusage();

    println!("case,appends,{appends},fragment_bytes,{fragment_bytes}");
    println!(
        "equivalence,versions,{versions},data_bytes,{data_bytes},versions_digest,{versions_digest:016x},replay_digest,{replay_digest:016x}"
    );
    println!("retained,live_heap_bytes,{live_retained}");
    println!(
        "writes,alloc_calls,{writes_alloc_calls},alloc_bytes,{writes_alloc_bytes},cpu_user_s,{:.6},cpu_sys_s,{:.6}",
        cpu_user_1 - cpu_user_0,
        cpu_sys_1 - cpu_sys_0
    );
    println!(
        "reads,random_alloc_bytes_per_read,{},cpu_user_s,{:.6},cpu_sys_s,{:.6}",
        random_alloc_bytes / random_ns.len() as u64,
        cpu_user_3 - cpu_user_2,
        cpu_sys_3 - cpu_sys_2
    );
    report("append_ns", write_ns);
    report("get_latest_ns", latest_ns);
    report("random_version_read_ns", random_ns);
    report("page100_read_ns", page_ns);
    report("replay100_read_ns", replay_ns);
    println!("process,max_rss_bytes,{max_rss}");
    drop(store);
}
