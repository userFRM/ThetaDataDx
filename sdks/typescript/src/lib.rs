//! TypeScript/Node.js bindings for `thetadatadx` — wraps the Rust SDK via napi-rs.
//!
//! This is NOT a reimplementation. Every call goes through the Rust crate,
//! giving Node.js users native performance for ThetaData market data access.

#[macro_use]
extern crate napi_derive;

use std::sync::{Arc, Mutex, OnceLock};

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

type EventRx = Arc<Mutex<Option<Arc<Mutex<std::sync::mpsc::Receiver<BufferedEvent>>>>>>;

#[napi]
pub struct ThetaDataDx {
    tdx: thetadatadx::ThetaDataDx,
    rx: EventRx,
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
            rx: Arc::new(Mutex::new(None)),
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
            rx: Arc::new(Mutex::new(None)),
        })
    }
}

// Generated historical endpoint methods.
include!("historical_methods.rs");

// Generated streaming/FPSS methods.
include!("streaming_methods.rs");
