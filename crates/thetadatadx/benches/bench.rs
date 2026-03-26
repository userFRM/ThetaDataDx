use criterion::{black_box, criterion_group, criterion_main, Criterion};

use thetadatadx::codec::decode_fit_buffer_bulk;
use thetadatadx::codec::fie::{string_to_fie_line, try_string_to_fie_line};
use thetadatadx::codec::fit::{apply_deltas, FitReader};
use thetadatadx::greeks;
use thetadatadx::types::price::Price;

// ── Helpers ──────────────────────────────────────────────────────────

/// Pack two 4-bit nibbles into a byte.
fn pack(high: u8, low: u8) -> u8 {
    (high << 4) | (low & 0x0F)
}

const FIELD_SEP: u8 = 0xB;
const ROW_SEP: u8 = 0xC;
const END: u8 = 0xD;

/// Build a FIT buffer containing `n_rows` of realistic trade-tick-shaped data.
fn build_fit_buffer(n_rows: usize) -> Vec<u8> {
    let mut buf = Vec::with_capacity(n_rows * 20);
    for i in 0..n_rows {
        // Encode a simplified trade tick row:
        // ms_of_day(34200+i) COMMA seq(i) SLASH size(100) COMMA exchange(4)
        // COMMA price(15025) COMMA COMMA COMMA COMMA pt(1) COMMA date(20240315) END
        //
        // We'll use small integers to keep the encoding compact.
        // "1,2/3,4,5,,,,1,6\n"
        let d0 = ((34200 + i) % 10) as u8;
        let d1 = (((34200 + i) / 10) % 10) as u8;
        buf.push(pack(d1, d0));
        buf.push(pack(FIELD_SEP, (i % 10) as u8));
        buf.push(pack(ROW_SEP, 1));
        buf.push(pack(0, 0));
        buf.push(pack(FIELD_SEP, 4));
        buf.push(pack(FIELD_SEP, 1));
        buf.push(pack(5, 0));
        buf.push(pack(2, 5));
        buf.push(pack(FIELD_SEP, FIELD_SEP));
        buf.push(pack(FIELD_SEP, FIELD_SEP));
        buf.push(pack(FIELD_SEP, 1));
        buf.push(pack(FIELD_SEP, 2));
        buf.push(pack(0, 2));
        buf.push(pack(4, 0));
        buf.push(pack(3, 1));
        buf.push(pack(5, END));
    }
    buf
}

// ── Benchmarks ───────────────────────────────────────────────────────

fn bench_fit_decode_100_rows(c: &mut Criterion) {
    let buf = build_fit_buffer(100);

    c.bench_function("fit_decode_100_rows", |b| {
        b.iter(|| {
            let mut reader = FitReader::new(black_box(&buf));
            let mut alloc = [0i32; 32];
            let mut prev = [0i32; 32];
            let mut first = true;
            while !reader.is_exhausted() {
                let n = reader.read_changes(&mut alloc);
                if n == 0 {
                    continue;
                }
                if first {
                    prev.copy_from_slice(&alloc);
                    first = false;
                } else {
                    apply_deltas(&mut alloc, &prev, n);
                    prev.copy_from_slice(&alloc);
                }
            }
            black_box(&prev);
        });
    });
}

fn bench_fit_decode_1000_rows_scalar(c: &mut Criterion) {
    let buf = build_fit_buffer(1000);

    c.bench_function("fit_decode_1000_rows_scalar", |b| {
        b.iter(|| {
            let mut reader = FitReader::new(black_box(&buf));
            let mut alloc = [0i32; 32];
            let mut prev = [0i32; 32];
            let mut first = true;
            while !reader.is_exhausted() {
                let n = reader.read_changes(&mut alloc);
                if n == 0 {
                    continue;
                }
                if first {
                    prev.copy_from_slice(&alloc);
                    first = false;
                } else {
                    apply_deltas(&mut alloc, &prev, n);
                    prev.copy_from_slice(&alloc);
                }
            }
            black_box(&prev);
        });
    });
}

fn bench_fit_decode_1000_rows_simd(c: &mut Criterion) {
    let buf = build_fit_buffer(1000);

    c.bench_function("fit_decode_1000_rows_simd_bulk", |b| {
        b.iter(|| {
            let rows = decode_fit_buffer_bulk(black_box(&buf), 32);
            black_box(&rows);
        });
    });
}

fn bench_price_to_f64_1000(c: &mut Criterion) {
    let prices: Vec<Price> = (0..1000).map(|i| Price::new(15025 + i, 8)).collect();

    c.bench_function("price_to_f64_1000", |b| {
        b.iter(|| {
            let mut sum = 0.0f64;
            for p in &prices {
                sum += p.to_f64();
            }
            black_box(sum);
        });
    });
}

fn bench_price_compare_1000(c: &mut Criterion) {
    let prices_a: Vec<Price> = (0..1000).map(|i| Price::new(15000 + i, 8)).collect();
    let prices_b: Vec<Price> = (0..1000)
        .map(|i| Price::new(1500000 + i * 100, 6))
        .collect();

    c.bench_function("price_compare_1000", |b| {
        b.iter(|| {
            let mut count = 0u32;
            for (a, bp) in prices_a.iter().zip(prices_b.iter()) {
                if a > bp {
                    count += 1;
                }
            }
            black_box(count);
        });
    });
}

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
            // Call each Greek individually (no shared intermediates)
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

fn bench_fie_encode(c: &mut Criterion) {
    let input = "21,0,1,0,20240315,0,15000";

    c.bench_function("fie_encode", |b| {
        b.iter(|| {
            black_box(string_to_fie_line(black_box(input)));
        });
    });
}

fn bench_fie_try_encode(c: &mut Criterion) {
    let input = "21,0,1,0,20240315,0,15000";

    c.bench_function("fie_try_encode", |b| {
        b.iter(|| {
            black_box(try_string_to_fie_line(black_box(input)));
        });
    });
}

criterion_group!(
    benches,
    bench_fit_decode_100_rows,
    bench_fit_decode_1000_rows_scalar,
    bench_fit_decode_1000_rows_simd,
    bench_price_to_f64_1000,
    bench_price_compare_1000,
    bench_all_greeks,
    bench_all_greeks_individual,
    bench_fie_encode,
    bench_fie_try_encode,
);
criterion_main!(benches);
