//! TypeScript / Node.js bindings over the Rust `thetadatadx` core. Every call
//! crosses the napi-rs boundary into the same Rust code path used by the CLI
//! and FFI.

#[macro_use]
extern crate napi_derive;

use std::io;
use std::sync::{Arc, Mutex, OnceLock};

use napi::Either;
use thetadatadx as tick;
use thetadatadx::auth;
use thetadatadx::config;
use thetadatadx::fpss;

/// Shared tokio runtime for running async Rust from Node.js.
///
/// The runtime is process-global and built exactly once. The first client
/// connected in the process seeds it from that client's `config.runtime`
/// via [`runtime_from_config`], so `Config.workerThreads` takes effect for
/// the first client created in the process.
///
/// The build is fallible — `build_runtime` returns an `io::Result` (it can
/// fail when the OS refuses the worker threads or event loop). The result
/// is memoised verbatim so a build failure surfaces as a typed error to
/// the connect caller instead of aborting the process, and so a later
/// connect re-observes the same outcome rather than racing a second build.
static RT: OnceLock<io::Result<tokio::runtime::Runtime>> = OnceLock::new();

/// Map a memoised runtime-build outcome to a borrowed runtime or a napi
/// error. The stored `io::Error` is not `Clone`, so its message is copied
/// into a fresh napi error on each failing call.
fn runtime_result(
    result: &'static io::Result<tokio::runtime::Runtime>,
) -> napi::Result<&'static tokio::runtime::Runtime> {
    result
        .as_ref()
        .map_err(|e| napi::Error::from_reason(format!("failed to create tokio runtime: {e}")))
}

/// Build (or return the already-built) process-global runtime, sizing the
/// worker pool from the first client's [`thetadatadx::RuntimeConfig`].
///
/// The first connect in the process seeds the pool from its
/// `config.runtime`; later connects share the already-built runtime, so
/// their `runtime` config is a no-op by design. A build failure is
/// returned as a napi error, never a panic.
pub(crate) fn runtime_from_config(
    cfg: &thetadatadx::RuntimeConfig,
) -> napi::Result<&'static tokio::runtime::Runtime> {
    runtime_result(RT.get_or_init(|| cfg.build_runtime()))
}

/// Return the process-global runtime, building it with tokio default
/// sizing if no client has seeded it from config yet.
///
/// Connect functions seed the pool via [`runtime_from_config`]; every
/// post-connect call resolves the already-built runtime here. A build
/// failure is returned as a napi error, never a panic.
pub(crate) fn runtime() -> napi::Result<&'static tokio::runtime::Runtime> {
    runtime_result(RT.get_or_init(|| thetadatadx::RuntimeConfig::default().build_runtime()))
}

/// Convert a `thetadatadx::Error` into a napi error whose `reason`
/// carries a typed class-name prefix (`"[SubscriptionError] ..."`,
/// `"[RateLimitError] ..."`, etc). The JS shim in `streaming-session.js`
/// intercepts every async-method rejection, parses the prefix, and
/// re-throws the right `thetadatadx.SubscriptionError` / `thetadatadx.RateLimitError`
/// subclass. The classes derive from the existing TypeScript-exported
/// base `ThetaDataError` so callers writing `catch (e instanceof
/// thetadatadx.ThetaDataError)` continue to observe every failure.
///
/// The canonical leaf set (`NotFoundError`, `DeadlineExceededError`,
/// `UnavailableError`, `InvalidParameterError`, ...) is identical to the
/// Python, C++, and C ABI leaf sets, so an `except`/`catch` clause ports
/// across bindings by name. Python additionally ships two back-compat
/// aliases (`NoDataFoundError` / `TimeoutError`) that do not exist here.
///
/// When the error is a rate limit carrying a server `retry_after` hint,
/// the prefix is widened to `"[RateLimitError retry_after_ms=N] ..."` so
/// the JS shim can surface the back-off as a typed `retryAfter` property
/// (seconds) on the thrown `RateLimitError`.
pub(crate) fn to_napi_err(e: thetadatadx::Error) -> napi::Error {
    let class = leaf_class_for(&e);
    let prefix = match e.retry_after() {
        Some(d) => format!("[{class} retry_after_ms={}]", d.as_millis()),
        None => format!("[{class}]"),
    };
    napi::Error::from_reason(format!("{prefix} {e}"))
}

/// Build a napi error whose `reason` carries the `[ClassName] ...` prefix
/// the JS shim keys on to re-throw the matching typed subclass. The
/// per-class wrappers below ([`invalid_parameter_err`], [`config_option_err`])
/// name their class so the call sites read intent.
fn typed_napi_err(class: &str, message: impl std::fmt::Display) -> napi::Error {
    napi::Error::from_reason(format!("[{class}] {message}"))
}

/// Build an `InvalidParameterError`-typed napi error for user-input
/// validation that fails before reaching the core client. The JS shim
/// keys on the `[ClassName]` prefix to re-throw the typed subclass, so
/// TypeScript callers branch on `instanceof InvalidParameterError`
/// exactly as Python callers catch the parity `ValueError`.
pub(crate) fn invalid_parameter_err(message: impl std::fmt::Display) -> napi::Error {
    typed_napi_err("InvalidParameterError", message)
}

/// Project the core's active full-subscription set into the cross-binding
/// `[{ kind, contract }]` JSON shape returned by `activeFullSubscriptions()`
/// on both the unified `Client` and the standalone `StreamingClient`. `kind`
/// is `"full_trades"` / `"full_open_interest"`; `contract` carries the
/// wire-level security type. Quote is never a valid full-stream kind on the
/// streaming wire, so any such row is dropped to keep the projection clean.
pub(crate) fn project_full_subscriptions(
    subs: Vec<(
        thetadatadx::fpss::protocol::SubscriptionKind,
        thetadatadx::SecType,
    )>,
) -> serde_json::Value {
    use thetadatadx::fpss::protocol::SubscriptionKind;
    serde_json::json!(subs
        .into_iter()
        .filter_map(|(kind, sec_type)| {
            let kind_str = match kind {
                SubscriptionKind::Trade => "full_trades",
                SubscriptionKind::OpenInterest => "full_open_interest",
                SubscriptionKind::Quote => return None,
                _ => return None,
            };
            Some(serde_json::json!({
                "kind": kind_str,
                "contract": format!("{sec_type:?}"),
            }))
        })
        .collect::<Vec<_>>())
}

// ── Credentials ──
//
// A first-class credentials handle mirroring the Python `Credentials`
// pyclass (`thetadatadx-py/src/lib.rs`), the C++ `thetadatadx::Credentials`, and the
// C ABI `ThetaDataDxCredentials` handle. Every binding builds credentials the
// same way — `new Credentials(email, password)` or
// `Credentials.fromFile(path)` — then hands the handle to `connect`, so
// the connect surface is `connect(creds, config?)` across the board
// rather than each binding spreading raw `email`/`password` strings.

/// ThetaData login credentials.
///
/// Build from an email + password pair (`new Credentials(email,
/// password)`) or load from a credentials file (`Credentials.fromFile`,
/// line 1 = email, line 2 = password), then pass the handle to a client
/// `connect(creds, config?)`.
///
/// ```ts
/// import { Credentials, Client } from "thetadatadx";
/// const creds = Credentials.fromFile("creds.txt");
/// const client = await Client.connect(creds);
/// ```
#[napi]
#[derive(Clone)]
pub struct Credentials {
    pub(crate) inner: auth::Credentials,
}

#[napi]
impl Credentials {
    /// Create credentials from an email and password.
    #[napi(constructor)]
    pub fn new(email: String, password: String) -> Credentials {
        Credentials {
            inner: auth::Credentials::new(email, password),
        }
    }

    /// Load credentials from a file (line 1 = email, line 2 = password).
    #[napi(factory, js_name = "fromFile")]
    pub fn from_file(path: String) -> napi::Result<Credentials> {
        let inner = auth::Credentials::from_file(&path).map_err(to_napi_err)?;
        Ok(Credentials { inner })
    }

