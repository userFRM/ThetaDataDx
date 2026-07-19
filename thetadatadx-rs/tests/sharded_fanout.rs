//! End-to-end sharded bulk-fetch tests against an in-process gRPC mock.
//!
//! The unit tests in `mdds/shard.rs` pin the planner math and the join
//! drivers in isolation; these tests drive the REAL generated builders
//! (`stock_history_trade`, buffered `.await` and `.stream(handler)`)
//! through `auto_plan` → band fan-out → per-band retry → merge/forward,
//! over the wire against a mock server, so a drift anywhere in that
//! pipeline — the projection macros, the band overrides, the fan-out
//! arms, the join semantics — fails here.
//!
//! The mock accepts any number of connections and streams. Each stream
//! decodes the inbound `StockHistoryTradeRequest`, keys the band by its
//! `start_date` override, and answers from a per-band script (rows, a
//! pre-stream `NotFound`, or rows-then-transient-error), recording every
//! request so the tests can assert the fan-out shape itself.

#![cfg(feature = "__test-helpers")]

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use bytes::{BufMut, Bytes, BytesMut};
use http::{HeaderMap, HeaderName, HeaderValue, Response, StatusCode};
use prost::Message;
use tokio::net::TcpListener;
use tokio::sync::Semaphore;

use thetadatadx::grpc::{Channel, ChannelPool};
use thetadatadx::mdds::MarketDataClient;
use thetadatadx::wire::{
    data_value, CompressionAlgo, CompressionDescription, DataTable, DataValue, DataValueList,
    ResponseData,
};
use thetadatadx::{DirectConfig, Error, RetryPolicy, ShardBand};

/// Tag-compatible mirror of the `StockHistoryTradeRequest` /
/// `StockHistoryTradeRequestQuery` wire pair, trimmed to the one field
/// the mock dispatches on. Protobuf decodes by field tag and skips
/// unknown fields, so this reads any band request the client encodes
/// without re-exporting the crate-internal request protos.
#[derive(Clone, PartialEq, prost::Message)]
struct BandRequestProbe {
    #[prost(message, optional, tag = "2")]
    params: Option<BandParamsProbe>,
}

#[derive(Clone, PartialEq, prost::Message)]
struct BandParamsProbe {
    /// `start_date` — tag 6 on `StockHistoryTradeRequestQuery`.
    #[prost(string, optional, tag = "6")]
    start_date: Option<String>,
}

// ─── Per-band scripting ──────────────────────────────────────────────────

/// One trade row: `(ms_of_day, price, date)`, all sent as `Number`
/// cells (`row_price_f64` accepts `Number` for `price`).
type Row = (i64, i64, i64);

/// One attempt's scripted reply for a band.
#[derive(Clone)]
struct BandResponse {
    /// `DataTable` chunks to stream, one `ResponseData` per entry.
    chunks: Vec<Vec<Row>>,
    /// Trailing `grpc-status`. With no chunks and a non-zero status the
    /// mock answers trailers-only — the pre-stream verdict shape MDDS
    /// uses for `NotFound`.
    status: u32,
}

impl BandResponse {
    fn ok(chunks: Vec<Vec<Row>>) -> Self {
        Self { chunks, status: 0 }
    }

    fn not_found() -> Self {
        Self {
            chunks: Vec::new(),
            status: 5,
        }
    }

    /// Stream `chunks`, then fail the stream with `Unavailable` — the
    /// mid-band transport-error shape.
    fn unavailable_after(chunks: Vec<Vec<Row>>) -> Self {
        Self { chunks, status: 14 }
    }
}

/// Scripted replies per band, keyed by the band's `start_date`
/// override. Attempt `n` (0-based) answers with `responses[n]`,
/// clamped to the last entry, so "fail once then succeed" and "always
/// fail" are both one vector.
struct BandScript {
    responses: Vec<BandResponse>,
    attempts: AtomicUsize,
}

struct MockScript {
    bands: HashMap<String, BandScript>,
    /// Every inbound request's band key, in arrival order.
    requests: Mutex<Vec<String>>,
}

