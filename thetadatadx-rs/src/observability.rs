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
//! default and binds the HTTP listener as part of the same install.
//! Calling [`try_install_exporter`] more than once in the same process
//! (a second [`crate::Client::connect`] with metrics enabled) would
//! re-bind the port and fail; the installer tracks its own first
//! success and treats any later install as a benign warn-and-continue,
//! so the SDK stays re-entrant in tests and multi-tenant embeddings.

use crate::config::DirectConfig;

/// Install the Prometheus exporter using the port configured on
/// `config`. Returns `Ok(())` when the feature is disabled or when
/// `config.metrics.port` is `None` — callers don't need to guard at
/// every call site.
///
/// # Errors
///
/// Returns [`crate::error::Error::Config`] if the exporter cannot bind
/// to the configured port. Re-installation in the same process logs a
/// warning and returns `Ok(())`.
pub fn try_install_exporter(config: &DirectConfig) -> Result<(), crate::error::Error> {
    let Some(port) = config.metrics.port else {
        return Ok(());
    };
    install_exporter_impl(port)
}

#[cfg(feature = "metrics-prometheus")]
fn install_exporter_impl(port: u16) -> Result<(), crate::error::Error> {
    use std::sync::atomic::{AtomicBool, Ordering};
    // Re-install detection by our own flag, not by parsing the bind error.
    // `install()` binds the HTTP listener INSIDE `build()`, before it registers
    // the global recorder, so a second connect on the same `metrics_port`
    // fails at bind with EADDRINUSE and never reaches the
    // `FailedToSetGlobalRecorder` arm below — the same-port re-install would
    // otherwise be classified as a hard error and fail `Client::connect`,
    // contradicting the documented "re-install returns Ok". The first
    // exporter already serves the port, so a subsequent install is a benign
    // no-op. A bind error cannot itself distinguish our own listener from a
    // foreign process, so gate on whether WE already installed.
    //
    // ponytail: a Relaxed swap, so two truly-concurrent first installs could
    // race (the loser returns Ok before the winner finishes binding); connect
    // is not driven concurrently on one port in practice. Promote to a `Once`
    // if that changes.
    static INSTALLED: AtomicBool = AtomicBool::new(false);
    if INSTALLED.swap(true, Ordering::Relaxed) {
        tracing::warn!(
            port,
            "Prometheus exporter already installed in this process; re-install ignored"
        );
        return Ok(());
    }
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
        Err(metrics_exporter_prometheus::BuildError::FailedToSetGlobalRecorder(err)) => {
            // A recorder is already registered in this process by code OUTSIDE
            // this installer (a host app or another crate) — the exporter bound
            // its listener but could not become the global recorder. Log and
            // continue so SDK consumers don't see a spurious hard error.
            tracing::warn!(
                error = %err,
                "Prometheus exporter not installed (another recorder is active)"
            );
            Ok(())
        }
        Err(err) => {
            // First install in this process failed to bind (a foreign process
            // holds the port, or the address is unbindable). Clear the flag so
            // a later connect with a corrected `metrics_port` can retry.
            INSTALLED.store(false, Ordering::Relaxed);
            // A real bind / runtime failure (for example the metrics port is
            // already in use, or the address could not be bound). Surface it
            // rather than masking a genuine misconfiguration as a benign
            // re-install.
            Err(crate::error::Error::config_invalid(
                "metrics.port",
                format!("failed to start Prometheus exporter on port {port}: {err}"),
            ))
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
        assert!(config.metrics.port.is_none());
        try_install_exporter(&config).expect("must be a no-op");
    }

    #[test]
    fn with_metrics_port_builder_sets_port() {
        let config = DirectConfig::production_defaults().with_metrics_port(9090);
        assert_eq!(config.metrics.port, Some(9090));
    }
}
