//! REST-routing surface for the historical-quote endpoints.
//!
//! Wraps the four historical-quote gRPC endpoints
//! (`option_history_quote`, `option_history_trade_quote`,
//! `option_history_greeks_implied_volatility`,
//! `option_history_greeks_first_order`) with policy-aware shims
//! ([`MddsClient::option_history_*_with_fallback`]) that consult
//! [`crate::config::FallbackPolicy`] and route to the REST
//! transport ([`crate::rest`]) when [`crate::config::FallbackPolicy::RestAlways`]
//! is set. Call sites that want gRPC-only behaviour keep using the
//! existing generated methods on [`MddsClient`].
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

    /// Fetch option NBBO history. Routes over REST when
    /// [`crate::config::FallbackPolicy::RestAlways`] is set; otherwise dispatches
    /// through the standard gRPC builder.
    ///
    /// # Errors
    ///
    /// Returns [`Error`] on transport, parse, or REST decode failure.
    /// Errors from the REST path are converted via `From<RestError>`
    /// (carrying the structured
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
        if let Some(base_url) = policy.base_url() {
            // Honour the resolved-tier concurrency ceiling uniformly
            // across gRPC and REST transports. Without this acquire a
            // free-tier client alternating gRPC and `_with_fallback`
            // could exceed `mdds.concurrent_requests` because the
            // gRPC-only builders acquire their own permit but the
            // REST arm previously dispatched permit-free.
            let _permit = self
                .request_semaphore
                .acquire()
                .await
                .map_err(|_| Error::config_internal("request semaphore closed"))?;
            tracing::trace!(
                target: "thetadatadx::mdds::fallback",
                endpoint = "option_history_quote",
                start_date,
                end_date,
                "routing to REST per FallbackPolicy::RestAlways"
            );
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
            return builder.execute().await.map_err(Error::from);
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
        builder.await
    }

    /// Fetch combined trade + quote history. Same dispatch semantics
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
        if let Some(base_url) = policy.base_url() {
            let _permit = self
                .request_semaphore
                .acquire()
                .await
                .map_err(|_| Error::config_internal("request semaphore closed"))?;
            tracing::trace!(
                target: "thetadatadx::mdds::fallback",
                endpoint = "option_history_trade_quote",
                start_date,
                end_date,
                "routing to REST per FallbackPolicy::RestAlways"
            );
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
            return builder.execute().await.map_err(Error::from);
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
        builder.await
    }

    /// Fetch option implied-volatility history. Same dispatch
    /// semantics as [`Self::option_history_quote_with_fallback`].
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
        if let Some(base_url) = policy.base_url() {
            let _permit = self
                .request_semaphore
                .acquire()
                .await
                .map_err(|_| Error::config_internal("request semaphore closed"))?;
            tracing::trace!(
                target: "thetadatadx::mdds::fallback",
                endpoint = "option_history_greeks_implied_volatility",
                start_date,
                end_date,
                "routing to REST per FallbackPolicy::RestAlways"
            );
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
            return builder.execute().await.map_err(Error::from);
        }

        let mut builder =
            self.option_history_greeks_implied_volatility(symbol, expiration, start_date);
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
        builder.await
    }

    /// Fetch option first-order Greeks history. Same dispatch
    /// semantics as [`Self::option_history_quote_with_fallback`].
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
        if let Some(base_url) = policy.base_url() {
            let _permit = self
                .request_semaphore
                .acquire()
                .await
                .map_err(|_| Error::config_internal("request semaphore closed"))?;
            tracing::trace!(
                target: "thetadatadx::mdds::fallback",
                endpoint = "option_history_greeks_first_order",
                start_date,
                end_date,
                "routing to REST per FallbackPolicy::RestAlways"
            );
            let rest = self.rest_client_for(base_url)?;
            let mut builder =
                rest.option_history_greeks_first_order(symbol, expiration, start_date);
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
            return builder.execute().await.map_err(Error::from);
        }

        let mut builder = self.option_history_greeks_first_order(symbol, expiration, start_date);
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
        builder.await
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicU64, Ordering};

    use super::*;

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
}
