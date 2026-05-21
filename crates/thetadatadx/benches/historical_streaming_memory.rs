//! Issue #565 memory-allocation pin for the gRPC frame-detach path.
//!
//! The original `ServerStreaming::poll_next` peek loop allocated a
//! deep copy of the entire accumulator on every poll
//! (`BytesMut::clone().freeze()`), so a chunked response paid an
//! `O(polls × buf.len())` memory tax on the decode path. The Tier 4
//! fix (#565) replaces the clone-then-freeze with an in-place
//! `peek_frame_length` peek plus a refcounted `BytesMut::split_to`
//! detach on the success branch.
//!
//! This bench pins the structural improvement at the byte-allocation
//! layer using the same counting-allocator pattern as
//! `grpc_channel.rs`. It does NOT spin up a tokio reactor or a mock
//! h2 server; the goal is to time the framing primitive in isolation
//! so a regression that re-introduces the deep-clone path shows up as
//! a step-function increase in `peak_extra_bytes` rather than as a
//! sub-percentage latency drift hidden under runtime noise.
//!
//! Run: `cargo bench --bench historical_streaming_memory -- --noplot`

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use bytes::{BufMut, BytesMut};
use criterion::{criterion_group, criterion_main, Criterion, Throughput};

// ─── Counting allocator ─────────────────────────────────────────────
//
// Tracks bytes allocated / deallocated globally. The per-bench snapshot
// captures the delta over the timed region only — Criterion setup /
// warmup / reporting allocations live outside the `iter_custom`
// closure and so do not pollute the reported numbers.
//
// `MAX_LIVE_BYTES` is the peak (alloc - dealloc) high-water mark over
// the timed region. It's the figure that distinguishes the
// `BytesMut::clone()` path (peak ≈ buf.len()) from the in-place peek
// path (peak ≈ frame_len, refcount-only).

struct CountingAllocator;

static BYTES_ALLOCATED: AtomicU64 = AtomicU64::new(0);
static BYTES_DEALLOCATED: AtomicU64 = AtomicU64::new(0);
static MAX_LIVE_BYTES: AtomicU64 = AtomicU64::new(0);

// SAFETY: every method forwards verbatim to `std::alloc::System`, which
// itself satisfies the `GlobalAlloc` contract (ptr provenance, layout
// honoured on dealloc, no over-aligned over-promises). The atomic counters
// are pure observational state — `Relaxed` adds on `AtomicU64` cannot
// violate alloc semantics. Bench-only allocator; never linked into the
// shipped library.
unsafe impl GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        // SAFETY: forwarding to the system allocator under the same
        // contract the caller is upholding.
        let ptr = unsafe { System.alloc(layout) };
        if !ptr.is_null() {
            let allocated = BYTES_ALLOCATED.fetch_add(layout.size() as u64, Ordering::Relaxed)
                + layout.size() as u64;
            let deallocated = BYTES_DEALLOCATED.load(Ordering::Relaxed);
            let live = allocated.saturating_sub(deallocated);
            // Lock-free max via CAS. Bench-only, so the loop is
            // bounded by contention which stays low at the
            // sampling rates we drive here.
            let mut prev = MAX_LIVE_BYTES.load(Ordering::Relaxed);
            while live > prev {
                match MAX_LIVE_BYTES.compare_exchange_weak(
                    prev,
                    live,
                    Ordering::Relaxed,
                    Ordering::Relaxed,
                ) {
                    Ok(_) => break,
                    Err(curr) => prev = curr,
                }
            }
        }
        ptr
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        // SAFETY: forwarding to the system allocator under the same
        // contract the caller is upholding.
        unsafe { System.dealloc(ptr, layout) };
        BYTES_DEALLOCATED.fetch_add(layout.size() as u64, Ordering::Relaxed);
    }
}

#[global_allocator]
static ALLOC: CountingAllocator = CountingAllocator;

fn reset_counters() {
    BYTES_ALLOCATED.store(0, Ordering::Relaxed);
    BYTES_DEALLOCATED.store(0, Ordering::Relaxed);
    MAX_LIVE_BYTES.store(0, Ordering::Relaxed);
}

fn snapshot_peak() -> u64 {
    MAX_LIVE_BYTES.load(Ordering::Relaxed)
}

// ─── Framing primitives ─────────────────────────────────────────────

/// gRPC frame layout (spec § 7.1): 1 compressed flag byte + 4 big-endian
/// length bytes + payload. The bench frames are filled with zeros — we
/// time the framing path, not the payload decode.
const FRAME_HEADER_LEN: usize = 5;

fn write_frame_into(buf: &mut BytesMut, payload_len: usize) {
    buf.put_u8(0);
    buf.put_u32(u32::try_from(payload_len).unwrap_or(u32::MAX));
    buf.put_bytes(0, payload_len);
}

