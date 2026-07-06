//! Allocations-per-decoded-row gate harness (issue #696, metric 1).
//!
//! Decodes a fixed fixture of each representative tick type — trade,
//! quote, OHLC — through the public MDDS decode path and records the
//! number of heap allocations charged per decoded row. The figure is
//! written to `target/perf-gate/decode_allocations.json`, which
//! `scripts/ci/check_perf_gate.py` diffs against the committed baseline.
//!
//! Why this gate does not flake on shared CI runners: the metric is an
//! allocation COUNT, not a wall-clock time. Decoding the identical
//! fixture performs the identical number of allocations on a 2-vCPU
//! hosted runner and on a developer workstation — CPU frequency,
//! scheduler pressure, and memory bandwidth do not change how many
//! times the decode path calls `alloc`. A purely relative wall-clock
//! gate against a fixed baseline would, by contrast, trip on the
//! runner's slower clock alone. The allocation count is the
//! deterministic invariant the low-allocation decode work earned, so it
//! is what we pin.
//!
//! The counting allocator is installed ONLY in this bench binary (via
//! the `#[global_allocator]` below). The shipped `thetadatadx` rlib
//! declares no custom global allocator and is unaffected.
//!
//! Run: `cargo bench -p thetadatadx-rs --features __internal \
//!         --bench bench_decode_allocations -- --noplot`

use std::hint::black_box;
use std::path::PathBuf;
use std::time::Duration;

use criterion::{criterion_group, criterion_main, Criterion};

use thetadatadx::decode::{
    decode_data_table, parse_ohlc_ticks, parse_quote_ticks, parse_trade_ticks,
};
use thetadatadx::wire as proto;

#[path = "perf_gate_support/perf_alloc.rs"]
mod perf_alloc;
use perf_alloc::{snapshot, write_metric_file, Sample};

#[path = "common/quote_fixture.rs"]
mod fixture;
use fixture::{build_quote_data_table, dv_number, dv_price};

#[global_allocator]
static ALLOC: perf_alloc::CountingAllocator = perf_alloc::CountingAllocator;

// ─── Fixtures ────────────────────────────────────────────────────────
//
// Fixed row count. Large enough that the per-row figure is stable and
// any constant one-shot allocation (the `Vec` header for the row
// container, say) amortizes toward zero; the gate keys on the
// steady-state per-row slope, so the exact constant does not matter as
// long as it is held fixed across baseline and CI.

const FIXTURE_ROWS: usize = 1000;

fn row_of(values: Vec<proto::DataValue>) -> proto::DataValueList {
    proto::DataValueList { values }
}

/// 15-column trade-tick `DataTable` — the canonical MDDS trade schema.
fn build_trade_data_table(n: usize) -> proto::DataTable {
    let headers = vec![
        "ms_of_day".into(),
        "sequence".into(),
        "ext_condition1".into(),
        "ext_condition2".into(),
        "ext_condition3".into(),
        "ext_condition4".into(),
        "condition".into(),
        "size".into(),
        "exchange".into(),
        "price".into(),
        "condition_flags".into(),
        "price_flags".into(),
        "volume_type".into(),
        "records_back".into(),
        "date".into(),
    ];
    let mut rows = Vec::with_capacity(n);
    for i in 0..n {
        rows.push(row_of(vec![
            dv_number(34_200_000 + i as i64 * 100),
            dv_number(i as i64 + 1),
            dv_number(0),
            dv_number(0),
            dv_number(0),
            dv_number(0),
            dv_number(0),
            dv_number(100 + (i % 50) as i64),
            dv_number(4),
            dv_price(15_025 + (i % 200) as i32, 8),
            dv_number(0),
            dv_number(0),
            dv_number(0),
            dv_number(0),
            dv_number(20_240_315),
        ]));
    }
    proto::DataTable {
        headers,
        data_table: rows,
    }
}

/// 8-column OHLC-tick `DataTable` — the canonical MDDS OHLC schema.
fn build_ohlc_data_table(n: usize) -> proto::DataTable {
    let headers = vec![
        "ms_of_day".into(),
        "open".into(),
        "high".into(),
        "low".into(),
        "close".into(),
        "volume".into(),
        "count".into(),
        "date".into(),
    ];
    let mut rows = Vec::with_capacity(n);
    for i in 0..n {
        let base = 15_000 + (i % 300) as i32;
        rows.push(row_of(vec![
            dv_number(34_200_000 + i as i64 * 60_000),
            dv_price(base, 8),
            dv_price(base + 50, 8),
            dv_price(base - 30, 8),
            dv_price(base + 10, 8),
            dv_number(10_000 + (i * 137 % 5000) as i64),
            dv_number(100 + (i % 200) as i64),
            dv_number(20_240_315),
        ]));
    }
    proto::DataTable {
        headers,
        data_table: rows,
    }
}

