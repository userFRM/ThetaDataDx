//! `#[repr(C)]` handle types, tick-array wrappers, `TdxStringArray`,
//! `TdxOptionContract*`, plus the shared string / slice helpers that cross
//! the FFI boundary.
//!
//! All symbols declared `#[no_mangle] extern "C" fn` here keep their original
//! names. The split from `lib.rs` is purely organizational — the exported C
//! ABI surface is identical.

use std::ffi::CString;
use std::os::raw::c_char;
use std::ptr;

use crate::error::{cstr_to_str, set_error};

// ── Opaque handle types ──

/// Opaque credentials handle.
pub struct TdxCredentials {
    pub(crate) inner: thetadatadx::Credentials,
}

/// Opaque client handle.
///
/// `repr(transparent)` guarantees `*const TdxClient` and `*const MddsClient`
/// have identical layout, allowing safe pointer casts in `tdx_unified_historical()`.
#[repr(transparent)]
pub struct TdxClient {
    pub(crate) inner: thetadatadx::mdds::MddsClient,
}

/// Opaque config handle.
pub struct TdxConfig {
    pub(crate) inner: thetadatadx::DirectConfig,
}

// ── C-string lifetime helpers (shared across all endpoint modules) ──

/// Free a string returned by any `tdx_*` function.
///
/// MUST be called for every non-null `*mut c_char` returned by this library.
#[no_mangle]
pub unsafe extern "C" fn tdx_string_free(s: *mut c_char) {
    ffi_boundary!((), {
        if !s.is_null() {
            // SAFETY: the pointer was produced by CString::into_raw on the matching free path, ownership returns to Rust here.
            drop(unsafe { CString::from_raw(s) });
        }
    })
}

// ── Endpoint-argument coercion helpers (shared with the generated
//    `endpoint_with_options.rs` include) ──

pub(crate) fn insert_optional_str_arg(
    args: &mut thetadatadx::EndpointArgs,
    key: &str,
    raw: *const c_char,
) -> Result<(), String> {
    // SAFETY: caller supplies a NUL-terminated C string allocated by the host runtime; cstr_to_str validates non-null + UTF-8.
    match unsafe { cstr_to_str(raw) } {
        Ok(None) => Ok(()),
        Ok(Some(value)) => {
            args.insert(
                key.to_string(),
                thetadatadx::EndpointArgValue::Str(value.to_string()),
            );
            Ok(())
        }
        Err(e) => Err(format!("{key} is not valid UTF-8: {e}")),
    }
}

pub(crate) fn insert_int_arg(args: &mut thetadatadx::EndpointArgs, key: &str, value: i32) {
    args.insert(
        key.to_string(),
        thetadatadx::EndpointArgValue::Int(i64::from(value)),
    );
}

pub(crate) fn insert_bool_arg(
    args: &mut thetadatadx::EndpointArgs,
    key: &str,
    value: i32,
) -> Result<(), String> {
    match value {
        0 => {
            args.insert(key.to_string(), thetadatadx::EndpointArgValue::Bool(false));
            Ok(())
        }
        1 => {
            args.insert(key.to_string(), thetadatadx::EndpointArgValue::Bool(true));
            Ok(())
        }
        other => Err(format!("{key} must be 0 (false) or 1 (true), got {other}")),
    }
}

pub(crate) fn insert_float_arg(args: &mut thetadatadx::EndpointArgs, key: &str, value: f64) {
    args.insert(key.to_string(), thetadatadx::EndpointArgValue::Float(value));
}

// ═══════════════════════════════════════════════════════════════════════
//  #[repr(C)] typed array types — zero-copy tick buffers for FFI
// ═══════════════════════════════════════════════════════════════════════

