//! Standalone utilities: Black-Scholes Greeks, implied volatility, and the
//! feature-gated panic-test entry points.
//!
//! Split verbatim from `lib.rs`; the exported C ABI is unchanged.

use std::os::raw::c_char;

use crate::error::set_error;
use crate::streaming::ThetaDataDxContract;

// ═══════════════════════════════════════════════════════════════════════
//  Greeks (standalone, not client methods)
// ═══════════════════════════════════════════════════════════════════════

/// All 23 Black-Scholes Greeks + IV as a typed C struct.
#[repr(C)]
pub struct ThetaDataDxGreeksResult {
    /// Black-Scholes theoretical option value.
    pub value: f64,
    /// First derivative of value in spot.
    pub delta: f64,
    /// Second derivative of value in spot.
    pub gamma: f64,
    /// Sensitivity to time, per calendar day.
    pub theta: f64,
    /// Sensitivity to volatility.
    pub vega: f64,
    /// Sensitivity to the risk-free rate.
    pub rho: f64,
    /// Sensitivity of value to the dividend yield.
    pub epsilon: f64,
    /// Elasticity: percentage change in value per percentage change in spot.
    pub lambda: f64,
    /// Sensitivity of delta to volatility.
    pub vanna: f64,
    /// Sensitivity of delta to time.
    pub charm: f64,
    /// Sensitivity of vega to volatility.
    pub vomma: f64,
    /// Sensitivity of vega to time.
    pub veta: f64,
    /// Sensitivity of vega to the risk-free rate.
    pub vera: f64,
    /// Sensitivity of gamma to spot.
    pub speed: f64,
    /// Sensitivity of gamma to volatility.
    pub zomma: f64,
    /// Sensitivity of gamma to time.
    pub color: f64,
    /// Sensitivity of vomma to volatility.
    pub ultima: f64,
    /// Implied volatility recovered from the option price.
    pub iv: f64,
    /// Relative residual of the IV solve (`(value - price) / price`), clamped to `[-100.0, 100.0]`.
    pub iv_error: f64,
    /// Black-Scholes `d1` term.
    pub d1: f64,
    /// Black-Scholes `d2` term.
    pub d2: f64,
    /// Sensitivity of value to the strike.
    pub dual_delta: f64,
    /// Sensitivity of dual delta to the strike.
    pub dual_gamma: f64,
}

/// Compute all 23 Black-Scholes Greeks + IV.
///
/// `right` accepts `"C"`/`"P"` or `"call"`/`"put"` case-insensitively (see
/// the `thetadatadx::greeks::parse_right` canonical parser). Returns a heap-allocated
/// `ThetaDataDxGreeksResult`, or null on error (invalid UTF-8 / unrecognised right /
/// resolves to `both`). Caller must free the result with
/// `thetadatadx_greeks_result_free`.
///
/// # Safety
///
/// `right` must be a valid NUL-terminated C string pointer (or null, which
/// returns null with an error set).
#[no_mangle]
pub unsafe extern "C" fn thetadatadx_all_greeks(
    spot: f64,
    strike: f64,
    rate: f64,
    div_yield: f64,
    tte: f64,
    option_price: f64,
    right: *const c_char,
) -> *mut ThetaDataDxGreeksResult {
    ffi_boundary!(std::ptr::null_mut(), {
        let right_str = require_cstr!(right, std::ptr::null_mut());
        let g = match thetadatadx::greeks::all_greeks(
            spot,
            strike,
            rate,
            div_yield,
            tte,
            option_price,
            right_str,
        ) {
            Ok(g) => g,
            Err(e) => {
                set_error(&e.to_string());
                return std::ptr::null_mut();
            }
        };
        let result = ThetaDataDxGreeksResult {
            value: g.value,
            delta: g.delta,
            gamma: g.gamma,
            theta: g.theta,
            vega: g.vega,
            rho: g.rho,
            epsilon: g.epsilon,
            lambda: g.lambda,
            vanna: g.vanna,
            charm: g.charm,
            vomma: g.vomma,
            veta: g.veta,
            vera: g.vera,
            speed: g.speed,
            zomma: g.zomma,
            color: g.color,
            ultima: g.ultima,
            iv: g.iv,
            iv_error: g.iv_error,
            d1: g.d1,
            d2: g.d2,
            dual_delta: g.dual_delta,
            dual_gamma: g.dual_gamma,
        };
        Box::into_raw(Box::new(result))
    })
}

