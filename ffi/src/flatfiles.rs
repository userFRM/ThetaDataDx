//! C ABI for the FLATFILES surface.
//!
//! Exposes:
//!
//! - `tdx_flatfile_request_decoded` — pull + decode + return an opaque
//!   row-list handle.
//! - `tdx_flatfile_rows_to_arrow_ipc` — serialise the row list as Arrow
//!   IPC bytes for any consumer with an Arrow IPC reader (apache-arrow,
//!   pyarrow, arrow-cpp).
//! - `tdx_flatfile_rows_count` — row count without materialising bytes.
//! - `tdx_flatfile_rowlist_free` — release the row-list handle.
//! - `tdx_flatfile_request_to_path` — pull + write raw vendor format
//!   directly to disk.
//!
//! The opaque `TdxFlatFileRowList` carries the typed `Vec<FlatFileRow>`
//! so language wrappers can defer the schema-inferring Arrow conversion
//! until the user picks a representation.

use std::io::Cursor;
use std::os::raw::c_char;
use std::ptr;

use arrow_ipc::writer::StreamWriter;
use thetadatadx::flatfiles::{self, FlatFileFormat, FlatFileRow, ReqType, SecType};

use crate::error::{cstr_to_str, set_error};
use crate::runtime;
use crate::streaming::TdxUnified;

// ── Heap-owned row-list handle ─────────────────────────────────────────

/// Opaque handle wrapping a decoded `Vec<FlatFileRow>`. Allocated by
/// `tdx_flatfile_request_decoded`; freed by `tdx_flatfile_rowlist_free`.
pub struct TdxFlatFileRowList {
    pub(crate) rows: Vec<FlatFileRow>,
}

/// Heap-owned byte buffer (Arrow IPC stream) returned by
/// `tdx_flatfile_rows_to_arrow_ipc`. Caller MUST free with
/// `tdx_flatfile_bytes_free`.
#[repr(C)]
pub struct TdxFlatFileBytes {
    pub data: *const u8,
    pub len: usize,
}

impl TdxFlatFileBytes {
    fn from_vec(buf: Vec<u8>) -> Self {
        if buf.is_empty() {
            return Self {
                data: ptr::null(),
                len: 0,
            };
        }
        let boxed = buf.into_boxed_slice();
        let len = boxed.len();
        let data = Box::into_raw(boxed) as *const u8;
        Self { data, len }
    }
}

// ── Helpers ────────────────────────────────────────────────────────────

unsafe fn parse_sec(raw: *const c_char) -> Result<SecType, String> {
    let s = unsafe { cstr_to_str(raw) }
        .map_err(|e| format!("sec_type is not valid UTF-8: {e}"))?
        .ok_or_else(|| "sec_type is null".to_string())?;
    match s.to_uppercase().as_str() {
        "OPTION" => Ok(SecType::Option),
        "STOCK" => Ok(SecType::Stock),
        "INDEX" => Ok(SecType::Index),
        other => Err(format!(
            "unknown sec_type: {other:?} (expected OPTION, STOCK, or INDEX)"
        )),
    }
}

unsafe fn parse_req(raw: *const c_char) -> Result<ReqType, String> {
    let s = unsafe { cstr_to_str(raw) }
        .map_err(|e| format!("req_type is not valid UTF-8: {e}"))?
        .ok_or_else(|| "req_type is null".to_string())?;
    match s.to_uppercase().as_str() {
        "EOD" => Ok(ReqType::Eod),
        "QUOTE" => Ok(ReqType::Quote),
        "OPEN_INTEREST" | "OPENINTEREST" => Ok(ReqType::OpenInterest),
        "OHLC" => Ok(ReqType::Ohlc),
        "TRADE" => Ok(ReqType::Trade),
        "TRADE_QUOTE" | "TRADEQUOTE" => Ok(ReqType::TradeQuote),
        other => Err(format!(
            "unknown req_type: {other:?} (expected EOD, QUOTE, OPEN_INTEREST, OHLC, TRADE, TRADE_QUOTE)"
        )),
    }
}

