//! Round-trip tests for the streaming wait-strategy + consumer-cpu C ABI:
//! the preset selector, the numeric tuning knobs, and the negative
//! consumer-cpu sentinel, plus invalid-value rejection.
//!
//! Every FFI call below operates on a non-null `ThetaDataDxConfig`
//! returned by `thetadatadx_config_production` and not yet freed, on a
//! single thread — the contract each `unsafe extern "C"` entry point
//! documents. The per-call `// SAFETY:` comments record that invariant.

use thetadatadx_ffi::{
    thetadatadx_config_free, thetadatadx_config_get_consumer_cpu,
    thetadatadx_config_get_wait_park_us, thetadatadx_config_get_wait_spin_iters,
    thetadatadx_config_get_wait_strategy, thetadatadx_config_get_wait_yield_iters,
    thetadatadx_config_production, thetadatadx_config_set_consumer_cpu,
    thetadatadx_config_set_wait_park_us, thetadatadx_config_set_wait_spin_iters,
    thetadatadx_config_set_wait_strategy, thetadatadx_config_set_wait_yield_iters,
    ThetaDataDxConfig,
};

/// SAFETY of every call in this module: `cfg` is the non-null,
/// not-yet-freed handle `thetadatadx_config_production` returns, used on
/// one thread, and the out-pointers are local stack slots live for the
/// call — exactly the contract each entry point documents.
unsafe fn set_wait_strategy(cfg: *mut ThetaDataDxConfig, mode: i32) -> i32 {
    // SAFETY: delegated under the module-level invariant on `cfg`.
    unsafe { thetadatadx_config_set_wait_strategy(cfg, mode) }
}

unsafe fn get_wait_strategy(cfg: *const ThetaDataDxConfig) -> (i32, i32) {
    let mut out = -1;
    // SAFETY: delegated under the module-level invariant on `cfg`;
    // `out` is a live stack slot for the call.
    let rc = unsafe { thetadatadx_config_get_wait_strategy(cfg, &mut out) };
    (rc, out)
}

#[test]
fn wait_strategy_presets_round_trip() {
    let cfg = thetadatadx_config_production();
    assert!(!cfg.is_null());

    // Default preset is LowLatency (0), preserving historical behaviour.
    // SAFETY: `cfg` is the non-null live handle from `thetadatadx_config_production`,
    // not yet freed; `get_wait_strategy` only reads it and returns rc + mode by value.
    let (rc, mode) = unsafe { get_wait_strategy(cfg) };
    assert_eq!(rc, 0);
    assert_eq!(mode, 0);

    for want in 0..=3 {
        // SAFETY: `cfg` is the same live, unfreed handle; `set_wait_strategy`
        // mutates its wait-strategy field in place on this single thread.
        assert_eq!(unsafe { set_wait_strategy(cfg, want) }, 0);
        // SAFETY: `cfg` is the live handle just written; `get_wait_strategy`
        // reads back the field `set_wait_strategy` set on the line above.
        let (rc, got) = unsafe { get_wait_strategy(cfg) };
        assert_eq!(rc, 0);
        assert_eq!(got, want);
    }

    // Out-of-range preset is rejected.
    // SAFETY: `cfg` is the live, unfreed handle; `set_wait_strategy` validates
    // the `9` argument and returns -1 without mutating `cfg`.
    assert_eq!(unsafe { set_wait_strategy(cfg, 9) }, -1);

    // SAFETY: `cfg` is owned here and freed exactly once.
    unsafe { thetadatadx_config_free(cfg) };
}

#[test]
fn wait_tuning_round_trips() {
    let cfg = thetadatadx_config_production();
    assert!(!cfg.is_null());

    let mut spin = 0;
    let mut yield_ = 0;
    let mut park = 0;
    // SAFETY: `cfg` is the live, unfreed handle; each wait-tuning set/get call
    // reads or mutates `cfg` in place, and `spin`/`yield_`/`park` are valid
    // live stack out-params for the duration of the block.
    unsafe {
        assert_eq!(thetadatadx_config_set_wait_spin_iters(cfg, 16), 0);
        assert_eq!(thetadatadx_config_set_wait_yield_iters(cfg, 2), 0);
        assert_eq!(thetadatadx_config_set_wait_park_us(cfg, 200), 0);
        assert_eq!(thetadatadx_config_get_wait_spin_iters(cfg, &mut spin), 0);
        assert_eq!(thetadatadx_config_get_wait_yield_iters(cfg, &mut yield_), 0);
        assert_eq!(thetadatadx_config_get_wait_park_us(cfg, &mut park), 0);
    }
    assert_eq!(spin, 16);
    assert_eq!(yield_, 2);
    assert_eq!(park, 200);

    // SAFETY: `cfg` is owned here and freed exactly once.
    unsafe { thetadatadx_config_free(cfg) };
}

#[test]
fn consumer_cpu_uses_negative_sentinel_for_unpinned() {
    let cfg = thetadatadx_config_production();
    assert!(!cfg.is_null());

    let mut core = 0;
    let mut pinned = -1;
    let mut cleared = 0;
    // SAFETY: `cfg` is the live, unfreed handle; the consumer-cpu get/set calls
    // read or mutate `cfg` in place, and `core` is a valid live stack out-param.
    unsafe {
        // Default is unpinned: the getter writes the -1 sentinel.
        assert_eq!(thetadatadx_config_get_consumer_cpu(cfg, &mut core), 0);
        assert_eq!(core, -1);
        // A non-negative core pins.
        assert_eq!(thetadatadx_config_set_consumer_cpu(cfg, 3), 0);
        assert_eq!(thetadatadx_config_get_consumer_cpu(cfg, &mut pinned), 0);
        // A negative value clears the pin back to the sentinel.
        assert_eq!(thetadatadx_config_set_consumer_cpu(cfg, -1), 0);
        assert_eq!(thetadatadx_config_get_consumer_cpu(cfg, &mut cleared), 0);
    }
    assert_eq!(pinned, 3);
    assert_eq!(cleared, -1);

    // SAFETY: `cfg` is owned here and freed exactly once.
    unsafe { thetadatadx_config_free(cfg) };
}