    /// Authenticate with an API key instead of an email + password. The
    /// key is trimmed and held as secret material; `toString` never
    /// exposes it.
    #[napi(factory, js_name = "fromApiKey")]
    pub fn from_api_key(api_key: String) -> Credentials {
        Credentials {
            inner: auth::Credentials::api_key(api_key),
        }
    }

    /// Authenticate with an API key paired with an account email. The
    /// email is lowercased and trimmed; an empty email is dropped.
    #[napi(factory, js_name = "fromApiKeyWithEmail")]
    pub fn from_api_key_with_email(email: String, api_key: String) -> Credentials {
        Credentials {
            inner: auth::Credentials::api_key_with_email(email, api_key),
        }
    }

    /// Source credentials strictly from the `THETADATA_API_KEY`
    /// environment variable. Strict: an unset or whitespace-only value
    /// rejects with `[ConfigError]` rather than falling back, and there is
    /// no `creds.txt` file fallback. Use `fromEnvOrFile` when a file
    /// fallback is wanted instead.
    #[napi(factory, js_name = "fromEnv")]
    pub fn from_env() -> napi::Result<Credentials> {
        let inner = auth::Credentials::from_env().map_err(to_napi_err)?;
        Ok(Credentials { inner })
    }

    /// Source credentials from the environment, falling back to a file.
    /// When `THETADATA_API_KEY` is set and non-empty an API key is used;
    /// otherwise the two-line file at `path` is read.
    #[napi(factory, js_name = "fromEnvOrFile")]
    pub fn from_env_or_file(path: String) -> napi::Result<Credentials> {
        let inner = auth::Credentials::from_env_or_file(&path).map_err(to_napi_err)?;
        Ok(Credentials { inner })
    }

    /// Source credentials from a `.env`-format file. The file uses the
    /// common `.env` grammar (one `KEY=VALUE` per line, optional `export`
    /// prefix, `#` comments, optional quotes). `THETADATA_API_KEY`
    /// selects an API key; otherwise `THETADATA_EMAIL` +
    /// `THETADATA_PASSWORD` build email + password credentials.
    #[napi(factory, js_name = "fromDotenv")]
    pub fn from_dotenv(path: String) -> napi::Result<Credentials> {
        let inner = auth::Credentials::from_dotenv(&path).map_err(to_napi_err)?;
        Ok(Credentials { inner })
    }

    /// Redacted string form — never exposes the email or password.
    #[napi(js_name = "toString")]
    pub fn to_string_js(&self) -> String {
        "Credentials(email=<redacted>)".to_string()
    }
}

/// Load credentials from a two-line file, seed the process-global runtime
/// from `cfg`, and run the connect handshake off the libuv thread,
/// returning the shared client handle.
///
/// Shared by the `connectFromFile` factory on both the unified [`Client`]
/// and the historical-only [`HistoricalClient`]: the connect core is
/// identical (`thetadatadx::Client::connect` opens the historical channel
/// and Nexus, never streaming until a streaming method is called); the two
/// factories differ only in the napi handle they wrap around the returned
/// `Arc`.
pub(crate) async fn connect_historical_from_file_core(
    path: String,
    cfg: config::DirectConfig,
) -> napi::Result<Arc<thetadatadx::Client>> {
    let creds = auth::Credentials::from_file(&path).map_err(to_napi_err)?;
    let rt = runtime_from_config(&cfg.runtime)?;
    let client = rt
        .spawn(async move { thetadatadx::Client::connect(&creds, cfg).await })
        .await
        .map_err(|e| napi::Error::from_reason(format!("connect task failed to complete: {e}")))?
        .map_err(to_napi_err)?;
    Ok(Arc::new(client))
}

/// Snapshot an optional [`Config`] handle into an owned [`DirectConfig`],
/// falling back to the production default when none is supplied. The
/// snapshot decouples the client from later mutations of the `Config`
/// handle, matching the connect-time snapshot semantics every binding
/// shares.
pub(crate) fn config_or_production(config: Option<&Config>) -> config::DirectConfig {
    match config {
        Some(c) => c.snapshot(),
        None => config::DirectConfig::production(),
    }
}

/// Build a napi `Error` tagged as a `ConfigError` for a malformed
/// client-construction option (conflicting or absent auth fields, an
/// unparseable `historicalType`). Matches the `[ConfigError]` prefix the JS
/// shim re-throws as a typed `ConfigError`, so the failure surfaces the
/// same branded class the other bindings raise.
fn config_option_err(message: impl AsRef<str>) -> napi::Error {
    typed_napi_err("ConfigError", message.as_ref())
}

/// Inline authentication + environment for [`Client::connectWith`].
///
/// The API key is a first-class field, distinct from the email +
/// password pair and from the `credentialsFile` path. Exactly one
/// authentication field must be set; [`Self::resolve`] enforces this and
/// rejects a conflict before any network round-trip.
#[napi(object)]
pub struct ClientConnectOptions {
    /// Inline API key — the primary, directly-passed auth field.
    pub api_key: Option<String>,
    /// Source the API key strictly from the `THETADATA_API_KEY`
    /// environment variable (set to `true` to select this source). Strict,
    /// with no file fallback: an unset or whitespace-only value is a
    /// configuration error. For the env-or-file convenience use
    /// `apiKeyFromDotenv`.
    pub api_key_from_env: Option<bool>,
    /// Source the credential from a `.env`-format file at this path.
    pub api_key_from_dotenv: Option<String>,
    /// Inline account email, paired with `password`.
    pub email: Option<String>,
    /// Inline account password, paired with `email`.
    pub password: Option<String>,
    /// Path to a two-line `creds.txt` file (line 1 = email, line 2 =
    /// password).
    pub credentials_file: Option<String>,
    /// Historical environment selector (`"PROD"` / `"STAGE"`,
    /// case-insensitive). Defaults to production. The historical and
    /// streaming channels are selected independently. For full host-level
    /// control, build a `Config` and use `Client.connect(creds, config)`.
    pub historical_type: Option<String>,
    /// Streaming environment selector (`"PROD"` / `"DEV"`,
    /// case-insensitive). Defaults to production. Selected independently of
    /// the historical channel.
    pub streaming_type: Option<String>,
}

impl ClientConnectOptions {
    /// Resolve the options into a concrete credential + config, enforcing
    /// exactly one authentication source.
    fn resolve(self) -> napi::Result<(auth::Credentials, config::DirectConfig)> {
        let ClientConnectOptions {
            api_key,
            api_key_from_env,
            api_key_from_dotenv,
            email,
            password,
            credentials_file,
            historical_type,
            streaming_type,
        } = self;

        // Count the distinct auth methods. `email` + `password` together
        // count as the single email/password method.
        let has_api_key = api_key.is_some();
        let has_env = api_key_from_env == Some(true);
        let has_dotenv = api_key_from_dotenv.is_some();
        let has_email_pw = email.is_some() || password.is_some();
        let has_creds_file = credentials_file.is_some();
        let set_count = u8::from(has_api_key)
            + u8::from(has_env)
            + u8::from(has_dotenv)
            + u8::from(has_email_pw)
            + u8::from(has_creds_file);

        if set_count == 0 {
            return Err(config_option_err(
                "no authentication field set — set one of apiKey, apiKeyFromEnv, \
                 apiKeyFromDotenv, the email/password pair, or credentialsFile",
            ));
        }
        if set_count > 1 {
            return Err(config_option_err(
                "conflicting authentication fields — set exactly one of apiKey, \
                 apiKeyFromEnv, apiKeyFromDotenv, the email/password pair, or credentialsFile",
            ));
        }

        let creds = if let Some(key) = api_key {
            auth::Credentials::api_key(key)
        } else if has_env {
            // Strict, no file fallback: an unset or whitespace-only
            // `THETADATA_API_KEY` is a configuration error, mirroring the
            // Rust `ClientBuilder::api_key_from_env` and the C++ / Python
            // bindings so the same-named capability agrees everywhere.
            auth::Credentials::from_env().map_err(to_napi_err)?
        } else if let Some(path) = api_key_from_dotenv {
            auth::Credentials::from_dotenv(&path).map_err(to_napi_err)?
        } else if has_email_pw {
            match (email, password) {
                (Some(email), Some(password)) => auth::Credentials::new(email, password),
                _ => {
                    return Err(config_option_err(
                        "email/password authentication needs both email and password",
                    ));
                }
            }
        } else if let Some(path) = credentials_file {
            auth::Credentials::from_file(&path).map_err(to_napi_err)?
        } else {
            // Unreachable: set_count == 1 covers every branch above.
            return Err(config_option_err("no authentication field set"));
        };

        // The historical and streaming channels are selected
        // independently on top of the production defaults; either absent
        // keeps that channel on production. An unrecognized value is a
        // config error naming the valid set, never a silent fallback.
        let mut cfg = config::DirectConfig::production();
        if let Some(raw) = historical_type.as_deref() {
            let environment = config::HistoricalEnvironment::parse(raw).ok_or_else(|| {
                config_option_err(format!(
                    "historicalType must be \"PROD\" or \"STAGE\" (case-insensitive); got {raw:?}"
                ))
            })?;
            cfg = cfg.with_historical_environment(environment);
        }
        if let Some(raw) = streaming_type.as_deref() {
            let environment = config::StreamingEnvironment::parse(raw).ok_or_else(|| {
                config_option_err(format!(
                    "streamingType must be \"PROD\" or \"DEV\" (case-insensitive); got {raw:?}"
                ))
            })?;
            cfg = cfg.with_streaming_environment(environment);
        }

        Ok((creds, cfg))
    }
}

