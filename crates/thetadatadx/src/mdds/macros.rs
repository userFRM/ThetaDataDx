//! Macros invoked by generated endpoint code from `build_support/endpoints/`.
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

/// Verdict produced by [`classify_error`] on a failed RPC attempt.
///
/// | Variant | Meaning |
/// |---|---|
/// | `Transient` | `Unavailable` / `DeadlineExceeded` / `ResourceExhausted` — retry with backoff |
/// | `NeedsRefresh` | `Unauthenticated` — refresh session then retry once |
/// | `Terminal` | Every other error — surface to caller unchanged |
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StatusClass {
    Transient,
    NeedsRefresh,
    Terminal,
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

/// Decision returned by the streaming retry classifier after a single
/// attempt of an MDDS server-streaming RPC.
///
/// The streaming retry shell drives one of three transitions on each
/// outcome:
///
/// | Variant     | Meaning                                                                                  |
/// |-------------|------------------------------------------------------------------------------------------|
/// | `Done`      | The stream completed (chunk handler saw end-of-stream cleanly).                          |
/// | `Refresh`   | `Unauthenticated` observed — refresh the session and restart from chunk zero.            |
/// | `Backoff`   | Transient (`Unavailable` / `DeadlineExceeded` / `ResourceExhausted`) — sleep + restart.  |
/// | `Terminal`  | Decode / decompress / non-retryable status — surface to caller.                          |
#[cfg_attr(test, derive(Debug))]
pub(crate) enum StreamingAttemptOutcome {
    Done,
    Refresh(crate::error::Error),
    Backoff(crate::error::Error),
    Terminal(crate::error::Error),
}

/// Classify the outcome of a single streaming-RPC attempt for the
/// retry / refresh shell driven from the generated streaming endpoints.
///
/// Mirrors [`classify_attempt`] for the non-streaming path but resolves
/// the refresh side-effect inline so the caller does not have to track
/// `refreshed_already` in two places. Refresh budget is the same as the
/// unary path: at most one refresh per call.
///
/// Upstream MDDS does not support mid-stream resume, so a successful
/// refresh restarts the stream from chunk zero. Callers that drive the
/// chunk handler must therefore tolerate seeing the first N chunks
/// twice on a refresh; idempotent counters / accumulators are the
/// expected handler shape.
pub(crate) async fn classify_streaming_attempt(
    session: &crate::auth::SessionToken,
    snap: &crate::auth::session::SessionSnapshot,
    refreshed_already: &mut bool,
    endpoint: &'static str,
    out: Result<(), crate::error::Error>,
) -> StreamingAttemptOutcome {
    match out {
        Ok(()) => StreamingAttemptOutcome::Done,
        Err(err) => match classify_error(&err) {
            StatusClass::Transient => {
                metrics::counter!(
                    "thetadatadx.grpc.errors",
                    "endpoint" => endpoint.to_string()
                )
                .increment(1);
                StreamingAttemptOutcome::Backoff(err)
            }
            StatusClass::NeedsRefresh => {
                if *refreshed_already {
                    metrics::counter!(
                        "thetadatadx.grpc.errors",
                        "endpoint" => endpoint.to_string()
                    )
                    .increment(1);
                    return StreamingAttemptOutcome::Terminal(err);
                }
                match session.refresh(snap).await {
                    Ok(_new_snap) => {
                        *refreshed_already = true;
                        StreamingAttemptOutcome::Refresh(err)
                    }
                    Err(refresh_err) => StreamingAttemptOutcome::Terminal(refresh_err),
                }
            }
            StatusClass::Terminal => {
                metrics::counter!(
                    "thetadatadx.grpc.errors",
                    "endpoint" => endpoint.to_string()
                )
                .increment(1);
                StreamingAttemptOutcome::Terminal(err)
            }
        },
    }
}

/// Drive the unary endpoint retry / refresh loop.
///
/// Single source of truth for the control flow previously open-coded
/// in three macro arms. The closure receives the current session
/// snapshot and returns the per-attempt result; the helper handles
/// snapshotting, classification, refresh, backoff, and the
/// post-refresh re-attempt budget.
///
/// Auth recovery (session refresh) is intentionally independent of
/// `policy.max_attempts`: even with `RetryPolicy::disabled()`
/// (budget = 1), a single `Unauthenticated` triggers refresh + one
/// post-refresh re-attempt. Subsequent failures surface to the
/// caller.
pub(crate) async fn run_unary_retry_loop<T, F, Fut>(
    session: &crate::auth::SessionToken,
    policy: &crate::config::RetryPolicy,
    endpoint: &'static str,
    mut attempt_fn: F,
) -> Result<T, crate::error::Error>
where
    F: FnMut(crate::auth::session::SessionSnapshot) -> Fut,
    Fut: std::future::Future<Output = Result<T, crate::error::Error>>,
{
    let budget = policy.max_attempts.max(1);
    let mut refreshed_already = false;
    let mut refresh_retry_used = false;
    let mut attempt: u32 = 1;
    loop {
        let snap = session.snapshot().await;
        let attempt_result = attempt_fn(snap.clone()).await;
        let refreshed_before = refreshed_already;
        match classify_attempt(
            session,
            &snap,
            &mut refreshed_already,
            endpoint,
            attempt_result,
        )
        .await
        {
            AttemptStep::Ok(v) => return Ok(v),
            AttemptStep::Terminal(err) => return Err(err),
            AttemptStep::Retry(err) => {
                let refresh_just_now = !refreshed_before && refreshed_already;
                let can_post_refresh = refresh_just_now && !refresh_retry_used;
                if attempt >= budget && !can_post_refresh {
                    return Err(err);
                }
                if can_post_refresh {
                    refresh_retry_used = true;
                } else {
                    sleep_for_retry(policy, attempt, endpoint, &err).await;
                }
                attempt += 1;
            }
        }
    }
}

/// Drive the streaming endpoint retry / refresh loop.
///
/// Streaming sibling of [`run_unary_retry_loop`]: each closure call
/// represents one full server-streaming attempt. Mid-stream
/// `Unauthenticated` triggers refresh + restart from chunk zero
/// (MDDS has no resume token); the closure is invoked again with
/// the post-refresh snapshot.
pub(crate) async fn run_streaming_retry_loop<F, Fut>(
    session: &crate::auth::SessionToken,
    policy: &crate::config::RetryPolicy,
    endpoint: &'static str,
    mut attempt_fn: F,
) -> Result<(), crate::error::Error>
where
    F: FnMut(crate::auth::session::SessionSnapshot) -> Fut,
    Fut: std::future::Future<Output = Result<(), crate::error::Error>>,
{
    let budget = policy.max_attempts.max(1);
    let mut refreshed_already = false;
    let mut refresh_retry_used = false;
    let mut attempt: u32 = 1;
    loop {
        let snap = session.snapshot().await;
        let attempt_result = attempt_fn(snap.clone()).await;
        match classify_streaming_attempt(
            session,
            &snap,
            &mut refreshed_already,
            endpoint,
            attempt_result,
        )
        .await
        {
            StreamingAttemptOutcome::Done => return Ok(()),
            StreamingAttemptOutcome::Terminal(err) => return Err(err),
            StreamingAttemptOutcome::Refresh(err) => {
                if refresh_retry_used {
                    return Err(err);
                }
                refresh_retry_used = true;
                tracing::warn!(
                    endpoint,
                    attempt,
                    error = %err,
                    "session refresh during stream — restarting from chunk zero"
                );
                attempt += 1;
            }
            StreamingAttemptOutcome::Backoff(err) => {
                if attempt >= budget {
                    return Err(err);
                }
                sleep_for_retry(policy, attempt, endpoint, &err).await;
                attempt += 1;
            }
        }
    }
}

/// Classify an [`Error`] for retry / refresh routing.
///
/// `From<tonic::Status>` folds the tonic enum into
/// `Error::Grpc { kind: GrpcStatusKind::*, .. }`. We dispatch on the
/// typed `kind` so the retry classifier no longer parses status
/// strings. Other `Error` variants are terminal — a `Decode` or
/// `Decompress` failure won't fix itself on retry.
fn classify_error(err: &crate::error::Error) -> StatusClass {
    use crate::error::GrpcStatusKind;
    match err {
        crate::error::Error::Grpc { kind, .. } => match kind {
            GrpcStatusKind::Unavailable
            | GrpcStatusKind::DeadlineExceeded
            | GrpcStatusKind::ResourceExhausted => StatusClass::Transient,
            GrpcStatusKind::Unauthenticated => StatusClass::NeedsRefresh,
            _ => StatusClass::Terminal,
        },
        _ => StatusClass::Terminal,
    }
}

/// Generate a list endpoint that returns `Vec<String>` by extracting a text
/// column from the response `DataTable`.
///
/// Pattern: build request -> gRPC call -> collect stream -> extract text column.
/// Emits one method on `MddsClient`:
/// - `pub async fn <name>(...)` — per-call deadline routed through
///   [`EndpointArgs::with_timeout_ms`] + the builder-style APIs.
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
                tracing::debug!(endpoint = stringify!($name), "gRPC request");
                metrics::counter!("thetadatadx.grpc.requests", "endpoint" => stringify!($name)).increment(1);
                let _metrics_start = std::time::Instant::now();
                let _permit = self.request_semaphore.acquire().await
                    .map_err(|_| Error::config_internal("request semaphore closed"))?;
                let policy = self.config().retry;
                let table: proto::DataTable = $crate::mdds::macros::run_unary_retry_loop(
                    self.session(),
                    &policy,
                    stringify!($name),
                    |snap| async move {
                        let qi = self.build_query_info(snap.uuid.clone());
                        let request = proto::$req {
                            query_info: Some(qi),
                            params: Some(proto::$query { $($field : $val),* }),
                        };
                        // Bind the lease to a local so it lives across
                        // the await — the pre-dispatch reservation
                        // must outlive `server_streaming` for the
                        // picker fix (Finding 4) to count pending
                        // opens correctly under burst contention.
                        // Deref coercion from `&ChannelLease` to
                        // `&Channel` satisfies the generated stub
                        // signature.
                        let lease = self.channel();
                        let stream = $crate::proto::beta_theta_terminal::$grpc(
                            &lease,
                            request,
                        )
                        .await
                        .map_err(|e| -> Error { e.into() })?;
                        self.collect_stream(stream).await
                    },
                ).await?;
                metrics::histogram!("thetadatadx.grpc.latency_ms", "endpoint" => stringify!($name))
                    .record(_metrics_start.elapsed().as_secs_f64() * 1_000.0);
                Ok(decode::extract_text_column(&table, $col)
                    .into_iter()
                    .flatten()
                    .collect())
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
/// // `ignore` here because the macro example references a live
/// // `client` value — there is no in-scope construction path for a
/// // doc-test to spin up an authenticated `MddsClient` without
/// // credentials.
/// // Simple -- just .await the builder directly
/// let ticks = client.stock_history_ohlc("AAPL", "20260401").await?;
///
/// // With options -- chain setters before .await
/// let ticks = client.stock_history_ohlc("AAPL", "20260401")
///     .interval("1m")
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
        item: $item:ty;
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

            /// Stream the response chunk-by-chunk via `handler`, never
            /// materializing the full `Vec<T>`.
            ///
            /// Issue #565 OOM fix: the buffered `.await -> Vec<T>` path
            /// holds three live copies (h2 frames + concatenated proto
            /// payload + decoded `Vec<T>`) plus a `Vec::push` doubling
            /// transient, yielding the 6× memory amplification the
            /// production user reproduced on `option_history_quote`
            /// with `interval=tick`, `strike_range=5`, 1DTE, 32-permit
            /// concurrency (~23 GiB RSS). The `.stream()` variant
            /// decodes one chunk at a time, hands the slice to
            /// `handler`, then drops the chunk before the next is
            /// fetched — bounded peak memory regardless of response
            /// size.
            ///
            /// # Retry / refresh semantics
            ///
            /// Same shell as the buffered path: transient gRPC
            /// statuses (`Unavailable`, `DeadlineExceeded`,
            /// `ResourceExhausted`) trigger backoff + restart;
            /// mid-stream `Unauthenticated` triggers one session
            /// refresh then restart from chunk zero (upstream MDDS
            /// has no resume token). Keep `handler` idempotent —
            /// the first N chunks of a failed attempt are visible
            /// to `handler` BEFORE the retry begins.
            ///
            /// Decode / decompress failures are terminal and surface
            /// immediately without retry — the wire bytes won't fix
            /// themselves on a re-attempt.
            ///
            /// # Errors
            ///
            /// Returns [`Error`] if the gRPC call fails terminally or
            /// response parsing fails. `Error::Timeout` on deadline
            /// expiry. `Error::Decompress { kind: MessageTooLarge }`
            /// when the channel's `max_message_size` ceiling rejects
            /// an oversized chunk.
            pub async fn stream<F>(self, mut handler: F) -> Result<(), Error>
            where
                F: FnMut(&[$item]),
            {
                let $builder_name {
                    client,
                    $($req_arg,)*
                    $($opt_name,)*
                    deadline,
                } = self;
                let _ = &client;
                $($($crate::mdds::validate::validate_date_required(&$date_arg)?;)+)?
                $crate::mdds::macros::run_with_optional_deadline(deadline, async move {
                    tracing::debug!(endpoint = stringify!($name), "gRPC streaming request");
                    metrics::counter!("thetadatadx.grpc.requests", "endpoint" => stringify!($name)).increment(1);
                    let _metrics_start = std::time::Instant::now();
                    let _permit = client.request_semaphore.acquire().await
                        .map_err(|_| Error::config_internal("request semaphore closed"))?;
                    let policy = client.config().retry;
                    // The retry-loop helper drives an `FnMut` closure
                    // that returns a future. The user `handler: FnMut`
                    // must outlive multiple closure invocations
                    // (post-refresh restart) AND be reachable from
                    // inside the `for_each_chunk` callback. Wrap it
                    // in a `RefCell` so the future can borrow it
                    // mutably without the outer closure capturing a
                    // `&mut` that would escape its body.
                    let handler_cell = std::cell::RefCell::new(&mut handler);
                    let handler_cell = &handler_cell;
                    $crate::mdds::macros::run_streaming_retry_loop(
                        client.session(),
                        &policy,
                        stringify!($name),
                        move |snap| {
                            // Clone per-attempt: the FnMut closure
                            // may be invoked twice (post-refresh
                            // restart), and the proto request takes
                            // ownership of the param values, so the
                            // owned bindings must outlive the loop
                            // and clone fresh on each iteration.
                            $(let $req_arg = $req_arg.clone();)*
                            $(let $opt_name = $opt_name.clone();)*
                            async move {
                                let qi = client.build_query_info(snap.uuid.clone());
                                let request = proto::$req {
                                    query_info: Some(qi),
                                    params: Some(proto::$query { $($field : $val),* }),
                                };
                                let lease = client.channel();
                                let stream = $crate::proto::beta_theta_terminal::$grpc(
                                    &lease,
                                    request,
                                )
                                .await
                                .map_err(|e| -> Error { e.into() })?;
                                // Strict decode: a parse error inside a
                                // chunk is captured here and surfaced
                                // after `for_each_chunk` returns. The
                                // `for_each_chunk` closure cannot
                                // propagate Result, so the short-circuit
                                // is via the captured `Option<Error>`.
                                let mut decode_error: Option<Error> = None;
                                let drain_result = client.for_each_chunk(stream, |_headers, rows| {
                                    if decode_error.is_some() {
                                        return;
                                    }
                                    let chunk_table = proto::DataTable {
                                        headers: _headers.to_vec(),
                                        data_table: rows.to_vec(),
                                    };
                                    match $parser(&chunk_table) {
                                        Ok(ticks) => (handler_cell.borrow_mut())(&ticks),
                                        Err(e) => decode_error = Some(Error::from(e)),
                                    }
                                }).await;
                                drain_result.and_then(|()| match decode_error {
                                    Some(e) => Err(e),
                                    None => Ok(()),
                                })
                            }
                        },
                    ).await?;
                    metrics::histogram!("thetadatadx.grpc.latency_ms", "endpoint" => stringify!($name))
                        .record(_metrics_start.elapsed().as_secs_f64() * 1_000.0);
                    Ok::<(), Error>(())
                }).await
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
                    $($($crate::mdds::validate::validate_date_required(&$date_arg)?;)+)?
                    let inner = async move {
                        tracing::debug!(endpoint = stringify!($name), "gRPC request");
                        metrics::counter!("thetadatadx.grpc.requests", "endpoint" => stringify!($name)).increment(1);
                        let _metrics_start = std::time::Instant::now();
                        let _permit = client.request_semaphore.acquire().await
                            .map_err(|_| Error::config_internal("request semaphore closed"))?;
                        let policy = client.config().retry;
                        let table: proto::DataTable = $crate::mdds::macros::run_unary_retry_loop(
                            client.session(),
                            &policy,
                            stringify!($name),
                            |snap| {
                                // Clone per-attempt: see stream() arm
                                // for the rationale — owned String /
                                // Vec<String> fields move into the
                                // proto request, so the FnMut closure
                                // must clone before each invocation.
                                $(let $req_arg = $req_arg.clone();)*
                                $(let $opt_name = $opt_name.clone();)*
                                async move {
                                    let qi = client.build_query_info(snap.uuid.clone());
                                    let request = proto::$req {
                                        query_info: Some(qi),
                                        params: Some(proto::$query { $($field : $val),* }),
                                    };
                                    // Bind the lease to a local so it
                                    // lives across the await — see
                                    // the sibling macro arm above for
                                    // the full rationale. Deref
                                    // coercion from `&ChannelLease`
                                    // to `&Channel` satisfies the
                                    // generated stub signature.
                                    let lease = client.channel();
                                    let stream = $crate::proto::beta_theta_terminal::$grpc(
                                        &lease,
                                        request,
                                    )
                                    .await
                                    .map_err(|e| -> Error { e.into() })?;
                                    client.collect_stream(stream).await
                                }
                            },
                        ).await?;
                        metrics::histogram!("thetadatadx.grpc.latency_ms", "endpoint" => stringify!($name))
                            .record(_metrics_start.elapsed().as_secs_f64() * 1_000.0);
                        // Strict decode: type mismatch in any cell propagates
                        // as Error::Decode via `From<DecodeError>`.
                        $parser(&table).map_err(Error::from)
                    };
                    $crate::mdds::macros::run_with_optional_deadline(deadline, inner).await
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
        impl Into<SymbolInput>
    };
}

