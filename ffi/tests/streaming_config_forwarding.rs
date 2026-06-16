//! Round-trip regression for the standalone streaming-config forwarding
//! over the C ABI.
//!
//! `wait_strategy.rs` round-trips only the wait-strategy preset + the
//! numeric wait knobs + the consumer-cpu sentinel. It never drives the
//! rest of the streaming transport surface or the reconnect ladder, so a
//! regression that dropped a `streaming_*` / `reconnect_*` field on the
//! C boundary would pass that test silently. This module pins the FULL
//! field set the higher-level bindings already cover (Python
//! `test_config_resilience.py::test_streaming_transport_defaults_and_round_trip`
//! plus the reconnect round-trip tests, and the TypeScript
//! `config_resilience` and `config_reconnect` suites), so the C ABI stays in lockstep.
//!
//! Every FFI call below operates on a non-null `ThetaDataDxConfig`
//! returned by `thetadatadx_config_production` and not yet freed, on a
//! single thread — the contract each `unsafe extern "C"` entry point
//! documents. The per-block `// SAFETY:` comments record that invariant.

use thetadatadx_ffi::{
    thetadatadx_config_free, thetadatadx_config_get_reconnect_jitter,
    thetadatadx_config_get_reconnect_max_attempts,
    thetadatadx_config_get_reconnect_max_elapsed_secs,
    thetadatadx_config_get_reconnect_max_rate_limited_attempts,
    thetadatadx_config_get_reconnect_max_server_restart_attempts,
    thetadatadx_config_get_reconnect_policy, thetadatadx_config_get_reconnect_replay_burst_size,
    thetadatadx_config_get_reconnect_replay_pace_ms,
    thetadatadx_config_get_reconnect_stable_window_secs,
    thetadatadx_config_get_reconnect_wait_max_ms, thetadatadx_config_get_reconnect_wait_ms,
    thetadatadx_config_get_reconnect_wait_rate_limited_ms,
    thetadatadx_config_get_reconnect_wait_server_restart_ms,
    thetadatadx_config_get_streaming_connect_timeout_ms,
    thetadatadx_config_get_streaming_data_watchdog_ms,
    thetadatadx_config_get_streaming_host_selection,
    thetadatadx_config_get_streaming_host_shuffle_seed,
    thetadatadx_config_get_streaming_io_read_slice_ms,
    thetadatadx_config_get_streaming_keepalive_idle_secs,
    thetadatadx_config_get_streaming_keepalive_interval_secs,
    thetadatadx_config_get_streaming_keepalive_retries,
    thetadatadx_config_get_streaming_ping_interval_ms, thetadatadx_config_get_streaming_ring_size,
    thetadatadx_config_get_streaming_timeout_ms, thetadatadx_config_production,
    thetadatadx_config_set_reconnect_jitter, thetadatadx_config_set_reconnect_max_attempts,
    thetadatadx_config_set_reconnect_max_elapsed_secs,
    thetadatadx_config_set_reconnect_max_rate_limited_attempts,
    thetadatadx_config_set_reconnect_max_server_restart_attempts,
    thetadatadx_config_set_reconnect_policy, thetadatadx_config_set_reconnect_replay_burst_size,
    thetadatadx_config_set_reconnect_replay_pace_ms,
    thetadatadx_config_set_reconnect_stable_window_secs,
    thetadatadx_config_set_reconnect_wait_max_ms, thetadatadx_config_set_reconnect_wait_ms,
    thetadatadx_config_set_reconnect_wait_rate_limited_ms,
    thetadatadx_config_set_reconnect_wait_server_restart_ms,
    thetadatadx_config_set_streaming_connect_timeout_ms,
    thetadatadx_config_set_streaming_data_watchdog_ms,
    thetadatadx_config_set_streaming_host_selection,
    thetadatadx_config_set_streaming_host_shuffle_seed,
    thetadatadx_config_set_streaming_io_read_slice_ms,
    thetadatadx_config_set_streaming_keepalive_idle_secs,
    thetadatadx_config_set_streaming_keepalive_interval_secs,
    thetadatadx_config_set_streaming_keepalive_retries,
    thetadatadx_config_set_streaming_ping_interval_ms, thetadatadx_config_set_streaming_ring_size,
    thetadatadx_config_set_streaming_timeout_ms,
};

