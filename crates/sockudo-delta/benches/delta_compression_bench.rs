use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main};
use sockudo_core::delta_types::DeltaCompressionConfig;
use sockudo_core::websocket::SocketId;
use sockudo_delta::DeltaCompressionManager;
use std::alloc::{GlobalAlloc, Layout, System};
use std::hint::black_box;
use std::sync::Arc;
use std::sync::Once;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

struct CountingAllocator;

static TRACK_ALLOCATIONS: AtomicBool = AtomicBool::new(false);
static ALLOCATION_COUNT: AtomicU64 = AtomicU64::new(0);
static ALLOCATED_BYTES: AtomicU64 = AtomicU64::new(0);
static DELTA_STATE_ALLOCATION_REPORT: Once = Once::new();

// SAFETY: every allocation operation delegates to the process system allocator.
unsafe impl GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        // SAFETY: the caller provided a valid allocation layout.
        let pointer = unsafe { System.alloc(layout) };
        if !pointer.is_null() && TRACK_ALLOCATIONS.load(Ordering::Relaxed) {
            ALLOCATION_COUNT.fetch_add(1, Ordering::Relaxed);
            ALLOCATED_BYTES.fetch_add(layout.size() as u64, Ordering::Relaxed);
        }
        pointer
    }

    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        // SAFETY: the pointer and layout came from the delegated system allocator.
        unsafe { System.dealloc(pointer, layout) };
    }

    unsafe fn realloc(&self, pointer: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        // SAFETY: the pointer and layout came from the delegated system allocator.
        let pointer = unsafe { System.realloc(pointer, layout, new_size) };
        if !pointer.is_null() && TRACK_ALLOCATIONS.load(Ordering::Relaxed) {
            ALLOCATION_COUNT.fetch_add(1, Ordering::Relaxed);
            ALLOCATED_BYTES.fetch_add(new_size as u64, Ordering::Relaxed);
        }
        pointer
    }
}

#[global_allocator]
static GLOBAL_ALLOCATOR: CountingAllocator = CountingAllocator;

fn measure_allocations<T>(operation: impl FnOnce() -> T) -> (T, u64, u64) {
    TRACK_ALLOCATIONS.store(false, Ordering::SeqCst);
    ALLOCATION_COUNT.store(0, Ordering::SeqCst);
    ALLOCATED_BYTES.store(0, Ordering::SeqCst);
    TRACK_ALLOCATIONS.store(true, Ordering::SeqCst);
    let result = operation();
    TRACK_ALLOCATIONS.store(false, Ordering::SeqCst);
    (
        result,
        ALLOCATION_COUNT.load(Ordering::SeqCst),
        ALLOCATED_BYTES.load(Ordering::SeqCst),
    )
}

fn delta_state_fixture(socket_count: usize) -> (DeltaCompressionManager, Vec<SocketId>) {
    let manager = DeltaCompressionManager::new(DeltaCompressionConfig::default());
    let sockets = (0..socket_count)
        .map(|_| {
            let socket_id = SocketId::new();
            manager.enable_for_socket(&socket_id);
            socket_id
        })
        .collect();
    (manager, sockets)
}

fn store_legacy_copies(
    runtime: &tokio::runtime::Runtime,
    manager: &DeltaCompressionManager,
    sockets: &[SocketId],
    payload: &Arc<Vec<u8>>,
) {
    runtime.block_on(async {
        for socket_id in sockets {
            manager
                .store_sent_message(
                    socket_id,
                    "bench-channel",
                    "bench-event",
                    payload.as_ref().clone(),
                    true,
                    None,
                )
                .await
                .unwrap();
        }
    });
}

fn store_shared_bytes(
    runtime: &tokio::runtime::Runtime,
    manager: &DeltaCompressionManager,
    sockets: &[SocketId],
    payload: &Arc<Vec<u8>>,
) {
    runtime.block_on(async {
        for socket_id in sockets {
            manager
                .store_shared_sent_message(
                    socket_id,
                    "bench-channel",
                    "bench-event",
                    Arc::clone(payload),
                    true,
                    None,
                )
                .await
                .unwrap();
        }
    });
}

