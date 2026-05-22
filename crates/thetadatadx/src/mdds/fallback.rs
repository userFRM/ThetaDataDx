//! REST-fallback surface for h2-cascading endpoints (issue #571).
//!
//! Wraps the four affected gRPC endpoints with fallback-aware shims that
//! consult [`crate::config::FallbackPolicy`] and dispatch to the REST
//! transport ([`crate::rest`]) when the policy applies. Call sites that
//! want gRPC-only behaviour keep using the existing generated methods
//! on [`MddsClient`] (e.g. `tdx.option_history_quote(...)`).
//!
//! These shims live on [`MddsClient`] (not on the higher-level
//! [`crate::ThetaDataDxClient`]) so every binding -- Python, FFI, TS,
//! C++ -- can reach them through the same handle type. The unified
//! client picks them up via its [`std::ops::Deref<Target = MddsClient>`]
//! implementation, so the public Rust API shape is unchanged.

use std::collections::HashMap;
use std::sync::{Arc, OnceLock, RwLock};

use crate::error::Error;
use crate::mdds::MddsClient;

/// True iff `err` matches the issue #571 h2-cascade signature. Distinct
/// from a generic `is_retryable` predicate -- only the specific
/// connection-closed transport error counts. Other gRPC failures
/// (auth, timeout, unauthenticated) propagate unchanged.
fn is_h2_disconnect(err: &Error) -> bool {
    matches!(
        err,
        Error::Transport {
            kind: crate::error::TransportErrorKind::ConnectionClosed,
            ..
        }
    )
}

/// Parse a `YYYYMMDD` field, returning a structured [`Error::Config`]
/// for shapes the SDK contract rejects. Centralized here so both the
/// gRPC pre-route check and the REST helpers fail with the same error
/// shape on out-of-range / non-numeric input.
fn parse_yyyymmdd(field: &'static str, date: &str) -> Result<i32, Error> {
    match date.parse::<i32>() {
        Ok(d) if (10_000_000..100_000_000).contains(&d) => Ok(d),
        _ => Err(Error::config_invalid(
            field,
            format!("expected YYYYMMDD, got {date:?}"),
        )),
    }
}

/// Per-base-URL `RestClient` cache helper. Factored out of
/// [`MddsClient::rest_client_for`] so the cache mechanics
/// (read-fast-path / upgrade-to-write / build-once double-check) can be
/// unit-tested without spinning up an authenticated client.
///
/// On a cache hit, returns the existing `Arc<T>` cloned. On a miss,
/// upgrades to a write lock, rechecks (a concurrent caller may have
/// raced ahead), then builds via `build` and inserts.
pub(crate) fn get_or_init_rest_client<T, F>(
    cell: &OnceLock<RwLock<HashMap<String, Arc<T>>>>,
    base_url: &str,
    build: F,
) -> Result<Arc<T>, Error>
where
    F: FnOnce() -> Result<T, Error>,
{
    let map = cell.get_or_init(|| RwLock::new(HashMap::new()));

    if let Some(client) = map
        .read()
        .map_err(|_| Error::config_internal("rest_clients RwLock poisoned"))?
        .get(base_url)
        .cloned()
    {
        return Ok(client);
    }

    let mut guard = map
        .write()
        .map_err(|_| Error::config_internal("rest_clients RwLock poisoned"))?;
    if let Some(client) = guard.get(base_url).cloned() {
        return Ok(client);
    }
    let built = Arc::new(build()?);
    guard.insert(base_url.to_owned(), Arc::clone(&built));
    Ok(built)
}

impl MddsClient {
    /// Return a shared [`Arc<crate::rest::RestClient>`] for `base_url`,
    /// building (and caching) one on first use.
    ///
    /// The fallback shims all funnel through this entry point so a
    /// `reqwest::Client` (TLS context + connection pool) is constructed
    /// at most once per distinct base URL, not once per request.
    fn rest_client_for(&self, base_url: &str) -> Result<Arc<crate::rest::RestClient>, Error> {
        get_or_init_rest_client(&self.rest_clients, base_url, || {
            crate::rest::RestClient::new(base_url).map_err(Error::from)
        })
    }

