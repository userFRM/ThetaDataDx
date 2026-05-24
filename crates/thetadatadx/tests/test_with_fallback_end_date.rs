//! Regression coverage for the `_with_fallback` shims:
//!
//! * `end_date` is forwarded to the REST builder (quote, IV,
//!   first-order Greeks).
//! * The REST arm honours the tier-clamp semaphore: a config with
//!   `concurrent_requests = 1` parks the second concurrent
//!   `_with_fallback` call until the first completes.
//! * The gRPC arm reaches the wire — exercised against the existing
//!   `grpc_mock_server` h2 fixture — confirming `FallbackPolicy::Disabled`
//!   routes through the macro-generated builder, where `end_date` is
//!   threaded by the explicit `builder.end_date(e)` calls.

mod grpc_mock_server;

use std::sync::Arc;
use std::time::{Duration, Instant};

use prost::Message;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{Notify, Semaphore};
use tokio::task::JoinHandle;

use thetadatadx::config::FallbackPolicy;
use thetadatadx::grpc::{Channel, ChannelPool};
use thetadatadx::mdds::MddsClient;
use thetadatadx::wire::ResponseData;
use thetadatadx::DirectConfig;

use grpc_mock_server::MockServer;

/// Raw-TCP HTTP/1 mock that records every inbound request's URL query
/// string. The handler hangs until `release` is notified, which lets
/// the tier-clamp test observe back-to-back calls serialising on the
/// semaphore. The `entered` notify fires once the request line has
/// been parsed and the query has been captured, so callers can wait
/// deterministically for "first call reached mock" instead of
/// sleeping.
struct RestMock {
    addr: std::net::SocketAddr,
    captured: Arc<tokio::sync::Mutex<Vec<String>>>,
    release: Arc<Notify>,
    entered: Arc<Notify>,
    task: Option<JoinHandle<()>>,
}

impl RestMock {
    async fn spawn(body: &'static str, hold_until_release: bool) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind to ephemeral port");
        let addr = listener.local_addr().expect("read local addr");
        let captured: Arc<tokio::sync::Mutex<Vec<String>>> = Arc::new(Default::default());
        let release = Arc::new(Notify::new());
        let entered = Arc::new(Notify::new());
        let captured_loop = Arc::clone(&captured);
        let release_loop = Arc::clone(&release);
        let entered_loop = Arc::clone(&entered);
        let task = tokio::spawn(async move {
            loop {
                let (socket, _) = match listener.accept().await {
                    Ok(s) => s,
                    Err(_) => return,
                };
                let captured = Arc::clone(&captured_loop);
                let release = Arc::clone(&release_loop);
                let entered = Arc::clone(&entered_loop);
                tokio::spawn(async move {
                    let _ =
                        handle_one(socket, captured, release, entered, body, hold_until_release)
                            .await;
                });
            }
        });
        Self {
            addr,
            captured,
            release,
            entered,
            task: Some(task),
        }
    }

    fn base_url(&self) -> String {
        format!("http://{}", self.addr)
    }

    async fn captured_queries(&self) -> Vec<String> {
        self.captured.lock().await.clone()
    }

    /// Wait for the next handler to reach the post-query-capture
    /// barrier. Resolves once *some* in-flight handler has logged its
    /// request line into `captured`. Used by the tier-clamp test to
    /// pin "first call reached mock" without a sleep race.
    async fn wait_for_entry(&self) {
        self.entered.notified().await;
    }

    fn release_all(&self) {
        // Notify enough waiters that every parked handler observes the
        // signal; extra notifies are harmless.
        for _ in 0..16 {
            self.release.notify_one();
        }
    }
}

impl Drop for RestMock {
    fn drop(&mut self) {
        if let Some(t) = self.task.take() {
            t.abort();
        }
    }
}

