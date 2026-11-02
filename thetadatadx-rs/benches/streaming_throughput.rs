//! End-to-end streaming throughput across binding shapes.
//!
//! The streaming client runs two threads: a reader that reads and decodes
//! every frame, and a consumer that hands each event to the user callback,
//! joined by the Disruptor ring. The sustained rate without drops is the
//! slower of the two, so this bench times each side on its own:
//!
//! - `decode`: the reader thread's per-frame work over an in-memory capture
//!   of a 10,000-contract option chain (frame read, clock reads, decode).
//! - the dispatch variants: the Disruptor pipeline under the callback shapes
//!   that matter for production integrators: a no-op closure as the floor,
//!   a Rust closure that retains the event, and an FFI-style indirection.
//!   Every variant drives the same pipeline; the only thing that changes is
//!   what the user closure does inside `catch_unwind`.
//!
//! # Methodology
//!
//! - One Disruptor + one consumer thread per Criterion iteration. Build
//!   cost (ring allocation, thread spawn, handler installation) is
//!   excluded via `b.iter_custom`: setup runs outside the timed region
//!   and only the publish loop + producer drop are inside.
//! - The producer retries `try_publish` on `RingBufferFull` so all
//!   `EVENTS_PER_ITER` events are delivered (no silent drops). The
//!   delivered count is asserted via `debug_assert_eq!`; the published
//!   numbers are per-DELIVERED-event by construction.
//! - Each variant emits a realistic `StreamEvent::Data(StreamData::Trade)`
//!   carrying an `Arc<Contract>`. The contract is allocated once and
//!   the publish closure clones the `Arc` (refcount bump, no
//!   allocation) — this matches the live decode path where the
//!   contract is interned in the FPSS contract cache and every event
//!   takes a refcount on the same pointer.
//!
//! Run: `cargo bench --bench streaming_throughput -- --noplot`

use std::ffi::c_void;
use std::hint::black_box;
use std::io::Cursor;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use criterion::{Criterion, Throughput, criterion_group, criterion_main};
use disruptor::{BusySpin, Producer, Sequence, build_single_producer};
use rustc_hash::FxHashMap;
use thetadatadx::fpss::internals::{
    DeltaState, FpssEventInternal, FrameRead, decode_frame, read_frame_into_with_stall_timeout,
};
use thetadatadx::fpss::protocol::Contract;
use thetadatadx::fpss::{StreamData, StreamEvent};
use thetadatadx::{Price, StreamMsgType};

/// Events shipped per Criterion sample. Sized so per-iteration wall
/// clock dwarfs Criterion measurement overhead and so the consumer
/// thread reaches steady state.
const EVENTS_PER_ITER: usize = 100_000;

/// Disruptor ring size. Matches the production default
/// `FpssConfig::ring_size = 131_072` (see `thetadatadx-rs/src/config/fpss.rs`)
/// so these numbers reflect the out-of-the-box live SDK configuration. With
/// `EVENTS_PER_ITER` events fitting inside one ring, the producer publishes
/// the whole batch without busy-spinning on a full ring, so the consumer
/// keeps full CPU — the same steady state a live stream reaches.
const RING_SIZE: usize = 131_072;

#[derive(Default)]
struct RingSlot {
    event: Option<StreamEvent>,
}

// SAFETY: matches `RingEvent` in `thetadatadx-rs/src/fpss/ring.rs`
// — `StreamEvent: Clone + Send`, the Disruptor's sequencing guarantees
// exclusive write / shared read.
unsafe impl Sync for RingSlot {}

/// Mutable user-handler cell, shape-compatible with the live `io_loop`
/// wiring (`Mutex<Box<dyn FnMut>>` — single-locker, no contention).
type BoxedHandler = Mutex<Box<dyn FnMut(&StreamEvent) + Send>>;

// ─── Realistic event factory ──────────────────────────────────────────

/// Build a `Trade` event carrying a shared `Arc<Contract>`. The
/// contract is interned by the caller (allocate once, clone the Arc
/// per event) so the publish closure pays only a refcount bump — same
/// shape as the live FPSS decode path where the contract cache hands
/// out `Arc::clone(&cached)` for every tick.
#[inline]
fn make_event(contract: &Arc<Contract>, idx: u64) -> StreamEvent {
    StreamEvent::Data(StreamData::Trade {
        contract: Arc::clone(contract),
        ms_of_day: (idx % 86_400_000) as i32,
        sequence: idx as i32,
        condition: 0,
        size: 100,
        exchange: 0,
        price: Price::new(15025, 8).to_f64(),
        date: 20240315,
        received_at_ns: idx,
    })
}

