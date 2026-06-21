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

use crate::tdbe::types::enums::{RemoveReason, StreamMsgType, StreamResponseType};

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
    self, is_drain_yield, read_frame_into_with_stall_timeout, write_raw_frame,
    write_raw_frame_no_flush, FrameReadState,
};
use super::protocol::{self, build_login_payload, Contract};
use super::reconnect_delay;
use super::ring::{
    self, AdaptiveWaitStrategy, RingCursors, RingEvent, RingProducer, SequencedProducer,
};

type ActiveSubs = Arc<Mutex<Vec<(super::protocol::SubscriptionKind, Contract)>>>;
/// Maps an in-flight subscribe's `req_id` to the tracked entry it created,
/// so a rejecting `REQ_RESPONSE` can untrack exactly that subscription.
///
/// Each value carries the [`Instant`] the correlation was recorded so the
/// registry stays bounded: a `REQ_RESPONSE` the server suppresses, or one that
/// echoes the uncorrelated `-1` sentinel a matching `remove` can never key on,
/// would otherwise leave its entry resident for the life of a session that
/// never reconnects. Stale entries are swept on insert (see
/// [`PENDING_SUB_TTL`] and [`PENDING_SUB_CAP`]).
type PendingSubs = Arc<Mutex<HashMap<i32, PendingSubEntry>>>;

/// A pending subscribe correlation plus the instant it was recorded.
#[derive(Debug, Clone)]
pub(in crate::fpss) struct PendingSubEntry {
    /// The tracked subscription this `req_id` answers.
    pub sub: super::protocol::PendingSub,
    /// When the correlation was recorded, for TTL-based eviction.
    pub recorded_at: std::time::Instant,
}

/// How long a pending subscribe correlation may live before it is treated as
/// abandoned and swept. The server answers a subscribe well inside this
/// window; an entry older than this never received its `REQ_RESPONSE` (the
/// server suppressed it, or answered with the uncorrelated `-1` sentinel) and
/// can never be matched, so retaining it only grows the map.
const PENDING_SUB_TTL: std::time::Duration = std::time::Duration::from_secs(300);

/// Hard ceiling on resident pending correlations. A burst of subscribes whose
/// responses are all suppressed could otherwise outrun the TTL sweep within a
/// single window; past this many entries the oldest are dropped so the map can
/// never grow without bound.
const PENDING_SUB_CAP: usize = 4096;

/// Drop pending correlations that can no longer be matched.
///
/// Sweeps entries older than [`PENDING_SUB_TTL`], then, if the map still
/// exceeds [`PENDING_SUB_CAP`], drops the oldest entries down to the cap. The
/// caller holds the `pending_subs` lock; the sweep touches only in-memory map
/// state and performs no I/O, so the lock is never held across a syscall.
pub(in crate::fpss) fn evict_stale_pending(map: &mut HashMap<i32, PendingSubEntry>) {
    let now = std::time::Instant::now();
    let before = map.len();
    map.retain(|_, entry| now.duration_since(entry.recorded_at) < PENDING_SUB_TTL);

    if map.len() > PENDING_SUB_CAP {
        let mut by_age: Vec<(i32, std::time::Instant)> =
            map.iter().map(|(id, e)| (*id, e.recorded_at)).collect();
        by_age.sort_unstable_by_key(|(_, recorded_at)| *recorded_at);
        let overflow = map.len() - PENDING_SUB_CAP;
        for (id, _) in by_age.into_iter().take(overflow) {
            map.remove(&id);
        }
    }

    let evicted = before.saturating_sub(map.len());
    if evicted > 0 {
        tracing::warn!(
            evicted,
            resident = map.len(),
            "evicted unanswered subscribe correlations; a server response was never matched"
        );
    }
}

/// Drop the in-flight subscribe correlation(s) for one tracked identity.
///
/// A pending correlation is only an authority to untrack while the tracked
/// entry it created is still live. Once that entry leaves the tracked set — an
/// unsubscribe removes it — the correlation points at nothing, so a later
/// rejection of its (now superseded) `req_id` must not act on the set: the
/// `(kind, contract)` slot may since have been re-subscribed into a new live
/// entry that a value match would wrongly drop. Removing the correlation at the
/// unsubscribe boundary keeps the invariant that at most one resident
/// correlation per identity exists and it always names the current live entry,
/// so an `apply_req_response` rejection can untrack purely by `req_id` lookup.
///
/// The map is keyed by `req_id`, so identity removal is a single retain pass.
/// The caller holds the `pending_subs` lock; this touches only in-memory map
/// state and performs no I/O.
pub(in crate::fpss) fn evict_pending_for_identity(
    map: &mut HashMap<i32, PendingSubEntry>,
    identity: &super::protocol::PendingSub,
) {
    map.retain(|_, entry| &entry.sub != identity);
}
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
    /// In-flight subscribe registry keyed by `req_id`. A subscribe records
    /// its tracked identity here when the frame is sent; the reader removes
    /// the entry when the server's `REQ_RESPONSE` lands, and on a rejection
    /// also drops the matching tracked subscription so it is not re-replayed.
    pub pending_subs: PendingSubs,
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

