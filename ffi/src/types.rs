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
tick_array_type!(TdxGreeksTickArray, tdbe::GreeksTick);
tick_array_type!(TdxIvTickArray, tdbe::IvTick);
tick_array_type!(TdxPriceTickArray, tdbe::PriceTick);
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
                unsafe { arr.free() };
            })
        }
    };
}

tick_array_free!(tdx_eod_tick_array_free, TdxEodTickArray);
tick_array_free!(tdx_ohlc_tick_array_free, TdxOhlcTickArray);
tick_array_free!(tdx_trade_tick_array_free, TdxTradeTickArray);
tick_array_free!(tdx_quote_tick_array_free, TdxQuoteTickArray);
tick_array_free!(tdx_greeks_tick_array_free, TdxGreeksTickArray);
tick_array_free!(tdx_iv_tick_array_free, TdxIvTickArray);
tick_array_free!(tdx_price_tick_array_free, TdxPriceTickArray);
tick_array_free!(tdx_open_interest_tick_array_free, TdxOpenInterestTickArray);
tick_array_free!(tdx_market_value_tick_array_free, TdxMarketValueTickArray);
tick_array_free!(tdx_calendar_day_array_free, TdxCalendarDayArray);
tick_array_free!(tdx_interest_rate_tick_array_free, TdxInterestRateTickArray);
tick_array_free!(tdx_trade_quote_tick_array_free, TdxTradeQuoteTickArray);

// ═══════════════════════════════════════════════════════════════════════
//  OptionContract FFI type (String field requires special handling)
// ═══════════════════════════════════════════════════════════════════════

/// FFI-safe option contract descriptor.
///
/// The `root` field is a heap-allocated C string. Freed when the array is freed.
#[repr(C)]
pub struct TdxOptionContract {
    /// Heap-allocated NUL-terminated C string. Freed with the array.
    pub root: *const c_char,
    pub expiration: i32,
    pub strike: f64,
    pub right: i32,
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
                    root: CString::new(c.root)?.into_raw().cast_const(),
                    expiration: c.expiration,
                    strike: c.strike,
                    right: c.right,
                })
            })
            .collect::<Result<Vec<_>, std::ffi::NulError>>()?;
        let boxed = ffi_contracts.into_boxed_slice();
        let data = Box::into_raw(boxed) as *const TdxOptionContract;
        Ok(Self { data, len })
    }
}

/// Free an option contract array, including all heap-allocated root strings.
#[no_mangle]
pub unsafe extern "C" fn tdx_option_contract_array_free(arr: TdxOptionContractArray) {
    ffi_boundary!((), {
        if !arr.data.is_null() && arr.len > 0 {
            // First free each root C string
            let slice = unsafe { std::slice::from_raw_parts(arr.data, arr.len) };
            for contract in slice {
                if !contract.root.is_null() {
                    drop(unsafe { CString::from_raw(contract.root.cast_mut()) });
                }
            }
            // Then free the array itself
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
            let slice = unsafe { std::slice::from_raw_parts(arr.data, arr.len) };
            for &s in slice {
                if !s.is_null() {
                    drop(unsafe { CString::from_raw(s.cast_mut()) });
                }
            }
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
    let ptrs = unsafe { std::slice::from_raw_parts(symbols, symbols_len) };
    let mut out = Vec::with_capacity(symbols_len);
    for (i, &p) in ptrs.iter().enumerate() {
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
