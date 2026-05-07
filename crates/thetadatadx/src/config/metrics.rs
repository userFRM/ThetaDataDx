//! Observability sub-configuration (Prometheus exporter binding).

/// Observability binding (Prometheus exporter port).
#[derive(Debug, Clone, Default)]
pub struct MetricsConfig {
    /// Port the Prometheus exporter binds to when the `metrics-prometheus`
    /// cargo feature is enabled. `None` disables the exporter even when the
    /// feature is compiled in; `Some(port)` starts an HTTP listener on
    /// `0.0.0.0:<port>` whose `/metrics` endpoint exposes every counter
    /// and histogram recorded through the `metrics` crate.
    pub port: Option<u16>,
}
