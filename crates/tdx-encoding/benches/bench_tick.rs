use criterion::{black_box, criterion_group, criterion_main, Criterion};

use tdx_encoding::types::tick::{OhlcTick, QuoteTick, TradeTick};

// ═══════════════════════════════════════════════════════════════════════════
//  Tick operation benchmarks
// ═══════════════════════════════════════════════════════════════════════════

fn bench_trade_tick_get_price(c: &mut Criterion) {
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
        price: 15025,
        condition_flags: 0,
        price_flags: 0,
        volume_type: 0,
        records_back: 0,
        price_type: 8,
        date: 20240315,
    };
    c.bench_function("trade_tick_get_price", |b| {
        b.iter(|| {
            black_box(black_box(&tick).get_price());
        });
    });
}

fn bench_quote_tick_midpoint(c: &mut Criterion) {
    let tick = QuoteTick {
        ms_of_day: 34_200_000,
        bid_size: 50,
        bid_exchange: 4,
        bid: 15020,
        bid_condition: 1,
        ask_size: 30,
        ask_exchange: 4,
        ask: 15030,
        ask_condition: 1,
        price_type: 8,
        date: 20240315,
    };
    c.bench_function("quote_tick_midpoint", |b| {
        b.iter(|| {
            black_box(black_box(&tick).midpoint_price());
        });
    });
}

fn bench_ohlc_tick_all_prices(c: &mut Criterion) {
    let tick = OhlcTick {
        ms_of_day: 34_200_000,
        open: 15000,
        high: 15050,
        low: 14970,
        close: 15010,
        volume: 50_000,
        count: 250,
        price_type: 8,
        date: 20240315,
    };
    c.bench_function("ohlc_tick_all_prices", |b| {
        b.iter(|| {
            let t = black_box(&tick);
            let o = t.open_price();
            let h = t.high_price();
            let l = t.low_price();
            let c = t.close_price();
            black_box((o, h, l, c));
        });
    });
}

criterion_group!(
    tick_benches,
    bench_trade_tick_get_price,
    bench_quote_tick_midpoint,
    bench_ohlc_tick_all_prices,
);

criterion_main!(tick_benches);
