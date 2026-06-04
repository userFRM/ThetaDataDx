//! Credentials, config, and historical-client lifecycle: `tdx_credentials_*`,
//! `tdx_config_*`, `tdx_client_connect` / `tdx_client_free`.
//!
//! Split verbatim from `lib.rs`; the exported C ABI is unchanged.

use std::os::raw::c_char;
use std::ptr;

use crate::error::{cstr_to_str, set_error, set_error_from};
use crate::runtime;
use crate::types::{TdxClient, TdxConfig, TdxCredentials};

// ── Credentials ──

/// Create credentials from email and password strings.
///
/// Returns null on invalid input (check `tdx_last_error()`).
#[no_mangle]
pub unsafe extern "C" fn tdx_credentials_new(
    email: *const c_char,
    password: *const c_char,
) -> *mut TdxCredentials {
    ffi_boundary!(ptr::null_mut(), {
        // SAFETY: caller supplies a NUL-terminated C string allocated by the host runtime; cstr_to_str validates non-null + UTF-8.
        let email = match unsafe { cstr_to_str(email) } {
            Ok(Some(s)) => s,
            Ok(None) => {
                set_error("email is null");
                return ptr::null_mut();
            }
            Err(e) => {
                set_error(&format!("email is not valid UTF-8: {e}"));
                return ptr::null_mut();
            }
        };
        // SAFETY: caller supplies a NUL-terminated C string allocated by the host runtime; cstr_to_str validates non-null + UTF-8.
        let password = match unsafe { cstr_to_str(password) } {
            Ok(Some(s)) => s,
            Ok(None) => {
                set_error("password is null");
                return ptr::null_mut();
            }
            Err(e) => {
                set_error(&format!("password is not valid UTF-8: {e}"));
                return ptr::null_mut();
            }
        };
        let creds = thetadatadx::Credentials::new(email, password);
        Box::into_raw(Box::new(TdxCredentials { inner: creds }))
    })
}

/// Load credentials from a file (line 1 = email, line 2 = password).
///
/// Returns null on error (check `tdx_last_error()`).
#[no_mangle]
pub unsafe extern "C" fn tdx_credentials_from_file(path: *const c_char) -> *mut TdxCredentials {
    ffi_boundary!(ptr::null_mut(), {
        // SAFETY: caller supplies a NUL-terminated C string allocated by the host runtime; cstr_to_str validates non-null + UTF-8.
        let path = match unsafe { cstr_to_str(path) } {
            Ok(Some(s)) => s,
            Ok(None) => {
                set_error("path is null");
                return ptr::null_mut();
            }
            Err(e) => {
                set_error(&format!("path is not valid UTF-8: {e}"));
                return ptr::null_mut();
            }
        };
        match thetadatadx::Credentials::from_file(path) {
            Ok(creds) => Box::into_raw(Box::new(TdxCredentials { inner: creds })),
            Err(e) => {
                set_error_from(&e);
                ptr::null_mut()
            }
        }
    })
}

/// Free a credentials handle.
#[no_mangle]
pub unsafe extern "C" fn tdx_credentials_free(creds: *mut TdxCredentials) {
    ffi_boundary!((), {
        if !creds.is_null() {
            // SAFETY: the pointer was returned by Box::into_raw / tdx_*_new and has not been freed; ownership returns to Rust.
            drop(unsafe { Box::from_raw(creds) });
        }
    })
}

// ── Config ──

/// Create a production config (`ThetaData` NJ datacenter).
#[no_mangle]
pub extern "C" fn tdx_config_production() -> *mut TdxConfig {
    ffi_boundary!(ptr::null_mut(), {
        Box::into_raw(Box::new(TdxConfig {
            inner: thetadatadx::DirectConfig::production(),
        }))
    })
}

/// Create a dev config (FPSS dev servers, port 20200, infinite replay).
#[no_mangle]
pub extern "C" fn tdx_config_dev() -> *mut TdxConfig {
    ffi_boundary!(ptr::null_mut(), {
        Box::into_raw(Box::new(TdxConfig {
            inner: thetadatadx::DirectConfig::dev(),
        }))
    })
}

/// Create a stage config (FPSS stage servers, port 20100, unstable).
#[no_mangle]
pub extern "C" fn tdx_config_stage() -> *mut TdxConfig {
    ffi_boundary!(ptr::null_mut(), {
        Box::into_raw(Box::new(TdxConfig {
            inner: thetadatadx::DirectConfig::stage(),
        }))
    })
}

/// Free a config handle.
#[no_mangle]
pub unsafe extern "C" fn tdx_config_free(config: *mut TdxConfig) {
    ffi_boundary!((), {
        if !config.is_null() {
            // SAFETY: the pointer was returned by Box::into_raw / tdx_*_new and has not been freed; ownership returns to Rust.
            drop(unsafe { Box::from_raw(config) });
        }
    })
}

/// Set FPSS flush mode on a config handle.
///
/// - `mode = 0`: Batched (default) -- flush only on PING every 100ms
/// - `mode = 1`: Immediate -- flush after every frame write (lowest latency)
///
/// Returns `0` on success. Returns `-1` and sets `tdx_last_error` /
/// `tdx_last_error_code = TDX_ERR_CONFIG` when `mode` is outside the
/// documented `{0, 1}` set or when `config` is null.
#[no_mangle]
pub unsafe extern "C" fn tdx_config_set_flush_mode(config: *mut TdxConfig, mode: i32) -> i32 {
    ffi_boundary!(-1, {
        if config.is_null() {
            crate::error::set_error_with_code(
                "tdx_config_set_flush_mode: config handle is null",
                crate::error::TDX_ERR_CONFIG,
            );
            return -1;
        }
        let value = match mode {
            0 => thetadatadx::FpssFlushMode::Batched,
            1 => thetadatadx::FpssFlushMode::Immediate,
            other => {
                crate::error::set_error_with_code(
                    &format!(
                        "tdx_config_set_flush_mode: invalid mode {other}; expected 0 (Batched) or 1 (Immediate)"
                    ),
                    crate::error::TDX_ERR_CONFIG,
                );
                return -1;
            }
        };
        // SAFETY: caller passes a pointer returned by `tdx_direct_config_new`
        // that has not been freed; null was rejected above; `&mut *` produces a
        // unique reference valid for the call duration because the caller owns
        // the Box and the FFI contract forbids concurrent calls on the same
        // handle.
        let config = unsafe { &mut *config };
        config.inner.fpss.flush_mode = value;
        0
    })
}

/// Set FPSS reconnect policy on a config handle.
///
/// - `policy = 0`: Auto (default) -- auto-reconnect with split per-class
///   attempt budgets (see `tdx_config_set_reconnect_max_attempts`,
///   `tdx_config_set_reconnect_max_rate_limited_attempts`,
///   `tdx_config_set_reconnect_stable_window_secs`).
/// - `policy = 1`: Manual -- no auto-reconnect, user calls reconnect explicitly
#[no_mangle]
pub unsafe extern "C" fn tdx_config_set_reconnect_policy(config: *mut TdxConfig, policy: i32) {
    ffi_boundary!((), {
        let config = require_config_mut!(config);
        config.inner.reconnect.policy = match policy {
            1 => thetadatadx::ReconnectPolicy::Manual,
            _ => thetadatadx::ReconnectPolicy::Auto(thetadatadx::ReconnectAttemptLimits::default()),
        };
    })
}

/// Set the per-class transient-failure attempt budget for the
/// auto-reconnect path. Default `3`. No effect unless the reconnect
/// policy is `Auto`.
#[no_mangle]
pub unsafe extern "C" fn tdx_config_set_reconnect_max_attempts(
    config: *mut TdxConfig,
    max_attempts: u32,
) {
    ffi_boundary!((), {
        let config = require_config_mut!(config);
        if let thetadatadx::ReconnectPolicy::Auto(ref mut limits) = config.inner.reconnect.policy {
            limits.max_attempts = max_attempts;
        }
    })
}

/// Set the per-class rate-limited (`TooManyRequests`) attempt budget
/// for the auto-reconnect path. Default `100`. No effect unless the
/// reconnect policy is `Auto`.
#[no_mangle]
pub unsafe extern "C" fn tdx_config_set_reconnect_max_rate_limited_attempts(
    config: *mut TdxConfig,
    max_rate_limited_attempts: u32,
) {
    ffi_boundary!((), {
        let config = require_config_mut!(config);
        if let thetadatadx::ReconnectPolicy::Auto(ref mut limits) = config.inner.reconnect.policy {
            limits.max_rate_limited_attempts = max_rate_limited_attempts;
        }
    })
}

/// Set the continuous successful-data-flow window (in seconds) after
/// which the auto-reconnect attempt counters reset. Default `60`. No
/// effect unless the reconnect policy is `Auto`.
#[no_mangle]
pub unsafe extern "C" fn tdx_config_set_reconnect_stable_window_secs(
    config: *mut TdxConfig,
    secs: u64,
) {
    ffi_boundary!((), {
        let config = require_config_mut!(config);
        if let thetadatadx::ReconnectPolicy::Auto(ref mut limits) = config.inner.reconnect.policy {
            limits.stable_window = std::time::Duration::from_secs(secs);
        }
    })
}

/// Set the reconnect delay (ms) honoured for generic transient
/// disconnects (TimedOut, ServerRestarting, Unspecified, …). Plumbed
/// through to the FPSS I/O loop at connect time and consumed by the
/// `Auto` reconnect arm via `reconnect_delay_for`. Default `2_000`.
#[no_mangle]
pub unsafe extern "C" fn tdx_config_set_reconnect_wait_ms(config: *mut TdxConfig, ms: u64) {
    ffi_boundary!((), {
        let config = require_config_mut!(config);
        config.inner.reconnect.wait_ms = ms;
    })
}

/// Read the current reconnect `wait_ms` setting.
///
/// Writes the configured millisecond delay into `*out_ms`. Returns
/// `0` on success, `-1` if either pointer is null.
#[no_mangle]
pub unsafe extern "C" fn tdx_config_get_reconnect_wait_ms(
    config: *const TdxConfig,
    out_ms: *mut u64,
) -> i32 {
    ffi_boundary!(-1, {
        if config.is_null() || out_ms.is_null() {
            set_error("config or out-parameter pointer is null");
            return -1;
        }
        // SAFETY: config is a non-null `*const TdxConfig` returned by `tdx_config_*` and not yet freed; `&*` produces a shared reference valid for the call duration.
        let config = unsafe { &*config };
        // SAFETY: out_ms checked non-null at line 238; FFI contract pins
        // the `u64` storage for the call. Writing the `reconnect.wait_ms`
        // field cannot tear under a concurrent reader because the FFI
        // surface is not thread-safe on a single config handle.
        unsafe {
            *out_ms = config.inner.reconnect.wait_ms;
        }
        0
    })
}

