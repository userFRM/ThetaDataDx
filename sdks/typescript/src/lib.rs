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
fn runtime() -> &'static tokio::runtime::Runtime {
    static RT: OnceLock<tokio::runtime::Runtime> = OnceLock::new();
    RT.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .expect("failed to create tokio runtime")
    })
}

fn to_napi_err(e: thetadatadx::Error) -> napi::Error {
    napi::Error::from_reason(e.to_string())
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

fn parse_sec_type(sec_type: &str) -> napi::Result<tdbe::types::enums::SecType> {
    match sec_type.to_uppercase().as_str() {
        "STOCK" => Ok(tdbe::types::enums::SecType::Stock),
        "OPTION" => Ok(tdbe::types::enums::SecType::Option),
        "INDEX" => Ok(tdbe::types::enums::SecType::Index),
        other => Err(napi::Error::from_reason(format!(
            "unknown sec_type: {other:?} (expected STOCK, OPTION, or INDEX)"
        ))),
    }
}

// Generated string enum exports.
include!("enums_generated.rs");

// ── Typed tick classes (generated from tick_schema.toml) ──
//
// Emits `#[napi(object)]` structs for every tick type plus
// `{tick}_to_class_vec` factories. These back every historical endpoint
// return so `index.d.ts` surfaces concrete `Tick[]` types instead of `any`.

include!("tick_classes.rs");

// ── Typed FPSS event classes (generated from fpss_event_schema.toml) ──

include!("fpss_event_classes.rs");

// ── Buffered FPSS events ──

//
// Generator-emitted from `fpss_event_schema.toml`. Same file content as
// the Python SDK copy — single source of truth. Change the schema and
// regenerate, never hand-edit the generated `buffered_event.rs`.

include!("buffered_event.rs");

// ── Unified ThetaDataDx client ──

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
pub struct ThetaDataDx {
    tdx: thetadatadx::ThetaDataDx,
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
impl ThetaDataDx {
    // Lifecycle: intentionally hand-written (language-specific constructor semantics).

    /// Connect to ThetaData. Historical (MDDS/gRPC) only; call startStreaming()
    /// to begin FPSS real-time data.
    #[napi(factory)]
    pub fn connect(email: String, password: String) -> napi::Result<ThetaDataDx> {
        let creds = auth::Credentials::new(email, password);
        let config = config::DirectConfig::production();
        let tdx = runtime()
            .block_on(thetadatadx::ThetaDataDx::connect(&creds, config))
            .map_err(to_napi_err)?;
        Ok(ThetaDataDx {
            tdx,
            callback: Mutex::new(None),
        })
    }

    /// Connect with a credentials file (line 1 = email, line 2 = password).
    #[napi(factory)]
    pub fn connect_from_file(path: String) -> napi::Result<ThetaDataDx> {
        let creds = auth::Credentials::from_file(&path).map_err(to_napi_err)?;
        let config = config::DirectConfig::production();
        let tdx = runtime()
            .block_on(thetadatadx::ThetaDataDx::connect(&creds, config))
            .map_err(to_napi_err)?;
        Ok(ThetaDataDx {
            tdx,
            callback: Mutex::new(None),
        })
    }

    /// Cumulative count of FPSS events the TLS reader could not
    /// publish into the Disruptor ring because the Disruptor consumer
    /// fell behind and the ring was full (`Producer::try_publish`
    /// returned `RingBufferFull`).
    ///
    /// Forwards to `thetadatadx::ThetaDataDx::dropped_event_count` so
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
include!("historical_methods.rs");

// Generated streaming/FPSS methods.
include!("streaming_methods.rs");
