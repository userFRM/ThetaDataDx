//! Hand-written napi-rs bindings for the FLATFILES surface.
//!
//! Mirrors the Python wrapper at `thetadatadx-py/src/flatfile_methods.rs`:
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
use serde_json::Value as JsonValue;

use thetadatadx::flatfiles::{self, FlatFileFormat, FlatFileRow, FlatFileValue, ReqType, SecType};

use crate::{invalid_parameter_err, spawn_endpoint_task, to_napi_err};

// ── Helpers ────────────────────────────────────────────────────────────

/// Pull and decode a flat-file blob off the libuv thread.
///
/// A flat-file pull is a full-day blob download — seconds of network
/// transfer and a large decode. Running it on the runtime's execution
/// thread via [`spawn_endpoint_task`] keeps the Node event loop free for
/// the whole call, matching every market-data endpoint. Callers are
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

/// Parse the string args, pull + decode the blob off the libuv thread, and
/// write the requested format to `path`. Shared verbatim by `Client` and
/// `MarketDataClient` — both hold an `Arc<thetadatadx::Client>` and open the
/// same data channel, so the napi methods only forward to this.
async fn flat_file_to_path_impl(
    client: &Arc<thetadatadx::Client>,
    sec_type: String,
    req_type: String,
    date: String,
    path: String,
    format: Option<String>,
) -> napi::Result<String> {
    let sec = sec_type.parse::<SecType>().map_err(invalid_parameter_err)?;
    let req = req_type.parse::<ReqType>().map_err(invalid_parameter_err)?;
    let fmt = format
        .as_deref()
        .map_or(Ok(FlatFileFormat::Csv), str::parse)
        .map_err(invalid_parameter_err)?;
    let client = Arc::clone(client);
    let path_buf = std::path::PathBuf::from(path);
    let final_path = spawn_endpoint_task(async move {
        client
            .flatfile_request(sec, req, &date, &path_buf, fmt)
            .await
    })
    .await?;
    Ok(final_path.to_string_lossy().into_owned())
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
    ///
    /// Keys keep the row's column order: `symbol`, `expiration`, `strike`,
    /// `right`, then the vendor's columns in file order, as Python's
    /// `to_list` does.
    #[napi(js_name = "toJson")]
    pub fn to_json(&self) -> String {
        // Written by hand: `serde_json::Map` sorts its keys unless its
        // `preserve_order` feature is on, and turning that on here would also
        // reorder the keys of the SDK's own JSON file writers in this build.
        let mut out = String::from("[");
        for (i, row) in self.rows.iter().enumerate() {
            if i > 0 {
                out.push(',');
            }
            let contract = [
                ("symbol", JsonValue::from(row.symbol.as_str())),
                (
                    "expiration",
                    row.expiration.map_or(JsonValue::Null, JsonValue::from),
                ),
                (
                    "strike",
                    row.strike.map_or(JsonValue::Null, JsonValue::from),
                ),
                (
                    "right",
                    row.right
                        .map_or(JsonValue::Null, |c| JsonValue::String(c.to_string())),
                ),
            ];
            let fields = row.fields.iter().map(|(name, value)| {
                let value = match value {
                    FlatFileValue::Int(v) => JsonValue::from(*v),
                    FlatFileValue::Price(v) => JsonValue::from(*v),
                };
                (name.as_str(), value)
            });
            for (j, (name, value)) in contract.into_iter().chain(fields).enumerate() {
                out.push(if j == 0 { '{' } else { ',' });
                out.push_str(&JsonValue::from(name).to_string());
                out.push(':');
                out.push_str(&value.to_string());
            }
            out.push('}');
        }
        out.push(']');
        out
    }
}

#[cfg(test)]
mod to_json_tests {
    use super::{FlatFileRowList, FlatFileValue};
    use thetadatadx::flatfiles::FlatFileRow;

    /// The vendor's columns come out in the order the file carries them, after
    /// the contract columns, not sorted by name.
    #[test]
    fn keys_keep_the_row_column_order() {
        let list = FlatFileRowList {
            rows: vec![FlatFileRow {
                symbol: "SPY".into(),
                expiration: Some(20_240_315),
                strike: Some(500.0),
                right: Some('C'),
                fields: vec![
                    ("ms_of_day".into(), FlatFileValue::Int(34_200_000)),
                    ("open".into(), FlatFileValue::Price(1.25)),
                    ("close".into(), FlatFileValue::Price(1.5)),
                ],
            }],
        };
        assert_eq!(
            list.to_json(),
            r#"[{"symbol":"SPY","expiration":20240315,"strike":500.0,"right":"C","ms_of_day":34200000,"open":1.25,"close":1.5}]"#
        );
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
        let sec = sec_type.parse::<SecType>().map_err(invalid_parameter_err)?;
        let req = req_type.parse::<ReqType>().map_err(invalid_parameter_err)?;
        let rows = pull_decoded(&self.client, sec, req, &date).await?;
        Ok(FlatFileRowList { rows })
    }
}

// ── Client napi extension ─────────────────────────────────────────

use crate::{Client, MarketDataClient};

#[napi]
impl Client {
    /// FLATFILES namespace handle. Cheap — shares the underlying client connection.
    #[napi(getter, js_name = "flatFiles")]
    pub fn flat_files(&self) -> napi::Result<FlatFilesNamespace> {
        Ok(FlatFilesNamespace {
            client: self.client_handle()?,
        })
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
        flat_file_to_path_impl(
            &self.client_handle()?,
            sec_type,
            req_type,
            date,
            path,
            format,
        )
        .await
    }
}

// ── MarketDataClient napi extension ────────────────────────────────

#[napi]
impl MarketDataClient {
    /// FLATFILES namespace handle. Cheap — shares the underlying client connection.
    /// The market-data-only client opens the same data channel as the unified
    /// client, so the full flat-file surface is reachable here unchanged.
    #[napi(getter, js_name = "flatFiles")]
    pub fn flat_files(&self) -> napi::Result<FlatFilesNamespace> {
        Ok(FlatFilesNamespace {
            client: self.client_handle()?,
        })
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
        flat_file_to_path_impl(
            &self.client_handle()?,
            sec_type,
            req_type,
            date,
            path,
            format,
        )
        .await
    }
}
