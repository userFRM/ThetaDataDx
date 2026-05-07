//! Ping heartbeat thread for the FPSS I/O worker.
//!
//! Owns the post-login background sender that emits a 1-byte PING frame
//! every [`PING_INTERVAL_MS`] via the outbound command channel. The
//! actual write happens on the I/O thread; this module only schedules
//! the cadence.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc as std_mpsc;
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use tdbe::types::enums::StreamMsgType;

use super::super::events::IoCommand;
use super::super::protocol::{build_ping_payload, PING_INTERVAL_MS};

/// Background thread that sends PING heartbeat every 100ms via the command channel.
///
/// # Behavior
///
/// After successful login, the client starts a thread that sends:
/// - Code 10 (PING)
/// - 1-byte payload: `[0x00]`
/// - Every 100ms (heartbeat interval).
// Reason: all parameters are moved into this function from a spawned thread closure.
#[allow(clippy::needless_pass_by_value)]
pub(in crate::fpss) fn ping_loop(
    cmd_tx: std_mpsc::Sender<IoCommand>,
    shutdown: Arc<AtomicBool>,
    authenticated: Arc<AtomicBool>,
) {
    let interval = Duration::from_millis(PING_INTERVAL_MS);
    let ping_payload = build_ping_payload();

    // scheduleAtFixedRate(task, 2000L, 100L): first execution at 2000ms,
    // then every 100ms. scheduleAtFixedRate sends THEN waits, so the first
    // ping fires at exactly 2000ms.
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

        // Send ping FIRST, then sleep — matches scheduleAtFixedRate semantics
        // (execute the task, then wait the interval).
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