/// Generate a `#[repr(C)]` array wrapper for a tick type, plus a free function.
///
/// Each generated type has:
/// - `data`: pointer to the first element (null if empty)
/// - `len`: number of elements
/// - `from_vec()`: consumes a `Vec<T>` and returns the array
/// - `free()`: deallocates the backing memory
macro_rules! tick_array_type {
    ($name:ident, $tick:ty) => {
        /// Heap-allocated array of ticks returned from FFI.
        /// Caller MUST free with the corresponding `tdx_*_array_free` function.
        #[repr(C)]
        pub struct $name {
            pub data: *const $tick,
            pub len: usize,
        }

        impl $name {
            /// Infallible for tick types (no `CString` allocation). Returns
            /// `Result` to match the signature of fallible sibling arrays
            /// (`TdxStringArray`, `TdxOptionContractArray`) so the shared
            /// FFI endpoint macros stay generic over `$array_type`.
            pub(crate) fn from_vec(v: Vec<$tick>) -> Result<Self, std::ffi::NulError> {
                let len = v.len();
                if len == 0 {
                    return Ok(Self {
                        data: ptr::null(),
                        len: 0,
                    });
                }
                let boxed = v.into_boxed_slice();
                let data = Box::into_raw(boxed) as *const $tick;
                Ok(Self { data, len })
            }

            unsafe fn free(self) {
                if !self.data.is_null() && self.len > 0 {
                    // SAFETY: `self.data` was returned by `Box::into_raw` on a `Box<[$tick]>` of length `self.len` in `from_vec`; ownership returns to Rust for drop. Null + zero-len gated by the surrounding `if`.
                    let _ = unsafe {
                        Box::from_raw(std::ptr::slice_from_raw_parts_mut(
                            self.data as *mut $tick,
                            self.len,
                        ))
                    };
                }
            }
        }
    };
}

tick_array_type!(TdxEodTickArray, tdbe::EodTick);
tick_array_type!(TdxOhlcTickArray, tdbe::OhlcTick);
tick_array_type!(TdxTradeTickArray, tdbe::TradeTick);
tick_array_type!(TdxQuoteTickArray, tdbe::QuoteTick);
// Per-order Greeks subsets emitted by `option_*_greeks_first_order` /
// `_second_order` / `_third_order`. The full union for the interval-sampled
// `option_*_greeks_all` endpoints lands on `TdxGreeksAllTickArray`; the
// end-of-day endpoint `option_history_greeks_eod` lands on
// `TdxGreeksEodTickArray` (carries 12 EOD trade/quote columns absent from
// the interval-sampled all-union shape).
tick_array_type!(TdxGreeksAllTickArray, tdbe::GreeksAllTick);
tick_array_type!(TdxGreeksEodTickArray, tdbe::GreeksEodTick);
tick_array_type!(TdxGreeksFirstOrderTickArray, tdbe::GreeksFirstOrderTick);
tick_array_type!(TdxGreeksSecondOrderTickArray, tdbe::GreeksSecondOrderTick);
tick_array_type!(TdxGreeksThirdOrderTickArray, tdbe::GreeksThirdOrderTick);
// Per-OPRA-trade Greeks emitted by `option_history_trade_greeks_*`. These
// carry the nine trade-side execution columns alongside the Greek values --
// distinct from the interval-sampled `TdxGreeks*TickArray` whose rows carry
// the bid/ask quote pair instead.
tick_array_type!(TdxTradeGreeksAllTickArray, tdbe::TradeGreeksAllTick);
tick_array_type!(
    TdxTradeGreeksFirstOrderTickArray,
    tdbe::TradeGreeksFirstOrderTick
);
tick_array_type!(
    TdxTradeGreeksSecondOrderTickArray,
    tdbe::TradeGreeksSecondOrderTick
);
tick_array_type!(
    TdxTradeGreeksThirdOrderTickArray,
    tdbe::TradeGreeksThirdOrderTick
);
tick_array_type!(
    TdxTradeGreeksImpliedVolatilityTickArray,
    tdbe::TradeGreeksImpliedVolatilityTick
);
tick_array_type!(TdxIvTickArray, tdbe::IvTick);
tick_array_type!(TdxPriceTickArray, tdbe::PriceTick);
// Trade-shaped row emitted by `index_at_time_price` (10 wire columns:
// `timestamp`, `sequence`, `ext_condition1..4`, `condition`, `size`,
// `exchange`, `price`). Distinct from the bare `TdxPriceTickArray`
// used by `index_snapshot_price` / `index_history_price` (3 columns).
tick_array_type!(TdxIndexPriceAtTimeTickArray, tdbe::IndexPriceAtTimeTick);
tick_array_type!(TdxOpenInterestTickArray, tdbe::OpenInterestTick);
tick_array_type!(TdxMarketValueTickArray, tdbe::MarketValueTick);
tick_array_type!(TdxCalendarDayArray, tdbe::CalendarDay);
tick_array_type!(TdxInterestRateTickArray, tdbe::InterestRateTick);
tick_array_type!(TdxTradeQuoteTickArray, tdbe::TradeQuoteTick);

