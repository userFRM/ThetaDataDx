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
//! `Err(Error::Timeout { duration_ms })`. The `HistoricalClient` is unaffected;
//! a subsequent call on the same handle succeeds.
//!
//! List endpoints additionally expose a parallel `<name>_with_deadline(...)`
//! async method on `HistoricalClient`: the existing `pub async fn <name>(...)`
//! signatures stay non-breaking, while the `_with_deadline` variant gives
//! the same cancellation contract for the validator and registry dispatch.

/// Run a future with an optional per-call deadline.
///
/// When `deadline` is `None` the future is awaited verbatim. When `Some(d)`
/// the future is wrapped in [`tokio::time::timeout`]; on elapsed the future
/// is dropped and `Error::Timeout { duration_ms }` is returned. Local state
/// captured by the future (`_permit`, `tonic::Streaming`) drops with it.
///
/// # Errors
///
/// Returns `Error::Timeout` when the deadline elapses; otherwise propagates
/// whatever error the wrapped future resolves to.
/// Resolve the effective per-request deadline.
///
/// An explicit `with_deadline(...)` always wins, including
/// `with_deadline(Duration::ZERO)` which means "disable the deadline" and
/// opts the request out of any deadline. A zero `explicit` duration is
/// normalized to `None` here so the opt-out holds on every endpoint —
/// letting a zero `Duration` reach [`tokio::time::timeout`] would fire on
/// the first poll and time the call out instantly, the opposite of the
/// documented contract. When the caller set nothing (`explicit == None`),
/// fall back to the configured
/// [`crate::config::HistoricalConfig::request_timeout_secs`] default so a
/// server holding the stream open without sending chunks cannot hang the
/// request indefinitely.
///
/// A configured default of `0` does NOT disable the fallback: it is floored to
/// the production default here — the single
/// point every historical request routes through — so the gRPC hang guard
/// holds regardless of whether [`crate::config::DirectConfig::validate`] ran
/// on the config (the connect paths and the SDK bindings pass unvalidated
/// snapshots). The only way to run a request with no deadline is the explicit
/// per-call `with_deadline(Duration::ZERO)` opt-out above.
pub(crate) fn effective_deadline(
    explicit: Option<std::time::Duration>,
    default_secs: u64,
) -> Option<std::time::Duration> {
    match explicit {
        // An explicit zero is the deadline opt-out: normalize to "no
        // deadline" rather than a zero-length timeout that fires at once.
        Some(d) if d.is_zero() => None,
        Some(d) => Some(d),
        // Unset: fall back to the configured default, flooring a `0` (which
        // would otherwise disable the guard) to the terminal-safe default so a
        // silent-but-live server cannot hang a deadline-less request forever.
        None => {
            let secs = if default_secs == 0 {
                crate::config::DEFAULT_REQUEST_TIMEOUT_SECS
            } else {
                default_secs
            };
            Some(std::time::Duration::from_secs(secs))
        }
    }
}

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
/// avoiding the higher-ranked trait bounds a closure-based helper
/// would impose.
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
                    "endpoint" => endpoint
                )
                .increment(1);
                AttemptStep::Retry(err)
            }
            StatusClass::NeedsRefresh => {
                if *refreshed_already {
                    metrics::counter!(
                        "thetadatadx.grpc.errors",
                        "endpoint" => endpoint
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
                    "endpoint" => endpoint
                )
                .increment(1);
                AttemptStep::Terminal(err)
            }
        },
    }
}

/// Sleep between retry attempts according to the client's policy.
/// Split out of the macros so the per-endpoint expansion stays flat.
///
/// When the failed attempt carried a server-supplied
/// `google.rpc.RetryInfo` hint (surfaced as
/// `Error::Grpc { retry_after, .. }`), the sleep is raised to at least
/// that value — capped at the policy's `max_delay` ceiling — so a
/// server-instructed cooldown is honoured even when the client-side
/// schedule would have retried sooner, while a hostile hint cannot pin
/// a request permit for an unbounded sleep.
pub(crate) async fn sleep_for_retry(
    policy: &crate::config::RetryPolicy,
    attempt: u32,
    endpoint: &'static str,
    err: &crate::error::Error,
) {
    let mut delay = policy.delay_for_attempt(attempt);
    if let crate::error::Error::Grpc {
        retry_after: Some(hint),
        ..
    } = err
    {
        // Clamp the server hint to the policy ceiling: a hostile RetryInfo can
        // name up to `i64::MAX` seconds, which under a `with_deadline(ZERO)`
        // request would pin a semaphore permit for an unbounded sleep. The
        // client-side backoff already saturates at `max_delay`, so honour the
        // hint only up to that same cap.
        let hint = (*hint).min(policy.max_delay);
        if hint > delay {
            tracing::debug!(
                endpoint,
                hint_ms = hint.as_millis() as u64,
                "raising retry delay to server-supplied RetryInfo hint"
            );
            delay = hint;
        }
    }
    metrics::counter!(
        "thetadatadx.grpc.retries",
        "endpoint" => endpoint
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
/// refresh restarts the stream from chunk zero. This classifier only
/// resolves the refresh side-effect and reports the outcome; the replay
/// itself is gated by [`run_streaming_retry_loop`], which restarts only
/// while no chunk has yet reached the handler (`delivered` unset). Once
/// delivery has begun a refresh or transient is surfaced terminal instead
/// of replaying, so the chunk handler never sees a duplicated prefix.
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
                    "endpoint" => endpoint
                )
                .increment(1);
                StreamingAttemptOutcome::Backoff(err)
            }
            StatusClass::NeedsRefresh => {
                if *refreshed_already {
                    metrics::counter!(
                        "thetadatadx.grpc.errors",
                        "endpoint" => endpoint
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
                    "endpoint" => endpoint
                )
                .increment(1);
                StreamingAttemptOutcome::Terminal(err)
            }
        },
    }
}

/// Drive the unary endpoint retry / refresh loop.
///
/// Single source of truth for the retry / refresh control flow shared
/// by the endpoint macro arms. The closure receives the current session
/// snapshot and returns the per-attempt result; the helper handles
/// snapshotting, classification, refresh, backoff, and the
/// post-refresh re-attempt budget.
///
/// Auth recovery (session refresh) is intentionally independent of
/// `policy.max_attempts`: even with `RetryPolicy::disabled()`
/// (budget = 1), a single `Unauthenticated` triggers refresh + one
/// post-refresh re-attempt. Subsequent failures surface to the
/// caller.
///
/// # Errors
///
/// Returns the last attempt's [`crate::error::Error`] once the retry
/// budget or wall-clock envelope is exhausted, or a terminal error
/// (including a failed session refresh) surfaced unchanged.
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
    let started = std::time::Instant::now();
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
                // Two stop conditions bound the sequence: the attempt
                // budget and the wall-clock envelope. A just-completed
                // session refresh still earns its one post-refresh
                // re-attempt regardless of either budget.
                let budget_spent =
                    attempt >= budget || !policy.within_elapsed_budget(started.elapsed());
                if budget_spent && !can_post_refresh {
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
/// represents one full server-streaming attempt.
///
/// A restart replays the stream from chunk zero — MDDS has no resume
/// token — so it is only safe while *no* chunk of the failed attempt
/// reached the handler. The caller sets `delivered` the first time a
/// chunk is dispatched downstream; once it is set, a subsequent
/// transient or `Unauthenticated` is surfaced as terminal rather than
/// silently replaying, because the downstream (buffered collectors and
/// streaming callbacks alike) cannot dedup a re-sent prefix without a
/// resume cursor. A transient before the first chunk still retries.
///
/// # Errors
///
/// Returns the last attempt's [`crate::error::Error`] once the retry
/// budget or wall-clock envelope is exhausted, a mid-stream transient
/// after delivery began, or a terminal error (including a failed
/// session refresh) surfaced unchanged.
pub(crate) async fn run_streaming_retry_loop<F, Fut>(
    session: &crate::auth::SessionToken,
    policy: &crate::config::RetryPolicy,
    endpoint: &'static str,
    delivered: &std::sync::atomic::AtomicBool,
    mut attempt_fn: F,
) -> Result<(), crate::error::Error>
where
    F: FnMut(crate::auth::session::SessionSnapshot) -> Fut,
    Fut: std::future::Future<Output = Result<(), crate::error::Error>>,
{
    let budget = policy.max_attempts.max(1);
    let started = std::time::Instant::now();
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
                // Once a chunk has reached the handler a restart would
                // replay the delivered prefix (no resume token); surface
                // the error instead of duplicating rows downstream.
                if refresh_retry_used || delivered.load(std::sync::atomic::Ordering::Relaxed) {
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
                // A restart replays from chunk zero; if any chunk was
                // already delivered the replayed prefix would duplicate
                // downstream, so a mid-stream transient is terminal.
                if attempt >= budget
                    || !policy.within_elapsed_budget(started.elapsed())
                    || delivered.load(std::sync::atomic::Ordering::Relaxed)
                {
                    return Err(err);
                }
                sleep_for_retry(policy, attempt, endpoint, &err).await;
                attempt += 1;
            }
        }
    }
}

