//! # thetadatadx-server -- Drop-in Rust replacement for the ThetaData JVM terminal
//!
//! Runs a local HTTP REST server (default :25503) and WebSocket server
//! (default :25520) that expose the same API as the JVM terminal. Existing
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
//!     |--- HistoricalClient (MDDS historical) for historical data
//!     |--- StreamingClient (FPSS streaming) for real-time streaming
//!     |
//! ThetaData upstream servers
//! ```

mod flatfile_routes;
mod format;
mod handler;
mod logging;
mod router;
mod row;
mod state;
mod validation;
mod ws;

use std::io::{IsTerminal, Write};
use std::net::SocketAddr;
use std::path::Path;

use clap::Parser;
use tower_http::cors::CorsLayer;
use zeroize::Zeroizing;

use thetadatadx::config::{HistoricalEnvironment, StreamingEnvironment};
use thetadatadx::{Client, Credentials, DirectConfig};

use crate::state::AppState;

/// A random 128-bit token as 32 lowercase hex chars. Backs the shutdown
/// token and the flat-file scratch-path suffix — both want an unguessable,
/// filesystem-safe unique string.
pub(crate) fn random_hex_token() -> String {
    use std::fmt::Write as _;
    let bytes: [u8; 16] = rand::random();
    bytes.iter().fold(String::with_capacity(32), |mut s, b| {
        let _ = write!(s, "{b:02x}");
        s
    })
}

// ---------------------------------------------------------------------------
//  CLI arguments
// ---------------------------------------------------------------------------

/// Drop-in replacement for the ThetaData JVM terminal.
#[derive(Parser, Debug)]
#[command(name = "thetadatadx-server", version, about)]
struct Args {
    /// Path to credentials file (email on line 1, password on line 2).
    #[arg(long, default_value = "creds.txt")]
    creds: String,

    /// Authenticate with a ThetaData API key (or set THETADATA_API_KEY).
    #[arg(long)]
    api_key: Option<String>,

    /// Email for ThetaData authentication (or set THETADATA_EMAIL +
    /// THETADATA_PASSWORD, or use a --creds file).
    #[arg(long)]
    email: Option<String>,

    /// Password for ThetaData authentication (or set THETADATA_EMAIL +
    /// THETADATA_PASSWORD, or use a --creds file).
    #[arg(long)]
    password: Option<String>,

    /// Path to TOML config file (same format as JVM terminal's config.toml).
    #[arg(long)]
    config: Option<String>,

    /// Streaming environment: "production" (default) or "dev".
    /// Selects the streaming channel independently of the historical
    /// channel; an invalid value is rejected at parse time.
    #[arg(long, default_value = "production", value_parser = ["production", "dev"])]
    streaming_region: String,

    /// Historical environment: "production" (default) or "stage".
    /// Selects the historical channel independently of the streaming
    /// channel and also drives the authentication marker; an invalid
    /// value is rejected at parse time.
    #[arg(long, default_value = "production", value_parser = ["production", "stage"])]
    historical_region: String,

    /// HTTP REST API port (default matches JVM terminal: 25503).
    #[arg(long, default_value_t = 25503)]
    http_port: u16,

    /// WebSocket server port (default matches JVM terminal: 25520).
    #[arg(long, default_value_t = 25520)]
    ws_port: u16,

    /// Bind address for both the HTTP REST and WebSocket servers. Defaults
    /// to `0.0.0.0` (all interfaces), matching the JVM terminal this server
    /// replaces. Set `--bind 127.0.0.1` to restrict to loopback only.
    #[arg(long, default_value = "0.0.0.0")]
    bind: String,

    /// Log level filter (e.g. "info", "debug", "thetadatadx=trace").
    /// The per-request access log emits at "info" under the
    /// "tower_http" target; silence it with e.g. "info,tower_http=off".
    #[arg(long, default_value = "info")]
    log_level: String,

