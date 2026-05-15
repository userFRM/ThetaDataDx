//! Mock h2 server that speaks the gRPC wire protocol against the
//! in-house `grpc::Channel`.
//!
//! The mock accepts one HTTP/2 stream, drains the framed request
//! payload, then writes back N hardcoded `ResponseData` chunks
//! followed by `grpc-status: N` trailers. It serves two callers:
//!
//! - the channel integration tests below, which exercise the full
//!   `connect_h2c` → `server_streaming` → `Stream::next` path with no
//!   external dependencies;
//! - the criterion bench in `benches/grpc_channel.rs`, which times
//!   the in-house path against the same mock.
//!
//! Each `MockServer` instance handles exactly one RPC and is dropped
//! by the test owning it — sufficient for per-test isolation.

use std::net::SocketAddr;
use std::time::Duration;

use bytes::{BufMut, Bytes, BytesMut};
use h2::server::SendResponse;
use http::{HeaderMap, HeaderName, HeaderValue, Response, StatusCode};
use prost::Message;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::oneshot;
use tokio::task::JoinHandle;
use tokio_stream::StreamExt;

use thetadatadx::grpc::{Channel, ChannelError, ChannelPool};
use thetadatadx::wire::{data_value, DataValue, DataValueList, ResponseData};

/// Compose a length-prefix gRPC frame from a protobuf message.
fn frame<M: Message>(msg: &M) -> Bytes {
    let payload = msg.encode_to_vec();
    let mut buf = BytesMut::with_capacity(5 + payload.len());
    buf.put_u8(0);
    buf.put_u32(u32::try_from(payload.len()).unwrap());
    buf.extend_from_slice(&payload);
    buf.freeze()
}

/// Build a `ResponseData` carrying a `DataValueList` row of symbols
/// in its `compressed_data` field. The real server zstd-compresses
/// the inner payload; the bench/test bypasses that step and asserts on
/// the framed protobuf alone.
fn make_response_data(symbols: &[&str]) -> ResponseData {
    let list = DataValueList {
        values: symbols
            .iter()
            .map(|s| DataValue {
                data_type: Some(data_value::DataType::Text((*s).to_string())),
            })
            .collect(),
    };
    ResponseData {
        compressed_data: list.encode_to_vec(),
        ..ResponseData::default()
    }
}

/// Handle to a running mock server. Drops the listener task on drop.
pub struct MockServer {
    pub addr: SocketAddr,
    shutdown: Option<oneshot::Sender<()>>,
    task: Option<JoinHandle<()>>,
}

/// Behaviour switches for the mock h2 server beyond the default
/// "drain request, emit chunks, close with grpc-status".
#[derive(Clone, Default)]
pub struct MockBehaviour {
    /// When `Some`, the mock sleeps this long after accepting the
    /// stream before sending DATA frames. Forces the client into a
    /// pending state to exercise deadlines and cancellation.
    pub pre_response_delay: Option<Duration>,
    /// When true, the mock sends GOAWAY (abrupt shutdown) instead of
    /// completing the RPC normally. Exercises connection-level error
    /// classification on the client.
    pub goaway_mid_stream: bool,
    /// When true, the mock sends a trailers-only response: the initial
    /// HEADERS frame carries `grpc-status` (and `grpc-message`) directly,
    /// no DATA frames, no separate trailing HEADERS frame. This is the
    /// legal gRPC encoding for an immediate error reply — every gRPC
    /// client must classify it from the response head, not the body.
    pub trailers_only: bool,
    /// When true, the mock decodes the inbound framed request body and
    /// asserts equality against the expected protobuf payload bytes set
    /// in `expected_request`. Lets tests confirm the client encoded the
    /// request correctly without re-parsing in the test body.
    pub assert_request_bytes: bool,
    /// Expected inbound request payload (after stripping the 5-byte gRPC
    /// frame prefix). Only consulted when `assert_request_bytes = true`.
    pub expected_request: Vec<u8>,
    /// When `Some`, the mock asserts the inbound request's `:scheme`
    /// pseudo-header matches this value (e.g. `"http"` for h2c,
    /// `"https"` for TLS). Tests use this to confirm the client
    /// records the right scheme for each transport — the gRPC spec
    /// pins `:scheme` to the underlying transport, and strict L7
    /// proxies / routers reject the mismatch.
    pub assert_scheme: Option<&'static str>,
    /// When `Some`, the mock sends a per-stream `RST_STREAM` with the
    /// supplied reason after the request body is fully received,
    /// without ever flushing a response head. The h2 connection
    /// itself stays open; only this stream is killed. Clients must
    /// classify the resulting error as stream-level
    /// (`ChannelError::H2Stream`), not connection-level
    /// (`ChannelError::ConnectionClosed`) — the pool would otherwise
    /// recycle a still-healthy channel.
    pub stream_reset_reason: Option<h2::Reason>,
    /// When `Some`, the mock advertises this `MAX_CONCURRENT_STREAMS`
    /// SETTINGS value during handshake. Tests use a value of 1 to
    /// saturate a channel with a single slow call, then confirm the
    /// pool routes around it via `has_capacity` rather than blocking.
    pub max_concurrent_streams: Option<u32>,
}

