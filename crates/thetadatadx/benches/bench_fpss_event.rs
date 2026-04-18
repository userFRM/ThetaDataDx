//! Microbenchmarks for `FpssEvent` hot-path operations.
//!
//! These measure the per-event cost the SDK pays on the streaming hot path
//! independent of I/O, server pacing, or user consumer code. Pairs with
//! `bench_framing.rs` (read/write of wire frames) to bracket the full
//! Rust-side cost of delivering one event.
//!
//! Criterion gives us warm-up, outlier rejection, and confidence intervals;
//! `black_box` prevents LLVM from folding the benchmarked work away as
//! dead code.
//!
//! Run: `cargo bench --bench bench_fpss_event`

use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion};
use std::hint::black_box;
use std::sync::Arc;

use thetadatadx::fpss::{FpssData, FpssEvent};

fn sample_quote(contract_id: i32) -> FpssEvent {
    FpssEvent::Data(FpssData::Quote {
        contract_id,
        symbol: Arc::from("SPY"),
        ms_of_day: 34_200_000,
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
    })
}

fn sample_trade(contract_id: i32) -> FpssEvent {
    FpssEvent::Data(FpssData::Trade {
        contract_id,
        symbol: Arc::from("SPY"),
        ms_of_day: 34_200_001,
        sequence: 42,
        ext_condition1: 0,
        ext_condition2: 0,
        ext_condition3: 0,
        ext_condition4: 0,
        condition: 50,
        size: 100,
        exchange: 1,
        price: 450.26,
        condition_flags: 0,
        price_flags: 0,
        volume_type: 0,
        records_back: 0,
        date: 20_260_418,
        received_at_ns: 1_700_000_000_000_000_001,
    })
}

fn sample_ohlcvc(contract_id: i32) -> FpssEvent {
    FpssEvent::Data(FpssData::Ohlcvc {
        contract_id,
        symbol: Arc::from("SPY"),
        ms_of_day: 34_200_000,
        open: 449.5,
        high: 450.3,
        low: 449.2,
        close: 450.26,
        volume: 1_234_567,
        count: 1234,
        date: 20_260_418,
        received_at_ns: 1_700_000_000_000_000_002,
    })
}

/// FpssEvent::clone cost per variant. `FpssData` variants use `Arc<str>` for
/// `symbol`, so cloning a Data event should be a field copy + single
/// refcount bump — no heap allocation on the hot path.
fn bench_event_clone(c: &mut Criterion) {
    let mut group = c.benchmark_group("FpssEvent::clone");
    for (label, ev) in [
        ("Quote", sample_quote(42)),
        ("Trade", sample_trade(42)),
        ("Ohlcvc", sample_ohlcvc(42)),
    ] {
        group.bench_with_input(BenchmarkId::from_parameter(label), &ev, |b, ev| {
            b.iter(|| black_box(black_box(ev).clone()))
        });
    }
    group.finish();
}

/// Cost of matching + destructuring an event — approximates what every
/// per-event consumer (numpy drain, PyDict builder, WS bridge, CLI) pays
/// on the dispatch side.
fn bench_event_match(c: &mut Criterion) {
    let mut group = c.benchmark_group("FpssEvent::match");
    for (label, ev) in [
        ("Quote", sample_quote(42)),
        ("Trade", sample_trade(42)),
        ("Ohlcvc", sample_ohlcvc(42)),
    ] {
        group.bench_with_input(BenchmarkId::from_parameter(label), &ev, |b, ev| {
            b.iter(|| {
                let score: i64 = match black_box(ev) {
                    FpssEvent::Data(FpssData::Quote {
                        contract_id,
                        bid_size,
                        ask_size,
                        ..
                    }) => i64::from(*contract_id) + i64::from(*bid_size) + i64::from(*ask_size),
                    FpssEvent::Data(FpssData::Trade {
                        contract_id, size, ..
                    }) => i64::from(*contract_id) + i64::from(*size),
                    FpssEvent::Data(FpssData::Ohlcvc {
                        contract_id,
                        volume,
                        ..
                    }) => i64::from(*contract_id) + *volume,
                    _ => 0,
                };
                black_box(score)
            })
        });
    }
    group.finish();
}

criterion_group!(benches, bench_event_clone, bench_event_match);
criterion_main!(benches);
