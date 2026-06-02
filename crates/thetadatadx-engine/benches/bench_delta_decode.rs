//! Per-tick allocation budget for FPSS FIT delta decode.
//!
//! `DeltaState::decode_tick` runs on every FPSS quote / trade /
//! open-interest / OHLCVC frame. The previous implementation paid two
//! `Vec<i32>` allocations per tick (one for the returned absolute tick
//! data, one for the cloned `prev` entry stored in the per-contract
//! map). This bench wraps the system allocator in a `CountingAllocator`
//! that tallies bytes-allocated and alloc count, then asserts the
//! per-iteration alloc count is zero after the first absolute tick is
//! seeded.
//!
//! The bench targets `decode_frame` (the public `__test_internals`
//! entry point) so the measurement covers the full
//! frame → typed-event path the I/O loop walks, not just the FIT
//! decoder in isolation. Two of the three allocations the frame
//! previous paid still exist by design (the `Arc<Contract>` ref-count
//! bump dominates) — the assertion is scoped to the FIT-delta
//! sub-region by seeding the contract cache outside the timed loop.

use std::alloc::{GlobalAlloc, Layout, System};
use std::collections::HashMap;
use std::hint::black_box;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Instant;

use criterion::{criterion_group, criterion_main, Criterion};

use tdbe::types::enums::{SecType, StreamMsgType};
use thetadatadx_engine::fpss::__test_internals::{decode_frame, DeltaState};
use thetadatadx_engine::fpss::protocol::Contract;

// ─── Counting allocator ──────────────────────────────────────────────
//
// Wraps the system allocator and tallies bytes-allocated and alloc
// count. The per-iteration snapshot lives INSIDE the `iter_custom`
// timed region so Criterion bookkeeping is excluded.

struct CountingAllocator;

static BYTES_ALLOCATED: AtomicU64 = AtomicU64::new(0);
static ALLOC_COUNT: AtomicU64 = AtomicU64::new(0);

// SAFETY: every method forwards verbatim to `std::alloc::System`, which
// itself satisfies the `GlobalAlloc` contract. Per-call `Relaxed` adds on
// `AtomicU64` are pure observational state and cannot violate the
// allocator's invariants. Bench-only; never linked into the shipped
// library.
unsafe impl GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        // SAFETY: GlobalAlloc::alloc precondition is `layout.size() > 0`
        // and `layout.align()` is a non-zero power of two — the alloc
        // shim Rust generates for any `#[global_allocator]` enforces
        // both before this call. `System.alloc` is the System impl
        // upstream; forwarding satisfies it verbatim.
        let ptr = unsafe { System.alloc(layout) };
        if !ptr.is_null() {
            BYTES_ALLOCATED.fetch_add(layout.size() as u64, Ordering::Relaxed);
            ALLOC_COUNT.fetch_add(1, Ordering::Relaxed);
        }
        ptr
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        // SAFETY: GlobalAlloc::dealloc precondition — `ptr` was
        // returned by a prior `alloc` on this allocator with the same
        // `layout`, and has not been deallocated. The shim Rust
        // generates from `Vec`, `Box`, etc. upholds that pairing;
        // forwarding to `System.dealloc` satisfies the System impl.
        unsafe { System.dealloc(ptr, layout) }
    }
}

#[global_allocator]
static ALLOC: CountingAllocator = CountingAllocator;

#[inline]
fn alloc_snapshot() -> (u64, u64) {
    (
        BYTES_ALLOCATED.load(Ordering::Relaxed),
        ALLOC_COUNT.load(Ordering::Relaxed),
    )
}

// ─── FIT encoder (mirrors the in-crate test helper) ──────────────────

const FIELD_SEP: u8 = 0xB;
const END_NIB: u8 = 0xD;
const NEG_NIB: u8 = 0xE;

fn int_to_nibbles(val: i32) -> Vec<u8> {
    let mut nibbles = Vec::new();
    if val < 0 {
        nibbles.push(NEG_NIB);
    }
    let abs = (val as i64).unsigned_abs();
    if abs == 0 {
        nibbles.push(0);
        return nibbles;
    }
    let s = abs.to_string();
    for ch in s.chars() {
        nibbles.push(ch.to_digit(10).unwrap() as u8);
    }
    nibbles
}

