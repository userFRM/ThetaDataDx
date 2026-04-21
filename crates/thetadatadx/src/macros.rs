//! Macros invoked by generated endpoint code from `build_support/endpoints.rs`.
//!
//! These macro_rules drive the builder-pattern gRPC wrappers emitted at build
//! time as well as the handwritten streaming endpoints in [`crate::mdds`].
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
//! `Err(Error::Timeout { duration_ms })`. The `MddsClient` is unaffected;
//! a subsequent call on the same handle succeeds.
//!
//! List endpoints additionally expose a parallel `<name>_with_deadline(...)`
//! async method on `MddsClient`: the existing `pub async fn <name>(...)`
//! signatures stay non-breaking, while the `_with_deadline` variant gives
//! the same cancellation contract for the validator and registry dispatch.

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

/// Policy tick consumed by the retry / refresh loop driven from the
/// endpoint macros. Each call returns either the completed value, a
/// request for another attempt after backoff, or a terminal failure.
pub(crate) enum AttemptStep<T> {
    Ok(T),
    Retry(crate::error::Error),
    Terminal(crate::error::Error),
}

/// Single step evaluated by the macro-driven retry loop.
///
/// `out` is the result of the last attempt (future already awaited).
/// `refreshed_already` tracks whether this call has already consumed
/// its session-refresh budget — a second `Unauthenticated` becomes
/// terminal.
///
/// Exists as a free function so the macros can call it with a plain
/// `Result` produced by their owned request + stream + collect chain,
/// avoiding the higher-ranked trait bounds that broke the previous
/// closure-based helper.
pub(crate) async fn classify_attempt<T>(
    session: &crate::auth::SessionToken,
    snap: &crate::auth::session::SessionSnapshot,
    refreshed_already: &mut bool,
    endpoint: &'static str,
    out: Result<T, crate::error::Error>,
) -> AttemptStep<T> {
    use crate::retry::StatusClass;
    match out {
        Ok(v) => AttemptStep::Ok(v),
        Err(err) => match classify_error(&err) {
            StatusClass::Transient => {
                metrics::counter!(
                    "thetadatadx.grpc.errors",
                    "endpoint" => endpoint.to_string()
                )
                .increment(1);
                AttemptStep::Retry(err)
            }
            StatusClass::NeedsRefresh => {
                if *refreshed_already {
                    metrics::counter!(
                        "thetadatadx.grpc.errors",
                        "endpoint" => endpoint.to_string()
                    )
                    .increment(1);
                    return AttemptStep::Terminal(err);
                }
                match session.refresh(snap).await {
                    Ok(_new_snap) => {
                        *refreshed_already = true;
                        AttemptStep::Retry(err)
                    }
                    Err(refresh_err) => AttemptStep::Terminal(refresh_err),
                }
            }
            StatusClass::Terminal => {
                metrics::counter!(
                    "thetadatadx.grpc.errors",
                    "endpoint" => endpoint.to_string()
                )
                .increment(1);
                AttemptStep::Terminal(err)
            }
        },
    }
}

/// Sleep between retry attempts according to the client's policy.
/// Split out of the macros so the per-endpoint expansion stays flat.
pub(crate) async fn sleep_for_retry(
    policy: &crate::config::RetryPolicy,
    attempt: u32,
    endpoint: &'static str,
    err: &crate::error::Error,
) {
    let delay = policy.delay_for_attempt(attempt);
    metrics::counter!(
        "thetadatadx.grpc.retries",
        "endpoint" => endpoint.to_string()
    )
    .increment(1);
    tracing::warn!(
        endpoint,
        attempt,
        delay_ms = delay.as_millis() as u64,
        error = %err,
        "transient gRPC error — retrying with backoff"
    );
    if !delay.is_zero() {
        tokio::time::sleep(delay).await;
    }
}

/// Classify an [`Error`] for retry / refresh routing.
///
/// `From<tonic::Status>` folds the tonic enum into `Error::Grpc { status, .. }`
/// where `status` is the `Debug` rendering of `tonic::Code` (e.g.
/// `"Unavailable"`, `"Unauthenticated"`). We re-derive the class from
/// that string so the macro-emitted call path stays unchanged. The
/// other `Error` variants are terminal — a `Decode` or `Decompress`
/// failure won't fix itself on retry.
fn classify_error(err: &crate::error::Error) -> crate::retry::StatusClass {
    use crate::retry::StatusClass;
    match err {
        crate::error::Error::Grpc { status, .. } => match status.as_str() {
            "Unavailable" | "DeadlineExceeded" | "ResourceExhausted" => StatusClass::Transient,
            "Unauthenticated" => StatusClass::NeedsRefresh,
            _ => StatusClass::Terminal,
        },
        _ => StatusClass::Terminal,
    }
}