/// Free a `ThetaDataDxGreeksResult` returned by `thetadatadx_all_greeks`.
#[no_mangle]
pub unsafe extern "C" fn thetadatadx_greeks_result_free(ptr: *mut ThetaDataDxGreeksResult) {
    ffi_boundary!((), {
        if !ptr.is_null() {
            // SAFETY: the pointer was returned by Box::into_raw / thetadatadx_*_new and has not been freed; ownership returns to Rust.
            drop(unsafe { Box::from_raw(ptr) });
        }
    })
}

/// Compute implied volatility via bisection.
///
/// `right` accepts `"C"`/`"P"` or `"call"`/`"put"` case-insensitively (see
/// the `thetadatadx::greeks::parse_right` canonical parser). Returns IV in `*out_iv`
/// and error in `*out_error`. Returns 0 on success, -1 on failure (null
/// pointers / invalid UTF-8 / unrecognised right / resolves to `both`).
///
/// # Safety
///
/// `right` must be a valid NUL-terminated C string pointer. `out_iv` and
/// `out_error` must be valid, writable `double` pointers.
#[no_mangle]
pub unsafe extern "C" fn thetadatadx_implied_volatility(
    spot: f64,
    strike: f64,
    rate: f64,
    div_yield: f64,
    tte: f64,
    option_price: f64,
    right: *const c_char,
    out_iv: *mut f64,
    out_error: *mut f64,
) -> i32 {
    ffi_boundary!(-1, {
        if out_iv.is_null() || out_error.is_null() {
            set_error("output pointers must not be null");
            return -1;
        }
        let right_str = require_cstr!(right, -1);
        let (iv, err) = match thetadatadx::greeks::implied_volatility(
            spot,
            strike,
            rate,
            div_yield,
            tte,
            option_price,
            right_str,
        ) {
            Ok(pair) => pair,
            Err(e) => {
                set_error(&e.to_string());
                return -1;
            }
        };
        // SAFETY: out_iv/out_error null-checked above; caller pins the storage they point at for the call duration.
        unsafe {
            *out_iv = iv;
            *out_error = err;
        }
        0
    })
}

// ═══════════════════════════════════════════════════════════════════════
//  Condition / exchange / sequence helper accessors
//
//  Cross-language utility parity. The lookup tables live in
//  `thetadatadx::{conditions, exchange, sequences}`. The C ABI wraps them as
//  string-returning entry points (returning `'static` UTF-8 NUL-terminated
//  C strings — the underlying tables are `&'static str`, so the caller
//  MUST NOT free the returned pointer) plus a couple of `bool`-returning
//  predicates and integer accessors for trade-sequence math.
// ═══════════════════════════════════════════════════════════════════════

// every `thetadatadx_condition_*` / `thetadatadx_exchange_*` / `thetadatadx_quote_condition_*`
// entry point wraps its body in `ffi_boundary!` so a panic in the
// condition / exchange lookup tables (debug-build invariant trips,
// etc.) or in `static_cstr` (cache mutex contention, allocator OOM) cannot abort
// the host process. The wrappers return the documented "unknown"
// sentinel:
//   - `*const c_char` returners → `std::ptr::null()` (caller already
//     handles NUL on unknown codes — every binding's lookup contract
//     is "NULL or unknown sentinel = no data").
//   - `bool` returners → `false` (the predicate convention is "false
//     by default; only true when the table positively asserts").

/// Look up the human-readable trade condition name for `code`.
///
/// Returns a NUL-terminated `'static` UTF-8 C string. The pointer is
/// owned by the library and MUST NOT be freed. Returns the literal
/// `"UNKNOWN"` for codes outside the table; returns `NULL` if the
/// boundary catches a panic — surfaced through `thetadatadx_last_error()`.
#[no_mangle]
pub extern "C" fn thetadatadx_condition_name(code: i32) -> *const c_char {
    ffi_boundary!(std::ptr::null(), {
        static_cstr(thetadatadx::utils::conditions::condition_name(code))
    })
}

