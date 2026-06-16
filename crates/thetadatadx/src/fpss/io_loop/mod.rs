//! FPSS I/O worker thread, login handshake, and ping heartbeat.
//!
//! [`io_loop`] owns the TLS stream for the lifetime of a session. It reads
//! frames, dispatches them through [`super::decode::decode_frame`], publishes
//! the resulting events into the event ring, and drains the outgoing
//! command channel between reads. On involuntary disconnect it re-runs the
//! login handshake in-place according to [`ReconnectPolicy`].
//!
//! Sub-modules:
//!
//! - [`login`] -- handshake (`wait_for_login` + `LoginResult`).
//! - [`ping`] -- background heartbeat scheduler.
//!
//! This file owns the main blocking read + Disruptor publish + command
//! drain loop and the auto-reconnect state machine, neither of which
//! decompose without leaking session state.

mod login;
mod ping;

pub(in crate::fpss) use login::{wait_for_login, LoginResult};
pub(in crate::fpss) use ping::ping_loop;

use std::collections::HashMap;
use std::io::BufReader;
use std::io::Write;
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU64, Ordering};
use std::sync::mpsc as std_mpsc;
use std::sync::{Arc, LazyLock, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use disruptor::{build_single_producer, EventPoller, SingleProducerBarrier};
use metrics::Counter;

// ─── Hoisted I/O-loop counter handles ───────────────────────────────
//
// Mirrors the `decode.rs` hoisting pattern: `metrics::counter!(name)`
// resolves the metric handle through the global recorder on every
// call (~30 ns per hit observed in the decode bench). The two
// io_loop counters fire on hot-but-not-per-tick paths
// (`drain_yields` on every mid-frame yield, `reconnects` on every
// reconnect attempt), but the lookup pattern is the same and there
// is no reason to leave them un-hoisted now that the surrounding
// counters are.
//
// One handle per metric name. `Counter::increment` is `&self` so a
// single `LazyLock<Counter>` serves every call site that fires the
// same metric.
static FPSS_DRAIN_YIELDS: LazyLock<Counter> =
    LazyLock::new(|| metrics::counter!("thetadatadx.fpss.drain_yields"));
static FPSS_RECONNECTS: LazyLock<Counter> =
    LazyLock::new(|| metrics::counter!("thetadatadx.fpss.reconnects"));

use crate::tdbe::types::enums::{RemoveReason, StreamMsgType};

use crate::auth::Credentials;
use crate::backoff::{BackoffSchedule, JitterMode};
use crate::config::{
    HostSelectionPolicy, ReconnectAttemptClass, ReconnectAttemptLimits, ReconnectPolicy,
    StreamingFlushMode, RATE_LIMITED_JITTER_WINDOW,
};
use crate::error::Error;

use super::connection;
use super::decode::decode_frame;
use super::delta::DeltaState;
use super::events::{FpssEventInternal, IoCommand, StreamControl};
use super::framing::{
    self, is_drain_yield, read_frame_into_with_stall_timeout, write_frame, write_raw_frame,
    write_raw_frame_no_flush, Frame, FrameReadState,
};
use super::protocol::{self, build_credentials_payload, Contract};
use super::reconnect_delay;
use super::ring::{
    self, AdaptiveWaitStrategy, RingCursors, RingEvent, RingProducer, SequencedProducer,
};

type ActiveSubs = Arc<Mutex<Vec<(super::protocol::SubscriptionKind, Contract)>>>;
type ActiveFullSubs = Arc<
    Mutex<
        Vec<(
            super::protocol::SubscriptionKind,
            crate::tdbe::types::enums::SecType,
        )>,
    >,
>;

// ---------------------------------------------------------------------------
// I/O thread: blocking read + Disruptor publish + command drain
// ---------------------------------------------------------------------------

/// The I/O thread owns the TLS stream. It does three things in a loop:
///
/// 1. Attempt a blocking read (with short timeout) for incoming frames
/// 2. Drain the command channel for outgoing writes (subscribe, ping, etc.)
/// 3. Publish decoded events into the Disruptor ring
///
/// On involuntary disconnect, the reconnection policy determines whether
/// to automatically re-establish the connection within this same thread
/// (no new threads spawned).
///
/// This thread IS the Disruptor producer. Events flow directly from the TLS
/// socket into the ring buffer with zero intermediate channels.
///
/// Argument bundle for [`io_loop`].
///
/// `connect_timeout` and `read_timeout` plumb the user-supplied
/// [`crate::config::StreamingConfig`] tuning into the auto-reconnect path
/// (so manual [`std::net::TcpStream::connect_timeout`] re-attempts and
/// the framing-layer mid-frame stall budget honour the configured
/// values, not the parity-reference hardcoded defaults).
///
/// `producer` is the ring publisher built by the caller via
/// [`build_poller_producer`]. The io_loop only ever calls
/// [`RingProducer::try_publish`] on it; the consumer side is driven
/// on the caller's own thread through `StreamingClient::next_event` /
/// `poll_batch` / `for_each` (or by the per-binding dispatcher thread
/// each language SDK owns).
pub(in crate::fpss) struct IoLoopArgs<P> {
    pub stream: connection::FpssStream,
    pub cmd_rx: std_mpsc::Receiver<IoCommand>,
    pub producer: P,
    pub ring_size: usize,
    pub shutdown: Arc<AtomicBool>,
    pub authenticated: Arc<AtomicBool>,
    pub permissions: String,
    pub pending_control: Vec<StreamControl>,
    pub derive_ohlcvc: bool,
    pub flush_mode: StreamingFlushMode,
    pub policy: ReconnectPolicy,
    /// Mirrors [`crate::config::ReconnectConfig::wait_ms`]: the
    /// initial delay of the generic-transient exponential ladder the
    /// [`ReconnectPolicy::Auto`] arm drives.
    pub wait_ms: u64,
    /// Mirrors [`crate::config::ReconnectConfig::wait_max_ms`]: the
    /// cap on the generic-transient ladder.
    pub wait_max_ms: u64,
    /// Mirrors [`crate::config::ReconnectConfig::wait_rate_limited_ms`].
    /// Floor delay for `TooManyRequests` drops; jitter samples
    /// `[floor, floor + RATE_LIMITED_JITTER_WINDOW]`.
    pub wait_rate_limited_ms: u64,
    /// Mirrors [`crate::config::ReconnectConfig::wait_server_restart_ms`].
    /// Flat cadence for `ServerRestarting` drops.
    pub wait_server_restart_ms: u64,
    /// Mirrors [`crate::config::ReconnectConfig::jitter`]. Applied to
    /// every computed reconnect delay.
    pub jitter: JitterMode,
    /// Mirrors [`crate::config::ReconnectConfig::replay_burst_size`].
    pub replay_burst_size: u32,
    /// Mirrors [`crate::config::ReconnectConfig::replay_pace_ms`].
    pub replay_pace_ms: u64,
    pub creds: Credentials,
    /// Declared FPSS host list. Reconnect re-applies the configured
    /// selection policy to this list, optionally pinning the last
    /// stable host first.
    pub hosts: Vec<(String, u16)>,
    pub host_selection: HostSelectionPolicy,
    pub host_shuffle_seed: u64,
    pub active_subs: ActiveSubs,
    pub active_full_subs: ActiveFullSubs,
    pub dropped: Arc<AtomicU64>,
    pub connect_timeout: Duration,
    pub read_timeout: Duration,
    /// Per-iteration blocking-read slice. Mirrors
    /// [`crate::config::StreamingConfig::io_read_slice_ms`].
    pub io_read_slice: Duration,
    /// Last-frame watchdog deadline; [`Duration::ZERO`] disables.
    /// Mirrors [`crate::config::StreamingConfig::data_watchdog_ms`].
    pub data_watchdog: Duration,
    /// Keepalive schedule for reconnect-time socket construction.
    pub keepalive: connection::TcpKeepaliveSpec,
    /// Wall-clock receive timestamp (UNIX nanoseconds; `0` = never) of
    /// the most recent inbound frame of any kind. Shared with the
    /// owning client so `millis_since_last_event()` reads it without
    /// touching I/O-thread state.
    pub last_event_at_ns: Arc<AtomicI64>,
    /// Address of the server this session is currently connected to.
    /// Updated after every successful (re)connect + login; read by
    /// `last_connected_addr()` on the owning client.
    pub connected_addr: Arc<Mutex<String>>,
    /// Shared monotonic request-id counter. The auto-reconnect path
    /// allocates fresh `req_id` values from this counter for each
    /// re-subscribe so `ReqResponse` events on the reconnected session
    /// carry ids correlatable to the original subscribe rather than
    /// the indistinguishable `-1` sentinel.
    ///
    /// Widened to `AtomicI64`; the wire boundary clamps to a positive
    /// `i32` via [`super::wire_req_id`].
    pub next_req_id: Arc<AtomicI64>,
}

fn host_index(hosts: &[(String, u16)], addr: &str) -> Option<usize> {
    hosts
        .iter()
        .position(|(host, port)| format!("{host}:{port}") == addr)
}

// Reason: all parameters are moved into this function from a spawned thread closure.
#[allow(clippy::needless_pass_by_value, clippy::too_many_lines)]
pub(in crate::fpss) fn io_loop<P>(args: IoLoopArgs<P>)
where
    P: RingProducer,
{
    let IoLoopArgs {
        stream,
        cmd_rx,
        mut producer,
        ring_size,
        shutdown,
        authenticated,
        permissions,
        mut pending_control,
        derive_ohlcvc,
        flush_mode,
        policy,
        wait_ms,
        wait_max_ms,
        wait_rate_limited_ms,
        wait_server_restart_ms,
        jitter,
        replay_burst_size,
        replay_pace_ms,
        creds,
        hosts,
        host_selection,
        host_shuffle_seed,
        active_subs,
        active_full_subs,
        dropped,
        connect_timeout,
        read_timeout,
        io_read_slice,
        data_watchdog,
        keepalive,
        last_event_at_ns,
        connected_addr,
        next_req_id,
    } = args;
    // `ring_size` was validated upstream by `ring::check_ring_size` at
    // the public `StreamingClient::connect` boundary; silent rounding here
    // would rewrite the caller's stated buffer budget after the fact.
    debug_assert!(
        ring_size >= ring::MIN_RING_SIZE && ring_size.is_power_of_two(),
        "io_loop received unvalidated ring_size {ring_size}; check upstream StreamingClientBuilder",
    );

    // The producer was built by the caller via
    // [`build_poller_producer`]. From here on the io_loop only
    // publishes into the ring via [`RingProducer::try_publish`]; the
    // consumer side runs on the caller's own thread (or each binding's
    // dispatcher thread) and drains the ring independently.

    // Publish every handshake-time typed control frame in wire order.
    // Emitted BEFORE LoginSuccess so the user sees exactly the sequence
    // the server delivered: every `Connected` / `Ping` / `ReconnectedServer`
    // / `Restart` that preceded METADATA, followed by LoginSuccess itself.
    // Without this, any of these frames that arrived during the handshake
    // were silently dropped because the handshake loop consumed them
    // before the post-login `decode_frame` dispatch ran.
    //
    // `try_publish` (rather than blocking `publish`) keeps the io_loop
    // thread non-blocking on a full ring — drops are surfaced via the
    // shared `dropped` counter and a `warn` log, never wedge the
    // reader. See `ring.rs` for the policy contract.
    for ctrl in pending_control.drain(..) {
        if producer
            .try_publish(|slot| {
                slot.event = FpssEventInternal::Control(ctrl);
            })
            .is_err()
        {
            dropped.fetch_add(1, Ordering::Relaxed);
            tracing::warn!(
                target: "thetadatadx::fpss::io_loop",
                "ring full while publishing pre-login control frame; dropped",
            );
        }
    }

    // Publish login success event (non-blocking — same policy as above).
    if producer
        .try_publish(|slot| {
            slot.event = FpssEventInternal::Control(StreamControl::LoginSuccess { permissions });
        })
        .is_err()
    {
        dropped.fetch_add(1, Ordering::Relaxed);
        tracing::warn!(
            target: "thetadatadx::fpss::io_loop",
            "ring full while publishing LoginSuccess; dropped",
        );
    }

    // Split the stream into buffered read + buffered write.
    let mut reader = BufReader::new(stream);

    // Per-contract delta state for FIT decompression.
    let mut delta_state: DeltaState = DeltaState::new();

    // Thread-local contract cache: contract_id -> Arc<Contract>.
    // Populated on ContractAssigned events, used by the decode hot path to
    // attach the parsed contract to every emitted data event with zero
    // Mutex locks. Downstream consumers that still want an id->contract
    // map build it from the `ContractAssigned` event stream — the SDK no
    // longer holds wire-internal `contract_id` state.
    let mut local_contracts: HashMap<i32, Arc<Contract>> = HashMap::new();

    // Reusable frame payload buffer.
    let mut frame_buf: Vec<u8> = Vec::with_capacity(framing::MAX_PAYLOAD_LEN);

    // Per-frame resumption state. Preserved across `read_frame_into`
    // calls so a drain-yield can hand control back to the command
    // drain without losing the bytes already delivered by the TLS
    // socket. Reset to idle on every complete frame.
    let mut frame_state = FrameReadState::new();

    // Outer reconnection loop: each iteration runs one connection session.
    // On involuntary disconnect, the policy decides whether to reconnect.
    //
    // Attempt counters split by failure class
    // ([`ReconnectAttemptClass`]) so a rate-limited transient
    // (`TooManyRequests`, 130 s spacing) does not burn through the
    // generic transient budget meant for fast TimedOut / Unspecified
    // retries, and a `ServerRestarting` pool bounce gets its own
    // evenly-paced window. Each counter resets to zero on a successful
    // read; an additional time-based reset fires when the connection
    // has been running cleanly for at least
    // `ReconnectAttemptLimits::stable_window`, so a connection that
    // ran cleanly for a minute before dropping picks up the full
    // budget again rather than inheriting the previous cycle's count.
    let mut reconnect_state = ReconnectCounters::new(BackoffSchedule::new(
        Duration::from_millis(wait_ms),
        Duration::from_millis(wait_max_ms),
    ));

    // The read deadline is enforced on a wall clock rather than by
    // counting timeout slices: `last_frame_at` advances on every
    // complete inbound frame, and a slice that expires with
    // `last_frame_at.elapsed() >= read_timeout` declares the session
    // dead. Slice-count accounting drifted (each slice is "roughly"
    // `io_read_slice` long, plus scheduling), so the deadline now
    // holds regardless of the configured slice size.
    let read_timeout_ms_total = u64::try_from(read_timeout.as_millis()).unwrap_or(u64::MAX);
    let mut current_host = host_index(
        &hosts,
        &connected_addr
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone(),
    );
    let mut last_known_good_host = None;

    'session: loop {
        // Session-local liveness clock: starts at session entry so a
        // session that never delivers a frame still times out exactly
        // `read_timeout` after it began, and feeds the shared
        // `last_event_at_ns` operator-facing staleness clock.
        let mut last_frame_at = Instant::now();

        // --- Inner read/write loop for one connection session ---
        // When the inner loop breaks, `disconnect_reason` holds the reason.
        let disconnect_reason: RemoveReason = 'inner: loop {
            if shutdown.load(Ordering::Relaxed) {
                break 'session;
            }

            // --- Phase 1: Try to read a frame (short blocking read) ---
            match read_frame_into_with_stall_timeout(
                &mut reader,
                &mut frame_buf,
                &mut frame_state,
                read_timeout,
            ) {
                Ok(Some((code, payload_len))) => {
                    last_frame_at = Instant::now();
                    last_event_at_ns.store(unix_nanos_now(), Ordering::Relaxed);
                    // Reset reconnect counters on successful data reception
                    // and mark "data did flow on this session" so the
                    // stable-window check on the next drop knows whether
                    // the connection ran long enough to deserve a fresh
                    // retry budget.
                    reconnect_state.reset_counters();
                    reconnect_state.note_data_received();

                    let (primary, secondary) = decode_frame(
                        code,
                        &frame_buf[..payload_len],
                        &authenticated,
                        &mut local_contracts,
                        &shutdown,
                        &mut delta_state,
                        derive_ohlcvc,
                    );

                    if let Some(evt) = primary {
                        if producer
                            .try_publish(|slot| {
                                slot.event = evt;
                            })
                            .is_err()
                        {
                            // Ring buffer full: consumer fell behind.
                            // Count the drop and keep reading — the
                            // alternative (blocking `publish`) would
                            // stall the TLS reader and cause the
                            // vendor session to drop on a slow
                            // user callback.
                            dropped.fetch_add(1, Ordering::Relaxed);
                        }
                    }
                    if let Some(evt) = secondary {
                        if producer
                            .try_publish(|slot| {
                                slot.event = evt;
                            })
                            .is_err()
                        {
                            dropped.fetch_add(1, Ordering::Relaxed);
                        }
                    }
                }
                Ok(None) => {
                    // Clean EOF
                    tracing::warn!("FPSS connection closed by server");
                    if producer
                        .try_publish(|slot| {
                            slot.event = FpssEventInternal::Control(StreamControl::Disconnected {
                                reason: RemoveReason::Unspecified,
                            });
                        })
                        .is_err()
                    {
                        dropped.fetch_add(1, Ordering::Relaxed);
                        tracing::warn!(
                            target: "thetadatadx::fpss::io_loop",
                            "ring full while publishing Disconnected (Unspecified); dropped",
                        );
                    }
                    authenticated.store(false, Ordering::Release);
                    break 'inner RemoveReason::Unspecified;
                }
                Err(ref e) if is_read_timeout(e) => {
                    let quiet = last_frame_at.elapsed();
                    let read_deadline_hit = quiet >= read_timeout;
                    // Last-frame watchdog: a hard wall-clock backstop
                    // above the read timeout. With the default 3 s
                    // read timeout the deadline above fires first;
                    // the watchdog catches configurations that widen
                    // the read timeout past the watchdog window.
                    let watchdog_hit = !data_watchdog.is_zero() && quiet >= data_watchdog;
                    if read_deadline_hit || watchdog_hit {
                        if watchdog_hit && !read_deadline_hit {
                            tracing::warn!(
                                watchdog_ms =
                                    u64::try_from(data_watchdog.as_millis()).unwrap_or(u64::MAX),
                                quiet_ms = u64::try_from(quiet.as_millis()).unwrap_or(u64::MAX),
                                "FPSS last-frame watchdog tripped; forcing reconnect",
                            );
                        } else {
                            tracing::warn!(
                                timeout_ms = read_timeout_ms_total,
                                quiet_ms = u64::try_from(quiet.as_millis()).unwrap_or(u64::MAX),
                                "FPSS read timed out (no frames inside the read deadline)",
                            );
                        }
                        if producer
                            .try_publish(|slot| {
                                slot.event =
                                    FpssEventInternal::Control(StreamControl::Disconnected {
                                        reason: RemoveReason::TimedOut,
                                    });
                            })
                            .is_err()
                        {
                            dropped.fetch_add(1, Ordering::Relaxed);
                            tracing::warn!(
                                target: "thetadatadx::fpss::io_loop",
                                "ring full while publishing Disconnected (TimedOut); dropped",
                            );
                        }
                        authenticated.store(false, Ordering::Release);
                        break 'inner RemoveReason::TimedOut;
                    }
                    // Otherwise, fall through to drain commands.
                }
                Err(ref e) if is_drain_yield(e) => {
                    // Mid-frame reader yielded so the command drain can
                    // keep up. `frame_state` has
                    // been updated with the exact byte offset in the
                    // header / payload, so the next `read_frame_into`
                    // call resumes without desync. Do NOT count this
                    // toward `consecutive_timeouts` -- the TLS socket
                    // IS delivering bytes, just slowly; a sustained
                    // drain-yield is expected behaviour on a trickling
                    // sender, not a sign of a dead connection. Fall
                    // through to the Phase 2 drain.
                    FPSS_DRAIN_YIELDS.increment(1);
                    tracing::trace!(
                        "mid-frame drain-yield -- draining outbound commands \
                         before re-entering read"
                    );
                }
                Err(e) => {
                    tracing::error!(error = %e, "FPSS read error");
                    if producer
                        .try_publish(|slot| {
                            slot.event = FpssEventInternal::Control(StreamControl::Disconnected {
                                reason: RemoveReason::Unspecified,
                            });
                        })
                        .is_err()
                    {
                        dropped.fetch_add(1, Ordering::Relaxed);
                        tracing::warn!(
                            target: "thetadatadx::fpss::io_loop",
                            "ring full while publishing Disconnected (read error); dropped",
                        );
                    }
                    authenticated.store(false, Ordering::Release);
                    break 'inner RemoveReason::Unspecified;
                }
            }

            // --- Phase 2: Drain command channel (non-blocking) ---
            loop {
                match cmd_rx.try_recv() {
                    Ok(IoCommand::WriteFrame { code, payload }) => {
                        let writer = reader.get_mut();
                        let result = if code == StreamMsgType::Ping
                            || flush_mode == StreamingFlushMode::Immediate
                        {
                            write_raw_frame(writer, code, &payload)
                        } else {
                            write_raw_frame_no_flush(writer, code, &payload)
                        };
                        if let Err(e) = result {
                            tracing::warn!(error = %e, "failed to write frame");
                        }
                    }
                    Ok(IoCommand::Shutdown) => {
                        let stop_payload = protocol::build_stop_payload();
                        let writer = reader.get_mut();
                        // Best-effort STOP: we're about to tear down the
                        // socket anyway, so write failure here is not
                        // actionable. But silent failure masks half-closed
                        // sockets and kernel buffer exhaustion -- surface
                        // the error so operators can diagnose kernel-side
                        // issues from logs rather than from stream
                        // truncation alone.
                        if let Err(e) = write_raw_frame(writer, StreamMsgType::Stop, &stop_payload)
                        {
                            tracing::warn!(
                                error = %e,
                                "failed to send STOP frame on shutdown"
                            );
                        }
                        tracing::debug!("sent STOP, I/O thread exiting");
                        shutdown.store(true, Ordering::Release);
                        break;
                    }
                    Err(std_mpsc::TryRecvError::Empty) => break,
                    Err(std_mpsc::TryRecvError::Disconnected) => {
                        tracing::debug!("command channel disconnected, I/O thread exiting");
                        shutdown.store(true, Ordering::Release);
                        break;
                    }
                }
            }
        }; // end 'inner loop (yields RemoveReason)

        // If shutdown was requested (explicit or channel disconnect), exit entirely.
        if shutdown.load(Ordering::Relaxed) {
            break 'session;
        }

        // --- Reconnection decision ---
        let reason = disconnect_reason;

        // Helper shape: publish the terminal "auto-recovery has
        // stopped" event before every break that is not a
        // user-initiated shutdown, so operators can distinguish
        // budget exhaustion from a clean `shutdown()` call.
        macro_rules! publish_exhausted {
            ($attempts:expr) => {
                if producer
                    .try_publish(|slot| {
                        slot.event =
                            FpssEventInternal::Control(StreamControl::ReconnectsExhausted {
                                reason,
                                attempts: $attempts,
                            });
                    })
                    .is_err()
                {
                    dropped.fetch_add(1, Ordering::Relaxed);
                    tracing::warn!(
                        target: "thetadatadx::fpss::io_loop",
                        "ring full while publishing ReconnectsExhausted; dropped",
                    );
                }
            };
        }

        let (delay, reconnect_attempt) = match &policy {
            ReconnectPolicy::Manual => {
                tracing::info!(reason = ?reason, "manual reconnect policy -- not reconnecting");
                publish_exhausted!(0);
                break 'session;
            }
            ReconnectPolicy::Auto(limits) => {
                // Permanent reasons short-circuit before consulting any
                // budget — no amount of retrying will fix bad credentials.
                let Some(class) = ReconnectAttemptLimits::class_for(reason) else {
                    tracing::error!(reason = ?reason, "permanent disconnect -- not reconnecting");
                    publish_exhausted!(0);
                    break 'session;
                };
                // Optional time-based reset BEFORE incrementing. A
                // session that ran cleanly for >= `stable_window`
                // before this drop earns a fresh budget across all
                // classes.
                if reconnect_state.maybe_reset_after_stable(limits) {
                    last_known_good_host = current_host;
                }
                let attempt = reconnect_state.record(class);
                let budget = limits.budget_for(class);
                // Two stop conditions, whichever trips first: the
                // per-class attempt budget and (for the classes it
                // applies to) the wall-clock envelope measured from
                // the first attempt of this consecutive-reconnect
                // sequence.
                let envelope_spent = ReconnectAttemptLimits::elapsed_budget_applies(class)
                    && !limits.max_elapsed.is_zero()
                    && reconnect_state.burst_elapsed() > limits.max_elapsed;
                if attempt > budget || envelope_spent {
                    let attempts_consumed = attempt - 1;
                    if envelope_spent {
                        tracing::error!(
                            attempts = attempts_consumed,
                            class = ?class,
                            max_elapsed_ms =
                                u64::try_from(limits.max_elapsed.as_millis()).unwrap_or(u64::MAX),
                            "reconnect wall-clock envelope exhausted, giving up"
                        );
                    } else {
                        tracing::error!(
                            attempts = attempts_consumed,
                            class = ?class,
                            "max reconnect attempts reached for this class, giving up"
                        );
                    }
                    publish_exhausted!(attempts_consumed);
                    break 'session;
                }
                let delay = match class {
                    ReconnectAttemptClass::Transient => {
                        // Exponential ladder `wait_ms * 2^(n-1)`
                        // capped at `wait_max_ms`, then jittered.
                        let base = reconnect_state.schedule.deterministic(attempt);
                        jitter.sample(base, &mut reconnect_state.schedule)
                    }
                    ReconnectAttemptClass::ServerRestart => {
                        // Flat patient cadence for a pool bounce,
                        // jittered so a fleet spreads its retries.
                        let base = Duration::from_millis(wait_server_restart_ms);
                        jitter.sample(base, &mut reconnect_state.schedule)
                    }
                    ReconnectAttemptClass::RateLimited => {
                        // The floor is an upstream-instructed cooldown
                        // and must be honoured in full — jitter ADDS a
                        // window on top rather than sampling below the
                        // floor.
                        let floor = Duration::from_millis(wait_rate_limited_ms);
                        match jitter {
                            JitterMode::None => floor,
                            _ => {
                                floor
                                    + crate::backoff::uniform_duration(
                                        Duration::ZERO,
                                        RATE_LIMITED_JITTER_WINDOW,
                                    )
                            }
                        }
                    }
                };
                (delay, attempt)
            }
            ReconnectPolicy::Custom(f) => {
                // Permanent reasons never reach the user closure —
                // no return value can turn a credential rejection
                // into a retry loop. This matches the `Auto` arm's
                // short-circuit and is part of the documented
                // `ReconnectPolicy::Custom` contract.
                if ReconnectAttemptLimits::class_for(reason).is_none() {
                    tracing::error!(reason = ?reason, "permanent disconnect -- not reconnecting");
                    publish_exhausted!(0);
                    break 'session;
                }
                // Custom policies bypass the split-budget enforcement
                // (no `Auto`-side budget check), and the user closure
                // receives the consecutive-transient attempt counter.
                // The `attempt` arg therefore reflects "how many
                // consecutive reconnects this session has issued for
                // non-permanent reasons", which is the natural input
                // for a user-supplied backoff curve. Rate-limited
                // (`TooManyRequests`) drops are NOT separately
                // counted on this path because a custom policy
                // already owns the per-reason delay decision and a
                // separate counter would force the user to merge
                // two attempt values to read total session pressure.
                let attempt = reconnect_state.record(ReconnectAttemptClass::Transient);
                let Some(d) = f(reason, attempt) else {
                    tracing::info!(reason = ?reason, "custom policy returned None -- not reconnecting");
                    publish_exhausted!(attempt - 1);
                    break 'session;
                };
                (d, attempt)
            }
        };

        // Emit Reconnecting event before sleeping.
        let delay_ms = u64::try_from(delay.as_millis()).unwrap_or(u64::MAX);
        tracing::info!(
            reason = ?reason,
            attempt = reconnect_attempt,
            delay_ms,
            "auto-reconnecting FPSS"
        );
        FPSS_RECONNECTS.increment(1);
        if producer
            .try_publish(|slot| {
                slot.event = FpssEventInternal::Control(StreamControl::Reconnecting {
                    reason,
                    attempt: reconnect_attempt,
                    delay_ms,
                });
            })
            .is_err()
        {
            dropped.fetch_add(1, Ordering::Relaxed);
            tracing::warn!(
                target: "thetadatadx::fpss::io_loop",
                "ring full while publishing Reconnecting; dropped",
            );
        }

        // Shutdown-responsive cooldown: sleep in short slices so a
        // `shutdown()` raised mid-cooldown wakes the thread within
        // ~100 ms instead of parking it for a full rate-limited
        // 130 s+ delay.
        sleep_until_or_shutdown(delay, &shutdown);

        if shutdown.load(Ordering::Relaxed) {
            break 'session;
        }

        // Discard commands queued against the dead session BEFORE
        // dialling the new one: stale heartbeats would land on a
        // fresh peer as a meaningless inbound burst, and stale
        // subscribe frames would duplicate the paced replay below
        // (the subscription sets are the source of truth for replay).
        // `Shutdown` is the one command that must survive the drain.
        loop {
            match cmd_rx.try_recv() {
                Ok(IoCommand::WriteFrame { .. }) => {}
                Ok(IoCommand::Shutdown) => {
                    shutdown.store(true, Ordering::Release);
                    break 'session;
                }
                Err(std_mpsc::TryRecvError::Empty) => break,
                Err(std_mpsc::TryRecvError::Disconnected) => {
                    shutdown.store(true, Ordering::Release);
                    break 'session;
                }
            }
        }

        // --- Attempt new TLS connection and re-authenticate ---
        // Pin the most recent host that survived the stable window,
        // then re-apply the configured policy to the remaining hosts.
        // Cold connects and unstable sessions stay pure-policy.
        let new_stream = {
            let ordered_hosts = connection::order_hosts(
                &hosts,
                host_selection,
                host_shuffle_seed,
                last_known_good_host,
            );
            let ordered: Vec<(&str, u16)> = ordered_hosts
                .iter()
                .map(|(host, port)| (host.as_str(), *port))
                .collect();
            connection::connect_to_servers(&ordered, connect_timeout, read_timeout, keepalive)
        };

        let (mut new_stream, new_addr) = match new_stream {
            Ok((s, addr)) => {
                tracing::info!(server = %addr, "reconnected to FPSS server");
                (s, addr)
            }
            Err(e) => {
                tracing::warn!(error = %e, "reconnection failed, will retry");
                // Loop around to try again. The per-class counter was
                // already incremented on the reconnection-decision
                // branch above and will be re-incremented on the next
                // failure-with-reason cycle through the loop.
                continue 'session;
            }
        };

        // Re-authenticate on the new stream.
        let cred_payload = match build_credentials_payload(&creds.email, &creds.password) {
            Ok(p) => p,
            Err(e) => {
                // Oversized credentials are a fatal configuration error, not a
                // transient I/O fault: retrying cannot make them fit. Surface
                // it and abandon the reconnect loop rather than spinning.
                tracing::error!(error = %e, "credentials payload invalid; aborting reconnect");
                break 'session;
            }
        };
        let frame = Frame::new(StreamMsgType::Credentials, cred_payload);
        if let Err(e) = write_frame(&mut new_stream, &frame) {
            tracing::warn!(error = %e, "failed to send credentials on reconnect");
            continue 'session;
        }

        let mut reconnect_pending_control: Vec<StreamControl> = Vec::new();
        let login_result = match wait_for_login(&mut new_stream, &mut reconnect_pending_control) {
            Ok(r) => r,
            Err(e) => {
                tracing::warn!(error = %e, "login failed on reconnect");
                continue 'session;
            }
        };

        let new_permissions = match login_result {
            LoginResult::Success(p) => {
                tracing::info!(permissions = %p, "re-authenticated on reconnect");
                p
            }
            LoginResult::Disconnected(reason) => {
                if matches!(
                    reason,
                    RemoveReason::InvalidCredentials
                        | RemoveReason::InvalidLoginValues
                        | RemoveReason::InvalidCredentialsNullUser
                ) {
                    tracing::warn!(
                        "FPSS login failed. If your password contains special characters, \
                         try URL-encoding them."
                    );
                }
                tracing::warn!(reason = ?reason, "server rejected login on reconnect");
                // Permanent rejection -- mirror the initial-login
                // `connect_with_stream` behaviour instead of burning
                // MAX_RECONNECT_ATTEMPTS cycles of Disconnected /
                // Reconnecting noise. `reconnect_delay(reason).is_none()`
                // is the single source of truth for "no amount of
                // retrying will fix this"; see `fpss/session.rs` for the
                // enumerated set (InvalidCredentials, InvalidLoginValues,
                // InvalidLoginSize, AccountAlreadyConnected, FreeAccount,
                // ServerUserDoesNotExist, InvalidCredentialsNullUser).
                if reconnect_delay(reason).is_none() {
                    tracing::error!(
                        reason = ?reason,
                        "permanent login rejection on reconnect -- exiting I/O loop"
                    );
                    if producer
                        .try_publish(|slot| {
                            slot.event =
                                FpssEventInternal::Control(StreamControl::Disconnected { reason });
                        })
                        .is_err()
                    {
                        dropped.fetch_add(1, Ordering::Relaxed);
                        tracing::warn!(
                            target: "thetadatadx::fpss::io_loop",
                            "ring full while publishing permanent-rejection Disconnected; dropped",
                        );
                    }
                    // Same terminal-event contract as the budget /
                    // permanent paths above: recovery has stopped for
                    // a non-user-initiated cause. The inner `reason`
                    // (the login rejection) is the one operators need.
                    if producer
                        .try_publish(|slot| {
                            slot.event =
                                FpssEventInternal::Control(StreamControl::ReconnectsExhausted {
                                    reason,
                                    attempts: reconnect_attempt,
                                });
                        })
                        .is_err()
                    {
                        dropped.fetch_add(1, Ordering::Relaxed);
                        tracing::warn!(
                            target: "thetadatadx::fpss::io_loop",
                            "ring full while publishing ReconnectsExhausted; dropped",
                        );
                    }
                    shutdown.store(true, Ordering::Release);
                    break 'session;
                }
                continue 'session;
            }
        };

        // Record the address that just accepted the login so the next
        // reconnect tries it first, and so `last_connected_addr()` on
        // the owning client reflects the live session.
        *connected_addr
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = new_addr.clone();
        current_host = host_index(&hosts, &new_addr);

        // Set the short I/O read timeout on the new stream so the io
        // loop can drain commands between reads. Matches the
        // initial-connect path in `StreamingClient::connect_with_stream`.
        if let Err(e) = new_stream.sock.set_read_timeout(Some(io_read_slice)) {
            tracing::warn!(error = %e, "failed to set read timeout on reconnect");
            continue 'session;
        }

        // Clear delta state -- fresh connection means fresh deltas.
        delta_state.clear();
        local_contracts.clear();

        // Fresh authenticated session: start the data-flow marker from
        // zero so the stable-window check on the NEXT drop uses the
        // wall-clock of THIS session, not the previous one. Counters
        // stay live (the budget was just decremented to permit this
        // attempt); they reset when the new session delivers data.
        reconnect_state.last_data_at = None;

        authenticated.store(true, Ordering::Release);
        // The login handshake just exchanged frames — feed the
        // staleness clock so `millis_since_last_event()` reflects the
        // live session immediately.
        last_event_at_ns.store(unix_nanos_now(), Ordering::Relaxed);

        // Publish reconnection events. Drain every handshake-time typed
        // control frame (`Connected` / `Ping` / `ReconnectedServer` /
        // `Restart`) in wire order before `LoginSuccess`, so the event
        // order matches the fresh-session bootstrap above. Every
        // publish is non-blocking so a saturated ring never wedges the
        // io_loop's reconnect path.
        for ctrl in reconnect_pending_control.drain(..) {
            if producer
                .try_publish(|slot| {
                    slot.event = FpssEventInternal::Control(ctrl);
                })
                .is_err()
            {
                dropped.fetch_add(1, Ordering::Relaxed);
                tracing::warn!(
                    target: "thetadatadx::fpss::io_loop",
                    "ring full while publishing post-reconnect control frame; dropped",
                );
            }
        }
        if producer
            .try_publish(|slot| {
                slot.event = FpssEventInternal::Control(StreamControl::LoginSuccess {
                    permissions: new_permissions,
                });
            })
            .is_err()
        {
            dropped.fetch_add(1, Ordering::Relaxed);
            tracing::warn!(
                target: "thetadatadx::fpss::io_loop",
                "ring full while publishing post-reconnect LoginSuccess; dropped",
            );
        }
        if producer
            .try_publish(|slot| {
                slot.event = FpssEventInternal::Control(StreamControl::Reconnected);
            })
            .is_err()
        {
            dropped.fetch_add(1, Ordering::Relaxed);
            tracing::warn!(
                target: "thetadatadx::fpss::io_loop",
                "ring full while publishing Reconnected; dropped",
            );
        }

        // Replace the reader with the new stream.
        reader = BufReader::new(new_stream);

        // Re-subscribe all active subscriptions on the new connection.
        // The METADATA handler iterates activeQuotes + activeTrades and
        // re-sends each. Without this, the server accepts the login but
        // receives no subscribe commands → data stops flowing.
        //
        // The replay is PACED: frames are written in bursts of
        // `replay_burst_size`, each burst flushed and followed by a
        // jittered `replay_pace_ms` pause. A large subscription set is
        // thereby spread over wall-clock time instead of being handed
        // to a recovering server as one syscall-sized burst — and a
        // fleet of reconnecting clients additionally de-phases through
        // the ±20% pause jitter.
        //
        // Snapshot + drop lock before writing: holding the mutex during
        // network I/O would stall concurrent subscribe/unsubscribe calls.
        let subs_snapshot = active_subs
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone();
        let full_subs_snapshot = active_full_subs
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone();

        let mut pacer = ReplayPacer::new(replay_burst_size, replay_pace_ms);
        let writer = reader.get_mut();
        for (kind, contract) in &subs_snapshot {
            // Allocate a fresh req_id per re-subscribe so the server's
            // `ReqResponse` events on the reconnected session carry
            // correlatable ids — `-1` is indistinguishable from a
            // manual subscribe and breaks user-side correlation.
            let req_id = super::wire_req_id(next_req_id.fetch_add(1, Ordering::Relaxed));
            let payload = match protocol::build_subscribe_payload(req_id, contract) {
                Ok(p) => p,
                Err(e) => {
                    tracing::warn!(error = %e, contract = %contract, "skipping re-subscribe; contract no longer encodes");
                    continue;
                }
            };
            let code = kind.subscribe_code();
            if let Err(e) = write_raw_frame_no_flush(writer, code, &payload) {
                tracing::warn!(error = %e, contract = %contract, req_id, "failed to re-subscribe on reconnect");
            } else {
                tracing::debug!(kind = ?kind, contract = %contract, req_id, "re-subscribed on auto-reconnect");
            }
            pacer.frame_written(writer, &shutdown);
            if shutdown.load(Ordering::Relaxed) {
                break 'session;
            }
        }
        for (kind, sec_type) in &full_subs_snapshot {
            let req_id = super::wire_req_id(next_req_id.fetch_add(1, Ordering::Relaxed));
            let payload = protocol::build_full_type_subscribe_payload(req_id, *sec_type);
            let code = kind.subscribe_code();
            if let Err(e) = write_raw_frame_no_flush(writer, code, &payload) {
                tracing::warn!(error = %e, sec_type = ?sec_type, req_id, "failed to re-subscribe full-type on reconnect");
            } else {
                tracing::debug!(kind = ?kind, sec_type = ?sec_type, req_id, "re-subscribed full-type on auto-reconnect");
            }
            pacer.frame_written(writer, &shutdown);
            if shutdown.load(Ordering::Relaxed) {
                break 'session;
            }
        }
        if !subs_snapshot.is_empty() || !full_subs_snapshot.is_empty() {
            if let Err(e) = writer.flush() {
                tracing::warn!(error = %e, "failed to flush re-subscribe batch on reconnect");
            }
        }

        // Drain any commands that queued up during reconnection (subscribe, ping, etc.)
        // and send them over the new connection.
        loop {
            match cmd_rx.try_recv() {
                Ok(IoCommand::WriteFrame { code, payload }) => {
                    let writer = reader.get_mut();
                    let result = if code == StreamMsgType::Ping
                        || flush_mode == StreamingFlushMode::Immediate
                    {
                        write_raw_frame(writer, code, &payload)
                    } else {
                        write_raw_frame_no_flush(writer, code, &payload)
                    };
                    if let Err(e) = result {
                        tracing::warn!(error = %e, "failed to write queued frame on reconnect");
                    }
                }
                Ok(IoCommand::Shutdown) => {
                    let stop_payload = protocol::build_stop_payload();
                    let writer = reader.get_mut();
                    // Best-effort STOP on the reconnect-path queued
                    // command drain. Mirror the diagnostic treatment in
                    // the primary shutdown branch above: log the error so
                    // half-closed sockets and kernel buffer exhaustion
                    // are observable in traces.
                    if let Err(e) = write_raw_frame(writer, StreamMsgType::Stop, &stop_payload) {
                        tracing::warn!(
                            error = %e,
                            "failed to send STOP frame on reconnect-path shutdown"
                        );
                    }
                    shutdown.store(true, Ordering::Release);
                    break;
                }
                Err(std_mpsc::TryRecvError::Empty) => break,
                Err(std_mpsc::TryRecvError::Disconnected) => {
                    shutdown.store(true, Ordering::Release);
                    break;
                }
            }
        }

        if shutdown.load(Ordering::Relaxed) {
            break 'session;
        }

        // Continue 'session loop: the inner read/write loop will run on the new stream.
    } // end 'session loop

    // Dropping the producer at scope exit stores the shutdown sequence
    // on the ring. The caller's consumer loop (`StreamingClient::next_event`
    // / `poll_batch` / `for_each` / `Iterator for &StreamingClient`) observes
    // `Polling::Shutdown` once it has drained every published event.
    drop(producer);
    tracing::debug!("fpss-io thread exiting");
}

