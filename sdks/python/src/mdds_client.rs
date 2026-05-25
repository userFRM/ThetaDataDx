//! Standalone Python `MddsClient` pyclass.
//!
//! Opens ONLY the MDDS gRPC channel and the Nexus HTTP authentication
//! flow — no FPSS TLS connection, no Disruptor ring, no streaming
//! state machine. Mirrors the standalone C ABI entry points
//! (`tdx_client_*` in `ffi/src/auth.rs`) and the C++ `tdx::Client`
//! pattern, letting Python users run a historical-only session
//! alongside a parallel FPSS process without the bundled
//! [`crate::ThetaDataDxClient`] preempting the parallel work at the
//! Nexus session layer.
//!
//! # Nexus session behaviour
//!
//! `MddsClient.__new__` issues exactly one Nexus authentication and
//! holds the resulting session UUID for the lifetime of the handle.
//! When a parallel process (or another `MddsClient` / bundled
//! `ThetaDataDxClient` in the same interpreter) authenticates against
//! Nexus with the same credentials, the upstream behaviour is the
//! user's environment concern — the SDK does not invalidate, share,
//! or coordinate session tokens across clients. The
//! `test_concurrent_fpss_and_mdds_share_creds` live test exercises
//! parallel-auth observability for callers that need to confirm
//! end-to-end behaviour.
//!
//! # Surface
//!
//! Internally this pyclass wraps an `Arc<thetadatadx::ThetaDataDxClient>`
//! and forwards historical / list / snapshot / at-time / FLATFILES
//! endpoint calls through PyO3 attribute lookup against an internally
//! held [`crate::ThetaDataDxClient`] pyclass instance. The bundled
//! client opens MDDS gRPC + Nexus at construction time and never
//! opens FPSS unless `start_streaming` is called — by construction
//! and by allowlist enforcement here, no FPSS-touching method is
//! reachable through `MddsClient`.
//!
//! This is the same delegation pattern [`crate::AsyncThetaDataDxClient`]
//! uses for its async-only surface, with an inverted allowlist
//! (block streaming instead of permitting only async-suffixed).

use pyo3::exceptions::PyAttributeError;
use pyo3::prelude::*;

use crate::{Config, Credentials, ThetaDataDxClient};

/// Methods on [`crate::ThetaDataDxClient`] that touch the FPSS
/// transport. Reaching for any of these through `MddsClient` raises
/// `AttributeError` so callers who chose the MDDS-only surface
/// cannot accidentally open an FPSS connection that would conflict
/// with a parallel FPSS process.
///
/// The block-list approach (vs. the inverted `AsyncThetaDataDxClient`
/// allowlist) keeps the historical / FLATFILES surface — which is
/// 50+ generated `*_builder` factories plus per-endpoint sync and
/// async terminals — accessible without listing each one. Adding a
/// new historical endpoint to `ThetaDataDxClient` is automatically
/// available on `MddsClient` with zero edit here.
///
/// Drift guard: the compile-time assertion below pins the generator-emitted
/// streaming surface (`PYTHON_UNIFIED_FPSS_METHODS`, generated from
/// `crates/thetadatadx/sdk_surface.toml`) as a strict subset of
/// `FPSS_TOUCHING_METHODS`. Adding a new generator-emitted FPSS method
/// without also extending this list fails the build, so the block-list
/// cannot silently fall behind. Hand-written FPSS methods on the unified
/// pyclass (`subscribe`, `streaming`, `start_streaming_iter`, …) are
/// covered by the offline coverage test in
/// `tests/test_standalone_clients.py::test_mdds_client_block_list_offline`,
/// which compares the Python-side `BLOCKED_FPSS_METHODS` against the
/// `_blocked_fpss_methods()` introspection helper exposed below.
pub(crate) const FPSS_TOUCHING_METHODS: &[&str] = &[
    // Generator-emitted streaming methods (declared in
    // `crates/thetadatadx/sdk_surface.toml`). The compile-time guard
    // below asserts every name in `PYTHON_UNIFIED_FPSS_METHODS` is
    // present here — adding a new generator-emitted FPSS method
    // without extending this list fails the build.
    "start_streaming",
    "stop_streaming",
    "shutdown",
    "reconnect",
    "is_streaming",
    "await_drain",
    "subscribe",
    "subscribe_many",
    "unsubscribe",
    "unsubscribe_many",
    "active_subscriptions",
    "active_full_subscriptions",
    "dropped_event_count",
    "panic_count",
    // Hand-written `#[pymethods]` entries on `ThetaDataDxClient` /
    // sibling streaming pyclasses. These factories return a
    // streaming-session pyclass (sync, sync-iter, or asyncio) — the
    // session itself transitively opens the FPSS surface, so an
    // MDDS-only handle must refuse access. Drift guard: the offline
    // coverage test in
    // `tests/test_standalone_clients.py::test_mdds_client_block_list_offline`
    // pairs every name here with the
    // `mdds_client._blocked_fpss_methods()` introspection helper.
    "start_streaming_iter",
    "streaming",
    "streaming_iter",
    // P2 closure: `streaming_async()` was added by PR #559 (async
    // FD-readiness surface) and the unified pyclass exposes it
    // hand-written in `streaming_async_session.rs`. Reaching for it
    // through `MddsClient` would open the FPSS surface bound to the
    // hidden inner unified client, so block at the proxy layer.
    "streaming_async",
];

