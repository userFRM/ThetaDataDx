//! Forward Rust `tracing` events to Python's stdlib `logging`.
//!
//! Vendor parity: the upstream vendor Python SDK installs a
//! `logging.NullHandler` at import time so Rust-side `tracing::info!` /
//! `warn!` calls emitted by `thetadatadx` can flow into user-configured
//! handlers. Prior to v8.0.2 the DX Python wheel silently swallowed every
//! `tracing` event; after this bridge is installed, callers can write:
//!
//! ```python
//! import logging
//! logging.getLogger("thetadatadx").setLevel(logging.DEBUG)
//! # Rust-side tracing events now flow into the configured handlers.
//! ```
//!
//! # Zero-cost when disabled
//!
//! `logging.Logger.isEnabledFor(level)` is a cheap stdlib bool check. The
//! bridge short-circuits on every event before formatting the message, so
//! a production wheel with the default WARN level pays the cost of one
//! bool roundtrip per event — no `format!` allocation, no visitor walk.
//!
//! # Threading model
//!
//! `tracing` events fire from any thread. Python's stdlib `logging` is
//! GIL-safe; we acquire the GIL via `Python::attach(|py| ...)` on every
//! emit so concurrent Rust workers can all drive the Python loggers
//! simultaneously. The GIL is released between events so throughput is
//! not bottlenecked by the bridge.

use std::fmt::Write as _;

use pyo3::prelude::*;
use tracing::field::{Field, Visit};
use tracing::{Event, Level, Subscriber};
use tracing_subscriber::layer::{Context, Layer};

/// Stdlib `logging` numeric levels (mirrored from `logging.py`):
///
/// ```text
/// CRITICAL = 50
/// ERROR    = 40
/// WARNING  = 30
/// INFO     = 20
/// DEBUG    = 10
/// NOTSET   = 0
/// ```
fn tracing_to_logging_level(level: &Level) -> u32 {
    match *level {
        Level::ERROR => 40,
        Level::WARN => 30,
        Level::INFO => 20,
        Level::DEBUG => 10,
        Level::TRACE => 5, // Below DEBUG — matches the common "SPAM" custom level.
    }
}

/// Visitor that concatenates every `tracing` field into a single message
/// string (matching the shape Python's stdlib `logging` expects). Fields
/// are rendered as `key=value` with space separators; the bare `message`
/// field is treated as the primary record text.
#[derive(Default)]
struct EventFormatter {
    message: String,
    fields: String,
}

impl Visit for EventFormatter {
    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        if field.name() == "message" {
            let _ = write!(self.message, "{value:?}");
        } else {
            let _ = write!(self.fields, " {}={:?}", field.name(), value);
        }
    }

    fn record_str(&mut self, field: &Field, value: &str) {
        if field.name() == "message" {
            self.message.push_str(value);
        } else {
            let _ = write!(self.fields, " {}={:?}", field.name(), value);
        }
    }
}

impl EventFormatter {
    fn into_message(self) -> String {
        if self.fields.is_empty() {
            self.message
        } else {
            format!("{}{}", self.message, self.fields)
        }
    }
}

/// `tracing_subscriber::Layer` that forwards each event to
/// `logging.getLogger(target).log(level, message)`. Constructed once at
/// module init; owns no state other than the cached `logging.getLogger`
/// callable (resolved lazily on first use per logger target to avoid a
/// cold-start import cycle).
pub struct PythonLoggingLayer;

