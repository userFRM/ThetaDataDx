//! Credentials, config, and market-data-client lifecycle: `thetadatadx_credentials_*`,
//! `thetadatadx_config_*`, `thetadatadx_market_data_connect` / `thetadatadx_market_data_free`.

use std::os::raw::c_char;
use std::ptr;

use crate::error::{cstr_to_str, set_error, set_error_from};
use crate::types::{ThetaDataDxConfig, ThetaDataDxCredentials, ThetaDataDxMarketDataClient};

// ── Credentials ──

/// Create credentials from email and password strings.
///
/// Returns null on invalid input (check `thetadatadx_last_error()`).
#[no_mangle]
pub unsafe extern "C" fn thetadatadx_credentials_from_email(
    email: *const c_char,
    password: *const c_char,
) -> *mut ThetaDataDxCredentials {
    ffi_boundary!(ptr::null_mut(), {
        let email = require_cstr!(email, ptr::null_mut());
        let password = require_cstr!(password, ptr::null_mut());
        let creds = thetadatadx::Credentials::new(email, password);
        Box::into_raw(Box::new(ThetaDataDxCredentials { inner: creds }))
    })
}

/// Create credentials that authenticate with an API key.
///
/// The API key is an alternative to email + password. It is trimmed and
/// held as secret material on the resulting handle.
///
/// Returns null on invalid input (check `thetadatadx_last_error()`).
#[no_mangle]
pub unsafe extern "C" fn thetadatadx_credentials_from_api_key(
    api_key: *const c_char,
) -> *mut ThetaDataDxCredentials {
    ffi_boundary!(ptr::null_mut(), {
        let api_key = require_cstr!(api_key, ptr::null_mut());
        let creds = thetadatadx::Credentials::api_key(api_key);
        Box::into_raw(Box::new(ThetaDataDxCredentials { inner: creds }))
    })
}

/// Create credentials that authenticate with an API key paired with an
/// account email.
///
/// The email is lowercased and trimmed; an empty email is dropped. The
/// API key is trimmed and held as secret material on the resulting
/// handle.
///
/// Returns null on invalid input (check `thetadatadx_last_error()`).
#[no_mangle]
pub unsafe extern "C" fn thetadatadx_credentials_from_api_key_with_email(
    email: *const c_char,
    api_key: *const c_char,
) -> *mut ThetaDataDxCredentials {
    ffi_boundary!(ptr::null_mut(), {
        let email = require_cstr!(email, ptr::null_mut());
        let api_key = require_cstr!(api_key, ptr::null_mut());
        let creds = thetadatadx::Credentials::api_key_with_email(email, api_key);
        Box::into_raw(Box::new(ThetaDataDxCredentials { inner: creds }))
    })
}

/// Load credentials from a file (line 1 = email, line 2 = password).
///
/// Returns null on error (check `thetadatadx_last_error()`).
#[no_mangle]
pub unsafe extern "C" fn thetadatadx_credentials_from_file(
    path: *const c_char,
) -> *mut ThetaDataDxCredentials {
    ffi_boundary!(ptr::null_mut(), {
        let path = require_cstr!(path, ptr::null_mut());
        match thetadatadx::Credentials::from_file(path) {
            Ok(creds) => Box::into_raw(Box::new(ThetaDataDxCredentials { inner: creds })),
            Err(e) => {
                set_error_from(&e);
                ptr::null_mut()
            }
        }
    })
}

/// Source credentials strictly from the `THETADATA_API_KEY` environment
/// variable.
///
/// Strict: an unset or whitespace-only value is an error rather than a
/// silent fallback, and there is no `creds.txt` file fallback. This is
/// the C-ABI equivalent of the Rust / Python / TypeScript strict
/// env-only resolver; use `thetadatadx_credentials_from_env_or_file`
/// when a file fallback is wanted instead.
///
/// Returns null on error (check `thetadatadx_last_error()`).
#[no_mangle]
pub extern "C" fn thetadatadx_credentials_from_env() -> *mut ThetaDataDxCredentials {
    ffi_boundary!(ptr::null_mut(), {
        match thetadatadx::Credentials::from_env() {
            Ok(creds) => Box::into_raw(Box::new(ThetaDataDxCredentials { inner: creds })),
            Err(e) => {
                set_error_from(&e);
                ptr::null_mut()
            }
        }
    })
}

/// Source credentials from the environment, falling back to a file.
///
/// When `THETADATA_API_KEY` is set and non-empty an API key is used;
/// otherwise the two-line file (line 1 = email, line 2 = password) at
/// `path` is read.
///
/// Returns null on error (check `thetadatadx_last_error()`).
#[no_mangle]
pub unsafe extern "C" fn thetadatadx_credentials_from_env_or_file(
    path: *const c_char,
) -> *mut ThetaDataDxCredentials {
    ffi_boundary!(ptr::null_mut(), {
        let path = require_cstr!(path, ptr::null_mut());
        match thetadatadx::Credentials::from_env_or_file(path) {
            Ok(creds) => Box::into_raw(Box::new(ThetaDataDxCredentials { inner: creds })),
            Err(e) => {
                set_error_from(&e);
                ptr::null_mut()
            }
        }
    })
}

/// Source credentials from a `.env`-format file.
///
/// The file uses the common `.env` grammar (one `KEY=VALUE` per line,
/// optional `export` prefix, `#` comment lines, optional matching
/// quotes). When `THETADATA_API_KEY` is present and non-empty an API key
/// is used; otherwise a complete `THETADATA_EMAIL` + `THETADATA_PASSWORD`
/// pair builds email + password credentials.
///
/// Returns null on error (check `thetadatadx_last_error()`).
#[no_mangle]
pub unsafe extern "C" fn thetadatadx_credentials_from_dotenv(
    path: *const c_char,
) -> *mut ThetaDataDxCredentials {
    ffi_boundary!(ptr::null_mut(), {
        let path = require_cstr!(path, ptr::null_mut());
        match thetadatadx::Credentials::from_dotenv(path) {
            Ok(creds) => Box::into_raw(Box::new(ThetaDataDxCredentials { inner: creds })),
            Err(e) => {
                set_error_from(&e);
                ptr::null_mut()
            }
        }
    })
}

/// Free a credentials handle.
#[no_mangle]
pub unsafe extern "C" fn thetadatadx_credentials_free(creds: *mut ThetaDataDxCredentials) {
    ffi_boundary!((), {
        if !creds.is_null() {
            // SAFETY: the pointer was returned by Box::into_raw / thetadatadx_*_new and has not been freed; ownership returns to Rust.
            drop(unsafe { Box::from_raw(creds) });
        }
    })
}

// ── Config ──

/// Create a production config (`ThetaData` NJ datacenter).
#[no_mangle]
pub extern "C" fn thetadatadx_config_production() -> *mut ThetaDataDxConfig {
    ffi_boundary!(ptr::null_mut(), {
        Box::into_raw(Box::new(ThetaDataDxConfig {
            inner: thetadatadx::DirectConfig::production(),
        }))
    })
}

/// Create a dev config (streaming dev servers, port 20200, infinite replay).
#[no_mangle]
pub extern "C" fn thetadatadx_config_dev() -> *mut ThetaDataDxConfig {
    ffi_boundary!(ptr::null_mut(), {
        Box::into_raw(Box::new(ThetaDataDxConfig {
            inner: thetadatadx::DirectConfig::dev(),
        }))
    })
}

/// Create a market-data-staging config (market-data staging cluster + auth marker;
/// streaming stays on production). Unstable.
#[no_mangle]
pub extern "C" fn thetadatadx_config_stage() -> *mut ThetaDataDxConfig {
    ffi_boundary!(ptr::null_mut(), {
        Box::into_raw(Box::new(ThetaDataDxConfig {
            inner: thetadatadx::DirectConfig::stage(),
        }))
    })
}

/// Select the market-data environment on a config handle in place.
///
/// `kind` is `0` for production or `1` for staging. The market-data and
/// streaming channels are selected independently, so this leaves the
/// streaming channel untouched. Returns `0` on success. Returns `-1` with
/// `thetadatadx_last_error` set when `config` is null or when `kind` is
/// outside the documented `{0, 1}` set.
#[no_mangle]
pub unsafe extern "C" fn thetadatadx_config_with_market_data_environment(
    config: *mut ThetaDataDxConfig,
    kind: i32,
) -> i32 {
    ffi_boundary!(-1, {
        if config.is_null() {
            set_error("config handle is null");
            return -1;
        }
        let environment = match kind {
            0 => thetadatadx::MarketDataEnvironment::Prod,
            1 => thetadatadx::MarketDataEnvironment::Stage,
            other => {
                set_error(&format!(
                    "market-data environment selector must be 0 (PROD) or 1 (STAGE); got {other}"
                ));
                return -1;
            }
        };
        // SAFETY: config is a non-null `*mut ThetaDataDxConfig` returned by `thetadatadx_config_*` and not yet freed; `&mut *` produces an exclusive reference valid for the call duration.
        let config = unsafe { &mut *config };
        // Run the consuming builder on a CLONE and store back only on success.
        // `with_market_data_environment` ends in `validate().expect(...)`, which
        // panics if a previously raw-set tuning knob is out of range; the
        // `ffi_boundary!` wrapper catches that and returns -1. The assignment
        // below only happens if the right-hand side completed without panicking,
        // so a panic leaves the original `config.inner` (custom hosts / tuning)
        // intact instead of wiping it to `Default`. (Taking the inner out first
        // and only restoring on success was the wiping bug.)
        config.inner = config
            .inner
            .clone()
            .with_market_data_environment(environment);
        0
    })
}

/// Select the streaming environment on a config handle in place.
///
/// `kind` is `0` for production or `1` for dev. The streaming and
/// market-data channels are selected independently, so this leaves the
/// market-data channel and the auth marker untouched. Returns `0` on
/// success. Returns `-1` with `thetadatadx_last_error` set when `config`
/// is null or when `kind` is outside the documented `{0, 1}` set.
#[no_mangle]
pub unsafe extern "C" fn thetadatadx_config_with_streaming_environment(
    config: *mut ThetaDataDxConfig,
    kind: i32,
) -> i32 {
    ffi_boundary!(-1, {
        if config.is_null() {
            set_error("config handle is null");
            return -1;
        }
        let environment = match kind {
            0 => thetadatadx::StreamingEnvironment::Prod,
            1 => thetadatadx::StreamingEnvironment::Dev,
            other => {
                set_error(&format!(
                    "streaming environment selector must be 0 (PROD) or 1 (DEV); got {other}"
                ));
                return -1;
            }
        };
        // SAFETY: config is a non-null `*mut ThetaDataDxConfig` returned by `thetadatadx_config_*` and not yet freed; `&mut *` produces an exclusive reference valid for the call duration.
        let config = unsafe { &mut *config };
        // Run the consuming builder on a CLONE and store back only on success
        // (see `thetadatadx_config_with_market_data_environment` for the
        // wiping-bug rationale): `with_streaming_environment` panics on an
        // out-of-range tuning knob via `validate().expect(...)`, and assigning
        // only after the RHS completes leaves the original config intact on the
        // caught-panic path rather than resetting it to `Default`.
        config.inner = config.inner.clone().with_streaming_environment(environment);
        0
    })
}

