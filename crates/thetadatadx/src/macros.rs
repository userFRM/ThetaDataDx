//! Macros invoked by generated endpoint code from `build_support/endpoints.rs`.
//!
//! These macro_rules drive the builder-pattern gRPC wrappers emitted at build
//! time as well as the handwritten streaming endpoints in [`crate::direct`].
//! They are declared with `#[macro_use]` in `lib.rs` so every sibling module
//! can reference them.

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
