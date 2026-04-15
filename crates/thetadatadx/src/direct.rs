//! Direct server client — MDDS gRPC without the Java terminal.
//!
//! `DirectClient` authenticates against the Nexus API, opens a gRPC channel
//! to the MDDS server, and exposes typed methods for every data endpoint.
//! Macro-driven builder patterns (`list_endpoint!`, `parsed_endpoint!`) live in
//! [`crate::macros`] and are applied here
//! via generated code (`include!`) from `endpoint_surface.toml`.
//!
//! # Architecture
//!
//! ```text
//! Credentials --> nexus::authenticate() --> SessionToken
//!                                              |
//!              +-------------------------------+
//!              |
//!       DirectClient
//!        |-- mdds_stub: BetaThetaTerminalClient  (gRPC, historical data)
//!        \-- session: SessionToken               (UUID in every QueryInfo)
//! ```
//!
//! Every MDDS request wraps parameters in a `QueryInfo` that carries the session
//! UUID obtained from Nexus auth. Responses are `stream ResponseData` — zstd-
//! compressed `DataTable` payloads decoded by [`crate::decode`].

use std::collections::HashMap;
use std::future::IntoFuture;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use tokio_stream::StreamExt;

use crate::auth::{self, Credentials, SessionToken};
use crate::config::DirectConfig;
use crate::decode;
use crate::error::Error;
use crate::proto;

use crate::proto::beta_theta_terminal_client::BetaThetaTerminalClient;
use tdbe::types::tick::{
    CalendarDay, EodTick, GreeksTick, InterestRateTick, IvTick, MarketValueTick, OhlcTick,
    OpenInterestTick, OptionContract, PriceTick, QuoteTick, TradeQuoteTick, TradeTick,
};

/// Crate version embedded in `QueryInfo.terminal_version` so `ThetaData` can
/// identify this client in server-side logs.
const CLIENT_TYPE: &str = "rust-thetadatadx";

/// Version string sent in `QueryInfo.terminal_version`.
const TERMINAL_VERSION: &str = env!("CARGO_PKG_VERSION");

// Macros (`list_endpoint!`, `parsed_endpoint!`, and the helper type-mapping
// macros) live in `crate::macros` and are brought into scope by `#[macro_use]`
// in lib.rs. All invocations are now generated from `endpoint_surface.toml` at
// build time.

/// Normalize the `right` parameter for the v3 MDDS server.
///
/// Delegates to [`crate::right::parse_right`] — the single source of truth
/// for accepted `right` forms. Maps `C`/`call` -> `"call"`, `P`/`put` ->
/// `"put"`, `both`/`*` -> `"both"`, case-insensitive.
///
/// # Panics
///
/// Panics with a descriptive message on unrecognised input. Endpoint-layer
/// callers already run `validate_right` before reaching the direct client,
/// so this path is defence-in-depth; use [`crate::right::parse_right`]
/// directly for fallible parsing from untrusted input.
fn normalize_right(right: &str) -> String {
    crate::right::parse_right(right)
        .unwrap_or_else(|err| panic!("{err}"))
        .as_mdds_str()
        .to_string()
}

/// Normalize the `expiration` parameter for the v3 MDDS server.
///
/// The server accepts `"*"` as the canonical wildcard ("all expirations")
/// and explicit `YYYYMMDD` dates, but rejects the legacy v3-terminal
/// sentinel `"0"` with an `InvalidArgument` parse error. This function
/// translates `"0"` -> `"*"` so callers can use either form; everything
/// else passes through unchanged.
fn normalize_expiration(expiration: &str) -> String {
    if expiration == "0" {
        "*".to_string()
    } else {
        expiration.to_string()
    }
}

/// Helper: build a `proto::ContractSpec` from the four standard option params.
macro_rules! contract_spec {
    ($symbol:expr, $expiration:expr, $strike:expr, $right:expr) => {
        Some(proto::ContractSpec {
            symbol: $symbol.to_string(),
            expiration: normalize_expiration(&$expiration.to_string()),
            strike: Some($strike.to_string()),
            right: Some(normalize_right(&$right.to_string())),
        })
    };
}