/// Source a config handle from a `.env`-format file.
///
/// Starts from the production configuration and applies the cluster keys
/// carried by the file: `THETADATA_MARKET_DATA_TYPE` (`PROD` / `STAGE`,
/// case-insensitive) selects the environment, and the optional
/// `THETADATA_MARKET_DATA_HOST` / `THETADATA_STREAMING_HOST` keys override the
/// hosts (an explicit host wins over the environment default). This is the
/// same file format and the same keys `thetadatadx_credentials_from_dotenv`
/// reads, so one `.env` can carry both `THETADATA_API_KEY` and
/// `THETADATA_MARKET_DATA_TYPE`.
///
/// Returns null on error (check `thetadatadx_last_error()`).
#[no_mangle]
pub unsafe extern "C" fn thetadatadx_config_from_dotenv(
    path: *const c_char,
) -> *mut ThetaDataDxConfig {
    ffi_boundary!(ptr::null_mut(), {
        let path = require_cstr!(path, ptr::null_mut());
        match thetadatadx::DirectConfig::from_dotenv(path) {
            Ok(config) => Box::into_raw(Box::new(ThetaDataDxConfig { inner: config })),
            Err(e) => {
                set_error_from(&e);
                ptr::null_mut()
            }
        }
    })
}

/// Free a config handle.
#[no_mangle]
pub unsafe extern "C" fn thetadatadx_config_free(config: *mut ThetaDataDxConfig) {
    ffi_boundary!((), {
        if !config.is_null() {
            // SAFETY: the pointer was returned by Box::into_raw / thetadatadx_*_new and has not been freed; ownership returns to Rust.
            drop(unsafe { Box::from_raw(config) });
        }
    })
}

/// Read the market-data environment carried by the config.
///
/// On success, returns a heap-owned NUL-terminated C string (`"PROD"` or
/// `"STAGE"`) the caller MUST release with `thetadatadx_string_free`. The
/// market-data and streaming environments are selected independently: the
/// `production` / `stage` / `dev` presets (and the `THETADATA_MARKET_DATA_TYPE`
/// dotenv key) set the market-data channel, and this is the readback of
/// that selection. Returns null if `config` is null (the diagnostic is
/// written to `thetadatadx_last_error()`).
#[no_mangle]
pub unsafe extern "C" fn thetadatadx_config_get_market_data_environment(
    config: *const ThetaDataDxConfig,
) -> *mut c_char {
    ffi_boundary!(ptr::null_mut(), {
        if config.is_null() {
            set_error("config handle is null");
            return ptr::null_mut();
        }
        // SAFETY: see `thetadatadx_config_get_streaming_ping_interval_ms`.
        let config = unsafe { &*config };
        // `MarketDataEnvironment::as_str` is a `'static` label free of
        // interior NULs, so `CString::new` never fails here.
        match std::ffi::CString::new(config.inner.market_data_environment().as_str()) {
            Ok(c) => c.into_raw(),
            Err(e) => {
                set_error(&format!(
                    "market-data environment label contains an interior NUL: {e}"
                ));
                ptr::null_mut()
            }
        }
    })
}

/// Read the streaming environment carried by the config.
///
/// On success, returns a heap-owned NUL-terminated C string (`"PROD"` or
/// `"DEV"`) the caller MUST release with `thetadatadx_string_free`. The
/// streaming and market-data environments are selected independently: the
/// `production` / `stage` / `dev` presets (and the `THETADATA_STREAMING_TYPE`
/// dotenv key) set the streaming channel, and this is the readback of that
/// selection. Returns null if `config` is null (the diagnostic is written
/// to `thetadatadx_last_error()`).
#[no_mangle]
pub unsafe extern "C" fn thetadatadx_config_get_streaming_environment(
    config: *const ThetaDataDxConfig,
) -> *mut c_char {
    ffi_boundary!(ptr::null_mut(), {
        if config.is_null() {
            set_error("config handle is null");
            return ptr::null_mut();
        }
        // SAFETY: see `thetadatadx_config_get_streaming_ping_interval_ms`.
        let config = unsafe { &*config };
        // `StreamingEnvironment::as_str` is a `'static` label free of
        // interior NULs, so `CString::new` never fails here.
        match std::ffi::CString::new(config.inner.streaming_environment().as_str()) {
            Ok(c) => c.into_raw(),
            Err(e) => {
                set_error(&format!(
                    "streaming environment label contains an interior NUL: {e}"
                ));
                ptr::null_mut()
            }
        }
    })
}

/// Pin the streaming consumer thread to a CPU core, or leave it under
/// the OS scheduler.
///
/// A NEGATIVE `core` (e.g. `-1`, the `THETADATADX_CONSUMER_CPU_UNPINNED`
/// sentinel) means "unpinned" — the default. A non-negative `core` pins
/// the tick-consumer thread to that core for deterministic, low-jitter
/// delivery; an out-of-range or offline core is a best-effort no-op at
/// the affinity layer rather than an error. Returns `0` on success, `-1`
/// (code `THETADATADX_ERR_CONFIG`) on a null handle.
#[no_mangle]
pub unsafe extern "C" fn thetadatadx_config_set_consumer_cpu(
    config: *mut ThetaDataDxConfig,
    core: i64,
) -> i32 {
    ffi_boundary!(-1, {
        if config.is_null() {
            crate::error::set_error_with_code(
                "thetadatadx_config_set_consumer_cpu: config handle is null",
                crate::error::THETADATADX_ERR_CONFIG,
            );
            return -1;
        }
        // SAFETY: see `thetadatadx_config_set_streaming_ping_interval_ms`.
        let config = unsafe { &mut *config };
        config.inner.streaming.consumer_cpu = usize::try_from(core).ok();
        0
    })
}

/// Read the streaming consumer-thread CPU pin. Writes the pinned core
/// into `*out_core`, or `THETADATADX_CONSUMER_CPU_UNPINNED` (`-1`) when
/// unpinned. Returns `0` on success, `-1` if either pointer is null.
#[no_mangle]
pub unsafe extern "C" fn thetadatadx_config_get_consumer_cpu(
    config: *const ThetaDataDxConfig,
    out_core: *mut i64,
) -> i32 {
    ffi_boundary!(-1, {
        if config.is_null() || out_core.is_null() {
            set_error("config or out-parameter pointer is null");
            return -1;
        }
        // SAFETY: see `thetadatadx_config_get_streaming_ping_interval_ms`.
        let config = unsafe { &*config };
        let value = config
            .inner
            .streaming
            .consumer_cpu
            .and_then(|c| i64::try_from(c).ok())
            .unwrap_or(-1);
        // SAFETY: out pointer checked non-null above.
        unsafe {
            *out_core = value;
        }
        0
    })
}

/// Set streaming reconnect policy on a config handle.
///
/// - `policy = 0`: Auto (default) -- auto-reconnect with split per-class
///   attempt budgets (see `thetadatadx_config_set_reconnect_max_attempts`,
///   `thetadatadx_config_set_reconnect_max_rate_limited_attempts`,
///   `thetadatadx_config_set_reconnect_stable_window_secs`).
/// - `policy = 1`: Manual -- no auto-reconnect, user calls reconnect explicitly
///
/// Returns `0` on success. Returns `-1` and sets `thetadatadx_last_error` /
/// `thetadatadx_last_error_code = THETADATADX_ERR_INVALID_PARAMETER` when `policy` is
/// outside the documented `{0, 1}` set, so an unknown policy is rejected
/// with the same typed class the Python / TypeScript bindings raise
/// rather than being silently coerced to `Auto`. A null `config` is
/// rejected with `THETADATADX_ERR_CONFIG`.
#[no_mangle]
pub unsafe extern "C" fn thetadatadx_config_set_reconnect_policy(
    config: *mut ThetaDataDxConfig,
    policy: i32,
) -> i32 {
    ffi_boundary!(-1, {
        if config.is_null() {
            crate::error::set_error_with_code(
                "thetadatadx_config_set_reconnect_policy: config handle is null",
                crate::error::THETADATADX_ERR_CONFIG,
            );
            return -1;
        }
        let value = match policy {
            0 => thetadatadx::ReconnectPolicy::Auto(thetadatadx::ReconnectAttemptLimits::default()),
            1 => thetadatadx::ReconnectPolicy::Manual,
            other => {
                crate::error::set_error_with_code(
                    &format!(
                        "thetadatadx_config_set_reconnect_policy: invalid policy {other}; expected 0 (Auto) or 1 (Manual)"
                    ),
                    crate::error::THETADATADX_ERR_INVALID_PARAMETER,
                );
                return -1;
            }
        };
        // SAFETY: caller passes a pointer returned by `thetadatadx_direct_config_new`
        // that has not been freed; null was rejected above; `&mut *` produces a
        // unique reference valid for the call duration because the caller owns
        // the Box and the FFI contract forbids concurrent calls on the same
        // handle.
        let config = unsafe { &mut *config };
        config.inner.reconnect.policy = value;
        0
    })
}

// ── Reconnect policy selector readback ─────────────────────────────
//
// The policy-variant selector. The per-class `Auto` attempt budgets are
// the generated `policy_limit` accessors (config_surface.toml); this
// reads the variant discriminant itself, which stays hand-written.

/// Read the configured reconnect policy selector.
///
/// Writes `0` (`Auto`), `1` (`Manual`), or `2` (`Custom`) into
/// `*out_policy`. Returns `0` on success, `-1` if either pointer is
/// null.
#[no_mangle]
pub unsafe extern "C" fn thetadatadx_config_get_reconnect_policy(
    config: *const ThetaDataDxConfig,
    out_policy: *mut i32,
) -> i32 {
    ffi_boundary!(-1, {
        if config.is_null() || out_policy.is_null() {
            set_error("config or out-parameter pointer is null");
            return -1;
        }
        // SAFETY: see `thetadatadx_config_get_streaming_ping_interval_ms`.
        let config = unsafe { &*config };
        let value = match &config.inner.reconnect.policy {
            thetadatadx::ReconnectPolicy::Auto(_) => 0,
            thetadatadx::ReconnectPolicy::Manual => 1,
            _ => 2,
        };
        // SAFETY: out pointer checked non-null above; the FFI contract pins the storage for the call duration and forbids concurrent calls on the same handle.
        unsafe {
            *out_policy = value;
        }
        0
    })
}

// ── Streaming transport knobs ────────────────────────────────────────────
//
// Scalar tuning on `StreamingConfig` exposed for embedded callers: read
// timeout, connect timeout, ping cadence, ring size, the I/O read
// slice, and the TCP keepalive schedule.
// Out-of-range values are rejected at connect time by the core
// validator; the setters here store verbatim so the rejection carries
// the canonical bounds message.

/// Set the streaming event ring buffer size (slots).
///
/// Must be a power of two `>= 64`. Invalid values are rejected at the
/// setter boundary: the config is left unchanged and the failure
/// reason is written to thread-local storage retrievable via
/// `thetadatadx_last_error()`. Default is `131_072`.
#[no_mangle]
pub unsafe extern "C" fn thetadatadx_config_set_streaming_ring_size(
    config: *mut ThetaDataDxConfig,
    n: usize,
) {
    ffi_boundary!((), {
        if config.is_null() {
            return;
        }
        // Same validation as the Rust core's `check_ring_size` —
        // surface the rejection here so the FFI caller sees it at the
        // setter rather than at connect.
        if n == 0 || !n.is_power_of_two() {
            crate::error::set_error_with_code(
                &format!("streaming_ring_size must be a power of two >= 64; got {n}"),
                crate::error::THETADATADX_ERR_INVALID_PARAMETER,
            );
            return;
        }
        if n < 64 {
            crate::error::set_error_with_code(
                &format!("streaming_ring_size must be >= 64; got {n}"),
                crate::error::THETADATADX_ERR_INVALID_PARAMETER,
            );
            return;
        }
        // SAFETY: config is a non-null pointer returned by thetadatadx_config_* and not yet freed.
        let config = unsafe { &mut *config };
        config.inner.streaming.ring_size = n;
    })
}

// ── Custom reconnect policy callback ────────────────────────────────

