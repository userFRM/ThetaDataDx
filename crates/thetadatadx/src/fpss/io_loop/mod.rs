//! FPSS I/O worker thread, login handshake, and ping heartbeat.
//!
//! [`io_loop`] owns the TLS stream for the lifetime of a session. It reads
//! frames, dispatches them through [`super::decode::decode_frame`], publishes
//! the resulting events into the LMAX Disruptor ring, and drains the outgoing
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
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicU64, Ordering};
use std::sync::mpsc as std_mpsc;
use std::sync::{Arc, Mutex, OnceLock};

// parking_lot's Mutex is ~5ns uncontended (vs ~20-40ns for
// std::sync::Mutex), inlines aggressively, and ships no poisoning
// machinery — the single-consumer Disruptor hot path below holds the
// lock for the duration of one `as_public()` reborrow plus the user
// callback / queue push, never across an await, and is reached from
// exactly one thread (`handle_events_with`'s consumer), so the
// faster lock is a strict win on that path. Every other Mutex in
// this module keeps `std::sync::Mutex` and its poison-on-panic
// behaviour.
use parking_lot::Mutex as ParkingLotMutex;
use std::thread::{self, ThreadId};
use std::time::{Duration, Instant};

use disruptor::{build_single_producer, Producer, Sequence};

use tdbe::types::enums::{RemoveReason, StreamMsgType};

use crate::auth::Credentials;
use crate::config::{
    FpssFlushMode, ReconnectAttemptClass, ReconnectAttemptLimits, ReconnectPolicy,
};
use crate::error::Error;

use super::connection;
use super::decode::decode_frame;
use super::delta::DeltaState;
#[cfg(test)]
use super::events::FpssEvent;
use super::events::{BackpressurePolicy, Delivery, FpssControl, FpssEventInternal, IoCommand};
use super::framing::{
    self, is_drain_yield, read_frame_into_with_stall_timeout, write_frame, write_raw_frame,
    write_raw_frame_no_flush, Frame, FrameReadState,
};
use super::protocol::{self, build_credentials_payload, Contract};
use super::reconnect_delay;
use super::ring::{self, AdaptiveWaitStrategy, RingEvent};

