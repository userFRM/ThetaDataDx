use criterion::{criterion_group, criterion_main, Criterion};
use std::hint::black_box;

use thetadatadx::auth::Credentials;

// ═══════════════════════════════════════════════════════════════════════════
//  Auth benchmarks
// ═══════════════════════════════════════════════════════════════════════════

fn bench_creds_parse(c: &mut Criterion) {
    let input = "Trader@Example.COM\ns3cret_p4ssw0rd!\n";
    c.bench_function("creds_parse", |b| {
        b.iter(|| {
            black_box(Credentials::parse(black_box(input)).unwrap());
        });
    });
}

fn bench_creds_new(c: &mut Criterion) {
    c.bench_function("creds_new", |b| {
        b.iter(|| {
            black_box(Credentials::new(
                black_box("Trader@Example.COM"),
                black_box("s3cret_p4ssw0rd!"),
            ));
        });
    });
}

criterion_group!(auth_benches, bench_creds_parse, bench_creds_new,);

criterion_main!(auth_benches);
