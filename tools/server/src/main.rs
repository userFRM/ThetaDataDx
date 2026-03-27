//! # thetadatadx-server -- Drop-in Rust replacement for the ThetaData Java Terminal
//!
//! Runs a local HTTP REST server (default :25503) and WebSocket server
//! (default :25520) that expose the same API as the Java terminal. Existing
//! clients (Python SDK, Excel, curl, browsers) connect without code changes.
//!
//! ## Architecture
//!
//! ```text
//! External apps (Python, Excel, browsers)
//!     |
//!     |--- HTTP REST :25503 (/v3/...)
//!     |--- WebSocket :25520 (/v1/events)
//!     |
//! thetadatadx-server (this binary)
//!     |
//!     |--- DirectClient (MDDS gRPC) for historical data
//!     |--- FpssClient (FPSS TCP) for real-time streaming
//!     |
//! ThetaData upstream servers
//! ```

mod format;
mod handler;
mod router;
mod state;
mod ws;

use std::net::SocketAddr;

use clap::Parser;
use tower_http::cors::CorsLayer;
use tracing_subscriber::EnvFilter;

use thetadatadx::{Credentials, DirectConfig, ThetaDataDx};

use crate::state::AppState;

// ---------------------------------------------------------------------------
//  CLI arguments
// ---------------------------------------------------------------------------

/// Drop-in replacement for the ThetaData Java Terminal.
#[derive(Parser, Debug)]
#[command(name = "thetadatadx-server", version, about)]
struct Args {
    /// Path to credentials file (email on line 1, password on line 2).
    #[arg(long, default_value = "creds.txt")]
    creds: String,

    /// HTTP REST API port (default matches Java terminal: 25503).
    #[arg(long, default_value_t = 25503)]
    http_port: u16,

    /// WebSocket server port (default matches Java terminal: 25520).
    #[arg(long, default_value_t = 25520)]
    ws_port: u16,

    /// Bind address for both servers (127.0.0.1 only, not 0.0.0.0).
    #[arg(long, default_value = "127.0.0.1")]
    bind: String,

    /// Log level filter (e.g. "info", "debug", "thetadatadx=trace").
    #[arg(long, default_value = "info")]
    log_level: String,

    /// Skip FPSS (streaming) connection at startup.
    #[arg(long)]
    no_fpss: bool,
}

// ---------------------------------------------------------------------------
//  Main
// ---------------------------------------------------------------------------

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();

    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(&args.log_level)),
        )
        .init();

    // Generate a random shutdown token and print it.
    let shutdown_token = uuid::Uuid::new_v4().to_string();

    tracing::info!(
        version = env!("CARGO_PKG_VERSION"),
        http_port = args.http_port,
        ws_port = args.ws_port,
        bind = %args.bind,
        shutdown_token = %shutdown_token,
        "starting thetadatadx-server"
    );

    eprintln!("Shutdown token: {shutdown_token}");
    eprintln!(
        "  curl http://{}:{}/v3/system/shutdown -H 'X-Shutdown-Token: {}'",
        args.bind, args.http_port, shutdown_token
    );

    // Step 1: Load credentials and authenticate.
    let creds = Credentials::from_file(&args.creds)?;
    tracing::info!(creds_file = %args.creds, "loaded credentials");

    // Step 2: Connect unified client (gRPC historical).
    let config = DirectConfig::production();
    let tdx = ThetaDataDx::connect(&creds, config).await?;
    tracing::info!("MDDS connected");

    // Step 3: Build shared state.
    let state = AppState::new(tdx, shutdown_token);

    // Step 4: Start FPSS streaming bridge.
    if !args.no_fpss {
        match ws::start_fpss_bridge(state.clone()) {
            Ok(()) => {
                tracing::info!("FPSS bridge connected");
            }
            Err(e) => {
                tracing::warn!(error = %e, "FPSS bridge failed to connect (streaming unavailable)");
            }
        }
    } else {
        tracing::info!("FPSS bridge skipped (--no-fpss)");
    }

    // Step 5: Build HTTP REST server with CORS.
    let allowed_origin = format!("http://{}:{}", args.bind, args.http_port);
    let cors = CorsLayer::new()
        .allow_origin(
            allowed_origin
                .parse::<axum::http::HeaderValue>()
                .map_err(|e| format!("invalid CORS origin: {e}"))?,
        )
        .allow_methods([axum::http::Method::GET])
        .allow_headers(tower_http::cors::Any);

    let http_app = router::build(state.clone()).layer(cors);
    let http_addr: SocketAddr = format!("{}:{}", args.bind, args.http_port).parse()?;

    // Step 6: Build WebSocket server.
    let ws_app = ws::router(state.clone());
    let ws_addr: SocketAddr = format!("{}:{}", args.bind, args.ws_port).parse()?;

    // Step 7: Start both servers concurrently.
    tracing::info!(%http_addr, "HTTP REST server starting");
    tracing::info!(%ws_addr, "WebSocket server starting");

    let shutdown_state = state.clone();
    let http_server = axum::serve(
        tokio::net::TcpListener::bind(http_addr).await?,
        http_app.into_make_service(),
    )
    .with_graceful_shutdown(shutdown_signal(shutdown_state.clone()));

    let ws_server = axum::serve(
        tokio::net::TcpListener::bind(ws_addr).await?,
        ws_app.into_make_service(),
    )
    .with_graceful_shutdown(shutdown_signal(shutdown_state));

    tokio::select! {
        result = http_server => {
            if let Err(e) = result {
                tracing::error!(error = %e, "HTTP server error");
            }
        }
        result = ws_server => {
            if let Err(e) = result {
                tracing::error!(error = %e, "WebSocket server error");
            }
        }
        _ = tokio::signal::ctrl_c() => {
            tracing::info!("ctrl-c received, shutting down");
            state.shutdown();
        }
    }

    tracing::info!("thetadatadx-server stopped");
    Ok(())
}

/// Combined shutdown signal: either ctrl-c or the AppState shutdown notification.
async fn shutdown_signal(state: AppState) {
    tokio::select! {
        _ = state.shutdown_signal() => {}
        _ = tokio::signal::ctrl_c() => {}
    }
}