#[test]
fn streaming_transport_fields_round_trip_through_c_abi() {
    let cfg = thetadatadx_config_production();
    assert!(!cfg.is_null());

    let mut timeout = 0u64;
    let mut connect = 0u64;
    let mut ping = 0u64;
    let mut ring = 0usize;
    let mut io_slice = 0u64;
    let mut watchdog = 0u64;
    let mut ka_idle = 0u64;
    let mut ka_interval = 0u64;
    let mut ka_retries = 0u32;
    let mut host_policy = -1i32;
    let mut seed_present = false;
    let mut seed = 0u64;

    // SAFETY: `cfg` is the non-null, not-yet-freed handle from
    // `thetadatadx_config_production`, mutated/read on a single thread; every
    // out-pointer is a live stack slot for the call.
    unsafe {
        // The scalar streaming knobs are infallible unit-returning
        // setters; `host_selection` / `host_shuffle_seed` validate their
        // input and return an `i32` status.
        thetadatadx_config_set_streaming_timeout_ms(cfg, 10_000);
        thetadatadx_config_set_streaming_connect_timeout_ms(cfg, 5_000);
        thetadatadx_config_set_streaming_ping_interval_ms(cfg, 1_000);
        thetadatadx_config_set_streaming_ring_size(cfg, 8_192);
        thetadatadx_config_set_streaming_io_read_slice_ms(cfg, 50);
        // 0 disables the data watchdog.
        thetadatadx_config_set_streaming_data_watchdog_ms(cfg, 0);
        thetadatadx_config_set_streaming_keepalive_idle_secs(cfg, 10);
        thetadatadx_config_set_streaming_keepalive_interval_secs(cfg, 5);
        thetadatadx_config_set_streaming_keepalive_retries(cfg, 4);
        // Host selection: 1 = the non-default preset (round-trips a
        // non-zero discriminant so a stuck-at-zero getter is caught).
        assert_eq!(thetadatadx_config_set_streaming_host_selection(cfg, 1), 0);
        // The host-shuffle seed carries the widened `(has_value, seed)`
        // ABI shape so the `Some(_)` presence survives the boundary.
        assert_eq!(
            thetadatadx_config_set_streaming_host_shuffle_seed(cfg, true, 42),
            0
        );

        assert_eq!(
            thetadatadx_config_get_streaming_timeout_ms(cfg, &mut timeout),
            0
        );
        assert_eq!(
            thetadatadx_config_get_streaming_connect_timeout_ms(cfg, &mut connect),
            0
        );
        assert_eq!(
            thetadatadx_config_get_streaming_ping_interval_ms(cfg, &mut ping),
            0
        );
        assert_eq!(
            thetadatadx_config_get_streaming_ring_size(cfg, &mut ring),
            0
        );
        assert_eq!(
            thetadatadx_config_get_streaming_io_read_slice_ms(cfg, &mut io_slice),
            0
        );
        assert_eq!(
            thetadatadx_config_get_streaming_data_watchdog_ms(cfg, &mut watchdog),
            0
        );
        assert_eq!(
            thetadatadx_config_get_streaming_keepalive_idle_secs(cfg, &mut ka_idle),
            0
        );
        assert_eq!(
            thetadatadx_config_get_streaming_keepalive_interval_secs(cfg, &mut ka_interval),
            0
        );
        assert_eq!(
            thetadatadx_config_get_streaming_keepalive_retries(cfg, &mut ka_retries),
            0
        );
        assert_eq!(
            thetadatadx_config_get_streaming_host_selection(cfg, &mut host_policy),
            0
        );
        assert_eq!(
            thetadatadx_config_get_streaming_host_shuffle_seed(cfg, &mut seed_present, &mut seed),
            0
        );
    }

    assert_eq!(timeout, 10_000);
    assert_eq!(connect, 5_000);
    assert_eq!(ping, 1_000);
    assert_eq!(ring, 8_192);
    assert_eq!(io_slice, 50);
    assert_eq!(watchdog, 0);
    assert_eq!(ka_idle, 10);
    assert_eq!(ka_interval, 5);
    assert_eq!(ka_retries, 4);
    assert_eq!(host_policy, 1);
    assert!(seed_present);
    assert_eq!(seed, 42);

    // SAFETY: `cfg` is owned here and freed exactly once.
    unsafe { thetadatadx_config_free(cfg) };
}