/// Reconnect-decision callback type for
/// `thetadatadx_config_set_reconnect_callback`.
///
/// Invoked on the streaming I/O thread after each retriable
/// involuntary disconnect. `reason` is the `RemoveReason` discriminant
/// as `i32`; `attempt` is the 1-based consecutive-reconnect counter.
/// Return the reconnect delay in milliseconds, or any negative value
/// to stop reconnecting (the I/O loop then emits the terminal
/// `ReconnectsExhausted` event and exits).
///
/// The callback runs under the C ABI and must not unwind across the
/// boundary. A C++ `throw` or a C `longjmp` that escapes the callback into
/// the calling Rust frame is undefined behavior. The I/O loop wraps each
/// invocation in [`std::panic::catch_unwind`], but that contains only a Rust
/// panic raised on our side of the boundary, not a foreign exception out of
/// the callback. Catch and handle every exception inside the callback before
/// returning a decision.
pub type ThetaDataDxReconnectCallback =
    unsafe extern "C" fn(reason: i32, attempt: u32, user_data: *mut std::ffi::c_void) -> i64;

/// Install a custom reconnect policy driven by a C callback.
///
/// Permanent disconnect reasons (invalid credentials, account
/// conflicts) never reach the callback — they stop the I/O loop
/// before any policy is consulted, so the callback cannot turn a
/// credential rejection into a retry loop.
///
/// # Thread-safety contract
///
/// The callback runs on the SDK's streaming I/O thread, not on the
/// thread that registered it. `cb` and `user_data` must therefore be
/// safe to use from another thread for as long as any client built
/// from this config is alive. Passing `cb = NULL` restores the
/// default `Auto` reconnect policy.
///
/// Returns `0` on success, `-1` if `config` is null.
#[no_mangle]
pub unsafe extern "C" fn thetadatadx_config_set_reconnect_callback(
    config: *mut ThetaDataDxConfig,
    cb: Option<ThetaDataDxReconnectCallback>,
    user_data: *mut std::ffi::c_void,
) -> i32 {
    ffi_boundary!(-1, {
        if config.is_null() {
            set_error("config handle is null");
            return -1;
        }
        // SAFETY: config is a non-null pointer returned by `thetadatadx_config_*` and not yet freed; `&mut *` produces a unique reference valid for the call duration because the caller owns the Box and the FFI contract forbids concurrent calls on the same handle.
        let config = unsafe { &mut *config };
        let Some(cb) = cb else {
            config.inner.reconnect.policy =
                thetadatadx::ReconnectPolicy::Auto(thetadatadx::ReconnectAttemptLimits::default());
            return 0;
        };
        // The raw context pointer travels into the closure. The
        // documented contract requires the callee side to be
        // thread-safe; the wrapper below carries that promise across
        // Rust's auto-trait checks.
        struct CallbackCtx {
            cb: ThetaDataDxReconnectCallback,
            user_data: *mut std::ffi::c_void,
        }
        // SAFETY: the public contract on `thetadatadx_config_set_reconnect_callback` requires `cb` + `user_data` to be callable from any thread for the lifetime of clients built from this config; the wrapper only forwards the pointer pair to that documented-thread-safe callback.
        unsafe impl Send for CallbackCtx {}
        // SAFETY: same documented contract as the `Send` impl — the wrapped pointer pair is only ever used to invoke the caller-supplied thread-safe callback.
        unsafe impl Sync for CallbackCtx {}
        impl CallbackCtx {
            fn invoke(&self, reason: i32, attempt: u32) -> i64 {
                // The decision callback runs on the streaming I/O thread,
                // not on a `ffi_boundary!`-guarded entry point, so a Rust
                // panic raised on this path would otherwise unwind across the
                // C ABI on a foreign thread. Wrap the invocation in
                // `catch_unwind` and fall back to the stop decision (`-1`),
                // the same defence the stream dispatcher applies per
                // invocation, so a panic from our own Rust code ends the
                // reconnect loop instead of aborting the process. This does
                // not contain a foreign exception thrown out of the callback;
                // that no-unwind contract is documented on
                // `ThetaDataDxReconnectCallback`.
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    // SAFETY: `self.cb` is the caller-registered function pointer and `self.user_data` the matching context; the registration contract guarantees both stay valid and thread-safe while any client built from the config is alive.
                    unsafe { (self.cb)(reason, attempt, self.user_data) }
                }));
                match result {
                    Ok(delay_ms) => delay_ms,
                    Err(_) => {
                        tracing::error!(
                            target: "thetadatadx::ffi",
                            "reconnect decision callback panicked; stopping reconnect attempts",
                        );
                        -1
                    }
                }
            }
        }
        let ctx = CallbackCtx { cb, user_data };
        config.inner.reconnect.policy =
            thetadatadx::ReconnectPolicy::Custom(std::sync::Arc::new(move |reason, attempt| {
                // Method call through `&ctx` captures the whole
                // wrapper struct (the carrier of the Send + Sync
                // promise), not its raw-pointer fields individually.
                let delay_ms = ctx.invoke(reason as i32, attempt);
                if delay_ms < 0 {
                    None
                } else {
                    Some(std::time::Duration::from_millis(delay_ms as u64))
                }
            }));
        0
    })
}

/// Set the async worker-thread count using the `(has_value, n)`
/// widened ABI shape that preserves the `Some(0)` sentinel across the C
/// boundary.
///
/// * `has_value = false` → `None` (default sizing, one worker per
///   logical CPU). `n` is ignored.
/// * `has_value = true` → `Some(n)`, clamping `0` to `1` so at least one
///   worker is started.
///
/// The async worker pool is process-global: it is built once, from the
/// `config` of the first client connected in the process. This setting is
/// therefore honoured when the first client in the process is created;
/// clients connected later share the already-built pool, so changing it
/// on a subsequent `config` has no effect.
///
/// Returns `0` on success, `-1` if `config` is null.
#[no_mangle]
pub unsafe extern "C" fn thetadatadx_config_set_worker_threads(
    config: *mut ThetaDataDxConfig,
    has_value: bool,
    n: usize,
) -> i32 {
    ffi_boundary!(-1, {
        if config.is_null() {
            set_error("config handle is null");
            return -1;
        }
        // SAFETY: config is a non-null pointer returned by
        // thetadatadx_config_production / thetadatadx_config_dev / thetadatadx_config_stage
        // and not yet freed.
        let config = unsafe { &mut *config };
        config.inner.runtime.tokio_worker_threads = if has_value { Some(n) } else { None };
        0
    })
}

/// Read the current async worker-thread count.
/// Widened `(has_value, n)` ABI:
///
/// * `*out_has_value = false` → `None` (auto-size). `*out_n` is left as `0`.
/// * `*out_has_value = true` → `Some(*out_n)`.
///
/// Returns `0` on success, `-1` if any pointer is null.
#[no_mangle]
pub unsafe extern "C" fn thetadatadx_config_get_worker_threads(
    config: *const ThetaDataDxConfig,
    out_has_value: *mut bool,
    out_n: *mut usize,
) -> i32 {
    ffi_boundary!(-1, {
        if config.is_null() || out_has_value.is_null() || out_n.is_null() {
            set_error("config or out-parameter pointer is null");
            return -1;
        }
        // SAFETY: see `thetadatadx_config_get_streaming_ping_interval_ms`.
        let config = unsafe { &*config };
        let (has_value, n) = match config.inner.runtime.tokio_worker_threads {
            Some(v) => (true, v),
            None => (false, 0),
        };
        // SAFETY: out_has_value / out_n null-checked above; caller pins the storage they point at for the call duration.
        unsafe {
            *out_has_value = has_value;
            *out_n = n;
        }
        0
    })
}

// `DirectConfig.retry` field accessors (`initial_delay` / `max_delay` ms,
// `max_attempts`, `jitter`) are the generated scalar / `ms` accessors in
// config_surface.toml; `delay_for_attempt` / `capped_backoff` /
// `disabled()` stay Rust-only method-shape helpers.

// `DirectConfig.auth` string fields (`nexus_url` / `client_type`) are the
// generated `string` accessors in config_surface.toml: the setter takes a
// validated `*const c_char`, the getter returns an owned `*mut c_char` the
// caller frees with `thetadatadx_string_free`.

// `DirectConfig` market-data endpoint overrides (`market_data_host` string
// via `set_market_data_host`, `market_data_port` `u16`) are the generated
// `string` / scalar accessors in config_surface.toml.

// The uniform static config accessors — scalar / duration setters and
// getters, plus the `policy_limit` (reconnect `Auto`-limit), `string`,
// `enum` (int code ↔ core variant), and `option` (`(has_value, value)` ↔
// `Option<T>`) carve-outs — are generated from `config_surface.toml`.
// The genuinely divergent accessors above (validated `ring_size`,
// `consumer_cpu` sentinel, the `(has_value, n)` worker-thread setting,
// and the reconnect-policy callback) stay hand-written.
include!("config_accessors.rs");

// ── MarketDataClient ──

/// Connect a market-data client to `ThetaData` servers
/// (authenticates via Nexus API).
///
/// Returns null on connection/auth failure (check `thetadatadx_last_error()`).
#[no_mangle]
pub unsafe extern "C" fn thetadatadx_market_data_connect(
    creds: *const ThetaDataDxCredentials,
    config: *const ThetaDataDxConfig,
) -> *mut ThetaDataDxMarketDataClient {
    ffi_boundary!(ptr::null_mut(), {
        crate::ensure_crypto_provider();
        if creds.is_null() {
            set_error("credentials handle is null");
            return ptr::null_mut();
        }
        if config.is_null() {
            set_error("config handle is null");
            return ptr::null_mut();
        }
        // SAFETY: creds is a non-null pointer returned by thetadatadx_credentials_from_email / thetadatadx_credentials_from_file and not yet freed.
        let creds = unsafe { &*creds };
        // SAFETY: config is a non-null pointer returned by thetadatadx_direct_config_new and not yet freed.
        let config = unsafe { &*config };
        match crate::runtime_from_config(&config.inner.runtime).block_on(
            thetadatadx::mdds::MarketDataClient::connect(&creds.inner, config.inner.clone()),
        ) {
            Ok(client) => Box::into_raw(Box::new(ThetaDataDxMarketDataClient { inner: client })),
            Err(e) => {
                set_error_from(&e);
                ptr::null_mut()
            }
        }
    })
}

/// Connect a market-data client, loading credentials from a file
/// (line 1 = email, line 2 = password) instead of a credentials handle.
///
/// One-call equivalent of `thetadatadx_credentials_from_file` followed by
/// `thetadatadx_market_data_connect`: the credentials are opened from `path`,
/// consumed for the connect, and freed internally. The returned handle
/// and its ownership / free convention are identical to
/// `thetadatadx_market_data_connect` (free with `thetadatadx_market_data_free`).
///
/// Returns null on argument validation or connection/auth failure
/// (check `thetadatadx_last_error()`).
#[no_mangle]
pub unsafe extern "C" fn thetadatadx_market_data_connect_from_file(
    path: *const c_char,
    config: *const ThetaDataDxConfig,
) -> *mut ThetaDataDxMarketDataClient {
    ffi_boundary!(ptr::null_mut(), {
        // SAFETY: `path` is a NUL-terminated C string valid for the call;
        // `thetadatadx_credentials_from_file` validates non-null + UTF-8 and sets
        // `thetadatadx_last_error()` on failure.
        let creds = unsafe { thetadatadx_credentials_from_file(path) };
        if creds.is_null() {
            return ptr::null_mut();
        }
        // SAFETY: `creds` was just allocated by `thetadatadx_credentials_from_file`
        // and is owned by this function; `thetadatadx_market_data_connect` borrows
        // it and we free it unconditionally below.
        let client = unsafe { thetadatadx_market_data_connect(creds, config) };
        // SAFETY: `creds` is the non-null handle checked above;
        // `thetadatadx_market_data_connect` only borrowed it, so this scope still
        // owns it and frees it exactly once.
        unsafe { thetadatadx_credentials_free(creds) };
        client
    })
}

