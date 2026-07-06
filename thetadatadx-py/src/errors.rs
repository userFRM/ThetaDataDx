//! Layered Python exception hierarchy for `thetadatadx`.
//!
//! A typed exception hierarchy lets user code `except` on the class it
//! actually cares about — distinguishing authentication failures from
//! rate limiting or transient network errors — rather than parsing the
//! message text off a generic `PyRuntimeError` / `PyConnectionError`.
//!
//! ```python
//! try:
//!     client.market_data.stock_history_eod("AAPL", "20240101", "20240301")
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
//! # Canonical leaf vocabulary
//!
//! The canonical leaf class names are identical across every binding
//! (Python, TypeScript, C++, and the C ABI discriminants): a `NotFound`
//! status raises `NotFoundError`, an expired deadline raises
//! `DeadlineExceededError`, and an `Unavailable` status raises
//! `UnavailableError`. Porting an `except` clause from one binding to
//! another keeps the same class name.
//!
//! `NoDataFoundError` and `TimeoutError` remain as documented
//! back-compatibility aliases: each is registered as the same type
//! object as its canonical replacement (`NotFoundError` /
//! `DeadlineExceededError`), so existing
//! `except thetadatadx.NoDataFoundError` / `except thetadatadx.TimeoutError`
//! clauses keep catching the same conditions. New code should reach for
//! the canonical names.

use std::ffi::CString;

use pyo3::create_exception;
use pyo3::exceptions::{PyException, PyValueError};
use pyo3::prelude::*;
use pyo3::sync::PyOnceLock;
use pyo3::types::{PyTuple, PyType};

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
// Carries a `retry_after` attribute (float seconds, or `None`) read from
// the upstream `google.rpc.RetryInfo` detail when the server attached
// one, so callers can honour a server-instructed back-off as a value.
create_exception!(thetadatadx, RateLimitError, ThetaDataError);

// Client-side input validation rejected a parameter (a bad value, an
// out-of-range number, a missing required field). Distinct from
// `ConfigError` so a malformed-but-rejected argument is distinguishable
// by class from an unrelated environmental configuration fault
// (config-file I/O, TOML parse, internal invariant), which raises
// `ConfigError`.
// `InvalidParameterError` needs two bases: the SDK root `ThetaDataError`
// (so `except ThetaDataError` catches it and it stays in the branded
// hierarchy) AND the built-in `ValueError` (so existing `except ValueError`
// callers keep working — a rejected argument is a value error). The
// single-base `create_exception!` macro cannot express two bases, so the
// type is built once at module registration with a `(ThetaDataError,
// ValueError)` bases tuple and cached here.
static INVALID_PARAMETER_ERROR: PyOnceLock<Py<PyType>> = PyOnceLock::new();

fn build_invalid_parameter_error(py: Python<'_>) -> PyResult<Py<PyType>> {
    let bases = PyTuple::new(
        py,
        [
            py.get_type::<ThetaDataError>().into_any(),
            py.get_type::<PyValueError>().into_any(),
        ],
    )?;
    let name =
        CString::new("thetadatadx.InvalidParameterError").expect("static name has no interior NUL");
    // SAFETY: `name` is a valid C string, `bases` is a non-null tuple of
    // exception types, and the null `dict` is accepted by the C-API. The
    // returned object is a new reference to a new exception type.
    let ptr = unsafe {
        pyo3::ffi::PyErr_NewException(name.as_ptr(), bases.as_ptr(), std::ptr::null_mut())
    };
    let obj = unsafe { Bound::from_owned_ptr_or_err(py, ptr)? };
    Ok(obj.cast_into::<PyType>()?.unbind())
}