unsafe fn parse_fmt(raw: *const c_char) -> Result<FlatFileFormat, String> {
    let s = unsafe { cstr_to_str(raw) }
        .map_err(|e| format!("format is not valid UTF-8: {e}"))?
        .unwrap_or("csv");
    match s.to_lowercase().as_str() {
        "csv" => Ok(FlatFileFormat::Csv),
        "jsonl" | "json" => Ok(FlatFileFormat::Jsonl),
        other => Err(format!(
            "unknown flat-file format: {other:?} (expected csv or jsonl)"
        )),
    }
}

// ── FFI entry points ───────────────────────────────────────────────────

/// Pull a decoded flat-file blob for `(sec_type, req_type, date)` and
/// return an opaque row-list handle. Returns null on error; check
/// `tdx_last_error()` for details.
///
/// The returned handle MUST be freed with `tdx_flatfile_rowlist_free`.
#[no_mangle]
pub unsafe extern "C" fn tdx_flatfile_request_decoded(
    handle: *const TdxUnified,
    sec_type: *const c_char,
    req_type: *const c_char,
    date: *const c_char,
) -> *mut TdxFlatFileRowList {
    ffi_boundary!(ptr::null_mut(), {
        if handle.is_null() {
            set_error("unified handle is null");
            return ptr::null_mut();
        }
        let sec = match unsafe { parse_sec(sec_type) } {
            Ok(v) => v,
            Err(e) => {
                set_error(&e);
                return ptr::null_mut();
            }
        };
        let req = match unsafe { parse_req(req_type) } {
            Ok(v) => v,
            Err(e) => {
                set_error(&e);
                return ptr::null_mut();
            }
        };
        let date_str = match unsafe { cstr_to_str(date) } {
            Ok(Some(s)) => s,
            Ok(None) => {
                set_error("date is null");
                return ptr::null_mut();
            }
            Err(e) => {
                set_error(&format!("date is not valid UTF-8: {e}"));
                return ptr::null_mut();
            }
        };
        let unified = unsafe { &*handle };
        let res = runtime().block_on(unified.inner.flatfile_request_decoded(sec, req, date_str));
        match res {
            Ok(rows) => Box::into_raw(Box::new(TdxFlatFileRowList { rows })),
            Err(e) => {
                set_error(&e.to_string());
                ptr::null_mut()
            }
        }
    })
}

/// Number of rows in a row-list handle. Returns 0 if the handle is null.
#[no_mangle]
pub unsafe extern "C" fn tdx_flatfile_rows_count(rowlist: *const TdxFlatFileRowList) -> usize {
    ffi_boundary!(0, {
        if rowlist.is_null() {
            return 0;
        }
        unsafe { (*rowlist).rows.len() }
    })
}

/// Serialise the row list as Arrow IPC stream bytes. The schema is
/// inferred from the first row by `flatfiles::arrow::rows_to_arrow`.
///
/// Returns `(data=null, len=0)` on error; check `tdx_last_error()`.
/// Caller MUST free the returned bytes with `tdx_flatfile_bytes_free`.
#[no_mangle]
pub unsafe extern "C" fn tdx_flatfile_rows_to_arrow_ipc(
    rowlist: *const TdxFlatFileRowList,
) -> TdxFlatFileBytes {
    ffi_boundary!(
        TdxFlatFileBytes {
            data: ptr::null(),
            len: 0
        },
        {
            if rowlist.is_null() {
                set_error("row list handle is null");
                return TdxFlatFileBytes {
                    data: ptr::null(),
                    len: 0,
                };
            }
            let rows = unsafe { &(*rowlist).rows };
            let batch = match flatfiles::arrow::rows_to_arrow(rows) {
                Ok(b) => b,
                Err(e) => {
                    set_error(&e.to_string());
                    return TdxFlatFileBytes {
                        data: ptr::null(),
                        len: 0,
                    };
                }
            };
            let schema = batch.schema();
            let mut buf: Vec<u8> = Vec::new();
            {
                let mut writer = match StreamWriter::try_new(Cursor::new(&mut buf), &schema) {
                    Ok(w) => w,
                    Err(e) => {
                        set_error(&format!("arrow ipc writer init failed: {e}"));
                        return TdxFlatFileBytes {
                            data: ptr::null(),
                            len: 0,
                        };
                    }
                };
                if let Err(e) = writer.write(&batch) {
                    set_error(&format!("arrow ipc write failed: {e}"));
                    return TdxFlatFileBytes {
                        data: ptr::null(),
                        len: 0,
                    };
                }
                if let Err(e) = writer.finish() {
                    set_error(&format!("arrow ipc finish failed: {e}"));
                    return TdxFlatFileBytes {
                        data: ptr::null(),
                        len: 0,
                    };
                }
            }
            TdxFlatFileBytes::from_vec(buf)
        }
    )
}

