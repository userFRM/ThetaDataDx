//! REST-routing policy + `_with_fallback` C ABI shims.
//!
//! Mirrors the Python `FallbackPolicy` pyclass + `option_history_*_with_fallback`
//! methods one-for-one, exposing the same surface to any C/C++/Go/etc.
//! consumer that links against `libthetadatadx_ffi`.
//!
//! # Memory model
//!
//! - [`TdxFallbackPolicy`] is an opaque heap-allocated handle. The
//!   factories (`tdx_fallback_policy_disabled`, `_rest_always`) return
//!   ownership to the caller; the caller MUST eventually call
//!   `tdx_fallback_policy_free`.
//! - `tdx_config_with_rest_fallback` borrows the policy (reads its
//!   inner enum + clones it onto the `TdxConfig`); the caller still
//!   owns the policy handle after the call returns.
//! - The four `option_history_*_with_fallback` shims block on the
//!   shared tokio runtime exactly like every other historical
//!   endpoint in the FFI surface; the returned tick arrays follow the
//!   same lifetime contract documented in [`crate::types`].

use std::os::raw::c_char;
use std::ptr;

use thetadatadx::config;

use crate::error::{cstr_to_str, set_error, set_error_from};
use crate::runtime;
use crate::types::{
    TdxClient, TdxConfig, TdxGreeksFirstOrderTickArray, TdxIvTickArray, TdxQuoteTickArray,
    TdxTradeQuoteTickArray,
};

/// Opaque REST-fallback policy handle.
///
/// Wraps [`thetadatadx::config::FallbackPolicy`]. Construct via one of
/// the two factory functions (`tdx_fallback_policy_disabled`,
/// `_rest_always`), install on a `TdxConfig` via
/// `tdx_config_with_rest_fallback`, and free with
/// `tdx_fallback_policy_free`.
pub struct TdxFallbackPolicy {
    pub(crate) inner: config::FallbackPolicy,
}

// ── Factories ────────────────────────────────────────────────────────

/// Construct a [`config::FallbackPolicy::Disabled`] policy. REST
/// routing is off — every historical-quote endpoint goes over gRPC.
/// Default state; identical to constructing a `TdxConfig` without
/// calling `tdx_config_with_rest_fallback`.
#[no_mangle]
pub extern "C" fn tdx_fallback_policy_disabled() -> *mut TdxFallbackPolicy {
    ffi_boundary!(ptr::null_mut(), {
        Box::into_raw(Box::new(TdxFallbackPolicy {
            inner: config::FallbackPolicy::Disabled,
        }))
    })
}

/// Construct a [`config::FallbackPolicy::RestAlways`] policy. Always
/// routes the four historical-quote endpoints over REST regardless of
/// the requested date range. Use when the caller wants a single
/// transport for every quote-bearing call.
#[no_mangle]
pub unsafe extern "C" fn tdx_fallback_policy_rest_always(
    base_url: *const c_char,
) -> *mut TdxFallbackPolicy {
    ffi_boundary!(ptr::null_mut(), {
        // SAFETY: caller supplies a NUL-terminated C string allocated by the host runtime; cstr_to_str validates non-null + UTF-8.
        let base_url = match unsafe { cstr_to_str(base_url) } {
            Ok(Some(s)) => s.to_string(),
            Ok(None) => {
                set_error("base_url is null");
                return ptr::null_mut();
            }
            Err(e) => {
                set_error(&format!("base_url is not valid UTF-8: {e}"));
                return ptr::null_mut();
            }
        };
        Box::into_raw(Box::new(TdxFallbackPolicy {
            inner: config::FallbackPolicy::RestAlways { base_url },
        }))
    })
}

/// Free a fallback policy handle returned by any
/// `tdx_fallback_policy_*` factory.
#[no_mangle]
pub unsafe extern "C" fn tdx_fallback_policy_free(policy: *mut TdxFallbackPolicy) {
    ffi_boundary!((), {
        if !policy.is_null() {
            // SAFETY: the pointer was returned by Box::into_raw / tdx_fallback_policy_* and has not been freed; ownership returns to Rust.
            drop(unsafe { Box::from_raw(policy) });
        }
    })
}

