//! Closed-loop measurement of the MDDS gRPC transport over a loopback
//! mock h2 server — the regression pin for the transport.
//!
//! The client issues `GetStockHistoryEod`-shaped RPCs through the
//! production transport surface (`Channel` / `ChannelPool` +
//! `bench_support::stock_history_eod`, exactly the dispatch + merge
//! shape `MddsClient` wires), sends the same prost-encoded request the
//! SDK sends, receives zstd-compressed `ResponseData` frames, and
//! performs the production decode work (zstd decompress + prost
//! `DataTable` decode + row merge + typed `EodTick` build) inline on
//! the request task.
//!
//! This harness is the baseline the transport is pinned against: run it
//! before and after a transport-touching change and compare the
//! reported throughput / latency / allocation tables.
//!
//! Topology controls:
//!
//! - `min(concurrency, 16)` TCP connections: one per worker at
//!   production-reachable levels (mirrors the pool shape where
//!   `pool_size == semaphore size`), capped at 16 with workers
//!   multiplexed across connections at the synthetic headroom levels
//!   (100/1000).
//! - h2 flow-control windows ride the channel default (the HTTP/2
//!   spec 64 KiB initial windows — the production config default).
//! - The mock pre-frames each response chunk once and clones
//!   refcounted `Bytes` per request, so server-side cost is constant.
//!
//! All traffic stays on 127.0.0.1. The harness never dials a production
//! host, never performs the Nexus auth handshake, and never reads a
//! credentials file.
//!
//! Run the full matrix (concurrency 1/2/4/8/16 + synthetic 100/1000,
//! ~1 KB and ~10 MB frames plus the 16-chunk fan-in shape):
//!
//! ```text
//! cargo bench -p thetadatadx --features __test-helpers \
//!     --bench grpc_transport_comparison
//! ```
//!
//! Environment knobs:
//!
//! - `THETADATADX_BENCH_QUICK=1` — one repeat, short windows, levels
//!   1/8, small frames only (harness smoke run).
//! - `THETADATADX_BENCH_LEVELS=1,2,4` — override the concurrency sweep.
//! - `THETADATADX_BENCH_SIZES=small,large,multi` — override the frame
//!   shapes. `multi` streams 16 chunks of ~640 KiB per RPC on one
//!   stream — the fan-in shape where many response chunks land on one
//!   decode path.
//! - `THETADATADX_BENCH_REPEATS=3` — repeats per cell.
//! - `THETADATADX_BENCH_CONNS=1` — pin the TCP connection count per
//!   side instead of `min(concurrency, 16)`. `1` measures a single
//!   multiplexed connection carrying every worker; unset keeps the
//!   production pool shape.

use std::alloc::{GlobalAlloc, Layout, System};
use std::io::Write as _;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use bytes::{BufMut, Bytes, BytesMut};
use http::{HeaderMap, HeaderName, HeaderValue, Response, StatusCode};
use prost::Message;
use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};
use tokio::net::{TcpListener, TcpStream};
use tokio::runtime::Runtime;
use tokio::sync::oneshot;
use tokio::task::JoinHandle;

use thetadatadx::grpc::endpoints::bench_support;
use thetadatadx::grpc::{Channel, ChannelPool};
use thetadatadx::wire::{
    data_value, CompressionAlgo, CompressionDescription, DataTable, DataValue, DataValueList,
    ResponseData,
};

// ─── Counting allocator ─────────────────────────────────────────────
//
// Wrap the system allocator and tally bytes allocated / deallocated so
// each measured window can report bytes-allocated-per-request. The mock
// server runs in-process, so the absolute number includes the server
// side of every RPC — comparable across runs of this harness, which
// all pay the identical server cost.

struct CountingAllocator;

static BYTES_ALLOCATED: AtomicU64 = AtomicU64::new(0);
static BYTES_DEALLOCATED: AtomicU64 = AtomicU64::new(0);

