//! FPSS I/O worker thread, login handshake, and ping heartbeat.
//!
//! [`io_loop`] owns the TLS stream for the lifetime of a session. It reads
//! frames, dispatches them through [`super::decode::decode_frame`], publishes
//! the resulting events into the LMAX Disruptor ring, and drains the outgoing
//! command channel between reads. On involuntary disconnect it re-runs the
//! login handshake in-place according to [`ReconnectPolicy`].

use std::collections::HashMap;
use std::io::BufReader;
use std::sync::atomic::{AtomicBool, Ordering};
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
    self, read_frame, read_frame_into, write_frame, write_raw_frame, write_raw_frame_no_flush,
    Frame,
};
use super::protocol::{
    self, build_credentials_payload, build_ping_payload, parse_disconnect_reason, Contract,
    PING_INTERVAL_MS,
};
use super::reconnect_delay;
use super::ring::{self, AdaptiveWaitStrategy, RingEvent};

// ---------------------------------------------------------------------------
// Login result (internal)
// ---------------------------------------------------------------------------

pub(super) enum LoginResult {
    Success(String),
    Disconnected(RemoveReason),
}

/// Wait for the server's login response (blocking).
///
/// Source: `FPSSClient.connect()` -- reads frames until METADATA or DISCONNECTED.
///
/// On `Metadata`, the payload is the server's "Bundle" string. We copy it
/// verbatim into [`LoginResult::Success`]; see
/// [`FpssControl::LoginSuccess`] for why this string is treated as opaque.
pub(super) fn wait_for_login(stream: &mut connection::FpssStream) -> Result<LoginResult, Error> {
    loop {
        let frame = read_frame(stream)?.ok_or_else(|| Error::Fpss {
            kind: crate::error::FpssErrorKind::Disconnected,
            message: "connection closed during login handshake".to_string(),
        })?;

        match frame.code {
            StreamMsgType::Metadata => {
                let permissions = String::from_utf8_lossy(&frame.payload).to_string();
                return Ok(LoginResult::Success(permissions));
            }
            StreamMsgType::Disconnected => {
                let reason = parse_disconnect_reason(&frame.payload);
                return Ok(LoginResult::Disconnected(reason));
            }
            StreamMsgType::Error => {
                let msg = String::from_utf8_lossy(&frame.payload);
                tracing::warn!(message = %msg, "server error during login");
                return Err(Error::Fpss {
                    kind: crate::error::FpssErrorKind::ConnectionRefused,
                    message: format!("server error during login: {msg}"),
                });
            }
            other => {
                tracing::trace!(code = ?other, "ignoring frame during login handshake");
            }
        }
    }
}

// ---------------------------------------------------------------------------
// I/O thread: blocking read + Disruptor publish + command drain
// ---------------------------------------------------------------------------