/// Look up the human-readable trade condition description for `code`.
///
/// Returns a NUL-terminated `'static` UTF-8 C string (empty string for
/// unknown codes). The pointer is owned by the library and MUST NOT be
/// freed. Returns `NULL` if the boundary catches a panic.
#[no_mangle]
pub extern "C" fn thetadatadx_condition_description(code: i32) -> *const c_char {
    ffi_boundary!(std::ptr::null(), {
        static_cstr(thetadatadx::utils::conditions::condition_description(code))
    })
}

/// True if the trade condition code represents a cancellation.
#[no_mangle]
pub extern "C" fn thetadatadx_condition_is_cancel(code: i32) -> bool {
    ffi_boundary!(false, { thetadatadx::utils::conditions::is_cancel(code) })
}

/// True if the trade condition code updates the volume bar.
#[no_mangle]
pub extern "C" fn thetadatadx_condition_updates_volume(code: i32) -> bool {
    ffi_boundary!(false, {
        thetadatadx::utils::conditions::updates_volume(code)
    })
}

/// Look up the human-readable quote condition name for `code`.
#[no_mangle]
pub extern "C" fn thetadatadx_quote_condition_name(code: i32) -> *const c_char {
    ffi_boundary!(std::ptr::null(), {
        static_cstr(thetadatadx::utils::conditions::quote_condition_name(code))
    })
}

/// Look up the human-readable quote condition description for `code`.
#[no_mangle]
pub extern "C" fn thetadatadx_quote_condition_description(code: i32) -> *const c_char {
    ffi_boundary!(std::ptr::null(), {
        static_cstr(thetadatadx::utils::conditions::quote_condition_description(
            code,
        ))
    })
}

/// True if the quote condition is firm (binding).
#[no_mangle]
pub extern "C" fn thetadatadx_quote_condition_is_firm(code: i32) -> bool {
    ffi_boundary!(false, { thetadatadx::utils::conditions::is_firm(code) })
}

/// True if the quote condition indicates a trading halt.
#[no_mangle]
pub extern "C" fn thetadatadx_quote_condition_is_halted(code: i32) -> bool {
    ffi_boundary!(false, { thetadatadx::utils::conditions::is_halted(code) })
}

/// Look up the human-readable exchange name for a numeric code.
#[no_mangle]
pub extern "C" fn thetadatadx_exchange_name(code: i32) -> *const c_char {
    ffi_boundary!(std::ptr::null(), {
        static_cstr(thetadatadx::utils::exchange::exchange_name(code))
    })
}

/// Look up the MIC-like symbol for a numeric exchange code.
#[no_mangle]
pub extern "C" fn thetadatadx_exchange_symbol(code: i32) -> *const c_char {
    ffi_boundary!(std::ptr::null(), {
        static_cstr(thetadatadx::utils::exchange::exchange_symbol(code))
    })
}

/// Convert a signed wire-encoded trade-sequence value to its unsigned
/// monotonic form. Mirrors `thetadatadx::utils::sequences::signed_to_unsigned`.
///
/// `signed` must lie in the i32 wire range
/// (`-2_147_483_648 ..= 2_147_483_647`): the upstream terminal encodes
/// trade sequences as i32, so a value outside that domain is not a wire
/// sequence and would otherwise be silently reinterpreted into a
/// look-correct-but-wrong id. Writes the converted value to `out` and
/// returns `0` on success; returns `-1` and sets `thetadatadx_last_error` /
/// `thetadatadx_last_error_code = TDX_ERR_INVALID_PARAMETER` when `signed` is
/// outside the wire range or `out` is null, matching the typed class the
/// Python / TypeScript bindings raise for the same input.
///
/// # Safety
/// `out` must be a valid, non-null pointer to a `uint64_t` the caller
/// keeps alive for the call duration.
#[no_mangle]
pub unsafe extern "C" fn thetadatadx_sequence_signed_to_unsigned(
    signed: i64,
    out: *mut u64,
) -> i32 {
    ffi_boundary!(-1, {
        if out.is_null() {
            crate::error::set_error_with_code(
                "thetadatadx_sequence_signed_to_unsigned: out pointer is null",
                crate::error::TDX_ERR_INVALID_PARAMETER,
            );
            return -1;
        }
        if !(thetadatadx::utils::sequences::SEQUENCE_MIN
            ..=thetadatadx::utils::sequences::SEQUENCE_MAX)
            .contains(&signed)
        {
            crate::error::set_error_with_code(
                &format!(
                    "thetadatadx_sequence_signed_to_unsigned: {signed} is outside the i32 wire range (-2_147_483_648 ..= 2_147_483_647)"
                ),
                crate::error::TDX_ERR_INVALID_PARAMETER,
            );
            return -1;
        }
        // SAFETY: `out` is non-null per the guard above and the FFI
        // contract pins the storage for the call duration.
        unsafe {
            *out = thetadatadx::utils::sequences::signed_to_unsigned(signed);
        }
        0
    })
}