/// Read the inbound HTTP/1 request, push its query string to `captured`,
/// optionally park on `release`, then emit a fixed 200 response with
/// `body` as the body. The implementation honours only the small subset
/// of HTTP/1 the `reqwest` client emits (single connection, no
/// keep-alive carry-over, headers separated from the empty `\r\n`).
async fn handle_one(
    mut socket: TcpStream,
    captured: Arc<tokio::sync::Mutex<Vec<String>>>,
    release: Arc<Notify>,
    entered: Arc<Notify>,
    body: &'static str,
    hold_until_release: bool,
) -> std::io::Result<()> {
    let _ = socket.set_nodelay(true);
    let mut buf = Vec::with_capacity(4 * 1024);
    let mut scratch = [0u8; 1024];
    let headers_end = loop {
        let n = socket.read(&mut scratch).await?;
        if n == 0 {
            return Ok(());
        }
        buf.extend_from_slice(&scratch[..n]);
        if let Some(idx) = find_subsequence(&buf, b"\r\n\r\n") {
            break idx;
        }
        if buf.len() > 64 * 1024 {
            return Ok(());
        }
    };
    let headers = std::str::from_utf8(&buf[..headers_end]).unwrap_or("");
    let request_line = headers.lines().next().unwrap_or("");
    let target = request_line.split_whitespace().nth(1).unwrap_or("");
    let query = target.split_once('?').map(|(_, q)| q).unwrap_or("");
    captured.lock().await.push(query.to_string());
    // Fire the entry barrier AFTER the query is captured. Test
    // callers awaiting `RestMock::wait_for_entry` are guaranteed to
    // see a non-empty `captured` snapshot when they wake.
    entered.notify_one();
    if hold_until_release {
        release.notified().await;
    }
    let response = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: text/csv\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        body.len(),
        body
    );
    socket.write_all(response.as_bytes()).await?;
    socket.shutdown().await?;
    Ok(())
}

fn find_subsequence(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|w| w == needle)
}

/// Build a `DirectConfig::production()` skeleton with the supplied
/// fallback policy. Skips the auth + transport details that the
/// fallback test does not exercise.
fn config_with_fallback(policy: FallbackPolicy) -> DirectConfig {
    let mut cfg = DirectConfig::production();
    cfg.fallback = policy;
    cfg
}

/// Spin up a gRPC mock that returns an empty quote-tick response.
/// The fallback shims only invoke this when `FallbackPolicy::Disabled`;
/// the REST tests still need a constructable channel pool because
/// `ChannelPool::from_channels` panics on an empty `Vec`.
async fn dummy_grpc_pool() -> (MockServer, ChannelPool) {
    let mock = MockServer::spawn(empty_quote_chunks(), 0).await;
    let channel = Channel::connect_h2c("127.0.0.1", mock.addr.port())
        .await
        .expect("h2c connect");
    let pool = ChannelPool::from_channels(vec![channel]);
    (mock, pool)
}

/// Empty-quote `ResponseData` chunks so the gRPC mock can return a
/// well-formed but row-less response.
fn empty_quote_chunks() -> Vec<ResponseData> {
    use thetadatadx::wire::{CompressionAlgo, CompressionDescription, DataTable};
    let table = DataTable {
        headers: vec![
            "ms_of_day".into(),
            "bid_size".into(),
            "bid_exchange".into(),
            "bid".into(),
            "bid_condition".into(),
            "ask_size".into(),
            "ask_exchange".into(),
            "ask".into(),
            "ask_condition".into(),
            "date".into(),
        ],
        data_table: vec![],
    };
    let bytes = table.encode_to_vec();
    vec![ResponseData {
        compressed_data: bytes.clone(),
        compression_description: Some(CompressionDescription {
            algo: CompressionAlgo::None as i32,
            level: 0,
        }),
        original_size: i32::try_from(bytes.len()).unwrap_or(0),
    }]
}

// ───── REST arm: `end_date` forwarding ─────────────────────────────

/// IV `_with_fallback` forwards `end_date` on the REST arm.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn iv_with_fallback_rest_arm_forwards_end_date() {
    static EMPTY_IV_CSV: &str = "ms_of_day,bid,ask,iv,underlying_price,date\n";
    let rest = RestMock::spawn(EMPTY_IV_CSV, false).await;
    let (_grpc, channels) = dummy_grpc_pool().await;
    let cfg = config_with_fallback(FallbackPolicy::RestAlways {
        base_url: rest.base_url(),
    });
    let sem = Arc::new(Semaphore::new(4));
    let client = MddsClient::for_fallback_test(cfg, channels, sem);
    let _ = client
        .option_history_greeks_implied_volatility_with_fallback(
            "AAPL",
            "20240920",
            "20240916",
            Some("20240920"),
            None,
            None,
            None,
        )
        .await
        .expect("REST mock returns 200");
    let queries = rest.captured_queries().await;
    assert_eq!(queries.len(), 1, "expected exactly one REST hit");
    assert!(
        queries[0].contains("end_date=20240920"),
        "REST arm dropped end_date: {}",
        queries[0]
    );
}

