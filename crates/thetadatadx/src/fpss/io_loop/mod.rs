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
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc as std_mpsc;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use disruptor::{build_single_producer, Producer, Sequence};

use tdbe::types::enums::{RemoveReason, StreamMsgType};

use crate::auth::Credentials;
use crate::config::{FpssFlushMode, ReconnectPolicy};
use crate::error::Error;

use super::connection;
use super::decode::decode_frame;
use super::delta::DeltaState;
use super::events::{FpssControl, FpssEvent, IoCommand};
use super::framing::{
    self, is_drain_yield, read_frame_into, write_frame, write_raw_frame, write_raw_frame_no_flush,
    Frame, FrameReadState,
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

/// Maximum number of consecutive reconnection attempts before giving up.
pub(in crate::fpss) const MAX_RECONNECT_ATTEMPTS: u32 = 5;

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
// Reason: all parameters are moved into this function from a spawned thread closure.
#[allow(
    clippy::too_many_arguments,
    clippy::needless_pass_by_value,
    clippy::too_many_lines
)]
pub(in crate::fpss) fn io_loop<F>(
    stream: connection::FpssStream,
    cmd_rx: std_mpsc::Receiver<IoCommand>,
    handler: F,
    ring_size: usize,
    shutdown: Arc<AtomicBool>,
    authenticated: Arc<AtomicBool>,
    permissions: String,
    mut pending_control: Vec<FpssControl>,
    _server_addr: String,
    derive_ohlcvc: bool,
    flush_mode: FpssFlushMode,
    policy: ReconnectPolicy,
    creds: Credentials,
    hosts: Vec<(String, u16)>,
    active_subs: ActiveSubs,
    active_full_subs: ActiveFullSubs,
    dropped: Arc<AtomicU64>,
    panics: Arc<AtomicU64>,
) where
    F: FnMut(&FpssEvent) + Send + 'static,
{
    let ring_size = ring::next_power_of_two(ring_size.max(ring::MIN_RING_SIZE));

    let factory = || RingEvent { event: None };
    let wait_strategy = AdaptiveWaitStrategy::fpss_default();

    // The Disruptor consumer thread is the SINGLE consumer between the
    // TLS reader and the user callback. The reader publishes events into
    // the ring; this closure runs on the consumer thread, filters
    // internal-only events, and invokes the user callback wrapped in
    // `catch_unwind` so a panic from user code (or binding glue such as
    // PyO3 / napi `ThreadsafeFunction`) is counted on `panics` and
    // surfaced via `tracing::error!` rather than killing the consumer.
    //
    // `handler` is `FnMut`, but `Producer::handle_events_with` requires
    // `Fn`. Wrap it in a `Mutex` so the consumer thread can call it
    // mutably across the `Fn` boundary. The lock is uncontended in
    // practice — only the Disruptor's single consumer thread ever takes
    // it (single-locker pattern) — so the cost collapses to one
    // unlocked-acquire / unlocked-release per event.
    let handler_cell = Mutex::new(handler);
    let panics_consumer = Arc::clone(&panics);

    let mut producer = build_single_producer(ring_size, factory, wait_strategy)
        .handle_events_with(
            move |ring_event: &RingEvent, _sequence: Sequence, _eob: bool| {
                if let Some(ref evt) = ring_event.event {
                    // Filter out internal-only events (Issue #185).
                    match evt {
                        FpssEvent::Empty | FpssEvent::RawData { .. } => {}
                        _ => {
                            let mut handler = handler_cell
                                .lock()
                                .unwrap_or_else(std::sync::PoisonError::into_inner);
                            // `AssertUnwindSafe` is sound here because
                            // the user callback's captured state lives
                            // behind the `Mutex<F>`; any side effects
                            // observable across a panic boundary are the
                            // user's responsibility, not the SDK's.
                            if catch_unwind(AssertUnwindSafe(|| handler(evt))).is_err() {
                                panics_consumer.fetch_add(1, Ordering::Relaxed);
                                tracing::error!(
                                    target: "thetadatadx::fpss::io_loop",
                                    "user callback panicked on Disruptor consumer thread; \
                                     panic_count incremented, consumer continuing",
                                );
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
    for ctrl in pending_control.drain(..) {
        producer.publish(|slot| {
            slot.event = Some(FpssEvent::Control(ctrl));
        });
    }

    // Publish login success event.
    producer.publish(|slot| {
        slot.event = Some(FpssEvent::Control(FpssControl::LoginSuccess {
            permissions,
        }));
    });

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
    let mut reconnect_attempt: u32 = 0;

    'session: loop {
        // Track consecutive read timeouts to detect the 10s overall timeout.
        let max_consecutive_timeouts = (protocol::READ_TIMEOUT_MS / 50).max(1);
        let mut consecutive_timeouts: u64 = 0;

        // --- Inner read/write loop for one connection session ---
        // When the inner loop breaks, `disconnect_reason` holds the reason.
        let disconnect_reason: RemoveReason = 'inner: loop {
            if shutdown.load(Ordering::Relaxed) {
                break 'session;
            }

            // --- Phase 1: Try to read a frame (short blocking read) ---
            match read_frame_into(&mut reader, &mut frame_buf, &mut frame_state) {
                Ok(Some((code, payload_len))) => {
                    consecutive_timeouts = 0;
                    // Reset reconnect counter on successful data reception.
                    reconnect_attempt = 0;

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
                                slot.event = Some(evt);
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
                                slot.event = Some(evt);
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
                    producer.publish(|slot| {
                        slot.event = Some(FpssEvent::Control(FpssControl::Disconnected {
                            reason: RemoveReason::Unspecified,
                        }));
                    });
                    authenticated.store(false, Ordering::Release);
                    break 'inner RemoveReason::Unspecified;
                }
                Err(ref e) if is_read_timeout(e) => {
                    consecutive_timeouts += 1;
                    if consecutive_timeouts >= max_consecutive_timeouts {
                        tracing::warn!(
                            timeout_ms = protocol::READ_TIMEOUT_MS,
                            "FPSS read timed out (no data for {}ms)",
                            consecutive_timeouts * 50
                        );
                        producer.publish(|slot| {
                            slot.event = Some(FpssEvent::Control(FpssControl::Disconnected {
                                reason: RemoveReason::TimedOut,
                            }));
                        });
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
                    producer.publish(|slot| {
                        slot.event = Some(FpssEvent::Control(FpssControl::Disconnected {
                            reason: RemoveReason::Unspecified,
                        }));
                    });
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
        reconnect_attempt += 1;

        let delay = match &policy {
            ReconnectPolicy::Manual => {
                tracing::info!(reason = ?reason, "manual reconnect policy -- not reconnecting");
                break 'session;
            }
            ReconnectPolicy::Auto => {
                if reconnect_attempt > MAX_RECONNECT_ATTEMPTS {
                    tracing::error!(
                        attempts = reconnect_attempt - 1,
                        "max reconnect attempts reached, giving up"
                    );
                    break 'session;
                }
                if let Some(ms) = reconnect_delay(reason) {
                    Duration::from_millis(ms)
                } else {
                    tracing::error!(reason = ?reason, "permanent disconnect -- not reconnecting");
                    break 'session;
                }
            }
            ReconnectPolicy::Custom(f) => {
                if let Some(d) = f(reason, reconnect_attempt) {
                    d
                } else {
                    tracing::info!(reason = ?reason, "custom policy returned None -- not reconnecting");
                    break 'session;
                }
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
        producer.publish(|slot| {
            slot.event = Some(FpssEvent::Control(FpssControl::Reconnecting {
                reason,
                attempt: reconnect_attempt,
                delay_ms,
            }));
        });

        thread::sleep(delay);

        if shutdown.load(Ordering::Relaxed) {
            break 'session;
        }

        // --- Attempt new TLS connection and re-authenticate ---
        let new_stream = {
            let borrowed: Vec<(&str, u16)> = hosts.iter().map(|(h, p)| (h.as_str(), *p)).collect();
            connection::connect_to_servers(&borrowed)
        };

        let mut new_stream = match new_stream {
            Ok((s, addr)) => {
                tracing::info!(server = %addr, "reconnected to FPSS server");
                s
            }
            Err(e) => {
                tracing::warn!(error = %e, "reconnection failed, will retry");
                // Loop around to try again (reconnect_attempt is already incremented).
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
                    producer.publish(|slot| {
                        slot.event = Some(FpssEvent::Control(FpssControl::Disconnected { reason }));
                    });
                    shutdown.store(true, Ordering::Release);
                    break 'session;
                }
                continue 'session;
            }
        };

        // Set the short I/O read timeout on the new stream.
        let io_read_timeout = Duration::from_millis(50);
        if let Err(e) = new_stream.sock.set_read_timeout(Some(io_read_timeout)) {
            tracing::warn!(error = %e, "failed to set read timeout on reconnect");
            continue 'session;
        }

        // Clear delta state -- fresh connection means fresh deltas.
        delta_state.clear();
        local_contracts.clear();

        authenticated.store(true, Ordering::Release);

        // Publish reconnection events. Drain every handshake-time typed
        // control frame (`Connected` / `Ping` / `ReconnectedServer` /
        // `Restart`) in wire order before `LoginSuccess`, so the event
        // order matches the fresh-session bootstrap above.
        for ctrl in reconnect_pending_control.drain(..) {
            producer.publish(|slot| {
                slot.event = Some(FpssEvent::Control(ctrl));
            });
        }
        producer.publish(|slot| {
            slot.event = Some(FpssEvent::Control(FpssControl::LoginSuccess {
                permissions: new_permissions,
            }));
        });
        producer.publish(|slot| {
            slot.event = Some(FpssEvent::Control(FpssControl::Reconnected));
        });

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
            let payload = match protocol::build_subscribe_payload(-1, contract) {
                Ok(p) => p,
                Err(e) => {
                    tracing::warn!(error = %e, contract = %contract, "skipping re-subscribe; contract no longer encodes");
                    continue;
                }
            };
            let code = kind.subscribe_code();
            if let Err(e) = write_raw_frame_no_flush(writer, code, &payload) {
                tracing::warn!(error = %e, contract = %contract, "failed to re-subscribe on reconnect");
            } else {
                tracing::debug!(kind = ?kind, contract = %contract, "re-subscribed on auto-reconnect");
            }
        }
        for (kind, sec_type) in &full_subs_snapshot {
            let payload = protocol::build_full_type_subscribe_payload(-1, *sec_type);
            let code = kind.subscribe_code();
            if let Err(e) = write_raw_frame_no_flush(writer, code, &payload) {
                tracing::warn!(error = %e, sec_type = ?sec_type, "failed to re-subscribe full-type on reconnect");
            } else {
                tracing::debug!(kind = ?kind, sec_type = ?sec_type, "re-subscribed full-type on auto-reconnect");
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
    fn max_reconnect_attempts_is_5() {
        assert_eq!(MAX_RECONNECT_ATTEMPTS, 5);
    }

    /// Finding #2 coverage: permanent disconnect reasons during the
    /// reconnect handshake must short-circuit the reconnect loop
    /// rather than burn MAX_RECONNECT_ATTEMPTS cycles.
    /// `reconnect_delay(reason).is_none()` is the single source of
    /// truth for "no amount of retrying will fix this", so the test
    /// asserts the predicate behaviour for every enumerated permanent
    /// reason. A regression that omits any of these from the
    /// short-circuit would burn ~5 cycles of Disconnected/Reconnecting
    /// noise before giving up, ballooning operator-facing log volume
    /// and delaying the Error bubble-up.
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