// SAFETY: every method forwards verbatim to `std::alloc::System`, which
// itself satisfies the `GlobalAlloc` contract. Per-call `Relaxed` adds on
// `AtomicU64` are pure observational state and cannot violate the
// allocator's invariants. Bench-only; never linked into the shipped
// library.
unsafe impl GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        // SAFETY: GlobalAlloc::alloc precondition is `layout.size() > 0`
        // and `layout.align()` is a non-zero power of two — the alloc
        // shim Rust generates for any `#[global_allocator]` enforces
        // both before this call. `System.alloc` is the System impl
        // upstream; forwarding satisfies it verbatim.
        let ptr = unsafe { System.alloc(layout) };
        if !ptr.is_null() {
            BYTES_ALLOCATED.fetch_add(layout.size() as u64, Ordering::Relaxed);
        }
        ptr
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        BYTES_DEALLOCATED.fetch_add(layout.size() as u64, Ordering::Relaxed);
        // SAFETY: GlobalAlloc::dealloc precondition — `ptr` was
        // returned by a prior `alloc` on this allocator with the same
        // `layout`, and has not been deallocated. The shim Rust
        // generates from `Vec`, `Box`, etc. upholds that pairing;
        // forwarding to `System.dealloc` satisfies the System impl.
        unsafe { System.dealloc(ptr, layout) }
    }
}

#[global_allocator]
static ALLOC: CountingAllocator = CountingAllocator;

fn alloc_snapshot() -> (u64, u64) {
    (
        BYTES_ALLOCATED.load(Ordering::Relaxed),
        BYTES_DEALLOCATED.load(Ordering::Relaxed),
    )
}

// ─── CPU-time snapshot ──────────────────────────────────────────────

/// Process CPU time (user + system) in microseconds via
/// `getrusage(RUSAGE_SELF)`. Includes the in-process mock server and
/// every runtime thread — identical accounting across runs, so the
/// per-request delta is comparable.
#[cfg(unix)]
fn process_cpu_micros() -> u64 {
    // SAFETY: `getrusage` writes a fully-initialised `rusage` into the
    // zeroed out-param when given the valid `RUSAGE_SELF` selector and
    // a properly aligned struct; both are satisfied here, and the
    // struct is read only after the call reports success.
    unsafe {
        let mut usage: libc::rusage = std::mem::zeroed();
        if libc::getrusage(libc::RUSAGE_SELF, &raw mut usage) != 0 {
            return 0;
        }
        let user = u64::try_from(usage.ru_utime.tv_sec).unwrap_or(0) * 1_000_000
            + u64::try_from(usage.ru_utime.tv_usec).unwrap_or(0);
        let sys = u64::try_from(usage.ru_stime.tv_sec).unwrap_or(0) * 1_000_000
            + u64::try_from(usage.ru_stime.tv_usec).unwrap_or(0);
        user + sys
    }
}

#[cfg(not(unix))]
fn process_cpu_micros() -> u64 {
    0
}

// ─── Mock h2 server ─────────────────────────────────────────────────
//
// One listener, one task per accepted connection, one task per
// multiplexed request. The
// response frame is pre-encoded once; each request clones the
// refcounted `Bytes`, so per-request server cost is the h2 send only.

#[derive(Clone)]
struct ServerConfig {
    /// Pre-framed gRPC messages (5-byte length prefix + encoded
    /// `ResponseData` each), sent in order on every response stream.
    /// Single-frame payloads carry one entry; the multi-chunk shape
    /// carries one entry per chunk. Cloning is a refcount bump per
    /// entry.
    framed: Vec<Bytes>,
}

struct MockServer {
    addr: SocketAddr,
    shutdown: Option<oneshot::Sender<()>>,
    task: Option<JoinHandle<()>>,
}

impl Drop for MockServer {
    fn drop(&mut self) {
        if let Some(tx) = self.shutdown.take() {
            let _ = tx.send(());
        }
        if let Some(task) = self.task.take() {
            task.abort();
        }
    }
}

async fn spawn_mock(rt: &Runtime, config: ServerConfig) -> MockServer {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind ephemeral");
    let addr = listener.local_addr().expect("local addr");
    let (tx, mut rx) = oneshot::channel();
    let task = rt.spawn(async move {
        loop {
            tokio::select! {
                biased;
                _ = &mut rx => return,
                accept = listener.accept() => {
                    if let Ok((socket, _)) = accept {
                        let cfg = config.clone();
                        tokio::spawn(async move {
                            let _ = serve_connection(socket, cfg).await;
                        });
                    }
                }
            }
        }
    });
    MockServer {
        addr,
        shutdown: Some(tx),
        task: Some(task),
    }
}

