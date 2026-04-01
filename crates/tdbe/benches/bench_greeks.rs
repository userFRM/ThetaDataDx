use criterion::{black_box, criterion_group, criterion_main, Criterion};

use tdbe::greeks;

// ═══════════════════════════════════════════════════════════════════════════
//  Greeks benchmarks
// ═══════════════════════════════════════════════════════════════════════════

fn bench_all_greeks(c: &mut Criterion) {
    c.bench_function("all_greeks", |b| {
        b.iter(|| {
            let s = black_box(150.0);
            let x = black_box(155.0);
            let r = black_box(0.05);
            let q = black_box(0.015);
            let t = black_box(45.0 / 365.0);
            let price = greeks::value(s, x, 0.22, r, q, t, true);
            black_box(greeks::all_greeks(s, x, r, q, t, price, true));
        });
    });
}

fn bench_all_greeks_individual(c: &mut Criterion) {
    c.bench_function("all_greeks_individual", |b| {
        b.iter(|| {
            let s = black_box(150.0);
            let x = black_box(155.0);
            let r = black_box(0.05);
            let q = black_box(0.015);
            let t = black_box(45.0 / 365.0);
            let v = 0.22;
            let val = greeks::value(s, x, v, r, q, t, true);
            let d = greeks::delta(s, x, v, r, q, t, true);
            let g = greeks::gamma(s, x, v, r, q, t);
            let th = greeks::theta(s, x, v, r, q, t, true);
            let ve = greeks::vega(s, x, v, r, q, t);
            let rh = greeks::rho(s, x, v, r, q, t, true);
            let ep = greeks::epsilon(s, x, v, r, q, t, true);
            let la = greeks::lambda(s, x, v, r, q, t, true);
            let va = greeks::vanna(s, x, v, r, q, t);
            let ch = greeks::charm(s, x, v, r, q, t, true);
            let vo = greeks::vomma(s, x, v, r, q, t);
            let vt = greeks::veta(s, x, v, r, q, t);
            let sp = greeks::speed(s, x, v, r, q, t);
            let zo = greeks::zomma(s, x, v, r, q, t);
            let co = greeks::color(s, x, v, r, q, t);
            let ul = greeks::ultima(s, x, v, r, q, t);
            let dd = greeks::dual_delta(s, x, v, r, q, t, true);
            let dg = greeks::dual_gamma(s, x, v, r, q, t);
            black_box((
                val, d, g, th, ve, rh, ep, la, va, ch, vo, vt, sp, zo, co, ul, dd, dg,
            ));
        });
    });
}

fn bench_greeks_delta_only(c: &mut Criterion) {
    c.bench_function("greeks_delta_only", |b| {
        b.iter(|| {
            black_box(greeks::delta(
                black_box(150.0),
                black_box(155.0),
                black_box(0.22),
                black_box(0.05),
                black_box(0.015),
                black_box(45.0 / 365.0),
                black_box(true),
            ));
        });
    });
}

fn bench_greeks_iv_solver(c: &mut Criterion) {
    // Pre-compute a realistic option price for the IV solver to find
    let option_price = greeks::value(150.0, 155.0, 0.22, 0.05, 0.015, 45.0 / 365.0, true);
    c.bench_function("greeks_iv_solver", |b| {
        b.iter(|| {
            black_box(greeks::implied_volatility(
                black_box(150.0),
                black_box(155.0),
                black_box(0.05),
                black_box(0.015),
                black_box(45.0 / 365.0),
                black_box(option_price),
                black_box(true),
            ));
        });
    });
}

fn bench_greeks_value(c: &mut Criterion) {
    c.bench_function("greeks_value", |b| {
        b.iter(|| {
            black_box(greeks::value(
                black_box(150.0),
                black_box(155.0),
                black_box(0.22),
                black_box(0.05),
                black_box(0.015),
                black_box(45.0 / 365.0),
                black_box(true),
            ));
        });
    });
}

criterion_group!(
    greeks_benches,
    bench_all_greeks,
    bench_all_greeks_individual,
    bench_greeks_delta_only,
    bench_greeks_iv_solver,
    bench_greeks_value,
);

criterion_main!(greeks_benches);
