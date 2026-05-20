//! Concurrent-burst bench: 64 in-flight `stock_history_eod` calls
//! fanned out across a 4-channel [`ChannelPool`] against a multi-mock
//! h2 backend. Exercises the realistic large-payload workload — the
//! same shape that drives the flamegraph profile.
//!
//! Setup (mock spawn, TCP connect, h2 handshake, pool build) sits
//! outside the timed region via `iter_custom`. The timed region is
//! the 64-way `join_all` over the RPC futures plus their decode
//! pipeline — everything from `request encode` through
//! `zstd decompress` and `prost decode` of every chunk.

use std::alloc::{GlobalAlloc, Layout, System};
use std::io::Write;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use bytes::{BufMut, Bytes, BytesMut};
use criterion::{criterion_group, criterion_main, Criterion, Throughput};
use http::{HeaderMap, HeaderName, HeaderValue, Response, StatusCode};
use prost::Message;
use tokio::net::{TcpListener, TcpStream};
use tokio::runtime::Runtime;
use tokio::sync::oneshot;
use tokio::task::JoinHandle;

use thetadatadx::grpc::endpoints::bench_support;
use thetadatadx::grpc::{default_decoder_thread_count, Channel, ChannelPool, DecoderPool};
use thetadatadx::wire::{
    data_value, CompressionAlgo, CompressionDescription, DataTable, DataValue, DataValueList,
    ResponseData,
};

// ─── Counting allocator ─────────────────────────────────────────────

struct CountingAllocator;

static BYTES_ALLOCATED: AtomicU64 = AtomicU64::new(0);
static BYTES_DEALLOCATED: AtomicU64 = AtomicU64::new(0);

unsafe impl GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        // SAFETY: forwarding to the system allocator under the same
        // contract the caller is upholding.
        let ptr = unsafe { System.alloc(layout) };
        if !ptr.is_null() {
            BYTES_ALLOCATED.fetch_add(layout.size() as u64, Ordering::Relaxed);
        }
        ptr
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        BYTES_DEALLOCATED.fetch_add(layout.size() as u64, Ordering::Relaxed);
        // SAFETY: forwarding to the system allocator under the same
        // contract the caller is upholding.
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

// ─── Mock h2 backend ────────────────────────────────────────────────
//
// One listener per pool member. Each listener accepts a single
// h2 connection and serves an unbounded number of RPCs over it (the
// burst fires multiple streams down the same connection because h2
// multiplexes). Each RPC echoes a hardcoded `ResponseData` chunk and
// closes with `grpc-status: 0`.

#[derive(Clone)]
struct ServerConfig {
    chunks: Vec<ResponseData>,
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
        let chunks = config.chunks.clone();
        tokio::spawn(async move {
            let _ = handle_request(request, respond, chunks).await;
        });
    }
    Ok(())
}

async fn handle_request(
    request: http::Request<h2::RecvStream>,
    mut respond: h2::server::SendResponse<Bytes>,
    chunks: Vec<ResponseData>,
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
    for chunk in &chunks {
        let framed = frame(chunk);
        send_stream.send_data(framed, false)?;
    }
    let mut trailers = HeaderMap::new();
    trailers.insert(
        HeaderName::from_static("grpc-status"),
        HeaderValue::from_static("0"),
    );
    send_stream.send_trailers(trailers)?;
    Ok(())
}

fn frame<M: Message>(msg: &M) -> Bytes {
    let payload = msg.encode_to_vec();
    let mut buf = BytesMut::with_capacity(5 + payload.len());
    buf.put_u8(0);
    buf.put_u32(u32::try_from(payload.len()).unwrap());
    buf.extend_from_slice(&payload);
    buf.freeze()
}

/// Synthesize a zstd-compressed `ResponseData` carrying a `DataTable`
/// of `row_count` EOD-shaped rows. The same shape `stock_history_eod`
/// emits in production — picked here because it's the
/// large-payload workload most likely to put pressure on decompress +
/// prost decode at the same time.
fn make_eod_response(row_count: usize) -> ResponseData {
    let rows: Vec<DataValueList> = (0..row_count)
        .map(|i| DataValueList {
            values: vec![
                DataValue {
                    data_type: Some(data_value::DataType::Number(34_200_000 + i as i64)),
                },
                DataValue {
                    data_type: Some(data_value::DataType::Text(format!("AAPL-{i}"))),
                },
            ],
        })
        .collect();
    let table = DataTable {
        headers: vec!["ms_of_day".to_string(), "symbol".to_string()],
        data_table: rows,
    };
    let inner = table.encode_to_vec();
    let mut encoder = zstd::stream::Encoder::new(Vec::new(), 3).expect("zstd encoder");
    encoder.write_all(&inner).expect("zstd write");
    let compressed = encoder.finish().expect("zstd finalize");

    ResponseData {
        compressed_data: compressed,
        compression_description: Some(CompressionDescription {
            algo: i32::from(CompressionAlgo::Zstd),
            ..CompressionDescription::default()
        }),
        original_size: i32::try_from(inner.len()).unwrap_or(0),
    }
}

// ─── Bench body ─────────────────────────────────────────────────────

const POOL_SIZE: usize = 4;
const BURST_SIZE: usize = 64;
/// Default row count per RPC response. Overridable via the
/// `THETADATADX_BENCH_ROWS` env var so the same bench can sweep the
/// small / medium / large payload regimes without recompiling.
const EOD_ROW_COUNT: usize = 256;
const SESSION_UUID: &str = "00000000-0000-0000-0000-000000000000";