/// Free a market-data client handle.
#[no_mangle]
pub unsafe extern "C" fn thetadatadx_market_data_free(client: *mut ThetaDataDxMarketDataClient) {
    ffi_boundary!((), {
        if !client.is_null() {
            // SAFETY: the pointer was returned by Box::into_raw / thetadatadx_*_new and has not been freed; ownership returns to Rust.
            drop(unsafe { Box::from_raw(client) });
        }
    })
}

#[cfg(test)]
mod pool_sizing_tests {
    //! Offline tests for the market-data pool-sizing setter.
    //!
    //! Each test allocates a fresh `ThetaDataDxConfig` via `thetadatadx_config_production`,
    //! calls the setter under test, then reads the underlying Rust
    //! `MarketDataConfig` to confirm the value round-tripped.

    #[test]
    fn warn_on_buffered_threshold_bytes_round_trips() {
        let cfg = super::thetadatadx_config_production();
        assert!(!cfg.is_null());
        // SAFETY: handle just returned by thetadatadx_config_production.
        unsafe {
            // Default seeded at 100 MiB by `MarketDataConfig::default()`.
            let mut current: usize = 0;
            assert_eq!(
                super::thetadatadx_config_get_warn_on_buffered_threshold_bytes(cfg, &mut current),
                0
            );
            assert_eq!(current, 100 * 1024 * 1024);
            // Override.
            super::thetadatadx_config_set_warn_on_buffered_threshold_bytes(cfg, 50 * 1024 * 1024);
            assert_eq!(
                (*cfg).inner.market_data.warn_on_buffered_threshold_bytes,
                50 * 1024 * 1024
            );
            assert_eq!(
                super::thetadatadx_config_get_warn_on_buffered_threshold_bytes(cfg, &mut current),
                0
            );
            assert_eq!(current, 50 * 1024 * 1024);
            // Disable.
            super::thetadatadx_config_set_warn_on_buffered_threshold_bytes(cfg, 0);
            assert_eq!((*cfg).inner.market_data.warn_on_buffered_threshold_bytes, 0);
            // Null-pointer guards: setter is a no-op (matches the
            // ffi_boundary `()` return); getter returns -1.
            super::thetadatadx_config_set_warn_on_buffered_threshold_bytes(std::ptr::null_mut(), 4);
            assert_eq!(
                super::thetadatadx_config_get_warn_on_buffered_threshold_bytes(
                    std::ptr::null(),
                    &mut current
                ),
                -1
            );
            super::thetadatadx_config_free(cfg);
        }
    }

    #[test]
    fn market_data_request_timeout_secs_round_trips() {
        let cfg = super::thetadatadx_config_production();
        assert!(!cfg.is_null());
        // SAFETY: handle just returned by thetadatadx_config_production.
        unsafe {
            // Default seeded at 300s by `MarketDataConfig::default()`.
            let mut current: u64 = 0;
            assert_eq!(
                super::thetadatadx_config_get_request_timeout_secs(cfg, &mut current),
                0
            );
            assert_eq!(current, 300);
            // Override.
            super::thetadatadx_config_set_request_timeout_secs(cfg, 45);
            assert_eq!((*cfg).inner.market_data.request_timeout_secs, 45);
            assert_eq!(
                super::thetadatadx_config_get_request_timeout_secs(cfg, &mut current),
                0
            );
            assert_eq!(current, 45);
            // Disable (no default deadline).
            super::thetadatadx_config_set_request_timeout_secs(cfg, 0);
            assert_eq!((*cfg).inner.market_data.request_timeout_secs, 0);
            // Null-pointer guards: setter is a no-op (matches the
            // ffi_boundary `()` return); getter returns -1.
            super::thetadatadx_config_set_request_timeout_secs(std::ptr::null_mut(), 4);
            assert_eq!(
                super::thetadatadx_config_get_request_timeout_secs(std::ptr::null(), &mut current),
                -1
            );
            super::thetadatadx_config_free(cfg);
        }
    }

    #[test]
    fn null_handle_is_safe() {
        // SAFETY: passing null to thetadatadx_config_set_* / thetadatadx_*_free is the
        // documented FFI contract — the call must return without
        // crashing. The test exercises that null-tolerance branch.
        unsafe {
            super::thetadatadx_config_set_request_timeout_secs(std::ptr::null_mut(), 4);
        }
    }
}

#[cfg(test)]
mod reconnect_setter_tests {
    //! Offline tests for the streaming ReconnectConfig setters on the FFI
    //! surface — cross-binding parity with Python / TypeScript / C++.
    //!
    //! Each test allocates a fresh `ThetaDataDxConfig` via
    //! `thetadatadx_config_production`, calls the setter under test, then reads
    //! the underlying `ReconnectConfig` to confirm the value
    //! round-tripped (or that the silent-no-op contract is honoured
    //! under non-Auto policies).
    //!
    //! Failure-class semantics (per-class budget enforcement and the
    //! stable-window timer reset) are exercised by the Rust unit tests
    //! under `fpss::session::tests` and
    //! `fpss::protocol::reconnect_delays_match_policy`; this module
    //! pins only the C-ABI forwarding contract.

    #[test]
    fn reconnect_policy_round_trips_auto_and_manual() {
        let cfg = super::thetadatadx_config_production();
        assert!(!cfg.is_null());
        // SAFETY: handle just returned by thetadatadx_config_production.
        unsafe {
            super::thetadatadx_config_set_reconnect_policy(cfg, 1);
            assert!(matches!(
                (*cfg).inner.reconnect.policy,
                thetadatadx::ReconnectPolicy::Manual
            ));
            super::thetadatadx_config_set_reconnect_policy(cfg, 0);
            assert!(matches!(
                (*cfg).inner.reconnect.policy,
                thetadatadx::ReconnectPolicy::Auto(_)
            ));
            super::thetadatadx_config_free(cfg);
        }
    }

    #[test]
    fn reconnect_policy_unknown_selector_rejected_with_typed_code() {
        // An int selector outside `{0, 1}` is rejected with the typed
        // invalid-parameter class rather than silently coerced to
        // `Auto` — the cross-binding contract the Python ValueError /
        // TypeScript InvalidParameterError already honour. The setter
        // returns `-1`, sets the typed code, and leaves the prior
        // policy untouched.
        let cfg = super::thetadatadx_config_production();
        // SAFETY: handle just returned by thetadatadx_config_production.
        unsafe {
            assert_eq!(super::thetadatadx_config_set_reconnect_policy(cfg, 1), 0);
            crate::error::thetadatadx_clear_error();
            assert_eq!(super::thetadatadx_config_set_reconnect_policy(cfg, 7), -1);
            assert_eq!(
                crate::error::thetadatadx_last_error_code(),
                crate::error::THETADATADX_ERR_INVALID_PARAMETER
            );
            // The rejected call leaves the previously-set Manual policy
            // in place rather than overwriting it with a coerced Auto.
            assert!(matches!(
                (*cfg).inner.reconnect.policy,
                thetadatadx::ReconnectPolicy::Manual
            ));
            super::thetadatadx_config_free(cfg);
        }
    }

    #[test]
    fn reconnect_max_attempts_round_trips_on_auto_policy() {
        let cfg = super::thetadatadx_config_production();
        // SAFETY: handle just returned by thetadatadx_config_production.
        unsafe {
            super::thetadatadx_config_set_reconnect_policy(cfg, 0);
            for n in [0u32, 1, 3, 10, 100, 1000] {
                super::thetadatadx_config_set_reconnect_max_attempts(cfg, n);
                let thetadatadx::ReconnectPolicy::Auto(limits) = &(*cfg).inner.reconnect.policy
                else {
                    panic!("policy must remain Auto across setter calls");
                };
                assert_eq!(limits.max_attempts, n);
            }
            super::thetadatadx_config_free(cfg);
        }
    }

    #[test]
    fn reconnect_max_rate_limited_attempts_round_trips_on_auto_policy() {
        let cfg = super::thetadatadx_config_production();
        // SAFETY: handle just returned by thetadatadx_config_production.
        unsafe {
            super::thetadatadx_config_set_reconnect_policy(cfg, 0);
            for n in [0u32, 1, 10, 100, 1000] {
                super::thetadatadx_config_set_reconnect_max_rate_limited_attempts(cfg, n);
                let thetadatadx::ReconnectPolicy::Auto(limits) = &(*cfg).inner.reconnect.policy
                else {
                    panic!("policy must remain Auto across setter calls");
                };
                assert_eq!(limits.max_rate_limited_attempts, n);
            }
            super::thetadatadx_config_free(cfg);
        }
    }

    #[test]
    fn reconnect_stable_window_secs_round_trips_on_auto_policy() {
        let cfg = super::thetadatadx_config_production();
        // SAFETY: handle just returned by thetadatadx_config_production.
        unsafe {
            super::thetadatadx_config_set_reconnect_policy(cfg, 0);
            for secs in [0u64, 1, 60, 3600, 86_400] {
                super::thetadatadx_config_set_reconnect_stable_window_secs(cfg, secs);
                let thetadatadx::ReconnectPolicy::Auto(limits) = &(*cfg).inner.reconnect.policy
                else {
                    panic!("policy must remain Auto across setter calls");
                };
                assert_eq!(limits.stable_window, std::time::Duration::from_secs(secs));
            }
            super::thetadatadx_config_free(cfg);
        }
    }

    #[test]
    fn per_class_budget_setters_are_silent_noop_on_manual_policy() {
        // Matches the cross-binding contract: per-class budget setters
        // only mutate `ReconnectAttemptLimits` when the policy variant
        // is `Auto`. Under `Manual` the calls are silently absorbed;
        // the underlying policy variant must not transition.
        let cfg = super::thetadatadx_config_production();
        // SAFETY: handle just returned by thetadatadx_config_production.
        unsafe {
            super::thetadatadx_config_set_reconnect_policy(cfg, 1);
            super::thetadatadx_config_set_reconnect_max_attempts(cfg, 5);
            super::thetadatadx_config_set_reconnect_max_rate_limited_attempts(cfg, 50);
            super::thetadatadx_config_set_reconnect_stable_window_secs(cfg, 120);
            assert!(matches!(
                (*cfg).inner.reconnect.policy,
                thetadatadx::ReconnectPolicy::Manual
            ));
            super::thetadatadx_config_free(cfg);
        }
    }

    #[test]
    fn null_handle_is_safe() {
        // SAFETY: passing null to thetadatadx_config_set_* / thetadatadx_*_free is the
        // documented FFI contract — the call must return without
        // crashing. The test exercises that null-tolerance branch.
        unsafe {
            super::thetadatadx_config_set_reconnect_policy(std::ptr::null_mut(), 0);
            super::thetadatadx_config_set_reconnect_max_attempts(std::ptr::null_mut(), 3);
            super::thetadatadx_config_set_reconnect_max_rate_limited_attempts(
                std::ptr::null_mut(),
                100,
            );
            super::thetadatadx_config_set_reconnect_stable_window_secs(std::ptr::null_mut(), 60);
        }
    }