/// Guard that a JS `number` (IEEE-754 double) carrying an integer query
/// parameter is a non-negative whole number within `[0, max]`, returning
/// it unchanged on success so the caller can cast to its integer domain.
///
/// The integer-typed bindings (Python / C++ / C ABI) cannot represent a
/// non-finite, negative, or fractional value, and a bare `as`-cast would
/// silently rewrite a hostile or oversized input (`NaN`/`Infinity` → `0`
/// or the type max, a fractional value → its truncation, an out-of-range
/// magnitude → a wrapped value) — each the opposite of a caller's intent.
/// A rejected value surfaces as `InvalidParameterError`, the typed class
/// the Python binding raises (`ValueError`) for the identical input, so a
/// caller's `catch (e instanceof InvalidParameterError)` branch ports
/// across bindings. `param` names the camelCase key in the message.
///
/// `unit` carries the singular unit noun (e.g. `"millisecond"`) when the
/// value has a physical unit, so the rejection names it
/// ("...integer number of milliseconds", "...representable millisecond
/// range"); `None` keeps the unit-free "...whole number" / "...representable
/// range" wording for a bare count.
fn validate_nonneg_whole(
    param: &str,
    value: f64,
    max: f64,
    unit: Option<&str>,
) -> napi::Result<f64> {
    if !value.is_finite() {
        let detail = match unit {
            Some(u) => format!("a non-negative integer number of {u}s"),
            None => "a non-negative whole number".to_string(),
        };
        return Err(invalid_parameter_err(format!(
            "{param} must be {detail}; got {value}"
        )));
    }
    if value < 0.0 {
        return Err(invalid_parameter_err(format!(
            "{param} must be non-negative; got {value}"
        )));
    }
    if value.fract() != 0.0 {
        let detail = match unit {
            Some(u) => format!("a whole number of {u}s"),
            None => "a whole number".to_string(),
        };
        return Err(invalid_parameter_err(format!(
            "{param} must be {detail}; got {value}"
        )));
    }
    if value > max {
        let range = match unit {
            Some(u) => format!("the representable {u} range"),
            None => "the representable range".to_string(),
        };
        return Err(invalid_parameter_err(format!(
            "{param} exceeds {range}; got {value}"
        )));
    }
    Ok(value)
}

/// Validate a JS `number` carrying a `u32` domain knob (worker-thread
/// count, CPU index, attempt budget, …) and narrow it to `u32`.
///
/// napi's bare `u32` argument binding is V8 `ToUint32`, which never
/// rejects: `-1` wraps to `u32::MAX`, `1.5` truncates to `1`, `2**32`
/// wraps to `0`. Every such rewrite is the opposite of the caller's
/// intent and the integer-typed bindings (Python / C++ / C ABI) cannot
/// express the value at all, so we take the argument as `f64` and reject
/// a non-finite, negative, fractional, or over-`u32` value as
/// `InvalidParameterError` — the typed class the Python binding raises
/// (`ValueError`) for the identical input. `name` is the camelCase key.
pub(crate) fn validate_u32_arg(name: &str, v: f64) -> napi::Result<u32> {
    Ok(validate_nonneg_whole(name, v, u32::MAX as f64, None)? as u32)
}

/// Validate a `u32` domain knob that additionally requires `>= 1` (a
/// burst size or attempt budget where `0` is a degenerate value the core
/// rejects at connect). Layered on [`validate_u32_arg`] so the
/// finite/whole/range checks stay in one place; only the extra `0` floor
/// lives here.
pub(crate) fn validate_u32_arg_min1(name: &str, v: f64) -> napi::Result<u32> {
    let value = validate_u32_arg(name, v)?;
    if value == 0 {
        return Err(invalid_parameter_err(format!(
            "{name} must be at least 1; got 0"
        )));
    }
    Ok(value)
}

/// Validate an optional `u32` domain knob, leaving an omitted value
/// (`None`) untouched. The pinned-CPU / worker-thread setters take this
/// shape: `null` defers to the OS / default sizing, a number pins the
/// value (with `0` a valid choice — verbatim worker/core count).
pub(crate) fn validate_optional_u32_arg(name: &str, v: Option<f64>) -> napi::Result<Option<u32>> {
    v.map(|v| validate_u32_arg(name, v)).transpose()
}

/// Validate a JavaScript `timeoutMs` deadline and convert it to the
/// integer millisecond domain the Python, C++, and C ABI bindings take.
pub(crate) fn validate_timeout_ms(timeout_ms: f64) -> napi::Result<u64> {
    Ok(validate_nonneg_whole(
        "timeoutMs",
        timeout_ms,
        u64::MAX as f64,
        Some("millisecond"),
    )? as u64)
}

/// Validate an optional non-negative integer query parameter (the bounded
/// integer filters `maxDte` / `strikeRange` — counts, never negative) and
/// convert it to the `i32` domain the core request builders take, leaving
/// an omitted value (`None`) untouched.
pub(crate) fn validate_optional_nonneg_i32(
    param: &str,
    value: Option<f64>,
) -> napi::Result<Option<i32>> {
    let Some(value) = value else { return Ok(None) };
    Ok(Some(
        validate_nonneg_whole(param, value, i32::MAX as f64, None)? as i32,
    ))
}

/// Run an endpoint round-trip off the runtime's execution thread and
/// hand the result back as a `napi::Result`.
///
/// Every generated historical endpoint method is an `async fn`: napi-rs
/// returns a JS Promise to the caller and polls the method's future on
/// its own runtime, never on the V8 execution thread. The actual
/// network round-trip is spawned onto [`runtime()`] — the same runtime
/// the gRPC connection was established on, so the request's sockets and
/// timers are driven by the reactor that owns them — and the spawned
/// task's `JoinHandle` is awaited from the method's future. The Node
/// event loop therefore stays free for the whole duration of the call:
/// timers fire, queued promises advance, and concurrent requests make
/// progress instead of stalling behind one fetch.
///
/// Errors from the round-trip carry the same typed class-name prefix as
/// the streaming surface via [`to_napi_err`]. A task that panics is
/// surfaced as a generic napi error rather than aborting the process.
pub(crate) async fn spawn_endpoint_task<F, T>(fut: F) -> napi::Result<T>
where
    F: std::future::Future<Output = Result<T, thetadatadx::Error>> + Send + 'static,
    T: Send + 'static,
{
    match runtime()?.spawn(fut).await {
        Ok(inner) => inner.map_err(to_napi_err),
        Err(join_err) => Err(napi::Error::from_reason(format!(
            "endpoint task failed to complete: {join_err}"
        ))),
    }
}