impl<S> Layer<S> for PythonLoggingLayer
where
    S: Subscriber + for<'a> tracing_subscriber::registry::LookupSpan<'a>,
{
    fn on_event(&self, event: &Event<'_>, _ctx: Context<'_, S>) {
        let meta = event.metadata();
        let target = meta.target();
        let level = tracing_to_logging_level(meta.level());

        // Filter before formatting — the visitor walk allocates, and the
        // common case (default WARN threshold with an INFO event) should
        // pay as close to zero as we can manage.
        Python::attach(|py| {
            let logger = match get_logger(py, target) {
                Ok(l) => l,
                // A logging module that can't produce loggers means the
                // interpreter is mid-teardown; silently drop the event.
                Err(_) => return,
            };

            match logger
                .call_method1("isEnabledFor", (level,))
                .and_then(|r| r.extract::<bool>())
            {
                Ok(true) => {}
                // Disabled or failed bool extract — drop the event. The
                // former is the hot path, the latter means the Python
                // logger was replaced with something non-standard; either
                // way, don't fight it.
                _ => return,
            }

            let mut formatter = EventFormatter::default();
            event.record(&mut formatter);
            let message = formatter.into_message();

            // `logger.log(level, msg)` — Python's stdlib logging entry
            // point. Errors from the log call itself are silently
            // dropped (the alternative is to spam stderr and break tests
            // that mute stderr to trace the harness).
            //
            // Build the args tuple via the heterogeneous `call_method1`
            // path: pyo3 erases the element types into `Py<PyAny>` inside
            // the tuple converter so we don't have to juggle
            // `Option<Bound<PyInt>>` and `Option<Bound<PyString>>` into a
            // common type.
            let _ = logger.call_method1("log", (level, message));
        });
    }
}

/// Resolve `logging.getLogger(target)` — pyo3 will cache the import site.
/// Small string allocation per call is acceptable; the filter cost
/// dominates.
fn get_logger<'py>(py: Python<'py>, target: &str) -> PyResult<Bound<'py, PyAny>> {
    let logging = py.import("logging")?;
    logging.call_method1("getLogger", (target,))
}

/// Install the `PythonLoggingLayer` as a global tracing subscriber.
///
/// Idempotent: repeated calls (e.g. from test harnesses) are no-ops after
/// the first successful install. Uses `tracing::subscriber::set_global_default`,
/// which returns `Err` once a global subscriber is already set — we swallow
/// that error on purpose so the module init doesn't fail when the host
/// application (e.g. a Rust binary embedding CPython) already installed
/// its own subscriber.
pub fn install_logging_bridge() {
    use tracing_subscriber::layer::SubscriberExt;

    let subscriber = tracing_subscriber::Registry::default().with(PythonLoggingLayer);
    // Only sets the default if none exists. If the host Rust program has
    // already configured `tracing`, we defer to it — the Python caller
    // can still drive per-logger levels through stdlib `logging` and
    // read back the Rust events that the host happens to forward.
    let _ = tracing::subscriber::set_global_default(subscriber);
}

#[cfg(test)]
mod tests {
    //! Level-filter tests for the tracing → logging bridge. The actual
    //! end-to-end forward path (Rust tracing → Python logger.log) requires
    //! a Python interpreter with `logging` available and is covered by
    //! the smoke test in `tests/test_logging_bridge.py` when the wheel is
    //! built; here we focus on the level-mapping pure function.

    use super::*;

    #[test]
    fn error_level_maps_to_logging_40() {
        assert_eq!(tracing_to_logging_level(&Level::ERROR), 40);
    }

    #[test]
    fn warn_level_maps_to_logging_30() {
        assert_eq!(tracing_to_logging_level(&Level::WARN), 30);
    }

    #[test]
    fn info_level_maps_to_logging_20() {
        assert_eq!(tracing_to_logging_level(&Level::INFO), 20);
    }

    #[test]
    fn debug_level_maps_to_logging_10() {
        assert_eq!(tracing_to_logging_level(&Level::DEBUG), 10);
    }

    #[test]
    fn trace_level_maps_below_debug() {
        // Below stdlib DEBUG (10) so `logging.getLogger(...).setLevel(DEBUG)`
        // does NOT include TRACE events by default. Matches tracing
        // semantics — TRACE is intended for per-iteration tight loops
        // that consumers have to explicitly opt into.
        let lvl = tracing_to_logging_level(&Level::TRACE);
        assert!(
            lvl < 10,
            "TRACE should sit below logging.DEBUG (10); got {lvl}"
        );
    }

    #[test]
    fn empty_event_formatter_returns_empty_message() {
        // Baseline: a visitor that observes no fields renders an empty
        // string. Non-empty paths are exercised end-to-end when the
        // wheel is loaded; `tracing::field::Field` has no public
        // constructor in the public API so we can't easily drive the
        // visitor by hand.
        let formatter = EventFormatter::default();
        assert_eq!(formatter.into_message(), "");
    }
}