fn uncompressed_response(table: &proto::DataTable) -> proto::ResponseData {
    use prost::Message;
    let mut raw = Vec::with_capacity(table.encoded_len());
    table.encode(&mut raw).expect("encode fixture table");
    proto::ResponseData {
        compressed_data: raw,
        compression_description: Some(proto::CompressionDescription {
            algo: proto::CompressionAlgo::None as i32,
            level: 0,
        }),
        original_size: 0,
    }
}

// ─── Measurement ─────────────────────────────────────────────────────
//
// Each sample brackets a single full decode of the fixed-row fixture
// with two counter snapshots and reports `(allocs delta) / rows`. The
// decode is run once inside the bracket (not `iters` times) because the
// metric is a per-row allocation count, not a throughput: one decode of
// N rows yields the exact per-row figure with no averaging needed, and
// `from_delta` divides by the row count. We still run the decode once
// before bracketing to retire any first-call lazy initialisation so the
// bracketed pass measures steady-state decode allocations only.

/// Decode `response` end to end: protobuf table decode followed by the
/// typed tick parse, exactly as a client receiving this payload would.
/// Returns the decoded row count so the caller can assert the fixture
/// drove the expected number of rows.
fn decode_trade(response: &proto::ResponseData) -> usize {
    let mut r = response.clone();
    let table = decode_data_table(&mut r).expect("decode trade table");
    let ticks = parse_trade_ticks(&table).expect("parse trade ticks");
    black_box(&ticks);
    ticks.len()
}

fn decode_quote(response: &proto::ResponseData) -> usize {
    let mut r = response.clone();
    let table = decode_data_table(&mut r).expect("decode quote table");
    let ticks = parse_quote_ticks(&table).expect("parse quote ticks");
    black_box(&ticks);
    ticks.len()
}

fn decode_ohlc(response: &proto::ResponseData) -> usize {
    let mut r = response.clone();
    let table = decode_data_table(&mut r).expect("decode ohlc table");
    let ticks = parse_ohlc_ticks(&table).expect("parse ohlc ticks");
    black_box(&ticks);
    ticks.len()
}

/// Bracket one decode of the fixture and return the per-row sample.
fn sample_decode(
    id: &str,
    response: &proto::ResponseData,
    decode: fn(&proto::ResponseData) -> usize,
) -> Sample {
    // Warm-up pass outside the bracket: retires any one-shot lazy init
    // (interned header lookups, etc.) so the measured pass is steady
    // state.
    let warm_rows = decode(response);
    assert_eq!(
        warm_rows, FIXTURE_ROWS,
        "{id}: fixture decoded {warm_rows} rows, expected {FIXTURE_ROWS}"
    );

    let before = snapshot();
    let rows = decode(response);
    let after = snapshot();
    assert_eq!(
        rows, FIXTURE_ROWS,
        "{id}: fixture decoded {rows} rows, expected {FIXTURE_ROWS}"
    );

    Sample::from_delta(id, before, after, rows as u64)
}

fn bench_decode_allocations(c: &mut Criterion) {
    let trade = uncompressed_response(&build_trade_data_table(FIXTURE_ROWS));
    let quote = uncompressed_response(&build_quote_data_table(FIXTURE_ROWS));
    let ohlc = uncompressed_response(&build_ohlc_data_table(FIXTURE_ROWS));

    let samples = vec![
        sample_decode("decode_allocations/trade", &trade, decode_trade),
        sample_decode("decode_allocations/quote", &quote, decode_quote),
        sample_decode("decode_allocations/ohlc", &ohlc, decode_ohlc),
    ];

    for s in &samples {
        eprintln!(
            "perf-gate decode: {:<28} {:.4} allocs/row  {:.1} bytes/row  ({} rows)",
            s.id, s.allocs_per_unit, s.bytes_per_unit, s.units
        );
    }

    let out = PathBuf::from(
        std::env::var("PERF_GATE_OUT_DIR").unwrap_or_else(|_| "target/perf-gate".to_string()),
    )
    .join("decode_allocations.json");
    write_metric_file(
        &out,
        "Allocations per decoded row for each representative MDDS tick type. \
         Deterministic: an allocation count is CPU-independent, so it gates safely on shared runners. \
         allocs_per_unit = heap allocations charged per decoded row over a fixed-row fixture.",
        &samples,
    );

    // Also register a no-op timed bench so `cargo bench` reports this
    // target in its summary line. The metric of record is the JSON
    // file above; the timing here is incidental and intentionally
    // unused by the gate.
    let mut group = c.benchmark_group("decode_allocations");
    group.bench_function("trade_decode", |b| {
        b.iter(|| black_box(decode_trade(black_box(&trade))));
    });
    group.finish();
}

criterion_group! {
    name = benches;
    config = Criterion::default()
        .measurement_time(Duration::from_secs(2))
        .sample_size(10);
    targets = bench_decode_allocations
}
criterion_main!(benches);
