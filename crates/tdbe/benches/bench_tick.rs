use criterion::{criterion_group, criterion_main, Criterion};
use std::hint::black_box;

use tdbe::types::tick::{OhlcTick, QuoteTick, TradeTick};

// ═══════════════════════════════════════════════════════════════════════════
//  Tick operation benchmarks
//
//  Post-f64 migration (v6.0.0+): price fields are `f64`, decoded at parse time.
//  These benches measure direct field access and simple arithmetic rather than
//  the old helper-method decoding path.
// ═══════════════════════════════════════════════════════════════════════════

fn bench_trade_tick_price_access(c: &mut Criterion) {
    let tick = TradeTick {
        ms_of_day: 34_200_000,
        sequence: 1,
        ext_condition1: 0,
        ext_condition2: 0,
        ext_condition3: 0,
        ext_condition4: 0,
        condition: 0,
        size: 100,
        exchange: 4,
        price: 150.25,
        condition_flags: 0,
        price_flags: 0,
        volume_type: 0,
        records_back: 0,
        date: 20240315,
        expiration: 0,
        strike: 0.0,
        right: 0,
    };
    c.bench_function("trade_tick_price_access", |b| {
        b.iter(|| {
            black_box(black_box(&tick).price);
        });
    });
}

fn bench_quote_tick_midpoint(c: &mut Criterion) {
    let tick = QuoteTick {
        ms_of_day: 34_200_000,
        bid_size: 50,
        bid_exchange: 4,
        bid: 150.20,
        bid_condition: 1,
        ask_size: 30,
        ask_exchange: 4,
        ask: 150.30,
        ask_condition: 1,
        midpoint: 150.25,
        date: 20240315,
        expiration: 0,
        strike: 0.0,
        right: 0,
    };
    c.bench_function("quote_tick_midpoint", |b| {
        b.iter(|| {
            black_box(black_box(&tick).midpoint);
        });
    });
}

fn bench_ohlc_tick_all_prices(c: &mut Criterion) {
    let tick = OhlcTick {
        ms_of_day: 34_200_000,
        open: 150.00,
        high: 150.50,
        low: 149.70,
        close: 150.10,
        volume: 50_000,
        count: 250,
        date: 20240315,
        expiration: 0,
        strike: 0.0,
        right: 0,
    };
    c.bench_function("ohlc_tick_all_prices", |b| {
        b.iter(|| {
            let t = black_box(&tick);
            let o = t.open;
            let h = t.high;
            let l = t.low;
            let c = t.close;
            black_box((o, h, l, c));
        });
    });
}

criterion_group!(
    tick_benches,
    bench_trade_tick_price_access,
    bench_quote_tick_midpoint,
    bench_ohlc_tick_all_prices,
);

criterion_main!(tick_benches);