/// Direct client for `ThetaData` server access.
///
/// Connects to MDDS (gRPC, historical data) without requiring the Java
/// terminal. Authenticates via the Nexus HTTP API, then issues gRPC
/// requests to the upstream MDDS server.
///
/// # Example
///
/// ```rust,no_run
/// use thetadatadx::{ThetaDataDx, Credentials, DirectConfig};
///
/// # async fn run() -> Result<(), thetadatadx::Error> {
/// let creds = Credentials::from_file("creds.txt")?;
/// let tdx = ThetaDataDx::connect(&creds, DirectConfig::production()).await?;
///
/// let eod = tdx.stock_history_eod("AAPL", "20240101", "20240301").await?;
/// println!("{} EOD ticks", eod.len());
/// # Ok(())
/// # }
/// ```
pub struct DirectClient {
    /// Session token from Nexus auth (UUID embedded in every request).
    session: SessionToken,
    /// gRPC channel to MDDS server.
    channel: tonic::transport::Channel,
    /// Configuration snapshot (retained for diagnostics/reconnect).
    config: DirectConfig,
    /// Pre-built `QueryInfo` template — cloned per-request instead of allocating
    /// new Strings each time.
    query_info_template: proto::QueryInfo,
    /// Semaphore limiting concurrent in-flight gRPC requests.
    ///
    /// The Java terminal limits concurrent requests to `2^subscription_tier`
    /// (Free=1, Value=2, Standard=4, Pro=16). This semaphore enforces the same
    /// bound to prevent server-side rate limiting / 429 disconnects.
    request_semaphore: Arc<tokio::sync::Semaphore>,
    /// Per-asset subscription tiers captured from the Nexus auth response.
    stock_tier: Option<i32>,
    options_tier: Option<i32>,
}

// ── Infrastructure (not generated — these are session/transport methods, not ThetaData endpoints) ──

impl DirectClient {
    /// Connect to `ThetaData` servers directly (no JVM terminal needed).
    ///
    /// 1. Authenticates against the Nexus HTTP API to obtain a session UUID.
    /// 2. Opens a gRPC channel (TLS) to the MDDS server.
    ///
    /// The FPSS (real-time streaming) connection is not established here;
    /// it will be added in a future release.
    /// # Errors
    ///
    /// Returns an error on network, authentication, or parsing failure.
    pub async fn connect(creds: &Credentials, config: DirectConfig) -> Result<Self, Error> {
        // Step 1: Authenticate against Nexus API.
        tracing::info!(mdds = %config.mdds_uri(), "authenticating with Nexus API");
        let auth_resp = auth::authenticate(creds).await?;
        let session = SessionToken::from_response(&auth_resp)?;

        tracing::debug!(
            session_id_prefix = %&session.session_uuid[..8.min(session.session_uuid.len())],
            stock_tier = ?auth_resp.user.as_ref().and_then(|u| u.stock_subscription),
            "session established (session_id redacted)"
        );

        // Step 2: Open gRPC channel to MDDS.
        let mdds_uri = config.mdds_uri();
        tracing::debug!(uri = %mdds_uri, "connecting to MDDS gRPC");

        let endpoint = tonic::transport::Channel::from_shared(mdds_uri.clone())
            .map_err(|e| Error::Config(format!("invalid MDDS URI '{mdds_uri}': {e}")))?
            .keep_alive_timeout(Duration::from_secs(config.mdds_keepalive_timeout_secs))
            .http2_keep_alive_interval(Duration::from_secs(config.mdds_keepalive_secs))
            .initial_stream_window_size(
                u32::try_from(config.mdds_window_size_kb * 1024).unwrap_or(u32::MAX),
            )
            .initial_connection_window_size(
                u32::try_from(config.mdds_connection_window_size_kb * 1024).unwrap_or(u32::MAX),
            )
            .connect_timeout(Duration::from_secs(10));

        let endpoint = if config.mdds_tls {
            endpoint.tls_config(tonic::transport::ClientTlsConfig::new().with_enabled_roots())?
        } else {
            endpoint
        };

        let channel = endpoint.connect().await?;
        tracing::info!("MDDS gRPC channel connected");

        let mut query_parameters = HashMap::new();
        // The Java terminal includes "client": "terminal" in every QueryInfo.
        // Source: MddsConnectionManager in decompiled terminal.
        query_parameters.insert("client".to_string(), "terminal".to_string());

        let query_info_template = proto::QueryInfo {
            auth_token: Some(proto::AuthToken {
                session_uuid: session.session_uuid.clone(),
            }),
            query_parameters,
            client_type: CLIENT_TYPE.to_string(),
            // Intentional divergence from Java (see jvm-deviations.md):
            // Java fills this with the terminal's build git commit hash.
            // We are not the Java terminal and have no git commit to report,
            // so we leave it empty. The server accepts empty strings here.
            terminal_git_commit: String::new(),
            terminal_version: TERMINAL_VERSION.to_string(),
        };

        // Auto-detect concurrency from subscription tier when config is 0.
        // Source: Java terminal uses 2^subscription_tier (FREE=1, VALUE=2, STANDARD=4, PRO=8).
        let concurrent = if config.mdds_concurrent_requests == 0 {
            auth_resp
                .user
                .as_ref()
                .map_or(2, super::auth::nexus::AuthUser::max_concurrent_requests)
        } else {
            config.mdds_concurrent_requests
        };

        let request_semaphore = Arc::new(tokio::sync::Semaphore::new(concurrent));

        tracing::debug!(
            mdds_concurrent_requests = concurrent,
            auto_detected = config.mdds_concurrent_requests == 0,
            "request semaphore initialized"
        );

        let stock_tier = auth_resp.user.as_ref().and_then(|u| u.stock_subscription);
        let options_tier = auth_resp.user.as_ref().and_then(|u| u.options_subscription);

        Ok(Self {
            session,
            channel,
            config,
            query_info_template,
            request_semaphore,
            stock_tier,
            options_tier,
        })
    }