    /// Also write logs to this path, rotated daily (e.g. "terminal.log"
    /// produces "terminal.log.YYYY-MM-DD"). Stderr output is unaffected.
    #[arg(long)]
    log_file: Option<String>,

    /// Log line format: "text" (default), "json", or "legacy"
    /// (bracketed "[YYYY-MM-DD HH:MM:SS] LEVEL: message", UTC).
    #[arg(long, value_enum, default_value_t = logging::LogFormat::Text)]
    log_format: logging::LogFormat,

    /// Skip the streaming connection at startup.
    #[arg(long)]
    no_streaming: bool,
}

/// Canonical environment variable names, shared with the SDK, the CLI, and
/// the MCP server so the same value authenticates every tool without
/// per-tool divergence.
///
/// `THETADATA_API_KEY` matches the standard variable the SDKs read via
/// `Credentials::from_env_or_file`; `THETADATA_EMAIL` + `THETADATA_PASSWORD`
/// match the pair `Credentials::from_dotenv` reads, so the same login works
/// whether it is exported into the process environment or supplied by flag
/// or creds file.
const API_KEY_ENV: &str = "THETADATA_API_KEY";
/// Environment variable that supplies the account email.
const EMAIL_ENV: &str = "THETADATA_EMAIL";
/// Environment variable that supplies the account password.
const PASSWORD_ENV: &str = "THETADATA_PASSWORD";

// ---------------------------------------------------------------------------
//  Credential source selection
// ---------------------------------------------------------------------------

/// Which authentication path the resolved CLI arguments and environment
/// select, before any credential value is constructed or any file is read.
///
/// Splitting the *decision* from the *construction* keeps the precedence
/// rules pure and unit-testable: the decision never touches the filesystem,
/// stdin, or the environment beyond a single presence check, so a test can
/// assert the precedence without a live upstream or an interactive prompt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CredentialSource {
    /// An explicit `--api-key` flag was passed; use that key directly.
    ApiKeyFlag,
    /// No flag, but `THETADATA_API_KEY` is set; source the key from the
    /// environment, falling back to the creds file when the variable is
    /// empty (delegated to `Credentials::from_env_or_file`).
    EnvApiKeyOrFile,
    /// No api key anywhere, but both `THETADATA_EMAIL` and
    /// `THETADATA_PASSWORD` are set; build email + password credentials
    /// from the environment.
    EnvEmailPassword,
    /// None of the above; use the flag/file email/password path
    /// (`--email`/`--password`/`--creds`/`creds.txt`).
    EmailPasswordFlagOrFile,
}

/// Decide which credential source to use from the presence of the
/// `--api-key` flag, the `THETADATA_API_KEY` variable, and the complete
/// `THETADATA_EMAIL` + `THETADATA_PASSWORD` pair.
///
/// Precedence (highest first): explicit `--api-key` flag, then the
/// `THETADATA_API_KEY` environment variable, then the
/// `THETADATA_EMAIL` + `THETADATA_PASSWORD` environment pair, then the
/// flag/file email/password path. This matches the CLI and MCP resolvers
/// and the SDK ordering, where an explicit constructor argument wins over
/// the environment, which in turn wins over the creds file.
fn select_credential_source(
    api_key_flag: bool,
    env_api_key_present: bool,
    env_email_password_present: bool,
) -> CredentialSource {
    if api_key_flag {
        CredentialSource::ApiKeyFlag
    } else if env_api_key_present {
        CredentialSource::EnvApiKeyOrFile
    } else if env_email_password_present {
        CredentialSource::EnvEmailPassword
    } else {
        CredentialSource::EmailPasswordFlagOrFile
    }
}

