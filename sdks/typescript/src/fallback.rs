//! REST-routing policy napi bindings + `Config` napi class.
//!
//! Mirrors the Python `FallbackPolicy` pyclass + `Config.with_rest_fallback`
//! method one-for-one, plus the four `option_history_*_with_fallback`
//! methods on the `ThetaDataDxClient` napi class.
//!
//! See [`thetadatadx::config::FallbackPolicy`] for the underlying
//! contract and `docs-site/docs/channel-pool-design.md` for the
//! gRPC channel-pool reconnect story.

use std::sync::{Arc, Mutex};

use thetadatadx::config;

use crate::to_napi_err;

/// REST-routing policy. Mirrors [`thetadatadx::config::FallbackPolicy`].
///
/// Constructed via one of the static factories, then installed on
/// a [`Config`] via [`Config::withRestFallback`]. A `Config` with an
/// installed policy is then passed to
/// [`ThetaDataDxClient.connectWithConfig`] / `connectFromFileWithConfig`
/// to bind the policy to a live client.
///
/// # Example
///
/// ```js
/// const { FallbackPolicy, Config, ThetaDataDxClient } = require('@userfrm/thetadatadx');
///
/// const policy = FallbackPolicy.restAlways('http://127.0.0.1:25503');
/// const cfg = Config.production();
/// cfg.withRestFallback(policy);
/// const tdx = ThetaDataDxClient.connectWithConfig('user@example.com', 'pw', cfg);
/// const ticks = await tdx.optionHistoryQuoteWithFallback({
///     symbol: 'AAPL', expiration: '20240105', startDate: '20240104',
/// });
/// ```
#[napi]
#[derive(Clone)]
pub struct FallbackPolicy {
    pub(crate) inner: config::FallbackPolicy,
}

#[napi]
impl FallbackPolicy {
    /// REST routing disabled. Every historical-quote endpoint goes
    /// over gRPC. Default state.
    #[napi(factory)]
    pub fn disabled() -> Self {
        Self {
            inner: config::FallbackPolicy::Disabled,
        }
    }

    /// Always route the four historical-quote endpoints over REST
    /// regardless of the requested date range.
    #[napi(factory, js_name = "restAlways")]
    pub fn rest_always(base_url: String) -> Self {
        Self {
            inner: config::FallbackPolicy::RestAlways { base_url },
        }
    }

    /// Human-readable variant name: `"Disabled"` or `"RestAlways"`.
    /// The Rust enum is `#[non_exhaustive]`, so a future variant
    /// returns `"Unknown"` here until the binding is updated.
    #[napi(getter)]
    pub fn variant(&self) -> &'static str {
        match &self.inner {
            config::FallbackPolicy::Disabled => "Disabled",
            config::FallbackPolicy::RestAlways { .. } => "RestAlways",
            _ => "Unknown",
        }
    }

    /// Return the REST base URL the policy would target, or `null`
    /// for `disabled()`.
    #[napi(getter, js_name = "baseUrl")]
    pub fn base_url(&self) -> Option<String> {
        self.inner.base_url().map(str::to_owned)
    }
}

/// SDK configuration. Mirrors [`thetadatadx::DirectConfig`].
///
/// Build a config via one of the three static factories
/// ([`Config::production`] / [`Config::dev`] / [`Config::stage`]),
/// install a [`FallbackPolicy`] via [`Config::withRestFallback`] if
/// needed, then pass to
/// [`ThetaDataDxClient.connectWithConfig`] /
/// `connectFromFileWithConfig`.
///
/// Mutating methods (`withRestFallback`, ...) follow JS convention and
/// return `void` (chain by calling `cfg.method(...)` then passing
/// `cfg` itself).
///
/// The TypeScript shim takes the inner [`thetadatadx::DirectConfig`]
/// at connect time via a single-shot consume on the napi side, so
/// once the config has been used to connect a client further mutations
/// have no effect on that client.
#[napi]
pub struct Config {
    /// Wrapped in `Arc<Mutex<...>>` so napi-rs can hand `&self` borrows
    /// to multiple JS calls. The mutex is only held for the duration
    /// of a single setter call -- napi-rs is single-threaded on the
    /// main loop, so there is no real contention here, just a
    /// requirement to obey the `Send + Sync` bound napi-rs places on
    /// the type.
    pub(crate) inner: Arc<Mutex<config::DirectConfig>>,
}

#[napi]
impl Config {
    /// Production config (`ThetaData` NJ datacenter).
    #[napi(factory)]
    pub fn production() -> Self {
        Self {
            inner: Arc::new(Mutex::new(config::DirectConfig::production())),
        }
    }

    /// Dev FPSS config (port 20200, infinite historical replay).
    #[napi(factory)]
    pub fn dev() -> Self {
        Self {
            inner: Arc::new(Mutex::new(config::DirectConfig::dev())),
        }
    }

    /// Stage FPSS config (port 20100, unstable testing servers).
    #[napi(factory)]
    pub fn stage() -> Self {
        Self {
            inner: Arc::new(Mutex::new(config::DirectConfig::stage())),
        }
    }

    /// Install a REST-fallback policy. Subsequent
    /// `option_history_*_with_fallback` calls on a client built from
    /// this config will consult the policy. Mirrors
    /// `Python`'s `Config.with_rest_fallback(policy)`.
    #[napi(js_name = "withRestFallback")]
    pub fn with_rest_fallback(&self, policy: &FallbackPolicy) -> napi::Result<()> {
        let mut guard = self
            .inner
            .lock()
            .map_err(|_| napi::Error::from_reason("Config mutex poisoned"))?;
        guard.fallback = policy.inner.clone();
        Ok(())
    }