/// Generate a `#[no_mangle] extern "C"` free function for a tick array type.
macro_rules! tick_array_free {
    ($fn_name:ident, $array_type:ident) => {
        /// Free a tick array returned by an FFI endpoint.
        #[no_mangle]
        pub unsafe extern "C" fn $fn_name(arr: $array_type) {
            ffi_boundary!((), {
                // SAFETY: `arr` is a `$array_type` returned by the matching FFI endpoint via `from_vec`; the enclosed `free()` matches the `Box::into_raw` that produced `arr.data`.
                unsafe { arr.free() };
            })
        }
    };
}

tick_array_free!(tdx_eod_tick_array_free, TdxEodTickArray);
tick_array_free!(tdx_ohlc_tick_array_free, TdxOhlcTickArray);
tick_array_free!(tdx_trade_tick_array_free, TdxTradeTickArray);
tick_array_free!(tdx_quote_tick_array_free, TdxQuoteTickArray);
tick_array_free!(tdx_greeks_all_tick_array_free, TdxGreeksAllTickArray);
tick_array_free!(tdx_greeks_eod_tick_array_free, TdxGreeksEodTickArray);
tick_array_free!(
    tdx_greeks_first_order_tick_array_free,
    TdxGreeksFirstOrderTickArray
);
tick_array_free!(
    tdx_greeks_second_order_tick_array_free,
    TdxGreeksSecondOrderTickArray
);
tick_array_free!(
    tdx_greeks_third_order_tick_array_free,
    TdxGreeksThirdOrderTickArray
);
tick_array_free!(
    tdx_trade_greeks_all_tick_array_free,
    TdxTradeGreeksAllTickArray
);
tick_array_free!(
    tdx_trade_greeks_first_order_tick_array_free,
    TdxTradeGreeksFirstOrderTickArray
);
tick_array_free!(
    tdx_trade_greeks_second_order_tick_array_free,
    TdxTradeGreeksSecondOrderTickArray
);
tick_array_free!(
    tdx_trade_greeks_third_order_tick_array_free,
    TdxTradeGreeksThirdOrderTickArray
);
tick_array_free!(
    tdx_trade_greeks_implied_volatility_tick_array_free,
    TdxTradeGreeksImpliedVolatilityTickArray
);
tick_array_free!(tdx_iv_tick_array_free, TdxIvTickArray);
tick_array_free!(tdx_price_tick_array_free, TdxPriceTickArray);
tick_array_free!(
    tdx_index_price_at_time_tick_array_free,
    TdxIndexPriceAtTimeTickArray
);
tick_array_free!(tdx_open_interest_tick_array_free, TdxOpenInterestTickArray);
tick_array_free!(tdx_market_value_tick_array_free, TdxMarketValueTickArray);
tick_array_free!(tdx_calendar_day_array_free, TdxCalendarDayArray);
tick_array_free!(tdx_interest_rate_tick_array_free, TdxInterestRateTickArray);
tick_array_free!(tdx_trade_quote_tick_array_free, TdxTradeQuoteTickArray);