async fn serve_connection(
    socket: TcpStream,
    config: ServerConfig,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let _ = socket.set_nodelay(true);
    let mut connection = h2::server::handshake(socket).await?;
    while let Some(request_result) = connection.accept().await {
        let (request, respond) = request_result?;
        let framed = config.framed.clone();
        tokio::spawn(async move {
            let _ = handle_request(request, respond, framed).await;
        });
    }
    Ok(())
}

async fn handle_request(
    request: http::Request<h2::RecvStream>,
    mut respond: h2::server::SendResponse<Bytes>,
    framed: Vec<Bytes>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let mut body = request.into_body();
    while let Some(chunk) = body.data().await {
        let chunk = chunk?;
        let _ = body.flow_control().release_capacity(chunk.len());
    }
    let mut response = Response::new(());
    *response.status_mut() = StatusCode::OK;
    response.headers_mut().insert(
        http::header::CONTENT_TYPE,
        HeaderValue::from_static("application/grpc+proto"),
    );
    let mut send_stream = respond.send_response(response, false)?;
    for frame in framed {
        send_stream.send_data(frame, false)?;
    }
    let mut trailers = HeaderMap::new();
    trailers.insert(
        HeaderName::from_static("grpc-status"),
        HeaderValue::from_static("0"),
    );
    send_stream.send_trailers(trailers)?;
    Ok(())
}

// ─── Payload synthesis ──────────────────────────────────────────────

/// A synthesized response — one or more chunks — plus the facts the
/// report needs. Single-frame payloads carry one chunk; the
/// multi-chunk shape carries `chunk_count` identical-schema chunks
/// that the client must decode and merge per RPC.
struct SynthPayload {
    responses: Vec<ResponseData>,
    /// Total length of the framed gRPC messages on the wire (5-byte
    /// prefix + encoded `ResponseData`, summed across chunks).
    framed_len: usize,
    /// Total decompressed `DataTable` encoding length across chunks.
    decoded_len: usize,
    /// Total row count across chunks (the per-RPC row assertion).
    rows: usize,
}

/// Deterministic EOD-shaped rows: 8 numeric columns, mostly full-range
/// random `i64` values. Random varints compress poorly, so the wire
/// frame stays close to the decoded size and the target is reachable.
///
/// Every row is valid input for the production `EodTick` build the
/// measured loop performs: `ms_of_day` stays inside the `i32`
/// milliseconds-of-day window and `date` is a real `YYYYMMDD` value
/// (both randomized so neither column compresses away); the six
/// remaining columns ride the unbounded `i64` decode arms and stay
/// full-range random.
fn build_rows(row_count: usize, seed: u64) -> DataTable {
    let mut rng = StdRng::seed_from_u64(seed);
    let rows: Vec<DataValueList> = (0..row_count)
        .map(|_| {
            let ms_of_day = i64::from(rng.random_range(0..=86_400_000_i32));
            let date = i64::from(
                rng.random_range(1900..=2099_i32) * 10_000
                    + rng.random_range(1..=12_i32) * 100
                    + rng.random_range(1..=28_i32),
            );
            let values = [
                ms_of_day,
                rng.random::<i64>(), // open
                rng.random::<i64>(), // high
                rng.random::<i64>(), // low
                rng.random::<i64>(), // close
                rng.random::<i64>(), // volume
                rng.random::<i64>(), // count
                date,
            ]
            .into_iter()
            .map(|n| DataValue {
                data_type: Some(data_value::DataType::Number(n)),
            })
            .collect();
            DataValueList { values }
        })
        .collect();
    DataTable {
        headers: [
            "ms_of_day",
            "open",
            "high",
            "low",
            "close",
            "volume",
            "count",
            "date",
        ]
        .iter()
        .map(|s| (*s).to_string())
        .collect(),
        data_table: rows,
    }
}