/// The cached `InvalidParameterError` type (dual base: `ThetaDataError` +
/// `ValueError`), building it on first use.
pub fn invalid_parameter_type(py: Python<'_>) -> PyResult<&'static Py<PyType>> {
    INVALID_PARAMETER_ERROR.get_or_try_init(py, || build_invalid_parameter_error(py))
}

/// Raise `InvalidParameterError` with `msg`. Callers catching either
/// `ValueError` or `thetadatadx.ThetaDataError` both handle it.
pub fn invalid_parameter_err(msg: impl Into<String>) -> PyErr {
    let msg = msg.into();
    Python::attach(|py| match invalid_parameter_type(py) {
        Ok(ty) => PyErr::from_type(ty.bind(py).clone(), (msg,)),
        // Fall back to a plain ValueError if the type could not be built
        // (only reachable before the module is initialised).
        Err(_) => PyValueError::new_err(msg),
    })
}

// Decoder schema mismatch — surfaces `Error::Decode` + `DecodeError`
// variants. Triggering cause is usually a proto bump on the server before
// the SDK is refreshed.
create_exception!(thetadatadx, SchemaMismatchError, ThetaDataError);

// Transport — TCP / TLS / IO failures other than an explicit
// `Unavailable` status (which routes to `UnavailableError`).
create_exception!(thetadatadx, NetworkError, ThetaDataError);

// Upstream unavailable — gRPC `Unavailable`. Split out from
// `NetworkError` so the canonical leaf set matches the other bindings:
// an `Unavailable` status raises `UnavailableError` in Python,
// TypeScript, and C++ alike.
create_exception!(thetadatadx, UnavailableError, ThetaDataError);

// Per-request deadline (`with_deadline(d)` / `timeout_ms` kwarg) and
// upstream `DeadlineExceeded`. Intentionally NOT inheriting from the
// stdlib `builtins.TimeoutError` — a user-imposed deadline is not an
// OS-level socket timeout. Callers who want the stdlib behaviour can
// `except (thetadatadx.DeadlineExceededError, TimeoutError)`.
create_exception!(thetadatadx, DeadlineExceededError, ThetaDataError);

// Empty result — gRPC `NotFound`. The canonical name matches the other
// bindings (`NotFoundError` in TypeScript / C++ / the C ABI codes).
create_exception!(thetadatadx, NotFoundError, ThetaDataError);

// Streaming protocol / state-machine failures.
create_exception!(thetadatadx, StreamError, ThetaDataError);

// Environmental configuration fault — a config-file read failure, a
// TOML parse error, or an internal config invariant. Distinct from
// `InvalidParameterError` (a rejected user-supplied argument): a
// `ConfigError` is the environment, not the call site. Pinned to the
// reserved `THETADATADX_ERR_CONFIG` discriminant so a `except
// thetadatadx.ConfigError` clause catches the same conditions the C++
// `ConfigError` and the C ABI config code surface.
create_exception!(thetadatadx, ConfigError, ThetaDataError);

// ── Back-compatibility aliases ────────────────────────────────────────
//
// `NoDataFoundError` and `TimeoutError` predate the cross-binding
// vocabulary unification. They are registered in [`register_exceptions`]
// as true assignment aliases of their canonical replacements
// (`NotFoundError` / `DeadlineExceededError`) — the same type object is
// added to the module under both names, so
// `thetadatadx.NoDataFoundError is thetadatadx.NotFoundError` holds and an
// existing `except thetadatadx.NoDataFoundError` clause keeps catching
// the conditions the dispatch raises. No separate subclass is created:
// an alias must share identity with its target to preserve the `except`
// semantics. New code should use the canonical names.

/// Register every exception class on the module. Called from
/// `thetadatadx_py` at module init time.
///
/// `NoDataFoundError` / `TimeoutError` are registered as assignment
/// aliases: the canonical `NotFoundError` / `DeadlineExceededError` type
/// objects are added under both names so the alias shares identity with
/// its target and an existing `except` clause on the legacy name keeps
/// catching the dispatched canonical class.
///
/// # Errors
///
/// Propagates any `PyErr` from adding a class to the module or seating
/// the `RateLimitError.retry_after` class attribute.
pub fn register_exceptions(py: Python<'_>, m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add("ThetaDataError", py.get_type::<ThetaDataError>())?;
    m.add("AuthenticationError", py.get_type::<AuthenticationError>())?;
    m.add(
        "InvalidCredentialsError",
        py.get_type::<InvalidCredentialsError>(),
    )?;
    m.add("SubscriptionError", py.get_type::<SubscriptionError>())?;
    let rate_limit_type = py.get_type::<RateLimitError>();
    // Seat a class-level `retry_after` default so the attribute is
    // present (as `None`) on every `RateLimitError` even before the
    // dispatch sets a per-instance value from a server `RetryInfo` hint.
    rate_limit_type.setattr("retry_after", py.None())?;
    m.add("RateLimitError", rate_limit_type)?;
    m.add(
        "InvalidParameterError",
        invalid_parameter_type(py)?.bind(py).clone(),
    )?;
    m.add("SchemaMismatchError", py.get_type::<SchemaMismatchError>())?;
    m.add("NetworkError", py.get_type::<NetworkError>())?;
    m.add("UnavailableError", py.get_type::<UnavailableError>())?;
    m.add(
        "DeadlineExceededError",
        py.get_type::<DeadlineExceededError>(),
    )?;
    m.add("NotFoundError", py.get_type::<NotFoundError>())?;
    m.add("StreamError", py.get_type::<StreamError>())?;
    m.add("ConfigError", py.get_type::<ConfigError>())?;
    // Back-compatibility aliases: same type object under the legacy name.
    m.add("NoDataFoundError", py.get_type::<NotFoundError>())?;
    m.add("TimeoutError", py.get_type::<DeadlineExceededError>())?;
    Ok(())
}

/// Build a `RateLimitError` whose `retry_after` attribute carries the
/// server-supplied back-off (float seconds) when the upstream attached a
/// `google.rpc.RetryInfo` detail, or `None` otherwise.
///
/// The attribute is always present so callers can read
/// `err.retry_after` unconditionally; it is `None` when no hint was
/// supplied. `Python::attach` acquires the interpreter lock only to seat
/// the attribute and is a no-op when the caller already holds it.
fn rate_limit_err(e: &thetadatadx::Error) -> PyErr {
    let retry_after_secs = e.retry_after().map(|d| d.as_secs_f64());
    let err = RateLimitError::new_err(e.to_string());
    Python::attach(|py| {
        // Surfacing the back-off is best-effort metadata. Setting an
        // attribute on a pyo3 exception instance does not fail in
        // practice; if it ever did, the typed class is still correct, so
        // the result is intentionally discarded rather than masking the
        // rate-limit error with a setattr failure. `setattr` returns the
        // error by value without touching the interpreter error state,
        // so discarding it leaves no error set.
        let _ = err.value(py).setattr("retry_after", retry_after_secs);
    });
    err
}

/// Raise a `ConfigError` for a malformed client-construction argument.
///
/// Used by the inline client constructor when the authentication kwargs
/// conflict, are absent, or carry an unparseable `market_data_type` — a local,
/// pre-network configuration fault distinct from a server-side auth
/// rejection.
pub fn config_err(message: impl Into<String>) -> PyErr {
    ConfigError::new_err(message.into())
}

/// Map a `thetadatadx::Error` into the closest Python exception class.
///
/// The mapping is deliberately narrow: every variant routes to one
/// concrete leaf class. The `#[non_exhaustive]` catch-all routes unknown
/// future variants to `ThetaDataError` so upstream SDK additions never
/// break the Python wheel build.
pub fn to_py_err(e: thetadatadx::Error) -> PyErr {
    use thetadatadx::error::{AuthErrorKind, GrpcStatusKind, StreamErrorKind};

    match &e {
        thetadatadx::Error::Auth { kind, .. } => match kind {
            AuthErrorKind::InvalidCredentials => InvalidCredentialsError::new_err(e.to_string()),
            AuthErrorKind::NetworkError => NetworkError::new_err(e.to_string()),
            AuthErrorKind::Timeout => DeadlineExceededError::new_err(e.to_string()),
            AuthErrorKind::ServerError => AuthenticationError::new_err(e.to_string()),
            // Future `AuthErrorKind` variants land on the parent family.
            _ => AuthenticationError::new_err(e.to_string()),
        },
        thetadatadx::Error::Grpc { kind, .. } => match kind {
            GrpcStatusKind::PermissionDenied => SubscriptionError::new_err(e.to_string()),
            GrpcStatusKind::ResourceExhausted => rate_limit_err(&e),
            GrpcStatusKind::NotFound => NotFoundError::new_err(e.to_string()),
            GrpcStatusKind::DeadlineExceeded => DeadlineExceededError::new_err(e.to_string()),
            GrpcStatusKind::Unauthenticated => AuthenticationError::new_err(e.to_string()),
            GrpcStatusKind::Unavailable => UnavailableError::new_err(e.to_string()),
            _ => ThetaDataError::new_err(e.to_string()),
        },
        thetadatadx::Error::NoData => NotFoundError::new_err(e.to_string()),
        thetadatadx::Error::Timeout { .. } => DeadlineExceededError::new_err(e.to_string()),
        thetadatadx::Error::Transport { .. } => NetworkError::new_err(e.to_string()),
        thetadatadx::Error::Tls(_) => NetworkError::new_err(e.to_string()),
        thetadatadx::Error::Io(_) => NetworkError::new_err(e.to_string()),
        thetadatadx::Error::Http(_) => NetworkError::new_err(e.to_string()),
        thetadatadx::Error::Decode { .. } => SchemaMismatchError::new_err(e.to_string()),
        thetadatadx::Error::Decompress { .. } => SchemaMismatchError::new_err(e.to_string()),
        // User-input validation failures route to the dedicated
        // invalid-parameter class; environmental config faults
        // (`Io` / `TomlParse` / `Internal`) route to `ConfigError`.
        thetadatadx::Error::Config { kind, .. } => {
            if kind.is_invalid_parameter() {
                invalid_parameter_err(e.to_string())
            } else {
                ConfigError::new_err(e.to_string())
            }
        }
        thetadatadx::Error::Stream { kind, .. } => match kind {
            StreamErrorKind::TooManyRequests => rate_limit_err(&e),
            StreamErrorKind::Timeout => DeadlineExceededError::new_err(e.to_string()),
            StreamErrorKind::ConnectionRefused | StreamErrorKind::Disconnected => {
                NetworkError::new_err(e.to_string())
            }
            StreamErrorKind::ProtocolError => StreamError::new_err(e.to_string()),
            _ => StreamError::new_err(e.to_string()),
        },
        // FlatFiles availability + partial-reconnect failures are
        // streaming-surface faults; route them to `StreamError` so a
        // `except StreamError` clause behaves identically to the C++
        // and C ABI mapping (both pin these to the stream discriminant).
        thetadatadx::Error::FlatFilesUnavailable(_)
        | thetadatadx::Error::PartialReconnect { .. } => StreamError::new_err(e.to_string()),
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
    use thetadatadx::error::{AuthErrorKind, GrpcStatusKind, StreamErrorKind};

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
    fn auth_timeout_maps_to_deadline_exceeded_error() {
        Python::initialize();
        Python::attach(|py| {
            let err = to_py_err(thetadatadx::Error::Auth {
                kind: AuthErrorKind::Timeout,
                message: "auth timed out".into(),
            });
            assert_exception_class(py, &err, "DeadlineExceededError");
        });
    }

    #[test]
    fn grpc_permission_denied_maps_to_subscription_error() {
        Python::initialize();
        Python::attach(|py| {
            let err = to_py_err(thetadatadx::Error::Grpc {
                kind: GrpcStatusKind::PermissionDenied,
                message: "tier insufficient".into(),
                retry_after: None,
            });
            assert_exception_class(py, &err, "SubscriptionError");
        });
    }

    #[test]
    fn grpc_resource_exhausted_maps_to_rate_limit() {
        Python::initialize();
        Python::attach(|py| {
            let err = to_py_err(thetadatadx::Error::Grpc {
                kind: GrpcStatusKind::ResourceExhausted,
                message: "429".into(),
                retry_after: None,
            });
            assert_exception_class(py, &err, "RateLimitError");
        });
    }

    #[test]
    fn grpc_not_found_maps_to_not_found() {
        Python::initialize();
        Python::attach(|py| {
            let err = to_py_err(thetadatadx::Error::Grpc {
                kind: GrpcStatusKind::NotFound,
                message: "no rows".into(),
                retry_after: None,
            });
            assert_exception_class(py, &err, "NotFoundError");
        });
    }

    #[test]
    fn grpc_unavailable_maps_to_unavailable() {
        Python::initialize();
        Python::attach(|py| {
            let err = to_py_err(thetadatadx::Error::Grpc {
                kind: GrpcStatusKind::Unavailable,
                message: "backend down".into(),
                retry_after: None,
            });
            assert_exception_class(py, &err, "UnavailableError");
        });
    }

    #[test]
    fn nodata_error_maps_to_not_found() {
        Python::initialize();
        Python::attach(|py| {
            let err = to_py_err(thetadatadx::Error::NoData);
            assert_exception_class(py, &err, "NotFoundError");
        });
    }

    #[test]
    fn timeout_variant_maps_to_deadline_exceeded_error() {
        Python::initialize();
        Python::attach(|py| {
            let err = to_py_err(thetadatadx::Error::Timeout { duration_ms: 500 });
            assert_exception_class(py, &err, "DeadlineExceededError");
        });
    }

    #[test]
    fn rate_limit_carries_retry_after_attribute() {
        Python::initialize();
        Python::attach(|py| {
            // With a RetryInfo hint: the attribute is the float seconds.
            let err = to_py_err(thetadatadx::Error::Grpc {
                kind: GrpcStatusKind::ResourceExhausted,
                message: "429".into(),
                retry_after: Some(std::time::Duration::from_millis(1500)),
            });
            assert_exception_class(py, &err, "RateLimitError");
            let value = err.value(py);
            let secs: Option<f64> = value
                .getattr("retry_after")
                .expect("retry_after attribute is present")
                .extract()
                .expect("retry_after is float | None");
            assert_eq!(secs, Some(1.5));

            // Without a hint: the attribute is present and None.
            let err_none = to_py_err(thetadatadx::Error::Grpc {
                kind: GrpcStatusKind::ResourceExhausted,
                message: "429".into(),
                retry_after: None,
            });
            let secs_none: Option<f64> = err_none
                .value(py)
                .getattr("retry_after")
                .expect("retry_after attribute is present")
                .extract()
                .expect("retry_after is float | None");
            assert_eq!(secs_none, None);
        });
    }

    #[test]
    fn decode_error_maps_to_schema_mismatch() {
        Python::initialize();
        Python::attach(|py| {
            let err = to_py_err(thetadatadx::Error::decode_codec("cell type mismatch"));
            assert_exception_class(py, &err, "SchemaMismatchError");
        });
    }

    #[test]
    fn fpss_too_many_requests_maps_to_rate_limit() {
        Python::initialize();
        Python::attach(|py| {
            let err = to_py_err(thetadatadx::Error::Stream {
                kind: StreamErrorKind::TooManyRequests,
                message: "back off".into(),
            });
            assert_exception_class(py, &err, "RateLimitError");
        });
    }

    #[test]
    fn fpss_protocol_error_maps_to_stream_error() {
        Python::initialize();
        Python::attach(|py| {
            let err = to_py_err(thetadatadx::Error::Stream {
                kind: StreamErrorKind::ProtocolError,
                message: "bad frame".into(),
            });
            assert_exception_class(py, &err, "StreamError");
        });
    }

    #[test]
    fn config_validation_failure_maps_to_invalid_parameter() {
        // A rejected user parameter (bad value, out-of-range, missing
        // field) routes to the dedicated invalid-parameter class.
        Python::initialize();
        Python::attach(|py| {
            let invalid_value = to_py_err(thetadatadx::Error::config_invalid(
                "historical.uri",
                "invalid URI",
            ));
            assert_exception_class(py, &invalid_value, "InvalidParameterError");

            let out_of_range = to_py_err(thetadatadx::Error::config_out_of_range(
                "streaming.timeout_ms",
                0,
                100,
                60_000,
            ));
            assert_exception_class(py, &out_of_range, "InvalidParameterError");

            let missing = to_py_err(thetadatadx::Error::config_missing("auth.email"));
            assert_exception_class(py, &missing, "InvalidParameterError");
        });
    }

    #[test]
    fn config_environmental_fault_maps_to_config_error() {
        // A non-input config fault (file I/O, TOML parse, internal
        // invariant) is not a user-parameter error; it routes to the
        // dedicated `ConfigError` leaf, matching the C++ `ConfigError`
        // and the C ABI `THETADATADX_ERR_CONFIG` discriminant.
        Python::initialize();
        Python::attach(|py| {
            let io = to_py_err(thetadatadx::Error::config_io("file not found"));
            assert_exception_class(py, &io, "ConfigError");

            let toml = to_py_err(thetadatadx::Error::config_toml("expected `]`"));
            assert_exception_class(py, &toml, "ConfigError");
        });
    }

    #[test]
    fn flatfiles_unavailable_maps_to_stream_error() {
        // FlatFiles availability failures are a streaming-surface fault;
        // they route to `StreamError` so the class matches the C++ / C
        // ABI mapping (both pin this to the stream discriminant).
        Python::initialize();
        Python::attach(|py| {
            let err = to_py_err(thetadatadx::Error::FlatFilesUnavailable(
                thetadatadx::flatfiles::FlatFilesUnavailableReason::RequestRejected {
                    server_message: "no subscription".into(),
                },
            ));
            assert_exception_class(py, &err, "StreamError");
        });
    }

    #[test]
    fn partial_reconnect_maps_to_stream_error() {
        Python::initialize();
        Python::attach(|py| {
            let err = to_py_err(thetadatadx::Error::PartialReconnect { failed: Vec::new() });
            assert_exception_class(py, &err, "StreamError");
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
                (
                    "InvalidParameterError",
                    invalid_parameter_type(py)
                        .expect("InvalidParameterError type builds")
                        .bind(py)
                        .clone(),
                ),
                ("SchemaMismatchError", py.get_type::<SchemaMismatchError>()),
                ("NetworkError", py.get_type::<NetworkError>()),
                ("UnavailableError", py.get_type::<UnavailableError>()),
                (
                    "DeadlineExceededError",
                    py.get_type::<DeadlineExceededError>(),
                ),
                ("NotFoundError", py.get_type::<NotFoundError>()),
                ("StreamError", py.get_type::<StreamError>()),
                ("ConfigError", py.get_type::<ConfigError>()),
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

    #[test]
    fn legacy_names_are_aliases_of_canonical_classes() {
        // `register_exceptions` adds the canonical type object under the
        // legacy name too, so the alias shares identity with its target.
        // An existing `except thetadatadx.NoDataFoundError` clause then
        // catches the dispatched `NotFoundError`.
        Python::initialize();
        Python::attach(|py| {
            let module = PyModule::new(py, "thetadatadx_alias_probe")
                .expect("module construction succeeds with GIL held");
            register_exceptions(py, &module).expect("registration succeeds");

            for (legacy, canonical) in [
                ("NoDataFoundError", "NotFoundError"),
                ("TimeoutError", "DeadlineExceededError"),
            ] {
                let legacy_obj = module.getattr(legacy).expect("legacy name is registered");
                let canonical_obj = module
                    .getattr(canonical)
                    .expect("canonical name is registered");
                assert!(
                    legacy_obj.is(&canonical_obj),
                    "{legacy} must be the same object as {canonical}"
                );
            }
        });
    }
}