// ═══════════════════════════════════════════════════════════════════════
//  Arrow IPC terminal for in-band history tick rows
// ═══════════════════════════════════════════════════════════════════════
//
// Mirrors the FlatFiles `tdx_flatfile_rows_to_arrow_ipc` terminal for the
// typed history rows: a C++ caller holding a `std::vector<EodTick>` (or any
// other tick vector) serialises it to an Arrow IPC stream and hands the
// bytes to arrow-cpp, the same columnar exit Python exposes via
// `<TickName>List.to_arrow()`. The tick structs are the layout-pinned
// `tdbe::*Tick` types the history endpoints already return, so the bytes go
// straight through `TicksArrowExt::to_arrow` with no re-marshaling.

/// Heap-owned byte buffer (Arrow IPC stream) returned by the per-tick
/// `tdx_*_to_arrow_ipc` terminals. Caller MUST free with
/// `tdx_arrow_bytes_free`. Distinct from `TdxFlatFileBytes` only in name —
/// the layout is identical so a future merge stays ABI-compatible.
#[repr(C)]
pub struct TdxArrowBytes {
    pub data: *const u8,
    pub len: usize,
}

impl TdxArrowBytes {
    const EMPTY: Self = Self {
        data: ptr::null(),
        len: 0,
    };

    fn from_vec(buf: Vec<u8>) -> Self {
        if buf.is_empty() {
            return Self::EMPTY;
        }
        let boxed = buf.into_boxed_slice();
        let len = boxed.len();
        let data = Box::into_raw(boxed) as *const u8;
        Self { data, len }
    }
}

/// Serialise a `&[$tick]` to Arrow IPC bytes through the shared
/// `TicksArrowExt::to_arrow` + IPC `StreamWriter` path. An empty input is a
/// valid zero-row stream (matching the FlatFiles terminal and Python).
/// Returns `(data=null, len=0)` on error with `tdx_last_error()` set.
macro_rules! tick_array_to_arrow_ipc {
    ($fn_name:ident, $tick:ty) => {
        /// Serialise a tick row span as an Arrow IPC stream. `rows` may be
        /// null only when `len` is 0. Caller MUST free the result with
        /// `tdx_arrow_bytes_free`.
        ///
        /// # Safety
        /// `rows` must point to `len` initialised `$tick` values (e.g. the
        /// `data` / `len` pair of the array a history endpoint returned, or
        /// a C++ `std::vector`'s `data()` / `size()`), valid for the call.
        #[no_mangle]
        pub unsafe extern "C" fn $fn_name(rows: *const $tick, len: usize) -> TdxArrowBytes {
            ffi_boundary!(TdxArrowBytes::EMPTY, {
                if rows.is_null() && len != 0 {
                    crate::error::set_error("rows pointer is null with non-zero len");
                    return TdxArrowBytes::EMPTY;
                }
                let slice: &[$tick] = if len == 0 {
                    &[]
                } else {
                    // SAFETY: caller's contract guarantees `rows` points to
                    // `len` initialised values for the call duration; the
                    // `len == 0` arm above never reaches here, so `rows` is the
                    // non-empty span the endpoint / vector handed us.
                    unsafe { std::slice::from_raw_parts(rows, len) }
                };
                let batch = match thetadatadx::frames::TicksArrowExt::to_arrow(slice) {
                    Ok(b) => b,
                    Err(e) => {
                        crate::error::set_error(&format!("arrow conversion failed: {e}"));
                        return TdxArrowBytes::EMPTY;
                    }
                };
                let mut buf: Vec<u8> = Vec::new();
                {
                    let mut writer = match arrow_ipc::writer::StreamWriter::try_new(
                        std::io::Cursor::new(&mut buf),
                        &batch.schema(),
                    ) {
                        Ok(w) => w,
                        Err(e) => {
                            crate::error::set_error(&format!("arrow ipc writer init failed: {e}"));
                            return TdxArrowBytes::EMPTY;
                        }
                    };
                    if let Err(e) = writer.write(&batch) {
                        crate::error::set_error(&format!("arrow ipc write failed: {e}"));
                        return TdxArrowBytes::EMPTY;
                    }
                    if let Err(e) = writer.finish() {
                        crate::error::set_error(&format!("arrow ipc finish failed: {e}"));
                        return TdxArrowBytes::EMPTY;
                    }
                }
                TdxArrowBytes::from_vec(buf)
            })
        }
    };
}

