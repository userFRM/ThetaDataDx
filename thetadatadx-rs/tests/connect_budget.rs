//! An abandoned connect attempt must not delay the next one.
//!
//! New connections are paced by a process-wide budget, and a connect
//! that is given up on while it waits there is ordinary: a per-call
//! deadline fires, a sharded fan-out aborts its siblings, an HTTP client
//! behind the local server disconnects. If each of those spent a slot,
//! the budget's horizon would advance faster than the clock for as long
//! as the load lasted, and the first connection after the load stopped
//! would wait for everything the abandoned ones had booked. That is the
//! opposite of what the budget is for: it would keep a client off a
//! service that has already recovered.
//!
//! This test has a binary to itself. The budget is process-wide, so
//! anything else dialling in the same process would move the figure
//! being measured.

use std::time::{Duration, Instant};

use thetadatadx::grpc::Channel;

/// Enough abandoned attempts that a budget which spent a slot on each
/// would hold the next connection for over ten seconds.
const ABANDONED: usize = 50;

#[tokio::test]
async fn abandoned_connects_do_not_delay_the_next_connection() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind loopback listener");
    let port = listener.local_addr().expect("listener addr").port();
    let server = tokio::spawn(async move {
        // Hold every accepted socket so the peer never closes and the
        // measured connect is the budget's wait, not a retry.
        let mut held = Vec::new();
        while let Ok((socket, _)) = listener.accept().await {
            held.push(socket);
        }
    });

    for _ in 0..ABANDONED {
        // A zero timeout polls the connect once, which is as far as the
        // budget, and then drops it: the shape of a deadline that fires
        // while the attempt is queued for a slot.
        let _ = tokio::time::timeout(Duration::ZERO, Channel::connect_h2c("127.0.0.1", port)).await;
    }

    let start = Instant::now();
    let connected = Channel::connect_h2c("127.0.0.1", port).await;
    let waited = start.elapsed();
    server.abort();
    connected.expect("h2c connect after the abandoned attempts");

    assert!(
        waited < Duration::from_secs(2),
        "{ABANDONED} abandoned attempts must not book the budget against the next \
         connection; it waited {waited:?}"
    );
}
