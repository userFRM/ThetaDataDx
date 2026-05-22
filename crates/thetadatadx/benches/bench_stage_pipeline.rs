//! Two-stage decode pipeline benchmarks (#584, Phase 2 of 3).
//!
//! Exercises [`thetadatadx::grpc::Stage2Pool`] — the shared
//! `prost::Message::decode` + `Tick`-build worker pool that runs
//! downstream of the per-channel zstd-decompress stage. Stage-1 hands
//! [`DecodedPayload`] handles (already-decompressed protobuf bytes)
//! through a bounded MPSC queue; stage-2 fans the prost decode out
//! across M worker threads so a single slow channel cannot saturate
//! decode capacity for the whole pool.
//!
//! # Scenarios
//!
//! * `pipeline_throughput/workers={1,2,4,8}` — sweep worker count at
//!   a fixed 1024-row quote payload. Queue depth scales with worker
//!   count (`workers * 64`) so the queue is not the bottleneck.
//!   Throughput in jobs/sec; should scale near-linearly until the
//!   host's available parallelism ceiling.
//!
//! * `pipeline_alloc_pressure/rows={256,1024,4096,16384}` — fix
//!   `(workers=4, queue=256)` and sweep payload row count. Throughput
//!   reported in ticks/sec so a constant ticks/sec across row counts
//!   confirms the prost decode scales linearly with payload size and
//!   the allocator is not the bottleneck at large row counts.
//!
//! * `pipeline_false_sharing/tiny_payloads` — saturate
//!   `(workers=8, queue=512)` with tiny (256-row) payloads. The
//!   [`Stage2Counters`] struct wraps each `AtomicU64` in
//!   [`crossbeam_utils::CachePadded`] so concurrent stage-1 and
//!   stage-2 increments on the three counters land on different
//!   cache lines. This scenario is a regression detector: if a future
//!   patch drops the `CachePadded` wrappers, the false-sharing stall
//!   on the hot counter increments will trip the baseline diff.
//!
//! * `pipeline_backpressure/parked_send` — feed a deliberately
//!   under-provisioned pool `(workers=2, queue=4)` at 4× the drain
//!   rate. The bench measures wall-clock for a burst large enough
//!   that the producer thread must park on a full queue many times,
//!   and asserts the `total_parked` counter advances. Pins the
//!   baseline park-rate observable so future regressions in
//!   stage-1's park-on-full path (a silent drop, a busy-spin, a lost
//!   counter increment) trip CI.
//!
//! # Why this matters
//!
//! Stage-2 is on the hot path for every gRPC response — its decode
//! latency is the per-response p50 the SDK reports to callers. The
//! four scenarios above lock in three invariants the production
//! pipeline relies on:
//!
//! 1. **Linear scaling in worker count** — stage-2 is embarrassingly
//!    parallel; a regression that introduced false-sharing or a
//!    shared lock would flatten the throughput curve.
//! 2. **Linear scaling in payload size** — prost decode is O(bytes);
//!    a regression in `Vec` growth strategy or the protobuf field
//!    iteration would inflate the per-byte cost at large payloads.
//! 3. **Bounded backpressure cost** — stage-1 parks instead of
//!    dropping; a regression that re-introduced drop-on-full would
//!    silently corrupt the market-data feed.
//!
//! # Note on baselines
//!
//! The companion `benches/baseline/bench_stage_pipeline.toml` is
//! informational only — the canonical baseline is
//! `benches/baseline/criterion.json`, which the
//! `scripts/check_bench_regression.py` CI gate consumes. This bench
//! is not yet in the gated set; the TOML carries the local-run
//! samples so a future opt-in (after a baseline is observed on the
//! GH-hosted runner) is a copy-paste away.

use std::hint::black_box;
use std::thread;
use std::time::Duration;

use bytes::Bytes;
use criterion::{
    criterion_group, criterion_main, AxisScale, BenchmarkId, Criterion, PlotConfiguration,
    Throughput,
};
use prost::Message;
use tokio::runtime::Builder as RuntimeBuilder;

use thetadatadx::grpc::{DecodedPayload, Stage2Pool};

#[path = "common/quote_fixture.rs"]
mod fixture;
use fixture::build_quote_data_table;

// ─── Payload fixture ────────────────────────────────────────────────

