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

/// `(has_value, n)` shape mirroring the FFI
/// `tdx_config_get_tokio_worker_threads` out-params and the Python
/// `Option<usize>` return — `has_value=false` encodes the `None`
/// sentinel, `has_value=true` carries the explicit worker count
/// (with `n=0` preserved verbatim, matching the `decode_threads`
/// cross-binding contract).
#[napi(object)]
#[derive(Clone, Copy)]
pub struct TokioWorkerThreadsSetting {
    pub has_value: bool,
    pub n: u32,
}

/// REST-routing policy. Mirrors [`thetadatadx::config::FallbackPolicy`].
///
/// Constructed via one of the static factories, then installed on
/// a [`Config`] via `Config.withRestFallback`. A `Config` with an
/// installed policy is then passed to
/// `ThetaDataDxClient.connectWithConfig` / `connectFromFileWithConfig`
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
/// install a [`FallbackPolicy`] via `Config.withRestFallback` if
/// needed, then pass to
/// `ThetaDataDxClient.connectWithConfig` /
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

    /// Dev streaming config (port 20200, infinite historical replay).
    #[napi(factory)]
    pub fn dev() -> Self {
        Self {
            inner: Arc::new(Mutex::new(config::DirectConfig::dev())),
        }
    }

    /// Stage streaming config (port 20100, unstable testing servers).
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

    /// Set the stage-2 worker thread count for the two-stage
    /// historical-channel decode pipeline.
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
    /// the two-stage historical-channel decode pipeline.
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

    // ── Streaming reconnect knobs — parity with Python / C++ / FFI ─

    /// Set the streaming reconnect policy.
    ///
    /// - `"auto"` (default): auto-reconnect with the per-class attempt
    ///   budgets supplied by `Config.setReconnectMaxAttempts` and
    ///   `Config.setReconnectMaxRateLimitedAttempts`.
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

    /// Set the reconnect delay (ms) honoured for generic transient
    /// disconnects (TimedOut, ServerRestarting, Unspecified, …).
    /// Plumbed through to the streaming I/O loop at connect time.
    /// Default `2_000`.
    ///
    /// Accepts a `bigint` for parity with Python / C++ / FFI (`u64`).
    #[napi(js_name = "setReconnectWaitMs")]
    pub fn set_reconnect_wait_ms(
        &self,
        ms: napi::bindgen_prelude::BigInt,
    ) -> napi::Result<()> {
        let (_signed, value, lossless) = ms.get_u64();
        if !lossless {
            return Err(napi::Error::from_reason(
                "setReconnectWaitMs: BigInt magnitude must fit in u64",
            ));
        }
        let mut guard = self
            .inner
            .lock()
            .map_err(|_| napi::Error::from_reason("Config mutex poisoned"))?;
        guard.reconnect.wait_ms = value;
        Ok(())
    }

    /// Current reconnect `wait_ms` value (default `2_000`).
    #[napi(getter, js_name = "reconnectWaitMs")]
    pub fn reconnect_wait_ms(&self) -> napi::Result<napi::bindgen_prelude::BigInt> {
        let guard = self
            .inner
            .lock()
            .map_err(|_| napi::Error::from_reason("Config mutex poisoned"))?;
        Ok(napi::bindgen_prelude::BigInt::from(guard.reconnect.wait_ms))
    }

    /// Set the reconnect delay (ms) honoured for `TooManyRequests`
    /// rate-limited disconnects. Default `130_000`.
    #[napi(js_name = "setReconnectWaitRateLimitedMs")]
    pub fn set_reconnect_wait_rate_limited_ms(
        &self,
        ms: napi::bindgen_prelude::BigInt,
    ) -> napi::Result<()> {
        let (_signed, value, lossless) = ms.get_u64();
        if !lossless {
            return Err(napi::Error::from_reason(
                "setReconnectWaitRateLimitedMs: BigInt magnitude must fit in u64",
            ));
        }
        let mut guard = self
            .inner
            .lock()
            .map_err(|_| napi::Error::from_reason("Config mutex poisoned"))?;
        guard.reconnect.wait_rate_limited_ms = value;
        Ok(())
    }

    /// Current reconnect `wait_rate_limited_ms` value (default `130_000`).
    #[napi(getter, js_name = "reconnectWaitRateLimitedMs")]
    pub fn reconnect_wait_rate_limited_ms(&self) -> napi::Result<napi::bindgen_prelude::BigInt> {
        let guard = self
            .inner
            .lock()
            .map_err(|_| napi::Error::from_reason("Config mutex poisoned"))?;
        Ok(napi::bindgen_prelude::BigInt::from(
            guard.reconnect.wait_rate_limited_ms,
        ))
    }

    /// Set the `RuntimeConfig.tokio_worker_threads` knob for embedded
    /// runtimes built via `RuntimeConfig::build_runtime`. `hasValue=false`
    /// defers to tokio's default sizing; `hasValue=true` pins worker
    /// count to `n` (with `n=0` preserved as the `Some(0)` sentinel,
    /// matching the `decode_threads` setter shape across the binding
    /// matrix).
    #[napi(js_name = "setTokioWorkerThreadsExplicit")]
    pub fn set_tokio_worker_threads_explicit(
        &self,
        has_value: bool,
        n: u32,
    ) -> napi::Result<()> {
        let mut guard = self
            .inner
            .lock()
            .map_err(|_| napi::Error::from_reason("Config mutex poisoned"))?;
        guard.runtime.tokio_worker_threads = if has_value { Some(n as usize) } else { None };
        Ok(())
    }

    /// Current `tokio_worker_threads` setting as `{ hasValue, n }`.
    /// `hasValue=false` encodes the `None` (auto) sentinel.
    #[napi(getter, js_name = "tokioWorkerThreads")]
    pub fn tokio_worker_threads(&self) -> napi::Result<TokioWorkerThreadsSetting> {
        let guard = self
            .inner
            .lock()
            .map_err(|_| napi::Error::from_reason("Config mutex poisoned"))?;
        Ok(match guard.runtime.tokio_worker_threads {
            Some(n) => TokioWorkerThreadsSetting {
                has_value: true,
                n: u32::try_from(n).unwrap_or(u32::MAX),
            },
            None => TokioWorkerThreadsSetting {
                has_value: false,
                n: 0,
            },
        })
    }

    // ── RetryPolicy field setters/getters ─────────────────────────

    /// Set the initial backoff delay (ms) for the historical-channel retry policy.
    /// Default `250n`. Subsequent retries double from here, capped at
    /// `retryMaxDelayMs`.
    #[napi(js_name = "setRetryInitialDelayMs")]
    pub fn set_retry_initial_delay_ms(
        &self,
        ms: napi::bindgen_prelude::BigInt,
    ) -> napi::Result<()> {
        let (_signed, value, lossless) = ms.get_u64();
        if !lossless {
            return Err(napi::Error::from_reason(
                "setRetryInitialDelayMs: BigInt magnitude must fit in u64",
            ));
        }
        let mut guard = self
            .inner
            .lock()
            .map_err(|_| napi::Error::from_reason("Config mutex poisoned"))?;
        guard.retry.initial_delay = std::time::Duration::from_millis(value);
        Ok(())
    }

    /// Current `retry.initial_delay` value (ms, returned as BigInt).
    #[napi(getter, js_name = "retryInitialDelayMs")]
    pub fn retry_initial_delay_ms(&self) -> napi::Result<napi::bindgen_prelude::BigInt> {
        let guard = self
            .inner
            .lock()
            .map_err(|_| napi::Error::from_reason("Config mutex poisoned"))?;
        let ms = u64::try_from(guard.retry.initial_delay.as_millis()).unwrap_or(u64::MAX);
        Ok(napi::bindgen_prelude::BigInt::from(ms))
    }

    /// Set the upper-bound backoff delay (ms) for the MDDS retry
    /// policy. Default `30_000n` (30 s).
    #[napi(js_name = "setRetryMaxDelayMs")]
    pub fn set_retry_max_delay_ms(
        &self,
        ms: napi::bindgen_prelude::BigInt,
    ) -> napi::Result<()> {
        let (_signed, value, lossless) = ms.get_u64();
        if !lossless {
            return Err(napi::Error::from_reason(
                "setRetryMaxDelayMs: BigInt magnitude must fit in u64",
            ));
        }
        let mut guard = self
            .inner
            .lock()
            .map_err(|_| napi::Error::from_reason("Config mutex poisoned"))?;
        guard.retry.max_delay = std::time::Duration::from_millis(value);
        Ok(())
    }

    /// Current `retry.max_delay` value (ms, returned as BigInt).
    #[napi(getter, js_name = "retryMaxDelayMs")]
    pub fn retry_max_delay_ms(&self) -> napi::Result<napi::bindgen_prelude::BigInt> {
        let guard = self
            .inner
            .lock()
            .map_err(|_| napi::Error::from_reason("Config mutex poisoned"))?;
        let ms = u64::try_from(guard.retry.max_delay.as_millis()).unwrap_or(u64::MAX);
        Ok(napi::bindgen_prelude::BigInt::from(ms))
    }

    /// Set the total attempt budget for the historical-channel retry policy. `1`
    /// disables retry; higher values permit retries up to
    /// `maxAttempts - 1` after the initial call. Default `5`.
    #[napi(js_name = "setRetryMaxAttempts")]
    pub fn set_retry_max_attempts(&self, n: u32) -> napi::Result<()> {
        let mut guard = self
            .inner
            .lock()
            .map_err(|_| napi::Error::from_reason("Config mutex poisoned"))?;
        guard.retry.max_attempts = n;
        Ok(())
    }

    /// Current `retry.max_attempts` value.
    #[napi(getter, js_name = "retryMaxAttempts")]
    pub fn retry_max_attempts(&self) -> napi::Result<u32> {
        let guard = self
            .inner
            .lock()
            .map_err(|_| napi::Error::from_reason("Config mutex poisoned"))?;
        Ok(guard.retry.max_attempts)
    }

    /// Toggle AWS-style full-jitter on the historical-channel retry policy. Default
    /// `true`. `false` gives the deterministic backoff schedule
    /// `min(max_delay, initial * 2^attempt)`, useful for tests that
    /// need to assert exact timings.
    #[napi(js_name = "setRetryJitter")]
    pub fn set_retry_jitter(&self, jitter: bool) -> napi::Result<()> {
        let mut guard = self
            .inner
            .lock()
            .map_err(|_| napi::Error::from_reason("Config mutex poisoned"))?;
        guard.retry.jitter = jitter;
        Ok(())
    }

    /// Current `retry.jitter` value.
    #[napi(getter, js_name = "retryJitter")]
    pub fn retry_jitter(&self) -> napi::Result<bool> {
        let guard = self
            .inner
            .lock()
            .map_err(|_| napi::Error::from_reason("Config mutex poisoned"))?;
        Ok(guard.retry.jitter)
    }

    // ── FlatFilesConfig field setters/getters ─────────────────────

    /// Set the total attempt budget for the flatfile driver retry
    /// loop. `1` disables retry (single call only); higher values
    /// permit retries up to `maxAttempts - 1` after the initial call.
    /// Default `3`. Validated to the range `[1, 10]` at connect time.
    #[napi(js_name = "setFlatFilesMaxAttempts")]
    pub fn set_flat_files_max_attempts(&self, n: u32) -> napi::Result<()> {
        let mut guard = self
            .inner
            .lock()
            .map_err(|_| napi::Error::from_reason("Config mutex poisoned"))?;
        guard.flatfiles.max_attempts = n;
        Ok(())
    }

    /// Current `flatfiles.max_attempts` value.
    #[napi(getter, js_name = "flatFilesMaxAttempts")]
    pub fn flat_files_max_attempts(&self) -> napi::Result<u32> {
        let guard = self
            .inner
            .lock()
            .map_err(|_| napi::Error::from_reason("Config mutex poisoned"))?;
        Ok(guard.flatfiles.max_attempts)
    }

    /// Set the initial backoff delay (seconds) for the flatfile
    /// driver retry loop. Doubles per attempt up to
    /// `flatFilesMaxBackoffSecs`. Default `1n`.
    ///
    /// Accepts a `bigint` for parity with the Python / C++ / FFI
    /// surface (`u64`).
    #[napi(js_name = "setFlatFilesInitialBackoffSecs")]
    pub fn set_flat_files_initial_backoff_secs(
        &self,
        secs: napi::bindgen_prelude::BigInt,
    ) -> napi::Result<()> {
        let (_signed, value, lossless) = secs.get_u64();
        if !lossless {
            return Err(napi::Error::from_reason(
                "setFlatFilesInitialBackoffSecs: BigInt magnitude must fit in u64",
            ));
        }
        let mut guard = self
            .inner
            .lock()
            .map_err(|_| napi::Error::from_reason("Config mutex poisoned"))?;
        guard.flatfiles.initial_backoff = std::time::Duration::from_secs(value);
        Ok(())
    }

    /// Current `flatfiles.initial_backoff` value (seconds, returned as BigInt).
    #[napi(getter, js_name = "flatFilesInitialBackoffSecs")]
    pub fn flat_files_initial_backoff_secs(&self) -> napi::Result<napi::bindgen_prelude::BigInt> {
        let guard = self
            .inner
            .lock()
            .map_err(|_| napi::Error::from_reason("Config mutex poisoned"))?;
        Ok(napi::bindgen_prelude::BigInt::from(
            guard.flatfiles.initial_backoff.as_secs(),
        ))
    }

    /// Set the upper-bound backoff delay (seconds) for the flatfile
    /// driver retry loop. The doubling schedule never exceeds this
    /// value regardless of attempt number. Default `4n`. Must be
    /// greater than or equal to `flatFilesInitialBackoffSecs`
    /// (rejected at connect-time validate otherwise).
    ///
    /// Accepts a `bigint` for parity with the Python / C++ / FFI
    /// surface (`u64`).
    #[napi(js_name = "setFlatFilesMaxBackoffSecs")]
    pub fn set_flat_files_max_backoff_secs(
        &self,
        secs: napi::bindgen_prelude::BigInt,
    ) -> napi::Result<()> {
        let (_signed, value, lossless) = secs.get_u64();
        if !lossless {
            return Err(napi::Error::from_reason(
                "setFlatFilesMaxBackoffSecs: BigInt magnitude must fit in u64",
            ));
        }
        let mut guard = self
            .inner
            .lock()
            .map_err(|_| napi::Error::from_reason("Config mutex poisoned"))?;
        guard.flatfiles.max_backoff = std::time::Duration::from_secs(value);
        Ok(())
    }

    /// Current `flatfiles.max_backoff` value (seconds, returned as BigInt).
    #[napi(getter, js_name = "flatFilesMaxBackoffSecs")]
    pub fn flat_files_max_backoff_secs(&self) -> napi::Result<napi::bindgen_prelude::BigInt> {
        let guard = self
            .inner
            .lock()
            .map_err(|_| napi::Error::from_reason("Config mutex poisoned"))?;
        Ok(napi::bindgen_prelude::BigInt::from(
            guard.flatfiles.max_backoff.as_secs(),
        ))
    }

    // ── AuthConfig field setters/getters ──────────────────────────

    /// Set the Nexus auth URL. Default matches the upstream
    /// production endpoint; override to redirect at a staging
    /// cluster for testing.
    #[napi(js_name = "setNexusUrl")]
    pub fn set_nexus_url(&self, url: String) -> napi::Result<()> {
        let mut guard = self
            .inner
            .lock()
            .map_err(|_| napi::Error::from_reason("Config mutex poisoned"))?;
        guard.auth.nexus_url = url;
        Ok(())
    }

    /// Current `auth.nexus_url` value.
    #[napi(getter, js_name = "nexusUrl")]
    pub fn nexus_url(&self) -> napi::Result<String> {
        let guard = self
            .inner
            .lock()
            .map_err(|_| napi::Error::from_reason("Config mutex poisoned"))?;
        Ok(guard.auth.nexus_url.clone())
    }

    /// Set the `QueryInfo.client_type` identifier. Default is
    /// `"rust-thetadatadx"`; override to identify a deployment fleet
    /// in server-side dashboards.
    #[napi(js_name = "setClientType")]
    pub fn set_client_type(&self, client_type: String) -> napi::Result<()> {
        let mut guard = self
            .inner
            .lock()
            .map_err(|_| napi::Error::from_reason("Config mutex poisoned"))?;
        guard.auth.client_type = client_type;
        Ok(())
    }

    /// Current `auth.client_type` value.
    #[napi(getter, js_name = "clientType")]
    pub fn client_type(&self) -> napi::Result<String> {
        let guard = self
            .inner
            .lock()
            .map_err(|_| napi::Error::from_reason("Config mutex poisoned"))?;
        Ok(guard.auth.client_type.clone())
    }

    // ── MetricsConfig field setter/getter ─────────────────────────

    /// Set the Prometheus exporter port. Pass `null` or `undefined`
    /// to leave the exporter disabled (the `None` default); pass a
    /// `number` to bind an HTTP listener on `0.0.0.0:<port>` when the
    /// `metrics-prometheus` feature is compiled in.
    ///
    /// Rejects values outside the `u16` range (`0..=65535`).
    #[napi(js_name = "setMetricsPort")]
    pub fn set_metrics_port(&self, port: Option<u32>) -> napi::Result<()> {
        let resolved = match port {
            Some(v) => Some(u16::try_from(v).map_err(|_| {
                napi::Error::from_reason(format!("setMetricsPort: port must be in 0..=65535; got {v}"))
            })?),
            None => None,
        };
        let mut guard = self
            .inner
            .lock()
            .map_err(|_| napi::Error::from_reason("Config mutex poisoned"))?;
        guard.metrics.port = resolved;
        Ok(())
    }

    /// Current `metrics.port` setting. `null` means the exporter is
    /// disabled; a `number` is the bound port.
    #[napi(getter, js_name = "metricsPort")]
    pub fn metrics_port(&self) -> napi::Result<Option<u32>> {
        let guard = self
            .inner
            .lock()
            .map_err(|_| napi::Error::from_reason("Config mutex poisoned"))?;
        Ok(guard.metrics.port.map(u32::from))
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
