use criterion::{criterion_group, criterion_main, Criterion};
use std::hint::black_box;

use thetadatadx::StreamMsgType;

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

criterion_group!(enum_benches, bench_stream_msg_type_from_code_1000);

criterion_main!(enum_benches);