/// Encode a `DataTable` to the exact `Bytes` shape stage-2 expects.
///
/// Stage-1 hands stage-2 the **already-decompressed** protobuf
/// payload (see `stage_pipeline.rs::run_stage2_worker`): the worker
/// runs `prost::Message::decode::<DataTable>(payload.payload.as_ref())`
/// directly, with no zstd step. The bench therefore skips the zstd
/// encoder that `bench_decoder_pool` runs and feeds raw protobuf
/// bytes through `DecodedPayload`.
fn build_payload_bytes(rows: usize) -> Bytes {
    let table = build_quote_data_table(rows);
    Bytes::from(table.encode_to_vec())
}

/// Build a `DecodedPayload` with the given identity + payload bytes.
fn make_payload(channel_id: u64, request_id: u64, payload: Bytes) -> DecodedPayload {
    DecodedPayload {
        channel_id,
        request_id,
        payload,
    }
}

// ─── Bench bodies ───────────────────────────────────────────────────

/// Submit `count` payloads to the stage-2 pool through
/// `submit_for_bench`, then await every reply. The bench wraps this
/// in `Runtime::block_on` so criterion measures wall-clock for one
/// submit-and-drain pass.
async fn drive_pipeline(pool: &Stage2Pool, payload: &Bytes, count: usize) {
    let mut rxs = Vec::with_capacity(count);
    for idx in 0..count {
        // Clone the `Bytes` handle (refcount bump — no allocation,
        // no copy) so the bench measures the pipeline cost, not the
        // payload-construction cost.
        let rx = pool
            .submit_for_bench(make_payload(0, idx as u64, payload.clone()), usize::MAX)
            .expect("stage-2 pool accepts payload");
        rxs.push(rx);
    }
    for rx in rxs {
        let outcome = rx.await.expect("oneshot delivered");
        let table = outcome.expect("decoded");
        black_box(table.data_table.len());
    }
}

/// Scenario 1 — `pipeline_throughput`: sweep worker count at fixed
/// 1024-row quote payload. Queue depth = `workers * 64` so the
/// stage-2 pool drains every burst without parking the producer.
fn bench_pipeline_throughput(c: &mut Criterion) {
    let payload = build_payload_bytes(1024);

    let mut group = c.benchmark_group("pipeline_throughput");
    group.plot_config(PlotConfiguration::default().summary_scale(AxisScale::Linear));
    for &workers in &[1usize, 2, 4, 8] {
        // 128 jobs per iteration. Large enough that worker spin-up
        // amortizes; small enough that the iteration count stays in
        // criterion's preferred 100-sample regime on a fast box.
        let jobs_per_iter = 128u64;
        group.throughput(Throughput::Elements(jobs_per_iter));
        group.bench_with_input(
            BenchmarkId::from_parameter(workers),
            &workers,
            |b, &workers| {
                let pool = Stage2Pool::new(workers, workers * 64);
                let rt = RuntimeBuilder::new_current_thread()
                    .enable_time()
                    .build()
                    .expect("tokio current-thread runtime");
                b.iter(|| {
                    rt.block_on(drive_pipeline(
                        black_box(&pool),
                        &payload,
                        jobs_per_iter as usize,
                    ));
                });
            },
        );
    }
    group.finish();
}

/// Scenario 2 — `pipeline_alloc_pressure`: fix
/// `(workers=4, queue=256)` and sweep payload row count to confirm
/// linear scaling in payload size. Throughput in ticks/sec so the
/// constant-ticks-per-second invariant is read straight off the
/// summary plot.
fn bench_pipeline_alloc_pressure(c: &mut Criterion) {
    let mut group = c.benchmark_group("pipeline_alloc_pressure");
    group.plot_config(PlotConfiguration::default().summary_scale(AxisScale::Logarithmic));
    // Larger payloads take longer per job → fewer jobs per iter to
    // keep wall-clock per sample in criterion's comfort range
    // (~10ms-1s). The product `jobs_per_iter * row_count` stays
    // roughly constant so the reported ticks/sec is comparable
    // across the sweep without one row-count dominating runtime.
    for &(rows, jobs_per_iter) in &[(256usize, 256u64), (1024, 64), (4096, 16), (16384, 4)] {
        let payload = build_payload_bytes(rows);
        let ticks_per_iter = jobs_per_iter * rows as u64;
        group.throughput(Throughput::Elements(ticks_per_iter));
        group.bench_with_input(BenchmarkId::from_parameter(rows), &rows, |b, _rows| {
            let pool = Stage2Pool::new(4, 256);
            let rt = RuntimeBuilder::new_current_thread()
                .enable_time()
                .build()
                .expect("tokio current-thread runtime");
            b.iter(|| {
                rt.block_on(drive_pipeline(
                    black_box(&pool),
                    &payload,
                    jobs_per_iter as usize,
                ));
            });
        });
    }
    group.finish();
}