/// Build the ring producer + poller.
///
/// Constructs the single-producer ring in polling mode, so
/// **no** consumer thread is spawned and **no** intermediate
/// queue is allocated. The returned producer is moved into the I/O
/// thread; the returned `EventPoller` is bundled into the assembled
/// `StreamingClient` and drained by `next_event` / `poll_batch` /
/// `for_each` / the `Iterator for &StreamingClient` impl.
///
/// `cursors` is the shared occupancy cursor pair: the returned
/// producer records every successfully published sequence into it,
/// and the consumer drain records batch completions, so
/// `StreamingClient::ring_occupancy` can sample in-flight depth.
///
/// Dropping the producer at io_loop exit stores the shutdown sequence
/// on the ring; the consumer side then drains every published event
/// and signals shutdown once it reaches that sequence — the EOF-drain guarantee.
///
/// Declared `pub` so the `__test-helpers`-gated `fpss::__test_internals`
/// re-export can hand it to the out-of-crate streaming bench; the
/// enclosing `mod io_loop` is private, so this stays crate-internal in
/// shipped builds and never reaches the public API or `cargo-semver-checks`.
pub fn build_poller_producer(
    ring_size: usize,
    cursors: Arc<RingCursors>,
    wait_strategy: AdaptiveWaitStrategy,
) -> (
    impl RingProducer,
    EventPoller<RingEvent, SingleProducerBarrier>,
) {
    let factory = RingEvent::default;
    // The disruptor builder is generic over `W: WaitStrategy` but erases
    // the concrete strategy type once built — `EventPoller<RingEvent,
    // SingleProducerBarrier>` does not name `W` — so swapping the
    // strategy preset never changes the poller or producer return types
    // and the public ring API stays stable.
    //
    // Polling mode (`new_event_poller`) spawns no processor thread: the
    // consumer IS the caller's own thread driving `next_event` /
    // `for_each`, so the builder's thread-name / core-affinity settings
    // would have no thread to apply to. Consumer-core pinning is applied
    // on the real drain thread instead — see
    // [`super::super::affinity::pin_consumer_thread`] and its call sites
    // in `StreamingClient::for_each_scoped` / `next_event`.
    let (poller, builder) =
        build_single_producer(ring_size, factory, wait_strategy).new_event_poller();
    (SequencedProducer::new(builder.build(), cursors), poller)
}