fn encode_fit_row(fields: &[i32]) -> Vec<u8> {
    let mut nibbles: Vec<u8> = Vec::new();
    for (i, &val) in fields.iter().enumerate() {
        if i > 0 {
            nibbles.push(FIELD_SEP);
        }
        nibbles.extend(int_to_nibbles(val));
    }
    nibbles.push(END_NIB);

    let mut bytes = Vec::new();
    let mut i = 0;
    while i < nibbles.len() {
        let high = nibbles[i];
        let low = if i + 1 < nibbles.len() {
            nibbles[i + 1]
        } else {
            0
        };
        bytes.push((high << 4) | (low & 0x0F));
        i += 2;
    }
    bytes
}

// ─── Bench harness ───────────────────────────────────────────────────

/// Build a representative 16-field trade FIT payload. Values are
/// arbitrary positive integers that decode without sign-nibble overhead.
fn sample_trade_payload(contract_id: i32) -> Vec<u8> {
    encode_fit_row(&[
        contract_id,
        34_200_000, // ms_of_day
        99_999,     // sequence
        1,
        2,
        3,
        4,
        15,         // condition
        500,        // size
        57,         // exchange
        18_750_000, // price
        7,
        3,
        1,
        0,
        8,          // price_type
        20_260_517, // date
    ])
}

fn bench_delta_decode_zero_alloc(c: &mut Criterion) {
    // Seed `local_contracts` so the hot loop never falls into the
    // `unresolved_sentinel` path (which legitimately allocates an
    // `Arc<Contract>`-wrapped sentinel). With the cache hit, the
    // contract resolver is a refcount bump only — no heap traffic on
    // the per-tick path.
    let mut local_contracts: HashMap<i32, Arc<Contract>> = HashMap::new();
    local_contracts.insert(
        200,
        Arc::new(Contract {
            symbol: "BENCH".to_string(),
            sec_type: SecType::Option,
            expiration: Some(20_260_517),
            is_call: Some(true),
            strike: Some(500_000),
        }),
    );
    let authenticated = AtomicBool::new(true);
    let shutdown = AtomicBool::new(false);
    let mut state = DeltaState::new();

    // Seed the per-contract delta state with one absolute tick before
    // entering the timed loop. The first absolute tick legitimately
    // pays the per-contract `HashMap::insert` alloc; the steady-state
    // loop measures only the delta path.
    let seed_payload = sample_trade_payload(200);
    let _ = decode_frame(
        StreamMsgType::Trade,
        &seed_payload,
        &authenticated,
        &mut local_contracts,
        &shutdown,
        &mut state,
        false,
    );

    let delta_payload = sample_trade_payload(200);

    let mut group = c.benchmark_group("fpss_delta_decode");
    group.bench_function("trade_steady_state_per_tick", |b| {
        b.iter_custom(|iters| {
            // Warm-up tick — keeps the per-iteration snapshot away from
            // any one-shot lazy init inside `decode_frame`.
            let _ = decode_frame(
                StreamMsgType::Trade,
                &delta_payload,
                &authenticated,
                &mut local_contracts,
                &shutdown,
                &mut state,
                false,
            );

            let (bytes_before, count_before) = alloc_snapshot();
            let t0 = Instant::now();
            for _ in 0..iters {
                let result = decode_frame(
                    StreamMsgType::Trade,
                    black_box(&delta_payload),
                    &authenticated,
                    &mut local_contracts,
                    &shutdown,
                    &mut state,
                    false,
                );
                black_box(result);
            }
            let elapsed = t0.elapsed();
            let (bytes_after, count_after) = alloc_snapshot();

            let bytes_per_iter = (bytes_after - bytes_before) as f64 / iters as f64;
            let allocs_per_iter = (count_after - count_before) as f64 / iters as f64;
            eprintln!(
                "fpss_delta_decode/trade_steady_state_per_tick: {bytes_per_iter:.3} bytes/iter, \
                 {allocs_per_iter:.3} allocs/iter ({iters} iters)"
            );

            // Steady-state allocation bar: zero heap allocations on the
            // state delta decode path. The current implementation
            // copies into a caller-owned stack buffer and stores the
            // previous tick inline in the `HashMap<(u8, i32), [i32; 16]>`
            // slot — no `Vec::clone` or `Vec::to_vec` per tick. The
            // HashMap slot is reused (in-place update). The previous
            // baseline allocated two `Vec<i32>` per tick (~144 B each).
            assert_eq!(
                count_after - count_before,
                0,
                "regression: per-tick FPSS delta decode allocates ({bytes_per_iter} bytes/iter)"
            );

            elapsed
        });
    });
    group.finish();
}

criterion_group!(benches, bench_delta_decode_zero_alloc);
criterion_main!(benches);