/// Scenario 3 — `pipeline_false_sharing`: saturate
/// `(workers=8, queue=512)` with tiny 256-row payloads so the
/// per-job decode cost is dominated by counter increments rather
/// than prost work. If a future patch drops the
/// [`crossbeam_utils::CachePadded`] wrappers on
/// [`thetadatadx::grpc::Stage2Counters`], the resulting false-sharing
/// stall on the three counter cache lines will widen the median
/// runtime beyond the baseline. Regression detector — there is no
/// "correct" absolute number, only "did it slow down vs the
/// last-blessed sample".
fn bench_pipeline_false_sharing(c: &mut Criterion) {
    let payload = build_payload_bytes(256);

    let mut group = c.benchmark_group("pipeline_false_sharing");
    // 512 jobs/iter = exactly one queue's worth of slots. Worker
    // count = 8 so all three counter cache lines are hammered from
    // multiple cores concurrently with the stage-1 producer side.
    let jobs_per_iter = 512u64;
    group.throughput(Throughput::Elements(jobs_per_iter));
    group.bench_function("tiny_payloads", |b| {
        let pool = Stage2Pool::new(8, 512);
        let rt = RuntimeBuilder::new_current_thread()
            .enable_time()
            .build()
            .expect("tokio current-thread runtime");
        b.iter(|| {
            rt.block_on(drive_pipeline(
                black_box(&pool),
                &payload,
                jobs_per_iter as usize,
            ));
        });
    });
    group.finish();
}

/// Scenario 4 — `pipeline_backpressure`: feed a deliberately
/// under-provisioned pool at far above its drain rate so stage-1
/// (the test producer) parks on a full queue many times per
/// iteration. The bench measures wall-clock for the burst and
/// asserts the `total_parked` counter advances at the end of every
/// iteration; if a regression silently dropped parked-jobs counting,
/// the assertion fails the bench fast rather than reporting a fake
/// "improvement". Pins the baseline park-rate observable so future
/// regressions in stage-1's park-on-full path trip CI.
fn bench_pipeline_backpressure(c: &mut Criterion) {
    let payload = build_payload_bytes(1024);

    let mut group = c.benchmark_group("pipeline_backpressure");
    // 64 jobs into a 4-slot queue served by 2 workers. The 16:1
    // job-to-slot ratio guarantees the producer parks repeatedly on
    // a full queue — exactly the backpressure regime the pipeline is
    // designed to absorb without dropping payloads.
    let jobs_per_iter = 64u64;
    group.throughput(Throughput::Elements(jobs_per_iter));
    group.bench_function("parked_send", |b| {
        let pool = Stage2Pool::new(2, 4);
        let counters = pool.counters();
        let rt = RuntimeBuilder::new_current_thread()
            .enable_time()
            .build()
            .expect("tokio current-thread runtime");
        b.iter(|| {
            let parked_before = counters.total_parked_nanos();
            rt.block_on(drive_pipeline(
                black_box(&pool),
                &payload,
                jobs_per_iter as usize,
            ));
            let parked_after = counters.total_parked_nanos();
            // The producer must have parked at least once during the
            // burst — the queue has 4 slots vs 64 jobs, served by 2
            // workers running real prost decodes. If this assertion
            // ever fails the pipeline's park-on-full path has
            // regressed (drop-on-full, counter not updated, lock-free
            // bug) and the bench should fail loudly.
            assert!(
                parked_after > parked_before,
                "stage-1 must park under 64-job burst into 4-slot queue \
                 (parked_before={parked_before}, parked_after={parked_after})"
            );
        });

        // Smoke test: a quiescent pause lets the workers drain any
        // residual queue entries before the bench tears the pool
        // down. Not load-bearing for the measurement, just hygiene.
        thread::sleep(Duration::from_millis(10));
    });
    group.finish();
}

criterion_group!(
    stage_pipeline_benches,
    bench_pipeline_throughput,
    bench_pipeline_alloc_pressure,
    bench_pipeline_false_sharing,
    bench_pipeline_backpressure,
);
criterion_main!(stage_pipeline_benches);