tick_array_to_arrow_ipc!(tdx_eod_ticks_to_arrow_ipc, tdbe::EodTick);
tick_array_to_arrow_ipc!(tdx_ohlc_ticks_to_arrow_ipc, tdbe::OhlcTick);
tick_array_to_arrow_ipc!(tdx_trade_ticks_to_arrow_ipc, tdbe::TradeTick);
tick_array_to_arrow_ipc!(tdx_quote_ticks_to_arrow_ipc, tdbe::QuoteTick);
tick_array_to_arrow_ipc!(tdx_greeks_all_ticks_to_arrow_ipc, tdbe::GreeksAllTick);
tick_array_to_arrow_ipc!(tdx_greeks_eod_ticks_to_arrow_ipc, tdbe::GreeksEodTick);
tick_array_to_arrow_ipc!(
    tdx_greeks_first_order_ticks_to_arrow_ipc,
    tdbe::GreeksFirstOrderTick
);
tick_array_to_arrow_ipc!(
    tdx_greeks_second_order_ticks_to_arrow_ipc,
    tdbe::GreeksSecondOrderTick
);
tick_array_to_arrow_ipc!(
    tdx_greeks_third_order_ticks_to_arrow_ipc,
    tdbe::GreeksThirdOrderTick
);
tick_array_to_arrow_ipc!(
    tdx_trade_greeks_all_ticks_to_arrow_ipc,
    tdbe::TradeGreeksAllTick
);
tick_array_to_arrow_ipc!(
    tdx_trade_greeks_first_order_ticks_to_arrow_ipc,
    tdbe::TradeGreeksFirstOrderTick
);
tick_array_to_arrow_ipc!(
    tdx_trade_greeks_second_order_ticks_to_arrow_ipc,
    tdbe::TradeGreeksSecondOrderTick
);
tick_array_to_arrow_ipc!(
    tdx_trade_greeks_third_order_ticks_to_arrow_ipc,
    tdbe::TradeGreeksThirdOrderTick
);
tick_array_to_arrow_ipc!(
    tdx_trade_greeks_implied_volatility_ticks_to_arrow_ipc,
    tdbe::TradeGreeksImpliedVolatilityTick
);
tick_array_to_arrow_ipc!(tdx_iv_ticks_to_arrow_ipc, tdbe::IvTick);
tick_array_to_arrow_ipc!(tdx_price_ticks_to_arrow_ipc, tdbe::PriceTick);
tick_array_to_arrow_ipc!(
    tdx_index_price_at_time_ticks_to_arrow_ipc,
    tdbe::IndexPriceAtTimeTick
);
tick_array_to_arrow_ipc!(tdx_open_interest_ticks_to_arrow_ipc, tdbe::OpenInterestTick);
tick_array_to_arrow_ipc!(tdx_market_value_ticks_to_arrow_ipc, tdbe::MarketValueTick);
tick_array_to_arrow_ipc!(tdx_calendar_days_to_arrow_ipc, tdbe::CalendarDay);
tick_array_to_arrow_ipc!(tdx_interest_rate_ticks_to_arrow_ipc, tdbe::InterestRateTick);
tick_array_to_arrow_ipc!(tdx_trade_quote_ticks_to_arrow_ipc, tdbe::TradeQuoteTick);

