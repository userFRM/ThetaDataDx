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
//! A token bucket spent by reservation rather than by polling: the next
//! free instant is the whole state, a grant moves it forward by
//! [`REFILL_INTERVAL`], and the caller waits until the instant it was
//! handed. There is no refill task and no shared wakeup, so a grant
//! costs one uncontended mutex acquisition, and the same reservation
//! serves the async callers and the streaming client's blocking thread.
//!
//! A slot is spent whether or not the caller goes on to dial, so a
//! cancelled connect costs one slot. That errs towards opening fewer
//! connections than allowed, never more.

use std::sync::{LazyLock, Mutex, PoisonError};
use std::time::{Duration, Instant};

/// Connections an idle process may open back to back.
const BURST: u32 = 4;

/// Spacing between slots once the burst is spent.
const REFILL_INTERVAL: Duration = Duration::from_millis(250);

/// Earliest instant the next slot may be used. Starts the burst
/// allowance in the past, so the first connections of a process open
/// without waiting.
static NEXT_SLOT: LazyLock<Mutex<Instant>> =
    LazyLock::new(|| Mutex::new(burst_floor(Instant::now())));

/// Wait for a slot before opening a connection.
///
/// Cancel-safe: the future holds nothing but a timer, and dropping it
/// abandons the slot it reserved.
pub(crate) async fn acquire() {
    let wait = reserve();
    if !wait.is_zero() {
        tokio::time::sleep(wait).await;
    }
}

/// Blocking counterpart of [`acquire`] for the streaming client, whose
/// connect path runs on its own thread with no runtime.
pub(crate) fn acquire_blocking() {
    let wait = reserve();
    if !wait.is_zero() {
        std::thread::sleep(wait);
    }
}

/// Reserve the next slot and return how long to wait for it.
///
/// Slots are handed out at most one per [`REFILL_INTERVAL`], except
/// that a process which has not dialled for a while may use up to
/// [`BURST`] at once: the next-free instant is allowed to lag the
/// present by the burst allowance, and no further. The most any
/// 2-second window can therefore hold is the burst plus one slot per
/// interval, which is 12, comfortably inside the vendor's limit, and
/// still fast enough to open a channel pool in about a second.
fn reserve() -> Duration {
    let now = Instant::now();
    // A plain `Instant` stays consistent across a panic, so a poisoned
    // lock is still usable.
    let mut next = NEXT_SLOT.lock().unwrap_or_else(PoisonError::into_inner);
    let at = (*next).max(burst_floor(now));
    *next = at + REFILL_INTERVAL;
    at.saturating_duration_since(now)
}

/// Furthest behind `now` the next-free instant may fall: one interval
/// short of the full burst, so an idle process gets [`BURST`] slots at
/// once rather than one more than that.
fn burst_floor(now: Instant) -> Instant {
    now.checked_sub(REFILL_INTERVAL * (BURST - 1))
        .unwrap_or(now)
}
