//! Macros invoked by generated endpoint code from `build_support/endpoints.rs`.
//!
//! These macro_rules drive the builder-pattern gRPC wrappers emitted at build
//! time as well as the handwritten streaming endpoints in [`crate::direct`].
//! They are declared with `#[macro_use]` in `lib.rs` so every sibling module
//! can reference them.
//!
//! ## Per-call deadlines
//!
//! Every generated builder exposes [`with_deadline(Duration)`](#with_deadline)
//! which wraps the in-flight gRPC call (`<grpc>` + `collect_stream`) in
//! [`tokio::time::timeout`]. On expiry the future is dropped: the local
//! `_permit` releases the request-semaphore slot, the tonic `Streaming` is
//! dropped (RST_STREAM on the underlying H2 stream), and the call returns
//! `Err(Error::Timeout { duration_ms })`. The `DirectClient` is unaffected;
//! a subsequent call on the same handle succeeds.
//!
//! List endpoints additionally expose a parallel `<name>_with_deadline(...)`
//! async method on `DirectClient`: the existing `pub async fn <name>(...)`
//! signatures stay non-breaking, while the `_with_deadline` variant gives
//! the same cancellation contract for the validator and registry dispatch.
//!
//! See [`docs/dev/w3-async-cancellation-design.md`].

/// Run a future with an optional per-call deadline.
///
/// When `deadline` is `None` the future is awaited verbatim. When `Some(d)`
/// the future is wrapped in [`tokio::time::timeout`]; on elapsed the future
/// is dropped and `Error::Timeout { duration_ms }` is returned. Local state
/// captured by the future (`_permit`, `tonic::Streaming`) drops with it.
pub(crate) async fn run_with_optional_deadline<F, T>(
    deadline: Option<std::time::Duration>,
    fut: F,
) -> Result<T, crate::error::Error>
where
    F: std::future::Future<Output = Result<T, crate::error::Error>>,
{
    match deadline {
        None => fut.await,
        Some(d) => match tokio::time::timeout(d, fut).await {
            Ok(inner) => inner,
            Err(_) => Err(crate::error::Error::Timeout {
                duration_ms: u64::try_from(d.as_millis()).unwrap_or(u64::MAX),
            }),
        },
    }
}

/// Generate a list endpoint that returns `Vec<String>` by extracting a text
/// column from the response `DataTable`.
///
/// Pattern: build request -> gRPC call -> collect stream -> extract text column.
/// Emits two methods on `DirectClient`:
/// - `pub async fn <name>(...)` — no deadline.
/// - `pub async fn <name>_with_deadline(deadline, ...)` — caps the call.
macro_rules! list_endpoint {
    (
        $(#[$meta:meta])*
        fn $name:ident( $($arg:ident : $arg_ty:ty),* ) -> $col:literal;
        grpc: $grpc:ident;
        request: $req:ident;
        query: $query:ident { $($field:ident : $val:expr),* $(,)? };
    ) => {
        ::paste::paste! {
            #[allow(clippy::too_many_arguments)] // Reason: ThetaData endpoints require many parameters (symbol, date, strike, exp, right, etc.).
            $(#[$meta])*
            /// # Errors
            ///
            /// Returns an error on network, authentication, or parsing failure.
            pub async fn $name(&self, $($arg : $arg_ty),*) -> Result<Vec<String>, Error> {
                self.[<__ $name _impl>](None, $($arg),*).await
            }

            #[allow(clippy::too_many_arguments)] // Reason: forwarding to ThetaData endpoint with cross-cutting deadline.
            $(#[$meta])*
            #[doc = "Variant with a per-call deadline. On expiry the in-flight gRPC"]
            #[doc = "call is cancelled and `Err(Error::Timeout)` is returned; the"]
            #[doc = "`DirectClient` is left intact for subsequent calls."]
            #[doc = ""]
            #[doc = "`Duration::ZERO` is normalized to \"no deadline\" for parity with"]
            #[doc = "the builder-style endpoints; pass a positive `Duration` for"]
            #[doc = "near-instant expiration."]
            /// # Errors
            ///
            /// Returns `Error::Timeout` if `deadline` elapses, otherwise the same
            /// errors as the deadline-less variant.
            pub async fn [<$name _with_deadline>](&self, deadline: std::time::Duration, $($arg : $arg_ty),*) -> Result<Vec<String>, Error> {
                let normalized = if deadline.is_zero() { None } else { Some(deadline) };
                self.[<__ $name _impl>](normalized, $($arg),*).await
            }

            #[allow(non_snake_case, clippy::too_many_arguments)] // Reason: synthetic-name impl shared between deadline / no-deadline entry points.
            async fn [<__ $name _impl>](&self, __deadline: Option<std::time::Duration>, $($arg : $arg_ty),*) -> Result<Vec<String>, Error> {
                let inner = async move {
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
                            return Err::<Vec<String>, Error>(e.into());
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
                };
                $crate::macros::run_with_optional_deadline(__deadline, inner).await
            }
        }
    };
}

/// Generate an endpoint that returns parsed tick data (`Vec<T>`) via a builder.
///
/// The endpoint method returns a builder struct that captures required params.
/// Optional params are set via chainable setter methods. A per-call deadline
/// is set via `with_deadline(Duration)`. `.await` (via `IntoFuture`) executes
/// the gRPC call.
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
///     .with_deadline(std::time::Duration::from_secs(60))
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
            pub(crate) deadline: Option<std::time::Duration>,
        }

        impl<'a> $builder_name<'a> {
            $(
                opt_setter!($opt_name, $opt_kind);
            )*

            /// Apply a per-call deadline.
            ///
            /// On expiry the in-flight gRPC call is cancelled and the
            /// builder's future resolves to `Err(Error::Timeout)`. The
            /// underlying `DirectClient` is unaffected; subsequent calls
            /// on the same handle succeed.
            ///
            /// `Duration::ZERO` is normalized to "no deadline". The
            /// alternative — wrapping in `tokio::time::timeout(ZERO, ...)` —
            /// would fire on the first poll and never let the call complete,
            /// almost certainly not the caller's intent. Pass a positive
            /// `Duration` (e.g. `Duration::from_millis(1)`) for a near-instant
            /// expiration.
            #[must_use]
            pub fn with_deadline(mut self, duration: std::time::Duration) -> Self {
                self.deadline = if duration.is_zero() { None } else { Some(duration) };
                self
            }
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
                        deadline,
                    } = self;
                    let _ = &client;
                    $($(validate_date(&$date_arg)?;)+)?
                    let inner = async move {
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
                                return Err::<$ret, Error>(e.into());
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
                    };
                    $crate::macros::run_with_optional_deadline(deadline, inner).await
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
                    deadline: None,
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
