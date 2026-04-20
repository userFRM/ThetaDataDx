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

/// A buffered FPSS event for Node.js consumption.
///
/// Intermediate form between the Rust FPSS callback and the typed
/// `FpssEvent` struct emitted through napi. Not serialized — the
/// `buffered_event_to_typed` dispatcher converts it into the napi
/// struct directly. Earlier revisions had `#[derive(serde::Serialize)]`
/// + `#[serde(tag = "kind")]` + `#[serde(rename = "...")]` here, but
/// nothing ever went through serde; the attrs were dead weight and
/// invited drift between the serde `kind` tag and the napi typed
/// `kind` discriminator.
#[derive(Clone, Debug)]
enum BufferedEvent {
    Quote {
        contract_id: i32,
        ms_of_day: i32,
        bid_size: i32,
        bid_exchange: i32,
        bid: f64,
        bid_condition: i32,
        ask_size: i32,
        ask_exchange: i32,
        ask: f64,
        ask_condition: i32,
        date: i32,
        received_at_ns: u64,
    },
    Trade {
        contract_id: i32,
        ms_of_day: i32,
        sequence: i32,
        ext_condition1: i32,
        ext_condition2: i32,
        ext_condition3: i32,
        ext_condition4: i32,
        condition: i32,
        size: i32,
        exchange: i32,
        price: f64,
        condition_flags: i32,
        price_flags: i32,
        volume_type: i32,
        records_back: i32,
        date: i32,
        received_at_ns: u64,
    },
    OpenInterest {
        contract_id: i32,
        ms_of_day: i32,
        open_interest: i32,
        date: i32,
        received_at_ns: u64,
    },
    Ohlcvc {
        contract_id: i32,
        ms_of_day: i32,
        open: f64,
        high: f64,
        low: f64,
        close: f64,
        volume: i64,
        count: i64,
        date: i32,
        received_at_ns: u64,
    },
    RawData { code: u8, payload: Vec<u8> },
    Simple {
        event_type: String,
        detail: Option<String>,
        id: Option<i32>,
    },
}

fn fpss_event_to_buffered(event: &fpss::FpssEvent) -> BufferedEvent {
    match event {
        fpss::FpssEvent::Data(data) => match data {
            fpss::FpssData::Quote {
                contract_id,
                ms_of_day,
                bid_size,
                bid_exchange,
                bid,
                bid_condition,
                ask_size,
                ask_exchange,
                ask,
                ask_condition,
                date,
                received_at_ns,
                ..
            } => BufferedEvent::Quote {
                contract_id: *contract_id,
                ms_of_day: *ms_of_day,
                bid_size: *bid_size,
                bid_exchange: *bid_exchange,
                bid: *bid,
                bid_condition: *bid_condition,
                ask_size: *ask_size,
                ask_exchange: *ask_exchange,
                ask: *ask,
                ask_condition: *ask_condition,
                date: *date,
                received_at_ns: *received_at_ns,
            },
            fpss::FpssData::Trade {
                contract_id,
                ms_of_day,
                sequence,
                ext_condition1,
                ext_condition2,
                ext_condition3,
                ext_condition4,
                condition,
                size,
                exchange,
                price,
                condition_flags,
                price_flags,
                volume_type,
                records_back,
                date,
                received_at_ns,
                ..
            } => BufferedEvent::Trade {
                contract_id: *contract_id,
                ms_of_day: *ms_of_day,
                sequence: *sequence,
                ext_condition1: *ext_condition1,
                ext_condition2: *ext_condition2,
                ext_condition3: *ext_condition3,
                ext_condition4: *ext_condition4,
                condition: *condition,
                size: *size,
                exchange: *exchange,
                price: *price,
                condition_flags: *condition_flags,
                price_flags: *price_flags,
                volume_type: *volume_type,
                records_back: *records_back,
                date: *date,
                received_at_ns: *received_at_ns,
            },
            fpss::FpssData::OpenInterest {
                contract_id,
                ms_of_day,
                open_interest,
                date,
                received_at_ns,
                ..
            } => BufferedEvent::OpenInterest {
                contract_id: *contract_id,
                ms_of_day: *ms_of_day,
                open_interest: *open_interest,
                date: *date,
                received_at_ns: *received_at_ns,
            },
            fpss::FpssData::Ohlcvc {
                contract_id,
                ms_of_day,
                open,
                high,
                low,
                close,
                volume,
                count,
                date,
                received_at_ns,
                ..
            } => BufferedEvent::Ohlcvc {
                contract_id: *contract_id,
                ms_of_day: *ms_of_day,
                open: *open,
                high: *high,
                low: *low,
                close: *close,
                volume: *volume,
                count: *count,
                date: *date,
                received_at_ns: *received_at_ns,
            },
            _ => BufferedEvent::Simple {
                event_type: "unknown_data".to_string(),
                detail: None,
                id: None,
            },
        },
        fpss::FpssEvent::Control(ctrl) => match ctrl {
            fpss::FpssControl::LoginSuccess { permissions } => BufferedEvent::Simple {
                event_type: "login_success".to_string(),
                detail: Some(permissions.clone()),
                id: None,
            },
            fpss::FpssControl::ContractAssigned { id, contract } => BufferedEvent::Simple {
                event_type: "contract_assigned".to_string(),
                detail: Some(format!("{contract}")),
                id: Some(*id),
            },
            fpss::FpssControl::ReqResponse { req_id, result } => BufferedEvent::Simple {
                event_type: "req_response".to_string(),
                detail: Some(format!("{result:?}")),
                id: Some(*req_id),
            },
            fpss::FpssControl::MarketOpen => BufferedEvent::Simple {
                event_type: "market_open".to_string(),
                detail: None,
                id: None,
            },
            fpss::FpssControl::MarketClose => BufferedEvent::Simple {
                event_type: "market_close".to_string(),
                detail: None,
                id: None,
            },
            fpss::FpssControl::ServerError { message } => BufferedEvent::Simple {
                event_type: "server_error".to_string(),
                detail: Some(message.clone()),
                id: None,
            },
            fpss::FpssControl::Disconnected { reason } => BufferedEvent::Simple {
                event_type: "disconnected".to_string(),
                detail: Some(format!("{reason:?}")),
                id: None,
            },
            fpss::FpssControl::Error { message } => BufferedEvent::Simple {
                event_type: "error".to_string(),
                detail: Some(message.clone()),
                id: None,
            },
            fpss::FpssControl::Reconnecting { reason, attempt, delay_ms } => BufferedEvent::Simple {
                event_type: "reconnecting".to_string(),
                detail: Some(format!("reason={reason:?} attempt={attempt} delay_ms={delay_ms}")),
                id: None,
            },
            fpss::FpssControl::Reconnected => BufferedEvent::Simple {
                event_type: "reconnected".to_string(),
                detail: None,
                id: None,
            },
            fpss::FpssControl::UnknownFrame { code, payload } => BufferedEvent::Simple {
                event_type: "unknown_frame".to_string(),
                detail: Some(format!(
                    "code={code} payload_hex={}",
                    payload.iter().map(|b| format!("{b:02x}")).collect::<String>()
                )),
                id: None,
            },
            _ => BufferedEvent::Simple {
                event_type: "unknown_control".to_string(),
                detail: None,
                id: None,
            },
        },
        fpss::FpssEvent::RawData { code, payload } => BufferedEvent::RawData {
            code: *code,
            payload: payload.clone(),
        },
        _ => BufferedEvent::Simple {
            event_type: "unknown".to_string(),
            detail: None,
            id: None,
        },
    }
}

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