impl MockScript {
    fn new(bands: Vec<(&str, Vec<BandResponse>)>) -> Arc<Self> {
        Arc::new(Self {
            bands: bands
                .into_iter()
                .map(|(key, responses)| {
                    (
                        key.to_string(),
                        BandScript {
                            responses,
                            attempts: AtomicUsize::new(0),
                        },
                    )
                })
                .collect(),
            requests: Mutex::new(Vec::new()),
        })
    }

    fn attempts(&self, band: &str) -> usize {
        self.bands[band].attempts.load(Ordering::Relaxed)
    }

    fn seen_bands(&self) -> Vec<String> {
        let mut seen = self.requests.lock().unwrap().clone();
        seen.sort();
        seen
    }
}

// ─── Mock server ─────────────────────────────────────────────────────────

/// Multi-connection, multi-stream gRPC mock: every accepted stream is
/// answered from the shared [`MockScript`]. The sibling
/// `grpc_mock_server.rs` mock serves one fixed reply to every stream;
/// the shard tests need per-band, per-attempt replies, so this mock
/// dispatches on the decoded request instead.
struct BandMockServer {
    addr: SocketAddr,
    task: tokio::task::JoinHandle<()>,
}

impl Drop for BandMockServer {
    fn drop(&mut self) {
        self.task.abort();
    }
}

async fn spawn_band_mock(script: Arc<MockScript>) -> BandMockServer {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind ephemeral port");
    let addr = listener.local_addr().expect("local addr");
    let task = tokio::spawn(async move {
        loop {
            let Ok((socket, _)) = listener.accept().await else {
                return;
            };
            let _ = socket.set_nodelay(true);
            let script = Arc::clone(&script);
            tokio::spawn(async move {
                let Ok(mut connection) = h2::server::handshake(socket).await else {
                    return;
                };
                while let Some(Ok((request, respond))) = connection.accept().await {
                    let script = Arc::clone(&script);
                    tokio::spawn(async move {
                        let _ = handle_stream(request, respond, &script).await;
                    });
                }
            });
        }
    });
    BandMockServer { addr, task }
}

async fn handle_stream(
    request: http::Request<h2::RecvStream>,
    mut respond: h2::server::SendResponse<Bytes>,
    script: &MockScript,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    // Drain the framed request body and decode the band key.
    let mut body = request.into_body();
    let mut buf: Vec<u8> = Vec::new();
    while let Some(chunk) = body.data().await {
        let chunk = chunk?;
        let _ = body.flow_control().release_capacity(chunk.len());
        buf.extend_from_slice(&chunk);
    }
    // gRPC frame: 1-byte compressed flag + 4-byte length + payload.
    let payload = buf.get(5..).unwrap_or_default();
    let decoded = BandRequestProbe::decode(payload)?;
    let band_key = decoded
        .params
        .and_then(|p| p.start_date)
        .unwrap_or_default();
    script.requests.lock().unwrap().push(band_key.clone());

    let band = script
        .bands
        .get(&band_key)
        .unwrap_or_else(|| panic!("request for unscripted band {band_key:?}"));
    let attempt = band.attempts.fetch_add(1, Ordering::Relaxed);
    let reply = band.responses[attempt.min(band.responses.len() - 1)].clone();

    if reply.chunks.is_empty() && reply.status != 0 {
        // Trailers-only response: `grpc-status` on the HEADERS frame
        // with END_STREAM — the pre-stream error verdict.
        let mut response = Response::new(());
        *response.status_mut() = StatusCode::OK;
        response.headers_mut().insert(
            http::header::CONTENT_TYPE,
            HeaderValue::from_static("application/grpc+proto"),
        );
        response.headers_mut().insert(
            HeaderName::from_static("grpc-status"),
            HeaderValue::from_str(&reply.status.to_string()).expect("numeric ASCII"),
        );
        if reply.status == 5 {
            response.headers_mut().insert(
                HeaderName::from_static("grpc-message"),
                HeaderValue::from_static("No data found for your request"),
            );
        }
        respond.send_response(response, true)?;
        return Ok(());
    }

    let mut response = Response::new(());
    *response.status_mut() = StatusCode::OK;
    response.headers_mut().insert(
        http::header::CONTENT_TYPE,
        HeaderValue::from_static("application/grpc+proto"),
    );
    let mut send_stream = respond.send_response(response, false)?;
    for rows in &reply.chunks {
        send_stream.send_data(frame(&trade_chunk(rows)), false)?;
    }
    let mut trailers = HeaderMap::new();
    trailers.insert(
        HeaderName::from_static("grpc-status"),
        HeaderValue::from_str(&reply.status.to_string()).expect("numeric ASCII"),
    );
    send_stream.send_trailers(trailers)?;
    Ok(())
}