/// Pick the typed leaf class name for a `thetadatadx::Error`. The
/// JS shim parses this prefix off the error reason. The canonical leaf
/// names match the Python `to_py_err` dispatch one-for-one.
fn leaf_class_for(e: &thetadatadx::Error) -> &'static str {
    use thetadatadx::error::{AuthErrorKind, GrpcStatusKind, StreamErrorKind};
    match e {
        thetadatadx::Error::Auth { kind, .. } => match kind {
            AuthErrorKind::InvalidCredentials => "InvalidCredentialsError",
            AuthErrorKind::NetworkError => "NetworkError",
            AuthErrorKind::Timeout => "DeadlineExceededError",
            _ => "AuthenticationError",
        },
        thetadatadx::Error::Grpc { kind, .. } => match kind {
            GrpcStatusKind::PermissionDenied => "SubscriptionError",
            GrpcStatusKind::ResourceExhausted => "RateLimitError",
            GrpcStatusKind::NotFound => "NotFoundError",
            GrpcStatusKind::DeadlineExceeded => "DeadlineExceededError",
            GrpcStatusKind::Unauthenticated => "AuthenticationError",
            GrpcStatusKind::Unavailable => "UnavailableError",
            _ => "ThetaDataError",
        },
        thetadatadx::Error::NoData => "NotFoundError",
        thetadatadx::Error::Timeout { .. } => "DeadlineExceededError",
        thetadatadx::Error::Transport { .. }
        | thetadatadx::Error::Tls(_)
        | thetadatadx::Error::Io(_)
        | thetadatadx::Error::Http(_) => "NetworkError",
        thetadatadx::Error::Decode { .. } | thetadatadx::Error::Decompress { .. } => {
            "SchemaMismatchError"
        }
        // User-input validation failures route to the dedicated
        // invalid-parameter class; environmental config faults route to
        // the dedicated `ConfigError` class.
        thetadatadx::Error::Config { kind, .. } => {
            if kind.is_invalid_parameter() {
                "InvalidParameterError"
            } else {
                "ConfigError"
            }
        }
        thetadatadx::Error::Stream { kind, .. } => match kind {
            StreamErrorKind::TooManyRequests => "RateLimitError",
            StreamErrorKind::Timeout => "DeadlineExceededError",
            StreamErrorKind::ConnectionRefused | StreamErrorKind::Disconnected => "NetworkError",
            _ => "StreamError",
        },
        // FlatFiles availability + partial-reconnect failures are
        // streaming-surface faults; pin them to `StreamError` so a
        // `catch (e instanceof StreamError)` clause behaves identically
        // to the C++ and C ABI mapping (both route these to the stream
        // discriminant).
        thetadatadx::Error::FlatFilesUnavailable(_)
        | thetadatadx::Error::PartialReconnect { .. } => "StreamError",
        _ => "ThetaDataError",
    }
}

/// Pin the ring rustls `CryptoProvider` as the process-wide default
/// when the `.node` module is loaded by Node.js. Without this, the
/// first `Client.connect()` call panics with
/// "Could not automatically determine the process-level CryptoProvider"
/// — rustls 0.23 requires `install_default` before the first handshake
/// even when a single provider is compiled in. The workspace builds
/// rustls / tokio-rustls / hyper-rustls with `default-features = false,
/// features = ["ring", ...]`, so ring is the only provider in the dep
/// graph; this hook seats it on Node module load. Mirrors the
/// equivalent call in the Python SDK's `#[pymodule]` init.
#[module_init]
fn init() {
    let _ = thetadatadx::__internal_install_ring_crypto_provider();
}

fn normalize_symbols(symbols: Either<String, Vec<String>>) -> Vec<String> {
    match symbols {
        Either::A(symbol) => vec![symbol],
        Either::B(symbols) => symbols,
    }
}

/// A `String` passes through verbatim (the caller supplied a wire-format
/// literal); a `DateTime` is rendered with `fmt` (`"%Y%m%d"` for dates,
/// `"%H:%M:%S"` for times).
fn normalize_dt(value: Either<String, chrono::DateTime<chrono::Utc>>, fmt: &str) -> String {
    match value {
        Either::A(value) => value,
        Either::B(value) => value.format(fmt).to_string(),
    }
}

fn normalize_date(value: Either<String, chrono::DateTime<chrono::Utc>>) -> String {
    normalize_dt(value, "%Y%m%d")
}

fn normalize_time(value: Either<String, chrono::DateTime<chrono::Utc>>) -> String {
    normalize_dt(value, "%H:%M:%S")
}

fn normalize_optional_date(
    value: Option<Either<String, chrono::DateTime<chrono::Utc>>>,
) -> Option<String> {
    value.map(normalize_date)
}

fn normalize_optional_time(
    value: Option<Either<String, chrono::DateTime<chrono::Utc>>>,
) -> Option<String> {
    value.map(normalize_time)
}

// Generated string enum exports.
include!("_generated/enums_generated.rs");

// ── Typed tick classes (generated from tick_schema.toml) ──
//
// Emits `#[napi(object)]` structs for every tick type plus
// `{tick}_to_class_vec` factories. These back every historical endpoint
// return so `index.d.ts` surfaces concrete `Tick[]` types instead of `any`.

include!("_generated/tick_classes.rs");

// ── Typed streaming event classes (generated from fpss_event_schema.toml) ──

include!("_generated/fpss_event_classes.rs");

// ── Buffered streaming events ──

//
// Generator-emitted from `fpss_event_schema.toml`. Same file content as
// the Python SDK copy — single source of truth. Change the schema and
// regenerate, never hand-edit the generated `buffered_event.rs`.

include!("_generated/buffered_event.rs");

// ── Offline streaming-saturation bench hook (no network) ──
//
// `__benchFloodEvents(n, callback)` pushes synthetic streaming events through
// the real `TsfnCallback` dispatch path (same bounded queue, same per-event
// marshal) so the TypeScript streaming ceiling can be measured offline.
// Bench-only; carved out of the parity utility roster via
// `_is_ts_internal_free_fn` in `scripts/ci/check_binding_parity.py`.
//
// Gated `cfg(not(test))`: the node bench loads the release-built addon, so
// the export is only ever reached from a non-test build. Under `cfg(test)`
// the napi registration glue for the exported async fn is not emitted, which
// would orphan the module's `make_event` helper and trip `-D dead_code` in
// the `--all-targets` clippy lane. Excluding the module from test builds
// removes that dead path at the root rather than `#[allow]`-suppressing it.
#[cfg(not(test))]
mod bench_streaming;

// ── Cross-language lookup-table helpers (generated from sdk_surface.toml) ──
//
// Emits the `Util` napi class of static condition / exchange / calendar /
// timestamp / sequence helpers, mirroring the Python `thetadatadx.util`
// submodule. Change `sdk_surface.toml` and regenerate, never hand-edit the
// generated file.

include!("_generated/utility_functions.rs");

// ── Unified Client client ──

/// Bound on the number of `StreamEvent` deliveries that may sit in the
/// napi callback queue between the streaming consumer thread and the Node
/// main thread before the consumer is made to wait.
///
/// This queue is the second buffer on the delivery path. The first is the
/// streaming event ring (the `streamingRingSize` setting, 65536 slots by
/// default), drained by the consumer thread; the consumer hands each event
/// to the callback queue here, and the registered JS function runs later on
/// the Node main thread. A bound is required for the `Blocking` call mode to
/// mean anything: with an unbounded queue the `call` never waits, so the
/// consumer drains the ring as fast as it arrives and parks the backlog in
/// this queue instead, where it is invisible to `ringOccupancy()` and
/// `droppedEventCount()` and grows without limit behind a persistently slow
/// JS callback. A finite bound makes a full queue block the consumer, which
/// lets the ring fill and the I/O reader account the overflow on
/// `droppedEventCount()`, the same observable back-pressure the bindings
/// that run the callback directly on the consumer thread already have.
///
/// The depth matches the default ring size so a healthy callback has a full
/// ring's worth of headroom before the consumer ever waits, while a wedged
/// callback can pin at most this many in-flight events.
pub(crate) const STREAMING_CALLBACK_QUEUE_DEPTH: usize = 65_536;