/// Decide whether the buffered `.await` path on a parsed builder
/// crossed the operator-visible warning threshold for response size.
///
/// Pure function — no side effects, no allocation, no I/O. Lets the
/// `parsed_endpoint!` macro keep the size check at the seam where
/// `row_count` and `size_of::<Item>` are both already in scope and
/// keeps the decision testable without a `tracing-subscriber` dep.
///
/// `row_size` is `size_of::<Tick>` at the call site — a lower bound
/// on the resident-memory cost since each tick may carry inline
/// `String`s + `Vec`s that allocate separately. The estimate is
/// intentionally conservative (under-counts heap-side allocations)
/// so the warn fires on the row-count axis we actually control;
/// callers tuning the threshold should think in "row count × wire
/// row size" terms rather than RSS.
///
/// Returns `Some(bytes_est)` when `bytes_est > threshold_bytes` and
/// `threshold_bytes > 0`. Returns `None` when the warn is disabled
/// (`threshold_bytes == 0`) or the response stayed under the
/// configured ceiling. The threshold check is strict `>` so the
/// caller can pin "exactly N bytes" silent in tests.
pub(crate) fn should_warn_buffered_size(
    row_count: usize,
    row_size: usize,
    threshold_bytes: usize,
) -> Option<usize> {
    // `0` is the documented "warn disabled" sentinel. Returning early
    // also keeps `saturating_mul` out of the hot path on the common
    // configuration.
    if threshold_bytes == 0 {
        return None;
    }
    // `saturating_mul` guards the (theoretical) overflow on a 32-bit
    // target — at 64-bit no realistic `row_count * row_size` reaches
    // `usize::MAX`, but the saturating path matches the rest of the
    // crate's arithmetic and costs one extra cmov.
    let bytes_est = row_count.saturating_mul(row_size);
    if bytes_est > threshold_bytes {
        Some(bytes_est)
    } else {
        None
    }
}

/// Emit a single `tracing::warn!` event when the buffered `.await`
/// path on a `parsed_endpoint!` builder crosses
/// `mdds.warn_on_buffered_threshold_bytes`.
///
/// Fires AT MOST ONCE per call — the macro invokes this helper
/// immediately after the `Vec<Tick>` materializes, before returning
/// to the caller, so a long-running operator workload sees exactly
/// one log line per offending request rather than a per-chunk
/// torrent. Threshold of `0` disables the warn entirely (see
/// [`crate::config::HistoricalConfig::warn_on_buffered_threshold_bytes`]).
pub(crate) fn warn_buffered_response_size(
    endpoint: &'static str,
    row_count: usize,
    row_size: usize,
    threshold_bytes: usize,
) {
    if let Some(bytes_est) = should_warn_buffered_size(row_count, row_size, threshold_bytes) {
        tracing::warn!(
            endpoint,
            row_count,
            bytes_est,
            threshold_bytes,
            "buffered .await returned a large response — consider .stream(handler) for this workload (see docs-site/docs/streaming/index.md)"
        );
    }
}

/// Classify an [`Error`] for retry / refresh routing.
///
/// The wire status converts once into the crate's own
/// [`crate::grpc::Status`] and reaches this classifier as
/// `Error::Grpc { kind: GrpcStatusKind::*, .. }`. We dispatch on the
/// typed `kind` so the retry classifier never parses status strings.
/// Other `Error` variants are terminal -- a `Decode` or `Decompress`
/// failure won't fix itself on retry.
///
/// `Error::Transport { kind: ConnectionClosed, .. }` covers every
/// connection-level h2 fault (GOAWAY, IO failure, peer shutdown,
/// open-phase drops; see
/// [`crate::grpc::ChannelError::ConnectionClosed`]). The underlying
/// stack reconnects lazily: the dead connection is replaced only when
/// the next RPC dispatches on that channel, so the Transient
/// classification here is what drives recovery — the retry shell
/// re-attempts on the next pool pick, which either lands on the same
/// channel (now dialing a fresh h2 session) or routes to a sibling
/// via the pool's load-balancing picker. Either way, a transient
/// connection blip on a long-running pool surfaces to the caller only
/// if the retry budget itself exhausts.
///
/// `Error::Transport { kind: H2StreamRefused, .. }` is the per-stream
/// `REFUSED_STREAM` reset: the server did not process the stream
/// (RFC 7540 § 8.1.4), so the RPC is safe to re-dispatch and is
/// classified Transient as well. A terminal per-stream reset
/// (`TransportErrorKind::H2Stream`) keeps its undefined outcome and
/// stays Terminal — only the not-processed reason is retried.
fn classify_error(err: &crate::error::Error) -> StatusClass {
    use crate::error::{GrpcStatusKind, TransportErrorKind};
    match err {
        crate::error::Error::Grpc { kind, .. } => match kind {
            GrpcStatusKind::Unavailable
            | GrpcStatusKind::DeadlineExceeded
            | GrpcStatusKind::ResourceExhausted => StatusClass::Transient,
            GrpcStatusKind::Unauthenticated => StatusClass::NeedsRefresh,
            _ => StatusClass::Terminal,
        },
        crate::error::Error::Transport {
            kind: TransportErrorKind::ConnectionClosed | TransportErrorKind::H2StreamRefused,
            ..
        } => StatusClass::Transient,
        _ => StatusClass::Terminal,
    }
}

