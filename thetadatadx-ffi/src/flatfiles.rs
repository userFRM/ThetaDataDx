//! C ABI for the FLATFILES surface.
//!
//! Exposes:
//!
//! - `thetadatadx_flatfile_request_decoded` — pull + decode + return an opaque
//!   row-list handle.
//! - `thetadatadx_flatfile_rows_to_arrow_ipc` — serialise the row list as Arrow
//!   IPC bytes for any consumer with an Arrow IPC reader (apache-arrow,
//!   pyarrow, arrow-cpp).
//! - `thetadatadx_flatfile_rows_count` — row count without materialising bytes.
//! - `thetadatadx_flatfile_rowlist_free` — release the row-list handle.
//! - `thetadatadx_flatfile_request_to_path` — pull + write raw vendor format
//!   directly to disk.
//!
//! The opaque `ThetaDataDxFlatFileRowList` carries the typed `Vec<FlatFileRow>`
//! so language wrappers can defer the schema-inferring Arrow conversion
//! until the user picks a representation.

use std::os::raw::c_char;
use std::ptr;

use thetadatadx::flatfiles::{self, FlatFileFormat, FlatFileRow, ReqType, SecType};

use crate::error::{
    cstr_to_str, set_error, set_error_from, set_error_with_code, THETADATADX_ERR_INVALID_PARAMETER,
};
use crate::runtime;
use crate::streaming::ThetaDataDxClient;
use crate::types::ThetaDataDxMarketDataClient;

// ── Heap-owned row-list handle ─────────────────────────────────────────

/// Opaque handle wrapping a decoded `Vec<FlatFileRow>`. Allocated by
/// `thetadatadx_flatfile_request_decoded`; freed by `thetadatadx_flatfile_rowlist_free`.
pub struct ThetaDataDxFlatFileRowList {
    pub(crate) rows: Vec<FlatFileRow>,
}

/// Heap-owned byte buffer (Arrow IPC stream) returned by
/// `thetadatadx_flatfile_rows_to_arrow_ipc`. Caller MUST free with
/// `thetadatadx_flatfile_bytes_free`.
#[repr(C)]
pub struct ThetaDataDxFlatFileBytes {
    /// Pointer to the first byte of the buffer; null when empty.
    pub data: *const u8,
    /// Length of the buffer in bytes.
    pub len: usize,
}

// Layout drift-guard: pin the LP64 `#[repr(C)]` size + alignment on the
// Rust side, the same values `abi_struct_layout_asserts.hpp.inc` pins. A
// field-width or member-order change that shifts the layout fails the build
// here; the C++ static_asserts alone cannot catch a Rust-side change.
const _: () = {
    assert!(core::mem::size_of::<ThetaDataDxFlatFileBytes>() == 16);
    assert!(core::mem::align_of::<ThetaDataDxFlatFileBytes>() == 8);
};

impl ThetaDataDxFlatFileBytes {
    fn from_vec(buf: Vec<u8>) -> Self {
        if buf.is_empty() {
            return Self {
                data: ptr::null(),
                len: 0,
            };
        }
        let (data, len) = crate::types::box_buf(buf);
        Self { data, len }
    }
}

// ── Helpers ────────────────────────────────────────────────────────────

unsafe fn parse_sec(raw: *const c_char) -> Result<SecType, String> {
    // SAFETY: caller supplies a NUL-terminated C string allocated by the host runtime; cstr_to_str validates non-null + UTF-8.
    let s = unsafe { cstr_to_str(raw) }
        .map_err(|e| format!("sec_type is not valid UTF-8: {e}"))?
        .ok_or_else(|| "sec_type is null".to_string())?;
    s.parse()
}

unsafe fn parse_req(raw: *const c_char) -> Result<ReqType, String> {
    // SAFETY: caller supplies a NUL-terminated C string allocated by the host runtime; cstr_to_str validates non-null + UTF-8.
    let s = unsafe { cstr_to_str(raw) }
        .map_err(|e| format!("req_type is not valid UTF-8: {e}"))?
        .ok_or_else(|| "req_type is null".to_string())?;
    s.parse()
}

