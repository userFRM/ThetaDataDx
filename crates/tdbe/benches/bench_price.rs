use criterion::{criterion_group, criterion_main, Criterion};
use std::hint::black_box;

use tdbe::types::price::Price;

// ═══════════════════════════════════════════════════════════════════════════
//  Price benchmarks
// ═══════════════════════════════════════════════════════════════════════════

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

fn bench_price_compare_same_type_1000(c: &mut Criterion) {
    let prices_a: Vec<Price> = (0..1000).map(|i| Price::new(15000 + i, 8)).collect();
    let prices_b: Vec<Price> = (0..1000).map(|i| Price::new(15100 + i, 8)).collect();
    c.bench_function("price_compare_same_type_1000", |b| {
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

fn bench_price_display_1000(c: &mut Criterion) {
    let prices: Vec<Price> = (0..1000).map(|i| Price::new(15025 + i, 8)).collect();
    c.bench_function("price_display_1000", |b| {
        b.iter(|| {
            let mut total_len = 0usize;
            for p in &prices {
                let s = p.to_string();
                total_len += s.len();
            }
            black_box(total_len);
        });
    });
}

fn bench_price_new_1000(c: &mut Criterion) {
    c.bench_function("price_new_1000", |b| {
        b.iter(|| {
            let mut sum = 0i32;
            for i in 0..1000i32 {
                let p = Price::new(black_box(15000 + i), black_box(8));
                sum += p.value;
            }
            black_box(sum);
        });
    });
}

criterion_group!(
    price_benches,
    bench_price_to_f64_1000,
    bench_price_compare_1000,
    bench_price_compare_same_type_1000,
    bench_price_display_1000,
    bench_price_new_1000,
);

criterion_main!(price_benches);
