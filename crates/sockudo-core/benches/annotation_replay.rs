use criterion::{Criterion, criterion_group, criterion_main};
use sockudo_core::annotations::{
    Annotation, AnnotationAction, AnnotationId, AnnotationSerial, AnnotationStore, AnnotationType,
    MemoryAnnotationStore, RawAnnotationReplayRequest, StoredAnnotationEvent,
};
use sockudo_core::versioned_messages::MessageSerial;
use std::alloc::{GlobalAlloc, Layout, System};
use std::hint::black_box;
use std::sync::Once;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

const APP_ID: &str = "bench-app";
const CHANNEL: &str = "bench-channel";
const NOISY_EVENTS: usize = 2_000;
const TARGET_EVENTS: usize = 100;
const PAGE_SIZE: usize = 50;

struct CountingAllocator;

static TRACK_ALLOCATIONS: AtomicBool = AtomicBool::new(false);
static ALLOCATION_COUNT: AtomicU64 = AtomicU64::new(0);
static ALLOCATED_BYTES: AtomicU64 = AtomicU64::new(0);
static ANNOTATION_ALLOCATION_REPORT: Once = Once::new();

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

fn stored_event(index: usize, message_serial: MessageSerial) -> StoredAnnotationEvent {
    StoredAnnotationEvent {
        app_id: APP_ID.to_string(),
        channel_id: CHANNEL.to_string(),
        stored_at_ms: index as i64,
        annotation: Annotation {
            id: AnnotationId::new(format!("id:{index:020}")).unwrap(),
            action: AnnotationAction::Create,
            serial: AnnotationSerial::new(format!("ann:{index:020}")).unwrap(),
            message_serial,
            annotation_type: AnnotationType::new("reaction:total.v1").unwrap(),
            name: None,
            client_id: None,
            count: None,
            data: None,
            encoding: None,
            timestamp: index as i64,
        },
    }
}

fn annotation_replay(c: &mut Criterion) {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let store = MemoryAnnotationStore::new();
    let target = MessageSerial::new("msg:target").unwrap();
    runtime.block_on(async {
        for index in 0..NOISY_EVENTS {
            store
                .append_event(stored_event(
                    index,
                    MessageSerial::new(format!("msg:noise-{index:020}")).unwrap(),
                ))
                .await
                .unwrap();
        }
        for offset in 0..TARGET_EVENTS {
            store
                .append_event(stored_event(NOISY_EVENTS + offset, target.clone()))
                .await
                .unwrap();
        }
    });

    ANNOTATION_ALLOCATION_REPORT.call_once(|| {
        let allocation_store = MemoryAnnotationStore::new();
        let allocation_target = MessageSerial::new("msg:allocation").unwrap();
        runtime.block_on(async {
            for index in 0..100 {
                allocation_store
                    .append_event(stored_event(index, allocation_target.clone()))
                    .await
                    .unwrap();
            }
        });
        let (projection, allocations, allocated_bytes) = measure_allocations(|| {
            runtime.block_on(
                allocation_store.append_event(stored_event(100, allocation_target.clone())),
            )
        });
        black_box(projection.unwrap());
        eprintln!(
            "allocation_profile name=memory_annotation_append_projection100 allocations={allocations} allocated_bytes={allocated_bytes}"
        );
    });

    let mut group = c.benchmark_group("annotation_rest_page");
    group.bench_function("channel_replay_then_filter", |b| {
        b.iter(|| {
            runtime.block_on(async {
                let page = store
                    .replay_raw(RawAnnotationReplayRequest {
                        app_id: APP_ID.to_string(),
                        channel_id: CHANNEL.to_string(),
                        message_serial: None,
                        after_annotation_serial: None,
                        limit: usize::MAX,
                    })
                    .await
                    .unwrap()
                    .into_iter()
                    .filter(|event| event.message_serial() == &target)
                    .take(PAGE_SIZE)
                    .collect::<Vec<_>>();
                black_box(page)
            })
        })
    });
    group.bench_function("message_scoped_bounded_replay", |b| {
        b.iter(|| {
            runtime.block_on(async {
                let page = store
                    .replay_raw(RawAnnotationReplayRequest {
                        app_id: APP_ID.to_string(),
                        channel_id: CHANNEL.to_string(),
                        message_serial: Some(target.clone()),
                        after_annotation_serial: None,
                        limit: PAGE_SIZE,
                    })
                    .await
                    .unwrap();
                black_box(page)
            })
        })
    });
    group.finish();
}

criterion_group!(benches, annotation_replay);
criterion_main!(benches);