impl MockServer {
    /// Spin up a mock that responds with `chunks` framed `ResponseData`
    /// messages then closes the stream with `grpc-status: status_code`.
    pub async fn spawn(chunks: Vec<ResponseData>, status_code: u32) -> Self {
        Self::spawn_with_message(chunks, status_code, String::new()).await
    }

    /// Variant that also sets `grpc-message` on the trailing trailers.
    pub async fn spawn_with_message(
        chunks: Vec<ResponseData>,
        status_code: u32,
        status_message: String,
    ) -> Self {
        Self::spawn_with_behaviour(
            chunks,
            status_code,
            status_message,
            MockBehaviour::default(),
        )
        .await
    }

    /// Most general spawn — explicit behaviour switches.
    pub async fn spawn_with_behaviour(
        chunks: Vec<ResponseData>,
        status_code: u32,
        status_message: String,
        behaviour: MockBehaviour,
    ) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind to ephemeral port");
        let addr = listener.local_addr().expect("read local addr");
        let (tx, rx) = oneshot::channel();

        let task = tokio::spawn(async move {
            tokio::select! {
                _ = rx => {},
                accept = run(listener, chunks, status_code, status_message, behaviour) => {
                    if let Err(e) = accept {
                        eprintln!("grpc_mock_server: accept loop ended: {e}");
                    }
                }
            }
        });

        Self {
            addr,
            shutdown: Some(tx),
            task: Some(task),
        }
    }
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

/// Accept exactly one TCP connection, run an h2 handshake on it,
/// service one RPC, then return.
async fn run(
    listener: TcpListener,
    chunks: Vec<ResponseData>,
    status_code: u32,
    status_message: String,
    behaviour: MockBehaviour,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let (socket, _peer) = listener.accept().await?;
    let _ = socket.set_nodelay(true);
    serve_one_connection(socket, chunks, status_code, status_message, behaviour).await
}

async fn serve_one_connection(
    socket: TcpStream,
    chunks: Vec<ResponseData>,
    status_code: u32,
    status_message: String,
    behaviour: MockBehaviour,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let mut builder = h2::server::Builder::new();
    if let Some(max) = behaviour.max_concurrent_streams {
        builder.max_concurrent_streams(max);
    }
    let mut connection = builder.handshake(socket).await?;
    // Drive the connection until either (a) an RPC is served and the
    // client closes, or (b) the connection itself shuts down. The
    // request handler runs on a separate task so the accept loop can
    // continue advancing the h2 connection state machine while DATA
    // and trailers flush.
    while let Some(request_result) = connection.accept().await {
        let (request, respond) = request_result?;
        let chunks = chunks.clone();
        let status_message = status_message.clone();
        let behaviour_inner = behaviour.clone();
        let (handler_done_tx, handler_done_rx) = oneshot::channel::<()>();
        tokio::spawn(async move {
            if let Err(e) = handle_request(
                request,
                respond,
                chunks,
                status_code,
                status_message,
                behaviour_inner,
            )
            .await
            {
                eprintln!("grpc_mock_server: request handler failed: {e}");
            }
            let _ = handler_done_tx.send(());
        });
        if behaviour.goaway_mid_stream {
            // Wait until the handler has emitted its chunks (it skips
            // trailers in goaway mode), then abrupt-shutdown the
            // connection. The client surfaces this as
            // `ChannelError::ConnectionClosed` rather than a
            // stream-level reset.
            let _ = handler_done_rx.await;
            connection.abrupt_shutdown(h2::Reason::NO_ERROR);
        }
    }
    Ok(())
}