/// `ThreadsafeFunction` that owns a JS callback reference and routes
/// `StreamEvent` deliveries onto the Node main thread via napi-rs's
/// internal `uv_async_t` queue. The fifth const generic `false` selects
/// `ErrorStrategy::Fatal`, so the napi-rs `call` API takes the
/// `StreamEvent` directly (not a `Result`) and the JS side relies on
/// its own try/catch for user-callback failures. The sixth (`false`) keeps
/// the function strong, so a pending event holds the event loop open until
/// it drains rather than being abandoned at shutdown. The seventh,
/// [`STREAMING_CALLBACK_QUEUE_DEPTH`], bounds the call queue so the
/// `Blocking` call mode applies real back-pressure (see that constant). The
/// two `StreamEvent` type parameters are the wire payload and the JS-call
/// arg type respectively; both are the same concrete object here.
///
/// napi-rs is the only safe path: Node's libuv requires JS callbacks
/// on the main thread, so calling V8 from any other thread is
/// undefined behavior. The dispatcher's drain thread therefore hands
/// every event to this `ThreadsafeFunction`, which queues it for the
/// main thread via `napi_call_threadsafe_function`.
pub(crate) type TsfnCallback = napi::threadsafe_function::ThreadsafeFunction<
    StreamEvent,
    (),
    StreamEvent,
    napi::Status,
    false,
    false,
    STREAMING_CALLBACK_QUEUE_DEPTH,
>;

#[napi]
pub struct Client {
    /// Wrapped in `Arc` so async napi methods (e.g. `awaitDrain`) can
    /// clone a cheap handle into a `tokio::task::spawn_blocking` future
    /// without violating the `Send + 'static` bound. The inner
    /// `thetadatadx::Client` is not `Clone` -- its streaming mutex and
    /// subscription-tier state forbid that -- so the outer `Arc` is the
    /// only way to hand a borrow off the napi main thread.
    client: Arc<thetadatadx::Client>,
    /// Stored JS callback registered via `startStreaming(callback)`.
    /// `None` until the first registration; persisted across
    /// `reconnect()` so the reconnect path can re-attach the same JS
    /// function without re-asking the caller for it. Cleared on
    /// `stopStreaming()` / `shutdown()` so the napi reference is
    /// released back to V8 and a subsequent `startStreaming()` sees a
    /// clean slot.
    ///
    /// Wrapped in `Arc` because the dispatcher closure (`Fn(&StreamEvent)
    /// + Send + 'static`) needs its own ref-counted clone of the
    /// callback handle. `ThreadsafeFunction` itself does not implement
    /// `Clone` in napi-rs 3.x (its inner `napi_threadsafe_function`
    /// is `Arc`-managed but only exposed through the
    /// `Arc<`ThreadsafeFunctionHandle`>` field on the struct), so the
    /// outer `Arc` here is the canonical way to share the handle.
    ///
    /// Wrapped in `Arc<Mutex<...>>` so the same callback slot is shared
    /// with the [`StreamView`] returned by `client.stream`: both the
    /// `Client` shell and every `StreamView` handle observe and mutate one
    /// registration, keeping `startStreaming` / `stopStreaming` /
    /// `reconnect` idempotent regardless of which surface the caller
    /// reaches through.
    callback: Arc<Mutex<Option<Arc<TsfnCallback>>>>,
}

/// User-facing historical-data sub-namespace returned by the
/// `client.historical` getter.
///
/// A lightweight handle that shares the underlying client connection;
/// constructing it performs no auth round-trip and mutates no streaming
/// state. Every historical endpoint method is generated onto this view
/// from a single declarative surface definition, so the surface stays a
/// single generated source of truth.
#[napi]
pub struct HistoricalView {
    client: Arc<thetadatadx::Client>,
}

/// User-facing real-time-streaming sub-namespace returned by the
/// `client.stream` getter.
///
/// Shares the parent client's connection and its registered streaming
/// callback, so `startStreaming`, `stopStreaming`, `reconnect`, and the
/// subscription methods observe the same registration the unified client
/// does.
#[napi]
pub struct StreamView {
    client: Arc<thetadatadx::Client>,
    callback: Arc<Mutex<Option<Arc<TsfnCallback>>>>,
}

#[napi]
impl Client {
    /// Historical-data sub-namespace: `client.historical.stockHistoryEOD(...)`.
    ///
    /// Returns a fresh [`HistoricalView`] that shares the underlying
    /// client connection. No auth round-trip, no streaming-state mutation.
    #[napi(getter)]
    pub fn historical(&self) -> HistoricalView {
        HistoricalView {
            client: Arc::clone(&self.client),
        }
    }

    /// Real-time-streaming sub-namespace: `client.stream.subscribe(...)`,
    /// `client.stream.startStreaming(cb)`, …
    ///
    /// Returns a fresh [`StreamView`] sharing the inner client and the
    /// parent's callback slot, so the streaming lifecycle observed through
    /// the view is the one the unified client manages.
    #[napi(getter)]
    pub fn stream(&self) -> StreamView {
        StreamView {
            client: Arc::clone(&self.client),
            callback: Arc::clone(&self.callback),
        }
    }
}

#[napi]
impl Client {
    // Lifecycle: intentionally hand-written (language-specific constructor semantics).

    /// Connect to ThetaData with a `Credentials` handle. Pass an
    /// optional `Config` (`dev` / `stage` / `production`, plus any
    /// tuned setters) to override the production-default endpoint.
    /// Historical only; call `client.stream.startStreaming(...)` to
    /// begin streaming real-time data.
    ///
    /// The config is snapshot at connect time: the `Config` handle may be
    /// reused or mutated afterward without affecting this client.
    ///
    /// ```ts
    /// import { Credentials, Client } from "thetadatadx";
    /// const creds = Credentials.fromFile("creds.txt");
    /// const client = await Client.connect(creds);
    /// ```
    ///
    /// The gRPC channel open plus the authentication handshake are
    /// network-bound, so this is `async`: the work runs on the runtime
    /// off the libuv thread and napi-rs returns a `Promise<Client>`,
    /// leaving the Node event loop free to service timers, IO, and queued
    /// promises for the whole handshake. A plain `async` associated
    /// function is used rather than a `#[napi(factory)]` because a factory
    /// must return its instance synchronously.
    #[napi]
    pub async fn connect(creds: &Credentials, config: Option<&Config>) -> napi::Result<Client> {
        let cfg = config_or_production(config);
        // Seed the process-global runtime from this client's config before
        // spawning onto it, then run the connect handshake off the libuv
        // thread. The credentials are cloned so the spawned future owns
        // `'static` data and does not borrow the napi argument.
        let rt = runtime_from_config(&cfg.runtime)?;
        let creds = creds.inner.clone();
        let client = rt
            .spawn(async move { thetadatadx::Client::connect(&creds, cfg).await })
            .await
            .map_err(|e| napi::Error::from_reason(format!("connect task failed to complete: {e}")))?
            .map_err(to_napi_err)?;
        Ok(Client {
            client: Arc::new(client),
            callback: Arc::new(Mutex::new(None)),
        })
    }

    /// Connect with a credentials file (line 1 = email, line 2 =
    /// password). Convenience wrapper over `Credentials.fromFile` +
    /// `connect`. Pass an optional `Config` to override the
    /// production-default endpoint.
    ///
    /// `async` for the same reason as [`Client::connect`]: the gRPC channel
    /// open plus authentication handshake run off the libuv thread and the
    /// method returns a `Promise<Client>`.
    #[napi(js_name = "connectFromFile")]
    pub async fn connect_from_file(path: String, config: Option<&Config>) -> napi::Result<Client> {
        let client = connect_historical_from_file_core(path, config_or_production(config)).await?;
        Ok(Client {
            client,
            callback: Arc::new(Mutex::new(None)),
        })
    }

