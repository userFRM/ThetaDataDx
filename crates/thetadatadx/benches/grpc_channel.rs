//! Criterion bench for the in-house `grpc::Channel` against the same
//! mock h2 server used by the integration tests. Three representative
//! MDDS endpoints are timed (`stock_list_symbols`, `stock_history_eod`,
//! `option_history_quote`) so latency and per-call allocation can be
//! tracked end-to-end. Latency percentiles come from criterion's
//! built-in HDR histogram; the per-iteration counting allocator at
//! the top of this file feeds an end-of-bench summary of
//! bytes-allocated-per-call.
//!
//! Bench is informational. CI runs it but does not gate on absolute
//! numbers — interpretation lives in the PR description.

use std::alloc::{GlobalAlloc, Layout, System};
use std::io::Write;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

use bytes::{BufMut, Bytes, BytesMut};
use criterion::{criterion_group, criterion_main, Criterion, Throughput};
use http::{HeaderMap, HeaderName, HeaderValue, Response, StatusCode};
use prost::Message;
use tokio::net::TcpListener;
use tokio::runtime::Runtime;
use tokio::sync::oneshot;
use tokio::task::JoinHandle;

use thetadatadx::grpc::{stock_list_symbols, Channel};
use thetadatadx::wire::{
    data_value, CompressionAlgo, CompressionDescription, DataTable, DataValue, DataValueList,
    ResponseData,
};

// ─── Counting allocator ─────────────────────────────────────────────
//
// Wraps the system allocator and tallies bytes allocated and bytes
// deallocated as atomic counters. The per-iteration snapshot lives
// INSIDE the `iter_custom` timed region in each bench body, so the
// reported numbers reflect only the work the iter loop is timing —
// Criterion setup, warmup, and reporting allocations are excluded.
// The averaged delta divides by the inner-iteration count (`iters`
// passed into `iter_custom`), not the outer group count, so each
// bench reports the actual per-RPC allocation cost.

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

#[inline]
fn alloc_snapshot() -> (u64, u64) {
    (
        BYTES_ALLOCATED.load(Ordering::Relaxed),
        BYTES_DEALLOCATED.load(Ordering::Relaxed),
    )
}

/// Aggregate of allocation activity across a single bench's timed
/// region (excluding Criterion bookkeeping). Updated atomically per
/// `iter_custom` invocation; reported at end of bench.
#[derive(Default)]
struct AllocAccumulator {
    alloc_total: AtomicU64,
    dealloc_total: AtomicU64,
    iterations: AtomicU64,
}

impl AllocAccumulator {
    fn record(&self, alloc_delta: u64, dealloc_delta: u64, iters: u64) {
        self.alloc_total.fetch_add(alloc_delta, Ordering::Relaxed);
        self.dealloc_total
            .fetch_add(dealloc_delta, Ordering::Relaxed);
        self.iterations.fetch_add(iters, Ordering::Relaxed);
    }

    fn report(&self, label: &str) {
        let alloc = self.alloc_total.load(Ordering::Relaxed);
        let dealloc = self.dealloc_total.load(Ordering::Relaxed);
        let iters = self.iterations.load(Ordering::Relaxed).max(1);
        let net = alloc.saturating_sub(dealloc);
        println!(
            "alloc/{label}: iterations={iters} alloc_total={alloc}B \
             dealloc_total={dealloc}B net_per_call={}B alloc_per_call={}B",
            net / iters,
            alloc / iters,
        );
    }
}

// ─── Mock h2 server ─────────────────────────────────────────────────
//
// Trimmed copy of `tests/grpc_mock_server.rs` that listens forever
// (one connection at a time) so a single criterion run can fire many
// iterations against it without re-binding ports. Each connection
// serves exactly one RPC then returns; the listener loops to accept
// the next.

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
                            let _ = serve_once(socket, cfg).await;
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

