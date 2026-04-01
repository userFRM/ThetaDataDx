use criterion::{black_box, criterion_group, criterion_main, Criterion};

use tdbe::types::enums::{DataType, StreamMsgType};

// ═══════════════════════════════════════════════════════════════════════════
//  Enum lookup benchmarks
// ═══════════════════════════════════════════════════════════════════════════

fn bench_stream_msg_type_from_code_1000(c: &mut Criterion) {
    // Realistic mix: valid codes hit most of the time, occasional miss
    let codes: Vec<u8> = (0..1000)
        .map(|i| match i % 20 {
            0 => 0,   // Credentials
            1 => 1,   // SessionToken
            2 => 10,  // Ping
            3 => 11,  // Error
            4 => 12,  // Disconnected
            5 => 20,  // Contract
            6 => 21,  // Quote
            7 => 22,  // Trade
            8 => 23,  // OpenInterest
            9 => 24,  // Ohlcvc
            10 => 30, // Start
            11 => 31, // Restart
            12 => 32, // Stop
            13 => 40, // ReqResponse
            14 => 51, // RemoveQuote
            15 => 52, // RemoveTrade
            16 => 53, // RemoveOpenInterest
            17 => 4,  // Connected
            18 => 13, // Reconnected
            _ => 255, // Unknown (miss)
        })
        .collect();
    c.bench_function("stream_msg_type_from_code_1000", |b| {
        b.iter(|| {
            let mut hits = 0u32;
            for &code in &codes {
                if StreamMsgType::from_code(black_box(code)).is_some() {
                    hits += 1;
                }
            }
            black_box(hits);
        });
    });
}

fn bench_data_type_from_code_1000(c: &mut Criterion) {
    // Mix of valid DataType codes
    let codes: Vec<i32> = (0..1000)
        .map(|i| match i % 20 {
            0 => 0,    // Date
            1 => 1,    // MsOfDay
            2 => 101,  // BidSize
            3 => 103,  // Bid
            4 => 107,  // Ask
            5 => 131,  // Sequence
            6 => 132,  // Size
            7 => 134,  // Price
            8 => 141,  // Volume
            9 => 151,  // Theta
            10 => 153, // Delta
            11 => 161, // Gamma
            12 => 191, // Open
            13 => 192, // High
            14 => 193, // Low
            15 => 194, // Close
            16 => 201, // ImpliedVol
            17 => 204, // UnderlyingPrice
            18 => 261, // OutstandingShares
            _ => 999,  // Unknown (miss)
        })
        .collect();
    c.bench_function("data_type_from_code_1000", |b| {
        b.iter(|| {
            let mut hits = 0u32;
            for &code in &codes {
                if DataType::from_code(black_box(code)).is_some() {
                    hits += 1;
                }
            }
            black_box(hits);
        });
    });
}

criterion_group!(
    enum_benches,
    bench_stream_msg_type_from_code_1000,
    bench_data_type_from_code_1000,
);

criterion_main!(enum_benches);