    /// Connect with the authentication and environment selected inline via
    /// an options object, with the API key as a first-class, directly-passed
    /// field.
    ///
    /// ```js
    /// const staged = await Client.connectWith({ apiKey: "td1_...", historicalType: "STAGE" });
    /// const withLogin = await Client.connectWith({ email: "u@e.com", password: "secret" });
    /// const fromEnv = await Client.connectWith({ apiKeyFromEnv: true });
    /// ```
    ///
    /// Exactly one authentication field must be set: `apiKey`,
    /// `apiKeyFromEnv`, `apiKeyFromDotenv`, the `email` + `password` pair,
    /// or `credentialsFile`. Passing none, or two different ones, rejects
    /// with a `ConfigError` before any network round-trip. `historicalType`
    /// (`"PROD"` / `"STAGE"`, case-insensitive) selects the historical
    /// environment and `streamingType` (`"PROD"` / `"DEV"`, case-insensitive)
    /// the streaming environment, independently. For a pre-built full
    /// `Config` (or a pre-built `Credentials` handle), use
    /// [`Client::connect`], which takes both.
    ///
    /// `async` for the same reason as [`Client::connect`].
    #[napi(js_name = "connectWith")]
    pub async fn connect_with(options: ClientConnectOptions) -> napi::Result<Client> {
        let (creds, cfg) = options.resolve()?;
        let rt = runtime_from_config(&cfg.runtime)?;
        let client = rt
            .spawn(async move { thetadatadx::Client::connect(&creds, cfg).await })
            .await
            .map_err(|e| napi::Error::from_reason(format!("connect task failed to complete: {e}")))?
            .map_err(to_napi_err)?;
        Ok(Client {
            client: Arc::new(client),
            callback: Arc::new(Mutex::new(None)),
        })
    }
}

#[napi]
impl StreamView {
    /// Whether the live streaming session is currently authenticated.
    ///
    /// Distinct from `isStreaming()`: the session can be live yet briefly
    /// unauthenticated mid-reconnect (the authenticated flag is cleared on
    /// disconnect and restored on a successful re-auth). Returns `false`
    /// before `startStreaming` and after `stopStreaming`. The value
    /// matches every other binding (C ABI, Python, C++).
    #[napi(js_name = "isAuthenticated")]
    pub fn is_authenticated(&self) -> bool {
        self.client.stream().is_authenticated()
    }

    /// Cumulative count of streaming events that were dropped because the
    /// callback fell behind and the in-flight buffer was full.
    ///
    /// The value matches every other binding (C ABI, Python, C++). The
    /// counter resets when the session is recreated -- that happens on
    /// `stopStreaming()` and `reconnect()`. Snapshot the value before
    /// reconnect if you need to accumulate drops across session
    /// boundaries.
    ///
    /// Returned as `bigint` so it can represent the full 64-bit unsigned range
    /// (Number would top out at 2^53).
    #[napi(js_name = "droppedEventCount")]
    pub fn dropped_event_count(&self) -> napi::bindgen_prelude::BigInt {
        napi::bindgen_prelude::BigInt::from(self.client.stream().dropped_event_count())
    }

    /// Point-in-time count of streaming events published into the
    /// event ring but not yet drained into your callback — the
    /// in-flight depth between the I/O thread and the dispatcher.
    ///
    /// The leading back-pressure signal: `droppedEventCount()` only
    /// moves AFTER data has been lost, while a rising occupancy that
    /// approaches `ringCapacity()` predicts those drops while there
    /// is still time to react. Sampling never blocks the feed; poll
    /// it from your own code at any cadence.
    ///
    /// The value matches every other binding (C ABI, Python, C++).
    /// Returns `0n` before `startStreaming` and after `stopStreaming`.
    /// Returned as `bigint` for shape-consistency with the other
    /// streaming counters.
    #[napi(js_name = "ringOccupancy")]
    pub fn ring_occupancy(&self) -> napi::bindgen_prelude::BigInt {
        napi::bindgen_prelude::BigInt::from(self.client.stream().ring_occupancy() as u64)
    }

    /// Configured capacity of the streaming event ring in slots (the
    /// `streamingRingSize` setting, a power of two).
    ///
    /// The fixed denominator for `ringOccupancy()`: when the
    /// occupancy sample approaches this value the ring is saturating
    /// and further events will be dropped (counted by
    /// `droppedEventCount()`). Returns `0n` before `startStreaming`
    /// and after `stopStreaming`. Returned as `bigint` for
    /// shape-consistency with the other streaming counters.
    #[napi(js_name = "ringCapacity")]
    pub fn ring_capacity(&self) -> napi::bindgen_prelude::BigInt {
        napi::bindgen_prelude::BigInt::from(self.client.stream().ring_capacity() as u64)
    }

    /// Milliseconds since the most recent inbound streaming frame of
    /// any kind (data tick, heartbeat, control), or `null` when
    /// streaming has not started or no frame has been received yet.
    ///
    /// The operator-facing staleness clock: a healthy session stays in
    /// the low hundreds of milliseconds (the upstream heartbeats even
    /// when no market data flows), so a steadily growing value is the
    /// earliest external signal of a dead or wedged connection.
    #[napi(js_name = "millisSinceLastEvent")]
    pub fn millis_since_last_event(&self) -> Option<napi::bindgen_prelude::BigInt> {
        self.client
            .stream()
            .millis_since_last_event()
            .map(napi::bindgen_prelude::BigInt::from)
    }

    /// UNIX-nanosecond receive timestamp of the most recent inbound
    /// streaming frame of any kind. Returns `0n` when streaming has
    /// not started or no frame has been received yet. Raw feed for
    /// `millisSinceLastEvent`, exposed for callers correlating against
    /// their own pipeline timestamps.
    #[napi(js_name = "lastEventReceivedAtUnixNanos")]
    pub fn last_event_received_at_unix_nanos(&self) -> napi::bindgen_prelude::BigInt {
        napi::bindgen_prelude::BigInt::from(
            self.client.stream().last_event_received_at_unix_nanos(),
        )
    }

    /// Address (`host:port`) of the streaming server the current
    /// session is connected to, following the session across
    /// auto-reconnects. `null` when streaming has not started.
    #[napi(js_name = "lastConnectedAddr")]
    pub fn last_connected_addr(&self) -> Option<String> {
        self.client.stream().last_connected_addr()
    }

    /// Cumulative count of user-callback panics caught at the per-event
    /// isolation boundary since the current stream started.
    ///
    /// A panic in the callback is caught, recorded here, and does not
    /// stop event delivery — the next event continues normally. The
    /// value matches every other binding (C ABI, Python, C++).
    ///
    /// Returned as `bigint` so it can represent the full 64-bit unsigned range
    /// (Number would top out at 2^53).
    #[napi(js_name = "panicCount")]
    pub fn panic_count(&self) -> napi::bindgen_prelude::BigInt {
        napi::bindgen_prelude::BigInt::from(self.client.stream().panic_count())
    }

    /// Set the slow-callback wall-clock threshold in microseconds.
    ///
    /// When a callback invocation runs longer than `thresholdUs`,
    /// `slowCallbackCount()` increments and a rate-limited warning is
    /// logged. Pass `0n` to disable the watchdog (the default).
    /// Observability only: the watchdog never cancels the callback. No-op
    /// before `startStreaming`. Accepts `bigint` for the full 64-bit
    /// unsigned range.
    #[napi(js_name = "setSlowCallbackThresholdUs")]
    pub fn set_slow_callback_threshold_us(
        &self,
        threshold_us: napi::bindgen_prelude::BigInt,
    ) -> napi::Result<()> {
        // Reject a negative or over-u64 BigInt rather than silently passing a
        // wrapped/truncated value, matching the config setters' lossless check.
        let value =
            crate::config_class::bigint_to_u64("setSlowCallbackThresholdUs", &threshold_us)?;
        self.client
            .stream()
            .set_slow_callback_threshold(std::time::Duration::from_micros(value));
        Ok(())
    }

