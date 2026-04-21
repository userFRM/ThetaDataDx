//! Layered Python exception hierarchy for `thetadatadx`.
//!
//! Before v8.0.2 every error path mapped to the generic `PyRuntimeError` or
//! `PyConnectionError` — callers had to parse the error message to
//! distinguish authentication failures from rate limiting or transient
//! network errors. This module introduces a typed hierarchy so user code
//! can `except` on the class it actually cares about.
//!
//! ```python
//! try:
//!     tdx.stock_history_eod("AAPL", "20240101", "20240301")
//! except thetadatadx.InvalidCredentialsError:
//!     refresh_session()
//! except thetadatadx.RateLimitError:
//!     time.sleep(120)
//! except thetadatadx.ThetaDataError:
//!     ...  # catch-all for the entire hierarchy
//! ```
//!
//! # Mapping
//!
//! The Rust `thetadatadx::Error` variants each map to exactly one leaf
//! exception. The map lives in [`to_py_err`]; adding a new variant at the
//! Rust SDK layer is picked up here via the `non_exhaustive` catch-all so
//! the build never breaks on upstream additions.
//!
//! The `NoDataFoundError` class name matches the upstream vendor's Python
//! SDK (Apache-2.0) by design — users porting from `thetadata` to
//! `thetadatadx` keep the same `except` clause.

use pyo3::create_exception;
use pyo3::exceptions::PyException;
use pyo3::prelude::*;

// Root — callers can use a single `except thetadatadx.ThetaDataError` to
// catch any branded error from the SDK. Intentionally inherits from
// `PyException`, not `OSError`, so it composes cleanly with asyncio /
// contextlib handlers.
create_exception!(thetadatadx, ThetaDataError, PyException);

// Authentication family. `AuthenticationError` is the parent of
// `InvalidCredentialsError` so user code can handle "any auth problem"
// with one clause.
create_exception!(thetadatadx, AuthenticationError, ThetaDataError);
create_exception!(thetadatadx, InvalidCredentialsError, AuthenticationError);

// Subscription / tier / plan restrictions returned as gRPC
// `PermissionDenied`.
create_exception!(thetadatadx, SubscriptionError, ThetaDataError);

// Rate limiting: gRPC `ResourceExhausted` / `TooManyRequests` / HTTP 429.
create_exception!(thetadatadx, RateLimitError, ThetaDataError);

// Decoder schema mismatch — surfaces `Error::Decode` + `DecodeError`
// variants. Triggering cause is usually a proto bump on the server before
// the SDK is refreshed.
create_exception!(thetadatadx, SchemaMismatchError, ThetaDataError);

// Transport — TCP / gRPC / TLS failures.
create_exception!(thetadatadx, NetworkError, ThetaDataError);

// Per-request deadline (`with_deadline(d)` / `timeout_ms` kwarg).
// Intentionally NOT inheriting from the stdlib builtins.TimeoutError — the
// v8.0.1 behavior mapped to `PyTimeoutError` but that conflates user-
// imposed deadlines with OS-level socket timeouts. Callers who want the
// stdlib behaviour can `except (thetadatadx.TimeoutError, TimeoutError)`.
// The local name deliberately shadows the stdlib name inside the
// `thetadatadx` namespace, matching the vendor SDK convention.
create_exception!(thetadatadx, TimeoutError, ThetaDataError);

// Empty result — gRPC `NotFound`. Class name matches the upstream vendor
// SDK's `NoDataFoundError` so users porting from `thetadata` to
// `thetadatadx` keep the same `except` clause.
create_exception!(thetadatadx, NoDataFoundError, ThetaDataError);

// FPSS streaming protocol / state-machine failures.
create_exception!(thetadatadx, StreamError, ThetaDataError);

/// Register every exception class on the module. Called from
/// `thetadatadx_py` at module init time.
pub fn register_exceptions(py: Python<'_>, m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add("ThetaDataError", py.get_type::<ThetaDataError>())?;
    m.add("AuthenticationError", py.get_type::<AuthenticationError>())?;
    m.add(
        "InvalidCredentialsError",
        py.get_type::<InvalidCredentialsError>(),
    )?;
    m.add("SubscriptionError", py.get_type::<SubscriptionError>())?;
    m.add("RateLimitError", py.get_type::<RateLimitError>())?;
    m.add("SchemaMismatchError", py.get_type::<SchemaMismatchError>())?;
    m.add("NetworkError", py.get_type::<NetworkError>())?;
    m.add("TimeoutError", py.get_type::<TimeoutError>())?;
    m.add("NoDataFoundError", py.get_type::<NoDataFoundError>())?;
    m.add("StreamError", py.get_type::<StreamError>())?;
    Ok(())
}

