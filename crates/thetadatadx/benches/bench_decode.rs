use criterion::{criterion_group, criterion_main, Criterion};
use std::hint::black_box;

use thetadatadx::decode::{
    decode_data_table, decompress_response, extract_number_column, extract_price_column,
    parse_ohlc_ticks, parse_quote_ticks, parse_trade_ticks,
};
use thetadatadx::proto;

// ═══════════════════════════════════════════════════════════════════════════
//  Helpers
// ═══════════════════════════════════════════════════════════════════════════

/// Build a DataValue containing a Number.
fn dv_number(n: i64) -> proto::DataValue {
    proto::DataValue {
        data_type: Some(proto::data_value::DataType::Number(n)),
    }
}

/// Build a DataValue containing a Price.
fn dv_price(value: i32, r#type: i32) -> proto::DataValue {
    proto::DataValue {
        data_type: Some(proto::data_value::DataType::Price(proto::Price {
            value,
            r#type,
        })),
    }
}

fn row_of(values: Vec<proto::DataValue>) -> proto::DataValueList {
    proto::DataValueList { values }
}

/// Build a realistic trade-tick DataTable with `n` rows.
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
            dv_number(34_200_000 + i as i64 * 100), // ms_of_day
            dv_number(i as i64 + 1),                // sequence
            dv_number(0),                           // ext_condition1
            dv_number(0),                           // ext_condition2
            dv_number(0),                           // ext_condition3
            dv_number(0),                           // ext_condition4
            dv_number(0),                           // condition
            dv_number(100 + (i % 50) as i64),       // size
            dv_number(4),                           // exchange (NYSE)
            dv_price(15025 + (i % 200) as i32, 8),  // price ~150.25
            dv_number(0),                           // condition_flags
            dv_number(0),                           // price_flags
            dv_number(0),                           // volume_type
            dv_number(0),                           // records_back
            dv_number(20240315),                    // date
        ]));
    }
    proto::DataTable {
        headers,
        data_table: rows,
    }
}

/// Build a realistic quote-tick DataTable with `n` rows.
fn build_quote_data_table(n: usize) -> proto::DataTable {
    let headers = vec![
        "ms_of_day".into(),
        "bid_size".into(),
        "bid_exchange".into(),
        "bid".into(),
        "bid_condition".into(),
        "ask_size".into(),
        "ask_exchange".into(),
        "ask".into(),
        "ask_condition".into(),
        "date".into(),
    ];
    let mut rows = Vec::with_capacity(n);
    for i in 0..n {
        rows.push(row_of(vec![
            dv_number(34_200_000 + i as i64 * 50),
            dv_number(10 + (i % 100) as i64),      // bid_size
            dv_number(4),                          // bid_exchange
            dv_price(15020 + (i % 100) as i32, 8), // bid ~150.20
            dv_number(1),                          // bid_condition
            dv_number(5 + (i % 80) as i64),        // ask_size
            dv_number(4),                          // ask_exchange
            dv_price(15030 + (i % 100) as i32, 8), // ask ~150.30
            dv_number(1),                          // ask_condition
            dv_number(20240315),                   // date
        ]));
    }
    proto::DataTable {
        headers,
        data_table: rows,
    }
}

/// Build a realistic OHLC-tick DataTable with `n` rows.
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
        let base = 15000 + (i % 300) as i32;
        rows.push(row_of(vec![
            dv_number(34_200_000 + i as i64 * 60_000), // ms_of_day (1-min bars)
            dv_price(base, 8),                         // open
            dv_price(base + 50, 8),                    // high
            dv_price(base - 30, 8),                    // low
            dv_price(base + 10, 8),                    // close
            dv_number(10_000 + (i * 137 % 5000) as i64), // volume
            dv_number(100 + (i % 200) as i64),         // count
            dv_number(20240315),                       // date
        ]));
    }
    proto::DataTable {
        headers,
        data_table: rows,
    }
}

/// Build a DataTable with a number column for extraction benchmarks.
fn build_number_column_table(n: usize) -> proto::DataTable {
    let headers = vec!["value".into()];
    let rows = (0..n)
        .map(|i| row_of(vec![dv_number(34_200_000 + i as i64 * 100)]))
        .collect();
    proto::DataTable {
        headers,
        data_table: rows,
    }
}

/// Build a DataTable with a price column for extraction benchmarks.
fn build_price_column_table(n: usize) -> proto::DataTable {
    let headers = vec!["price".into()];
    let rows = (0..n)
        .map(|i| row_of(vec![dv_price(15000 + (i % 500) as i32, 8)]))
        .collect();
    proto::DataTable {
        headers,
        data_table: rows,
    }
}