/// First-order Greeks `_with_fallback` forwards `end_date` on the REST arm.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn first_order_with_fallback_rest_arm_forwards_end_date() {
    static EMPTY_GREEKS_CSV: &str =
        "ms_of_day,bid,ask,delta,gamma,theta,vega,rho,iv,underlying_price,date\n";
    let rest = RestMock::spawn(EMPTY_GREEKS_CSV, false).await;
    let (_grpc, channels) = dummy_grpc_pool().await;
    let cfg = config_with_fallback(FallbackPolicy::RestAlways {
        base_url: rest.base_url(),
    });
    let sem = Arc::new(Semaphore::new(4));
    let client = MddsClient::for_fallback_test(cfg, channels, sem);
    let _ = client
        .option_history_greeks_first_order_with_fallback(
            "AAPL",
            "20240920",
            "20240916",
            Some("20240924"),
            None,
            None,
            None,
        )
        .await
        .expect("REST mock returns 200");
    let queries = rest.captured_queries().await;
    assert_eq!(queries.len(), 1);
    assert!(
        queries[0].contains("end_date=20240924"),
        "REST arm dropped end_date: {}",
        queries[0]
    );
}

// ───── gRPC arm: outbound request pins `end_date` on the wire
//
// `FallbackPolicy::Disabled` routes the `_with_fallback` shims down
// the gRPC builder path. PR #592 fixed a missed `builder.end_date(e)`
// call in that path; these tests pin the regression by capturing the
// outbound protobuf bytes at the mock and decoding the request proto
// to assert the `end_date` field is present and carries the value the
// caller passed.

use std::sync::Mutex;
use thetadatadx::wire::test_requests::{
    OptionHistoryGreeksFirstOrderRequest, OptionHistoryGreeksImpliedVolatilityRequest,
};