/// Generate a list endpoint that returns `Vec<String>` by extracting a text
/// column from the response `DataTable`.
///
/// Pattern: build request -> gRPC call -> collect stream -> extract text column.
/// Emits two methods on `HistoricalClient`, matching the deadline contract
/// of the builder endpoints:
/// - `pub async fn <name>(...)` — bounded by the configured
///   [`crate::config::HistoricalConfig::request_timeout_secs`] default so a
///   live-but-silent stream cannot hang the request forever.
/// - `pub async fn <name>_with_deadline(..., deadline: Duration)` — same
///   call with an explicit per-call deadline. `Duration::ZERO` opts out of
///   any deadline (see [`effective_deadline`]).
///
/// Both route through [`run_with_optional_deadline`], so the in-flight gRPC
/// call (`<grpc>` + `collect_stream`) is dropped on expiry — the `_permit`
/// releases its semaphore slot and the `tonic::Streaming` is dropped
/// (RST_STREAM), returning `Err(Error::Timeout)`.
macro_rules! list_endpoint {
    (
        $(#[$meta:meta])*
        fn $name:ident( $($arg:ident : $arg_ty:ty),* ) -> $col:literal;
        with_deadline_fn: $with_deadline:ident;
        grpc: $grpc:ident;
        request: $req:ident;
        query: $query:ident { $($field:ident : $val:expr),* $(,)? };
    ) => {
        #[allow(clippy::too_many_arguments)] // Reason: ThetaData endpoints require many parameters (symbol, date, strike, exp, right, etc.).
        $(#[$meta])*
        /// # Errors
        ///
        /// Returns an error on network, authentication, or parsing failure.
        /// Returns [`Error::Timeout`] when the configured request deadline
        /// elapses before the response completes.
        pub async fn $name(&self, $($arg : $arg_ty),*) -> Result<Vec<String>, Error> {
            // No explicit deadline: fall back to the configured default so
            // the request is always bounded.
            let deadline = $crate::mdds::macros::effective_deadline(
                None,
                self.config().historical.request_timeout_secs,
            );
            list_endpoint_impl_body!(
                self, deadline, $name, $grpc, $req,
                $query { $($field : $val),* }, $col
            )
        }

        #[allow(clippy::too_many_arguments)] // Reason: ThetaData endpoints require many parameters (symbol, date, strike, exp, right, etc.).
        $(#[$meta])*
        /// Same as the deadline-free variant, but bounds the call with an
        /// explicit per-call deadline. `Duration::ZERO` opts out of any
        /// deadline.
        ///
        /// # Errors
        ///
        /// Returns an error on network, authentication, or parsing failure.
        /// Returns [`Error::Timeout`] when `deadline` elapses first.
        pub async fn $with_deadline(
            &self,
            $($arg : $arg_ty,)*
            deadline: std::time::Duration,
        ) -> Result<Vec<String>, Error> {
            let deadline = $crate::mdds::macros::effective_deadline(
                Some(deadline),
                self.config().historical.request_timeout_secs,
            );
            list_endpoint_impl_body!(
                self, deadline, $name, $grpc, $req,
                $query { $($field : $val),* }, $col
            )
        }
    };
}

/// Shared request/collect body for the [`list_endpoint!`] pair (`<name>` /
/// `<name>_with_deadline`). Defined once so the deadline-bounded
/// request/collect path is written a single time; the two public methods
/// differ only in how they resolve the `deadline` they pass in.
///
/// `macro_rules!` cannot synthesize a private impl-method name (no `paste`
/// dependency), so rather than a shared method this is a shared macro: each
/// public method expands it inline with the already-resolved `deadline` and
/// the per-endpoint specifics (receiver, gRPC stub, request/query types,
/// fields, column).
macro_rules! list_endpoint_impl_body {
    (
        $client:expr, $deadline:ident, $name:ident, $grpc:ident, $req:ident,
        $query:ident { $($field:ident : $val:expr),* $(,)? }, $col:literal
    ) => {{
        // `&HistoricalClient` is `Copy`, so bind the receiver once and reuse
        // it across the (possibly retried) request closure. Passing `self`
        // through a macro parameter is required: a `self` token synthesized
        // by the macro body cannot reach the caller's `self`.
        let client: &HistoricalClient = $client;
        $crate::mdds::macros::run_with_optional_deadline($deadline, async move {
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
                |snap| async move {
                    let qi = client.build_query_info(snap.uuid.clone());
                    let request = proto::$req {
                        query_info: Some(qi),
                        params: Some(proto::$query { $($field : $val),* }),
                    };
                    // Bind the lease to a local so it lives across
                    // the await — the pre-dispatch reservation
                    // must outlive `server_streaming` so the picker
                    // counts pending opens correctly under burst
                    // contention. Deref coercion from `&ChannelLease`
                    // to `&Channel` satisfies the generated stub
                    // signature.
                    let lease = client.channel();
                    let stream = $crate::proto::beta_theta_terminal::$grpc(
                        &lease,
                        request,
                    )
                    .await
                    .map_err(|e| -> Error { e.into() })?;
                    client.collect_stream(stream).await
                },
            ).await?;
            metrics::histogram!("thetadatadx.grpc.latency_ms", "endpoint" => stringify!($name))
                .record(_metrics_start.elapsed().as_secs_f64() * 1_000.0);
            // List returns are sorted ascending (numeric-aware for
            // strike / date lists) — the wire order is unspecified.
            Ok(decode::sorted_list_values(
                decode::extract_text_column(&table, $col)
                    .into_iter()
                    .flatten()
                    .collect(),
            ))
        }).await
    }};
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
/// // doc-test to spin up an authenticated `HistoricalClient` without
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
        /// Builder for the [`HistoricalClient::$name`] endpoint.
        pub struct $builder_name<'a> {
            client: &'a HistoricalClient,
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
            /// underlying `HistoricalClient` is unaffected; subsequent calls
            /// on the same handle succeed.
            ///
            /// `Duration::ZERO` means "disable the deadline": it opts the
            /// call out of any bound, including the configured
            /// `request_timeout_secs` default. The explicit zero is carried
            /// through to [`effective_deadline`], which normalizes it to "no
            /// deadline" — leaving it to fall back to the configured default
            /// (the behaviour of an unset deadline) would make the opt-out
            /// silently impose a bound. Pass a positive `Duration` (e.g.
            /// `Duration::from_millis(1)`) for a near-instant expiration.
            #[must_use]
            pub fn with_deadline(mut self, duration: std::time::Duration) -> Self {
                // Carry the value through verbatim — including `ZERO`. A
                // bare `None` here means "caller set nothing" and routes to
                // the configured default; an explicit `ZERO` must instead
                // disable the deadline, which `effective_deadline` resolves.
                self.deadline = Some(duration);
                self
            }

            /// Stream the response chunk-by-chunk via `handler`, never
            /// materializing the full `Vec<T>`.
            ///
            /// The buffered `.await -> Vec<T>` path holds three live
            /// copies (h2 frames + concatenated proto payload +
            /// decoded `Vec<T>`) plus a `Vec::push` doubling
            /// transient. The `.stream()` variant decodes one chunk
            /// at a time, hands the slice to `handler`, then drops
            /// the chunk before the next is fetched — bounded peak
            /// memory regardless of response size.
            ///
            /// # Retry / refresh semantics
            ///
            /// Same shell as the buffered path: transient gRPC
            /// statuses (`Unavailable`, `DeadlineExceeded`,
            /// `ResourceExhausted`) and mid-stream `Unauthenticated`
            /// trigger backoff / refresh + restart from chunk zero
            /// (upstream MDDS has no resume token) ONLY while no chunk
            /// has yet reached `handler`. Once the first chunk is
            /// dispatched, a later transient is surfaced as an error
            /// rather than replaying the delivered prefix, so `handler`
            /// never sees a duplicated chunk.
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
            pub async fn stream<F>(self, handler: F) -> Result<(), Error>
            where
                F: FnMut(&[$item]) + Send,
            {
                let $builder_name {
                    client,
                    $($req_arg,)*
                    $($opt_name,)*
                    deadline,
                } = self;
                let _ = &client;
                $($($crate::mdds::validate::validate_date_required(&$date_arg)?;)+)?
                let deadline = $crate::mdds::macros::effective_deadline(
                    deadline,
                    client.config().historical.request_timeout_secs,
                );
                $crate::mdds::macros::run_with_optional_deadline(deadline, async move {
                    tracing::debug!(endpoint = stringify!($name), "gRPC streaming request");
                    metrics::counter!("thetadatadx.grpc.requests", "endpoint" => stringify!($name)).increment(1);
                    let _metrics_start = std::time::Instant::now();
                    let _permit = client.request_semaphore.acquire().await
                        .map_err(|_| Error::config_internal("request semaphore closed"))?;
                    let policy = client.config().retry;
                    // The user handler is `FnMut + Send`; wrap it in a
                    // `Mutex` so the per-attempt closure passed to
                    // `run_streaming_retry_loop` can acquire a unique
                    // mutable borrow on each invocation without
                    // capturing a `&mut` whose lifetime would escape
                    // the closure body. `Mutex<F>` where `F: Send` is
                    // `Send + Sync`, so the future stays Send (the
                    // Python SDK's `spawn_awaitable` requires this).
                    let handler_mutex = std::sync::Mutex::new(handler);
                    let handler_mutex = &handler_mutex;
                    // Set once a chunk reaches `handler`: it makes a later
                    // transient terminal so the no-resume restart never
                    // replays an already-delivered prefix.
                    let delivered = std::sync::atomic::AtomicBool::new(false);
                    let delivered = &delivered;
                    $crate::mdds::macros::run_streaming_retry_loop(
                        client.session(),
                        &policy,
                        stringify!($name),
                        delivered,
                        move |snap| {
                            // Clone per-attempt: the FnMut closure may
                            // be invoked twice (post-refresh restart),
                            // and the proto request takes ownership of
                            // the param values, so the owned bindings
                            // must outlive the loop and clone fresh on
                            // each iteration.
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
                                let mut decode_error: Option<Error> = None;
                                let drain_result = client.for_each_chunk_control(stream, |_headers, rows| {
                                    if decode_error.is_some() {
                                        return std::ops::ControlFlow::Break(());
                                    }
                                    let chunk_table = proto::DataTable {
                                        headers: _headers.to_vec(),
                                        data_table: rows.to_vec(),
                                    };
                                    match $parser(&chunk_table) {
                                        Ok(ticks) => {
                                            // This terminal's handler takes `&[$item]`
                                            // (raw rows), so it never reads column
                                            // presence — compute none here. The
                                            // presence-carrying `Ticks` wrap lives only
                                            // in `stream_ticks`, whose handler reads it;
                                            // computing it here would be per-chunk dead
                                            // work on the streaming hot path.
                                            // Mutex is single-threaded
                                            // in practice (one call
                                            // chain at a time); a
                                            // poisoned mutex here only
                                            // surfaces if `for_each_chunk`
                                            // panicked mid-callback,
                                            // which would already be
                                            // a hard error path.
                                            if let Ok(mut h) = handler_mutex.lock() {
                                                (*h)(&ticks);
                                            }
                                            // Only mark the stream as delivered
                                            // once a chunk carried rows: an empty
                                            // chunk (headers-only keepalive /
                                            // terminator) hands the downstream no
                                            // rows, so a refresh replay from chunk
                                            // zero would duplicate nothing. Gating
                                            // here keeps a recoverable
                                            // `Unauthenticated` after only empty
                                            // chunks from being forced terminal.
                                            if !ticks.is_empty() {
                                                delivered.store(true, std::sync::atomic::Ordering::Relaxed);
                                            }
                                            std::ops::ControlFlow::Continue(())
                                        }
                                        Err(e) => {
                                            decode_error = Some(Error::from(e));
                                            std::ops::ControlFlow::Break(())
                                        }
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

            #[doc(hidden)]
            pub async fn stream_ticks<F>(self, handler: F) -> Result<(), Error>
            where
                F: FnMut($crate::columns::Ticks<$item>) + Send,
            {
                let $builder_name {
                    client,
                    $($req_arg,)*
                    $($opt_name,)*
                    deadline,
                } = self;
                let _ = &client;
                $($($crate::mdds::validate::validate_date_required(&$date_arg)?;)+)?
                let deadline = $crate::mdds::macros::effective_deadline(
                    deadline,
                    client.config().historical.request_timeout_secs,
                );
                $crate::mdds::macros::run_with_optional_deadline(deadline, async move {
                    tracing::debug!(endpoint = stringify!($name), "gRPC streaming request");
                    metrics::counter!("thetadatadx.grpc.requests", "endpoint" => stringify!($name)).increment(1);
                    let _metrics_start = std::time::Instant::now();
                    let _permit = client.request_semaphore.acquire().await
                        .map_err(|_| Error::config_internal("request semaphore closed"))?;
                    let policy = client.config().retry;
                    let handler_mutex = std::sync::Mutex::new(handler);
                    let handler_mutex = &handler_mutex;
                    let delivered = std::sync::atomic::AtomicBool::new(false);
                    let delivered = &delivered;
                    $crate::mdds::macros::run_streaming_retry_loop(
                        client.session(),
                        &policy,
                        stringify!($name),
                        delivered,
                        move |snap| {
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
                                let mut decode_error: Option<Error> = None;
                                let drain_result = client.for_each_chunk_control(stream, |_headers, rows| {
                                    if decode_error.is_some() {
                                        return std::ops::ControlFlow::Break(());
                                    }
                                    let chunk_table = proto::DataTable {
                                        headers: _headers.to_vec(),
                                        data_table: rows.to_vec(),
                                    };
                                    match $parser(&chunk_table) {
                                        Ok(rows) => {
                                            let columns = $crate::mdds::stream::chunk_columns::<$item>(&chunk_table);
                                            let ticks = $crate::columns::Ticks::new(rows, columns);
                                            let delivered_nonempty = !ticks.is_empty();
                                            if let Ok(mut h) = handler_mutex.lock() {
                                                (*h)(ticks);
                                            }
                                            if delivered_nonempty {
                                                delivered.store(true, std::sync::atomic::Ordering::Relaxed);
                                            }
                                            std::ops::ControlFlow::Continue(())
                                        }
                                        Err(e) => {
                                            decode_error = Some(Error::from(e));
                                            std::ops::ControlFlow::Break(())
                                        }
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

            /// Async twin of [`stream`](Self::stream): the per-chunk
            /// `handler` returns a future that is awaited before the next
            /// chunk is fetched.
            ///
            /// Identical retry / refresh / no-resume-replay semantics and
            /// identical bounded-peak-memory behaviour as [`stream`]; the
            /// only difference is that the handler is `async`. Awaiting the
            /// handler future in-line preserves once-per-chunk, in-order
            /// delivery and the chunk-freed-before-next backpressure. The
            /// Python SDK's `*_stream_async` terminal uses this to offload
            /// its GIL-bound user handler onto a blocking-pool task without
            /// parking a shared async worker.
            ///
            /// # Errors
            ///
            /// Same as [`stream`](Self::stream).
            pub async fn stream_async<F, HFut>(self, handler: F) -> Result<(), Error>
            where
                F: FnMut(&[$item]) -> HFut + Send,
                HFut: std::future::Future<Output = ()> + Send,
            {
                let $builder_name {
                    client,
                    $($req_arg,)*
                    $($opt_name,)*
                    deadline,
                } = self;
                let _ = &client;
                $($($crate::mdds::validate::validate_date_required(&$date_arg)?;)+)?
                let deadline = $crate::mdds::macros::effective_deadline(
                    deadline,
                    client.config().historical.request_timeout_secs,
                );
                $crate::mdds::macros::run_with_optional_deadline(deadline, async move {
                    tracing::debug!(endpoint = stringify!($name), "gRPC streaming request");
                    metrics::counter!("thetadatadx.grpc.requests", "endpoint" => stringify!($name)).increment(1);
                    let _metrics_start = std::time::Instant::now();
                    let _permit = client.request_semaphore.acquire().await
                        .map_err(|_| Error::config_internal("request semaphore closed"))?;
                    let policy = client.config().retry;
                    // Same `Mutex<F>` wrap as the sync path: the per-attempt
                    // closure needs a unique mutable borrow on each call, and
                    // `Mutex<F>` where `F: Send` is `Send + Sync` so the
                    // spawned future stays Send (the Python SDK's
                    // `spawn_awaitable` requires this).
                    let handler_mutex = std::sync::Mutex::new(handler);
                    let handler_mutex = &handler_mutex;
                    let delivered = std::sync::atomic::AtomicBool::new(false);
                    let delivered = &delivered;
                    $crate::mdds::macros::run_streaming_retry_loop(
                        client.session(),
                        &policy,
                        stringify!($name),
                        delivered,
                        move |snap| {
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
                                let mut decode_error: Option<Error> = None;
                                let drain_result = client.for_each_chunk_async_control(stream, |headers, rows| {
                                    // Synchronous section: parse the chunk and
                                    // build the owned user future while holding
                                    // the handler lock, then release the lock
                                    // before the future is awaited. The chunk is
                                    // handed in by value, so nothing borrowed
                                    // from it can be held across the await.
                                    let mut stop_stream = decode_error.is_some();
                                    let user_fut = if stop_stream {
                                        None
                                    } else {
                                        let chunk_table = proto::DataTable {
                                            headers,
                                            data_table: rows,
                                        };
                                        match $parser(&chunk_table) {
                                            Ok(ticks) => {
                                                // `&[$item]` handler — no presence read,
                                                // so skip the per-chunk `chunk_columns`
                                                // scan (dead work on the hot path). The
                                                // `Ticks` wrap lives in `stream_ticks_async`.
                                                let fut = handler_mutex
                                                    .lock()
                                                    .ok()
                                                    .map(|mut h| (*h)(&ticks));
                                                // Mark delivered once the handler
                                                // has taken a non-empty chunk, so a
                                                // later transient cannot replay an
                                                // already-delivered prefix. Empty
                                                // chunks (headers-only keepalive)
                                                // hand the handler no rows, so a
                                                // replay would duplicate nothing.
                                                if !ticks.is_empty() {
                                                    delivered.store(true, std::sync::atomic::Ordering::Relaxed);
                                                }
                                                fut
                                            }
                                            Err(e) => {
                                                decode_error = Some(Error::from(e));
                                                stop_stream = true;
                                                None
                                            }
                                        }
                                    };
                                    async move {
                                        if let Some(user_fut) = user_fut {
                                            user_fut.await;
                                        }
                                        if stop_stream {
                                            std::ops::ControlFlow::Break(())
                                        } else {
                                            std::ops::ControlFlow::Continue(())
                                        }
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

            #[doc(hidden)]
            pub async fn stream_ticks_async<F, HFut>(self, handler: F) -> Result<(), Error>
            where
                F: FnMut($crate::columns::Ticks<$item>) -> HFut + Send,
                HFut: std::future::Future<Output = ()> + Send,
            {
                let $builder_name {
                    client,
                    $($req_arg,)*
                    $($opt_name,)*
                    deadline,
                } = self;
                let _ = &client;
                $($($crate::mdds::validate::validate_date_required(&$date_arg)?;)+)?
                let deadline = $crate::mdds::macros::effective_deadline(
                    deadline,
                    client.config().historical.request_timeout_secs,
                );
                $crate::mdds::macros::run_with_optional_deadline(deadline, async move {
                    tracing::debug!(endpoint = stringify!($name), "gRPC streaming request");
                    metrics::counter!("thetadatadx.grpc.requests", "endpoint" => stringify!($name)).increment(1);
                    let _metrics_start = std::time::Instant::now();
                    let _permit = client.request_semaphore.acquire().await
                        .map_err(|_| Error::config_internal("request semaphore closed"))?;
                    let policy = client.config().retry;
                    let handler_mutex = std::sync::Mutex::new(handler);
                    let handler_mutex = &handler_mutex;
                    let delivered = std::sync::atomic::AtomicBool::new(false);
                    let delivered = &delivered;
                    $crate::mdds::macros::run_streaming_retry_loop(
                        client.session(),
                        &policy,
                        stringify!($name),
                        delivered,
                        move |snap| {
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
                                let mut decode_error: Option<Error> = None;
                                let drain_result = client.for_each_chunk_async_control(stream, |headers, rows| {
                                    let mut stop_stream = decode_error.is_some();
                                    let user_fut = if stop_stream {
                                        None
                                    } else {
                                        let chunk_table = proto::DataTable {
                                            headers,
                                            data_table: rows,
                                        };
                                        match $parser(&chunk_table) {
                                            Ok(rows) => {
                                                let columns = $crate::mdds::stream::chunk_columns::<$item>(&chunk_table);
                                                let ticks = $crate::columns::Ticks::new(rows, columns);
                                                let delivered_nonempty = !ticks.is_empty();
                                                let fut = handler_mutex
                                                    .lock()
                                                    .ok()
                                                    .map(|mut h| (*h)(ticks));
                                                if delivered_nonempty {
                                                    delivered.store(true, std::sync::atomic::Ordering::Relaxed);
                                                }
                                                fut
                                            }
                                            Err(e) => {
                                                decode_error = Some(Error::from(e));
                                                stop_stream = true;
                                                None
                                            }
                                        }
                                    };
                                    async move {
                                        if let Some(user_fut) = user_fut {
                                            user_fut.await;
                                        }
                                        if stop_stream {
                                            std::ops::ControlFlow::Break(())
                                        } else {
                                            std::ops::ControlFlow::Continue(())
                                        }
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
                        let parsed = $parser(&table).map_err(Error::from)?;
                        // Capture which columns this response's wire carried,
                        // resolved through the same alias-aware lookup the
                        // parser used, so the buffered `Ticks` projects to the
                        // terminal's exact column set at its DataFrame terminal.
                        let present_headers: Vec<&str> =
                            table.headers.iter().map(String::as_str).collect();
                        let columns = <$item as $crate::columns::WireColumns>::present_columns(
                            &present_headers,
                        );
                        // Carry the response's `symbol` (root) so the projected
                        // frame emits it as the leading column: option/index and
                        // single-symbol snapshots send a constant broadcast; a
                        // multi-symbol snapshot sends a per-row `symbol` column
                        // that attributes each row to its underlying; stock
                        // history sends neither.
                        use $crate::mdds::decode::extract::ResponseSymbol;
                        // A tick that already owns a `symbol` column (the
                        // contract universe) emits it from its own field, so skip
                        // the root classifier entirely: for a varying-root
                        // universe it would collect one `Box<str>` per row that no
                        // builder reads. Only the flat POD ticks, which carry no
                        // `symbol` field, run it and attach the result.
                        let columns = if columns.contains("symbol") {
                            columns
                        } else {
                            match $crate::mdds::decode::extract::response_symbol(&table) {
                                ResponseSymbol::Constant(symbol) => columns.with_symbol(symbol),
                                ResponseSymbol::PerRow(symbols) => columns.with_symbols(symbols),
                                ResponseSymbol::Absent => columns,
                            }
                        };
                        // Surface the wrong-API-for-this-workload
                        // signal exactly once per request, after the buffered
                        // `Vec` materialized — before this point the row count
                        // is unknown, after this point the caller has
                        // already paid the buffered cost. `row_count` is the
                        // length the caller is about to receive; `row_size`
                        // is the wire-shape lower bound (`size_of::<Item>`).
                        // Configurable via
                        // `DirectConfig::historical.warn_on_buffered_threshold_bytes`;
                        // set to 0 to disable.
                        let threshold = client
                            .config()
                            .historical
                            .warn_on_buffered_threshold_bytes;
                        $crate::mdds::macros::warn_buffered_response_size(
                            stringify!($name),
                            parsed.len(),
                            std::mem::size_of::<$item>(),
                            threshold,
                        );
                        Ok($crate::columns::Ticks::new(parsed, columns))
                    };
                    let deadline = $crate::mdds::macros::effective_deadline(
                        deadline,
                        client.config().historical.request_timeout_secs,
                    );
                    $crate::mdds::macros::run_with_optional_deadline(deadline, inner).await
                })
            }
        }

        impl HistoricalClient {
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
            retry_after: None,
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
            classify_error(&Error::config_invalid("historical.endpoint", "bad config")),
            StatusClass::Terminal
        );
        assert_eq!(
            classify_error(&Error::decode_codec("parse fail")),
            StatusClass::Terminal
        );
    }

    /// Connection-level transport errors
    /// (`Error::Transport { kind: ConnectionClosed, .. }`) must
    /// classify as Transient so the retry shell re-attempts the RPC.
    /// Combined with the channel's in-place reconnect of its inner
    /// `SendRequest<Bytes>`, the retry lands either on the same
    /// channel (post-swap) or on a sibling pool member and the call
    /// succeeds. If this test flips back to Terminal a future
    /// contributor has re-broken the long-running-pool recovery —
    /// every GOAWAY / network blip would terminate the user-facing
    /// call instead of healing transparently.
    #[test]
    fn connection_closed_transport_error_maps_to_transient() {
        use crate::error::TransportErrorKind;
        let err = Error::Transport {
            kind: TransportErrorKind::ConnectionClosed,
            message: "h2 connection closed".to_string(),
        };
        assert_eq!(classify_error(&err), StatusClass::Transient);
    }

    /// A `REFUSED_STREAM` per-stream reset is retry-safe: the server did
    /// not process the stream, so the retry shell must re-dispatch it on
    /// the next pool pick rather than surfacing it as a terminal user
    /// error. If this flips to Terminal a future contributor has
    /// re-broken the not-processed-reset recovery.
    #[test]
    fn refused_stream_transport_error_maps_to_transient() {
        use crate::error::TransportErrorKind;
        let err = Error::Transport {
            kind: TransportErrorKind::H2StreamRefused,
            message: "h2 stream refused: REFUSED_STREAM".to_string(),
        };
        assert_eq!(classify_error(&err), StatusClass::Transient);
    }

    /// Companion: other transport errors stay terminal. A genuine
    /// TLS / DNS / Codec failure won't fix itself on retry, so the
    /// retry shell must propagate. A terminal per-stream reset
    /// (`H2Stream`, e.g. `CANCEL` / `INTERNAL_ERROR`) keeps its
    /// undefined outcome and stays terminal — only the not-processed
    /// `H2StreamRefused` reset above is retried. Pin every variant
    /// explicitly so a future `TransportErrorKind` addition cannot
    /// accidentally inherit the `Transient` classification.
    #[test]
    fn other_transport_error_kinds_stay_terminal() {
        use crate::error::TransportErrorKind;
        let kinds = [
            TransportErrorKind::Tcp,
            TransportErrorKind::Tls,
            TransportErrorKind::InvalidServerName,
            TransportErrorKind::H2Handshake,
            TransportErrorKind::H2Stream,
            TransportErrorKind::InvalidPath,
        ];
        for kind in kinds {
            let err = Error::Transport {
                kind,
                message: String::new(),
            };
            assert_eq!(
                classify_error(&err),
                StatusClass::Terminal,
                "TransportErrorKind::{kind:?} must stay terminal"
            );
        }
    }
}

#[cfg(test)]
mod effective_deadline_tests {
    use super::{effective_deadline, run_with_optional_deadline};
    use std::time::Duration;

    /// With no explicit `with_deadline(...)`, the configured default is
    /// applied. This is the load-bearing assertion that a request which
    /// set no deadline still gets one, so a server holding the stream
    /// open without sending chunks cannot hang the call forever.
    #[test]
    fn applies_configured_default_when_caller_set_none() {
        assert_eq!(
            effective_deadline(None, 300),
            Some(Duration::from_secs(300))
        );
    }

    /// An explicit deadline always wins over the configured default.
    #[test]
    fn explicit_deadline_overrides_default() {
        assert_eq!(
            effective_deadline(Some(Duration::from_secs(5)), 300),
            Some(Duration::from_secs(5))
        );
    }

    /// A configured default of `0` is FLOORED to the production default
    /// rather than disabling the guard: a deadline-less request must never be
    /// left unbounded just because the config carried a `0` (validated or
    /// not). This is the single consumption-point floor — the only way to run
    /// a request with no deadline is the explicit `with_deadline(Duration::ZERO)`
    /// opt-out (covered separately), NOT a `0` in the config.
    #[test]
    fn zero_default_is_floored_to_the_production_default() {
        assert_eq!(
            effective_deadline(None, 0),
            Some(Duration::from_secs(
                crate::config::DEFAULT_REQUEST_TIMEOUT_SECS
            )),
        );
    }

    /// The production default seeds a positive per-request deadline, so
    /// the historical request path is bounded out of the box.
    #[test]
    fn production_default_is_positive() {
        let cfg = crate::config::HistoricalConfig::production_defaults();
        assert!(cfg.request_timeout_secs > 0);
        assert_eq!(
            effective_deadline(None, cfg.request_timeout_secs),
            Some(Duration::from_secs(cfg.request_timeout_secs))
        );
    }

    /// An explicit `Duration::ZERO` is the deadline opt-out and must
    /// normalize to `None` even when a positive configured default is in
    /// force — the explicit zero wins over the fallback. Letting the zero
    /// flow through would wrap the call in `timeout(ZERO, ..)` and fire on
    /// the first poll, breaking the advertised opt-out on every list
    /// endpoint.
    #[test]
    fn explicit_zero_disables_deadline_over_configured_default() {
        assert_eq!(effective_deadline(Some(Duration::ZERO), 300), None);
    }

    /// End-to-end guard for the list-endpoint deadline contract: the
    /// `_with_deadline` arm resolves its deadline via `effective_deadline`,
    /// so an explicit `Duration::ZERO` must yield `None` and the wrapped
    /// future must run to completion rather than timing out on the first
    /// poll. Drive a future that only resolves after a real `await` point
    /// to prove the opt-out actually disables the timeout.
    #[tokio::test]
    async fn explicit_zero_deadline_runs_to_completion() {
        let deadline = effective_deadline(Some(Duration::ZERO), 300);
        assert_eq!(deadline, None, "explicit zero must disable the deadline");
        let out: Result<u8, crate::error::Error> = run_with_optional_deadline(deadline, async {
            // Yield once so a zero-length timeout would have a poll to
            // fire on before this resolves.
            tokio::task::yield_now().await;
            Ok(7)
        })
        .await;
        assert!(
            matches!(out, Ok(7)),
            "an explicit-zero deadline must let the call complete, got {out:?}"
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
    use crate::config::HistoricalEnvironment;
    use crate::error::{Error, GrpcStatusKind};

    fn fake_token(uuid: &str) -> SessionToken {
        SessionToken::new(
            uuid.to_string(),
            "https://nexus.example.invalid/auth".to_string(),
            HistoricalEnvironment::Prod,
            Credentials::new("user@example.com", "hunter2"),
        )
    }

    fn grpc(kind: GrpcStatusKind) -> Error {
        Error::Grpc {
            kind,
            message: String::new(),
            retry_after: None,
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
    use crate::config::{HistoricalEnvironment, RetryPolicy};
    use crate::error::{Error, GrpcStatusKind};
    use std::cell::RefCell;

    fn fake_token(uuid: &str) -> SessionToken {
        SessionToken::new(
            uuid.to_string(),
            "https://nexus.example.invalid/auth".to_string(),
            HistoricalEnvironment::Prod,
            Credentials::new("user@example.com", "hunter2"),
        )
    }

    fn grpc(kind: GrpcStatusKind) -> Error {
        Error::Grpc {
            kind,
            message: String::new(),
            retry_after: None,
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
    ///
    /// `deliver_on` names the attempt numbers (1-based) that mark a
    /// chunk as delivered downstream before returning their outcome —
    /// the test stand-in for `for_each_chunk` handing a chunk to the
    /// handler. That flips `delivered`, which makes a same-attempt
    /// transient / refresh terminal in the loop.
    async fn drive_streaming(
        session: &SessionToken,
        policy: &RetryPolicy,
        outcomes: Vec<Result<(), Error>>,
        deliver_on: &[u32],
    ) -> (Result<(), Error>, u32) {
        let mut rev: Vec<_> = outcomes;
        rev.reverse();
        let outcomes = RefCell::new(rev);
        let attempts = RefCell::new(0u32);
        let delivered = std::sync::atomic::AtomicBool::new(false);
        let result = run_streaming_retry_loop(
            session,
            policy,
            "test_stream_endpoint",
            &delivered,
            |_snap| {
                let attempts_ref = &attempts;
                let outcomes_ref = &outcomes;
                let delivered_ref = &delivered;
                async move {
                    let n = {
                        let mut c = attempts_ref.borrow_mut();
                        *c += 1;
                        *c
                    };
                    if n == 1 {
                        session.bump_for_test("v-bumped").await;
                    }
                    if deliver_on.contains(&n) {
                        delivered_ref.store(true, std::sync::atomic::Ordering::Relaxed);
                    }
                    outcomes_ref
                        .borrow_mut()
                        .pop()
                        .expect("test fed fewer outcomes than the loop drove")
                }
            },
        )
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
            &[],
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
            &[],
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
    async fn streaming_transient_before_first_chunk_still_retries() {
        // Regression guard for the mid-stream fix: a transient BEFORE
        // any chunk reached the handler (delivered stays unset) must
        // still retry from chunk zero, since the buffered collector is
        // empty and cannot duplicate. Attempt 1 fails Unavailable
        // without delivering; attempt 2 completes.
        let session = fake_token("v0");
        let policy = RetryPolicy {
            initial_delay: std::time::Duration::ZERO,
            max_delay: std::time::Duration::ZERO,
            max_attempts: 3,
            max_elapsed: std::time::Duration::ZERO,
            jitter: false,
        };

        let (result, attempts) = drive_streaming(
            &session,
            &policy,
            vec![Err(grpc(GrpcStatusKind::Unavailable)), Ok(())],
            &[],
        )
        .await;

        result.expect("pre-first-chunk transient must retry and complete");
        assert_eq!(attempts, 2);
    }

    #[tokio::test]
    async fn streaming_transient_after_delivery_is_terminal() {
        // Core fix: attempt 1 delivers chunks to the handler (buffered
        // collector now holds rows 0..N) and THEN fails transiently.
        // A restart would replay rows 0..N and duplicate the buffered
        // leading rows, so the loop must surface the error instead of
        // retrying — even with retry budget to spare.
        let session = fake_token("v0");
        let policy = RetryPolicy {
            initial_delay: std::time::Duration::ZERO,
            max_delay: std::time::Duration::ZERO,
            max_attempts: 3,
            max_elapsed: std::time::Duration::ZERO,
            jitter: false,
        };

        let (result, attempts) = drive_streaming(
            &session,
            &policy,
            vec![Err(grpc(GrpcStatusKind::Unavailable))],
            &[1],
        )
        .await;

        assert!(
            matches!(
                result,
                Err(Error::Grpc {
                    kind: GrpcStatusKind::Unavailable,
                    ..
                })
            ),
            "mid-stream transient after delivery must be terminal"
        );
        assert_eq!(attempts, 1, "no restart once a chunk was delivered");
    }

    #[tokio::test]
    async fn streaming_refresh_after_delivery_is_terminal() {
        // Same guarantee for the refresh path: a mid-stream
        // Unauthenticated after a chunk was delivered must not replay
        // from chunk zero (the buffered prefix cannot be deduped).
        let session = fake_token("v0");
        let policy = RetryPolicy::disabled();

        let (result, attempts) = drive_streaming(
            &session,
            &policy,
            vec![Err(grpc(GrpcStatusKind::Unauthenticated))],
            &[1],
        )
        .await;

        assert!(matches!(
            result,
            Err(Error::Grpc {
                kind: GrpcStatusKind::Unauthenticated,
                ..
            })
        ));
        assert_eq!(attempts, 1, "no refresh-restart once a chunk was delivered");
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
            max_elapsed: std::time::Duration::ZERO,
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

#[cfg(test)]
mod retry_hint_clamp_tests {
    use super::sleep_for_retry;
    use crate::config::RetryPolicy;
    use crate::error::{Error, GrpcStatusKind};
    use std::time::Duration;

    /// A hostile `RetryInfo` hint cannot stretch the backoff past the policy
    /// ceiling: the sleep is clamped to `max_delay`, so a `with_deadline(ZERO)`
    /// request cannot be pinned for an unbounded cooldown while holding a
    /// request-semaphore permit. Bound the call with a `timeout` so a
    /// regression (the raw ~i64::MAX-second hint) fails fast instead of
    /// hanging the suite.
    #[tokio::test]
    async fn server_hint_is_clamped_to_max_delay() {
        let policy = RetryPolicy {
            initial_delay: Duration::from_millis(1),
            max_delay: Duration::from_millis(20),
            max_attempts: 3,
            max_elapsed: Duration::from_secs(3600),
            jitter: false,
        };
        let err = Error::Grpc {
            kind: GrpcStatusKind::Unavailable,
            message: "hostile hint".into(),
            retry_after: Some(Duration::from_secs(i64::MAX as u64)),
        };
        let clamped = tokio::time::timeout(
            Duration::from_secs(5),
            sleep_for_retry(&policy, 1, "test", &err),
        )
        .await;
        assert!(clamped.is_ok(), "retry sleep was not clamped to max_delay");
    }
}

#[cfg(test)]
mod warn_buffered_tests {
    //! Coverage for the large-buffered-response warn helper.
    //!
    //! Two layers of tests:
    //!
    //! 1. `should_warn_buffered_size`: pure decision function, hit with
    //!    the boundary cases (zero threshold, exact-equal, off-by-one
    //!    above + below, overflow). No tracing wiring needed.
    //! 2. `warn_buffered_response_size`: end-to-end check that the
    //!    helper actually emits a `tracing::warn!` event when the
    //!    decision returns `Some` and stays silent when it returns
    //!    `None`. Uses a custom `tracing_subscriber::Layer` that
    //!    captures events into a `Mutex<Vec<_>>` so the test stays
    //!    self-contained (no global `init()`, no race against parallel
    //!    tests). Scoped via `tracing::subscriber::with_default`.
    use super::{should_warn_buffered_size, warn_buffered_response_size};
    use std::sync::{Arc, Mutex};
    use tracing::{
        field::{Field, Visit},
        subscriber::with_default,
        Event, Level, Subscriber,
    };
    use tracing_subscriber::{layer::Context, prelude::*, registry::Registry, Layer};

    /// Captured snapshot of a single emitted event. Only the fields
    /// we assert on are extracted — message + the three structured
    /// fields the warn helper sets.
    #[derive(Default, Debug, Clone)]
    struct CapturedEvent {
        level: Option<Level>,
        message: Option<String>,
        endpoint: Option<String>,
        row_count: Option<u64>,
        bytes_est: Option<u64>,
        threshold_bytes: Option<u64>,
    }

    /// `Visit` impl that pulls our four structured fields + the
    /// `message` literal off the event.
    impl Visit for CapturedEvent {
        fn record_u64(&mut self, field: &Field, value: u64) {
            match field.name() {
                "row_count" => self.row_count = Some(value),
                "bytes_est" => self.bytes_est = Some(value),
                "threshold_bytes" => self.threshold_bytes = Some(value),
                _ => {}
            }
        }

        fn record_i64(&mut self, field: &Field, value: i64) {
            // `tracing` may surface usize literals as i64 on some
            // builds — accept that path too so the test isn't
            // fragile across versions.
            if value >= 0 {
                self.record_u64(field, value as u64);
            }
        }

        fn record_str(&mut self, field: &Field, value: &str) {
            if field.name() == "endpoint" {
                self.endpoint = Some(value.to_string());
            }
            if field.name() == "message" {
                self.message = Some(value.to_string());
            }
        }

        fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
            // `tracing::warn!(..., "literal")` records the literal
            // message via the implicit `message` field with the
            // `Debug` recorder. Strip the surrounding quotes so the
            // captured string matches the source literal.
            if field.name() == "message" {
                self.message = Some(format!("{value:?}").trim_matches('"').to_string());
            } else if field.name() == "endpoint" && self.endpoint.is_none() {
                self.endpoint = Some(format!("{value:?}").trim_matches('"').to_string());
            }
        }
    }

    /// Custom `Layer` that appends every event it sees to a shared
    /// `Mutex<Vec<CapturedEvent>>`. The collector is cloned into the
    /// layer; the test holds the original `Arc` so it can read the
    /// captured events after `with_default` returns.
    struct CaptureLayer {
        sink: Arc<Mutex<Vec<CapturedEvent>>>,
    }

    impl<S: Subscriber> Layer<S> for CaptureLayer {
        fn on_event(&self, event: &Event<'_>, _ctx: Context<'_, S>) {
            let mut captured = CapturedEvent {
                level: Some(*event.metadata().level()),
                ..Default::default()
            };
            event.record(&mut captured);
            if let Ok(mut sink) = self.sink.lock() {
                sink.push(captured);
            }
        }
    }

    /// Run `body` with a fresh capturing subscriber active for the
    /// duration of the closure. Returns whatever events `body`
    /// emitted via `tracing`.
    fn capture_warns(body: impl FnOnce()) -> Vec<CapturedEvent> {
        let sink: Arc<Mutex<Vec<CapturedEvent>>> = Arc::default();
        let layer = CaptureLayer {
            sink: Arc::clone(&sink),
        };
        let subscriber = Registry::default().with(layer);
        with_default(subscriber, body);
        let guard = sink
            .lock()
            .expect("captured-events mutex must not be poisoned");
        guard.clone()
    }

    #[test]
    fn should_warn_returns_none_when_threshold_is_zero() {
        // Threshold = 0 is the documented "warn disabled" sentinel.
        // Even a humongous response must NOT trigger a warn under
        // this configuration — the operator opted out explicitly.
        assert_eq!(should_warn_buffered_size(10_000_000, 256, 0), None);
    }

    #[test]
    fn should_warn_returns_none_when_size_at_or_below_threshold() {
        // Strict `>` keeps the documented "no warn at exactly N
        // bytes" behaviour — callers can pin a threshold equal to
        // the expected payload size and stay silent.
        assert_eq!(should_warn_buffered_size(100, 256, 25_600), None);
        // Off-by-one under: still silent.
        assert_eq!(should_warn_buffered_size(100, 256, 25_601), None);
    }

    #[test]
    fn should_warn_returns_some_above_threshold() {
        // 101 * 256 = 25_856 bytes > 25_600 → warn fires.
        let bytes = should_warn_buffered_size(101, 256, 25_600)
            .expect("response over threshold must trigger warn");
        assert_eq!(bytes, 25_856);
    }

    #[test]
    fn should_warn_saturates_on_arithmetic_overflow() {
        // On a 32-bit target `usize::MAX * 2` would overflow; the
        // helper saturates instead so the warn still fires (the
        // overflowed product is, by definition, above any
        // realistic threshold).
        let bytes =
            should_warn_buffered_size(usize::MAX, 2, 1).expect("saturated product must warn");
        assert_eq!(bytes, usize::MAX);
    }

    #[test]
    fn warn_helper_emits_tracing_event_above_threshold() {
        // End-to-end seam: 200 rows * 1 KiB = 200 KiB > 100 KiB
        // threshold. The helper must emit exactly one `WARN` event
        // carrying the three structured fields the operator reads
        // from `RUST_LOG=warn`.
        let events = capture_warns(|| {
            warn_buffered_response_size("option_history_quote", 200, 1024, 100 * 1024);
        });
        assert_eq!(
            events.len(),
            1,
            "exactly one warn event must fire per offending request"
        );
        let evt = &events[0];
        assert_eq!(evt.level, Some(Level::WARN));
        assert_eq!(evt.endpoint.as_deref(), Some("option_history_quote"));
        assert_eq!(evt.row_count, Some(200));
        assert_eq!(evt.bytes_est, Some(200 * 1024));
        assert_eq!(evt.threshold_bytes, Some(100 * 1024));
        // Spot-check the prose hint so a future refactor cannot
        // silently strip the `.stream(handler)` recommendation
        // that the issue brief explicitly asks for.
        let msg = evt.message.as_deref().unwrap_or_default();
        assert!(
            msg.contains(".stream(handler)"),
            "warn message must point operators at .stream(handler); got {msg:?}"
        );
    }

    #[test]
    fn warn_helper_stays_silent_below_threshold() {
        // 200 rows * 1 KiB = 200 KiB, threshold 1 MiB → no warn.
        let events = capture_warns(|| {
            warn_buffered_response_size("option_history_quote", 200, 1024, 1024 * 1024);
        });
        assert!(
            events.is_empty(),
            "below-threshold response must not emit a warn; got {events:?}"
        );
    }

    #[test]
    fn warn_helper_stays_silent_when_threshold_is_zero() {
        // Threshold = 0 is the documented "warn disabled" sentinel
        // (see `HistoricalConfig::warn_on_buffered_threshold_bytes`). Even
        // a deliberately huge response must NOT emit a warn — the
        // operator explicitly opted out.
        let events = capture_warns(|| {
            warn_buffered_response_size("option_history_quote", 10_000_000, 1024, 0);
        });
        assert!(
            events.is_empty(),
            "threshold=0 must disable the warn entirely; got {events:?}"
        );
    }
}
