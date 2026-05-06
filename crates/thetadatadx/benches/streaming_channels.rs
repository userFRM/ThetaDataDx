//! Streaming hot-path channel benchmarks (issue #482).
//!
//! Measures the per-event cost of the producer/consumer hand-off the FFI
//! streaming surface pays for every FPSS event delivered to a buffered
//! consumer. The producer thread models the FPSS sync read loop pushing
//! one event per iteration; the consumer thread models the FFI side
//! draining via `recv` / `recv_timeout` exactly like
//! `ffi/src/streaming.rs::tdx_unified_next_event` and
//! `tdx_fpss_next_event` do today.
//!
//! Five variants are timed end-to-end (1 producer + 1 consumer thread,
//! 100k events each, payload sized like the real `FfiBufferedEvent` —
//! ~488 bytes including the tagged union and two heap-owned tails):
//!
//! 1. `std_mpsc_unbounded` — `std::sync::mpsc::channel()` with
//!    `recv_timeout(100ms)`, the live shape.
//! 2. `crossbeam_bounded_256` — `crossbeam_channel::bounded(256)`.
//! 3. `crossbeam_bounded_1024` — `crossbeam_channel::bounded(1024)`.
//! 4. `crossbeam_bounded_8192` — `crossbeam_channel::bounded(8192)`.
//! 5. `direct_callback` — no channel; producer invokes a
//!    `extern "C" fn(*const Event, *mut c_void)` directly through a
//!    `Box<dyn Fn>` adapter, modelling the C/C++ tier-1 path proposed
//!    in issue #482.
//!
//! The buffered event mirrors the field layout of
//! `ffi::streaming::FfiBufferedEvent`: a `#[repr(C)]` tagged event
//! (Quote/Trade/OHLCVC/OpenInterest/Control/RawData) with a `TdxContract`
//! embedded in every data variant, plus two heap-owned tails (`CString`
//! detail + `Vec<u8>` raw payload) that hold the backing memory for
//! pointer fields. The mirror is local to this bench file so the runtime
//! dep graph is untouched; sizes and field shapes match the generated
//! `fpss_event_structs.rs` byte-for-byte on x86_64.
//!
//! Run: `cargo bench --bench streaming_channels`

use std::ffi::{c_void, CString};
use std::hint::black_box;
use std::os::raw::c_char;
use std::ptr;
use std::sync::mpsc as std_mpsc;
use std::thread;
use std::time::Duration;

use criterion::{criterion_group, criterion_main, Criterion, Throughput};

// ─── Event payload mirror ──────────────────────────────────────────────
//
// Field-for-field copy of the relevant prefix of
// `ffi/src/streaming.rs::FfiBufferedEvent` and the generated
// `TdxFpssEvent` tagged union. Kept in this bench file (not pulled from
// the ffi crate) so the bench is self-contained and the runtime dep
// graph stays clean. Sizes match: `Event` = 448 B, `BufferedEvent` = 488 B
// on x86_64 Linux.

#[repr(C)]
struct Contract {
    root: *const c_char,
    sec_type: i32,
    has_exp_date: bool,
    exp_date: i32,
    has_is_call: bool,
    is_call: bool,
    has_strike: bool,
    strike: i32,
}

#[repr(C)]
struct Ohlcvc {
    contract_id: i32,
    contract: Contract,
    ms_of_day: i32,
    open: f64,
    high: f64,
    low: f64,
    close: f64,
    volume: i64,
    count: i64,
    date: i32,
    received_at_ns: u64,
}

#[repr(C)]
struct OpenInterest {
    contract_id: i32,
    contract: Contract,
    ms_of_day: i32,
    open_interest: i32,
    date: i32,
    received_at_ns: u64,
}

#[repr(C)]
struct Quote {
    contract_id: i32,
    contract: Contract,
    ms_of_day: i32,
    bid_size: i32,
    bid_exchange: i32,
    bid: f64,
    bid_condition: i32,
    ask_size: i32,
    ask_exchange: i32,
    ask: f64,
    ask_condition: i32,
    date: i32,
    received_at_ns: u64,
}

