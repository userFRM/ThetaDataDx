//! Hand-written napi-rs bindings for the FLATFILES surface.
//!
//! Mirrors the Python wrapper at `sdks/python/src/flatfile_methods.rs`:
//! one method per `(SecType, ReqType)` plus a generic `request()`
//! dispatcher, all returning a [`FlatFileRowList`] with terminals
//! `.toArrowIpc()` / `.toJson()` / matching the Python surface.
//!
//! Unlike the PyO3 path, napi-rs does not have a zero-copy bridge to
//! a JS Arrow library, so the Arrow terminal returns Arrow IPC bytes.
//! The user deserialises with `apache-arrow`:
//!
//! ```ts
//! import { tableFromIPC } from "apache-arrow";
//! const ipc = rows.toArrowIpc();
//! const table = tableFromIPC(ipc);
//! ```

use std::io::Cursor;
use std::sync::Arc;

use arrow_ipc::writer::StreamWriter;
use napi::bindgen_prelude::Buffer;
use serde_json::{json, Map as JsonMap, Value as JsonValue};

use thetadatadx::flatfiles::{self, FlatFileFormat, FlatFileRow, FlatFileValue, ReqType, SecType};

use crate::{spawn_endpoint_task, to_napi_err};

// ── Helpers ────────────────────────────────────────────────────────────

fn parse_flatfile_sec_type(sec: &str) -> napi::Result<SecType> {
    match sec.to_uppercase().as_str() {
        "OPTION" => Ok(SecType::Option),
        "STOCK" => Ok(SecType::Stock),
        "INDEX" => Ok(SecType::Index),
        other => Err(crate::invalid_parameter_err(format!(
            "unknown flat-file sec_type: {other:?} (expected OPTION, STOCK, or INDEX)"
        ))),
    }
}

fn parse_flatfile_req_type(req: &str) -> napi::Result<ReqType> {
    match req.to_uppercase().as_str() {
        "EOD" => Ok(ReqType::Eod),
        "QUOTE" => Ok(ReqType::Quote),
        "OPEN_INTEREST" | "OPENINTEREST" => Ok(ReqType::OpenInterest),
        "OHLC" => Ok(ReqType::Ohlc),
        "TRADE" => Ok(ReqType::Trade),
        "TRADE_QUOTE" | "TRADEQUOTE" => Ok(ReqType::TradeQuote),
        other => Err(crate::invalid_parameter_err(format!(
            "unknown flat-file req_type: {other:?} (expected EOD, QUOTE, OPEN_INTEREST, OHLC, TRADE, TRADE_QUOTE)"
        ))),
    }
}

fn parse_flatfile_format(fmt: Option<&str>) -> napi::Result<FlatFileFormat> {
    match fmt.unwrap_or("csv").to_lowercase().as_str() {
        "csv" => Ok(FlatFileFormat::Csv),
        "jsonl" | "json" => Ok(FlatFileFormat::Jsonl),
        other => Err(crate::invalid_parameter_err(format!(
            "unknown flat-file format: {other:?} (expected csv or jsonl)"
        ))),
    }
}

/// Pull and decode a flat-file blob off the libuv thread.
///
/// A flat-file pull is a full-day blob download — seconds of network
/// transfer and a large decode. Running it on the runtime's execution
/// thread via [`spawn_endpoint_task`] keeps the Node event loop free for
/// the whole call, matching every historical endpoint. Callers are
/// `async fn`s, so napi-rs returns a JS `Promise` resolved off-thread.
async fn pull_decoded(
    client: &Arc<thetadatadx::Client>,
    sec: SecType,
    req: ReqType,
    date: &str,
) -> napi::Result<Vec<FlatFileRow>> {
    let client = Arc::clone(client);
    let date = date.to_string();
    spawn_endpoint_task(async move { client.flatfile_request_decoded(sec, req, &date).await }).await
}

// ── FlatFileRowList ────────────────────────────────────────────────────

/// JS class wrapping a decoded flat-file row vector. Created by every
/// method on `FlatFilesNamespace`; carries the typed
/// rows until the user picks a terminal.
#[napi]
pub struct FlatFileRowList {
    rows: Vec<FlatFileRow>,
}

#[napi]
impl FlatFileRowList {
    /// Number of decoded rows. Same value as `.length` on the JSON
    /// representation, exposed as a method so the API stays stable if
    /// the list later gains first-class iterator support.
    #[napi(js_name = "len")]
    pub fn len(&self) -> u32 {
        u32::try_from(self.rows.len()).unwrap_or(u32::MAX)
    }

    /// Whether the decoded row vector is empty.
    #[napi(js_name = "isEmpty")]
    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    /// Serialise the typed rows as Arrow IPC stream bytes. The dynamic
    /// schema is inferred from the first row. Deserialise on
    /// the JS side with `apache-arrow`'s `tableFromIPC`.
    #[napi(js_name = "toArrowIpc")]
    pub fn to_arrow_ipc(&self) -> napi::Result<Buffer> {
        let batch = flatfiles::arrow::rows_to_arrow(&self.rows).map_err(to_napi_err)?;
        let schema = batch.schema();
        let mut buf: Vec<u8> = Vec::new();
        {
            let mut writer = StreamWriter::try_new(Cursor::new(&mut buf), &schema)
                .map_err(|e| napi::Error::from_reason(e.to_string()))?;
            writer
                .write(&batch)
                .map_err(|e| napi::Error::from_reason(e.to_string()))?;
            writer
                .finish()
                .map_err(|e| napi::Error::from_reason(e.to_string()))?;
        }
        Ok(Buffer::from(buf))
    }

