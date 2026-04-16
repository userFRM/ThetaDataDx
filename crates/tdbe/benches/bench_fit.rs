use criterion::{criterion_group, criterion_main, Criterion};
use std::hint::black_box;

use tdbe::codec::fit::{apply_deltas, decode_fit_buffer_bulk, FitReader};

// ═══════════════════════════════════════════════════════════════════════════
//  Helpers
// ═══════════════════════════════════════════════════════════════════════════

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

// ═══════════════════════════════════════════════════════════════════════════
//  FIT codec benchmarks
// ═══════════════════════════════════════════════════════════════════════════

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

fn bench_fit_decode_single_row(c: &mut Criterion) {
    let buf = build_fit_buffer(1);
    c.bench_function("fit_decode_single_row", |b| {
        b.iter(|| {
            let mut reader = FitReader::new(black_box(&buf));
            let mut alloc = [0i32; 32];
            let n = reader.read_changes(&mut alloc);
            black_box((n, &alloc));
        });
    });
}

fn bench_fit_delta_decompression(c: &mut Criterion) {
    // Simulate a 16-field tick with realistic delta values
    let mut tick = [0i32; 16];
    let prev = [
        34_200_100, 42, 0, 0, 0, 0, 0, 100, 4, 15025, 0, 0, 0, 0, 8, 20240315,
    ];
    // deltas: small increments typical of consecutive ticks
    let deltas = [100, 1, 0, 0, 0, 0, 0, -5, 0, 3, 0, 0, 0, 0, 0, 0];
    c.bench_function("fit_delta_decompression", |b| {
        b.iter(|| {
            tick.copy_from_slice(&deltas);
            apply_deltas(black_box(&mut tick), black_box(&prev), 16);
            black_box(&tick);
        });
    });
}

fn bench_fit_bulk_1000_rows(c: &mut Criterion) {
    let buf = build_fit_buffer(1000);
    c.bench_function("fit_bulk_1000_rows", |b| {
        b.iter(|| {
            let rows = decode_fit_buffer_bulk(black_box(&buf), 16);
            black_box(rows);
        });
    });
}

criterion_group!(
    fit_benches,
    bench_fit_decode_100_rows,
    bench_fit_decode_1000_rows_scalar,
    bench_fit_decode_single_row,
    bench_fit_delta_decompression,
    bench_fit_bulk_1000_rows,
);

criterion_main!(fit_benches);