#[repr(C)]
struct Trade {
    contract_id: i32,
    contract: Contract,
    ms_of_day: i32,
    sequence: i32,
    ext_condition1: i32,
    ext_condition2: i32,
    ext_condition3: i32,
    ext_condition4: i32,
    condition: i32,
    size: i32,
    exchange: i32,
    price: f64,
    condition_flags: i32,
    price_flags: i32,
    volume_type: i32,
    records_back: i32,
    date: i32,
    received_at_ns: u64,
}

#[repr(C)]
struct Control {
    kind: i32,
    id: i32,
    detail: *const c_char,
}

#[repr(C)]
struct RawData {
    code: u8,
    payload: *const u8,
    payload_len: usize,
}

/// Tag prefix on `Event`. The real `TdxFpssEventKind` has six variants
/// (Quote/Trade/OpenInterest/Ohlcvc/Control/RawData); the bench only
/// constructs `Quote` (the dominant FPSS variant by event count), so
/// only that variant is declared. `#[repr(C)]` discriminant width is
/// the C `int` width regardless of variant count, so size parity with
/// the real `TdxFpssEvent` is preserved.
#[repr(C)]
enum Kind {
    Quote = 0,
}

#[repr(C)]
struct Event {
    kind: Kind,
    ohlcvc: Ohlcvc,
    open_interest: OpenInterest,
    quote: Quote,
    trade: Trade,
    control: Control,
    raw_data: RawData,
}

/// Buffered wrapper that owns heap data backing the pointer fields,
/// mirroring `FfiBufferedEvent` exactly: `event` at offset 0, then
/// `Option<CString>` (control detail) and `Option<Vec<u8>>` (raw
/// payload). Sending this across a channel is what the streaming hot
/// path actually pays for today.
#[repr(C)]
struct BufferedEvent {
    event: Event,
    _detail_string: Option<CString>,
    _raw_payload: Option<Vec<u8>>,
}

// SAFETY: identical reasoning to `FfiBufferedEvent` in the ffi crate —
// owned heap data is not aliased after send; receiving thread is the
// only reader.
unsafe impl Send for BufferedEvent {}

const ZERO_CONTRACT: Contract = Contract {
    root: ptr::null(),
    sec_type: 0,
    has_exp_date: false,
    exp_date: 0,
    has_is_call: false,
    is_call: false,
    has_strike: false,
    strike: 0,
};

const ZERO_OHLCVC: Ohlcvc = Ohlcvc {
    contract_id: 0,
    contract: ZERO_CONTRACT,
    ms_of_day: 0,
    open: 0.0,
    high: 0.0,
    low: 0.0,
    close: 0.0,
    volume: 0,
    count: 0,
    date: 0,
    received_at_ns: 0,
};

const ZERO_OI: OpenInterest = OpenInterest {
    contract_id: 0,
    contract: ZERO_CONTRACT,
    ms_of_day: 0,
    open_interest: 0,
    date: 0,
    received_at_ns: 0,
};

const ZERO_TRADE: Trade = Trade {
    contract_id: 0,
    contract: ZERO_CONTRACT,
    ms_of_day: 0,
    sequence: 0,
    ext_condition1: 0,
    ext_condition2: 0,
    ext_condition3: 0,
    ext_condition4: 0,
    condition: 0,
    size: 0,
    exchange: 0,
    price: 0.0,
    condition_flags: 0,
    price_flags: 0,
    volume_type: 0,
    records_back: 0,
    date: 0,
    received_at_ns: 0,
};

const ZERO_CONTROL: Control = Control {
    kind: 0,
    id: 0,
    detail: ptr::null(),
};

const ZERO_RAW: RawData = RawData {
    code: 0,
    payload: ptr::null(),
    payload_len: 0,
};

