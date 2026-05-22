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
#[no_mangle]
pub unsafe extern "C" fn tdx_config_set_flush_mode(config: *mut TdxConfig, mode: i32) {
    ffi_boundary!((), {
        if config.is_null() {
            return;
        }
        // SAFETY: config is a non-null pointer returned by tdx_direct_config_new and not yet freed.
        let config = unsafe { &mut *config };
        config.inner.fpss.flush_mode = match mode {
            1 => thetadatadx::FpssFlushMode::Immediate,
            _ => thetadatadx::FpssFlushMode::Batched,
        };
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
        if config.is_null() {
            return;
        }
        // SAFETY: config is a non-null pointer returned by tdx_direct_config_new and not yet freed.
        let config = unsafe { &mut *config };
        config.inner.reconnect.policy = match policy {
            1 => thetadatadx::ReconnectPolicy::Manual,
            _ => thetadatadx::ReconnectPolicy::Auto(thetadatadx::ReconnectAttemptLimits::default()),
        };
    })
}

/// Set the per-class transient-failure attempt budget for the
/// auto-reconnect path. Default `3`. Has no effect when the reconnect
/// policy is not `Auto`.
#[no_mangle]
pub unsafe extern "C" fn tdx_config_set_reconnect_max_attempts(
    config: *mut TdxConfig,
    max_attempts: u32,
) {
    ffi_boundary!((), {
        if config.is_null() {
            return;
        }
        // SAFETY: config is a non-null pointer returned by tdx_direct_config_new and not yet freed.
        let config = unsafe { &mut *config };
        if let thetadatadx::ReconnectPolicy::Auto(ref mut limits) = config.inner.reconnect.policy {
            limits.max_attempts = max_attempts;
        }
    })
}

/// Set the per-class rate-limited (`TooManyRequests`) attempt budget
/// for the auto-reconnect path. Default `100`. Has no effect when the
/// reconnect policy is not `Auto`.
#[no_mangle]
pub unsafe extern "C" fn tdx_config_set_reconnect_max_rate_limited_attempts(
    config: *mut TdxConfig,
    max_rate_limited_attempts: u32,
) {
    ffi_boundary!((), {
        if config.is_null() {
            return;
        }
        // SAFETY: config is a non-null pointer returned by tdx_direct_config_new and not yet freed.
        let config = unsafe { &mut *config };
        if let thetadatadx::ReconnectPolicy::Auto(ref mut limits) = config.inner.reconnect.policy {
            limits.max_rate_limited_attempts = max_rate_limited_attempts;
        }
    })
}

/// Set the continuous successful-data-flow window (in seconds) after
/// which the auto-reconnect attempt counters reset. Default `60`. Has
/// no effect when the reconnect policy is not `Auto`.
#[no_mangle]
pub unsafe extern "C" fn tdx_config_set_reconnect_stable_window_secs(
    config: *mut TdxConfig,
    secs: u64,
) {
    ffi_boundary!((), {
        if config.is_null() {
            return;
        }
        // SAFETY: config is a non-null pointer returned by tdx_direct_config_new and not yet freed.
        let config = unsafe { &mut *config };
        if let thetadatadx::ReconnectPolicy::Auto(ref mut limits) = config.inner.reconnect.policy {
            limits.stable_window = std::time::Duration::from_secs(secs);
        }
    })
}

/// Set FPSS OHLCVC derivation on a config handle.
///
/// - `enabled = 1` (default): derive OHLCVC bars locally from trade events
/// - `enabled = 0`: only emit server-sent OHLCVC frames (lower overhead)
#[no_mangle]
pub unsafe extern "C" fn tdx_config_set_derive_ohlcvc(config: *mut TdxConfig, enabled: i32) {
    ffi_boundary!((), {
        if config.is_null() {
            return;
        }
        // SAFETY: config is a non-null pointer returned by tdx_direct_config_new and not yet freed.
        let config = unsafe { &mut *config };
        config.inner.fpss.derive_ohlcvc = enabled != 0;
    })
}

// ── MDDS pool sizing — issue #584 ──────────────────────────────────

