//! Proves `thetadatadx_config_set_worker_threads` is wired through to the embedded
//! async runtime rather than being a no-op.
//!
//! The embedded runtime is a process-global `OnceLock`, so this test runs
//! in its own integration-test binary: it is the only thing that builds
//! the runtime in this process, making the `num_workers()` assertion
//! deterministic.

/// Setting `worker_threads` to a small N must produce a runtime whose
/// worker pool is exactly N. A pre-fix no-op runtime would report the
/// default (one worker per logical CPU) and fail this assertion on any
/// host with more than two cores.
#[test]
fn worker_threads_sizes_the_embedded_runtime() {
    let mut cfg = thetadatadx::RuntimeConfig::default();
    cfg.tokio_worker_threads = Some(2);
    let workers = thetadatadx_ffi::__test_runtime_worker_count(&cfg);
    assert_eq!(
        workers, 2,
        "worker_threads=2 must size the embedded runtime to 2 workers; \
         got {workers} (the knob is a no-op if this is the host CPU count)"
    );
}