/// Maximum number of consecutive reconnection attempts before giving up.
pub(super) const MAX_RECONNECT_ATTEMPTS: u32 = 5;

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
pub(super) fn io_loop<F>(
    stream: connection::FpssStream,
    cmd_rx: std_mpsc::Receiver<IoCommand>,
    mut handler: F,
    ring_size: usize,
    shutdown: Arc<AtomicBool>,
    authenticated: Arc<AtomicBool>,
    contract_map: Arc<Mutex<HashMap<i32, Contract>>>,
    permissions: String,
    _server_addr: String,
    derive_ohlcvc: bool,
    flush_mode: FpssFlushMode,
    policy: ReconnectPolicy,
    creds: Credentials,
    hosts: Vec<(String, u16)>,
) where
    F: FnMut(&FpssEvent) + Send + 'static,
{
    let ring_size = ring::next_power_of_two(ring_size.max(ring::MIN_RING_SIZE));

    let factory = || RingEvent { event: None };
    let wait_strategy = AdaptiveWaitStrategy::fpss_default();

    let mut producer = build_single_producer(ring_size, factory, wait_strategy)
        .handle_events_with(
            move |ring_event: &RingEvent, _sequence: Sequence, _eob: bool| {
                if let Some(ref evt) = ring_event.event {
                    // Filter out internal-only events (Issue #185).
                    match evt {
                        FpssEvent::Empty | FpssEvent::RawData { .. } => {}
                        _ => handler(evt),
                    }
                }
            },
        )
        .build();

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

    // Thread-local symbol cache: contract_id -> pre-rendered symbol string.
    // Populated on ContractAssigned events, used by resolve_symbol() and
    // warn_unknown_contract() on every tick -- zero Mutex locks on the hot path.
    // The shared contract_map (Mutex-backed) is still updated for external callers
    // (contract_map(), contract_lookup() public APIs).
    let mut local_symbols: HashMap<i32, Arc<str>> = HashMap::new();

    // Reusable frame payload buffer.
    let mut frame_buf: Vec<u8> = Vec::with_capacity(framing::MAX_PAYLOAD_LEN);

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
            match read_frame_into(&mut reader, &mut frame_buf) {
                Ok(Some((code, payload_len))) => {
                    consecutive_timeouts = 0;
                    // Reset reconnect counter on successful data reception.
                    reconnect_attempt = 0;

                    let (primary, secondary) = decode_frame(
                        code,
                        &frame_buf[..payload_len],
                        &authenticated,
                        &contract_map,
                        &mut local_symbols,
                        &shutdown,
                        &mut delta_state,
                        derive_ohlcvc,
                    );

                    if let Some(evt) = primary {
                        producer.publish(|slot| {
                            slot.event = Some(evt);
                        });
                    }
                    if let Some(evt) = secondary {
                        producer.publish(|slot| {
                            slot.event = Some(evt);
                        });
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
                        let _ = write_raw_frame(writer, StreamMsgType::Stop, &stop_payload);
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

        let login_result = match wait_for_login(&mut new_stream) {
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
        local_symbols.clear();
        contract_map
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clear();

        authenticated.store(true, Ordering::Release);

        // Publish reconnection events.
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

        // Drain any commands that queued up during reconnection (subscribe, ping, etc.)
        // and send them over the new connection to re-establish subscriptions.
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
                    let _ = write_raw_frame(writer, StreamMsgType::Stop, &stop_payload);
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

/// Check if an error is a read timeout (`WouldBlock` or `TimedOut`).
fn is_read_timeout(e: &Error) -> bool {
    match e {
        Error::Io(io_err) => matches!(
            io_err.kind(),
            std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
        ),
        _ => false,
    }
}

// ---------------------------------------------------------------------------
// Ping heartbeat loop
// ---------------------------------------------------------------------------

/// Background thread that sends PING heartbeat every 100ms via the command channel.
///
/// # Behavior (from `FPSSClient.java`)
///
/// After successful login, the Java client starts a thread that sends:
/// - Code 10 (PING)
/// - 1-byte payload: `[0x00]`
/// - Every 100ms
///
/// Source: `FPSSClient.java` heartbeat thread, interval = 100ms.
// Reason: all parameters are moved into this function from a spawned thread closure.
#[allow(clippy::needless_pass_by_value)]
pub(super) fn ping_loop(
    cmd_tx: std_mpsc::Sender<IoCommand>,
    shutdown: Arc<AtomicBool>,
    authenticated: Arc<AtomicBool>,
) {
    let interval = Duration::from_millis(PING_INTERVAL_MS);
    let ping_payload = build_ping_payload();

    // Java: scheduleAtFixedRate(task, 2000L, 100L) — first execution at 2000ms,
    // then every 100ms. scheduleAtFixedRate sends THEN waits, so the first ping
    // fires at exactly 2000ms.
    thread::sleep(Duration::from_millis(2000));

    loop {
        if shutdown.load(Ordering::Relaxed) {
            break;
        }
        if !authenticated.load(Ordering::Relaxed) {
            // Don't send pings if not authenticated
            thread::sleep(interval);
            continue;
        }

        // Send ping FIRST, then sleep — matches Java's scheduleAtFixedRate
        // which executes the task then waits the interval.
        let cmd = IoCommand::WriteFrame {
            code: StreamMsgType::Ping,
            payload: ping_payload.clone(),
        };
        if cmd_tx.send(cmd).is_err() {
            // I/O thread has exited
            break;
        }

        thread::sleep(interval);
    }

    tracing::debug!("fpss-ping thread exiting");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn max_reconnect_attempts_is_5() {
        assert_eq!(MAX_RECONNECT_ATTEMPTS, 5);
    }
}
