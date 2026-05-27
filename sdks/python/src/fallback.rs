//! REST-routing policy + the four `_with_fallback` methods on
//! `ThetaDataDxClient`.
//!
//! See [`thetadatadx::config::FallbackPolicy`] for the underlying
//! contract; this module wraps it as a Python-side `FallbackPolicy`
//! pyclass with the named constructors mirroring the Rust enum
//! variants, then surfaces the four `option_history_*_with_fallback`
//! methods on the Python `ThetaDataDxClient`.
//!
//! The shims are intentionally simple wrappers around the Rust core
//! -- no per-binding logic, no extra retry envelope. The Rust core
//! owns the policy decision; the Python layer only translates the
//! Python arg shape (Optional[str]) into the Rust one.

use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;

use thetadatadx::config;

/// Python-side mirror of [`thetadatadx::config::FallbackPolicy`].
///
/// Pyclass is `frozen` so a constructed policy is observably
/// immutable from Python -- callers either build a new instance via
/// the static constructors or rely on `Config.with_rest_fallback(...)`
/// to swap policies on a `Config`. Mirrors the Rust enum's
/// non-exhaustive shape: callers cannot construct the underlying
/// variant from the outside, only through the named factories.
///
/// # Examples
///
/// ```python
/// from thetadatadx import FallbackPolicy, Config, DEFAULT_REST_BASE_URL
///
/// # Disabled -- gRPC for every endpoint.
/// policy = FallbackPolicy.disabled()
///
/// # Always REST -- single transport for every historical-quote call.
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
    /// REST routing disabled -- every request goes over gRPC.
    /// Default state; identical to constructing a `Config` without
    /// calling `with_rest_fallback`.
    #[staticmethod]
    fn disabled() -> Self {
        Self {
            inner: config::FallbackPolicy::Disabled,
        }
    }

    /// Always route the four historical-quote endpoints over REST
    /// regardless of the requested date range.
    #[staticmethod]
    fn rest_always(base_url: String) -> Self {
        Self {
            inner: config::FallbackPolicy::RestAlways { base_url },
        }
    }

    /// Return the REST base URL the policy would target, or `None`
    /// for `disabled()`.
    #[getter]
    fn base_url(&self) -> Option<&str> {
        self.inner.base_url()
    }

    /// Human-readable variant name. The two current returns are
    /// `"Disabled"` and `"RestAlways"`. The Rust enum is
    /// `#[non_exhaustive]`, so a future variant returns `"Unknown"`
    /// here until the binding is updated.
    #[getter]
    fn variant(&self) -> &'static str {
        variant_label(&self.inner)
    }

    fn __repr__(&self) -> String {
        match &self.inner {
            config::FallbackPolicy::Disabled => "FallbackPolicy.disabled()".to_string(),
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
/// to the Rust shim. Mirrors the historical-quote builder's contract
/// on the Python boundary so a malformed date surfaces as `ValueError`
/// instead of bubbling out of the Rust shim as a generic transport
/// error.
pub(crate) fn validate_yyyymmdd(field: &'static str, date: &str) -> PyResult<()> {
    match date.parse::<i32>() {
        Ok(d) if (10_000_000..100_000_000).contains(&d) => Ok(()),
        _ => Err(PyValueError::new_err(format!(
            "{field}: expected YYYYMMDD, got {date:?}"
        ))),
    }
}