    /// Return a clone of the pre-built `QueryInfo` template.
    ///
    /// The template is constructed once at connection time, avoiding per-call
    /// String allocations for session UUID, client type, and version.
    #[inline]
    fn query_info(&self) -> proto::QueryInfo {
        self.query_info_template.clone()
    }

    /// Create a new gRPC stub from the shared channel.
    ///
    /// Tonic channels are cheap to clone (internally Arc'd), and stubs take
    /// `&mut self` for each call, so we mint a fresh stub per request to
    /// allow concurrent requests without external `Mutex`.
    fn stub(&self) -> BetaThetaTerminalClient<tonic::transport::Channel> {
        BetaThetaTerminalClient::new(self.channel.clone())
            // MDDS can return large DataTables (e.g. full day of trades).
            // Uses the config-specified max message size.
            .max_decoding_message_size(self.config.mdds_max_message_size)
    }

    /// Collect all streamed `ResponseData` chunks into a single `DataTable`.
    ///
    /// MDDS returns server-streaming responses where each chunk is a zstd-
    /// compressed `DataTable`. This helper decompresses, decodes, and merges
    /// all chunks into one contiguous table.
    ///
    /// Pre-allocates the row buffer based on the `original_size` hint from the
    /// first response, reducing reallocations for large responses.
    ///
    /// For truly large responses (millions of rows), prefer [`for_each_chunk`]
    /// which processes each chunk without materializing all rows in memory.
    ///
    /// [`for_each_chunk`]: Self::for_each_chunk
    async fn collect_stream(
        &self,
        mut stream: tonic::Streaming<proto::ResponseData>,
    ) -> Result<proto::DataTable, Error> {
        let mut all_rows = Vec::new();
        let mut headers = Vec::new();

        while let Some(response) = stream.next().await {
            let response = response?;

            // Use original_size as a rough pre-allocation hint on the first chunk.
            // Each DataValueList row is ~64 bytes on average (header-dependent),
            // so original_size / 64 gives a reasonable row-count estimate.
            if all_rows.is_empty() && response.original_size > 0 {
                all_rows.reserve(usize::try_from(response.original_size).unwrap_or(0) / 64);
            }

            let table = decode::decode_data_table(&response)?;
            if headers.is_empty() {
                headers = table.headers;
            }
            all_rows.extend(table.data_table);
        }

        // An empty stream is valid (e.g. no trades on a holiday) — return an
        // empty DataTable instead of Error::NoData. Callers that need to
        // distinguish "no data" can check `table.data_table.is_empty()`.
        Ok(proto::DataTable {
            headers,
            data_table: all_rows,
        })
    }