/// Set the reconnect delay (ms) honoured for `TooManyRequests`
/// rate-limited disconnects. Plumbed through to the FPSS I/O loop at
/// connect time and consumed by the `Auto` reconnect arm via
/// `reconnect_delay_for`. Default `130_000` (matches the Java terminal's
/// 130 s rate-limit cooldown).
#[no_mangle]
pub unsafe extern "C" fn tdx_config_set_reconnect_wait_rate_limited_ms(
    config: *mut TdxConfig,
    ms: u64,
) {
    ffi_boundary!((), {
        let config = require_config_mut!(config);
        config.inner.reconnect.wait_rate_limited_ms = ms;
    })
}

/// Read the current reconnect `wait_rate_limited_ms` setting. Same
/// shape as [`tdx_config_get_reconnect_wait_ms`].
#[no_mangle]
pub unsafe extern "C" fn tdx_config_get_reconnect_wait_rate_limited_ms(
    config: *const TdxConfig,
    out_ms: *mut u64,
) -> i32 {
    ffi_boundary!(-1, {
        if config.is_null() || out_ms.is_null() {
            set_error("config or out-parameter pointer is null");
            return -1;
        }
        // SAFETY: config is a non-null `*const TdxConfig` returned by `tdx_config_*` and not yet freed; `&*` produces a shared reference valid for the call duration.
        let config = unsafe { &*config };
        // SAFETY: out_ms checked non-null at line 279; FFI contract pins
        // the `u64` storage for the call. Writing
        // `reconnect.wait_rate_limited_ms` cannot tear under a concurrent
        // reader — FFI handles are not thread-safe per the public contract.
        unsafe {
            *out_ms = config.inner.reconnect.wait_rate_limited_ms;
        }
        0
    })
}

/// Set the `RuntimeConfig.tokio_worker_threads` knob using the
/// `(has_value, n)` widened ABI shape that preserves the `Some(0)`
/// sentinel across the C boundary.
///
/// * `has_value = false` → `None` (tokio default sizing, one worker per
///   logical CPU). `n` is ignored.
/// * `has_value = true` → `Some(n)`. Embedders consuming
///   [`thetadatadx::RuntimeConfig::build_runtime`] honour the value
///   verbatim, clamping `0` to `1` to keep tokio from panicking on
///   `worker_threads(0)`.
///
/// Returns `0` on success, `-1` if `config` is null.
#[no_mangle]
pub unsafe extern "C" fn tdx_config_set_tokio_worker_threads_explicit(
    config: *mut TdxConfig,
    has_value: bool,
    n: usize,
) -> i32 {
    ffi_boundary!(-1, {
        if config.is_null() {
            set_error("config handle is null");
            return -1;
        }
        // SAFETY: config is a non-null pointer returned by
        // tdx_config_production / tdx_config_dev / tdx_config_stage
        // and not yet freed.
        let config = unsafe { &mut *config };
        config.inner.runtime.tokio_worker_threads = if has_value { Some(n) } else { None };
        0
    })
}

