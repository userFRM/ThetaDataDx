//! Layered Python exception hierarchy for `thetadatadx`.
//!
//! A typed exception hierarchy lets user code `except` on the class it
//! actually cares about — distinguishing authentication failures from
//! rate limiting or transient network errors — rather than parsing the
//! message text off a generic `PyRuntimeError` / `PyConnectionError`.
//!
//! ```python
//! try:
//!     client.historical.stock_history_eod("AAPL", "20240101", "20240301")
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
create_exception!(thetadatadx, InvalidParameterError, ThetaDataError);

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

// FPSS streaming protocol / state-machine failures.
create_exception!(thetadatadx, StreamError, ThetaDataError);

// Environmental configuration fault — a config-file read failure, a
// TOML parse error, or an internal config invariant. Distinct from
// `InvalidParameterError` (a rejected user-supplied argument): a
// `ConfigError` is the environment, not the call site. Pinned to the
// reserved `TDX_ERR_CONFIG` discriminant so a `except
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
        py.get_type::<InvalidParameterError>(),
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

/// Map a `thetadatadx::Error` into the closest Python exception class.
///
/// The mapping is deliberately narrow: every variant routes to one
/// concrete leaf class. The `#[non_exhaustive]` catch-all routes unknown
/// future variants to `ThetaDataError` so upstream SDK additions never
/// break the Python wheel build.
pub fn to_py_err(e: thetadatadx::Error) -> PyErr {
    use thetadatadx::error::{AuthErrorKind, FpssErrorKind, GrpcStatusKind};

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
                InvalidParameterError::new_err(e.to_string())
            } else {
                ConfigError::new_err(e.to_string())
            }
        }
        thetadatadx::Error::Fpss { kind, .. } => match kind {
            FpssErrorKind::TooManyRequests => rate_limit_err(&e),
            FpssErrorKind::Timeout => DeadlineExceededError::new_err(e.to_string()),
            FpssErrorKind::ConnectionRefused | FpssErrorKind::Disconnected => {
                NetworkError::new_err(e.to_string())
            }
            FpssErrorKind::ProtocolError => StreamError::new_err(e.to_string()),
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
    use thetadatadx::error::{AuthErrorKind, FpssErrorKind, GrpcStatusKind};

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
    fn config_validation_failure_maps_to_invalid_parameter() {
        // A rejected user parameter (bad value, out-of-range, missing
        // field) routes to the dedicated invalid-parameter class.
        Python::initialize();
        Python::attach(|py| {
            let invalid_value = to_py_err(thetadatadx::Error::config_invalid(
                "mdds.uri",
                "invalid URI",
            ));
            assert_exception_class(py, &invalid_value, "InvalidParameterError");

            let out_of_range = to_py_err(thetadatadx::Error::config_out_of_range(
                "fpss.timeout_ms",
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
        // and the C ABI `TDX_ERR_CONFIG` discriminant.
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
                    py.get_type::<InvalidParameterError>(),
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
