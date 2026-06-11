//! Connection-recycle contract after connection-level faults.
//!
//! Drives the transport's reconnect behaviour against the in-memory
//! mock server harness:
//!
//! 1. A GOAWAY mid-RPC surfaces as `ConnectionClosed` (the transient
//!    classification the retry shell re-dispatches on), and the NEXT
//!    RPC dispatched through the same `Channel` handle succeeds — the
//!    underlying connection is replaced in place, no channel rebuild,
//!    no pool-slot surgery.
//! 2. The same contract holds for a pooled channel: the pool slot
//!    keeps its `Arc<Channel>` identity across the recycle.
//! 3. When the reconnect target is gone, the follow-up RPC surfaces
//!    `ConnectionClosed` again (never a panic or a hang), keeping the
//!    retry shell in its transient loop until the budget decides.
//!
//! These are the load-bearing contracts long-running pools rely on:
//! hosted MDDS occasionally emits GOAWAY during scheduled restarts,
//! and a transient connection blip must heal transparently underneath
//! the caller's retry shell.

mod grpc_mock_server;

use std::net::SocketAddr;
use std::time::Duration;

use tokio::net::TcpListener;
use tokio::sync::oneshot;
use tokio::task::JoinHandle;

use thetadatadx::grpc::{Channel, ChannelError, ChannelPool};
use thetadatadx::wire::{data_value, DataValue, DataValueList, ResponseData};

use grpc_mock_server::{serve_one_connection, MockBehaviour};

/// Multi-accept mock: accepts up to `behaviours.len()` TCP connections
/// in sequence, serving connection `i` with `behaviours[i]`. The
/// listener stays open across connections so a reconnect dial from the
/// transport lands on a live socket and observes the next scripted
/// behaviour.
struct MultiAcceptMock {
    addr: SocketAddr,
    shutdown: Option<oneshot::Sender<()>>,
    task: Option<JoinHandle<()>>,
}

impl MultiAcceptMock {
    async fn spawn(chunks: Vec<ResponseData>, behaviours: Vec<MockBehaviour>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind to ephemeral port");
        let addr = listener.local_addr().expect("read local addr");
        let (tx, rx) = oneshot::channel();

        let task = tokio::spawn(async move {
            tokio::select! {
                _ = rx => {},
                _ = async {
                    for behaviour in behaviours {
                        let (socket, _peer) = match listener.accept().await {
                            Ok(sp) => sp,
                            Err(_) => return,
                        };
                        let _ = socket.set_nodelay(true);
                        // Each accepted connection is served on its own
                        // task so a connection the client abandons
                        // (post-GOAWAY) cannot stall the accept loop
                        // for the reconnect dial.
                        let chunks = chunks.clone();
                        tokio::spawn(async move {
                            let _ = serve_one_connection(
                                socket,
                                chunks,
                                0,
                                String::new(),
                                behaviour,
                            )
                            .await;
                        });
                    }
                } => {}
            }
        });

        Self {
            addr,
            shutdown: Some(tx),
            task: Some(task),
        }
    }
}

impl Drop for MultiAcceptMock {
    fn drop(&mut self) {
        if let Some(tx) = self.shutdown.take() {
            let _ = tx.send(());
        }
        if let Some(task) = self.task.take() {
            task.abort();
        }
    }
}

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
        compressed_data: prost::Message::encode_to_vec(&list),
        ..ResponseData::default()
    }
}

fn empty_request() -> DataValueList {
    DataValueList::default()
}

/// Drain a stream to its terminal state, returning the first error.
async fn drain<S>(mut stream: S) -> Result<Vec<ResponseData>, ChannelError>
where
    S: futures_core::Stream<Item = Result<ResponseData, ChannelError>> + Unpin,
{
    use futures::StreamExt;
    let mut out = Vec::new();
    while let Some(item) = stream.next().await {
        out.push(item?);
    }
    Ok(out)
}

/// Dispatch one RPC on `channel` and return its drained outcome —
/// folds open-phase and stream-phase errors into one `Result` since
/// a GOAWAY can surface on either, depending on flush timing.
async fn one_rpc(channel: &Channel) -> Result<Vec<ResponseData>, ChannelError> {
    let stream = channel
        .server_streaming::<DataValueList, ResponseData>(
            "/BetaEndpoints.BetaThetaTerminal/GetStockListSymbols",
            empty_request(),
        )
        .await?;
    drain(stream).await
}