    /// Fetch option NBBO history via gRPC, falling back to REST per
    /// [`crate::config::FallbackPolicy`] (issue #571).
    ///
    /// Behaviour by policy variant:
    ///
    /// * [`crate::config::FallbackPolicy::Disabled`] -- always gRPC, no fallback.
    ///   Identical to `tdx.option_history_quote(symbol, expiration,
    ///   date).await`.
    /// * [`crate::config::FallbackPolicy::RestOnH2Disconnect`] -- try gRPC first; on
    ///   [`crate::error::TransportErrorKind::ConnectionClosed`] (the
    ///   issue #571 signature) re-issue over REST.
    /// * [`crate::config::FallbackPolicy::RestAlwaysForDateRange`] -- compare
    ///   `start_date` (parsed as `YYYYMMDD` integer) against `before`.
    ///   Strictly-earlier dates skip gRPC and route to REST immediately;
    ///   on-or-after dates flow through gRPC.
    /// * [`crate::config::FallbackPolicy::RestAlways`] -- always REST.
    ///
    /// # Errors
    ///
    /// Returns [`Error`] on transport, parse, or REST decode failure.
    /// Errors from the REST fallback path are converted via
    /// `From<RestError>` (carrying the structured
    /// [`crate::error::TransportErrorKind::UnexpectedHttpStatus`] /
    /// `ConnectionClosed` discriminator).
    #[allow(clippy::too_many_arguments)]
    pub async fn option_history_quote_with_fallback(
        &self,
        symbol: &str,
        expiration: &str,
        start_date: &str,
        end_date: Option<&str>,
        strike: Option<&str>,
        right: Option<&str>,
        interval: Option<&str>,
    ) -> Result<Vec<tdbe::types::tick::QuoteTick>, Error> {
        let policy = &self.config().fallback;
        let start_int = parse_yyyymmdd("start_date", start_date)?;
        if let Some(end) = end_date {
            parse_yyyymmdd("end_date", end)?;
        }

        if policy.pre_routes_to_rest(start_int) {
            if let Some(base_url) = policy.base_url() {
                tracing::info!(
                    target: "thetadatadx::mdds::fallback",
                    endpoint = "option_history_quote",
                    start_date,
                    end_date,
                    "pre-routing to REST per FallbackPolicy"
                );
                return self
                    .rest_quote(
                        base_url, symbol, expiration, start_date, end_date, strike, right, interval,
                    )
                    .await;
            }
        }

        // gRPC path.
        let mut builder = self.option_history_quote(symbol, expiration, start_date);
        if let Some(e) = end_date {
            builder = builder.end_date(e);
        }
        if let Some(s) = strike {
            builder = builder.strike(s);
        }
        if let Some(r) = right {
            builder = builder.right(r);
        }
        if let Some(i) = interval {
            builder = builder.interval(i);
        }
        match builder.await {
            Ok(ticks) => Ok(ticks),
            Err(err) => {
                if policy.falls_back_on_h2_disconnect() && is_h2_disconnect(&err) {
                    if let Some(base_url) = policy.base_url() {
                        tracing::warn!(
                            target: "thetadatadx::mdds::fallback",
                            endpoint = "option_history_quote",
                            error = %err,
                            "h2 disconnect on gRPC, falling back to REST"
                        );
                        return self
                            .rest_quote(
                                base_url, symbol, expiration, start_date, end_date, strike, right,
                                interval,
                            )
                            .await;
                    }
                }
                Err(err)
            }
        }
    }