/// Generate a list endpoint that returns `Vec<String>` by extracting a text
/// column from the response `DataTable`.
///
/// Pattern: build request -> gRPC call -> collect stream -> extract text column.
/// Emits two methods on `MddsClient`:
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
        ::pastey::paste! {
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
            #[doc = "`MddsClient` is left intact for subsequent calls."]
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
                    let policy = self.config().retry_policy;
                    let budget = policy.max_attempts.max(1);
                    let mut refreshed_already = false;
                    let mut last_err: Option<Error> = None;
                    let table: proto::DataTable = 'retry: loop {
                        for attempt in 1..=budget {
                            let snap = self.session().snapshot().await;
                            let qi = self.build_query_info(snap.uuid.clone());
                            let request = proto::$req {
                                query_info: Some(qi),
                                params: Some(proto::$query { $($field : $val),* }),
                            };
                            let attempt_result: Result<proto::DataTable, Error> = async {
                                let stream = self.stub().$grpc(request).await
                                    .map_err(|e| -> Error { e.into() })?;
                                self.collect_stream(stream.into_inner()).await
                            }.await;
                            match $crate::macros::classify_attempt(
                                self.session(),
                                &snap,
                                &mut refreshed_already,
                                stringify!($name),
                                attempt_result,
                            ).await {
                                $crate::macros::AttemptStep::Ok(t) => break 'retry t,
                                $crate::macros::AttemptStep::Terminal(err) => return Err::<Vec<String>, Error>(err),
                                $crate::macros::AttemptStep::Retry(err) => {
                                    if attempt == budget {
                                        last_err = Some(err);
                                        break;
                                    }
                                    $crate::macros::sleep_for_retry(&policy, attempt, stringify!($name), &err).await;
                                    last_err = Some(err);
                                }
                            }
                        }
                        return Err(last_err.unwrap_or_else(|| Error::Config("retry loop exited without result".into())));
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
        /// Builder for the [`MddsClient::$name`] endpoint.
        pub struct $builder_name<'a> {
            client: &'a MddsClient,
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
            /// underlying `MddsClient` is unaffected; subsequent calls
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
                        let policy = client.config().retry_policy;
                        let budget = policy.max_attempts.max(1);
                        let mut refreshed_already = false;
                        let mut last_err: Option<Error> = None;
                        let table: proto::DataTable = 'retry: loop {
                            for attempt in 1..=budget {
                                let snap = client.session().snapshot().await;
                                let qi = client.build_query_info(snap.uuid.clone());
                                let request = proto::$req {
                                    query_info: Some(qi),
                                    params: Some(proto::$query { $($field : $val),* }),
                                };
                                let attempt_result: Result<proto::DataTable, Error> = async {
                                    let stream = client.stub().$grpc(request).await
                                        .map_err(|e| -> Error { e.into() })?;
                                    client.collect_stream(stream.into_inner()).await
                                }.await;
                                match $crate::macros::classify_attempt(
                                    client.session(),
                                    &snap,
                                    &mut refreshed_already,
                                    stringify!($name),
                                    attempt_result,
                                ).await {
                                    $crate::macros::AttemptStep::Ok(t) => break 'retry t,
                                    $crate::macros::AttemptStep::Terminal(err) => return Err::<$ret, Error>(err),
                                    $crate::macros::AttemptStep::Retry(err) => {
                                        if attempt == budget {
                                            last_err = Some(err);
                                            break;
                                        }
                                        $crate::macros::sleep_for_retry(&policy, attempt, stringify!($name), &err).await;
                                        last_err = Some(err);
                                    }
                                }
                            }
                            return Err(last_err.unwrap_or_else(|| Error::Config("retry loop exited without result".into())));
                        };
                        metrics::histogram!("thetadatadx.grpc.latency_ms", "endpoint" => stringify!($name))
                            .record(_metrics_start.elapsed().as_secs_f64() * 1_000.0);
                        // Strict decode: type mismatch in any cell propagates
                        // as Error::Decode via `From<DecodeError>`.
                        $parser(&table).map_err(Error::from)
                    };
                    $crate::macros::run_with_optional_deadline(deadline, inner).await
                })
            }
        }

        impl MddsClient {
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

// Tests live at the bottom of the file so `clippy::items-after-test-module`
// stays clean: the macro_rules! blocks above are actual items, and clippy
// forbids items after a `#[cfg(test)] mod tests`.
#[cfg(test)]
mod classify_error_tests {
    use super::classify_error;
    use crate::error::Error;
    use crate::retry::StatusClass;

    fn grpc(status: &str) -> Error {
        Error::Grpc {
            status: status.to_string(),
            message: String::new(),
        }
    }

    #[test]
    fn transient_status_strings_map_to_transient() {
        assert_eq!(classify_error(&grpc("Unavailable")), StatusClass::Transient);
        assert_eq!(
            classify_error(&grpc("DeadlineExceeded")),
            StatusClass::Transient
        );
        assert_eq!(
            classify_error(&grpc("ResourceExhausted")),
            StatusClass::Transient
        );
    }

    #[test]
    fn unauthenticated_maps_to_needs_refresh() {
        assert_eq!(
            classify_error(&grpc("Unauthenticated")),
            StatusClass::NeedsRefresh
        );
    }

    #[test]
    fn unknown_status_maps_to_terminal() {
        assert_eq!(
            classify_error(&grpc("PermissionDenied")),
            StatusClass::Terminal
        );
        assert_eq!(classify_error(&grpc("NotFound")), StatusClass::Terminal);
        assert_eq!(
            classify_error(&grpc("InvalidArgument")),
            StatusClass::Terminal
        );
    }

    #[test]
    fn non_grpc_errors_are_terminal() {
        assert_eq!(
            classify_error(&Error::Config("bad config".into())),
            StatusClass::Terminal
        );
        assert_eq!(
            classify_error(&Error::Decode("parse fail".into())),
            StatusClass::Terminal
        );
    }
}
