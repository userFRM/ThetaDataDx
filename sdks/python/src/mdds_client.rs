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
const FPSS_TOUCHING_METHODS: &[&str] = &[
    "start_streaming",
    "start_streaming_iter",
    "stop_streaming",
    "shutdown",
    "reconnect",
    "streaming",
    "streaming_iter",
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
];

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
#[pyclass(module = "thetadatadx", name = "MddsClient")]
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
    /// Loads credentials from a two-line file and connects with
    /// production defaults.
    #[staticmethod]
    fn from_file(py: Python<'_>, path: &str) -> PyResult<Self> {
        let creds = Credentials::from_file(path)?;
        let config = Config::production();
        let inner = Py::new(py, ThetaDataDxClient::new(py, &creds, &config)?)?;
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

    fn __repr__(&self, py: Python<'_>) -> PyResult<String> {
        let bound = self.inner.bind(py);
        let inner_repr: String = bound.call_method0("__repr__")?.extract()?;
        Ok(inner_repr.replace("ThetaDataDxClient", "MddsClient"))
    }
}