/// Set the number of concurrent in-flight gRPC requests on a config
/// handle.
///
/// `n = 0` (default) auto-detects from the Nexus subscription tier
/// (Free=1 / Value=2 / Standard=4 / Pro=8). Explicit values above
/// the tier cap are clamped at connect time with a `tracing::warn!`.
#[no_mangle]
pub unsafe extern "C" fn tdx_config_set_concurrent_requests(config: *mut TdxConfig, n: u32) {
    ffi_boundary!((), {
        if config.is_null() {
            return;
        }
        // SAFETY: config is a non-null pointer returned by tdx_direct_config_new and not yet freed.
        let config = unsafe { &mut *config };
        config.inner.mdds.concurrent_requests = n as usize;
    })
}

/// Set the number of dedicated decoder threads in the MDDS pool.
///
/// `n = 0` (default) auto-sizes to
/// `max(available_parallelism / 2, 1)`. Override on shared hosts or
/// to widen the decode pipeline on historical backfills with wide
/// `strike_range`.
#[no_mangle]
pub unsafe extern "C" fn tdx_config_set_decoder_threads(config: *mut TdxConfig, n: u32) {
    ffi_boundary!((), {
        if config.is_null() {
            return;
        }
        // SAFETY: config is a non-null pointer returned by tdx_direct_config_new and not yet freed.
        let config = unsafe { &mut *config };
        config.inner.mdds.decoder_threads = n as usize;
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
        // the disruptor minimum — surface the rejection here so the
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

// ── Legacy n-only ABI (kept for v10 compatibility) ─────────────────
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

// ── Getters (BLOCKER closure: pin the round-trip across C++) ───────

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
        // SAFETY: callers supply non-null pointers per the contract
        // documented above; `config` was returned by `tdx_config_*`,
        // out-pointers reference caller stack/heap storage with at
        // least `sizeof(T)` lifetime extending past this call.
        let config = unsafe { &*config };
        let (has_value, n) = match config.inner.mdds.decode_threads {
            Some(v) => (true, v),
            None => (false, 0),
        };
        // SAFETY: see above.
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
        // SAFETY: callers supply non-null pointers per the contract;
        // see `tdx_config_get_decode_threads` for the full invariant.
        let config = unsafe { &*config };
        let (has_value, n) = match config.inner.mdds.decode_queue_depth {
            Some(v) => (true, v),
            None => (false, 0),
        };
        // SAFETY: see above.
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
    //! Offline tests for the MDDS pool-sizing setters (issue #584).
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
    fn decoder_threads_round_trips() {
        let cfg = super::tdx_config_production();
        assert!(!cfg.is_null());
        // SAFETY: handle just returned by tdx_config_production.
        unsafe {
            super::tdx_config_set_decoder_threads(cfg, 16);
            assert_eq!((*cfg).inner.mdds.decoder_threads, 16);
            super::tdx_config_set_decoder_threads(cfg, 0);
            assert_eq!((*cfg).inner.mdds.decoder_threads, 0);
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
        // SAFETY: passing null is the contract — the setters must
        // return without crashing.
        unsafe {
            super::tdx_config_set_concurrent_requests(std::ptr::null_mut(), 4);
            super::tdx_config_set_decoder_threads(std::ptr::null_mut(), 4);
            super::tdx_config_set_decoder_ring_size(std::ptr::null_mut(), 256);
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
        // SAFETY: passing null is the contract.
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
        // SAFETY: passing null is the contract.
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
        // SAFETY: passing null is the contract.
        unsafe {
            let rc =
                super::tdx_config_set_decode_queue_depth_explicit(std::ptr::null_mut(), true, 1024);
            assert_eq!(rc, -1);
        }
        let msg = last_error_text();
        assert!(msg.contains("null"));
    }

    #[test]
    fn decode_pipeline_setters_compose_with_legacy_pool_sizing() {
        let cfg = super::tdx_config_production();
        // SAFETY: handle just returned by tdx_config_production.
        unsafe {
            super::tdx_config_set_concurrent_requests(cfg, 8);
            super::tdx_config_set_decoder_threads(cfg, 4);
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
            assert_eq!((*cfg).inner.mdds.decoder_threads, 4);
            assert_eq!((*cfg).inner.mdds.decoder_ring_size, 1024);
            assert_eq!((*cfg).inner.mdds.decode_threads, Some(16));
            assert_eq!((*cfg).inner.mdds.decode_queue_depth, Some(4096));
            super::tdx_config_free(cfg);
        }
    }
}