/// Whether an environment variable is set to a non-empty (after trim) value.
///
/// An empty or whitespace-only variable is treated as absent so it does not
/// shadow a lower-precedence credential source, mirroring
/// `Credentials::from_env_or_file`.
fn env_var_present(name: &str) -> bool {
    std::env::var(name).is_ok_and(|v| !v.trim().is_empty())
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

    // Initialise tracing. The returned guard owns the non-blocking
    // file-writer thread; it must live until process exit or buffered
    // log lines are lost on shutdown.
    let _log_guard = logging::init(&args.log_level, args.log_format, args.log_file.as_deref())?;

    // Generate a random shutdown token and print it.
    let shutdown_token = random_hex_token();

    // Startup banner. Named after the binary so operator automation
    // matching the banner string keys on the same identifier as the
    // process list and the docs.
    let version = env!("CARGO_PKG_VERSION");
    eprintln!();
    eprintln!("thetadatadx-server v{version}");
    eprintln!(
        "Configuration: Historical: {}, Streaming: {}",
        args.historical_region, args.streaming_region
    );
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

    // Step 1: Load credentials.
    //
    // Precedence (highest first): an explicit `--api-key` flag, then the
    // `THETADATA_API_KEY` environment variable, then the
    // `THETADATA_EMAIL` + `THETADATA_PASSWORD` environment pair, then the
    // flag/file email/password path (`--email`/`--password`/`--creds`/
    // `creds.txt`). This mirrors the CLI and MCP resolvers and the SDK
    // ordering so the server and every binding accept the same credential
    // without per-tool divergence.
    //
    // The API key is a secret: it is never logged or echoed. The
    // "loaded credentials" lines below name the source generically ("api
    // key") and never interpolate the key, matching how the password is
    // handled (it is never printed either).
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
    // `thetadatadx-rs/src/auth/creds.rs`), so passing a temporary
    // `String` clone via `as_str().to_string()` is safe: the
    // intermediate `String` is consumed by the `Into<String>` bound and
    // re-wrapped in `Credentials`'s own `Zeroizing<String>`. Our
    // `Zeroizing<String>` here guarantees that by the time this block
    // exits, the clap-produced allocation has been overwritten.
    // An empty / whitespace-only `--api-key` is treated as unset so it falls
    // through to the lower-precedence sources instead of shadowing them with a
    // blank key that could never authenticate. Symmetric with the empty-env
    // handling and the CLI binary.
    let api_key_flag = args.api_key.take().filter(|k| !k.trim().is_empty());
    let creds = match select_credential_source(
        api_key_flag.is_some(),
        env_var_present(API_KEY_ENV),
        env_var_present(EMAIL_ENV) && env_var_present(PASSWORD_ENV),
    ) {
        CredentialSource::ApiKeyFlag => {
            // Move the key out into `Zeroizing` so the argv-sourced allocation
            // is scrubbed when this scope exits. `Credentials::api_key` keeps
            // its own zeroized copy. The key itself is never logged.
            let raw_key = api_key_flag.expect("api_key is Some on this arm");
            let key: Zeroizing<String> = Zeroizing::new(raw_key);
            tracing::info!("loaded credentials from --api-key flag");
            let c = Credentials::api_key(key.as_str());
            drop(key); // explicit for readers; `Zeroizing` scrubs on drop
            c
        }
        CredentialSource::EnvApiKeyOrFile => {
            // `from_env_or_file` reads `THETADATA_API_KEY` (already known
            // non-empty) and keeps the key inside its own `Zeroizing`
            // buffer; the key is never surfaced here.
            tracing::info!("loaded credentials from the THETADATA_API_KEY environment variable");
            Credentials::from_env_or_file(&args.creds)?
        }
        CredentialSource::EnvEmailPassword => {
            // Both values are known non-empty. Move the password into a
            // `Zeroizing` buffer so the env-sourced allocation is scrubbed
            // on drop; `Credentials::new` keeps its own zeroized copy.
            let email = std::env::var(EMAIL_ENV).unwrap_or_default();
            let password: Zeroizing<String> =
                Zeroizing::new(std::env::var(PASSWORD_ENV).unwrap_or_default());
            tracing::info!(
                "loaded credentials from the THETADATA_EMAIL/THETADATA_PASSWORD environment variables"
            );
            let c = Credentials::new(email.trim(), password.trim());
            drop(password); // explicit for readers; `Zeroizing` scrubs on drop
            c
        }
        CredentialSource::EmailPasswordFlagOrFile => {
            if let Some(email) = args.email.as_ref() {
                match args.password.take() {
                    Some(raw_password) => {
                        let password: Zeroizing<String> = Zeroizing::new(raw_password);
                        tracing::info!("loaded credentials from --email/--password flags");
                        let c = Credentials::new(email.clone(), password.as_str().to_string());
                        drop(password); // explicit for readers; `Zeroizing` scrubs on drop
                        c
                    }
                    None => load_or_prompt_credentials(&args.creds)?,
                }
            } else {
                load_or_prompt_credentials(&args.creds)?
            }
        }
    };

    // Step 2: Load config -- prefer --config file, then compose the two
    // per-channel environments from --historical-region and --streaming-region.
    //
    // The historical (MDDS) and streaming (FPSS) channels select their
    // environment independently, so we start from the all-production config
    // and apply each axis with its own setter. Composing this way (rather
    // than reaching for the `stage()` / `dev()` whole-config presets, which
    // each move only one axis) means `--historical-region stage --streaming-region dev`
    // yields historical-staging plus streaming-dev, and every other
    // combination resolves correctly too. The arg parser has already
    // rejected any value outside each channel's allowed set, so the matches
    // below are total over the values that reach here.
    let config = if let Some(config_path) = &args.config {
        tracing::info!(config_file = %config_path, "loaded config from file");
        DirectConfig::from_file(config_path)?
    } else {
        let mut config = DirectConfig::production();
        if args.historical_region == "stage" {
            config = config.with_historical_environment(HistoricalEnvironment::Stage);
        }
        if args.streaming_region == "dev" {
            config = config.with_streaming_environment(StreamingEnvironment::Dev);
        }
        config
    };

    // Step 3: Connect unified client (gRPC historical).
    let client = Client::connect(&creds, config).await?;
    tracing::info!("MDDS connected");

    // Step 4: Build shared state.
    let state = AppState::new(client, shutdown_token);

    // Step 5: Start FPSS streaming bridge.
    if !args.no_streaming {
        match ws::start_fpss_bridge(state.clone()) {
            Ok(()) => {
                tracing::info!("FPSS bridge connected");
            }
            Err(e) => {
                tracing::warn!(error = %e, "FPSS bridge failed to connect (streaming unavailable)");
            }
        }
    } else {
        tracing::info!("FPSS bridge skipped (--no-streaming)");
    }

    // Step 6: Build HTTP REST server with CORS.
    //
    // Permissive by design, matching the legacy terminal: browser-based
    // dashboards on any local origin must be able to call both the GET
    // data routes and the POST routes (`/v3/system/shutdown`,
    // `/v3/flatfile/request`). The previous configuration pinned
    // `allow_origin` to the server's own listener address — a client
    // running on the server's origin IS the server, so the restriction
    // blocked every real browser client while protecting nothing — and
    // `allow_methods=[GET]` failed every POST preflight. Real protection
    // for the mutating route is the `X-Shutdown-Token` header plus the
    // route-scoped rate limiter, not CORS.
    let cors = CorsLayer::new()
        .allow_origin(tower_http::cors::Any)
        .allow_methods([axum::http::Method::GET, axum::http::Method::POST])
        .allow_headers(tower_http::cors::Any);

    // The terminal this server replaces does no per-IP rate limiting, so
    // the default must not either: with neither rate-limit env var set the
    // general per-IP governor is attached nowhere, regardless of the bind
    // address. An operator exposing the server as a relay opts in by
    // setting THETADATADX_RATE_LIMIT_PER_SECOND and/or
    // THETADATADX_RATE_LIMIT_BURST_SIZE. The same resolved pair drives both
    // the HTTP general governor and the WS upgrade governor; the tighter
    // shutdown-route limiter stays active on every bind regardless.
    let rate_limit = router::resolve_rate_limit();
    match rate_limit {
        Some((per_second, burst_size)) => tracing::info!(
            per_second,
            burst_size,
            "general per-IP rate limiter enabled by operator (opt-in)"
        ),
        None => tracing::info!(
            "general per-IP rate limiter disabled (default; shutdown-route limiter stays active)"
        ),
    }

    let http_app = router::build(state.clone(), rate_limit).layer(cors);
    let http_addr: SocketAddr = format!("{}:{}", args.bind, args.http_port).parse()?;

    // Step 7: Build WebSocket server.
    let ws_app = ws::router(state.clone(), rate_limit);
    let ws_addr: SocketAddr = format!("{}:{}", args.bind, args.ws_port).parse()?;

    // Step 8: Start both servers concurrently.
    tracing::info!(%http_addr, "HTTP REST server starting");
    tracing::info!(%ws_addr, "WebSocket server starting");

    let shutdown_state = state.clone();
    // `into_make_service_with_connect_info::<SocketAddr>()` is required for
    // the per-IP rate limiter (`tower_governor::PeerIpKeyExtractor`) to
    // read the peer address on each request. Without it, the PeerIp key
    // extractor falls back to rejecting every request as "no client IP".
    // The extractor uses PeerIp (not SmartIp) so downstream clients
    // can't bypass the rate limit by forging `X-Forwarded-For`.
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

/// Resolve credentials when neither `--email` nor `--password` was passed.
///
/// Resolution order:
/// 1. If the credentials file at `creds_path` exists, load it (the long-
///    standing `--creds` behaviour: email on line 1, password on line 2).
/// 2. If the file is absent and stdin is an interactive terminal, prompt
///    for email and password on first run, then persist them to
///    `creds_path` in the same two-line format so the prompt does not
///    repeat on the next launch.
/// 3. If the file is absent and stdin is **not** a terminal (CI, a piped
///    invocation, a service manager), fall through to `Credentials::from_file`
///    so the process fails fast with the existing missing-file error instead
///    of blocking forever on a read that will never receive input.
fn load_or_prompt_credentials(creds_path: &str) -> Result<Credentials, Box<dyn std::error::Error>> {
    if Path::new(creds_path).exists() {
        let c = Credentials::from_file(creds_path)?;
        tracing::info!(creds_file = %creds_path, "loaded credentials from file");
        return Ok(c);
    }

    if !std::io::stdin().is_terminal() {
        // Non-interactive and no file: preserve the historical behaviour of
        // erroring on missing credentials rather than hanging on stdin.
        let c = Credentials::from_file(creds_path)?;
        tracing::info!(creds_file = %creds_path, "loaded credentials from file");
        return Ok(c);
    }

    let (email, password) = prompt_credentials(creds_path)?;
    persist_credentials(creds_path, &email, password.as_str())?;
    tracing::info!(creds_file = %creds_path, "saved credentials entered at the first-run prompt");
    let c = Credentials::new(email, password.as_str().to_string());
    drop(password); // explicit for readers; `Zeroizing` scrubs on drop
    Ok(c)
}

/// Prompt the operator for an email and password on first run.
///
/// The email is read from stdin in the clear; the password is read with
/// echo suppressed via `rpassword` and handed back inside `Zeroizing` so the
/// typed bytes are wiped from the heap on drop, matching the `--password`
/// flag path. The password is never logged or echoed.
fn prompt_credentials(
    creds_path: &str,
) -> Result<(String, Zeroizing<String>), Box<dyn std::error::Error>> {
    // Prompts go to stderr, mirroring the startup banner, so stdout stays
    // free for anything a wrapper script might capture.
    eprintln!();
    eprintln!("No credentials file found at \"{creds_path}\".");
    eprintln!("Enter your ThetaData login to continue; it will be saved to that file.");

    eprint!("Email: ");
    std::io::stderr().flush()?;
    let mut email = String::new();
    std::io::stdin().read_line(&mut email)?;
    let email = email.trim().to_string();
    if email.is_empty() {
        return Err("email must not be empty".into());
    }

    // `rpassword` reads without echoing the typed characters. Move the
    // returned `String` straight into `Zeroizing` so the plaintext is
    // scrubbed when this value drops.
    let password = Zeroizing::new(rpassword::prompt_password("Password: ")?);
    if password.trim().is_empty() {
        return Err("password must not be empty".into());
    }

    Ok((email, password))
}

/// Persist first-run credentials to `creds_path` in the two-line format
/// `Credentials::from_file` expects (email on line 1, password on line 2).
///
/// On Unix the file is created with `0600` (owner read/write only) so the
/// saved secret is not world-readable; on other platforms the default
/// permissions apply.
fn persist_credentials(
    creds_path: &str,
    email: &str,
    password: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    use std::fs::OpenOptions;

    let mut options = OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(creds_path)?;
    // Trailing newline keeps the file shape identical to a hand-written
    // creds.txt and round-trips through `Credentials::from_file`, which
    // trims each line.
    write!(file, "{email}\n{password}\n")?;
    file.flush()?;
    Ok(())
}

/// Combined shutdown signal: either ctrl-c or the AppState shutdown notification.
async fn shutdown_signal(state: AppState) {
    tokio::select! {
        _ = state.shutdown_signal() => {}
        _ = tokio::signal::ctrl_c() => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Precedence: an explicit `--api-key` flag wins over everything else,
    // including the environment variable and the env email/password pair.
    #[test]
    fn api_key_flag_takes_precedence_over_env() {
        for env_api_key in [true, false] {
            for env_pair in [true, false] {
                assert_eq!(
                    select_credential_source(true, env_api_key, env_pair),
                    CredentialSource::ApiKeyFlag,
                    "flag must win (env_api_key={env_api_key}, env_pair={env_pair})"
                );
            }
        }
    }

    // With no flag but `THETADATA_API_KEY` present, the env-or-file path is
    // selected so a user who sets only `THETADATA_API_KEY` (no flags, no
    // creds.txt) still authenticates. The api key outranks the env pair.
    #[test]
    fn env_api_key_selects_env_or_file_when_no_flag() {
        assert_eq!(
            select_credential_source(false, true, false),
            CredentialSource::EnvApiKeyOrFile
        );
        assert_eq!(
            select_credential_source(false, true, true),
            CredentialSource::EnvApiKeyOrFile,
            "THETADATA_API_KEY must outrank the THETADATA_EMAIL/PASSWORD pair"
        );
    }

    // With no flag and no `THETADATA_API_KEY`, but the complete
    // `THETADATA_EMAIL` + `THETADATA_PASSWORD` pair present, the bare
    // process-env email/password path is selected, matching the CLI and
    // MCP resolvers so the same exported login authenticates every tool.
    #[test]
    fn env_email_password_selects_env_pair_when_no_api_key() {
        assert_eq!(
            select_credential_source(false, false, true),
            CredentialSource::EnvEmailPassword
        );
    }

    // With no flag, no `THETADATA_API_KEY`, and no env pair, the flag/file
    // email/password path stays in force, so existing invocations are
    // unchanged.
    #[test]
    fn no_env_credentials_falls_back_to_flag_or_file() {
        assert_eq!(
            select_credential_source(false, false, false),
            CredentialSource::EmailPasswordFlagOrFile
        );
    }

    // The `--api-key` arm constructs API-key credentials, and the
    // email/password arm constructs password credentials. This pins the
    // credential *kind* each selected source resolves to.
    #[test]
    fn selected_source_constructs_expected_credential_kind() {
        let api = Credentials::api_key("secret-key");
        assert!(api.is_api_key());
        assert_eq!(api.api_key_secret(), Some("secret-key"));
        assert_eq!(api.password(), None);

        let pw = Credentials::new("you@example.com", "hunter2");
        assert!(!pw.is_api_key());
        assert_eq!(pw.password(), Some("hunter2"));
        assert_eq!(pw.api_key_secret(), None);
    }
}