// ── Config integration ───────────────────────────────────────────────

/// Install a REST-routing policy on a [`TdxConfig`].
///
/// Subsequent calls to `tdx_option_history_*_with_fallback` against any
/// client built from this config will consult the policy.
///
/// Returns `0` on success, non-zero on null-pointer error (also sets
/// `tdx_last_error`). The policy handle is NOT consumed -- the caller
/// retains ownership and MUST still free it via
/// `tdx_fallback_policy_free` when finished.
///
/// Equivalent to `Python`'s `cfg.with_rest_fallback(policy)`.
#[no_mangle]
pub unsafe extern "C" fn tdx_config_with_rest_fallback(
    config: *mut TdxConfig,
    policy: *const TdxFallbackPolicy,
) -> i32 {
    ffi_boundary!(-1, {
        if config.is_null() {
            set_error("config is null");
            return -1;
        }
        if policy.is_null() {
            set_error("policy is null");
            return -1;
        }
        // SAFETY: config is a non-null pointer returned by tdx_config_production / _dev / _stage and not yet freed.
        let config = unsafe { &mut *config };
        // SAFETY: policy is a non-null pointer returned by tdx_fallback_policy_* and not yet freed.
        let policy = unsafe { &*policy };
        config.inner.fallback = policy.inner.clone();
        0
    })
}

// ── _with_fallback historical endpoint shims ─────────────────────────
//
// One per historical-quote endpoint. The signature shape mirrors the
// Rust core method (`option_history_*_with_fallback`) with each
// `Option<&str>` represented as a nullable `*const c_char` -- pass
// `NULL` to omit the arg.

/// Validate a required `*const c_char` arg, set `tdx_last_error` on
/// null / invalid UTF-8, and short-circuit the enclosing fn with the
/// supplied empty-array sentinel.
macro_rules! require_str {
    ($arg:expr, $name:literal, $empty:expr) => {{
        // SAFETY: caller supplies a NUL-terminated C string allocated by the host runtime; cstr_to_str validates non-null + UTF-8.
        match unsafe { cstr_to_str($arg) } {
            Ok(Some(v)) => v.to_string(),
            Ok(None) => {
                set_error(concat!($name, " is null"));
                return $empty;
            }
            Err(e) => {
                set_error(&format!(concat!($name, " is not valid UTF-8: {}"), e));
                return $empty;
            }
        }
    }};
}

/// Validate an optional `*const c_char` arg, returning `Option<String>`.
/// Null is the legal "omit" sentinel; invalid UTF-8 short-circuits with
/// the supplied empty sentinel.
macro_rules! optional_str {
    ($arg:expr, $name:literal, $empty:expr) => {{
        // SAFETY: caller supplies a NUL-terminated C string allocated by the host runtime; cstr_to_str validates non-null + UTF-8.
        match unsafe { cstr_to_str($arg) } {
            Ok(Some(v)) => Some(v.to_string()),
            Ok(None) => None,
            Err(e) => {
                set_error(&format!(concat!($name, " is not valid UTF-8: {}"), e));
                return $empty;
            }
        }
    }};
}