async fn handle_request(
    request: http::Request<h2::RecvStream>,
    mut respond: SendResponse<Bytes>,
    chunks: Vec<ResponseData>,
    status_code: u32,
    status_message: String,
    behaviour: MockBehaviour,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    if let Some(expected_scheme) = behaviour.assert_scheme {
        let scheme = request.uri().scheme_str().unwrap_or("");
        assert_eq!(
            scheme, expected_scheme,
            "inbound :scheme pseudo-header does not match expected transport"
        );
    }
    // Drain (and optionally validate) the request body so flow-control
    // accounting mirrors a real gRPC server. When `assert_request_bytes`
    // is set, we re-assemble the gRPC frame and confirm the inner
    // protobuf payload matches what the test sent — gives the test
    // suite end-to-end confidence the client encoded the request
    // correctly without re-parsing in the test body.
    let mut body = request.into_body();
    let mut request_buf: Vec<u8> = Vec::new();
    while let Some(chunk) = body.data().await {
        let chunk = chunk?;
        let _ = body.flow_control().release_capacity(chunk.len());
        if behaviour.assert_request_bytes {
            request_buf.extend_from_slice(&chunk);
        }
    }
    if behaviour.assert_request_bytes {
        // gRPC frame layout: 1 compressed flag + 4 big-endian length + payload.
        assert!(
            request_buf.len() >= 5,
            "request body shorter than gRPC frame header: {} bytes",
            request_buf.len()
        );
        let declared = u32::from_be_bytes([
            request_buf[1],
            request_buf[2],
            request_buf[3],
            request_buf[4],
        ]) as usize;
        let payload = &request_buf[5..];
        assert_eq!(
            payload.len(),
            declared,
            "declared frame length {} does not match payload {}",
            declared,
            payload.len()
        );
        assert_eq!(
            payload,
            &behaviour.expected_request[..],
            "request payload does not match expected protobuf bytes"
        );
    }
    if let Some(d) = behaviour.pre_response_delay {
        tokio::time::sleep(d).await;
    }
    if behaviour.goaway_mid_stream {
        // Send response head + one DATA chunk to get the stream
        // running, then exit without trailers so the outer
        // accept-loop fires GOAWAY mid-stream.
        respond_partial_then_drop(respond, &chunks).await?;
        return Ok(());
    }
    if let Some(reason) = behaviour.stream_reset_reason {
        // Per-stream RST_STREAM with the supplied reason. The h2
        // connection stays open; only this stream is killed. `h2`'s
        // `send_reset` must be called before any `send_response`.
        respond.send_reset(reason);
        return Ok(());
    }
    if behaviour.trailers_only {
        // gRPC trailers-only encoding: `grpc-status` (and optional
        // `grpc-message`) live on the response HEADERS frame, with
        // END_STREAM set. No DATA frames, no trailing HEADERS frame.
        respond_trailers_only(respond, status_code, &status_message)?;
        return Ok(());
    }
    respond_chunks(respond, &chunks, status_code, &status_message)?;
    Ok(())
}

/// Send a trailers-only gRPC response: HTTP 200 with `grpc-status`
/// (and optional `grpc-message`) on the initial HEADERS frame, no body.
fn respond_trailers_only(
    mut respond: SendResponse<Bytes>,
    status_code: u32,
    status_message: &str,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let mut response = Response::new(());
    *response.status_mut() = StatusCode::OK;
    response.headers_mut().insert(
        http::header::CONTENT_TYPE,
        HeaderValue::from_static("application/grpc+proto"),
    );
    response.headers_mut().insert(
        HeaderName::from_static("grpc-status"),
        HeaderValue::from_str(&status_code.to_string()).expect("status is numeric ASCII"),
    );
    if !status_message.is_empty() {
        response.headers_mut().insert(
            HeaderName::from_static("grpc-message"),
            HeaderValue::from_str(status_message).expect("status message is ASCII"),
        );
    }
    // `end_of_stream = true` makes this a HEADERS-only response with
    // END_STREAM. h2 emits it as a single frame.
    let _send_stream = respond.send_response(response, true)?;
    Ok(())
}

/// Send the response head and one optional chunk, then drop the
/// `SendResponse` without trailers so the outer accept loop can issue
/// GOAWAY mid-stream. Used by the GOAWAY classification test.
async fn respond_partial_then_drop(
    mut respond: SendResponse<Bytes>,
    chunks: &[ResponseData],
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let mut response = Response::new(());
    *response.status_mut() = StatusCode::OK;
    response.headers_mut().insert(
        http::header::CONTENT_TYPE,
        HeaderValue::from_static("application/grpc+proto"),
    );
    let mut send_stream = respond.send_response(response, false)?;
    if let Some(first) = chunks.first() {
        send_stream.send_data(frame(first), false)?;
    }
    // Give h2 a tick to actually flush the response head + DATA to the
    // wire before the outer loop tears the connection down. Without
    // this, abrupt_shutdown can race the response-head emission and
    // the client sees an IO error instead of a clean GOAWAY.
    tokio::time::sleep(Duration::from_millis(50)).await;
    // Deliberately drop without sending trailers. The connection-level
    // GOAWAY follows on the outer task.
    Ok(())
}