/// Build the standard SPY contract used as the event payload across
/// every variant. Allocated once per sample, then shared via
/// `Arc::clone`.
fn make_contract() -> Arc<Contract> {
    Arc::new(Contract::stock("SPY"))
}

// ─── Disruptor harness with a user-supplied consumer body ─────────────

/// Builds the Disruptor + consumer thread, returns the producer plus a
/// shared delivery counter. `body` is invoked on the consumer thread
/// for every delivered event, wrapped in `catch_unwind` to match the
/// live SSOT pipeline.
fn build_pipeline<F>(body: F) -> (impl Producer<RingSlot>, Arc<AtomicU64>)
where
    F: FnMut(&StreamEvent) + Send + 'static,
{
    let delivered = Arc::new(AtomicU64::new(0));
    let delivered_consumer = Arc::clone(&delivered);
    let user_handler: BoxedHandler = Mutex::new(Box::new(body));

    let factory = || RingSlot { event: None };
    let producer = build_single_producer(RING_SIZE, factory, BusySpin)
        .handle_events_with(move |slot: &RingSlot, _seq: Sequence, _eob: bool| {
            if let Some(ref evt) = slot.event {
                delivered_consumer.fetch_add(1, Ordering::Relaxed);
                let mut h = user_handler
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                let _ = catch_unwind(AssertUnwindSafe(|| h(evt)));
            }
        })
        .build();
    (producer, delivered)
}

/// Drive the publish loop + drain. `EVENTS_PER_ITER` deliveries are
/// guaranteed by the retry-on-overflow loop. Returns
/// `(delivered_count, elapsed)` where `elapsed` is the publish loop +
/// producer drop only (setup is excluded by the caller).
fn drive_publish<P: Producer<RingSlot>>(
    mut producer: P,
    delivered: Arc<AtomicU64>,
    contract: Arc<Contract>,
) -> (u64, Duration) {
    let start = Instant::now();
    for i in 0..EVENTS_PER_ITER as u64 {
        loop {
            let evt = make_event(&contract, i);
            if producer
                .try_publish(|slot| {
                    slot.event = Some(evt);
                })
                .is_ok()
            {
                break;
            }
            std::hint::spin_loop();
        }
    }
    drop(producer);
    let elapsed = start.elapsed();
    (delivered.load(Ordering::Relaxed), elapsed)
}

// NOTE on `make_event` placement vs the timed region: building the
// `StreamEvent` (one Arc-clone + struct fill) is part of every realistic
// publish path — the live decode loop materializes the event before
// `try_publish`. We deliberately count it inside the timed region so
// the bench reflects what an integrator actually pays. The variants
// differ only in what the *consumer* closure does; the producer side
// is identical across all of them.

// ─── Variant 1: Rust no-op closure ────────────────────────────────────

fn run_rust_noop(contract: Arc<Contract>) -> (u64, Duration) {
    let (producer, delivered) = build_pipeline(|_evt: &StreamEvent| {
        // No-op: the pipeline's own cost, the floor every other variant
        // is read against.
    });
    drive_publish(producer, delivered, contract)
}

// ─── Variant 2: clone into pre-allocated Vec<StreamEvent> ───────────────

fn run_rust_vec_push(contract: Arc<Contract>) -> (u64, Duration) {
    // Pre-allocate so Vec growth is excluded from the timed cost.
    let buf: Arc<Mutex<Vec<StreamEvent>>> =
        Arc::new(Mutex::new(Vec::with_capacity(EVENTS_PER_ITER + 16)));
    let buf_consumer = Arc::clone(&buf);
    let (producer, delivered) = build_pipeline(move |evt: &StreamEvent| {
        let mut v = buf_consumer
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        v.push(evt.clone());
    });
    let result = drive_publish(producer, delivered, contract);
    // Consume the Vec so the optimiser cannot elide the push.
    let v = buf
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    black_box(v.len());
    result
}

// ─── Variant 4: FFI-style extern "C" callback indirection ─────────────

/// Counter used as the FFI context — pointer-only, mimics what a C or
/// C++ shim would pass as `void* ctx`.
struct FfiCtx {
    counter: AtomicU64,
}