/// Retry shell for the post-GOAWAY RPC: the transport replaces the
/// connection lazily, so the dispatch immediately after the fault may
/// still observe the dying connection. A bounded re-dispatch loop —
/// the same shape the production retry shell drives — must land on the
/// fresh connection within a few attempts.
async fn rpc_with_bounded_retry(channel: &Channel) -> Result<Vec<ResponseData>, ChannelError> {
    let mut last_err = None;
    for _ in 0..8 {
        match one_rpc(channel).await {
            Ok(messages) => return Ok(messages),
            Err(e @ (ChannelError::ConnectionClosed(_) | ChannelError::H2Stream(_))) => {
                last_err = Some(e);
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
            Err(other) => return Err(other),
        }
    }
    Err(last_err.expect("loop ran at least once"))
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn channel_serves_rpc_after_goaway_recycle() {
    // Connection 1 GOAWAYs mid-stream; connection 2 serves normally.
    // The same `Channel` handle must carry both RPCs: the first
    // surfaces `ConnectionClosed` (transient for the retry shell), the
    // retried second lands on the replacement connection.
    let mock = MultiAcceptMock::spawn(
        vec![make_response_data(&["AAPL"])],
        vec![
            MockBehaviour {
                goaway_mid_stream: true,
                ..MockBehaviour::default()
            },
            MockBehaviour::default(),
        ],
    )
    .await;

    let channel = Channel::connect_h2c("127.0.0.1", mock.addr.port())
        .await
        .expect("h2c connect");

    let first = one_rpc(&channel).await;
    match first {
        Err(ChannelError::ConnectionClosed(_)) => {}
        Err(other) => panic!("GOAWAY must classify connection-level, got {other:?}"),
        Ok(msgs) => panic!("expected ConnectionClosed, got {} messages", msgs.len()),
    }

    let second = rpc_with_bounded_retry(&channel)
        .await
        .expect("post-GOAWAY RPC must succeed on the recycled connection");
    assert_eq!(
        second.len(),
        1,
        "recycled connection serves the full response"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn pool_slot_keeps_channel_identity_across_recycle() {
    // Same contract through the pool: the slot is never marked dead or
    // replaced — the identical `Arc<Channel>` serves the post-recycle
    // RPC.
    let mock = MultiAcceptMock::spawn(
        vec![make_response_data(&["MSFT", "QQQ"])],
        vec![
            MockBehaviour {
                goaway_mid_stream: true,
                ..MockBehaviour::default()
            },
            MockBehaviour::default(),
        ],
    )
    .await;

    let channel = Channel::connect_h2c("127.0.0.1", mock.addr.port())
        .await
        .expect("h2c connect");
    let pool = ChannelPool::from_channels(vec![channel]);

    let member_before = std::sync::Arc::as_ptr(pool.member_for_test(0));

    let lease = pool.next();
    let first = one_rpc(&lease).await;
    drop(lease);
    assert!(
        matches!(first, Err(ChannelError::ConnectionClosed(_))),
        "GOAWAY through the pool classifies connection-level: {first:?}"
    );

    let lease = pool.next();
    let second = rpc_with_bounded_retry(&lease)
        .await
        .expect("pooled channel serves the post-recycle RPC");
    assert_eq!(second.len(), 1);
    drop(lease);

    let member_after = std::sync::Arc::as_ptr(pool.member_for_test(0));
    assert_eq!(
        member_before, member_after,
        "the pool slot must keep its channel identity across the recycle"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn dead_reconnect_target_stays_connection_closed() {
    // Connection 1 GOAWAYs and the listener then disappears (the mock
    // accepted its full script). Every follow-up RPC must keep
    // surfacing `ConnectionClosed` — the transient classification that
    // keeps the caller's retry shell in its bounded loop — never a
    // panic, hang, or a misclassified terminal error.
    let mock = MultiAcceptMock::spawn(
        vec![make_response_data(&["AAPL"])],
        vec![MockBehaviour {
            goaway_mid_stream: true,
            ..MockBehaviour::default()
        }],
    )
    .await;

    let channel = Channel::connect_h2c("127.0.0.1", mock.addr.port())
        .await
        .expect("h2c connect");

    let first = one_rpc(&channel).await;
    assert!(
        matches!(first, Err(ChannelError::ConnectionClosed(_))),
        "GOAWAY classifies connection-level: {first:?}"
    );

    // Tear the listener down so the reconnect dial has no target.
    drop(mock);

    let followup = tokio::time::timeout(Duration::from_secs(10), one_rpc(&channel))
        .await
        .expect("dead-target dispatch must fail fast, not hang");
    assert!(
        matches!(followup, Err(ChannelError::ConnectionClosed(_))),
        "dead reconnect target stays connection-level: {followup:?}"
    );
}
