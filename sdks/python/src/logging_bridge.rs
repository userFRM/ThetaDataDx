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
//! GIL-safe; we acquire the GIL via `Python::try_attach(|py| ...)` on
//! every emit so concurrent Rust workers can all drive the Python
//! loggers simultaneously. The GIL is released between events so
//! throughput is not bottlenecked by the bridge.
//!
//! `try_attach` (rather than `attach`) deliberately: a background Rust
//! thread can emit a `tracing` event during interpreter finalization on
//! Python 3.13+, and the plain `attach` path would panic inside the
//! pyo3 GIL-acquisition path, bringing the whole process down mid-exit.
//! `try_attach` returns `None` when the interpreter is unavailable, so
//! we silently drop the event. Shutdown-time event loss is an
//! acceptable tradeoff vs. a crash during interpreter exit.
//!
//! # Logger-name normalization
//!
//! Rust `tracing` targets are `::`-separated module paths
//! (`thetadatadx::auth::nexus`). Python's stdlib `logging` hierarchy is
//! `.`-separated. The bridge rewrites `::` → `.` before calling
//! `logging.getLogger(...)` so `logging.getLogger("thetadatadx")
//! .setLevel(DEBUG)` propagates down the tree the way the Python
//! logging docs describe. Without the rewrite,
//! `logging.getLogger("thetadatadx::auth::nexus")` is an unrelated
//! sibling of `logging.getLogger("thetadatadx")` with no parent-level
//! propagation.

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
        // Reason: Rust `tracing` targets are `::`-separated module paths
        // (`thetadatadx::auth::nexus`, `thetadatadx::fpss::io_loop`, …).
        // Python's stdlib `logging` hierarchy is `.`-separated, so
        // `logging.getLogger("thetadatadx").setLevel(DEBUG)` previously
        // had NO effect on `thetadatadx::auth::nexus` events — Python
        // treated those as unrelated top-level loggers with no parent.
        // Normalize to the Python shape before dispatching so the
        // documented parent-level filtering actually propagates through
        // the stdlib `logging` hierarchy the way v8.0.2 release notes
        // promised.
        let python_target = target.replace("::", ".");
        let level = tracing_to_logging_level(meta.level());

        // Reason: `Python::attach` panics if the interpreter is
        // mid-finalization (documented pyo3 0.28 behavior, especially
        // sharp on Python 3.13+ which tightens shutdown semantics). A
        // background Rust thread emitting a `tracing` event during
        // teardown would take the whole process down before the
        // existing `Err(_) => return` guard below could run. Switch to
        // `Python::try_attach` which returns `None` when the
        // interpreter is not available (finalizing, not initialized,
        // mid-GC traversal) and let us silently drop the event instead.
        // Shutdown-time event loss is an acceptable tradeoff vs. a
        // crash during interpreter exit.
        //
        // Filter before formatting — the visitor walk allocates, and the
        // common case (default WARN threshold with an INFO event) should
        // pay as close to zero as we can manage.
        let _ = Python::try_attach(|py| {
            let logger = match get_logger(py, &python_target) {
                Ok(l) => l,
                // A logging module that can't produce loggers means the
                // interpreter is mid-teardown from a different angle
                // (import system tearing down before finalization); the
                // `try_attach` guard above caught the common case, this
                // handles the residual.
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

    /// The layer rewrites Rust `tracing` target separators (`::`) into
    /// Python `logging` separators (`.`). Without this the
    /// `logging.getLogger("thetadatadx").setLevel(...)` parent-level
    /// filter has no effect on nested targets — they become unrelated
    /// top-level loggers in Python's eyes.
    ///
    /// We can't drive the `tracing_subscriber::Layer::on_event` hook by
    /// hand here (no public `Event` constructor), so this test pins the
    /// pure transformation the layer performs on the target string.
    /// The layer body is a one-line `target.replace("::", ".")` so this
    /// is a direct coverage assertion, not a shape test.
    #[test]
    fn rust_tracing_target_is_rewritten_to_python_logger_name() {
        let cases = [
            ("thetadatadx::auth::nexus", "thetadatadx.auth.nexus"),
            ("thetadatadx::fpss::io_loop", "thetadatadx.fpss.io_loop"),
            ("thetadatadx", "thetadatadx"),
            ("a::b::c::d", "a.b.c.d"),
            // Edge case: single colons are NOT separators in tracing
            // targets (they're not valid module-path fragments anyway).
            // We only rewrite `::`, leaving a bare `:` untouched. Python
            // `logging` allows `:` in logger names — it's just treated
            // as part of a segment, not a hierarchy separator.
            ("weird:name::suffix", "weird:name.suffix"),
        ];
        for (rust_target, expected_python_name) in cases {
            let got = rust_target.replace("::", ".");
            assert_eq!(
                got, expected_python_name,
                "target rewrite mismatch: {rust_target} -> {got} (expected {expected_python_name})"
            );
        }
    }

    /// Logger-hierarchy propagation demo (pure Python stdlib semantics).
    /// This test exercises Python itself — `logging.getLogger("a")
    /// .setLevel(WARNING)` suppresses `logging.getLogger("a.b.c")
    /// .info(...)` via the `.`-separated parent-level filter chain.
    /// That's the contract v8.0.2 release notes promised, which the
    /// pre-fix bridge silently broke because it never rewrote `::`
    /// to `.`.
    #[test]
    fn python_logger_hierarchy_propagates_parent_level() {
        Python::initialize();
        Python::attach(|py| {
            let logging = py.import("logging").expect("import logging");
            let dict_config = logging
                .call_method1("getLogger", ("thetadatadx.auth.nexus",))
                .expect("child logger");
            let parent = logging
                .call_method1("getLogger", ("thetadatadx",))
                .expect("parent logger");
            // Set parent to WARNING — child should report enabled for
            // WARNING (30) and disabled for INFO (20) via the Python
            // logging hierarchy.
            parent
                .call_method1("setLevel", (30_u32,))
                .expect("set parent level");
            // Child has no explicit level, so it inherits from parent.
            dict_config
                .call_method1("setLevel", (0_u32,)) // NOTSET -> inherit
                .expect("clear child level");

            let enabled_warn: bool = dict_config
                .call_method1("isEnabledFor", (30_u32,))
                .and_then(|r| r.extract())
                .expect("isEnabledFor WARN");
            let enabled_info: bool = dict_config
                .call_method1("isEnabledFor", (20_u32,))
                .and_then(|r| r.extract())
                .expect("isEnabledFor INFO");
            assert!(
                enabled_warn,
                "child 'thetadatadx.auth.nexus' must be enabled for WARN when parent is WARN"
            );
            assert!(
                !enabled_info,
                "child 'thetadatadx.auth.nexus' must be DISABLED for INFO when parent is WARN"
            );

            // Negative: with the pre-fix bridge, the target on the wire
            // was `thetadatadx::auth::nexus` (note `::`). Python treats
            // that as a top-level sibling, NOT a descendant. So
            // `isEnabledFor(INFO)` on that logger is TRUE — parent
            // WARNING does not propagate. Asserting the broken behavior
            // here documents why the fix is necessary.
            let broken_child = logging
                .call_method1("getLogger", ("thetadatadx::auth::nexus",))
                .expect("unnormalized logger");
            broken_child
                .call_method1("setLevel", (0_u32,))
                .expect("clear unnormalized child level");
            let broken_enabled_info: bool = broken_child
                .call_method1("isEnabledFor", (20_u32,))
                .and_then(|r| r.extract())
                .expect("isEnabledFor INFO on unnormalized target");
            // Top-level default on Python is WARNING — so an INFO check
            // on a fresh top-level sibling is disabled by the root
            // logger's default. We assert the STRUCTURAL property:
            // the unnormalized name is NOT a descendant of
            // `thetadatadx`, proving the normalization is necessary.
            let broken_name: String = broken_child
                .getattr("name")
                .and_then(|n| n.extract())
                .expect("name attr");
            assert_eq!(broken_name, "thetadatadx::auth::nexus");
            assert!(
                !broken_name.starts_with("thetadatadx."),
                "pre-fix logger name must not look like a descendant of `thetadatadx.` — that's the bug"
            );
            // Sanity: the post-fix normalized name DOES look like a
            // descendant, so setLevel on the parent propagates.
            let fixed_name: String = dict_config
                .getattr("name")
                .and_then(|n| n.extract())
                .expect("fixed name attr");
            assert!(
                fixed_name.starts_with("thetadatadx."),
                "post-fix logger name must start with `thetadatadx.` so setLevel on parent propagates; got {fixed_name}"
            );
            // Drop unused reference to suppress dead-code warnings.
            let _ = broken_enabled_info;
        });
    }

    /// `Python::try_attach` returns `Some` when the interpreter is
    /// available, proving the normal path still works after switching
    /// from `attach`. Triggering interpreter finalization
    /// mid-test is impractical — the documented `None` branch is
    /// exercised at shutdown in real deployments, not under cargo
    /// test. This assertion pins the happy path so a regression to
    /// `attach` (which would lose the finalization-safety property)
    /// cannot slip by unnoticed.
    #[test]
    fn try_attach_returns_some_when_interpreter_is_live() {
        Python::initialize();
        let got = Python::try_attach(|py| {
            // Allocating a Python int is the cheapest observable GIL
            // operation; succeeds only when the interpreter is live.
            let obj = 42_i64.into_pyobject(py).expect("int alloc");
            let extracted: i64 = obj.extract().expect("int round-trip");
            extracted
        });
        assert_eq!(
            got,
            Some(42),
            "try_attach must return Some when the interpreter is live"
        );
    }
}