    /// Current REST-fallback policy variant name. Same string ladder
    /// as [`FallbackPolicy::variant`]. Returns `"Disabled"` when no
    /// fallback policy has been installed.
    #[napi(getter, js_name = "fallbackVariant")]
    pub fn fallback_variant(&self) -> napi::Result<&'static str> {
        let guard = self
            .inner
            .lock()
            .map_err(|_| napi::Error::from_reason("Config mutex poisoned"))?;
        Ok(match &guard.fallback {
            config::FallbackPolicy::Disabled => "Disabled",
            config::FallbackPolicy::RestAlways { .. } => "RestAlways",
            _ => "Unknown",
        })
    }

    // ── MDDS pool sizing — issue #584 ──────────────────────────────

    /// Set the number of concurrent in-flight gRPC requests.
    ///
    /// `0` (default) auto-detects from the Nexus subscription tier
    /// (Free=1 / Value=2 / Standard=4 / Pro=8). Explicit values above
    /// the tier cap are clamped at connect time with a warn.
    #[napi(js_name = "setConcurrentRequests")]
    pub fn set_concurrent_requests(&self, n: u32) -> napi::Result<()> {
        let mut guard = self
            .inner
            .lock()
            .map_err(|_| napi::Error::from_reason("Config mutex poisoned"))?;
        guard.mdds.concurrent_requests = n as usize;
        Ok(())
    }

    /// Current `concurrent_requests` setting (`0` = auto-detect).
    #[napi(getter, js_name = "concurrentRequests")]
    pub fn concurrent_requests(&self) -> napi::Result<u32> {
        let guard = self
            .inner
            .lock()
            .map_err(|_| napi::Error::from_reason("Config mutex poisoned"))?;
        Ok(u32::try_from(guard.mdds.concurrent_requests).unwrap_or(u32::MAX))
    }

    /// Set the number of dedicated decoder threads in the MDDS pool.
    ///
    /// `0` (default) auto-sizes to `max(available_parallelism / 2, 1)`,
    /// leaving half the logical cores for the tokio reactor and the
    /// application's own work. Override on shared hosts or to widen
    /// the decode pipeline on heavy historical backfills.
    #[napi(js_name = "setDecoderThreads")]
    pub fn set_decoder_threads(&self, n: u32) -> napi::Result<()> {
        let mut guard = self
            .inner
            .lock()
            .map_err(|_| napi::Error::from_reason("Config mutex poisoned"))?;
        guard.mdds.decoder_threads = n as usize;
        Ok(())
    }

    /// Current `decoder_threads` setting (`0` = auto-detect).
    #[napi(getter, js_name = "decoderThreads")]
    pub fn decoder_threads(&self) -> napi::Result<u32> {
        let guard = self
            .inner
            .lock()
            .map_err(|_| napi::Error::from_reason("Config mutex poisoned"))?;
        Ok(u32::try_from(guard.mdds.decoder_threads).unwrap_or(u32::MAX))
    }

    /// Set the per-thread decoder ring size.
    ///
    /// Must be a power of two, `>= 64`. The setter rejects invalid
    /// values immediately rather than waiting for the connect-time
    /// `validate()` to fail. Default is `256`.
    #[napi(js_name = "setDecoderRingSize")]
    pub fn set_decoder_ring_size(&self, n: u32) -> napi::Result<()> {
        if n == 0 || !n.is_power_of_two() {
            return Err(napi::Error::from_reason(format!(
                "decoder_ring_size must be a power of two >= 64; got {n}"
            )));
        }
        if n < 64 {
            return Err(napi::Error::from_reason(format!(
                "decoder_ring_size must be >= 64; got {n}"
            )));
        }
        let mut guard = self
            .inner
            .lock()
            .map_err(|_| napi::Error::from_reason("Config mutex poisoned"))?;
        guard.mdds.decoder_ring_size = n as usize;
        Ok(())
    }

    /// Current `decoder_ring_size` setting.
    #[napi(getter, js_name = "decoderRingSize")]
    pub fn decoder_ring_size(&self) -> napi::Result<u32> {
        let guard = self
            .inner
            .lock()
            .map_err(|_| napi::Error::from_reason("Config mutex poisoned"))?;
        Ok(u32::try_from(guard.mdds.decoder_ring_size).unwrap_or(u32::MAX))
    }

    /// Take a snapshot of the underlying [`thetadatadx::DirectConfig`]
    /// for use by `ThetaDataDxClient.connectWithConfig`. Returns a
    /// fresh `DirectConfig` clone -- the napi `Config` remains usable
    /// after the call (subsequent mutations only affect new connects).
    pub(crate) fn snapshot(&self) -> napi::Result<config::DirectConfig> {
        let guard = self
            .inner
            .lock()
            .map_err(|_| napi::Error::from_reason("Config mutex poisoned"))?;
        Ok(guard.clone())
    }
}

/// Default base URL for the local Terminal's REST surface. Mirrors
/// [`thetadatadx::config::DEFAULT_REST_BASE_URL`]. Exposed as a module-
/// level constant so callers can write
/// `FallbackPolicy.restAlways(DEFAULT_REST_BASE_URL)`
/// instead of repeating the URL literal.
#[napi]
pub const DEFAULT_REST_BASE_URL: &str = config::DEFAULT_REST_BASE_URL;

/// Forwarder used by the `ThetaDataDxClient` napi class to dispatch the
/// four `_with_fallback` endpoint calls.
pub(crate) fn err_from_thetadatadx(e: thetadatadx::Error) -> napi::Error {
    to_napi_err(e)
}