/// Current wall-clock time as UNIX nanoseconds, clamped into `i64`.
/// Feeds the shared `last_event_at_ns` staleness clock (`0` = never).
fn unix_nanos_now() -> i64 {
    i64::try_from(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos()),
    )
    .unwrap_or(i64::MAX)
}

/// Sleep for `delay`, waking within ~100 ms of `shutdown` being
/// raised.
///
/// The reconnect cooldowns reach 130 s+ on the rate-limited class; an
/// uninterruptible `thread::sleep` there would pin a shutting-down
/// process for the full cooldown, and an operator who concludes the
/// process is hung will SIGKILL it — leaking the TCP socket until the
/// OS-level timeout. Sleeping in bounded slices caps the
/// shutdown-observation latency regardless of how long the cooldown
/// grows.
fn sleep_until_or_shutdown(delay: Duration, shutdown: &AtomicBool) {
    const SLICE: Duration = Duration::from_millis(100);
    let deadline = Instant::now() + delay;
    loop {
        if shutdown.load(Ordering::Relaxed) {
            return;
        }
        let now = Instant::now();
        if now >= deadline {
            return;
        }
        thread::sleep(SLICE.min(deadline - now));
    }
}

/// Jittered pause between subscription-replay bursts: ±20 % around the
/// configured pace so a fleet of reconnecting clients does not flush
/// replay bursts in phase. Returns [`Duration::ZERO`] for a zero pace.
fn replay_pause(pace: Duration) -> Duration {
    if pace.is_zero() {
        return Duration::ZERO;
    }
    let lo = pace.mul_f64(0.8);
    let hi = pace.mul_f64(1.2);
    crate::backoff::uniform_duration(lo, hi)
}

