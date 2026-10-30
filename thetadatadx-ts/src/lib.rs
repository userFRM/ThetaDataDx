//! TypeScript / Node.js bindings over the Rust `thetadatadx` core. Every call
//! crosses the napi-rs boundary into the same Rust code path used by the CLI
//! and FFI.

// The endpoint table expands through recursive macros whose depth grows with
// an endpoint's parameter count; the default limit of 128 is too shallow.
#![recursion_limit = "256"]

#[macro_use]
extern crate napi_derive;

use std::io;
use std::sync::{Arc, Mutex, OnceLock};

use napi::Either;
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
    serde_json::json!(
        subs.into_iter()
            .filter_map(|(kind, sec_type)| {
                let kind_str = match kind {
                    SubscriptionKind::Trade => "full_trades",
                    SubscriptionKind::OpenInterest => "full_open_interest",
                    SubscriptionKind::Quote => return None,
                    _ => return None,
                };
                Some(serde_json::json!({
                    "kind": kind_str,
                    "contract": sec_type.as_str(),
                }))
            })
            .collect::<Vec<_>>()
    )
}

#[cfg(test)]
mod full_subscription_projection_tests {
    use super::project_full_subscriptions;
    use thetadatadx::SecType;
    use thetadatadx::fpss::protocol::SubscriptionKind;