    /// Process streamed responses chunk-by-chunk without materializing all rows.
    ///
    /// Each gRPC `ResponseData` message is decoded independently and passed to
    /// the callback as `(headers, rows)`. This keeps peak memory proportional to
    /// a single chunk rather than the entire result set — critical for endpoints
    /// that return millions of rows (e.g. full-day trade history).
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// let request = /* build your gRPC request */;
    /// let stream = client.stub().get_stock_history_trade(request).await?.into_inner();
    ///
    /// let mut count = 0usize;
    /// client.for_each_chunk(stream, |_headers, rows| {
    ///     count += rows.len();
    /// }).await?;
    /// println!("processed {count} rows without buffering them all");
    /// ```
    /// # Errors
    ///
    /// Returns an error on network, authentication, or parsing failure.
    pub async fn for_each_chunk<F>(
        &self,
        mut stream: tonic::Streaming<proto::ResponseData>,
        mut f: F,
    ) -> Result<(), Error>
    where
        F: FnMut(&[String], &[proto::DataValueList]),
    {
        // Preserve first-chunk headers across all chunks, matching collect_stream behavior.
        let mut saved_headers: Option<Vec<String>> = None;
        while let Some(response) = stream.next().await {
            let response = response?;
            let table = decode::decode_data_table(&response)?;
            if saved_headers.is_none() && !table.headers.is_empty() {
                saved_headers = Some(table.headers.clone());
            }
            let headers = if table.headers.is_empty() {
                saved_headers.as_deref().unwrap_or(&[])
            } else {
                &table.headers
            };
            f(headers, &table.data_table);
        }
        Ok(())
    }

    /// Return a reference to the underlying config for diagnostics.
    #[must_use]
    pub fn config(&self) -> &DirectConfig {
        &self.config
    }

    /// Return the session UUID string.
    #[must_use]
    pub fn session_uuid(&self) -> &str {
        &self.session.session_uuid
    }

    /// Stock subscription tier from Nexus auth response (0=Free, 1=Value, 2=Standard, 3=Pro).
    #[must_use]
    pub fn stock_tier(&self) -> Option<i32> {
        self.stock_tier
    }

    /// Options subscription tier from Nexus auth response (0=Free, 1=Value, 2=Standard, 3=Pro).
    #[must_use]
    pub fn options_tier(&self) -> Option<i32> {
        self.options_tier
    }

    // ═══════════════════════════════════════════════════════════════════
    //  Raw query — escape hatch for unwrapped endpoints
    // ═══════════════════════════════════════════════════════════════════

    /// Execute a raw gRPC query and return the merged `DataTable`.
    /// # Errors
    ///
    /// Returns an error on network, authentication, or parsing failure.
    pub async fn raw_query<F, Fut>(&self, call: F) -> Result<proto::DataTable, Error>
    where
        F: FnOnce(BetaThetaTerminalClient<tonic::transport::Channel>) -> Fut,
        Fut: std::future::Future<Output = Result<tonic::Streaming<proto::ResponseData>, Error>>,
    {
        let stream = call(self.stub()).await?;
        self.collect_stream(stream).await
    }

    /// Get a `QueryInfo` for use with [`raw_query`](Self::raw_query).
    #[must_use]
    pub fn raw_query_info(&self) -> proto::QueryInfo {
        self.query_info()
    }

    /// Get direct access to the underlying gRPC channel.
    #[must_use]
    pub fn channel(&self) -> &tonic::transport::Channel {
        &self.channel
    }
}

// Shared build-time source of truth for non-streaming list endpoints.
include!(concat!(
    env!("OUT_DIR"),
    "/direct_list_endpoints_generated.rs"
));

// ═══════════════════════════════════════════════════════════════════════
//  Builder-pattern endpoints — structs + IntoFuture at module scope
// ═══════════════════════════════════════════════════════════════════════

// Shared build-time source of truth for non-streaming builder endpoints.
include!(concat!(
    env!("OUT_DIR"),
    "/direct_parsed_endpoints_generated.rs"
));