    /// Cumulative count of user-callback invocations whose wall-clock
    /// duration exceeded the threshold set by `setSlowCallbackThresholdUs()`.
    /// Returns `0n` when the watchdog is disabled or before `startStreaming`.
    /// The value matches every other binding (C ABI, Python, C++). Returned
    /// as `bigint` for the full 64-bit unsigned range.
    #[napi(js_name = "slowCallbackCount")]
    pub fn slow_callback_count(&self) -> napi::bindgen_prelude::BigInt {
        napi::bindgen_prelude::BigInt::from(self.client.stream().slow_callback_count())
    }

    /// Snapshot of full-stream subscriptions (e.g. `OPTION` /
    /// `full_trades`, `OPTION` / `full_open_interest`).
    ///
    /// Each entry has the same `{ kind, contract }` shape returned by
    /// `activeSubscriptions()`, where `kind` is one of
    /// `"full_trades"` / `"full_open_interest"` and `contract` carries
    /// the wire-level security type (`"OPTION"`, `"STOCK"`, ...).
    /// Quote is never a valid full-stream kind on the streaming wire, so
    /// any such row from the core is dropped from the projection.
    /// Empty array when streaming has not started.
    #[napi(js_name = "activeFullSubscriptions")]
    pub fn active_full_subscriptions(&self) -> napi::Result<serde_json::Value> {
        self.client
            .stream()
            .active_full_subscriptions()
            .map(project_full_subscriptions)
            .map_err(to_napi_err)
    }
}

// ── Standalone HistoricalClient (historical-only) ──

/// Standalone historical-only client.
///
/// Opens ONLY the historical data channel and the Nexus authentication
/// flow — no real-time streaming connection or streaming state machine.
/// This lets a caller run a historical-only session alongside a parallel
/// streaming process without the unified `Client` taking over
/// the Nexus session at connect time.
///
/// The full historical / list / snapshot / at-time / flat-files surface
/// is identical to the unified client, so `historicalClient.stockHistoryEOD(...)`
/// behaves exactly like `client.stockHistoryEOD(...)`. The streaming and
/// subscription methods are simply not present: there is no
/// `startStreaming` / `subscribe` on this class, so a historical-only handle
/// cannot open a streaming slot. Use `StreamingClient` for streaming, or the
/// unified `Client` when you need both surfaces.
///
/// ```ts
/// import { HistoricalClient } from "thetadatadx";
/// const historical = await HistoricalClient.connectFromFile("creds.txt");
/// const eod = await historical.stockHistoryEOD("AAPL", "20240101", "20240301");
/// ```
#[napi]
pub struct HistoricalClient {
    /// Wrapped in `Arc` so the generated async endpoint methods can
    /// clone a cheap `'static` handle into the worker future, exactly
    /// like the unified client's `client` field. The generated method
    /// bodies reference `self.client`, so the historical impl block the
    /// codegen projects onto this class compiles unchanged. This client
    /// holds the same `thetadatadx::Client` core but never
    /// reaches its streaming methods — no streaming TLS slot is opened for a
    /// session that lives entirely through `HistoricalClient`.
    client: Arc<thetadatadx::Client>,
}

#[napi]
impl HistoricalClient {
    // Lifecycle: intentionally hand-written (language-specific constructor
    // semantics), mirroring the unified `Client` factories. The
    // connect core is identical — `thetadatadx::Client::connect`
    // opens the historical channel + Nexus and never opens streaming until a streaming method is
    // called, which this class does not surface.

    /// Connect to ThetaData with a `Credentials` handle and open the
    /// historical data channel. Historical only — this client never
    /// opens the streaming transport. Pass an optional `Config` to
    /// override the production-default endpoint. Use `StreamingClient` for
    /// real-time data.
    ///
    /// The config is snapshot at connect time: the `Config` handle may be
    /// reused or mutated afterward without affecting this client.
    ///
    /// `async` for the same reason as [`Client::connect`]: the channel open
    /// plus authentication handshake run off the libuv thread and the
    /// method returns a `Promise<HistoricalClient>`, so the Node event loop
    /// is never frozen for the handshake.
    #[napi]
    pub async fn connect(
        creds: &Credentials,
        config: Option<&Config>,
    ) -> napi::Result<HistoricalClient> {
        let cfg = config_or_production(config);
        let rt = runtime_from_config(&cfg.runtime)?;
        let creds = creds.inner.clone();
        let client = rt
            .spawn(async move { thetadatadx::Client::connect(&creds, cfg).await })
            .await
            .map_err(|e| napi::Error::from_reason(format!("connect task failed to complete: {e}")))?
            .map_err(to_napi_err)?;
        Ok(HistoricalClient {
            client: Arc::new(client),
        })
    }

    /// Connect with a credentials file (line 1 = email, line 2 =
    /// password). Convenience wrapper over `Credentials.fromFile` +
    /// `connect`. Historical only. Pass an optional
    /// `Config` to override the production-default endpoint.
    ///
    /// `async` for the same reason as [`HistoricalClient::connect`].
    #[napi(js_name = "connectFromFile")]
    pub async fn connect_from_file(
        path: String,
        config: Option<&Config>,
    ) -> napi::Result<HistoricalClient> {
        let client = connect_historical_from_file_core(path, config_or_production(config)).await?;
        Ok(HistoricalClient { client })
    }
}

// Generated historical endpoint methods. The codegen projects the same
// per-endpoint method bodies onto both `Client` and
// `HistoricalClient` (see `HISTORICAL_IMPL_CLASSES` in the TypeScript SDK
// emitter); both classes expose an `Arc<thetadatadx::Client>`
// field named `client`, so the shared bodies compile against either.
include!("_generated/historical_methods.rs");

// Generated streaming methods.
include!("_generated/streaming_methods.rs");

// `startStreaming(cb)` is the sole streaming entry point. Callers that
// want a for-await shape can wrap a queue inside the callback.

// SDK configuration class. Adds `Config` napi class with
// `production()` / `dev()` / `stage()` factories plus the full setter
// surface for historical pool sizing, retry policy, reconnect policy,
// and flat-file backoff.
mod config_class;
pub use config_class::Config;

// Hand-written FLATFILES bindings — dynamic schema, see module docs.
mod flatfile_methods;

// Hand-written support for the pull-based Arrow `RecordBatch` reader —
// the napi handle (`RecordBatchStreamHandle`) the generated
// `StreamView.batches(..)` entry returns. See module docs.
mod streaming_batches;
pub use streaming_batches::RecordBatchStreamHandle;

// Fluent contract-first API. Adds `ContractRef`,
// `Subscription`, `SecType` napi classes and the polymorphic
// `subscribe(sub)` / `subscribeMany([sub, ...])` methods on the
// unified client. The `Contract` name on the JS side is taken by the
// streaming-event payload object in `_generated/fpss_event_classes.rs`; the package
// `index.ts` re-exports the fluent class under both `ContractRef` and
// `Contract` so users write `Contract.stock("AAPL")` per the
// documented surface.
mod fluent;
pub use fluent::{ContractRef, SecType, Subscription};

// Standalone streaming-only client. Adds the `StreamingClient` napi class
// over `thetadatadx::fpss::StreamingClient` (the streaming primitive), mirroring the
// Python `StreamingClient` and the C++ `thetadatadx::StreamingClient`. It opens only the streaming
// TLS transport, no historical channel or Nexus, and drives its own dispatcher thread,
// routing events through the same `TsfnCallback` mechanism as the unified
// client's streaming surface.
mod fpss_client;
pub use fpss_client::StreamingClient;

#[napi]
impl StreamView {
    /// Polymorphic subscribe — primary fluent entry point. Accepts the
    /// `Subscription` value returned by `Contract.quote()` /
    /// `Contract.trade()` / `Contract.openInterest()` (per-contract
    /// scope) or by `SecType.option().fullTrades()` /
    /// `SecType.option().fullOpenInterest()` (full-stream scope).
    #[napi]
    pub fn subscribe(&self, sub: &fluent::Subscription) -> napi::Result<()> {
        self.client
            .stream()
            .subscribe(sub.snapshot())
            .map_err(to_napi_err)
    }