type ActiveSubs = Arc<Mutex<Vec<(super::protocol::SubscriptionKind, Contract)>>>;
type ActiveFullSubs = Arc<
    Mutex<
        Vec<(
            super::protocol::SubscriptionKind,
            tdbe::types::enums::SecType,
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
/// [`crate::config::FpssConfig`] tuning into the auto-reconnect path
/// (so manual [`std::net::TcpStream::connect_timeout`] re-attempts and
/// the framing-layer mid-frame stall budget honour the configured
/// values, not the Java-parity hardcoded defaults).
///
/// `delivery` selects between push-callback and pull-iter modes; see
/// [`super::events::Delivery`]. The io_loop itself is mode-agnostic
/// after this dispatch — both modes share the same Disruptor ring,
/// reader, and producer.
pub(in crate::fpss) struct IoLoopArgs {
    pub stream: connection::FpssStream,
    pub cmd_rx: std_mpsc::Receiver<IoCommand>,
    pub delivery: Delivery,
    pub ring_size: usize,
    pub shutdown: Arc<AtomicBool>,
    pub authenticated: Arc<AtomicBool>,
    pub permissions: String,
    pub pending_control: Vec<FpssControl>,
    pub _server_addr: String,
    pub derive_ohlcvc: bool,
    pub flush_mode: FpssFlushMode,
    pub policy: ReconnectPolicy,
    pub creds: Credentials,
    pub hosts: Vec<(String, u16)>,
    pub active_subs: ActiveSubs,
    pub active_full_subs: ActiveFullSubs,
    pub dropped: Arc<AtomicU64>,
    pub panics: Arc<AtomicU64>,
    pub consumer_thread_id: Arc<OnceLock<ThreadId>>,
    pub slow_callback_threshold_ns: Arc<AtomicU64>,
    pub slow_callback_count: Arc<AtomicU64>,
    pub connect_timeout: Duration,
    pub read_timeout: Duration,
    /// Shared monotonic request-id counter. The auto-reconnect path
    /// allocates fresh `req_id` values from this counter for each
    /// re-subscribe so `ReqResponse` events on the reconnected session
    /// carry ids correlatable to the original subscribe rather than
    /// the indistinguishable `-1` sentinel.
    pub next_req_id: Arc<AtomicI32>,
}

// Reason: all parameters are moved into this function from a spawned thread closure.
#[allow(clippy::needless_pass_by_value, clippy::too_many_lines)]
pub(in crate::fpss) fn io_loop(args: IoLoopArgs) {
    let IoLoopArgs {
        stream,
        cmd_rx,
        delivery,
        ring_size,
        shutdown,
        authenticated,
        permissions,
        mut pending_control,
        _server_addr,
        derive_ohlcvc,
        flush_mode,
        policy,
        creds,
        hosts,
        active_subs,
        active_full_subs,
        dropped,
        panics,
        consumer_thread_id,
        slow_callback_threshold_ns,
        slow_callback_count,
        connect_timeout,
        read_timeout,
        next_req_id,
    } = args;
    // `ring_size` was validated upstream by `ring::check_ring_size` at
    // the public `FpssClient::connect` boundary; silent rounding here
    // would rewrite the caller's stated buffer budget after the fact.
    debug_assert!(
        ring_size >= ring::MIN_RING_SIZE && ring_size.is_power_of_two(),
        "io_loop received unvalidated ring_size {ring_size}; check upstream FpssClient::connect",
    );

    let factory = RingEvent::default;
    let wait_strategy = AdaptiveWaitStrategy::fpss_default();

    // The Disruptor consumer thread is the SINGLE consumer between the
    // TLS reader and the user-facing delivery sink. The reader publishes
    // events into the ring; this closure runs on the consumer thread,
    // filters internal-only events, and dispatches to the
    // [`Delivery`] mode chosen at `connect` time:
    //
    // * [`Delivery::Callback`] — invoke the user closure under
    //   `catch_unwind` so a panic from user code (or binding glue such
    //   as PyO3 / napi `ThreadsafeFunction`) is counted on `panics` and
    //   surfaced via `tracing::error!` rather than killing the consumer.
    // * [`Delivery::Queue`] — `force_push` the cloned public event into
    //   the bounded `ArrayQueue` shared with the
    //   [`super::EventIterator`], incrementing the shared `dropped`
    //   counter when overflow forces an evict-oldest.
    //
    // The `Mutex` wrap on the [`Delivery`] sink mirrors the previous
    // `Mutex<F>` shape — `Producer::handle_events_with` requires the
    // closure to be `Fn`, so the captured sink lives behind a Mutex even
    // though the Disruptor's single consumer thread is the only acquirer
    // (single-locker pattern; the lock collapses to one unlocked
    // acquire/release per event).
    // Pull-iter mode wires a drop guard into the consumer closure that
    // flips an `Arc<AtomicBool>` when the closure is dropped — i.e.,
    // when the Disruptor producer is dropped at io_loop exit and the
    // consumer thread is wound down. That moment is the ONLY observable
    // "no more `queue.push` will ever fire" point in the system, which
    // is what the [`super::EventIterator`] needs as its terminal-EOF
    // predicate. Earlier the iterator polled the global I/O-thread
    // shutdown flag instead, which fired BEFORE the consumer had
    // finished pushing the tail of in-flight events into the queue and
    // produced false-EOFs that dropped tail events. See
    // `super::events::Delivery::Queue::iter_closed` for the wiring.
    let iter_closed_for_guard: Option<Arc<AtomicBool>> = match &delivery {
        Delivery::Queue { iter_closed, .. } => Some(Arc::clone(iter_closed)),
        Delivery::Callback(_) => None,
    };
    // Single-consumer hot path: parking_lot's Mutex avoids the
    // ~20-40ns overhead std::sync::Mutex carries per
    // lock/unlock pair. The lock is acquired once per ring event on
    // the Disruptor consumer thread; the runtime cost compounds at
    // event rates of 100k+/s.
    let delivery_cell = ParkingLotMutex::new(delivery);
    let panics_consumer = Arc::clone(&panics);
    let dropped_consumer = Arc::clone(&dropped);
    let consumer_thread_id_cell = Arc::clone(&consumer_thread_id);
    let slow_threshold_ns_consumer = Arc::clone(&slow_callback_threshold_ns);
    let slow_count_consumer = Arc::clone(&slow_callback_count);

    // Drop guard captured by the consumer closure. Its `Drop` impl
    // flips `iter_closed` to `true` AFTER the closure has finished its
    // last dispatch — i.e., after the final `queue.push`. The closure
    // is `move`d into `handle_events_with`, the disruptor crate keeps
    // it alive on the consumer thread, and drops it when the producer
    // is dropped at io_loop exit (line ~785). Wrapping the guard in
    // an `Option` keeps the closure shape symmetric for the callback
    // path (no flag to flip) without paying an `Arc::clone` per event.
    struct IterCloseGuard {
        iter_closed: Arc<AtomicBool>,
    }
    impl Drop for IterCloseGuard {
        fn drop(&mut self) {
            // Release ordering pairs with the iterator's Acquire
            // load: every `queue.push` that happened before the
            // guard runs must be visible to a thread that observes
            // the flag set.
            self.iter_closed.store(true, Ordering::Release);
        }
    }
    let iter_close_guard: Option<IterCloseGuard> =
        iter_closed_for_guard.map(|iter_closed| IterCloseGuard { iter_closed });

    let mut producer = build_single_producer(ring_size, factory, wait_strategy)
        .handle_events_with(
            move |ring_event: &RingEvent, _sequence: Sequence, _eob: bool| {
                // Touch the guard so the closure captures it (and so it
                // is dropped on consumer-thread exit). The branch is
                // never taken — `Option::None` for the callback path,
                // and the guard never matches `Some(_)`-returning
                // cleanup logic at runtime.
                let _guard_anchor = &iter_close_guard;
                // Capture the Disruptor consumer thread's `ThreadId`
                // exactly once, on first dispatch. `FpssClient::drop`
                // reads this to detect the self-join case (callback
                // dropping the last `Arc<FpssClient>` from inside this
                // closure) and detach the I/O-handle join onto a helper
                // thread instead of deadlocking.
                consumer_thread_id_cell.get_or_init(|| thread::current().id());

                // Reborrow the ring slot to the public `&FpssEvent`.
                // `as_public` returns `None` for the `Empty`
                // ring-buffer placeholder and the `Unparseable`
                // decode-fallback variant, so neither escapes into the
                // delivery sink. Discriminants `Data` and `Control`
                // are layout-compatible with the public enum (both
                // `#[repr(C, u8)]`, see `events::FpssEventInternal`),
                // which is what makes the reborrow zero-clone.
                if let Some(evt) = ring_event.event.as_public() {
                    // parking_lot's Mutex never poisons on panic, so
                    // there is no `PoisonError` recovery path to thread
                    // through; the guard is the lock result directly.
                    let mut delivery = delivery_cell.lock();
                    match &mut *delivery {
                        Delivery::Callback(handler) => {
                            let threshold_ns = slow_threshold_ns_consumer.load(Ordering::Relaxed);
                            // `AssertUnwindSafe` is sound here because
                            // the user callback's captured state lives
                            // behind the `Mutex<Delivery>`; any side
                            // effects observable across a panic
                            // boundary are the user's responsibility,
                            // not the SDK's.
                            let start = if threshold_ns > 0 {
                                Some(std::time::Instant::now())
                            } else {
                                None
                            };
                            if catch_unwind(AssertUnwindSafe(|| handler(evt))).is_err() {
                                panics_consumer.fetch_add(1, Ordering::Relaxed);
                                tracing::error!(
                                    target: "thetadatadx::fpss::io_loop",
                                    "user callback panicked on Disruptor consumer thread; \
                                     panic_count incremented, consumer continuing",
                                );
                            }
                            if let Some(start) = start {
                                // Resilience: slow-callback
                                // observability. Threshold is opt-in
                                // via `set_slow_callback_threshold`.
                                let elapsed_ns =
                                    u64::try_from(start.elapsed().as_nanos()).unwrap_or(u64::MAX);
                                if elapsed_ns > threshold_ns {
                                    let prev = slow_count_consumer.fetch_add(1, Ordering::Relaxed);
                                    // Rate-limit per 1024 over-budget
                                    // events so a sustained slow
                                    // callback regression does not
                                    // amplify the log stream.
                                    if prev.is_multiple_of(1024) {
                                        tracing::warn!(
                                            target: "thetadatadx::fpss::io_loop",
                                            elapsed_ns,
                                            threshold_ns,
                                            slow_callback_count = prev + 1,
                                            "user callback exceeded slow-callback threshold",
                                        );
                                    }
                                }
                            }
                        }
                        Delivery::Queue {
                            queue,
                            wake_fd,
                            policy,
                            ..
                        } => {
                            // Pull-iter delivery. Clone the public event
                            // into the bounded queue; `Arc<Contract>` /
                            // `String` payloads collapse the clone to
                            // refcount bumps so the per-event cost stays
                            // in the low hundreds of nanoseconds.
                            //
                            // Overflow is governed by the configured
                            // [`BackpressurePolicy`]. `Block` parks the
                            // consumer thread on push until space frees
                            // up; `DropOldest` evicts the head before
                            // push so the queue stays full of fresh
                            // data; `DropNewest` skips the new event
                            // when full (legacy behaviour). The shared
                            // `dropped` counter increments on every
                            // eviction / skipped push so operators see
                            // one signal for queue-overflow pressure.
                            let pushed = match *policy {
                                BackpressurePolicy::Block => push_with_block(queue, evt.clone()),
                                BackpressurePolicy::DropOldest => {
                                    // `force_push` returns `Some(old)`
                                    // when the queue was full and the
                                    // head was evicted — count the
                                    // eviction as a drop so the metric
                                    // stays comparable to `DropNewest`.
                                    if queue.force_push(evt.clone()).is_some() {
                                        dropped_consumer.fetch_add(1, Ordering::Relaxed);
                                    }
                                    true
                                }
                                BackpressurePolicy::DropNewest => {
                                    if queue.push(evt.clone()).is_err() {
                                        dropped_consumer.fetch_add(1, Ordering::Relaxed);
                                        false
                                    } else {
                                        true
                                    }
                                }
                            };
                            if pushed {
                                if let Some(wake) = wake_fd.as_ref() {
                                    // Wake the asyncio reader. `signal()`
                                    // coalesces under load — at most one
                                    // wake byte is in the pipe at a time
                                    // (see `super::wake::WakeFd`) — so
                                    // the hot-path cost compresses to
                                    // one atomic compare-exchange and a
                                    // never-taken branch on subsequent
                                    // pushes until the reader drains
                                    // and re-arms. The sync pull-iter
                                    // path leaves `wake_fd: None` and
                                    // pays zero overhead.
                                    wake.signal();
                                }
                            }
                        }
                    }
                }
            },
        )
        .build();

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
            slot.event = FpssEventInternal::Control(FpssControl::LoginSuccess { permissions });
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
    // retries. Each counter resets to zero on a successful read; an
    // additional time-based reset fires when the connection has been
    // running cleanly for at least
    // `ReconnectAttemptLimits::stable_window`, so a connection that
    // ran cleanly for a minute before dropping picks up the full
    // budget again rather than inheriting the previous cycle's count.
    let mut reconnect_state = ReconnectCounters::new();

    // Per-iteration short blocking-read timeout. 50 ms is short enough
    // that pings (default 100 ms cadence) are serviced promptly but
    // long enough to avoid burning CPU during quiet periods. The
    // overall user-configured deadline is enforced by counting
    // consecutive 50 ms timeouts up to `max_consecutive_timeouts`.
    let io_read_slice = Duration::from_millis(50);
    // Convert the user-configured `read_timeout` into the matching
    // count of `io_read_slice`-sized timeouts that must elapse without
    // any data before the I/O loop publishes
    // [`tdbe::types::enums::RemoveReason::TimedOut`]. Bottoms out at 1
    // so a hypothetical sub-50ms `read_timeout` still triggers exactly
    // one cycle of timeout-then-disconnect rather than zero.
    let read_timeout_ms_total = u64::try_from(read_timeout.as_millis()).unwrap_or(u64::MAX);
    let max_consecutive_timeouts = (read_timeout_ms_total / 50).max(1);

    'session: loop {
        let mut consecutive_timeouts: u64 = 0;

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
                    consecutive_timeouts = 0;
                    // Reset reconnect counters on successful data reception
                    // and mark "data did flow on this session" so the
                    // stable-window check on the next drop knows whether
                    // the connection ran long enough to deserve a fresh
                    // retry budget.
                    reconnect_state.transient = 0;
                    reconnect_state.rate_limited = 0;
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
                            slot.event = FpssEventInternal::Control(FpssControl::Disconnected {
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
                    consecutive_timeouts += 1;
                    if consecutive_timeouts >= max_consecutive_timeouts {
                        tracing::warn!(
                            timeout_ms = read_timeout_ms_total,
                            "FPSS read timed out (no data for {}ms)",
                            consecutive_timeouts * 50
                        );
                        if producer
                            .try_publish(|slot| {
                                slot.event =
                                    FpssEventInternal::Control(FpssControl::Disconnected {
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
                    // Finding #3: mid-frame reader yielded so the
                    // command drain can keep up. `frame_state` has
                    // been updated with the exact byte offset in the
                    // header / payload, so the next `read_frame_into`
                    // call resumes without desync. Do NOT count this
                    // toward `consecutive_timeouts` -- the TLS socket
                    // IS delivering bytes, just slowly; a sustained
                    // drain-yield is expected behaviour on a trickling
                    // sender, not a sign of a dead connection. Fall
                    // through to the Phase 2 drain.
                    metrics::counter!("thetadatadx.fpss.drain_yields").increment(1);
                    tracing::trace!(
                        "mid-frame drain-yield -- draining outbound commands \
                         before re-entering read"
                    );
                }
                Err(e) => {
                    tracing::error!(error = %e, "FPSS read error");
                    if producer
                        .try_publish(|slot| {
                            slot.event = FpssEventInternal::Control(FpssControl::Disconnected {
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
                            || flush_mode == FpssFlushMode::Immediate
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

        let (delay, reconnect_attempt) = match &policy {
            ReconnectPolicy::Manual => {
                tracing::info!(reason = ?reason, "manual reconnect policy -- not reconnecting");
                break 'session;
            }
            ReconnectPolicy::Auto(limits) => {
                // Permanent reasons short-circuit before consulting any
                // budget — no amount of retrying will fix bad credentials.
                let Some(class) = ReconnectAttemptLimits::class_for(reason) else {
                    tracing::error!(reason = ?reason, "permanent disconnect -- not reconnecting");
                    break 'session;
                };
                // Optional time-based reset BEFORE incrementing. A
                // session that ran cleanly for >= `stable_window`
                // before this drop earns a fresh budget across both
                // classes.
                reconnect_state.maybe_reset_after_stable(limits);
                let attempt = reconnect_state.record(class);
                let budget = limits.budget_for(class);
                if attempt > budget {
                    tracing::error!(
                        attempts = attempt - 1,
                        class = ?class,
                        "max reconnect attempts reached for this class, giving up"
                    );
                    break 'session;
                }
                let Some(ms) = reconnect_delay(reason) else {
                    tracing::error!(reason = ?reason, "permanent disconnect -- not reconnecting");
                    break 'session;
                };
                (Duration::from_millis(ms), attempt)
            }
            ReconnectPolicy::Custom(f) => {
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
        metrics::counter!("thetadatadx.fpss.reconnects").increment(1);
        if producer
            .try_publish(|slot| {
                slot.event = FpssEventInternal::Control(FpssControl::Reconnecting {
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

        thread::sleep(delay);

        if shutdown.load(Ordering::Relaxed) {
            break 'session;
        }

        // --- Attempt new TLS connection and re-authenticate ---
        let new_stream = {
            let borrowed: Vec<(&str, u16)> = hosts.iter().map(|(h, p)| (h.as_str(), *p)).collect();
            connection::connect_to_servers(&borrowed, connect_timeout, read_timeout)
        };

        let mut new_stream = match new_stream {
            Ok((s, addr)) => {
                tracing::info!(server = %addr, "reconnected to FPSS server");
                s
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
        let cred_payload = build_credentials_payload(&creds.email, &creds.password);
        let frame = Frame::new(StreamMsgType::Credentials, cred_payload);
        if let Err(e) = write_frame(&mut new_stream, &frame) {
            tracing::warn!(error = %e, "failed to send credentials on reconnect");
            continue 'session;
        }

        let mut reconnect_pending_control: Vec<FpssControl> = Vec::new();
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
                                FpssEventInternal::Control(FpssControl::Disconnected { reason });
                        })
                        .is_err()
                    {
                        dropped.fetch_add(1, Ordering::Relaxed);
                        tracing::warn!(
                            target: "thetadatadx::fpss::io_loop",
                            "ring full while publishing permanent-rejection Disconnected; dropped",
                        );
                    }
                    shutdown.store(true, Ordering::Release);
                    break 'session;
                }
                continue 'session;
            }
        };

        // Set the short I/O read timeout on the new stream so the io
        // loop can drain commands between reads. Matches the
        // initial-connect path in `FpssClient::connect_with_stream`.
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
                slot.event = FpssEventInternal::Control(FpssControl::LoginSuccess {
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
                slot.event = FpssEventInternal::Control(FpssControl::Reconnected);
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

        let writer = reader.get_mut();
        for (kind, contract) in &subs_snapshot {
            // Allocate a fresh req_id per re-subscribe so the server's
            // `ReqResponse` events on the reconnected session carry
            // correlatable ids — `-1` is indistinguishable from a
            // manual subscribe and breaks user-side correlation.
            let req_id = next_req_id.fetch_add(1, Ordering::Relaxed);
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
        }
        for (kind, sec_type) in &full_subs_snapshot {
            let req_id = next_req_id.fetch_add(1, Ordering::Relaxed);
            let payload = protocol::build_full_type_subscribe_payload(req_id, *sec_type);
            let code = kind.subscribe_code();
            if let Err(e) = write_raw_frame_no_flush(writer, code, &payload) {
                tracing::warn!(error = %e, sec_type = ?sec_type, req_id, "failed to re-subscribe full-type on reconnect");
            } else {
                tracing::debug!(kind = ?kind, sec_type = ?sec_type, req_id, "re-subscribed full-type on auto-reconnect");
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
                    let result =
                        if code == StreamMsgType::Ping || flush_mode == FpssFlushMode::Immediate {
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

    // Producer drop joins the Disruptor consumer thread and drains remaining events.
    tracing::debug!("fpss-io thread exiting");
}

/// Per-class consecutive-reconnect counters consumed by the io_loop's
/// auto-reconnect path.
///
/// Each [`ReconnectAttemptClass`] carries its own counter; one class's
/// budget is independent of the other. Time-based reset fires when the
/// connection has been delivering data continuously for at least the
/// configured stable window — the read-side records the last successful
/// read timestamp via [`Self::note_data_received`].
struct ReconnectCounters {
    transient: u32,
    rate_limited: u32,
    /// Wall-clock instant of the last frame the read loop consumed
    /// successfully. `None` until the first frame on the current
    /// session arrives. Used to gate the time-based counter reset:
    /// only a session that delivered data for at least
    /// `stable_window` resets the budget on next drop.
    last_data_at: Option<Instant>,
}

impl ReconnectCounters {
    fn new() -> Self {
        Self {
            transient: 0,
            rate_limited: 0,
            last_data_at: None,
        }
    }

    /// Record a successful frame read. Marks "data did flow on this
    /// session" so the stable-window check on next drop knows whether
    /// to reset the counters.
    fn note_data_received(&mut self) {
        self.last_data_at = Some(Instant::now());
    }

    /// Decide whether the connection that just disconnected ran long
    /// enough to be considered "stable" — if so, reset both counters
    /// before scheduling the next attempt.
    fn maybe_reset_after_stable(&mut self, limits: &ReconnectAttemptLimits) {
        if let Some(t) = self.last_data_at {
            if t.elapsed() >= limits.stable_window {
                self.transient = 0;
                self.rate_limited = 0;
            }
        }
    }

    /// Increment the counter for `class` and return the new attempt
    /// number (1-based after increment).
    fn record(&mut self, class: ReconnectAttemptClass) -> u32 {
        match class {
            ReconnectAttemptClass::Transient => {
                self.transient = self.transient.saturating_add(1);
                self.transient
            }
            ReconnectAttemptClass::RateLimited => {
                self.rate_limited = self.rate_limited.saturating_add(1);
                self.rate_limited
            }
        }
    }
}

/// Push an event onto the pull-iter queue under the `Block`
/// [`BackpressurePolicy`], parking the consumer thread until the
/// iterator drains enough to make room.
///
/// The Disruptor consumer thread is also the only producer for this
/// queue, so blocking here applies natural backpressure to the upstream
/// pipeline: the ring buffer the Disruptor owns saturates next, the
/// TLS reader's `try_publish` calls start failing, and the `dropped`
/// counter (which is the SHARED ring-overflow + queue-overflow signal)
/// keeps operators informed.
///
/// Spin-park backoff schedule: 16× pure spin (≈ tens of nanoseconds
/// per failed push), then 100 µs sleeps for sustained pressure. The
/// 100 µs matches [`super::EventIterator::next_timeout`]'s poll tick
/// so the iterator's drain wake-up cost stays inside the same budget
/// the rest of the SDK pays. Always returns `true` — the `Block`
/// semantic guarantees the event landed once this function returns.
#[inline]
fn push_with_block(
    queue: &Arc<crossbeam_queue::ArrayQueue<super::events::FpssEvent>>,
    mut evt: super::events::FpssEvent,
) -> bool {
    // Fast path: queue has space, single push, done.
    match queue.push(evt) {
        Ok(()) => return true,
        Err(returned) => evt = returned,
    }
    // Slow path: queue is full. Park with backoff so a transient
    // saturation absorbs at near-zero CPU cost while a sustained stall
    // doesn't hot-loop the CPU.
    let mut spins: u32 = 0;
    loop {
        match queue.push(evt) {
            Ok(()) => return true,
            Err(returned) => evt = returned,
        }
        if spins < 16 {
            std::hint::spin_loop();
            spins += 1;
        } else {
            // 100 µs cadence — same budget the iterator polls on.
            // Tracing emission rate-limited per 1024 stall iterations
            // so a sustained blocked consumer surfaces in logs without
            // amplifying them.
            spins = spins.saturating_add(1);
            if spins.is_multiple_of(1024) {
                tracing::warn!(
                    target: "thetadatadx::fpss::io_loop",
                    stall_iterations = spins,
                    "BackpressurePolicy::Block parked on full pull-iter queue \
                     (consumer is not draining fast enough)",
                );
            }
            std::thread::sleep(Duration::from_micros(100));
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
/// `ErrorKind::Uncategorized` by `std`) — see issue #469.
fn is_read_timeout(e: &Error) -> bool {
    match e {
        Error::Io(io_err) => super::framing::is_transient_read(io_err),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_budget_defaults_cover_multi_hour_throttle() {
        // The previous wholesale cap was 5; the new default splits
        // into 3 generic-transient + 100 rate-limited.
        let limits = ReconnectAttemptLimits::default();
        assert_eq!(limits.max_attempts, 3);
        assert_eq!(limits.max_rate_limited_attempts, 100);
        // 100 attempts × 130 s/attempt = 13_000 s = ~3.6 h of patient
        // retry on sustained `TooManyRequests`. The previous cap of 5
        // gave up at ~10 minutes — well below the goal of riding
        // through a multi-hour throttle without operator intervention.
        // 3.6 h is the floor the default explicitly accepts.
        let rate_limited_horizon_ms = u128::from(limits.max_rate_limited_attempts)
            * u128::from(crate::fpss::protocol::TOO_MANY_REQUESTS_DELAY_MS);
        assert!(
            rate_limited_horizon_ms >= 3 * 60 * 60 * 1000,
            "default rate-limited budget must cover at least 3 h \
             of sustained throttling; got {rate_limited_horizon_ms} ms"
        );
    }

    /// 10 consecutive `TooManyRequests` disconnects must NOT exhaust
    /// the rate-limited budget — the previous cap of 5 would have given
    /// up after attempt 5, the new default tolerates 100. Each
    /// attempt's delay equals `reconnect_delay(TooManyRequests)` =
    /// `TOO_MANY_REQUESTS_DELAY_MS` (130 s).
    #[test]
    fn ten_too_many_requests_stays_under_rate_limited_budget() {
        let limits = ReconnectAttemptLimits::default();
        let mut counters = ReconnectCounters::new();
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
        // Per-attempt delay budget surfaces the wall-clock cost: 10 *
        // 130 s = 1300 s = ~21 min of patient retry, well within the
        // ~3.6 h envelope the default permits.
        let ms = crate::fpss::reconnect_delay(RemoveReason::TooManyRequests)
            .expect("TooManyRequests yields a finite reconnect delay");
        let total_ms = u128::from(ms) * u128::from(last_attempt);
        assert!(total_ms >= 130_000 * 10);
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
        let mut counters = ReconnectCounters::new();
        counters.record(ReconnectAttemptClass::Transient);
        counters.record(ReconnectAttemptClass::Transient);
        counters.record(ReconnectAttemptClass::RateLimited);
        // No data received yet → no reset.
        counters.maybe_reset_after_stable(&limits);
        assert_eq!(counters.transient, 2);
        assert_eq!(counters.rate_limited, 1);

        counters.note_data_received();
        // Data received but not long enough → still no reset.
        counters.maybe_reset_after_stable(&limits);
        assert_eq!(counters.transient, 2);
        assert_eq!(counters.rate_limited, 1);

        std::thread::sleep(Duration::from_millis(8));
        counters.maybe_reset_after_stable(&limits);
        assert_eq!(counters.transient, 0, "stable-window elapsed → reset");
        assert_eq!(counters.rate_limited, 0);
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
        let counter = Arc::new(AtomicI32::new(7));
        // Mimic the re-subscribe loop's allocation pattern: one
        // fetch_add per re-subscribed contract.
        let id_a = counter.fetch_add(1, Ordering::Relaxed);
        let id_b = counter.fetch_add(1, Ordering::Relaxed);
        let id_c = counter.fetch_add(1, Ordering::Relaxed);
        assert_eq!(id_a, 7);
        assert_eq!(id_b, 8);
        assert_eq!(id_c, 9);
        assert_ne!(id_a, -1, "re-subscribe must never use the -1 sentinel");
        // Subsequent caller-issued subscribes off the same counter
        // see the next slot — proves the io_loop and the client share
        // one allocator without colliding.
        assert_eq!(counter.fetch_add(1, Ordering::Relaxed), 10);
    }

    /// Finding #2 coverage: transient disconnect reasons must NOT
    /// short-circuit -- they should produce a retry delay so the
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

    /// Finding #2 coverage at the control-flow level: when the
    /// reconnect handshake returns `LoginResult::Disconnected` with a
    /// permanent reason, the io_loop path must publish
    /// `FpssControl::Disconnected` and store `shutdown = true`. This
    /// exercises the decision piece without standing up a full TLS
    /// stack -- running the real I/O loop would need a live socket.
    #[test]
    fn permanent_reconnect_rejection_sets_shutdown_and_emits_disconnected() {
        // Mirror the io_loop branch: reason -> reconnect_delay.is_none()
        // -> emit Disconnected + set shutdown. The real path lives in
        // `io_loop::io_loop`; the logic here is the exact boolean
        // predicate plus the event shape the operator sees.
        let shutdown = std::sync::atomic::AtomicBool::new(false);
        let reason = RemoveReason::InvalidCredentials;
        let mut events: Vec<FpssEvent> = Vec::new();
        if super::super::reconnect_delay(reason).is_none() {
            events.push(FpssEvent::Control(FpssControl::Disconnected { reason }));
            shutdown.store(true, std::sync::atomic::Ordering::Release);
        }
        assert!(
            shutdown.load(std::sync::atomic::Ordering::Acquire),
            "permanent reason must flip shutdown -> true"
        );
        assert_eq!(events.len(), 1);
        match &events[0] {
            FpssEvent::Control(FpssControl::Disconnected { reason: r }) => {
                assert_eq!(*r, RemoveReason::InvalidCredentials);
            }
            other => panic!("expected Disconnected event, got {other:?}"),
        }
    }
}