/// Compose a length-prefixed gRPC frame from a protobuf message.
fn frame<M: Message>(msg: &M) -> Bytes {
    let payload = msg.encode_to_vec();
    let mut buf = BytesMut::with_capacity(5 + payload.len());
    buf.put_u8(0);
    buf.put_u32(u32::try_from(payload.len()).unwrap());
    buf.extend_from_slice(&payload);
    buf.freeze()
}

/// Build one `ResponseData` chunk carrying trade rows under the
/// v3-canonical headers `parse_trade_ticks` reads.
fn trade_chunk(rows: &[Row]) -> ResponseData {
    let number = |n: i64| DataValue {
        data_type: Some(data_value::DataType::Number(n)),
    };
    let table = DataTable {
        headers: vec![
            "ms_of_day".to_string(),
            "price".to_string(),
            "date".to_string(),
        ],
        data_table: rows
            .iter()
            .map(|&(ms, price, date)| DataValueList {
                values: vec![number(ms), number(price), number(date)],
            })
            .collect(),
    };
    ResponseData {
        compression_description: Some(CompressionDescription {
            algo: CompressionAlgo::None as i32,
            level: 0,
        }),
        original_size: 0,
        compressed_data: table.encode_to_vec(),
    }
}

// ─── Client wiring ───────────────────────────────────────────────────────

/// `MarketDataClient` with a pool of `width` channels to the mock —
/// `auto_plan` sizes the fan-out from the pool, so `width = 2` yields a
/// two-band plan for a two-day range. Retry backoff is shrunk to keep
/// the recovery tests fast; the budget (3 attempts) stays real.
async fn client_for_mock(mock: &BandMockServer, width: usize) -> MarketDataClient {
    let mut channels = Vec::with_capacity(width);
    for _ in 0..width {
        channels.push(
            Channel::connect_h2c("127.0.0.1", mock.addr.port())
                .await
                .expect("h2c connect to mock"),
        );
    }
    let pool = ChannelPool::from_channels(channels);
    let mut cfg = DirectConfig::production();
    let mut retry = RetryPolicy::default();
    retry.initial_delay = Duration::from_millis(1);
    retry.max_delay = Duration::from_millis(5);
    retry.max_attempts = 3;
    retry.jitter = false;
    cfg.retry = retry;
    let sem = Arc::new(Semaphore::new(width));
    MarketDataClient::for_endpoint_routing_test(cfg, pool, sem)
}

/// The two-day query every test issues: `auto_plan` cuts it into one
/// date band per day at pool width 2.
const DAY1: &str = "20240101";
const DAY2: &str = "20240102";

fn day1_rows() -> Vec<Row> {
    vec![(34_200_000, 101, 20_240_101), (34_200_500, 102, 20_240_101)]
}

fn day2_rows() -> Vec<Row> {
    vec![(34_200_250, 201, 20_240_102), (34_201_000, 202, 20_240_102)]
}

fn prices(ticks: &[thetadatadx::TradeTick]) -> Vec<i64> {
    ticks.iter().map(|t| t.price as i64).collect()
}