/// Burst accounting for the post-reconnect subscription replay.
///
/// Counts frames written via [`ReplayPacer::frame_written`]; when a
/// burst fills, flushes the writer and pauses for a jittered
/// [`replay_pause`] (shutdown-responsive). The final partial burst is
/// flushed by the caller's tail flush.
struct ReplayPacer {
    burst_size: u32,
    pace: Duration,
    written_in_burst: u32,
}

impl ReplayPacer {
    fn new(burst_size: u32, pace_ms: u64) -> Self {
        Self {
            // A zero burst size would never flush; clamp to 1 (the
            // config validator rejects 0 up front, this is the
            // belt-and-braces guard for direct builder callers).
            burst_size: burst_size.max(1),
            pace: Duration::from_millis(pace_ms),
            written_in_burst: 0,
        }
    }

    /// Account one written frame; on a full burst, flush + pause.
    fn frame_written<W: Write>(&mut self, writer: &mut W, shutdown: &AtomicBool) {
        self.written_in_burst += 1;
        if self.written_in_burst < self.burst_size {
            return;
        }
        self.written_in_burst = 0;
        if let Err(e) = writer.flush() {
            tracing::warn!(error = %e, "failed to flush re-subscribe burst on reconnect");
        }
        if !self.pace.is_zero() {
            sleep_until_or_shutdown(replay_pause(self.pace), shutdown);
        }
    }
}