    /// REST helper for [`Self::option_history_quote_with_fallback`].
    /// Kept private so the high-level surface presents one method per
    /// affected endpoint -- callers do not need to know the REST
    /// transport exists unless they reach for [`crate::rest::RestClient`]
    /// directly.
    #[allow(clippy::too_many_arguments)]
    async fn rest_quote(
        &self,
        base_url: &str,
        symbol: &str,
        expiration: &str,
        start_date: &str,
        end_date: Option<&str>,
        strike: Option<&str>,
        right: Option<&str>,
        interval: Option<&str>,
    ) -> Result<Vec<tdbe::types::tick::QuoteTick>, Error> {
        let rest = self.rest_client_for(base_url)?;
        let mut builder = rest.option_history_quote(symbol, expiration, start_date);
        if let Some(e) = end_date {
            builder = builder.end_date(e);
        }
        if let Some(s) = strike {
            builder = builder.strike(s);
        }
        if let Some(r) = right {
            builder = builder.right(r);
        }
        if let Some(i) = interval {
            builder = builder.interval(i);
        }
        builder.execute().await.map_err(Error::from)
    }

    /// Fetch combined trade + quote history via gRPC, falling back to
    /// REST per [`crate::config::FallbackPolicy`] (issue #571). Same dispatch semantics
    /// as [`Self::option_history_quote_with_fallback`].
    ///
    /// # Errors
    ///
    /// See [`Self::option_history_quote_with_fallback`].
    pub async fn option_history_trade_quote_with_fallback(
        &self,
        symbol: &str,
        expiration: &str,
        start_date: &str,
        end_date: Option<&str>,
        strike: Option<&str>,
        right: Option<&str>,
    ) -> Result<Vec<tdbe::types::tick::TradeQuoteTick>, Error> {
        let policy = &self.config().fallback;
        let start_int = parse_yyyymmdd("start_date", start_date)?;
        if let Some(end) = end_date {
            parse_yyyymmdd("end_date", end)?;
        }

        if policy.pre_routes_to_rest(start_int) {
            if let Some(base_url) = policy.base_url() {
                tracing::info!(
                    target: "thetadatadx::mdds::fallback",
                    endpoint = "option_history_trade_quote",
                    start_date,
                    end_date,
                    "pre-routing to REST per FallbackPolicy"
                );
                return self
                    .rest_trade_quote(
                        base_url, symbol, expiration, start_date, end_date, strike, right,
                    )
                    .await;
            }
        }

        let mut builder = self.option_history_trade_quote(symbol, expiration, start_date);
        if let Some(e) = end_date {
            builder = builder.end_date(e);
        }
        if let Some(s) = strike {
            builder = builder.strike(s);
        }
        if let Some(r) = right {
            builder = builder.right(r);
        }
        match builder.await {
            Ok(ticks) => Ok(ticks),
            Err(err) => {
                if policy.falls_back_on_h2_disconnect() && is_h2_disconnect(&err) {
                    if let Some(base_url) = policy.base_url() {
                        tracing::warn!(
                            target: "thetadatadx::mdds::fallback",
                            endpoint = "option_history_trade_quote",
                            error = %err,
                            "h2 disconnect on gRPC, falling back to REST"
                        );
                        return self
                            .rest_trade_quote(
                                base_url, symbol, expiration, start_date, end_date, strike, right,
                            )
                            .await;
                    }
                }
                Err(err)
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    async fn rest_trade_quote(
        &self,
        base_url: &str,
        symbol: &str,
        expiration: &str,
        start_date: &str,
        end_date: Option<&str>,
        strike: Option<&str>,
        right: Option<&str>,
    ) -> Result<Vec<tdbe::types::tick::TradeQuoteTick>, Error> {
        let rest = self.rest_client_for(base_url)?;
        let mut builder = rest.option_history_trade_quote(symbol, expiration, start_date);
        if let Some(e) = end_date {
            builder = builder.end_date(e);
        }
        if let Some(s) = strike {
            builder = builder.strike(s);
        }
        if let Some(r) = right {
            builder = builder.right(r);
        }
        builder.execute().await.map_err(Error::from)
    }

    /// Fetch option implied-volatility history via gRPC, falling back
    /// to REST per [`crate::config::FallbackPolicy`] (issue #571). Same dispatch
    /// semantics as [`Self::option_history_quote_with_fallback`].
    ///
    /// The implied-volatility endpoint joins each option's quote with
    /// the underlying's NBBO; the underlying-side join is what triggers
    /// the issue #571 cascade on 2022-era rows, so this shim covers
    /// the same date range the quote endpoint does.
    ///
    /// # Errors
    ///
    /// See [`Self::option_history_quote_with_fallback`].
    #[allow(clippy::too_many_arguments)]
    pub async fn option_history_greeks_implied_volatility_with_fallback(
        &self,
        symbol: &str,
        expiration: &str,
        start_date: &str,
        end_date: Option<&str>,
        strike: Option<&str>,
        right: Option<&str>,
        interval: Option<&str>,
    ) -> Result<Vec<tdbe::types::tick::IvTick>, Error> {
        let policy = &self.config().fallback;
        let start_int = parse_yyyymmdd("start_date", start_date)?;
        if let Some(end) = end_date {
            parse_yyyymmdd("end_date", end)?;
        }

        if policy.pre_routes_to_rest(start_int) {
            if let Some(base_url) = policy.base_url() {
                tracing::info!(
                    target: "thetadatadx::mdds::fallback",
                    endpoint = "option_history_greeks_implied_volatility",
                    start_date,
                    end_date,
                    "pre-routing to REST per FallbackPolicy"
                );
                return self
                    .rest_iv(
                        base_url, symbol, expiration, start_date, end_date, strike, right, interval,
                    )
                    .await;
            }
        }

        let mut builder =
            self.option_history_greeks_implied_volatility(symbol, expiration, start_date);
        if let Some(s) = strike {
            builder = builder.strike(s);
        }
        if let Some(r) = right {
            builder = builder.right(r);
        }
        if let Some(i) = interval {
            builder = builder.interval(i);
        }
        match builder.await {
            Ok(ticks) => Ok(ticks),
            Err(err) => {
                if policy.falls_back_on_h2_disconnect() && is_h2_disconnect(&err) {
                    if let Some(base_url) = policy.base_url() {
                        tracing::warn!(
                            target: "thetadatadx::mdds::fallback",
                            endpoint = "option_history_greeks_implied_volatility",
                            error = %err,
                            "h2 disconnect on gRPC, falling back to REST"
                        );
                        return self
                            .rest_iv(
                                base_url, symbol, expiration, start_date, end_date, strike, right,
                                interval,
                            )
                            .await;
                    }
                }
                Err(err)
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    async fn rest_iv(
        &self,
        base_url: &str,
        symbol: &str,
        expiration: &str,
        start_date: &str,
        end_date: Option<&str>,
        strike: Option<&str>,
        right: Option<&str>,
        interval: Option<&str>,
    ) -> Result<Vec<tdbe::types::tick::IvTick>, Error> {
        let rest = self.rest_client_for(base_url)?;
        let mut builder =
            rest.option_history_greeks_implied_volatility(symbol, expiration, start_date);
        if let Some(e) = end_date {
            builder = builder.end_date(e);
        }
        if let Some(s) = strike {
            builder = builder.strike(s);
        }
        if let Some(r) = right {
            builder = builder.right(r);
        }
        if let Some(i) = interval {
            builder = builder.interval(i);
        }
        builder.execute().await.map_err(Error::from)
    }

    /// Fetch option first-order Greeks history via gRPC, falling back
    /// to REST per [`crate::config::FallbackPolicy`] (issue #571). Same dispatch
    /// semantics as [`Self::option_history_quote_with_fallback`].
    ///
    /// The Greeks endpoint shares the same underlying-quote join as
    /// the implied-volatility endpoint, so it cascades on the same
    /// 2022-era rows.
    ///
    /// # Errors
    ///
    /// See [`Self::option_history_quote_with_fallback`].
    #[allow(clippy::too_many_arguments)]
    pub async fn option_history_greeks_first_order_with_fallback(
        &self,
        symbol: &str,
        expiration: &str,
        start_date: &str,
        end_date: Option<&str>,
        strike: Option<&str>,
        right: Option<&str>,
        interval: Option<&str>,
    ) -> Result<Vec<tdbe::types::tick::GreeksFirstOrderTick>, Error> {
        let policy = &self.config().fallback;
        let start_int = parse_yyyymmdd("start_date", start_date)?;
        if let Some(end) = end_date {
            parse_yyyymmdd("end_date", end)?;
        }

        if policy.pre_routes_to_rest(start_int) {
            if let Some(base_url) = policy.base_url() {
                tracing::info!(
                    target: "thetadatadx::mdds::fallback",
                    endpoint = "option_history_greeks_first_order",
                    start_date,
                    end_date,
                    "pre-routing to REST per FallbackPolicy"
                );
                return self
                    .rest_greeks_first_order(
                        base_url, symbol, expiration, start_date, end_date, strike, right, interval,
                    )
                    .await;
            }
        }

        let mut builder = self.option_history_greeks_first_order(symbol, expiration, start_date);
        if let Some(s) = strike {
            builder = builder.strike(s);
        }
        if let Some(r) = right {
            builder = builder.right(r);
        }
        if let Some(i) = interval {
            builder = builder.interval(i);
        }
        match builder.await {
            Ok(ticks) => Ok(ticks),
            Err(err) => {
                if policy.falls_back_on_h2_disconnect() && is_h2_disconnect(&err) {
                    if let Some(base_url) = policy.base_url() {
                        tracing::warn!(
                            target: "thetadatadx::mdds::fallback",
                            endpoint = "option_history_greeks_first_order",
                            error = %err,
                            "h2 disconnect on gRPC, falling back to REST"
                        );
                        return self
                            .rest_greeks_first_order(
                                base_url, symbol, expiration, start_date, end_date, strike, right,
                                interval,
                            )
                            .await;
                    }
                }
                Err(err)
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    async fn rest_greeks_first_order(
        &self,
        base_url: &str,
        symbol: &str,
        expiration: &str,
        start_date: &str,
        end_date: Option<&str>,
        strike: Option<&str>,
        right: Option<&str>,
        interval: Option<&str>,
    ) -> Result<Vec<tdbe::types::tick::GreeksFirstOrderTick>, Error> {
        let rest = self.rest_client_for(base_url)?;
        let mut builder = rest.option_history_greeks_first_order(symbol, expiration, start_date);
        if let Some(e) = end_date {
            builder = builder.end_date(e);
        }
        if let Some(s) = strike {
            builder = builder.strike(s);
        }
        if let Some(r) = right {
            builder = builder.right(r);
        }
        if let Some(i) = interval {
            builder = builder.interval(i);
        }
        builder.execute().await.map_err(Error::from)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicU64, Ordering};

    use super::*;

    /// Well-formed YYYYMMDD passes through bit-exact.
    #[test]
    fn parse_yyyymmdd_accepts_valid_dates() {
        assert_eq!(parse_yyyymmdd("date", "20220414").unwrap(), 20_220_414);
        assert_eq!(parse_yyyymmdd("date", "20240605").unwrap(), 20_240_605);
        // Boundary: smallest plausible date (year 1000) accepted.
        assert_eq!(parse_yyyymmdd("date", "10000101").unwrap(), 10_000_101);
    }

    /// Malformed input must surface `Error::Config { InvalidValue }`
    /// carrying the field name — never the silent `0` fall-through
    /// that previously coerced typos into the most-permissive REST
    /// pre-route branch.
    #[test]
    fn parse_yyyymmdd_rejects_malformed_input() {
        for bad in ["", "bad", "2024", "abcd0605", "20240605xx"] {
            let err = parse_yyyymmdd("start_date", bad).expect_err(bad);
            match err {
                Error::Config { kind, message } => {
                    assert!(
                        matches!(
                            kind,
                            crate::error::ConfigErrorKind::InvalidValue {
                                ref field,
                                ..
                            }
                            if field == "start_date"
                        ),
                        "expected InvalidValue(start_date), got kind={kind:?}"
                    );
                    assert!(
                        message.contains("YYYYMMDD"),
                        "message missing format hint: {message}"
                    );
                }
                other => panic!("expected Config::InvalidValue, got {other:?}"),
            }
        }
    }

    /// Out-of-range integers (parses to i32 but not in the YYYYMMDD
    /// band) are rejected too — guards against "0" or future
    /// year-10000 wraparounds.
    #[test]
    fn parse_yyyymmdd_rejects_out_of_band_integers() {
        for bad in ["0", "1", "9999999", "100000000", "-20240605"] {
            assert!(
                parse_yyyymmdd("date", bad).is_err(),
                "expected error for out-of-band {bad:?}"
            );
        }
    }

    // -- REST-client cache --------------------------------------------
    //
    // The cache machinery is factored into the standalone
    // `get_or_init_rest_client` so we can unit-test the read/write/
    // race-recheck path without spinning up an authenticated
    // `MddsClient`. A `String` stand-in plays the role of the
    // `RestClient` -- the cache contract is type-agnostic.

    #[test]
    fn get_or_init_rest_client_returns_same_arc_on_hit() {
        let cell: OnceLock<RwLock<HashMap<String, Arc<String>>>> = OnceLock::new();
        let build_calls = AtomicU64::new(0);

        let first = get_or_init_rest_client(&cell, "http://127.0.0.1:25503", || {
            build_calls.fetch_add(1, Ordering::SeqCst);
            Ok("client-25503".to_string())
        })
        .unwrap();
        assert_eq!(build_calls.load(Ordering::SeqCst), 1);

        // Second call against the same URL -- cache hit. `build`
        // closure must NOT fire, and the returned Arc must point to the
        // same allocation.
        let second = get_or_init_rest_client(&cell, "http://127.0.0.1:25503", || {
            build_calls.fetch_add(1, Ordering::SeqCst);
            Ok("DIFFERENT-client".to_string())
        })
        .unwrap();
        assert_eq!(
            build_calls.load(Ordering::SeqCst),
            1,
            "second call must hit"
        );
        assert!(Arc::ptr_eq(&first, &second));
        assert_eq!(*second, "client-25503");
    }

    #[test]
    fn get_or_init_rest_client_distinct_urls_get_distinct_clients() {
        let cell: OnceLock<RwLock<HashMap<String, Arc<String>>>> = OnceLock::new();

        let a =
            get_or_init_rest_client(&cell, "http://a:1", || Ok("client-a".to_string())).unwrap();
        let b =
            get_or_init_rest_client(&cell, "http://b:2", || Ok("client-b".to_string())).unwrap();
        assert!(!Arc::ptr_eq(&a, &b));
        assert_eq!(*a, "client-a");
        assert_eq!(*b, "client-b");

        let a2 = get_or_init_rest_client(&cell, "http://a:1", || Ok("WRONG".to_string())).unwrap();
        assert!(Arc::ptr_eq(&a, &a2));
    }

    #[test]
    fn get_or_init_rest_client_propagates_build_error() {
        let cell: OnceLock<RwLock<HashMap<String, Arc<String>>>> = OnceLock::new();
        let res: Result<Arc<String>, Error> = get_or_init_rest_client(&cell, "http://x:1", || {
            Err(Error::config_invalid("x", "bad"))
        });
        assert!(res.is_err(), "build failure must propagate");

        // The failed slot must NOT be cached -- a follow-up successful
        // build should populate it cleanly.
        let res2 = get_or_init_rest_client(&cell, "http://x:1", || Ok("ok".to_string())).unwrap();
        assert_eq!(*res2, "ok");
    }

    #[test]
    fn is_h2_disconnect_matches_only_connection_closed() {
        let err = Error::Transport {
            kind: crate::error::TransportErrorKind::ConnectionClosed,
            message: "h2 cascade".to_string(),
        };
        assert!(is_h2_disconnect(&err));

        let other = Error::Transport {
            kind: crate::error::TransportErrorKind::H2Stream,
            message: "stream reset".to_string(),
        };
        assert!(!is_h2_disconnect(&other));
    }
}