fn respond_chunks(
    mut respond: SendResponse<Bytes>,
    chunks: &[ResponseData],
    status_code: u32,
    status_message: &str,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let mut response = Response::new(());
    *response.status_mut() = StatusCode::OK;
    response.headers_mut().insert(
        http::header::CONTENT_TYPE,
        HeaderValue::from_static("application/grpc+proto"),
    );
    // gRPC over HTTP/2: HEADERS (response head) + DATA* + HEADERS
    // (trailers). `send_response(end_of_stream=false)` opens the body
    // half of the stream.
    let mut send_stream = respond.send_response(response, false)?;

    for chunk in chunks {
        let framed = frame(chunk);
        send_stream.send_data(framed, false)?;
    }

    let mut trailers = HeaderMap::new();
    trailers.insert(
        HeaderName::from_static("grpc-status"),
        HeaderValue::from_str(&status_code.to_string()).expect("status is numeric ASCII"),
    );
    if !status_message.is_empty() {
        trailers.insert(
            HeaderName::from_static("grpc-message"),
            HeaderValue::from_str(status_message).expect("status message is ASCII"),
        );
    }
    send_stream.send_trailers(trailers)?;
    Ok(())
}

/// Pull every message off a server-streaming response, returning
/// either the collected payloads or the first error.
async fn collect<S>(mut stream: S) -> Result<Vec<ResponseData>, ChannelError>
where
    S: futures_core::Stream<Item = Result<ResponseData, ChannelError>> + Unpin,
{
    let mut out = Vec::new();
    while let Some(item) = stream.next().await {
        out.push(item?);
    }
    Ok(out)
}

/// The mock doesn't decode the request body — any well-formed prost
/// message satisfies the wire contract. `DataValueList` is the
/// simplest type already on the public surface.
fn empty_request() -> DataValueList {
    DataValueList::default()
}