/// Per-class consecutive-reconnect counters with a stable-window reset
/// driven from the read-side's last-frame timestamp, plus the
/// wall-clock anchor for the reconnect envelope and the jitter
/// schedule state.
struct ReconnectCounters {
    transient: u32,
    rate_limited: u32,
    server_restart: u32,
    /// Wall-clock instant of the last successful frame read; `None`
    /// until the first frame on the current session arrives.
    last_data_at: Option<Instant>,
    /// Instant of the first attempt in the current
    /// consecutive-reconnect sequence; `None` outside a sequence.
    /// Anchors the `max_elapsed` envelope.
    burst_started_at: Option<Instant>,
    /// Exponential-ladder bounds + decorrelated-jitter walk state for
    /// the generic-transient class.
    schedule: BackoffSchedule,
}

impl ReconnectCounters {
    fn new(schedule: BackoffSchedule) -> Self {
        Self {
            transient: 0,
            rate_limited: 0,
            server_restart: 0,
            last_data_at: None,
            burst_started_at: None,
            schedule,
        }
    }

    /// Record a successful frame read. Marks "data did flow on this
    /// session" so the stable-window check on next drop knows whether
    /// to reset the counters.
    fn note_data_received(&mut self) {
        self.last_data_at = Some(Instant::now());
    }