/// Read the current `RuntimeConfig.tokio_worker_threads` setting. Same
/// `(has_value, n)` ABI as [`tdx_config_get_decode_threads`]:
///
/// * `*out_has_value = false` → `None` (auto-size). `*out_n` is left as `0`.
/// * `*out_has_value = true` → `Some(*out_n)`.
///
/// Returns `0` on success, `-1` if any pointer is null.
#[no_mangle]
pub unsafe extern "C" fn tdx_config_get_tokio_worker_threads(
    config: *const TdxConfig,
    out_has_value: *mut bool,
    out_n: *mut usize,
) -> i32 {
    ffi_boundary!(-1, {
        if config.is_null() || out_has_value.is_null() || out_n.is_null() {
            set_error("config or out-parameter pointer is null");
            return -1;
        }
        // SAFETY: config is a non-null `*const TdxConfig` returned by `tdx_config_*` and not yet freed; `&*` produces a shared reference valid for the call duration.
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

// ── RetryPolicy ────────────────────────────────────────────────────
//
// Per-field setters/getters on `DirectConfig.retry`. The two
// `Duration` fields (`initial_delay`, `max_delay`) cross the ABI as
// `u64` milliseconds; `max_attempts` is `u32`; `jitter` is `bool`.
// `delay_for_attempt` / `capped_backoff` / `disabled()` factory stay
// Rust-only — they are method-shape helpers that bindings can
// reproduce on top of the four field setters if needed.

/// Set the initial backoff delay (ms) for the MDDS retry policy.
/// Default `250`. Subsequent retries double from here, capped at
/// `tdx_config_set_retry_max_delay_ms`.
#[no_mangle]
pub unsafe extern "C" fn tdx_config_set_retry_initial_delay_ms(config: *mut TdxConfig, ms: u64) {
    ffi_boundary!((), {
        let config = require_config_mut!(config);
        config.inner.retry.initial_delay = std::time::Duration::from_millis(ms);
    })
}

/// Read the current `retry.initial_delay` setting (ms). Returns `0` on
/// success, `-1` if either pointer is null.
#[no_mangle]
pub unsafe extern "C" fn tdx_config_get_retry_initial_delay_ms(
    config: *const TdxConfig,
    out_ms: *mut u64,
) -> i32 {
    ffi_boundary!(-1, {
        if config.is_null() || out_ms.is_null() {
            set_error("config or out-parameter pointer is null");
            return -1;
        }
        // SAFETY: config is a non-null `*const TdxConfig` returned by `tdx_config_*` and not yet freed; `&*` produces a shared reference valid for the call duration.
        let config = unsafe { &*config };
        let ms = u64::try_from(config.inner.retry.initial_delay.as_millis()).unwrap_or(u64::MAX);
        // SAFETY: out_ms checked non-null at line 389; FFI contract
        // pins the `u64` storage for the call. The `ms` local lives
        // for the entire scope so the write is in-bounds for the
        // pointer's lifetime.
        unsafe {
            *out_ms = ms;
        }
        0
    })
}

/// Set the upper-bound backoff delay (ms) for the MDDS retry policy.
/// Default `30_000` (30 s). The exponential schedule never exceeds
/// this value regardless of attempt number.
#[no_mangle]
pub unsafe extern "C" fn tdx_config_set_retry_max_delay_ms(config: *mut TdxConfig, ms: u64) {
    ffi_boundary!((), {
        let config = require_config_mut!(config);
        config.inner.retry.max_delay = std::time::Duration::from_millis(ms);
    })
}

/// Read the current `retry.max_delay` setting (ms).
#[no_mangle]
pub unsafe extern "C" fn tdx_config_get_retry_max_delay_ms(
    config: *const TdxConfig,
    out_ms: *mut u64,
) -> i32 {
    ffi_boundary!(-1, {
        if config.is_null() || out_ms.is_null() {
            set_error("config or out-parameter pointer is null");
            return -1;
        }
        // SAFETY: config is a non-null `*const TdxConfig` returned by `tdx_config_*` and not yet freed; `&*` produces a shared reference valid for the call duration.
        let config = unsafe { &*config };
        let ms = u64::try_from(config.inner.retry.max_delay.as_millis()).unwrap_or(u64::MAX);
        // SAFETY: out_ms checked non-null at line 425; FFI contract pins
        // the `u64` storage. Writing `retry.max_delay` (a saturating
        // `Duration::as_millis` clamp) cannot exceed `u64::MAX`.
        unsafe {
            *out_ms = ms;
        }
        0
    })
}

/// Set the total attempt budget for the MDDS retry policy. `1`
/// disables retry (single call only); higher values permit
/// retries up to `max_attempts - 1` after the initial call. Default
/// `5`.
#[no_mangle]
pub unsafe extern "C" fn tdx_config_set_retry_max_attempts(config: *mut TdxConfig, n: u32) {
    ffi_boundary!((), {
        let config = require_config_mut!(config);
        config.inner.retry.max_attempts = n;
    })
}

/// Read the current `retry.max_attempts` setting.
#[no_mangle]
pub unsafe extern "C" fn tdx_config_get_retry_max_attempts(
    config: *const TdxConfig,
    out_n: *mut u32,
) -> i32 {
    ffi_boundary!(-1, {
        if config.is_null() || out_n.is_null() {
            set_error("config or out-parameter pointer is null");
            return -1;
        }
        // SAFETY: config is a non-null `*const TdxConfig` returned by `tdx_config_*` and not yet freed; `&*` produces a shared reference valid for the call duration.
        let config = unsafe { &*config };
        // SAFETY: out_n null-checked above; caller pins the storage for the call duration.
        unsafe {
            *out_n = config.inner.retry.max_attempts;
        }
        0
    })
}

/// Toggle AWS-style full-jitter on the MDDS retry policy. Default
/// `true`. With `jitter=false` the backoff schedule is deterministic
/// (`min(max_delay, initial * 2^attempt)`), which is useful for tests
/// that need to assert exact timings.
#[no_mangle]
pub unsafe extern "C" fn tdx_config_set_retry_jitter(config: *mut TdxConfig, jitter: bool) {
    ffi_boundary!((), {
        let config = require_config_mut!(config);
        config.inner.retry.jitter = jitter;
    })
}

/// Read the current `retry.jitter` setting.
#[no_mangle]
pub unsafe extern "C" fn tdx_config_get_retry_jitter(
    config: *const TdxConfig,
    out_jitter: *mut bool,
) -> i32 {
    ffi_boundary!(-1, {
        if config.is_null() || out_jitter.is_null() {
            set_error("config or out-parameter pointer is null");
            return -1;
        }
        // SAFETY: config is a non-null `*const TdxConfig` returned by `tdx_config_*` and not yet freed; `&*` produces a shared reference valid for the call duration.
        let config = unsafe { &*config };
        // SAFETY: out_jitter null-checked above; caller pins the storage for the call duration.
        unsafe {
            *out_jitter = config.inner.retry.jitter;
        }
        0
    })
}

/// Set FPSS OHLCVC derivation on a config handle.
///
/// - `enabled = 1` (default): derive OHLCVC bars locally from trade events
/// - `enabled = 0`: only emit server-sent OHLCVC frames (lower overhead)
#[no_mangle]
pub unsafe extern "C" fn tdx_config_set_derive_ohlcvc(config: *mut TdxConfig, enabled: i32) {
    ffi_boundary!((), {
        let config = require_config_mut!(config);
        config.inner.fpss.derive_ohlcvc = enabled != 0;
    })
}

// ── FlatFilesConfig ────────────────────────────────────────────────
//
// Per-field setters/getters on `DirectConfig.flatfiles` mirror the
// `RetryPolicy` shape: `max_attempts` is `u32`, the two `Duration`
// fields cross the ABI as `u64` seconds (matching the human-meaningful
// units `FlatFilesConfig` documents). `backoff_for_attempt` /
// `production_defaults` stay Rust-only — they are method-shape helpers
// callers can recompute from the three field values.

/// Set the total attempt budget for the flatfile driver retry loop.
/// `1` disables retry (single call only); higher values permit
/// retries up to `max_attempts - 1` after the initial call. Default
/// `3`. Validated to the range `[1, 10]` at
/// [`thetadatadx::DirectConfig::validate`] time.
#[no_mangle]
pub unsafe extern "C" fn tdx_config_set_flatfiles_max_attempts(config: *mut TdxConfig, n: u32) {
    ffi_boundary!((), {
        let config = require_config_mut!(config);
        config.inner.flatfiles.max_attempts = n;
    })
}

/// Read the current `flatfiles.max_attempts` setting. Returns `0` on
/// success, `-1` if either pointer is null.
#[no_mangle]
pub unsafe extern "C" fn tdx_config_get_flatfiles_max_attempts(
    config: *const TdxConfig,
    out_n: *mut u32,
) -> i32 {
    ffi_boundary!(-1, {
        if config.is_null() || out_n.is_null() {
            set_error("config or out-parameter pointer is null");
            return -1;
        }
        // SAFETY: config is a non-null `*const TdxConfig` returned by `tdx_config_*` and not yet freed; `&*` produces a shared reference valid for the call duration.
        let config = unsafe { &*config };
        // SAFETY: out_n null-checked above; FFI contract pins the `u32`
        // storage for the call. `flatfiles.max_attempts` is a `u32`
        // field, so the write is layout-compatible with the pointee.
        unsafe {
            *out_n = config.inner.flatfiles.max_attempts;
        }
        0
    })
}

/// Set the initial backoff delay (seconds) for the flatfile driver
/// retry loop. Doubles per attempt up to `max_backoff_secs`. Default
/// `1`.
#[no_mangle]
pub unsafe extern "C" fn tdx_config_set_flatfiles_initial_backoff_secs(
    config: *mut TdxConfig,
    secs: u64,
) {
    ffi_boundary!((), {
        let config = require_config_mut!(config);
        config.inner.flatfiles.initial_backoff = std::time::Duration::from_secs(secs);
    })
}

/// Read the current `flatfiles.initial_backoff` setting (seconds).
/// Returns `0` on success, `-1` if either pointer is null.
#[no_mangle]
pub unsafe extern "C" fn tdx_config_get_flatfiles_initial_backoff_secs(
    config: *const TdxConfig,
    out_secs: *mut u64,
) -> i32 {
    ffi_boundary!(-1, {
        if config.is_null() || out_secs.is_null() {
            set_error("config or out-parameter pointer is null");
            return -1;
        }
        // SAFETY: config is a non-null `*const TdxConfig` returned by `tdx_config_*` and not yet freed; `&*` produces a shared reference valid for the call duration.
        let config = unsafe { &*config };
        // SAFETY: out_secs null-checked above. `Duration::as_secs`
        // returns a `u64` (the seconds component truncates the
        // sub-second remainder), so the write is layout-compatible
        // with the caller-pinned `u64` storage.
        unsafe {
            *out_secs = config.inner.flatfiles.initial_backoff.as_secs();
        }
        0
    })
}

/// Set the upper-bound backoff delay (seconds) for the flatfile
/// driver retry loop. The doubling schedule never exceeds this value
/// regardless of attempt number. Default `4`. Must be greater than
/// or equal to `initial_backoff_secs` (rejected at
/// [`thetadatadx::DirectConfig::validate`] time otherwise).
#[no_mangle]
pub unsafe extern "C" fn tdx_config_set_flatfiles_max_backoff_secs(
    config: *mut TdxConfig,
    secs: u64,
) {
    ffi_boundary!((), {
        let config = require_config_mut!(config);
        config.inner.flatfiles.max_backoff = std::time::Duration::from_secs(secs);
    })
}

/// Read the current `flatfiles.max_backoff` setting (seconds). Returns
/// `0` on success, `-1` if either pointer is null.
#[no_mangle]
pub unsafe extern "C" fn tdx_config_get_flatfiles_max_backoff_secs(
    config: *const TdxConfig,
    out_secs: *mut u64,
) -> i32 {
    ffi_boundary!(-1, {
        if config.is_null() || out_secs.is_null() {
            set_error("config or out-parameter pointer is null");
            return -1;
        }
        // SAFETY: config is a non-null `*const TdxConfig` returned by `tdx_config_*` and not yet freed; `&*` produces a shared reference valid for the call duration.
        let config = unsafe { &*config };
        // SAFETY: out_secs null-checked above. The flatfile retry-loop
        // upper bound is a whole-second value (validated against
        // `initial_backoff` at connect time), so `Duration::as_secs()`
        // round-trips losslessly into the caller-pinned `u64`.
        unsafe {
            *out_secs = config.inner.flatfiles.max_backoff.as_secs();
        }
        0
    })
}

// ── AuthConfig ─────────────────────────────────────────────────────
//
// Per-field setters/getters on `DirectConfig.auth`. Both fields are
// `String`, so the setter takes a `*const c_char` (validated non-null
// + UTF-8, rejected with an error code on bad input) and the getter
// returns a heap-owned `*mut c_char` the caller must release with
// `tdx_string_free` — the same lifetime convention every other owned
// C string returned by this library follows.

/// Set the Nexus auth URL on a config handle.
///
/// `url` must be a non-null, NUL-terminated, valid-UTF-8 C string.
/// Returns `0` on success, `-1` if `config` is null or `url` is
/// null / not valid UTF-8 (the diagnostic is written to thread-local
/// storage retrievable via `tdx_last_error()`).
#[no_mangle]
pub unsafe extern "C" fn tdx_config_set_nexus_url(
    config: *mut TdxConfig,
    url: *const c_char,
) -> i32 {
    ffi_boundary!(-1, {
        if config.is_null() {
            set_error("config handle is null");
            return -1;
        }
        // SAFETY: caller supplies a NUL-terminated C string allocated by the host runtime; cstr_to_str validates non-null + UTF-8.
        let url = match unsafe { cstr_to_str(url) } {
            Ok(Some(s)) => s,
            Ok(None) => {
                set_error("nexus_url is null");
                return -1;
            }
            Err(e) => {
                set_error(&format!("nexus_url is not valid UTF-8: {e}"));
                return -1;
            }
        };
        // SAFETY: config is a non-null pointer returned by
        // tdx_config_production / tdx_config_dev / tdx_config_stage
        // and not yet freed.
        let config = unsafe { &mut *config };
        config.inner.auth.nexus_url = url.to_string();
        0
    })
}

/// Read the current `auth.nexus_url` setting.
///
/// On success, returns a heap-owned NUL-terminated C string the
/// caller MUST release with `tdx_string_free`. Returns null if
/// `config` is null or the stored value contains an interior NUL
/// (the diagnostic is written to `tdx_last_error()`).
#[no_mangle]
pub unsafe extern "C" fn tdx_config_get_nexus_url(config: *const TdxConfig) -> *mut c_char {
    ffi_boundary!(ptr::null_mut(), {
        if config.is_null() {
            set_error("config handle is null");
            return ptr::null_mut();
        }
        // SAFETY: config is a non-null `*const TdxConfig` returned by `tdx_config_*` and not yet freed; `&*` produces a shared reference valid for the call duration.
        let config = unsafe { &*config };
        match std::ffi::CString::new(config.inner.auth.nexus_url.as_str()) {
            Ok(c) => c.into_raw(),
            Err(e) => {
                set_error(&format!("nexus_url contains an interior NUL: {e}"));
                ptr::null_mut()
            }
        }
    })
}

/// Set the `QueryInfo.client_type` identifier on a config handle.
///
/// `client_type` must be a non-null, NUL-terminated, valid-UTF-8 C
/// string. Returns `0` on success, `-1` if `config` is null or
/// `client_type` is null / not valid UTF-8 (the diagnostic is written
/// to thread-local storage retrievable via `tdx_last_error()`).
#[no_mangle]
pub unsafe extern "C" fn tdx_config_set_client_type(
    config: *mut TdxConfig,
    client_type: *const c_char,
) -> i32 {
    ffi_boundary!(-1, {
        if config.is_null() {
            set_error("config handle is null");
            return -1;
        }
        // SAFETY: caller supplies a NUL-terminated C string allocated by the host runtime; cstr_to_str validates non-null + UTF-8.
        let client_type = match unsafe { cstr_to_str(client_type) } {
            Ok(Some(s)) => s,
            Ok(None) => {
                set_error("client_type is null");
                return -1;
            }
            Err(e) => {
                set_error(&format!("client_type is not valid UTF-8: {e}"));
                return -1;
            }
        };
        // SAFETY: config is a non-null pointer returned by
        // tdx_config_production / tdx_config_dev / tdx_config_stage
        // and not yet freed.
        let config = unsafe { &mut *config };
        config.inner.auth.client_type = client_type.to_string();
        0
    })
}

/// Read the current `auth.client_type` setting.
///
/// On success, returns a heap-owned NUL-terminated C string the
/// caller MUST release with `tdx_string_free`. Returns null if
/// `config` is null or the stored value contains an interior NUL
/// (the diagnostic is written to `tdx_last_error()`).
#[no_mangle]
pub unsafe extern "C" fn tdx_config_get_client_type(config: *const TdxConfig) -> *mut c_char {
    ffi_boundary!(ptr::null_mut(), {
        if config.is_null() {
            set_error("config handle is null");
            return ptr::null_mut();
        }
        // SAFETY: config is a non-null `*const TdxConfig` returned by `tdx_config_*` and not yet freed; `&*` produces a shared reference valid for the call duration.
        let config = unsafe { &*config };
        match std::ffi::CString::new(config.inner.auth.client_type.as_str()) {
            Ok(c) => c.into_raw(),
            Err(e) => {
                set_error(&format!("client_type contains an interior NUL: {e}"));
                ptr::null_mut()
            }
        }
    })
}

// ── MetricsConfig ──────────────────────────────────────────────────
//
// `MetricsConfig.port` is `Option<u16>`. The widened
// `(has_value: bool, port: u16)` ABI shape mirrors the `Option`
// fields already on the C surface (`decode_threads`,
// `tokio_worker_threads`): `has_value = false` encodes the `None`
// (exporter disabled) sentinel; `has_value = true` encodes
// `Some(port)`.

/// Set the Prometheus exporter port on a config handle.
///
/// * `has_value = false` encodes `None`: the exporter stays disabled
///   even when the `metrics-prometheus` cargo feature is compiled in.
///   `port` is ignored.
/// * `has_value = true` encodes `Some(port)`: the exporter binds an
///   HTTP listener on `0.0.0.0:<port>` whose `/metrics` endpoint
///   exposes every counter and histogram recorded through the
///   `metrics` crate.
///
/// Returns `0` on success, `-1` if `config` is null.
#[no_mangle]
pub unsafe extern "C" fn tdx_config_set_metrics_port(
    config: *mut TdxConfig,
    has_value: bool,
    port: u16,
) -> i32 {
    ffi_boundary!(-1, {
        if config.is_null() {
            set_error("config handle is null");
            return -1;
        }
        // SAFETY: config is a non-null pointer returned by
        // tdx_config_production / tdx_config_dev / tdx_config_stage
        // and not yet freed.
        let config = unsafe { &mut *config };
        config.inner.metrics.port = if has_value { Some(port) } else { None };
        0
    })
}

/// Read the current `metrics.port` setting.
///
/// * `*out_has_value = false` → the config holds `None` (exporter
///   disabled). `*out_port` is left as `0`.
/// * `*out_has_value = true` → the config holds `Some(*out_port)`.
///
/// Returns `0` on success, `-1` if any pointer is null.
#[no_mangle]
pub unsafe extern "C" fn tdx_config_get_metrics_port(
    config: *const TdxConfig,
    out_has_value: *mut bool,
    out_port: *mut u16,
) -> i32 {
    ffi_boundary!(-1, {
        if config.is_null() || out_has_value.is_null() || out_port.is_null() {
            set_error("config or out-parameter pointer is null");
            return -1;
        }
        // SAFETY: config is a non-null `*const TdxConfig` returned by `tdx_config_*` and not yet freed; `&*` produces a shared reference valid for the call duration.
        let config = unsafe { &*config };
        let (has_value, port) = match config.inner.metrics.port {
            Some(v) => (true, v),
            None => (false, 0),
        };
        // SAFETY: out_has_value / out_port null-checked above; caller pins the storage they point at for the call duration.
        unsafe {
            *out_has_value = has_value;
            *out_port = port;
        }
        0
    })
}

// ── MDDS pool sizing ───────────────────────────────────────────────

/// Set the number of concurrent in-flight gRPC requests on a config
/// handle.
///
/// `n = 0` (default) auto-detects from the Nexus subscription tier
/// (Free=1 / Value=2 / Standard=4 / Pro=8). Explicit values above
/// the tier cap are clamped at connect time with a `tracing::warn!`.
#[no_mangle]
pub unsafe extern "C" fn tdx_config_set_concurrent_requests(config: *mut TdxConfig, n: u32) {
    ffi_boundary!((), {
        let config = require_config_mut!(config);
        config.inner.mdds.concurrent_requests = n as usize;
    })
}

/// Set the `warn_on_buffered_threshold_bytes` ceiling on a config
/// handle. Streaming endpoints log a `tracing::warn!` when a
/// pre-stream-API caller receives a buffered response whose decoded
/// total size exceeds this threshold (default 100 MiB). The warning
/// guides users towards the `.stream()` surface on large pulls; the
/// data is still delivered.
///
/// `n = 0` disables the warning entirely.
#[no_mangle]
pub unsafe extern "C" fn tdx_config_set_warn_on_buffered_threshold_bytes(
    config: *mut TdxConfig,
    n: usize,
) {
    ffi_boundary!((), {
        let config = require_config_mut!(config);
        config.inner.mdds.warn_on_buffered_threshold_bytes = n;
    })
}

/// Read the current `warn_on_buffered_threshold_bytes` setting.
///
/// Writes the configured byte count into `*out_n`. Returns `0` on
/// success, `-1` if either pointer is null.
#[no_mangle]
pub unsafe extern "C" fn tdx_config_get_warn_on_buffered_threshold_bytes(
    config: *const TdxConfig,
    out_n: *mut usize,
) -> i32 {
    ffi_boundary!(-1, {
        if config.is_null() || out_n.is_null() {
            set_error("config or out-parameter pointer is null");
            return -1;
        }
        // SAFETY: config is a non-null `*const TdxConfig` returned by `tdx_config_*` and not yet freed; `&*` produces a shared reference valid for the call duration.
        let config = unsafe { &*config };
        // SAFETY: out_n null-checked above; caller pins the storage for the call duration.
        unsafe {
            *out_n = config.inner.mdds.warn_on_buffered_threshold_bytes;
        }
        0
    })
}

/// Set the per-thread decoder ring size.
///
/// Must be a power of two, `>= 64`. Invalid values are rejected at
/// the setter boundary: the config is left unchanged and the failure
/// reason is written to thread-local storage retrievable via
/// `tdx_last_error()`. Default is `256`.
#[no_mangle]
pub unsafe extern "C" fn tdx_config_set_decoder_ring_size(config: *mut TdxConfig, n: u32) {
    ffi_boundary!((), {
        if config.is_null() {
            return;
        }
        // Same validation as the Rust core's `check_ring_size` plus
        // the event ring minimum — surface the rejection here so the
        // FFI caller sees it at the setter rather than at connect.
        if n == 0 || !n.is_power_of_two() {
            set_error(&format!(
                "decoder_ring_size must be a power of two >= 64; got {n}"
            ));
            return;
        }
        if n < 64 {
            set_error(&format!("decoder_ring_size must be >= 64; got {n}"));
            return;
        }
        // SAFETY: config is a non-null pointer returned by tdx_direct_config_new and not yet freed.
        let config = unsafe { &mut *config };
        config.inner.mdds.decoder_ring_size = n as usize;
    })
}

// ── MDDS two-stage decode pipeline (Phase 3 / PR #587 #588) ────────
//
// Mirror of `MddsConfig::decode_threads` and `decode_queue_depth`
// (both `Option<usize>`) onto the C ABI. The widened
// `(has_value: bool, n: usize)` shape lets callers distinguish the
// `None` (auto-size) sentinel from an explicit `Some(0)` — the
// latter survives across the C boundary and clamps to `1` inside
// `Stage2Pool::new`, matching the contract Python/TS bindings
// already preserve.
//
// Each setter returns `0` on success, `-1` on null-handle (the
// diagnostic is also written to TLS via `set_error`).

/// Set the stage-2 worker thread count for the two-stage MDDS
/// decode pipeline.
///
/// Stage-2 runs prost decode + Tick build off a bounded MPSC queue
/// fed by the stage-1 per-channel decompress threads.
///
/// * `has_value = false` encodes the `None` (auto-size) sentinel:
///   the pool sizes from `std::thread::available_parallelism()` at
///   connect time. `n` is ignored.
/// * `has_value = true` encodes `Some(n)`: the pool pins the
///   stage-2 worker count to `n`. The pool clamps internally to a
///   minimum of `1`, so explicit `0` clamps but is preserved as
///   `Some(0)` on the config — matches the Python `None` vs
///   `Some(0)` semantics across the binding matrix.
///
/// Returns `0` on success, `-1` if `config` is null.
#[no_mangle]
pub unsafe extern "C" fn tdx_config_set_decode_threads_explicit(
    config: *mut TdxConfig,
    has_value: bool,
    n: usize,
) -> i32 {
    ffi_boundary!(-1, {
        if config.is_null() {
            set_error("config handle is null");
            return -1;
        }
        // SAFETY: config is a non-null pointer returned by
        // tdx_config_production / tdx_config_dev / tdx_config_stage
        // and not yet freed.
        let config = unsafe { &mut *config };
        config.inner.mdds.decode_threads = if has_value { Some(n) } else { None };
        0
    })
}

/// Set the bounded queue depth between stage-1 and stage-2 of the
/// two-stage MDDS decode pipeline.
///
/// When stage-2 cannot keep up, stage-1 parks rather than drops —
/// silent drops on a market-data feed are unacceptable.
///
/// * `has_value = false` encodes the `None` (auto-size) sentinel:
///   the queue sizes to `concurrent_requests * 64` (floor of `64`)
///   at connect time. `n` is ignored.
/// * `has_value = true` encodes `Some(n)`. The queue clamps
///   internally to a minimum of `1`.
///
/// Returns `0` on success, `-1` if `config` is null.
#[no_mangle]
pub unsafe extern "C" fn tdx_config_set_decode_queue_depth_explicit(
    config: *mut TdxConfig,
    has_value: bool,
    n: usize,
) -> i32 {
    ffi_boundary!(-1, {
        if config.is_null() {
            set_error("config handle is null");
            return -1;
        }
        // SAFETY: config is a non-null pointer returned by
        // tdx_config_production / tdx_config_dev / tdx_config_stage
        // and not yet freed.
        let config = unsafe { &mut *config };
        config.inner.mdds.decode_queue_depth = if has_value { Some(n) } else { None };
        0
    })
}

// ── Legacy n-only ABI (kept for backwards compatibility) ─────────────────
//
// `n = 0` maps to `None` (auto-size); `n > 0` maps to `Some(n)`.
// Callers that need to encode an explicit `Some(0)` should switch
// to the `_explicit` variants above.

/// Legacy n-only setter. Prefer `tdx_config_set_decode_threads_explicit`.
#[no_mangle]
pub unsafe extern "C" fn tdx_config_set_decode_threads(config: *mut TdxConfig, n: usize) -> i32 {
    let has_value = n != 0;
    // SAFETY: forwarded to the validating wrapper; null-handle and
    // pointer-validity contract identical to the public variant.
    unsafe { tdx_config_set_decode_threads_explicit(config, has_value, n) }
}

/// Legacy n-only setter. Prefer `tdx_config_set_decode_queue_depth_explicit`.
#[no_mangle]
pub unsafe extern "C" fn tdx_config_set_decode_queue_depth(
    config: *mut TdxConfig,
    n: usize,
) -> i32 {
    let has_value = n != 0;
    // SAFETY: forwarded to the validating wrapper; null-handle and
    // pointer-validity contract identical to the public variant.
    unsafe { tdx_config_set_decode_queue_depth_explicit(config, has_value, n) }
}

// ── Getters: round-trip parity with C++ ────────────────────────────

/// Read the current `decode_threads` setting.
///
/// On return:
/// * `*out_has_value = 0` → the config holds `None` (auto-size).
///   `*out_n` is left as `0`.
/// * `*out_has_value = 1` → the config holds `Some(*out_n)`.
///
/// Returns `0` on success, `-1` if any pointer is null.
#[no_mangle]
pub unsafe extern "C" fn tdx_config_get_decode_threads(
    config: *const TdxConfig,
    out_has_value: *mut bool,
    out_n: *mut usize,
) -> i32 {
    ffi_boundary!(-1, {
        if config.is_null() || out_has_value.is_null() || out_n.is_null() {
            set_error("config or out-parameter pointer is null");
            return -1;
        }
        // SAFETY: config is a non-null `*const TdxConfig` returned by `tdx_config_*` and not yet freed; `&*` produces a shared reference valid for the call duration.
        let config = unsafe { &*config };
        let (has_value, n) = match config.inner.mdds.decode_threads {
            Some(v) => (true, v),
            None => (false, 0),
        };
        // SAFETY: out_has_value/out_n null-checked above; caller pins the storage they point at for the call duration.
        unsafe {
            *out_has_value = has_value;
            *out_n = n;
        }
        0
    })
}

/// Read the current `decode_queue_depth` setting. Same semantics as
/// [`tdx_config_get_decode_threads`].
#[no_mangle]
pub unsafe extern "C" fn tdx_config_get_decode_queue_depth(
    config: *const TdxConfig,
    out_has_value: *mut bool,
    out_n: *mut usize,
) -> i32 {
    ffi_boundary!(-1, {
        if config.is_null() || out_has_value.is_null() || out_n.is_null() {
            set_error("config or out-parameter pointer is null");
            return -1;
        }
        // SAFETY: config is a non-null `*const TdxConfig` returned by `tdx_config_*` and not yet freed; `&*` produces a shared reference valid for the call duration.
        let config = unsafe { &*config };
        let (has_value, n) = match config.inner.mdds.decode_queue_depth {
            Some(v) => (true, v),
            None => (false, 0),
        };
        // SAFETY: out_has_value/out_n null-checked above; caller pins the storage they point at for the call duration.
        unsafe {
            *out_has_value = has_value;
            *out_n = n;
        }
        0
    })
}

// ── Client ──

/// Connect to `ThetaData` servers (authenticates via Nexus API).
///
/// Returns null on connection/auth failure (check `tdx_last_error()`).
#[no_mangle]
pub unsafe extern "C" fn tdx_client_connect(
    creds: *const TdxCredentials,
    config: *const TdxConfig,
) -> *mut TdxClient {
    ffi_boundary!(ptr::null_mut(), {
        if creds.is_null() {
            set_error("credentials handle is null");
            return ptr::null_mut();
        }
        if config.is_null() {
            set_error("config handle is null");
            return ptr::null_mut();
        }
        // SAFETY: creds is a non-null pointer returned by tdx_credentials_new / tdx_credentials_from_file and not yet freed.
        let creds = unsafe { &*creds };
        // SAFETY: config is a non-null pointer returned by tdx_direct_config_new and not yet freed.
        let config = unsafe { &*config };
        match runtime().block_on(thetadatadx::mdds::MddsClient::connect(
            &creds.inner,
            config.inner.clone(),
        )) {
            Ok(client) => Box::into_raw(Box::new(TdxClient { inner: client })),
            Err(e) => {
                set_error_from(&e);
                ptr::null_mut()
            }
        }
    })
}

/// Free a client handle.
#[no_mangle]
pub unsafe extern "C" fn tdx_client_free(client: *mut TdxClient) {
    ffi_boundary!((), {
        if !client.is_null() {
            // SAFETY: the pointer was returned by Box::into_raw / tdx_*_new and has not been freed; ownership returns to Rust.
            drop(unsafe { Box::from_raw(client) });
        }
    })
}

#[cfg(test)]
mod pool_sizing_tests {
    //! Offline tests for the MDDS pool-sizing setters.
    //!
    //! Each test allocates a fresh `TdxConfig` via `tdx_config_production`,
    //! calls the setter under test, then reads the underlying Rust
    //! `MddsConfig` to confirm the value round-tripped (or, in the
    //! rejection cases, that the value is unchanged and the error string
    //! reached `tdx_last_error`).

    use crate::error::tdx_last_error;
    use std::ffi::CStr;

    /// Sentinel marker used by `tdx_last_error` when no thread-local
    /// error has been set since the last call. Matches the behaviour
    /// of [`crate::error::tdx_last_error`] returning a `"\0"` placeholder.
    fn no_error_set() -> bool {
        // SAFETY: `tdx_last_error` always returns a valid C string pointer.
        unsafe {
            let p = tdx_last_error();
            if p.is_null() {
                return true;
            }
            CStr::from_ptr(p).to_bytes().is_empty()
        }
    }

    #[test]
    fn concurrent_requests_round_trips() {
        let cfg = super::tdx_config_production();
        assert!(!cfg.is_null());
        // SAFETY: handle just returned by tdx_config_production.
        unsafe {
            super::tdx_config_set_concurrent_requests(cfg, 8);
            assert_eq!((*cfg).inner.mdds.concurrent_requests, 8);
            super::tdx_config_set_concurrent_requests(cfg, 0);
            assert_eq!((*cfg).inner.mdds.concurrent_requests, 0);
            super::tdx_config_free(cfg);
        }
    }

    #[test]
    fn warn_on_buffered_threshold_bytes_round_trips() {
        let cfg = super::tdx_config_production();
        assert!(!cfg.is_null());
        // SAFETY: handle just returned by tdx_config_production.
        unsafe {
            // Default seeded at 100 MiB by `MddsConfig::default()`.
            let mut current: usize = 0;
            assert_eq!(
                super::tdx_config_get_warn_on_buffered_threshold_bytes(cfg, &mut current),
                0
            );
            assert_eq!(current, 100 * 1024 * 1024);
            // Override.
            super::tdx_config_set_warn_on_buffered_threshold_bytes(cfg, 50 * 1024 * 1024);
            assert_eq!(
                (*cfg).inner.mdds.warn_on_buffered_threshold_bytes,
                50 * 1024 * 1024
            );
            assert_eq!(
                super::tdx_config_get_warn_on_buffered_threshold_bytes(cfg, &mut current),
                0
            );
            assert_eq!(current, 50 * 1024 * 1024);
            // Disable.
            super::tdx_config_set_warn_on_buffered_threshold_bytes(cfg, 0);
            assert_eq!((*cfg).inner.mdds.warn_on_buffered_threshold_bytes, 0);
            // Null-pointer guards: setter is a no-op (matches the
            // ffi_boundary `()` return); getter returns -1.
            super::tdx_config_set_warn_on_buffered_threshold_bytes(std::ptr::null_mut(), 4);
            assert_eq!(
                super::tdx_config_get_warn_on_buffered_threshold_bytes(
                    std::ptr::null(),
                    &mut current
                ),
                -1
            );
            super::tdx_config_free(cfg);
        }
    }

    #[test]
    fn decoder_ring_size_accepts_valid_power_of_two() {
        let cfg = super::tdx_config_production();
        // SAFETY: handle just returned by tdx_config_production.
        unsafe {
            for n in [64u32, 128, 256, 512, 1024, 2048, 4096] {
                super::tdx_config_set_decoder_ring_size(cfg, n);
                assert_eq!((*cfg).inner.mdds.decoder_ring_size, n as usize);
            }
            super::tdx_config_free(cfg);
        }
    }

    #[test]
    fn decoder_ring_size_rejects_below_minimum() {
        let cfg = super::tdx_config_production();
        // SAFETY: handle just returned by tdx_config_production above.
        let baseline = unsafe { (*cfg).inner.mdds.decoder_ring_size };
        // SAFETY: handle just returned by tdx_config_production.
        unsafe {
            super::tdx_config_set_decoder_ring_size(cfg, 32);
            // Config left unchanged on rejection.
            assert_eq!((*cfg).inner.mdds.decoder_ring_size, baseline);
            // Error message landed in TLS.
            assert!(!no_error_set(), "rejection must surface via tdx_last_error");
            let p = tdx_last_error();
            let msg = CStr::from_ptr(p).to_string_lossy();
            assert!(
                msg.contains("decoder_ring_size"),
                "error must mention the offending field, got {msg:?}",
            );
            super::tdx_config_free(cfg);
        }
    }

    #[test]
    fn decoder_ring_size_rejects_non_power_of_two() {
        let cfg = super::tdx_config_production();
        // SAFETY: handle just returned by tdx_config_production above.
        let baseline = unsafe { (*cfg).inner.mdds.decoder_ring_size };
        // SAFETY: handle just returned by tdx_config_production.
        unsafe {
            super::tdx_config_set_decoder_ring_size(cfg, 100);
            assert_eq!((*cfg).inner.mdds.decoder_ring_size, baseline);
            super::tdx_config_free(cfg);
        }
    }

    #[test]
    fn decoder_ring_size_rejects_zero() {
        let cfg = super::tdx_config_production();
        // SAFETY: handle just returned by tdx_config_production above.
        let baseline = unsafe { (*cfg).inner.mdds.decoder_ring_size };
        // SAFETY: handle just returned by tdx_config_production.
        unsafe {
            super::tdx_config_set_decoder_ring_size(cfg, 0);
            assert_eq!((*cfg).inner.mdds.decoder_ring_size, baseline);
            super::tdx_config_free(cfg);
        }
    }

    #[test]
    fn null_handle_is_safe() {
        // SAFETY: passing null to tdx_config_set_* / tdx_*_free is the
        // documented FFI contract — the call must return without
        // crashing. The test exercises that null-tolerance branch.
        unsafe {
            super::tdx_config_set_concurrent_requests(std::ptr::null_mut(), 4);
            super::tdx_config_set_decoder_ring_size(std::ptr::null_mut(), 256);
        }
    }
}

#[cfg(test)]
mod reconnect_setter_tests {
    //! Offline tests for the FPSS ReconnectConfig setters on the FFI
    //! surface — cross-binding parity with Python / TypeScript / C++.
    //!
    //! Each test allocates a fresh `TdxConfig` via
    //! `tdx_config_production`, calls the setter under test, then reads
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
        let cfg = super::tdx_config_production();
        assert!(!cfg.is_null());
        // SAFETY: handle just returned by tdx_config_production.
        unsafe {
            super::tdx_config_set_reconnect_policy(cfg, 1);
            assert!(matches!(
                (*cfg).inner.reconnect.policy,
                thetadatadx::ReconnectPolicy::Manual
            ));
            super::tdx_config_set_reconnect_policy(cfg, 0);
            assert!(matches!(
                (*cfg).inner.reconnect.policy,
                thetadatadx::ReconnectPolicy::Auto(_)
            ));
            super::tdx_config_free(cfg);
        }
    }

    #[test]
    fn reconnect_policy_unknown_selector_falls_through_to_auto() {
        // The C ABI accepts an int selector; values other than 0/1
        // resolve to the documented default (`Auto`). Pin the
        // behaviour so the FFI cannot drift away from the documented
        // contract without trapping the test.
        let cfg = super::tdx_config_production();
        // SAFETY: handle just returned by tdx_config_production.
        unsafe {
            super::tdx_config_set_reconnect_policy(cfg, 1);
            super::tdx_config_set_reconnect_policy(cfg, 7);
            assert!(matches!(
                (*cfg).inner.reconnect.policy,
                thetadatadx::ReconnectPolicy::Auto(_)
            ));
            super::tdx_config_free(cfg);
        }
    }

    #[test]
    fn reconnect_max_attempts_round_trips_on_auto_policy() {
        let cfg = super::tdx_config_production();
        // SAFETY: handle just returned by tdx_config_production.
        unsafe {
            super::tdx_config_set_reconnect_policy(cfg, 0);
            for n in [0u32, 1, 3, 10, 100, 1000] {
                super::tdx_config_set_reconnect_max_attempts(cfg, n);
                let thetadatadx::ReconnectPolicy::Auto(limits) = &(*cfg).inner.reconnect.policy
                else {
                    panic!("policy must remain Auto across setter calls");
                };
                assert_eq!(limits.max_attempts, n);
            }
            super::tdx_config_free(cfg);
        }
    }

    #[test]
    fn reconnect_max_rate_limited_attempts_round_trips_on_auto_policy() {
        let cfg = super::tdx_config_production();
        // SAFETY: handle just returned by tdx_config_production.
        unsafe {
            super::tdx_config_set_reconnect_policy(cfg, 0);
            for n in [0u32, 1, 10, 100, 1000] {
                super::tdx_config_set_reconnect_max_rate_limited_attempts(cfg, n);
                let thetadatadx::ReconnectPolicy::Auto(limits) = &(*cfg).inner.reconnect.policy
                else {
                    panic!("policy must remain Auto across setter calls");
                };
                assert_eq!(limits.max_rate_limited_attempts, n);
            }
            super::tdx_config_free(cfg);
        }
    }

    #[test]
    fn reconnect_stable_window_secs_round_trips_on_auto_policy() {
        let cfg = super::tdx_config_production();
        // SAFETY: handle just returned by tdx_config_production.
        unsafe {
            super::tdx_config_set_reconnect_policy(cfg, 0);
            for secs in [0u64, 1, 60, 3600, 86_400] {
                super::tdx_config_set_reconnect_stable_window_secs(cfg, secs);
                let thetadatadx::ReconnectPolicy::Auto(limits) = &(*cfg).inner.reconnect.policy
                else {
                    panic!("policy must remain Auto across setter calls");
                };
                assert_eq!(limits.stable_window, std::time::Duration::from_secs(secs));
            }
            super::tdx_config_free(cfg);
        }
    }

    #[test]
    fn per_class_budget_setters_are_silent_noop_on_manual_policy() {
        // Matches the cross-binding contract: per-class budget setters
        // only mutate `ReconnectAttemptLimits` when the policy variant
        // is `Auto`. Under `Manual` the calls are silently absorbed;
        // the underlying policy variant must not transition.
        let cfg = super::tdx_config_production();
        // SAFETY: handle just returned by tdx_config_production.
        unsafe {
            super::tdx_config_set_reconnect_policy(cfg, 1);
            super::tdx_config_set_reconnect_max_attempts(cfg, 5);
            super::tdx_config_set_reconnect_max_rate_limited_attempts(cfg, 50);
            super::tdx_config_set_reconnect_stable_window_secs(cfg, 120);
            assert!(matches!(
                (*cfg).inner.reconnect.policy,
                thetadatadx::ReconnectPolicy::Manual
            ));
            super::tdx_config_free(cfg);
        }
    }

    #[test]
    fn null_handle_is_safe() {
        // SAFETY: passing null to tdx_config_set_* / tdx_*_free is the
        // documented FFI contract — the call must return without
        // crashing. The test exercises that null-tolerance branch.
        unsafe {
            super::tdx_config_set_reconnect_policy(std::ptr::null_mut(), 0);
            super::tdx_config_set_reconnect_max_attempts(std::ptr::null_mut(), 3);
            super::tdx_config_set_reconnect_max_rate_limited_attempts(std::ptr::null_mut(), 100);
            super::tdx_config_set_reconnect_stable_window_secs(std::ptr::null_mut(), 60);
        }
    }

    #[test]
    fn reconnect_setters_compose_with_pool_sizing_setters() {
        // Cross-binding interleaved-survival contract: reconnect setter
        // calls and pool-sizing setter calls on the same `TdxConfig`
        // must land in `inner` independently and persist. Mirrors the
        // Python `test_reconnect_setter_state_survives_interleaved_calls`,
        // TypeScript `Pool-sizing setter state survives interleaved
        // reconnect setter calls`, and C++ `Reconnect setters compose
        // with pool-sizing setters` cases.
        let cfg = super::tdx_config_production();
        assert!(!cfg.is_null());
        // SAFETY: handle just returned by tdx_config_production.
        unsafe {
            // Apply pool-sizing knobs.
            super::tdx_config_set_concurrent_requests(cfg, 8);
            super::tdx_config_set_decoder_ring_size(cfg, 256);

            // Apply reconnect knobs.
            super::tdx_config_set_reconnect_policy(cfg, 0);
            super::tdx_config_set_reconnect_max_attempts(cfg, 5);
            super::tdx_config_set_reconnect_max_rate_limited_attempts(cfg, 3);
            super::tdx_config_set_reconnect_stable_window_secs(cfg, 60);

            // Pool-sizing mutations survived the reconnect setter sequence.
            let mdds = &(*cfg).inner.mdds;
            assert_eq!(mdds.concurrent_requests, 8);
            assert_eq!(mdds.decoder_ring_size, 256);

            // Reconnect mutations landed on `Auto(limits)`.
            let thetadatadx::ReconnectPolicy::Auto(limits) = &(*cfg).inner.reconnect.policy else {
                panic!("expected ReconnectPolicy::Auto after set_reconnect_policy(0)");
            };
            assert_eq!(limits.max_attempts, 5);
            assert_eq!(limits.max_rate_limited_attempts, 3);
            assert_eq!(limits.stable_window, std::time::Duration::from_secs(60));

            super::tdx_config_free(cfg);
        }
    }

    #[test]
    fn reconnect_wait_ms_round_trips_via_getter() {
        let cfg = super::tdx_config_production();
        // SAFETY: handle just returned by tdx_config_production.
        unsafe {
            let mut got: u64 = 0;
            // Default seeded from ReconnectConfig::production_defaults().
            assert_eq!(super::tdx_config_get_reconnect_wait_ms(cfg, &mut got), 0);
            assert_eq!(got, 2_000);
            for ms in [0u64, 1, 500, 2_000, 60_000, u64::MAX] {
                super::tdx_config_set_reconnect_wait_ms(cfg, ms);
                assert_eq!((*cfg).inner.reconnect.wait_ms, ms);
                assert_eq!(super::tdx_config_get_reconnect_wait_ms(cfg, &mut got), 0);
                assert_eq!(got, ms);
            }
            super::tdx_config_free(cfg);
        }
    }

    #[test]
    fn reconnect_wait_rate_limited_ms_round_trips_via_getter() {
        let cfg = super::tdx_config_production();
        // SAFETY: handle just returned by tdx_config_production.
        unsafe {
            let mut got: u64 = 0;
            // Default seeded from ReconnectConfig::production_defaults().
            assert_eq!(
                super::tdx_config_get_reconnect_wait_rate_limited_ms(cfg, &mut got),
                0
            );
            assert_eq!(got, 130_000);
            for ms in [0u64, 1, 30_000, 130_000, 600_000, u64::MAX] {
                super::tdx_config_set_reconnect_wait_rate_limited_ms(cfg, ms);
                assert_eq!((*cfg).inner.reconnect.wait_rate_limited_ms, ms);
                assert_eq!(
                    super::tdx_config_get_reconnect_wait_rate_limited_ms(cfg, &mut got),
                    0
                );
                assert_eq!(got, ms);
            }
            super::tdx_config_free(cfg);
        }
    }

    #[test]
    fn reconnect_wait_ms_null_handle_returns_minus_one() {
        // SAFETY: passing null to tdx_config_* is the documented FFI
        // contract — getter returns sentinel, setter no-ops.
        unsafe {
            let mut got: u64 = 42;
            assert_eq!(
                super::tdx_config_get_reconnect_wait_ms(std::ptr::null(), &mut got),
                -1
            );
            assert_eq!(
                super::tdx_config_get_reconnect_wait_rate_limited_ms(std::ptr::null(), &mut got),
                -1
            );
            super::tdx_config_set_reconnect_wait_ms(std::ptr::null_mut(), 1_234);
            super::tdx_config_set_reconnect_wait_rate_limited_ms(std::ptr::null_mut(), 1_234);
        }
    }
}

#[cfg(test)]
mod runtime_setter_tests {
    //! Offline tests for the `RuntimeConfig.tokio_worker_threads`
    //! setter/getter pair on the FFI surface — cross-binding parity
    //! with Python / TypeScript / C++. The `(has_value, n)` shape
    //! preserves `Some(0)` across the C boundary the same way the
    //! decode-pipeline setters do.

    #[test]
    fn tokio_worker_threads_explicit_round_trips_via_getter() {
        let cfg = super::tdx_config_production();
        // SAFETY: handle just returned by tdx_config_production.
        unsafe {
            // None sentinel (default).
            let mut got_has = true;
            let mut got_n: usize = 99;
            assert_eq!(
                super::tdx_config_get_tokio_worker_threads(cfg, &mut got_has, &mut got_n),
                0
            );
            assert!(!got_has, "default tokio_worker_threads must be None");
            assert_eq!(got_n, 0);

            // Explicit values round-trip including the Some(0) sentinel.
            for n in [0usize, 1, 2, 4, 8, 16, 32, 64] {
                let rc = super::tdx_config_set_tokio_worker_threads_explicit(cfg, true, n);
                assert_eq!(rc, 0);
                assert_eq!((*cfg).inner.runtime.tokio_worker_threads, Some(n));
                assert_eq!(
                    super::tdx_config_get_tokio_worker_threads(cfg, &mut got_has, &mut got_n),
                    0
                );
                assert!(got_has);
                assert_eq!(got_n, n);
            }

            // Reset to None.
            let rc = super::tdx_config_set_tokio_worker_threads_explicit(cfg, false, 999);
            assert_eq!(rc, 0);
            assert_eq!((*cfg).inner.runtime.tokio_worker_threads, None);
            super::tdx_config_free(cfg);
        }
    }

    #[test]
    fn tokio_worker_threads_null_handle_returns_minus_one() {
        // SAFETY: passing null to tdx_config_* is the documented FFI
        // contract — getter returns sentinel, setter no-ops.
        unsafe {
            let rc =
                super::tdx_config_set_tokio_worker_threads_explicit(std::ptr::null_mut(), true, 4);
            assert_eq!(rc, -1);
            let mut got_has = false;
            let mut got_n: usize = 0;
            assert_eq!(
                super::tdx_config_get_tokio_worker_threads(
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
        let cfg = super::tdx_config_production();
        // SAFETY: handle just returned by tdx_config_production.
        unsafe {
            let mut got: u64 = 0;
            // Default seeded by RetryPolicy::default().
            assert_eq!(
                super::tdx_config_get_retry_initial_delay_ms(cfg, &mut got),
                0
            );
            assert_eq!(got, 250);
            for ms in [0u64, 1, 100, 250, 2_000, 60_000] {
                super::tdx_config_set_retry_initial_delay_ms(cfg, ms);
                assert_eq!(
                    super::tdx_config_get_retry_initial_delay_ms(cfg, &mut got),
                    0
                );
                assert_eq!(got, ms);
            }
            super::tdx_config_free(cfg);
        }
    }

    #[test]
    fn retry_max_delay_ms_round_trips_via_getter() {
        let cfg = super::tdx_config_production();
        // SAFETY: handle just returned by tdx_config_production.
        unsafe {
            let mut got: u64 = 0;
            assert_eq!(super::tdx_config_get_retry_max_delay_ms(cfg, &mut got), 0);
            assert_eq!(got, 30_000);
            for ms in [0u64, 1, 1_000, 30_000, 300_000] {
                super::tdx_config_set_retry_max_delay_ms(cfg, ms);
                assert_eq!(super::tdx_config_get_retry_max_delay_ms(cfg, &mut got), 0);
                assert_eq!(got, ms);
            }
            super::tdx_config_free(cfg);
        }
    }

    #[test]
    fn retry_max_attempts_round_trips_via_getter() {
        let cfg = super::tdx_config_production();
        // SAFETY: handle just returned by tdx_config_production.
        unsafe {
            let mut got: u32 = 0;
            assert_eq!(super::tdx_config_get_retry_max_attempts(cfg, &mut got), 0);
            assert_eq!(got, 5);
            for n in [0u32, 1, 3, 5, 10, 100] {
                super::tdx_config_set_retry_max_attempts(cfg, n);
                assert_eq!(super::tdx_config_get_retry_max_attempts(cfg, &mut got), 0);
                assert_eq!(got, n);
            }
            super::tdx_config_free(cfg);
        }
    }

    #[test]
    fn retry_jitter_round_trips_via_getter() {
        let cfg = super::tdx_config_production();
        // SAFETY: handle just returned by tdx_config_production.
        unsafe {
            let mut got = false;
            assert_eq!(super::tdx_config_get_retry_jitter(cfg, &mut got), 0);
            assert!(got, "default jitter is true");
            super::tdx_config_set_retry_jitter(cfg, false);
            assert_eq!(super::tdx_config_get_retry_jitter(cfg, &mut got), 0);
            assert!(!got);
            super::tdx_config_set_retry_jitter(cfg, true);
            assert_eq!(super::tdx_config_get_retry_jitter(cfg, &mut got), 0);
            assert!(got);
            super::tdx_config_free(cfg);
        }
    }

    #[test]
    fn retry_setters_null_handle_returns_minus_one_or_noop() {
        // SAFETY: passing null to tdx_config_* is the documented FFI
        // contract — getter returns sentinel, setter no-ops.
        unsafe {
            super::tdx_config_set_retry_initial_delay_ms(std::ptr::null_mut(), 100);
            super::tdx_config_set_retry_max_delay_ms(std::ptr::null_mut(), 1_000);
            super::tdx_config_set_retry_max_attempts(std::ptr::null_mut(), 3);
            super::tdx_config_set_retry_jitter(std::ptr::null_mut(), false);
            let mut got_ms: u64 = 0;
            let mut got_n: u32 = 0;
            let mut got_b = false;
            assert_eq!(
                super::tdx_config_get_retry_initial_delay_ms(std::ptr::null(), &mut got_ms),
                -1
            );
            assert_eq!(
                super::tdx_config_get_retry_max_delay_ms(std::ptr::null(), &mut got_ms),
                -1
            );
            assert_eq!(
                super::tdx_config_get_retry_max_attempts(std::ptr::null(), &mut got_n),
                -1
            );
            assert_eq!(
                super::tdx_config_get_retry_jitter(std::ptr::null(), &mut got_b),
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
        let cfg = super::tdx_config_production();
        // SAFETY: handle just returned by tdx_config_production.
        unsafe {
            super::tdx_config_set_retry_initial_delay_ms(cfg, 500);
            super::tdx_config_set_retry_max_delay_ms(cfg, 60_000);
            super::tdx_config_set_retry_max_attempts(cfg, 7);
            super::tdx_config_set_retry_jitter(cfg, false);
            let retry = &(*cfg).inner.retry;
            assert_eq!(retry.initial_delay, std::time::Duration::from_millis(500));
            assert_eq!(retry.max_delay, std::time::Duration::from_millis(60_000));
            assert_eq!(retry.max_attempts, 7);
            assert!(!retry.jitter);
            super::tdx_config_free(cfg);
        }
    }
}

#[cfg(test)]
mod decode_pipeline_tests {
    //! Offline tests for the two-stage decode pipeline setters /
    //! getters on the FFI surface.
    //!
    //! The widened `_explicit(has_value, n)` setters preserve
    //! `Some(0)` across the C boundary (matches Python / TS
    //! semantics); the legacy n-only setters map `n=0` to `None`.
    //! The getters round-trip both shapes verbatim.

    use crate::error::tdx_last_error;
    use std::ffi::CStr;

    /// Snapshot the current TLS error string for assertions below.
    fn last_error_text() -> String {
        // SAFETY: `tdx_last_error` always returns a valid C string
        // pointer (potentially the empty-string sentinel).
        unsafe {
            let p = tdx_last_error();
            if p.is_null() {
                return String::new();
            }
            CStr::from_ptr(p).to_string_lossy().into_owned()
        }
    }

    #[test]
    fn decode_threads_explicit_preserves_some_zero() {
        let cfg = super::tdx_config_production();
        assert!(!cfg.is_null());
        // SAFETY: handle just returned by tdx_config_production.
        unsafe {
            let rc = super::tdx_config_set_decode_threads_explicit(cfg, true, 0);
            assert_eq!(rc, 0);
            assert_eq!(
                (*cfg).inner.mdds.decode_threads,
                Some(0),
                "has_value=true, n=0 must encode Some(0), not None",
            );
            // None sentinel: has_value=false ignores n.
            let rc = super::tdx_config_set_decode_threads_explicit(cfg, false, 99);
            assert_eq!(rc, 0);
            assert_eq!((*cfg).inner.mdds.decode_threads, None);
            super::tdx_config_free(cfg);
        }
    }

    #[test]
    fn decode_threads_legacy_n_only_maps_zero_to_none() {
        let cfg = super::tdx_config_production();
        // SAFETY: handle just returned by tdx_config_production.
        unsafe {
            (*cfg).inner.mdds.decode_threads = Some(8);
            let rc = super::tdx_config_set_decode_threads(cfg, 0);
            assert_eq!(rc, 0);
            assert_eq!((*cfg).inner.mdds.decode_threads, None);
            let rc = super::tdx_config_set_decode_threads(cfg, 16);
            assert_eq!(rc, 0);
            assert_eq!((*cfg).inner.mdds.decode_threads, Some(16));
            super::tdx_config_free(cfg);
        }
    }

    #[test]
    fn decode_threads_explicit_round_trips_via_getter() {
        let cfg = super::tdx_config_production();
        // SAFETY: handle just returned by tdx_config_production.
        unsafe {
            for n in [0usize, 1, 2, 4, 8, 16, 32, 64, 4096] {
                let rc = super::tdx_config_set_decode_threads_explicit(cfg, true, n);
                assert_eq!(rc, 0);
                let mut got_has = false;
                let mut got_n = 0usize;
                let grc = super::tdx_config_get_decode_threads(cfg, &mut got_has, &mut got_n);
                assert_eq!(grc, 0);
                assert!(got_has, "getter must report Some for n={n}");
                assert_eq!(got_n, n, "getter must round-trip n={n}");
            }
            // None round-trip.
            let rc = super::tdx_config_set_decode_threads_explicit(cfg, false, 0);
            assert_eq!(rc, 0);
            let mut got_has = true;
            let mut got_n = 99usize;
            let grc = super::tdx_config_get_decode_threads(cfg, &mut got_has, &mut got_n);
            assert_eq!(grc, 0);
            assert!(!got_has, "getter must report None");
            assert_eq!(got_n, 0, "getter must zero out_n on None");
            super::tdx_config_free(cfg);
        }
    }

    #[test]
    fn decode_threads_null_handle_returns_minus_one() {
        // SAFETY: passing null to tdx_config_* is the documented FFI
        // contract — getter returns sentinel, setter no-ops.
        unsafe {
            let rc = super::tdx_config_set_decode_threads_explicit(std::ptr::null_mut(), true, 4);
            assert_eq!(rc, -1);
        }
        let msg = last_error_text();
        assert!(
            msg.contains("null"),
            "null-handle diagnostic must mention the failure mode, got {msg:?}"
        );
    }

    #[test]
    fn decode_threads_getter_null_handle_returns_minus_one() {
        // SAFETY: passing null to tdx_config_* is the documented FFI
        // contract — getter returns sentinel, setter no-ops.
        unsafe {
            let mut hv = false;
            let mut n = 0usize;
            let rc = super::tdx_config_get_decode_threads(std::ptr::null(), &mut hv, &mut n);
            assert_eq!(rc, -1);
        }
    }

    #[test]
    fn decode_queue_depth_explicit_preserves_some_zero() {
        let cfg = super::tdx_config_production();
        // SAFETY: handle just returned by tdx_config_production.
        unsafe {
            let rc = super::tdx_config_set_decode_queue_depth_explicit(cfg, true, 0);
            assert_eq!(rc, 0);
            assert_eq!((*cfg).inner.mdds.decode_queue_depth, Some(0));
            let rc = super::tdx_config_set_decode_queue_depth_explicit(cfg, false, 0);
            assert_eq!(rc, 0);
            assert_eq!((*cfg).inner.mdds.decode_queue_depth, None);
            super::tdx_config_free(cfg);
        }
    }

    #[test]
    fn decode_queue_depth_legacy_n_only_maps_zero_to_none() {
        let cfg = super::tdx_config_production();
        // SAFETY: handle just returned by tdx_config_production.
        unsafe {
            (*cfg).inner.mdds.decode_queue_depth = Some(1024);
            let rc = super::tdx_config_set_decode_queue_depth(cfg, 0);
            assert_eq!(rc, 0);
            assert_eq!((*cfg).inner.mdds.decode_queue_depth, None);
            super::tdx_config_free(cfg);
        }
    }

    #[test]
    fn decode_queue_depth_explicit_round_trips_via_getter() {
        let cfg = super::tdx_config_production();
        // SAFETY: handle just returned by tdx_config_production.
        unsafe {
            for n in [0usize, 1, 64, 128, 512, 2048, 8192, 65536] {
                let rc = super::tdx_config_set_decode_queue_depth_explicit(cfg, true, n);
                assert_eq!(rc, 0);
                let mut got_has = false;
                let mut got_n = 0usize;
                let grc = super::tdx_config_get_decode_queue_depth(cfg, &mut got_has, &mut got_n);
                assert_eq!(grc, 0);
                assert!(got_has);
                assert_eq!(got_n, n);
            }
            super::tdx_config_free(cfg);
        }
    }

    #[test]
    fn decode_queue_depth_null_handle_returns_minus_one() {
        // SAFETY: passing null to tdx_config_* is the documented FFI
        // contract — getter returns sentinel, setter no-ops.
        unsafe {
            let rc =
                super::tdx_config_set_decode_queue_depth_explicit(std::ptr::null_mut(), true, 1024);
            assert_eq!(rc, -1);
        }
        let msg = last_error_text();
        assert!(msg.contains("null"));
    }

    #[test]
    fn decode_pipeline_setters_compose_with_pool_sizing() {
        let cfg = super::tdx_config_production();
        // SAFETY: handle just returned by tdx_config_production.
        unsafe {
            super::tdx_config_set_concurrent_requests(cfg, 8);
            super::tdx_config_set_decoder_ring_size(cfg, 1024);
            assert_eq!(
                super::tdx_config_set_decode_threads_explicit(cfg, true, 16),
                0
            );
            assert_eq!(
                super::tdx_config_set_decode_queue_depth_explicit(cfg, true, 4096),
                0
            );
            assert_eq!((*cfg).inner.mdds.concurrent_requests, 8);
            assert_eq!((*cfg).inner.mdds.decoder_ring_size, 1024);
            assert_eq!((*cfg).inner.mdds.decode_threads, Some(16));
            assert_eq!((*cfg).inner.mdds.decode_queue_depth, Some(4096));
            super::tdx_config_free(cfg);
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
        let cfg = super::tdx_config_production();
        assert!(!cfg.is_null());
        // SAFETY: handle just returned by tdx_config_production.
        unsafe {
            let mut got: u32 = 0;
            // Default seeded from FlatFilesConfig::production_defaults().
            assert_eq!(
                super::tdx_config_get_flatfiles_max_attempts(cfg, &mut got),
                0
            );
            assert_eq!(got, 3);
            for n in [0u32, 1, 3, 5, 10, 100] {
                super::tdx_config_set_flatfiles_max_attempts(cfg, n);
                assert_eq!((*cfg).inner.flatfiles.max_attempts, n);
                assert_eq!(
                    super::tdx_config_get_flatfiles_max_attempts(cfg, &mut got),
                    0
                );
                assert_eq!(got, n);
            }
            super::tdx_config_free(cfg);
        }
    }

    #[test]
    fn flatfiles_initial_backoff_secs_round_trips() {
        let cfg = super::tdx_config_production();
        // SAFETY: handle just returned by tdx_config_production.
        unsafe {
            let mut got: u64 = 0;
            // Default seeded from FlatFilesConfig::production_defaults().
            assert_eq!(
                super::tdx_config_get_flatfiles_initial_backoff_secs(cfg, &mut got),
                0
            );
            assert_eq!(got, 1);
            for secs in [0u64, 1, 2, 4, 10, 60, 3600] {
                super::tdx_config_set_flatfiles_initial_backoff_secs(cfg, secs);
                assert_eq!(
                    (*cfg).inner.flatfiles.initial_backoff,
                    std::time::Duration::from_secs(secs),
                );
                assert_eq!(
                    super::tdx_config_get_flatfiles_initial_backoff_secs(cfg, &mut got),
                    0
                );
                assert_eq!(got, secs);
            }
            super::tdx_config_free(cfg);
        }
    }

    #[test]
    fn flatfiles_max_backoff_secs_round_trips() {
        let cfg = super::tdx_config_production();
        // SAFETY: handle just returned by tdx_config_production.
        unsafe {
            let mut got: u64 = 0;
            // Default seeded from FlatFilesConfig::production_defaults().
            assert_eq!(
                super::tdx_config_get_flatfiles_max_backoff_secs(cfg, &mut got),
                0
            );
            assert_eq!(got, 4);
            for secs in [0u64, 1, 4, 10, 60, 3600, 86_400] {
                super::tdx_config_set_flatfiles_max_backoff_secs(cfg, secs);
                assert_eq!(
                    (*cfg).inner.flatfiles.max_backoff,
                    std::time::Duration::from_secs(secs),
                );
                assert_eq!(
                    super::tdx_config_get_flatfiles_max_backoff_secs(cfg, &mut got),
                    0
                );
                assert_eq!(got, secs);
            }
            super::tdx_config_free(cfg);
        }
    }

    #[test]
    fn flatfiles_setters_null_handle_returns_minus_one_or_noop() {
        // SAFETY: passing null to tdx_config_* is the documented FFI
        // contract — getter returns sentinel, setter no-ops.
        unsafe {
            super::tdx_config_set_flatfiles_max_attempts(std::ptr::null_mut(), 3);
            super::tdx_config_set_flatfiles_initial_backoff_secs(std::ptr::null_mut(), 1);
            super::tdx_config_set_flatfiles_max_backoff_secs(std::ptr::null_mut(), 4);
            let mut got_n: u32 = 0;
            let mut got_secs: u64 = 0;
            assert_eq!(
                super::tdx_config_get_flatfiles_max_attempts(std::ptr::null(), &mut got_n),
                -1
            );
            assert_eq!(
                super::tdx_config_get_flatfiles_initial_backoff_secs(
                    std::ptr::null(),
                    &mut got_secs
                ),
                -1
            );
            assert_eq!(
                super::tdx_config_get_flatfiles_max_backoff_secs(std::ptr::null(), &mut got_secs),
                -1
            );
        }
    }

    #[test]
    fn flatfiles_field_setters_compose_into_consistent_config() {
        // After mutating all three fields the `DirectConfig.flatfiles`
        // struct must reflect the composed shape — proves the setters
        // target the same underlying field rather than duplicating state.
        let cfg = super::tdx_config_production();
        // SAFETY: handle just returned by tdx_config_production.
        unsafe {
            super::tdx_config_set_flatfiles_max_attempts(cfg, 5);
            super::tdx_config_set_flatfiles_initial_backoff_secs(cfg, 2);
            super::tdx_config_set_flatfiles_max_backoff_secs(cfg, 30);
            let ff = &(*cfg).inner.flatfiles;
            assert_eq!(ff.max_attempts, 5);
            assert_eq!(ff.initial_backoff, std::time::Duration::from_secs(2));
            assert_eq!(ff.max_backoff, std::time::Duration::from_secs(30));
            super::tdx_config_free(cfg);
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
    //! caller frees with `tdx_string_free`); `MetricsConfig.port` is
    //! `Option<u16>` carried as the widened `(has_value, port)` shape.

    use crate::types::tdx_string_free;
    use std::ffi::{CStr, CString};

    /// Read a `*mut c_char` getter result into an owned `String` and
    /// release the heap allocation via `tdx_string_free`.
    fn take_owned(p: *mut std::os::raw::c_char) -> Option<String> {
        if p.is_null() {
            return None;
        }
        // SAFETY: `p` is a non-null pointer just returned by a
        // `tdx_config_get_*` getter (produced by CString::into_raw);
        // it is read once and then handed back to tdx_string_free.
        let owned = unsafe { CStr::from_ptr(p) }.to_string_lossy().into_owned();
        // SAFETY: `p` was produced by CString::into_raw; tdx_string_free
        // reclaims it via CString::from_raw exactly once.
        unsafe { tdx_string_free(p) };
        Some(owned)
    }

    #[test]
    fn nexus_url_round_trips_via_getter() {
        let cfg = super::tdx_config_production();
        assert!(!cfg.is_null());
        // SAFETY: handle just returned by tdx_config_production.
        unsafe {
            // Default seeded from AuthConfig::production_defaults().
            let got = take_owned(super::tdx_config_get_nexus_url(cfg));
            assert_eq!(
                got.as_deref(),
                Some("https://nexus-api.thetadata.us/identity/terminal/auth_user"),
            );
            let url = CString::new("https://staging.example.invalid/auth").unwrap();
            assert_eq!(super::tdx_config_set_nexus_url(cfg, url.as_ptr()), 0);
            assert_eq!(
                (*cfg).inner.auth.nexus_url,
                "https://staging.example.invalid/auth"
            );
            let got = take_owned(super::tdx_config_get_nexus_url(cfg));
            assert_eq!(got.as_deref(), Some("https://staging.example.invalid/auth"));
            super::tdx_config_free(cfg);
        }
    }

    #[test]
    fn client_type_round_trips_via_getter() {
        let cfg = super::tdx_config_production();
        // SAFETY: handle just returned by tdx_config_production.
        unsafe {
            // Default seeded from AuthConfig::production_defaults().
            let got = take_owned(super::tdx_config_get_client_type(cfg));
            assert_eq!(got.as_deref(), Some("rust-thetadatadx"));
            let ct = CString::new("fleet-east-1").unwrap();
            assert_eq!(super::tdx_config_set_client_type(cfg, ct.as_ptr()), 0);
            assert_eq!((*cfg).inner.auth.client_type, "fleet-east-1");
            let got = take_owned(super::tdx_config_get_client_type(cfg));
            assert_eq!(got.as_deref(), Some("fleet-east-1"));
            super::tdx_config_free(cfg);
        }
    }

    #[test]
    fn nexus_url_rejects_null_and_leaves_config_unchanged() {
        let cfg = super::tdx_config_production();
        // SAFETY: handle just returned by tdx_config_production.
        let baseline = unsafe { (*cfg).inner.auth.nexus_url.clone() };
        // SAFETY: handle just returned by tdx_config_production.
        unsafe {
            assert_eq!(
                super::tdx_config_set_nexus_url(cfg, std::ptr::null()),
                -1,
                "null url must be rejected with -1",
            );
            assert_eq!((*cfg).inner.auth.nexus_url, baseline);
            super::tdx_config_free(cfg);
        }
    }

    #[test]
    fn auth_string_setters_null_handle_returns_minus_one() {
        // SAFETY: passing null to tdx_config_* is the documented FFI
        // contract — string setters return -1, string getters null.
        unsafe {
            let url = CString::new("x").unwrap();
            assert_eq!(
                super::tdx_config_set_nexus_url(std::ptr::null_mut(), url.as_ptr()),
                -1
            );
            assert_eq!(
                super::tdx_config_set_client_type(std::ptr::null_mut(), url.as_ptr()),
                -1
            );
            assert!(super::tdx_config_get_nexus_url(std::ptr::null()).is_null());
            assert!(super::tdx_config_get_client_type(std::ptr::null()).is_null());
        }
    }

    #[test]
    fn metrics_port_round_trips_via_getter() {
        let cfg = super::tdx_config_production();
        // SAFETY: handle just returned by tdx_config_production.
        unsafe {
            // Default seeded from MetricsConfig::default() — None.
            let mut got_has = true;
            let mut got_port: u16 = 99;
            assert_eq!(
                super::tdx_config_get_metrics_port(cfg, &mut got_has, &mut got_port),
                0
            );
            assert!(!got_has, "default metrics.port must be None");
            assert_eq!(got_port, 0);

            for port in [0u16, 1, 9090, 9100, u16::MAX] {
                assert_eq!(super::tdx_config_set_metrics_port(cfg, true, port), 0);
                assert_eq!((*cfg).inner.metrics.port, Some(port));
                assert_eq!(
                    super::tdx_config_get_metrics_port(cfg, &mut got_has, &mut got_port),
                    0
                );
                assert!(got_has);
                assert_eq!(got_port, port);
            }

            // Reset to None.
            assert_eq!(super::tdx_config_set_metrics_port(cfg, false, 9090), 0);
            assert_eq!((*cfg).inner.metrics.port, None);
            assert_eq!(
                super::tdx_config_get_metrics_port(cfg, &mut got_has, &mut got_port),
                0
            );
            assert!(!got_has);
            assert_eq!(got_port, 0);
            super::tdx_config_free(cfg);
        }
    }

    #[test]
    fn metrics_port_null_handle_returns_minus_one() {
        // SAFETY: passing null to tdx_config_* is the documented FFI
        // contract — getter returns sentinel, setter returns -1.
        unsafe {
            assert_eq!(
                super::tdx_config_set_metrics_port(std::ptr::null_mut(), true, 9090),
                -1
            );
            let mut got_has = false;
            let mut got_port: u16 = 0;
            assert_eq!(
                super::tdx_config_get_metrics_port(std::ptr::null(), &mut got_has, &mut got_port),
                -1
            );
        }
    }
}