    #[test]
    fn reconnect_setters_compose_with_pool_sizing_setters() {
        // Cross-binding interleaved-survival contract: reconnect setter
        // calls and market-data tuning setter calls on the same
        // `ThetaDataDxConfig` must land in `inner` independently and
        // persist. Mirrors the Python
        // `test_reconnect_setter_state_survives_interleaved_calls`,
        // TypeScript `Pool-sizing setter state survives interleaved
        // reconnect setter calls`, and C++ `Reconnect setters compose
        // with pool-sizing setters` cases.
        let cfg = super::thetadatadx_config_production();
        assert!(!cfg.is_null());
        // SAFETY: handle just returned by thetadatadx_config_production.
        unsafe {
            // Apply a market-data tuning knob.
            super::thetadatadx_config_set_warn_on_buffered_threshold_bytes(cfg, 8 * 1024 * 1024);

            // Apply reconnect knobs.
            super::thetadatadx_config_set_reconnect_policy(cfg, 0);
            super::thetadatadx_config_set_reconnect_max_attempts(cfg, 5);
            super::thetadatadx_config_set_reconnect_max_rate_limited_attempts(cfg, 3);
            super::thetadatadx_config_set_reconnect_stable_window_secs(cfg, 60);

            // Market-data tuning mutations survived the reconnect setter sequence.
            let mdds = &(*cfg).inner.market_data;
            assert_eq!(mdds.warn_on_buffered_threshold_bytes, 8 * 1024 * 1024);

            // Reconnect mutations landed on `Auto(limits)`.
            let thetadatadx::ReconnectPolicy::Auto(limits) = &(*cfg).inner.reconnect.policy else {
                panic!("expected ReconnectPolicy::Auto after set_reconnect_policy(0)");
            };
            assert_eq!(limits.max_attempts, 5);
            assert_eq!(limits.max_rate_limited_attempts, 3);
            assert_eq!(limits.stable_window, std::time::Duration::from_secs(60));

            super::thetadatadx_config_free(cfg);
        }
    }

    #[test]
    fn reconnect_wait_ms_round_trips_via_getter() {
        let cfg = super::thetadatadx_config_production();
        // SAFETY: handle just returned by thetadatadx_config_production.
        unsafe {
            let mut got: u64 = 0;
            // Default seeded from ReconnectConfig::production_defaults().
            assert_eq!(
                super::thetadatadx_config_get_reconnect_wait_ms(cfg, &mut got),
                0
            );
            assert_eq!(got, 250);
            for ms in [0u64, 1, 500, 2_000, 60_000, u64::MAX] {
                super::thetadatadx_config_set_reconnect_wait_ms(cfg, ms);
                assert_eq!((*cfg).inner.reconnect.wait_ms, ms);
                assert_eq!(
                    super::thetadatadx_config_get_reconnect_wait_ms(cfg, &mut got),
                    0
                );
                assert_eq!(got, ms);
            }
            super::thetadatadx_config_free(cfg);
        }
    }

    #[test]
    fn reconnect_wait_rate_limited_ms_round_trips_via_getter() {
        let cfg = super::thetadatadx_config_production();
        // SAFETY: handle just returned by thetadatadx_config_production.
        unsafe {
            let mut got: u64 = 0;
            // Default seeded from ReconnectConfig::production_defaults().
            assert_eq!(
                super::thetadatadx_config_get_reconnect_wait_rate_limited_ms(cfg, &mut got),
                0
            );
            assert_eq!(got, 130_000);
            for ms in [0u64, 1, 30_000, 130_000, 600_000, u64::MAX] {
                super::thetadatadx_config_set_reconnect_wait_rate_limited_ms(cfg, ms);
                assert_eq!((*cfg).inner.reconnect.wait_rate_limited_ms, ms);
                assert_eq!(
                    super::thetadatadx_config_get_reconnect_wait_rate_limited_ms(cfg, &mut got),
                    0
                );
                assert_eq!(got, ms);
            }
            super::thetadatadx_config_free(cfg);
        }
    }

    #[test]
    fn reconnect_wait_ms_null_handle_returns_minus_one() {
        // SAFETY: passing null to thetadatadx_config_* is the documented FFI
        // contract — getter returns sentinel, setter no-ops.
        unsafe {
            let mut got: u64 = 42;
            assert_eq!(
                super::thetadatadx_config_get_reconnect_wait_ms(std::ptr::null(), &mut got),
                -1
            );
            assert_eq!(
                super::thetadatadx_config_get_reconnect_wait_rate_limited_ms(
                    std::ptr::null(),
                    &mut got
                ),
                -1
            );
            super::thetadatadx_config_set_reconnect_wait_ms(std::ptr::null_mut(), 1_234);
            super::thetadatadx_config_set_reconnect_wait_rate_limited_ms(
                std::ptr::null_mut(),
                1_234,
            );
        }
    }
}

#[cfg(test)]
mod runtime_setter_tests {
    //! Offline tests for the async `worker_threads` setter/getter pair
    //! on the FFI surface — cross-binding parity with Python /
    //! TypeScript / C++. The `(has_value, n)` shape preserves `Some(0)`
    //! across the C boundary the same way the decode-pipeline setters
    //! do.

    #[test]
    fn worker_threads_round_trips_via_getter() {
        let cfg = super::thetadatadx_config_production();
        // SAFETY: handle just returned by thetadatadx_config_production.
        unsafe {
            // None sentinel (default).
            let mut got_has = true;
            let mut got_n: usize = 99;
            assert_eq!(
                super::thetadatadx_config_get_worker_threads(cfg, &mut got_has, &mut got_n),
                0
            );
            assert!(!got_has, "default worker_threads must be None");
            assert_eq!(got_n, 0);

            // Explicit values round-trip including the Some(0) sentinel.
            for n in [0usize, 1, 2, 4, 8, 16, 32, 64] {
                let rc = super::thetadatadx_config_set_worker_threads(cfg, true, n);
                assert_eq!(rc, 0);
                assert_eq!((*cfg).inner.runtime.tokio_worker_threads, Some(n));
                assert_eq!(
                    super::thetadatadx_config_get_worker_threads(cfg, &mut got_has, &mut got_n),
                    0
                );
                assert!(got_has);
                assert_eq!(got_n, n);
            }

            // Reset to None.
            let rc = super::thetadatadx_config_set_worker_threads(cfg, false, 999);
            assert_eq!(rc, 0);
            assert_eq!((*cfg).inner.runtime.tokio_worker_threads, None);
            super::thetadatadx_config_free(cfg);
        }
    }

    #[test]
    fn worker_threads_null_handle_returns_minus_one() {
        // SAFETY: passing null to thetadatadx_config_* is the documented FFI
        // contract — getter returns sentinel, setter no-ops.
        unsafe {
            let rc = super::thetadatadx_config_set_worker_threads(std::ptr::null_mut(), true, 4);
            assert_eq!(rc, -1);
            let mut got_has = false;
            let mut got_n: usize = 0;
            assert_eq!(
                super::thetadatadx_config_get_worker_threads(
                    std::ptr::null(),
                    &mut got_has,
                    &mut got_n,
                ),
                -1
            );
        }
    }
}

#[cfg(test)]
mod retry_setter_tests {
    //! Offline tests for the four `RetryPolicy` field setters/getters
    //! on the FFI surface — cross-binding parity with Python /
    //! TypeScript / C++. The `delay_for_attempt` / `capped_backoff`
    //! helpers stay Rust-only; this module pins only field round-trip.

    #[test]
    fn retry_initial_delay_ms_round_trips_via_getter() {
        let cfg = super::thetadatadx_config_production();
        // SAFETY: handle just returned by thetadatadx_config_production.
        unsafe {
            let mut got: u64 = 0;
            // Default seeded by RetryPolicy::default().
            assert_eq!(
                super::thetadatadx_config_get_retry_initial_delay_ms(cfg, &mut got),
                0
            );
            assert_eq!(got, 250);
            for ms in [0u64, 1, 100, 250, 2_000, 60_000] {
                super::thetadatadx_config_set_retry_initial_delay_ms(cfg, ms);
                assert_eq!(
                    super::thetadatadx_config_get_retry_initial_delay_ms(cfg, &mut got),
                    0
                );
                assert_eq!(got, ms);
            }
            super::thetadatadx_config_free(cfg);
        }
    }

    #[test]
    fn retry_max_delay_ms_round_trips_via_getter() {
        let cfg = super::thetadatadx_config_production();
        // SAFETY: handle just returned by thetadatadx_config_production.
        unsafe {
            let mut got: u64 = 0;
            assert_eq!(
                super::thetadatadx_config_get_retry_max_delay_ms(cfg, &mut got),
                0
            );
            assert_eq!(got, 30_000);
            for ms in [0u64, 1, 1_000, 30_000, 300_000] {
                super::thetadatadx_config_set_retry_max_delay_ms(cfg, ms);
                assert_eq!(
                    super::thetadatadx_config_get_retry_max_delay_ms(cfg, &mut got),
                    0
                );
                assert_eq!(got, ms);
            }
            super::thetadatadx_config_free(cfg);
        }
    }

    #[test]
    fn retry_max_attempts_round_trips_via_getter() {
        let cfg = super::thetadatadx_config_production();
        // SAFETY: handle just returned by thetadatadx_config_production.
        unsafe {
            let mut got: u32 = 0;
            assert_eq!(
                super::thetadatadx_config_get_retry_max_attempts(cfg, &mut got),
                0
            );
            assert_eq!(got, 20);
            for n in [0u32, 1, 3, 5, 10, 100] {
                super::thetadatadx_config_set_retry_max_attempts(cfg, n);
                assert_eq!(
                    super::thetadatadx_config_get_retry_max_attempts(cfg, &mut got),
                    0
                );
                assert_eq!(got, n);
            }
            super::thetadatadx_config_free(cfg);
        }
    }

    #[test]
    fn retry_jitter_round_trips_via_getter() {
        let cfg = super::thetadatadx_config_production();
        // SAFETY: handle just returned by thetadatadx_config_production.
        unsafe {
            let mut got = false;
            assert_eq!(super::thetadatadx_config_get_retry_jitter(cfg, &mut got), 0);
            assert!(got, "default jitter is true");
            super::thetadatadx_config_set_retry_jitter(cfg, false);
            assert_eq!(super::thetadatadx_config_get_retry_jitter(cfg, &mut got), 0);
            assert!(!got);
            super::thetadatadx_config_set_retry_jitter(cfg, true);
            assert_eq!(super::thetadatadx_config_get_retry_jitter(cfg, &mut got), 0);
            assert!(got);
            super::thetadatadx_config_free(cfg);
        }
    }

    #[test]
    fn retry_setters_null_handle_returns_minus_one_or_noop() {
        // SAFETY: passing null to thetadatadx_config_* is the documented FFI
        // contract — getter returns sentinel, setter no-ops.
        unsafe {
            super::thetadatadx_config_set_retry_initial_delay_ms(std::ptr::null_mut(), 100);
            super::thetadatadx_config_set_retry_max_delay_ms(std::ptr::null_mut(), 1_000);
            super::thetadatadx_config_set_retry_max_attempts(std::ptr::null_mut(), 3);
            super::thetadatadx_config_set_retry_jitter(std::ptr::null_mut(), false);
            let mut got_ms: u64 = 0;
            let mut got_n: u32 = 0;
            let mut got_b = false;
            assert_eq!(
                super::thetadatadx_config_get_retry_initial_delay_ms(std::ptr::null(), &mut got_ms),
                -1
            );
            assert_eq!(
                super::thetadatadx_config_get_retry_max_delay_ms(std::ptr::null(), &mut got_ms),
                -1
            );
            assert_eq!(
                super::thetadatadx_config_get_retry_max_attempts(std::ptr::null(), &mut got_n),
                -1
            );
            assert_eq!(
                super::thetadatadx_config_get_retry_jitter(std::ptr::null(), &mut got_b),
                -1
            );
        }
    }

    #[test]
    fn retry_field_setters_compose_into_consistent_policy() {
        // After mutating all four fields the `DirectConfig::retry`
        // struct must reflect the composed shape — proves the
        // setters target the same underlying field rather than
        // duplicating state.
        let cfg = super::thetadatadx_config_production();
        // SAFETY: handle just returned by thetadatadx_config_production.
        unsafe {
            super::thetadatadx_config_set_retry_initial_delay_ms(cfg, 500);
            super::thetadatadx_config_set_retry_max_delay_ms(cfg, 60_000);
            super::thetadatadx_config_set_retry_max_attempts(cfg, 7);
            super::thetadatadx_config_set_retry_jitter(cfg, false);
            let retry = &(*cfg).inner.retry;
            assert_eq!(retry.initial_delay, std::time::Duration::from_millis(500));
            assert_eq!(retry.max_delay, std::time::Duration::from_millis(60_000));
            assert_eq!(retry.max_attempts, 7);
            assert!(!retry.jitter);
            super::thetadatadx_config_free(cfg);
        }
    }
}

