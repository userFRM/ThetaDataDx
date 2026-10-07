//! Process-wide pacing for new outbound connections.
//!
//! The vendor rate-limits new connections per public address: an address
//! that opens about 20 connections to its streaming or market-data
//! services within 2 seconds is blocked for up to an hour, and while it
//! is blocked every new connection from it times out, so a client cannot
//! recover until the block clears. Two paths reach that rate without any
//! help: a market-data pool opening its whole channel set at startup,
//! and a retry ladder during an outage, where every attempt whose
//! connection is gone dials a new one.
//!
//! Every path that opens a connection takes a slot here first: the
//! market-data channel dial (and so the pool's open at startup), the
//! streaming client's connect and reconnect, and the flat-file session.
//! An established connection never touches the budget, so nothing on
//! the request path pays for it.
//!
//! # Shape
//!
//! A token bucket whose whole state is the instant the next slot comes
//! due. A caller waits for that instant and then takes the slot, which
//! moves the instant on by [`REFILL_INTERVAL`]. There is no refill task
//! and no shared wakeup, so taking a slot costs one uncontended mutex
//! acquisition, and the same two calls serve the async paths and the
//! streaming client's blocking thread.
//!
//! A slot is spent when it is taken, never when it is merely waited
//! for, so a connect that is abandoned while waiting leaves the budget
//! as it found it. That is what keeps the budget from outliving the
//! trouble that filled it: the due instant can never be more than one
//! interval ahead of the present, however many callers are waiting or
//! giving up, so a client whose requests all time out during an outage
//! still dials the moment the service is back.
//!
//! Waiters are not served in order. Each one sleeps until the due
//! instant and takes the slot if it is still there, so a caller can be
//! overtaken by one that arrived later and waits another interval. Every
//! waiter is a connection about to be opened and any order opens them at
//! the same rate, so the arithmetic that matters is unaffected and the
//! wait stays bounded by the number of connections being opened at once.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{LazyLock, Mutex, PoisonError};
use std::time::{Duration, Instant};

/// Connections an idle process may open back to back.
const BURST: u32 = 4;

/// Spacing between slots once the burst is spent.
const REFILL_INTERVAL: Duration = Duration::from_millis(250);

/// Instant the next slot comes due. Starts the burst allowance in the
/// past, so the first connections of a process open without waiting.
static NEXT_SLOT: LazyLock<Mutex<Instant>> =
    LazyLock::new(|| Mutex::new(burst_floor(Instant::now())));

/// Longest a blocking wait sleeps before it looks at `shutdown` again.
const SHUTDOWN_CHECK_SLICE: Duration = Duration::from_millis(100);

/// Wait for a slot, then take it.
///
/// Cancel-safe in the strong sense: the future holds nothing but a
/// timer until the slot is due, so dropping it leaves the budget
/// untouched rather than spending a slot no connection used.
pub(crate) async fn acquire() {
    while let Err(wait) = claim() {
        tokio::time::sleep(wait).await;
    }
}

/// Blocking counterpart of [`acquire`] for the streaming client, whose
/// connect path runs on its own thread with no runtime.
///
/// Returns `false` when `shutdown` is raised instead of taking a slot,
/// so a client being dropped mid-reconnect stops here rather than
/// waiting out the budget and then dialling a connection nobody will
/// read. The wait is broken into [`SHUTDOWN_CHECK_SLICE`] sleeps, since
/// the thread that raised the flag is joining this one.
pub(crate) fn acquire_blocking(shutdown: &AtomicBool) -> bool {
    loop {
        if shutdown.load(Ordering::Relaxed) {
            return false;
        }
        match claim() {
            Ok(()) => return true,
            Err(wait) => std::thread::sleep(wait.min(SHUTDOWN_CHECK_SLICE)),
        }
    }
}

/// Take the next slot if it is due, or report how long until it is.
///
/// Slots come due at most one per [`REFILL_INTERVAL`], except that a
/// process which has not dialled for a while may take up to [`BURST`]
/// at once: the due instant is allowed to lag the present by the burst
/// allowance, and no further. The most any 2-second window can
/// therefore hold is the burst plus one slot per interval, which is 12,
/// comfortably inside the vendor's limit, and still fast enough to open
/// a channel pool in about a second.
///
/// Because the due instant only moves when a slot is taken, and a slot
/// is only taken once it is due, the instant is never more than one
/// interval ahead of the present. Waiting callers cannot run it away
/// from the clock.
fn claim() -> Result<(), Duration> {
    let now = Instant::now();
    // A plain `Instant` stays consistent across a panic, so a poisoned
    // lock is still usable.
    let mut next = NEXT_SLOT.lock().unwrap_or_else(PoisonError::into_inner);
    let at = (*next).max(burst_floor(now));
    let wait = at.saturating_duration_since(now);
    if !wait.is_zero() {
        return Err(wait);
    }
    *next = at + REFILL_INTERVAL;
    Ok(())
}

/// Furthest behind `now` the due instant may fall: one interval short
/// of the full burst, so an idle process gets [`BURST`] slots at once
/// rather than one more than that.
fn burst_floor(now: Instant) -> Instant {
    now.checked_sub(REFILL_INTERVAL * (BURST - 1))
        .unwrap_or(now)
}