/// Build a representative Quote event — the dominant variant on a live
/// FPSS stream by event count. Fields chosen to match
/// `bench_fpss_event::sample_quote` so the cost of building the payload
/// itself is consistent with the existing decode benches.
fn sample_quote(seq: i32) -> BufferedEvent {
    BufferedEvent {
        event: Event {
            kind: Kind::Quote,
            ohlcvc: ZERO_OHLCVC,
            open_interest: ZERO_OI,
            quote: Quote {
                contract_id: seq,
                contract: ZERO_CONTRACT,
                ms_of_day: 34_200_000 + seq,
                bid_size: 100,
                bid_exchange: 1,
                bid: 450.25,
                bid_condition: 0,
                ask_size: 200,
                ask_exchange: 1,
                ask: 450.27,
                ask_condition: 0,
                date: 20_260_418,
                received_at_ns: 1_700_000_000_000_000_000,
            },
            trade: ZERO_TRADE,
            control: ZERO_CONTROL,
            raw_data: ZERO_RAW,
        },
        _detail_string: None,
        _raw_payload: None,
    }
}

// Number of events shipped through the channel (or callback) per
// criterion sample. Sized so the per-iteration wall-clock is large
// enough to dwarf criterion's own measurement overhead, and so the
// p50/p99 reported by criterion reflects steady-state behaviour rather
// than warm-up.
const EVENTS_PER_ITER: usize = 100_000;

// ─── Variant 1: std::sync::mpsc unbounded + recv_timeout ───────────────

fn run_std_mpsc() {
    let (tx, rx) = std_mpsc::channel::<BufferedEvent>();

    let producer = thread::spawn(move || {
        for i in 0..EVENTS_PER_ITER {
            // `expect` (not `unwrap_or_default`): a closed channel during
            // the bench is a hard failure, not silent data loss.
            tx.send(sample_quote(i as i32))
                .expect("std_mpsc producer: receiver dropped mid-bench");
        }
    });

    // Mirrors `tdx_unified_next_event` / `tdx_fpss_next_event`: a
    // recv_timeout(100ms) poll loop, breaking on disconnect.
    let mut received = 0usize;
    loop {
        match rx.recv_timeout(Duration::from_millis(100)) {
            Ok(ev) => {
                black_box(&ev);
                received += 1;
            }
            Err(std_mpsc::RecvTimeoutError::Disconnected) => break,
            Err(std_mpsc::RecvTimeoutError::Timeout) => {
                panic!("std_mpsc consumer: 100ms timeout with producer alive");
            }
        }
    }

    producer.join().expect("std_mpsc producer thread panicked");
    assert_eq!(received, EVENTS_PER_ITER);
}

// ─── Variants 2-4: crossbeam-channel bounded ───────────────────────────

fn run_crossbeam_bounded(capacity: usize) {
    let (tx, rx) = crossbeam_channel::bounded::<BufferedEvent>(capacity);

    let producer = thread::spawn(move || {
        for i in 0..EVENTS_PER_ITER {
            tx.send(sample_quote(i as i32))
                .expect("crossbeam producer: receiver dropped mid-bench");
        }
    });

    let mut received = 0usize;
    loop {
        match rx.recv_timeout(Duration::from_millis(100)) {
            Ok(ev) => {
                black_box(&ev);
                received += 1;
            }
            Err(crossbeam_channel::RecvTimeoutError::Disconnected) => break,
            Err(crossbeam_channel::RecvTimeoutError::Timeout) => {
                panic!("crossbeam consumer: 100ms timeout with producer alive");
            }
        }
    }

    producer.join().expect("crossbeam producer thread panicked");
    assert_eq!(received, EVENTS_PER_ITER);
}

// ─── Variant 5: direct extern "C" fn callback (C/C++ tier-1 path) ──────
//
// Models the proposed FFI surface from issue #482: the FPSS thread
// invokes the user's `extern "C" fn(*const FfiBufferedEvent, *mut c_void)`
// pointer in-place, no channel between producer and consumer. The
// `Box<dyn Fn>` adapter is the realistic shape — Rust closures cannot
// be coerced to `extern "C" fn` directly when they capture state, so
// production code routes through a thin trampoline that loads the
// user's `extern "C" fn` from a `*mut c_void` cookie. We measure exactly
// that two-step: trampoline (closure call) -> user callback (extern fn).