/// Build a `MockBehaviour` whose request-capture hook writes into the
/// supplied buffer. Caller-owned `Arc` so the test reads the captured
/// bytes off its own clone after the RPC completes.
fn capture_behaviour(target: Arc<Mutex<Vec<u8>>>) -> grpc_mock_server::MockBehaviour {
    grpc_mock_server::MockBehaviour {
        capture_request_bytes: Some(target),
        ..grpc_mock_server::MockBehaviour::default()
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn iv_with_fallback_grpc_arm_forwards_end_date() {
    let captured: Arc<Mutex<Vec<u8>>> = Arc::new(Mutex::new(Vec::new()));
    let mock = MockServer::spawn_with_behaviour(
        empty_quote_chunks(),
        0,
        String::new(),
        capture_behaviour(Arc::clone(&captured)),
    )
    .await;
    let channel = Channel::connect_h2c("127.0.0.1", mock.addr.port())
        .await
        .expect("h2c connect");
    let channels = ChannelPool::from_channels(vec![channel]);
    let cfg = config_with_fallback(FallbackPolicy::Disabled);
    let sem = Arc::new(Semaphore::new(4));
    let client = MddsClient::for_fallback_test(cfg, channels, sem);
    let _ = client
        .option_history_greeks_implied_volatility_with_fallback(
            "AAPL",
            "20240920",
            "20240916",
            Some("20240920"),
            None,
            None,
            None,
        )
        .await;
    let payload = captured.lock().expect("captured mutex poisoned").clone();
    assert!(
        !payload.is_empty(),
        "mock did not capture the outbound request body"
    );
    let request = OptionHistoryGreeksImpliedVolatilityRequest::decode(payload.as_slice())
        .expect("decode IV request proto");
    let params = request.params.expect("IV request has params");
    assert_eq!(
        params.end_date.as_deref(),
        Some("20240920"),
        "IV gRPC arm dropped end_date from the outbound request"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn first_order_with_fallback_grpc_arm_forwards_end_date() {
    let captured: Arc<Mutex<Vec<u8>>> = Arc::new(Mutex::new(Vec::new()));
    let mock = MockServer::spawn_with_behaviour(
        empty_quote_chunks(),
        0,
        String::new(),
        capture_behaviour(Arc::clone(&captured)),
    )
    .await;
    let channel = Channel::connect_h2c("127.0.0.1", mock.addr.port())
        .await
        .expect("h2c connect");
    let channels = ChannelPool::from_channels(vec![channel]);
    let cfg = config_with_fallback(FallbackPolicy::Disabled);
    let sem = Arc::new(Semaphore::new(4));
    let client = MddsClient::for_fallback_test(cfg, channels, sem);
    let _ = client
        .option_history_greeks_first_order_with_fallback(
            "AAPL",
            "20240920",
            "20240916",
            Some("20240924"),
            None,
            None,
            None,
        )
        .await;
    let payload = captured.lock().expect("captured mutex poisoned").clone();
    assert!(
        !payload.is_empty(),
        "mock did not capture the outbound request body"
    );
    let request = OptionHistoryGreeksFirstOrderRequest::decode(payload.as_slice())
        .expect("decode first-order request proto");
    let params = request.params.expect("first-order request has params");
    assert_eq!(
        params.end_date.as_deref(),
        Some("20240924"),
        "first-order gRPC arm dropped end_date from the outbound request"
    );
}

// ───── REST arm: tier-clamp semaphore ──────────────────────────────

/// With `concurrent_requests = 1` the second concurrent
/// `option_history_quote_with_fallback` call parks on the semaphore
/// until the first completes.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn rest_arm_respects_tier_clamp_semaphore() {
    static EMPTY_QUOTE_CSV: &str =
        "ms_of_day,bid_size,bid_exchange,bid,bid_condition,ask_size,ask_exchange,ask,ask_condition,date\n";
    let rest = RestMock::spawn(EMPTY_QUOTE_CSV, true).await;
    let (_grpc, channels) = dummy_grpc_pool().await;
    let cfg = config_with_fallback(FallbackPolicy::RestAlways {
        base_url: rest.base_url(),
    });
    let sem = Arc::new(Semaphore::new(1));
    let client = Arc::new(MddsClient::for_fallback_test(cfg, channels, sem));

    let started = Instant::now();
    let c1 = Arc::clone(&client);
    let c2 = Arc::clone(&client);
    let t1 = tokio::spawn(async move {
        c1.option_history_quote_with_fallback(
            "AAPL", "20240920", "20240916", None, None, None, None,
        )
        .await
    });
    let t2 = tokio::spawn(async move {
        c2.option_history_quote_with_fallback(
            "AAPL", "20240920", "20240916", None, None, None, None,
        )
        .await
    });

    // Wait deterministically for the first call to land at the mock —
    // the handler parks on `release` after capturing the query, so
    // the entry-side barrier resolves before any drain happens. A
    // generous 5s timeout protects against runtime stalls without
    // racing the semaphore on a slow runner.
    tokio::time::timeout(Duration::from_secs(5), rest.wait_for_entry())
        .await
        .expect("first REST call reached the mock");
    let captured = rest.captured_queries().await.len();
    assert_eq!(
        captured, 1,
        "tier-clamp let two REST calls through concurrently: {captured} captured"
    );

    rest.release_all();
    let _ = t1.await.expect("task1 join").expect("rest call 1");
    let _ = t2.await.expect("task2 join").expect("rest call 2");
    let _elapsed = started.elapsed();
    let captured = rest.captured_queries().await.len();
    assert_eq!(captured, 2, "both calls should ultimately reach the mock");
    // The strong tier-clamp invariant is asserted above: after the
    // first call lands at the mock and BEFORE we release it, only one
    // query was captured. Releasing then lets the second through. The
    // prior `elapsed >= 1ms` floor was trivially satisfied by any
    // network handshake and proved nothing about serialization.
    // Per audit S38, the entry-side barrier IS the serialization
    // proof — we keep `_elapsed` bound only as a debug-print hook.
}