// Shared build-time source of truth for streaming builder endpoints.
include!(concat!(
    env!("OUT_DIR"),
    "/direct_streaming_endpoints_generated.rs"
));

// ═══════════════════════════════════════════════════════════════════════
//  Private helpers
// ═══════════════════════════════════════════════════════════════════════

/// Convert an interval to the format the MDDS gRPC server accepts.
///
/// Users can pass either:
/// - Milliseconds as a string: `"60000"`, `"300000"`, `"900000"`
/// - Shorthand directly: `"1m"`, `"5m"`, `"1h"`
///
/// The server accepts these specific presets:
/// `100ms`, `500ms`, `1s`, `5s`, `10s`, `15s`, `30s`, `1m`, `5m`, `10m`, `15m`, `30m`, `1h`
///
/// If milliseconds are passed, they're converted to the nearest matching preset.
/// If already a valid shorthand (contains 's', 'm', or 'h'), passed through as-is.
fn normalize_interval(interval: &str) -> String {
    // If it already looks like shorthand (ends with s/m/h), pass through.
    if interval.ends_with('s') || interval.ends_with('m') || interval.ends_with('h') {
        return interval.to_string();
    }

    // Try parsing as milliseconds and convert to the nearest valid preset.
    //
    // Valid presets: 100ms, 500ms, 1s, 5s, 10s, 15s, 30s, 1m, 5m, 10m, 15m, 30m, 1h
    match interval.parse::<u64>() {
        Ok(ms) => match ms {
            0..=100 => "100ms".to_string(),
            101..=500 => "500ms".to_string(),
            501..=1000 => "1s".to_string(),
            1_001..=5_000 => "5s".to_string(),
            5_001..=10_000 => "10s".to_string(),
            10_001..=15_000 => "15s".to_string(),
            15_001..=30_000 => "30s".to_string(),
            30_001..=60_000 => "1m".to_string(),
            60_001..=300_000 => "5m".to_string(),
            300_001..=600_000 => "10m".to_string(),
            600_001..=900_000 => "15m".to_string(),
            900_001..=1_800_000 => "30m".to_string(),
            _ => "1h".to_string(),
        },
        // Not a number -- pass through and let the server decide.
        Err(_) => interval.to_string(),
    }
}

/// Convert `time_of_day` values into the canonical `HH:MM:SS.SSS` format.
///
/// ThetaData's v3 at-time endpoints expect a formatted ET wall-clock time such
/// as `"09:30:00.000"`. Older ThetaDataDx docs and examples used millisecond
/// strings like `"34200000"`. To preserve compatibility while aligning the
/// public contract, this helper accepts either form and normalizes to
/// `HH:MM:SS.SSS`.
///
/// Accepted inputs:
/// - Milliseconds from midnight as a decimal string: `"34200000"`
/// - Formatted times: `"09:30"`, `"09:30:00"`, `"09:30:00.000"`
///
/// Invalid or out-of-range values are passed through unchanged so the server
/// can return the canonical validation error.
fn normalize_time_of_day(time_of_day: &str) -> String {
    if time_of_day.bytes().all(|b| b.is_ascii_digit()) {
        if let Ok(total_ms) = time_of_day.parse::<u64>() {
            if total_ms < 86_400_000 {
                let hours = total_ms / 3_600_000;
                let minutes = (total_ms % 3_600_000) / 60_000;
                let seconds = (total_ms % 60_000) / 1_000;
                let millis = total_ms % 1_000;
                return format!("{hours:02}:{minutes:02}:{seconds:02}.{millis:03}");
            }
        }
        return time_of_day.to_string();
    }

    let mut parts = time_of_day.split(':');
    let Some(hours) = parts.next().and_then(|part| part.parse::<u64>().ok()) else {
        return time_of_day.to_string();
    };
    let Some(minutes) = parts.next().and_then(|part| part.parse::<u64>().ok()) else {
        return time_of_day.to_string();
    };
    let seconds_part = parts.next();
    if parts.next().is_some() {
        return time_of_day.to_string();
    }

    let (seconds, millis) = match seconds_part {
        None => (0, 0),
        Some(part) => match part.split_once('.') {
            Some((sec, frac)) => {
                let Some(seconds) = sec.parse::<u64>().ok() else {
                    return time_of_day.to_string();
                };
                let millis = match frac.len() {
                    1 => frac.parse::<u64>().ok().map(|value| value * 100),
                    2 => frac.parse::<u64>().ok().map(|value| value * 10),
                    3 => frac.parse::<u64>().ok(),
                    _ => None,
                };
                let Some(millis) = millis else {
                    return time_of_day.to_string();
                };
                (seconds, millis)
            }
            None => {
                let Some(seconds) = part.parse::<u64>().ok() else {
                    return time_of_day.to_string();
                };
                (seconds, 0)
            }
        },
    };

    if hours >= 24 || minutes >= 60 || seconds >= 60 || millis >= 1_000 {
        return time_of_day.to_string();
    }

    format!("{hours:02}:{minutes:02}:{seconds:02}.{millis:03}")
}