fn compress_table(table: &DataTable) -> SynthPayload {
    let inner = table.encode_to_vec();
    let mut encoder = zstd::stream::Encoder::new(Vec::new(), 3).expect("zstd encoder");
    encoder.write_all(&inner).expect("zstd write");
    let compressed = encoder.finish().expect("zstd finalize");
    let response = ResponseData {
        compressed_data: compressed,
        compression_description: Some(CompressionDescription {
            algo: i32::from(CompressionAlgo::Zstd),
            ..CompressionDescription::default()
        }),
        original_size: i32::try_from(inner.len()).unwrap_or(0),
    };
    let framed_len = 5 + response.encoded_len();
    SynthPayload {
        responses: vec![response],
        framed_len,
        decoded_len: inner.len(),
        rows: table.data_table.len(),
    }
}

/// Synthesize a `ResponseData` whose framed wire length lands close to
/// `target_wire_len`. Probes the bytes-per-row cost once, scales, then
/// refines once more — the report prints the actual size achieved.
fn synthesize_response(target_wire_len: usize) -> SynthPayload {
    const SEED: u64 = 0x7e7a_da7a;
    let probe_rows = 1024.min((target_wire_len / 16).max(8));
    let probe = compress_table(&build_rows(probe_rows, SEED));
    let bytes_per_row = (probe.framed_len as f64 / probe.rows as f64).max(1.0);
    let mut rows = ((target_wire_len as f64 / bytes_per_row) as usize).max(1);
    let mut best = compress_table(&build_rows(rows, SEED));
    // One refinement pass corrects the residual error from the
    // compression ratio shifting with table size.
    if best.framed_len > 0 {
        rows = ((rows as f64) * (target_wire_len as f64 / best.framed_len as f64)) as usize;
        let refined = compress_table(&build_rows(rows.max(1), SEED));
        let best_err = best.framed_len.abs_diff(target_wire_len);
        let refined_err = refined.framed_len.abs_diff(target_wire_len);
        if refined_err < best_err {
            best = refined;
        }
    }
    best
}

/// Synthesize a `chunk_count`-chunk response stream: each chunk lands
/// close to `per_chunk_wire_len` on the wire and every chunk carries
/// the same column schema (the wire contract `collect_stream`
/// enforces). Per-chunk row payloads differ (distinct seeds) so the
/// decode work is not artificially cache-warm across chunks.
fn synthesize_multi_chunk(chunk_count: usize, per_chunk_wire_len: usize) -> SynthPayload {
    let shape = synthesize_response(per_chunk_wire_len);
    let rows_per_chunk = shape.rows.max(1);
    let mut responses = Vec::with_capacity(chunk_count);
    let mut framed_len = 0usize;
    let mut decoded_len = 0usize;
    let mut rows = 0usize;
    for chunk_idx in 0..chunk_count {
        let seed = 0x7e7a_da7a ^ (chunk_idx as u64).wrapping_mul(0x9e37_79b9_7f4a_7c15);
        let chunk = compress_table(&build_rows(rows_per_chunk, seed));
        framed_len += chunk.framed_len;
        decoded_len += chunk.decoded_len;
        rows += chunk.rows;
        responses.extend(chunk.responses);
    }
    SynthPayload {
        responses,
        framed_len,
        decoded_len,
        rows,
    }
}

fn frame(msg: &ResponseData) -> Bytes {
    let payload = msg.encode_to_vec();
    let mut buf = BytesMut::with_capacity(5 + payload.len());
    buf.put_u8(0);
    buf.put_u32(u32::try_from(payload.len()).expect("frame length fits u32"));
    buf.extend_from_slice(&payload);
    buf.freeze()
}

// ─── Request constants ──────────────────────────────────────────────

const SESSION_UUID: &str = "00000000-0000-0000-0000-000000000000";
const CLIENT_TYPE: &str = "rust-thetadatadx-grpc";

/// Connection ceiling on both sides. Matches the upstream per-account
/// concurrency ceiling; the production pool never exceeds the Pro tier
/// cap of 8 channels, so 16 already carries headroom.
const MAX_CONNECTIONS_PER_SIDE: usize = 16;

// ─── Measurement core ───────────────────────────────────────────────

struct CellSpec {
    concurrency: usize,
    payload_name: &'static str,
    framed_len: usize,
    expected_rows: usize,
    max_message_size: usize,
    warmup: Duration,
    measure: Duration,
    repeats: usize,
}