unsafe fn parse_fmt(raw: *const c_char) -> Result<FlatFileFormat, String> {
    // SAFETY: caller supplies a NUL-terminated C string allocated by the host runtime; cstr_to_str validates non-null + UTF-8.
    let s = unsafe { cstr_to_str(raw) }
        .map_err(|e| format!("format is not valid UTF-8: {e}"))?
        .unwrap_or("csv");
    s.parse()
}

// ── FFI entry points ───────────────────────────────────────────────────

/// Parse the shared `(sec_type, req_type, date)` request arguments. On any
/// parse fault it sets the thread-local FFI error and returns `None`; the
/// caller then returns its own null / `-1` sentinel. `date` is returned
/// owned so the borrow of the raw pointer does not outlive this call.
///
/// # Safety
/// Each pointer must be a NUL-terminated C string the caller pins for the
/// call duration (or null for `date`, rejected here).
unsafe fn parse_ff_args(
    sec_type: *const c_char,
    req_type: *const c_char,
    date: *const c_char,
) -> Option<(SecType, ReqType, String)> {
    // SAFETY: `sec_type` is a caller-pinned NUL-terminated C string; `parse_sec` validates non-null + UTF-8 before reading.
    let sec = match unsafe { parse_sec(sec_type) } {
        Ok(v) => v,
        Err(e) => {
            set_error_with_code(&e, THETADATADX_ERR_INVALID_PARAMETER);
            return None;
        }
    };
    // SAFETY: `req_type` is a caller-pinned NUL-terminated C string; `parse_req` validates non-null + UTF-8 before reading.
    let req = match unsafe { parse_req(req_type) } {
        Ok(v) => v,
        Err(e) => {
            set_error_with_code(&e, THETADATADX_ERR_INVALID_PARAMETER);
            return None;
        }
    };
    // SAFETY: `date` is a caller-pinned NUL-terminated C string; `cstr_to_str` validates non-null + UTF-8 before reading.
    let date = match unsafe { cstr_to_str(date) } {
        Ok(Some(s)) => s.to_owned(),
        Ok(None) => {
            set_error("date is null");
            return None;
        }
        Err(e) => {
            set_error(&format!("date is not valid UTF-8: {e}"));
            return None;
        }
    };
    Some((sec, req, date))
}

/// Pull a decoded flat-file blob for `(sec_type, req_type, date)` from the
/// unified client and return an opaque row-list handle. Returns null on
/// error; check `thetadatadx_last_error()` for details.
///
/// The returned handle MUST be freed with `thetadatadx_flatfile_rowlist_free`.
#[no_mangle]
pub unsafe extern "C" fn thetadatadx_flatfile_request_decoded(
    handle: *const ThetaDataDxClient,
    sec_type: *const c_char,
    req_type: *const c_char,
    date: *const c_char,
) -> *mut ThetaDataDxFlatFileRowList {
    ffi_boundary!(ptr::null_mut(), {
        if handle.is_null() {
            set_error("unified handle is null");
            return ptr::null_mut();
        }
        // SAFETY: request-arg pointers are caller-pinned NUL-terminated C strings.
        let Some((sec, req, date)) = (unsafe { parse_ff_args(sec_type, req_type, date) }) else {
            return ptr::null_mut();
        };
        // SAFETY: handle is a non-null pointer returned by the matching thetadatadx_*_new and not yet passed to thetadatadx_*_free.
        let unified = unsafe { &*handle };
        flatfile_rowlist_or_null(
            runtime().block_on(unified.inner.flatfile_request_decoded(sec, req, &date)),
        )
    })
}