    /// Return a JSON array of objects, one per row. Useful for quick
    /// inspection, structured logging, or wiring into JS-side
    /// dataframes that don't read Arrow IPC.
    #[napi(js_name = "toJson")]
    pub fn to_json(&self) -> napi::Result<String> {
        let mut out: Vec<JsonValue> = Vec::with_capacity(self.rows.len());
        for row in &self.rows {
            let mut obj = JsonMap::new();
            obj.insert("symbol".into(), json!(row.symbol));
            obj.insert(
                "expiration".into(),
                row.expiration.map_or(JsonValue::Null, JsonValue::from),
            );
            obj.insert(
                "strike".into(),
                row.strike.map_or(JsonValue::Null, JsonValue::from),
            );
            obj.insert(
                "right".into(),
                row.right
                    .map_or(JsonValue::Null, |c| JsonValue::String(c.to_string())),
            );
            for (name, value) in &row.fields {
                let v = match value {
                    FlatFileValue::Int(v) => JsonValue::from(*v),
                    FlatFileValue::Price(v) => JsonValue::from(*v),
                };
                obj.insert(name.clone(), v);
            }
            out.push(JsonValue::Object(obj));
        }
        serde_json::to_string(&out).map_err(|e| napi::Error::from_reason(e.to_string()))
    }
}

// ── FlatFilesNamespace ─────────────────────────────────────────────────

/// JS class returned from `client.flatFiles`. Each method maps to one
/// (security type, request type) pair and returns a `FlatFileRowList`.
#[napi]
pub struct FlatFilesNamespace {
    pub(crate) client: Arc<thetadatadx::Client>,
}

#[napi]
impl FlatFilesNamespace {
    /// Option trade-with-quote flat file for the given `YYYYMMDD` date.
    #[napi(js_name = "optionTradeQuote")]
    pub async fn option_trade_quote(&self, date: String) -> napi::Result<FlatFileRowList> {
        let rows = pull_decoded(&self.client, SecType::Option, ReqType::TradeQuote, &date).await?;
        Ok(FlatFileRowList { rows })
    }

    /// Option open-interest flat file for the given `YYYYMMDD` date.
    #[napi(js_name = "optionOpenInterest")]
    pub async fn option_open_interest(&self, date: String) -> napi::Result<FlatFileRowList> {
        let rows =
            pull_decoded(&self.client, SecType::Option, ReqType::OpenInterest, &date).await?;
        Ok(FlatFileRowList { rows })
    }

    /// Option end-of-day flat file for the given `YYYYMMDD` date.
    #[napi(js_name = "optionEod")]
    pub async fn option_eod(&self, date: String) -> napi::Result<FlatFileRowList> {
        let rows = pull_decoded(&self.client, SecType::Option, ReqType::Eod, &date).await?;
        Ok(FlatFileRowList { rows })
    }

    /// Stock trade-with-quote flat file for the given `YYYYMMDD` date.
    #[napi(js_name = "stockTradeQuote")]
    pub async fn stock_trade_quote(&self, date: String) -> napi::Result<FlatFileRowList> {
        let rows = pull_decoded(&self.client, SecType::Stock, ReqType::TradeQuote, &date).await?;
        Ok(FlatFileRowList { rows })
    }

    /// Stock end-of-day flat file for the given `YYYYMMDD` date.
    #[napi(js_name = "stockEod")]
    pub async fn stock_eod(&self, date: String) -> napi::Result<FlatFileRowList> {
        let rows = pull_decoded(&self.client, SecType::Stock, ReqType::Eod, &date).await?;
        Ok(FlatFileRowList { rows })
    }

    /// Generic dispatcher — `secType` and `reqType` accept `"OPTION"` /
    /// `"QUOTE"` style strings.
    #[napi]
    pub async fn request(
        &self,
        sec_type: String,
        req_type: String,
        date: String,
    ) -> napi::Result<FlatFileRowList> {
        let sec = parse_flatfile_sec_type(&sec_type)?;
        let req = parse_flatfile_req_type(&req_type)?;
        let rows = pull_decoded(&self.client, sec, req, &date).await?;
        Ok(FlatFileRowList { rows })
    }
}

// ── Client napi extension ─────────────────────────────────────────

use crate::Client;

#[napi]
impl Client {
    /// FLATFILES namespace handle. Cheap — shares the underlying client connection.
    #[napi(getter, js_name = "flatFiles")]
    pub fn flat_files(&self) -> FlatFilesNamespace {
        FlatFilesNamespace {
            client: Arc::clone(&self.client),
        }
    }

    /// Pull a flat-file blob and write the requested format to `path`.
    /// Returns the final on-disk path with the format extension
    /// auto-appended if missing.
    #[napi(js_name = "flatFileToPath")]
    pub async fn flat_file_to_path(
        &self,
        sec_type: String,
        req_type: String,
        date: String,
        path: String,
        format: Option<String>,
    ) -> napi::Result<String> {
        let sec = parse_flatfile_sec_type(&sec_type)?;
        let req = parse_flatfile_req_type(&req_type)?;
        let fmt = parse_flatfile_format(format.as_deref())?;
        let client = Arc::clone(&self.client);
        let path_buf = std::path::PathBuf::from(path);
        let final_path = spawn_endpoint_task(async move {
            client
                .flatfile_request(sec, req, &date, &path_buf, fmt)
                .await
        })
        .await?;
        Ok(final_path.to_string_lossy().into_owned())
    }
}