// ─── Buffered `.await` path ──────────────────────────────────────────────

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn buffered_sharded_pull_fans_out_and_merges_in_band_order() {
    let script = MockScript::new(vec![
        (DAY1, vec![BandResponse::ok(vec![day1_rows()])]),
        (DAY2, vec![BandResponse::ok(vec![day2_rows()])]),
    ]);
    let mock = spawn_band_mock(Arc::clone(&script)).await;
    let client = client_for_mock(&mock, 2).await;

    let ticks = client
        .stock_history_trade("AAPL")
        .start_date(DAY1)
        .end_date(DAY2)
        .await
        .expect("sharded buffered pull");

    // The fan-out issued exactly one request per band, one band per day.
    assert_eq!(
        script.seen_bands(),
        vec![DAY1.to_string(), DAY2.to_string()]
    );
    // The merge concatenates the bands in band order — the single-stream
    // row order for a stock pull (bands partition the date axis).
    assert_eq!(prices(&ticks), vec![101, 102, 201, 202]);
    assert_eq!(ticks[0].date, 20_240_101);
    assert_eq!(ticks[3].date, 20_240_102);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn buffered_sharded_pull_folds_an_empty_band() {
    // One band empty (pre-stream NotFound): it contributes zero rows and
    // the union — the sibling band's rows — comes back clean.
    let script = MockScript::new(vec![
        (DAY1, vec![BandResponse::ok(vec![day1_rows()])]),
        (DAY2, vec![BandResponse::not_found()]),
    ]);
    let mock = spawn_band_mock(Arc::clone(&script)).await;
    let client = client_for_mock(&mock, 2).await;

    let ticks = client
        .stock_history_trade("AAPL")
        .start_date(DAY1)
        .end_date(DAY2)
        .await
        .expect("empty band folds, sibling has data");

    assert_eq!(prices(&ticks), vec![101, 102]);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn buffered_sharded_pull_refetches_a_failed_band_from_scratch() {
    // Band 2 delivers a partial chunk and then dies with a transient
    // error; its rows never reached the caller, so the band re-fetches
    // from scratch (attempt-local buffer discarded) and the merged
    // result carries the full band exactly once — no loss, no
    // duplicates, no visible hiccup.
    let script = MockScript::new(vec![
        (DAY1, vec![BandResponse::ok(vec![day1_rows()])]),
        (
            DAY2,
            vec![
                BandResponse::unavailable_after(vec![vec![(34_200_250, 201, 20_240_102)]]),
                BandResponse::ok(vec![day2_rows()]),
            ],
        ),
    ]);
    let mock = spawn_band_mock(Arc::clone(&script)).await;
    let client = client_for_mock(&mock, 2).await;

    let ticks = client
        .stock_history_trade("AAPL")
        .start_date(DAY1)
        .end_date(DAY2)
        .await
        .expect("failed band re-fetches within its retry budget");

    assert_eq!(
        script.attempts(DAY2),
        2,
        "the failed band must be re-fetched once"
    );
    assert_eq!(
        prices(&ticks),
        vec![101, 102, 201, 202],
        "partial first attempt discarded — full band exactly once"
    );
}

// ─── Streaming `.stream(handler)` path ───────────────────────────────────

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn streaming_sharded_pull_forwards_every_bands_chunks() {
    let script = MockScript::new(vec![
        (DAY1, vec![BandResponse::ok(vec![day1_rows()])]),
        (DAY2, vec![BandResponse::ok(vec![day2_rows()])]),
    ]);
    let mock = spawn_band_mock(Arc::clone(&script)).await;
    let client = client_for_mock(&mock, 2).await;

    let sink: Arc<Mutex<Vec<i64>>> = Arc::new(Mutex::new(Vec::new()));
    let seen = Arc::clone(&sink);
    client
        .stock_history_trade("AAPL")
        .start_date(DAY1)
        .end_date(DAY2)
        .stream(move |ticks| {
            seen.lock().unwrap().extend(prices(ticks));
        })
        .await
        .expect("sharded streaming pull");

    assert_eq!(
        script.seen_bands(),
        vec![DAY1.to_string(), DAY2.to_string()]
    );
    // Chunks interleave across bands in arrival order; the union must be
    // every row exactly once.
    let mut rows = std::mem::take(&mut *sink.lock().unwrap());
    rows.sort_unstable();
    assert_eq!(rows, vec![101, 102, 201, 202]);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn streaming_sharded_pull_reports_the_failed_band_window() {
    // Band 2 hands the handler a chunk and then fails terminally (a
    // mid-delivery transient cannot replay — no resume token). The
    // sibling band must drain to completion, and the error must name
    // band 2's exact window so the caller can re-pull that slice.
    let script = MockScript::new(vec![
        (DAY1, vec![BandResponse::ok(vec![day1_rows()])]),
        (
            DAY2,
            vec![BandResponse::unavailable_after(vec![vec![(
                34_200_250, 201, 20_240_102,
            )]])],
        ),
    ]);
    let mock = spawn_band_mock(Arc::clone(&script)).await;
    let client = client_for_mock(&mock, 2).await;

    let sink: Arc<Mutex<Vec<i64>>> = Arc::new(Mutex::new(Vec::new()));
    let seen = Arc::clone(&sink);
    let err = client
        .stock_history_trade("AAPL")
        .start_date(DAY1)
        .end_date(DAY2)
        .stream(move |ticks| {
            seen.lock().unwrap().extend(prices(ticks));
        })
        .await
        .expect_err("a mid-delivery band failure must surface");

    let Error::PartialShardFetch { failed } = err else {
        panic!("expected PartialShardFetch, got {err:?}");
    };
    assert_eq!(
        failed,
        vec![ShardBand::Date {
            start_date: DAY2.to_string(),
            end_date: DAY2.to_string(),
        }],
        "the failed band's exact window is named"
    );
    assert_eq!(
        script.attempts(DAY2),
        1,
        "a mid-delivery transient must not replay the band"
    );
    let mut rows = std::mem::take(&mut *sink.lock().unwrap());
    rows.sort_unstable();
    assert_eq!(
        rows,
        vec![101, 102, 201],
        "the surviving band drains fully; the failed band's delivered prefix stays delivered"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn streaming_sharded_pull_retries_a_band_that_fails_before_delivery() {
    // A band that fails BEFORE any chunk reaches the handler keeps its
    // transparent retry: attempt 1 dies pre-delivery, attempt 2 serves
    // the band, and the caller sees one clean stream.
    let script = MockScript::new(vec![
        (DAY1, vec![BandResponse::ok(vec![day1_rows()])]),
        (
            DAY2,
            vec![
                BandResponse::unavailable_after(vec![]),
                BandResponse::ok(vec![day2_rows()]),
            ],
        ),
    ]);
    let mock = spawn_band_mock(Arc::clone(&script)).await;
    let client = client_for_mock(&mock, 2).await;

    let sink: Arc<Mutex<Vec<i64>>> = Arc::new(Mutex::new(Vec::new()));
    let seen = Arc::clone(&sink);
    client
        .stock_history_trade("AAPL")
        .start_date(DAY1)
        .end_date(DAY2)
        .stream(move |ticks| {
            seen.lock().unwrap().extend(prices(ticks));
        })
        .await
        .expect("pre-delivery band failure retries transparently");

    assert_eq!(script.attempts(DAY2), 2);
    let mut rows = std::mem::take(&mut *sink.lock().unwrap());
    rows.sort_unstable();
    assert_eq!(rows, vec![101, 102, 201, 202]);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn streaming_sharded_pull_folds_an_empty_band() {
    let script = MockScript::new(vec![
        (DAY1, vec![BandResponse::ok(vec![day1_rows()])]),
        (DAY2, vec![BandResponse::not_found()]),
    ]);
    let mock = spawn_band_mock(Arc::clone(&script)).await;
    let client = client_for_mock(&mock, 2).await;

    let sink: Arc<Mutex<Vec<i64>>> = Arc::new(Mutex::new(Vec::new()));
    let seen = Arc::clone(&sink);
    client
        .stock_history_trade("AAPL")
        .start_date(DAY1)
        .end_date(DAY2)
        .stream(move |ticks| {
            seen.lock().unwrap().extend(prices(ticks));
        })
        .await
        .expect("empty band folds, sibling has data");

    let mut rows = std::mem::take(&mut *sink.lock().unwrap());
    rows.sort_unstable();
    assert_eq!(rows, vec![101, 102]);
}