/// Convert an unsigned monotonic trade-sequence value back to its
/// signed wire encoding. Mirrors `thetadatadx::utils::sequences::unsigned_to_signed`.
///
/// `unsigned` must lie in the unsigned wire range (`0 ..= 2^32 - 1`):
/// the monotonic sequence id is never wider than one i32 cycle, so a
/// value above that domain is not a wire sequence and would otherwise be
/// silently reinterpreted. Writes the converted value to `out` and
/// returns `0` on success; returns `-1` and sets `thetadatadx_last_error` /
/// `thetadatadx_last_error_code = TDX_ERR_INVALID_PARAMETER` when `unsigned` is
/// above the wire range or `out` is null, matching the typed class the
/// Python / TypeScript bindings raise for the same input.
///
/// # Safety
/// `out` must be a valid, non-null pointer to an `int64_t` the caller
/// keeps alive for the call duration.
#[no_mangle]
pub unsafe extern "C" fn thetadatadx_sequence_unsigned_to_signed(
    unsigned: u64,
    out: *mut i64,
) -> i32 {
    ffi_boundary!(-1, {
        if out.is_null() {
            crate::error::set_error_with_code(
                "thetadatadx_sequence_unsigned_to_signed: out pointer is null",
                crate::error::TDX_ERR_INVALID_PARAMETER,
            );
            return -1;
        }
        if unsigned > u64::from(u32::MAX) {
            crate::error::set_error_with_code(
                &format!(
                    "thetadatadx_sequence_unsigned_to_signed: {unsigned} is above the unsigned wire range (0 ..= 2^32 - 1)"
                ),
                crate::error::TDX_ERR_INVALID_PARAMETER,
            );
            return -1;
        }
        // SAFETY: `out` is non-null per the guard above and the FFI
        // contract pins the storage for the call duration.
        unsafe {
            *out = thetadatadx::utils::sequences::unsigned_to_signed(unsigned);
        }
        0
    })
}

/// Look up the vendor vocabulary text for a `ThetaDataDxCalendarDay.status`
/// code (`0` -> `"open"`, `1` -> `"early_close"`, `2` -> `"full_close"`,
/// `3` -> `"weekend"`).
///
/// Returns a NUL-terminated `'static` UTF-8 C string. The pointer is
/// owned by the library and MUST NOT be freed. Returns the literal
/// `"UNKNOWN"` for codes outside the table; returns `NULL` if the
/// boundary catches a panic — surfaced through `thetadatadx_last_error()`.
#[no_mangle]
pub extern "C" fn thetadatadx_calendar_status_name(code: i32) -> *const c_char {
    ffi_boundary!(std::ptr::null(), {
        static_cstr(
            thetadatadx::CalendarStatus::from_code(code)
                .map_or("UNKNOWN", thetadatadx::CalendarStatus::as_str),
        )
    })
}

/// Combine an Eastern-Time `YYYYMMDD` date and milliseconds-of-day into
/// Unix epoch milliseconds (UTC, DST-aware). Mirrors the
/// `*_timestamp_ms()` accessors the Rust / Python row surfaces expose,
/// usable with any `(date, *_ms_of_day)` pair on the tick structs.
///
/// Returns `-1` when `date` is not a valid Gregorian `YYYYMMDD`
/// (including the `0` absent fill) or `ms_of_day` is outside
/// `0..86_400_000` — `-1` is unreachable for real market data (it
/// denotes 1969-12-31T23:59:59.999Z).
#[no_mangle]
pub extern "C" fn thetadatadx_timestamp_ms(date: i32, ms_of_day: i32) -> i64 {
    ffi_boundary!(-1, {
        thetadatadx::time::date_ms_to_epoch_ms(date, ms_of_day).unwrap_or(-1)
    })
}