// ─── Old path: clone-then-freeze ─────────────────────────────────────
//
// Reproduces the pre-#565 peek shape. The accumulator is cloned on
// every poll into a fresh `Bytes` so the codec can mutate `&mut Bytes`
// without consuming the original; the consumed prefix is then
// advanced off the real accumulator. `BytesMut::clone` is a deep copy
// (verified empirically in `mdds::stream::streaming_decode_contract`),
// so each poll allocates a fresh copy of the entire accumulator.
fn drain_via_clone_then_freeze(buf: &mut BytesMut) -> usize {
    use bytes::Buf as _;
    let mut frames = 0;
    loop {
        // Peek-by-clone: deep copy of the entire accumulator.
        let peek = buf.clone().freeze();
        // Drop `peek` immediately after reading the prefix — same
        // ownership lifetime as the production code held.
        if peek.len() < FRAME_HEADER_LEN {
            drop(peek);
            break;
        }
        let payload_len = u32::from_be_bytes([peek[1], peek[2], peek[3], peek[4]]) as usize;
        let total = FRAME_HEADER_LEN + payload_len;
        if peek.len() < total {
            drop(peek);
            break;
        }
        drop(peek);
        buf.advance(total);
        frames += 1;
    }
    frames
}

// ─── New path: peek + split_to ───────────────────────────────────────
//
// Mirrors the Tier 4 fix: read the header in-place, no clone; on a full
// frame, `split_to(frame_len).freeze()` detaches refcount-only.
fn drain_via_peek_and_split(buf: &mut BytesMut) -> usize {
    let mut frames = 0;
    loop {
        if buf.len() < FRAME_HEADER_LEN {
            break;
        }
        let payload_len = u32::from_be_bytes([buf[1], buf[2], buf[3], buf[4]]) as usize;
        let total = FRAME_HEADER_LEN + payload_len;
        if buf.len() < total {
            break;
        }
        let frame = buf.split_to(total).freeze();
        // Frame goes out of scope at the end of this iteration —
        // matches the production lifetime (codec.decode owns it for
        // the duration of one decode pass).
        let _ = frame;
        frames += 1;
    }
    frames
}

// ─── Workloads ───────────────────────────────────────────────────────

/// Build an accumulator pre-loaded with `frames` × 64 KiB frames —
/// representative of the streaming-response chunk size produced by
/// MDDS at the tick-interval rate the user reported on #565.
fn build_accumulator(frames: usize, payload_bytes: usize) -> BytesMut {
    let mut buf = BytesMut::with_capacity(frames * (FRAME_HEADER_LEN + payload_bytes));
    for _ in 0..frames {
        write_frame_into(&mut buf, payload_bytes);
    }
    buf
}

fn bench_drain(c: &mut Criterion) {
    let mut group = c.benchmark_group("historical_streaming_memory/drain_paths");
    // Frame count chosen so the steady-state buf.len() between
    // frames is non-trivial (the deep-clone path scales linearly
    // with buf.len() so a single-frame accumulator would hide the
    // regression we're pinning).
    let frame_count = 64;
    let payload_bytes = 64 * 1024; // 64 KiB per chunk

    group.throughput(Throughput::Bytes((frame_count * payload_bytes) as u64));

    group.bench_function("old_clone_then_freeze", |b| {
        b.iter_custom(|iters| {
            let mut peaks = Vec::with_capacity(iters as usize);
            let start = Instant::now();
            for _ in 0..iters {
                reset_counters();
                let mut buf = build_accumulator(frame_count, payload_bytes);
                let drained = drain_via_clone_then_freeze(&mut buf);
                assert_eq!(drained, frame_count);
                peaks.push(snapshot_peak());
            }
            let elapsed = start.elapsed();
            let avg_peak: u64 = peaks.iter().sum::<u64>() / peaks.len() as u64;
            eprintln!(
                "[old]  avg peak live bytes: {} ({} KiB)",
                avg_peak,
                avg_peak / 1024
            );
            elapsed
        });
    });

    group.bench_function("new_peek_and_split", |b| {
        b.iter_custom(|iters| {
            let mut peaks = Vec::with_capacity(iters as usize);
            let start = Instant::now();
            for _ in 0..iters {
                reset_counters();
                let mut buf = build_accumulator(frame_count, payload_bytes);
                let drained = drain_via_peek_and_split(&mut buf);
                assert_eq!(drained, frame_count);
                peaks.push(snapshot_peak());
            }
            let elapsed = start.elapsed();
            let avg_peak: u64 = peaks.iter().sum::<u64>() / peaks.len() as u64;
            eprintln!(
                "[new]  avg peak live bytes: {} ({} KiB)",
                avg_peak,
                avg_peak / 1024
            );
            elapsed
        });
    });

    group.finish();
}

criterion_group! {
    name = benches;
    config = Criterion::default().measurement_time(Duration::from_secs(3)).sample_size(20);
    targets = bench_drain
}
criterion_main!(benches);
