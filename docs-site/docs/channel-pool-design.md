---
title: Channel pool design — in-place reconnect
---

# Channel pool — in-place reconnect on `ConnectionClosed`

This page documents the gRPC channel pool's reconnect contract and
the operational guarantees long-running clients can rely on.

## Symptom

Long-running clients running 8-concurrent (or any sustained
concurrency above 1) against MDDS gRPC observed a ConnectionClosed
regression where, after several hours of uptime, every subsequent
RPC failed with `TransportErrorKind::ConnectionClosed` regardless of
the date range or endpoint. The pool never recovered without a
process restart.

## Root cause

`crate::grpc::pool::ChannelPool` (versions through PR #588) marked
each channel dead on the first observed `ConnectionClosed` and had
no recycling counterpart. Production h2 connections occasionally
drop: hosted MDDS rotates connections on a scheduled cadence
(server-side GOAWAY), network blips happen, tokio runtime hiccups
can surface as `inactive stream` errors. Each drop flipped one
channel's death flag permanently. Over a long enough uptime every
pool member accumulated a drop, and the picker's last-resort
fallback routed every subsequent RPC through a permanently-dead
channel — instant `ConnectionClosed` forever.

The previous transport (tonic 0.x via `tonic::transport::Channel`)
did not exhibit this because tonic's channel reconnects transparently
on connection-level faults. PR #524 swapped tonic for the in-house
`h2` transport for two-stage decode pipeline reasons, and the
recycling contract was not carried over.

## Fix

`Channel` now holds its `SendRequest<Bytes>` behind an
[`ArcSwap`](https://docs.rs/arc-swap) keyed on a captured
`ConnectTarget` (host, port, optional TLS config, scheme,
max-message-size). On the first observed `ConnectionClosed`:

1. The classifier triggers `Channel::trigger_reconnect`, which wins
   an `AtomicBool` single-flight CAS to claim sole responsibility
   for the reconnect.
2. Losers of the CAS return immediately — concurrent observers do
   not open redundant TCP+TLS+h2 sessions.
3. The winner spawns a background task that re-opens the connection
   with bounded exponential backoff (50 ms initial, capped at 30 s,
   8 attempts max).
4. On success the `ArcSwap` atomically swaps in the fresh
   `SendRequest<Bytes>`; the previous h2 connection driver is
   aborted; subsequent RPCs picking the channel see the new sender
   transparently.
5. On exhaustion the inner sender stays unchanged; the next
   observer of `ConnectionClosed` triggers a fresh reconnect cycle.

The pool slot is never marked dead, never replaced, never skipped —
the same `Arc<Channel>` handle stays in the pool for its entire
lifetime, only its inner h2 session swaps.

## Caller-side semantics

The user-facing retry loop (`crate::mdds::macros::classify_error`)
classifies `TransportErrorKind::ConnectionClosed` as `Transient` and
re-dispatches once. By the time the retry runs, either:

* The reconnect winner finished its handshake and the same channel
  serves the retry on its fresh sender; or
* The pool's load-balancing picker routes the retry to a sibling
  channel whose h2 connection is still healthy.

The user sees the call complete normally. The only way a
`ConnectionClosed` surfaces to the caller is if the reconnect
exhausts its retry budget AND every sibling channel is mid-reconnect
simultaneously — by design a rare-event signal that the upstream is
genuinely unreachable.

## Verifying

The in-tree integration test
`crates/thetadatadx/tests/test_pool_reconnect.rs` covers:

* Force-kill the underlying TCP connection on every pool member
  mid-stream → subsequent RPCs succeed via transparent reconnect.
* 1 000 RPCs against a synthetic-failure mock that drops connections
  every N RPCs → pool never enters a permanently-dead state.
* 100 concurrent observers of the same `ConnectionClosed` → exactly
  one fresh TCP connection opened to the server (single-flight
  guarantee).

For a production sanity check, run the long-running smoke test
`THETADX_LIVE_CREDS=… cargo test --test reconnect_storm -- --ignored`
across a 60-minute window with 8-concurrent dispatch; the pool's
reconnect-event metric (`thetadatadx.grpc.channel.reconnects_total`)
should increment as connections rotate without any RPC failing
upward to the caller.