/// Encode a DataTable to protobuf bytes (for decode benchmarks).
fn encode_data_table(table: &proto::DataTable) -> Vec<u8> {
    use prost::Message;
    let mut buf = Vec::with_capacity(table.encoded_len());
    table.encode(&mut buf).unwrap();
    buf
}

/// Build a ResponseData with zstd-compressed DataTable.
fn build_zstd_response(table: &proto::DataTable) -> proto::ResponseData {
    let raw = encode_data_table(table);
    let original_size = raw.len() as i32;
    let compressed = zstd::bulk::compress(&raw, 3).unwrap();
    proto::ResponseData {
        compressed_data: compressed,
        compression_description: Some(proto::CompressionDescription {
            algo: proto::CompressionAlgo::Zstd as i32,
            level: 3,
        }),
        original_size,
    }
}

/// Build a ResponseData with no compression.
fn build_uncompressed_response(table: &proto::DataTable) -> proto::ResponseData {
    let raw = encode_data_table(table);
    proto::ResponseData {
        compressed_data: raw,
        compression_description: Some(proto::CompressionDescription {
            algo: proto::CompressionAlgo::None as i32,
            level: 0,
        }),
        original_size: 0,
    }
}

// ═══════════════════════════════════════════════════════════════════════════
//  Response decoding benchmarks
// ═══════════════════════════════════════════════════════════════════════════

fn bench_decode_zstd_small(c: &mut Criterion) {
    // ~1KB payload: small DataTable (10 rows)
    let table = build_trade_data_table(10);
    let response = build_zstd_response(&table);
    c.bench_function("decode_zstd_small", |b| {
        b.iter(|| {
            black_box(decompress_response(black_box(&response)).unwrap());
        });
    });
}

fn bench_decode_zstd_large(c: &mut Criterion) {
    // ~100KB payload: large DataTable (1000 rows)
    let table = build_trade_data_table(1000);
    let response = build_zstd_response(&table);
    c.bench_function("decode_zstd_large", |b| {
        b.iter(|| {
            black_box(decompress_response(black_box(&response)).unwrap());
        });
    });
}

fn bench_decode_data_table_10_rows(c: &mut Criterion) {
    let table = build_trade_data_table(10);
    let response = build_uncompressed_response(&table);
    c.bench_function("decode_data_table_10_rows", |b| {
        b.iter(|| {
            black_box(decode_data_table(black_box(&response)).unwrap());
        });
    });
}

fn bench_decode_data_table_1000_rows(c: &mut Criterion) {
    let table = build_trade_data_table(1000);
    let response = build_uncompressed_response(&table);
    c.bench_function("decode_data_table_1000_rows", |b| {
        b.iter(|| {
            black_box(decode_data_table(black_box(&response)).unwrap());
        });
    });
}

fn bench_decode_extract_number_column(c: &mut Criterion) {
    let table = build_number_column_table(1000);
    c.bench_function("decode_extract_number_column", |b| {
        b.iter(|| {
            black_box(extract_number_column(black_box(&table), "value"));
        });
    });
}

fn bench_decode_extract_price_column(c: &mut Criterion) {
    let table = build_price_column_table(1000);
    c.bench_function("decode_extract_price_column", |b| {
        b.iter(|| {
            black_box(extract_price_column(black_box(&table), "price"));
        });
    });
}

fn bench_parse_trade_ticks_100(c: &mut Criterion) {
    let table = build_trade_data_table(100);
    c.bench_function("parse_trade_ticks_100", |b| {
        b.iter(|| {
            black_box(parse_trade_ticks(black_box(&table)));
        });
    });
}

fn bench_parse_quote_ticks_100(c: &mut Criterion) {
    let table = build_quote_data_table(100);
    c.bench_function("parse_quote_ticks_100", |b| {
        b.iter(|| {
            black_box(parse_quote_ticks(black_box(&table)));
        });
    });
}

fn bench_parse_ohlc_ticks_100(c: &mut Criterion) {
    let table = build_ohlc_data_table(100);
    c.bench_function("parse_ohlc_ticks_100", |b| {
        b.iter(|| {
            black_box(parse_ohlc_ticks(black_box(&table)));
        });
    });
}

criterion_group!(
    decode_benches,
    bench_decode_zstd_small,
    bench_decode_zstd_large,
    bench_decode_data_table_10_rows,
    bench_decode_data_table_1000_rows,
    bench_decode_extract_number_column,
    bench_decode_extract_price_column,
    bench_parse_trade_ticks_100,
    bench_parse_quote_ticks_100,
    bench_parse_ohlc_ticks_100,
);

criterion_main!(decode_benches);
