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
mod validation;
mod ws;

use std::net::SocketAddr;

use clap::Parser;
use tower_http::cors::CorsLayer;
use tracing_subscriber::EnvFilter;
use zeroize::Zeroizing;

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

    /// Email for ThetaData authentication (alternative to --creds file).
    #[arg(long)]
    email: Option<String>,

    /// Password for ThetaData authentication (alternative to --creds file).
    #[arg(long)]
    password: Option<String>,

    /// Path to TOML config file (same format as Java terminal's config.toml).
    #[arg(long)]
    config: Option<String>,

    /// FPSS region: "production" (default), "dev", "stage".
    #[arg(long, default_value = "production")]
    fpss_region: String,

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

    /// Disable OHLCVC bar derivation from trades on the FPSS stream.
    #[arg(long)]
    no_ohlcvc: bool,
}

// ---------------------------------------------------------------------------
//  Main
// ---------------------------------------------------------------------------

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    rustls::crypto::ring::default_provider()
        .install_default()
        .expect("failed to install rustls crypto provider");

    let mut args = Args::parse();

    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(&args.log_level)),
        )
        .init();

    // Generate a random shutdown token and print it.
    let shutdown_token = uuid::Uuid::new_v4().to_string();

    // Startup banner matching the Java terminal style.
    let version = env!("CARGO_PKG_VERSION");
    eprintln!();
    eprintln!("ThetaDataDx Server v{version}");
    eprintln!("Configuration: {}", args.fpss_region);
    eprintln!("REST API: http://{}:{}/", args.bind, args.http_port);
    eprintln!("WebSocket: ws://{}:{}/v1/events", args.bind, args.ws_port);
    eprintln!();
    eprintln!("Shutdown token: {shutdown_token}");
    eprintln!(
        "  curl -X POST http://{}:{}/v3/system/shutdown -H 'X-Shutdown-Token: {}'",
        args.bind, args.http_port, shutdown_token
    );

    // The shutdown token is deliberately NOT part of the structured log --
    // structured logs flow to aggregators / SIEMs / persisted buffers and
    // the token is a bearer credential for the shutdown endpoint. The
    // eprintln! banner above already prints it once to stderr for the
    // operator starting the process; keeping it out of `tracing::info!`
    // means it never reaches a log pipeline.
    tracing::info!(
        version,
        http_port = args.http_port,
        ws_port = args.ws_port,
        bind = %args.bind,
        "starting thetadatadx-server"
    );

    // Step 1: Load credentials -- prefer --email/--password over --creds file.
    //
    // # Zeroization of the CLI password
    //
    // `args.password` is a `Option<String>` populated by clap from argv.
    // We cannot make clap allocate it inside `Zeroizing` without a custom
    // value parser, but we can minimize the lifetime of the unzeroized
    // bytes: `args.password.take()` moves the raw `String` out of the
    // clap struct the moment we touch it, then `Zeroizing::new` takes
    // ownership. When the wrapper drops at the end of this scope, its
    // `Drop` impl zeros the backing allocation before it is freed.
    //
    // `Credentials::new` allocates its own internally-zeroized copy (see
    // `crates/thetadatadx/src/auth/creds.rs`), so passing a temporary
    // `String` clone via `as_str().to_string()` is safe: the
    // intermediate `String` is consumed by the `Into<String>` bound and
    // re-wrapped in `Credentials`'s own `Zeroizing<String>`. Our
    // `Zeroizing<String>` here guarantees that by the time this block
    // exits, the clap-produced allocation has been overwritten.
    let creds = if let Some(email) = args.email.as_ref() {
        match args.password.take() {
            Some(raw_password) => {
                let password: Zeroizing<String> = Zeroizing::new(raw_password);
                tracing::info!("loaded credentials from --email/--password flags");
                let c = Credentials::new(email.clone(), password.as_str().to_string());
                drop(password); // explicit for readers; `Zeroizing` scrubs on drop
                c
            }
            None => {
                let c = Credentials::from_file(&args.creds)?;
                tracing::info!(creds_file = %args.creds, "loaded credentials from file");
                c
            }
        }
    } else {
        let c = Credentials::from_file(&args.creds)?;
        tracing::info!(creds_file = %args.creds, "loaded credentials from file");
        c
    };

    // Step 2: Load config -- prefer --config file, then --fpss-region.
    let config = if let Some(config_path) = &args.config {
        tracing::info!(config_file = %config_path, "loaded config from file");
        DirectConfig::from_file(config_path)?
    } else {
        match args.fpss_region.as_str() {
            "dev" => DirectConfig::dev(),
            "stage" => DirectConfig::stage(),
            _ => DirectConfig::production(),
        }
    };

    // Step 2b: Apply CLI overrides to config.
    let config = if args.no_ohlcvc {
        config.derive_ohlcvc(false)
    } else {
        config
    };

    // Step 3: Connect unified client (gRPC historical).
    let tdx = ThetaDataDx::connect(&creds, config).await?;
    tracing::info!("MDDS connected");

    // Step 4: Build shared state.
    let state = AppState::new(tdx, shutdown_token);

    // Step 5: Start FPSS streaming bridge.
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

    // Step 6: Build HTTP REST server with CORS.
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

    // Step 7: Build WebSocket server.
    let ws_app = ws::router(state.clone());
    let ws_addr: SocketAddr = format!("{}:{}", args.bind, args.ws_port).parse()?;

    // Step 8: Start both servers concurrently.
    tracing::info!(%http_addr, "HTTP REST server starting");
    tracing::info!(%ws_addr, "WebSocket server starting");

    let shutdown_state = state.clone();
    // `into_make_service_with_connect_info::<SocketAddr>()` is required for
    // the per-IP rate limiter (`tower_governor::PeerIpKeyExtractor`) to
    // read the peer address on each request. Without it, the PeerIp key
    // extractor falls back to rejecting every request as "no client IP".
    // PR #378 switched the extractor from SmartIp to PeerIp so downstream
    // clients can't bypass the rate limit by forging `X-Forwarded-For`.
    let http_server = axum::serve(
        tokio::net::TcpListener::bind(http_addr).await?,
        http_app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .with_graceful_shutdown(shutdown_signal(shutdown_state.clone()));

    let ws_server = axum::serve(
        tokio::net::TcpListener::bind(ws_addr).await?,
        ws_app.into_make_service_with_connect_info::<SocketAddr>(),
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