#[cfg(test)]
mod flatfiles_setter_tests {
    //! Offline tests for the three `FlatFilesConfig` field
    //! setters/getters on the FFI surface — cross-binding parity with
    //! Python / TypeScript / C++. The `backoff_for_attempt` /
    //! `production_defaults` helpers stay Rust-only; this module pins
    //! only field round-trip across the C ABI.

    #[test]
    fn flatfiles_max_attempts_round_trips() {
        let cfg = super::thetadatadx_config_production();
        assert!(!cfg.is_null());
        // SAFETY: handle just returned by thetadatadx_config_production.
        unsafe {
            let mut got: u32 = 0;
            // Default seeded from FlatFilesConfig::production_defaults().
            assert_eq!(
                super::thetadatadx_config_get_flatfiles_max_attempts(cfg, &mut got),
                0
            );
            assert_eq!(got, 10);
            for n in [0u32, 1, 3, 5, 10, 100] {
                super::thetadatadx_config_set_flatfiles_max_attempts(cfg, n);
                assert_eq!((*cfg).inner.flatfiles.max_attempts, n);
                assert_eq!(
                    super::thetadatadx_config_get_flatfiles_max_attempts(cfg, &mut got),
                    0
                );
                assert_eq!(got, n);
            }
            super::thetadatadx_config_free(cfg);
        }
    }

    #[test]
    fn flatfiles_initial_backoff_secs_round_trips() {
        let cfg = super::thetadatadx_config_production();
        // SAFETY: handle just returned by thetadatadx_config_production.
        unsafe {
            let mut got: u64 = 0;
            // Default seeded from FlatFilesConfig::production_defaults().
            assert_eq!(
                super::thetadatadx_config_get_flatfiles_initial_backoff_secs(cfg, &mut got),
                0
            );
            assert_eq!(got, 1);
            for secs in [0u64, 1, 2, 4, 10, 60, 3600] {
                super::thetadatadx_config_set_flatfiles_initial_backoff_secs(cfg, secs);
                assert_eq!(
                    (*cfg).inner.flatfiles.initial_backoff,
                    std::time::Duration::from_secs(secs),
                );
                assert_eq!(
                    super::thetadatadx_config_get_flatfiles_initial_backoff_secs(cfg, &mut got),
                    0
                );
                assert_eq!(got, secs);
            }
            super::thetadatadx_config_free(cfg);
        }
    }

    #[test]
    fn flatfiles_max_backoff_secs_round_trips() {
        let cfg = super::thetadatadx_config_production();
        // SAFETY: handle just returned by thetadatadx_config_production.
        unsafe {
            let mut got: u64 = 0;
            // Default seeded from FlatFilesConfig::production_defaults().
            assert_eq!(
                super::thetadatadx_config_get_flatfiles_max_backoff_secs(cfg, &mut got),
                0
            );
            assert_eq!(got, 30);
            for secs in [0u64, 1, 4, 10, 60, 3600, 86_400] {
                super::thetadatadx_config_set_flatfiles_max_backoff_secs(cfg, secs);
                assert_eq!(
                    (*cfg).inner.flatfiles.max_backoff,
                    std::time::Duration::from_secs(secs),
                );
                assert_eq!(
                    super::thetadatadx_config_get_flatfiles_max_backoff_secs(cfg, &mut got),
                    0
                );
                assert_eq!(got, secs);
            }
            super::thetadatadx_config_free(cfg);
        }
    }

    #[test]
    fn flatfiles_connect_timeout_secs_round_trips() {
        let cfg = super::thetadatadx_config_production();
        // SAFETY: handle just returned by thetadatadx_config_production.
        unsafe {
            let mut got: u64 = 0;
            // Default seeded from FlatFilesConfig::production_defaults().
            assert_eq!(
                super::thetadatadx_config_get_flatfiles_connect_timeout_secs(cfg, &mut got),
                0
            );
            assert_eq!(got, 10);
            for secs in [0u64, 1, 4, 10, 60, 3600] {
                super::thetadatadx_config_set_flatfiles_connect_timeout_secs(cfg, secs);
                assert_eq!((*cfg).inner.flatfiles.connect_timeout_secs, secs);
                assert_eq!(
                    super::thetadatadx_config_get_flatfiles_connect_timeout_secs(cfg, &mut got),
                    0
                );
                assert_eq!(got, secs);
            }
            super::thetadatadx_config_free(cfg);
        }
    }

    #[test]
    fn flatfiles_read_timeout_secs_round_trips() {
        let cfg = super::thetadatadx_config_production();
        // SAFETY: handle just returned by thetadatadx_config_production.
        unsafe {
            let mut got: u64 = 0;
            // Default seeded from FlatFilesConfig::production_defaults().
            assert_eq!(
                super::thetadatadx_config_get_flatfiles_read_timeout_secs(cfg, &mut got),
                0
            );
            assert_eq!(got, 60);
            for secs in [0u64, 1, 4, 10, 60, 3600, 86_400] {
                super::thetadatadx_config_set_flatfiles_read_timeout_secs(cfg, secs);
                assert_eq!((*cfg).inner.flatfiles.read_timeout_secs, secs);
                assert_eq!(
                    super::thetadatadx_config_get_flatfiles_read_timeout_secs(cfg, &mut got),
                    0
                );
                assert_eq!(got, secs);
            }
            super::thetadatadx_config_free(cfg);
        }
    }

    #[test]
    fn flatfiles_setters_null_handle_returns_minus_one_or_noop() {
        // SAFETY: passing null to thetadatadx_config_* is the documented FFI
        // contract — getter returns sentinel, setter no-ops.
        unsafe {
            super::thetadatadx_config_set_flatfiles_max_attempts(std::ptr::null_mut(), 3);
            super::thetadatadx_config_set_flatfiles_initial_backoff_secs(std::ptr::null_mut(), 1);
            super::thetadatadx_config_set_flatfiles_max_backoff_secs(std::ptr::null_mut(), 4);
            super::thetadatadx_config_set_flatfiles_connect_timeout_secs(std::ptr::null_mut(), 10);
            super::thetadatadx_config_set_flatfiles_read_timeout_secs(std::ptr::null_mut(), 60);
            let mut got_n: u32 = 0;
            let mut got_secs: u64 = 0;
            assert_eq!(
                super::thetadatadx_config_get_flatfiles_max_attempts(std::ptr::null(), &mut got_n),
                -1
            );
            assert_eq!(
                super::thetadatadx_config_get_flatfiles_initial_backoff_secs(
                    std::ptr::null(),
                    &mut got_secs
                ),
                -1
            );
            assert_eq!(
                super::thetadatadx_config_get_flatfiles_max_backoff_secs(
                    std::ptr::null(),
                    &mut got_secs
                ),
                -1
            );
            assert_eq!(
                super::thetadatadx_config_get_flatfiles_connect_timeout_secs(
                    std::ptr::null(),
                    &mut got_secs
                ),
                -1
            );
            assert_eq!(
                super::thetadatadx_config_get_flatfiles_read_timeout_secs(
                    std::ptr::null(),
                    &mut got_secs
                ),
                -1
            );
        }
    }

    #[test]
    fn flatfiles_field_setters_compose_into_consistent_config() {
        // After mutating all three fields the `DirectConfig.flatfiles`
        // struct must reflect the composed shape — proves the setters
        // target the same underlying field rather than duplicating state.
        let cfg = super::thetadatadx_config_production();
        // SAFETY: handle just returned by thetadatadx_config_production.
        unsafe {
            super::thetadatadx_config_set_flatfiles_max_attempts(cfg, 5);
            super::thetadatadx_config_set_flatfiles_initial_backoff_secs(cfg, 2);
            super::thetadatadx_config_set_flatfiles_max_backoff_secs(cfg, 30);
            super::thetadatadx_config_set_flatfiles_connect_timeout_secs(cfg, 15);
            super::thetadatadx_config_set_flatfiles_read_timeout_secs(cfg, 90);
            let ff = &(*cfg).inner.flatfiles;
            assert_eq!(ff.max_attempts, 5);
            assert_eq!(ff.initial_backoff, std::time::Duration::from_secs(2));
            assert_eq!(ff.max_backoff, std::time::Duration::from_secs(30));
            assert_eq!(ff.connect_timeout_secs, 15);
            assert_eq!(ff.read_timeout_secs, 90);
            super::thetadatadx_config_free(cfg);
        }
    }
}

#[cfg(test)]
mod auth_metrics_setter_tests {
    //! Offline tests for the `AuthConfig` (`nexus_url` / `client_type`)
    //! and `MetricsConfig` (`port`) field setters/getters on the FFI
    //! surface — cross-binding parity with Python / TypeScript / C++.
    //!
    //! The two `AuthConfig` fields are `String` (setter takes a
    //! `*const c_char`, getter returns a heap-owned `*mut c_char` the
    //! caller frees with `thetadatadx_string_free`); `MetricsConfig.port` is
    //! `Option<u16>` carried as the widened `(has_value, port)` shape.

    use crate::types::thetadatadx_string_free;
    use std::ffi::{CStr, CString};

    /// Read a `*mut c_char` getter result into an owned `String` and
    /// release the heap allocation via `thetadatadx_string_free`.
    fn take_owned(p: *mut std::os::raw::c_char) -> Option<String> {
        if p.is_null() {
            return None;
        }
        // SAFETY: `p` is a non-null pointer just returned by a
        // `thetadatadx_config_get_*` getter (produced by CString::into_raw);
        // it is read once and then handed back to thetadatadx_string_free.
        let owned = unsafe { CStr::from_ptr(p) }.to_string_lossy().into_owned();
        // SAFETY: `p` was produced by CString::into_raw; thetadatadx_string_free
        // reclaims it via CString::from_raw exactly once.
        unsafe { thetadatadx_string_free(p) };
        Some(owned)
    }

    #[test]
    fn nexus_url_round_trips_via_getter() {
        let cfg = super::thetadatadx_config_production();
        assert!(!cfg.is_null());
        // SAFETY: handle just returned by thetadatadx_config_production.
        unsafe {
            // Default seeded from AuthConfig::production_defaults().
            let got = take_owned(super::thetadatadx_config_get_nexus_url(cfg));
            assert_eq!(
                got.as_deref(),
                Some("https://nexus-api.thetadata.us/identity/terminal/auth_user"),
            );
            let url = CString::new("https://staging.example.invalid/auth").unwrap();
            assert_eq!(
                super::thetadatadx_config_set_nexus_url(cfg, url.as_ptr()),
                0
            );
            assert_eq!(
                (*cfg).inner.auth.nexus_url,
                "https://staging.example.invalid/auth"
            );
            let got = take_owned(super::thetadatadx_config_get_nexus_url(cfg));
            assert_eq!(got.as_deref(), Some("https://staging.example.invalid/auth"));
            super::thetadatadx_config_free(cfg);
        }
    }

    #[test]
    fn client_type_round_trips_via_getter() {
        let cfg = super::thetadatadx_config_production();
        // SAFETY: handle just returned by thetadatadx_config_production.
        unsafe {
            // Default seeded from AuthConfig::production_defaults().
            let got = take_owned(super::thetadatadx_config_get_client_type(cfg));
            assert_eq!(got.as_deref(), Some("rust-thetadatadx"));
            let ct = CString::new("fleet-east-1").unwrap();
            assert_eq!(
                super::thetadatadx_config_set_client_type(cfg, ct.as_ptr()),
                0
            );
            assert_eq!((*cfg).inner.auth.client_type, "fleet-east-1");
            let got = take_owned(super::thetadatadx_config_get_client_type(cfg));
            assert_eq!(got.as_deref(), Some("fleet-east-1"));
            super::thetadatadx_config_free(cfg);
        }
    }

