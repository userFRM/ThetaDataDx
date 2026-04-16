use criterion::{criterion_group, criterion_main, Criterion};
use std::hint::black_box;

use thetadatadx::fpss::protocol::{build_credentials_payload, build_subscribe_payload, Contract};

// ═══════════════════════════════════════════════════════════════════════════
//  FPSS protocol benchmarks
// ═══════════════════════════════════════════════════════════════════════════

fn bench_contract_stock_to_bytes(c: &mut Criterion) {
    let contract = Contract::stock("AAPL");
    c.bench_function("contract_stock_to_bytes", |b| {
        b.iter(|| {
            black_box(black_box(&contract).to_bytes());
        });
    });
}

fn bench_contract_option_to_bytes(c: &mut Criterion) {
    let contract = Contract::option("SPY", "20261218", "60", "C").unwrap();
    c.bench_function("contract_option_to_bytes", |b| {
        b.iter(|| {
            black_box(black_box(&contract).to_bytes());
        });
    });
}

fn bench_contract_from_bytes(c: &mut Criterion) {
    let contract = Contract::option("SPY", "20261218", "60", "C").unwrap();
    let bytes = contract.to_bytes();
    c.bench_function("contract_from_bytes", |b| {
        b.iter(|| {
            black_box(Contract::from_bytes(black_box(&bytes)).unwrap());
        });
    });
}

fn bench_contract_roundtrip(c: &mut Criterion) {
    let contract = Contract::option("AAPL", "20261220", "17.5", "P").unwrap();
    c.bench_function("contract_roundtrip", |b| {
        b.iter(|| {
            let bytes = black_box(&contract).to_bytes();
            let (parsed, _) = Contract::from_bytes(&bytes).unwrap();
            black_box(parsed);
        });
    });
}

fn bench_build_credentials_payload(c: &mut Criterion) {
    c.bench_function("build_credentials_payload", |b| {
        b.iter(|| {
            black_box(build_credentials_payload(
                black_box("trader@example.com"),
                black_box("s3cret_p4ssw0rd!"),
            ));
        });
    });
}

fn bench_build_subscribe_payload(c: &mut Criterion) {
    let contract = Contract::option("SPY", "20261218", "60", "C").unwrap();
    c.bench_function("build_subscribe_payload", |b| {
        b.iter(|| {
            black_box(build_subscribe_payload(black_box(42), black_box(&contract)));
        });
    });
}

criterion_group!(
    protocol_benches,
    bench_contract_stock_to_bytes,
    bench_contract_option_to_bytes,
    bench_contract_from_bytes,
    bench_contract_roundtrip,
    bench_build_credentials_payload,
    bench_build_subscribe_payload,
);

criterion_main!(protocol_benches);