/// Resolve the row count for a single response chunk: take
/// `THETADATADX_BENCH_ROWS` when set, otherwise the compile-time
/// [`EOD_ROW_COUNT`] default. Parses the env value as a positive
/// integer; bad input falls back to the default so the bench is
/// always runnable.
fn resolve_row_count() -> usize {
    std::env::var("THETADATADX_BENCH_ROWS")
        .ok()
        .and_then(|s| s.parse::<usize>().ok())
        .filter(|&n| n > 0)
        .unwrap_or(EOD_ROW_COUNT)
}

fn bench_concurrent_burst(c: &mut Criterion) {
    let rt = Runtime::new().expect("tokio runtime");
    let rows = resolve_row_count();
    let response = make_eod_response(rows);
    let config = ServerConfig {
        chunks: vec![response],
    };

    // One mock per pool member. h2 multiplexes the 16 streams each
    // pool member sees onto its single connection.
    let mut mocks = Vec::with_capacity(POOL_SIZE);
    let mut channels = Vec::with_capacity(POOL_SIZE);
    rt.block_on(async {
        for _ in 0..POOL_SIZE {
            let mock = spawn_mock(&rt, config.clone()).await;
            let channel = Channel::connect_h2c("127.0.0.1", mock.addr.port())
                .await
                .expect("h2c connect");
            channels.push(channel);
            mocks.push(mock);
        }
    });
    // Dedicated decoder pool: same shape as production
    // `MddsClient::connect` wires up. Threads = `available_parallelism
    // / 2` capped at `POOL_SIZE`, ring = 256 slots (the production
    // default).
    let decoder_pool =
        DecoderPool::new(default_decoder_thread_count(POOL_SIZE), 256).expect("decoder pool init");
    let pool = ChannelPool::from_channels_with_decoders(channels, decoder_pool);

    let alloc_start = alloc_snapshot();
    let iterations = AtomicU64::new(0);

    // Optional embedded sampling profiler. Set `THETADATADX_FLAMEGRAPH=1`
    // to start `pprof::ProfilerGuard` over the bench group; the SVG
    // lands at `$THETADATADX_FLAMEGRAPH_OUT` (defaulting to
    // `/tmp/grpc_burst_flame.svg`). The profiler uses SIGPROF
    // sampling — no perf / root required, no kernel privileges.
    let profiler = if std::env::var_os("THETADATADX_FLAMEGRAPH").is_some() {
        Some(
            pprof::ProfilerGuardBuilder::default()
                .frequency(997)
                .blocklist(&["libc", "libgcc", "pthread", "vdso"])
                .build()
                .expect("pprof profiler"),
        )
    } else {
        None
    };

    let mut group = c.benchmark_group("concurrent_burst");
    group.throughput(Throughput::Elements(BURST_SIZE as u64));
    group.bench_function("stock_history_eod_64way", |b| {
        b.iter_custom(|iters| {
            let pool = pool.clone();
            rt.block_on(async {
                let start = Instant::now();
                for _ in 0..iters {
                    iterations.fetch_add(1, Ordering::Relaxed);
                    // Each dispatch captures its own `ChannelLease`
                    // synchronously so the picker (Finding 4) sees
                    // every prior reservation before issuing the
                    // next pick. The lease's `Deref` to `&Channel`
                    // satisfies the `stock_history_eod` signature;
                    // the lease lives across the await, keeping the
                    // in-flight reservation committed for the
                    // dispatch window.
                    let futures = (0..BURST_SIZE).map(|_| {
                        let lease = pool.next();
                        async move {
                            bench_support::stock_history_eod(
                                &lease,
                                SESSION_UUID.to_string(),
                                "rust-thetadatadx-grpc".to_string(),
                                "AAPL",
                                "20240101",
                                "20240329",
                            )
                            .await
                        }
                    });
                    let results = futures::future::join_all(futures).await;
                    for r in results {
                        r.expect("rpc ok");
                    }
                }
                start.elapsed()
            })
        });
    });
    group.finish();

    if let Some(profiler) = profiler {
        let out = std::env::var("THETADATADX_FLAMEGRAPH_OUT")
            .unwrap_or_else(|_| "/tmp/grpc_burst_flame.svg".to_string());
        match profiler.report().build() {
            Ok(report) => match std::fs::File::create(&out) {
                Ok(file) => {
                    if let Err(e) = report.flamegraph(file) {
                        eprintln!("flamegraph write failed: {e}");
                    } else {
                        println!("flamegraph written to {out}");
                    }
                }
                Err(e) => eprintln!("flamegraph open failed: {e}"),
            },
            Err(e) => eprintln!("pprof report build failed: {e}"),
        }
    }

    let alloc_end = alloc_snapshot();
    let iters_total = iterations.load(Ordering::Relaxed);
    let calls_total = iters_total.saturating_mul(BURST_SIZE as u64);
    let allocated = alloc_end.0.saturating_sub(alloc_start.0);
    let deallocated = alloc_end.1.saturating_sub(alloc_start.1);
    let denom = calls_total.max(1);
    println!(
        "alloc/concurrent_burst: bursts={iters_total} rpcs={calls_total} \
         alloc_total={allocated}B dealloc_total={deallocated}B \
         alloc_per_call={}B net_per_call={}B",
        allocated / denom,
        allocated.saturating_sub(deallocated) / denom,
    );

    // Keep the mocks alive past the bench group so dangling streams
    // do not race the listener teardown.
    drop(pool);
    drop(mocks);
}

// Some hosts/CI miss `perf_event_paranoid` tuning — give criterion a
// small warm-up before sampling so jit-cold paths do not skew the
// flamegraph.
fn config() -> Criterion {
    Criterion::default()
        .warm_up_time(Duration::from_secs(3))
        .measurement_time(Duration::from_secs(8))
}

criterion_group! {
    name = burst;
    config = config();
    targets = bench_concurrent_burst
}
criterion_main!(burst);