    #[test]
    fn environment_reads_back_the_selected_clusters_via_getters() {
        // The readback getters mirrored across the bindings. The two
        // channels are selected independently: the stage preset moves the
        // market-data channel to staging while streaming stays on
        // production, the dev preset moves the streaming channel to dev
        // while market-data stays on production, and production keeps both.
        let staged = super::thetadatadx_config_stage();
        let prod = super::thetadatadx_config_production();
        let dev = super::thetadatadx_config_dev();
        // SAFETY: all three handles were just returned by the config constructors.
        unsafe {
            let got = take_owned(super::thetadatadx_config_get_market_data_environment(
                staged,
            ));
            assert_eq!(got.as_deref(), Some("STAGE"));
            let got = take_owned(super::thetadatadx_config_get_streaming_environment(staged));
            assert_eq!(got.as_deref(), Some("PROD"));

            let got = take_owned(super::thetadatadx_config_get_market_data_environment(prod));
            assert_eq!(got.as_deref(), Some("PROD"));
            let got = take_owned(super::thetadatadx_config_get_streaming_environment(prod));
            assert_eq!(got.as_deref(), Some("PROD"));

            let got = take_owned(super::thetadatadx_config_get_market_data_environment(dev));
            assert_eq!(got.as_deref(), Some("PROD"));
            let got = take_owned(super::thetadatadx_config_get_streaming_environment(dev));
            assert_eq!(got.as_deref(), Some("DEV"));

            // A null handle yields null on both getters.
            assert!(
                super::thetadatadx_config_get_market_data_environment(std::ptr::null()).is_null()
            );
            assert!(
                super::thetadatadx_config_get_streaming_environment(std::ptr::null()).is_null()
            );
            super::thetadatadx_config_free(staged);
            super::thetadatadx_config_free(prod);
            super::thetadatadx_config_free(dev);
        }
    }

    #[test]
    fn with_channel_setters_compose_both_environments() {
        // The two channel selectors compose to any combination, including
        // market-data-staging + streaming-dev, mirroring the Rust builder.
        let cfg = super::thetadatadx_config_production();
        // SAFETY: handle just returned by thetadatadx_config_production.
        unsafe {
            assert_eq!(
                super::thetadatadx_config_with_market_data_environment(cfg, 1),
                0
            );
            assert_eq!(
                super::thetadatadx_config_with_streaming_environment(cfg, 1),
                0
            );
            let got = take_owned(super::thetadatadx_config_get_market_data_environment(cfg));
            assert_eq!(got.as_deref(), Some("STAGE"));
            let got = take_owned(super::thetadatadx_config_get_streaming_environment(cfg));
            assert_eq!(got.as_deref(), Some("DEV"));
            // An out-of-range selector is rejected and leaves the config unchanged.
            assert_eq!(
                super::thetadatadx_config_with_market_data_environment(cfg, 2),
                -1
            );
            assert_eq!(
                super::thetadatadx_config_with_streaming_environment(cfg, 9),
                -1
            );
            let got = take_owned(super::thetadatadx_config_get_market_data_environment(cfg));
            assert_eq!(got.as_deref(), Some("STAGE"));
            // A null handle is rejected.
            assert_eq!(
                super::thetadatadx_config_with_market_data_environment(std::ptr::null_mut(), 0),
                -1
            );
            super::thetadatadx_config_free(cfg);
        }
    }

    #[test]
    fn nexus_url_rejects_null_and_leaves_config_unchanged() {
        let cfg = super::thetadatadx_config_production();
        // SAFETY: handle just returned by thetadatadx_config_production.
        let baseline = unsafe { (*cfg).inner.auth.nexus_url.clone() };
        // SAFETY: handle just returned by thetadatadx_config_production.
        unsafe {
            assert_eq!(
                super::thetadatadx_config_set_nexus_url(cfg, std::ptr::null()),
                -1,
                "null url must be rejected with -1",
            );
            assert_eq!((*cfg).inner.auth.nexus_url, baseline);
            super::thetadatadx_config_free(cfg);
        }
    }

    #[test]
    fn auth_string_setters_null_handle_returns_minus_one() {
        // SAFETY: passing null to thetadatadx_config_* is the documented FFI
        // contract — string setters return -1, string getters null.
        unsafe {
            let url = CString::new("x").unwrap();
            assert_eq!(
                super::thetadatadx_config_set_nexus_url(std::ptr::null_mut(), url.as_ptr()),
                -1
            );
            assert_eq!(
                super::thetadatadx_config_set_client_type(std::ptr::null_mut(), url.as_ptr()),
                -1
            );
            assert!(super::thetadatadx_config_get_nexus_url(std::ptr::null()).is_null());
            assert!(super::thetadatadx_config_get_client_type(std::ptr::null()).is_null());
        }
    }

    #[test]
    fn metrics_port_round_trips_via_getter() {
        let cfg = super::thetadatadx_config_production();
        // SAFETY: handle just returned by thetadatadx_config_production.
        unsafe {
            // Default seeded from MetricsConfig::default() — None.
            let mut got_has = true;
            let mut got_port: u16 = 99;
            assert_eq!(
                super::thetadatadx_config_get_metrics_port(cfg, &mut got_has, &mut got_port),
                0
            );
            assert!(!got_has, "default metrics.port must be None");
            assert_eq!(got_port, 0);

            for port in [0u16, 1, 9090, 9100, u16::MAX] {
                assert_eq!(
                    super::thetadatadx_config_set_metrics_port(cfg, true, port),
                    0
                );
                assert_eq!((*cfg).inner.metrics.port, Some(port));
                assert_eq!(
                    super::thetadatadx_config_get_metrics_port(cfg, &mut got_has, &mut got_port),
                    0
                );
                assert!(got_has);
                assert_eq!(got_port, port);
            }

            // Reset to None.
            assert_eq!(
                super::thetadatadx_config_set_metrics_port(cfg, false, 9090),
                0
            );
            assert_eq!((*cfg).inner.metrics.port, None);
            assert_eq!(
                super::thetadatadx_config_get_metrics_port(cfg, &mut got_has, &mut got_port),
                0
            );
            assert!(!got_has);
            assert_eq!(got_port, 0);
            super::thetadatadx_config_free(cfg);
        }
    }

    #[test]
    fn metrics_port_null_handle_returns_minus_one() {
        // SAFETY: passing null to thetadatadx_config_* is the documented FFI
        // contract — getter returns sentinel, setter returns -1.
        unsafe {
            assert_eq!(
                super::thetadatadx_config_set_metrics_port(std::ptr::null_mut(), true, 9090),
                -1
            );
            let mut got_has = false;
            let mut got_port: u16 = 0;
            assert_eq!(
                super::thetadatadx_config_get_metrics_port(
                    std::ptr::null(),
                    &mut got_has,
                    &mut got_port
                ),
                -1
            );
        }
    }
}

#[cfg(test)]
mod resilience_knob_tests {
    //! Round-trip coverage for the connection-resilience knobs across
    //! the C ABI: every setter/getter pair added for the reconnect
    //! engine, the streaming transport, the market-data retry envelope, and
    //! the flatfile jitter toggle.

    #[test]
    fn reconnect_budget_getters_read_auto_limits() {
        let cfg = super::thetadatadx_config_production();
        // SAFETY: handle just returned by thetadatadx_config_production.
        unsafe {
            let mut policy: i32 = -1;
            assert_eq!(
                super::thetadatadx_config_get_reconnect_policy(cfg, &mut policy),
                0
            );
            assert_eq!(policy, 0, "production default policy is Auto");

            let mut got_u32: u32 = 0;
            assert_eq!(
                super::thetadatadx_config_get_reconnect_max_attempts(cfg, &mut got_u32),
                0
            );
            assert_eq!(got_u32, 30);
            assert_eq!(
                super::thetadatadx_config_get_reconnect_max_rate_limited_attempts(
                    cfg,
                    &mut got_u32
                ),
                0
            );
            assert_eq!(got_u32, 100);
            assert_eq!(
                super::thetadatadx_config_get_reconnect_max_server_restart_attempts(
                    cfg,
                    &mut got_u32
                ),
                0
            );
            assert_eq!(got_u32, 60);

            let mut got_u64: u64 = 0;
            assert_eq!(
                super::thetadatadx_config_get_reconnect_stable_window_secs(cfg, &mut got_u64),
                0
            );
            assert_eq!(got_u64, 60);
            assert_eq!(
                super::thetadatadx_config_get_reconnect_max_elapsed_secs(cfg, &mut got_u64),
                0
            );
            assert_eq!(got_u64, 300);

            // Setters write through and read back.
            super::thetadatadx_config_set_reconnect_max_server_restart_attempts(cfg, 7);
            assert_eq!(
                super::thetadatadx_config_get_reconnect_max_server_restart_attempts(
                    cfg,
                    &mut got_u32
                ),
                0
            );
            assert_eq!(got_u32, 7);
            super::thetadatadx_config_set_reconnect_max_elapsed_secs(cfg, 0);
            assert_eq!(
                super::thetadatadx_config_get_reconnect_max_elapsed_secs(cfg, &mut got_u64),
                0
            );
            assert_eq!(got_u64, 0, "0 (envelope disabled) round-trips");

            super::thetadatadx_config_free(cfg);
        }
    }

    #[test]
    fn reconnect_policy_round_trips_and_rejects_invalid() {
        let cfg = super::thetadatadx_config_production();
        // SAFETY: handle just returned by thetadatadx_config_production.
        unsafe {
            let mut policy: i32 = -1;
            assert_eq!(
                super::thetadatadx_config_get_reconnect_policy(cfg, &mut policy),
                0
            );
            assert_eq!(policy, 0, "production default policy is Auto");
            for p in [1, 0] {
                assert_eq!(super::thetadatadx_config_set_reconnect_policy(cfg, p), 0);
                assert_eq!(
                    super::thetadatadx_config_get_reconnect_policy(cfg, &mut policy),
                    0
                );
                assert_eq!(policy, p);
            }
            // An unknown selector is rejected with the typed
            // invalid-parameter class rather than silently coerced to
            // Auto — the cross-binding contract the Python ValueError /
            // TypeScript InvalidParameterError already honour.
            assert_eq!(
                super::thetadatadx_config_set_reconnect_policy(cfg, 7),
                -1,
                "unknown policy rejected, not coerced"
            );
            assert_eq!(
                crate::error::thetadatadx_last_error_code(),
                crate::error::THETADATADX_ERR_INVALID_PARAMETER
            );
            assert_eq!(super::thetadatadx_config_set_reconnect_policy(cfg, -5), -1);
            assert_eq!(
                crate::error::thetadatadx_last_error_code(),
                crate::error::THETADATADX_ERR_INVALID_PARAMETER
            );
            assert_eq!(
                super::thetadatadx_config_get_reconnect_policy(cfg, &mut policy),
                0
            );
            assert_eq!(policy, 0, "rejected policy leaves the config unchanged");
            super::thetadatadx_config_free(cfg);
        }
    }

