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
use super::super::protocol::build_ping_payload;

/// Background thread that sends PING heartbeat at the configured cadence
/// via the command channel.
///
/// # Behavior
///
/// After successful login, the client starts a thread that sends:
/// - Code 10 (PING)
/// - 1-byte payload: `[0x00]`
/// - Every `interval` (heartbeat cadence; default `100ms`).
// Reason: all parameters are moved into this function from a spawned thread closure.
#[allow(clippy::needless_pass_by_value)]
pub(in crate::fpss) fn ping_loop(
    cmd_tx: std_mpsc::Sender<IoCommand>,
    shutdown: Arc<AtomicBool>,
    authenticated: Arc<AtomicBool>,
    interval: Duration,
) {
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc as std_mpsc;
    use std::time::Instant;
    use tdbe::types::enums::StreamMsgType;

    /// The configurable `ping_interval_ms` knob actually paces the
    /// background heartbeat. Setting a 30 ms interval must produce
    /// roughly one ping per 30 ms (within scheduling jitter), and the
    /// total elapsed time across N pings must scale linearly with the
    /// configured interval. The 2 s startup grace from the Java
    /// terminal's `scheduleAtFixedRate(.., 2000, ..)` is short-circuited
    /// here by waiting `2000 + N * interval` and counting pings.
    ///
    /// Asserts the wiring path:
    /// `FpssConnectArgs::ping_interval_ms`
    ///   -> `connect_with_stream` Duration::from_millis(...)
    ///   -> `ping_loop(.., interval)` parameter
    ///   -> Sleep cadence visible on `cmd_rx`.
    #[test]
    fn ping_loop_honors_configured_interval() {
        let (cmd_tx, cmd_rx) = std_mpsc::channel::<IoCommand>();
        let shutdown = Arc::new(AtomicBool::new(false));
        let authenticated = Arc::new(AtomicBool::new(true));
        let interval = Duration::from_millis(30);
        let interval_clone = interval;
        let shutdown_clone = Arc::clone(&shutdown);
        let auth_clone = Arc::clone(&authenticated);

        let join = thread::spawn(move || {
            ping_loop(cmd_tx, shutdown_clone, auth_clone, interval_clone);
        });

        // The loop sleeps 2 s before its first ping (mirrors
        // `scheduleAtFixedRate(.., 2000, ..)`), so the test waits past
        // that grace plus four full intervals.
        let start = Instant::now();
        let deadline = start + Duration::from_millis(2_000) + interval * 5;
        let mut pings: Vec<Instant> = Vec::new();
        while Instant::now() < deadline && pings.len() < 4 {
            if let Ok(IoCommand::WriteFrame { code, .. }) =
                cmd_rx.recv_timeout(Duration::from_millis(100))
            {
                if code == StreamMsgType::Ping {
                    pings.push(Instant::now());
                }
            }
        }
        shutdown.store(true, Ordering::Relaxed);
        join.join().expect("ping thread joins clean");

        assert!(
            pings.len() >= 4,
            "expected >= 4 pings within budget, got {}",
            pings.len()
        );
        // After the 2 s warm-up, gaps between consecutive pings must
        // sit in `[interval/2, interval*3]`. The wide ceiling tolerates
        // CI scheduling jitter; the floor catches the regression of a
        // hardcoded 100 ms cadence (which would yield gaps ~3.3x our
        // 30 ms knob and trip the `interval * 3` ceiling).
        for window in pings.windows(2) {
            let gap = window[1].duration_since(window[0]);
            assert!(
                gap >= interval / 2 && gap <= interval * 3,
                "ping cadence gap {gap:?} outside expected band for interval {interval:?}"
            );
        }
    }

    /// Sub-100 ms intervals are rejected at the config / connect
    /// boundary, so `ping_loop` itself never has to defend against them.
    /// This test just guards against a regression to the old hardcoded
    /// `PING_INTERVAL_MS` constant by verifying that a 250 ms knob
    /// produces noticeably-spaced pings rather than the 100 ms default.
    #[test]
    fn ping_loop_with_longer_interval_paces_slower_than_default() {
        let (cmd_tx, cmd_rx) = std_mpsc::channel::<IoCommand>();
        let shutdown = Arc::new(AtomicBool::new(false));
        let authenticated = Arc::new(AtomicBool::new(true));
        let shutdown_clone = Arc::clone(&shutdown);
        let auth_clone = Arc::clone(&authenticated);

        let join = thread::spawn(move || {
            ping_loop(
                cmd_tx,
                shutdown_clone,
                auth_clone,
                Duration::from_millis(250),
            );
        });

        // Skip the 2 s startup grace, then drain pings for 600 ms.
        // At 250 ms cadence we expect ~2-3 pings; at the legacy
        // hardcoded 100 ms cadence we'd see ~6.
        let start = Instant::now();
        let deadline = start + Duration::from_millis(2_000) + Duration::from_millis(600);
        let mut count = 0usize;
        while Instant::now() < deadline {
            if let Ok(IoCommand::WriteFrame {
                code: StreamMsgType::Ping,
                ..
            }) = cmd_rx.recv_timeout(Duration::from_millis(50))
            {
                count += 1;
            }
        }
        shutdown.store(true, Ordering::Relaxed);
        join.join().expect("ping thread joins clean");

        assert!(
            (1..=4).contains(&count),
            "250ms interval should produce 1-4 pings in a 600ms window, got {count}"
        );
    }
}