/// Free a byte buffer returned by any `tdx_*_to_arrow_ipc` terminal.
#[no_mangle]
pub unsafe extern "C" fn tdx_arrow_bytes_free(bytes: TdxArrowBytes) {
    ffi_boundary!((), {
        if !bytes.data.is_null() && bytes.len > 0 {
            // SAFETY: `bytes.data` was produced by `Box::into_raw` on a
            // `Box<[u8]>` of length `bytes.len` in `TdxArrowBytes::from_vec`;
            // ownership returns to Rust here for drop. Null + zero-len gated.
            let _ = unsafe {
                Box::from_raw(std::ptr::slice_from_raw_parts_mut(
                    bytes.data as *mut u8,
                    bytes.len,
                ))
            };
        }
    })
}

// ═══════════════════════════════════════════════════════════════════════
//  OptionContract FFI type (String field requires special handling)
// ═══════════════════════════════════════════════════════════════════════

/// FFI-safe option contract descriptor.
///
/// The `symbol` field is a heap-allocated C string. Freed when the array
/// is freed.
#[repr(C)]
pub struct TdxOptionContract {
    /// Heap-allocated NUL-terminated C string. Freed with the array.
    pub symbol: *const c_char,
    /// Expiration date as a `YYYYMMDD` integer.
    pub expiration: i32,
    /// Strike price in dollars.
    pub strike: f64,
    /// Contract right as the Unicode scalar value of the character:
    /// `'C'` (67) for a call, `'P'` (80) for a put. Cast to `char` for
    /// display. Same 4-byte slot the previous ASCII integer occupied.
    pub right: u32,
}

/// Array of FFI-safe option contracts.
#[repr(C)]
pub struct TdxOptionContractArray {
    pub data: *const TdxOptionContract,
    pub len: usize,
}

impl TdxOptionContractArray {
    pub(crate) fn from_vec(
        contracts: Vec<tdbe::OptionContract>,
    ) -> Result<Self, std::ffi::NulError> {
        let len = contracts.len();
        if len == 0 {
            return Ok(Self {
                data: ptr::null(),
                len: 0,
            });
        }
        let ffi_contracts = contracts
            .into_iter()
            .map(|c| {
                Ok(TdxOptionContract {
                    symbol: CString::new(c.symbol)?.into_raw().cast_const(),
                    expiration: c.expiration,
                    strike: c.strike,
                    right: c.right as u32,
                })
            })
            .collect::<Result<Vec<_>, std::ffi::NulError>>()?;
        let boxed = ffi_contracts.into_boxed_slice();
        let data = Box::into_raw(boxed) as *const TdxOptionContract;
        Ok(Self { data, len })
    }
}

/// Free an option contract array, including all heap-allocated symbol
/// strings.
#[no_mangle]
pub unsafe extern "C" fn tdx_option_contract_array_free(arr: TdxOptionContractArray) {
    ffi_boundary!((), {
        if !arr.data.is_null() && arr.len > 0 {
            // First free each symbol C string
            // SAFETY: data + len describe a contiguous slice the caller is required to keep valid for the call duration.
            let slice = unsafe { std::slice::from_raw_parts(arr.data, arr.len) };
            for contract in slice {
                if !contract.symbol.is_null() {
                    // SAFETY: the pointer was produced by CString::into_raw on the matching free path, ownership returns to Rust here.
                    drop(unsafe { CString::from_raw(contract.symbol.cast_mut()) });
                }
            }
            // Then free the array itself
            // SAFETY: `arr.data` was returned by `Box::into_raw` on a `Box<[TdxOptionContract]>` of length `arr.len`; ownership returns to Rust for drop. Null + zero-len gated above; per-element symbol strings were freed in the loop above.
            let _ = unsafe {
                Box::from_raw(std::ptr::slice_from_raw_parts_mut(
                    arr.data.cast_mut(),
                    arr.len,
                ))
            };
        }
    })
}