    #[test]
    fn reconnect_cadence_and_replay_round_trip() {
        let cfg = super::thetadatadx_config_production();
        // SAFETY: handle just returned by thetadatadx_config_production.
        unsafe {
            let mut got: u64 = 0;
            assert_eq!(
                super::thetadatadx_config_get_reconnect_wait_max_ms(cfg, &mut got),
                0
            );
            assert_eq!(got, 30_000);
            super::thetadatadx_config_set_reconnect_wait_max_ms(cfg, 45_000);
            assert_eq!(
                super::thetadatadx_config_get_reconnect_wait_max_ms(cfg, &mut got),
                0
            );
            assert_eq!(got, 45_000);

            assert_eq!(
                super::thetadatadx_config_get_reconnect_wait_server_restart_ms(cfg, &mut got),
                0
            );
            assert_eq!(got, 5_000);
            super::thetadatadx_config_set_reconnect_wait_server_restart_ms(cfg, 9_000);
            assert_eq!(
                super::thetadatadx_config_get_reconnect_wait_server_restart_ms(cfg, &mut got),
                0
            );
            assert_eq!(got, 9_000);

            let mut got_u32: u32 = 0;
            assert_eq!(
                super::thetadatadx_config_get_reconnect_replay_burst_size(cfg, &mut got_u32),
                0
            );
            assert_eq!(got_u32, 50);
            super::thetadatadx_config_set_reconnect_replay_burst_size(cfg, 200);
            assert_eq!(
                super::thetadatadx_config_get_reconnect_replay_burst_size(cfg, &mut got_u32),
                0
            );
            assert_eq!(got_u32, 200);

            assert_eq!(
                super::thetadatadx_config_get_reconnect_replay_pace_ms(cfg, &mut got),
                0
            );
            assert_eq!(got, 5);
            super::thetadatadx_config_set_reconnect_replay_pace_ms(cfg, 0);
            assert_eq!(
                super::thetadatadx_config_get_reconnect_replay_pace_ms(cfg, &mut got),
                0
            );
            assert_eq!(got, 0);

            super::thetadatadx_config_free(cfg);
        }
    }

    #[test]
    fn reconnect_jitter_round_trips_and_rejects_invalid() {
        let cfg = super::thetadatadx_config_production();
        // SAFETY: handle just returned by thetadatadx_config_production.
        unsafe {
            let mut mode: i32 = -1;
            assert_eq!(
                super::thetadatadx_config_get_reconnect_jitter(cfg, &mut mode),
                0
            );
            assert_eq!(mode, 0, "default jitter mode is Full");
            for m in [1, 0] {
                assert_eq!(super::thetadatadx_config_set_reconnect_jitter(cfg, m), 0);
                assert_eq!(
                    super::thetadatadx_config_get_reconnect_jitter(cfg, &mut mode),
                    0
                );
                assert_eq!(mode, m);
            }
            assert_eq!(
                super::thetadatadx_config_set_reconnect_jitter(cfg, 9),
                -1,
                "invalid mode rejected"
            );
            assert_eq!(
                crate::error::thetadatadx_last_error_code(),
                crate::error::THETADATADX_ERR_INVALID_PARAMETER,
                "a rejected enum value surfaces the typed invalid-parameter class"
            );
            assert_eq!(
                super::thetadatadx_config_get_reconnect_jitter(cfg, &mut mode),
                0
            );
            assert_eq!(mode, 0, "rejected mode leaves the config unchanged");
            super::thetadatadx_config_free(cfg);
        }
    }

    #[test]
    fn fpss_transport_knobs_round_trip() {
        let cfg = super::thetadatadx_config_production();
        // SAFETY: handle just returned by thetadatadx_config_production.
        unsafe {
            let mut got: u64 = 0;
            assert_eq!(
                super::thetadatadx_config_get_streaming_timeout_ms(cfg, &mut got),
                0
            );
            assert_eq!(got, 10_000);
            super::thetadatadx_config_set_streaming_timeout_ms(cfg, 9_000);
            assert_eq!(
                super::thetadatadx_config_get_streaming_timeout_ms(cfg, &mut got),
                0
            );
            assert_eq!(got, 9_000);

            assert_eq!(
                super::thetadatadx_config_get_streaming_connect_timeout_ms(cfg, &mut got),
                0
            );
            assert_eq!(got, 2_000);
            assert_eq!(
                super::thetadatadx_config_get_streaming_ping_interval_ms(cfg, &mut got),
                0
            );
            assert_eq!(got, 250);
            assert_eq!(
                super::thetadatadx_config_get_streaming_io_read_slice_ms(cfg, &mut got),
                0
            );
            assert_eq!(got, 25);
            assert_eq!(
                super::thetadatadx_config_get_streaming_keepalive_idle_secs(cfg, &mut got),
                0
            );
            assert_eq!(got, 5);
            assert_eq!(
                super::thetadatadx_config_get_streaming_keepalive_interval_secs(cfg, &mut got),
                0
            );
            assert_eq!(got, 2);
            let mut got_u32: u32 = 0;
            assert_eq!(
                super::thetadatadx_config_get_streaming_keepalive_retries(cfg, &mut got_u32),
                0
            );
            assert_eq!(got_u32, 2);

            let mut got_usize: usize = 0;
            assert_eq!(
                super::thetadatadx_config_get_streaming_ring_size(cfg, &mut got_usize),
                0
            );
            assert_eq!(got_usize, 131_072);
            super::thetadatadx_config_set_streaming_ring_size(cfg, 4_096);
            assert_eq!(
                super::thetadatadx_config_get_streaming_ring_size(cfg, &mut got_usize),
                0
            );
            assert_eq!(got_usize, 4_096);
            // Non-power-of-two rejected at the setter; value unchanged.
            super::thetadatadx_config_set_streaming_ring_size(cfg, 5_000);
            assert_eq!(
                super::thetadatadx_config_get_streaming_ring_size(cfg, &mut got_usize),
                0
            );
            assert_eq!(got_usize, 4_096);

            super::thetadatadx_config_free(cfg);
        }
    }

    #[test]
    fn retry_envelope_and_flatfiles_jitter_round_trip() {
        let cfg = super::thetadatadx_config_production();
        // SAFETY: handle just returned by thetadatadx_config_production.
        unsafe {
            let mut got: u64 = 0;
            assert_eq!(
                super::thetadatadx_config_get_retry_max_elapsed_secs(cfg, &mut got),
                0
            );
            assert_eq!(got, 300);
            super::thetadatadx_config_set_retry_max_elapsed_secs(cfg, 0);
            assert_eq!(
                super::thetadatadx_config_get_retry_max_elapsed_secs(cfg, &mut got),
                0
            );
            assert_eq!(got, 0);

            let mut jitter = false;
            assert_eq!(
                super::thetadatadx_config_get_flatfiles_jitter(cfg, &mut jitter),
                0
            );
            assert!(jitter, "flatfile jitter defaults on");
            super::thetadatadx_config_set_flatfiles_jitter(cfg, false);
            assert_eq!(
                super::thetadatadx_config_get_flatfiles_jitter(cfg, &mut jitter),
                0
            );
            assert!(!jitter);

            super::thetadatadx_config_free(cfg);
        }
    }

    /// The registered C callback becomes the Custom reconnect policy:
    /// retriable reasons route through it with the attempt counter, a
    /// non-negative return becomes the delay, a negative return stops.
    #[test]
    fn reconnect_callback_drives_custom_policy() {
        unsafe extern "C" fn decide(
            reason: i32,
            attempt: u32,
            user_data: *mut std::ffi::c_void,
        ) -> i64 {
            // SAFETY: the test passes a valid `*mut i32` it owns for the test duration.
            unsafe {
                *(user_data as *mut i32) += 1;
            }
            if attempt >= 3 {
                return -1;
            }
            i64::from(reason) * 10 + i64::from(attempt)
        }

        let cfg = super::thetadatadx_config_production();
        let mut calls: i32 = 0;
        // SAFETY: handle just returned by thetadatadx_config_production; the
        // callback + user_data outlive every policy invocation below.
        unsafe {
            assert_eq!(
                super::thetadatadx_config_set_reconnect_callback(
                    cfg,
                    Some(decide),
                    std::ptr::addr_of_mut!(calls).cast(),
                ),
                0
            );
            let mut policy: i32 = -1;
            assert_eq!(
                super::thetadatadx_config_get_reconnect_policy(cfg, &mut policy),
                0
            );
            assert_eq!(policy, 2, "callback registration installs Custom");

            match &(*cfg).inner.reconnect.policy {
                thetadatadx::ReconnectPolicy::Custom(f) => {
                    // TimedOut = 4 on the wire; attempt 1 -> 41 ms.
                    let d = f(thetadatadx::RemoveReason::TimedOut, 1)
                        .expect("non-negative return becomes a delay");
                    assert_eq!(d, std::time::Duration::from_millis(41));
                    // Attempt 3 -> negative return -> stop.
                    assert!(f(thetadatadx::RemoveReason::TimedOut, 3).is_none());
                }
                other => panic!("expected Custom policy, got {other:?}"),
            }
            assert_eq!(calls, 2, "callback invoked once per decision");

            // NULL callback restores Auto.
            assert_eq!(
                super::thetadatadx_config_set_reconnect_callback(cfg, None, std::ptr::null_mut()),
                0
            );
            let mut policy: i32 = -1;
            assert_eq!(
                super::thetadatadx_config_get_reconnect_policy(cfg, &mut policy),
                0
            );
            assert_eq!(policy, 0);

            super::thetadatadx_config_free(cfg);
        }
    }
}

#[cfg(test)]
mod credentials_dotenv_tests {
    //! Offline smoke coverage for `thetadatadx_credentials_from_dotenv`:
    //! build a credentials handle from a temporary `.env` file carrying a
    //! dummy `THETADATA_API_KEY`, confirm the handle is non-null, and
    //! confirm the error path on a file with no recognized keys.

    use std::ffi::CString;
    use std::io::Write as _;

    fn write_temp(suffix: &str, body: &str) -> std::path::PathBuf {
        let path = std::env::temp_dir().join(format!(
            "thetadatadx-ffi-dotenv-{}-{suffix}",
            std::process::id()
        ));
        let mut f = std::fs::File::create(&path).expect("create tmp .env");
        f.write_all(body.as_bytes()).expect("write tmp .env");
        path
    }

    #[test]
    fn from_dotenv_builds_handle_from_api_key() {
        let path = write_temp("ok.env", "THETADATA_API_KEY=\"td_example_key\"\n");
        let c_path = CString::new(path.to_str().unwrap()).unwrap();
        // SAFETY: c_path is a valid NUL-terminated string for the call's duration.
        let creds = unsafe { super::thetadatadx_credentials_from_dotenv(c_path.as_ptr()) };
        assert!(!creds.is_null());
        // SAFETY: handle just returned by thetadatadx_credentials_from_dotenv.
        unsafe { super::thetadatadx_credentials_free(creds) };
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn from_dotenv_returns_null_when_no_recognized_keys() {
        let path = write_temp("bad.env", "OTHER=value\n");
        let c_path = CString::new(path.to_str().unwrap()).unwrap();
        // SAFETY: c_path is a valid NUL-terminated string for the call's duration.
        let creds = unsafe { super::thetadatadx_credentials_from_dotenv(c_path.as_ptr()) };
        assert!(creds.is_null());
        std::fs::remove_file(&path).ok();
    }
}

#[cfg(test)]
mod credentials_from_env_tests {
    //! Offline smoke coverage for the strict `thetadatadx_credentials_from_env`
    //! resolver: with `THETADATA_API_KEY` unset it returns a null handle and
    //! sets the last error, rather than falling back to a file.

    use std::sync::{Mutex, OnceLock};

    /// Serialize the env mutation so a parallel test never observes the
    /// transient unset state. Held for the body of the test.
    fn env_lock() -> std::sync::MutexGuard<'static, ()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
    }

    #[test]
    fn from_env_returns_null_when_unset() {
        let _guard = env_lock();
        // SAFETY: `_guard` pins the process-global env lock for the body of
        // this test, so no other thread reads or writes the environment while
        // the unset lands.
        unsafe {
            std::env::remove_var("THETADATA_API_KEY");
        }
        let creds = super::thetadatadx_credentials_from_env();
        assert!(creds.is_null());
    }
}