/// Reconcile the tracked-subscription set against a server `REQ_RESPONSE`.
///
/// The `req_id` is the only correlation key the wire carries back, so the
/// pending registry — populated when the subscribe frame was sent — is the
/// authority on which tracked entry this response answers. A `Subscribed`
/// outcome leaves the subscription tracked (it is live and must be replayed
/// on reconnect); a rejection (`Error` / `MaxStreamsReached` / `InvalidPerms`)
/// removes the matching entry from `active_subs` / `active_full_subs` so the
/// reconnect replay does not re-attempt it forever and
/// `active_subscriptions()` does not over-report it. Either way the pending
/// entry is consumed.
///
/// A response whose `req_id` is unknown (the uncorrelated `-1` sentinel, or an
/// id from a span longer than the 31-bit counter cycle) leaves the tracked set
/// untouched: with no correlation, untracking would risk dropping a healthy
/// subscription, so the conservative choice is to keep it.
///
/// The `req_id` lookup is a safe untrack because the pending registry holds the
/// invariant that at most one correlation per `(kind, contract)` (or `(kind,
/// sec_type)`) identity is resident, and it always names the current live
/// entry. A duplicate subscribe shares the live entry and registers no second
/// correlation; an unsubscribe removes the entry and evicts its correlation
/// (see [`evict_pending_for_identity`]). So a subscribe that is superseded by an
/// unsubscribe + re-subscribe of the same identity has no resident correlation
/// for its old `req_id`, and a late rejection of that id is a no-op rather than
/// a value match that would drop the re-subscribed live entry.
fn apply_req_response(
    req_id: i32,
    result: StreamResponseType,
    pending_subs: &PendingSubs,
    active_subs: &ActiveSubs,
    active_full_subs: &ActiveFullSubs,
) {
    let pending = pending_subs
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .remove(&req_id);

    let Some(pending) = pending else {
        return;
    };

    if matches!(result, StreamResponseType::Subscribed) {
        return;
    }

    match pending.sub {
        super::protocol::PendingSub::Contract(kind, contract) => {
            active_subs
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .retain(|(k, c)| !(*k == kind && *c == contract));
        }
        super::protocol::PendingSub::Full(kind, sec_type) => {
            active_full_subs
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .retain(|(k, s)| !(*k == kind && *s == sec_type));
        }
    }
    tracing::debug!(
        req_id,
        result = ?result,
        "untracked rejected subscription; it will not be replayed on reconnect"
    );
}

