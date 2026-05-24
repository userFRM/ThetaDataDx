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

    // ── MDDS pool sizing ───────────────────────────────────────────

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

    /// Set the warning threshold (in bytes) for buffered (non-streaming)
    /// historical responses. Endpoints whose decoded total exceeds this
    /// value emit a Rust-side `tracing::warn!` pointing the caller at
    /// the `.stream()` surface; the data is still delivered. `0n`
    /// disables the warning entirely. Default is `100n * 1024n * 1024n`
    /// (100 MiB). Byte budgets can exceed `u32::MAX`, so the setter
    /// takes a `BigInt` matching the underlying `usize` field.
    #[napi(js_name = "setWarnOnBufferedThresholdBytes")]
    pub fn set_warn_on_buffered_threshold_bytes(
        &self,
        n: napi::bindgen_prelude::BigInt,
    ) -> napi::Result<()> {
        let (_signed, value, lossless) = n.get_u64();
        if !lossless {
            return Err(napi::Error::from_reason(
                "warn_on_buffered_threshold_bytes must fit in u64",
            ));
        }
        let value = usize::try_from(value)
            .map_err(|_| napi::Error::from_reason("value exceeds usize on this platform"))?;
        let mut guard = self
            .inner
            .lock()
            .map_err(|_| napi::Error::from_reason("Config mutex poisoned"))?;
        guard.mdds.warn_on_buffered_threshold_bytes = value;
        Ok(())
    }

    /// Current `warn_on_buffered_threshold_bytes` setting (bytes,
    /// returned as a `BigInt`).
    #[napi(getter, js_name = "warnOnBufferedThresholdBytes")]
    pub fn warn_on_buffered_threshold_bytes(
        &self,
    ) -> napi::Result<napi::bindgen_prelude::BigInt> {
        let guard = self
            .inner
            .lock()
            .map_err(|_| napi::Error::from_reason("Config mutex poisoned"))?;
        Ok(napi::bindgen_prelude::BigInt::from(
            guard.mdds.warn_on_buffered_threshold_bytes as u64,
        ))
    }

    /// Set the number of dedicated decoder threads in the MDDS pool.
    ///
    /// `0` (default) auto-sizes to `max(available_parallelism / 2, 1)`,
    /// leaving half the logical cores for the tokio reactor and the
    /// application's own work. Override on shared hosts or to widen
    /// the decode pipeline on heavy historical backfills.
    ///
    /// @deprecated since v10.0.1, use setDecodeThreads().
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
    ///
    /// @deprecated since v10.0.1, use decodeThreads.
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

    // ── MDDS two-stage decode pipeline knobs — Phase 3 of 3 ────────
    //
    // Mirror of the Rust core's `MddsConfig::decode_threads` and
    // `decode_queue_depth` fields, both `Option<usize>`. The JS
    // surface accepts `null` / `undefined` (mapped to `None` on the
    // napi side) for the auto-sized default and a `number` for an
    // explicit override. `0` is a legal explicit input — the core
    // clamps `Some(0)` to `1` at pool construction time so a
    // zero-worker pool cannot deadlock stage-1 on the first push.

    /// Set the stage-2 worker thread count for the two-stage MDDS
    /// decode pipeline.
    ///
    /// Stage-2 runs `prost::Message::decode` and the downstream Tick
    /// build off a bounded MPSC queue fed by the stage-1 (per-channel
    /// zstd decompress) threads. Pass `null` or `undefined` for the
    /// auto-sized default (`std::thread::available_parallelism()` on
    /// the Rust side); pass a `number` for an explicit override.
    /// `0` is a legal explicit value — the pool clamps it to `1`
    /// internally.
    #[napi(js_name = "setDecodeThreads")]
    pub fn set_decode_threads(&self, n: Option<u32>) -> napi::Result<()> {
        let mut guard = self
            .inner
            .lock()
            .map_err(|_| napi::Error::from_reason("Config mutex poisoned"))?;
        guard.mdds.decode_threads = n.map(|v| v as usize);
        Ok(())
    }

    /// Current `decode_threads` setting. `null` means auto-size at
    /// connect time; a `number` is the explicit override.
    #[napi(getter, js_name = "decodeThreads")]
    pub fn decode_threads(&self) -> napi::Result<Option<u32>> {
        let guard = self
            .inner
            .lock()
            .map_err(|_| napi::Error::from_reason("Config mutex poisoned"))?;
        Ok(guard
            .mdds
            .decode_threads
            .map(|n| u32::try_from(n).unwrap_or(u32::MAX)))
    }

    /// Set the bounded queue depth between stage-1 and stage-2 of
    /// the two-stage MDDS decode pipeline.
    ///
    /// Stage-1 pushes `DecodedPayload`s into the queue; stage-2
    /// workers pull them out. When stage-2 cannot keep up, stage-1
    /// parks rather than drops. Pass `null` or `undefined` for the
    /// auto-sized default (`concurrent_requests * 64` with a floor
    /// of `64`); pass a `number` for an explicit override. `0` is a
    /// legal explicit value — the queue clamps it to `1` internally.
    #[napi(js_name = "setDecodeQueueDepth")]
    pub fn set_decode_queue_depth(&self, n: Option<u32>) -> napi::Result<()> {
        let mut guard = self
            .inner
            .lock()
            .map_err(|_| napi::Error::from_reason("Config mutex poisoned"))?;
        guard.mdds.decode_queue_depth = n.map(|v| v as usize);
        Ok(())
    }

    /// Current `decode_queue_depth` setting. `null` means auto-size
    /// at connect time; a `number` is the explicit override.
    #[napi(getter, js_name = "decodeQueueDepth")]
    pub fn decode_queue_depth(&self) -> napi::Result<Option<u32>> {
        let guard = self
            .inner
            .lock()
            .map_err(|_| napi::Error::from_reason("Config mutex poisoned"))?;
        Ok(guard
            .mdds
            .decode_queue_depth
            .map(|n| u32::try_from(n).unwrap_or(u32::MAX)))
    }

    // ── FPSS reconnect knobs — parity with Python / C++ / FFI ──────

    /// Set the FPSS reconnect policy.
    ///
    /// - `"auto"` (default): auto-reconnect with the per-class attempt
    ///   budgets supplied by [`Config::setReconnectMaxAttempts`] and
    ///   [`Config::setReconnectMaxRateLimitedAttempts`].
    /// - `"manual"`: no auto-reconnect; callers reconnect explicitly.
    #[napi(js_name = "setReconnectPolicy")]
    pub fn set_reconnect_policy(&self, policy: String) -> napi::Result<()> {
        let parsed = match policy.to_lowercase().as_str() {
            "manual" => config::ReconnectPolicy::Manual,
            "auto" => config::ReconnectPolicy::Auto(config::ReconnectAttemptLimits::default()),
            other => {
                return Err(napi::Error::from_reason(format!(
                    "unknown reconnect_policy: {other:?} (expected \"auto\" or \"manual\")"
                )));
            }
        };
        let mut guard = self
            .inner
            .lock()
            .map_err(|_| napi::Error::from_reason("Config mutex poisoned"))?;
        guard.reconnect.policy = parsed;
        Ok(())
    }

    /// Set the per-class transient-failure attempt budget for the
    /// auto-reconnect path. Default `3`. No effect unless the
    /// reconnect policy is `Auto`.
    #[napi(js_name = "setReconnectMaxAttempts")]
    pub fn set_reconnect_max_attempts(&self, max_attempts: u32) -> napi::Result<()> {
        let mut guard = self
            .inner
            .lock()
            .map_err(|_| napi::Error::from_reason("Config mutex poisoned"))?;
        if let config::ReconnectPolicy::Auto(ref mut limits) = guard.reconnect.policy {
            limits.max_attempts = max_attempts;
        }
        Ok(())
    }

    /// Set the per-class rate-limited (`TooManyRequests`) attempt
    /// budget for the auto-reconnect path. Default `100`. No effect
    /// unless the reconnect policy is `Auto`.
    #[napi(js_name = "setReconnectMaxRateLimitedAttempts")]
    pub fn set_reconnect_max_rate_limited_attempts(
        &self,
        max_rate_limited_attempts: u32,
    ) -> napi::Result<()> {
        let mut guard = self
            .inner
            .lock()
            .map_err(|_| napi::Error::from_reason("Config mutex poisoned"))?;
        if let config::ReconnectPolicy::Auto(ref mut limits) = guard.reconnect.policy {
            limits.max_rate_limited_attempts = max_rate_limited_attempts;
        }
        Ok(())
    }

    /// Set the continuous successful-data-flow window (in seconds)
    /// after which the auto-reconnect attempt counters reset. Default
    /// `60`. No effect unless the reconnect policy is `Auto`.
    ///
    /// Accepts a `bigint` for parity with the Python / C++ / FFI
    /// surface (`u64`). JavaScript `Number` callers should wrap their
    /// value: `setReconnectStableWindowSecs(BigInt(60))`.
    #[napi(js_name = "setReconnectStableWindowSecs")]
    pub fn set_reconnect_stable_window_secs(
        &self,
        secs: napi::bindgen_prelude::BigInt,
    ) -> napi::Result<()> {
        // BigInt → u64. napi's `BigInt` represents value as
        // `sign_bit + words[Vec<u64>]`; the magnitude is the
        // first word when the value fits in 64 bits, with all
        // subsequent words required to be zero.
        if secs.sign_bit && !secs.words.iter().all(|w| *w == 0) {
            return Err(napi::Error::from_reason(
                "setReconnectStableWindowSecs: negative BigInt rejected; \
                 stable_window seconds must be non-negative",
            ));
        }
        if secs.words.len() > 1 {
            return Err(napi::Error::from_reason(
                "setReconnectStableWindowSecs: BigInt magnitude above u64::MAX",
            ));
        }
        let value = secs.words.first().copied().unwrap_or(0);
        let mut guard = self
            .inner
            .lock()
            .map_err(|_| napi::Error::from_reason("Config mutex poisoned"))?;
        if let config::ReconnectPolicy::Auto(ref mut limits) = guard.reconnect.policy {
            limits.stable_window = std::time::Duration::from_secs(value);
        }
        Ok(())
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
