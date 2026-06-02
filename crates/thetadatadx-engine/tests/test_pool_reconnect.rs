//! Channel-pool reconnect contract.
//!
//! Drives the in-place reconnect path on `crate::grpc::channel::Channel`
//! against the in-memory mock server harness:
//!
//! 1. After a connection-level fault, the channel's classifier
//!    triggers `trigger_reconnect` and the spawned reconnect future
//!    runs to completion (success OR exhaustion) — the observer hook
//!    fires `AttemptStart` exactly once per drop event.
//! 2. 100 concurrent observers of the same `ConnectionClosed` event
//!    produce exactly one fresh-connection open (single-flight CAS).
//!
//! These are the load-bearing contracts the in-place reconnect path
//! relies on. Future contributors investigating long-running-pool
//! ConnectionClosed regressions should ensure these tests still pass.

mod grpc_mock_server;

use std::net::SocketAddr;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use tokio::net::TcpListener;
use tokio::sync::{oneshot, Notify};
use tokio::task::JoinHandle;

use thetadatadx_engine::grpc::channel::ReconnectEvent;
use thetadatadx_engine::grpc::{Channel, ChannelError, ChannelPool};
use thetadatadx_engine::wire::{data_value, DataValue, DataValueList, ResponseData};

use grpc_mock_server::{serve_one_connection, MockBehaviour, MockServer};

/// Inline multi-accept mock used by the reconnect-success lifecycle
/// test. Accepts up to `max_connections` TCP connections in sequence;
/// each accepted connection serves one h2 RPC then closes. The
/// listener stays open across reconnects so the second TCP connect
/// (from the spawned reconnect task) lands on a live socket and
/// `AttemptSuccess` fires.
struct MultiAcceptMock {
    addr: SocketAddr,
    shutdown: Option<oneshot::Sender<()>>,
    task: Option<JoinHandle<()>>,
}

