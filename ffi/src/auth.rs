//! Credentials, config, and historical-client lifecycle: `tdx_credentials_*`,
//! `tdx_config_*`, `tdx_mdds_client_connect` / `tdx_mdds_client_free`.

use std::os::raw::c_char;
use std::ptr;

use crate::error::{cstr_to_str, set_error, set_error_from};
use crate::runtime;
use crate::types::{TdxConfig, TdxCredentials, TdxMddsClient};

// ── Credentials ──

/// Create credentials from email and password strings.
///
/// Returns null on invalid input (check `tdx_last_error()`).
#[no_mangle]
pub unsafe extern "C" fn tdx_credentials_from_email(
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
/// Returns `0` on success. Returns `-1` and sets `tdx_last_error` when
/// `mode` is outside the documented `{0, 1}` set or when `config` is
/// null. A rejected `mode` value carries
/// `tdx_last_error_code = TDX_ERR_INVALID_PARAMETER` (the same typed
/// class the Python / TypeScript bindings raise for a bad enum value);
/// a null `config` carries `TDX_ERR_CONFIG`.
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
                    crate::error::TDX_ERR_INVALID_PARAMETER,
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

/// Read the configured FPSS flush mode. Same encoding as
/// `tdx_config_set_flush_mode`: writes `0` (`Batched`) or `1`
/// (`Immediate`) into `*out_mode`. Returns `0` on success, `-1` if
/// either pointer is null.
#[no_mangle]
pub unsafe extern "C" fn tdx_config_get_flush_mode(
    config: *const TdxConfig,
    out_mode: *mut i32,
) -> i32 {
    ffi_boundary!(-1, {
        if config.is_null() || out_mode.is_null() {
            set_error("config or out-parameter pointer is null");
            return -1;
        }
        // SAFETY: config is a non-null `*const TdxConfig` returned by `tdx_config_*` and not yet freed; `&*` produces a shared reference valid for the call duration.
        let config = unsafe { &*config };
        let value = match config.inner.fpss.flush_mode {
            thetadatadx::FpssFlushMode::Batched => 0,
            thetadatadx::FpssFlushMode::Immediate => 1,
            _ => 0,
        };
        // SAFETY: out pointer checked non-null above; the FFI contract pins the storage for the call duration and forbids concurrent calls on the same handle.
        unsafe {
            *out_mode = value;
        }
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
///
/// Returns `0` on success. Returns `-1` and sets `tdx_last_error` /
/// `tdx_last_error_code = TDX_ERR_INVALID_PARAMETER` when `policy` is
/// outside the documented `{0, 1}` set, so an unknown policy is rejected
/// with the same typed class the Python / TypeScript bindings raise
/// rather than being silently coerced to `Auto`. A null `config` is
/// rejected with `TDX_ERR_CONFIG`.
#[no_mangle]
pub unsafe extern "C" fn tdx_config_set_reconnect_policy(
    config: *mut TdxConfig,
    policy: i32,
) -> i32 {
    ffi_boundary!(-1, {
        if config.is_null() {
            crate::error::set_error_with_code(
                "tdx_config_set_reconnect_policy: config handle is null",
                crate::error::TDX_ERR_CONFIG,
            );
            return -1;
        }
        let value = match policy {
            0 => thetadatadx::ReconnectPolicy::Auto(thetadatadx::ReconnectAttemptLimits::default()),
            1 => thetadatadx::ReconnectPolicy::Manual,
            other => {
                crate::error::set_error_with_code(
                    &format!(
                        "tdx_config_set_reconnect_policy: invalid policy {other}; expected 0 (Auto) or 1 (Manual)"
                    ),
                    crate::error::TDX_ERR_INVALID_PARAMETER,
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
        config.inner.reconnect.policy = value;
        0
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

// ── Reconnect budget readback (Auto-policy limits) ─────────────────
//
// Getters mirroring the existing `tdx_config_set_reconnect_*` family
// so operator dashboards can read the configured policy back out of a
// handle. When the policy is `Manual` or `Custom`, the limits getters
// write the default-limits values: the per-class budgets only apply
// under the `Auto` policy and the setters are no-ops there too.

/// Read the configured reconnect policy selector.
///
/// Writes `0` (`Auto`), `1` (`Manual`), or `2` (`Custom`) into
/// `*out_policy`. Returns `0` on success, `-1` if either pointer is
/// null.
#[no_mangle]
pub unsafe extern "C" fn tdx_config_get_reconnect_policy(
    config: *const TdxConfig,
    out_policy: *mut i32,
) -> i32 {
    ffi_boundary!(-1, {
        if config.is_null() || out_policy.is_null() {
            set_error("config or out-parameter pointer is null");
            return -1;
        }
        // SAFETY: config is a non-null `*const TdxConfig` returned by `tdx_config_*` and not yet freed; `&*` produces a shared reference valid for the call duration.
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

/// Read the generic-transient reconnect attempt budget. Default `30`.
///
/// Writes the configured value into `*out`. When the reconnect policy
/// is not `Auto`, writes the default-limits value (the budgets only
/// apply under the `Auto` policy). Returns `0` on success, `-1` if
/// either pointer is null.
#[no_mangle]
pub unsafe extern "C" fn tdx_config_get_reconnect_max_attempts(
    config: *const TdxConfig,
    out: *mut u32,
) -> i32 {
    ffi_boundary!(-1, {
        if config.is_null() || out.is_null() {
            set_error("config or out-parameter pointer is null");
            return -1;
        }
        // SAFETY: config is a non-null `*const TdxConfig` returned by `tdx_config_*` and not yet freed; `&*` produces a shared reference valid for the call duration.
        let config = unsafe { &*config };
        let value = match &config.inner.reconnect.policy {
            thetadatadx::ReconnectPolicy::Auto(limits) => limits.max_attempts,
            _ => thetadatadx::ReconnectAttemptLimits::default().max_attempts,
        };
        // SAFETY: out pointer checked non-null above; the FFI contract pins the storage for the call duration and forbids concurrent calls on the same handle.
        unsafe {
            *out = value;
        }
        0
    })
}

/// Read the rate-limited (`TooManyRequests`) reconnect attempt budget. Default `100`.
///
/// Writes the configured value into `*out`. When the reconnect policy
/// is not `Auto`, writes the default-limits value (the budgets only
/// apply under the `Auto` policy). Returns `0` on success, `-1` if
/// either pointer is null.
#[no_mangle]
pub unsafe extern "C" fn tdx_config_get_reconnect_max_rate_limited_attempts(
    config: *const TdxConfig,
    out: *mut u32,
) -> i32 {
    ffi_boundary!(-1, {
        if config.is_null() || out.is_null() {
            set_error("config or out-parameter pointer is null");
            return -1;
        }
        // SAFETY: config is a non-null `*const TdxConfig` returned by `tdx_config_*` and not yet freed; `&*` produces a shared reference valid for the call duration.
        let config = unsafe { &*config };
        let value = match &config.inner.reconnect.policy {
            thetadatadx::ReconnectPolicy::Auto(limits) => limits.max_rate_limited_attempts,
            _ => thetadatadx::ReconnectAttemptLimits::default().max_rate_limited_attempts,
        };
        // SAFETY: out pointer checked non-null above; the FFI contract pins the storage for the call duration and forbids concurrent calls on the same handle.
        unsafe {
            *out = value;
        }
        0
    })
}

/// Set the `ServerRestarting` reconnect attempt budget. Default `60`. No effect unless the reconnect policy is `Auto`.
#[no_mangle]
pub unsafe extern "C" fn tdx_config_set_reconnect_max_server_restart_attempts(
    config: *mut TdxConfig,
    n: u32,
) {
    ffi_boundary!((), {
        let config = require_config_mut!(config);
        if let thetadatadx::ReconnectPolicy::Auto(ref mut limits) = config.inner.reconnect.policy {
            limits.max_server_restart_attempts = n;
        }
    })
}

/// Read the `ServerRestarting` reconnect attempt budget. Default `60`.
///
/// Writes the configured value into `*out`. When the reconnect policy
/// is not `Auto`, writes the default-limits value (the budgets only
/// apply under the `Auto` policy). Returns `0` on success, `-1` if
/// either pointer is null.
#[no_mangle]
pub unsafe extern "C" fn tdx_config_get_reconnect_max_server_restart_attempts(
    config: *const TdxConfig,
    out: *mut u32,
) -> i32 {
    ffi_boundary!(-1, {
        if config.is_null() || out.is_null() {
            set_error("config or out-parameter pointer is null");
            return -1;
        }
        // SAFETY: config is a non-null `*const TdxConfig` returned by `tdx_config_*` and not yet freed; `&*` produces a shared reference valid for the call duration.
        let config = unsafe { &*config };
        let value = match &config.inner.reconnect.policy {
            thetadatadx::ReconnectPolicy::Auto(limits) => limits.max_server_restart_attempts,
            _ => thetadatadx::ReconnectAttemptLimits::default().max_server_restart_attempts,
        };
        // SAFETY: out pointer checked non-null above; the FFI contract pins the storage for the call duration and forbids concurrent calls on the same handle.
        unsafe {
            *out = value;
        }
        0
    })
}

/// Read the stable-window reset interval (seconds). Default `60`.
///
/// Writes the configured value into `*out`. When the reconnect policy
/// is not `Auto`, writes the default-limits value (the budgets only
/// apply under the `Auto` policy). Returns `0` on success, `-1` if
/// either pointer is null.
#[no_mangle]
pub unsafe extern "C" fn tdx_config_get_reconnect_stable_window_secs(
    config: *const TdxConfig,
    out: *mut u64,
) -> i32 {
    ffi_boundary!(-1, {
        if config.is_null() || out.is_null() {
            set_error("config or out-parameter pointer is null");
            return -1;
        }
        // SAFETY: config is a non-null `*const TdxConfig` returned by `tdx_config_*` and not yet freed; `&*` produces a shared reference valid for the call duration.
        let config = unsafe { &*config };
        let value = match &config.inner.reconnect.policy {
            thetadatadx::ReconnectPolicy::Auto(limits) => limits.stable_window,
            _ => thetadatadx::ReconnectAttemptLimits::default().stable_window,
        };
        // SAFETY: out pointer checked non-null above; the FFI contract pins the storage for the call duration and forbids concurrent calls on the same handle.
        unsafe {
            *out = value.as_secs();
        }
        0
    })
}

/// Set the wall-clock reconnect envelope (seconds) for the generic-transient and server-restart classes, measured from the first attempt of a consecutive-reconnect sequence. `0` disables the envelope (attempt budgets only). Default `300`. No effect unless the reconnect policy is `Auto`.
#[no_mangle]
pub unsafe extern "C" fn tdx_config_set_reconnect_max_elapsed_secs(
    config: *mut TdxConfig,
    secs: u64,
) {
    ffi_boundary!((), {
        let config = require_config_mut!(config);
        if let thetadatadx::ReconnectPolicy::Auto(ref mut limits) = config.inner.reconnect.policy {
            limits.max_elapsed = std::time::Duration::from_secs(secs);
        }
    })
}

/// Read the wall-clock reconnect envelope (seconds). `0` means the envelope is disabled. Default `300`.
///
/// Writes the configured value into `*out`. When the reconnect policy
/// is not `Auto`, writes the default-limits value (the budgets only
/// apply under the `Auto` policy). Returns `0` on success, `-1` if
/// either pointer is null.
#[no_mangle]
pub unsafe extern "C" fn tdx_config_get_reconnect_max_elapsed_secs(
    config: *const TdxConfig,
    out: *mut u64,
) -> i32 {
    ffi_boundary!(-1, {
        if config.is_null() || out.is_null() {
            set_error("config or out-parameter pointer is null");
            return -1;
        }
        // SAFETY: config is a non-null `*const TdxConfig` returned by `tdx_config_*` and not yet freed; `&*` produces a shared reference valid for the call duration.
        let config = unsafe { &*config };
        let value = match &config.inner.reconnect.policy {
            thetadatadx::ReconnectPolicy::Auto(limits) => limits.max_elapsed,
            _ => thetadatadx::ReconnectAttemptLimits::default().max_elapsed,
        };
        // SAFETY: out pointer checked non-null above; the FFI contract pins the storage for the call duration and forbids concurrent calls on the same handle.
        unsafe {
            *out = value.as_secs();
        }
        0
    })
}

// ── Reconnect cadence + replay pacing ──────────────────────────────

/// Set the cap (ms) on the exponential generic-transient reconnect ladder. The ladder starts at `reconnect_wait_ms` and doubles per consecutive attempt up to this value. Default `30_000`.
#[no_mangle]
pub unsafe extern "C" fn tdx_config_set_reconnect_wait_max_ms(config: *mut TdxConfig, v: u64) {
    ffi_boundary!((), {
        let config = require_config_mut!(config);
        config.inner.reconnect.wait_max_ms = v;
    })
}

/// Read the current reconnect `wait_max_ms` setting (default `30_000`).
///
/// Writes the configured value into `*out`. Returns `0` on success,
/// `-1` if either pointer is null.
#[no_mangle]
pub unsafe extern "C" fn tdx_config_get_reconnect_wait_max_ms(
    config: *const TdxConfig,
    out: *mut u64,
) -> i32 {
    ffi_boundary!(-1, {
        if config.is_null() || out.is_null() {
            set_error("config or out-parameter pointer is null");
            return -1;
        }
        // SAFETY: config is a non-null `*const TdxConfig` returned by `tdx_config_*` and not yet freed; `&*` produces a shared reference valid for the call duration.
        let config = unsafe { &*config };
        // SAFETY: out pointer checked non-null above; the FFI contract pins the storage for the call duration and forbids concurrent calls on the same handle.
        unsafe {
            *out = config.inner.reconnect.wait_max_ms;
        }
        0
    })
}

/// Set the flat reconnect cadence (ms) for `ServerRestarting` disconnects. Default `5_000`.
#[no_mangle]
pub unsafe extern "C" fn tdx_config_set_reconnect_wait_server_restart_ms(
    config: *mut TdxConfig,
    v: u64,
) {
    ffi_boundary!((), {
        let config = require_config_mut!(config);
        config.inner.reconnect.wait_server_restart_ms = v;
    })
}

/// Read the current reconnect `wait_server_restart_ms` setting (default `5_000`).
///
/// Writes the configured value into `*out`. Returns `0` on success,
/// `-1` if either pointer is null.
#[no_mangle]
pub unsafe extern "C" fn tdx_config_get_reconnect_wait_server_restart_ms(
    config: *const TdxConfig,
    out: *mut u64,
) -> i32 {
    ffi_boundary!(-1, {
        if config.is_null() || out.is_null() {
            set_error("config or out-parameter pointer is null");
            return -1;
        }
        // SAFETY: config is a non-null `*const TdxConfig` returned by `tdx_config_*` and not yet freed; `&*` produces a shared reference valid for the call duration.
        let config = unsafe { &*config };
        // SAFETY: out pointer checked non-null above; the FFI contract pins the storage for the call duration and forbids concurrent calls on the same handle.
        unsafe {
            *out = config.inner.reconnect.wait_server_restart_ms;
        }
        0
    })
}

/// Set the jitter strategy applied to every reconnect delay.
///
/// - `mode = 0`: Full (default) — sample uniformly from `[0, delay]`.
/// - `mode = 1`: Equal — `delay/2 + uniform(0, delay/2)`.
/// - `mode = 2`: Decorrelated — walk relative to the previous delay.
/// - `mode = 3`: None — deterministic delays (tests only).
///
/// Returns `0` on success. Returns `-1` and sets `tdx_last_error` when
/// `mode` is outside the documented `{0, 1, 2, 3}` set or `config` is
/// null. A rejected `mode` value carries
/// `tdx_last_error_code = TDX_ERR_INVALID_PARAMETER` so an out-of-domain
/// enum int surfaces the same typed class across every binding.
#[no_mangle]
pub unsafe extern "C" fn tdx_config_set_reconnect_jitter(config: *mut TdxConfig, mode: i32) -> i32 {
    ffi_boundary!(-1, {
        if config.is_null() {
            set_error("tdx_config_set_reconnect_jitter: config handle is null");
            return -1;
        }
        let value = match mode {
            0 => thetadatadx::JitterMode::Full,
            1 => thetadatadx::JitterMode::Equal,
            2 => thetadatadx::JitterMode::Decorrelated,
            3 => thetadatadx::JitterMode::None,
            other => {
                crate::error::set_error_with_code(
                    &format!(
                        "tdx_config_set_reconnect_jitter: invalid mode {other}; expected 0 (Full), 1 (Equal), 2 (Decorrelated), or 3 (None)"
                    ),
                    crate::error::TDX_ERR_INVALID_PARAMETER,
                );
                return -1;
            }
        };
        // SAFETY: config is a non-null pointer returned by `tdx_config_*` and not yet freed; `&mut *` produces a unique reference valid for the call duration because the caller owns the Box and the FFI contract forbids concurrent calls on the same handle.
        let config = unsafe { &mut *config };
        config.inner.reconnect.jitter = value;
        0
    })
}

/// Read the configured reconnect jitter mode. Same encoding as
/// `tdx_config_set_reconnect_jitter`. Returns `0` on success, `-1` if
/// either pointer is null.
#[no_mangle]
pub unsafe extern "C" fn tdx_config_get_reconnect_jitter(
    config: *const TdxConfig,
    out_mode: *mut i32,
) -> i32 {
    ffi_boundary!(-1, {
        if config.is_null() || out_mode.is_null() {
            set_error("config or out-parameter pointer is null");
            return -1;
        }
        // SAFETY: config is a non-null `*const TdxConfig` returned by `tdx_config_*` and not yet freed; `&*` produces a shared reference valid for the call duration.
        let config = unsafe { &*config };
        let value = match config.inner.reconnect.jitter {
            thetadatadx::JitterMode::Full => 0,
            thetadatadx::JitterMode::Equal => 1,
            thetadatadx::JitterMode::Decorrelated => 2,
            _ => 3,
        };
        // SAFETY: out pointer checked non-null above; the FFI contract pins the storage for the call duration and forbids concurrent calls on the same handle.
        unsafe {
            *out_mode = value;
        }
        0
    })
}

/// Set the subscription-replay burst size used after an
/// auto-reconnect: frames are written in bursts of this many, each
/// burst flushed and followed by a jittered `replay_pace_ms` pause.
/// Minimum `1` (validated at connect). Default `50`.
#[no_mangle]
pub unsafe extern "C" fn tdx_config_set_reconnect_replay_burst_size(
    config: *mut TdxConfig,
    n: u32,
) {
    ffi_boundary!((), {
        let config = require_config_mut!(config);
        config.inner.reconnect.replay_burst_size = n;
    })
}

/// Read the current `replay_burst_size` setting (default `50`).
///
/// Writes the configured value into `*out`. Returns `0` on success,
/// `-1` if either pointer is null.
#[no_mangle]
pub unsafe extern "C" fn tdx_config_get_reconnect_replay_burst_size(
    config: *const TdxConfig,
    out: *mut u32,
) -> i32 {
    ffi_boundary!(-1, {
        if config.is_null() || out.is_null() {
            set_error("config or out-parameter pointer is null");
            return -1;
        }
        // SAFETY: config is a non-null `*const TdxConfig` returned by `tdx_config_*` and not yet freed; `&*` produces a shared reference valid for the call duration.
        let config = unsafe { &*config };
        // SAFETY: out pointer checked non-null above; the FFI contract pins the storage for the call duration and forbids concurrent calls on the same handle.
        unsafe {
            *out = config.inner.reconnect.replay_burst_size;
        }
        0
    })
}

/// Set the pause (ms) between subscription-replay bursts after an auto-reconnect. `0` removes the pause. Default `5`.
#[no_mangle]
pub unsafe extern "C" fn tdx_config_set_reconnect_replay_pace_ms(config: *mut TdxConfig, v: u64) {
    ffi_boundary!((), {
        let config = require_config_mut!(config);
        config.inner.reconnect.replay_pace_ms = v;
    })
}

/// Read the current `replay_pace_ms` setting (default `5`).
///
/// Writes the configured value into `*out`. Returns `0` on success,
/// `-1` if either pointer is null.
#[no_mangle]
pub unsafe extern "C" fn tdx_config_get_reconnect_replay_pace_ms(
    config: *const TdxConfig,
    out: *mut u64,
) -> i32 {
    ffi_boundary!(-1, {
        if config.is_null() || out.is_null() {
            set_error("config or out-parameter pointer is null");
            return -1;
        }
        // SAFETY: config is a non-null `*const TdxConfig` returned by `tdx_config_*` and not yet freed; `&*` produces a shared reference valid for the call duration.
        let config = unsafe { &*config };
        // SAFETY: out pointer checked non-null above; the FFI contract pins the storage for the call duration and forbids concurrent calls on the same handle.
        unsafe {
            *out = config.inner.reconnect.replay_pace_ms;
        }
        0
    })
}

// ── FPSS transport knobs ────────────────────────────────────────────
//
// Scalar tuning on `FpssConfig` exposed for embedded callers: read
// timeout, connect timeout, ping cadence, ring size, the I/O read
// slice, the last-frame watchdog, the TCP keepalive schedule, and the
// host-selection policy. Out-of-range values are rejected at connect
// time by the core validator; the setters here store verbatim so the
// rejection carries the canonical bounds message.

/// Set the FPSS read timeout (ms): the no-frames deadline after which the streaming I/O loop declares the session dead and reconnects. Default `3_000`; validated to `[100, 60_000]` at connect.
#[no_mangle]
pub unsafe extern "C" fn tdx_config_set_fpss_timeout_ms(config: *mut TdxConfig, v: u64) {
    ffi_boundary!((), {
        let config = require_config_mut!(config);
        config.inner.fpss.timeout_ms = v;
    })
}

/// Read the current FPSS `timeout_ms` setting (default `3_000`).
///
/// Writes the configured value into `*out`. Returns `0` on success,
/// `-1` if either pointer is null.
#[no_mangle]
pub unsafe extern "C" fn tdx_config_get_fpss_timeout_ms(
    config: *const TdxConfig,
    out: *mut u64,
) -> i32 {
    ffi_boundary!(-1, {
        if config.is_null() || out.is_null() {
            set_error("config or out-parameter pointer is null");
            return -1;
        }
        // SAFETY: config is a non-null `*const TdxConfig` returned by `tdx_config_*` and not yet freed; `&*` produces a shared reference valid for the call duration.
        let config = unsafe { &*config };
        // SAFETY: out pointer checked non-null above; the FFI contract pins the storage for the call duration and forbids concurrent calls on the same handle.
        unsafe {
            *out = config.inner.fpss.timeout_ms;
        }
        0
    })
}

/// Set the per-server FPSS TCP connect timeout (ms). Default `2_000`; validated to `[1_000, 60_000]` at connect.
#[no_mangle]
pub unsafe extern "C" fn tdx_config_set_fpss_connect_timeout_ms(config: *mut TdxConfig, v: u64) {
    ffi_boundary!((), {
        let config = require_config_mut!(config);
        config.inner.fpss.connect_timeout_ms = v;
    })
}

/// Read the current FPSS `connect_timeout_ms` setting (default `2_000`).
///
/// Writes the configured value into `*out`. Returns `0` on success,
/// `-1` if either pointer is null.
#[no_mangle]
pub unsafe extern "C" fn tdx_config_get_fpss_connect_timeout_ms(
    config: *const TdxConfig,
    out: *mut u64,
) -> i32 {
    ffi_boundary!(-1, {
        if config.is_null() || out.is_null() {
            set_error("config or out-parameter pointer is null");
            return -1;
        }
        // SAFETY: config is a non-null `*const TdxConfig` returned by `tdx_config_*` and not yet freed; `&*` produces a shared reference valid for the call duration.
        let config = unsafe { &*config };
        // SAFETY: out pointer checked non-null above; the FFI contract pins the storage for the call duration and forbids concurrent calls on the same handle.
        unsafe {
            *out = config.inner.fpss.connect_timeout_ms;
        }
        0
    })
}

/// Set the FPSS heartbeat ping interval (ms). Default `250`; validated to `[100, 300_000]` at connect.
#[no_mangle]
pub unsafe extern "C" fn tdx_config_set_fpss_ping_interval_ms(config: *mut TdxConfig, v: u64) {
    ffi_boundary!((), {
        let config = require_config_mut!(config);
        config.inner.fpss.ping_interval_ms = v;
    })
}

/// Read the current FPSS `ping_interval_ms` setting (default `250`).
///
/// Writes the configured value into `*out`. Returns `0` on success,
/// `-1` if either pointer is null.
#[no_mangle]
pub unsafe extern "C" fn tdx_config_get_fpss_ping_interval_ms(
    config: *const TdxConfig,
    out: *mut u64,
) -> i32 {
    ffi_boundary!(-1, {
        if config.is_null() || out.is_null() {
            set_error("config or out-parameter pointer is null");
            return -1;
        }
        // SAFETY: config is a non-null `*const TdxConfig` returned by `tdx_config_*` and not yet freed; `&*` produces a shared reference valid for the call duration.
        let config = unsafe { &*config };
        // SAFETY: out pointer checked non-null above; the FFI contract pins the storage for the call duration and forbids concurrent calls on the same handle.
        unsafe {
            *out = config.inner.fpss.ping_interval_ms;
        }
        0
    })
}

/// Set the per-iteration blocking-read slice (ms) for the streaming I/O loop. Shorter slices service outbound commands more promptly at slightly higher idle CPU. Default `25`; validated to `[10, 500]` at connect.
#[no_mangle]
pub unsafe extern "C" fn tdx_config_set_fpss_io_read_slice_ms(config: *mut TdxConfig, v: u64) {
    ffi_boundary!((), {
        let config = require_config_mut!(config);
        config.inner.fpss.io_read_slice_ms = v;
    })
}

/// Read the current FPSS `io_read_slice_ms` setting (default `25`).
///
/// Writes the configured value into `*out`. Returns `0` on success,
/// `-1` if either pointer is null.
#[no_mangle]
pub unsafe extern "C" fn tdx_config_get_fpss_io_read_slice_ms(
    config: *const TdxConfig,
    out: *mut u64,
) -> i32 {
    ffi_boundary!(-1, {
        if config.is_null() || out.is_null() {
            set_error("config or out-parameter pointer is null");
            return -1;
        }
        // SAFETY: config is a non-null `*const TdxConfig` returned by `tdx_config_*` and not yet freed; `&*` produces a shared reference valid for the call duration.
        let config = unsafe { &*config };
        // SAFETY: out pointer checked non-null above; the FFI contract pins the storage for the call duration and forbids concurrent calls on the same handle.
        unsafe {
            *out = config.inner.fpss.io_read_slice_ms;
        }
        0
    })
}

/// Set the last-frame watchdog (ms): when no frame of any kind has arrived for this long the I/O loop force-reconnects, regardless of the read-timeout accounting. `0` disables. Default `30_000`.
#[no_mangle]
pub unsafe extern "C" fn tdx_config_set_fpss_data_watchdog_ms(config: *mut TdxConfig, v: u64) {
    ffi_boundary!((), {
        let config = require_config_mut!(config);
        config.inner.fpss.data_watchdog_ms = v;
    })
}

/// Read the current FPSS `data_watchdog_ms` setting (default `30_000`; `0` = disabled).
///
/// Writes the configured value into `*out`. Returns `0` on success,
/// `-1` if either pointer is null.
#[no_mangle]
pub unsafe extern "C" fn tdx_config_get_fpss_data_watchdog_ms(
    config: *const TdxConfig,
    out: *mut u64,
) -> i32 {
    ffi_boundary!(-1, {
        if config.is_null() || out.is_null() {
            set_error("config or out-parameter pointer is null");
            return -1;
        }
        // SAFETY: config is a non-null `*const TdxConfig` returned by `tdx_config_*` and not yet freed; `&*` produces a shared reference valid for the call duration.
        let config = unsafe { &*config };
        // SAFETY: out pointer checked non-null above; the FFI contract pins the storage for the call duration and forbids concurrent calls on the same handle.
        unsafe {
            *out = config.inner.fpss.data_watchdog_ms;
        }
        0
    })
}

/// Set the TCP keepalive idle time (seconds) before the kernel sends the first probe on a silent FPSS socket. Default `5`; validated to `[1, 7_200]` at connect.
#[no_mangle]
pub unsafe extern "C" fn tdx_config_set_fpss_keepalive_idle_secs(config: *mut TdxConfig, v: u64) {
    ffi_boundary!((), {
        let config = require_config_mut!(config);
        config.inner.fpss.keepalive_idle_secs = v;
    })
}

/// Read the current FPSS `keepalive_idle_secs` setting (default `5`).
///
/// Writes the configured value into `*out`. Returns `0` on success,
/// `-1` if either pointer is null.
#[no_mangle]
pub unsafe extern "C" fn tdx_config_get_fpss_keepalive_idle_secs(
    config: *const TdxConfig,
    out: *mut u64,
) -> i32 {
    ffi_boundary!(-1, {
        if config.is_null() || out.is_null() {
            set_error("config or out-parameter pointer is null");
            return -1;
        }
        // SAFETY: config is a non-null `*const TdxConfig` returned by `tdx_config_*` and not yet freed; `&*` produces a shared reference valid for the call duration.
        let config = unsafe { &*config };
        // SAFETY: out pointer checked non-null above; the FFI contract pins the storage for the call duration and forbids concurrent calls on the same handle.
        unsafe {
            *out = config.inner.fpss.keepalive_idle_secs;
        }
        0
    })
}

/// Set the interval (seconds) between TCP keepalive probes. Default `2`; validated to `[1, 75]` at connect.
#[no_mangle]
pub unsafe extern "C" fn tdx_config_set_fpss_keepalive_interval_secs(
    config: *mut TdxConfig,
    v: u64,
) {
    ffi_boundary!((), {
        let config = require_config_mut!(config);
        config.inner.fpss.keepalive_interval_secs = v;
    })
}

/// Read the current FPSS `keepalive_interval_secs` setting (default `2`).
///
/// Writes the configured value into `*out`. Returns `0` on success,
/// `-1` if either pointer is null.
#[no_mangle]
pub unsafe extern "C" fn tdx_config_get_fpss_keepalive_interval_secs(
    config: *const TdxConfig,
    out: *mut u64,
) -> i32 {
    ffi_boundary!(-1, {
        if config.is_null() || out.is_null() {
            set_error("config or out-parameter pointer is null");
            return -1;
        }
        // SAFETY: config is a non-null `*const TdxConfig` returned by `tdx_config_*` and not yet freed; `&*` produces a shared reference valid for the call duration.
        let config = unsafe { &*config };
        // SAFETY: out pointer checked non-null above; the FFI contract pins the storage for the call duration and forbids concurrent calls on the same handle.
        unsafe {
            *out = config.inner.fpss.keepalive_interval_secs;
        }
        0
    })
}

/// Set the number of unanswered TCP keepalive probes after which the kernel declares the FPSS connection dead (where the platform exposes the knob). Default `2`; validated to `[1, 10]` at connect.
#[no_mangle]
pub unsafe extern "C" fn tdx_config_set_fpss_keepalive_retries(config: *mut TdxConfig, v: u32) {
    ffi_boundary!((), {
        let config = require_config_mut!(config);
        config.inner.fpss.keepalive_retries = v;
    })
}

/// Read the current FPSS `keepalive_retries` setting (default `2`).
///
/// Writes the configured value into `*out`. Returns `0` on success,
/// `-1` if either pointer is null.
#[no_mangle]
pub unsafe extern "C" fn tdx_config_get_fpss_keepalive_retries(
    config: *const TdxConfig,
    out: *mut u32,
) -> i32 {
    ffi_boundary!(-1, {
        if config.is_null() || out.is_null() {
            set_error("config or out-parameter pointer is null");
            return -1;
        }
        // SAFETY: config is a non-null `*const TdxConfig` returned by `tdx_config_*` and not yet freed; `&*` produces a shared reference valid for the call duration.
        let config = unsafe { &*config };
        // SAFETY: out pointer checked non-null above; the FFI contract pins the storage for the call duration and forbids concurrent calls on the same handle.
        unsafe {
            *out = config.inner.fpss.keepalive_retries;
        }
        0
    })
}

/// Set the FPSS event ring buffer size (slots).
///
/// Must be a power of two `>= 64`. Invalid values are rejected at the
/// setter boundary: the config is left unchanged and the failure
/// reason is written to thread-local storage retrievable via
/// `tdx_last_error()`. Default is `131_072`.
#[no_mangle]
pub unsafe extern "C" fn tdx_config_set_fpss_ring_size(config: *mut TdxConfig, n: usize) {
    ffi_boundary!((), {
        if config.is_null() {
            return;
        }
        // Same validation as the Rust core's `check_ring_size` —
        // surface the rejection here so the FFI caller sees it at the
        // setter rather than at connect.
        if n == 0 || !n.is_power_of_two() {
            set_error(&format!(
                "fpss_ring_size must be a power of two >= 64; got {n}"
            ));
            return;
        }
        if n < 64 {
            set_error(&format!("fpss_ring_size must be >= 64; got {n}"));
            return;
        }
        // SAFETY: config is a non-null pointer returned by tdx_config_* and not yet freed.
        let config = unsafe { &mut *config };
        config.inner.fpss.ring_size = n;
    })
}

/// Read the current FPSS `ring_size` setting (default `131_072`).
///
/// Writes the configured value into `*out`. Returns `0` on success,
/// `-1` if either pointer is null.
#[no_mangle]
pub unsafe extern "C" fn tdx_config_get_fpss_ring_size(
    config: *const TdxConfig,
    out: *mut usize,
) -> i32 {
    ffi_boundary!(-1, {
        if config.is_null() || out.is_null() {
            set_error("config or out-parameter pointer is null");
            return -1;
        }
        // SAFETY: config is a non-null `*const TdxConfig` returned by `tdx_config_*` and not yet freed; `&*` produces a shared reference valid for the call duration.
        let config = unsafe { &*config };
        // SAFETY: out pointer checked non-null above; the FFI contract pins the storage for the call duration and forbids concurrent calls on the same handle.
        unsafe {
            *out = config.inner.fpss.ring_size;
        }
        0
    })
}

/// Set the FPSS host-selection policy.
///
/// - `policy = 0`: Shuffled (default) — fault-domain-aware per-client
///   shuffle; a fleet spreads across hosts and consecutive failover
///   attempts cross physical machines.
/// - `policy = 1`: FixedOrder — use the declared host order verbatim.
///
/// Returns `0` on success. Returns `-1` and sets `tdx_last_error`
/// when `policy` is outside the documented `{0, 1}` set or `config`
/// is null. A rejected `policy` value carries
/// `tdx_last_error_code = TDX_ERR_INVALID_PARAMETER` so an out-of-domain
/// enum int surfaces the same typed class across every binding.
#[no_mangle]
pub unsafe extern "C" fn tdx_config_set_fpss_host_selection(
    config: *mut TdxConfig,
    policy: i32,
) -> i32 {
    ffi_boundary!(-1, {
        if config.is_null() {
            set_error("tdx_config_set_fpss_host_selection: config handle is null");
            return -1;
        }
        let value = match policy {
            0 => thetadatadx::HostSelectionPolicy::Shuffled,
            1 => thetadatadx::HostSelectionPolicy::FixedOrder,
            other => {
                crate::error::set_error_with_code(
                    &format!(
                        "tdx_config_set_fpss_host_selection: invalid policy {other}; expected 0 (Shuffled) or 1 (FixedOrder)"
                    ),
                    crate::error::TDX_ERR_INVALID_PARAMETER,
                );
                return -1;
            }
        };
        // SAFETY: config is a non-null pointer returned by `tdx_config_*` and not yet freed; `&mut *` produces a unique reference valid for the call duration because the caller owns the Box and the FFI contract forbids concurrent calls on the same handle.
        let config = unsafe { &mut *config };
        config.inner.fpss.host_selection = value;
        0
    })
}

/// Read the configured FPSS host-selection policy. Same encoding as
/// `tdx_config_set_fpss_host_selection`. Returns `0` on success, `-1`
/// if either pointer is null.
#[no_mangle]
pub unsafe extern "C" fn tdx_config_get_fpss_host_selection(
    config: *const TdxConfig,
    out_policy: *mut i32,
) -> i32 {
    ffi_boundary!(-1, {
        if config.is_null() || out_policy.is_null() {
            set_error("config or out-parameter pointer is null");
            return -1;
        }
        // SAFETY: config is a non-null `*const TdxConfig` returned by `tdx_config_*` and not yet freed; `&*` produces a shared reference valid for the call duration.
        let config = unsafe { &*config };
        let value = match config.inner.fpss.host_selection {
            thetadatadx::HostSelectionPolicy::Shuffled => 0,
            _ => 1,
        };
        // SAFETY: out pointer checked non-null above; the FFI contract pins the storage for the call duration and forbids concurrent calls on the same handle.
        unsafe {
            *out_policy = value;
        }
        0
    })
}

/// Set the FPSS host-shuffle seed using the `(has_value, seed)`
/// widened ABI shape that preserves the `None` sentinel across the C
/// boundary.
///
/// * `has_value = false` → `None` (default): every client derives a
///   fresh per-instance seed, so a fleet shuffles independently.
///   `seed` is ignored.
/// * `has_value = true` → `Some(seed)`: the shuffled order becomes
///   deterministic — useful for fleet sharding and tests.
///
/// Ignored under the `FixedOrder` host-selection policy. Returns `0`
/// on success, `-1` if `config` is null.
#[no_mangle]
pub unsafe extern "C" fn tdx_config_set_fpss_host_shuffle_seed_explicit(
    config: *mut TdxConfig,
    has_value: bool,
    seed: u64,
) -> i32 {
    ffi_boundary!(-1, {
        if config.is_null() {
            set_error("config handle is null");
            return -1;
        }
        // SAFETY: config is a non-null pointer returned by `tdx_config_*` and not yet freed; `&mut *` produces a unique reference valid for the call duration because the caller owns the Box and the FFI contract forbids concurrent calls on the same handle.
        let config = unsafe { &mut *config };
        config.inner.fpss.host_shuffle_seed = if has_value { Some(seed) } else { None };
        0
    })
}

/// Read the current FPSS host-shuffle seed. Same `(has_value, seed)`
/// ABI as `tdx_config_set_fpss_host_shuffle_seed_explicit`:
///
/// * `*out_has_value = false` → `None` (per-client entropy). `*out_seed` is left `0`.
/// * `*out_has_value = true` → `Some(*out_seed)`.
///
/// Returns `0` on success, `-1` if any pointer is null.
#[no_mangle]
pub unsafe extern "C" fn tdx_config_get_fpss_host_shuffle_seed(
    config: *const TdxConfig,
    out_has_value: *mut bool,
    out_seed: *mut u64,
) -> i32 {
    ffi_boundary!(-1, {
        if config.is_null() || out_has_value.is_null() || out_seed.is_null() {
            set_error("config or out-parameter pointer is null");
            return -1;
        }
        // SAFETY: config is a non-null `*const TdxConfig` returned by `tdx_config_*` and not yet freed; `&*` produces a shared reference valid for the call duration.
        let config = unsafe { &*config };
        // SAFETY: out pointer checked non-null above; the FFI contract pins the storage for the call duration and forbids concurrent calls on the same handle.
        unsafe {
            match config.inner.fpss.host_shuffle_seed {
                Some(seed) => {
                    *out_has_value = true;
                    *out_seed = seed;
                }
                None => {
                    *out_has_value = false;
                    *out_seed = 0;
                }
            }
        }
        0
    })
}

// ── Historical-channel retry envelope + flatfile jitter ────────────

/// Set the wall-clock envelope (seconds) for one historical-channel
/// retry sequence, measured from the first attempt. `0` disables the
/// envelope (attempt budget only). Default `300`.
#[no_mangle]
pub unsafe extern "C" fn tdx_config_set_retry_max_elapsed_secs(config: *mut TdxConfig, secs: u64) {
    ffi_boundary!((), {
        let config = require_config_mut!(config);
        config.inner.retry.max_elapsed = std::time::Duration::from_secs(secs);
    })
}

/// Read the current `retry.max_elapsed` value in seconds (default
/// `300`; `0` = disabled). Returns `0` on success, `-1` if either
/// pointer is null.
#[no_mangle]
pub unsafe extern "C" fn tdx_config_get_retry_max_elapsed_secs(
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
        // SAFETY: out pointer checked non-null above; the FFI contract pins the storage for the call duration and forbids concurrent calls on the same handle.
        unsafe {
            *out_secs = config.inner.retry.max_elapsed.as_secs();
        }
        0
    })
}

/// Toggle AWS-style full jitter on the flatfile retry ladder. Default
/// `true`; `false` gives the deterministic schedule, useful for tests
/// that assert exact timings.
#[no_mangle]
pub unsafe extern "C" fn tdx_config_set_flatfiles_jitter(config: *mut TdxConfig, jitter: bool) {
    ffi_boundary!((), {
        let config = require_config_mut!(config);
        config.inner.flatfiles.jitter = jitter;
    })
}

/// Read the current `flatfiles.jitter` value (default `true`).
/// Returns `0` on success, `-1` if either pointer is null.
#[no_mangle]
pub unsafe extern "C" fn tdx_config_get_flatfiles_jitter(
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
        // SAFETY: out pointer checked non-null above; the FFI contract pins the storage for the call duration and forbids concurrent calls on the same handle.
        unsafe {
            *out_jitter = config.inner.flatfiles.jitter;
        }
        0
    })
}

// ── Custom reconnect policy callback ────────────────────────────────

/// Reconnect-decision callback type for
/// `tdx_config_set_reconnect_callback`.
///
/// Invoked on the streaming I/O thread after each retriable
/// involuntary disconnect. `reason` is the `RemoveReason` discriminant
/// as `i32`; `attempt` is the 1-based consecutive-reconnect counter.
/// Return the reconnect delay in milliseconds, or any negative value
/// to stop reconnecting (the I/O loop then emits the terminal
/// `ReconnectsExhausted` event and exits).
pub type TdxReconnectCallback =
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
pub unsafe extern "C" fn tdx_config_set_reconnect_callback(
    config: *mut TdxConfig,
    cb: Option<TdxReconnectCallback>,
    user_data: *mut std::ffi::c_void,
) -> i32 {
    ffi_boundary!(-1, {
        if config.is_null() {
            set_error("config handle is null");
            return -1;
        }
        // SAFETY: config is a non-null pointer returned by `tdx_config_*` and not yet freed; `&mut *` produces a unique reference valid for the call duration because the caller owns the Box and the FFI contract forbids concurrent calls on the same handle.
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
            cb: TdxReconnectCallback,
            user_data: *mut std::ffi::c_void,
        }
        // SAFETY: the public contract on `tdx_config_set_reconnect_callback` requires `cb` + `user_data` to be callable from any thread for the lifetime of clients built from this config; the wrapper only forwards the pointer pair to that documented-thread-safe callback.
        unsafe impl Send for CallbackCtx {}
        // SAFETY: same documented contract as the `Send` impl — the wrapped pointer pair is only ever used to invoke the caller-supplied thread-safe callback.
        unsafe impl Sync for CallbackCtx {}
        impl CallbackCtx {
            fn invoke(&self, reason: i32, attempt: u32) -> i64 {
                // SAFETY: `self.cb` is the caller-registered function pointer and `self.user_data` the matching context; the registration contract guarantees both stay valid and thread-safe while any client built from the config is alive.
                unsafe { (self.cb)(reason, attempt, self.user_data) }
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
/// * `has_value = true` → `Some(n)`. Embedders consuming
///   [`thetadatadx::RuntimeConfig::build_runtime`] honour the value
///   verbatim, clamping `0` to `1` so at least one worker is started.
///
/// Returns `0` on success, `-1` if `config` is null.
#[no_mangle]
pub unsafe extern "C" fn tdx_config_set_worker_threads_explicit(
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

/// Read the current async worker-thread count.
/// Widened `(has_value, n)` ABI:
///
/// * `*out_has_value = false` → `None` (auto-size). `*out_n` is left as `0`.
/// * `*out_has_value = true` → `Some(*out_n)`.
///
/// Returns `0` on success, `-1` if any pointer is null.
#[no_mangle]
pub unsafe extern "C" fn tdx_config_get_worker_threads(
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
/// - `enabled = true` (default): derive OHLCVC bars locally from trade events
/// - `enabled = false`: only emit server-sent OHLCVC frames (lower overhead)
#[no_mangle]
pub unsafe extern "C" fn tdx_config_set_derive_ohlcvc(config: *mut TdxConfig, enabled: bool) {
    ffi_boundary!((), {
        let config = require_config_mut!(config);
        config.inner.fpss.derive_ohlcvc = enabled;
    })
}

/// Read the configured FPSS OHLCVC-derivation flag. Writes `true` /
/// `false` into `*out_enabled`. Returns `0` on success, `-1` if either
/// pointer is null.
#[no_mangle]
pub unsafe extern "C" fn tdx_config_get_derive_ohlcvc(
    config: *const TdxConfig,
    out_enabled: *mut bool,
) -> i32 {
    ffi_boundary!(-1, {
        if config.is_null() || out_enabled.is_null() {
            set_error("config or out-parameter pointer is null");
            return -1;
        }
        // SAFETY: config is a non-null `*const TdxConfig` returned by `tdx_config_*` and not yet freed; `&*` produces a shared reference valid for the call duration.
        let config = unsafe { &*config };
        // SAFETY: out pointer checked non-null above; caller pins the storage for the call duration.
        unsafe {
            *out_enabled = config.inner.fpss.derive_ohlcvc;
        }
        0
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
// fields already on the C surface (`tokio_worker_threads`):
// `has_value = false` encodes the `None`
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

// ── MDDS endpoint ──────────────────────────────────────────────────
//
// The historical (MDDS) gRPC host / port advanced overrides. Both
// default to the upstream production endpoint; point them at a known
// host to redirect the historical channel (e.g. a refused endpoint in
// structural tests that prove the streaming-only surface never opens
// it). The host crosses the ABI as a `*const c_char` (validated non-null
// + UTF-8); the port is a bare `u16`.

/// Set the historical (MDDS) gRPC host on a config handle.
///
/// `host` must be a non-null, NUL-terminated, valid-UTF-8 C string.
/// Returns `0` on success, `-1` if `config` is null or `host` is
/// null / not valid UTF-8 (the diagnostic is written to thread-local
/// storage retrievable via `tdx_last_error()`).
#[no_mangle]
pub unsafe extern "C" fn tdx_config_set_mdds_host(
    config: *mut TdxConfig,
    host: *const c_char,
) -> i32 {
    ffi_boundary!(-1, {
        if config.is_null() {
            set_error("config handle is null");
            return -1;
        }
        // SAFETY: caller supplies a NUL-terminated C string allocated by the host runtime; cstr_to_str validates non-null + UTF-8.
        let host = match unsafe { cstr_to_str(host) } {
            Ok(Some(s)) => s,
            Ok(None) => {
                set_error("mdds_host is null");
                return -1;
            }
            Err(e) => {
                set_error(&format!("mdds_host is not valid UTF-8: {e}"));
                return -1;
            }
        };
        // SAFETY: config is a non-null pointer returned by tdx_config_* and not yet freed.
        let config = unsafe { &mut *config };
        config.inner.mdds.host = host.to_string();
        0
    })
}

/// Read the current historical (MDDS) gRPC host.
///
/// On success, returns a heap-owned NUL-terminated C string the caller
/// MUST release with `tdx_string_free`. Returns null if `config` is
/// null or the stored value contains an interior NUL (the diagnostic is
/// written to `tdx_last_error()`).
#[no_mangle]
pub unsafe extern "C" fn tdx_config_get_mdds_host(config: *const TdxConfig) -> *mut c_char {
    ffi_boundary!(ptr::null_mut(), {
        if config.is_null() {
            set_error("config handle is null");
            return ptr::null_mut();
        }
        // SAFETY: config is a non-null `*const TdxConfig` returned by `tdx_config_*` and not yet freed; `&*` produces a shared reference valid for the call duration.
        let config = unsafe { &*config };
        match std::ffi::CString::new(config.inner.mdds.host.as_str()) {
            Ok(c) => c.into_raw(),
            Err(e) => {
                set_error(&format!("mdds_host contains an interior NUL: {e}"));
                ptr::null_mut()
            }
        }
    })
}

/// Set the historical (MDDS) gRPC port on a config handle.
#[no_mangle]
pub unsafe extern "C" fn tdx_config_set_mdds_port(config: *mut TdxConfig, port: u16) {
    ffi_boundary!((), {
        let config = require_config_mut!(config);
        config.inner.mdds.port = port;
    })
}

/// Read the configured historical (MDDS) gRPC port. Writes the value
/// into `*out_port`. Returns `0` on success, `-1` if either pointer is
/// null.
#[no_mangle]
pub unsafe extern "C" fn tdx_config_get_mdds_port(
    config: *const TdxConfig,
    out_port: *mut u16,
) -> i32 {
    ffi_boundary!(-1, {
        if config.is_null() || out_port.is_null() {
            set_error("config or out-parameter pointer is null");
            return -1;
        }
        // SAFETY: config is a non-null `*const TdxConfig` returned by `tdx_config_*` and not yet freed; `&*` produces a shared reference valid for the call duration.
        let config = unsafe { &*config };
        // SAFETY: out pointer checked non-null above; caller pins the storage for the call duration.
        unsafe {
            *out_port = config.inner.mdds.port;
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

/// Read the configured concurrent in-flight gRPC request count. Writes
/// the value into `*out_n` (`0` = auto-detect from the subscription
/// tier). A stored value above `u32::MAX` saturates to `u32::MAX`.
/// Returns `0` on success, `-1` if either pointer is null.
#[no_mangle]
pub unsafe extern "C" fn tdx_config_get_concurrent_requests(
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
        let value = u32::try_from(config.inner.mdds.concurrent_requests).unwrap_or(u32::MAX);
        // SAFETY: out pointer checked non-null above; caller pins the storage for the call duration.
        unsafe {
            *out_n = value;
        }
        0
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

// ── MddsClient ──

/// Connect a historical (MDDS) client to `ThetaData` servers
/// (authenticates via Nexus API).
///
/// Returns null on connection/auth failure (check `tdx_last_error()`).
#[no_mangle]
pub unsafe extern "C" fn tdx_mdds_client_connect(
    creds: *const TdxCredentials,
    config: *const TdxConfig,
) -> *mut TdxMddsClient {
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
        // SAFETY: creds is a non-null pointer returned by tdx_credentials_from_email / tdx_credentials_from_file and not yet freed.
        let creds = unsafe { &*creds };
        // SAFETY: config is a non-null pointer returned by tdx_direct_config_new and not yet freed.
        let config = unsafe { &*config };
        match runtime().block_on(thetadatadx::mdds::MddsClient::connect(
            &creds.inner,
            config.inner.clone(),
        )) {
            Ok(client) => Box::into_raw(Box::new(TdxMddsClient { inner: client })),
            Err(e) => {
                set_error_from(&e);
                ptr::null_mut()
            }
        }
    })
}

/// Free a historical (MDDS) client handle.
#[no_mangle]
pub unsafe extern "C" fn tdx_mdds_client_free(client: *mut TdxMddsClient) {
    ffi_boundary!((), {
        if !client.is_null() {
            // SAFETY: the pointer was returned by Box::into_raw / tdx_*_new and has not been freed; ownership returns to Rust.
            drop(unsafe { Box::from_raw(client) });
        }
    })
}

#[cfg(test)]
mod pool_sizing_tests {
    //! Offline tests for the MDDS pool-sizing setter.
    //!
    //! Each test allocates a fresh `TdxConfig` via `tdx_config_production`,
    //! calls the setter under test, then reads the underlying Rust
    //! `MddsConfig` to confirm the value round-tripped.

    #[test]
    fn concurrent_requests_round_trips() {
        let cfg = super::tdx_config_production();
        assert!(!cfg.is_null());
        // SAFETY: handle just returned by tdx_config_production.
        unsafe {
            let mut current: u32 = 99;
            super::tdx_config_set_concurrent_requests(cfg, 8);
            assert_eq!((*cfg).inner.mdds.concurrent_requests, 8);
            assert_eq!(
                super::tdx_config_get_concurrent_requests(cfg, &mut current),
                0
            );
            assert_eq!(current, 8);
            super::tdx_config_set_concurrent_requests(cfg, 0);
            assert_eq!((*cfg).inner.mdds.concurrent_requests, 0);
            assert_eq!(
                super::tdx_config_get_concurrent_requests(cfg, &mut current),
                0
            );
            assert_eq!(current, 0);
            // Null-pointer guard on the getter returns -1.
            assert_eq!(
                super::tdx_config_get_concurrent_requests(std::ptr::null(), &mut current),
                -1
            );
            super::tdx_config_free(cfg);
        }
    }

    #[test]
    fn flush_mode_round_trips() {
        let cfg = super::tdx_config_production();
        assert!(!cfg.is_null());
        // SAFETY: handle just returned by tdx_config_production.
        unsafe {
            let mut mode: i32 = -1;
            // Default is Batched (0).
            assert_eq!(super::tdx_config_get_flush_mode(cfg, &mut mode), 0);
            assert_eq!(mode, 0);
            assert_eq!(super::tdx_config_set_flush_mode(cfg, 1), 0);
            assert_eq!(super::tdx_config_get_flush_mode(cfg, &mut mode), 0);
            assert_eq!(mode, 1);
            assert_eq!(super::tdx_config_set_flush_mode(cfg, 0), 0);
            assert_eq!(super::tdx_config_get_flush_mode(cfg, &mut mode), 0);
            assert_eq!(mode, 0);
            // Null-pointer guard on the getter returns -1.
            assert_eq!(
                super::tdx_config_get_flush_mode(std::ptr::null(), &mut mode),
                -1
            );
            super::tdx_config_free(cfg);
        }
    }

    #[test]
    fn derive_ohlcvc_round_trips() {
        let cfg = super::tdx_config_production();
        assert!(!cfg.is_null());
        // SAFETY: handle just returned by tdx_config_production.
        unsafe {
            let mut enabled = true;
            super::tdx_config_set_derive_ohlcvc(cfg, false);
            assert_eq!(super::tdx_config_get_derive_ohlcvc(cfg, &mut enabled), 0);
            assert!(!enabled);
            super::tdx_config_set_derive_ohlcvc(cfg, true);
            assert_eq!(super::tdx_config_get_derive_ohlcvc(cfg, &mut enabled), 0);
            assert!(enabled);
            // Null-pointer guard on the getter returns -1.
            assert_eq!(
                super::tdx_config_get_derive_ohlcvc(std::ptr::null(), &mut enabled),
                -1
            );
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
    fn null_handle_is_safe() {
        // SAFETY: passing null to tdx_config_set_* / tdx_*_free is the
        // documented FFI contract — the call must return without
        // crashing. The test exercises that null-tolerance branch.
        unsafe {
            super::tdx_config_set_concurrent_requests(std::ptr::null_mut(), 4);
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
    fn reconnect_policy_unknown_selector_rejected_with_typed_code() {
        // An int selector outside `{0, 1}` is rejected with the typed
        // invalid-parameter class rather than silently coerced to
        // `Auto` — the cross-binding contract the Python ValueError /
        // TypeScript InvalidParameterError already honour. The setter
        // returns `-1`, sets the typed code, and leaves the prior
        // policy untouched.
        let cfg = super::tdx_config_production();
        // SAFETY: handle just returned by tdx_config_production.
        unsafe {
            assert_eq!(super::tdx_config_set_reconnect_policy(cfg, 1), 0);
            crate::error::tdx_clear_error();
            assert_eq!(super::tdx_config_set_reconnect_policy(cfg, 7), -1);
            assert_eq!(
                crate::error::tdx_last_error_code(),
                crate::error::TDX_ERR_INVALID_PARAMETER
            );
            // The rejected call leaves the previously-set Manual policy
            // in place rather than overwriting it with a coerced Auto.
            assert!(matches!(
                (*cfg).inner.reconnect.policy,
                thetadatadx::ReconnectPolicy::Manual
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

            // Apply reconnect knobs.
            super::tdx_config_set_reconnect_policy(cfg, 0);
            super::tdx_config_set_reconnect_max_attempts(cfg, 5);
            super::tdx_config_set_reconnect_max_rate_limited_attempts(cfg, 3);
            super::tdx_config_set_reconnect_stable_window_secs(cfg, 60);

            // Pool-sizing mutations survived the reconnect setter sequence.
            let mdds = &(*cfg).inner.mdds;
            assert_eq!(mdds.concurrent_requests, 8);

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
            assert_eq!(got, 250);
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
    //! Offline tests for the async `worker_threads` setter/getter pair
    //! on the FFI surface — cross-binding parity with Python /
    //! TypeScript / C++. The `(has_value, n)` shape preserves `Some(0)`
    //! across the C boundary the same way the decode-pipeline setters
    //! do.

    #[test]
    fn worker_threads_explicit_round_trips_via_getter() {
        let cfg = super::tdx_config_production();
        // SAFETY: handle just returned by tdx_config_production.
        unsafe {
            // None sentinel (default).
            let mut got_has = true;
            let mut got_n: usize = 99;
            assert_eq!(
                super::tdx_config_get_worker_threads(cfg, &mut got_has, &mut got_n),
                0
            );
            assert!(!got_has, "default worker_threads must be None");
            assert_eq!(got_n, 0);

            // Explicit values round-trip including the Some(0) sentinel.
            for n in [0usize, 1, 2, 4, 8, 16, 32, 64] {
                let rc = super::tdx_config_set_worker_threads_explicit(cfg, true, n);
                assert_eq!(rc, 0);
                assert_eq!((*cfg).inner.runtime.tokio_worker_threads, Some(n));
                assert_eq!(
                    super::tdx_config_get_worker_threads(cfg, &mut got_has, &mut got_n),
                    0
                );
                assert!(got_has);
                assert_eq!(got_n, n);
            }

            // Reset to None.
            let rc = super::tdx_config_set_worker_threads_explicit(cfg, false, 999);
            assert_eq!(rc, 0);
            assert_eq!((*cfg).inner.runtime.tokio_worker_threads, None);
            super::tdx_config_free(cfg);
        }
    }

    #[test]
    fn worker_threads_null_handle_returns_minus_one() {
        // SAFETY: passing null to tdx_config_* is the documented FFI
        // contract — getter returns sentinel, setter no-ops.
        unsafe {
            let rc = super::tdx_config_set_worker_threads_explicit(std::ptr::null_mut(), true, 4);
            assert_eq!(rc, -1);
            let mut got_has = false;
            let mut got_n: usize = 0;
            assert_eq!(
                super::tdx_config_get_worker_threads(std::ptr::null(), &mut got_has, &mut got_n,),
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
            assert_eq!(got, 20);
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
            assert_eq!(got, 10);
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
            assert_eq!(got, 30);
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

#[cfg(test)]
mod resilience_knob_tests {
    //! Round-trip coverage for the connection-resilience knobs across
    //! the C ABI: every setter/getter pair added for the reconnect
    //! engine, the FPSS transport, the historical retry envelope, and
    //! the flatfile jitter toggle.

    #[test]
    fn reconnect_budget_getters_read_auto_limits() {
        let cfg = super::tdx_config_production();
        // SAFETY: handle just returned by tdx_config_production.
        unsafe {
            let mut policy: i32 = -1;
            assert_eq!(super::tdx_config_get_reconnect_policy(cfg, &mut policy), 0);
            assert_eq!(policy, 0, "production default policy is Auto");

            let mut got_u32: u32 = 0;
            assert_eq!(
                super::tdx_config_get_reconnect_max_attempts(cfg, &mut got_u32),
                0
            );
            assert_eq!(got_u32, 30);
            assert_eq!(
                super::tdx_config_get_reconnect_max_rate_limited_attempts(cfg, &mut got_u32),
                0
            );
            assert_eq!(got_u32, 100);
            assert_eq!(
                super::tdx_config_get_reconnect_max_server_restart_attempts(cfg, &mut got_u32),
                0
            );
            assert_eq!(got_u32, 60);

            let mut got_u64: u64 = 0;
            assert_eq!(
                super::tdx_config_get_reconnect_stable_window_secs(cfg, &mut got_u64),
                0
            );
            assert_eq!(got_u64, 60);
            assert_eq!(
                super::tdx_config_get_reconnect_max_elapsed_secs(cfg, &mut got_u64),
                0
            );
            assert_eq!(got_u64, 300);

            // Setters write through and read back.
            super::tdx_config_set_reconnect_max_server_restart_attempts(cfg, 7);
            assert_eq!(
                super::tdx_config_get_reconnect_max_server_restart_attempts(cfg, &mut got_u32),
                0
            );
            assert_eq!(got_u32, 7);
            super::tdx_config_set_reconnect_max_elapsed_secs(cfg, 0);
            assert_eq!(
                super::tdx_config_get_reconnect_max_elapsed_secs(cfg, &mut got_u64),
                0
            );
            assert_eq!(got_u64, 0, "0 (envelope disabled) round-trips");

            super::tdx_config_free(cfg);
        }
    }

    #[test]
    fn reconnect_policy_round_trips_and_rejects_invalid() {
        let cfg = super::tdx_config_production();
        // SAFETY: handle just returned by tdx_config_production.
        unsafe {
            let mut policy: i32 = -1;
            assert_eq!(super::tdx_config_get_reconnect_policy(cfg, &mut policy), 0);
            assert_eq!(policy, 0, "production default policy is Auto");
            for p in [1, 0] {
                assert_eq!(super::tdx_config_set_reconnect_policy(cfg, p), 0);
                assert_eq!(super::tdx_config_get_reconnect_policy(cfg, &mut policy), 0);
                assert_eq!(policy, p);
            }
            // An unknown selector is rejected with the typed
            // invalid-parameter class rather than silently coerced to
            // Auto — the cross-binding contract the Python ValueError /
            // TypeScript InvalidParameterError already honour.
            assert_eq!(
                super::tdx_config_set_reconnect_policy(cfg, 7),
                -1,
                "unknown policy rejected, not coerced"
            );
            assert_eq!(
                crate::error::tdx_last_error_code(),
                crate::error::TDX_ERR_INVALID_PARAMETER
            );
            assert_eq!(super::tdx_config_set_reconnect_policy(cfg, -5), -1);
            assert_eq!(
                crate::error::tdx_last_error_code(),
                crate::error::TDX_ERR_INVALID_PARAMETER
            );
            assert_eq!(super::tdx_config_get_reconnect_policy(cfg, &mut policy), 0);
            assert_eq!(policy, 0, "rejected policy leaves the config unchanged");
            super::tdx_config_free(cfg);
        }
    }

    #[test]
    fn flush_mode_round_trips_and_rejects_invalid_with_typed_code() {
        let cfg = super::tdx_config_production();
        // SAFETY: handle just returned by tdx_config_production.
        unsafe {
            assert_eq!(super::tdx_config_set_flush_mode(cfg, 0), 0);
            assert_eq!(super::tdx_config_set_flush_mode(cfg, 1), 0);
            // A rejected enum value surfaces the typed invalid-parameter
            // class, not the generic config code.
            assert_eq!(super::tdx_config_set_flush_mode(cfg, 9), -1);
            assert_eq!(
                crate::error::tdx_last_error_code(),
                crate::error::TDX_ERR_INVALID_PARAMETER
            );
            super::tdx_config_free(cfg);
        }
    }

    #[test]
    fn reconnect_cadence_and_replay_round_trip() {
        let cfg = super::tdx_config_production();
        // SAFETY: handle just returned by tdx_config_production.
        unsafe {
            let mut got: u64 = 0;
            assert_eq!(
                super::tdx_config_get_reconnect_wait_max_ms(cfg, &mut got),
                0
            );
            assert_eq!(got, 30_000);
            super::tdx_config_set_reconnect_wait_max_ms(cfg, 45_000);
            assert_eq!(
                super::tdx_config_get_reconnect_wait_max_ms(cfg, &mut got),
                0
            );
            assert_eq!(got, 45_000);

            assert_eq!(
                super::tdx_config_get_reconnect_wait_server_restart_ms(cfg, &mut got),
                0
            );
            assert_eq!(got, 5_000);
            super::tdx_config_set_reconnect_wait_server_restart_ms(cfg, 9_000);
            assert_eq!(
                super::tdx_config_get_reconnect_wait_server_restart_ms(cfg, &mut got),
                0
            );
            assert_eq!(got, 9_000);

            let mut got_u32: u32 = 0;
            assert_eq!(
                super::tdx_config_get_reconnect_replay_burst_size(cfg, &mut got_u32),
                0
            );
            assert_eq!(got_u32, 50);
            super::tdx_config_set_reconnect_replay_burst_size(cfg, 200);
            assert_eq!(
                super::tdx_config_get_reconnect_replay_burst_size(cfg, &mut got_u32),
                0
            );
            assert_eq!(got_u32, 200);

            assert_eq!(
                super::tdx_config_get_reconnect_replay_pace_ms(cfg, &mut got),
                0
            );
            assert_eq!(got, 5);
            super::tdx_config_set_reconnect_replay_pace_ms(cfg, 0);
            assert_eq!(
                super::tdx_config_get_reconnect_replay_pace_ms(cfg, &mut got),
                0
            );
            assert_eq!(got, 0);

            super::tdx_config_free(cfg);
        }
    }

    #[test]
    fn reconnect_jitter_round_trips_and_rejects_invalid() {
        let cfg = super::tdx_config_production();
        // SAFETY: handle just returned by tdx_config_production.
        unsafe {
            let mut mode: i32 = -1;
            assert_eq!(super::tdx_config_get_reconnect_jitter(cfg, &mut mode), 0);
            assert_eq!(mode, 0, "default jitter mode is Full");
            for m in [1, 2, 3, 0] {
                assert_eq!(super::tdx_config_set_reconnect_jitter(cfg, m), 0);
                assert_eq!(super::tdx_config_get_reconnect_jitter(cfg, &mut mode), 0);
                assert_eq!(mode, m);
            }
            assert_eq!(
                super::tdx_config_set_reconnect_jitter(cfg, 9),
                -1,
                "invalid mode rejected"
            );
            assert_eq!(
                crate::error::tdx_last_error_code(),
                crate::error::TDX_ERR_INVALID_PARAMETER,
                "a rejected enum value surfaces the typed invalid-parameter class"
            );
            assert_eq!(super::tdx_config_get_reconnect_jitter(cfg, &mut mode), 0);
            assert_eq!(mode, 0, "rejected mode leaves the config unchanged");
            super::tdx_config_free(cfg);
        }
    }

    #[test]
    fn fpss_transport_knobs_round_trip() {
        let cfg = super::tdx_config_production();
        // SAFETY: handle just returned by tdx_config_production.
        unsafe {
            let mut got: u64 = 0;
            assert_eq!(super::tdx_config_get_fpss_timeout_ms(cfg, &mut got), 0);
            assert_eq!(got, 3_000);
            super::tdx_config_set_fpss_timeout_ms(cfg, 9_000);
            assert_eq!(super::tdx_config_get_fpss_timeout_ms(cfg, &mut got), 0);
            assert_eq!(got, 9_000);

            assert_eq!(
                super::tdx_config_get_fpss_connect_timeout_ms(cfg, &mut got),
                0
            );
            assert_eq!(got, 2_000);
            assert_eq!(
                super::tdx_config_get_fpss_ping_interval_ms(cfg, &mut got),
                0
            );
            assert_eq!(got, 250);
            assert_eq!(
                super::tdx_config_get_fpss_io_read_slice_ms(cfg, &mut got),
                0
            );
            assert_eq!(got, 25);
            assert_eq!(
                super::tdx_config_get_fpss_data_watchdog_ms(cfg, &mut got),
                0
            );
            assert_eq!(got, 30_000);
            super::tdx_config_set_fpss_data_watchdog_ms(cfg, 0);
            assert_eq!(
                super::tdx_config_get_fpss_data_watchdog_ms(cfg, &mut got),
                0
            );
            assert_eq!(got, 0, "0 (watchdog disabled) round-trips");

            assert_eq!(
                super::tdx_config_get_fpss_keepalive_idle_secs(cfg, &mut got),
                0
            );
            assert_eq!(got, 5);
            assert_eq!(
                super::tdx_config_get_fpss_keepalive_interval_secs(cfg, &mut got),
                0
            );
            assert_eq!(got, 2);
            let mut got_u32: u32 = 0;
            assert_eq!(
                super::tdx_config_get_fpss_keepalive_retries(cfg, &mut got_u32),
                0
            );
            assert_eq!(got_u32, 2);

            let mut got_usize: usize = 0;
            assert_eq!(super::tdx_config_get_fpss_ring_size(cfg, &mut got_usize), 0);
            assert_eq!(got_usize, 131_072);
            super::tdx_config_set_fpss_ring_size(cfg, 4_096);
            assert_eq!(super::tdx_config_get_fpss_ring_size(cfg, &mut got_usize), 0);
            assert_eq!(got_usize, 4_096);
            // Non-power-of-two rejected at the setter; value unchanged.
            super::tdx_config_set_fpss_ring_size(cfg, 5_000);
            assert_eq!(super::tdx_config_get_fpss_ring_size(cfg, &mut got_usize), 0);
            assert_eq!(got_usize, 4_096);

            super::tdx_config_free(cfg);
        }
    }

    #[test]
    fn fpss_host_selection_and_seed_round_trip() {
        let cfg = super::tdx_config_production();
        // SAFETY: handle just returned by tdx_config_production.
        unsafe {
            let mut policy: i32 = -1;
            assert_eq!(
                super::tdx_config_get_fpss_host_selection(cfg, &mut policy),
                0
            );
            assert_eq!(policy, 0, "default host selection is Shuffled");
            assert_eq!(super::tdx_config_set_fpss_host_selection(cfg, 1), 0);
            assert_eq!(
                super::tdx_config_get_fpss_host_selection(cfg, &mut policy),
                0
            );
            assert_eq!(policy, 1);
            assert_eq!(
                super::tdx_config_set_fpss_host_selection(cfg, 5),
                -1,
                "invalid policy rejected"
            );
            assert_eq!(
                crate::error::tdx_last_error_code(),
                crate::error::TDX_ERR_INVALID_PARAMETER,
                "a rejected enum value surfaces the typed invalid-parameter class"
            );

            let mut has_value = true;
            let mut seed: u64 = 7;
            assert_eq!(
                super::tdx_config_get_fpss_host_shuffle_seed(cfg, &mut has_value, &mut seed),
                0
            );
            assert!(
                !has_value,
                "default seed is the per-client-entropy sentinel"
            );
            assert_eq!(seed, 0);
            assert_eq!(
                super::tdx_config_set_fpss_host_shuffle_seed_explicit(cfg, true, 42),
                0
            );
            assert_eq!(
                super::tdx_config_get_fpss_host_shuffle_seed(cfg, &mut has_value, &mut seed),
                0
            );
            assert!(has_value);
            assert_eq!(seed, 42);
            assert_eq!(
                super::tdx_config_set_fpss_host_shuffle_seed_explicit(cfg, false, 0),
                0
            );
            assert_eq!(
                super::tdx_config_get_fpss_host_shuffle_seed(cfg, &mut has_value, &mut seed),
                0
            );
            assert!(!has_value, "explicit None restores the sentinel");

            super::tdx_config_free(cfg);
        }
    }

    #[test]
    fn retry_envelope_and_flatfiles_jitter_round_trip() {
        let cfg = super::tdx_config_production();
        // SAFETY: handle just returned by tdx_config_production.
        unsafe {
            let mut got: u64 = 0;
            assert_eq!(
                super::tdx_config_get_retry_max_elapsed_secs(cfg, &mut got),
                0
            );
            assert_eq!(got, 300);
            super::tdx_config_set_retry_max_elapsed_secs(cfg, 0);
            assert_eq!(
                super::tdx_config_get_retry_max_elapsed_secs(cfg, &mut got),
                0
            );
            assert_eq!(got, 0);

            let mut jitter = false;
            assert_eq!(super::tdx_config_get_flatfiles_jitter(cfg, &mut jitter), 0);
            assert!(jitter, "flatfile jitter defaults on");
            super::tdx_config_set_flatfiles_jitter(cfg, false);
            assert_eq!(super::tdx_config_get_flatfiles_jitter(cfg, &mut jitter), 0);
            assert!(!jitter);

            super::tdx_config_free(cfg);
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

        let cfg = super::tdx_config_production();
        let mut calls: i32 = 0;
        // SAFETY: handle just returned by tdx_config_production; the
        // callback + user_data outlive every policy invocation below.
        unsafe {
            assert_eq!(
                super::tdx_config_set_reconnect_callback(
                    cfg,
                    Some(decide),
                    std::ptr::addr_of_mut!(calls).cast(),
                ),
                0
            );
            let mut policy: i32 = -1;
            assert_eq!(super::tdx_config_get_reconnect_policy(cfg, &mut policy), 0);
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
                super::tdx_config_set_reconnect_callback(cfg, None, std::ptr::null_mut()),
                0
            );
            let mut policy: i32 = -1;
            assert_eq!(super::tdx_config_get_reconnect_policy(cfg, &mut policy), 0);
            assert_eq!(policy, 0);

            super::tdx_config_free(cfg);
        }
    }
}