#[derive(Default)]
struct CellResult {
    latencies_ns: Vec<u64>,
    requests_per_repeat: Vec<u64>,
    wall_per_repeat: Vec<Duration>,
    alloc_bytes: u64,
    cpu_micros: u64,
    total_requests: u64,
}

fn percentile(sorted: &[u64], q: f64) -> u64 {
    if sorted.is_empty() {
        return 0;
    }
    let rank = ((sorted.len() as f64) * q).ceil() as usize;
    sorted[rank.clamp(1, sorted.len()) - 1]
}

/// One measured window: `concurrency` workers, each looping
/// request-after-request (closed loop) until the shared deadline.
/// Returns per-request latencies and the wall time.
async fn run_window(
    pool: &Arc<ChannelPool>,
    spec: &CellSpec,
    window: Duration,
    record: bool,
) -> (Vec<u64>, Duration) {
    let gate = Arc::new(tokio::sync::Barrier::new(spec.concurrency + 1));
    let mut tasks = Vec::with_capacity(spec.concurrency);
    for _ in 0..spec.concurrency {
        let pool = Arc::clone(pool);
        let gate = Arc::clone(&gate);
        let expected_rows = spec.expected_rows;
        tasks.push(tokio::spawn(async move {
            let mut latencies = Vec::new();
            gate.wait().await;
            let deadline = Instant::now() + window;
            while Instant::now() < deadline {
                let started = Instant::now();
                let lease = pool.next();
                let ticks = bench_support::stock_history_eod(
                    &lease,
                    SESSION_UUID.to_string(),
                    CLIENT_TYPE.to_string(),
                    "AAPL",
                    "20240101",
                    "20240329",
                )
                .await
                .expect("in-house rpc");
                let elapsed = started.elapsed();
                assert_eq!(ticks.len(), expected_rows, "row count drift");
                if record {
                    latencies.push(u64::try_from(elapsed.as_nanos()).unwrap_or(u64::MAX));
                }
            }
            latencies
        }));
    }

    gate.wait().await;
    let started = Instant::now();
    let mut all = Vec::new();
    for task in tasks {
        let mut latencies = task.await.expect("worker join");
        all.append(&mut latencies);
    }
    (all, started.elapsed())
}

/// Build the client state, run warmup + measured repeats, and
/// aggregate. Fresh runtime, mock, and connections per cell so no
/// state crosses cells.
fn run_cell(spec: &CellSpec, payload: &[ResponseData]) -> CellResult {
    let rt = Runtime::new().expect("tokio runtime");
    let config = ServerConfig {
        framed: payload.iter().map(frame).collect(),
    };
    let mock = rt.block_on(spawn_mock(&rt, config));
    let addr = mock.addr;
    let mut result = CellResult::default();

    let record_repeat = |result: &mut CellResult,
                         repeat: usize,
                         mut latencies: Vec<u64>,
                         wall: Duration,
                         alloc_delta: u64,
                         cpu_delta: u64| {
        let requests = latencies.len() as u64;
        result.requests_per_repeat.push(requests);
        result.wall_per_repeat.push(wall);
        result.total_requests += requests;
        result.alloc_bytes += alloc_delta;
        result.cpu_micros += cpu_delta;
        result.latencies_ns.append(&mut latencies);
        eprintln!(
            "  [{} c={} {}] repeat {}/{}: {} reqs in {:.2?}",
            spec.payload_name,
            spec.concurrency,
            human_bytes(spec.framed_len),
            repeat + 1,
            spec.repeats,
            requests,
            wall,
        );
    };

    // One connection per worker up to the production pool ceiling;
    // synthetic levels above it multiplex workers over 16 connections
    // (h2 streams carry the fan-in). The `THETADATADX_BENCH_CONNS`
    // override pins the count instead — `1` measures every worker
    // multiplexed onto a single connection.
    let connections = std::env::var("THETADATADX_BENCH_CONNS")
        .ok()
        .and_then(|s| s.parse::<usize>().ok())
        .filter(|n| *n > 0)
        .unwrap_or_else(|| spec.concurrency.min(MAX_CONNECTIONS_PER_SIDE));

    let mut channels = Vec::with_capacity(connections);
    rt.block_on(async {
        for _ in 0..connections {
            let channel = Channel::connect_h2c_with_max_message_size(
                "127.0.0.1",
                addr.port(),
                spec.max_message_size,
            )
            .await
            .expect("transport connect");
            channels.push(channel);
        }
    });
    let pool = Arc::new(ChannelPool::from_channels(channels));

    rt.block_on(run_window(&pool, spec, spec.warmup, false));
    for repeat in 0..spec.repeats {
        let alloc_before = alloc_snapshot();
        let cpu_before = process_cpu_micros();
        let (latencies, wall) = rt.block_on(run_window(&pool, spec, spec.measure, true));
        let cpu_delta = process_cpu_micros().saturating_sub(cpu_before);
        let alloc_delta = alloc_snapshot().0.saturating_sub(alloc_before.0);
        record_repeat(&mut result, repeat, latencies, wall, alloc_delta, cpu_delta);
    }
    drop(pool);

    drop(mock);
    result
}