    /// Bulk-subscribe an array of `Subscription` values. Stops at the
    /// first error and returns it; previously-installed subscriptions
    /// are NOT rolled back.
    #[napi(js_name = "subscribeMany")]
    pub fn subscribe_many(&self, subs: Vec<&fluent::Subscription>) -> napi::Result<()> {
        let snaps: Vec<_> = subs.iter().map(|s| s.snapshot()).collect();
        self.client
            .stream()
            .subscribe_many(snaps)
            .map_err(to_napi_err)
    }

    /// Polymorphic unsubscribe — fluent counterpart to `subscribe(sub)`.
    #[napi]
    pub fn unsubscribe(&self, sub: &fluent::Subscription) -> napi::Result<()> {
        self.client
            .stream()
            .unsubscribe(sub.snapshot())
            .map_err(to_napi_err)
    }

    /// Bulk-unsubscribe an array of `Subscription` values.
    #[napi(js_name = "unsubscribeMany")]
    pub fn unsubscribe_many(&self, subs: Vec<&fluent::Subscription>) -> napi::Result<()> {
        let snaps: Vec<_> = subs.iter().map(|s| s.snapshot()).collect();
        self.client
            .stream()
            .unsubscribe_many(snaps)
            .map_err(to_napi_err)
    }
}

// `Client` is the public name (rename complete; no alias).

#[cfg(test)]
mod callback_queue_tests {
    use super::*;

    /// Recover the `MaxQueueSize` const generic from a concrete
    /// `ThreadsafeFunction` type so the test reads the bound that is
    /// actually compiled into [`TsfnCallback`], not a value re-typed by
    /// hand. A change to the alias that drops the seventh generic (back to
    /// the napi default of `0`, an unbounded queue) is observed here.
    const fn max_queue_size<
        T: 'static,
        Return: 'static + napi::bindgen_prelude::FromNapiValue,
        CallJsBackArgs: 'static + napi::bindgen_prelude::JsValuesTupleIntoVec,
        ErrorStatus: AsRef<str> + From<napi::Status>,
        const CALLEE_HANDLED: bool,
        const WEAK: bool,
        const MAX_QUEUE_SIZE: usize,
    >(
        _: std::marker::PhantomData<
            napi::threadsafe_function::ThreadsafeFunction<
                T,
                Return,
                CallJsBackArgs,
                ErrorStatus,
                CALLEE_HANDLED,
                WEAK,
                MAX_QUEUE_SIZE,
            >,
        >,
    ) -> usize {
        MAX_QUEUE_SIZE
    }

    /// The streaming callback queue must be bounded: a zero (unbounded)
    /// queue lets the `Blocking` call mode return without ever waiting, so
    /// a persistently slow JS callback grows the queue without limit while
    /// `ringOccupancy()` and `droppedEventCount()` stay flat. A finite
    /// bound is what couples a slow consumer back to the ring and the drop
    /// counter.
    #[test]
    fn streaming_callback_queue_is_bounded() {
        // Read the bound off the alias type rather than the bare constant
        // so the check fails if the seventh generic is dropped, even if the
        // constant itself is left untouched.
        let alias_depth = max_queue_size(std::marker::PhantomData::<TsfnCallback>);
        assert_eq!(
            alias_depth, STREAMING_CALLBACK_QUEUE_DEPTH,
            "TsfnCallback must carry STREAMING_CALLBACK_QUEUE_DEPTH as its MaxQueueSize"
        );
        assert_ne!(
            alias_depth, 0,
            "an unbounded (zero) queue defeats the Blocking back-pressure"
        );
        assert_eq!(
            alias_depth, 65_536,
            "the queue depth must match its documented value (one default ring)"
        );
    }
}

#[cfg(test)]
mod connect_options_tests {
    use super::*;

    /// Build a default (all-`None`) options object so each test sets only
    /// the fields it exercises.
    fn empty() -> ClientConnectOptions {
        ClientConnectOptions {
            api_key: None,
            api_key_from_env: None,
            api_key_from_dotenv: None,
            email: None,
            password: None,
            credentials_file: None,
            historical_type: None,
            streaming_type: None,
        }
    }

    #[test]
    fn api_key_inline_resolves_to_api_key_credentials() {
        let opts = ClientConnectOptions {
            api_key: Some("td1_example".to_string()),
            ..empty()
        };
        let (creds, cfg) = opts.resolve().expect("api_key resolves");
        assert!(creds.is_api_key());
        assert_eq!(creds.api_key_secret(), Some("td1_example"));
        assert_eq!(
            cfg.historical_environment(),
            config::HistoricalEnvironment::Prod
        );
        assert_eq!(
            cfg.streaming_environment(),
            config::StreamingEnvironment::Prod
        );
    }

    #[test]
    fn email_password_with_stage_resolves() {
        let opts = ClientConnectOptions {
            email: Some("You@Example.COM".to_string()),
            password: Some("hunter2".to_string()),
            historical_type: Some("STAGE".to_string()),
            ..empty()
        };
        let (creds, cfg) = opts.resolve().expect("email/password resolves");
        assert!(!creds.is_api_key());
        assert_eq!(creds.email(), Some("you@example.com"));
        // historicalType selects only the historical channel; streaming stays prod.
        assert_eq!(
            cfg.historical_environment(),
            config::HistoricalEnvironment::Stage
        );
        assert_eq!(
            cfg.streaming_environment(),
            config::StreamingEnvironment::Prod
        );
    }

    #[test]
    fn streaming_type_dev_selects_only_the_streaming_channel() {
        let opts = ClientConnectOptions {
            api_key: Some("td1_example".to_string()),
            streaming_type: Some("dev".to_string()),
            ..empty()
        };
        let (_creds, cfg) = opts.resolve().expect("streamingType resolves");
        // streamingType selects only the streaming channel; historical stays prod.
        assert_eq!(
            cfg.historical_environment(),
            config::HistoricalEnvironment::Prod
        );
        assert_eq!(
            cfg.streaming_environment(),
            config::StreamingEnvironment::Dev
        );
    }

    #[test]
    fn no_auth_field_is_an_error() {
        let msg = match empty().resolve() {
            Ok(_) => panic!("expected an error for an empty options object"),
            Err(e) => e.reason.clone(),
        };
        assert!(msg.contains("ConfigError"), "got: {msg}");
        assert!(msg.contains("no authentication field"), "got: {msg}");
    }

    #[test]
    fn conflicting_auth_fields_are_an_error() {
        let opts = ClientConnectOptions {
            api_key: Some("k".to_string()),
            email: Some("a@b.com".to_string()),
            password: Some("pw".to_string()),
            ..empty()
        };
        let msg = match opts.resolve() {
            Ok(_) => panic!("expected a conflict error"),
            Err(e) => e.reason.clone(),
        };
        assert!(msg.contains("ConfigError"), "got: {msg}");
        assert!(msg.contains("conflicting authentication"), "got: {msg}");
    }

    #[test]
    fn bad_historical_type_is_an_error() {
        let opts = ClientConnectOptions {
            api_key: Some("k".to_string()),
            historical_type: Some("nope".to_string()),
            ..empty()
        };
        let msg = match opts.resolve() {
            Ok(_) => panic!("expected an historicalType parse error"),
            Err(e) => e.reason.clone(),
        };
        assert!(msg.contains("historicalType"), "got: {msg}");
    }

    #[test]
    fn bad_streaming_type_is_an_error() {
        let opts = ClientConnectOptions {
            api_key: Some("k".to_string()),
            streaming_type: Some("nope".to_string()),
            ..empty()
        };
        let msg = match opts.resolve() {
            Ok(_) => panic!("expected an streamingType parse error"),
            Err(e) => e.reason.clone(),
        };
        assert!(msg.contains("streamingType"), "got: {msg}");
    }

    #[test]
    fn email_without_password_is_an_error() {
        let opts = ClientConnectOptions {
            email: Some("a@b.com".to_string()),
            ..empty()
        };
        assert!(opts.resolve().is_err());
    }
}