/// Read the option strike of a streaming `ThetaDataDxContract` in dollars, folding
/// the `has_strike` presence flag into the return value. Mirrors the C++
/// `thetadatadx::strike(const ThetaDataDxContract&)` accessor and the Python / TypeScript
/// `contract.strike` surface, which return an absent value for non-option
/// contracts rather than a bare `0.0` the caller must special-case.
///
/// `contract.strike` already carries dollars, so this only surfaces the
/// presence flag a plain field read would otherwise drop: writes the
/// dollar value to `out_dollars` and returns `true` when the contract is
/// an option, leaves `out_dollars` untouched and returns `false`
/// otherwise (non-option, or a null contract / output pointer).
///
/// # Safety
/// `contract` must be a valid `ThetaDataDxContract` pointer (e.g. the
/// `event.<variant>.contract` field of a `ThetaDataDxStreamEvent`). `out_dollars`
/// must be a valid, writable `double` pointer.
#[no_mangle]
pub unsafe extern "C" fn thetadatadx_contract_strike_dollars(
    contract: *const ThetaDataDxContract,
    out_dollars: *mut f64,
) -> bool {
    ffi_boundary!(false, {
        if contract.is_null() || out_dollars.is_null() {
            return false;
        }
        // SAFETY: both pointers null-checked above; the caller pins the
        // contract and output storage for the call duration.
        let contract = unsafe { &*contract };
        if !contract.has_strike {
            return false;
        }
        // SAFETY: out_dollars null-checked above; the caller pins the
        // output storage for the call duration.
        unsafe {
            *out_dollars = contract.strike;
        }
        true
    })
}

/// Convert a `&'static str` from the lookup tables into a stable
/// `*const c_char` for FFI. The lookup tables are compile-time arrays
/// of NUL-free `&'static str`; we register one `CString` per distinct
/// string in a process-lifetime `OnceLock<Mutex<HashMap<...>>>` and
/// return the cached pointer so the C side can hold it indefinitely.
///
/// poison-tolerant via `PoisonError::into_inner` rather than
/// `.expect(...)`. A panic in a previous holder of this cache mutex
/// (e.g. an OOM during `Box::leak`) leaves the map structurally
/// valid — we have no transient half-mutated state because every
/// insertion is `guard.insert(k, v)` after a successful allocation,
/// not a partial update. Recovering the inner map rather than
/// panicking again keeps every `thetadatadx_condition_*` / `thetadatadx_exchange_*`
/// /`thetadatadx_quote_condition_*` lookup non-aborting, matching the
/// `ffi_boundary!` contract on every other FFI entry point.
fn static_cstr(s: &'static str) -> *const c_char {
    use std::collections::HashMap;
    use std::ffi::CString;
    use std::sync::Mutex;
    static CACHE: std::sync::OnceLock<Mutex<HashMap<&'static str, &'static CString>>> =
        std::sync::OnceLock::new();
    let cache = CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    let mut guard = cache.lock().unwrap_or_else(|e| e.into_inner());
    let cstr_ref: &'static CString = match guard.get(&s) {
        Some(existing) => existing,
        None => {
            // Tables are compile-time `&'static str` literals known to
            // be NUL-free; CString::new only fails on interior NULs.
            let owned =
                CString::new(s).expect("lookup-table strings must not contain interior NULs");
            // Leak so the pointer is `'static` for the caller. There is
            // a finite, small number of distinct entries (≤ a few hundred
            // across all tables), so the leak is bounded.
            let leaked: &'static CString = Box::leak(Box::new(owned));
            guard.insert(s, leaked);
            leaked
        }
    };
    cstr_ref.as_ptr()
}

// ═══════════════════════════════════════════════════════════════════════
//  Test-only panic entry points (feature `testing-panic-boundary`)
//
//  These exist purely so the integration test at
//  `ffi/tests/panic_boundary.rs` can prove that panics inside an
//  `extern "C"` body:
//    1. do NOT abort the process (the test binary would crash),
//    2. return the declared default (`-1` here, matching the
//       existing `i32` status-code convention),
//    3. make the panic payload retrievable via `thetadatadx_last_error()`.
//
//  The symbols are only compiled in when the feature is enabled, so
//  the shared library shipped to Go / C++ / Python consumers never
//  carries a "panic-on-demand" entry point.
// ═══════════════════════════════════════════════════════════════════════