/// Drive [`apply_req_response`] from the client-module tests, which need to
/// reconcile a tracked set against a synthetic server response without a live
/// session. Argument order mirrors the client's `(state, response)` framing.
#[cfg(test)]
pub(in crate::fpss) fn apply_req_response_for_test(
    pending_subs: &PendingSubs,
    active_subs: &ActiveSubs,
    active_full_subs: &ActiveFullSubs,
    req_id: i32,
    result: StreamResponseType,
) {
    apply_req_response(req_id, result, pending_subs, active_subs, active_full_subs);
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
        pending_subs,
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

    // Originating disconnect reason carried across consecutive FAILED
    // redials. A rate-limited (`TooManyRequests`) or `ServerRestarting`
    // drop sets a long cooldown floor and a large attempt budget; if the
    // first redial after that drop fails before the reader is replaced
    // with a live stream, control returns to `'session` top with only the
    // dead, pre-drop `reader` to read from. That read can only time out
    // and re-derive a generic `TimedOut` (Transient) reason, which would
    // silently downgrade the class to the fast ladder and the smaller
    // budget. Holding the originating reason here preserves the class
    // across the redial-failure streak. It is `take`n at the top of each
    // session: a live read on a successfully reconnected session always
    // re-derives a fresh reason, so a genuinely new disconnect reason
    // still overrides once the connection is re-established.
    let mut pending_reason: Option<RemoveReason> = None;

    'session: loop {
        // Session-local liveness clock: starts at session entry so a
        // session that never delivers a frame still times out exactly
        // `read_timeout` after it began, and feeds the shared
        // `last_event_at_ns` operator-facing staleness clock.
        let mut last_frame_at = Instant::now();

        // A carried reason from a failed redial short-circuits the inner
        // read: the only `reader` available here is the dead pre-drop
        // stream, so reading it would waste a full `read_timeout` and emit
        // a spurious `TimedOut` Disconnected/Reconnecting pair before
        // re-deriving the wrong class. Honour the shutdown flag first (the
        // inner loop's own first act) so a carried reason can never wedge
        // a shutting-down thread, then drive the reconnect decision
        // straight off the preserved reason instead of the stale read. A
        // session that successfully reconnected has no carried reason, so
        // a live read on it always re-derives a fresh class below: the
        // carry only holds the class WHILE we are still failing to
        // re-establish the connection, never after.
        if pending_reason.is_some() && shutdown.load(Ordering::Relaxed) {
            break 'session;
        }

        // --- Inner read/write loop for one connection session ---
        // When the inner loop breaks, `disconnect_reason` holds the reason.
        // A carried reason from a failed redial bypasses the read so the
        // stale dead stream is never re-read and the class is preserved.
        let disconnect_reason: RemoveReason = if let Some(carried) = pending_reason.take() {
            // The liveness clock belongs to the read path; the carried
            // branch never consults it but the binding must remain for the
            // read arm below.
            let _ = &last_frame_at;
            carried
        } else {
            'inner: loop {
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

                        // Correlate a subscription response to the subscribe it
                        // answers (by `req_id`) before publishing it. A rejecting
                        // response untracks the offending subscription so it is
                        // neither re-replayed on the next reconnect nor over-
                        // reported by `active_subscriptions()`; an accepting one
                        // simply clears the pending entry and keeps it tracked.
                        if let Some(FpssEventInternal::Control(StreamControl::ReqResponse {
                            req_id,
                            result,
                        })) = &primary
                        {
                            apply_req_response(
                                *req_id,
                                *result,
                                &pending_subs,
                                &active_subs,
                                &active_full_subs,
                            );
                        }

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
                                slot.event =
                                    FpssEventInternal::Control(StreamControl::Disconnected {
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
                                    watchdog_ms = u64::try_from(data_watchdog.as_millis())
                                        .unwrap_or(u64::MAX),
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
                                slot.event =
                                    FpssEventInternal::Control(StreamControl::Disconnected {
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
                                // A steady-state write failure means the socket's
                                // write side is broken. Reading on never recovers
                                // the queued subscribe/command that just failed, so
                                // escalate to reconnect immediately rather than
                                // deferring to the next read timeout. Mirror the
                                // disconnect-and-break shape the read-error and
                                // EOF branches use.
                                tracing::warn!(error = %e, "frame write failed; treating socket as broken and reconnecting");
                                if producer
                                    .try_publish(|slot| {
                                        slot.event = FpssEventInternal::Control(
                                            StreamControl::Disconnected {
                                                reason: RemoveReason::Unspecified,
                                            },
                                        );
                                    })
                                    .is_err()
                                {
                                    dropped.fetch_add(1, Ordering::Relaxed);
                                    tracing::warn!(
                                        target: "thetadatadx::fpss::io_loop",
                                        "ring full while publishing Disconnected (write error); dropped",
                                    );
                                }
                                authenticated.store(false, Ordering::Release);
                                break 'inner RemoveReason::Unspecified;
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
                            if let Err(e) =
                                write_raw_frame(writer, StreamMsgType::Stop, &stop_payload)
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
            }
        }; // end inner read loop (yields RemoveReason)

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
            // The write timeout shares the read timeout's budget: both
            // bound a single unacknowledged transport operation during the
            // reconnect window, so the re-auth write cannot wedge the I/O
            // thread against a peer that has stopped draining its socket.
            connection::connect_to_servers(
                &ordered,
                connect_timeout,
                read_timeout,
                read_timeout,
                keepalive,
            )
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
                // failure-with-reason cycle through the loop. Carry the
                // originating reason so the next cycle re-derives the same
                // class (floor + budget) rather than reading the dead
                // stream and downgrading to a generic TimedOut.
                pending_reason = Some(reason);
                continue 'session;
            }
        };

        // Re-authenticate on the new stream.
        let cred_payload = match build_login_payload(&creds) {
            Ok(p) => p,
            Err(e) => {
                // Oversized credentials are a fatal configuration error, not a
                // transient I/O fault: retrying cannot make them fit. Surface
                // it and abandon the reconnect loop rather than spinning.
                tracing::error!(error = %e, "credentials payload invalid; aborting reconnect");
                break 'session;
            }
        };
        // Write the credentials from the `Zeroizing` buffer directly rather than
        // moving the cleartext into a `Frame`, so the secret bytes are wiped on
        // drop instead of lingering in a frame-owned `Vec`.
        if let Err(e) = write_raw_frame(&mut new_stream, StreamMsgType::Credentials, &cred_payload)
        {
            tracing::warn!(error = %e, "failed to send credentials on reconnect");
            // Reader is still the dead pre-drop stream here; carry the
            // class so the next cycle keeps its floor + budget.
            pending_reason = Some(reason);
            continue 'session;
        }

        let mut reconnect_pending_control: Vec<StreamControl> = Vec::new();
        let login_result = match wait_for_login(
            &mut new_stream,
            &mut reconnect_pending_control,
            read_timeout,
        ) {
            Ok(r) => r,
            Err(e) => {
                tracing::warn!(error = %e, "login failed on reconnect");
                // Reader is still the dead pre-drop stream here; carry the
                // class so the next cycle keeps its floor + budget.
                pending_reason = Some(reason);
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
                // Transient login rejection. The server just handed us a
                // fresh, authoritative reason (e.g. ServerRestarting /
                // TooManyRequests); carry THAT class forward rather than
                // the originating drop's, so the next cycle's floor +
                // budget reflect the most recent server signal instead of
                // a stale read's generic TimedOut.
                pending_reason = Some(reason);
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
            // The new stream never became the live `reader` (that swap is
            // below), so the next cycle would re-read the dead pre-drop
            // stream. Carry the class to keep its floor + budget.
            pending_reason = Some(reason);
            continue 'session;
        }

        // Clear delta state -- fresh connection means fresh deltas.
        delta_state.clear();
        local_contracts.clear();

        // Reset the frame reader to a clean header boundary. A drop that
        // interrupted a partially transmitted frame leaves `frame_state`
        // with `payload_phase == true` and a partial `payload_read`. The
        // reborn reader below reads a brand-new session whose bytes start
        // at a frame header, so a stale mid-frame position would consume
        // the new session's leading bytes as the tail of a phantom frame
        // and desync every frame after it. The invariant: a new
        // connection always begins at a frame boundary.
        frame_state = FrameReadState::new();

        // Fresh authenticated session: start the data-flow marker from
        // zero so the stable-window check on the NEXT drop uses the
        // wall-clock of THIS session, not the previous one. Counters
        // stay live (the budget was just decremented to permit this
        // attempt); they reset when the new session delivers data.
        reconnect_state.last_data_at = None;

        // The session is NOT marked live yet. `authenticated` stays
        // `false` (the inner read loop cleared it on the drop that
        // started this reconnect) until the replay below — re-subscribe
        // plus the queued-command drain — is proven on the fresh socket.
        // A reconnect dial can hand back a socket that accepts the login
        // but breaks on the very next write; flipping `authenticated`
        // here would let `decode_frame` and the command path treat that
        // broken socket as live and accept commands until a later read
        // timeout, instead of re-entering reconnect immediately. The
        // reconnect-success events (`LoginSuccess` / `Reconnected`) are
        // published from the same post-replay point so they never
        // announce a session that the replay then proved dead.

        // Replace the reader with the new stream so the replay writes
        // below target the fresh socket.
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

        // A new session invalidates every previously allocated `req_id`, so
        // any pending correlations from the prior session can never be
        // answered — drop them before the replay re-registers fresh ones.
        pending_subs
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clear();

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
            // A replay write failure means the freshly reconnected socket is
            // already broken; continuing would leave this contract silently
            // unsubscribed until the next disconnect. Re-enter the reconnect
            // loop instead so recovery is driven the same way every other
            // mid-replay I/O failure is, rather than swallowed in a warning.
            if let Err(e) = write_raw_frame_no_flush(writer, code, &payload) {
                tracing::warn!(error = %e, contract = %contract, req_id, "re-subscribe write failed; treating socket as broken and reconnecting");
                // The session is not live (`authenticated` was never set
                // for this reconnect); carry the class so the next cycle
                // re-enters reconnect on the right ladder instead of
                // reading the broken socket and downgrading to a generic
                // TimedOut.
                pending_reason = Some(reason);
                continue 'session;
            }
            pending_subs
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .insert(
                    req_id,
                    PendingSubEntry {
                        sub: super::protocol::PendingSub::Contract(*kind, contract.clone()),
                        recorded_at: std::time::Instant::now(),
                    },
                );
            tracing::debug!(kind = ?kind, contract = %contract, req_id, "re-subscribed on auto-reconnect");
            if let Err(e) = pacer.frame_written(writer, &shutdown) {
                tracing::warn!(error = %e, "re-subscribe burst flush failed; treating socket as broken and reconnecting");
                pending_reason = Some(reason);
                continue 'session;
            }
            if shutdown.load(Ordering::Relaxed) {
                break 'session;
            }
        }
        for (kind, sec_type) in &full_subs_snapshot {
            let req_id = super::wire_req_id(next_req_id.fetch_add(1, Ordering::Relaxed));
            let payload = protocol::build_full_type_subscribe_payload(req_id, *sec_type);
            let code = kind.subscribe_code();
            if let Err(e) = write_raw_frame_no_flush(writer, code, &payload) {
                tracing::warn!(error = %e, sec_type = ?sec_type, req_id, "re-subscribe write failed; treating socket as broken and reconnecting");
                pending_reason = Some(reason);
                continue 'session;
            }
            pending_subs
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .insert(
                    req_id,
                    PendingSubEntry {
                        sub: super::protocol::PendingSub::Full(*kind, *sec_type),
                        recorded_at: std::time::Instant::now(),
                    },
                );
            tracing::debug!(kind = ?kind, sec_type = ?sec_type, req_id, "re-subscribed full-type on auto-reconnect");
            if let Err(e) = pacer.frame_written(writer, &shutdown) {
                tracing::warn!(error = %e, "re-subscribe burst flush failed; treating socket as broken and reconnecting");
                pending_reason = Some(reason);
                continue 'session;
            }
            if shutdown.load(Ordering::Relaxed) {
                break 'session;
            }
        }
        if !subs_snapshot.is_empty() || !full_subs_snapshot.is_empty() {
            // The trailing flush covers the final partial burst the pacer did
            // not flush. A failure here means the socket broke after the last
            // write, so escalate to reconnect rather than continuing onto a
            // session whose replay never reached the server.
            if let Err(e) = writer.flush() {
                tracing::warn!(error = %e, "re-subscribe batch flush failed; treating socket as broken and reconnecting");
                pending_reason = Some(reason);
                continue 'session;
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
                        // The freshly reconnected socket is already broken if
                        // the queued-command drain cannot write. Re-enter the
                        // reconnect loop so recovery is driven the same way the
                        // re-subscribe replay above handles a mid-flush
                        // failure, rather than running a session whose queued
                        // command never reached the server. `authenticated`
                        // is still `false` here (it is set live only after
                        // this drain succeeds), so the broken socket never
                        // looks live.
                        tracing::warn!(error = %e, "queued-frame write failed on reconnect; treating socket as broken and reconnecting");
                        pending_reason = Some(reason);
                        continue 'session;
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

        // Replay is proven on the fresh socket: re-subscribe and the
        // queued-command drain both wrote and flushed without error. ONLY
        // now is the session marked live and the success announced. Doing
        // this here (rather than right after login) is the invariant that
        // keeps a socket which accepted the login but broke on the next
        // write from ever looking live: on any replay/drain failure above
        // the code re-enters reconnect with `authenticated` still `false`,
        // so `decode_frame` and the command path never treat the broken
        // socket as authenticated, and no `LoginSuccess` / `Reconnected`
        // is published for a session the replay disproved.
        authenticated.store(true, Ordering::Release);
        // The handshake + replay just exchanged frames — feed the
        // staleness clock so `millis_since_last_event()` reflects the live
        // session immediately.
        last_event_at_ns.store(unix_nanos_now(), Ordering::Relaxed);

        // Publish reconnection events. Drain every handshake-time typed
        // control frame (`Connected` / `Ping` / `ReconnectedServer` /
        // `Restart`) in wire order before `LoginSuccess`, so the event
        // order matches the fresh-session bootstrap. Every publish is
        // non-blocking so a saturated ring never wedges the io_loop's
        // reconnect path.
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
    ///
    /// Returns `Err` if the burst flush fails: a failed flush means the
    /// reconnected socket is broken, so the caller re-enters the reconnect
    /// loop rather than pacing out a replay the server never received.
    fn frame_written<W: Write>(
        &mut self,
        writer: &mut W,
        shutdown: &AtomicBool,
    ) -> std::io::Result<()> {
        self.written_in_burst += 1;
        if self.written_in_burst < self.burst_size {
            return Ok(());
        }
        self.written_in_burst = 0;
        writer.flush()?;
        if !self.pace.is_zero() {
            sleep_until_or_shutdown(replay_pause(self.pace), shutdown);
        }
        Ok(())
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

    /// A rate-limited (`TooManyRequests`) drop whose FIRST redial fails
    /// before the reader is replaced with a live stream must keep its
    /// class on the SECOND attempt — the rate-limited floor + budget, not
    /// the fast transient ladder.
    ///
    /// The io_loop re-enters `'session` after a failed redial with only
    /// the dead pre-drop stream to read, whose read can only time out and
    /// re-derive a generic `TimedOut` (Transient). The fix carries the
    /// originating reason across that failure so the next decision class
    /// is derived from the preserved reason, not the stale read. This test
    /// models the exact attempt sequence the loop drives: attempt 1 from
    /// the live drop, attempt 2 from the carried reason. The regression
    /// signature is the counter bucket — a downgraded re-derive would land
    /// attempt 2 in `transient`, leaving `rate_limited` stuck at 1.
    #[test]
    fn rate_limited_class_is_preserved_across_a_failed_first_redial() {
        let limits = ReconnectAttemptLimits::default();
        let mut counters = ReconnectCounters::new(test_schedule());

        // Attempt 1: the live `TooManyRequests` drop. The io_loop derives
        // the class from the just-read reason exactly this way.
        let drop_reason = RemoveReason::TooManyRequests;
        let class1 = ReconnectAttemptLimits::class_for(drop_reason)
            .expect("TooManyRequests is not permanent");
        assert_eq!(class1, ReconnectAttemptClass::RateLimited);
        let attempt1 = counters.record(class1);
        assert_eq!(attempt1, 1);

        // The first redial fails before the reader is reassigned, so the
        // loop carries the originating reason instead of re-deriving from
        // the dead stream. Model the carry: the reason for attempt 2 is the
        // preserved drop reason, NOT a re-read `TimedOut`.
        let carried = drop_reason;
        let class2 = ReconnectAttemptLimits::class_for(carried)
            .expect("carried TooManyRequests is not permanent");

        // The class MUST still be rate-limited. A regression that read the
        // dead stream would yield `TimedOut` here, classified Transient.
        assert_eq!(
            class2,
            ReconnectAttemptClass::RateLimited,
            "a carried TooManyRequests must stay rate-limited on the second attempt"
        );
        assert_ne!(
            ReconnectAttemptLimits::class_for(RemoveReason::TimedOut),
            Some(ReconnectAttemptClass::RateLimited),
            "TimedOut classifies Transient -- the downgrade this test guards against",
        );

        let attempt2 = counters.record(class2);

        // Both attempts incremented the SAME (rate-limited) counter, so the
        // budget consumed is the rate-limited one. A downgrade to Transient
        // would have left this at 1 and bumped `transient` to 1 instead.
        assert_eq!(
            attempt2, 2,
            "the second rate-limited attempt must advance the rate-limited counter to 2"
        );
        assert_eq!(
            counters.transient, 0,
            "no transient attempt may be consumed for a carried rate-limited drop"
        );

        // The second attempt draws on the large rate-limited budget and the
        // 130 s floor, not the 30-attempt transient budget / fast ladder.
        assert_eq!(
            limits.budget_for(class2),
            limits.max_rate_limited_attempts,
            "the carried attempt must spend the rate-limited budget"
        );
        assert!(
            limits.budget_for(class2) > limits.max_attempts,
            "the rate-limited budget must exceed the transient budget the bug would use"
        );
        let floor = crate::fpss::reconnect_delay(carried)
            .expect("TooManyRequests yields a finite reconnect delay");
        assert_eq!(
            floor,
            crate::fpss::protocol::TOO_MANY_REQUESTS_DELAY_MS,
            "the carried attempt must honour the rate-limited floor, not the transient ladder"
        );
    }

    /// A transient login rejection on reconnect (`ServerRestarting`) hands
    /// the loop a FRESH, authoritative server reason. The carry must
    /// forward THAT class, so the next attempt is paced as a server
    /// restart rather than downgraded by a stale read.
    #[test]
    fn transient_login_rejection_carries_its_own_class() {
        let mut counters = ReconnectCounters::new(test_schedule());

        // Originating drop was a generic timeout (Transient)...
        let _ = counters.record(
            ReconnectAttemptLimits::class_for(RemoveReason::TimedOut)
                .expect("TimedOut is not permanent"),
        );

        // ...the redial connects but the server rejects login with
        // ServerRestarting. The loop carries that reason, not the original.
        let carried = RemoveReason::ServerRestarting;
        let class =
            ReconnectAttemptLimits::class_for(carried).expect("ServerRestarting is not permanent");
        assert_eq!(
            class,
            ReconnectAttemptClass::ServerRestart,
            "a transient login rejection must drive the class it reported"
        );
        let attempt = counters.record(class);
        assert_eq!(
            attempt, 1,
            "the server-restart counter advances on its own class, independent of the prior transient attempt"
        );
        assert_eq!(counters.server_restart, 1);
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
            pacer
                .frame_written(&mut writer, &shutdown)
                .expect("counting writer never fails to flush");
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
            pacer
                .frame_written(&mut writer, &shutdown)
                .expect("counting writer never fails to flush");
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

    /// A rejecting `REQ_RESPONSE` (here `MaxStreamsReached`) must untrack the
    /// exact subscription it answers, correlated by `req_id`. After the
    /// rejection the contract is gone from `active_subs` — so it is absent
    /// both from `active_subscriptions()` and from the reconnect-replay set,
    /// which is a clone of `active_subs`. A second, accepted subscription on a
    /// different `req_id` stays tracked and replayable.
    #[test]
    fn rejected_req_response_untracks_sub_and_is_not_replayed() {
        use super::super::protocol::{PendingSub, SubscriptionKind};
        use std::collections::HashMap;

        let rejected = Contract::stock("AAAA");
        let accepted = Contract::stock("BBBB");

        let active_subs: ActiveSubs = Arc::new(Mutex::new(vec![
            (SubscriptionKind::Trade, rejected.clone()),
            (SubscriptionKind::Trade, accepted.clone()),
        ]));
        let active_full_subs: ActiveFullSubs = Arc::new(Mutex::new(Vec::new()));

        let pending_subs: PendingSubs = Arc::new(Mutex::new(HashMap::new()));
        pending_subs.lock().unwrap().insert(
            10,
            PendingSubEntry {
                sub: PendingSub::Contract(SubscriptionKind::Trade, rejected.clone()),
                recorded_at: std::time::Instant::now(),
            },
        );
        pending_subs.lock().unwrap().insert(
            11,
            PendingSubEntry {
                sub: PendingSub::Contract(SubscriptionKind::Trade, accepted.clone()),
                recorded_at: std::time::Instant::now(),
            },
        );

        // req_id 10 is rejected: the server hit the account stream cap.
        apply_req_response(
            10,
            StreamResponseType::MaxStreamsReached,
            &pending_subs,
            &active_subs,
            &active_full_subs,
        );
        // req_id 11 is accepted: stays tracked.
        apply_req_response(
            11,
            StreamResponseType::Subscribed,
            &pending_subs,
            &active_subs,
            &active_full_subs,
        );

        let tracked = active_subs.lock().unwrap().clone();
        assert!(
            !tracked.iter().any(|(_, c)| *c == rejected),
            "a rejected subscription must be removed from active_subs"
        );
        assert!(
            tracked.iter().any(|(_, c)| *c == accepted),
            "an accepted subscription must remain tracked"
        );

        // The reconnect replay snapshots `active_subs`; the rejected contract
        // is therefore not in the replay set and will not be re-attempted.
        let replay_snapshot = active_subs.lock().unwrap().clone();
        assert!(
            !replay_snapshot.iter().any(|(_, c)| *c == rejected),
            "rejected subscription must not appear in the reconnect replay set"
        );

        // Both responses consumed their pending entries.
        assert!(
            pending_subs.lock().unwrap().is_empty(),
            "pending registry must be drained once both responses land"
        );
    }

    /// An unknown `req_id` (the `-1` uncorrelated sentinel, or any id with no
    /// pending entry) must leave the tracked set untouched: without a
    /// correlation, untracking would risk dropping a healthy subscription.
    #[test]
    fn uncorrelated_req_response_leaves_tracked_set_intact() {
        use super::super::protocol::SubscriptionKind;
        use std::collections::HashMap;

        let contract = Contract::stock("CCCC");
        let active_subs: ActiveSubs = Arc::new(Mutex::new(vec![(
            SubscriptionKind::Trade,
            contract.clone(),
        )]));
        let active_full_subs: ActiveFullSubs = Arc::new(Mutex::new(Vec::new()));
        let pending_subs: PendingSubs = Arc::new(Mutex::new(HashMap::new()));

        apply_req_response(
            -1,
            StreamResponseType::Error,
            &pending_subs,
            &active_subs,
            &active_full_subs,
        );

        assert_eq!(
            active_subs.lock().unwrap().len(),
            1,
            "an uncorrelated rejection must not drop a tracked subscription"
        );
    }

    /// A rejecting `REQ_RESPONSE` for a full-stream subscribe untracks the
    /// matching `(kind, sec_type)` entry from `active_full_subs`.
    #[test]
    fn rejected_full_stream_req_response_untracks_full_sub() {
        use super::super::protocol::{PendingSub, SubscriptionKind};
        use crate::tdbe::types::enums::SecType;
        use std::collections::HashMap;

        let active_subs: ActiveSubs = Arc::new(Mutex::new(Vec::new()));
        let active_full_subs: ActiveFullSubs =
            Arc::new(Mutex::new(vec![(SubscriptionKind::Trade, SecType::Option)]));
        let pending_subs: PendingSubs = Arc::new(Mutex::new(HashMap::new()));
        pending_subs.lock().unwrap().insert(
            7,
            PendingSubEntry {
                sub: PendingSub::Full(SubscriptionKind::Trade, SecType::Option),
                recorded_at: std::time::Instant::now(),
            },
        );

        apply_req_response(
            7,
            StreamResponseType::InvalidPerms,
            &pending_subs,
            &active_subs,
            &active_full_subs,
        );

        assert!(
            active_full_subs.lock().unwrap().is_empty(),
            "a rejected full-stream subscription must be removed from active_full_subs"
        );
    }

    /// A correlation older than the TTL is swept; a fresh one survives.
    ///
    /// This bounds the registry against a server that suppresses a
    /// `REQ_RESPONSE` (or answers with the uncorrelated `-1` sentinel a
    /// matching `remove` can never key on) over a long-lived session that
    /// never reconnects.
    #[test]
    fn stale_pending_correlations_are_evicted_by_ttl() {
        use super::super::protocol::{PendingSub, SubscriptionKind};
        use std::collections::HashMap;

        let mut map: HashMap<i32, PendingSubEntry> = HashMap::new();
        // An entry recorded one tick past the TTL is unmatchable and must go.
        map.insert(
            1,
            PendingSubEntry {
                sub: PendingSub::Contract(SubscriptionKind::Trade, Contract::stock("OLD")),
                recorded_at: std::time::Instant::now()
                    - PENDING_SUB_TTL
                    - std::time::Duration::from_secs(1),
            },
        );
        // A just-recorded entry is still awaiting its response and must stay.
        map.insert(
            2,
            PendingSubEntry {
                sub: PendingSub::Contract(SubscriptionKind::Trade, Contract::stock("NEW")),
                recorded_at: std::time::Instant::now(),
            },
        );

        evict_stale_pending(&mut map);

        assert!(
            !map.contains_key(&1),
            "a correlation past its TTL must be evicted"
        );
        assert!(
            map.contains_key(&2),
            "a fresh correlation must survive eviction"
        );
    }

    /// Past the hard cap the oldest correlations are dropped so the registry
    /// can never grow without bound within a single TTL window.
    #[test]
    fn pending_registry_is_capped() {
        use super::super::protocol::{PendingSub, SubscriptionKind};
        use std::collections::HashMap;

        let mut map: HashMap<i32, PendingSubEntry> = HashMap::new();
        let base = std::time::Instant::now();
        // One past the cap, all within the TTL window so only the cap sweep
        // applies. Older `recorded_at` for lower ids so the oldest is id 0.
        for id in 0..=(PENDING_SUB_CAP as i32) {
            map.insert(
                id,
                PendingSubEntry {
                    sub: PendingSub::Contract(SubscriptionKind::Trade, Contract::stock("SYM")),
                    recorded_at: base + std::time::Duration::from_millis(id as u64),
                },
            );
        }
        assert_eq!(map.len(), PENDING_SUB_CAP + 1);

        evict_stale_pending(&mut map);

        assert_eq!(
            map.len(),
            PENDING_SUB_CAP,
            "registry must be held at the cap"
        );
        assert!(
            !map.contains_key(&0),
            "the oldest entry must be the one dropped"
        );
    }

    /// A writer that succeeds for the first `ok_writes` calls, then fails
    /// every subsequent `write`/`flush`. Models a freshly reconnected
    /// socket that accepts the login but breaks part-way through the
    /// re-subscribe replay or the queued-command drain.
    struct FailAfter {
        ok_writes: usize,
        writes: usize,
    }

    impl Write for FailAfter {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            if self.writes >= self.ok_writes {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::BrokenPipe,
                    "reconnected socket broke mid-replay",
                ));
            }
            self.writes += 1;
            Ok(buf.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            if self.writes >= self.ok_writes {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::BrokenPipe,
                    "reconnected socket broke on flush",
                ));
            }
            Ok(())
        }
    }

    /// Faithful reproduction of the reconnect-path replay-then-mark-live
    /// control flow the io_loop runs, parameterised by the writer so a
    /// test can inject a mid-replay break. Returns the post-replay
    /// `(authenticated, pending_reason_set, login_success_published)`
    /// triple. Mirrors the production ordering exactly: `authenticated`
    /// is flipped — and `LoginSuccess` published — ONLY after every
    /// replay write and flush succeeds; any failure sets `pending_reason`
    /// and bails with `authenticated` still `false`.
    ///
    /// Finding #3 invariant under test: a reconnect whose replay fails
    /// must not report the session as authenticated/live.
    fn run_reconnect_replay<W: Write>(
        writer: &mut W,
        subs: &[Contract],
        reason: RemoveReason,
        authenticated: &AtomicBool,
        shutdown: &AtomicBool,
    ) -> (bool, bool, bool) {
        // Entry invariant the loop guarantees: the inner read loop cleared
        // `authenticated` on the drop that started this reconnect.
        authenticated.store(false, Ordering::Release);
        let mut pending_reason: Option<RemoveReason> = None;
        let mut pacer = ReplayPacer::new(4, 0);

        // Re-subscribe replay: a write or burst-flush failure marks the
        // session not-live + reconnect-pending, exactly like the loop.
        let code = super::protocol::SubscriptionKind::Quote.subscribe_code();
        let mut replay_ok = true;
        for contract in subs {
            let payload = match protocol::build_subscribe_payload(1, contract) {
                Ok(p) => p,
                Err(_) => continue,
            };
            if write_raw_frame_no_flush(writer, code, &payload).is_err() {
                pending_reason = Some(reason);
                replay_ok = false;
                break;
            }
            if pacer.frame_written(writer, shutdown).is_err() {
                pending_reason = Some(reason);
                replay_ok = false;
                break;
            }
        }
        if replay_ok && !subs.is_empty() && writer.flush().is_err() {
            pending_reason = Some(reason);
            replay_ok = false;
        }

        if !replay_ok {
            // Re-enter reconnect: authenticated is still false, no success
            // event published.
            return (
                authenticated.load(Ordering::Acquire),
                pending_reason.is_some(),
                false,
            );
        }

        // Replay proven: ONLY now is the session live + success announced.
        authenticated.store(true, Ordering::Release);
        let login_success_published = true;
        (
            authenticated.load(Ordering::Acquire),
            pending_reason.is_some(),
            login_success_published,
        )
    }

    /// Finding #3: a reconnect whose re-subscribe replay fails part-way
    /// must NOT mark the session authenticated/live, must set
    /// `pending_reason` (so the next cycle re-enters reconnect on the
    /// right ladder), and must NOT publish `LoginSuccess`. Before the fix
    /// the loop flipped `authenticated` true right after login — so a
    /// socket that broke during replay looked live and accepted commands
    /// until a later read timeout.
    #[test]
    fn reconnect_replay_failure_does_not_mark_session_live() {
        let authenticated = AtomicBool::new(false);
        let shutdown = AtomicBool::new(false);
        let subs = [
            Contract::stock("AAAA"),
            Contract::stock("BBBB"),
            Contract::stock("CCCC"),
        ];
        // Accept the first write, then break — mid-replay socket death.
        let mut writer = FailAfter {
            ok_writes: 1,
            writes: 0,
        };

        let (live, pending_set, login_published) = run_reconnect_replay(
            &mut writer,
            &subs,
            RemoveReason::TooManyRequests,
            &authenticated,
            &shutdown,
        );

        assert!(
            !live,
            "a reconnect whose replay failed must NOT report the session as live"
        );
        assert!(
            !authenticated.load(Ordering::Acquire),
            "the shared `authenticated` flag must stay false on a failed replay"
        );
        assert!(
            pending_set,
            "a failed replay must set pending_reason so the next cycle re-enters \
             reconnect on the originating class rather than re-reading the broken socket"
        );
        assert!(
            !login_published,
            "no LoginSuccess may be published for a session the replay disproved"
        );
    }

    /// Companion success path: when every replay write and flush
    /// succeeds, the session IS marked live and the success event is
    /// published — the production behaviour the fix must preserve.
    #[test]
    fn reconnect_replay_success_marks_session_live() {
        let authenticated = AtomicBool::new(false);
        let shutdown = AtomicBool::new(false);
        let subs = [Contract::stock("AAAA"), Contract::stock("BBBB")];
        // Never fails.
        let mut writer = FailAfter {
            ok_writes: usize::MAX,
            writes: 0,
        };

        let (live, pending_set, login_published) = run_reconnect_replay(
            &mut writer,
            &subs,
            RemoveReason::TimedOut,
            &authenticated,
            &shutdown,
        );

        assert!(
            live,
            "a fully-replayed reconnect must mark the session live"
        );
        assert!(
            authenticated.load(Ordering::Acquire),
            "the shared `authenticated` flag must be true after a proven replay"
        );
        assert!(
            !pending_set,
            "a successful replay must not set pending_reason"
        );
        assert!(
            login_published,
            "LoginSuccess must be published once the replay is proven"
        );
    }

    /// Finding #3 source guard: in the reconnect path the live-flip
    /// (`authenticated.store(true, ...)`) and the post-reconnect
    /// `LoginSuccess` publish must appear AFTER the re-subscribe replay
    /// and the queued-command drain — never right after login. This pins
    /// the ordering so a future edit cannot reintroduce the premature
    /// flip that let a broken reconnected socket look live.
    #[test]
    fn reconnect_marks_live_only_after_replay_in_source() {
        let src = include_str!("mod.rs");
        let cfg_test_pos = src
            .find("#[cfg(test)]\nmod tests")
            .expect("test module marker present");
        let prod = &src[..cfg_test_pos];

        // Anchor on the reconnect path's reader swap — the replay writes
        // target the stream installed here.
        let reader_swap = prod
            .find("Replace the reader with the new stream so the replay writes")
            .expect("reconnect-path reader swap comment present");
        let after_swap = &prod[reader_swap..];

        let resubscribe_pos = after_swap
            .find("Re-subscribe all active subscriptions on the new connection")
            .expect("reconnect-path re-subscribe replay present");
        let queued_drain_pos = after_swap
            .find("Drain any commands that queued up during reconnection")
            .expect("reconnect-path queued-command drain present");
        let live_flip_pos = after_swap
            .find("authenticated.store(true, Ordering::Release)")
            .expect("reconnect-path live flip present");
        let login_success_pos = after_swap
            .find("StreamControl::LoginSuccess")
            .expect("reconnect-path LoginSuccess publish present");

        assert!(
            resubscribe_pos < live_flip_pos,
            "the live flip must come AFTER the re-subscribe replay starts"
        );
        assert!(
            queued_drain_pos < live_flip_pos,
            "the live flip must come AFTER the queued-command drain"
        );
        assert!(
            queued_drain_pos < login_success_pos,
            "the post-reconnect LoginSuccess must be published AFTER the queued-command drain"
        );
    }

    /// Finding #3 source guard: every reconnect-path replay/drain failure
    /// branch that re-enters the session loop must first set
    /// `pending_reason`, so a broken reconnected socket re-enters
    /// reconnect on the originating class instead of being re-read as a
    /// generic timeout. Counts the `continue 'session` sites in the
    /// replay/drain region and asserts each is immediately preceded by a
    /// `pending_reason = Some(reason)` assignment.
    #[test]
    fn reconnect_replay_failures_set_pending_reason_before_continue() {
        let src = include_str!("mod.rs");
        let cfg_test_pos = src
            .find("#[cfg(test)]\nmod tests")
            .expect("test module marker present");
        let prod = &src[..cfg_test_pos];

        // Scope to the replay + queued-drain region: from the reader swap
        // to the post-drain live flip.
        let start = prod
            .find("Replace the reader with the new stream so the replay writes")
            .expect("reconnect-path reader swap present");
        let end_rel = prod[start..]
            .find("Replay is proven on the fresh socket")
            .expect("post-drain live-flip comment present");
        let region = &prod[start..start + end_rel];

        // The `continue 'session` sites in this region are all
        // socket-broke-mid-replay escalations; each must be reconnect-
        // pending-marked. `break 'session` (shutdown) sites are exempt.
        let continue_sites = region.matches("continue 'session;").count();
        assert!(
            continue_sites >= 5,
            "expected the five replay/drain failure escalations; found {continue_sites}"
        );
        let pending_marks = region.matches("pending_reason = Some(reason);").count();
        assert_eq!(
            pending_marks, continue_sites,
            "every replay/drain `continue 'session` must be preceded by a \
             `pending_reason = Some(reason)` so the broken socket re-enters reconnect \
             on the originating class; found {pending_marks} marks for {continue_sites} \
             continue sites"
        );
    }
}