/// Fetch option NBBO history per the configured
/// [`config::FallbackPolicy`] on the client's `TdxConfig`. See
/// [`thetadatadx::mdds::MddsClient::option_history_quote_with_fallback`]
/// for the dispatch semantics.
///
/// `symbol`, `expiration`, `start_date` are required. `end_date`,
/// `strike`, `right`, `interval` may be NULL to omit.
#[no_mangle]
#[allow(clippy::too_many_arguments)]
pub unsafe extern "C" fn tdx_option_history_quote_with_fallback(
    client: *const TdxClient,
    symbol: *const c_char,
    expiration: *const c_char,
    start_date: *const c_char,
    end_date: *const c_char,
    strike: *const c_char,
    right: *const c_char,
    interval: *const c_char,
) -> TdxQuoteTickArray {
    ffi_boundary!(
        TdxQuoteTickArray {
            data: ptr::null(),
            len: 0,
        },
        {
            let empty = TdxQuoteTickArray {
                data: ptr::null(),
                len: 0,
            };
            if client.is_null() {
                set_error("client handle is null");
                return empty;
            }
            let symbol = require_str!(symbol, "symbol", empty);
            let expiration = require_str!(expiration, "expiration", empty);
            let start_date = require_str!(start_date, "start_date", empty);
            let end_date = optional_str!(end_date, "end_date", empty);
            let strike = optional_str!(strike, "strike", empty);
            let right = optional_str!(right, "right", empty);
            let interval = optional_str!(interval, "interval", empty);

            // SAFETY: client is a non-null pointer returned by tdx_client_connect and not yet freed.
            let client = unsafe { &*client };
            let result = runtime().block_on(async {
                client
                    .inner
                    .option_history_quote_with_fallback(
                        &symbol,
                        &expiration,
                        &start_date,
                        end_date.as_deref(),
                        strike.as_deref(),
                        right.as_deref(),
                        interval.as_deref(),
                    )
                    .await
            });
            match result {
                Ok(ticks) => match TdxQuoteTickArray::from_vec(ticks) {
                    Ok(arr) => arr,
                    Err(e) => {
                        set_error(&format!("interior NUL in server string: {e}"));
                        empty
                    }
                },
                Err(err) => {
                    set_error_from(&err);
                    empty
                }
            }
        }
    )
}

/// Fetch combined trade+quote history per the configured
/// [`config::FallbackPolicy`]. See
/// [`thetadatadx::mdds::MddsClient::option_history_trade_quote_with_fallback`].
///
/// `symbol`, `expiration`, `start_date` are required. `end_date`,
/// `strike`, `right` may be NULL to omit.
#[no_mangle]
#[allow(clippy::too_many_arguments)]
pub unsafe extern "C" fn tdx_option_history_trade_quote_with_fallback(
    client: *const TdxClient,
    symbol: *const c_char,
    expiration: *const c_char,
    start_date: *const c_char,
    end_date: *const c_char,
    strike: *const c_char,
    right: *const c_char,
) -> TdxTradeQuoteTickArray {
    ffi_boundary!(
        TdxTradeQuoteTickArray {
            data: ptr::null(),
            len: 0,
        },
        {
            let empty = TdxTradeQuoteTickArray {
                data: ptr::null(),
                len: 0,
            };
            if client.is_null() {
                set_error("client handle is null");
                return empty;
            }
            let symbol = require_str!(symbol, "symbol", empty);
            let expiration = require_str!(expiration, "expiration", empty);
            let start_date = require_str!(start_date, "start_date", empty);
            let end_date = optional_str!(end_date, "end_date", empty);
            let strike = optional_str!(strike, "strike", empty);
            let right = optional_str!(right, "right", empty);

            // SAFETY: client is a non-null pointer returned by tdx_client_connect and not yet freed.
            let client = unsafe { &*client };
            let result = runtime().block_on(async {
                client
                    .inner
                    .option_history_trade_quote_with_fallback(
                        &symbol,
                        &expiration,
                        &start_date,
                        end_date.as_deref(),
                        strike.as_deref(),
                        right.as_deref(),
                    )
                    .await
            });
            match result {
                Ok(ticks) => match TdxTradeQuoteTickArray::from_vec(ticks) {
                    Ok(arr) => arr,
                    Err(e) => {
                        set_error(&format!("interior NUL in server string: {e}"));
                        empty
                    }
                },
                Err(err) => {
                    set_error_from(&err);
                    empty
                }
            }
        }
    )
}