/// Validate a date string via the canonical [`crate::validate`] module.
///
/// This wrapper adapts the two-arg canonical signature to the single-arg
/// convention used by the builder macros (where the param name is implicit).
fn validate_date(date: &str) -> Result<(), Error> {
    crate::validate::validate_date(date, "date").map_err(Error::from)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validate_date_valid() {
        assert!(validate_date("20240101").is_ok());
        assert!(validate_date("20231231").is_ok());
        assert!(validate_date("00000000").is_ok());
    }

    #[test]
    fn validate_date_invalid() {
        // Too short
        assert!(validate_date("2024010").is_err());
        // Too long
        assert!(validate_date("202401011").is_err());
        // Contains non-digit
        assert!(validate_date("2024-101").is_err());
        assert!(validate_date("2024Jan1").is_err());
        // Empty
        assert!(validate_date("").is_err());
        // Whitespace
        assert!(validate_date("2024 101").is_err());
    }

    #[test]
    fn normalize_time_of_day_accepts_legacy_milliseconds() {
        assert_eq!(normalize_time_of_day("34200000"), "09:30:00.000");
    }

    #[test]
    fn normalize_time_of_day_accepts_short_formatted_values() {
        assert_eq!(normalize_time_of_day("09:30"), "09:30:00.000");
        assert_eq!(normalize_time_of_day("09:30:00"), "09:30:00.000");
        assert_eq!(normalize_time_of_day("09:30:00.5"), "09:30:00.500");
    }

    #[test]
    fn normalize_time_of_day_preserves_invalid_values_for_server_rejection() {
        assert_eq!(normalize_time_of_day("86400000"), "86400000");
        assert_eq!(normalize_time_of_day("09:61"), "09:61");
        assert_eq!(normalize_time_of_day("not-a-time"), "not-a-time");
    }

    #[test]
    fn parse_eod_handles_empty_table() {
        let table = proto::DataTable {
            headers: vec!["ms_of_day".into(), "open".into(), "date".into()],
            data_table: vec![],
        };
        let ticks = decode::parse_eod_ticks(&table);
        assert!(ticks.is_empty());
    }

    #[test]
    fn parse_eod_handles_number_typed_columns() {
        let table = proto::DataTable {
            headers: vec![
                "ms_of_day".into(),
                "open".into(),
                "close".into(),
                "date".into(),
            ],
            data_table: vec![proto::DataValueList {
                values: vec![
                    proto::DataValue {
                        data_type: Some(proto::data_value::DataType::Number(34200000)),
                    },
                    proto::DataValue {
                        data_type: Some(proto::data_value::DataType::Number(15000)),
                    },
                    proto::DataValue {
                        data_type: Some(proto::data_value::DataType::Number(15100)),
                    },
                    proto::DataValue {
                        data_type: Some(proto::data_value::DataType::Number(20240301)),
                    },
                ],
            }],
        };
        let ticks = decode::parse_eod_ticks(&table);
        assert_eq!(ticks.len(), 1);
        assert_eq!(ticks[0].ms_of_day, 34200000);
        assert!((ticks[0].open - 15000.0).abs() < 1e-10);
        assert!((ticks[0].close - 15100.0).abs() < 1e-10);
        assert_eq!(ticks[0].date, 20240301);
    }
}
