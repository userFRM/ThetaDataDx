//! Protobuf decode throughput baseline (issue #584).
//!
//! Captures the prost-side decode cost across the payload shapes
//! production endpoints emit, so the SIMD / zero-copy follow-up has a
//! concrete baseline to beat. Three variants:
//!
//! * `protobuf_decode/quote_chunk` — 1024-row quote-tick `DataTable`,
//!   the per-chunk shape MDDS emits at `interval=1s` strike-range
//!   bursts.
//! * `protobuf_decode/quote_chunk_large` — 8192-row quote-tick
//!   `DataTable`, matches the burst-mode jumbo frames the h2
//!   pipeline forwards when several response chunks coalesce into a
//!   single DATA frame.
//! * `protobuf_decode/trade_chunk` — 1024-row trade-tick `DataTable`,
//!   used by `*_history_trade` endpoints. Wider per-row schema
//!   (15 columns vs 10 for quotes) so the per-row prost decode cost
//!   diverges from the quote shape.
//!
//! Reported throughput is bytes/sec at the protobuf decode boundary —
//! divide by ~80 bytes/row (post-prost) for ticks/sec. A future SIMD
//! decoder targeting the `repeated DataValueList` field would need to
//! clear ≥ 1.5× the prost baseline reported here before the swap is
//! worth the cognitive cost of a new dependency in the decode path.

use std::hint::black_box;

use criterion::{
    criterion_group, criterion_main, AxisScale, BenchmarkId, Criterion, PlotConfiguration,
    Throughput,
};
use prost::Message;

use thetadatadx::wire::{DataTable, DataValueList};

#[path = "common/quote_fixture.rs"]
mod fixture;
use fixture::{build_quote_data_table, dv_number, dv_price};

// ─── Trade fixture (quote fixture lives in `fixture`) ───────────────

/// Build a trade-tick `DataTable` with `n` rows. Mirrors the
/// canonical 15-column trade schema MDDS emits.
fn build_trade_data_table(n: usize) -> DataTable {
    let headers = vec![
        "ms_of_day".to_string(),
        "sequence".to_string(),
        "ext_condition1".to_string(),
        "ext_condition2".to_string(),
        "ext_condition3".to_string(),
        "ext_condition4".to_string(),
        "condition".to_string(),
        "size".to_string(),
        "exchange".to_string(),
        "price".to_string(),
        "condition_flags".to_string(),
        "price_flags".to_string(),
        "volume_type".to_string(),
        "records_back".to_string(),
        "date".to_string(),
    ];
    let mut rows = Vec::with_capacity(n);
    for i in 0..n {
        rows.push(DataValueList {
            values: vec![
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
            ],
        });
    }
    DataTable {
        headers,
        data_table: rows,
    }
}

/// Encode a `DataTable` to bytes — the input to prost decode.
fn encode_table(table: &DataTable) -> Vec<u8> {
    let mut buf = Vec::with_capacity(table.encoded_len());
    table.encode(&mut buf).expect("encode");
    buf
}

// ─── Benches ───────────────────────────────────────────────────────

/// Prost decode at varying quote-tick payload sizes. Reports
/// throughput in bytes/sec (the criterion runner converts the input
/// size into a rate based on elapsed time).
fn bench_protobuf_decode_quote(c: &mut Criterion) {
    let mut group = c.benchmark_group("protobuf_decode/quote_chunk");
    group.plot_config(PlotConfiguration::default().summary_scale(AxisScale::Logarithmic));
    for &rows in &[256usize, 1024, 4096, 8192] {
        let table = build_quote_data_table(rows);
        let encoded = encode_table(&table);
        let size_bytes = encoded.len() as u64;
        group.throughput(Throughput::Bytes(size_bytes));
        group.bench_with_input(BenchmarkId::from_parameter(rows), &encoded, |b, encoded| {
            b.iter(|| {
                let decoded =
                    DataTable::decode(black_box(encoded.as_slice())).expect("benchmark fixture");
                black_box(decoded.data_table.len());
            });
        });
    }
    group.finish();
}

/// Prost decode at varying trade-tick payload sizes. Wider per-row
/// schema (15 cols vs 10 for quotes) — the per-row prost cost
/// diverges from the quote shape, surfacing whether the bottleneck
/// is total bytes or per-row varint overhead.
fn bench_protobuf_decode_trade(c: &mut Criterion) {
    let mut group = c.benchmark_group("protobuf_decode/trade_chunk");
    group.plot_config(PlotConfiguration::default().summary_scale(AxisScale::Logarithmic));
    for &rows in &[256usize, 1024, 4096] {
        let table = build_trade_data_table(rows);
        let encoded = encode_table(&table);
        let size_bytes = encoded.len() as u64;
        group.throughput(Throughput::Bytes(size_bytes));
        group.bench_with_input(BenchmarkId::from_parameter(rows), &encoded, |b, encoded| {
            b.iter(|| {
                let decoded =
                    DataTable::decode(black_box(encoded.as_slice())).expect("benchmark fixture");
                black_box(decoded.data_table.len());
            });
        });
    }
    group.finish();
}

criterion_group!(
    protobuf_decode_benches,
    bench_protobuf_decode_quote,
    bench_protobuf_decode_trade,
);
criterion_main!(protobuf_decode_benches);
