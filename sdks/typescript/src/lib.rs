//! TypeScript / Node.js bindings over the Rust `thetadatadx` core. Every call
//! crosses the napi-rs boundary into the same Rust code path used by the CLI
//! and FFI.

#[macro_use]
extern crate napi_derive;

use std::sync::{Arc, Mutex, OnceLock};

use napi::Either;
use thetadatadx as tick;
use thetadatadx::auth;
use thetadatadx::config;
use thetadatadx::fpss;

/// Shared tokio runtime for running async Rust from Node.js.
pub(crate) fn runtime() -> &'static tokio::runtime::Runtime {
    static RT: OnceLock<tokio::runtime::Runtime> = OnceLock::new();
    RT.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .expect("failed to create tokio runtime")
    })
}

/// Convert a `thetadatadx::Error` into a napi error whose `reason`
/// carries a typed class-name prefix (`"[SubscriptionError] ..."`,
/// `"[RateLimitError] ..."`, etc). The JS shim in `streaming-session.js`
/// intercepts every async-method rejection, parses the prefix, and
/// re-throws the right `tdx.SubscriptionError` / `tdx.RateLimitError`
/// subclass. The classes derive from the existing TypeScript-exported
/// base `ThetaDataError` so callers writing `catch (e instanceof
/// tdx.ThetaDataError)` continue to observe every failure.
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

/// Build an `InvalidParameterError`-typed napi error for user-input
/// validation that fails before reaching the core client. The JS shim
/// keys on the `[ClassName]` prefix to re-throw the typed subclass, so
/// TypeScript callers branch on `instanceof InvalidParameterError`
/// exactly as Python callers catch the parity `ValueError`.
pub(crate) fn invalid_parameter_err(message: impl std::fmt::Display) -> napi::Error {
    napi::Error::from_reason(format!("[InvalidParameterError] {message}"))
}

// ── Credentials ──
//
// A first-class credentials handle mirroring the Python `Credentials`
// pyclass (`sdks/python/src/lib.rs`), the C++ `tdx::Credentials`, and the
// C ABI `TdxCredentials` handle. Every binding builds credentials the
// same way — `new Credentials(email, password)` or
// `Credentials.fromFile(path)` — then hands the handle to `connect`, so
// the connect surface is `connect(creds, config?)` across the board
// rather than each binding spreading raw `email`/`password` strings.

/// ThetaData login credentials.
///
/// Build from an email + password pair (`new Credentials(email,
/// password)`) or load from a credentials file (`Credentials.fromFile`,
/// line 1 = email, line 2 = password), then pass the handle to a client
/// `connect(creds, config?)`. Mirrors the Python `Credentials` and the
/// C++ `tdx::Credentials`.
///
/// ```js
/// const { Credentials, ThetaDataDxClient } = require("@thetadatadx/sdk");
/// const creds = Credentials.fromFile("creds.txt");
/// const tdx = ThetaDataDxClient.connect(creds);
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

    /// Redacted string form — never exposes the email or password. Matches
    /// the redacted `Debug` impl on the Rust `auth::Credentials` and the
    /// Python `Credentials.__repr__`.
    #[napi(js_name = "toString")]
    pub fn to_string_js(&self) -> String {
        "Credentials(email=<redacted>)".to_string()
    }
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