    /// `contract` carries the upper-case wire name `SecType.name` reports, so
    /// a caller can compare the two directly.
    #[test]
    fn contract_is_the_wire_security_type_name() {
        let projected = project_full_subscriptions(vec![
            (SubscriptionKind::Trade, SecType::Option),
            (SubscriptionKind::OpenInterest, SecType::Stock),
        ]);
        assert_eq!(
            projected,
            serde_json::json!([
                {"kind": "full_trades", "contract": "OPTION"},
                {"kind": "full_open_interest", "contract": "STOCK"},
            ])
        );
    }
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
/// import { Credentials, Client } from "thetadatadx-ts";
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
/// and the market-data-only [`MarketDataClient`]: the connect core is
/// identical (`thetadatadx::Client::connect` opens the market-data channel
/// and Nexus, never streaming until a streaming method is called); the two
/// factories differ only in the napi handle they wrap around the returned
/// `Arc`.
pub(crate) async fn connect_market_data_from_file_core(
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

/// The production defaults with the environment overrides applied.
///
/// An unrecognised `THETADATA_*_TYPE` selector or an out-of-range override is
/// returned as a typed error. The panicking `DirectConfig::production()` must
/// not be called from a napi entry point: a panic cannot unwind out of the
/// native callback, so it aborts the whole Node process.
pub(crate) fn production_config() -> napi::Result<config::DirectConfig> {
    config::DirectConfig::try_production().map_err(to_napi_err)
}

/// Snapshot an optional [`Config`] handle into an owned [`DirectConfig`],
/// falling back to the production default when none is supplied. The
/// snapshot decouples the client from later mutations of the `Config`
/// handle, matching the connect-time snapshot semantics every binding
/// shares.
pub(crate) fn config_or_production(config: Option<&Config>) -> napi::Result<config::DirectConfig> {
    match config {
        Some(c) => Ok(c.snapshot()),
        None => production_config(),
    }
}

/// Build a napi `Error` tagged as a `ConfigError` for a malformed
/// client-construction option (conflicting or absent auth fields, an
/// unparseable `marketDataType`). Matches the `[ConfigError]` prefix the JS
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
    /// Market-data environment selector (`"PROD"` / `"STAGE"`,
    /// case-insensitive). Defaults to production. The market-data and
    /// streaming channels are selected independently. For full host-level
    /// control, build a `Config` and use `Client.connect(creds, config)`.
    pub market_data_type: Option<String>,
    /// Streaming environment selector (`"PROD"` / `"STAGE"` / `"DEV"`,
    /// case-insensitive). Defaults to production. Selected independently of
    /// the market-data channel.
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
            market_data_type,
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

        // The market-data and streaming channels are selected
        // independently on top of the production defaults; either absent
        // keeps that channel on production. An unrecognized value is a
        // config error naming the valid set, never a silent fallback.
        let mut cfg = production_config()?;
        if let Some(raw) = market_data_type.as_deref() {
            let environment = config::MarketDataEnvironment::parse(raw).ok_or_else(|| {
                config_option_err(format!(
                    "marketDataType must be \"PROD\" or \"STAGE\" (case-insensitive); got {raw:?}"
                ))
            })?;
            cfg = cfg.with_market_data_environment(environment);
        }
        if let Some(raw) = streaming_type.as_deref() {
            let environment = config::StreamingEnvironment::parse(raw).ok_or_else(|| {
                config_option_err(format!(
                    "streamingType must be \"PROD\", \"STAGE\" or \"DEV\" (case-insensitive); got {raw:?}"
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

/// Validate an optional `u32` domain knob, leaving an omitted value
/// (`None`) untouched. The pinned-CPU / worker-thread setters take this
/// shape: `null` defers to the OS / default sizing, a number pins the
/// value (with `0` a valid choice — verbatim worker/core count).
pub(crate) fn validate_optional_u32_arg(name: &str, v: Option<f64>) -> napi::Result<Option<u32>> {
    v.map(|v| validate_u32_arg(name, v)).transpose()
}

#[cfg(test)]
mod numeric_arg_validation_tests {
    use super::validate_u32_arg;

    #[test]
    fn rejects_v8_touint32_footguns_and_accepts_valid() {
        // The bug this guards: a bare napi `u32`/`Option<u32>` runs V8
        // `ToUint32`, so `-1` reaches Rust as 4_294_967_295 (a catastrophic
        // pool size for `setMaxConcurrentRequests`). Taking `f64` + this
        // validator rejects it instead of silently wrapping.
        assert!(validate_u32_arg("maxConcurrentRequests", -1.0).is_err());
        assert!(validate_u32_arg("maxConcurrentRequests", 1.5).is_err());
        assert!(validate_u32_arg("maxConcurrentRequests", 4_294_967_296.0).is_err());
        assert!(validate_u32_arg("maxConcurrentRequests", f64::NAN).is_err());
        assert!(validate_u32_arg("maxConcurrentRequests", f64::INFINITY).is_err());
        assert_eq!(validate_u32_arg("maxConcurrentRequests", 8.0).unwrap(), 8);
        assert_eq!(validate_u32_arg("maxConcurrentRequests", 0.0).unwrap(), 0);
    }
}

/// Validate a JavaScript `timeoutMs` deadline and convert it to the
/// integer millisecond domain the Python, C++, and C ABI bindings take.
pub(crate) fn validate_timeout_ms(timeout_ms: f64) -> napi::Result<u64> {
    Ok(validate_nonneg_whole(
        "timeoutMs",
        timeout_ms,
        // MAX_SAFE_INTEGER: largest f64 with an exact u64. A u64::MAX bound
        // rounds to 2^64 as f64, so 2**64 would pass it and saturate on the cast.
        9_007_199_254_740_991.0,
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
/// Every generated market-data endpoint method is an `async fn`: napi-rs
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

/// Run an endpoint task whose failure is already a napi error.
///
/// Market-data stream callbacks use this path because JavaScript handler
/// exceptions must reject the returned Promise with the original napi error,
/// not be converted through the core `thetadatadx::Error` hierarchy.
pub(crate) async fn spawn_napi_task<F, T>(fut: F) -> napi::Result<T>
where
    F: std::future::Future<Output = napi::Result<T>> + Send + 'static,
    T: Send + 'static,
{
    match runtime()?.spawn(fut).await {
        Ok(inner) => inner,
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
        // FlatFiles availability, partial-reconnect, and partial
        // shard-fetch failures are streaming-surface faults; pin them to
        // `StreamError` so a `catch (e instanceof StreamError)` clause
        // behaves identically to the C++ and C ABI mapping (both route
        // these to the stream discriminant).
        thetadatadx::Error::FlatFilesUnavailable(_)
        | thetadatadx::Error::PartialReconnect { .. }
        | thetadatadx::Error::PartialShardFetch { .. } => "StreamError",
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

// Calendar dates and times are wire-format strings only (`"YYYYMMDD"` /
// `"HH:MM:SS"`). A JS `Date` is deliberately NOT accepted: it is an instant,
// not a calendar day, so converting it forced a UTC rendering that shifted the
// requested day in non-UTC time zones. These pass the string through as the
// single normalization seam every date/time argument routes through.
fn normalize_date(value: String) -> String {
    value
}

fn normalize_time(value: String) -> String {
    value
}

fn normalize_optional_date(value: Option<String>) -> Option<String> {
    value
}

fn normalize_optional_time(value: Option<String>) -> Option<String> {
    value
}

/// Expands `__enums!` into napi string enums. napi renders a doc line only
/// when it is one string literal, so `__paste!` joins it.
macro_rules! ts_string_enums {
    ($($name:ident { $($variant:ident $wire:tt $py:ident [$($alias:tt)*]),+ $(,)? })*) => {
        $(thetadatadx::__paste! {
            #[doc = "Wire string enum `" $name "`."]
            #[napi(string_enum)]
            pub enum $name {
                $(
                    #[napi(value = $wire)]
                    $variant,
                )+
            }
        })*
    };
}

thetadatadx::__enums! { ts_string_enums {} }

// ── Tick row objects, expanded from the core tick table ──
//
// `#[napi(object)]` structs for every tick type plus `{tick}_to_class_vec`
// factories and the Arrow IPC functions. These back every market-data
// endpoint return so `index.d.ts` surfaces concrete `Tick[]` types instead of
// `any`. `BigInt` carries their 64-bit fields and those of the streaming event
// classes below.

use napi::bindgen_prelude::BigInt;

mod ticks;
pub use ticks::*;

// ── Typed streaming events, expanded from the core event table ──

mod stream_events;
use stream_events::{StreamEvent, buffered_event_to_typed, fpss_event_to_buffered};

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

// ── Cross-language lookup-table helpers ──

/// Cross-language lookup-table namespace. Exposes the static condition,
/// exchange, calendar, timestamp, and sequence helpers as `Util.*` static
/// methods so the JS surface mirrors the Python / C++ / C ABI utility sets.
#[napi(js_name = "Util")]
pub struct Util;

#[napi]
impl Util {
    /// Symbolic name for a trade `condition` code (e.g. `0` -> `"REGULAR"`).
    /// Returns `"UNKNOWN"` for codes outside the table.
    #[napi(js_name = "conditionName")]
    pub fn condition_name(code: i32) -> String {
        thetadatadx::utils::conditions::condition_name(code).to_string()
    }

    /// Human-readable description for a trade `condition` code.
    #[napi(js_name = "conditionDescription")]
    pub fn condition_description(code: i32) -> String {
        thetadatadx::utils::conditions::condition_description(code).to_string()
    }

    /// Whether a trade `condition` code marks a trade cancellation.
    #[napi(js_name = "isCancel")]
    pub fn is_cancel(code: i32) -> bool {
        thetadatadx::utils::conditions::is_cancel(code)
    }

    /// Whether a trade with this `condition` code contributes to the
    /// running session volume.
    #[napi(js_name = "updatesVolume")]
    pub fn updates_volume(code: i32) -> bool {
        thetadatadx::utils::conditions::updates_volume(code)
    }

    /// Symbolic name for a quote `condition` code.
    #[napi(js_name = "quoteConditionName")]
    pub fn quote_condition_name(code: i32) -> String {
        thetadatadx::utils::conditions::quote_condition_name(code).to_string()
    }

    /// Human-readable description for a quote `condition` code.
    #[napi(js_name = "quoteConditionDescription")]
    pub fn quote_condition_description(code: i32) -> String {
        thetadatadx::utils::conditions::quote_condition_description(code).to_string()
    }

    /// Whether a quote `condition` code marks a firm (binding) quote.
    #[napi(js_name = "isFirm")]
    pub fn is_firm(code: i32) -> bool {
        thetadatadx::utils::conditions::is_firm(code)
    }

    /// Whether a quote `condition` code marks a trading halt.
    #[napi(js_name = "isHalted")]
    pub fn is_halted(code: i32) -> bool {
        thetadatadx::utils::conditions::is_halted(code)
    }

    /// Symbolic name for an `exchange` code (e.g. `3` ->
    /// `"NewYorkStockExchange"`).
    #[napi(js_name = "exchangeName")]
    pub fn exchange_name(code: i32) -> String {
        thetadatadx::utils::exchange::exchange_name(code).to_string()
    }

    /// Short exchange symbol for an `exchange` code (e.g. `3` ->
    /// `"NYSE"`).
    #[napi(js_name = "exchangeSymbol")]
    pub fn exchange_symbol(code: i32) -> String {
        thetadatadx::utils::exchange::exchange_symbol(code).to_string()
    }

    /// Vendor vocabulary text for a calendar-day `status` code (`0` ->
    /// `"open"`, `1` -> `"early_close"`, `2` -> `"full_close"`, `3` ->
    /// `"weekend"`). Returns the literal `"UNKNOWN"` for codes outside
    /// the table.
    #[napi(js_name = "calendarStatusName")]
    pub fn calendar_status_name(code: i32) -> String {
        thetadatadx::CalendarStatus::from_code(code)
            .map_or("UNKNOWN", thetadatadx::CalendarStatus::as_str)
            .to_string()
    }

    /// Combine an Eastern-Time `YYYYMMDD` date and milliseconds-of-day
    /// into Unix epoch milliseconds (UTC, DST-aware) as a JS BigInt.
    /// Usable with any `(date, *_ms_of_day)` pair on the tick structs.
    /// Returns `null` when `date` is absent (`0`) or either input is out
    /// of domain. BigInt matches the `*TimestampMs` tick accessors so the
    /// epoch domain is uniform.
    #[napi(js_name = "timestampMs")]
    pub fn timestamp_ms(date: i32, ms_of_day: i32) -> Option<napi::bindgen_prelude::BigInt> {
        thetadatadx::time::date_ms_to_epoch_ms(date, ms_of_day)
            .map(napi::bindgen_prelude::BigInt::from)
    }

    /// Convert a signed wire-encoded trade-sequence value to its unsigned
    /// monotonic form. Accepts a JS BigInt in the **32-bit signed wire
    /// range** (`-2_147_483_648 ..= 2_147_483_647`); the upstream feed
    /// encodes trade sequences as a 32-bit signed integer. Returns a JS
    /// BigInt because the unsigned monotonic sequence id can exceed
    /// `Number.MAX_SAFE_INTEGER`. Inputs outside the wire range throw so
    /// silent coercion cannot produce a look-correct-but-wrong sequence id
    /// downstream.
    #[napi(js_name = "sequenceSignedToUnsigned")]
    pub fn sequence_signed_to_unsigned(
        signed_value: napi::bindgen_prelude::BigInt,
    ) -> napi::Result<napi::bindgen_prelude::BigInt> {
        let signed: i64 = bigint_to_i32(&signed_value).map(i64::from).ok_or_else(|| {
            crate::invalid_parameter_err(
                "sequenceSignedToUnsigned: BigInt outside the i32 wire range \
                 (-2_147_483_648 ..= 2_147_483_647)",
            )
        })?;
        Ok(napi::bindgen_prelude::BigInt::from(
            thetadatadx::utils::sequences::signed_to_unsigned(signed),
        ))
    }

    /// Convert an unsigned monotonic trade-sequence value back to its
    /// signed wire encoding. Accepts a JS BigInt in the unsigned wire
    /// range (`0 ..= 2^32 - 1`); returns a JS BigInt for symmetry with
    /// `sequenceSignedToUnsigned`. Negative inputs and inputs above the
    /// wire range throw; the unsigned monotonic sequence id is always
    /// non-negative and never wider than the 32-bit wire range.
    #[napi(js_name = "sequenceUnsignedToSigned")]
    pub fn sequence_unsigned_to_signed(
        unsigned_value: napi::bindgen_prelude::BigInt,
    ) -> napi::Result<napi::bindgen_prelude::BigInt> {
        if unsigned_value.sign_bit && !unsigned_value.words.iter().all(|w| *w == 0) {
            return Err(crate::invalid_parameter_err(
                "sequenceUnsignedToSigned: negative BigInt rejected; the unsigned \
                 monotonic sequence id is always non-negative",
            ));
        }
        if unsigned_value.words.len() > 1 {
            return Err(crate::invalid_parameter_err(
                "sequenceUnsignedToSigned: BigInt above the wire range \
                 (0 ..= 2^32 - 1)",
            ));
        }
        let value = unsigned_value.words.first().copied().unwrap_or(0);
        if value > u32::MAX as u64 {
            return Err(crate::invalid_parameter_err(
                "sequenceUnsignedToSigned: BigInt above the wire range \
                 (0 ..= 2^32 - 1)",
            ));
        }
        Ok(napi::bindgen_prelude::BigInt::from(
            thetadatadx::utils::sequences::unsigned_to_signed(value),
        ))
    }
}

/// Decode a napi `BigInt` into the i32 wire range, accepting the
/// asymmetric `i32::MIN` boundary. Returns `None` for any value
/// outside `[i32::MIN, i32::MAX]`.
fn bigint_to_i32(value: &napi::bindgen_prelude::BigInt) -> Option<i32> {
    if value.words.len() > 1 {
        return None;
    }
    let magnitude = value.words.first().copied().unwrap_or(0);
    if value.sign_bit {
        if magnitude == 0 {
            Some(0)
        } else if magnitude <= i32::MAX as u64 {
            // SAFETY: `magnitude` fits in i32 here.
            Some(-(magnitude as i32))
        } else if magnitude == (i32::MAX as u64) + 1 {
            Some(i32::MIN)
        } else {
            None
        }
    } else {
        i32::try_from(magnitude).ok()
    }
}

// ── Unified Client client ──

/// Bound on the number of `StreamEvent` deliveries that may sit in the
/// napi callback queue between the streaming consumer thread and the Node
/// main thread before the consumer is made to wait.
///
/// This queue is the second buffer on the delivery path. The first is the
/// streaming event ring (the `streamingRingSize` setting, 131072 slots by
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
/// The depth is half the default ring size, so a healthy callback has ample
/// headroom before the consumer ever waits, while a wedged callback can pin
/// at most this many in-flight events.
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
    /// Held as `Option` behind a `Mutex` so `close()` can DETERMINISTICALLY
    /// RELEASE the handle: closing takes the `Arc<Client>` out of the slot and
    /// drops it, freeing the market-data gRPC channel pool once the last
    /// co-owning surface (a `MarketDataView` / `StreamView` vended before
    /// close) is also gone — matching the C++ wrapper, where `close()` resets
    /// the handle. A closed slot (`None`) makes every access via
    /// [`Client::client_handle`] reject with "client is closed", so the client
    /// is UNUSABLE after close and a second close is a no-op.
    ///
    /// The inner `thetadatadx::Client` is not `Clone` -- its streaming mutex
    /// and subscription-tier state forbid that -- so surfaces co-own a cheap
    /// `Arc<Client>` clone (surviving past a parent close, exactly as the C++
    /// `MarketData` / `Stream` views co-own the `shared_ptr`).
    client: Mutex<Option<Arc<thetadatadx::Client>>>,
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

/// User-facing market-data sub-namespace returned by the
/// `client.marketData` getter.
///
/// A lightweight handle that shares the underlying client connection;
/// constructing it performs no auth round-trip and mutates no streaming
/// state. Every market-data endpoint method is generated onto this view
/// from a single declarative surface definition, so the surface stays a
/// single generated source of truth.
#[napi]
pub struct MarketDataView {
    client: Arc<thetadatadx::Client>,
}

impl MarketDataView {
    /// The core client handle for the generated endpoint bodies.
    ///
    /// The view co-owns its `Arc<Client>`, so it stays usable even after the
    /// parent `Client` closes — matching the C++ `MarketData` view, which
    /// co-owns the `shared_ptr`. Infallible; the `napi::Result` signature is
    /// shared with `MarketDataClient::client_handle` so the identical generated
    /// bodies compile against either receiver.
    pub(crate) fn client_handle(&self) -> napi::Result<Arc<thetadatadx::Client>> {
        Ok(Arc::clone(&self.client))
    }
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
    /// The Node main thread the `stream` getter ran on, which drains the
    /// callback queue. See `fpss_client::abort_hook_expect_closing`.
    js_thread: std::thread::ThreadId,
}

#[napi]
impl Client {
    /// Market-data sub-namespace: `client.marketData.stockHistoryEOD(...)`.
    ///
    /// Returns a fresh [`MarketDataView`] that shares the underlying
    /// client connection. No auth round-trip, no streaming-state mutation.
    #[napi(getter)]
    pub fn market_data(&self) -> napi::Result<MarketDataView> {
        Ok(MarketDataView {
            client: self.client_handle()?,
        })
    }

    /// Real-time-streaming sub-namespace: `client.stream.subscribe(...)`,
    /// `client.stream.startStreaming(cb)`, …
    ///
    /// Returns a fresh [`StreamView`] sharing the inner client and the
    /// parent's callback slot, so the streaming lifecycle observed through
    /// the view is the one the unified client manages.
    #[napi(getter)]
    pub fn stream(&self) -> napi::Result<StreamView> {
        Ok(StreamView {
            client: self.client_handle()?,
            callback: Arc::clone(&self.callback),
            js_thread: std::thread::current().id(),
        })
    }

    /// The live core client handle, or a "client is closed" error.
    ///
    /// Every surface the client vends (`marketData` / `stream` / `flatFiles`)
    /// resolves the handle through here, so once `close()` has taken it the
    /// client is uniformly unusable. Returns a cheap `Arc` clone the caller
    /// co-owns.
    pub(crate) fn client_handle(&self) -> napi::Result<Arc<thetadatadx::Client>> {
        self.client
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
            .ok_or_else(|| {
                napi::Error::from_reason(
                    "client is closed; construct a new client to make further calls",
                )
            })
    }
}

#[napi]
impl Client {
    // Lifecycle: intentionally hand-written (language-specific constructor semantics).

    /// Connect to ThetaData with a `Credentials` handle. Pass an
    /// optional `Config` (`dev` / `stage` / `production`, plus any
    /// tuned setters) to override the production-default endpoint.
    /// Market-data only; call `client.stream.startStreaming(...)` to
    /// begin streaming real-time data.
    ///
    /// The config is snapshot at connect time: the `Config` handle may be
    /// reused or mutated afterward without affecting this client.
    ///
    /// ```ts
    /// import { Credentials, Client } from "thetadatadx-ts";
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
        let cfg = config_or_production(config)?;
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
            client: Mutex::new(Some(Arc::new(client))),
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
        let client =
            connect_market_data_from_file_core(path, config_or_production(config)?).await?;
        Ok(Client {
            client: Mutex::new(Some(client)),
            callback: Arc::new(Mutex::new(None)),
        })
    }

    /// Connect with the authentication and environment selected inline via
    /// an options object, with the API key as a first-class, directly-passed
    /// field.
    ///
    /// ```js
    /// const staged = await Client.connectWith({ apiKey: "td1_...", marketDataType: "STAGE" });
    /// const withLogin = await Client.connectWith({ email: "u@e.com", password: "secret" });
    /// const fromEnv = await Client.connectWith({ apiKeyFromEnv: true });
    /// ```
    ///
    /// Exactly one authentication field must be set: `apiKey`,
    /// `apiKeyFromEnv`, `apiKeyFromDotenv`, the `email` + `password` pair,
    /// or `credentialsFile`. Passing none, or two different ones, rejects
    /// with a `ConfigError` before any network round-trip. `marketDataType`
    /// (`"PROD"` / `"STAGE"`, case-insensitive) selects the market-data
    /// environment and `streamingType` (`"PROD"` / `"STAGE"` / `"DEV"`, case-insensitive)
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
            client: Mutex::new(Some(Arc::new(client))),
            callback: Arc::new(Mutex::new(None)),
        })
    }

    /// Deterministically close the client.
    ///
    /// Stops streaming if it is live (idempotent), RELEASES the core client
    /// handle, and releases the registered callback back to V8. Taking the
    /// handle out of its slot and dropping it frees the market-data gRPC channel
    /// pool once no vended surface still co-owns it, and makes the client
    /// UNUSABLE — every subsequent `marketData` / `stream` / `flatFiles` access
    /// rejects with "client is closed". Safe to call more than once (a second
    /// close finds an empty slot and is a no-op) and safe on a client that only
    /// ran market-data queries.
    ///
    /// This is the recommended teardown. Prefer the `using` declaration
    /// (`using client = connect(...)`) so `close()` runs on scope exit through
    /// `[Symbol.dispose]`; for a full streaming-drain barrier use the
    /// `[Symbol.asyncDispose]` pairing (`stopStreaming()` + `awaitDrain`) the
    /// context-managed session exposes.
    ///
    /// `close()` calls `stopStreaming()` synchronously, which retires the
    /// dispatcher by joining it, so `close()` can block briefly while the
    /// dispatcher finishes a callback already in flight. It does NOT run the
    /// `awaitDrain` ring barrier that the `[Symbol.asyncDispose]` path runs; if
    /// you need the full drain before release, use the streaming session's
    /// async disposer. Dropping the taken handle then runs the core
    /// `Client::Drop` (the detached streaming quiesce), which returns
    /// immediately and finishes on a helper thread. Callers wanting a
    /// non-blocking release let the handle drop instead of calling `close()`.
    #[napi]
    pub fn close(&self) {
        // Take the handle: idempotent. A second close finds `None`.
        let Some(client) = self.client.lock().unwrap_or_else(|e| e.into_inner()).take() else {
            return;
        };
        // Stop streaming synchronously on the calling thread, then release the
        // stored callback. Dropping `client` at scope end frees the pool and
        // runs the detached core `Drop`.
        client.stream().stop_streaming();
        let mut guard = self.callback.lock().unwrap_or_else(|e| e.into_inner());
        *guard = None;
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
    /// The value matches every other binding (C ABI, Python, C++), and it
    /// counts every session this client has run: `stopStreaming()` and
    /// `reconnect()` retire the session, and the drops it recorded stay in
    /// the total.
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

// ── Standalone MarketDataClient (market-data-only) ──

/// Standalone market-data-only client.
///
/// Opens ONLY the historical data channel and the Nexus authentication
/// flow — no real-time streaming connection or streaming state machine.
/// This lets a caller run a market-data-only session alongside a parallel
/// streaming process without the unified `Client` taking over
/// the Nexus session at connect time.
///
/// The full historical / list / snapshot / at-time / flat-files surface
/// is identical to the unified client, so `marketDataClient.stockHistoryEOD(...)`
/// behaves exactly like `client.marketData.stockHistoryEOD(...)`. The streaming and
/// subscription methods are simply not present: there is no
/// `startStreaming` / `subscribe` on this class, so a market-data-only handle
/// cannot open a streaming slot. Use `StreamingClient` for streaming, or the
/// unified `Client` when you need both surfaces.
///
/// ```ts
/// import { MarketDataClient } from "thetadatadx-ts";
/// const marketData = await MarketDataClient.connectFromFile("creds.txt");
/// const eod = await marketData.stockHistoryEOD("AAPL", "20240101", "20240301");
/// ```
#[napi]
pub struct MarketDataClient {
    /// Held as `Option` behind a `Mutex` (like the unified `Client`) so
    /// `close()` DETERMINISTICALLY RELEASES the handle: closing takes the
    /// `Arc<Client>` out and drops it, freeing the market-data gRPC channel pool,
    /// and makes the client UNUSABLE — the generated endpoint bodies resolve the
    /// handle through [`MarketDataClient::client_handle`], which rejects with
    /// "client is closed" once the slot is empty. A second close is a no-op.
    client: Mutex<Option<Arc<thetadatadx::Client>>>,
}

impl MarketDataClient {
    /// The live core client handle for the generated endpoint bodies, or a
    /// "client is closed" error. Shares its signature with
    /// [`MarketDataView::client_handle`] so the identical generated bodies
    /// compile against either receiver.
    pub(crate) fn client_handle(&self) -> napi::Result<Arc<thetadatadx::Client>> {
        self.client
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
            .ok_or_else(|| {
                napi::Error::from_reason(
                    "client is closed; construct a new client to make further calls",
                )
            })
    }
}

#[napi]
impl MarketDataClient {
    // Lifecycle: intentionally hand-written (language-specific constructor
    // semantics), mirroring the unified `Client` factories. The
    // connect core is identical — `thetadatadx::Client::connect`
    // opens the market-data channel + Nexus and never opens streaming until a streaming method is
    // called, which this class does not surface.

    /// Connect to ThetaData with a `Credentials` handle and open the
    /// historical data channel. Market-data only — this client never
    /// opens the streaming transport. Pass an optional `Config` to
    /// override the production-default endpoint. Use `StreamingClient` for
    /// real-time data.
    ///
    /// The config is snapshot at connect time: the `Config` handle may be
    /// reused or mutated afterward without affecting this client.
    ///
    /// `async` for the same reason as [`Client::connect`]: the channel open
    /// plus authentication handshake run off the libuv thread and the
    /// method returns a `Promise<MarketDataClient>`, so the Node event loop
    /// is never frozen for the handshake.
    #[napi]
    pub async fn connect(
        creds: &Credentials,
        config: Option<&Config>,
    ) -> napi::Result<MarketDataClient> {
        let cfg = config_or_production(config)?;
        let rt = runtime_from_config(&cfg.runtime)?;
        let creds = creds.inner.clone();
        let client = rt
            .spawn(async move { thetadatadx::Client::connect(&creds, cfg).await })
            .await
            .map_err(|e| napi::Error::from_reason(format!("connect task failed to complete: {e}")))?
            .map_err(to_napi_err)?;
        Ok(MarketDataClient {
            client: Mutex::new(Some(Arc::new(client))),
        })
    }

    /// Connect with a credentials file (line 1 = email, line 2 =
    /// password). Convenience wrapper over `Credentials.fromFile` +
    /// `connect`. Market-data only. Pass an optional
    /// `Config` to override the production-default endpoint.
    ///
    /// `async` for the same reason as [`MarketDataClient::connect`].
    #[napi(js_name = "connectFromFile")]
    pub async fn connect_from_file(
        path: String,
        config: Option<&Config>,
    ) -> napi::Result<MarketDataClient> {
        let client =
            connect_market_data_from_file_core(path, config_or_production(config)?).await?;
        Ok(MarketDataClient {
            client: Mutex::new(Some(client)),
        })
    }

    /// Deterministically close the market-data client.
    ///
    /// The market-data-only surface never opens streaming, so there is no
    /// dispatcher to drain; closing takes the core client handle out of its slot
    /// and drops it, RELEASING the gRPC channel pool once no vended surface still
    /// co-owns it and making the client UNUSABLE (every endpoint call rejects
    /// with "client is closed"). Matches the unified `Client` lifecycle across
    /// every binding. Idempotent — a second close finds an empty slot. Prefer the
    /// `using` declaration (`using c = await MarketDataClient.connect(...)`) so
    /// `close()` runs on scope exit through `[Symbol.dispose]`.
    #[napi]
    pub fn close(&self) {
        // Take the handle out of its slot and drop it (idempotent). Dropping the
        // last co-owning `Arc` frees the channel pool and runs the detached core
        // `Drop`. A market-data-only client has no streaming dispatcher to stop.
        let _ = self.client.lock().unwrap_or_else(|e| e.into_inner()).take();
    }
}

// Market-data endpoint methods, expanded from the core endpoint table onto
// both `MarketDataView` and `MarketDataClient`; each class resolves its
// `Arc<thetadatadx::Client>` through `client_handle()`, so the shared
// bodies compile against either.
mod market_data;

#[napi]
impl StreamView {
    /// Start streaming and register a JS callback for incoming events.
    ///
    /// Each typed streaming event is delivered to your
    /// `callback(event)` on the Node main thread, so the
    /// callback may use any JS API safely. Rust-side delivery
    /// panics are isolated and counted by `panicCount()`; a
    /// JavaScript exception follows Node's normal exception
    /// handling.
    ///
    /// Backpressure: a slow callback first fills a bounded
    /// delivery queue and then the event ring behind it, at
    /// which point the newest incoming events are dropped and counted by
    /// `droppedEventCount()` while `ringOccupancy()` reports the
    /// in-flight depth. Watch those two signals to detect a
    /// callback that cannot keep up. The receive path is never
    /// blocked by a slow callback, so the upstream connection
    /// stays healthy regardless of callback speed.
    #[napi(js_name = "startStreaming")]
    pub async fn start_streaming(
        &self,
        callback: napi::threadsafe_function::ThreadsafeFunction<
            StreamEvent,
            (),
            StreamEvent,
            napi::Status,
            false,
            false,
            { crate::STREAMING_CALLBACK_QUEUE_DEPTH },
        >,
    ) -> napi::Result<()> {
        // Bind the callback behind a cheap `Arc` so the FPSS callback
        // closure (`Fn(&StreamEvent) + Send + 'static`) can clone the
        // handle into each per-event invocation. `ThreadsafeFunction`
        // is `Send + Sync` but does not implement `Clone` in napi-rs
        // 3.x; the outer `Arc` is the canonical way to share the
        // handle between the closure and the stored copy on `self`.
        let callback_arc: Arc<TsfnCallback> = Arc::new(callback);
        {
            // Reserve the registration slot before yielding to the
            // blocking handshake. Holding the `Mutex<Option<Arc<...>>>`
            // guard across an `.await` is not allowed (the guard is
            // `!Send`), so we take the lock in this inner scope, reject a
            // double-registration, and store the handle eagerly. A
            // handshake failure below clears the slot again. This keeps
            // the double-registration check correct even though two
            // `startStreaming` calls could otherwise both pass it while
            // the first is still awaiting.
            let mut cb_guard = self.callback.lock().unwrap_or_else(|e| e.into_inner());
            // Reject double-registration with a clear napi error. The
            // underlying `Client::start_streaming` would also error,
            // but matching the napi binding's own state first gives a
            // language-idiomatic message.
            if cb_guard.is_some() {
                return Err(napi::Error::from_reason(
                    "streaming already started -- call stopStreaming() before startStreaming() again",
                ));
            }
            *cb_guard = Some(Arc::clone(&callback_arc));
        }
        // Own the reservation until the handshake succeeds. On any non-success
        // exit -- the panic branch or the handshake `Err` below -- the guard
        // clears the slot, but ONLY if it still holds THIS reservation
        // (`Arc::ptr_eq`). The lock is released across the awaited connect, so a
        // concurrent stop + restart may have replaced the slot with a newer
        // callback that must keep its registration; clearing unconditionally
        // would strand that live session with no registration. Disarmed on
        // success.
        let mut reservation =
            crate::fpss_client::CallbackReservation::armed(&self.callback, &callback_arc);
        let dispatch_cb = Arc::clone(&callback_arc);
        let callback_closing_expected = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let delivery_failed = Arc::new(std::sync::atomic::AtomicBool::new(false));

        // Teardown wake hook. The dispatcher does NOT park only on the
        // event ring: once the bounded callback queue is full it blocks inside
        // the `Blocking` `dispatch_cb.call(...)` below, waiting for the Node
        // main thread to drain the queue. On `stopStreaming()` the main thread
        // is itself inside the dispatcher join, so it can never drain the queue,
        // so the consumer would block forever and the join would hang. The hook
        // aborts the threadsafe function so the in-flight `Blocking` call
        // returns `Status::Closing`, the consumer resumes, observes the client
        // shutdown, exits `for_each_scoped`, and the join completes. It is
        // built from `callback_arc` (a shared-handle clone, NOT the stored
        // slot) so installing it consumes nothing the dispatcher needs.
        let teardown_hook = crate::fpss_client::abort_hook_expect_closing(
            &callback_arc,
            Arc::clone(&callback_closing_expected),
            self.js_thread,
        );

        // The FPSS connect and authentication handshake are network-bound
        // and run synchronously inside `start_streaming`. Move that work
        // onto a blocking worker so the single libuv thread stays free to
        // service timers, IO, and queued promises for the whole handshake.
        // The dispatcher closure and the `Arc<thetadatadx::Client>` are
        // both `Send + 'static`, so the closure satisfies the
        // `spawn_blocking` bound.
        let client = Arc::clone(&self.client);
        let callback_closing_expected_for_dispatch = Arc::clone(&callback_closing_expected);
        let delivery_failed_for_dispatch = Arc::clone(&delivery_failed);
        let delivery_failed_for_scope = Arc::clone(&delivery_failed);
        let join_result = tokio::task::spawn_blocking(move || {
            client.stream().start_streaming_scoped(
                move |event: &fpss::StreamEvent| {
                    // Convert to the typed `StreamEvent` napi object on the
                    // dispatcher thread, then hand the value
                    // to `ThreadsafeFunction::call`. napi-rs' internal
                    // `uv_async_t` queue routes the call onto the Node
                    // main thread, which is the only thread allowed to
                    // execute V8 (libuv invariant). The FPSS TLS reader
                    // thread itself never touches V8: the streaming ring
                    // sits between the reader and the consumer that runs
                    // this closure.
                    let buffered = fpss_event_to_buffered(event);
                    let typed = buffered_event_to_typed(buffered);
                    // `Blocking`, paired with the bounded
                    // `STREAMING_CALLBACK_QUEUE_DEPTH` on `TsfnCallback`,
                    // makes this consumer wait once the call queue is full
                    // rather than parking an unbounded backlog of events
                    // behind a slow JS callback. While the consumer waits it
                    // stops draining the ring, so the ring fills and the FPSS
                    // reader accounts the overflow on `droppedEventCount()`;
                    // the reader itself is never blocked, so the upstream
                    // connection stays healthy.
                    //
                    // `ErrorStrategy::Fatal` (the fifth `false` const generic
                    // on `TsfnCallback`) means we pass `T` directly, not
                    // `Result<T, _>`; exceptions in the JS callback are
                    // the JS side's problem, surfaced through Node's own
                    // `uncaughtException`.
                    let status = dispatch_cb.call(
                        typed,
                        napi::threadsafe_function::ThreadsafeFunctionCallMode::Blocking,
                    );
                    crate::fpss_client::note_callback_delivery_status(
                        status,
                        &callback_closing_expected_for_dispatch,
                        &delivery_failed_for_dispatch,
                    );
                },
                move |drain| {
                    crate::fpss_client::callback_delivery_outcome(
                        drain(),
                        &delivery_failed_for_scope,
                    )
                },
                Some(teardown_hook),
            )
        })
        .await;

        let result = match join_result {
            Ok(result) => result,
            Err(e) => {
                // The connect task itself panicked. The `reservation` clears
                // the slot on drop, but only when this start still owns it, so
                // a concurrent stop + restart's newer registration survives.
                return Err(napi::Error::from_reason(format!(
                    "start_streaming task panicked: {e}"
                )));
            }
        };

        if let Err(e) = result {
            // The handshake failed -- the `reservation` clears the slot on
            // drop, but only when this start still owns it.
            return Err(to_napi_err(e));
        }
        // Connect succeeded: hand the reservation off to the live session so the
        // guard does not clear it on return. No post-connect ownership re-check
        // is needed: the core holds its dispatcher lock from before the connect
        // through the install, and every core teardown takes that lock, so a
        // concurrent stop runs either wholly before the connect, which then
        // installs normally, or after the install, retiring the fresh session.
        // Either way the core is left consistent, so the binding must not
        // itself call the shared `stopStreaming` here -- doing so would tear down
        // a DIFFERENT session a concurrent restart had already installed. The
        // standalone client shuts down its OWN unpublished client on
        // supersession; the unified client has no own client to discard, only
        // the shared core the gate governs.
        reservation.disarm();
        Ok(())
    }

    /// Whether the streaming connection is active.
    #[napi(js_name = "isStreaming")]
    pub fn is_streaming(&self) -> bool {
        self.client.stream().is_streaming()
    }

    /// Open a pull-based columnar reader over the live stream, a sibling to the per-event callback. Returns a reader of Apache Arrow record batches under a fixed schema; the same subscriptions feed it. Tune batch_size, linger, and backpressure on the returned builder/reader.
    /// Open a pull-based columnar reader over the live stream.
    ///
    /// Returns a reader handle, a sibling to the per-event
    /// `startStreaming(callback)`. The same subscriptions feed it,
    /// but market-data events arrive as apache-arrow `RecordBatch`
    /// values under a fixed schema, consumed with `for await`. The
    /// reader closes (unsubscribes + tears down) on `close()` or
    /// `Symbol.asyncDispose`. Open the reader first (that starts the
    /// session), then subscribe on this same surface.
    ///
    /// `batchSize` rows per batch (default 65536); `lingerMs`
    /// flushes a partial batch on a quiet stream (default 50);
    /// `backpressure` is `"block"` (default; no queue-side drops) or
    /// `"dropOldest"`; `capacity` bounds the drop-oldest buffer.
    #[napi(js_name = "batches", skip_typescript)]
    pub async fn batches(
        &self,
        options: Option<crate::streaming_batches::BatchesOptions>,
    ) -> napi::Result<crate::streaming_batches::RecordBatchStreamHandle> {
        let options = options.unwrap_or_default();
        crate::streaming_batches::open_handle(
            std::sync::Arc::clone(&self.client),
            options.batch_size,
            options.linger_ms,
            options.backpressure,
            options.capacity,
        )
        .await
    }

    /// Get a snapshot of currently active subscriptions.
    #[napi(js_name = "activeSubscriptions")]
    pub fn active_subscriptions(&self) -> napi::Result<serde_json::Value> {
        self.client
            .stream()
            .active_subscriptions()
            .map(|subs| {
                serde_json::json!(subs.into_iter()
                    .map(|(kind, contract)| {
                        serde_json::json!({ "kind": format!("{kind:?}"), "contract": format!("{contract}") })
                    })
                    .collect::<Vec<_>>())
            })
            .map_err(to_napi_err)
    }

    /// Reconnect streaming and re-register the previously installed callback.
    ///
    /// Requires a prior `startStreaming(callback)`; throws if
    /// no callback is registered. All active subscriptions are
    /// restored on the new connection. If some subscriptions
    /// cannot be restored, the reconnect still completes for
    /// the rest and the failures are reported through the
    /// callback.
    ///
    /// # Callback lifetime across `stopStreaming`
    ///
    /// `stopStreaming()` and `shutdown()` clear the registered
    /// callback. To resume streaming on this client after
    /// `stopStreaming()`, you MUST call `startStreaming(callback)`
    /// again with a freshly bound function; `reconnect()` throws
    /// because no callback is held.
    ///
    /// This explicit-handoff model matches the C++ wrapper's RAII
    /// destructor and the Python `with` block's `__exit__`: the
    /// resource (the JS callback handle) is cleared at the same
    /// scope boundary the application observes. The unified C API
    /// preserves the callback across stop/reconnect, but the
    /// TypeScript and Python bindings deliberately diverge to enforce
    /// the explicit handoff and avoid retaining captured references
    /// past a teardown the caller has already observed.
    #[napi(js_name = "reconnect")]
    pub async fn reconnect(&self) -> napi::Result<()> {
        // `reconnect` re-registers the previously installed callback
        // on a fresh FPSS connection. Require a prior
        // `startStreaming(callback)` so the napi surface stays
        // consistent: there is no implicit zero-callback streaming
        // mode on this binding.
        let callback_arc: Arc<TsfnCallback> = {
            let guard = self.callback.lock().unwrap_or_else(|e| e.into_inner());
            match guard.as_ref() {
                Some(cb) => Arc::clone(cb),
                None => {
                    return Err(napi::Error::from_reason(
                        "no callback registered -- call startStreaming(callback) before reconnect()",
                    ));
                }
            }
        };
        crate::fpss_client::ensure_callback_open(&callback_arc)?;
        let dispatch_cb = Arc::clone(&callback_arc);
        let callback_for_result = Arc::clone(&callback_arc);
        let callback_closing_expected = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let delivery_failed = Arc::new(std::sync::atomic::AtomicBool::new(false));
        // Fresh teardown wake hook for the reconnected session. The
        // reconnect re-uses the same `ThreadsafeFunction`, but the hook is
        // `FnOnce`, so a new one is built from the persistent callback handle
        // per reconnect to wake a dispatcher blocked in the full callback queue
        // when the NEW session tears down. See `startStreaming`.
        let teardown_hook = crate::fpss_client::abort_hook_expect_closing(
            &callback_arc,
            Arc::clone(&callback_closing_expected),
            self.js_thread,
        );

        // The reconnect re-runs the FPSS connect and authentication
        // handshake plus a paced subscription restore, all network-bound.
        // Move it onto a blocking worker so the single libuv thread stays
        // free for timers, IO, and queued promises. The stored callback
        // handle stays in place; `reconnect_streaming` re-uses the same
        // `ThreadsafeFunction`, so there is no state shape change.
        let client = Arc::clone(&self.client);
        let callback_closing_expected_for_dispatch = Arc::clone(&callback_closing_expected);
        let callback_closing_expected_for_result = Arc::clone(&callback_closing_expected);
        let delivery_failed_for_dispatch = Arc::clone(&delivery_failed);
        let delivery_failed_for_scope = Arc::clone(&delivery_failed);
        tokio::task::spawn_blocking(move || {
            let stream = client.stream();
            let result = stream.reconnect_streaming_scoped(
                move |event: &fpss::StreamEvent| {
                    let buffered = fpss_event_to_buffered(event);
                    let typed = buffered_event_to_typed(buffered);
                    let status = dispatch_cb.call(
                        typed,
                        napi::threadsafe_function::ThreadsafeFunctionCallMode::Blocking,
                    );
                    crate::fpss_client::note_callback_delivery_status(
                        status,
                        &callback_closing_expected_for_dispatch,
                        &delivery_failed_for_dispatch,
                    );
                },
                move |drain| {
                    crate::fpss_client::callback_delivery_outcome(
                        drain(),
                        &delivery_failed_for_scope,
                    )
                },
                Some(teardown_hook),
            );
            if callback_closing_expected_for_result.load(std::sync::atomic::Ordering::Acquire) {
                stream.stop_streaming();
                return Err(crate::fpss_client::aborted_callback_error());
            }
            if let Err(err) = crate::fpss_client::ensure_callback_open(&callback_for_result) {
                stream.stop_streaming();
                return Err(err);
            }
            result.map_err(to_napi_err)
        })
        .await
        .map_err(|e| napi::Error::from_reason(format!("reconnect task panicked: {e}")))?
    }

    /// Stop streaming while keeping the market-data client usable.
    ///
    /// Clears the registered callback. To resume streaming, start streaming again with a freshly bound callback -- reconnect will fail because no callback is held. See the reconnect docs for the rationale: the callback is released at the same scope boundary the application observes, so a stopped session never retains a captured reference past a teardown the caller has already seen.
    #[napi(js_name = "stopStreaming")]
    pub fn stop_streaming(&self) {
        let previous_callback = {
            let guard = self.callback.lock().unwrap_or_else(|e| e.into_inner());
            guard.as_ref().map(Arc::clone)
        };
        self.client.stream().stop_streaming();
        if let Some(previous_callback) = previous_callback {
            let mut guard = self.callback.lock().unwrap_or_else(|e| e.into_inner());
            if guard
                .as_ref()
                .is_some_and(|cb| Arc::ptr_eq(cb, &previous_callback))
            {
                *guard = None;
            }
        }
    }

    /// Shut down the streaming connection.
    ///
    /// On the Python and TypeScript bindings, this clears the registered callback (same explicit-handoff semantics as stopping the stream); reconnect will then fail until the caller starts streaming again with a freshly bound callback. The C++ binding preserves the underlying connection's behaviour.
    #[napi(js_name = "shutdown")]
    pub fn shutdown(&self) {
        let previous_callback = {
            let guard = self.callback.lock().unwrap_or_else(|e| e.into_inner());
            guard.as_ref().map(Arc::clone)
        };
        self.client.stream().stop_streaming();
        if let Some(previous_callback) = previous_callback {
            let mut guard = self.callback.lock().unwrap_or_else(|e| e.into_inner());
            if guard
                .as_ref()
                .is_some_and(|cb| Arc::ptr_eq(cb, &previous_callback))
            {
                *guard = None;
            }
        }
    }

    /// Block until the previous streaming session's consumer thread has finished firing the registered callback. Returns true if the drain completed within the timeout, false otherwise.
    #[napi(js_name = "awaitDrain")]
    pub async fn await_drain(&self, timeout_ms: f64) -> napi::Result<bool> {
        let timeout_ms = crate::validate_u32_arg("timeoutMs", timeout_ms)?;
        let client = self.client.clone();
        let timeout = std::time::Duration::from_millis(u64::from(timeout_ms));
        tokio::task::spawn_blocking(move || client.stream().await_drain(timeout))
            .await
            .map_err(|e| napi::Error::from_reason(format!("await_drain task panicked: {e}")))
    }
}

// `startStreaming(cb)` is the sole streaming entry point. Callers that
// want a for-await shape can wrap a queue inside the callback.

// SDK configuration class. Adds `Config` napi class with
// `production()` / `dev()` / `stage()` factories plus the full setter
// surface for market-data pool sizing, retry policy, reconnect policy,
// and flat-file backoff.
mod config_class;
pub use config_class::Config;
mod config_accessors;

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
// streaming-event payload object in `stream_events.rs`; the package
// `index.ts` re-exports the fluent class under both `ContractRef` and
// `Contract` so users write `Contract.stock("AAPL")` per the
// documented surface.
mod fluent;
pub use fluent::{ContractRef, SecType, Subscription};

// Standalone streaming-only client. Adds the `StreamingClient` napi class
// over `thetadatadx::fpss::StreamingClient` (the streaming primitive), mirroring the
// Python `StreamingClient` and the C++ `thetadatadx::StreamingClient`. It opens only the streaming
// TLS transport, no market-data channel or Nexus, and drives its own dispatcher thread,
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
            market_data_type: None,
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
            cfg.market_data_environment(),
            config::MarketDataEnvironment::Prod
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
            market_data_type: Some("STAGE".to_string()),
            ..empty()
        };
        let (creds, cfg) = opts.resolve().expect("email/password resolves");
        assert!(!creds.is_api_key());
        assert_eq!(creds.email(), Some("you@example.com"));
        // marketDataType selects only the market-data channel; streaming stays prod.
        assert_eq!(
            cfg.market_data_environment(),
            config::MarketDataEnvironment::Stage
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
        // streamingType selects only the streaming channel; market-data stays prod.
        assert_eq!(
            cfg.market_data_environment(),
            config::MarketDataEnvironment::Prod
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
    fn bad_market_data_type_is_an_error() {
        let opts = ClientConnectOptions {
            api_key: Some("k".to_string()),
            market_data_type: Some("nope".to_string()),
            ..empty()
        };
        let msg = match opts.resolve() {
            Ok(_) => panic!("expected an marketDataType parse error"),
            Err(e) => e.reason.clone(),
        };
        assert!(msg.contains("marketDataType"), "got: {msg}");
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