fn make_payload(symbol: &str, seq: u32, price: f64, volume: f64) -> String {
    format!(
        "{{\"symbol\":\"{}\",\"seq\":{},\"price\":{:.6},\"volume\":{:.4},\"bid\":{:.6},\"ask\":{:.6}}}",
        symbol,
        seq,
        price,
        volume,
        price - 0.01,
        price + 0.01
    )
}

fn bench_fossil_delta(c: &mut Criterion) {
    let mut group = c.benchmark_group("delta_compression");

    for size in ["small", "medium", "large"] {
        let (base, next) = match size {
            "small" => (
                make_payload("BTC", 1000, 50000.1234, 10.0),
                make_payload("BTC", 1001, 50000.2234, 10.2),
            ),
            "medium" => {
                let mut base = String::new();
                let mut next = String::new();
                for i in 0..40 {
                    base.push_str(&make_payload("ETH", 2000 + i, 3000.0 + i as f64, 8.0));
                    next.push_str(&make_payload("ETH", 2001 + i, 3000.1 + i as f64, 8.1));
                }
                (base, next)
            }
            _ => {
                let mut base = String::new();
                let mut next = String::new();
                for i in 0..200 {
                    base.push_str(&make_payload("SOL", 5000 + i, 120.0 + i as f64, 4.0));
                    next.push_str(&make_payload("SOL", 5001 + i, 120.05 + i as f64, 4.1));
                }
                (base, next)
            }
        };

        group.bench_with_input(
            BenchmarkId::new("fossil", size),
            &(&base, &next),
            |b, (old_msg, new_msg)| {
                b.iter(|| {
                    // Keep argument order aligned with server implementation: delta(new, old)
                    let d = fossil_delta::delta(black_box(new_msg), black_box(old_msg));
                    black_box(d.len());
                });
            },
        );
    }

    group.finish();
}

fn bench_shared_delta_state(c: &mut Criterion) {
    const SOCKETS: usize = 256;
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let payload = Arc::new(vec![42_u8; 64 * 1024]);

    DELTA_STATE_ALLOCATION_REPORT.call_once(|| {
        let (legacy_manager, legacy_sockets) = delta_state_fixture(SOCKETS);
        let (_, legacy_allocations, legacy_bytes) = measure_allocations(|| {
            store_legacy_copies(&runtime, &legacy_manager, &legacy_sockets, &payload)
        });
        let (shared_manager, shared_sockets) = delta_state_fixture(SOCKETS);
        let (_, shared_allocations, shared_bytes) = measure_allocations(|| {
            store_shared_bytes(&runtime, &shared_manager, &shared_sockets, &payload)
        });
        eprintln!(
            "allocation_profile name=delta_state_256x64k variant=legacy allocations={legacy_allocations} allocated_bytes={legacy_bytes}"
        );
        eprintln!(
            "allocation_profile name=delta_state_256x64k variant=shared allocations={shared_allocations} allocated_bytes={shared_bytes}"
        );
    });

    let (legacy_manager, legacy_sockets) = delta_state_fixture(SOCKETS);
    let (shared_manager, shared_sockets) = delta_state_fixture(SOCKETS);
    let mut group = c.benchmark_group("delta_state_fanout");
    group.bench_function("legacy_vec_copy_256x64k", |b| {
        b.iter(|| {
            store_legacy_copies(
                &runtime,
                black_box(&legacy_manager),
                black_box(&legacy_sockets),
                black_box(&payload),
            )
        })
    });
    group.bench_function("shared_arc_256x64k", |b| {
        b.iter(|| {
            store_shared_bytes(
                &runtime,
                black_box(&shared_manager),
                black_box(&shared_sockets),
                black_box(&payload),
            )
        })
    });
    group.finish();
}

criterion_group!(benches, bench_fossil_delta, bench_shared_delta_state);
criterion_main!(benches);