    /// Zero every per-class counter and end the current reconnect
    /// sequence (envelope anchor + jitter walk state).
    fn reset_counters(&mut self) {
        self.transient = 0;
        self.rate_limited = 0;
        self.server_restart = 0;
        self.burst_started_at = None;
        self.schedule.reset();
    }

    /// Decide whether the connection that just disconnected ran long
    /// enough to be considered "stable" — if so, reset all counters
    /// before scheduling the next attempt.
    fn maybe_reset_after_stable(&mut self, limits: &ReconnectAttemptLimits) -> bool {
        if let Some(t) = self.last_data_at {
            if t.elapsed() >= limits.stable_window {
                self.reset_counters();
                return true;
            }
        }
        false
    }

    /// Wall-clock time since the first attempt of the current
    /// consecutive-reconnect sequence. Zero outside a sequence.
    fn burst_elapsed(&self) -> Duration {
        self.burst_started_at
            .map_or(Duration::ZERO, |t| t.elapsed())
    }

    /// Increment the counter for `class` and return the new attempt
    /// number (1-based after increment). The first record of a
    /// sequence anchors the wall-clock envelope.
    fn record(&mut self, class: ReconnectAttemptClass) -> u32 {
        if self.burst_started_at.is_none() {
            self.burst_started_at = Some(Instant::now());
        }
        match class {
            ReconnectAttemptClass::Transient => {
                self.transient = self.transient.saturating_add(1);
                self.transient
            }
            ReconnectAttemptClass::RateLimited => {
                self.rate_limited = self.rate_limited.saturating_add(1);
                self.rate_limited
            }
            ReconnectAttemptClass::ServerRestart => {
                self.server_restart = self.server_restart.saturating_add(1);
                self.server_restart
            }
        }
    }
}

