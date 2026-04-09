//! Direct server client — MDDS gRPC without the Java terminal.
//!
//! `DirectClient` authenticates against the Nexus API, opens a gRPC channel
//! to the MDDS server, and exposes typed methods for every data endpoint.
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

// ═══════════════════════════════════════════════════════════════════════
//  Endpoint macros — builder pattern with IntoFuture for all gRPC RPCs
// ═══════════════════════════════════════════════════════════════════════

/// Generate a list endpoint that returns `Vec<String>` by extracting a text
/// column from the response `DataTable`.
///
/// Pattern: build request -> gRPC call -> collect stream -> extract text column.
macro_rules! list_endpoint {
    (
        $(#[$meta:meta])*
        fn $name:ident( $($arg:ident : $arg_ty:ty),* ) -> $col:literal;
        grpc: $grpc:ident;
        request: $req:ident;
        query: $query:ident { $($field:ident : $val:expr),* $(,)? };
    ) => {
        #[allow(clippy::too_many_arguments)]
        $(#[$meta])*
        /// # Errors
        ///
        /// Returns an error on network, authentication, or parsing failure.
        pub async fn $name(&self, $($arg : $arg_ty),*) -> Result<Vec<String>, Error> {
            tracing::debug!(endpoint = stringify!($name), "gRPC request");
            metrics::counter!("thetadatadx.grpc.requests", "endpoint" => stringify!($name)).increment(1);
            let _metrics_start = std::time::Instant::now();
            let _permit = self.request_semaphore.acquire().await
                .map_err(|_| Error::Config("request semaphore closed".into()))?;
            let request = proto::$req {
                query_info: Some(self.query_info()),
                params: Some(proto::$query { $($field : $val),* }),
            };
            let stream = match self.stub().$grpc(request).await {
                Ok(resp) => resp.into_inner(),
                Err(e) => {
                    metrics::counter!("thetadatadx.grpc.errors", "endpoint" => stringify!($name)).increment(1);
                    return Err(e.into());
                }
            };
            let table = match self.collect_stream(stream).await {
                Ok(t) => t,
                Err(e) => {
                    metrics::counter!("thetadatadx.grpc.errors", "endpoint" => stringify!($name)).increment(1);
                    return Err(e);
                }
            };
            metrics::histogram!("thetadatadx.grpc.latency_ms", "endpoint" => stringify!($name))
                .record(_metrics_start.elapsed().as_secs_f64() * 1_000.0);
            Ok(decode::extract_text_column(&table, $col)
                .into_iter()
                .flatten()
                .collect())
        }
    };
}

/// Generate an endpoint that returns parsed tick data (`Vec<T>`) via a builder.
///
/// The endpoint method returns a builder struct that captures required params.
/// Optional params are set via chainable setter methods. `.await` (via `IntoFuture`)
/// executes the gRPC call.
///
/// # Example
///
/// ```rust,ignore
/// // Simple -- just .await the builder directly
/// let ticks = client.stock_history_ohlc("AAPL", "20260401", "1m").await?;
///
/// // With options -- chain setters before .await
/// let ticks = client.stock_history_ohlc("AAPL", "20260401", "1m")
///     .venue("arca")
///     .start_time("04:00:00")
///     .await?;
/// ```
macro_rules! parsed_endpoint {
    (
        $(#[$meta:meta])*
        builder $builder_name:ident;
        fn $name:ident(
            $($req_arg:ident : $req_kind:tt),*
        ) -> $ret:ty;
        grpc: $grpc:ident;
        request: $req:ident;
        query: $query:ident { $($field:ident : $val:expr),* $(,)? };
        parse: $parser:expr;
        $(dates: $($date_arg:ident),+ ;)?
        optional { $($opt_name:ident : $opt_kind:tt = $opt_default:expr),* $(,)? }
    ) => {
        /// Builder for the [`DirectClient::$name`] endpoint.
        pub struct $builder_name<'a> {
            client: &'a DirectClient,
            $(pub(crate) $req_arg: req_field_type!($req_kind),)*
            $(pub(crate) $opt_name: opt_field_type!($opt_kind),)*
        }

        impl<'a> $builder_name<'a> {
            $(
                opt_setter!($opt_name, $opt_kind);
            )*
        }

        impl<'a> IntoFuture for $builder_name<'a> {
            type Output = Result<$ret, Error>;
            type IntoFuture = Pin<Box<dyn std::future::Future<Output = Self::Output> + Send + 'a>>;

            fn into_future(self) -> Self::IntoFuture {
                Box::pin(async move {
                    let $builder_name {
                        client,
                        $($req_arg,)*
                        $($opt_name,)*
                    } = self;
                    let _ = &client;
                    $($(validate_date(&$date_arg)?;)+)?
                    tracing::debug!(endpoint = stringify!($name), "gRPC request");
                    metrics::counter!("thetadatadx.grpc.requests", "endpoint" => stringify!($name)).increment(1);
                    let _metrics_start = std::time::Instant::now();
                    let _permit = client.request_semaphore.acquire().await
                        .map_err(|_| Error::Config("request semaphore closed".into()))?;
                    let request = proto::$req {
                        query_info: Some(client.query_info()),
                        params: Some(proto::$query { $($field : $val),* }),
                    };
                    let stream = match client.stub().$grpc(request).await {
                        Ok(resp) => resp.into_inner(),
                        Err(e) => {
                            metrics::counter!("thetadatadx.grpc.errors", "endpoint" => stringify!($name)).increment(1);
                            return Err(e.into());
                        }
                    };
                    let table = match client.collect_stream(stream).await {
                        Ok(t) => t,
                        Err(e) => {
                            metrics::counter!("thetadatadx.grpc.errors", "endpoint" => stringify!($name)).increment(1);
                            return Err(e);
                        }
                    };
                    metrics::histogram!("thetadatadx.grpc.latency_ms", "endpoint" => stringify!($name))
                        .record(_metrics_start.elapsed().as_secs_f64() * 1_000.0);
                    Ok($parser(&table))
                })
            }
        }

        impl DirectClient {
            $(#[$meta])*
            pub fn $name(&self, $($req_arg: req_param_type!($req_kind)),*) -> $builder_name<'_> {
                $builder_name {
                    client: self,
                    $($req_arg: req_convert!($req_kind, $req_arg),)*
                    $($opt_name: $opt_default,)*
                }
            }
        }
    };
}

/// Map a required-param tag to the struct field type.
macro_rules! req_field_type {
    (str)      => { String };
    (str_vec)  => { Vec<String> };
}

/// Map a required-param tag to the constructor parameter type.
macro_rules! req_param_type {
    (str) => {
        &str
    };
    (str_vec) => {
        &[&str]
    };
}

/// Convert a required param from the user-facing type to the stored type.
macro_rules! req_convert {
    (str, $v:ident) => {
        $v.to_string()
    };
    (str_vec, $v:ident) => {
        $v.iter().map(|s| s.to_string()).collect()
    };
}

/// Map a tag token to the actual Rust type for struct fields.
macro_rules! opt_field_type {
    (opt_str)  => { Option<String> };
    (opt_i32)  => { Option<i32> };
    (opt_f64)  => { Option<f64> };
    (opt_bool) => { Option<bool> };
    (string)   => { String };
}

/// Generate a chainable setter method based on the tag token.
macro_rules! opt_setter {
    ($opt_name:ident, opt_str) => {
        #[must_use]
        pub fn $opt_name(mut self, v: &str) -> Self {
            self.$opt_name = Some(v.to_string());
            self
        }
    };
    ($opt_name:ident, opt_i32) => {
        #[must_use]
        pub fn $opt_name(mut self, v: i32) -> Self {
            self.$opt_name = Some(v);
            self
        }
    };
    ($opt_name:ident, opt_f64) => {
        #[must_use]
        pub fn $opt_name(mut self, v: f64) -> Self {
            self.$opt_name = Some(v);
            self
        }
    };
    ($opt_name:ident, opt_bool) => {
        #[must_use]
        pub fn $opt_name(mut self, v: bool) -> Self {
            self.$opt_name = Some(v);
            self
        }
    };
    ($opt_name:ident, string) => {
        #[must_use]
        pub fn $opt_name(mut self, v: &str) -> Self {
            self.$opt_name = v.to_string();
            self
        }
    };
}

/// Generate a streaming endpoint that yields parsed ticks per-chunk via a callback.
///
/// Returns a builder. Call `.stream(handler)` to execute the streaming request.
///
/// # Example
///
/// ```rust,ignore
/// client.stock_history_trade_stream("AAPL", "20260401")
///     .start_time("04:00:00")
///     .stream(|ticks| {
///         println!("got {} ticks", ticks.len());
///     })
///     .await?;
/// ```
macro_rules! streaming_endpoint {
    (
        $(#[$meta:meta])*
        builder $builder_name:ident;
        fn $name:ident(
            $($req_arg:ident : $req_kind:tt),*
        ) -> $tick_ty:ty;
        grpc: $grpc:ident;
        request: $req:ident;
        query: $query:ident { $($field:ident : $val:expr),* $(,)? };
        parse: $parser:expr;
        $(dates: $($date_arg:ident),+ ;)?
        optional { $($opt_name:ident : $opt_kind:tt = $opt_default:expr),* $(,)? }
    ) => {
        /// Builder for the [`DirectClient::$name`] streaming endpoint.
        pub struct $builder_name<'a> {
            client: &'a DirectClient,
            $(pub(crate) $req_arg: req_field_type!($req_kind),)*
            $(pub(crate) $opt_name: opt_field_type!($opt_kind),)*
        }

        impl<'a> $builder_name<'a> {
            $(
                opt_setter!($opt_name, $opt_kind);
            )*

            /// Execute the streaming request, calling `handler` for each chunk.
            ///
            /// # Errors
            ///
            /// Returns [`Error`] if the gRPC call fails or response parsing fails.
            pub async fn stream<F>(self, mut handler: F) -> Result<(), Error>
            where
                F: FnMut(&[$tick_ty]),
            {
                let $builder_name {
                    client,
                    $($req_arg,)*
                    $($opt_name,)*
                } = self;
                let _ = &client;
                $($(validate_date(&$date_arg)?;)+)?
                tracing::debug!(endpoint = stringify!($name), "gRPC streaming request");
                metrics::counter!("thetadatadx.grpc.requests", "endpoint" => stringify!($name)).increment(1);
                let _metrics_start = std::time::Instant::now();
                let _permit = client.request_semaphore.acquire().await
                    .map_err(|_| Error::Config("request semaphore closed".into()))?;
                let request = proto::$req {
                    query_info: Some(client.query_info()),
                    params: Some(proto::$query { $($field : $val),* }),
                };
                let stream = match client.stub().$grpc(request).await {
                    Ok(resp) => resp.into_inner(),
                    Err(e) => {
                        metrics::counter!("thetadatadx.grpc.errors", "endpoint" => stringify!($name)).increment(1);
                        return Err(e.into());
                    }
                };
                let result = client.for_each_chunk(stream, |_headers, rows| {
                    let table = proto::DataTable {
                        headers: _headers.to_vec(),
                        data_table: rows.to_vec(),
                    };
                    let ticks = $parser(&table);
                    handler(&ticks);
                }).await;
                match &result {
                    Ok(()) => {
                        metrics::histogram!("thetadatadx.grpc.latency_ms", "endpoint" => stringify!($name))
                            .record(_metrics_start.elapsed().as_secs_f64() * 1_000.0);
                    }
                    Err(_) => {
                        metrics::counter!("thetadatadx.grpc.errors", "endpoint" => stringify!($name)).increment(1);
                    }
                }
                result
            }
        }

        impl DirectClient {
            $(#[$meta])*
            pub fn $name(&self, $($req_arg: req_param_type!($req_kind)),*) -> $builder_name<'_> {
                $builder_name {
                    client: self,
                    $($req_arg: req_convert!($req_kind, $req_arg),)*
                    $($opt_name: $opt_default,)*
                }
            }
        }
    };
}

/// Normalize the `right` parameter for the v3 MDDS server.
///
/// Accepts: `"C"`, `"P"`, `"call"`, `"put"`, `"both"`, `"*"`.
/// Maps `"C"` -> `"call"`, `"P"` -> `"put"`, `"*"` -> `"both"`.
fn normalize_right(right: &str) -> String {
    match right {
        "C" | "c" => "call".to_string(),
        "P" | "p" => "put".to_string(),
        "*" => "both".to_string(),
        other => other.to_lowercase(),
    }
}

/// Helper: build a `proto::ContractSpec` from the four standard option params.
macro_rules! contract_spec {
    ($symbol:expr, $expiration:expr, $strike:expr, $right:expr) => {
        Some(proto::ContractSpec {
            symbol: $symbol.to_string(),
            expiration: $expiration.to_string(),
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

// Streaming convenience methods remain handwritten because they expose
// chunk-by-chunk iteration semantics rather than the registry's
// collect-then-return model.

streaming_endpoint! {
    /// Stream all trades for a stock on a given date, chunk-by-chunk.
    builder StockHistoryTradeStreamBuilder;
    fn stock_history_trade_stream(symbol: str, date: str) -> TradeTick;
    grpc: get_stock_history_trade;
    request: StockHistoryTradeRequest;
    query: StockHistoryTradeRequestQuery {
        symbol: symbol.clone(),
        date: Some(date.clone()),
        start_time: Some(start_time.clone()),
        end_time: Some(end_time.clone()),
        venue: venue.clone().or_else(|| Some("nqb".to_string())),
        start_date: start_date.clone(),
        end_date: end_date.clone(),
    };
    parse: decode::parse_trade_ticks;
    dates: date;
    optional {
        start_time: string = "09:30:00".to_string(),
        end_time: string = "16:00:00".to_string(),
        venue: opt_str = None,
        start_date: opt_str = None,
        end_date: opt_str = None,
    }
}

streaming_endpoint! {
    /// Stream NBBO quotes for a stock on a given date, chunk-by-chunk.
    builder StockHistoryQuoteStreamBuilder;
    fn stock_history_quote_stream(symbol: str, date: str, interval: str) -> QuoteTick;
    grpc: get_stock_history_quote;
    request: StockHistoryQuoteRequest;
    query: StockHistoryQuoteRequestQuery {
        symbol: symbol.clone(),
        date: Some(date.clone()),
        interval: normalize_interval(&interval),
        start_time: Some(start_time.clone()),
        end_time: Some(end_time.clone()),
        venue: venue.clone().or_else(|| Some("nqb".to_string())),
        start_date: start_date.clone(),
        end_date: end_date.clone(),
    };
    parse: decode::parse_quote_ticks;
    dates: date;
    optional {
        start_time: string = "09:30:00".to_string(),
        end_time: string = "16:00:00".to_string(),
        venue: opt_str = None,
        start_date: opt_str = None,
        end_date: opt_str = None,
    }
}

streaming_endpoint! {
    /// Stream all trades for an option contract, chunk-by-chunk.
    builder OptionHistoryTradeStreamBuilder;
    fn option_history_trade_stream(symbol: str, expiration: str, strike: str, right: str, date: str) -> TradeTick;
    grpc: get_option_history_trade;
    request: OptionHistoryTradeRequest;
    query: OptionHistoryTradeRequestQuery {
        contract_spec: contract_spec!(symbol, expiration, strike, right),
        date: Some(date.clone()),
        expiration: expiration.clone(),
        start_time: Some(start_time.clone()),
        end_time: Some(end_time.clone()),
        max_dte: max_dte,
        strike_range: strike_range,
        start_date: start_date.clone(),
        end_date: end_date.clone(),
    };
    parse: decode::parse_trade_ticks;
    dates: date;
    optional {
        start_time: string = "09:30:00".to_string(),
        end_time: string = "16:00:00".to_string(),
        max_dte: opt_i32 = None,
        strike_range: opt_i32 = None,
        start_date: opt_str = None,
        end_date: opt_str = None,
    }
}

streaming_endpoint! {
    /// Stream NBBO quotes for an option contract, chunk-by-chunk.
    builder OptionHistoryQuoteStreamBuilder;
    fn option_history_quote_stream(symbol: str, expiration: str, strike: str, right: str, date: str, interval: str) -> QuoteTick;
    grpc: get_option_history_quote;
    request: OptionHistoryQuoteRequest;
    query: OptionHistoryQuoteRequestQuery {
        contract_spec: contract_spec!(symbol, expiration, strike, right),
        date: Some(date.clone()),
        expiration: expiration.clone(),
        start_time: Some(start_time.clone()),
        end_time: Some(end_time.clone()),
        interval: normalize_interval(&interval),
        max_dte: max_dte,
        strike_range: strike_range,
        start_date: start_date.clone(),
        end_date: end_date.clone(),
    };
    parse: decode::parse_quote_ticks;
    dates: date;
    optional {
        start_time: string = "09:30:00".to_string(),
        end_time: string = "16:00:00".to_string(),
        max_dte: opt_i32 = None,
        strike_range: opt_i32 = None,
        start_date: opt_str = None,
        end_date: opt_str = None,
    }
}

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

/// Validate that a date string is in YYYYMMDD format (exactly 8 ASCII digits).
// Reason: internal validation helper with known-safe unwrap.
#[allow(clippy::missing_panics_doc, clippy::needless_pass_by_value)]
fn validate_date(date: &str) -> Result<(), Error> {
    if date.len() != 8 || !date.bytes().all(|b| b.is_ascii_digit()) {
        return Err(Error::Config(format!(
            "invalid date '{date}': expected YYYYMMDD format (8 digits)"
        )));
    }
    Ok(())
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