fn human_bytes(len: usize) -> String {
    if len >= 1024 * 1024 {
        format!("{:.1}MiB", len as f64 / (1024.0 * 1024.0))
    } else if len >= 1024 {
        format!("{:.1}KiB", len as f64 / 1024.0)
    } else {
        format!("{len}B")
    }
}

fn human_ns(ns: u64) -> String {
    if ns >= 1_000_000_000 {
        format!("{:.2}s", ns as f64 / 1e9)
    } else if ns >= 1_000_000 {
        format!("{:.2}ms", ns as f64 / 1e6)
    } else if ns >= 1_000 {
        format!("{:.1}us", ns as f64 / 1e3)
    } else {
        format!("{ns}ns")
    }
}

// ─── Matrix driver ──────────────────────────────────────────────────

struct PayloadSpec {
    name: &'static str,
    target_wire_len: usize,
    /// Number of response chunks streamed per RPC. `1` is the
    /// dominant production shape; the `multi` payload streams 16 so
    /// many chunks fan into one decode path per request.
    chunk_count: usize,
    max_message_size: usize,
    warmup: Duration,
    measure: Duration,
    /// Levels above this are skipped for the payload (in-flight bytes
    /// would dominate the box rather than the transport).
    max_concurrency: usize,
}

fn env_list(name: &str) -> Option<Vec<String>> {
    std::env::var(name).ok().map(|raw| {
        raw.split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect()
    })
}

