//! REST CSV decode hot-path bench.
//!
//! Baseline harness for the M2 (column_index hoisting on the Greeks
//! first-order decoder) and M3 (per-base-url RestClient cache)
//! optimizations. Bench delta is the metric of record in the PR body.
//!
//! `decode_quote_csv` already hoists every `column_index` call above
//! the row loop; it is included here as a parity baseline -- the
//! ratio between the two decoders calibrates how much overhead the
//! pre-M2 inline `column_index` calls were adding on the
//! `_greeks_first_order` path.

use std::hint::black_box;

use criterion::{criterion_group, criterion_main, BatchSize, Criterion};
use thetadatadx::rest::client::{decode_greeks_first_order_csv, decode_quote_csv};

/// Build a 6-column QuoteTick CSV with `n` rows. Exercises the
/// subset NBBO row layout that the lenient REST decoder accepts.
fn build_quote_csv(n: usize) -> String {
    let mut s = String::with_capacity(64 + n * 48);
    s.push_str("ms_of_day,bid_size,bid,ask_size,ask,date\n");
    for i in 0..n {
        // Vary the ms_of_day so the parser sees realistic distinct
        // values rather than a hot constant.
        let ms = 34_200_000 + i as i32 * 500;
        s.push_str(&format!("{ms},50,1.5022,75,1.5041,20220414\n"));
    }
    s
}

/// Build a Greeks first-order CSV with `n` rows + 13 columns. The
/// pre-M2 hot path resolves each column index per row via
/// `column_index(...)`; this fixture drives that work.
fn build_greeks_csv(n: usize) -> String {
    let mut s = String::with_capacity(160 + n * 96);
    s.push_str(
        "ms_of_day,bid,ask,delta,theta,vega,rho,epsilon,lambda,\
         implied_volatility,iv_error,underlying_ms_of_day,underlying_price,date\n",
    );
    for i in 0..n {
        let ms = 34_200_000 + i as i32 * 500;
        s.push_str(&format!(
            "{ms},1.50,1.51,0.42,-0.07,0.11,0.04,0.0,0.0,0.21,0.0001,{ms},423.41,20240605\n"
        ));
    }
    s
}

fn bench_decode_quote(c: &mut Criterion) {
    let mut group = c.benchmark_group("rest_decode_quote_csv");
    for n in [128_usize, 1024, 4096] {
        let body = build_quote_csv(n);
        group.bench_function(format!("rows={n}"), |b| {
            b.iter_batched(
                || body.clone(),
                |s| {
                    let out = decode_quote_csv(black_box(&s)).expect("decode");
                    black_box(out)
                },
                BatchSize::SmallInput,
            );
        });
    }
    group.finish();
}

fn bench_decode_greeks_first_order(c: &mut Criterion) {
    let mut group = c.benchmark_group("rest_decode_greeks_first_order_csv");
    for n in [128_usize, 1024, 4096] {
        let body = build_greeks_csv(n);
        group.bench_function(format!("rows={n}"), |b| {
            b.iter_batched(
                || body.clone(),
                |s| {
                    let out = decode_greeks_first_order_csv(black_box(&s)).expect("decode");
                    black_box(out)
                },
                BatchSize::SmallInput,
            );
        });
    }
    group.finish();
}

criterion_group!(benches, bench_decode_quote, bench_decode_greeks_first_order);
criterion_main!(benches);