extern "C" fn user_callback(ev: *const BufferedEvent, cookie: *mut c_void) {
    // SAFETY: the bench harness owns `ev` for the duration of the call
    // and passes a real `*mut u64` counter as the cookie.
    let counter = unsafe { &mut *(cookie as *mut u64) };
    let ev_ref = unsafe { &*ev };
    black_box(ev_ref);
    *counter = counter.wrapping_add(1);
}

fn run_direct_callback() {
    let mut counter: u64 = 0;
    let cookie: *mut c_void = (&mut counter as *mut u64).cast();

    // The trampoline: a `Box<dyn Fn>` closing over the user-supplied
    // `extern "C" fn` and cookie. Invocation cost = vtable dispatch +
    // extern fn call, which is what tier-1 C/C++ consumers will pay.
    let trampoline: Box<dyn Fn(&BufferedEvent)> = Box::new(move |ev: &BufferedEvent| {
        user_callback(ev as *const BufferedEvent, cookie);
    });

    // Same-thread producer + callback — the realistic path where the
    // FPSS reader thread IS the callback thread. No queueing, no
    // wake-up, no allocation past the `BufferedEvent` itself.
    for i in 0..EVENTS_PER_ITER {
        let ev = sample_quote(i as i32);
        trampoline(&ev);
    }

    assert_eq!(counter, EVENTS_PER_ITER as u64);
}

// ─── Criterion driver ──────────────────────────────────────────────────

fn bench_std_mpsc_unbounded(c: &mut Criterion) {
    let mut group = c.benchmark_group("streaming_channels/std_mpsc_unbounded");
    group.throughput(Throughput::Elements(EVENTS_PER_ITER as u64));
    group.sample_size(10);
    group.bench_function("100k_events", |b| b.iter(run_std_mpsc));
    group.finish();
}

fn bench_crossbeam_bounded_256(c: &mut Criterion) {
    let mut group = c.benchmark_group("streaming_channels/crossbeam_bounded_256");
    group.throughput(Throughput::Elements(EVENTS_PER_ITER as u64));
    group.sample_size(10);
    group.bench_function("100k_events", |b| b.iter(|| run_crossbeam_bounded(256)));
    group.finish();
}

fn bench_crossbeam_bounded_1024(c: &mut Criterion) {
    let mut group = c.benchmark_group("streaming_channels/crossbeam_bounded_1024");
    group.throughput(Throughput::Elements(EVENTS_PER_ITER as u64));
    group.sample_size(10);
    group.bench_function("100k_events", |b| b.iter(|| run_crossbeam_bounded(1024)));
    group.finish();
}

fn bench_crossbeam_bounded_8192(c: &mut Criterion) {
    let mut group = c.benchmark_group("streaming_channels/crossbeam_bounded_8192");
    group.throughput(Throughput::Elements(EVENTS_PER_ITER as u64));
    group.sample_size(10);
    group.bench_function("100k_events", |b| b.iter(|| run_crossbeam_bounded(8192)));
    group.finish();
}

fn bench_direct_callback(c: &mut Criterion) {
    let mut group = c.benchmark_group("streaming_channels/direct_callback");
    group.throughput(Throughput::Elements(EVENTS_PER_ITER as u64));
    group.sample_size(10);
    group.bench_function("100k_events", |b| b.iter(run_direct_callback));
    group.finish();
}

criterion_group!(
    streaming_channels,
    bench_std_mpsc_unbounded,
    bench_crossbeam_bounded_256,
    bench_crossbeam_bounded_1024,
    bench_crossbeam_bounded_8192,
    bench_direct_callback,
);
criterion_main!(streaming_channels);
