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
use crate::proto_v3;
use crate::proto_v3::beta_theta_terminal_client::BetaThetaTerminalClient;
use tdbe::types::tick::*;

/// Crate version embedded in `QueryInfo.terminal_version` so ThetaData can
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
        pub async fn $name(&self, $($arg : $arg_ty),*) -> Result<Vec<String>, Error> {
            tracing::debug!(endpoint = stringify!($name), "gRPC request");
            metrics::counter!("thetadatadx.grpc.requests", "endpoint" => stringify!($name)).increment(1);
            let _metrics_start = std::time::Instant::now();
            let _permit = self.request_semaphore.acquire().await
                .map_err(|_| Error::Fpss("request semaphore closed".into()))?;
            let request = proto_v3::$req {
                query_info: Some(self.query_info()),
                params: Some(proto_v3::$query { $($field : $val),* }),
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
                .record(_metrics_start.elapsed().as_millis() as f64);
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
                        .map_err(|_| Error::Fpss("request semaphore closed".into()))?;
                    let request = proto_v3::$req {
                        query_info: Some(client.query_info()),
                        params: Some(proto_v3::$query { $($field : $val),* }),
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
                        .record(_metrics_start.elapsed().as_millis() as f64);
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
        pub fn $opt_name(mut self, v: &str) -> Self {
            self.$opt_name = Some(v.to_string());
            self
        }
    };
    ($opt_name:ident, opt_i32) => {
        pub fn $opt_name(mut self, v: i32) -> Self {
            self.$opt_name = Some(v);
            self
        }
    };
    ($opt_name:ident, opt_f64) => {
        pub fn $opt_name(mut self, v: f64) -> Self {
            self.$opt_name = Some(v);
            self
        }
    };
    ($opt_name:ident, opt_bool) => {
        pub fn $opt_name(mut self, v: bool) -> Self {
            self.$opt_name = Some(v);
            self
        }
    };
    ($opt_name:ident, string) => {
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
                    .map_err(|_| Error::Fpss("request semaphore closed".into()))?;
                let request = proto_v3::$req {
                    query_info: Some(client.query_info()),
                    params: Some(proto_v3::$query { $($field : $val),* }),
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
                            .record(_metrics_start.elapsed().as_millis() as f64);
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

/// Helper: build a `proto::ContractSpec` from the four standard option params.
macro_rules! contract_spec {
    ($symbol:expr, $expiration:expr, $strike:expr, $right:expr) => {
        Some(proto::ContractSpec {
            symbol: $symbol.to_string(),
            expiration: $expiration.to_string(),
            strike: Some($strike.to_string()),
            right: Some($right.to_string()),
        })
    };
}

/// Direct client for ThetaData server access.
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
    /// Pre-built QueryInfo template — cloned per-request instead of allocating
    /// new Strings each time.
    query_info_template: proto_v3::QueryInfo,
    /// Semaphore limiting concurrent in-flight gRPC requests.
    ///
    /// The Java terminal limits concurrent requests to `2^subscription_tier`
    /// (Free=1, Value=2, Standard=4, Pro=16). This semaphore enforces the same
    /// bound to prevent server-side rate limiting / 429 disconnects.
    request_semaphore: Arc<tokio::sync::Semaphore>,
}

impl DirectClient {
    /// Connect to ThetaData servers directly (no JVM terminal needed).
    ///
    /// 1. Authenticates against the Nexus HTTP API to obtain a session UUID.
    /// 2. Opens a gRPC channel (TLS) to the MDDS server.
    ///
    /// The FPSS (real-time streaming) connection is not established here;
    /// it will be added in a future release.
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
            .initial_stream_window_size((config.mdds_window_size_kb * 1024) as u32)
            .initial_connection_window_size((config.mdds_connection_window_size_kb * 1024) as u32)
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

        let query_info_template = proto_v3::QueryInfo {
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
                .map(|u| u.max_concurrent_requests())
                .unwrap_or(2)
        } else {
            config.mdds_concurrent_requests
        };

        let request_semaphore = Arc::new(tokio::sync::Semaphore::new(concurrent));

        tracing::debug!(
            mdds_concurrent_requests = concurrent,
            auto_detected = config.mdds_concurrent_requests == 0,
            "request semaphore initialized"
        );

        Ok(Self {
            session,
            channel,
            config,
            query_info_template,
            request_semaphore,
        })
    }

    /// Return a clone of the pre-built `QueryInfo` template.
    ///
    /// The template is constructed once at connection time, avoiding per-call
    /// String allocations for session UUID, client type, and version.
    #[inline]
    fn query_info(&self) -> proto_v3::QueryInfo {
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
                all_rows.reserve(response.original_size as usize / 64);
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
    pub fn config(&self) -> &DirectConfig {
        &self.config
    }

    /// Return the session UUID string.
    pub fn session_uuid(&self) -> &str {
        &self.session.session_uuid
    }

    // ═══════════════════════════════════════════════════════════════════
    //  Stock — List endpoints (2)
    // ═══════════════════════════════════════════════════════════════════

    // 1. GetStockListSymbols
    list_endpoint! {
        /// List all available stock symbols.
        ///
        /// gRPC: `BetaThetaTerminal/GetStockListSymbols`
        fn stock_list_symbols() -> "symbol";
        grpc: get_stock_list_symbols;
        request: StockListSymbolsRequest;
        query: StockListSymbolsRequestQuery {};
    }

    // 2. GetStockListDates
    list_endpoint! {
        /// List available dates for a stock by request type.
        ///
        /// `request_type` is e.g. `"EOD"`, `"TRADE"`, `"QUOTE"`, etc.
        ///
        /// gRPC: `BetaThetaTerminal/GetStockListDates`
        fn stock_list_dates(request_type: &str, symbol: &str) -> "date";
        grpc: get_stock_list_dates;
        request: StockListDatesRequest;
        query: StockListDatesRequestQuery {
            request_type: request_type.to_string(),
            symbol: vec![symbol.to_string()],
        };
    }

    // ═══════════════════════════════════════════════════════════════════
    //  Option — List endpoints (5)
    // ═══════════════════════════════════════════════════════════════════

    // 14. GetOptionListSymbols
    list_endpoint! {
        /// List all available option underlying symbols.
        ///
        /// gRPC: `BetaThetaTerminal/GetOptionListSymbols`
        fn option_list_symbols() -> "symbol";
        grpc: get_option_list_symbols;
        request: OptionListSymbolsRequest;
        query: OptionListSymbolsRequestQuery {};
    }

    // 15. GetOptionListDates
    list_endpoint! {
        /// List available dates for an option contract by request type.
        ///
        /// gRPC: `BetaThetaTerminal/GetOptionListDates`
        fn option_list_dates(
            request_type: &str, symbol: &str, expiration: &str, strike: &str, right: &str
        ) -> "date";
        grpc: get_option_list_dates;
        request: OptionListDatesRequest;
        query: OptionListDatesRequestQuery {
            request_type: request_type.to_string(),
            contract_spec: contract_spec!(symbol, expiration, strike, right),
            expiration: expiration.to_string(),
        };
    }

    // 16. GetOptionListExpirations
    list_endpoint! {
        /// List available expiration dates for an option underlying.
        ///
        /// gRPC: `BetaThetaTerminal/GetOptionListExpirations`
        fn option_list_expirations(symbol: &str) -> "expiration";
        grpc: get_option_list_expirations;
        request: OptionListExpirationsRequest;
        query: OptionListExpirationsRequestQuery {
            symbol: vec![symbol.to_string()],
        };
    }

    // 17. GetOptionListStrikes
    list_endpoint! {
        /// List available strike prices for an option underlying at a given expiration.
        ///
        /// gRPC: `BetaThetaTerminal/GetOptionListStrikes`
        ///
        /// `expiration` is `YYYYMMDD`.
        fn option_list_strikes(symbol: &str, expiration: &str) -> "strike";
        grpc: get_option_list_strikes;
        request: OptionListStrikesRequest;
        query: OptionListStrikesRequestQuery {
            symbol: vec![symbol.to_string()],
            expiration: expiration.to_string(),
        };
    }

    // ═══════════════════════════════════════════════════════════════════
    //  Index — List endpoints (2)
    // ═══════════════════════════════════════════════════════════════════

    // 48. GetIndexListSymbols
    list_endpoint! {
        /// List all available index symbols.
        ///
        /// gRPC: `BetaThetaTerminal/GetIndexListSymbols`
        fn index_list_symbols() -> "symbol";
        grpc: get_index_list_symbols;
        request: IndexListSymbolsRequest;
        query: IndexListSymbolsRequestQuery {};
    }

    // 49. GetIndexListDates
    list_endpoint! {
        /// List available dates for an index symbol.
        ///
        /// gRPC: `BetaThetaTerminal/GetIndexListDates`
        fn index_list_dates(symbol: &str) -> "date";
        grpc: get_index_list_dates;
        request: IndexListDatesRequest;
        query: IndexListDatesRequestQuery {
            symbol: vec![symbol.to_string()],
        };
    }

    // ═══════════════════════════════════════════════════════════════════
    //  Raw query — escape hatch for unwrapped endpoints
    // ═══════════════════════════════════════════════════════════════════

    /// Execute a raw gRPC query and return the merged `DataTable`.
    pub async fn raw_query<F, Fut>(&self, call: F) -> Result<proto::DataTable, Error>
    where
        F: FnOnce(BetaThetaTerminalClient<tonic::transport::Channel>) -> Fut,
        Fut: std::future::Future<Output = Result<tonic::Streaming<proto::ResponseData>, Error>>,
    {
        let stream = call(self.stub()).await?;
        self.collect_stream(stream).await
    }

    /// Get a `QueryInfo` for use with [`raw_query`](Self::raw_query).
    pub fn raw_query_info(&self) -> proto_v3::QueryInfo {
        self.query_info()
    }

    /// Get direct access to the underlying gRPC channel.
    pub fn channel(&self) -> &tonic::transport::Channel {
        &self.channel
    }
}

// ═══════════════════════════════════════════════════════════════════════
//  Builder-pattern endpoints — structs + IntoFuture at module scope
// ═══════════════════════════════════════════════════════════════════════

// ── Stock Snapshot (4) ─────────────────────────────────────────────

// 3. GetStockSnapshotOhlc
parsed_endpoint! {
    /// Get the latest OHLC snapshot for one or more stocks.
    builder StockSnapshotOhlcBuilder;
    fn stock_snapshot_ohlc(symbols: str_vec) -> Vec<OhlcTick>;
    grpc: get_stock_snapshot_ohlc;
    request: StockSnapshotOhlcRequest;
    query: StockSnapshotOhlcRequestQuery {
        symbol: symbols.iter().map(|s| s.to_string()).collect(),
        venue: venue.clone().or_else(|| Some("nqb".to_string())),
        min_time: min_time.clone(),
    };
    parse: decode::parse_ohlc_ticks;
    optional { venue: opt_str = None, min_time: opt_str = None }
}

// 4. GetStockSnapshotTrade
parsed_endpoint! {
    /// Get the latest trade snapshot for one or more stocks.
    builder StockSnapshotTradeBuilder;
    fn stock_snapshot_trade(symbols: str_vec) -> Vec<TradeTick>;
    grpc: get_stock_snapshot_trade;
    request: StockSnapshotTradeRequest;
    query: StockSnapshotTradeRequestQuery {
        symbol: symbols.iter().map(|s| s.to_string()).collect(),
        venue: venue.clone().or_else(|| Some("nqb".to_string())),
        min_time: min_time.clone(),
    };
    parse: decode::parse_trade_ticks;
    optional { venue: opt_str = None, min_time: opt_str = None }
}

// 5. GetStockSnapshotQuote
parsed_endpoint! {
    /// Get the latest NBBO quote snapshot for one or more stocks.
    builder StockSnapshotQuoteBuilder;
    fn stock_snapshot_quote(symbols: str_vec) -> Vec<QuoteTick>;
    grpc: get_stock_snapshot_quote;
    request: StockSnapshotQuoteRequest;
    query: StockSnapshotQuoteRequestQuery {
        symbol: symbols.iter().map(|s| s.to_string()).collect(),
        venue: venue.clone().or_else(|| Some("nqb".to_string())),
        min_time: min_time.clone(),
    };
    parse: decode::parse_quote_ticks;
    optional { venue: opt_str = None, min_time: opt_str = None }
}

// 6. GetStockSnapshotMarketValue
parsed_endpoint! {
    /// Get the latest market value snapshot for one or more stocks.
    builder StockSnapshotMarketValueBuilder;
    fn stock_snapshot_market_value(symbols: str_vec) -> Vec<MarketValueTick>;
    grpc: get_stock_snapshot_market_value;
    request: StockSnapshotMarketValueRequest;
    query: StockSnapshotMarketValueRequestQuery {
        symbol: symbols.iter().map(|s| s.to_string()).collect(),
        venue: venue.clone().or_else(|| Some("nqb".to_string())),
        min_time: min_time.clone(),
    };
    parse: decode::parse_market_value_ticks;
    optional { venue: opt_str = None, min_time: opt_str = None }
}

// ── Stock History (5) ──────────────────────────────────────────────

// 7. GetStockHistoryEod
parsed_endpoint! {
    /// Fetch end-of-day stock data for a date range.
    builder StockHistoryEodBuilder;
    fn stock_history_eod(symbol: str, start: str, end: str) -> Vec<EodTick>;
    grpc: get_stock_history_eod;
    request: StockHistoryEodRequest;
    query: StockHistoryEodRequestQuery {
        symbol: symbol.to_string(),
        start_date: start.to_string(),
        end_date: end.to_string(),
    };
    parse: decode::parse_eod_ticks;
    dates: start, end;
    optional {}
}

// 8. GetStockHistoryOhlc
parsed_endpoint! {
    /// Fetch intraday OHLC bars for a stock on a single date.
    ///
    /// `interval` accepts milliseconds (`"60000"`) or shorthand (`"1m"`).
    builder StockHistoryOhlcBuilder;
    fn stock_history_ohlc(symbol: str, date: str, interval: str) -> Vec<OhlcTick>;
    grpc: get_stock_history_ohlc;
    request: StockHistoryOhlcRequest;
    query: StockHistoryOhlcRequestQuery {
        symbol: symbol.to_string(),
        date: Some(date.to_string()),
        interval: normalize_interval(&interval),
        start_time: Some(start_time.clone()),
        end_time: Some(end_time.clone()),
        venue: venue.clone().or_else(|| Some("nqb".to_string())),
        start_date: start_date.clone(),
        end_date: end_date.clone(),
    };
    parse: decode::parse_ohlc_ticks;
    dates: date;
    optional {
        start_time: string = "09:30:00".to_string(),
        end_time: string = "16:00:00".to_string(),
        venue: opt_str = None,
        start_date: opt_str = None,
        end_date: opt_str = None,
    }
}

// (bonus variant) GetStockHistoryOhlc with date range
parsed_endpoint! {
    /// Fetch intraday OHLC bars across a date range.
    builder StockHistoryOhlcRangeBuilder;
    fn stock_history_ohlc_range(symbol: str, start_date: str, end_date: str, interval: str) -> Vec<OhlcTick>;
    grpc: get_stock_history_ohlc;
    request: StockHistoryOhlcRequest;
    query: StockHistoryOhlcRequestQuery {
        symbol: symbol.to_string(),
        date: None,
        interval: normalize_interval(&interval),
        start_time: Some(start_time.clone()),
        end_time: Some(end_time.clone()),
        venue: venue.clone().or_else(|| Some("nqb".to_string())),
        start_date: Some(start_date.to_string()),
        end_date: Some(end_date.to_string()),
    };
    parse: decode::parse_ohlc_ticks;
    dates: start_date, end_date;
    optional {
        start_time: string = "09:30:00".to_string(),
        end_time: string = "16:00:00".to_string(),
        venue: opt_str = None,
    }
}

// 9. GetStockHistoryTrade
parsed_endpoint! {
    /// Fetch all trades for a stock on a given date.
    builder StockHistoryTradeBuilder;
    fn stock_history_trade(symbol: str, date: str) -> Vec<TradeTick>;
    grpc: get_stock_history_trade;
    request: StockHistoryTradeRequest;
    query: StockHistoryTradeRequestQuery {
        symbol: symbol.to_string(),
        date: Some(date.to_string()),
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

// 10. GetStockHistoryQuote
parsed_endpoint! {
    /// Fetch NBBO quotes for a stock on a given date at a given interval.
    builder StockHistoryQuoteBuilder;
    fn stock_history_quote(symbol: str, date: str, interval: str) -> Vec<QuoteTick>;
    grpc: get_stock_history_quote;
    request: StockHistoryQuoteRequest;
    query: StockHistoryQuoteRequestQuery {
        symbol: symbol.to_string(),
        date: Some(date.to_string()),
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

// ── Stock Streaming History ────────────────────────────────────────

// 9s. GetStockHistoryTrade (streaming)
streaming_endpoint! {
    /// Stream all trades for a stock on a given date, chunk-by-chunk.
    builder StockHistoryTradeStreamBuilder;
    fn stock_history_trade_stream(symbol: str, date: str) -> TradeTick;
    grpc: get_stock_history_trade;
    request: StockHistoryTradeRequest;
    query: StockHistoryTradeRequestQuery {
        symbol: symbol.to_string(),
        date: Some(date.to_string()),
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

// 10s. GetStockHistoryQuote (streaming)
streaming_endpoint! {
    /// Stream NBBO quotes for a stock on a given date, chunk-by-chunk.
    builder StockHistoryQuoteStreamBuilder;
    fn stock_history_quote_stream(symbol: str, date: str, interval: str) -> QuoteTick;
    grpc: get_stock_history_quote;
    request: StockHistoryQuoteRequest;
    query: StockHistoryQuoteRequestQuery {
        symbol: symbol.to_string(),
        date: Some(date.to_string()),
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

// 11. GetStockHistoryTradeQuote
parsed_endpoint! {
    /// Fetch combined trade + quote ticks for a stock on a given date.
    builder StockHistoryTradeQuoteBuilder;
    fn stock_history_trade_quote(symbol: str, date: str) -> Vec<TradeQuoteTick>;
    grpc: get_stock_history_trade_quote;
    request: StockHistoryTradeQuoteRequest;
    query: StockHistoryTradeQuoteRequestQuery {
        symbol: symbol.to_string(),
        date: Some(date.to_string()),
        start_time: Some(start_time.clone()),
        end_time: Some(end_time.clone()),
        exclusive: exclusive,
        venue: venue.clone().or_else(|| Some("nqb".to_string())),
        start_date: start_date.clone(),
        end_date: end_date.clone(),
    };
    parse: decode::parse_trade_quote_ticks;
    dates: date;
    optional {
        start_time: string = "09:30:00".to_string(),
        end_time: string = "16:00:00".to_string(),
        exclusive: opt_bool = None,
        venue: opt_str = None,
        start_date: opt_str = None,
        end_date: opt_str = None,
    }
}

// ── Stock At-Time (2) ──────────────────────────────────────────────

// 12. GetStockAtTimeTrade
parsed_endpoint! {
    /// Fetch the trade at a specific time of day across a date range.
    builder StockAtTimeTradeBuilder;
    fn stock_at_time_trade(symbol: str, start_date: str, end_date: str, time_of_day: str) -> Vec<TradeTick>;
    grpc: get_stock_at_time_trade;
    request: StockAtTimeTradeRequest;
    query: StockAtTimeTradeRequestQuery {
        symbol: symbol.to_string(),
        start_date: start_date.to_string(),
        end_date: end_date.to_string(),
        time_of_day: time_of_day.to_string(),
        venue: venue.clone().or_else(|| Some("nqb".to_string())),
    };
    parse: decode::parse_trade_ticks;
    dates: start_date, end_date;
    optional { venue: opt_str = None }
}

// 13. GetStockAtTimeQuote
parsed_endpoint! {
    /// Fetch the quote at a specific time of day across a date range.
    builder StockAtTimeQuoteBuilder;
    fn stock_at_time_quote(symbol: str, start_date: str, end_date: str, time_of_day: str) -> Vec<QuoteTick>;
    grpc: get_stock_at_time_quote;
    request: StockAtTimeQuoteRequest;
    query: StockAtTimeQuoteRequestQuery {
        symbol: symbol.to_string(),
        start_date: start_date.to_string(),
        end_date: end_date.to_string(),
        time_of_day: time_of_day.to_string(),
        venue: venue.clone().or_else(|| Some("nqb".to_string())),
    };
    parse: decode::parse_quote_ticks;
    dates: start_date, end_date;
    optional { venue: opt_str = None }
}

// ── Option List Contracts (1) ──────────────────────────────────────

// 18. GetOptionListContracts
parsed_endpoint! {
    /// List all option contracts for a symbol on a given date.
    builder OptionListContractsBuilder;
    fn option_list_contracts(request_type: str, symbol: str, date: str) -> Vec<OptionContract>;
    grpc: get_option_list_contracts;
    request: OptionListContractsRequest;
    query: OptionListContractsRequestQuery {
        request_type: request_type.to_string(),
        symbol: vec![symbol.to_string()],
        date: date.to_string(),
        max_dte: max_dte,
    };
    parse: decode::parse_option_contracts_v3;
    dates: date;
    optional { max_dte: opt_i32 = None }
}

// ── Option Snapshot (10) ───────────────────────────────────────────

// 19. GetOptionSnapshotOhlc
parsed_endpoint! {
    /// Get the latest OHLC snapshot for option contracts.
    builder OptionSnapshotOhlcBuilder;
    fn option_snapshot_ohlc(symbol: str, expiration: str, strike: str, right: str) -> Vec<OhlcTick>;
    grpc: get_option_snapshot_ohlc;
    request: OptionSnapshotOhlcRequest;
    query: OptionSnapshotOhlcRequestQuery {
        contract_spec: contract_spec!(symbol, expiration, strike, right),
        expiration: expiration.to_string(),
        max_dte: max_dte,
        strike_range: strike_range,
        min_time: min_time.clone(),
    };
    parse: decode::parse_ohlc_ticks;
    optional { max_dte: opt_i32 = None, strike_range: opt_i32 = None, min_time: opt_str = None }
}

// 20. GetOptionSnapshotTrade
parsed_endpoint! {
    /// Get the latest trade snapshot for option contracts.
    builder OptionSnapshotTradeBuilder;
    fn option_snapshot_trade(symbol: str, expiration: str, strike: str, right: str) -> Vec<TradeTick>;
    grpc: get_option_snapshot_trade;
    request: OptionSnapshotTradeRequest;
    query: OptionSnapshotTradeRequestQuery {
        contract_spec: contract_spec!(symbol, expiration, strike, right),
        expiration: expiration.to_string(),
        strike_range: strike_range,
        min_time: min_time.clone(),
    };
    parse: decode::parse_trade_ticks;
    optional { strike_range: opt_i32 = None, min_time: opt_str = None }
}

// 21. GetOptionSnapshotQuote
parsed_endpoint! {
    /// Get the latest NBBO quote snapshot for option contracts.
    builder OptionSnapshotQuoteBuilder;
    fn option_snapshot_quote(symbol: str, expiration: str, strike: str, right: str) -> Vec<QuoteTick>;
    grpc: get_option_snapshot_quote;
    request: OptionSnapshotQuoteRequest;
    query: OptionSnapshotQuoteRequestQuery {
        contract_spec: contract_spec!(symbol, expiration, strike, right),
        expiration: expiration.to_string(),
        max_dte: max_dte,
        strike_range: strike_range,
        min_time: min_time.clone(),
    };
    parse: decode::parse_quote_ticks;
    optional { max_dte: opt_i32 = None, strike_range: opt_i32 = None, min_time: opt_str = None }
}

// 22. GetOptionSnapshotOpenInterest
parsed_endpoint! {
    /// Get the latest open interest snapshot for option contracts.
    builder OptionSnapshotOpenInterestBuilder;
    fn option_snapshot_open_interest(symbol: str, expiration: str, strike: str, right: str) -> Vec<OpenInterestTick>;
    grpc: get_option_snapshot_open_interest;
    request: OptionSnapshotOpenInterestRequest;
    query: OptionSnapshotOpenInterestRequestQuery {
        contract_spec: contract_spec!(symbol, expiration, strike, right),
        expiration: expiration.to_string(),
        max_dte: max_dte,
        strike_range: strike_range,
        min_time: min_time.clone(),
    };
    parse: decode::parse_open_interest_ticks;
    optional { max_dte: opt_i32 = None, strike_range: opt_i32 = None, min_time: opt_str = None }
}

// 23. GetOptionSnapshotMarketValue
parsed_endpoint! {
    /// Get the latest market value snapshot for option contracts.
    builder OptionSnapshotMarketValueBuilder;
    fn option_snapshot_market_value(symbol: str, expiration: str, strike: str, right: str) -> Vec<MarketValueTick>;
    grpc: get_option_snapshot_market_value;
    request: OptionSnapshotMarketValueRequest;
    query: OptionSnapshotMarketValueRequestQuery {
        contract_spec: contract_spec!(symbol, expiration, strike, right),
        expiration: expiration.to_string(),
        max_dte: max_dte,
        strike_range: strike_range,
        min_time: min_time.clone(),
    };
    parse: decode::parse_market_value_ticks;
    optional { max_dte: opt_i32 = None, strike_range: opt_i32 = None, min_time: opt_str = None }
}

// 24-28: Option Snapshot Greeks (5 endpoints, same optional params)
macro_rules! option_snapshot_greeks_endpoint {
    ($builder:ident, $name:ident, $grpc:ident, $req:ident, $query:ident, $ret:ty, $parser:expr, $doc:literal) => {
        parsed_endpoint! {
            #[doc = $doc]
            builder $builder;
            fn $name(symbol: str, expiration: str, strike: str, right: str) -> $ret;
            grpc: $grpc;
            request: $req;
            query: $query {
                contract_spec: contract_spec!(symbol, expiration, strike, right),
                expiration: expiration.to_string(),
                annual_dividend: annual_dividend,
                rate_type: rate_type.clone(),
                rate_value: rate_value,
                stock_price: stock_price,
                version: version.clone(),
                max_dte: max_dte,
                strike_range: strike_range,
                min_time: min_time.clone(),
                use_market_value: use_market_value,
            };
            parse: $parser;
            optional {
                max_dte: opt_i32 = None,
                strike_range: opt_i32 = None,
                min_time: opt_str = None,
                annual_dividend: opt_f64 = None,
                rate_type: opt_str = None,
                rate_value: opt_f64 = None,
                stock_price: opt_f64 = None,
                version: opt_str = None,
                use_market_value: opt_bool = None,
            }
        }
    };
}

option_snapshot_greeks_endpoint!(
    OptionSnapshotGreeksIvBuilder,
    option_snapshot_greeks_implied_volatility,
    get_option_snapshot_greeks_implied_volatility,
    OptionSnapshotGreeksImpliedVolatilityRequest,
    OptionSnapshotGreeksImpliedVolatilityRequestQuery,
    Vec<IvTick>,
    decode::parse_iv_ticks,
    "Get implied volatility snapshot for option contracts."
);

option_snapshot_greeks_endpoint!(
    OptionSnapshotGreeksAllBuilder,
    option_snapshot_greeks_all,
    get_option_snapshot_greeks_all,
    OptionSnapshotGreeksAllRequest,
    OptionSnapshotGreeksAllRequestQuery,
    Vec<GreeksTick>,
    decode::parse_greeks_ticks,
    "Get all Greeks snapshot for option contracts."
);

option_snapshot_greeks_endpoint!(
    OptionSnapshotGreeksFirstOrderBuilder,
    option_snapshot_greeks_first_order,
    get_option_snapshot_greeks_first_order,
    OptionSnapshotGreeksFirstOrderRequest,
    OptionSnapshotGreeksFirstOrderRequestQuery,
    Vec<GreeksTick>,
    decode::parse_greeks_ticks,
    "Get first-order Greeks snapshot (delta, theta, rho, etc.)."
);

option_snapshot_greeks_endpoint!(
    OptionSnapshotGreeksSecondOrderBuilder,
    option_snapshot_greeks_second_order,
    get_option_snapshot_greeks_second_order,
    OptionSnapshotGreeksSecondOrderRequest,
    OptionSnapshotGreeksSecondOrderRequestQuery,
    Vec<GreeksTick>,
    decode::parse_greeks_ticks,
    "Get second-order Greeks snapshot (gamma, vanna, charm, etc.)."
);

option_snapshot_greeks_endpoint!(
    OptionSnapshotGreeksThirdOrderBuilder,
    option_snapshot_greeks_third_order,
    get_option_snapshot_greeks_third_order,
    OptionSnapshotGreeksThirdOrderRequest,
    OptionSnapshotGreeksThirdOrderRequestQuery,
    Vec<GreeksTick>,
    decode::parse_greeks_ticks,
    "Get third-order Greeks snapshot (speed, color, ultima, etc.)."
);

// ── Option History (6) ─────────────────────────────────────────────

// 29. GetOptionHistoryEod
parsed_endpoint! {
    /// Fetch end-of-day option data for a contract over a date range.
    builder OptionHistoryEodBuilder;
    fn option_history_eod(symbol: str, expiration: str, strike: str, right: str, start: str, end: str) -> Vec<EodTick>;
    grpc: get_option_history_eod;
    request: OptionHistoryEodRequest;
    query: OptionHistoryEodRequestQuery {
        contract_spec: contract_spec!(symbol, expiration, strike, right),
        start_date: start.to_string(),
        end_date: end.to_string(),
        expiration: expiration.to_string(),
        max_dte: max_dte,
        strike_range: strike_range,
    };
    parse: decode::parse_eod_ticks;
    dates: start, end;
    optional { max_dte: opt_i32 = None, strike_range: opt_i32 = None }
}

// 30. GetOptionHistoryOhlc
parsed_endpoint! {
    /// Fetch intraday OHLC bars for an option contract.
    builder OptionHistoryOhlcBuilder;
    fn option_history_ohlc(symbol: str, expiration: str, strike: str, right: str, date: str, interval: str) -> Vec<OhlcTick>;
    grpc: get_option_history_ohlc;
    request: OptionHistoryOhlcRequest;
    query: OptionHistoryOhlcRequestQuery {
        contract_spec: contract_spec!(symbol, expiration, strike, right),
        date: Some(date.to_string()),
        expiration: expiration.to_string(),
        interval: normalize_interval(&interval),
        start_time: Some(start_time.clone()),
        end_time: Some(end_time.clone()),
        strike_range: strike_range,
        start_date: start_date.clone(),
        end_date: end_date.clone(),
    };
    parse: decode::parse_ohlc_ticks;
    dates: date;
    optional {
        start_time: string = "09:30:00".to_string(),
        end_time: string = "16:00:00".to_string(),
        strike_range: opt_i32 = None,
        start_date: opt_str = None,
        end_date: opt_str = None,
    }
}

// 31. GetOptionHistoryTrade
parsed_endpoint! {
    /// Fetch all trades for an option contract on a given date.
    builder OptionHistoryTradeBuilder;
    fn option_history_trade(symbol: str, expiration: str, strike: str, right: str, date: str) -> Vec<TradeTick>;
    grpc: get_option_history_trade;
    request: OptionHistoryTradeRequest;
    query: OptionHistoryTradeRequestQuery {
        contract_spec: contract_spec!(symbol, expiration, strike, right),
        date: Some(date.to_string()),
        expiration: expiration.to_string(),
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

// 32. GetOptionHistoryQuote
parsed_endpoint! {
    /// Fetch NBBO quotes for an option contract on a given date.
    builder OptionHistoryQuoteBuilder;
    fn option_history_quote(symbol: str, expiration: str, strike: str, right: str, date: str, interval: str) -> Vec<QuoteTick>;
    grpc: get_option_history_quote;
    request: OptionHistoryQuoteRequest;
    query: OptionHistoryQuoteRequestQuery {
        contract_spec: contract_spec!(symbol, expiration, strike, right),
        date: Some(date.to_string()),
        expiration: expiration.to_string(),
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

// ── Option Streaming History ───────────────────────────────────────

// 31s. GetOptionHistoryTrade (streaming)
streaming_endpoint! {
    /// Stream all trades for an option contract, chunk-by-chunk.
    builder OptionHistoryTradeStreamBuilder;
    fn option_history_trade_stream(symbol: str, expiration: str, strike: str, right: str, date: str) -> TradeTick;
    grpc: get_option_history_trade;
    request: OptionHistoryTradeRequest;
    query: OptionHistoryTradeRequestQuery {
        contract_spec: contract_spec!(symbol, expiration, strike, right),
        date: Some(date.to_string()),
        expiration: expiration.to_string(),
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

// 32s. GetOptionHistoryQuote (streaming)
streaming_endpoint! {
    /// Stream NBBO quotes for an option contract, chunk-by-chunk.
    builder OptionHistoryQuoteStreamBuilder;
    fn option_history_quote_stream(symbol: str, expiration: str, strike: str, right: str, date: str, interval: str) -> QuoteTick;
    grpc: get_option_history_quote;
    request: OptionHistoryQuoteRequest;
    query: OptionHistoryQuoteRequestQuery {
        contract_spec: contract_spec!(symbol, expiration, strike, right),
        date: Some(date.to_string()),
        expiration: expiration.to_string(),
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

// 33. GetOptionHistoryTradeQuote
parsed_endpoint! {
    /// Fetch combined trade + quote ticks for an option contract.
    builder OptionHistoryTradeQuoteBuilder;
    fn option_history_trade_quote(symbol: str, expiration: str, strike: str, right: str, date: str) -> Vec<TradeQuoteTick>;
    grpc: get_option_history_trade_quote;
    request: OptionHistoryTradeQuoteRequest;
    query: OptionHistoryTradeQuoteRequestQuery {
        contract_spec: contract_spec!(symbol, expiration, strike, right),
        date: Some(date.to_string()),
        expiration: expiration.to_string(),
        start_time: Some(start_time.clone()),
        end_time: Some(end_time.clone()),
        exclusive: exclusive,
        max_dte: max_dte,
        strike_range: strike_range,
        start_date: start_date.clone(),
        end_date: end_date.clone(),
    };
    parse: decode::parse_trade_quote_ticks;
    dates: date;
    optional {
        start_time: string = "09:30:00".to_string(),
        end_time: string = "16:00:00".to_string(),
        exclusive: opt_bool = None,
        max_dte: opt_i32 = None,
        strike_range: opt_i32 = None,
        start_date: opt_str = None,
        end_date: opt_str = None,
    }
}

// 34. GetOptionHistoryOpenInterest
parsed_endpoint! {
    /// Fetch open interest history for an option contract.
    builder OptionHistoryOpenInterestBuilder;
    fn option_history_open_interest(symbol: str, expiration: str, strike: str, right: str, date: str) -> Vec<OpenInterestTick>;
    grpc: get_option_history_open_interest;
    request: OptionHistoryOpenInterestRequest;
    query: OptionHistoryOpenInterestRequestQuery {
        contract_spec: contract_spec!(symbol, expiration, strike, right),
        date: Some(date.to_string()),
        expiration: expiration.to_string(),
        max_dte: max_dte,
        strike_range: strike_range,
        start_date: start_date.clone(),
        end_date: end_date.clone(),
    };
    parse: decode::parse_open_interest_ticks;
    dates: date;
    optional {
        max_dte: opt_i32 = None,
        strike_range: opt_i32 = None,
        start_date: opt_str = None,
        end_date: opt_str = None,
    }
}

// ── Option History Greeks (12) ─────────────────────────────────────

// 35. GetOptionHistoryGreeksEod
parsed_endpoint! {
    /// Fetch end-of-day Greeks history for an option contract.
    builder OptionHistoryGreeksEodBuilder;
    fn option_history_greeks_eod(symbol: str, expiration: str, strike: str, right: str, start_date: str, end_date: str) -> Vec<GreeksTick>;
    grpc: get_option_history_greeks_eod;
    request: OptionHistoryGreeksEodRequest;
    query: OptionHistoryGreeksEodRequestQuery {
        contract_spec: contract_spec!(symbol, expiration, strike, right),
        expiration: expiration.to_string(),
        start_date: start_date.to_string(),
        end_date: end_date.to_string(),
        annual_dividend: annual_dividend,
        rate_type: rate_type.clone(),
        rate_value: rate_value,
        version: version.clone(),
        underlyer_use_nbbo: underlyer_use_nbbo,
        max_dte: max_dte,
        strike_range: strike_range,
    };
    parse: decode::parse_greeks_ticks;
    dates: start_date, end_date;
    optional {
        max_dte: opt_i32 = None,
        strike_range: opt_i32 = None,
        annual_dividend: opt_f64 = None,
        rate_type: opt_str = None,
        rate_value: opt_f64 = None,
        version: opt_str = None,
        underlyer_use_nbbo: opt_bool = None,
    }
}

// Helper macro for option history greeks intraday endpoints (interval-based)
macro_rules! option_history_greeks_interval_endpoint {
    ($builder:ident, $name:ident, $grpc:ident, $req:ident, $query:ident, $ret:ty, $parser:expr, $doc:literal) => {
        parsed_endpoint! {
            #[doc = $doc]
            builder $builder;
            fn $name(symbol: str, expiration: str, strike: str, right: str, date: str, interval: str) -> $ret;
            grpc: $grpc;
            request: $req;
            query: $query {
                contract_spec: contract_spec!(symbol, expiration, strike, right),
                date: Some(date.to_string()),
                expiration: expiration.to_string(),
                start_time: Some(start_time.clone()),
                end_time: Some(end_time.clone()),
                interval: normalize_interval(&interval),
                annual_dividend: annual_dividend,
                rate_type: rate_type.clone(),
                rate_value: rate_value,
                version: version.clone(),
                strike_range: strike_range,
                start_date: start_date.clone(),
                end_date: end_date.clone(),
            };
            parse: $parser;
            dates: date;
            optional {
                start_time: string = "09:30:00".to_string(),
                end_time: string = "16:00:00".to_string(),
                strike_range: opt_i32 = None,
                start_date: opt_str = None,
                end_date: opt_str = None,
                annual_dividend: opt_f64 = None,
                rate_type: opt_str = None,
                rate_value: opt_f64 = None,
                version: opt_str = None,
            }
        }
    };
}

// Helper macro for option history trade-greeks endpoints (no interval)
macro_rules! option_history_trade_greeks_endpoint {
    ($builder:ident, $name:ident, $grpc:ident, $req:ident, $query:ident, $ret:ty, $parser:expr, $doc:literal) => {
        parsed_endpoint! {
            #[doc = $doc]
            builder $builder;
            fn $name(symbol: str, expiration: str, strike: str, right: str, date: str) -> $ret;
            grpc: $grpc;
            request: $req;
            query: $query {
                contract_spec: contract_spec!(symbol, expiration, strike, right),
                date: Some(date.to_string()),
                expiration: expiration.to_string(),
                start_time: Some(start_time.clone()),
                end_time: Some(end_time.clone()),
                annual_dividend: annual_dividend,
                rate_type: rate_type.clone(),
                rate_value: rate_value,
                version: version.clone(),
                max_dte: max_dte,
                strike_range: strike_range,
                start_date: start_date.clone(),
                end_date: end_date.clone(),
            };
            parse: $parser;
            dates: date;
            optional {
                start_time: string = "09:30:00".to_string(),
                end_time: string = "16:00:00".to_string(),
                max_dte: opt_i32 = None,
                strike_range: opt_i32 = None,
                start_date: opt_str = None,
                end_date: opt_str = None,
                annual_dividend: opt_f64 = None,
                rate_type: opt_str = None,
                rate_value: opt_f64 = None,
                version: opt_str = None,
            }
        }
    };
}

// 36. GetOptionHistoryGreeksAll
option_history_greeks_interval_endpoint!(
    OptionHistoryGreeksAllBuilder,
    option_history_greeks_all,
    get_option_history_greeks_all,
    OptionHistoryGreeksAllRequest,
    OptionHistoryGreeksAllRequestQuery,
    Vec<GreeksTick>,
    decode::parse_greeks_ticks,
    "Fetch all Greeks history for an option contract (intraday)."
);

// 37. GetOptionHistoryTradeGreeksAll
option_history_trade_greeks_endpoint!(
    OptionHistoryTradeGreeksAllBuilder,
    option_history_trade_greeks_all,
    get_option_history_trade_greeks_all,
    OptionHistoryTradeGreeksAllRequest,
    OptionHistoryTradeGreeksAllRequestQuery,
    Vec<GreeksTick>,
    decode::parse_greeks_ticks,
    "Fetch all Greeks on each trade for an option contract."
);

// 38. GetOptionHistoryGreeksFirstOrder
option_history_greeks_interval_endpoint!(
    OptionHistoryGreeksFirstOrderBuilder,
    option_history_greeks_first_order,
    get_option_history_greeks_first_order,
    OptionHistoryGreeksFirstOrderRequest,
    OptionHistoryGreeksFirstOrderRequestQuery,
    Vec<GreeksTick>,
    decode::parse_greeks_ticks,
    "Fetch first-order Greeks history (intraday)."
);

// 39. GetOptionHistoryTradeGreeksFirstOrder
option_history_trade_greeks_endpoint!(
    OptionHistoryTradeGreeksFirstOrderBuilder,
    option_history_trade_greeks_first_order,
    get_option_history_trade_greeks_first_order,
    OptionHistoryTradeGreeksFirstOrderRequest,
    OptionHistoryTradeGreeksFirstOrderRequestQuery,
    Vec<GreeksTick>,
    decode::parse_greeks_ticks,
    "Fetch first-order Greeks on each trade for an option contract."
);

// 40. GetOptionHistoryGreeksSecondOrder
option_history_greeks_interval_endpoint!(
    OptionHistoryGreeksSecondOrderBuilder,
    option_history_greeks_second_order,
    get_option_history_greeks_second_order,
    OptionHistoryGreeksSecondOrderRequest,
    OptionHistoryGreeksSecondOrderRequestQuery,
    Vec<GreeksTick>,
    decode::parse_greeks_ticks,
    "Fetch second-order Greeks history (intraday)."
);

// 41. GetOptionHistoryTradeGreeksSecondOrder
option_history_trade_greeks_endpoint!(
    OptionHistoryTradeGreeksSecondOrderBuilder,
    option_history_trade_greeks_second_order,
    get_option_history_trade_greeks_second_order,
    OptionHistoryTradeGreeksSecondOrderRequest,
    OptionHistoryTradeGreeksSecondOrderRequestQuery,
    Vec<GreeksTick>,
    decode::parse_greeks_ticks,
    "Fetch second-order Greeks on each trade for an option contract."
);

// 42. GetOptionHistoryGreeksThirdOrder
option_history_greeks_interval_endpoint!(
    OptionHistoryGreeksThirdOrderBuilder,
    option_history_greeks_third_order,
    get_option_history_greeks_third_order,
    OptionHistoryGreeksThirdOrderRequest,
    OptionHistoryGreeksThirdOrderRequestQuery,
    Vec<GreeksTick>,
    decode::parse_greeks_ticks,
    "Fetch third-order Greeks history (intraday)."
);

// 43. GetOptionHistoryTradeGreeksThirdOrder
option_history_trade_greeks_endpoint!(
    OptionHistoryTradeGreeksThirdOrderBuilder,
    option_history_trade_greeks_third_order,
    get_option_history_trade_greeks_third_order,
    OptionHistoryTradeGreeksThirdOrderRequest,
    OptionHistoryTradeGreeksThirdOrderRequestQuery,
    Vec<GreeksTick>,
    decode::parse_greeks_ticks,
    "Fetch third-order Greeks on each trade for an option contract."
);

// 44. GetOptionHistoryGreeksImpliedVolatility
option_history_greeks_interval_endpoint!(
    OptionHistoryGreeksIvBuilder,
    option_history_greeks_implied_volatility,
    get_option_history_greeks_implied_volatility,
    OptionHistoryGreeksImpliedVolatilityRequest,
    OptionHistoryGreeksImpliedVolatilityRequestQuery,
    Vec<IvTick>,
    decode::parse_iv_ticks,
    "Fetch implied volatility history (intraday)."
);

// 45. GetOptionHistoryTradeGreeksImpliedVolatility
option_history_trade_greeks_endpoint!(
    OptionHistoryTradeGreeksIvBuilder,
    option_history_trade_greeks_implied_volatility,
    get_option_history_trade_greeks_implied_volatility,
    OptionHistoryTradeGreeksImpliedVolatilityRequest,
    OptionHistoryTradeGreeksImpliedVolatilityRequestQuery,
    Vec<IvTick>,
    decode::parse_iv_ticks,
    "Fetch implied volatility on each trade for an option contract."
);

// ── Option At-Time (2) ────────────────────────────────────────────

// 46. GetOptionAtTimeTrade
parsed_endpoint! {
    /// Fetch the trade at a specific time of day for an option.
    builder OptionAtTimeTradeBuilder;
    fn option_at_time_trade(symbol: str, expiration: str, strike: str, right: str, start_date: str, end_date: str, time_of_day: str) -> Vec<TradeTick>;
    grpc: get_option_at_time_trade;
    request: OptionAtTimeTradeRequest;
    query: OptionAtTimeTradeRequestQuery {
        contract_spec: contract_spec!(symbol, expiration, strike, right),
        start_date: start_date.to_string(),
        end_date: end_date.to_string(),
        time_of_day: time_of_day.to_string(),
        expiration: expiration.to_string(),
        max_dte: max_dte,
        strike_range: strike_range,
    };
    parse: decode::parse_trade_ticks;
    dates: start_date, end_date;
    optional { max_dte: opt_i32 = None, strike_range: opt_i32 = None }
}

// 47. GetOptionAtTimeQuote
parsed_endpoint! {
    /// Fetch the quote at a specific time of day for an option.
    builder OptionAtTimeQuoteBuilder;
    fn option_at_time_quote(symbol: str, expiration: str, strike: str, right: str, start_date: str, end_date: str, time_of_day: str) -> Vec<QuoteTick>;
    grpc: get_option_at_time_quote;
    request: OptionAtTimeQuoteRequest;
    query: OptionAtTimeQuoteRequestQuery {
        contract_spec: contract_spec!(symbol, expiration, strike, right),
        start_date: start_date.to_string(),
        end_date: end_date.to_string(),
        time_of_day: time_of_day.to_string(),
        expiration: expiration.to_string(),
        max_dte: max_dte,
        strike_range: strike_range,
    };
    parse: decode::parse_quote_ticks;
    dates: start_date, end_date;
    optional { max_dte: opt_i32 = None, strike_range: opt_i32 = None }
}

// ── Index Snapshot (3) ─────────────────────────────────────────────

// 50. GetIndexSnapshotOhlc
parsed_endpoint! {
    /// Get the latest OHLC snapshot for one or more indices.
    builder IndexSnapshotOhlcBuilder;
    fn index_snapshot_ohlc(symbols: str_vec) -> Vec<OhlcTick>;
    grpc: get_index_snapshot_ohlc;
    request: IndexSnapshotOhlcRequest;
    query: IndexSnapshotOhlcRequestQuery {
        symbol: symbols.iter().map(|s| s.to_string()).collect(),
        min_time: min_time.clone(),
    };
    parse: decode::parse_ohlc_ticks;
    optional { min_time: opt_str = None }
}

// 51. GetIndexSnapshotPrice
parsed_endpoint! {
    /// Get the latest price snapshot for one or more indices.
    builder IndexSnapshotPriceBuilder;
    fn index_snapshot_price(symbols: str_vec) -> Vec<PriceTick>;
    grpc: get_index_snapshot_price;
    request: IndexSnapshotPriceRequest;
    query: IndexSnapshotPriceRequestQuery {
        symbol: symbols.iter().map(|s| s.to_string()).collect(),
        min_time: min_time.clone(),
    };
    parse: decode::parse_price_ticks;
    optional { min_time: opt_str = None }
}

// 52. GetIndexSnapshotMarketValue
parsed_endpoint! {
    /// Get the latest market value snapshot for one or more indices.
    builder IndexSnapshotMarketValueBuilder;
    fn index_snapshot_market_value(symbols: str_vec) -> Vec<MarketValueTick>;
    grpc: get_index_snapshot_market_value;
    request: IndexSnapshotMarketValueRequest;
    query: IndexSnapshotMarketValueRequestQuery {
        symbol: symbols.iter().map(|s| s.to_string()).collect(),
        min_time: min_time.clone(),
    };
    parse: decode::parse_market_value_ticks;
    optional { min_time: opt_str = None }
}

// ── Index History (3) ──────────────────────────────────────────────

// 53. GetIndexHistoryEod
parsed_endpoint! {
    /// Fetch end-of-day index data for a date range.
    builder IndexHistoryEodBuilder;
    fn index_history_eod(symbol: str, start: str, end: str) -> Vec<EodTick>;
    grpc: get_index_history_eod;
    request: IndexHistoryEodRequest;
    query: IndexHistoryEodRequestQuery {
        symbol: symbol.to_string(),
        start_date: start.to_string(),
        end_date: end.to_string(),
    };
    parse: decode::parse_eod_ticks;
    dates: start, end;
    optional {}
}

// 54. GetIndexHistoryOhlc
parsed_endpoint! {
    /// Fetch intraday OHLC bars for an index.
    builder IndexHistoryOhlcBuilder;
    fn index_history_ohlc(symbol: str, start_date: str, end_date: str, interval: str) -> Vec<OhlcTick>;
    grpc: get_index_history_ohlc;
    request: IndexHistoryOhlcRequest;
    query: IndexHistoryOhlcRequestQuery {
        symbol: symbol.to_string(),
        start_date: start_date.to_string(),
        end_date: end_date.to_string(),
        interval: normalize_interval(&interval),
        start_time: Some(start_time.clone()),
        end_time: Some(end_time.clone()),
    };
    parse: decode::parse_ohlc_ticks;
    dates: start_date, end_date;
    optional {
        start_time: string = "09:30:00".to_string(),
        end_time: string = "16:00:00".to_string(),
    }
}

// 55. GetIndexHistoryPrice
parsed_endpoint! {
    /// Fetch intraday price history for an index.
    builder IndexHistoryPriceBuilder;
    fn index_history_price(symbol: str, date: str, interval: str) -> Vec<PriceTick>;
    grpc: get_index_history_price;
    request: IndexHistoryPriceRequest;
    query: IndexHistoryPriceRequestQuery {
        date: Some(date.to_string()),
        symbol: symbol.to_string(),
        start_time: Some(start_time.clone()),
        end_time: Some(end_time.clone()),
        interval: normalize_interval(&interval),
        start_date: start_date.clone(),
        end_date: end_date.clone(),
    };
    parse: decode::parse_price_ticks;
    dates: date;
    optional {
        start_time: string = "09:30:00".to_string(),
        end_time: string = "16:00:00".to_string(),
        start_date: opt_str = None,
        end_date: opt_str = None,
    }
}

// ── Index At-Time (1) ──────────────────────────────────────────────

// 56. GetIndexAtTimePrice
parsed_endpoint! {
    /// Fetch the index price at a specific time of day across a date range.
    builder IndexAtTimePriceBuilder;
    fn index_at_time_price(symbol: str, start_date: str, end_date: str, time_of_day: str) -> Vec<PriceTick>;
    grpc: get_index_at_time_price;
    request: IndexAtTimePriceRequest;
    query: IndexAtTimePriceRequestQuery {
        symbol: symbol.to_string(),
        start_date: start_date.to_string(),
        end_date: end_date.to_string(),
        time_of_day: time_of_day.to_string(),
    };
    parse: decode::parse_price_ticks;
    dates: start_date, end_date;
    optional {}
}

// ── Calendar (3) ───────────────────────────────────────────────────

// 57. GetCalendarOpenToday
parsed_endpoint! {
    /// Check whether the market is open today.
    builder CalendarOpenTodayBuilder;
    fn calendar_open_today() -> Vec<CalendarDay>;
    grpc: get_calendar_open_today;
    request: CalendarOpenTodayRequest;
    query: CalendarOpenTodayRequestQuery {};
    parse: decode::parse_calendar_days;
    optional {}
}

// 58. GetCalendarOnDate
parsed_endpoint! {
    /// Get calendar information for a specific date.
    builder CalendarOnDateBuilder;
    fn calendar_on_date(date: str) -> Vec<CalendarDay>;
    grpc: get_calendar_on_date;
    request: CalendarOnDateRequest;
    query: CalendarOnDateRequestQuery {
        date: date.to_string(),
    };
    parse: decode::parse_calendar_days;
    dates: date;
    optional {}
}

// 59. GetCalendarYear
parsed_endpoint! {
    /// Get calendar information for an entire year.
    builder CalendarYearBuilder;
    fn calendar_year(year: str) -> Vec<CalendarDay>;
    grpc: get_calendar_year;
    request: CalendarYearRequest;
    query: CalendarYearRequestQuery {
        year: year.to_string(),
    };
    parse: decode::parse_calendar_days;
    optional {}
}

// ── Interest Rate (1) ──────────────────────────────────────────────

// 60. GetInterestRateHistoryEod
parsed_endpoint! {
    /// Fetch end-of-day interest rate history.
    builder InterestRateHistoryEodBuilder;
    fn interest_rate_history_eod(symbol: str, start_date: str, end_date: str) -> Vec<InterestRateTick>;
    grpc: get_interest_rate_history_eod;
    request: InterestRateHistoryEodRequest;
    query: InterestRateHistoryEodRequestQuery {
        symbol: symbol.to_string(),
        start_date: start_date.to_string(),
        end_date: end_date.to_string(),
    };
    parse: decode::parse_interest_rate_ticks;
    dates: start_date, end_date;
    optional {}
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
            0 => "100ms".to_string(),
            1..=100 => "100ms".to_string(),
            101..=500 => "500ms".to_string(),
            501..=1000 => "1s".to_string(),
            1001..=5000 => "5s".to_string(),
            5001..=10000 => "10s".to_string(),
            10001..=15000 => "15s".to_string(),
            15001..=30000 => "30s".to_string(),
            30001..=60000 => "1m".to_string(),
            60001..=300000 => "5m".to_string(),
            300001..=600000 => "10m".to_string(),
            600001..=900000 => "15m".to_string(),
            900001..=1800000 => "30m".to_string(),
            _ => "1h".to_string(),
        },
        // Not a number -- pass through and let the server decide.
        Err(_) => interval.to_string(),
    }
}

/// Validate that a date string is in YYYYMMDD format (exactly 8 ASCII digits).
fn validate_date(date: &str) -> Result<(), Error> {
    if date.len() != 8 || !date.bytes().all(|b| b.is_ascii_digit()) {
        return Err(Error::Config(format!(
            "invalid date '{}': expected YYYYMMDD format (8 digits)",
            date
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
        assert_eq!(ticks[0].open, 15000);
        assert_eq!(ticks[0].close, 15100);
        assert_eq!(ticks[0].date, 20240301);
    }
}
