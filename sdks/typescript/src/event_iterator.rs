// Hand-written napi-rs wrapper for the pull-iter FPSS delivery mode.
//
// Mirrors the Python `EventIterator` (see
// `sdks/python/src/event_iterator.rs`) and the C++ `tdx::EventIterator`
// (see `sdks/cpp/include/thetadx.hpp`). One per-client iterator,
// returned by `ThetaDataDxClient.startStreamingIter()` and surfaced as
// a JS async iterable so user code writes:
//
//   const iter = client.startStreamingIter();
//   for await (const event of iter) {
//       // ...
//   }
//
// `next()` is async — it does the queue wait on
// `tokio::task::spawn_blocking` so the Node main thread is never
// blocked. The Rust side's `EventIterator::next_timeout` returns
// within a fixed 50 ms slice so a `for await` loop wakes up on
// `KeyboardInterrupt` / `process.kill` within a human-perceptible
// delay even when the upstream is quiet.

// This file is `include!`'d into lib.rs (parallel to
// `buffered_event.rs` and `fpss_event_classes.rs`) so the
// `BufferedEvent`, `FpssEvent`, `fpss_event_to_buffered`, and
// `buffered_event_to_typed` symbols resolve in the lib.rs scope
// without an explicit `use crate::...` line.

// `Arc` is in scope from `lib.rs::use std::sync::{Arc, Mutex, OnceLock}`.
use std::sync::atomic::{AtomicBool, Ordering};
use thetadatadx::{EventIterator as RustEventIterator, NextEvent};

/// Per-client pull-iter handle. Exposed as a napi class with an
/// async `next()` method and a `[Symbol.asyncIterator]` hook so JS
/// code drains it with `for await (const event of iter)`.
///
/// Returned by [`crate::ThetaDataDxClient::start_streaming_iter`].
/// Mutually exclusive with `startStreaming(callback)` on the same
/// client; switch by calling `stopStreaming()` first.
#[napi]
pub struct EventIterator {
    /// Wrapped in `Arc` so async methods can clone a cheap handle
    /// into the `spawn_blocking` future. The Rust-side
    /// `RustEventIterator::next_timeout` takes `&self` so the Arc
    /// also lets us share the iterator across the async/blocking
    /// boundary without `&mut`-aliasing concerns.
    inner: Arc<RustEventIterator>,
    /// Set by `close()` so subsequent `next()` calls bail out once
    /// the queue is drained, even if the upstream client is still
    /// live. Independent of the Rust iterator's own `finished`
    /// flag, which is also flipped by close — but the napi class
    /// keeps a parallel mirror so a subsequent `next()` short-
    /// circuits without paying the queue-wait slice.
    closed: Arc<AtomicBool>,
}

impl EventIterator {
    pub(crate) fn new(inner: RustEventIterator) -> Self {
        Self {
            inner: Arc::new(inner),
            closed: Arc::new(AtomicBool::new(false)),
        }
    }
}

