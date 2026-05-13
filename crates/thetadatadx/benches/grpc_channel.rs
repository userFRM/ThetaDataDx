//! Criterion bench: in-house `grpc::Channel` vs `tonic::transport::Channel`
//! on the `BetaThetaTerminal::GetStockListSymbols` method.
//!
//! Both paths talk to the same mock h2 server defined in
//! `tests/grpc_mock_server.rs` so the only difference under measurement
//! is the client-side transport stack. Latency percentiles come from
//! criterion's built-in HDR histogram; the per-iteration counting
//! allocator at the top of this file feeds an end-of-bench summary of
//! bytes-allocated-per-call.
//!
//! Bench is informational. CI runs it but does not gate on absolute
//! numbers — interpretation lives in the PR description.

#![cfg(feature = "inhouse-grpc")]

use std::alloc::{GlobalAlloc, Layout, System};
use std::io::Write;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicU64, Ordering};

use bytes::{BufMut, Bytes, BytesMut};
use criterion::{criterion_group, criterion_main, Criterion, Throughput};
use http::{HeaderMap, HeaderName, HeaderValue, Response, StatusCode};
use prost::Message;
use tokio::net::TcpListener;
use tokio::runtime::Runtime;
use tokio::sync::oneshot;
use tokio::task::JoinHandle;

use thetadatadx::grpc::{stock_list_symbols, stock_list_symbols_via_tonic, Channel};
use thetadatadx::wire::{
    data_value, CompressionAlgo, CompressionDescription, DataTable, DataValue, DataValueList,
    ResponseData,
};

// ─── Counting allocator ─────────────────────────────────────────────
//
// Wraps the system allocator and tallies bytes allocated and bytes
// deallocated as atomic counters. Callers snapshot before a bench
// iteration, run the iteration, then snapshot again to compute the
// per-call high-water mark. The bench reports `mean(alloc - dealloc)`
// per call so transient buffers that are immediately freed do not
// inflate the number.

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

    let alloc_start = alloc_snapshot();
    let iterations = std::sync::atomic::AtomicU64::new(0);

    let mut group = c.benchmark_group("stock_list_symbols");
    group.throughput(Throughput::Elements(1));
    group.bench_function("in_house", |b| {
        b.to_async(&rt).iter(|| {
            iterations.fetch_add(1, Ordering::Relaxed);
            async {
                stock_list_symbols(
                    &channel,
                    SESSION_UUID.to_string(),
                    "rust-thetadatadx-grpc".to_string(),
                )
                .await
                .expect("in-house rpc ok")
            }
        });
    });
    group.finish();

    let alloc_end = alloc_snapshot();
    report_alloc(
        "in_house",
        alloc_start,
        alloc_end,
        iterations.load(Ordering::Relaxed),
    );

    // Hold the mock until the bench group is fully measured.
    drop(channel);
    drop(mock);
}

fn bench_tonic(c: &mut Criterion) {
    let rt = Runtime::new().expect("tokio runtime");
    let symbols: Vec<&str> = SYMBOL_NAMES.iter().take(SYMBOL_COUNT).copied().collect();
    let config = ServerConfig {
        chunks: vec![make_response(&symbols)],
    };
    let mock = rt.block_on(spawn_mock(&rt, config));
    let uri = format!("http://127.0.0.1:{}", mock.addr.port());

    // tonic 0.14 spelling: `Channel::from_shared` returns an Endpoint.
    let channel = rt
        .block_on(async {
            tonic::transport::Channel::from_shared(uri.clone())
                .expect("endpoint")
                .connect()
                .await
        })
        .expect("tonic connect");

    let alloc_start = alloc_snapshot();
    let iterations = std::sync::atomic::AtomicU64::new(0);

    let mut group = c.benchmark_group("stock_list_symbols");
    group.throughput(Throughput::Elements(1));
    group.bench_function("tonic", |b| {
        let channel = channel.clone();
        b.to_async(&rt).iter(|| {
            iterations.fetch_add(1, Ordering::Relaxed);
            let channel = channel.clone();
            async move {
                stock_list_symbols_via_tonic(
                    channel,
                    SESSION_UUID.to_string(),
                    "rust-thetadatadx-grpc".to_string(),
                )
                .await
                .expect("tonic rpc ok")
            }
        });
    });
    group.finish();

    let alloc_end = alloc_snapshot();
    report_alloc(
        "tonic",
        alloc_start,
        alloc_end,
        iterations.load(Ordering::Relaxed),
    );

    drop(channel);
    drop(mock);
}

fn report_alloc(label: &str, start: (u64, u64), end: (u64, u64), iterations: u64) {
    let allocated = end.0.saturating_sub(start.0);
    let deallocated = end.1.saturating_sub(start.1);
    let net = allocated.saturating_sub(deallocated);
    let denom = iterations.max(1);
    println!(
        "alloc/{label}: iterations={iterations} alloc_total={allocated}B \
         dealloc_total={deallocated}B net_per_call={}B alloc_per_call={}B",
        net / denom,
        allocated / denom,
    );
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

criterion_group!(benches, bench_in_house, bench_tonic);
criterion_main!(benches);