fn main() {
    // `cargo bench` passes `--bench` (and criterion-style filters) to
    // every harness; this hand-rolled harness ignores them.
    let quick = std::env::var("THETADATADX_BENCH_QUICK").is_ok_and(|v| v == "1");

    let default_levels: Vec<usize> = if quick {
        vec![1, 8]
    } else {
        vec![1, 2, 4, 8, 16, 100, 1000]
    };
    let levels: Vec<usize> = env_list("THETADATADX_BENCH_LEVELS")
        .map(|items| {
            items
                .iter()
                .map(|s| {
                    s.parse::<usize>()
                        .expect("level must be a positive integer")
                })
                .collect()
        })
        .unwrap_or(default_levels);

    let repeats: usize = std::env::var("THETADATADX_BENCH_REPEATS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(if quick { 1 } else { 3 });

    let small_window = if quick {
        (Duration::from_millis(300), Duration::from_secs(1))
    } else {
        (Duration::from_millis(1500), Duration::from_secs(6))
    };
    let large_window = if quick {
        (Duration::from_secs(1), Duration::from_secs(2))
    } else {
        (Duration::from_secs(4), Duration::from_secs(12))
    };

    let mut payloads = vec![
        PayloadSpec {
            name: "small",
            target_wire_len: 1024,
            chunk_count: 1,
            max_message_size: 4 * 1024 * 1024,
            warmup: small_window.0,
            measure: small_window.1,
            max_concurrency: usize::MAX,
        },
        PayloadSpec {
            name: "large",
            target_wire_len: 10 * 1024 * 1024,
            chunk_count: 1,
            max_message_size: 64 * 1024 * 1024,
            warmup: large_window.0,
            measure: large_window.1,
            // 10 MB frames above the upstream concurrency ceiling would
            // measure allocator pressure, not transport — skip.
            max_concurrency: 16,
        },
        PayloadSpec {
            name: "multi",
            // 16 chunks x ~640 KiB lands the same ~10 MiB per RPC as
            // `large`, but as a chunked stream: per chunk the client
            // runs one zstd decompress + one prost decode, so 16
            // decodes fan into one decode path per request — the
            // fan-in shape the decoder-pool rationale targets.
            target_wire_len: 640 * 1024,
            chunk_count: 16,
            max_message_size: 64 * 1024 * 1024,
            warmup: large_window.0,
            measure: large_window.1,
            // Same in-flight-bytes ceiling rationale as `large`.
            max_concurrency: 16,
        },
    ];
    if quick {
        payloads.truncate(1);
    }
    if let Some(sizes) = env_list("THETADATADX_BENCH_SIZES") {
        payloads.retain(|p| sizes.iter().any(|s| s == p.name));
    }

    println!("# gRPC transport measurement — closed loop, loopback mock, warmed");
    println!();
    println!(
        "levels={levels:?} repeats={repeats} host_cores={}",
        std::thread::available_parallelism().map_or(0, std::num::NonZero::get)
    );

    for payload_spec in &payloads {
        let synth = if payload_spec.chunk_count > 1 {
            synthesize_multi_chunk(payload_spec.chunk_count, payload_spec.target_wire_len)
        } else {
            synthesize_response(payload_spec.target_wire_len)
        };
        println!();
        println!(
            "## payload `{}`: wire {} across {} chunk(s) ({} rows, decoded {}), \
             decode ceiling {}",
            payload_spec.name,
            human_bytes(synth.framed_len),
            synth.responses.len(),
            synth.rows,
            human_bytes(synth.decoded_len),
            human_bytes(payload_spec.max_message_size),
        );
        println!();
        println!(
            "| concurrency | p50 | p99 | p99.9 | mean | req/s (min..max) | wire MB/s | alloc/req | cpu/req |"
        );
        println!("|---|---|---|---|---|---|---|---|---|");

        for &concurrency in &levels {
            if concurrency > payload_spec.max_concurrency {
                eprintln!(
                    "  [skip {} c={concurrency}] above payload's concurrency ceiling",
                    payload_spec.name
                );
                continue;
            }
            {
                let spec = CellSpec {
                    concurrency,
                    payload_name: payload_spec.name,
                    framed_len: synth.framed_len,
                    expected_rows: synth.rows,
                    max_message_size: payload_spec.max_message_size,
                    warmup: payload_spec.warmup,
                    measure: payload_spec.measure,
                    repeats,
                };
                let mut cell = run_cell(&spec, &synth.responses);
                cell.latencies_ns.sort_unstable();

                let total_wall: f64 = cell.wall_per_repeat.iter().map(Duration::as_secs_f64).sum();
                let rates: Vec<f64> = cell
                    .requests_per_repeat
                    .iter()
                    .zip(&cell.wall_per_repeat)
                    .map(|(reqs, wall)| *reqs as f64 / wall.as_secs_f64())
                    .collect();
                let rate_min = rates.iter().copied().fold(f64::INFINITY, f64::min);
                let rate_max = rates.iter().copied().fold(0.0_f64, f64::max);
                let rate_total = cell.total_requests as f64 / total_wall;
                let mbps = rate_total * synth.framed_len as f64 / 1e6;
                let denom = cell.total_requests.max(1);
                let mean_ns = cell.latencies_ns.iter().sum::<u64>() / denom;

                println!(
                    "| {} | {} | {} | {} | {} | {:.0} ({:.0}..{:.0}) | {:.1} | {} | {} |",
                    concurrency,
                    human_ns(percentile(&cell.latencies_ns, 0.50)),
                    human_ns(percentile(&cell.latencies_ns, 0.99)),
                    human_ns(percentile(&cell.latencies_ns, 0.999)),
                    human_ns(mean_ns),
                    rate_total,
                    rate_min,
                    rate_max,
                    mbps,
                    human_bytes(usize::try_from(cell.alloc_bytes / denom).unwrap_or(usize::MAX)),
                    format_args!("{}us", cell.cpu_micros / denom),
                );
            }
        }
    }
}
