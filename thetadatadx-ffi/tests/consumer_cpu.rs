//! Round-trip test for the streaming consumer-cpu C ABI: the negative
//! sentinel for "unpinned" and pin/clear via a non-negative core.
//!
//! Every FFI call below operates on a non-null `ThetaDataDxConfig`
//! returned by `thetadatadx_config_production` and not yet freed, on a
//! single thread — the contract each `unsafe extern "C"` entry point
//! documents. The per-call `// SAFETY:` comments record that invariant.

use thetadatadx_ffi::{
    thetadatadx_config_free, thetadatadx_config_get_consumer_cpu, thetadatadx_config_production,
    thetadatadx_config_set_consumer_cpu,
};

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