/// `extern "C"` callback shape, matching the FFI surface a Python /
/// Node native binding would dispatch through. Takes a `*const c_void`
/// for the event pointer so the indirection cost (function-pointer
/// call + opaque pointer cast) is observable.
extern "C" fn ffi_trampoline(ctx: *mut c_void, event_ptr: *const c_void) {
    // SAFETY: `ctx` points to an `FfiCtx` that outlives the bench
    // (held inside `run_ffi_simulated` for the duration of the
    // `drive_publish` call). `event_ptr` is a `*const StreamEvent` cast
    // to `*const c_void` by the caller.
    let ctx = unsafe { &*(ctx as *const FfiCtx) };
    // SAFETY: `event_ptr` is a `*const StreamEvent` produced by the bench
    // harness one stack frame above; the referent lives until
    // `drive_publish` returns, which is strictly after this callback
    // ends. No aliasing — the consumer is the sole reader.
    let evt = unsafe { &*(event_ptr as *const StreamEvent) };
    // Touch the event so the read is not elided. Match-arm chosen for
    // its uniqueness so the optimiser cannot collapse the load.
    if let StreamEvent::Data(StreamData::Trade { sequence, .. }) = evt {
        ctx.counter
            .fetch_add((*sequence as u64) & 1, Ordering::Relaxed);
    }
}

fn run_ffi_simulated(contract: Arc<Contract>) -> (u64, Duration) {
    let ctx = Arc::new(FfiCtx {
        counter: AtomicU64::new(0),
    });
    let ctx_consumer = Arc::clone(&ctx);
    // `extern "C" fn` coerces to a function pointer — same indirection
    // an FFI binding pays per event.
    let cb: extern "C" fn(*mut c_void, *const c_void) = ffi_trampoline;
    let (producer, delivered) = build_pipeline(move |evt: &StreamEvent| {
        let ctx_ptr = (Arc::as_ptr(&ctx_consumer)) as *mut c_void;
        let evt_ptr = (evt as *const StreamEvent) as *const c_void;
        cb(ctx_ptr, evt_ptr);
    });
    let result = drive_publish(producer, delivered, contract);
    black_box(ctx.counter.load(Ordering::Relaxed));
    result
}

// ─── Reader thread: frame read + decode ───────────────────────────────

/// Option contracts in the decode capture, about a whole SPXW chain near
/// the money.
const DECODE_CONTRACTS: i32 = 10_000;

/// Delta-quote passes over every contract after its absolute rows.
const DECODE_ROUNDS: i32 = 100;

/// FIT-encode one row: decimal digits as nibbles, `0xB` between fields,
/// `0xE` before a negative value, `0xD` at the end.
fn fit_row(fields: &[i32]) -> Vec<u8> {
    let mut nibbles = Vec::new();
    for (i, value) in fields.iter().enumerate() {
        if i > 0 {
            nibbles.push(0xB);
        }
        if *value < 0 {
            nibbles.push(0xE);
        }
        nibbles.extend(value.unsigned_abs().to_string().bytes().map(|b| b - b'0'));
    }
    nibbles.push(0xD);
    nibbles
        .chunks(2)
        .map(|pair| pair[0] << 4 | pair.get(1).copied().unwrap_or(0))
        .collect()
}

fn push_frame(wire: &mut Vec<u8>, code: StreamMsgType, payload: &[u8]) {
    wire.push(u8::try_from(payload.len()).expect("payload fits one frame"));
    wire.push(code as u8);
    wire.extend_from_slice(payload);
}

/// Frames as the server sends them: each contract's assignment and one
/// absolute quote and trade, then `DECODE_ROUNDS` passes of delta quotes
/// with a delta trade on every 100th contract. Returns the wire bytes, the
/// number of frames and how many of them carry a quote or a trade.
fn decode_capture() -> (Vec<u8>, u64, u64) {
    let mut wire = Vec::new();
    let mut frames = 0;
    for id in 0..DECODE_CONTRACTS {
        let contract =
            Contract::option_raw("SPXW", 20250428, id % 2 == 0, 4_000_000 + id / 2 * 5_000);
        let mut assignment = id.to_be_bytes().to_vec();
        assignment.extend(contract.to_bytes());
        push_frame(&mut wire, StreamMsgType::Contract, &assignment);
        let quote = [
            id, 34_200_000, 10, 5, 12_345, 50, 12, 5, 12_445, 50, 4, 20250428,
        ];
        push_frame(&mut wire, StreamMsgType::Quote, &fit_row(&quote));
        let trade = [id, 34_200_000, 1, 2, 18, 12_400, 5, 4, 20250428];
        push_frame(&mut wire, StreamMsgType::Trade, &fit_row(&trade));
        frames += 3;
    }
    for round in 0..DECODE_ROUNDS {
        for id in 0..DECODE_CONTRACTS {
            let d: i32 = if (round + id) % 2 == 0 { 5 } else { -5 };
            let quote = [id, 37, d.signum(), 0, d, 0, -d.signum(), 0, d];
            push_frame(&mut wire, StreamMsgType::Quote, &fit_row(&quote));
            frames += 1;
            if id % 100 == 0 {
                let trade = [id, 41, 1, d.signum(), 0, d];
                push_frame(&mut wire, StreamMsgType::Trade, &fit_row(&trade));
                frames += 1;
            }
        }
    }
    (
        wire,
        frames,
        frames - u64::from(DECODE_CONTRACTS.unsigned_abs()),
    )
}