/// Fetch implied-volatility history per the configured
/// [`config::FallbackPolicy`]. See
/// [`thetadatadx::mdds::MddsClient::option_history_greeks_implied_volatility_with_fallback`].
///
/// `symbol`, `expiration`, `start_date` are required. `end_date`,
/// `strike`, `right`, `interval` may be NULL to omit.
#[no_mangle]
#[allow(clippy::too_many_arguments)]
pub unsafe extern "C" fn tdx_option_history_greeks_implied_volatility_with_fallback(
    client: *const TdxClient,
    symbol: *const c_char,
    expiration: *const c_char,
    start_date: *const c_char,
    end_date: *const c_char,
    strike: *const c_char,
    right: *const c_char,
    interval: *const c_char,
) -> TdxIvTickArray {
    ffi_boundary!(
        TdxIvTickArray {
            data: ptr::null(),
            len: 0,
        },
        {
            let empty = TdxIvTickArray {
                data: ptr::null(),
                len: 0,
            };
            if client.is_null() {
                set_error("client handle is null");
                return empty;
            }
            let symbol = require_str!(symbol, "symbol", empty);
            let expiration = require_str!(expiration, "expiration", empty);
            let start_date = require_str!(start_date, "start_date", empty);
            let end_date = optional_str!(end_date, "end_date", empty);
            let strike = optional_str!(strike, "strike", empty);
            let right = optional_str!(right, "right", empty);
            let interval = optional_str!(interval, "interval", empty);

            // SAFETY: client is a non-null pointer returned by tdx_client_connect and not yet freed.
            let client = unsafe { &*client };
            let result = runtime().block_on(async {
                client
                    .inner
                    .option_history_greeks_implied_volatility_with_fallback(
                        &symbol,
                        &expiration,
                        &start_date,
                        end_date.as_deref(),
                        strike.as_deref(),
                        right.as_deref(),
                        interval.as_deref(),
                    )
                    .await
            });
            match result {
                Ok(ticks) => match TdxIvTickArray::from_vec(ticks) {
                    Ok(arr) => arr,
                    Err(e) => {
                        set_error(&format!("interior NUL in server string: {e}"));
                        empty
                    }
                },
                Err(err) => {
                    set_error_from(&err);
                    empty
                }
            }
        }
    )
}

