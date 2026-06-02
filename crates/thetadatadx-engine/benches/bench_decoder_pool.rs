//! Decoder-pool sizing benchmarks (issue #584).
//!
//! Measures end-to-end decode throughput across the MDDS decoder
//! pool — zstd decompress + protobuf `DataTable` decode running on
//! the dedicated `std::thread` workers — at varying thread counts
//! and ring depths.
//!
//! The fixture builds a synthetic `ResponseData` carrying a
//! quote-tick `DataTable` of representative density (1024 rows, the
//! per-chunk shape MDDS emits at `interval=1s` strike-range bursts),
//! then submits `N` such responses through the pool concurrently and
//! waits for every reply oneshot. The result line is responses/sec —
//! divide by 1024 for ticks/sec.
//!
//! # Variants
//!
//! - `decoder_pool/threads={1,2,4,8}` — fixed ring size = `256`
//!   (production default), thread count swept. Larger thread counts
//!   should scale near-linearly until they hit the host's available
//!   parallelism ceiling.
//! - `decoder_pool/ring={64,256,1024,4096}` — fixed threads = `4`,
//!   ring depth swept. Larger rings absorb burstier producer cadence
//!   without back-pressuring `try_publish` (which back-offs in
//!   50 µs windows when full); the read-out is whether deeper rings
//!   reduce the inter-burst tail latency.
//!
//! Both variants run a `2 × ring` request load so the ring fills at
//! least once per iteration — that exercises the back-off path the
//! deeper ring is supposed to mitigate.

use std::hint::black_box;
use std::io::Write;
use std::sync::Arc;

use criterion::{
    criterion_group, criterion_main, AxisScale, BenchmarkId, Criterion, PlotConfiguration,
    Throughput,
};
use prost::Message;
use tokio::runtime::Builder as RuntimeBuilder;

use thetadatadx_engine::wire::{CompressionAlgo, CompressionDescription, DataTable, ResponseData};

// Public API: the pool itself.
use thetadatadx_engine::grpc::DecoderPool;

#[path = "common/quote_fixture.rs"]
mod fixture;
use fixture::build_quote_data_table;

// ─── Fixture builder ────────────────────────────────────────────────

/// Compress a `DataTable` into a zstd-wrapped `ResponseData`.
fn build_zstd_response(table: &DataTable) -> ResponseData {
    let inner = table.encode_to_vec();
    let original_size = i32::try_from(inner.len()).unwrap_or(i32::MAX);
    let mut encoder = zstd::stream::Encoder::new(Vec::new(), 3).expect("zstd encoder");
    encoder.write_all(&inner).expect("zstd write");
    let compressed = encoder.finish().expect("zstd finalize");
    ResponseData {
        compressed_data: compressed,
        compression_description: Some(CompressionDescription {
            algo: i32::from(CompressionAlgo::Zstd),
            level: 3,
        }),
        original_size,
    }
}

// ─── Bench body ─────────────────────────────────────────────────────

/// Submit `count` responses across `pool.len()` decoder handles
/// round-robin, then await every reply. Returns once every decoded
/// `DataTable` has been delivered. The bench wraps this in
/// `Runtime::block_on` so criterion measures the wall-clock of one
/// full submit-and-drain pass.
async fn drive_pool(pool: &DecoderPool, response: Arc<ResponseData>, count: usize) {
    let handles_len = pool.len();
    let mut rxs = Vec::with_capacity(count);
    for idx in 0..count {
        let handle = pool.handle(idx % handles_len);
        // Clone the captured `Arc<ResponseData>` cheaply rather than
        // re-encoding per iteration — the bench is measuring the
        // decode pipeline, not the encoder.
        let rx = handle
            .submit((*response).clone(), usize::MAX)
            .expect("pool not poisoned");
        rxs.push(rx);
    }
    for rx in rxs {
        let outcome = rx.await.expect("oneshot delivered");
        let table = outcome.expect("decoded");
        black_box(table.data_table.len());
    }
}

fn bench_decoder_thread_count(c: &mut Criterion) {
    let table = build_quote_data_table(1024);
    let response = Arc::new(build_zstd_response(&table));
    let row_count = 1024u64;

    let mut group = c.benchmark_group("decoder_pool/threads");
    group.plot_config(PlotConfiguration::default().summary_scale(AxisScale::Linear));
    // Throughput in ticks/sec — `responses_per_iter * rows_per_response`.
    // The criterion runner divides by elapsed time automatically.
    for &threads in &[1usize, 2, 4, 8] {
        let responses_per_iter = 64u64;
        group.throughput(Throughput::Elements(responses_per_iter * row_count));
        group.bench_with_input(
            BenchmarkId::from_parameter(threads),
            &threads,
            |b, &threads| {
                let pool = DecoderPool::new(threads, 256).expect("pool");
                let rt = RuntimeBuilder::new_multi_thread()
                    .worker_threads(4)
                    .enable_time()
                    .build()
                    .expect("runtime");
                b.iter(|| {
                    rt.block_on(drive_pool(
                        black_box(&pool),
                        Arc::clone(&response),
                        responses_per_iter as usize,
                    ));
                });
            },
        );
    }
    group.finish();
}

fn bench_decoder_ring_depth(c: &mut Criterion) {
    let table = build_quote_data_table(1024);
    let response = Arc::new(build_zstd_response(&table));
    let row_count = 1024u64;

    let mut group = c.benchmark_group("decoder_pool/ring");
    group.plot_config(PlotConfiguration::default().summary_scale(AxisScale::Logarithmic));
    for &ring in &[64usize, 256, 1024, 4096] {
        // Load = 2 × ring so the producer hits a full ring at least
        // once per iteration. Without that the back-off path is
        // never exercised and the bench reports raw decode latency
        // rather than the absorb-burst characteristic the ring
        // depth is supposed to improve.
        let responses_per_iter = (ring as u64) * 2;
        group.throughput(Throughput::Elements(responses_per_iter * row_count));
        group.bench_with_input(BenchmarkId::from_parameter(ring), &ring, |b, &ring| {
            let pool = DecoderPool::new(4, ring).expect("pool");
            let rt = RuntimeBuilder::new_multi_thread()
                .worker_threads(4)
                .enable_time()
                .build()
                .expect("runtime");
            b.iter(|| {
                rt.block_on(drive_pool(
                    black_box(&pool),
                    Arc::clone(&response),
                    responses_per_iter as usize,
                ));
            });
        });
    }
    group.finish();
}

criterion_group!(
    decoder_pool_benches,
    bench_decoder_thread_count,
    bench_decoder_ring_depth,
);
criterion_main!(decoder_pool_benches);