/// Convert a required param from the user-facing type to the stored type.
macro_rules! req_convert {
    (str, $v:ident) => {
        $v.to_string()
    };
    (str_vec, $v:ident) => {
        $v.into().into_vec()
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
    use super::{classify_error, StatusClass};
    use crate::error::{Error, GrpcStatusKind};

    fn grpc(kind: GrpcStatusKind) -> Error {
        Error::Grpc {
            kind,
            message: String::new(),
        }
    }

    #[test]
    fn transient_status_kinds_map_to_transient() {
        assert_eq!(
            classify_error(&grpc(GrpcStatusKind::Unavailable)),
            StatusClass::Transient
        );
        assert_eq!(
            classify_error(&grpc(GrpcStatusKind::DeadlineExceeded)),
            StatusClass::Transient
        );
        assert_eq!(
            classify_error(&grpc(GrpcStatusKind::ResourceExhausted)),
            StatusClass::Transient
        );
    }

    #[test]
    fn unauthenticated_maps_to_needs_refresh() {
        assert_eq!(
            classify_error(&grpc(GrpcStatusKind::Unauthenticated)),
            StatusClass::NeedsRefresh
        );
    }

    #[test]
    fn unknown_status_maps_to_terminal() {
        assert_eq!(
            classify_error(&grpc(GrpcStatusKind::PermissionDenied)),
            StatusClass::Terminal
        );
        assert_eq!(
            classify_error(&grpc(GrpcStatusKind::NotFound)),
            StatusClass::Terminal
        );
        assert_eq!(
            classify_error(&grpc(GrpcStatusKind::InvalidArgument)),
            StatusClass::Terminal
        );
    }

    #[test]
    fn non_grpc_errors_are_terminal() {
        assert_eq!(
            classify_error(&Error::config_invalid("mdds.endpoint", "bad config")),
            StatusClass::Terminal
        );
        assert_eq!(
            classify_error(&Error::decode_codec("parse fail")),
            StatusClass::Terminal
        );
    }
}

#[cfg(test)]
mod streaming_attempt_tests {
    //! Outcome routing for the streaming retry / refresh shell driven by
    //! the generated streaming endpoints. The classifier is the seam the
    //! generator hooks into — these tests pin its behaviour so a future
    //! refactor of the generated code cannot accidentally re-introduce
    //! the silent-fail-on-mid-stream-Unauthenticated regression.
    use super::{classify_streaming_attempt, StreamingAttemptOutcome};
    use crate::auth::session::{SessionSnapshot, SessionToken};
    use crate::auth::Credentials;
    use crate::error::{Error, GrpcStatusKind};

    fn fake_token(uuid: &str) -> SessionToken {
        SessionToken::new(
            uuid.to_string(),
            "https://nexus.example.invalid/auth".to_string(),
            Credentials::new("user@example.com", "hunter2"),
        )
    }

    fn grpc(kind: GrpcStatusKind) -> Error {
        Error::Grpc {
            kind,
            message: String::new(),
        }
    }

    #[tokio::test]
    async fn ok_attempt_yields_done() {
        let session = fake_token("v0");
        let snap = SessionSnapshot {
            uuid: "v0".to_string(),
            version: 0,
        };
        let mut refreshed = false;
        let out = classify_streaming_attempt(
            &session,
            &snap,
            &mut refreshed,
            "test_stream_endpoint",
            Ok::<(), Error>(()),
        )
        .await;
        assert!(matches!(out, StreamingAttemptOutcome::Done));
        assert!(!refreshed, "Done path must not consume refresh budget");
    }

    #[tokio::test]
    async fn transient_status_routes_to_backoff() {
        let session = fake_token("v0");
        let snap = session.snapshot().await;
        let mut refreshed = false;
        for kind in [
            GrpcStatusKind::Unavailable,
            GrpcStatusKind::DeadlineExceeded,
            GrpcStatusKind::ResourceExhausted,
        ] {
            let out = classify_streaming_attempt(
                &session,
                &snap,
                &mut refreshed,
                "test_stream_endpoint",
                Err::<(), Error>(grpc(kind)),
            )
            .await;
            assert!(
                matches!(out, StreamingAttemptOutcome::Backoff(_)),
                "transient kind {kind:?} should route to Backoff"
            );
        }
        assert!(
            !refreshed,
            "Backoff path must not consume the refresh budget"
        );
    }

    #[tokio::test]
    async fn unauthenticated_exhausted_budget_routes_to_terminal() {
        let session = fake_token("v0");
        let snap = session.snapshot().await;
        let mut refreshed = true; // budget already consumed by a prior attempt
        let out = classify_streaming_attempt(
            &session,
            &snap,
            &mut refreshed,
            "test_stream_endpoint",
            Err::<(), Error>(grpc(GrpcStatusKind::Unauthenticated)),
        )
        .await;
        match out {
            StreamingAttemptOutcome::Terminal(err) => match err {
                Error::Grpc {
                    kind: GrpcStatusKind::Unauthenticated,
                    ..
                } => {}
                other => panic!("expected Unauthenticated, got {other:?}"),
            },
            other => panic!("expected Terminal after refresh budget exhausted, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn unauthenticated_with_failed_refresh_routes_to_terminal() {
        // Refresh attempt hits the unreachable Nexus URL — the classifier
        // must surface the refresh error as terminal rather than silently
        // pretending a refresh happened. The `refreshed_already` flag
        // must NOT flip when the refresh failed.
        let session = fake_token("v0");
        let snap = session.snapshot().await;
        let mut refreshed = false;
        let out = classify_streaming_attempt(
            &session,
            &snap,
            &mut refreshed,
            "test_stream_endpoint",
            Err::<(), Error>(grpc(GrpcStatusKind::Unauthenticated)),
        )
        .await;
        assert!(
            matches!(out, StreamingAttemptOutcome::Terminal(_)),
            "failed refresh must terminate"
        );
        assert!(
            !refreshed,
            "refresh budget must NOT flip when the refresh round-trip itself failed"
        );
    }

    #[tokio::test]
    async fn non_retryable_status_routes_to_terminal() {
        let session = fake_token("v0");
        let snap = session.snapshot().await;
        let mut refreshed = false;
        for kind in [
            GrpcStatusKind::PermissionDenied,
            GrpcStatusKind::NotFound,
            GrpcStatusKind::InvalidArgument,
        ] {
            let out = classify_streaming_attempt(
                &session,
                &snap,
                &mut refreshed,
                "test_stream_endpoint",
                Err::<(), Error>(grpc(kind)),
            )
            .await;
            assert!(
                matches!(out, StreamingAttemptOutcome::Terminal(_)),
                "kind {kind:?} should route to Terminal"
            );
        }
    }

    #[tokio::test]
    async fn decode_failure_routes_to_terminal() {
        // Decode and decompress errors are payload-shape failures — they
        // cannot fix themselves on retry, so the streaming shell must
        // surface them immediately without backoff or refresh.
        let session = fake_token("v0");
        let snap = session.snapshot().await;
        let mut refreshed = false;
        let out = classify_streaming_attempt(
            &session,
            &snap,
            &mut refreshed,
            "test_stream_endpoint",
            Err::<(), Error>(Error::decode_codec("cell type mismatch")),
        )
        .await;
        assert!(matches!(out, StreamingAttemptOutcome::Terminal(_)));
        assert!(!refreshed, "decode terminal must not touch refresh budget");
    }
}

#[cfg(test)]
mod refresh_retry_disabled_tests {
    //! Regression tests for the auth-recovery contract: when
    //! `RetryPolicy::disabled()` is set (`max_attempts = 1`), a
    //! `NeedsRefresh` outcome must still trigger session refresh +
    //! one post-refresh re-attempt. Auth recovery is a separate
    //! contract from transient-retry policy.
    //!
    //! These tests exercise `run_unary_retry_loop` /
    //! `run_streaming_retry_loop` directly — the same helpers the
    //! `list_endpoint!` / `parsed_endpoint!` macros call. No replica
    //! of the loop algorithm lives in this module, so a change to the
    //! retry control-flow can never silently bypass test coverage.
    //!
    //! Per-attempt outcomes are driven by a FIFO queue inside the
    //! closure; `SessionToken` points at an unreachable Nexus URL and
    //! the driver bumps the token version on the first attempt so
    //! `session.refresh(&snap)` short-circuits on the dedup
    //! fast-path and returns Ok without HTTP.
    use super::{run_streaming_retry_loop, run_unary_retry_loop};
    use crate::auth::session::SessionToken;
    use crate::auth::Credentials;
    use crate::config::RetryPolicy;
    use crate::error::{Error, GrpcStatusKind};
    use std::cell::RefCell;

    fn fake_token(uuid: &str) -> SessionToken {
        SessionToken::new(
            uuid.to_string(),
            "https://nexus.example.invalid/auth".to_string(),
            Credentials::new("user@example.com", "hunter2"),
        )
    }

    fn grpc(kind: GrpcStatusKind) -> Error {
        Error::Grpc {
            kind,
            message: String::new(),
        }
    }

    /// Drive `run_unary_retry_loop` against a queue of canned
    /// outcomes and report the final result plus the attempt count.
    /// The closure pops one outcome per invocation — the loop's own
    /// scheduling decides how many it makes.
    ///
    /// After the first attempt, bump the session token version so
    /// `session.refresh(&first_snap)` short-circuits on the dedup
    /// fast-path (guard.version != first_snap.version) and returns
    /// Ok without making the Nexus HTTP round-trip.
    async fn drive_unary(
        session: &SessionToken,
        policy: &RetryPolicy,
        outcomes: Vec<Result<&'static str, Error>>,
    ) -> (Result<&'static str, Error>, u32) {
        let mut rev: Vec<_> = outcomes;
        rev.reverse();
        let outcomes = RefCell::new(rev);
        let attempts = RefCell::new(0u32);
        let result = run_unary_retry_loop(session, policy, "test_endpoint", |_snap| {
            let attempts_ref = &attempts;
            let outcomes_ref = &outcomes;
            async move {
                let n = {
                    let mut c = attempts_ref.borrow_mut();
                    *c += 1;
                    *c
                };
                // Stage the refresh dedup fast-path AFTER the first
                // snapshot is taken: bump guard.version so that when
                // classify_attempt → session.refresh(&snap) fires on
                // the first Unauthenticated outcome, version drift is
                // visible and refresh returns Ok without HTTP.
                if n == 1 {
                    session.bump_for_test("v-bumped").await;
                }
                outcomes_ref
                    .borrow_mut()
                    .pop()
                    .expect("test fed fewer outcomes than the loop drove")
            }
        })
        .await;
        let count = *attempts.borrow();
        (result, count)
    }

    /// Streaming sibling of [`drive_unary`].
    async fn drive_streaming(
        session: &SessionToken,
        policy: &RetryPolicy,
        outcomes: Vec<Result<(), Error>>,
    ) -> (Result<(), Error>, u32) {
        let mut rev: Vec<_> = outcomes;
        rev.reverse();
        let outcomes = RefCell::new(rev);
        let attempts = RefCell::new(0u32);
        let result = run_streaming_retry_loop(session, policy, "test_stream_endpoint", |_snap| {
            let attempts_ref = &attempts;
            let outcomes_ref = &outcomes;
            async move {
                let n = {
                    let mut c = attempts_ref.borrow_mut();
                    *c += 1;
                    *c
                };
                if n == 1 {
                    session.bump_for_test("v-bumped").await;
                }
                outcomes_ref
                    .borrow_mut()
                    .pop()
                    .expect("test fed fewer outcomes than the loop drove")
            }
        })
        .await;
        let count = *attempts.borrow();
        (result, count)
    }

    #[tokio::test]
    async fn disabled_policy_unary_refresh_then_retry_succeeds() {
        // Setup: token at v0, pre-bump to v1 so refresh fast-paths
        // without HTTP. First attempt returns NeedsRefresh; classify
        // calls refresh (fast-path Ok, refreshed_already flips); the
        // post-refresh grant fires ONE retry even though budget = 1.
        // Second attempt returns Ok("payload").
        let session = fake_token("v0");
        let policy = RetryPolicy::disabled();
        assert_eq!(policy.max_attempts, 1, "preconditions: budget=1");

        let (result, attempts) = drive_unary(
            &session,
            &policy,
            vec![Err(grpc(GrpcStatusKind::Unauthenticated)), Ok("payload")],
        )
        .await;

        assert_eq!(result.expect("post-refresh retry must succeed"), "payload");
        assert_eq!(
            attempts, 2,
            "loop must fire a second attempt after refresh even under RetryPolicy::disabled"
        );
    }

    #[tokio::test]
    async fn disabled_policy_unary_refresh_then_terminal_surfaces_second_err() {
        // After a successful refresh + retry, if the second attempt
        // also fails terminally, surface that terminal error
        // (NotFound here) — not a fabricated "retry exhausted" shape.
        let session = fake_token("v0");
        let policy = RetryPolicy::disabled();

        let (result, attempts) = drive_unary(
            &session,
            &policy,
            vec![
                Err(grpc(GrpcStatusKind::Unauthenticated)),
                Err(grpc(GrpcStatusKind::NotFound)),
            ],
        )
        .await;

        let err = result.expect_err("second attempt failed terminally");
        match err {
            Error::Grpc {
                kind: GrpcStatusKind::NotFound,
                ..
            } => {}
            other => panic!("expected NotFound on second attempt, got {other:?}"),
        }
        assert_eq!(attempts, 2);
    }

    #[tokio::test]
    async fn disabled_policy_unary_transient_does_not_get_extra_attempt() {
        // Control case: a transient (Unavailable) under
        // RetryPolicy::disabled() still surfaces after one attempt.
        // The post-refresh grant targets refresh recovery
        // specifically — it must NOT grant a free retry to bare
        // transients.
        let session = fake_token("v0");
        let policy = RetryPolicy::disabled();

        let (result, attempts) = drive_unary(
            &session,
            &policy,
            vec![Err(grpc(GrpcStatusKind::Unavailable))],
        )
        .await;

        assert!(matches!(
            result,
            Err(Error::Grpc {
                kind: GrpcStatusKind::Unavailable,
                ..
            })
        ));
        assert_eq!(
            attempts, 1,
            "disabled policy must NOT grant a free transient retry"
        );
    }

    #[tokio::test]
    async fn disabled_policy_unary_only_one_post_refresh_attempt() {
        // The refresh-recovery budget is one post-refresh attempt.
        // If the post-refresh attempt also returns NeedsRefresh,
        // classify_attempt surfaces it as Terminal
        // (refreshed_already is true), and the loop ends without a
        // third attempt.
        let session = fake_token("v0");
        let policy = RetryPolicy::disabled();

        let (result, attempts) = drive_unary(
            &session,
            &policy,
            vec![
                Err(grpc(GrpcStatusKind::Unauthenticated)),
                Err(grpc(GrpcStatusKind::Unauthenticated)),
            ],
        )
        .await;

        assert!(matches!(
            result,
            Err(Error::Grpc {
                kind: GrpcStatusKind::Unauthenticated,
                ..
            })
        ));
        assert_eq!(
            attempts, 2,
            "exactly one post-refresh attempt, then surface as terminal"
        );
    }

    #[tokio::test]
    async fn disabled_policy_streaming_refresh_then_done_succeeds() {
        // Streaming arm: same contract as unary. Disabled policy +
        // mid-stream Unauthenticated must refresh + restart from
        // chunk zero, and the restart must succeed (Done).
        let session = fake_token("v0");
        let policy = RetryPolicy::disabled();

        let (result, attempts) = drive_streaming(
            &session,
            &policy,
            vec![Err(grpc(GrpcStatusKind::Unauthenticated)), Ok(())],
        )
        .await;

        result.expect("post-refresh stream must complete");
        assert_eq!(attempts, 2);
    }

    #[tokio::test]
    async fn disabled_policy_streaming_transient_no_extra_attempt() {
        // Streaming arm: disabled policy + transient (Unavailable)
        // must surface after the first attempt, no free retry.
        let session = fake_token("v0");
        let policy = RetryPolicy::disabled();

        let (result, attempts) = drive_streaming(
            &session,
            &policy,
            vec![Err(grpc(GrpcStatusKind::Unavailable))],
        )
        .await;

        assert!(matches!(
            result,
            Err(Error::Grpc {
                kind: GrpcStatusKind::Unavailable,
                ..
            })
        ));
        assert_eq!(attempts, 1);
    }

    #[tokio::test]
    async fn default_policy_unary_refresh_does_not_consume_transient_budget() {
        // With max_attempts = 3, a NeedsRefresh on attempt 1 must
        // grant the refresh-retry attempt without burning a
        // transient budget slot. The post-refresh attempt
        // succeeds — final attempts = 2, not 3.
        let session = fake_token("v0");
        let policy = RetryPolicy {
            initial_delay: std::time::Duration::ZERO,
            max_delay: std::time::Duration::ZERO,
            max_attempts: 3,
            jitter: false,
        };

        let (result, attempts) = drive_unary(
            &session,
            &policy,
            vec![Err(grpc(GrpcStatusKind::Unauthenticated)), Ok("payload")],
        )
        .await;
        assert_eq!(result.expect("must succeed"), "payload");
        assert_eq!(attempts, 2);
    }
}
