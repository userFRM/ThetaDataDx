//! `#[repr(C)]` handle types, tick-array wrappers, `ThetaDataDxStringArray`,
//! `ThetaDataDxOptionContract*`, plus the shared string / slice helpers that cross
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
pub struct ThetaDataDxCredentials {
    pub(crate) inner: thetadatadx::Credentials,
}

/// Opaque historical (MDDS) client handle.
///
/// `repr(transparent)` guarantees `*const ThetaDataDxHistoricalClient` and
/// `*const HistoricalClient` have identical layout, allowing safe pointer casts in
/// `thetadatadx_client_historical()`.
#[repr(transparent)]
pub struct ThetaDataDxHistoricalClient {
    pub(crate) inner: thetadatadx::mdds::HistoricalClient,
}

/// Opaque config handle.
pub struct ThetaDataDxConfig {
    pub(crate) inner: thetadatadx::DirectConfig,
}

// ── C-string lifetime helpers (shared across all endpoint modules) ──

/// Free a string returned by any `thetadatadx_*` function.
///
/// MUST be called for every non-null `*mut c_char` returned by this library.
#[no_mangle]
pub unsafe extern "C" fn thetadatadx_string_free(s: *mut c_char) {
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
        /// Caller MUST free with the corresponding `thetadatadx_*_array_free` function.
        #[repr(C)]
        pub struct $name {
            /// Pointer to the first element; null when empty.
            pub data: *const $tick,
            /// Number of elements in the array.
            pub len: usize,
        }

        impl $name {
            /// Infallible for tick types (no `CString` allocation). Returns
            /// `Result` to match the signature of fallible sibling arrays
            /// (`ThetaDataDxStringArray`, `ThetaDataDxOptionContractArray`) so the shared
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

tick_array_type!(ThetaDataDxEodTickArray, thetadatadx::EodTick);
tick_array_type!(ThetaDataDxOhlcTickArray, thetadatadx::OhlcTick);
tick_array_type!(ThetaDataDxTradeTickArray, thetadatadx::TradeTick);
tick_array_type!(ThetaDataDxQuoteTickArray, thetadatadx::QuoteTick);
// Per-order Greeks subsets emitted by `option_*_greeks_first_order` /
// `_second_order` / `_third_order`. The full union for the interval-sampled
// `option_*_greeks_all` endpoints lands on `ThetaDataDxGreeksAllTickArray`; the
// end-of-day endpoint `option_history_greeks_eod` lands on
// `ThetaDataDxGreeksEodTickArray` (carries 12 EOD trade/quote columns absent from
// the interval-sampled all-union shape).
tick_array_type!(ThetaDataDxGreeksAllTickArray, thetadatadx::GreeksAllTick);
tick_array_type!(ThetaDataDxGreeksEodTickArray, thetadatadx::GreeksEodTick);
tick_array_type!(
    ThetaDataDxGreeksFirstOrderTickArray,
    thetadatadx::GreeksFirstOrderTick
);
tick_array_type!(
    ThetaDataDxGreeksSecondOrderTickArray,
    thetadatadx::GreeksSecondOrderTick
);
tick_array_type!(
    ThetaDataDxGreeksThirdOrderTickArray,
    thetadatadx::GreeksThirdOrderTick
);
// Per-OPRA-trade Greeks emitted by `option_history_trade_greeks_*`. These
// carry the nine trade-side execution columns alongside the Greek values --
// distinct from the interval-sampled `ThetaDataDxGreeks*TickArray` whose rows carry
// the bid/ask quote pair instead.
tick_array_type!(
    ThetaDataDxTradeGreeksAllTickArray,
    thetadatadx::TradeGreeksAllTick
);
tick_array_type!(
    ThetaDataDxTradeGreeksFirstOrderTickArray,
    thetadatadx::TradeGreeksFirstOrderTick
);
tick_array_type!(
    ThetaDataDxTradeGreeksSecondOrderTickArray,
    thetadatadx::TradeGreeksSecondOrderTick
);
tick_array_type!(
    ThetaDataDxTradeGreeksThirdOrderTickArray,
    thetadatadx::TradeGreeksThirdOrderTick
);
tick_array_type!(
    ThetaDataDxTradeGreeksImpliedVolatilityTickArray,
    thetadatadx::TradeGreeksImpliedVolatilityTick
);
tick_array_type!(ThetaDataDxIvTickArray, thetadatadx::IvTick);
tick_array_type!(ThetaDataDxPriceTickArray, thetadatadx::PriceTick);
// Trade-shaped row emitted by `index_at_time_price` (10 wire columns:
// `timestamp`, `sequence`, `ext_condition1..4`, `condition`, `size`,
// `exchange`, `price`). Distinct from the bare `ThetaDataDxPriceTickArray`
// used by `index_snapshot_price` / `index_history_price` (3 columns).
tick_array_type!(
    ThetaDataDxIndexPriceAtTimeTickArray,
    thetadatadx::IndexPriceAtTimeTick
);
tick_array_type!(
    ThetaDataDxOpenInterestTickArray,
    thetadatadx::OpenInterestTick
);
tick_array_type!(
    ThetaDataDxMarketValueTickArray,
    thetadatadx::MarketValueTick
);
tick_array_type!(ThetaDataDxCalendarDayArray, thetadatadx::CalendarDay);
tick_array_type!(
    ThetaDataDxInterestRateTickArray,
    thetadatadx::InterestRateTick
);
tick_array_type!(ThetaDataDxTradeQuoteTickArray, thetadatadx::TradeQuoteTick);

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

tick_array_free!(thetadatadx_eod_tick_array_free, ThetaDataDxEodTickArray);
tick_array_free!(thetadatadx_ohlc_tick_array_free, ThetaDataDxOhlcTickArray);
tick_array_free!(thetadatadx_trade_tick_array_free, ThetaDataDxTradeTickArray);
tick_array_free!(thetadatadx_quote_tick_array_free, ThetaDataDxQuoteTickArray);
tick_array_free!(
    thetadatadx_greeks_all_tick_array_free,
    ThetaDataDxGreeksAllTickArray
);
tick_array_free!(
    thetadatadx_greeks_eod_tick_array_free,
    ThetaDataDxGreeksEodTickArray
);
tick_array_free!(
    thetadatadx_greeks_first_order_tick_array_free,
    ThetaDataDxGreeksFirstOrderTickArray
);
tick_array_free!(
    thetadatadx_greeks_second_order_tick_array_free,
    ThetaDataDxGreeksSecondOrderTickArray
);
tick_array_free!(
    thetadatadx_greeks_third_order_tick_array_free,
    ThetaDataDxGreeksThirdOrderTickArray
);
tick_array_free!(
    thetadatadx_trade_greeks_all_tick_array_free,
    ThetaDataDxTradeGreeksAllTickArray
);
tick_array_free!(
    thetadatadx_trade_greeks_first_order_tick_array_free,
    ThetaDataDxTradeGreeksFirstOrderTickArray
);
tick_array_free!(
    thetadatadx_trade_greeks_second_order_tick_array_free,
    ThetaDataDxTradeGreeksSecondOrderTickArray
);
tick_array_free!(
    thetadatadx_trade_greeks_third_order_tick_array_free,
    ThetaDataDxTradeGreeksThirdOrderTickArray
);
tick_array_free!(
    thetadatadx_trade_greeks_implied_volatility_tick_array_free,
    ThetaDataDxTradeGreeksImpliedVolatilityTickArray
);
tick_array_free!(thetadatadx_iv_tick_array_free, ThetaDataDxIvTickArray);
tick_array_free!(thetadatadx_price_tick_array_free, ThetaDataDxPriceTickArray);
tick_array_free!(
    thetadatadx_index_price_at_time_tick_array_free,
    ThetaDataDxIndexPriceAtTimeTickArray
);
tick_array_free!(
    thetadatadx_open_interest_tick_array_free,
    ThetaDataDxOpenInterestTickArray
);
tick_array_free!(
    thetadatadx_market_value_tick_array_free,
    ThetaDataDxMarketValueTickArray
);
tick_array_free!(
    thetadatadx_calendar_day_array_free,
    ThetaDataDxCalendarDayArray
);
tick_array_free!(
    thetadatadx_interest_rate_tick_array_free,
    ThetaDataDxInterestRateTickArray
);
tick_array_free!(
    thetadatadx_trade_quote_tick_array_free,
    ThetaDataDxTradeQuoteTickArray
);

// ═══════════════════════════════════════════════════════════════════════
//  Arrow IPC terminal for in-band history tick rows
// ═══════════════════════════════════════════════════════════════════════
//
// Mirrors the FlatFiles `thetadatadx_flatfile_rows_to_arrow_ipc` terminal for the
// typed history rows: a C++ caller holding a `std::vector<EodTick>` (or any
// other tick vector) serialises it to an Arrow IPC stream and hands the
// bytes to arrow-cpp, the same columnar exit Python exposes via
// `<TickName>List.to_arrow()`. The tick structs are the layout-pinned
// `thetadatadx::*Tick` types the history endpoints already return, so the bytes go
// straight through `TicksArrowExt::to_arrow` with no re-marshaling.

/// Heap-owned byte buffer (Arrow IPC stream) returned by the per-tick
/// `thetadatadx_*_to_arrow_ipc` terminals. Caller MUST free with
/// `thetadatadx_arrow_bytes_free`. Distinct from `ThetaDataDxFlatFileBytes` only in name —
/// the layout is identical so a future merge stays ABI-compatible.
#[repr(C)]
pub struct ThetaDataDxArrowBytes {
    /// Pointer to the first byte of the IPC stream; null when empty.
    pub data: *const u8,
    /// Length of the buffer in bytes.
    pub len: usize,
}

// Layout drift-guard: pin the LP64 `#[repr(C)]` size + alignment on the
// Rust side, the same values the C++ `abi_struct_layout_asserts.hpp.inc`
// pins. A field-width or member-order change that shifts the layout fails
// the build here, before the C header and its C++ asserts can drift; the
// C++ static_asserts alone cannot catch a Rust-side `#[repr(C)]` change.
const _: () = {
    assert!(core::mem::size_of::<ThetaDataDxArrowBytes>() == 16);
    assert!(core::mem::align_of::<ThetaDataDxArrowBytes>() == 8);
};

impl ThetaDataDxArrowBytes {
    const EMPTY: Self = Self {
        data: ptr::null(),
        len: 0,
    };

    /// An empty (`data = null, len = 0`) buffer. Shared with the streaming
    /// `RecordBatch` reader's C ABI so its out-param can be initialised to a
    /// well-formed empty value before each pull.
    pub(crate) const fn empty() -> Self {
        Self::EMPTY
    }

    pub(crate) fn from_vec(buf: Vec<u8>) -> Self {
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
/// Returns `(data=null, len=0)` on error with `thetadatadx_last_error()` set.
macro_rules! tick_array_to_arrow_ipc {
    ($fn_name:ident, $tick:ty) => {
        /// Serialise a tick row span as an Arrow IPC stream. `rows` may be
        /// null only when `len` is 0. Caller MUST free the result with
        /// `thetadatadx_arrow_bytes_free`.
        ///
        /// # Safety
        /// `rows` must point to `len` initialised `$tick` values (e.g. the
        /// `data` / `len` pair of the array a history endpoint returned, or
        /// a C++ `std::vector`'s `data()` / `size()`), valid for the call.
        #[no_mangle]
        pub unsafe extern "C" fn $fn_name(rows: *const $tick, len: usize) -> ThetaDataDxArrowBytes {
            ffi_boundary!(ThetaDataDxArrowBytes::EMPTY, {
                if rows.is_null() && len != 0 {
                    crate::error::set_error("rows pointer is null with non-zero len");
                    return ThetaDataDxArrowBytes::EMPTY;
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
                        return ThetaDataDxArrowBytes::EMPTY;
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
                            return ThetaDataDxArrowBytes::EMPTY;
                        }
                    };
                    if let Err(e) = writer.write(&batch) {
                        crate::error::set_error(&format!("arrow ipc write failed: {e}"));
                        return ThetaDataDxArrowBytes::EMPTY;
                    }
                    if let Err(e) = writer.finish() {
                        crate::error::set_error(&format!("arrow ipc finish failed: {e}"));
                        return ThetaDataDxArrowBytes::EMPTY;
                    }
                }
                ThetaDataDxArrowBytes::from_vec(buf)
            })
        }
    };
}

tick_array_to_arrow_ipc!(thetadatadx_eod_ticks_to_arrow_ipc, thetadatadx::EodTick);
tick_array_to_arrow_ipc!(thetadatadx_ohlc_ticks_to_arrow_ipc, thetadatadx::OhlcTick);
tick_array_to_arrow_ipc!(thetadatadx_trade_ticks_to_arrow_ipc, thetadatadx::TradeTick);
tick_array_to_arrow_ipc!(thetadatadx_quote_ticks_to_arrow_ipc, thetadatadx::QuoteTick);
tick_array_to_arrow_ipc!(
    thetadatadx_greeks_all_ticks_to_arrow_ipc,
    thetadatadx::GreeksAllTick
);
tick_array_to_arrow_ipc!(
    thetadatadx_greeks_eod_ticks_to_arrow_ipc,
    thetadatadx::GreeksEodTick
);
tick_array_to_arrow_ipc!(
    thetadatadx_greeks_first_order_ticks_to_arrow_ipc,
    thetadatadx::GreeksFirstOrderTick
);
tick_array_to_arrow_ipc!(
    thetadatadx_greeks_second_order_ticks_to_arrow_ipc,
    thetadatadx::GreeksSecondOrderTick
);
tick_array_to_arrow_ipc!(
    thetadatadx_greeks_third_order_ticks_to_arrow_ipc,
    thetadatadx::GreeksThirdOrderTick
);
tick_array_to_arrow_ipc!(
    thetadatadx_trade_greeks_all_ticks_to_arrow_ipc,
    thetadatadx::TradeGreeksAllTick
);
tick_array_to_arrow_ipc!(
    thetadatadx_trade_greeks_first_order_ticks_to_arrow_ipc,
    thetadatadx::TradeGreeksFirstOrderTick
);
tick_array_to_arrow_ipc!(
    thetadatadx_trade_greeks_second_order_ticks_to_arrow_ipc,
    thetadatadx::TradeGreeksSecondOrderTick
);
tick_array_to_arrow_ipc!(
    thetadatadx_trade_greeks_third_order_ticks_to_arrow_ipc,
    thetadatadx::TradeGreeksThirdOrderTick
);
tick_array_to_arrow_ipc!(
    thetadatadx_trade_greeks_implied_volatility_ticks_to_arrow_ipc,
    thetadatadx::TradeGreeksImpliedVolatilityTick
);
tick_array_to_arrow_ipc!(thetadatadx_iv_ticks_to_arrow_ipc, thetadatadx::IvTick);
tick_array_to_arrow_ipc!(thetadatadx_price_ticks_to_arrow_ipc, thetadatadx::PriceTick);
tick_array_to_arrow_ipc!(
    thetadatadx_index_price_at_time_ticks_to_arrow_ipc,
    thetadatadx::IndexPriceAtTimeTick
);
tick_array_to_arrow_ipc!(
    thetadatadx_open_interest_ticks_to_arrow_ipc,
    thetadatadx::OpenInterestTick
);
tick_array_to_arrow_ipc!(
    thetadatadx_market_value_ticks_to_arrow_ipc,
    thetadatadx::MarketValueTick
);
tick_array_to_arrow_ipc!(
    thetadatadx_calendar_days_to_arrow_ipc,
    thetadatadx::CalendarDay
);
tick_array_to_arrow_ipc!(
    thetadatadx_interest_rate_ticks_to_arrow_ipc,
    thetadatadx::InterestRateTick
);
tick_array_to_arrow_ipc!(
    thetadatadx_trade_quote_ticks_to_arrow_ipc,
    thetadatadx::TradeQuoteTick
);

/// Free a byte buffer returned by any `thetadatadx_*_to_arrow_ipc` terminal.
#[no_mangle]
pub unsafe extern "C" fn thetadatadx_arrow_bytes_free(bytes: ThetaDataDxArrowBytes) {
    ffi_boundary!((), {
        if !bytes.data.is_null() && bytes.len > 0 {
            // SAFETY: `bytes.data` was produced by `Box::into_raw` on a
            // `Box<[u8]>` of length `bytes.len` in `ThetaDataDxArrowBytes::from_vec`;
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
pub struct ThetaDataDxOptionContract {
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

// Layout drift-guard: pin the LP64 `#[repr(C)]` size + alignment on the
// Rust side, matching `abi_struct_layout_asserts.hpp.inc`. `symbol` (ptr)
// @0, `expiration` (i32) @8 + 4-byte pad, `strike` (f64) @16, `right`
// (u32) @24 + 4-byte tail pad -> 32 bytes, align 8.
const _: () = {
    assert!(core::mem::size_of::<ThetaDataDxOptionContract>() == 32);
    assert!(core::mem::align_of::<ThetaDataDxOptionContract>() == 8);
};

/// Array of FFI-safe option contracts.
#[repr(C)]
pub struct ThetaDataDxOptionContractArray {
    /// Pointer to the first element; null when empty.
    pub data: *const ThetaDataDxOptionContract,
    /// Number of elements in the array.
    pub len: usize,
}

// Layout drift-guard: pin the LP64 `#[repr(C)]` size + alignment on the
// Rust side, matching `abi_struct_layout_asserts.hpp.inc`.
const _: () = {
    assert!(core::mem::size_of::<ThetaDataDxOptionContractArray>() == 16);
    assert!(core::mem::align_of::<ThetaDataDxOptionContractArray>() == 8);
};

impl ThetaDataDxOptionContractArray {
    pub(crate) fn from_vec(
        contracts: Vec<thetadatadx::OptionContract>,
    ) -> Result<Self, std::ffi::NulError> {
        let len = contracts.len();
        if len == 0 {
            return Ok(Self {
                data: ptr::null(),
                len: 0,
            });
        }
        // Pass 1: validate every symbol into an owned `CString` BEFORE any
        // `into_raw()`. If a later symbol carries an interior NUL this returns
        // `Err` and the already-built `CString`s drop and free normally, so no
        // raw pointer is ever orphaned across the FFI boundary.
        let owned = contracts
            .into_iter()
            .map(|c| CString::new(c.symbol).map(|symbol| (symbol, c.expiration, c.strike, c.right)))
            .collect::<Result<Vec<_>, std::ffi::NulError>>()?;
        // Pass 2: the whole batch validated, so handing each symbol to C now
        // cannot leave a partially-converted vector behind.
        let ffi_contracts = owned
            .into_iter()
            .map(
                |(symbol, expiration, strike, right)| ThetaDataDxOptionContract {
                    symbol: symbol.into_raw().cast_const(),
                    expiration,
                    strike,
                    right: right as u32,
                },
            )
            .collect::<Vec<_>>();
        let boxed = ffi_contracts.into_boxed_slice();
        let data = Box::into_raw(boxed) as *const ThetaDataDxOptionContract;
        Ok(Self { data, len })
    }
}

/// Free an option contract array, including all heap-allocated symbol
/// strings.
#[no_mangle]
pub unsafe extern "C" fn thetadatadx_option_contract_array_free(
    arr: ThetaDataDxOptionContractArray,
) {
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
            // SAFETY: `arr.data` was returned by `Box::into_raw` on a `Box<[ThetaDataDxOptionContract]>` of length `arr.len`; ownership returns to Rust for drop. Null + zero-len gated above; per-element symbol strings were freed in the loop above.
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
//  ThetaDataDxStringArray — for list endpoints returning Vec<String>
// ═══════════════════════════════════════════════════════════════════════

/// Array of heap-allocated C strings.
#[repr(C)]
pub struct ThetaDataDxStringArray {
    /// Array of pointers to NUL-terminated C strings.
    pub data: *const *const c_char,
    /// Number of strings in the array.
    pub len: usize,
}

// Layout drift-guard: pin the LP64 `#[repr(C)]` size + alignment on the
// Rust side, matching `abi_struct_layout_asserts.hpp.inc`.
const _: () = {
    assert!(core::mem::size_of::<ThetaDataDxStringArray>() == 16);
    assert!(core::mem::align_of::<ThetaDataDxStringArray>() == 8);
};

impl ThetaDataDxStringArray {
    pub(crate) fn from_vec(strings: Vec<String>) -> Result<Self, std::ffi::NulError> {
        let len = strings.len();
        if len == 0 {
            return Ok(Self {
                data: ptr::null(),
                len: 0,
            });
        }
        // Pass 1: validate every string into an owned `CString` BEFORE any
        // `into_raw()`. A later interior-NUL error then drops the owned
        // `CString`s normally instead of orphaning the raw pointers already
        // produced for earlier elements.
        let owned = strings
            .into_iter()
            .map(CString::new)
            .collect::<Result<Vec<_>, std::ffi::NulError>>()?;
        // Pass 2: hand the validated batch to C in one shot.
        let cstrings = owned
            .into_iter()
            .map(|c| c.into_raw().cast_const())
            .collect::<Vec<*const c_char>>();
        let boxed = cstrings.into_boxed_slice();
        let data = Box::into_raw(boxed) as *const *const c_char;
        Ok(Self { data, len })
    }
}

/// Free a string array, including all individual C strings.
#[no_mangle]
pub unsafe extern "C" fn thetadatadx_string_array_free(arr: ThetaDataDxStringArray) {
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
/// When `symbols` is null and `symbols_len` is 0 (empty-slice convention),
/// returns `Some(vec![])`. Returns `None` and sets the thread-local error if
/// the pointer is null with a non-zero length, or any element is null / invalid
/// UTF-8.
pub(crate) unsafe fn parse_symbol_array(
    symbols: *const *const c_char,
    symbols_len: usize,
) -> Option<Vec<String>> {
    if symbols.is_null() {
        if symbols_len == 0 {
            // A (null, 0) pair denotes an empty slice — that's valid.
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

#[cfg(test)]
mod array_construction_tests {
    use super::{
        thetadatadx_option_contract_array_free, thetadatadx_string_array_free,
        ThetaDataDxOptionContractArray, ThetaDataDxStringArray,
    };
    use thetadatadx::OptionContract;

    fn contract(symbol: &str) -> OptionContract {
        OptionContract {
            symbol: symbol.to_string(),
            expiration: 20240119,
            strike: 100.0,
            right: 'C',
        }
    }

    /// An interior NUL in a LATER element must fail the whole construction
    /// without orphaning the raw pointer already produced for an earlier,
    /// valid element. The two-pass build validates every symbol into an owned
    /// `CString` before any `into_raw()`, so this returns `Err` with nothing
    /// leaked. (The previous single-pass `collect` called `into_raw()` on the
    /// first element before hitting the second's error, orphaning it.)
    #[test]
    fn option_contract_array_rejects_interior_nul_in_second_element() {
        let input = vec![contract("AAPL"), contract("MS\0FT")];
        let result = ThetaDataDxOptionContractArray::from_vec(input);
        assert!(
            result.is_err(),
            "interior NUL in the second symbol must fail construction"
        );
    }

    /// The valid path round-trips through the matching free function, proving
    /// the two-pass conversion produces exactly the ownership the free path
    /// expects (no leak, no double-free).
    #[test]
    fn option_contract_array_success_round_trips_through_free() {
        let arr =
            ThetaDataDxOptionContractArray::from_vec(vec![contract("AAPL"), contract("MSFT")])
                .expect("all-valid symbols build");
        assert_eq!(arr.len, 2);
        assert!(!arr.data.is_null());
        // SAFETY: `arr` was produced by `from_vec` above; the free matches the
        // `Box::into_raw` + per-symbol `CString::into_raw` that built it.
        unsafe { thetadatadx_option_contract_array_free(arr) };
    }

    /// Same interior-NUL leak guard for the list-endpoint string array.
    #[test]
    fn string_array_rejects_interior_nul_in_second_element() {
        let input = vec!["AAPL".to_string(), "MS\0FT".to_string()];
        let result = ThetaDataDxStringArray::from_vec(input);
        assert!(
            result.is_err(),
            "interior NUL in the second string must fail construction"
        );
    }

    #[test]
    fn string_array_success_round_trips_through_free() {
        let arr = ThetaDataDxStringArray::from_vec(vec!["AAPL".to_string(), "MSFT".to_string()])
            .expect("all-valid strings build");
        assert_eq!(arr.len, 2);
        assert!(!arr.data.is_null());
        // SAFETY: `arr` was produced by `from_vec` above; the free matches the
        // `Box::into_raw` + per-string `CString::into_raw` that built it.
        unsafe { thetadatadx_string_array_free(arr) };
    }
}
