//! TypeScript / Node.js bindings over the Rust `thetadatadx` core. Every call
//! crosses the napi-rs boundary into the same Rust code path used by the CLI
//! and FFI.

#[macro_use]
extern crate napi_derive;

use std::sync::{Arc, Mutex, OnceLock};

use napi::Either;
use tdbe::types::tick;
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
/// Mirrors the Python `to_py_err` leaf set one-for-one so the
/// cross-binding error contract stays uniform.
pub(crate) fn to_napi_err(e: thetadatadx::Error) -> napi::Error {
    let class = leaf_class_for(&e);
    napi::Error::from_reason(format!("[{class}] {e}"))
}

/// Pick the typed leaf class name for a `thetadatadx::Error`. The
/// JS shim parses this prefix off the error reason. Mirrors the
/// Python `to_py_err` dispatch table.
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
        thetadatadx::Error::Config { .. } => "ThetaDataError",
        thetadatadx::Error::Fpss { kind, .. } => match kind {
            FpssErrorKind::TooManyRequests => "RateLimitError",
            FpssErrorKind::Timeout => "DeadlineExceededError",
            FpssErrorKind::ConnectionRefused | FpssErrorKind::Disconnected => "NetworkError",
            _ => "StreamError",
        },
        _ => "ThetaDataError",
    }
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
type TsfnCallback = napi::threadsafe_function::ThreadsafeFunction<
    FpssEvent,
    (),
    FpssEvent,
    napi::Status,
    false,
>;

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
    /// `Arc<ThreadsafeFunctionHandle>` field on the struct), so the
    /// outer `Arc` here is the canonical way to share the handle.
    callback: Mutex<Option<Arc<TsfnCallback>>>,
}

#[napi]
impl ThetaDataDxClient {
    // Lifecycle: intentionally hand-written (language-specific constructor semantics).

    /// Connect to ThetaData. Historical (MDDS/gRPC) only; call startStreaming()
    /// to begin FPSS real-time data.
    #[napi(factory)]
    pub fn connect(email: String, password: String) -> napi::Result<ThetaDataDxClient> {
        let creds = auth::Credentials::new(email, password);
        let config = config::DirectConfig::production();
        let tdx = runtime()
            .block_on(thetadatadx::ThetaDataDxClient::connect(&creds, config))
            .map_err(to_napi_err)?;
        Ok(ThetaDataDxClient {
            tdx: Arc::new(tdx),
            callback: Mutex::new(None),
        })
    }

    /// Connect with a credentials file (line 1 = email, line 2 = password).
    #[napi(factory)]
    pub fn connect_from_file(path: String) -> napi::Result<ThetaDataDxClient> {
        let creds = auth::Credentials::from_file(&path).map_err(to_napi_err)?;
        let config = config::DirectConfig::production();
        let tdx = runtime()
            .block_on(thetadatadx::ThetaDataDxClient::connect(&creds, config))
            .map_err(to_napi_err)?;
        Ok(ThetaDataDxClient {
            tdx: Arc::new(tdx),
            callback: Mutex::new(None),
        })
    }

    /// Cumulative count of FPSS events the TLS reader could not
    /// publish into the Disruptor ring because the Disruptor consumer
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
}

// Generated historical endpoint methods.
include!("_generated/historical_methods.rs");

// Generated streaming/FPSS methods.
include!("_generated/streaming_methods.rs");

// Pull-iter delivery. Hand-written napi-rs wrapper around
// `thetadatadx::EventIterator`. Surfaced as
// `client.startStreamingIter()` returning an `EventIterator` napi
// class; the JS side wraps it in `for await (const event of iter)`
// via `Symbol.asyncIterator` declared in the `index.d.ts` companion.
include!("event_iterator.rs");

#[napi]
impl ThetaDataDxClient {
    /// Start FPSS streaming in pull-iter delivery mode.
    ///
    /// Returns an [`EventIterator`] handle whose `next()` resolves
    /// to the next typed FPSS event or `null` once the streaming
    /// session has shut down and the residual queue is drained. JS
    /// callers iterate with `for await (const event of iter)`.
    ///
    /// Mutually exclusive with `startStreaming(callback)`. Calling
    /// either while streaming is already running rejects with
    /// `"streaming already started"`.
    #[napi(js_name = "startStreamingIter")]
    pub fn start_streaming_iter(&self) -> napi::Result<EventIterator> {
        let inner = self.tdx.start_streaming_iter().map_err(to_napi_err)?;
        Ok(EventIterator::new(inner))
    }
}

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

// Cross-language utility helpers (issue #424). Adds the `Util` napi
// class re-exporting `tdbe::{conditions, exchange, sequences}` lookup
// tables under camelCase JS method names.
mod util_helpers;
pub use util_helpers::Util;

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