/// Map a `thetadatadx::Error` into the closest Python exception class.
///
/// The mapping is deliberately narrow: every variant routes to one
/// concrete leaf class. The `#[non_exhaustive]` catch-all routes unknown
/// future variants to `ThetaDataError` so upstream SDK additions never
/// break the Python wheel build.
pub fn to_py_err(e: thetadatadx::Error) -> PyErr {
    use thetadatadx::error::{AuthErrorKind, FpssErrorKind};

    match &e {
        thetadatadx::Error::Auth { kind, .. } => match kind {
            AuthErrorKind::InvalidCredentials => InvalidCredentialsError::new_err(e.to_string()),
            AuthErrorKind::NetworkError => NetworkError::new_err(e.to_string()),
            AuthErrorKind::Timeout => TimeoutError::new_err(e.to_string()),
            AuthErrorKind::ServerError => AuthenticationError::new_err(e.to_string()),
            // Future `AuthErrorKind` variants land on the parent family.
            _ => AuthenticationError::new_err(e.to_string()),
        },
        thetadatadx::Error::Grpc { status, .. } => match status.as_str() {
            "PermissionDenied" => SubscriptionError::new_err(e.to_string()),
            "ResourceExhausted" => RateLimitError::new_err(e.to_string()),
            "NotFound" => NoDataFoundError::new_err(e.to_string()),
            "DeadlineExceeded" => TimeoutError::new_err(e.to_string()),
            "Unauthenticated" => AuthenticationError::new_err(e.to_string()),
            "Unavailable" => NetworkError::new_err(e.to_string()),
            _ => ThetaDataError::new_err(e.to_string()),
        },
        thetadatadx::Error::NoData => NoDataFoundError::new_err(e.to_string()),
        thetadatadx::Error::Timeout { .. } => TimeoutError::new_err(e.to_string()),
        thetadatadx::Error::Transport(_) => NetworkError::new_err(e.to_string()),
        thetadatadx::Error::Tls(_) => NetworkError::new_err(e.to_string()),
        thetadatadx::Error::Io(_) => NetworkError::new_err(e.to_string()),
        thetadatadx::Error::Http(_) => NetworkError::new_err(e.to_string()),
        thetadatadx::Error::Decode(_) => SchemaMismatchError::new_err(e.to_string()),
        thetadatadx::Error::Decompress(_) => SchemaMismatchError::new_err(e.to_string()),
        thetadatadx::Error::Config(_) => ThetaDataError::new_err(e.to_string()),
        thetadatadx::Error::Fpss { kind, .. } => match kind {
            FpssErrorKind::TooManyRequests => RateLimitError::new_err(e.to_string()),
            FpssErrorKind::Timeout => TimeoutError::new_err(e.to_string()),
            FpssErrorKind::ConnectionRefused | FpssErrorKind::Disconnected => {
                NetworkError::new_err(e.to_string())
            }
            FpssErrorKind::ProtocolError => StreamError::new_err(e.to_string()),
            _ => StreamError::new_err(e.to_string()),
        },
        // Catch-all for future `#[non_exhaustive]` variants added on the
        // Rust side. We keep the build green and route to the root class
        // so the caller still sees a branded exception rather than an
        // opaque `Exception`.
        _ => ThetaDataError::new_err(e.to_string()),
    }
}

#[cfg(test)]
mod tests {
    //! Dispatch-table tests for `to_py_err`. We verify that each Rust
    //! `Error` variant lands on the expected Python leaf class. The
    //! tests run under `pyo3::Python::with_gil` so exception classes are
    //! instantiable.

    use super::*;
    use thetadatadx::error::{AuthErrorKind, FpssErrorKind};

    /// Helper: check that `err` is an instance of the named Python
    /// exception class. Equivalent to `isinstance(err, cls)` in Python.
    fn assert_exception_class(py: Python<'_>, err: &PyErr, expected_name: &str) {
        let type_obj = err.get_type(py);
        let name: String = type_obj
            .qualname()
            .and_then(|q| q.extract::<String>())
            .expect("every pyo3 exception class has a qualname");
        assert_eq!(
            name, expected_name,
            "expected Python class {expected_name}, got {name}"
        );
    }

    #[test]
    fn auth_invalid_credentials_maps_to_invalid_credentials_error() {
        Python::initialize();
        Python::attach(|py| {
            let err = to_py_err(thetadatadx::Error::Auth {
                kind: AuthErrorKind::InvalidCredentials,
                message: "wrong password".into(),
            });
            assert_exception_class(py, &err, "InvalidCredentialsError");
        });
    }

    #[test]
    fn auth_network_error_maps_to_network_error() {
        Python::initialize();
        Python::attach(|py| {
            let err = to_py_err(thetadatadx::Error::Auth {
                kind: AuthErrorKind::NetworkError,
                message: "DNS lookup failed".into(),
            });
            assert_exception_class(py, &err, "NetworkError");
        });
    }

    #[test]
    fn auth_timeout_maps_to_timeout_error() {
        Python::initialize();
        Python::attach(|py| {
            let err = to_py_err(thetadatadx::Error::Auth {
                kind: AuthErrorKind::Timeout,
                message: "auth timed out".into(),
            });
            assert_exception_class(py, &err, "TimeoutError");
        });
    }