/// Pull a decoded flat-file blob from a standalone [`ThetaDataDxMarketDataClient`].
/// Flat files are account-authenticated market data, so the market-data
/// handle exposes the identical surface as the unified client. Returns null
/// on error; the returned handle MUST be freed with
/// `thetadatadx_flatfile_rowlist_free`.
#[no_mangle]
pub unsafe extern "C" fn thetadatadx_market_data_flatfile_request_decoded(
    handle: *const ThetaDataDxMarketDataClient,
    sec_type: *const c_char,
    req_type: *const c_char,
    date: *const c_char,
) -> *mut ThetaDataDxFlatFileRowList {
    ffi_boundary!(ptr::null_mut(), {
        if handle.is_null() {
            set_error("market-data handle is null");
            return ptr::null_mut();
        }
        // SAFETY: request-arg pointers are caller-pinned NUL-terminated C strings.
        let Some((sec, req, date)) = (unsafe { parse_ff_args(sec_type, req_type, date) }) else {
            return ptr::null_mut();
        };
        // SAFETY: handle is a non-null pointer returned by the matching thetadatadx_*_new and not yet passed to thetadatadx_*_free.
        let mdc = unsafe { &*handle };
        flatfile_rowlist_or_null(
            runtime().block_on(mdc.inner.flatfile_request_decoded(sec, req, &date)),
        )
    })
}

/// Box a decoded row vector into an opaque row-list handle, or set the FFI
/// error and return null.
fn flatfile_rowlist_or_null(
    res: Result<Vec<FlatFileRow>, thetadatadx::Error>,
) -> *mut ThetaDataDxFlatFileRowList {
    match res {
        Ok(rows) => Box::into_raw(Box::new(ThetaDataDxFlatFileRowList { rows })),
        Err(e) => {
            set_error_from(&e);
            ptr::null_mut()
        }
    }
}

/// Number of rows in a row-list handle. Returns 0 if the handle is null.
#[no_mangle]
pub unsafe extern "C" fn thetadatadx_flatfile_rows_count(
    rowlist: *const ThetaDataDxFlatFileRowList,
) -> usize {
    ffi_boundary!(0, {
        if rowlist.is_null() {
            return 0;
        }
        // SAFETY: caller's contract on this FFI function requires
        // `rowlist` to be either null (rejected above) or the value
        // returned by `thetadatadx_flatfile_request_decoded`, which built it
        // via `Box::into_raw(Box::new(ThetaDataDxFlatFileRowList { .. }))`.
        // No mutating call (only `thetadatadx_flatfile_rowlist_free`, which
        // consumes the pointer) runs concurrently — single-threaded
        // FFI ownership — so the box is live, `#[repr(Rust)]`
        // well-aligned, and a shared `&ThetaDataDxFlatFileRowList` reborrow
        // (`(*rowlist).rows.len()` reads only the `len` field of the
        // inner `Vec`, no field of `rowlist` is mutated) is sound.
        unsafe { (*rowlist).rows.len() }
    })
}

/// Serialise the row list as Arrow IPC stream bytes. The schema is
/// inferred from the first row by `flatfiles::arrow::rows_to_arrow`.
///
/// Returns `(data=null, len=0)` on error; check `thetadatadx_last_error()`.
/// Caller MUST free the returned bytes with `thetadatadx_flatfile_bytes_free`.
#[no_mangle]
pub unsafe extern "C" fn thetadatadx_flatfile_rows_to_arrow_ipc(
    rowlist: *const ThetaDataDxFlatFileRowList,
) -> ThetaDataDxFlatFileBytes {
    ffi_boundary!(
        ThetaDataDxFlatFileBytes {
            data: ptr::null(),
            len: 0
        },
        {
            if rowlist.is_null() {
                set_error("row list handle is null");
                return ThetaDataDxFlatFileBytes {
                    data: ptr::null(),
                    len: 0,
                };
            }
            // SAFETY: caller's contract on this FFI function requires
            // `rowlist` to be either null (rejected above) or the value
            // returned by `thetadatadx_flatfile_request_decoded`, which built
            // it via `Box::into_raw(Box::new(ThetaDataDxFlatFileRowList { .. }))`.
            // The reborrowed `&Vec<FlatFileRow>` lives only for the
            // duration of this expression (it is consumed by
            // `rows_to_arrow` synchronously below); since the only
            // function that invalidates the box —
            // `thetadatadx_flatfile_rowlist_free` — takes `*mut` and cannot run
            // concurrently across a single FFI call, the borrow is
            // valid for that span.
            let rows = unsafe { &(*rowlist).rows };
            let batch = match flatfiles::arrow::rows_to_arrow(rows) {
                Ok(b) => b,
                Err(e) => {
                    set_error_from(&e);
                    return ThetaDataDxFlatFileBytes {
                        data: ptr::null(),
                        len: 0,
                    };
                }
            };
            match crate::streaming_batches_ipc::batch_to_ipc(&batch, 0) {
                Ok(buf) => ThetaDataDxFlatFileBytes::from_vec(buf),
                Err(e) => {
                    set_error(&e);
                    ThetaDataDxFlatFileBytes {
                        data: ptr::null(),
                        len: 0,
                    }
                }
            }
        }
    )
}

