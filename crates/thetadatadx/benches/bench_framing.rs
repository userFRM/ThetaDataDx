use criterion::{black_box, criterion_group, criterion_main, Criterion};
use std::io::Cursor;

use tdbe::types::enums::StreamMsgType;
use thetadatadx::fpss::framing::{read_frame, write_frame, Frame};

// ═══════════════════════════════════════════════════════════════════════════
//  FPSS framing benchmarks
// ═══════════════════════════════════════════════════════════════════════════

fn bench_frame_write(c: &mut Criterion) {
    let payload: Vec<u8> = (0..50).map(|i| (i * 7 + 3) as u8).collect();
    let frame = Frame::new(StreamMsgType::Trade, payload);
    c.bench_function("frame_write", |b| {
        b.iter(|| {
            let mut buf = Vec::with_capacity(64);
            write_frame(&mut buf, black_box(&frame)).unwrap();
            black_box(&buf);
        });
    });
}

fn bench_frame_read(c: &mut Criterion) {
    let payload: Vec<u8> = (0..50).map(|i| (i * 7 + 3) as u8).collect();
    let frame = Frame::new(StreamMsgType::Trade, payload);
    let mut wire = Vec::new();
    write_frame(&mut wire, &frame).unwrap();
    c.bench_function("frame_read", |b| {
        b.iter(|| {
            let mut cursor = Cursor::new(black_box(&wire));
            black_box(read_frame(&mut cursor).unwrap());
        });
    });
}

fn bench_frame_roundtrip(c: &mut Criterion) {
    let payload: Vec<u8> = (0..50).map(|i| (i * 7 + 3) as u8).collect();
    let frame = Frame::new(StreamMsgType::Trade, payload);
    c.bench_function("frame_roundtrip", |b| {
        b.iter(|| {
            let mut buf = Vec::with_capacity(64);
            write_frame(&mut buf, black_box(&frame)).unwrap();
            let mut cursor = Cursor::new(&buf);
            black_box(read_frame(&mut cursor).unwrap());
        });
    });
}

criterion_group!(
    framing_benches,
    bench_frame_write,
    bench_frame_read,
    bench_frame_roundtrip,
);

criterion_main!(framing_benches);
