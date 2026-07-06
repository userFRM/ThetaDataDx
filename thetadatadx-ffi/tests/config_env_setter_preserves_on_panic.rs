//! Regression: the per-channel environment setters must not wipe the config
//! when the consuming builder's internal validation rejects a previously
//! raw-set tuning knob.
//!
//! `thetadatadx_config_with_market_data_environment` /
//! `_with_streaming_environment` run `DirectConfig::with_*_environment`, which
//! ends in `validate().expect(...)` and panics if a knob set earlier via a raw
//! setter (no validation) is out of range. The `ffi_boundary!` wrapper catches
//! that panic and returns -1. The bug: the inner config was moved OUT (replaced
//! with `Default`) BEFORE the consuming builder ran, so the caught panic left
//! the handle holding a default config — every custom host / tuning knob wiped.
//! The fix runs the builder on a clone and stores back only on success, so a
//! caught panic leaves the original config intact.
//!
//! Each FFI call operates on a non-null `ThetaDataDxConfig` returned by
//! `thetadatadx_config_production` and not yet freed, on a single thread — the
//! contract each `unsafe extern "C"` entry point documents.

use std::ffi::{CStr, CString};

use thetadatadx_ffi::{
    thetadatadx_config_free, thetadatadx_config_get_market_data_host,
    thetadatadx_config_production, thetadatadx_config_set_flatfiles_connect_timeout_secs,
    thetadatadx_config_set_market_data_host, thetadatadx_config_with_market_data_environment,
    thetadatadx_config_with_streaming_environment, thetadatadx_string_free,
};

/// Read `thetadatadx_config_get_market_data_host` into an owned String, freeing
/// the heap-owned C string the getter returns.
fn read_market_data_host(config: *const thetadatadx_ffi::ThetaDataDxConfig) -> String {
    // SAFETY: callers pass a live, non-null config handle; the getter returns a
    // heap-owned NUL-terminated C string that must be freed with
    // `thetadatadx_string_free` after copying.
    unsafe {
        let ptr = thetadatadx_config_get_market_data_host(config);
        assert!(!ptr.is_null(), "market-data host getter returned null");
        let s = CStr::from_ptr(ptr).to_string_lossy().into_owned();
        thetadatadx_string_free(ptr);
        s
    }
}

#[test]
fn market_data_env_setter_preserves_custom_host_when_a_bad_knob_panics() {
    // production() takes no pointer and returns a non-null owned config handle.
    let config = thetadatadx_config_production();
    assert!(!config.is_null());

    let custom = CString::new("custom.example.test").unwrap();
    // SAFETY: `config` is the live handle from production() above; `custom`
    // outlives this call and is a NUL-terminated C string from CString.
    let rc = unsafe { thetadatadx_config_set_market_data_host(config, custom.as_ptr()) };
    assert_eq!(rc, 0, "setting the custom host should succeed");
    assert_eq!(read_market_data_host(config), "custom.example.test");

    // Raw-set a flat-file connect timeout of 0, which the consuming builder's
    // `validate()` rejects (CONNECT_TIMEOUT_SECS range) -> `with_*` panics.
    // SAFETY: `config` is still the live handle; this setter raw-writes a u64
    // field through it and reads no pointer argument.
    unsafe { thetadatadx_config_set_flatfiles_connect_timeout_secs(config, 0) };

    // The setter must catch the panic and return -1...
    // SAFETY: `config` is live; the i32 selector (1 == STAGE) is the only other
    // argument and carries no pointer.
    let rc = unsafe { thetadatadx_config_with_market_data_environment(config, 1) };
    assert_eq!(
        rc, -1,
        "an out-of-range knob must make the env setter fail, not panic across the boundary",
    );

    // ...AND must NOT have wiped the config: the custom host survives.
    assert_eq!(
        read_market_data_host(config),
        "custom.example.test",
        "the env setter wiped the custom host on the caught-panic path (config-wipe regression)",
    );

    // SAFETY: `config` was returned by production() and has not been freed on
    // any path above, so this is the sole owner releasing it exactly once.
    unsafe { thetadatadx_config_free(config) };
}

#[test]
fn streaming_env_setter_preserves_custom_host_when_a_bad_knob_panics() {
    // production() takes no pointer and returns a non-null owned config handle.
    let config = thetadatadx_config_production();
    assert!(!config.is_null());

    let custom = CString::new("custom.example.test").unwrap();
    // SAFETY: `config` is the live handle from production() above; `custom`
    // outlives this call and is a NUL-terminated C string from CString.
    let rc = unsafe { thetadatadx_config_set_market_data_host(config, custom.as_ptr()) };
    assert_eq!(rc, 0);

    // SAFETY: `config` is still the live handle; this setter raw-writes a u64
    // field through it and reads no pointer argument.
    unsafe { thetadatadx_config_set_flatfiles_connect_timeout_secs(config, 0) };

    // SAFETY: `config` is live; the i32 selector (1 == DEV) is the only other
    // argument and carries no pointer.
    let rc = unsafe { thetadatadx_config_with_streaming_environment(config, 1) };
    assert_eq!(rc, -1);

    assert_eq!(
        read_market_data_host(config),
        "custom.example.test",
        "the streaming env setter wiped the config on the caught-panic path",
    );

    // SAFETY: `config` was returned by production() and has not been freed on
    // any path above, so this is the sole owner releasing it exactly once.
    unsafe { thetadatadx_config_free(config) };
}