/// Free a byte buffer returned by `thetadatadx_flatfile_rows_to_arrow_ipc`.
#[no_mangle]
pub unsafe extern "C" fn thetadatadx_flatfile_bytes_free(bytes: ThetaDataDxFlatFileBytes) {
    ffi_boundary!((), {
        if !bytes.data.is_null() && bytes.len > 0 {
            // SAFETY: `bytes.data` was returned by `Box::into_raw` on a `Box<[u8]>` of length `bytes.len`; ownership returns to Rust here for drop. Null + zero-len gated by the surrounding `if`.
            let _ = unsafe {
                Box::from_raw(std::ptr::slice_from_raw_parts_mut(
                    bytes.data.cast_mut(),
                    bytes.len,
                ))
            };
        }
    })
}

/// Free a row-list handle returned by `thetadatadx_flatfile_request_decoded`.
#[no_mangle]
pub unsafe extern "C" fn thetadatadx_flatfile_rowlist_free(
    rowlist: *mut ThetaDataDxFlatFileRowList,
) {
    ffi_boundary!((), {
        if !rowlist.is_null() {
            // SAFETY: the pointer was returned by Box::into_raw / thetadatadx_*_new and has not been freed; ownership returns to Rust.
            drop(unsafe { Box::from_raw(rowlist) });
        }
    })
}

/// Parse the `to_path`-only trailing args (`format`, `path`) shared by the
/// unified and market-data write paths. On any parse fault it sets the FFI
/// error and returns `None`. `path` is returned owned so the borrow of the
/// raw pointer does not outlive this call.
///
/// # Safety
/// `format` is a NUL-terminated C string or null (the `csv` default);
/// `path` is a caller-pinned NUL-terminated C string.
unsafe fn parse_ff_write_args(
    format: *const c_char,
    path: *const c_char,
) -> Option<(FlatFileFormat, String)> {
    // SAFETY: `format` is validated (UTF-8, or null → csv) by `parse_fmt`.
    let fmt = match unsafe { parse_fmt(format) } {
        Ok(v) => v,
        Err(e) => {
            set_error_with_code(&e, THETADATADX_ERR_INVALID_PARAMETER);
            return None;
        }
    };
    // SAFETY: `path` is a caller-pinned NUL-terminated C string; `cstr_to_str` validates non-null + UTF-8 before reading.
    let path = match unsafe { cstr_to_str(path) } {
        Ok(Some(s)) => s.to_owned(),
        Ok(None) => {
            set_error("path is null");
            return None;
        }
        Err(e) => {
            set_error(&format!("path is not valid UTF-8: {e}"));
            return None;
        }
    };
    Some((fmt, path))
}

