//! Async runtime sub-configuration (tokio worker thread sizing).

/// Async runtime tuning.
///
/// The `thetadatadx` crate itself is runtime-agnostic — every public
/// async entry point runs on whatever runtime the caller provides
/// (typically via `#[tokio::main(flavor = "multi_thread")]` or an
/// explicit `tokio::runtime::Builder::new_multi_thread().build()`).
/// The embedded bindings that DO own their runtime (FFI, the
/// `thetadatadx-py` and `thetadatadx-napi` SDKs) read this struct when
/// the first client in the process connects to size the worker thread
/// pool. That pool is process-global and built once, so the field is
/// honoured for the first client created in the process.
///
/// For Rust callers building their own runtime, use
/// [`RuntimeConfig::build_runtime`] to honour the field directly.
#[derive(Debug, Clone, Default)]
#[non_exhaustive]
pub struct RuntimeConfig {
    /// Number of tokio worker threads.
    ///
    /// * `None` (default) — tokio default sizing (the number of logical
    ///   CPUs visible to the process).
    /// * `Some(0)` — clamps to 1 inside [`Self::build_runtime`] so the
    ///   runtime always has at least one worker; the explicit `Some(0)`
    ///   sentinel survives across binding boundaries.
    /// * `Some(n)` for `n >= 1` — pins the worker pool to exactly `n`.
    ///
    /// JVM equivalent: `-Xmx` + `HTTP_CONCURRENCY` thread pool sizing.
    pub tokio_worker_threads: Option<usize>,
}

impl RuntimeConfig {
    /// Build a fresh multi-threaded tokio runtime honouring
    /// [`Self::tokio_worker_threads`].
    ///
    /// * `None` → `Builder::new_multi_thread().enable_all().build()`
    ///   (tokio default sizing).
    /// * `Some(n)` → same builder with
    ///   `.worker_threads(n.max(1))` applied.
    ///
    /// This is the single helper the bindings (and Rust embedders that
    /// own their runtime) consume to apply the config. The library
    /// itself never calls this — it never owns a runtime — but the
    /// helper lives next to the field it interprets so the
    /// "config-to-runtime" contract is single-source.
    ///
    /// # Errors
    /// Propagates [`std::io::Error`] from
    /// [`tokio::runtime::Builder::build`] (typically `EMFILE` /
    /// `EAGAIN` if the process is out of FDs or threads).
    pub fn build_runtime(&self) -> std::io::Result<tokio::runtime::Runtime> {
        let mut builder = tokio::runtime::Builder::new_multi_thread();
        builder.enable_all();
        if let Some(n) = self.tokio_worker_threads {
            builder.worker_threads(n.max(1));
        }
        builder.build()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_runtime_default_returns_ok() {
        let cfg = RuntimeConfig::default();
        let rt = cfg.build_runtime().expect("default runtime must build");
        rt.block_on(async { 1 + 1 }); // VOCAB-OK: tokio Runtime::block_on in unit test, not PyO3 GIL path
    }

    #[test]
    fn build_runtime_honours_explicit_worker_count() {
        let cfg = RuntimeConfig {
            tokio_worker_threads: Some(2),
        };
        let rt = cfg.build_runtime().expect("2-worker runtime must build");
        let value = rt.block_on(async { 42 }); // VOCAB-OK: tokio Runtime::block_on in unit test, not PyO3 GIL path
        assert_eq!(value, 42);
    }

    #[test]
    fn build_runtime_clamps_zero_to_one() {
        // `Some(0)` is a valid serialized value across the binding
        // matrix — the builder clamps it to 1 so tokio does not panic
        // on `worker_threads(0)`.
        let cfg = RuntimeConfig {
            tokio_worker_threads: Some(0),
        };
        let rt = cfg
            .build_runtime()
            .expect("clamped 0 worker count must build");
        rt.block_on(async {}); // VOCAB-OK: tokio Runtime::block_on in unit test, not PyO3 GIL path
    }
}