/// Free a byte buffer returned by `tdx_flatfile_rows_to_arrow_ipc`.
#[no_mangle]
pub unsafe extern "C" fn tdx_flatfile_bytes_free(bytes: TdxFlatFileBytes) {
    ffi_boundary!((), {
        if !bytes.data.is_null() && bytes.len > 0 {
            let _ = unsafe {
                Box::from_raw(std::ptr::slice_from_raw_parts_mut(
                    bytes.data.cast_mut(),
                    bytes.len,
                ))
            };
        }
    })
}

/// Free a row-list handle returned by `tdx_flatfile_request_decoded`.
#[no_mangle]
pub unsafe extern "C" fn tdx_flatfile_rowlist_free(rowlist: *mut TdxFlatFileRowList) {
    ffi_boundary!((), {
        if !rowlist.is_null() {
            drop(unsafe { Box::from_raw(rowlist) });
        }
    })
}

/// Pull a flat-file blob and write the requested vendor format
/// (`csv` / `jsonl`) directly to `path`. Skips the typed-row decode
/// step. Returns 0 on success, -1 on error; check `tdx_last_error()`.
#[no_mangle]
pub unsafe extern "C" fn tdx_flatfile_request_to_path(
    handle: *const TdxUnified,
    sec_type: *const c_char,
    req_type: *const c_char,
    date: *const c_char,
    path: *const c_char,
    format: *const c_char,
) -> i32 {
    ffi_boundary!(-1, {
        if handle.is_null() {
            set_error("unified handle is null");
            return -1;
        }
        let sec = match unsafe { parse_sec(sec_type) } {
            Ok(v) => v,
            Err(e) => {
                set_error(&e);
                return -1;
            }
        };
        let req = match unsafe { parse_req(req_type) } {
            Ok(v) => v,
            Err(e) => {
                set_error(&e);
                return -1;
            }
        };
        let fmt = match unsafe { parse_fmt(format) } {
            Ok(v) => v,
            Err(e) => {
                set_error(&e);
                return -1;
            }
        };
        let date_str = match unsafe { cstr_to_str(date) } {
            Ok(Some(s)) => s,
            Ok(None) => {
                set_error("date is null");
                return -1;
            }
            Err(e) => {
                set_error(&format!("date is not valid UTF-8: {e}"));
                return -1;
            }
        };
        let path_str = match unsafe { cstr_to_str(path) } {
            Ok(Some(s)) => s,
            Ok(None) => {
                set_error("path is null");
                return -1;
            }
            Err(e) => {
                set_error(&format!("path is not valid UTF-8: {e}"));
                return -1;
            }
        };
        let unified = unsafe { &*handle };
        match runtime().block_on(unified.inner.flatfile_request(
            sec,
            req,
            date_str,
            std::path::Path::new(path_str),
            fmt,
        )) {
            Ok(_) => 0,
            Err(e) => {
                set_error(&e.to_string());
                -1
            }
        }
    })
}