/// `const fn` byte-wise string compare for the compile-time guard
/// below. PyO3 attribute names are ASCII so byte equality is safe.
const fn const_bytes_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut i = 0;
    while i < a.len() {
        if a[i] != b[i] {
            return false;
        }
        i += 1;
    }
    true
}

/// Compile-time drift check: every name in
/// `PYTHON_UNIFIED_FPSS_METHODS` (emitted by
/// `crates/thetadatadx/build_support_bin/sdk_surface/python.rs` from
/// `sdk_surface.toml`) must appear in `FPSS_TOUCHING_METHODS`. Adding
/// a new generator-emitted FPSS method without extending the
/// hand-written block-list above fails the build.
const _: () = {
    let mut i = 0;
    while i < crate::PYTHON_UNIFIED_FPSS_METHODS.len() {
        let needle = crate::PYTHON_UNIFIED_FPSS_METHODS[i].as_bytes();
        let mut found = false;
        let mut j = 0;
        while j < FPSS_TOUCHING_METHODS.len() {
            if const_bytes_eq(FPSS_TOUCHING_METHODS[j].as_bytes(), needle) {
                found = true;
                break;
            }
            j += 1;
        }
        assert!(
            found,
            "PYTHON_UNIFIED_FPSS_METHODS contains a name not in \
             `mdds_client::FPSS_TOUCHING_METHODS` — extend the \
             block-list (and the offline-coverage test) so the MDDS \
             surface stays FPSS-free."
        );
        i += 1;
    }
};

/// Standalone MDDS-only historical client.
///
/// Opens ONLY the MDDS gRPC channel — no FPSS TLS connection.
/// Authenticates once against Nexus at construction time. Use when a
/// parallel FPSS process is already running in the same environment
/// and you need to test historical / FLATFILES endpoints without the
/// bundled [`crate::ThetaDataDxClient`] also opening an FPSS slot.
///
/// ```python
/// from thetadatadx import MddsClient, Credentials, Config
///
/// creds = Credentials.from_file("creds.txt")
/// mdds = MddsClient(creds, Config.production())
///
/// eod = mdds.stock_history_eod("AAPL", "20240101", "20240301")
/// print(eod.to_pandas().head())
/// ```
///
/// Calling streaming / subscribe methods on this pyclass raises
/// `AttributeError` — use the standalone [`crate::FpssClient`] or the
/// bundled [`crate::ThetaDataDxClient`] when you need both surfaces.
// N5: `frozen` — every `#[pymethods]` entry takes `&self` (never
// `&mut self`). The wrapped `inner: Py<ThetaDataDxClient>` carries its
// own interior state; the pyclass shell is immutable. A future
// `&mut self` regression surfaces as a `cargo check` failure rather
// than slipping silently.
#[pyclass(module = "thetadatadx", name = "MddsClient", frozen)]
pub(crate) struct MddsClient {
    /// Hidden inner unified client. Opens MDDS gRPC + Nexus at
    /// `connect` time and lazily opens FPSS only on `start_streaming*`
    /// — neither of which we surface through this pyclass, so no FPSS
    /// TLS slot is ever opened for a session that lives entirely
    /// through `MddsClient`.
    inner: Py<ThetaDataDxClient>,
}

