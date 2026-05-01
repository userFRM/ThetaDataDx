//! Standalone utilities: Black-Scholes Greeks, implied volatility, and the
//! feature-gated panic-test entry points.
//!
//! Split verbatim from `lib.rs`; the exported C ABI is unchanged.

use std::os::raw::c_char;

use crate::error::set_error;

// ═══════════════════════════════════════════════════════════════════════
//  Greeks (standalone, not client methods)
// ═══════════════════════════════════════════════════════════════════════

/// All 23 Black-Scholes Greeks + IV as a typed C struct.
#[repr(C)]
pub struct TdxGreeksResult {
    pub value: f64,
    pub delta: f64,
    pub gamma: f64,
    pub theta: f64,
    pub vega: f64,
    pub rho: f64,
    pub epsilon: f64,
    pub lambda: f64,
    pub vanna: f64,
    pub charm: f64,
    pub vomma: f64,
    pub veta: f64,
    pub vera: f64,
    pub speed: f64,
    pub zomma: f64,
    pub color: f64,
    pub ultima: f64,
    pub iv: f64,
    pub iv_error: f64,
    pub d1: f64,
    pub d2: f64,
    pub dual_delta: f64,
    pub dual_gamma: f64,
}

/// Compute all 22 Black-Scholes Greeks + IV.
///
/// `right` accepts `"C"`/`"P"` or `"call"`/`"put"` case-insensitively (see
/// the `tdbe::right::parse_right` canonical parser). Returns a heap-allocated
/// `TdxGreeksResult`, or null on error (invalid UTF-8 / unrecognised right /
/// resolves to `both`). Caller must free the result with
/// `tdx_greeks_result_free`.
///
/// # Safety
///
/// `right` must be a valid NUL-terminated C string pointer (or null, which
/// returns null with an error set).
#[no_mangle]
pub unsafe extern "C" fn tdx_all_greeks(
    spot: f64,
    strike: f64,
    rate: f64,
    div_yield: f64,
    tte: f64,
    option_price: f64,
    right: *const c_char,
) -> *mut TdxGreeksResult {
    ffi_boundary!(std::ptr::null_mut(), {
        let right_str = require_cstr!(right, std::ptr::null_mut());
        let g = match tdbe::greeks::all_greeks(
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
        let result = TdxGreeksResult {
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

/// Free a `TdxGreeksResult` returned by `tdx_all_greeks`.
#[no_mangle]
pub unsafe extern "C" fn tdx_greeks_result_free(ptr: *mut TdxGreeksResult) {
    ffi_boundary!((), {
        if !ptr.is_null() {
            drop(unsafe { Box::from_raw(ptr) });
        }
    })
}

/// Compute implied volatility via bisection.
///
/// `right` accepts `"C"`/`"P"` or `"call"`/`"put"` case-insensitively (see
/// the `tdbe::right::parse_right` canonical parser). Returns IV in `*out_iv`
/// and error in `*out_error`. Returns 0 on success, -1 on failure (null
/// pointers / invalid UTF-8 / unrecognised right / resolves to `both`).
///
/// # Safety
///
/// `right` must be a valid NUL-terminated C string pointer. `out_iv` and
/// `out_error` must be valid, writable `double` pointers.
#[no_mangle]
pub unsafe extern "C" fn tdx_implied_volatility(
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
        let (iv, err) = match tdbe::greeks::implied_volatility(
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
        unsafe {
            *out_iv = iv;
            *out_error = err;
        }
        0
    })
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
//    3. make the panic payload retrievable via `tdx_last_error()`.
//
//  The symbols are only compiled in when the feature is enabled, so
//  the shared library shipped to Go / C++ / Python consumers never
//  carries a "panic-on-demand" entry point.
// ═══════════════════════════════════════════════════════════════════════

/// Deliberately panic with a `&'static str` payload. Returns -1 via the
/// boundary's default handler. The panic message becomes part of the
/// `tdx_last_error()` string so the caller can verify the downcast path
/// that handles `&'static str` payloads works end to end.
#[cfg(feature = "testing-panic-boundary")]
#[no_mangle]
pub extern "C" fn tdx_test_panic_str() -> i32 {
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
pub extern "C" fn tdx_test_panic_string() -> i32 {
    ffi_boundary!(-1, {
        panic!("{}", String::from("intentional test panic via String"));
    })
}