/// The reader thread's work per frame, as the live loop does it: read the
/// frame, read the stall clock and the wall clock, decode. Returns the
/// number of data events, so a row the decoder rejects fails the check.
fn decode_all(wire: &[u8]) -> u64 {
    let (authenticated, shutdown) = (AtomicBool::new(true), AtomicBool::new(false));
    let mut contracts = FxHashMap::default();
    let mut delta = DeltaState::new();
    let mut buf = Vec::new();
    let mut reader = Cursor::new(wire);
    let mut events = 0;
    while let FrameRead::Frame(code, len) =
        read_frame_into_with_stall_timeout(&mut reader, &mut buf, Duration::from_secs(1))
            .expect("in-memory read")
    {
        black_box(Instant::now());
        let received_at_ns = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| u64::try_from(d.as_nanos()).unwrap_or(u64::MAX));
        let event = decode_frame(
            code,
            &buf[..len],
            received_at_ns,
            &authenticated,
            &mut contracts,
            &shutdown,
            &mut delta,
        );
        events += u64::from(matches!(black_box(event), Some(FpssEventInternal::Data(_))));
    }
    events
}

fn bench_decode(c: &mut Criterion) {
    let (wire, frames, data_events) = decode_capture();
    let mut group = c.benchmark_group("streaming_throughput/decode");
    group.throughput(Throughput::Elements(frames));
    group.sample_size(10);
    group.bench_function("option_chain", |b| {
        b.iter(|| assert_eq!(decode_all(&wire), data_events));
    });
    group.finish();
}

// ─── Criterion driver ──────────────────────────────────────────────────

/// Run `runner` exactly `iters` times, summing the in-loop wall clock
/// (publish loop + producer drop). Setup (Disruptor build, thread
/// spawn, deque/Queue construction) is excluded.
fn time_iters<R>(iters: u64, mut runner: R) -> Duration
where
    R: FnMut(Arc<Contract>) -> (u64, Duration),
{
    let mut total = Duration::ZERO;
    for _ in 0..iters {
        let contract = make_contract();
        let (delivered, elapsed) = runner(contract);
        debug_assert_eq!(
            delivered, EVENTS_PER_ITER as u64,
            "retry-on-overflow loop must deliver every attempted event \
             (delivered={delivered}, expected={EVENTS_PER_ITER})",
        );
        black_box(delivered);
        total += elapsed;
    }
    total
}

fn bench_variant<R>(c: &mut Criterion, name: &str, runner: R)
where
    R: FnMut(Arc<Contract>) -> (u64, Duration) + Copy,
{
    let mut group = c.benchmark_group(format!("streaming_throughput/{name}"));
    group.throughput(Throughput::Elements(EVENTS_PER_ITER as u64));
    group.sample_size(10);
    group.bench_function("100k_events", |b| {
        b.iter_custom(|iters| time_iters(iters, runner));
    });
    group.finish();
}

fn bench_rust_noop(c: &mut Criterion) {
    bench_variant(c, "rust_noop", run_rust_noop);
}

fn bench_rust_vec_push(c: &mut Criterion) {
    bench_variant(c, "rust_vec_push", run_rust_vec_push);
}

fn bench_ffi_simulated(c: &mut Criterion) {
    bench_variant(c, "ffi_simulated", run_ffi_simulated);
}

criterion_group!(
    streaming_throughput,
    bench_decode,
    bench_rust_noop,
    bench_rust_vec_push,
    bench_ffi_simulated,
);
criterion_main!(streaming_throughput);