/// Deliberately panic with a `&'static str` payload. Returns -1 via the
/// boundary's default handler. The panic message becomes part of the
/// `thetadatadx_last_error()` string so the caller can verify the downcast path
/// that handles `&'static str` payloads works end to end.
#[cfg(feature = "testing-panic-boundary")]
#[no_mangle]
pub extern "C" fn thetadatadx_test_panic_str() -> i32 {
    ffi_boundary!(-1, {
        panic!("intentional test panic via &'static str");
    })
}

/// Deliberately panic with a heap-allocated `String` payload. Returns -1
/// via the boundary's default handler. Separate from the `&'static str`
/// variant so the test suite can exercise both `downcast_ref::<&'static
/// str>` and `downcast_ref::<String>` branches of the macro.
#[cfg(feature = "testing-panic-boundary")]
#[no_mangle]
pub extern "C" fn thetadatadx_test_panic_string() -> i32 {
    ffi_boundary!(-1, {
        panic!("{}", String::from("intentional test panic via String"));
    })
}

#[cfg(test)]
mod sequence_tests {
    //! Wire-range validation for the trade-sequence converters. Out-of
    //! -wire-range inputs must be rejected with the typed
    //! invalid-parameter class rather than silently reinterpreted — the
    //! cross-binding contract the Python `ValueError` / TypeScript
    //! `InvalidParameterError` already honour.

    #[test]
    fn signed_to_unsigned_round_trips_in_wire_range() {
        for signed in [i64::from(i32::MIN), -1, 0, 1, i64::from(i32::MAX)] {
            let mut out: u64 = 0;
            // SAFETY: `out` points at a live stack slot for the call.
            let rc = unsafe { super::thetadatadx_sequence_signed_to_unsigned(signed, &mut out) };
            assert_eq!(rc, 0, "in-range input {signed} accepted");
            assert_eq!(
                out,
                thetadatadx::utils::sequences::signed_to_unsigned(signed)
            );
        }
    }

    #[test]
    fn signed_to_unsigned_rejects_out_of_wire_range() {
        for signed in [i64::from(i32::MAX) + 1, i64::from(i32::MIN) - 1] {
            let mut out: u64 = 0;
            crate::error::thetadatadx_clear_error();
            // SAFETY: `out` points at a live stack slot for the call.
            let rc = unsafe { super::thetadatadx_sequence_signed_to_unsigned(signed, &mut out) };
            assert_eq!(rc, -1, "out-of-range input {signed} rejected");
            assert_eq!(
                crate::error::thetadatadx_last_error_code(),
                crate::error::TDX_ERR_INVALID_PARAMETER
            );
        }
    }

    #[test]
    fn unsigned_to_signed_round_trips_in_wire_range() {
        for unsigned in [0u64, 1, u64::from(i32::MAX as u32), u64::from(u32::MAX)] {
            let mut out: i64 = 0;
            // SAFETY: `out` points at a live stack slot for the call.
            let rc = unsafe { super::thetadatadx_sequence_unsigned_to_signed(unsigned, &mut out) };
            assert_eq!(rc, 0, "in-range input {unsigned} accepted");
            assert_eq!(
                out,
                thetadatadx::utils::sequences::unsigned_to_signed(unsigned)
            );
        }
    }

    #[test]
    fn unsigned_to_signed_rejects_above_wire_range() {
        // 2^32 is the first value past the unsigned wire range.
        let mut out: i64 = 0;
        crate::error::thetadatadx_clear_error();
        // SAFETY: `out` points at a live stack slot for the call.
        let rc = unsafe {
            super::thetadatadx_sequence_unsigned_to_signed(u64::from(u32::MAX) + 1, &mut out)
        };
        assert_eq!(rc, -1, "2^32 rejected as above the wire range");
        assert_eq!(
            crate::error::thetadatadx_last_error_code(),
            crate::error::TDX_ERR_INVALID_PARAMETER
        );
    }

    #[test]
    fn null_out_pointer_rejected() {
        crate::error::thetadatadx_clear_error();
        // SAFETY: deliberately passing null to exercise the guard.
        let rc = unsafe { super::thetadatadx_sequence_signed_to_unsigned(0, std::ptr::null_mut()) };
        assert_eq!(rc, -1);
        assert_eq!(
            crate::error::thetadatadx_last_error_code(),
            crate::error::TDX_ERR_INVALID_PARAMETER
        );
    }
}