/// Validate a JavaScript `timeoutMs` deadline and convert it to the
/// integer millisecond domain the Python, C++, and C ABI bindings take.
///
/// `timeoutMs` rides in the options object as a JS `number` (an IEEE-754
/// double). The integer-typed bindings cannot represent a fractional,
/// negative, or non-finite deadline, so this binding rejects the same
/// inputs rather than coercing them: an `as u64` cast would silently
/// rewrite `NaN` and a negative value to `0` (an instant deadline),
/// `Infinity` to `u64::MAX` (a multi-century deadline), and a fractional
/// value to its truncation — each the opposite of a caller's intent. A
/// rejected value surfaces as `InvalidParameterError`, the typed class
/// the Python binding raises (`ValueError`) for the identical input, so
/// a caller's `catch (e instanceof InvalidParameterError)` branch ports
/// across bindings.
pub(crate) fn validate_timeout_ms(timeout_ms: f64) -> napi::Result<u64> {
    if !timeout_ms.is_finite() {
        return Err(invalid_parameter_err(format!(
            "timeoutMs must be a non-negative integer number of milliseconds; got {timeout_ms}"
        )));
    }
    if timeout_ms < 0.0 {
        return Err(invalid_parameter_err(format!(
            "timeoutMs must be non-negative; got {timeout_ms}"
        )));
    }
    if timeout_ms.fract() != 0.0 {
        return Err(invalid_parameter_err(format!(
            "timeoutMs must be a whole number of milliseconds; got {timeout_ms}"
        )));
    }
    if timeout_ms > u64::MAX as f64 {
        return Err(invalid_parameter_err(format!(
            "timeoutMs exceeds the representable millisecond range; got {timeout_ms}"
        )));
    }
    Ok(timeout_ms as u64)
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
    match runtime().spawn(fut).await {
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
    use thetadatadx::error::{AuthErrorKind, FpssErrorKind, GrpcStatusKind};
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
        thetadatadx::Error::Fpss { kind, .. } => match kind {
            FpssErrorKind::TooManyRequests => "RateLimitError",
            FpssErrorKind::Timeout => "DeadlineExceededError",
            FpssErrorKind::ConnectionRefused | FpssErrorKind::Disconnected => "NetworkError",
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
/// first `ThetaDataDxClient.connect()` call panics with
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

fn normalize_date(value: Either<String, chrono::DateTime<chrono::Utc>>) -> String {
    match value {
        Either::A(value) => value,
        Either::B(value) => value.format("%Y%m%d").to_string(),
    }
}

fn normalize_time(value: Either<String, chrono::DateTime<chrono::Utc>>) -> String {
    match value {
        Either::A(value) => value,
        Either::B(value) => value.format("%H:%M:%S").to_string(),
    }
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

// ── Typed FPSS event classes (generated from fpss_event_schema.toml) ──

include!("_generated/fpss_event_classes.rs");

// ── Buffered FPSS events ──

//
// Generator-emitted from `fpss_event_schema.toml`. Same file content as
// the Python SDK copy — single source of truth. Change the schema and
// regenerate, never hand-edit the generated `buffered_event.rs`.

include!("_generated/buffered_event.rs");

// ── Offline Greeks calculator free functions (generated from sdk_surface.toml) ──
//
// Emits the `AllGreeks` `#[napi(object)]` plus the `allGreeks(...)` /
// `impliedVolatility(...)` napi free functions. They cross the napi
// boundary into the same `thetadatadx::greeks::{all_greeks,
// implied_volatility}` core the Python / C++ / C ABI calculators call,
// so the Greek values are bit-identical across every binding. Change
// `sdk_surface.toml` and regenerate, never hand-edit the generated file.

include!("_generated/utility_functions.rs");

// ── Unified ThetaDataDxClient client ──

/// `ThreadsafeFunction` that owns a JS callback reference and routes
/// `FpssEvent` deliveries onto the Node main thread via napi-rs's
/// internal `uv_async_t` queue. The const generic `false` selects
/// `ErrorStrategy::Fatal`, so the napi-rs `call` API takes the
/// `FpssEvent` directly (not a `Result`) and the JS side relies on
/// its own try/catch for user-callback failures. The two `FpssEvent`
/// type parameters are the wire payload and the JS-call arg type
/// respectively; both are the same concrete object here.
///
/// napi-rs is the only safe path: Node's libuv requires JS callbacks
/// on the main thread, so calling V8 from any other thread is
/// undefined behavior. The dispatcher's drain thread therefore hands
/// every event to this `ThreadsafeFunction`, which queues it for the
/// main thread via `napi_call_threadsafe_function`.
pub(crate) type TsfnCallback =
    napi::threadsafe_function::ThreadsafeFunction<FpssEvent, (), FpssEvent, napi::Status, false>;

#[napi]
pub struct ThetaDataDxClient {
    /// Wrapped in `Arc` so async napi methods (e.g. `awaitDrain`) can
    /// clone a cheap handle into a `tokio::task::spawn_blocking` future
    /// without violating the `Send + 'static` bound. The inner
    /// `thetadatadx::ThetaDataDxClient` is not `Clone` -- its FPSS mutex and
    /// subscription-tier state forbid that -- so the outer `Arc` is the
    /// only way to hand a borrow off the napi main thread.
    tdx: Arc<thetadatadx::ThetaDataDxClient>,
    /// Stored JS callback registered via `startStreaming(callback)`.
    /// `None` until the first registration; persisted across
    /// `reconnect()` so the reconnect path can re-attach the same JS
    /// function without re-asking the caller for it. Cleared on
    /// `stopStreaming()` / `shutdown()` so the napi reference is
    /// released back to V8 and a subsequent `startStreaming()` sees a
    /// clean slot.
    ///
    /// Wrapped in `Arc` because the dispatcher closure (`Fn(&FpssEvent)
    /// + Send + 'static`) needs its own ref-counted clone of the
    /// callback handle. `ThreadsafeFunction` itself does not implement
    /// `Clone` in napi-rs 3.x (its inner `napi_threadsafe_function`
    /// is `Arc`-managed but only exposed through the
    /// `Arc<`ThreadsafeFunctionHandle`>` field on the struct), so the
    /// outer `Arc` here is the canonical way to share the handle.
    callback: Mutex<Option<Arc<TsfnCallback>>>,
}

#[napi]
impl ThetaDataDxClient {
    // Lifecycle: intentionally hand-written (language-specific constructor semantics).

    /// Connect to ThetaData with a [`Credentials`] handle. Pass an
    /// optional [`Config`] (`dev` / `stage` / `production`, plus any
    /// tuned setters) to override the production-default endpoint.
    /// Historical (MDDS/gRPC) only; call startStreaming() to begin FPSS
    /// real-time data.
    ///
    /// The config is snapshot at connect time: the `Config` handle may be
    /// reused or mutated afterward without affecting this client.
    ///
    /// ```js
    /// const creds = Credentials.fromFile("creds.txt");
    /// const tdx = ThetaDataDxClient.connect(creds);
    /// ```
    #[napi(factory)]
    pub fn connect(
        creds: &Credentials,
        config: Option<&Config>,
    ) -> napi::Result<ThetaDataDxClient> {
        let cfg = config_or_production(config);
        let tdx = runtime()
            .block_on(thetadatadx::ThetaDataDxClient::connect(
                // VOCAB-OK: tokio Runtime::block_on in NAPI bridge
                &creds.inner,
                cfg,
            ))
            .map_err(to_napi_err)?;
        Ok(ThetaDataDxClient {
            tdx: Arc::new(tdx),
            callback: Mutex::new(None),
        })
    }

    /// Connect with a credentials file (line 1 = email, line 2 =
    /// password). Convenience wrapper over `Credentials.fromFile` +
    /// `connect`. Pass an optional [`Config`] to override the
    /// production-default endpoint.
    #[napi(factory, js_name = "connectFromFile")]
    pub fn connect_from_file(
        path: String,
        config: Option<&Config>,
    ) -> napi::Result<ThetaDataDxClient> {
        let creds = auth::Credentials::from_file(&path).map_err(to_napi_err)?;
        let cfg = config_or_production(config);
        let tdx = runtime()
            .block_on(thetadatadx::ThetaDataDxClient::connect(
                // VOCAB-OK: tokio Runtime::block_on in NAPI bridge
                &creds, cfg,
            ))
            .map_err(to_napi_err)?;
        Ok(ThetaDataDxClient {
            tdx: Arc::new(tdx),
            callback: Mutex::new(None),
        })
    }

    /// Cumulative count of FPSS events the TLS reader could not
    /// publish into the event ring because the event-dispatch consumer
    /// fell behind and the ring was full (`Producer::try_publish`
    /// returned `RingBufferFull`).
    ///
    /// Forwards to `thetadatadx::ThetaDataDxClient::dropped_event_count` so
    /// the value matches every other binding (C ABI, Python, C++).
    /// The counter lives on the underlying `FpssClient` and resets
    /// when the client is recreated -- that happens on
    /// `stop_streaming` and `reconnect` (which calls
    /// `stop_streaming` + `start_streaming` internally). Snapshot the
    /// value before reconnect if you need to accumulate drops across
    /// session boundaries.
    ///
    /// Returned as `bigint` so it can represent the full `u64` range
    /// (Number would top out at 2^53).
    #[napi(js_name = "droppedEventCount")]
    pub fn dropped_event_count(&self) -> napi::bindgen_prelude::BigInt {
        napi::bindgen_prelude::BigInt::from(self.tdx.dropped_event_count())
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
    /// Forwards to `thetadatadx::ThetaDataDxClient::ring_occupancy`
    /// so the value matches every other binding (C ABI, Python,
    /// C++). Returns `0n` before `startStreaming` and after
    /// `stopStreaming`. Returned as `bigint` for shape-consistency
    /// with the other streaming counters.
    #[napi(js_name = "ringOccupancy")]
    pub fn ring_occupancy(&self) -> napi::bindgen_prelude::BigInt {
        napi::bindgen_prelude::BigInt::from(self.tdx.ring_occupancy() as u64)
    }

    /// Configured capacity of the streaming event ring in slots (the
    /// `fpssRingSize` setting, a power of two).
    ///
    /// The fixed denominator for `ringOccupancy()`: when the
    /// occupancy sample approaches this value the ring is saturating
    /// and further events will be dropped (counted by
    /// `droppedEventCount()`). Returns `0n` before `startStreaming`
    /// and after `stopStreaming`. Returned as `bigint` for
    /// shape-consistency with the other streaming counters.
    #[napi(js_name = "ringCapacity")]
    pub fn ring_capacity(&self) -> napi::bindgen_prelude::BigInt {
        napi::bindgen_prelude::BigInt::from(self.tdx.ring_capacity() as u64)
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
        self.tdx
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
        napi::bindgen_prelude::BigInt::from(self.tdx.last_event_received_at_unix_nanos())
    }

    /// Address (`host:port`) of the streaming server the current
    /// session is connected to, following the session across
    /// auto-reconnects. `null` when streaming has not started.
    #[napi(js_name = "lastConnectedAddr")]
    pub fn last_connected_addr(&self) -> Option<String> {
        self.tdx.last_connected_addr()
    }

    /// Cumulative count of user-callback panics caught by the
    /// per-invocation `catch_unwind` boundary since the current stream
    /// started.
    ///
    /// A panic in the callback is caught, recorded here, and does not
    /// stop event delivery — the next event continues normally.
    /// Forwards to `thetadatadx::ThetaDataDxClient::panic_count` so
    /// the value matches every other binding (C ABI, Python, C++).
    ///
    /// Returned as `bigint` so it can represent the full `u64` range
    /// (Number would top out at 2^53).
    #[napi(js_name = "panicCount")]
    pub fn panic_count(&self) -> napi::bindgen_prelude::BigInt {
        napi::bindgen_prelude::BigInt::from(self.tdx.panic_count())
    }

    /// Snapshot of full-stream subscriptions (e.g. `OPTION` /
    /// `full_trades`, `OPTION` / `full_open_interest`).
    ///
    /// Each entry has the same `{ kind, contract }` shape returned by
    /// `activeSubscriptions()`, where `kind` is one of
    /// `"full_trades"` / `"full_open_interest"` and `contract` carries
    /// the wire-level security type (`"OPTION"`, `"STOCK"`, ...).
    /// Quote is never a valid full-stream kind on the FPSS wire, so
    /// any such row from the core is dropped from the projection.
    /// Empty array when streaming has not started.
    ///
    /// Mirrors the Python `ThetaDataDxClient.active_full_subscriptions()`
    /// (`sdks/python/src/lib.rs`) and the C++
    /// `UnifiedClient::active_full_subscriptions`
    /// (`sdks/cpp/include/thetadx.hpp`) so every binding reports the
    /// full-stream subscription set with the same projection shape.
    #[napi(js_name = "activeFullSubscriptions")]
    pub fn active_full_subscriptions(&self) -> napi::Result<serde_json::Value> {
        use thetadatadx::fpss::protocol::SubscriptionKind;
        self.tdx
            .active_full_subscriptions()
            .map(|subs| {
                serde_json::json!(subs
                    .into_iter()
                    .filter_map(|(kind, sec_type)| {
                        let kind_str = match kind {
                            SubscriptionKind::Trade => "full_trades",
                            SubscriptionKind::OpenInterest => "full_open_interest",
                            // Quote is not a valid full-stream kind on
                            // the FPSS wire — drop the row to keep the
                            // projection cross-binding clean.
                            SubscriptionKind::Quote => return None,
                            _ => return None,
                        };
                        Some(serde_json::json!({
                            "kind": kind_str,
                            "contract": format!("{sec_type:?}"),
                        }))
                    })
                    .collect::<Vec<_>>())
            })
            .map_err(to_napi_err)
    }
}

// ── Standalone MddsClient (historical-only) ──

/// Standalone MDDS-only historical client.
///
/// Opens ONLY the MDDS channel and the Nexus authentication flow —
/// no FPSS TLS connection, no event ring, no streaming state machine.
/// Mirrors the Python `MddsClient` (`sdks/python/src/mdds_client.rs`),
/// the C++ `tdx::Client`, and the standalone C ABI entry points
/// (`tdx_client_*`), letting a caller run a historical-only session
/// alongside a parallel FPSS process without the unified
/// [`ThetaDataDxClient`] taking over the Nexus session at connect time.
///
/// The full historical / list / snapshot / at-time / FLATFILES surface
/// is generated onto this class identically to the unified client (see
/// `_generated/historical_methods.rs`), so `mddsClient.stockHistoryEod(...)`
/// behaves exactly like `client.stockHistoryEod(...)`. The streaming and
/// subscription methods are simply not present: there is no
/// `startStreaming` / `subscribe` on this class, so an MDDS-only handle
/// cannot open an FPSS slot. Use [`FpssClient`] for streaming, or the
/// unified [`ThetaDataDxClient`] when you need both surfaces.
///
/// ```js
/// const { MddsClient, Config } = require("@thetadatadx/sdk");
/// const mdds = MddsClient.connectFromFile("creds.txt");
/// const eod = await mdds.stockHistoryEod("AAPL", "20240101", "20240301");
/// ```
#[napi]
pub struct MddsClient {
    /// Wrapped in `Arc` so the generated async endpoint methods can
    /// clone a cheap `'static` handle into the worker future, exactly
    /// like the unified client's `tdx` field. The generated method
    /// bodies reference `self.tdx`, so the historical impl block the
    /// codegen projects onto this class compiles unchanged. This client
    /// holds the same `thetadatadx::ThetaDataDxClient` core but never
    /// reaches its streaming methods — no FPSS TLS slot is opened for a
    /// session that lives entirely through `MddsClient`.
    tdx: Arc<thetadatadx::ThetaDataDxClient>,
}

#[napi]
impl MddsClient {
    // Lifecycle: intentionally hand-written (language-specific constructor
    // semantics), mirroring the unified `ThetaDataDxClient` factories. The
    // connect core is identical — `thetadatadx::ThetaDataDxClient::connect`
    // opens MDDS + Nexus and never opens FPSS until a streaming method is
    // called, which this class does not surface.

    /// Connect to ThetaData with a [`Credentials`] handle and open the
    /// MDDS channel. Historical (MDDS/gRPC) only — this client never
    /// opens the FPSS streaming transport. Pass an optional [`Config`] to
    /// override the production-default endpoint. Use [`FpssClient`] for
    /// real-time data.
    ///
    /// The config is snapshot at connect time: the `Config` handle may be
    /// reused or mutated afterward without affecting this client.
    #[napi(factory)]
    pub fn connect(creds: &Credentials, config: Option<&Config>) -> napi::Result<MddsClient> {
        let cfg = config_or_production(config);
        let tdx = runtime()
            .block_on(thetadatadx::ThetaDataDxClient::connect(
                // VOCAB-OK: tokio Runtime::block_on in NAPI bridge
                &creds.inner,
                cfg,
            ))
            .map_err(to_napi_err)?;
        Ok(MddsClient { tdx: Arc::new(tdx) })
    }

    /// Connect with a credentials file (line 1 = email, line 2 =
    /// password). Convenience wrapper over `Credentials.fromFile` +
    /// `connect`. Historical (MDDS/gRPC) only. Pass an optional
    /// [`Config`] to override the production-default endpoint.
    #[napi(factory, js_name = "connectFromFile")]
    pub fn connect_from_file(path: String, config: Option<&Config>) -> napi::Result<MddsClient> {
        let creds = auth::Credentials::from_file(&path).map_err(to_napi_err)?;
        let cfg = config_or_production(config);
        let tdx = runtime()
            .block_on(thetadatadx::ThetaDataDxClient::connect(
                // VOCAB-OK: tokio Runtime::block_on in NAPI bridge
                &creds, cfg,
            ))
            .map_err(to_napi_err)?;
        Ok(MddsClient { tdx: Arc::new(tdx) })
    }
}

// Generated historical endpoint methods. The codegen projects the same
// per-endpoint method bodies onto both `ThetaDataDxClient` and
// `MddsClient` (see `HISTORICAL_IMPL_CLASSES` in the TypeScript SDK
// emitter); both classes expose an `Arc<thetadatadx::ThetaDataDxClient>`
// field named `tdx`, so the shared bodies compile against either.
include!("_generated/historical_methods.rs");

// Generated streaming/FPSS methods.
include!("_generated/streaming_methods.rs");

// `startStreaming(cb)` is the sole streaming entry point. Callers that
// want a for-await shape can wrap a queue inside the callback.

// SDK configuration class. Adds `Config` napi class with
// `production()` / `dev()` / `stage()` factories plus the full setter
// surface for historical pool sizing, retry policy, reconnect policy,
// and flat-file backoff.
mod config_class;
pub use config_class::{Config, WorkerThreadsSetting};

// Hand-written FLATFILES bindings — dynamic schema, see module docs.
mod flatfile_methods;

// Fluent contract-first API. Adds `ContractRef`,
// `Subscription`, `SecType` napi classes and the polymorphic
// `subscribe(sub)` / `subscribeMany([sub, ...])` methods on the
// unified client. The `Contract` name on the JS side is taken by the
// FPSS-event payload object in `_generated/fpss_event_classes.rs`; the package
// `index.ts` re-exports the fluent class under both `ContractRef` and
// `Contract` so users write `Contract.stock("AAPL")` per the
// documented surface.
mod fluent;
pub use fluent::{ContractRef, SecType, Subscription};

// Cross-language utility helpers. Adds the `Util` napi class
// re-exporting `thetadatadx::utils::{conditions, exchange, sequences}` lookup tables
// under camelCase JS method names.
mod util_helpers;
pub use util_helpers::Util;

// Standalone FPSS-only streaming client. Adds the `FpssClient` napi class
// over `thetadatadx::fpss::FpssClient` (the FPSS primitive), mirroring the
// Python `FpssClient` and the C++ `tdx::FpssClient`. It opens only the FPSS
// TLS transport — no MDDS / Nexus — and drives its own dispatcher thread,
// routing events through the same `TsfnCallback` mechanism as the unified
// client's streaming surface.
mod fpss_client;
pub use fpss_client::FpssClient;

#[napi]
impl ThetaDataDxClient {
    /// Polymorphic subscribe — primary fluent entry point. Accepts the
    /// `Subscription` value returned by `Contract.quote()` /
    /// `Contract.trade()` / `Contract.openInterest()` (per-contract
    /// scope) or by `SecType.option().fullTrades()` /
    /// `SecType.option().fullOpenInterest()` (full-stream scope).
    #[napi]
    pub fn subscribe(&self, sub: &fluent::Subscription) -> napi::Result<()> {
        self.tdx.subscribe(sub.snapshot()).map_err(to_napi_err)
    }

    /// Bulk-subscribe an array of `Subscription` values. Stops at the
    /// first error and returns it; previously-installed subscriptions
    /// are NOT rolled back.
    #[napi(js_name = "subscribeMany")]
    pub fn subscribe_many(&self, subs: Vec<&fluent::Subscription>) -> napi::Result<()> {
        let snaps: Vec<_> = subs.iter().map(|s| s.snapshot()).collect();
        self.tdx.subscribe_many(snaps).map_err(to_napi_err)
    }

    /// Polymorphic unsubscribe — fluent counterpart to `subscribe(sub)`.
    #[napi]
    pub fn unsubscribe(&self, sub: &fluent::Subscription) -> napi::Result<()> {
        self.tdx.unsubscribe(sub.snapshot()).map_err(to_napi_err)
    }

    /// Bulk-unsubscribe an array of `Subscription` values.
    #[napi(js_name = "unsubscribeMany")]
    pub fn unsubscribe_many(&self, subs: Vec<&fluent::Subscription>) -> napi::Result<()> {
        let snaps: Vec<_> = subs.iter().map(|s| s.snapshot()).collect();
        self.tdx.unsubscribe_many(snaps).map_err(to_napi_err)
    }
}

// `ThetaDataDxClient` is the public name (rename complete; no alias).
