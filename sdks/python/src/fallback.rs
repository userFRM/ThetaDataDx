//! REST fallback policy + the four `_with_fallback` methods on
//! `ThetaDataDxClient`.
//!
//! See [`thetadatadx::config::FallbackPolicy`] for the underlying
//! contract; this module wraps it as a Python-side `FallbackPolicy`
//! pyclass with the four named constructors mirroring the Rust enum
//! variants, then surfaces the four `option_history_*_with_fallback`
//! methods on the Python `ThetaDataDxClient`.
//!
//! The shims are intentionally simple wrappers around the Rust core
//! -- no per-binding logic, no extra retry envelope. The Rust core
//! owns the policy decision; the Python layer only translates the
//! Python arg shape (Optional[str] / Optional[int]) into the Rust
//! one.

use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;

use thetadatadx::config;

/// Python-side mirror of [`thetadatadx::config::FallbackPolicy`].
///
/// `#[pyclass(frozen)]` so a constructed policy is observably
/// immutable from Python -- callers either build a new instance via
/// the static constructors or rely on `Config.with_rest_fallback(...)`
/// to swap policies on a `Config`. Mirrors the Rust enum's
/// `#[non_exhaustive]` shape: callers cannot construct the underlying
/// enum from the outside, only through the named factories.
///
/// # Examples
///
/// ```python
/// from thetadatadx import FallbackPolicy, Config, DEFAULT_REST_BASE_URL
///
/// # Disabled -- no REST fallback.
/// policy = FallbackPolicy.disabled()
///
/// # Fall back only on h2-disconnect (the issue #571 signature).
/// policy = FallbackPolicy.rest_on_h2_disconnect(DEFAULT_REST_BASE_URL)
///
/// # Pre-route every request before 2023-01-01 to REST.
/// policy = FallbackPolicy.rest_always_for_date_range(
///     DEFAULT_REST_BASE_URL, before=20230101
/// )
///
/// # Always REST.
/// policy = FallbackPolicy.rest_always(DEFAULT_REST_BASE_URL)
///
/// cfg = Config.production()
/// cfg.with_rest_fallback(policy)
/// ```
#[pyclass(module = "thetadatadx", frozen, skip_from_py_object)]
#[derive(Clone)]
pub(crate) struct FallbackPolicy {
    pub(crate) inner: config::FallbackPolicy,
}

#[pymethods]
impl FallbackPolicy {
    /// REST fallback disabled -- every request goes over gRPC.
    /// Default state; identical to constructing a `Config` without
    /// calling `with_rest_fallback`.
    #[staticmethod]
    fn disabled() -> Self {
        Self {
            inner: config::FallbackPolicy::Disabled,
        }
    }

    /// Fall back to REST only when gRPC returns the
    /// `TransportErrorKind::ConnectionClosed` signature (the
    /// issue #571 h2 cascade).
    ///
    /// Cheaper than the always-REST variants for workloads where the
    /// gRPC path is the fast common case; pays one failed gRPC
    /// round trip per affected request.
    #[staticmethod]
    fn rest_on_h2_disconnect(base_url: String) -> Self {
        Self {
            inner: config::FallbackPolicy::RestOnH2Disconnect { base_url },
        }
    }

    /// Route every request whose `start_date` is strictly before
    /// `before` (`YYYYMMDD` integer) directly to REST without trying
    /// gRPC first. Requests on or after `before` flow through gRPC.
    ///
    /// Use when the caller knows the symbol / date range is squarely
    /// inside the 2022-era legacy-row window.
    #[staticmethod]
    fn rest_always_for_date_range(base_url: String, before: i32) -> Self {
        Self {
            inner: config::FallbackPolicy::RestAlwaysForDateRange { base_url, before },
        }
    }

    /// Always route the four affected endpoints over REST regardless
    /// of the requested date range.
    #[staticmethod]
    fn rest_always(base_url: String) -> Self {
        Self {
            inner: config::FallbackPolicy::RestAlways { base_url },
        }
    }

    /// Return the REST base URL the policy would target on a
    /// fallback, or `None` for `disabled()`.
    #[getter]
    fn base_url(&self) -> Option<&str> {
        self.inner.base_url()
    }

    /// Human-readable variant name. The four current returns are
    /// `"Disabled"`, `"RestOnH2Disconnect"`, `"RestAlwaysForDateRange"`,
    /// `"RestAlways"`. The Rust enum is `#[non_exhaustive]`, so a
    /// future variant returns `"Unknown"` here until the binding is
    /// updated.
    #[getter]
    fn variant(&self) -> &'static str {
        variant_label(&self.inner)
    }

    fn __repr__(&self) -> String {
        match &self.inner {
            config::FallbackPolicy::Disabled => "FallbackPolicy.disabled()".to_string(),
            config::FallbackPolicy::RestOnH2Disconnect { base_url } => {
                format!("FallbackPolicy.rest_on_h2_disconnect({base_url:?})")
            }
            config::FallbackPolicy::RestAlwaysForDateRange { base_url, before } => format!(
                "FallbackPolicy.rest_always_for_date_range({base_url:?}, before={before})"
            ),
            config::FallbackPolicy::RestAlways { base_url } => {
                format!("FallbackPolicy.rest_always({base_url:?})")
            }
            _ => "FallbackPolicy(<unknown variant>)".to_string(),
        }
    }
}

/// Map a [`config::FallbackPolicy`] to the variant-label string the
/// Python pyclass exposes via the `variant` getter. Centralized so the
/// `Config.fallback_variant` getter on `lib.rs` can re-use the same
/// mapping without dispatching back through the pyclass.
pub(crate) fn variant_label(p: &config::FallbackPolicy) -> &'static str {
    match p {
        config::FallbackPolicy::Disabled => "Disabled",
        config::FallbackPolicy::RestOnH2Disconnect { .. } => "RestOnH2Disconnect",
        config::FallbackPolicy::RestAlwaysForDateRange { .. } => "RestAlwaysForDateRange",
        config::FallbackPolicy::RestAlways { .. } => "RestAlways",
        _ => "Unknown",
    }
}

/// Default base URL for the local Terminal's REST surface. Mirrors
/// `thetadatadx::config::DEFAULT_REST_BASE_URL`.
pub(crate) const DEFAULT_REST_BASE_URL: &str = config::DEFAULT_REST_BASE_URL;

/// Register the `FallbackPolicy` class + `DEFAULT_REST_BASE_URL`
/// module constant on the parent `thetadatadx` module.
pub(crate) fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<FallbackPolicy>()?;
    m.add("DEFAULT_REST_BASE_URL", DEFAULT_REST_BASE_URL)?;
    Ok(())
}

/// Helper used by `Config::with_rest_fallback` to validate the
/// argument shape before swapping the policy on the underlying
/// `DirectConfig`.
pub(crate) fn validate_policy_argument(p: &FallbackPolicy) -> PyResult<config::FallbackPolicy> {
    Ok(p.inner.clone())
}

/// Validate a Python-supplied `YYYYMMDD` date string before forwarding
/// to the Rust shim. Mirrors `crate::client::parse_yyyymmdd`'s contract
/// on the Python boundary so the error surfaces as `ValueError` instead
/// of bubbling out of the Rust shim as a generic transport error.
pub(crate) fn validate_yyyymmdd(field: &'static str, date: &str) -> PyResult<()> {
    match date.parse::<i32>() {
        Ok(d) if (10_000_000..100_000_000).contains(&d) => Ok(()),
        _ => Err(PyValueError::new_err(format!(
            "{field}: expected YYYYMMDD, got {date:?}"
        ))),
    }
}
