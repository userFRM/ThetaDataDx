//! Credentials, config, and historical-client lifecycle: `tdx_credentials_*`,
//! `tdx_config_*`, `tdx_client_connect` / `tdx_client_free`.
//!
//! Split verbatim from `lib.rs`; the exported C ABI is unchanged.

use std::os::raw::c_char;
use std::ptr;

use crate::error::{cstr_to_str, set_error};
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
                set_error(&e.to_string());
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
        let config = unsafe { &mut *config };
        config.inner.fpss_flush_mode = match mode {
            1 => thetadatadx::FpssFlushMode::Immediate,
            _ => thetadatadx::FpssFlushMode::Batched,
        };
    })
}

/// Set FPSS reconnect policy on a config handle.
///
/// - `policy = 0`: Auto (default) -- auto-reconnect matching Java terminal behavior
/// - `policy = 1`: Manual -- no auto-reconnect, user calls reconnect explicitly
#[no_mangle]
pub unsafe extern "C" fn tdx_config_set_reconnect_policy(config: *mut TdxConfig, policy: i32) {
    ffi_boundary!((), {
        if config.is_null() {
            return;
        }
        let config = unsafe { &mut *config };
        config.inner.reconnect_policy = match policy {
            1 => thetadatadx::ReconnectPolicy::Manual,
            _ => thetadatadx::ReconnectPolicy::Auto,
        };
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
        let config = unsafe { &mut *config };
        config.inner.derive_ohlcvc = enabled != 0;
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
        let creds = unsafe { &*creds };
        let config = unsafe { &*config };
        match runtime().block_on(thetadatadx::mdds::MddsClient::connect(
            &creds.inner,
            config.inner.clone(),
        )) {
            Ok(client) => Box::into_raw(Box::new(TdxClient { inner: client })),
            Err(e) => {
                set_error(&e.to_string());
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
            drop(unsafe { Box::from_raw(client) });
        }
    })
}