async fn serve_once(
    socket: tokio::net::TcpStream,
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

fn make_response(symbols: &[&str]) -> ResponseData {
    let rows: Vec<DataValueList> = symbols
        .iter()
        .map(|s| DataValueList {
            values: vec![DataValue {
                data_type: Some(data_value::DataType::Text((*s).to_string())),
            }],
        })
        .collect();
    let table = DataTable {
        headers: vec!["symbol".to_string()],
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

// ─── Bench bodies ───────────────────────────────────────────────────

const SYMBOL_COUNT: usize = 256;
const SESSION_UUID: &str = "00000000-0000-0000-0000-000000000000";

fn bench_in_house(c: &mut Criterion) {
    let rt = Runtime::new().expect("tokio runtime");
    let symbols: Vec<&str> = SYMBOL_NAMES.iter().take(SYMBOL_COUNT).copied().collect();
    let config = ServerConfig {
        chunks: vec![make_response(&symbols)],
    };
    let mock = rt.block_on(spawn_mock(&rt, config));
    let channel = rt
        .block_on(Channel::connect_h2c("127.0.0.1", mock.addr.port()))
        .expect("in-house h2c connect");

    let alloc = AllocAccumulator::default();
    let channel_ref = &channel;
    let alloc_ref = &alloc;

    let mut group = c.benchmark_group("stock_list_symbols");
    group.throughput(Throughput::Elements(1));
    group.bench_function("in_house", |b| {
        b.to_async(&rt).iter_custom(move |iters| async move {
            // Snapshot the allocator counters and the wall clock
            // INSIDE the timed region. Criterion's warmup, group
            // setup, and reporting overhead are excluded — the
            // reported alloc_per_call reflects only the RPC work.
            let (alloc_before, dealloc_before) = alloc_snapshot();
            let timing_start = Instant::now();
            for _ in 0..iters {
                std::hint::black_box(
                    stock_list_symbols(
                        channel_ref,
                        SESSION_UUID.to_string(),
                        "rust-thetadatadx-grpc".to_string(),
                    )
                    .await
                    .expect("in-house rpc ok"),
                );
            }
            let elapsed = timing_start.elapsed();
            let (alloc_after, dealloc_after) = alloc_snapshot();
            alloc_ref.record(
                alloc_after.saturating_sub(alloc_before),
                dealloc_after.saturating_sub(dealloc_before),
                iters,
            );
            elapsed
        });
    });
    group.finish();
    alloc.report("in_house");

    // Hold the mock until the bench group is fully measured.
    drop(channel);
    drop(mock);
}

fn bench_stock_history_eod(c: &mut Criterion) {
    use thetadatadx::grpc::endpoints::bench_support;

    let rt = Runtime::new().expect("tokio runtime");
    // Approximate a 60-row EOD response so the bench reflects a
    // multi-bar payload rather than a single ticker list.
    let symbols: Vec<&str> = SYMBOL_NAMES.iter().take(60).copied().collect();
    let config = ServerConfig {
        chunks: vec![make_response(&symbols)],
    };
    let mock = rt.block_on(spawn_mock(&rt, config));
    let channel = rt
        .block_on(Channel::connect_h2c("127.0.0.1", mock.addr.port()))
        .expect("in-house h2c connect");

    let alloc = AllocAccumulator::default();
    let channel_ref = &channel;
    let alloc_ref = &alloc;

    let mut group = c.benchmark_group("stock_history_eod");
    group.throughput(Throughput::Elements(1));
    group.bench_function("in_house", |b| {
        b.to_async(&rt).iter_custom(move |iters| async move {
            let (alloc_before, dealloc_before) = alloc_snapshot();
            let timing_start = Instant::now();
            for _ in 0..iters {
                std::hint::black_box(
                    bench_support::stock_history_eod(
                        channel_ref,
                        SESSION_UUID.to_string(),
                        "rust-thetadatadx-grpc".to_string(),
                        "AAPL",
                        "20240101",
                        "20240329",
                    )
                    .await
                    .expect("stock_history_eod rpc ok"),
                );
            }
            let elapsed = timing_start.elapsed();
            let (alloc_after, dealloc_after) = alloc_snapshot();
            alloc_ref.record(
                alloc_after.saturating_sub(alloc_before),
                dealloc_after.saturating_sub(dealloc_before),
                iters,
            );
            elapsed
        });
    });
    group.finish();
    alloc.report("stock_history_eod");

    drop(channel);
    drop(mock);
}

fn bench_option_history_quote(c: &mut Criterion) {
    use thetadatadx::grpc::endpoints::bench_support;

    let rt = Runtime::new().expect("tokio runtime");
    // Approximate an option quote stream payload.
    let symbols: Vec<&str> = SYMBOL_NAMES.iter().take(128).copied().collect();
    let config = ServerConfig {
        chunks: vec![make_response(&symbols)],
    };
    let mock = rt.block_on(spawn_mock(&rt, config));
    let channel = rt
        .block_on(Channel::connect_h2c("127.0.0.1", mock.addr.port()))
        .expect("in-house h2c connect");

    let alloc = AllocAccumulator::default();
    let channel_ref = &channel;
    let alloc_ref = &alloc;

    let mut group = c.benchmark_group("option_history_quote");
    group.throughput(Throughput::Elements(1));
    group.bench_function("in_house", |b| {
        b.to_async(&rt).iter_custom(move |iters| async move {
            let (alloc_before, dealloc_before) = alloc_snapshot();
            let timing_start = Instant::now();
            for _ in 0..iters {
                std::hint::black_box(
                    bench_support::option_history_quote(
                        channel_ref,
                        SESSION_UUID.to_string(),
                        "rust-thetadatadx-grpc".to_string(),
                        "SPY",
                        "20240419",
                        "500000",
                        "C",
                        "20240329",
                    )
                    .await
                    .expect("option_history_quote rpc ok"),
                );
            }
            let elapsed = timing_start.elapsed();
            let (alloc_after, dealloc_after) = alloc_snapshot();
            alloc_ref.record(
                alloc_after.saturating_sub(alloc_before),
                dealloc_after.saturating_sub(dealloc_before),
                iters,
            );
            elapsed
        });
    });
    group.finish();
    alloc.report("option_history_quote");

    drop(channel);
    drop(mock);
}

// 256 unique ASCII tickers — pure constants, no allocation.
const SYMBOL_NAMES: &[&str] = &[
    "AAPL", "MSFT", "GOOG", "META", "AMZN", "NVDA", "TSLA", "AMD", "INTC", "ORCL", "IBM", "CSCO",
    "CRM", "ADBE", "PYPL", "QCOM", "AVGO", "TXN", "MU", "WDC", "STX", "HPQ", "DELL", "NFLX", "DIS",
    "CMCSA", "T", "VZ", "TMUS", "S", "GS", "JPM", "BAC", "WFC", "C", "MS", "BLK", "BX", "USB",
    "TFC", "PNC", "COF", "AXP", "V", "MA", "FIS", "FISV", "WU", "GPN", "VRSN", "AKAM", "FFIV",
    "CTSH", "INFY", "WIT", "TSM", "ASML", "ARM", "MRVL", "NXPI", "ON", "MCHP", "MPWR", "SWKS",
    "QRVO", "NTAP", "SMCI", "ANET", "PANW", "FTNT", "CRWD", "ZS", "OKTA", "DDOG", "MDB", "SNOW",
    "PLTR", "NOW", "WDAY", "TEAM", "ADSK", "INTU", "CDNS", "SNPS", "ANSS", "PTC", "DASH", "UBER",
    "LYFT", "ABNB", "BKNG", "EXPE", "MAR", "HLT", "RCL", "CCL", "NCLH", "UAL", "DAL", "AAL", "LUV",
    "DE", "CAT", "HON", "GE", "RTX", "LMT", "NOC", "BA", "GD", "TXT", "EMR", "PH", "ITW", "ROK",
    "MMM", "PEP", "KO", "PG", "JNJ", "PFE", "MRK", "BMY", "ABBV", "ABT", "LLY", "TMO", "DHR",
    "MDT", "ISRG", "BSX", "REGN", "VRTX", "GILD", "BIIB", "AMGN", "ZTS", "MCD", "SBUX", "YUM",
    "CMG", "DPZ", "QSR", "NKE", "LULU", "UAA", "VFC", "TPR", "RL", "PVH", "HBI", "GPS", "ANF",
    "AEO", "URBN", "ROST", "TJX", "TGT", "WMT", "COST", "BBY", "HD", "LOW", "AZO", "ORLY", "ULTA",
    "DG", "DLTR", "KR", "SYY", "CL", "KMB", "PFG", "MET", "AIG", "PRU", "ALL", "TRV", "PGR", "HIG",
    "CB", "RGA", "RNR", "CINF", "L", "TRAVE", "AIZ", "AFL", "AON", "MMC", "AJG", "WRB", "BRO",
    "WTW", "ICE", "CME", "MCO", "MSCI", "SPGI", "FDS", "NDAQ", "MKTX", "TROW", "BEN", "IVZ", "AMG",
    "JEF", "RJF", "LPLA", "EVR", "PJT", "PIPR", "SF", "HLI", "MS5", "PFGC", "GHC", "SCHW", "ETFC",
    "AMTD", "CAT2", "DE2", "AGCO", "EMR2", "FLR", "JCI", "PNR", "ROP", "DOV", "FAST", "GWW", "WSO",
    "AME", "ALLE", "MAS", "FBHS", "BLD", "TOL", "DHI", "LEN", "PHM", "NVR", "MTH", "KBH", "MDC",
    "BZH", "TPH", "TMHC", "MTX", "EXP", "EXR", "PSA", "AVB", "EQR", "ESS", "MAA", "UDR", "INVH",
];

criterion_group!(
    benches,
    bench_in_house,
    bench_stock_history_eod,
    bench_option_history_quote
);
criterion_main!(benches);