// ─── Integration tests against the mock ───────────────────────────────

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn channel_decodes_single_response_chunk() {
    let mock = MockServer::spawn(vec![make_response_data(&["AAPL", "MSFT"])], 0).await;

    let channel = Channel::connect_h2c("127.0.0.1", mock.addr.port())
        .await
        .expect("h2c connect");

    let stream = channel
        .server_streaming::<DataValueList, ResponseData>(
            "/BetaEndpoints.BetaThetaTerminal/GetStockListSymbols",
            empty_request(),
        )
        .await
        .expect("rpc opens");

    let messages = collect(stream).await.expect("rpc completes ok");
    assert_eq!(messages.len(), 1, "exactly one response chunk");
    let list = DataValueList::decode(&messages[0].compressed_data[..]).expect("inner list decodes");
    assert_eq!(list.values.len(), 2);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn channel_streams_multiple_chunks_in_order() {
    let chunks = vec![
        make_response_data(&["AAPL"]),
        make_response_data(&["MSFT", "GOOG"]),
        make_response_data(&["SPY"]),
    ];
    let mock = MockServer::spawn(chunks.clone(), 0).await;

    let channel = Channel::connect_h2c("127.0.0.1", mock.addr.port())
        .await
        .expect("h2c connect");

    let stream = channel
        .server_streaming::<DataValueList, ResponseData>(
            "/BetaEndpoints.BetaThetaTerminal/GetStockListSymbols",
            empty_request(),
        )
        .await
        .expect("rpc opens");

    let messages = collect(stream).await.expect("rpc completes ok");
    assert_eq!(messages.len(), chunks.len(), "all chunks delivered");
    for (i, got) in messages.iter().enumerate() {
        assert_eq!(
            got.compressed_data, chunks[i].compressed_data,
            "chunk {i} bytes match wire order"
        );
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn channel_surfaces_non_ok_status_as_rpc_error() {
    // `grpc-status: 13` (Internal), `grpc-message: "boom"`.
    let mock = MockServer::spawn_with_message(Vec::new(), 13, "boom".to_string()).await;

    let channel = Channel::connect_h2c("127.0.0.1", mock.addr.port())
        .await
        .expect("h2c connect");

    let stream = channel
        .server_streaming::<DataValueList, ResponseData>(
            "/BetaEndpoints.BetaThetaTerminal/GetStockListSymbols",
            empty_request(),
        )
        .await
        .expect("rpc opens");

    let result = collect(stream).await;
    match result {
        Err(ChannelError::Rpc { status }) => {
            assert_eq!(status.code(), 13);
            assert_eq!(status.message(), "boom");
        }
        Err(other) => panic!("expected ChannelError::Rpc, got {other:?}"),
        Ok(msgs) => panic!("expected error, got {} messages", msgs.len()),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn channel_rejects_connect_to_closed_port() {
    // Bind a port, then drop the listener so the connect target is
    // unreachable. The connect call must surface a clean error rather
    // than hang.
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let port = listener.local_addr().unwrap().port();
    drop(listener);

    let result = tokio::time::timeout(
        Duration::from_secs(2),
        Channel::connect_h2c("127.0.0.1", port),
    )
    .await
    .expect("connect did not hang past the deadline");

    match result {
        Ok(_) => panic!("connect should have failed against a closed port"),
        Err(ChannelError::Tcp { .. }) | Err(ChannelError::H2Handshake(_)) => {
            // either flavor is acceptable — the kernel may either
            // refuse immediately or accept then drop on the h2
            // handshake.
        }
        Err(other) => panic!("unexpected error variant: {other:?}"),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn channel_deadline_elapses_during_streaming() {
    // Mock holds the request 300ms before sending DATA. Deadline is
    // 100ms — so either the open call surfaces `DeadlineExceeded`
    // directly, or the stream surfaces it on first poll. Both paths
    // are acceptable; the test asserts on the variant either way.
    let mock = MockServer::spawn_with_behaviour(
        vec![make_response_data(&["AAPL"])],
        0,
        String::new(),
        MockBehaviour {
            pre_response_delay: Some(Duration::from_millis(300)),
            ..MockBehaviour::default()
        },
    )
    .await;

    let channel = Channel::connect_h2c("127.0.0.1", mock.addr.port())
        .await
        .expect("h2c connect");

    let result = channel
        .server_streaming_with_deadline::<DataValueList, ResponseData>(
            "/BetaEndpoints.BetaThetaTerminal/GetStockListSymbols",
            empty_request(),
            Duration::from_millis(100),
        )
        .await;

    let final_err = match result {
        Err(e) => e,
        Ok(stream) => match collect(stream).await {
            Err(e) => e,
            Ok(msgs) => panic!("expected DeadlineExceeded, got {} messages", msgs.len()),
        },
    };

    match final_err {
        ChannelError::DeadlineExceeded { duration_ms } => {
            assert!(
                duration_ms <= 100,
                "deadline carried forward; got {duration_ms}ms"
            );
        }
        other => panic!("expected DeadlineExceeded, got {other:?}"),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn channel_classifies_goaway_distinctly_from_reset() {
    // Mock sends one chunk + response head, then the outer loop
    // issues abrupt_shutdown (GOAWAY). The client must surface this
    // as `ConnectionClosed`, distinct from a stream-level reset, so
    // pool consumers can recycle the channel.
    let mock = MockServer::spawn_with_behaviour(
        vec![make_response_data(&["AAPL"])],
        0,
        String::new(),
        MockBehaviour {
            goaway_mid_stream: true,
            ..MockBehaviour::default()
        },
    )
    .await;

    let channel = Channel::connect_h2c("127.0.0.1", mock.addr.port())
        .await
        .expect("h2c connect");

    let result = channel
        .server_streaming::<DataValueList, ResponseData>(
            "/BetaEndpoints.BetaThetaTerminal/GetStockListSymbols",
            empty_request(),
        )
        .await;

    // The error can surface either on the open call or on the stream
    // poll, depending on whether the response head was flushed before
    // GOAWAY arrived. Both surfaces must classify the failure as
    // connection-level (`ConnectionClosed`) so pool consumers recycle
    // the channel rather than retrying on the same one.
    let final_err = match result {
        Err(e) => e,
        Ok(stream) => match collect(stream).await {
            Err(e) => e,
            Ok(msgs) => panic!("expected ConnectionClosed, got {} messages", msgs.len()),
        },
    };

    match final_err {
        ChannelError::ConnectionClosed(_) => {
            // expected — connection-level shutdown
        }
        other => panic!("expected ConnectionClosed, got {other:?}"),
    }
}

// ─── Finding 1: trailers-only error path ─────────────────────────────

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn channel_decodes_trailers_only_error() {
    // Trailers-only encoding: response HEADERS frame carries
    // `grpc-status: 16` (Unauthenticated) and `grpc-message`, no DATA
    // frames, no separate trailing HEADERS frame. The legal gRPC way
    // for the server to refuse an RPC without ever opening the body.
    // Every gRPC client must classify this as `Rpc { code: 16, .. }`,
    // not a transport-layer (no-body / EmptyResponse / StatusParse)
    // error — the cross-binding error mapping (Python / TS / C++) keys
    // off `ChannelError::Rpc.status.code()`.
    let mock = MockServer::spawn_with_behaviour(
        Vec::new(),
        16,
        "session expired".to_string(),
        MockBehaviour {
            trailers_only: true,
            ..MockBehaviour::default()
        },
    )
    .await;

    let channel = Channel::connect_h2c("127.0.0.1", mock.addr.port())
        .await
        .expect("h2c connect");

    let result = channel
        .server_streaming::<DataValueList, ResponseData>(
            "/BetaEndpoints.BetaThetaTerminal/GetStockListSymbols",
            empty_request(),
        )
        .await;

    // The error can surface either on the open call (when the response
    // HEADERS frame already carries the trailers) or on the first poll
    // of the body stream (when h2 surfaces the trailers via the
    // post-body trailers slot). Both surfaces must classify as
    // `ChannelError::Rpc` so the binding error tables stay correct.
    let final_err = match result {
        Err(e) => e,
        Ok(stream) => match collect(stream).await {
            Err(e) => e,
            Ok(msgs) => panic!(
                "expected ChannelError::Rpc(Unauthenticated), got {} messages",
                msgs.len()
            ),
        },
    };

    match final_err {
        ChannelError::Rpc { status } => {
            assert_eq!(
                status.code(),
                16,
                "trailers-only status code preserved (Unauthenticated)"
            );
            assert_eq!(
                status.message(),
                "session expired",
                "trailers-only message preserved"
            );
        }
        other => panic!("expected ChannelError::Rpc, got {other:?}"),
    }
}

// ─── Finding 3: :scheme pseudo-header matches the transport ──────────

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn channel_sets_http_scheme_for_h2c_transport() {
    // h2c (plaintext HTTP/2) clients must send `:scheme = http` on
    // every request. The mock asserts the inbound pseudo-header — a
    // mismatch panics the handler and fails the test. The matching
    // TLS-side assertion (scheme=https) is covered by the unit test
    // in `crates/thetadatadx/src/grpc/channel.rs` that drives the
    // handshake helper over an in-memory IO pair.
    let mock = MockServer::spawn_with_behaviour(
        vec![make_response_data(&["AAPL"])],
        0,
        String::new(),
        MockBehaviour {
            assert_scheme: Some("http"),
            ..MockBehaviour::default()
        },
    )
    .await;

    let channel = Channel::connect_h2c("127.0.0.1", mock.addr.port())
        .await
        .expect("h2c connect");
    assert_eq!(
        channel.scheme_str(),
        "http",
        "h2c channel records :scheme=http"
    );

    let stream = channel
        .server_streaming::<DataValueList, ResponseData>(
            "/BetaEndpoints.BetaThetaTerminal/GetStockListSymbols",
            empty_request(),
        )
        .await
        .expect("rpc opens");

    let messages = collect(stream).await.expect("rpc completes ok");
    assert_eq!(messages.len(), 1);
}

// ─── Finding 2: codec max_message_size threaded through Channel ──────

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn channel_decodes_oversized_frame_to_codec_error() {
    // Send back a single response chunk whose framed protobuf payload
    // exceeds a 1 KiB codec ceiling. The codec must surface
    // `FrameTooLarge` so the configured `mdds.max_message_size` is
    // load-bearing, not decoration.
    let payload_size: usize = 16 * 1024;
    let big_chunk = ResponseData {
        compressed_data: vec![0u8; payload_size],
        ..ResponseData::default()
    };
    let mock = MockServer::spawn(vec![big_chunk], 0).await;

    let channel = Channel::connect_h2c_with_max_message_size("127.0.0.1", mock.addr.port(), 1024)
        .await
        .expect("h2c connect with bounded codec");

    let stream = channel
        .server_streaming::<DataValueList, ResponseData>(
            "/BetaEndpoints.BetaThetaTerminal/GetStockListSymbols",
            empty_request(),
        )
        .await
        .expect("rpc opens");

    let err = collect(stream)
        .await
        .expect_err("oversized frame must surface CodecError");
    match err {
        ChannelError::Codec(thetadatadx::grpc::CodecError::FrameTooLarge { length, max }) => {
            assert!(length > max, "wire length {length} must exceed max {max}");
            assert_eq!(max, 1024, "codec ceiling threaded through");
        }
        other => panic!("expected Codec(FrameTooLarge), got {other:?}"),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn channel_decodes_within_codec_ceiling() {
    // Same shape as the previous test but the configured ceiling is
    // wide enough to admit the frame. Confirms the codec parameter
    // doesn't accidentally reject legitimately-sized responses when
    // tuned upward — covers the upper end of the parameter sweep.
    let chunk = make_response_data(&["AAPL", "MSFT", "SPY", "QQQ"]);
    let mock = MockServer::spawn(vec![chunk], 0).await;

    let channel =
        Channel::connect_h2c_with_max_message_size("127.0.0.1", mock.addr.port(), 64 * 1024 * 1024)
            .await
            .expect("h2c connect with wide codec");

    let stream = channel
        .server_streaming::<DataValueList, ResponseData>(
            "/BetaEndpoints.BetaThetaTerminal/GetStockListSymbols",
            empty_request(),
        )
        .await
        .expect("rpc opens");

    let messages = collect(stream).await.expect("rpc completes ok");
    assert_eq!(messages.len(), 1, "exactly one response chunk");
}

// ─── Finding 5: pool picks credit-available channel over round-robin ─

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn channel_pool_routes_around_saturated_channel() {
    // Pool of 4 channels. Open a long-running RPC against pool
    // member 0 — the mock holds the response indefinitely so the
    // RPC stays in flight. While the slow RPC is open, `pool.next()`
    // must route around member 0: the in-flight stream counter on
    // member 0 is `1`, the others are `0`, and the pool picks the
    // lowest-loaded channel.
    //
    // Failure mode the test pins: strict round-robin would return
    // member 0 on a quarter of calls even while it's saturated,
    // reintroducing head-of-line blocking at the pool level.
    let mut mocks = Vec::new();
    let mut channels = Vec::new();
    for idx in 0..4 {
        let behaviour = if idx == 0 {
            // Member 0 holds the slow RPC: pre-response delay covers
            // the entire test so the response never lands and the
            // stream stays in flight.
            MockBehaviour {
                pre_response_delay: Some(Duration::from_secs(30)),
                ..MockBehaviour::default()
            }
        } else {
            MockBehaviour::default()
        };
        let mock = MockServer::spawn_with_behaviour(
            vec![make_response_data(&["AAPL"])],
            0,
            String::new(),
            behaviour,
        )
        .await;
        let channel = Channel::connect_h2c("127.0.0.1", mock.addr.port())
            .await
            .expect("h2c connect");
        channels.push(channel);
        mocks.push(mock);
    }
    let pool = ChannelPool::from_channels(channels);
    assert_eq!(pool.len(), 4);

    // Fire a slow RPC directly against pool member 0. We deliberately
    // detach the task and let it stay open for the test duration —
    // the mock holds the response for 30s.
    let member_zero_ptr: *const Channel = pool.member_for_test(0);
    let pool_arc = pool.clone();
    let slow_handle = tokio::spawn(async move {
        let chan = pool_arc.member_for_test(0);
        let _ = chan
            .server_streaming::<DataValueList, ResponseData>(
                "/BetaEndpoints.BetaThetaTerminal/GetStockListSymbols",
                empty_request(),
            )
            .await
            .expect("rpc opens");
        // Drain the stream (which never produces messages — the mock
        // never sends DATA). The handle stays alive until the test
        // aborts the task.
        // Note: we cannot easily drive the stream here because we
        // don't have an Unpin reference. The drop of `_` keeps the
        // stream open until the task is aborted.
    });

    // Give the slow RPC time to land on the wire so member 0's
    // in-flight counter advances. The server processes its body and
    // begins the pre-response delay; the client's stream is now in
    // its response-receiving state with the in-flight token held.
    tokio::time::sleep(Duration::from_millis(150)).await;

    // Confirm member 0's in-flight counter actually advanced — this
    // is the assertion the rest of the test depends on.
    assert_eq!(
        pool.member_for_test(0).in_flight_count(),
        1,
        "slow RPC is in flight on member 0"
    );
    for idx in 1..4 {
        assert_eq!(
            pool.member_for_test(idx).in_flight_count(),
            0,
            "member {idx} has no in-flight RPCs"
        );
    }

    // `pool.next()` must skip member 0 — it has `1` in-flight while
    // members 1-3 have `0`. The least-loaded pick wins.
    let mut saw_zero = 0_usize;
    let mut saw_non_zero = 0_usize;
    for _ in 0..16 {
        let pick: *const Channel = pool.next();
        if pick == member_zero_ptr {
            saw_zero += 1;
        } else {
            saw_non_zero += 1;
        }
    }
    assert_eq!(
        saw_zero, 0,
        "pool.next() must skip the saturated channel ({saw_zero} zero / {saw_non_zero} non-zero picks of 16)"
    );
    assert_eq!(
        saw_non_zero, 16,
        "every pick should land on a non-saturated channel"
    );

    slow_handle.abort();
}

// ─── Finding 4: per-stream RST_STREAM stays stream-level ─────────────

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn channel_classifies_per_stream_rst_as_stream_level() {
    // Per-stream RST_STREAM with REFUSED_STREAM. The h2 connection
    // itself is healthy — only this stream is killed. Classifying it
    // as `ConnectionClosed` (per the previous behaviour, which mapped
    // every `is_remote()` to ConnectionClosed) would force pool
    // consumers to recycle a still-good channel and burn retry
    // budgets. The fix surfaces `H2Stream` for per-stream resets and
    // reserves `ConnectionClosed` for GOAWAY / IO failures.
    //
    // HTTP/2 spec § 7 (Error Codes):
    //   <https://datatracker.ietf.org/doc/html/rfc9113#name-error-codes>
    //   CANCEL / REFUSED_STREAM / INTERNAL_ERROR are stream-level.
    let mock = MockServer::spawn_with_behaviour(
        Vec::new(),
        0,
        String::new(),
        MockBehaviour {
            stream_reset_reason: Some(h2::Reason::REFUSED_STREAM),
            ..MockBehaviour::default()
        },
    )
    .await;

    let channel = Channel::connect_h2c("127.0.0.1", mock.addr.port())
        .await
        .expect("h2c connect");

    let result = channel
        .server_streaming::<DataValueList, ResponseData>(
            "/BetaEndpoints.BetaThetaTerminal/GetStockListSymbols",
            empty_request(),
        )
        .await;

    let final_err = match result {
        Err(e) => e,
        Ok(stream) => match collect(stream).await {
            Err(e) => e,
            Ok(msgs) => panic!(
                "expected H2Stream for stream-level RST_STREAM, got {} messages",
                msgs.len()
            ),
        },
    };

    match final_err {
        ChannelError::H2Stream(_) => {
            // expected — per-stream RST, connection still alive.
        }
        ChannelError::ConnectionClosed(msg) => {
            panic!("per-stream RST_STREAM must classify as H2Stream, not ConnectionClosed ({msg})")
        }
        other => panic!("expected H2Stream, got {other:?}"),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn channel_classifies_per_stream_cancel_as_stream_level() {
    // Same shape as `channel_classifies_per_stream_rst_as_stream_level`
    // but with CANCEL — the gRPC spec uses CANCEL when the client (or
    // a load balancer) drops a stream. Confirms the classifier
    // doesn't accidentally key off the specific reason code.
    let mock = MockServer::spawn_with_behaviour(
        Vec::new(),
        0,
        String::new(),
        MockBehaviour {
            stream_reset_reason: Some(h2::Reason::CANCEL),
            ..MockBehaviour::default()
        },
    )
    .await;

    let channel = Channel::connect_h2c("127.0.0.1", mock.addr.port())
        .await
        .expect("h2c connect");

    let result = channel
        .server_streaming::<DataValueList, ResponseData>(
            "/BetaEndpoints.BetaThetaTerminal/GetStockListSymbols",
            empty_request(),
        )
        .await;

    let final_err = match result {
        Err(e) => e,
        Ok(stream) => match collect(stream).await {
            Err(e) => e,
            Ok(_) => panic!("expected H2Stream, got messages"),
        },
    };

    assert!(
        matches!(final_err, ChannelError::H2Stream(_)),
        "CANCEL RST_STREAM must classify as H2Stream, got {final_err:?}"
    );
}

// ─── Gap B: explicit request-byte assertion ──────────────────────────

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn channel_sends_expected_request_bytes() {
    // Confirms the mock's request-body decoder + assertion helper is
    // wired correctly. The mock decodes the inbound gRPC frame and
    // compares against the protobuf bytes the test pre-computed —
    // gives the full request/response loop a contract check rather
    // than asserting on the response shape alone.
    let request_msg = empty_request();
    let expected_bytes = request_msg.encode_to_vec();
    let mock = MockServer::spawn_with_behaviour(
        vec![make_response_data(&["AAPL"])],
        0,
        String::new(),
        MockBehaviour {
            assert_request_bytes: true,
            expected_request: expected_bytes,
            ..MockBehaviour::default()
        },
    )
    .await;

    let channel = Channel::connect_h2c("127.0.0.1", mock.addr.port())
        .await
        .expect("h2c connect");

    let stream = channel
        .server_streaming::<DataValueList, ResponseData>(
            "/BetaEndpoints.BetaThetaTerminal/GetStockListSymbols",
            request_msg,
        )
        .await
        .expect("rpc opens");

    let messages = collect(stream).await.expect("rpc completes ok");
    assert_eq!(messages.len(), 1, "exactly one response chunk");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn channel_pool_distributes_across_members() {
    // Spin four independent mock servers, each behind its own port,
    // and build a pool that round-robins over them. Eight requests
    // should land two per member.
    let mut mocks = Vec::new();
    let mut channels = Vec::new();
    for _ in 0..4 {
        let mock = MockServer::spawn(vec![make_response_data(&["AAPL"])], 0).await;
        let channel = Channel::connect_h2c("127.0.0.1", mock.addr.port())
            .await
            .expect("h2c connect");
        channels.push(channel);
        mocks.push(mock);
    }
    let pool = ChannelPool::from_channels(channels);
    assert_eq!(pool.len(), 4);

    // Eight successive `pool.next()` calls must wrap around exactly
    // twice. With 4 channels and a Relaxed atomic counter, indices
    // 0..8 % 4 = [0,1,2,3,0,1,2,3].
    let picks: Vec<*const Channel> = (0..8).map(|_| pool.next() as *const Channel).collect();
    assert_eq!(picks[0], picks[4]);
    assert_eq!(picks[1], picks[5]);
    assert_eq!(picks[2], picks[6]);
    assert_eq!(picks[3], picks[7]);
    // The four distinct addresses must all be different.
    let mut distinct: Vec<*const Channel> = picks[..4].to_vec();
    distinct.sort();
    distinct.dedup();
    assert_eq!(distinct.len(), 4, "first four picks are distinct channels");
}