/// Pull a flat-file blob from the unified client and write the requested
/// vendor format (`csv` / `json` / `jsonl` / `ndjson` / `html`) directly to
/// `path`. Skips the typed-row decode step. Returns 0 on success, -1 on
/// error; check `thetadatadx_last_error()`.
#[no_mangle]
pub unsafe extern "C" fn thetadatadx_flatfile_request_to_path(
    handle: *const ThetaDataDxClient,
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
        // SAFETY: request-arg pointers are caller-pinned NUL-terminated C strings.
        let Some((sec, req, date)) = (unsafe { parse_ff_args(sec_type, req_type, date) }) else {
            return -1;
        };
        // SAFETY: `format` / `path` are caller-pinned (format may be null → csv).
        let Some((fmt, path)) = (unsafe { parse_ff_write_args(format, path) }) else {
            return -1;
        };
        // SAFETY: handle is a non-null pointer returned by the matching thetadatadx_*_new and not yet passed to thetadatadx_*_free.
        let unified = unsafe { &*handle };
        flatfile_write_rc(runtime().block_on(unified.inner.flatfile_request(
            sec,
            req,
            &date,
            std::path::Path::new(&path),
            fmt,
        )))
    })
}

/// Pull a flat-file blob from a standalone [`ThetaDataDxMarketDataClient`] and
/// write it to `path`. Identical surface to the unified write path — flat
/// files are account-authenticated market data. Returns 0 on success, -1 on
/// error; check `thetadatadx_last_error()`.
#[no_mangle]
pub unsafe extern "C" fn thetadatadx_market_data_flatfile_request_to_path(
    handle: *const ThetaDataDxMarketDataClient,
    sec_type: *const c_char,
    req_type: *const c_char,
    date: *const c_char,
    path: *const c_char,
    format: *const c_char,
) -> i32 {
    ffi_boundary!(-1, {
        if handle.is_null() {
            set_error("market-data handle is null");
            return -1;
        }
        // SAFETY: request-arg pointers are caller-pinned NUL-terminated C strings.
        let Some((sec, req, date)) = (unsafe { parse_ff_args(sec_type, req_type, date) }) else {
            return -1;
        };
        // SAFETY: `format` / `path` are caller-pinned (format may be null → csv).
        let Some((fmt, path)) = (unsafe { parse_ff_write_args(format, path) }) else {
            return -1;
        };
        // SAFETY: handle is a non-null pointer returned by the matching thetadatadx_*_new and not yet passed to thetadatadx_*_free.
        let mdc = unsafe { &*handle };
        flatfile_write_rc(runtime().block_on(mdc.inner.flatfile_request(
            sec,
            req,
            &date,
            std::path::Path::new(&path),
            fmt,
        )))
    })
}

/// Map a flat-file write result to the C return code (0 / -1), setting the
/// FFI error on failure.
fn flatfile_write_rc(res: Result<std::path::PathBuf, thetadatadx::Error>) -> i32 {
    match res {
        Ok(_) => 0,
        Err(e) => {
            set_error_from(&e);
            -1
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{parse_ff_args, parse_ff_write_args};
    use crate::error::{thetadatadx_last_error_code, THETADATADX_ERR_INVALID_PARAMETER};

    /// An unknown sec_type, req_type or format is a bad caller value, so it
    /// carries the invalid-parameter code the C++ `InvalidParameterError`
    /// and the other bindings key on.
    #[test]
    fn unknown_flat_file_names_are_invalid_parameters() {
        let assert_invalid = |name: &str, rejected: bool| {
            assert!(rejected, "an unknown {name} was accepted");
            assert_eq!(
                thetadatadx_last_error_code(),
                THETADATADX_ERR_INVALID_PARAMETER,
                "an unknown {name} is not typed as an invalid parameter",
            );
        };
        // SAFETY: NUL-terminated literals, alive for the call.
        let rejected =
            unsafe { parse_ff_args(c"FUTURE".as_ptr(), c"EOD".as_ptr(), c"20240105".as_ptr()) };
        assert_invalid("sec_type", rejected.is_none());
        // SAFETY: NUL-terminated literals, alive for the call.
        let rejected =
            unsafe { parse_ff_args(c"OPTION".as_ptr(), c"BOGUS".as_ptr(), c"20240105".as_ptr()) };
        assert_invalid("req_type", rejected.is_none());
        // SAFETY: NUL-terminated literals, alive for the call.
        let rejected = unsafe { parse_ff_write_args(c"parquet".as_ptr(), c"out.parquet".as_ptr()) };
        assert_invalid("format", rejected.is_none());
    }
}