#[test]
fn reconnect_ladder_fields_round_trip_through_c_abi() {
    let cfg = thetadatadx_config_production();
    assert!(!cfg.is_null());

    let mut policy = -1i32;
    let mut max_attempts = 0u32;
    let mut max_rate_limited = 0u32;
    let mut wait_ms = 0u64;
    let mut wait_rate_limited = 0u64;
    let mut wait_max = 0u64;
    let mut wait_server_restart = 0u64;
    let mut jitter = -1i32;
    let mut stable_window = 0u64;
    let mut max_elapsed = 0u64;
    let mut max_server_restart = 0u32;
    let mut replay_burst = 0u32;
    let mut replay_pace = 0u64;

    // SAFETY: `cfg` is the non-null, not-yet-freed handle, mutated/read on
    // one thread; every out-pointer is a live stack slot for the call.
    unsafe {
        // Policy 0 = the Auto ladder. The per-class budget + wait knobs
        // below live inside the Auto limits, so the policy must be Auto
        // for them to forward (a Manual policy ignores the ladder fields).
        // `policy` / `jitter` validate their discriminant and return an
        // `i32` status; the remaining ladder knobs are infallible
        // unit-returning setters.
        assert_eq!(thetadatadx_config_set_reconnect_policy(cfg, 0), 0);
        thetadatadx_config_set_reconnect_max_attempts(cfg, 7);
        thetadatadx_config_set_reconnect_max_rate_limited_attempts(cfg, 3);
        thetadatadx_config_set_reconnect_wait_ms(cfg, 500);
        thetadatadx_config_set_reconnect_wait_rate_limited_ms(cfg, 2_000);
        thetadatadx_config_set_reconnect_wait_max_ms(cfg, 30_000);
        thetadatadx_config_set_reconnect_wait_server_restart_ms(cfg, 15_000);
        // Jitter 1 = the non-default mode.
        assert_eq!(thetadatadx_config_set_reconnect_jitter(cfg, 1), 0);
        thetadatadx_config_set_reconnect_stable_window_secs(cfg, 60);
        thetadatadx_config_set_reconnect_max_elapsed_secs(cfg, 600);
        thetadatadx_config_set_reconnect_max_server_restart_attempts(cfg, 5);
        thetadatadx_config_set_reconnect_replay_burst_size(cfg, 256);
        thetadatadx_config_set_reconnect_replay_pace_ms(cfg, 10);

        assert_eq!(thetadatadx_config_get_reconnect_policy(cfg, &mut policy), 0);
        assert_eq!(
            thetadatadx_config_get_reconnect_max_attempts(cfg, &mut max_attempts),
            0
        );
        assert_eq!(
            thetadatadx_config_get_reconnect_max_rate_limited_attempts(cfg, &mut max_rate_limited),
            0
        );
        assert_eq!(
            thetadatadx_config_get_reconnect_wait_ms(cfg, &mut wait_ms),
            0
        );
        assert_eq!(
            thetadatadx_config_get_reconnect_wait_rate_limited_ms(cfg, &mut wait_rate_limited),
            0
        );
        assert_eq!(
            thetadatadx_config_get_reconnect_wait_max_ms(cfg, &mut wait_max),
            0
        );
        assert_eq!(
            thetadatadx_config_get_reconnect_wait_server_restart_ms(cfg, &mut wait_server_restart),
            0
        );
        assert_eq!(thetadatadx_config_get_reconnect_jitter(cfg, &mut jitter), 0);
        assert_eq!(
            thetadatadx_config_get_reconnect_stable_window_secs(cfg, &mut stable_window),
            0
        );
        assert_eq!(
            thetadatadx_config_get_reconnect_max_elapsed_secs(cfg, &mut max_elapsed),
            0
        );
        assert_eq!(
            thetadatadx_config_get_reconnect_max_server_restart_attempts(
                cfg,
                &mut max_server_restart
            ),
            0
        );
        assert_eq!(
            thetadatadx_config_get_reconnect_replay_burst_size(cfg, &mut replay_burst),
            0
        );
        assert_eq!(
            thetadatadx_config_get_reconnect_replay_pace_ms(cfg, &mut replay_pace),
            0
        );
    }

    assert_eq!(policy, 0);
    assert_eq!(max_attempts, 7);
    assert_eq!(max_rate_limited, 3);
    assert_eq!(wait_ms, 500);
    assert_eq!(wait_rate_limited, 2_000);
    assert_eq!(wait_max, 30_000);
    assert_eq!(wait_server_restart, 15_000);
    assert_eq!(jitter, 1);
    assert_eq!(stable_window, 60);
    assert_eq!(max_elapsed, 600);
    assert_eq!(max_server_restart, 5);
    assert_eq!(replay_burst, 256);
    assert_eq!(replay_pace, 10);

    // SAFETY: `cfg` is owned here and freed exactly once.
    unsafe { thetadatadx_config_free(cfg) };
}