#[napi]
impl EventIterator {
    /// Pop the next typed FPSS event, awaiting until one arrives or
    /// the streaming session shuts down. Resolves to the typed
    /// `FpssEvent` napi object on success or `null` once the queue
    /// is drained on a stopped session.
    ///
    /// Long-lived `for await` loops should prefer the async
    /// iterator (`for await (const event of iter)`); `next()` is
    /// retained for callers that want explicit Promise-based
    /// pulling.
    #[napi(js_name = "next")]
    pub async fn next(&self) -> napi::Result<Option<FpssEvent>> {
        // Closed locally? Drain residual without waiting and signal
        // end. Matches the Python iterator's StopIteration ordering.
        if self.closed.load(Ordering::Acquire) {
            // `try_next` since 9.1.0 returns the typed `NextEvent`
            // trichotomy. Only `Ready(evt)` surfaces an event; both
            // `Timeout` (queue empty) and `Closed` (drained +
            // shutdown) resolve to `null` here because the local
            // close flag has already fired, so the JS caller's
            // contract is "drain residuals, then end".
            return Ok(match self.inner.try_next() {
                NextEvent::Ready(evt) => Some(convert_event(evt)),
                NextEvent::Timeout | NextEvent::Closed => None,
            });
        }
        let inner = Arc::clone(&self.inner);
        let closed = Arc::clone(&self.closed);
        // Spawn-blocking so the Node main thread is never blocked.
        // 50 ms slice means the worker thread re-checks the close
        // flag at most 20 times per second; under load the queue
        // is rarely empty so the slice path is cold.
        //
        // The blocking loop terminates on three distinct signals:
        //   * Ready(evt) — resolves the JS promise to the typed event
        //   * Closed     — upstream shutdown observed AND queue drained;
        //                  resolves to `null` so a `for await` loop
        //                  exits via `done: true`. Earlier the
        //                  promise spun forever after `stopStreaming()`
        //                  because `Timeout` and `Closed` were both
        //                  `None` on the Rust side.
        //   * Timeout    — wait window expired but upstream is live;
        //                  re-check the local `close` flag and loop.
        let popped: Option<thetadatadx::fpss::FpssEvent> =
            tokio::task::spawn_blocking(move || loop {
                if closed.load(Ordering::Acquire) {
                    // Drain residual on local close; both `Timeout`
                    // (empty-but-live) and `Closed` (drained +
                    // shutdown) map to `None` so the JS promise
                    // resolves to `null` regardless of which
                    // sub-state the queue is in. Only `Ready` surfaces
                    // a tail event the user hasn't seen yet.
                    return match inner.try_next() {
                        NextEvent::Ready(evt) => Some(evt),
                        NextEvent::Timeout | NextEvent::Closed => None,
                    };
                }
                match inner.next_timeout(std::time::Duration::from_millis(50)) {
                    NextEvent::Ready(evt) => return Some(evt),
                    NextEvent::Closed => return None,
                    NextEvent::Timeout => continue,
                }
            })
            .await
            .map_err(|join_err| {
                napi::Error::from_reason(format!(
                    "EventIterator background task panicked: {join_err}"
                ))
            })?;
        Ok(popped.map(convert_event))
    }

    /// Try to pop the next event without awaiting. Resolves to
    /// `null` on either an empty-but-live queue OR a terminal
    /// end-of-stream (queue drained on a stopped session). Useful
    /// for non-blocking polling integrations. Callers that need to
    /// distinguish the two cases should use the awaiting `next()`
    /// path, which resolves to `null` only on terminal end-of-stream
    /// after the queue has fully drained.
    #[napi(js_name = "tryNext")]
    pub fn try_next(&self) -> Option<FpssEvent> {
        // Both `Timeout` (queue empty, upstream live) and `Closed`
        // (drained + shutdown) collapse to `None` here so the JS
        // public surface stays a simple optional. The 9.1.0 typed-
        // enum upgrade lives on the awaiting `next()` path which
        // resolves to `null` only on `Closed`; non-blocking polling
        // stays single-state by design.
        match self.inner.try_next() {
            NextEvent::Ready(evt) => Some(convert_event(evt)),
            NextEvent::Timeout | NextEvent::Closed => None,
        }
    }

    /// Number of events currently buffered between the Disruptor
    /// consumer and this iterator. Diagnostic only — the value is
    /// racy because the consumer pushes concurrently.
    #[napi(js_name = "queueLen")]
    pub fn queue_len(&self) -> u32 {
        u32::try_from(self.inner.queue_len()).unwrap_or(u32::MAX)
    }

    /// Mark the iterator closed. Subsequent `next()` calls resolve
    /// to `null` once the queue is drained, without shutting down
    /// the underlying streaming session.
    #[napi(js_name = "close")]
    pub fn close(&self) {
        self.closed.store(true, Ordering::Release);
        self.inner.close();
    }
}

fn convert_event(event: thetadatadx::fpss::FpssEvent) -> FpssEvent {
    let buffered: BufferedEvent = fpss_event_to_buffered(&event);
    buffered_event_to_typed(buffered)
}