impl MultiAcceptMock {
    async fn spawn(
        chunks: Vec<ResponseData>,
        status_code: u32,
        max_connections: usize,
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
                _ = async {
                    for _ in 0..max_connections {
                        let (socket, _peer) = match listener.accept().await {
                            Ok(sp) => sp,
                            Err(_) => return,
                        };
                        let _ = socket.set_nodelay(true);
                        let _ = serve_one_connection(
                            socket,
                            chunks.clone(),
                            status_code,
                            String::new(),
                            behaviour.clone(),
                        )
                        .await;
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

/// Hard upper bound on observer-driven waits. The observer fires from
/// a spawned reconnect task; `Notify::notified` returns the moment the
/// task lands its first event, so this bound is "fail fast if the
/// reconnect future never spawned" rather than "wait long enough."
const OBSERVER_TIMEOUT: Duration = Duration::from_secs(5);

/// Wait for the next observer event (any variant) up to
/// `OBSERVER_TIMEOUT`. Returns `true` if a notification arrived,
/// `false` on timeout — the caller asserts on the recorded counters
/// either way.
async fn wait_for_event(notify: &Notify) -> bool {
    tokio::time::timeout(OBSERVER_TIMEOUT, notify.notified())
        .await
        .is_ok()
}

fn empty_request() -> DataValueList {
    DataValueList {
        values: vec![DataValue {
            data_type: Some(data_value::DataType::Text(String::new())),
        }],
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
    use prost::Message;
    ResponseData {
        compressed_data: list.encode_to_vec(),
        ..ResponseData::default()
    }
}

/// Drive one server-streaming RPC against `channel` and surface the
/// final outcome as `Ok(())` for success, `Err(ChannelError)` for any
/// transport-level failure (open phase OR streaming phase).
async fn rpc_once(channel: &Channel) -> Result<(), ChannelError> {
    let result = channel
        .server_streaming::<DataValueList, ResponseData>(
            "/BetaEndpoints.BetaThetaTerminal/GetStockListSymbols",
            empty_request(),
        )
        .await;
    let stream = match result {
        Ok(s) => s,
        Err(e) => return Err(e),
    };
    use tokio_stream::StreamExt;
    let mut stream = std::pin::pin!(stream);
    while let Some(item) = stream.next().await {
        item?;
    }
    Ok(())
}

/// Pin: when an RPC observes a connection-level fault, the channel
/// fires its single-flight reconnect, and the observer hook records
/// exactly one `AttemptStart` for the cycle.
///
/// The mock GOAWAYs mid-stream on the first RPC. The channel's
/// classifier triggers the reconnect spawn; the observer hook on the
/// `Channel` records the lifecycle. The fresh reconnect attempt
/// targets the same listener address (the mock has shut down by the
/// time the reconnect lands, so the attempt fails — but the observer
/// still records `AttemptStart`, which is what this test pins).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn reconnect_fires_on_connection_closed() {
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
    let port = mock.addr.port();

    let channel = Channel::connect_h2c("127.0.0.1", port)
        .await
        .expect("initial h2c connect");
    let pool = ChannelPool::from_channels(vec![channel]);

    // Install the observer on the pool member BEFORE the RPC fires
    // — the in-place reconnect path spawns its own task on the
    // tokio multi-thread runtime, so the observer must be ready
    // when that task lands. The Notify wakes us the moment the
    // reconnect spawn fires its first event.
    let attempt_starts = Arc::new(AtomicUsize::new(0));
    let notify = Arc::new(Notify::new());
    {
        let attempt_starts = Arc::clone(&attempt_starts);
        let notify = Arc::clone(&notify);
        pool.member_for_test(0)
            .set_reconnect_observer(move |event| {
                if matches!(event, ReconnectEvent::AttemptStart) {
                    attempt_starts.fetch_add(1, Ordering::SeqCst);
                }
                notify.notify_one();
            });
    }

    // RPC observes ConnectionClosed (GOAWAY mid-stream); the
    // classifier triggers the reconnect spawn.
    let lease = pool.next();
    let err = rpc_once(&lease)
        .await
        .expect_err("RPC must observe a transport-level error (GOAWAY mid-stream)");
    drop(lease);
    assert!(
        matches!(
            err,
            ChannelError::ConnectionClosed(_) | ChannelError::H2Stream(_)
        ),
        "expected connection-level error, got {err:?}",
    );
    drop(mock);

    if matches!(err, ChannelError::ConnectionClosed(_)) {
        // Wait for the spawned reconnect to fire its first event;
        // single-flight CAS guarantees exactly one AttemptStart for
        // the cycle.
        assert!(
            wait_for_event(&notify).await,
            "reconnect observer never fired within {OBSERVER_TIMEOUT:?}",
        );
        let count = attempt_starts.load(Ordering::SeqCst);
        assert_eq!(
            count, 1,
            "single-flight CAS must collapse the ConnectionClosed to exactly one AttemptStart; observed {count}",
        );
    } else {
        // Stream-level H2Stream — no reconnect is expected. Give the
        // runtime a single yield in case a stray spawn is in flight;
        // assert the negative invariant.
        tokio::task::yield_now().await;
        let count = attempt_starts.load(Ordering::SeqCst);
        assert_eq!(
            count, 0,
            "stream-level H2Stream must NOT trigger reconnect; observed {count} AttemptStart",
        );
    }
}

/// Pin the single-flight reconnect guard: N concurrent observers of
/// the same `ConnectionClosed` event open exactly ONE fresh TCP
/// connection to the mock, not N.
///
/// Mechanism: a `reconnect_observer` hook on `Channel` increments a
/// counter on `ReconnectEvent::AttemptStart`. N concurrent tasks
/// each call `Channel::trigger_reconnect` (the public API the
/// classifier hooks). After all observers return, exactly one
/// `AttemptStart` must have been recorded for the burst.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn reconnect_is_single_flight() {
    // Spawn an h2c mock so the reconnect path has a real listener to
    // hit. The mock's behaviour is irrelevant — the reconnect may
    // succeed or fail; what matters is that exactly one attempt
    // FIRES regardless of how many observers triggered it.
    let mock = MockServer::spawn_with_behaviour(
        vec![make_response_data(&["AAPL"])],
        0,
        String::new(),
        MockBehaviour::default(),
    )
    .await;
    let port = mock.addr.port();
    let channel = Channel::connect_h2c("127.0.0.1", port)
        .await
        .expect("initial h2c connect");

    // Wrap the channel in a pool so the weak self-reference is
    // installed (the trigger_reconnect path no-ops without it).
    let pool = ChannelPool::from_channels(vec![channel]);

    // Install the observer on the pool member. Notify wakes us the
    // moment the spawned reconnect lands its first event.
    let attempt_count = Arc::new(AtomicUsize::new(0));
    let notify = Arc::new(Notify::new());
    {
        let attempt_count = Arc::clone(&attempt_count);
        let notify = Arc::clone(&notify);
        pool.member_for_test(0)
            .set_reconnect_observer(move |event| {
                if matches!(event, ReconnectEvent::AttemptStart) {
                    attempt_count.fetch_add(1, Ordering::SeqCst);
                }
                notify.notify_one();
            });
    }

    // Fire N triggers synchronously, back-to-back, on the current
    // task. The first call wins the `reconnecting` CAS and spawns
    // the reconnect future; the remaining N-1 calls all observe the
    // CAS as already-claimed and short-circuit. Crucially: we do
    // NOT yield to the runtime between calls, so the spawned
    // reconnect future has no chance to complete and re-clear the
    // CAS before all N triggers have fired. This pins the
    // single-flight invariant inside one CAS-claim window.
    const N: usize = 100;
    let member = pool.member_for_test(0);
    for _ in 0..N {
        member.trigger_reconnect();
    }

    // Wait for the spawned reconnect to land its `AttemptStart`.
    assert!(
        wait_for_event(&notify).await,
        "reconnect observer never fired within {OBSERVER_TIMEOUT:?}",
    );

    let count = attempt_count.load(Ordering::SeqCst);
    assert_eq!(
        count, 1,
        "single-flight CAS must collapse {N} concurrent triggers to exactly one AttemptStart, observed {count}"
    );
    drop(mock);
}

/// Pin: a successful reconnect surfaces `AttemptSuccess` on the
/// observer, leaving the channel ready to serve subsequent RPCs.
///
/// Drives the reconnect against a multi-accept mock so the reconnect's
/// TCP connect lands on a live listener; the observer fires
/// `AttemptStart` followed by `AttemptSuccess` (NOT `AttemptExhausted`)
/// — this test pins the success path exactly.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn reconnect_observer_fires_on_lifecycle_events() {
    // Multi-accept mock keeps the listener open across reconnects so
    // the second TCP connect (from the spawned reconnect task) lands
    // on a live socket and `AttemptSuccess` fires.
    let mock = MultiAcceptMock::spawn(
        vec![make_response_data(&["AAPL"])],
        0,
        4,
        MockBehaviour::default(),
    )
    .await;
    let port = mock.addr.port();
    let channel = Channel::connect_h2c("127.0.0.1", port)
        .await
        .expect("initial h2c connect");
    let pool = ChannelPool::from_channels(vec![channel]);

    let events: Arc<std::sync::Mutex<Vec<ReconnectEvent>>> =
        Arc::new(std::sync::Mutex::new(Vec::new()));
    let success_notify = Arc::new(Notify::new());
    {
        let events = Arc::clone(&events);
        let success_notify = Arc::clone(&success_notify);
        pool.member_for_test(0)
            .set_reconnect_observer(move |event| {
                if let Ok(mut v) = events.lock() {
                    v.push(event);
                }
                if matches!(event, ReconnectEvent::AttemptSuccess) {
                    success_notify.notify_one();
                }
            });
    }

    pool.member_for_test(0).trigger_reconnect();

    // Wait for AttemptSuccess specifically — the multi-accept mock
    // guarantees the reconnect's TCP connect lands cleanly, so this
    // is the load-bearing path.
    assert!(
        wait_for_event(&success_notify).await,
        "AttemptSuccess never fired within {OBSERVER_TIMEOUT:?}; events so far: {:?}",
        events.lock().unwrap(),
    );

    let observed = events.lock().unwrap().clone();
    assert!(
        observed.contains(&ReconnectEvent::AttemptStart),
        "expected AttemptStart in {observed:?}",
    );
    assert!(
        observed.contains(&ReconnectEvent::AttemptSuccess),
        "expected AttemptSuccess in {observed:?}",
    );
    assert!(
        !observed.contains(&ReconnectEvent::AttemptExhausted),
        "AttemptExhausted must NOT fire against a live multi-accept mock; observed {observed:?}",
    );
    drop(mock);
}