    #[test]
    fn grpc_permission_denied_maps_to_subscription_error() {
        Python::initialize();
        Python::attach(|py| {
            let err = to_py_err(thetadatadx::Error::Grpc {
                status: "PermissionDenied".into(),
                message: "tier insufficient".into(),
            });
            assert_exception_class(py, &err, "SubscriptionError");
        });
    }

    #[test]
    fn grpc_resource_exhausted_maps_to_rate_limit() {
        Python::initialize();
        Python::attach(|py| {
            let err = to_py_err(thetadatadx::Error::Grpc {
                status: "ResourceExhausted".into(),
                message: "429".into(),
            });
            assert_exception_class(py, &err, "RateLimitError");
        });
    }

    #[test]
    fn grpc_not_found_maps_to_no_data_found() {
        Python::initialize();
        Python::attach(|py| {
            let err = to_py_err(thetadatadx::Error::Grpc {
                status: "NotFound".into(),
                message: "no rows".into(),
            });
            assert_exception_class(py, &err, "NoDataFoundError");
        });
    }

    #[test]
    fn nodata_error_maps_to_no_data_found() {
        Python::initialize();
        Python::attach(|py| {
            let err = to_py_err(thetadatadx::Error::NoData);
            assert_exception_class(py, &err, "NoDataFoundError");
        });
    }

    #[test]
    fn timeout_variant_maps_to_timeout_error() {
        Python::initialize();
        Python::attach(|py| {
            let err = to_py_err(thetadatadx::Error::Timeout { duration_ms: 500 });
            assert_exception_class(py, &err, "TimeoutError");
        });
    }

    #[test]
    fn decode_error_maps_to_schema_mismatch() {
        Python::initialize();
        Python::attach(|py| {
            let err = to_py_err(thetadatadx::Error::Decode("cell type mismatch".into()));
            assert_exception_class(py, &err, "SchemaMismatchError");
        });
    }

    #[test]
    fn fpss_too_many_requests_maps_to_rate_limit() {
        Python::initialize();
        Python::attach(|py| {
            let err = to_py_err(thetadatadx::Error::Fpss {
                kind: FpssErrorKind::TooManyRequests,
                message: "back off".into(),
            });
            assert_exception_class(py, &err, "RateLimitError");
        });
    }

    #[test]
    fn fpss_protocol_error_maps_to_stream_error() {
        Python::initialize();
        Python::attach(|py| {
            let err = to_py_err(thetadatadx::Error::Fpss {
                kind: FpssErrorKind::ProtocolError,
                message: "bad frame".into(),
            });
            assert_exception_class(py, &err, "StreamError");
        });
    }

    #[test]
    fn config_error_maps_to_root_class() {
        Python::initialize();
        Python::attach(|py| {
            let err = to_py_err(thetadatadx::Error::Config("invalid URI".into()));
            assert_exception_class(py, &err, "ThetaDataError");
        });
    }

    #[test]
    fn exception_hierarchy_roots_at_theta_data_error() {
        // Every branded exception should `isinstance(ThetaDataError)`.
        // This guards against a future refactor accidentally dropping the
        // class-hierarchy parent, which would break `except
        // thetadatadx.ThetaDataError as e` catch-alls.
        Python::initialize();
        Python::attach(|py| {
            let root = py.get_type::<ThetaDataError>();
            for (name, cls_ty) in [
                ("AuthenticationError", py.get_type::<AuthenticationError>()),
                (
                    "InvalidCredentialsError",
                    py.get_type::<InvalidCredentialsError>(),
                ),
                ("SubscriptionError", py.get_type::<SubscriptionError>()),
                ("RateLimitError", py.get_type::<RateLimitError>()),
                ("SchemaMismatchError", py.get_type::<SchemaMismatchError>()),
                ("NetworkError", py.get_type::<NetworkError>()),
                ("TimeoutError", py.get_type::<TimeoutError>()),
                ("NoDataFoundError", py.get_type::<NoDataFoundError>()),
                ("StreamError", py.get_type::<StreamError>()),
            ] {
                let is_sub = cls_ty
                    .is_subclass(&root)
                    .expect("is_subclass should succeed with GIL held");
                assert!(
                    is_sub,
                    "{name} is not a subclass of ThetaDataError — broken hierarchy"
                );
            }
        });
    }

    #[test]
    fn invalid_credentials_is_subclass_of_authentication_error() {
        // InvalidCredentialsError → AuthenticationError → ThetaDataError.
        // Catch clause semantics depend on this transitive relationship.
        Python::initialize();
        Python::attach(|py| {
            let auth = py.get_type::<AuthenticationError>();
            let inv = py.get_type::<InvalidCredentialsError>();
            let is_sub = inv
                .is_subclass(&auth)
                .expect("is_subclass should succeed with GIL held");
            assert!(
                is_sub,
                "InvalidCredentialsError must inherit from AuthenticationError"
            );
        });
    }
}
