//! The channel integration suite against the mock h2 server.
//!
//! The mock itself lives in `common/grpc_mock.rs` and is included here and by
//! every other target that needs it. The tests below are this target's own, so
//! including the harness elsewhere no longer drags the suite along.
//!
//! These exercise the full `connect_h2c` -> `server_streaming` ->
//! `Stream::next` path with no external dependencies. The criterion bench in
//! `benches/grpc_channel.rs` times the in-house path against the same mock.

use std::sync::Arc;
use std::time::Duration;

use futures::StreamExt;
use prost::Message;
use tokio::net::TcpListener;
use tokio::sync::Notify;

use thetadatadx::grpc::{Channel, ChannelError, ChannelPool};
use thetadatadx::wire::{DataValueList, ResponseData};

#[path = "common/grpc_mock.rs"]
mod harness;

pub use harness::*;

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
    // than hang. The deadline is a hang guard, not a latency bound: a
    // dial first waits for a slot in the process-wide connection budget,
    // and every other test in this binary is competing for the same
    // slots.
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let port = listener.local_addr().unwrap().port();
    drop(listener);

    let result = tokio::time::timeout(
        Duration::from_secs(30),
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

/// Assert the typed error a contained status-parser panic must surface:
/// `ChannelError::Rpc` with the canonical `Internal` code and a message
/// naming the undecodable trailer. Shared by the two poisoned-trailer
/// shapes (trailers-only head, end-of-stream trailers).
fn assert_undecodable_trailer_error(err: &ChannelError) {
    match err {
        ChannelError::Rpc { status } => {
            assert_eq!(
                status.code(),
                13,
                "contained status-parser panic maps to the canonical Internal code"
            );
            assert!(
                status.message().contains("undecodable status trailer"),
                "message names the malformed trailer, got: {}",
                status.message()
            );
        }
        other => panic!("expected ChannelError::Rpc, got {other:?}"),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn channel_contains_undecodable_status_trailer_on_trailers_only_response() {
    // The reference client's status parser `.expect()`s the base64
    // decode of `grpc-status-details-bin`, so a trailers-only response
    // head carrying a malformed value would panic the dispatching task.
    // The open-phase containment must surface a typed terminal error
    // instead, and the connection must survive: the next RPC on the
    // same channel (stream 2, served clean by the mock) round-trips.
    let mock = MockServer::spawn_with_behaviour(
        Vec::new(),
        0,
        String::new(),
        MockBehaviour {
            trailers_only: true,
            invalid_status_details_streams: 1,
            ..MockBehaviour::default()
        },
    )
    .await;

    let channel = Channel::connect_h2c("127.0.0.1", mock.addr.port())
        .await
        .expect("h2c connect");

    // The contained error can surface on the open call (response
    // HEADERS already carry the trailers) or on the first poll of the
    // body stream — both boundaries run the same containment, so
    // accept either surface like `channel_decodes_trailers_only_error`
    // does.
    let result = channel
        .server_streaming::<DataValueList, ResponseData>(
            "/BetaEndpoints.BetaThetaTerminal/GetStockListSymbols",
            empty_request(),
        )
        .await;
    let err = match result {
        Err(e) => e,
        Ok(stream) => match collect(stream).await {
            Err(e) => e,
            Ok(msgs) => panic!(
                "expected a contained undecodable-trailer error, got {} messages",
                msgs.len()
            ),
        },
    };
    assert_undecodable_trailer_error(&err);

    // Channel still usable: the mock serves stream 2 clean
    // (trailers-only `grpc-status: 0`), which decodes as an empty
    // OK stream.
    let stream = channel
        .server_streaming::<DataValueList, ResponseData>(
            "/BetaEndpoints.BetaThetaTerminal/GetStockListSymbols",
            empty_request(),
        )
        .await
        .expect("channel dispatches a fresh RPC after the contained panic");
    let messages = collect(stream)
        .await
        .expect("clean follow-up RPC completes");
    assert!(messages.is_empty(), "trailers-only OK carries no messages");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn channel_contains_undecodable_status_trailer_at_end_of_stream() {
    // Same malformed `grpc-status-details-bin`, this time on the
    // trailing HEADERS after data frames — the shape that reaches
    // `ServerStreaming::poll_next`. The stream must yield its data
    // frame, then the typed terminal error (no panic), then fuse; and
    // the channel must keep serving.
    let chunks = vec![make_response_data(&["AAPL", "MSFT"])];
    let mock = MockServer::spawn_with_behaviour(
        chunks,
        0,
        String::new(),
        MockBehaviour {
            invalid_status_details_streams: 1,
            ..MockBehaviour::default()
        },
    )
    .await;

    let channel = Channel::connect_h2c("127.0.0.1", mock.addr.port())
        .await
        .expect("h2c connect");

    let mut stream = channel
        .server_streaming::<DataValueList, ResponseData>(
            "/BetaEndpoints.BetaThetaTerminal/GetStockListSymbols",
            empty_request(),
        )
        .await
        .expect("rpc opens — the poisoned trailers arrive after the data frames");

    // The data frame ahead of the poisoned trailers decodes normally.
    let first = stream
        .next()
        .await
        .expect("one data frame precedes the trailers")
        .expect("data frame decodes");
    let list = DataValueList::decode(&first.compressed_data[..]).expect("inner list decodes");
    assert_eq!(list.values.len(), 2);

    // The poisoned end-of-stream trailers surface as the typed
    // terminal error, and the stream fuses after it.
    let err = stream
        .next()
        .await
        .expect("terminal item follows the data frame")
        .expect_err("undecodable trailer surfaces as an error");
    assert_undecodable_trailer_error(&err);
    assert!(
        stream.next().await.is_none(),
        "stream is fused after the terminal error"
    );

    // Channel still usable: stream 2 is served clean and decodes
    // end to end.
    let stream = channel
        .server_streaming::<DataValueList, ResponseData>(
            "/BetaEndpoints.BetaThetaTerminal/GetStockListSymbols",
            empty_request(),
        )
        .await
        .expect("channel dispatches a fresh RPC after the contained panic");
    let messages = collect(stream)
        .await
        .expect("clean follow-up RPC completes");
    assert_eq!(messages.len(), 1, "follow-up RPC decodes its chunk");
}

// ─── Finding 3: :scheme pseudo-header matches the transport ──────────

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn channel_sets_http_scheme_for_h2c_transport() {
    // h2c (plaintext HTTP/2) clients must send `:scheme = http` on
    // every request. The mock asserts the inbound pseudo-header — a
    // mismatch panics the handler and fails the test. The TLS side
    // (`:scheme = https`) has no equivalent here: the client would have
    // to accept a certificate for a loopback listener, and the SDK
    // builds its market-data TLS from webpki roots, so nothing in this
    // suite covers that half.
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
async fn channel_rejects_oversized_frame_at_decode_ceiling() {
    // Send back a single response chunk whose framed protobuf payload
    // exceeds a 1 KiB decode ceiling. The transport must reject the
    // frame so the configured `mdds.max_message_size` is load-bearing,
    // not decoration. The canonical rejection is the `OutOfRange`
    // status the gRPC decode layer emits for over-limit messages —
    // terminal for the retry shell, same as the previous transport's
    // codec-level rejection.
    let payload_size: usize = 16 * 1024;
    let big_chunk = ResponseData {
        compressed_data: vec![0u8; payload_size],
        ..ResponseData::default()
    };
    let mock = MockServer::spawn(vec![big_chunk], 0).await;

    let channel = Channel::connect_h2c_with_max_message_size("127.0.0.1", mock.addr.port(), 1024)
        .await
        .expect("h2c connect with bounded decode ceiling");

    let stream = channel
        .server_streaming::<DataValueList, ResponseData>(
            "/BetaEndpoints.BetaThetaTerminal/GetStockListSymbols",
            empty_request(),
        )
        .await
        .expect("rpc opens");

    let err = collect(stream)
        .await
        .expect_err("oversized frame must be rejected at the decode ceiling");
    match err {
        ChannelError::Rpc { status } => {
            // 11 = OutOfRange on the canonical gRPC code table.
            assert_eq!(status.code(), 11, "over-limit frames map to OutOfRange");
            assert!(
                status.message().contains("length too large"),
                "diagnostic names the limit violation: {:?}",
                status.message()
            );
        }
        other => panic!("expected Rpc(OutOfRange), got {other:?}"),
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
    let member_zero_drained = Arc::new(Notify::new());
    let mut mocks = Vec::new();
    let mut channels = Vec::new();
    for idx in 0..4 {
        let behaviour = if idx == 0 {
            // Member 0 holds the slow RPC: pre-response delay covers
            // the entire test so the response never lands and the
            // stream stays in flight. The `on_request_drained` Notify
            // signals the instant the server has drained the request
            // body — i.e. the slow RPC has reached "response-receiving"
            // state and the client's in-flight counter has ticked.
            MockBehaviour {
                pre_response_delay: Some(Duration::from_secs(30)),
                on_request_drained: Some(Arc::clone(&member_zero_drained)),
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
    let member_zero_ptr: *const Channel = std::sync::Arc::as_ptr(pool.member_for_test(0));
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

    // Wait for the slow RPC to actually land on the wire — the mock
    // signals `member_zero_drained` the moment it finishes draining
    // the request body, which is the same moment the client's
    // in-flight counter advances. The 5s timeout is a runaway
    // protector; the notify normally fires inside a few ms. This
    // replaces a fixed `sleep(150ms)` barrier (avoids fixed-sleep
    // barriers).
    tokio::time::timeout(Duration::from_secs(5), member_zero_drained.notified())
        .await
        .expect("slow RPC reached the wire within 5s");

    // Confirm member 0's in-flight counter actually advanced — this
    // is the assertion the rest of the test depends on. The server-side
    // drain Notify and the client-side in-flight counter advance on
    // opposite ends of the connection, so the counter can lag the drain
    // signal by a scheduler tick; poll until it observes the in-flight
    // RPC rather than reading once. The bound still requires the count to
    // reach exactly 1 — it only tolerates the cross-side ordering gap.
    let in_flight_deadline = tokio::time::Instant::now() + Duration::from_secs(2);
    loop {
        if pool.member_for_test(0).in_flight_count() == 1 {
            break;
        }
        assert!(
            tokio::time::Instant::now() < in_flight_deadline,
            "slow RPC never registered as in flight on member 0 (count = {})",
            pool.member_for_test(0).in_flight_count()
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    for idx in 1..4 {
        assert_eq!(
            pool.member_for_test(idx).in_flight_count(),
            0,
            "member {idx} has no in-flight RPCs"
        );
    }

    // `pool.next()` must skip member 0 — it has `1` in-flight while
    // members 1-3 have `0`. The least-loaded pick wins. Bind each
    // lease to a local so its pre-dispatch reservation drops cleanly
    // and the picker accounting stays sane across iterations.
    let mut saw_zero = 0_usize;
    let mut saw_non_zero = 0_usize;
    for _ in 0..16 {
        let lease = pool.next();
        let pick: *const Channel = std::sync::Arc::as_ptr(lease.channel());
        if pick == member_zero_ptr {
            saw_zero += 1;
        } else {
            saw_non_zero += 1;
        }
        drop(lease);
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
    // Per-stream RST_STREAM with a terminal reason (CANCEL). The h2
    // connection itself is healthy — only this stream is killed.
    // Classifying it as `ConnectionClosed` (per the older behaviour,
    // which mapped every `is_remote()` to ConnectionClosed) would force
    // pool consumers to recycle a still-good channel and burn retry
    // budgets. The fix surfaces `H2Stream` for terminal per-stream resets
    // and reserves `ConnectionClosed` for GOAWAY / IO failures.
    //
    // CANCEL is deliberately a NON-`REFUSED_STREAM` reason: a refused
    // stream is retry-safe and routes to the distinct `H2StreamRefused`
    // (see `channel_classifies_refused_stream_as_retry_safe`), whereas a
    // CANCEL may have had server-side effect and stays terminal.
    //
    // HTTP/2 spec § 7 (Error Codes):
    //   <https://datatracker.ietf.org/doc/html/rfc9113#name-error-codes>
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
            Ok(msgs) => panic!(
                "expected H2Stream for stream-level RST_STREAM, got {} messages",
                msgs.len()
            ),
        },
    };

    match final_err {
        ChannelError::H2Stream(_) => {
            // expected — terminal per-stream RST, connection still alive.
        }
        ChannelError::ConnectionClosed(msg) => {
            panic!("per-stream RST_STREAM must classify as H2Stream, not ConnectionClosed ({msg})")
        }
        other => panic!("expected H2Stream, got {other:?}"),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn channel_classifies_refused_stream_as_retry_safe() {
    // Finding 4: a per-stream RST_STREAM with reason `REFUSED_STREAM`
    // means the server did not process the stream at all (RFC 7540
    // § 8.1.4), so the RPC is safe to re-dispatch. It must surface as the
    // distinct, retry-safe `H2StreamRefused` rather than the terminal
    // `H2Stream` — otherwise a not-processed reset reaches the caller as a
    // hard failure instead of being retried on the pool.
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
                "expected H2StreamRefused for REFUSED_STREAM, got {} messages",
                msgs.len()
            ),
        },
    };

    match final_err {
        ChannelError::H2StreamRefused(_) => {
            // expected — server never processed the stream; retry-safe.
        }
        other => panic!("REFUSED_STREAM must classify as H2StreamRefused, got {other:?}"),
    }

    // And the crate-level error it converts to must classify as transient
    // for the retry shell — i.e. the RPC is actually retried, not surfaced
    // terminally. The retry classifier is private; the public proxy is the
    // typed `TransportErrorKind::H2StreamRefused` it routes through.
    let err: thetadatadx::Error = final_err.into();
    match err {
        thetadatadx::Error::Transport {
            kind: thetadatadx::error::TransportErrorKind::H2StreamRefused,
            ..
        } => {}
        other => panic!("expected Transport(H2StreamRefused), got {other:?}"),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn channel_classifies_goaway_before_response_head_as_connection_closed() {
    // Server GOAWAYs after receiving the request body without ever
    // sending response HEADERS or DATA. Open-path classification used
    // to wrap every h2 error in `H2Stream(...)` regardless of
    // connection-level scope; the fix routes `ready()` /
    // `send_request()` / `send_data()` failures through
    // `classify_h2_error` so GOAWAY surfaces as `ConnectionClosed`
    // and the pool recycles the dead channel.
    let mock = MockServer::spawn_with_behaviour(
        Vec::new(),
        0,
        String::new(),
        MockBehaviour {
            goaway_pre_response: true,
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
            Ok(msgs) => panic!("expected ConnectionClosed, got {} messages", msgs.len()),
        },
    };

    match final_err {
        ChannelError::ConnectionClosed(_) => {
            // expected — open-path GOAWAY classified as connection-level.
        }
        other => panic!("expected ConnectionClosed, got {other:?}"),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn channel_classifies_connection_drop_after_handshake_as_connection_closed() {
    // Server drops the TCP socket the instant the h2 handshake
    // completes — no HEADERS, no DATA, no GOAWAY. The client's
    // pending `ready()` / `send_request()` observes an h2 IO error.
    // Per the same open-path classification fix that handles GOAWAY,
    // an IO-layer connection loss must surface as `ConnectionClosed`,
    // not `H2Stream`.
    let mock = MockServer::spawn_with_behaviour(
        Vec::new(),
        0,
        String::new(),
        MockBehaviour {
            drop_after_handshake: true,
            ..MockBehaviour::default()
        },
    )
    .await;

    let channel = match Channel::connect_h2c("127.0.0.1", mock.addr.port()).await {
        Ok(c) => c,
        Err(ChannelError::ConnectionClosed(_)) => {
            // Already classified as connection-level at handshake;
            // acceptable terminus for this test.
            return;
        }
        Err(other) => panic!("unexpected connect error: {other:?}"),
    };

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
            Ok(msgs) => panic!("expected ConnectionClosed, got {} messages", msgs.len()),
        },
    };

    match final_err {
        ChannelError::ConnectionClosed(_) => {
            // expected — IO-layer drop classifies as connection-level.
        }
        other => panic!("expected ConnectionClosed, got {other:?}"),
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

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn channel_pool_burst_dispatch_spreads_across_members() {
    // The exact pattern the criterion bench (`grpc_concurrent_burst`)
    // and any `join_all` consumer hits: a synchronous batch of
    // `pool.next()` calls followed by polling the resulting futures.
    // Under the previous picker every `pool.next()` saw
    // `in_flight = 0` because the open path only incremented after
    // `send_request()` succeeded — none of which had run yet at the
    // moment of the picks. The whole batch pinned to one channel.
    //
    // The fix (Finding 4): `pool.next()` returns a `ChannelLease`
    // that pre-reserves a slot on the picked channel synchronously.
    // Subsequent picks in the same batch observe each prior
    // reservation and route around it. We assert the burst spreads
    // across all four pool members; the broken picker would put
    // every pick on member 0.
    let mut mocks = Vec::new();
    let mut channels = Vec::new();
    for _ in 0..4 {
        let mock = MockServer::spawn_with_behaviour(
            vec![make_response_data(&["AAPL"])],
            0,
            String::new(),
            MockBehaviour {
                pre_response_delay: Some(Duration::from_secs(5)),
                ..MockBehaviour::default()
            },
        )
        .await;
        let channel = Channel::connect_h2c("127.0.0.1", mock.addr.port())
            .await
            .expect("h2c connect");
        channels.push(channel);
        mocks.push(mock);
    }
    let pool = ChannelPool::from_channels(channels);

    // Capture channel pointers so picks can be attributed to members.
    let member_ptrs: Vec<*const Channel> = (0..4)
        .map(|i| std::sync::Arc::as_ptr(pool.member_for_test(i)))
        .collect();

    // ── The exact burst-contention shape ───────────────────────────
    // Synchronously call `pool.next()` 16 times, recording the
    // picked channel for each. The leases are held in a Vec so
    // their pre-dispatch reservations stay committed for the full
    // assertion window — mirrors the real-world `join_all` pattern
    // where each pick's dispatch future hasn't run yet at the time
    // the next pick happens.
    let leases: Vec<_> = (0..16).map(|_| pool.next()).collect();
    let picks: Vec<*const Channel> = leases
        .iter()
        .map(|lease| std::sync::Arc::as_ptr(lease.channel()))
        .collect();

    let mut per_member = [0usize; 4];
    for pick in &picks {
        let idx = member_ptrs
            .iter()
            .position(|p| p == pick)
            .expect("picked channel exists in pool");
        per_member[idx] += 1;
    }
    let max_per_member = *per_member.iter().max().expect("non-empty");
    // With 16 picks across 4 channels under a picker that counts
    // pending opens, the spread is exactly 4-per-channel — the
    // round-robin tie-breaker fans the first cycle and the per-
    // channel `in_flight = 1` after each pick redirects each
    // subsequent pick to a still-idle member. Under the broken
    // picker all 16 picks pin to member 0 (per_member = [16, 0, 0, 0]).
    assert_eq!(
        max_per_member, 4,
        "16 picks across 4 channels must distribute exactly 4-per-channel: picks={per_member:?}"
    );
    for (idx, count) in per_member.iter().enumerate() {
        assert_eq!(
            *count, 4,
            "member {idx} must receive exactly 4 picks: counts={per_member:?}"
        );
    }

    // Drop the leases so the reservations return to the pool.
    drop(leases);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn channel_pool_concurrent_dispatch_spreads_across_members() {
    // True-concurrent stress test for the picker's pick/reserve race.
    // N independent tokio tasks call `pool.next()` concurrently;
    // without the CAS commit guard two
    // tasks could both scan, both observe the same least-loaded
    // channel, and both commit reservations — pinning the channel
    // and pushing the rest of the burst to a hot member.
    //
    // The CAS-retry pick in `ChannelPool::next` rolls back the loser
    // of every race and re-scans. The test asserts the resulting
    // spread stays within a reasonable bound — well below the
    // pathological "all on one member" the broken picker produces.
    //
    // `ChannelPool` is clone-cheap (`Arc` inside), but the
    // `ChannelLease<'a>` it returns borrows from its parent — so
    // every concurrent task gets its own `ChannelPool` clone and
    // the lease lives the lifetime of the spawned task. Each task
    // records its picked channel pointer via a shared atomic vec.
    let mut mocks = Vec::new();
    let mut channels = Vec::new();
    for _ in 0..4 {
        let mock = MockServer::spawn_with_behaviour(
            vec![make_response_data(&["AAPL"])],
            0,
            String::new(),
            MockBehaviour::default(),
        )
        .await;
        let channel = Channel::connect_h2c("127.0.0.1", mock.addr.port())
            .await
            .expect("h2c connect");
        channels.push(channel);
        mocks.push(mock);
    }
    let pool = ChannelPool::from_channels(channels);

    // `Channel` pointers are `!Send`; identify each pool member by
    // its address-as-usize so the per-task lookup works across
    // `tokio::spawn` boundaries.
    let member_addrs: Vec<usize> = (0..4)
        .map(|i| std::sync::Arc::as_ptr(pool.member_for_test(i)) as usize)
        .collect();

    // Synchronize N tasks to call `pool.next()` at the same instant
    // via a `tokio::sync::Barrier`. After the pick, every task
    // waits at a second barrier so the leases stay held across the
    // pick-spread assertion window. Each task writes its picked
    // member index into a per-task slot in a shared atomic vec.
    const CONCURRENT: usize = 16;
    let pick_barrier = Arc::new(tokio::sync::Barrier::new(CONCURRENT + 1));
    let hold_barrier = Arc::new(tokio::sync::Barrier::new(CONCURRENT + 1));
    let pick_slots: Arc<Vec<std::sync::atomic::AtomicUsize>> = Arc::new(
        (0..CONCURRENT)
            .map(|_| std::sync::atomic::AtomicUsize::new(usize::MAX))
            .collect(),
    );

    let mut handles = Vec::with_capacity(CONCURRENT);
    for slot_idx in 0..CONCURRENT {
        let pool = pool.clone();
        let pick_barrier = Arc::clone(&pick_barrier);
        let hold_barrier = Arc::clone(&hold_barrier);
        let pick_slots = Arc::clone(&pick_slots);
        let member_addrs = member_addrs.clone();
        handles.push(tokio::spawn(async move {
            // Every task waits on the pick barrier so the
            // `pool.next()` calls fire as simultaneously as the
            // tokio runtime allows. The barrier covers N + 1
            // arrivals — the test driver below is the +1.
            pick_barrier.wait().await;
            let lease = pool.next();
            // Compute the picked-member index synchronously, before
            // the next await — addresses are recovered as `usize`
            // so the value crosses the spawn boundary cleanly.
            let chan_addr = std::sync::Arc::as_ptr(lease.channel()) as usize;
            let idx = member_addrs
                .iter()
                .position(|p| *p == chan_addr)
                .expect("picked channel exists in pool");
            pick_slots[slot_idx].store(idx, std::sync::atomic::Ordering::Release);
            // Hold the lease across the assertion window — the
            // test driver releases this barrier after observing
            // all picks. Dropping the lease here would let the
            // reservation race ahead of the assertion.
            hold_barrier.wait().await;
            drop(lease);
        }));
    }

    // Driver: release the pick barrier so every task issues
    // `pool.next()` concurrently, await each slot's commitment,
    // then release the hold barrier so the leases drop.
    pick_barrier.wait().await;
    // Spin until every slot has been written. AtomicUsize::MAX is
    // the "unset" sentinel.
    let deadline = std::time::Instant::now() + Duration::from_secs(2);
    let observed_per_member: [usize; 4] = loop {
        let mut current = [0usize; 4];
        let mut unset = 0usize;
        for slot in pick_slots.iter() {
            let v = slot.load(std::sync::atomic::Ordering::Acquire);
            if v == usize::MAX {
                unset += 1;
            } else {
                current[v] += 1;
            }
        }
        if unset == 0 {
            break current;
        }
        if std::time::Instant::now() > deadline {
            panic!("{unset} of {CONCURRENT} concurrent picks never committed within deadline");
        }
        tokio::task::yield_now().await;
    };
    hold_barrier.wait().await;
    for h in handles {
        h.await.expect("task joined");
    }

    let max_per_member = *observed_per_member.iter().max().expect("non-empty");
    // Under the CAS-retry picker, even with full concurrency the
    // spread stays tight. 16 picks across 4 channels ideal-balances
    // at 4 per member; we allow at most one over ideal (5) to absorb
    // a single lost CAS race without flapping on the GH-hosted
    // runner. Every channel must also see at least one pick.
    assert!(
        max_per_member <= 5,
        "concurrent burst spread must stay bounded: picks={observed_per_member:?}"
    );
    for (idx, count) in observed_per_member.iter().enumerate() {
        assert!(
            *count > 0,
            "every pool member must receive at least one pick: counts={observed_per_member:?} (member {idx} got {count})"
        );
    }
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
    // twice across the four pool members. Hold the leases in a Vec
    // so each pre-dispatch reservation stays committed — under the
    // pre-reservation picker (Finding 4) the spread for a sustained
    // burst is exactly N/M-per-member, which means picks[0..4] and
    // picks[4..8] both visit each pool member once.
    let leases: Vec<_> = (0..8).map(|_| pool.next()).collect();
    let picks: Vec<*const Channel> = leases
        .iter()
        .map(|lease| std::sync::Arc::as_ptr(lease.channel()))
        .collect();
    assert_eq!(picks[0], picks[4]);
    assert_eq!(picks[1], picks[5]);
    assert_eq!(picks[2], picks[6]);
    assert_eq!(picks[3], picks[7]);
    // The four distinct addresses must all be different.
    let mut distinct: Vec<*const Channel> = picks[..4].to_vec();
    distinct.sort();
    distinct.dedup();
    assert_eq!(distinct.len(), 4, "first four picks are distinct channels");
    drop(leases);
}

// ─── h2 protocol-level coverage: PING / WINDOW_UPDATE / MAX_CONCURRENT_STREAMS ──
//
// The three tests below exercise h2 control-frame paths the rest of
// the suite leaves implicit:
//
// * `channel_keepalive_survives_server_ping`: mock pumps server-
//   initiated PING frames; the client's reader must answer PONG and
//   keep the connection healthy for a follow-up RPC.
// * `channel_observes_small_initial_window`: mock clamps the per-
//   stream INITIAL_WINDOW_SIZE so a multi-chunk response only
//   advances when the client emits WINDOW_UPDATE. Asserts the response
//   completes (i.e. the client did emit WINDOW_UPDATE; otherwise the
//   stream would stall and the test would time out).
// * `channel_pool_routes_around_saturated_member`: per-channel
//   MAX_CONCURRENT_STREAMS = 1 + a slow in-flight RPC saturate one
//   pool member; the next pool pick must route to the second member
//   rather than blocking on the saturated one.

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn channel_keepalive_survives_server_ping() {
    // Pump a PING every 50ms. The connection sits idle between the
    // first and second RPC long enough for several PONG round-trips
    // to actually exercise the keep-alive code path; a broken
    // keep-alive (no PONG) would surface as the second RPC failing
    // or the connection already being torn down. We assert that a
    // second RPC on the same channel completes after the idle window
    // — that is the load-bearing keep-alive contract.
    //
    // The mock's PING driver signals `ping_pongs_done` after 5
    // successful PING/PONG round-trips, so the test waits
    // deterministically on the Notify rather than sleeping for a
    // wall-clock interval (avoids fixed-sleep barriers).
    let ping_pongs_done = Arc::new(Notify::new());
    let chunks = vec![make_response_data(&["AAPL"])];
    let mock = MockServer::spawn_with_behaviour(
        chunks.clone(),
        0,
        String::new(),
        MockBehaviour {
            inject_ping: Some(Duration::from_millis(50)),
            ping_pong_signal: Some((Arc::clone(&ping_pongs_done), 5)),
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
            empty_request(),
        )
        .await
        .expect("first RPC opens");
    let _first = collect(stream).await.expect("first RPC completes");

    // Wait for 5 PING/PONG round-trips to complete — exercises the
    // keep-alive code path deterministically. The 5s timeout is a
    // runaway protector; the notify normally fires within ~250ms
    // (5 round-trips * 50ms PING interval).
    tokio::time::timeout(Duration::from_secs(5), ping_pongs_done.notified())
        .await
        .expect("5 PING/PONG round-trips completed within 5s");

    // Second RPC on the same channel after the keep-alive window.
    // The mock's `accept().await` loop multiplexes streams over the
    // single TCP+h2 connection, so the same channel can drive a
    // fresh stream without re-handshaking; if the connection has
    // been torn down by a missed PONG round-trip this open call
    // surfaces a connection-level error.
    let second = channel
        .server_streaming::<DataValueList, ResponseData>(
            "/BetaEndpoints.BetaThetaTerminal/GetStockListSymbols",
            empty_request(),
        )
        .await
        .expect("second RPC must open on the same channel after keep-alive idle");
    let second_msgs = collect(second).await.expect("second RPC completes");
    assert_eq!(
        second_msgs.len(),
        chunks.len(),
        "second RPC must deliver the full chunk count over the surviving connection",
    );

    drop(channel);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn channel_observes_small_initial_window() {
    // 1 KiB initial flow-control window. The mock fans out four
    // ~1 KiB DATA chunks; without WINDOW_UPDATE from the client the
    // server stalls after the first frame and the stream never
    // completes. A successful collect proves the client emitted
    // WINDOW_UPDATE as it consumed each chunk.
    let big_payload = "A".repeat(1024);
    let chunks = vec![
        make_response_data(&[big_payload.as_str()]),
        make_response_data(&[big_payload.as_str()]),
        make_response_data(&[big_payload.as_str()]),
        make_response_data(&[big_payload.as_str()]),
    ];
    let mock = MockServer::spawn_with_behaviour(
        chunks.clone(),
        0,
        String::new(),
        MockBehaviour {
            clamp_initial_window: Some(1024),
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
            empty_request(),
        )
        .await
        .expect("RPC opens under clamped window");

    // If the client did not emit WINDOW_UPDATE the stream would
    // stall and `collect` would never return. tokio's test runtime
    // bounds the wait so a regression fails as a hang/timeout
    // rather than a wrong-value assertion.
    let messages = tokio::time::timeout(Duration::from_secs(5), collect(stream))
        .await
        .expect("stream completes within timeout (WINDOW_UPDATE flow-control healthy)")
        .expect("rpc completes ok");
    assert_eq!(messages.len(), chunks.len(), "every chunk delivered");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn channel_pool_routes_around_saturated_member() {
    // Two mocks; the first advertises MAX_CONCURRENT_STREAMS=1 and
    // holds the in-flight RPC open via `pre_response_delay`. The
    // second is a normal fast responder. The pool's `next()` must
    // pick the second mock once the first is saturated.
    let slow_mock = MockServer::spawn_with_behaviour(
        vec![make_response_data(&["AAPL"])],
        0,
        String::new(),
        MockBehaviour {
            max_concurrent_streams: Some(1),
            pre_response_delay: Some(Duration::from_millis(500)),
            ..MockBehaviour::default()
        },
    )
    .await;
    let fast_mock = MockServer::spawn(vec![make_response_data(&["MSFT"])], 0).await;

    let slow_channel = Channel::connect_h2c("127.0.0.1", slow_mock.addr.port())
        .await
        .expect("h2c connect (slow)");
    let fast_channel = Channel::connect_h2c("127.0.0.1", fast_mock.addr.port())
        .await
        .expect("h2c connect (fast)");
    let pool = ChannelPool::from_channels(vec![slow_channel, fast_channel]);

    // Saturate the slow channel: open one RPC that holds the only
    // slot on that channel for 500ms.
    let saturation = pool.next();
    let saturating_channel = std::sync::Arc::as_ptr(saturation.channel());
    let saturation_stream = saturation
        .channel()
        .server_streaming::<DataValueList, ResponseData>(
            "/BetaEndpoints.BetaThetaTerminal/GetStockListSymbols",
            empty_request(),
        );

    // Confirm the next pool pick lands on a DIFFERENT channel — the
    // pre-dispatch reservation on the saturated one must steer
    // `next()` to the only other member with capacity.
    let routed = pool.next();
    let routed_channel = std::sync::Arc::as_ptr(routed.channel());
    assert_ne!(
        saturating_channel, routed_channel,
        "pool.next() must route around the saturated channel"
    );

    // Sanity: the second pick can actually serve an RPC end-to-end
    // (the fast mock).
    let fast_stream = routed
        .channel()
        .server_streaming::<DataValueList, ResponseData>(
            "/BetaEndpoints.BetaThetaTerminal/GetStockListSymbols",
            empty_request(),
        )
        .await
        .expect("fast-channel RPC opens");
    let fast_messages = collect(fast_stream).await.expect("fast-channel RPC ok");
    assert_eq!(fast_messages.len(), 1);

    // Drain the saturating RPC so the pool lease drops cleanly
    // (helps the mock task exit before the test returns).
    let saturation_stream = saturation_stream.await.expect("slow-channel RPC opens");
    let _ = collect(saturation_stream)
        .await
        .expect("slow-channel RPC ok");

    // Touch `slow_addr` so the binding's lifetime extends to the
    // end of the test (the live mock listener is dropped here, not
    // earlier by an aggressive optimiser pass).
    let _slow_addr = slow_mock.addr;
}
