//! Mock h2 server that speaks the gRPC wire protocol against the
//! in-house `grpc::Channel`.
//!
//! The mock accepts one HTTP/2 stream, drains the framed request
//! payload, then writes back N hardcoded `ResponseData` chunks
//! followed by `grpc-status: N` trailers.
//!
//! Harness only: no `#[test]` item lives here. Several test targets drive
//! the same mock, and a `#[path]`-included module brings its tests with it,
//! so every includer used to compile and run the channel suite again. The
//! suite itself lives in `tests/grpc_mock_server.rs`, which includes this
//! file and is the one target that runs it.
//!
//! Each `MockServer` instance handles exactly one RPC and is dropped
//! by the test owning it — sufficient for per-test isolation.

use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use bytes::{BufMut, Bytes, BytesMut};
use futures::StreamExt;
use h2::server::SendResponse;
use http::{HeaderMap, HeaderName, HeaderValue, Response, StatusCode};
use prost::Message;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{oneshot, Notify};
use tokio::task::JoinHandle;

use thetadatadx::grpc::ChannelError;
use thetadatadx::wire::{data_value, DataValue, DataValueList, ResponseData};

/// Compose a length-prefix gRPC frame from a protobuf message.
pub fn frame<M: Message>(msg: &M) -> Bytes {
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
pub fn make_response_data(symbols: &[&str]) -> ResponseData {
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
    /// When true, the mock GOAWAYs the connection immediately after
    /// receiving the request body — before sending any response
    /// HEADERS or DATA. The client's open path (`ready()` /
    /// `send_request()` / `send_data()`) must classify the resulting
    /// h2 error as connection-level (`ChannelError::ConnectionClosed`)
    /// rather than stream-level — the pool keys off this distinction
    /// to recycle a dead channel.
    pub goaway_pre_response: bool,
    /// When true, the mock drops the underlying TCP socket immediately
    /// after the h2 handshake completes (after SETTINGS, before any
    /// HEADERS / DATA from either side). Exercises the
    /// connection-loss path: the client's open phase observes an h2 IO
    /// error and must classify it as `ConnectionClosed`.
    pub drop_after_handshake: bool,
    /// When `Some`, the mock sends an h2 PING frame at this interval
    /// while the connection is alive. Tests use this to confirm the
    /// client's reader thread answers with PONG and treats the round-
    /// trip as keep-alive evidence (the connection stays usable past
    /// its idle timeout). Implementation note: h2 client side
    /// auto-responds PONG to inbound PING frames, so the assertion is
    /// indirect — after the keep-alive window elapses the connection
    /// must still serve a fresh RPC successfully.
    pub inject_ping: Option<Duration>,
    /// When `Some(buf)`, the mock copies the inbound gRPC frame
    /// payload (after stripping the 5-byte header) into the supplied
    /// `Arc<Mutex<Vec<u8>>>`. Tests construct the buffer themselves,
    /// pass an `Arc::clone` here, and read the captured bytes back
    /// off their own handle after the RPC completes. Independent of
    /// `assert_request_bytes` — the assert variant compares to a
    /// pre-baked vector, this hook lets the test decode after the
    /// fact.
    pub capture_request_bytes: Option<Arc<Mutex<Vec<u8>>>>,
    /// When `Some`, the mock clamps the per-stream `INITIAL_WINDOW_SIZE`
    /// to this value via SETTINGS. A small window forces the client to
    /// emit WINDOW_UPDATE frames as it consumes the response body —
    /// otherwise the server runs out of flow-control credit and the
    /// stream stalls. Tests pair this with a multi-chunk response to
    /// confirm forward progress.
    pub clamp_initial_window: Option<u32>,
    /// When `Some`, the mock's request handler signals this `Notify`
    /// the instant the inbound request body has been fully drained —
    /// i.e. when the client's `send_request()` has finished writing
    /// its body and the server-side has reached "ready to respond".
    /// Tests pair this with a `tokio::time::timeout(secs, notify.
    /// notified()).await` to deterministically wait for "first call
    /// reached the wire" instead of using a fixed `tokio::time::
    /// sleep(...)` barrier (avoids fixed-sleep barriers).
    pub on_request_drained: Option<Arc<Notify>>,
    /// When `Some((notify, n))`, the mock's PING-driver task (active
    /// when `inject_ping = Some(_)`) signals `notify` after `n`
    /// successful PING/PONG round-trips. Tests pair this with a
    /// `tokio::time::timeout(secs, notify.notified()).await` to
    /// deterministically wait for keep-alive evidence instead of
    /// sleeping for several PING intervals (avoids fixed-sleep
    /// barriers).
    pub ping_pong_signal: Option<(Arc<Notify>, u32)>,
    /// When `n > 0`, the status-bearing HEADERS of the first `n`
    /// streams on the connection carry a `grpc-status-details-bin`
    /// value that is not valid base64 — on the trailers-only response
    /// head when `trailers_only` is set, on the trailing trailers
    /// otherwise. Exercises the client's containment of the status
    /// parser's decode panic: the poisoned RPC must surface a typed
    /// terminal error, and later streams on the same connection must
    /// still be served.
    pub invalid_status_details_streams: usize,
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
                accept = run(
                    listener,
                    chunks,
                    status_code,
                    status_message,
                    behaviour,
                ) => {
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

pub async fn serve_one_connection(
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
    if let Some(window) = behaviour.clamp_initial_window {
        // `initial_window_size` becomes the per-stream SETTINGS value
        // the client honours when computing its peer's flow-control
        // budget. Clamping it to a small value forces the client to
        // emit WINDOW_UPDATE as it drains DATA frames.
        builder.initial_window_size(window);
    }
    let mut connection = builder.handshake(socket).await?;
    // Server-initiated PING driver. h2 routes PONG responses through
    // the same `ping_pong()` handle automatically; the mock just
    // pumps PING frames at the requested cadence so the client's
    // reader thread observes liveness traffic. The driver task halts
    // when the connection's `ping_pong()` future returns an error
    // (connection closed / GOAWAY).
    //
    // When `ping_pong_signal = Some((notify, n))`, the driver tracks
    // successful round-trips and signals `notify` after `n` PONGs
    // have come back. Tests use this to wait deterministically for
    // keep-alive evidence instead of sleeping for several PING
    // intervals (avoids fixed-sleep barriers).
    let _ping_driver = behaviour.inject_ping.map(|interval| {
        let mut ping_pong = connection
            .ping_pong()
            .expect("ping_pong handle available on a fresh connection");
        let ping_pong_signal = behaviour.ping_pong_signal.clone();
        tokio::spawn(async move {
            // Cycle: send PING, await PONG, sleep `interval`. Bail
            // silently when the connection-level future returns Err.
            let mut completed: u32 = 0;
            loop {
                if ping_pong.ping(h2::Ping::opaque()).await.is_err() {
                    return;
                }
                completed = completed.saturating_add(1);
                if let Some((notify, target)) = ping_pong_signal.as_ref() {
                    if completed == *target {
                        // `notify_one` stores a permit if the waiter is not yet
                        // parked, so this one-shot edge cannot be lost to a race.
                        notify.notify_one();
                    }
                }
                tokio::time::sleep(interval).await;
            }
        })
    });
    if behaviour.drop_after_handshake {
        // Drop the connection at the IO layer the moment the h2
        // handshake completes — before the client's HEADERS arrive.
        // The client's pending `ready()` / `send_request()` observes
        // an IO error which must classify as
        // `ChannelError::ConnectionClosed`.
        drop(connection);
        return Ok(());
    }
    // Drive the connection until either (a) an RPC is served and the
    // client closes, or (b) the connection itself shuts down. The
    // request handler runs on a separate task so the accept loop can
    // continue advancing the h2 connection state machine while DATA
    // and trailers flush.
    let mut stream_index: usize = 0;
    while let Some(request_result) = connection.accept().await {
        let (request, respond) = request_result?;
        if behaviour.goaway_mid_stream || behaviour.goaway_pre_response {
            // GOAWAY after the request body is in: with only the
            // response head and one DATA chunk sent in
            // `goaway_mid_stream` mode, with no response at all in
            // `goaway_pre_response` mode. Either path surfaces
            // `ChannelError::ConnectionClosed` on the client; the pool
            // relies on that distinction to recycle the dead channel
            // rather than treating it as a stream-level reset. The two
            // modes are mutually exclusive at the call site (configure
            // one OR the other).
            //
            // Served inline so this task alone decides when the
            // connection is polled. It is driven while the body drains,
            // because the body's DATA frames arrive only while it is
            // polled and can trail the HEADERS frame `accept` returned
            // with. It is then left alone until the GOAWAY, so the
            // partial response and the GOAWAY leave together, with no
            // reset of the abandoned stream in between.
            let mut body = request.into_body();
            let drain = async {
                while let Some(chunk) = body.data().await {
                    let chunk = chunk?;
                    let _ = body.flow_control().release_capacity(chunk.len());
                }
                Ok::<(), h2::Error>(())
            };
            let driven = std::future::poll_fn(|cx| connection.poll_closed(cx));
            tokio::select! {
                drained = drain => drained?,
                _ = driven => {}
            }
            if behaviour.goaway_mid_stream {
                respond_partial_then_drop(respond, &chunks).await?;
            } else {
                drop(respond);
            }
            connection.abrupt_shutdown(h2::Reason::NO_ERROR);
            continue;
        }
        let chunks = chunks.clone();
        let status_message = status_message.clone();
        let behaviour_inner = behaviour.clone();
        // Poison the status-bearing HEADERS of the first N streams on
        // the connection (see `invalid_status_details_streams`); later
        // streams respond clean so tests can confirm the connection
        // survived the poisoned exchange.
        let poison_status_details = stream_index < behaviour.invalid_status_details_streams;
        stream_index += 1;
        tokio::spawn(async move {
            if let Err(e) = handle_request(
                request,
                respond,
                chunks,
                status_code,
                status_message,
                behaviour_inner,
                poison_status_details,
            )
            .await
            {
                eprintln!("grpc_mock_server: request handler failed: {e}");
            }
        });
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
    poison_status_details: bool,
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
    let need_buffer = behaviour.assert_request_bytes || behaviour.capture_request_bytes.is_some();
    while let Some(chunk) = body.data().await {
        let chunk = chunk?;
        let _ = body.flow_control().release_capacity(chunk.len());
        if need_buffer {
            request_buf.extend_from_slice(&chunk);
        }
    }
    if let Some(target) = behaviour.capture_request_bytes.as_ref() {
        if request_buf.len() >= 5 {
            // gRPC frame layout: 1 compressed flag + 4 big-endian
            // length + payload. Strip the header so tests get clean
            // protobuf bytes ready for `prost::Message::decode`.
            if let Ok(mut guard) = target.lock() {
                guard.clear();
                guard.extend_from_slice(&request_buf[5..]);
            }
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
    // Signal "request body drained" before any pre-response delay so
    // the test's barrier observes the client-side state advance
    // (request fully on the wire, in-flight counter ticked) without
    // racing the pre-response sleep. Pre-existing capture / assert
    // hooks above already inspect the drained body — this fires after
    // both so the test sees a coherent post-drain state.
    if let Some(notify) = behaviour.on_request_drained.as_ref() {
        // `notify_one`, not `notify_waiters`: the drain can fire before the test
        // registers its waiter, and only `notify_one` stores a permit so the edge
        // survives the race (the test awaits this signal exactly once).
        notify.notify_one();
    }
    if let Some(d) = behaviour.pre_response_delay {
        tokio::time::sleep(d).await;
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
        respond_trailers_only(respond, status_code, &status_message, poison_status_details)?;
        return Ok(());
    }
    respond_chunks(
        respond,
        &chunks,
        status_code,
        &status_message,
        poison_status_details,
    )?;
    Ok(())
}

/// `grpc-status-details-bin` value outside the base64 alphabet (`!` is
/// not a base64 character), so any conforming decoder rejects it. The
/// reference client's status parser `.expect()`s that decode — the
/// poisoned-trailer tests below pin the containment of the resulting
/// panic.
const INVALID_STATUS_DETAILS_BIN: &str = "!!!not-base64!!!";

/// Send a trailers-only gRPC response: HTTP 200 with `grpc-status`
/// (and optional `grpc-message`) on the initial HEADERS frame, no body.
fn respond_trailers_only(
    mut respond: SendResponse<Bytes>,
    status_code: u32,
    status_message: &str,
    poison_status_details: bool,
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
    if poison_status_details {
        response.headers_mut().insert(
            HeaderName::from_static("grpc-status-details-bin"),
            HeaderValue::from_static(INVALID_STATUS_DETAILS_BIN),
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
    // Give h2 a deterministic chance to actually flush the response
    // head + DATA to the wire before the outer loop tears the
    // connection down. Without this, abrupt_shutdown can race the
    // response-head emission and the client sees an IO error rather
    // than a clean GOAWAY.
    //
    // The prior fixed `sleep(50ms)` here was a wall-clock barrier;
    // it is replaced with a cooperative yield. The replacement is
    // cooperative: yielding to the runtime hands control back to the
    // h2 connection driver task spawned by `serve_one_connection`,
    // which drains the SendStream's outbound buffer and pushes the
    // response HEADERS + DATA frames onto the TCP socket. Three
    // yields cover (a) the dispatch of HEADERS, (b) the DATA frame
    // emission, and (c) the socket write completion. No
    // sleep-derived timing assumption survives. The test
    // `channel_classifies_goaway_distinctly_from_reset` accepts
    // both early- and mid-stream error surfaces regardless.
    for _ in 0..3 {
        tokio::task::yield_now().await;
    }
    // Deliberately drop without sending trailers. The connection-level
    // GOAWAY follows on the outer task.
    Ok(())
}

fn respond_chunks(
    mut respond: SendResponse<Bytes>,
    chunks: &[ResponseData],
    status_code: u32,
    status_message: &str,
    poison_status_details: bool,
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
    if poison_status_details {
        trailers.insert(
            HeaderName::from_static("grpc-status-details-bin"),
            HeaderValue::from_static(INVALID_STATUS_DETAILS_BIN),
        );
    }
    send_stream.send_trailers(trailers)?;
    Ok(())
}

/// Pull every message off a server-streaming response, returning
/// either the collected payloads or the first error.
pub async fn collect<S>(mut stream: S) -> Result<Vec<ResponseData>, ChannelError>
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
pub fn empty_request() -> DataValueList {
    DataValueList::default()
}