// ═══════════════════════════════════════════════════════════════════════
//  TdxStringArray — for list endpoints returning Vec<String>
// ═══════════════════════════════════════════════════════════════════════

/// Array of heap-allocated C strings.
#[repr(C)]
pub struct TdxStringArray {
    /// Array of pointers to NUL-terminated C strings.
    pub data: *const *const c_char,
    pub len: usize,
}

impl TdxStringArray {
    pub(crate) fn from_vec(strings: Vec<String>) -> Result<Self, std::ffi::NulError> {
        let len = strings.len();
        if len == 0 {
            return Ok(Self {
                data: ptr::null(),
                len: 0,
            });
        }
        let cstrings = strings
            .into_iter()
            .map(|s| CString::new(s).map(|c| c.into_raw().cast_const()))
            .collect::<Result<Vec<*const c_char>, std::ffi::NulError>>()?;
        let boxed = cstrings.into_boxed_slice();
        let data = Box::into_raw(boxed) as *const *const c_char;
        Ok(Self { data, len })
    }
}

/// Free a string array, including all individual C strings.
#[no_mangle]
pub unsafe extern "C" fn tdx_string_array_free(arr: TdxStringArray) {
    ffi_boundary!((), {
        if !arr.data.is_null() && arr.len > 0 {
            // SAFETY: data + len describe a contiguous slice the caller is required to keep valid for the call duration.
            let slice = unsafe { std::slice::from_raw_parts(arr.data, arr.len) };
            for &s in slice {
                if !s.is_null() {
                    // SAFETY: the pointer was produced by CString::into_raw on the matching free path, ownership returns to Rust here.
                    drop(unsafe { CString::from_raw(s.cast_mut()) });
                }
            }
            // SAFETY: `arr.data` was returned by `Box::into_raw` on a `Box<[*const c_char]>` of length `arr.len`; ownership returns to Rust for drop. Null + zero-len gated above; per-element C strings were freed in the loop above.
            let _ = unsafe {
                Box::from_raw(std::ptr::slice_from_raw_parts_mut(
                    arr.data.cast_mut(),
                    arr.len,
                ))
            };
        }
    })
}

/// Parse a C array of C string pointers into `Vec<String>`.
///
/// When `symbols` is null and `symbols_len` is 0 (Go empty-slice convention),
/// returns `Some(vec![])`. Returns `None` and sets the thread-local error if
/// the pointer is null with a non-zero length, or any element is null / invalid
/// UTF-8.
pub(crate) unsafe fn parse_symbol_array(
    symbols: *const *const c_char,
    symbols_len: usize,
) -> Option<Vec<String>> {
    if symbols.is_null() {
        if symbols_len == 0 {
            // Go sends (nil, 0) for empty slices — that's valid.
            return Some(vec![]);
        }
        set_error("symbols array pointer is null");
        return None;
    }
    // SAFETY: data + len describe a contiguous slice the caller is required to keep valid for the call duration.
    let ptrs = unsafe { std::slice::from_raw_parts(symbols, symbols_len) };
    let mut out = Vec::with_capacity(symbols_len);
    for (i, &p) in ptrs.iter().enumerate() {
        // SAFETY: caller supplies a NUL-terminated C string allocated by the host runtime; cstr_to_str validates non-null + UTF-8.
        match unsafe { cstr_to_str(p) } {
            Ok(Some(s)) => out.push(s.to_owned()),
            Ok(None) => {
                set_error(&format!("symbols[{i}] is null"));
                return None;
            }
            Err(e) => {
                set_error(&format!("symbols[{i}] is not valid UTF-8: {e}"));
                return None;
            }
        }
    }
    Some(out)
}
