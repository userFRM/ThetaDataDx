use criterion::{black_box, criterion_group, criterion_main, Criterion};

use tdx_encoding::codec::fie::{fie_line_to_string, string_to_fie_line, try_string_to_fie_line};

// ═══════════════════════════════════════════════════════════════════════════
//  FIE encoder benchmarks
// ═══════════════════════════════════════════════════════════════════════════

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

fn bench_fie_encode_long(c: &mut Criterion) {
    // 50-char string: typical complex subscription line
    let input = "21,0,1,0,20240315,0,15000,20240920,1,60000,0,0,0";
    c.bench_function("fie_encode_long", |b| {
        b.iter(|| {
            black_box(string_to_fie_line(black_box(input)));
        });
    });
}

fn bench_fie_decode(c: &mut Criterion) {
    let input = "21,0,1,0,20240315,0,15000";
    let encoded = string_to_fie_line(input);
    c.bench_function("fie_decode", |b| {
        b.iter(|| {
            black_box(fie_line_to_string(black_box(&encoded)));
        });
    });
}

criterion_group!(
    fie_benches,
    bench_fie_encode,
    bench_fie_try_encode,
    bench_fie_encode_long,
    bench_fie_decode,
);

criterion_main!(fie_benches);