/// Check if an error is a transient read condition that should drain
/// commands and retry rather than tear the connection down.
///
/// Delegates to [`super::framing::is_transient_read`] for the kind
/// classification so all three FPSS read sites (this loop, mid-header,
/// mid-payload) share one definition. Recognises `WouldBlock`,
/// `TimedOut`, and Windows `ERROR_IO_PENDING` (raw OS 997, surfaced as
/// `ErrorKind::Uncategorized` by `std`).
fn is_read_timeout(e: &Error) -> bool {
    match e {
        Error::Io(io_err) => super::framing::is_transient_read(io_err),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_schedule() -> BackoffSchedule {
        BackoffSchedule::new(Duration::from_millis(250), Duration::from_secs(30))
    }

    #[test]
    fn split_budget_defaults_cover_multi_hour_throttle() {
        let limits = ReconnectAttemptLimits::default();
        assert_eq!(limits.max_attempts, 30);
        assert_eq!(limits.max_rate_limited_attempts, 100);
        assert_eq!(limits.max_server_restart_attempts, 60);
        // 100 attempts × 130 s/attempt = 13_000 s = ~3.6 h of patient
        // retry on sustained `TooManyRequests` — the goal of riding
        // through a multi-hour throttle without operator intervention.
        let rate_limited_horizon_ms = u128::from(limits.max_rate_limited_attempts)
            * u128::from(crate::fpss::protocol::TOO_MANY_REQUESTS_DELAY_MS);
        assert!(
            rate_limited_horizon_ms >= 3 * 60 * 60 * 1000,
            "default rate-limited budget must cover at least 3 h \
             of sustained throttling; got {rate_limited_horizon_ms} ms"
        );
    }

    /// The generic-transient defaults must survive a multi-minute
    /// outage: the attempt budget (30) outlasts the 5-minute envelope
    /// at the un-jittered ladder, so `max_elapsed` is the effective
    /// operator-facing bound — exactly the contract the docs state.
    #[test]
    fn transient_defaults_survive_a_multi_minute_outage() {
        let limits = ReconnectAttemptLimits::default();
        let schedule = test_schedule();
        let total: Duration = (1..=limits.max_attempts)
            .map(|a| schedule.deterministic(a))
            .sum();
        assert!(
            total >= limits.max_elapsed,
            "un-jittered ladder across the attempt budget ({total:?}) must \
             outlast the wall-clock envelope ({:?})",
            limits.max_elapsed
        );
        // First attempts are fast (sub-second) so a brief blip
        // recovers quickly...
        assert_eq!(schedule.deterministic(1), Duration::from_millis(250));
        assert_eq!(schedule.deterministic(2), Duration::from_millis(500));
        // ...and the tail rides the 30 s cap.
        assert_eq!(schedule.deterministic(8), Duration::from_secs(30));
        assert_eq!(schedule.deterministic(30), Duration::from_secs(30));
    }

    /// 10 consecutive `TooManyRequests` disconnects must NOT exhaust
    /// the rate-limited budget. Each attempt's floor equals
    /// `reconnect_delay(TooManyRequests)` = `TOO_MANY_REQUESTS_DELAY_MS`
    /// (130 s).
    #[test]
    fn ten_too_many_requests_stays_under_rate_limited_budget() {
        let limits = ReconnectAttemptLimits::default();
        let mut counters = ReconnectCounters::new(test_schedule());
        let mut last_attempt = 0;
        for _ in 0..10 {
            let class = ReconnectAttemptLimits::class_for(RemoveReason::TooManyRequests)
                .expect("TooManyRequests is not permanent");
            assert_eq!(class, ReconnectAttemptClass::RateLimited);
            last_attempt = counters.record(class);
        }
        assert_eq!(last_attempt, 10);
        assert!(
            last_attempt <= limits.budget_for(ReconnectAttemptClass::RateLimited),
            "10 consecutive TooManyRequests must stay inside the rate-limited budget"
        );
        // Per-attempt floor surfaces the wall-clock cost: 10 * 130 s =
        // 1300 s = ~21 min of patient retry, well within the ~3.6 h
        // envelope the default permits.
        let ms = crate::fpss::reconnect_delay(RemoveReason::TooManyRequests)
            .expect("TooManyRequests yields a finite reconnect delay");
        let total_ms = u128::from(ms) * u128::from(last_attempt);
        // 130 000 ms (TooManyRequests cooldown) * 10 attempts = 1 300 000.
        // Use `assert_eq!` so a future drift in either factor surfaces
        // immediately rather than passing on any value <= the bound.
        assert_eq!(total_ms, 1_300_000);
    }

    /// Stable-window reset: a session that ran cleanly for at least
    /// `stable_window` before the drop earns a fresh budget. A
    /// session shorter than the window keeps the previous count.
    #[test]
    fn stable_window_resets_counters() {
        let limits = ReconnectAttemptLimits {
            stable_window: Duration::from_millis(5),
            ..ReconnectAttemptLimits::default()
        };
        let mut counters = ReconnectCounters::new(test_schedule());
        counters.record(ReconnectAttemptClass::Transient);
        counters.record(ReconnectAttemptClass::Transient);
        counters.record(ReconnectAttemptClass::RateLimited);
        counters.record(ReconnectAttemptClass::ServerRestart);
        // No data received yet → no reset.
        assert!(!counters.maybe_reset_after_stable(&limits));
        assert_eq!(counters.transient, 2);
        assert_eq!(counters.rate_limited, 1);
        assert_eq!(counters.server_restart, 1);

        counters.note_data_received();
        // Data received but not long enough → still no reset.
        assert!(!counters.maybe_reset_after_stable(&limits));
        assert_eq!(counters.transient, 2);
        assert_eq!(counters.rate_limited, 1);

        std::thread::sleep(Duration::from_millis(8));
        assert!(counters.maybe_reset_after_stable(&limits));
        assert_eq!(counters.transient, 0, "stable-window elapsed → reset");
        assert_eq!(counters.rate_limited, 0);
        assert_eq!(counters.server_restart, 0);
        assert!(
            counters.burst_started_at.is_none(),
            "stable-window reset must also end the envelope sequence"
        );
    }

    /// The wall-clock envelope anchors at the FIRST attempt of a
    /// consecutive-reconnect sequence, accumulates while the sequence
    /// runs, and resets with the counters.
    #[test]
    fn burst_elapsed_anchors_and_resets() {
        let mut counters = ReconnectCounters::new(test_schedule());
        assert_eq!(counters.burst_elapsed(), Duration::ZERO);
        counters.record(ReconnectAttemptClass::Transient);
        std::thread::sleep(Duration::from_millis(10));
        counters.record(ReconnectAttemptClass::Transient);
        assert!(
            counters.burst_elapsed() >= Duration::from_millis(10),
            "envelope must measure from the first attempt of the sequence"
        );
        counters.reset_counters();
        assert_eq!(counters.burst_elapsed(), Duration::ZERO);
        // A fresh sequence re-anchors.
        counters.record(ReconnectAttemptClass::ServerRestart);
        assert!(counters.burst_elapsed() < Duration::from_millis(10));
    }

    /// The reconnect cooldown must wake promptly on shutdown: signal
    /// the flag ~50 ms into a 5 s cooldown and require the sleeper to
    /// return well under the full delay (bounded by the 100 ms slice
    /// plus scheduling slack).
    #[test]
    fn reconnect_sleep_wakes_promptly_on_shutdown() {
        let shutdown = Arc::new(AtomicBool::new(false));
        let signaller = Arc::clone(&shutdown);
        let signal_thread = thread::spawn(move || {
            thread::sleep(Duration::from_millis(50));
            signaller.store(true, Ordering::Release);
        });
        let start = Instant::now();
        sleep_until_or_shutdown(Duration::from_secs(5), &shutdown);
        let elapsed = start.elapsed();
        signal_thread.join().expect("signal thread joins");
        assert!(
            elapsed < Duration::from_millis(500),
            "5 s cooldown must be interrupted within one slice of the \
             shutdown signal; slept {elapsed:?}"
        );
        assert!(
            elapsed >= Duration::from_millis(45),
            "sleeper must not return before the signal; slept {elapsed:?}"
        );
    }

    /// A shutdown raised BEFORE the sleep starts must return
    /// immediately, and a zero delay must not sleep at all.
    #[test]
    fn reconnect_sleep_degenerate_cases() {
        let raised = AtomicBool::new(true);
        let start = Instant::now();
        sleep_until_or_shutdown(Duration::from_secs(5), &raised);
        assert!(start.elapsed() < Duration::from_millis(50));

        let clear = AtomicBool::new(false);
        let start = Instant::now();
        sleep_until_or_shutdown(Duration::ZERO, &clear);
        assert!(start.elapsed() < Duration::from_millis(50));
    }

    /// Replay pacing: a writer fed N frames through the pacer must see
    /// one flush per full burst, and the burst pauses must keep the
    /// total replay duration at roughly `(N / burst - 1) * pace`.
    #[test]
    fn replay_pacer_flushes_per_burst_and_paces() {
        struct CountingWriter {
            flushes: usize,
        }
        impl Write for CountingWriter {
            fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
                Ok(buf.len())
            }
            fn flush(&mut self) -> std::io::Result<()> {
                self.flushes += 1;
                Ok(())
            }
        }

        let shutdown = AtomicBool::new(false);
        let mut writer = CountingWriter { flushes: 0 };
        // 125 frames at burst 50 → 2 full bursts (flush + pause each),
        // 25-frame tail left for the caller's final flush.
        let mut pacer = ReplayPacer::new(50, 20);
        let start = Instant::now();
        for _ in 0..125 {
            pacer.frame_written(&mut writer, &shutdown);
        }
        let elapsed = start.elapsed();
        assert_eq!(writer.flushes, 2, "one flush per completed burst");
        // Two pauses jittered across [16 ms, 24 ms] each.
        assert!(
            elapsed >= Duration::from_millis(30),
            "two paced bursts must take at least 2 × 0.8 × pace; got {elapsed:?}"
        );
        assert!(
            elapsed < Duration::from_millis(200),
            "pacing must not balloon past the jitter ceiling; got {elapsed:?}"
        );
    }

    /// Pacer accounting must clamp a zero burst size to 1 instead of
    /// never flushing, and a zero pace must flush without sleeping.
    #[test]
    fn replay_pacer_degenerate_knobs() {
        struct CountingWriter {
            flushes: usize,
        }
        impl Write for CountingWriter {
            fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
                Ok(buf.len())
            }
            fn flush(&mut self) -> std::io::Result<()> {
                self.flushes += 1;
                Ok(())
            }
        }
        let shutdown = AtomicBool::new(false);
        let mut writer = CountingWriter { flushes: 0 };
        let mut pacer = ReplayPacer::new(0, 0);
        let start = Instant::now();
        for _ in 0..8 {
            pacer.frame_written(&mut writer, &shutdown);
        }
        assert_eq!(writer.flushes, 8, "burst size 0 must clamp to 1");
        assert!(
            start.elapsed() < Duration::from_millis(100),
            "zero pace must not sleep"
        );
    }

    /// The inter-burst pause is jittered ±20 % around the configured
    /// pace and degrades to zero for a zero pace.
    #[test]
    fn replay_pause_jitter_bounds() {
        let pace = Duration::from_millis(100);
        for _ in 0..128 {
            let pause = replay_pause(pace);
            assert!(pause >= pace.mul_f64(0.8) && pause <= pace.mul_f64(1.2));
        }
        assert_eq!(replay_pause(Duration::ZERO), Duration::ZERO);
    }

    /// The staleness clock helper must produce a strictly-positive,
    /// monotone-enough UNIX timestamp (`0` is the "never" sentinel).
    #[test]
    fn unix_nanos_now_is_positive() {
        let a = unix_nanos_now();
        let b = unix_nanos_now();
        assert!(a > 0);
        assert!(b >= a);
    }

    /// Permanent disconnect reasons during the reconnect handshake
    /// must short-circuit the reconnect loop rather than burn through
    /// the per-class budget. `reconnect_delay(reason).is_none()` is
    /// the single source of truth for "no amount of retrying will fix
    /// this", so the test asserts the predicate behaviour for every
    /// enumerated permanent reason. A regression that omits any of
    /// these from the short-circuit would burn ~budget cycles of
    /// Disconnected/Reconnecting noise before giving up.
    #[test]
    fn reconnect_login_rejection_permanent_reasons_short_circuit() {
        // All 7 permanent reasons from fpss/session.rs::reconnect_delay
        // must return None -- the io_loop checks this predicate before
        // continue'session.
        let permanent = [
            RemoveReason::InvalidCredentials,
            RemoveReason::InvalidLoginValues,
            RemoveReason::InvalidLoginSize,
            RemoveReason::AccountAlreadyConnected,
            RemoveReason::FreeAccount,
            RemoveReason::ServerUserDoesNotExist,
            RemoveReason::InvalidCredentialsNullUser,
        ];
        for reason in permanent {
            assert_eq!(
                super::super::reconnect_delay(reason),
                None,
                "reason {reason:?} must be classified as permanent so the reconnect \
                 path short-circuits instead of looping"
            );
        }
    }

    /// The io_loop must never call blocking `producer.publish(...)` —
    /// every publish goes through `try_publish` so a saturated ring
    /// never wedges the TLS reader. A textual grep against the
    /// source pins the contract. Walks only the production code
    /// region (everything before the `#[cfg(test)] mod tests`
    /// marker) so the test-body literals don't trip the scan.
    #[test]
    fn io_loop_uses_only_try_publish() {
        let src = include_str!("mod.rs");
        // Locate the test-module marker (the `#[cfg(test)] mod tests {`
        // block at the bottom of the file) — there's an earlier
        // `#[cfg(test)] use ...` import we must skip over.
        let cfg_test_pos = src
            .find("#[cfg(test)]\nmod tests")
            .expect("test module marker present");
        let prod = &src[..cfg_test_pos];
        let code_only: String = prod
            .lines()
            .filter(|line| {
                let t = line.trim_start();
                !t.starts_with("//") && !t.starts_with("///") && !t.starts_with("/*")
            })
            .collect::<Vec<_>>()
            .join("\n");
        let stripped = code_only.replace(".try_publish(", "");
        assert!(
            !stripped.contains(".publish("),
            "io_loop must use try_publish only — found blocking .publish( call site"
        );
        assert!(
            code_only.contains(".try_publish("),
            "io_loop must use try_publish at least once"
        );
    }

    /// Re-subscribe on reconnect must allocate fresh `req_id` values
    /// from the shared counter instead of the `-1` sentinel — server-
    /// side `ReqResponse` events with `req_id = -1` collide with
    /// manual-subscribe responses and break user correlation.
    #[test]
    fn next_req_id_allocates_fresh_ids_for_resubscribe() {
        let counter = Arc::new(AtomicI64::new(7));
        // Mimic the re-subscribe loop's allocation pattern: one
        // fetch_add + `wire_req_id` clamp per re-subscribed contract.
        let id_a = super::super::wire_req_id(counter.fetch_add(1, Ordering::Relaxed));
        let id_b = super::super::wire_req_id(counter.fetch_add(1, Ordering::Relaxed));
        let id_c = super::super::wire_req_id(counter.fetch_add(1, Ordering::Relaxed));
        assert_eq!(id_a, 7);
        assert_eq!(id_b, 8);
        assert_eq!(id_c, 9);
        assert_ne!(id_a, -1, "re-subscribe must never use the -1 sentinel");
        // Subsequent caller-issued subscribes off the same counter
        // see the next slot — proves the io_loop and the client share
        // one allocator without colliding.
        assert_eq!(
            super::super::wire_req_id(counter.fetch_add(1, Ordering::Relaxed)),
            10
        );
    }

    /// The `AtomicI64` counter is widened so a long-running session
    /// (5k subs/sec ≈ 5 days for `2^31`) cannot wrap into the `-1`
    /// sentinel. `wire_req_id` masks off the sign bit and casts to
    /// `i32`, so even past `i32::MAX` we stay strictly non-negative
    /// and never collide with `-1`.
    #[test]
    fn wire_req_id_clamps_positive_past_i32_max() {
        use super::super::wire_req_id;
        // Below i32::MAX: pass through unchanged.
        assert_eq!(wire_req_id(1), 1);
        assert_eq!(wire_req_id(i64::from(i32::MAX)), i32::MAX);
        // At i32::MAX + 1: the sign-bit mask wraps to 0 (NOT -1).
        assert_eq!(wire_req_id(i64::from(i32::MAX) + 1), 0);
        // Way past i32::MAX: stays non-negative (mask clears the
        // sign bit). `assert!(wire >= 0)` already implies
        // `wire != -1`, so the second assert was redundant -- pin
        // the exact derived value instead.
        for n in 0..256_i64 {
            let v = i64::from(i32::MAX) + 1 + n * 1_000_000;
            let wire = wire_req_id(v);
            let expected = (v & i64::from(i32::MAX)) as i32;
            assert_eq!(wire, expected, "wire id must match low-31-bit mask of {v}");
        }
        // Counter value 2^32: low 31 bits clear, masks to 0.
        assert_eq!(wire_req_id(1_i64 << 32), 0);
        // Counter value 2^32 + 7: clamps to 7.
        assert_eq!(wire_req_id((1_i64 << 32) + 7), 7);
    }

    /// Transient disconnect reasons must NOT short-circuit -- they
    /// should produce a retry delay so the
    /// reconnect loop proceeds. Paired with the permanent-reasons
    /// test above, this pins the exact set that triggers the
    /// shutdown-store-break path versus the continue'session path.
    #[test]
    fn reconnect_login_rejection_transient_reasons_do_not_short_circuit() {
        let transient = [
            RemoveReason::TimedOut,
            RemoveReason::ServerRestarting,
            RemoveReason::TooManyRequests,
            RemoveReason::ClientForcedDisconnect,
            RemoveReason::Unspecified,
        ];
        for reason in transient {
            assert!(
                super::super::reconnect_delay(reason).is_some(),
                "reason {reason:?} must be classified as transient so the reconnect \
                 path keeps looping instead of tearing down the I/O thread"
            );
        }
    }
}
