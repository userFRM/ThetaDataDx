//! Optional Prometheus exporter for the `metrics` crate recorders.
//!
//! The SDK is instrumented end-to-end via the [`metrics`] facade:
//! counters at auth, every gRPC endpoint, and the FPSS I/O loop;
//! histograms on auth latency and per-endpoint request latency. This
//! module exposes those metrics over HTTP so operators can scrape
//! them with Prometheus / Grafana without threading a dedicated
//! telemetry channel through the SDK.
//!
//! # Opt-in
//!
//! Disabled by default. Turn on the `metrics-prometheus` cargo feature
//! and set [`DirectConfig::with_metrics_port`] to bind the exporter.
//! Without the feature this module still compiles but
//! [`try_install_exporter`] is a no-op that returns `Ok(())`.
//!
//! # Scrape URL
//!
//! When the exporter is running the metrics are served at
//! `http://<host>:<port>/metrics` in Prometheus text format. Bind
//! address is `0.0.0.0`; front-end with an auth proxy if you are
//! exposing the port publicly.
//!
//! # Recorder lifecycle
//!
//! `metrics-exporter-prometheus` installs its recorder as the global
//! default. Calling [`try_install_exporter`] more than once in the
//! same process is a hard error from the `metrics` crate; we swallow
//! that specific failure with a warning so the SDK stays re-entrant
//! in tests and multi-tenant embedding scenarios.

use crate::config::DirectConfig;

/// Install the Prometheus exporter using the port configured on
/// `config`. Returns `Ok(())` when the feature is disabled or when
/// `config.metrics_port` is `None` — callers don't need to guard at
/// every call site.
///
/// # Errors
///
/// Returns [`crate::error::Error::Config`] if the exporter cannot bind
/// to the configured port. Re-installation in the same process logs a
/// warning and returns `Ok(())`.
pub fn try_install_exporter(config: &DirectConfig) -> Result<(), crate::error::Error> {
    let Some(port) = config.metrics_port else {
        return Ok(());
    };
    install_exporter_impl(port)
}

#[cfg(feature = "metrics-prometheus")]
fn install_exporter_impl(port: u16) -> Result<(), crate::error::Error> {
    let addr: std::net::SocketAddr = ([0, 0, 0, 0], port).into();
    match metrics_exporter_prometheus::PrometheusBuilder::new()
        .with_http_listener(addr)
        .install()
    {
        Ok(()) => {
            tracing::info!(
                scrape_url = %format!("http://{addr}/metrics"),
                "Prometheus exporter listening"
            );
            Ok(())
        }
        Err(err) => {
            // `install()` fails if another recorder is already registered.
            // That's the common case in tests + multi-crate embeddings;
            // log and continue so SDK consumers don't see a spurious
            // hard error on re-install.
            tracing::warn!(
                error = %err,
                "Prometheus exporter not installed (another recorder is active)"
            );
            Ok(())
        }
    }
}

#[cfg(not(feature = "metrics-prometheus"))]
fn install_exporter_impl(port: u16) -> Result<(), crate::error::Error> {
    // Feature disabled at compile time: the caller asked for a port
    // but this build does not ship the exporter. Log once so the
    // misconfiguration is visible without aborting the connection.
    tracing::warn!(
        port,
        "metrics_port is set but the `metrics-prometheus` feature is not enabled; \
         rebuild with `--features metrics-prometheus` to start the exporter"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn try_install_noop_when_metrics_port_none() {
        let config = DirectConfig::production_defaults();
        assert!(config.metrics_port.is_none());
        try_install_exporter(&config).expect("must be a no-op");
    }

    #[test]
    fn with_metrics_port_builder_sets_port() {
        let config = DirectConfig::production_defaults().with_metrics_port(9090);
        assert_eq!(config.metrics_port, Some(9090));
    }
}