#[pymethods]
impl MddsClient {
    /// Connect to ThetaData and open the MDDS gRPC channel.
    ///
    /// Authenticates against Nexus once and opens the in-house gRPC
    /// channel pool — same first-step behaviour as
    /// [`crate::ThetaDataDxClient`] but the FPSS streaming slot is
    /// never entered. A parallel FPSS process running under the same
    /// credentials is unaffected by this constructor's authentication
    /// (the Nexus-side parallel-session behaviour is the user's
    /// environment concern; see the module-level docstring).
    #[new]
    fn new(py: Python<'_>, creds: &Credentials, config: &Config) -> PyResult<Self> {
        let inner = Py::new(py, ThetaDataDxClient::new(py, creds, config)?)?;
        Ok(Self { inner })
    }

    /// Convenience constructor: `MddsClient.from_file("creds.txt")`.
    /// Loads credentials from a two-line file and connects with the
    /// supplied `config`, defaulting to `Config.production()`.
    ///
    /// P6 closure: the `config` kwarg is optional. The historical
    /// behaviour (no kwarg = production endpoint) is preserved; tests
    /// and dev / stage environments now reach a single-arg constructor
    /// shape via `MddsClient.from_file("creds.txt", config=Config.dev())`.
    #[staticmethod]
    #[pyo3(signature = (path, config=None))]
    fn from_file(py: Python<'_>, path: &str, config: Option<&Config>) -> PyResult<Self> {
        let creds = Credentials::from_file(path)?;
        let owned_default;
        let cfg = match config {
            Some(c) => c,
            None => {
                owned_default = Config::production();
                &owned_default
            }
        };
        let inner = Py::new(py, ThetaDataDxClient::new(py, &creds, cfg)?)?;
        Ok(Self { inner })
    }

    /// Forward unknown attribute access to the wrapped
    /// [`crate::ThetaDataDxClient`].
    ///
    /// Block-list applied first: every FPSS-touching method raises
    /// `AttributeError` so an MDDS-only handle cannot accidentally
    /// race a parallel FPSS process. Everything else (historical
    /// endpoints, FLATFILES, snapshot / list / at-time builders,
    /// `flat_files` namespace) reaches the unified client transparently.
    fn __getattr__(&self, py: Python<'_>, name: &str) -> PyResult<Py<PyAny>> {
        if FPSS_TOUCHING_METHODS.contains(&name) {
            return Err(PyAttributeError::new_err(format!(
                "MddsClient is the standalone historical surface and does not expose `{name}`. \
                 Use FpssClient(creds, config) for FPSS streaming, or ThetaDataDxClient(creds, config) \
                 for the unified handle."
            )));
        }
        let bound = self.inner.bind(py);
        Ok(bound.getattr(name)?.unbind())
    }

    fn __repr__(&self) -> String {
        // Drop the inherited-then-rewritten `streaming=` slot. MDDS-only
        // surface never opens the FPSS TLS transport, so reporting a
        // streaming state at all is misleading. The historical channel
        // is always connected by construction (the constructor errored
        // out otherwise).
        "MddsClient(historical=connected)".to_string()
    }
}

/// Introspection helper exposed as a module-level Python function for
/// the offline block-list coverage test in
/// `tests/test_standalone_clients.py`. Mirrors the Rust
/// [`FPSS_TOUCHING_METHODS`] const so a regression that quietly trims
/// the block-list fails the Python test even when no live credentials
/// are configured.
///
/// The leading underscore marks it as private; production callers
/// have no reason to read this list.
#[pyfunction]
#[pyo3(name = "_blocked_fpss_methods")]
pub(crate) fn blocked_fpss_methods() -> Vec<&'static str> {
    FPSS_TOUCHING_METHODS.to_vec()
}
