//! Async runtime sub-configuration (tokio worker thread sizing).

/// Async runtime tuning.
#[derive(Debug, Clone, Default)]
pub struct RuntimeConfig {
    /// Number of tokio worker threads. `None` = tokio default (number of CPU cores).
    ///
    /// JVM equivalent: `-Xmx` + `HTTP_CONCURRENCY` thread pool sizing.
    ///
    /// NOTE: Not automatically wired — caller should use this when building
    /// a custom tokio runtime.
    pub tokio_worker_threads: Option<usize>,
}