/// Fetch first-order Greeks history per the configured
/// [`config::FallbackPolicy`]. See
/// [`thetadatadx::mdds::MddsClient::option_history_greeks_first_order_with_fallback`].
///
/// `symbol`, `expiration`, `start_date` are required. `end_date`,
/// `strike`, `right`, `interval` may be NULL to omit.
#[no_mangle]
#[allow(clippy::too_many_arguments)]
pub unsafe extern "C" fn tdx_option_history_greeks_first_order_with_fallback(
    client: *const TdxClient,
    symbol: *const c_char,
    expiration: *const c_char,
    start_date: *const c_char,
    end_date: *const c_char,
    strike: *const c_char,
    right: *const c_char,
    interval: *const c_char,
) -> TdxGreeksFirstOrderTickArray {
    ffi_boundary!(
        TdxGreeksFirstOrderTickArray {
            data: ptr::null(),
            len: 0,
        },
        {
            let empty = TdxGreeksFirstOrderTickArray {
                data: ptr::null(),
                len: 0,
            };
            if client.is_null() {
                set_error("client handle is null");
                return empty;
            }
            let symbol = require_str!(symbol, "symbol", empty);
            let expiration = require_str!(expiration, "expiration", empty);
            let start_date = require_str!(start_date, "start_date", empty);
            let end_date = optional_str!(end_date, "end_date", empty);
            let strike = optional_str!(strike, "strike", empty);
            let right = optional_str!(right, "right", empty);
            let interval = optional_str!(interval, "interval", empty);

            // SAFETY: client is a non-null pointer returned by tdx_client_connect and not yet freed.
            let client = unsafe { &*client };
            let result = runtime().block_on(async {
                client
                    .inner
                    .option_history_greeks_first_order_with_fallback(
                        &symbol,
                        &expiration,
                        &start_date,
                        end_date.as_deref(),
                        strike.as_deref(),
                        right.as_deref(),
                        interval.as_deref(),
                    )
                    .await
            });
            match result {
                Ok(ticks) => match TdxGreeksFirstOrderTickArray::from_vec(ticks) {
                    Ok(arr) => arr,
                    Err(e) => {
                        set_error(&format!("interior NUL in server string: {e}"));
                        empty
                    }
                },
                Err(err) => {
                    set_error_from(&err);
                    empty
                }
            }
        }
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::CString;

    #[test]
    fn disabled_factory_round_trips() {
        let policy = tdx_fallback_policy_disabled();
        assert!(!policy.is_null());
        // SAFETY: policy was just allocated above.
        let inner = unsafe { &(*policy).inner };
        assert!(matches!(inner, config::FallbackPolicy::Disabled));
        // SAFETY: policy was just allocated above.
        unsafe { tdx_fallback_policy_free(policy) };
    }

    #[test]
    fn rest_always_carries_base_url() {
        let url = CString::new("http://127.0.0.1:25503").unwrap();
        // SAFETY: url is a valid NUL-terminated C string.
        let policy = unsafe { tdx_fallback_policy_rest_always(url.as_ptr()) };
        assert!(!policy.is_null());
        // SAFETY: policy was just allocated above.
        let inner = unsafe { &(*policy).inner };
        match inner {
            config::FallbackPolicy::RestAlways { base_url } => {
                assert_eq!(base_url, "http://127.0.0.1:25503");
            }
            other => panic!("expected RestAlways, got {other:?}"),
        }
        // SAFETY: policy was just allocated above.
        unsafe { tdx_fallback_policy_free(policy) };
    }

    #[test]
    fn null_base_url_returns_null() {
        // SAFETY: deliberately pass null to exercise the error path.
        let policy = unsafe { tdx_fallback_policy_rest_always(ptr::null()) };
        assert!(policy.is_null());
    }

    #[test]
    fn free_handles_null() {
        // SAFETY: tdx_fallback_policy_free explicitly accepts null.
        unsafe { tdx_fallback_policy_free(ptr::null_mut()) };
    }

    #[test]
    fn config_with_rest_fallback_installs_policy() {
        let config = crate::auth::tdx_config_production();
        let url = CString::new("http://127.0.0.1:25503").unwrap();
        // SAFETY: url is a valid NUL-terminated C string.
        let policy = unsafe { tdx_fallback_policy_rest_always(url.as_ptr()) };
        // SAFETY: config + policy are non-null pointers freshly allocated above.
        let rc = unsafe { tdx_config_with_rest_fallback(config, policy) };
        assert_eq!(rc, 0);
        // SAFETY: config is the pointer just returned by tdx_config_production.
        let cfg_ref = unsafe { &*config };
        assert!(matches!(
            cfg_ref.inner.fallback,
            config::FallbackPolicy::RestAlways { .. }
        ));
        // SAFETY: both handles were just allocated above.
        unsafe { tdx_fallback_policy_free(policy) };
        // SAFETY: config came from tdx_config_production.
        unsafe { crate::auth::tdx_config_free(config) };
    }

    #[test]
    fn config_with_rest_fallback_rejects_null_config() {
        let url = CString::new("http://127.0.0.1:25503").unwrap();
        // SAFETY: url is a valid NUL-terminated C string.
        let policy = unsafe { tdx_fallback_policy_rest_always(url.as_ptr()) };
        // SAFETY: deliberately pass null config to exercise the error path.
        let rc = unsafe { tdx_config_with_rest_fallback(ptr::null_mut(), policy) };
        assert_eq!(rc, -1);
        // SAFETY: policy was just allocated above.
        unsafe { tdx_fallback_policy_free(policy) };
    }
}
