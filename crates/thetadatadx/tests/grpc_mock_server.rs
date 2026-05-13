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

#![cfg(feature = "inhouse-grpc")]

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

use thetadatadx::grpc::{Channel, ChannelError};
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
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind to ephemeral port");
        let addr = listener.local_addr().expect("read local addr");
        let (tx, rx) = oneshot::channel();

        let task = tokio::spawn(async move {
            tokio::select! {
                _ = rx => {},
                accept = run(listener, chunks, status_code, status_message) => {
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
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let (socket, _peer) = listener.accept().await?;
    let _ = socket.set_nodelay(true);
    serve_one_connection(socket, chunks, status_code, status_message).await
}

async fn serve_one_connection(
    socket: TcpStream,
    chunks: Vec<ResponseData>,
    status_code: u32,
    status_message: String,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let mut connection = h2::server::handshake(socket).await?;
    // Drive the connection until either (a) an RPC is served and the
    // client closes, or (b) the connection itself shuts down. The
    // request handler runs on a separate task so the accept loop can
    // continue advancing the h2 connection state machine while DATA
    // and trailers flush.
    while let Some(request_result) = connection.accept().await {
        let (request, respond) = request_result?;
        let chunks = chunks.clone();
        let status_message = status_message.clone();
        tokio::spawn(async move {
            if let Err(e) =
                handle_request(request, respond, chunks, status_code, status_message).await
            {
                eprintln!("grpc_mock_server: request handler failed: {e}");
            }
        });
    }
    Ok(())
}

async fn handle_request(
    request: http::Request<h2::RecvStream>,
    respond: SendResponse<Bytes>,
    chunks: Vec<ResponseData>,
    status_code: u32,
    status_message: String,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    // Drain the request body so flow-control accounting mirrors a
    // real gRPC server. We don't decode it — the channel tests assert
    // on response shape only.
    let mut body = request.into_body();
    while let Some(chunk) = body.data().await {
        let chunk = chunk?;
        let _ = body.flow_control().release_capacity(chunk.len());
    }
    respond_chunks(respond, &chunks, status_code, &status_message)?;
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
