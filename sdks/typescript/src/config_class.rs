//! `Config` napi class for the TypeScript SDK.
use std::sync::{Arc, Mutex};

use thetadatadx::config;

/// `(hasValue, n)` shape for the worker-threads setting. `hasValue=false`
/// encodes the unset sentinel; `hasValue=true` carries the explicit
/// worker count (with `n=0` preserved verbatim).
#[napi(object)]
#[derive(Clone, Copy)]
pub struct WorkerThreadsSetting {
    pub has_value: bool,
    pub n: u32,
}

/// `(reason, attempt)` argument object handed to the JS reconnect
/// callback registered via `Config.setReconnectCallback`. `reason` is
/// the integer disconnect code; `attempt` is the
/// 1-based consecutive-reconnect counter.
#[napi(object)]
#[derive(Clone, Copy)]
pub struct ReconnectDecisionArgs {
    pub reason: i32,
    pub attempt: u32,
}

/// Decode a non-negative `u64` from a JS `bigint` argument, with the
/// setter name in the failure diagnostic.
fn bigint_to_u64(name: &str, v: &napi::bindgen_prelude::BigInt) -> napi::Result<u64> {
    let (_signed, value, lossless) = v.get_u64();
    if !lossless {
        return Err(napi::Error::from_reason(format!(
            "{name}: BigInt magnitude must fit in u64",
        )));
    }
    Ok(value)
}

/// SDK configuration.
///
/// Build a config via one of the three static factories
/// (`Config.production` / `Config.dev` / `Config.stage`), tune
/// it with the setters below, then pass it as the optional second
/// argument to `Client.connect(creds, config)` /
/// `Client.connectFromFile(path, config)`.
///
/// Mutating methods follow JS convention and
/// return `void` (chain by calling `cfg.method(...)` then passing
/// `cfg` itself).
///
/// The config is consumed at connect time, so once it has been used
/// to connect a client further mutations have no effect on that client.
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

    /// Source the target environment from a `.env`-format file.
    ///
    /// Starts from the production config and applies the cluster keys
    /// carried by the file: `THETADATA_MDDS_TYPE` (`PROD` / `STAGE`,
    /// case-insensitive) selects the environment, and the optional
    /// `THETADATA_HISTORICAL_HOST` / `THETADATA_STREAMING_HOST` keys
    /// override the hosts (an explicit host wins over the environment
    /// default).
    ///
    /// Reads the same file format and keys as `Credentials.fromDotenv`, so
    /// a single `.env` file can carry both `THETADATA_API_KEY` and
    /// `THETADATA_MDDS_TYPE`.
    #[napi(factory, js_name = "fromDotenv")]
    pub fn from_dotenv(path: String) -> napi::Result<Self> {
        let inner = config::DirectConfig::from_dotenv(&path).map_err(crate::to_napi_err)?;
        Ok(Self {
            inner: Arc::new(Mutex::new(inner)),
        })
    }

    /// Snapshot the inner [`config::DirectConfig`] for a connect call.
    ///
    /// The connect factories take an owned `DirectConfig`, while this
    /// handle may be reused or mutated afterward, so the value is cloned
    /// out under the mutex rather than moved. A poisoned mutex is
    /// recovered (the guarded value stays valid — a setter cannot leave
    /// the config half-written), matching the Python binding.
    pub(crate) fn snapshot(&self) -> config::DirectConfig {
        let guard = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        guard.clone()
    }

    // ── historical tuning ────────────────────────────────────────────────

    /// Set the warning threshold (in bytes) for buffered (non-streaming)
    /// historical responses. Endpoints whose decoded total exceeds this
    /// value log a warning pointing the caller at the
    /// matching `<endpoint>Stream(...)` method (e.g. `optionHistoryTradeStream`),
    /// which delivers the same rows chunk-by-chunk through a callback with
    /// memory bounded to a single chunk; the buffered data is still
    /// delivered. `0n` disables the warning entirely. Default is
    /// `100n * 1024n * 1024n` (100 MiB). Byte budgets can exceed the
    /// 32-bit unsigned range, so the setter takes a `BigInt`.
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
        guard.historical.warn_on_buffered_threshold_bytes = value;
        Ok(())
    }

    /// Current `warn_on_buffered_threshold_bytes` setting (bytes,
    /// returned as a `BigInt`).
    #[napi(getter, js_name = "warnOnBufferedThresholdBytes")]
    pub fn warn_on_buffered_threshold_bytes(&self) -> napi::Result<napi::bindgen_prelude::BigInt> {
        let guard = self
            .inner
            .lock()
            .map_err(|_| napi::Error::from_reason("Config mutex poisoned"))?;
        Ok(napi::bindgen_prelude::BigInt::from(
            guard.historical.warn_on_buffered_threshold_bytes as u64,
        ))
    }

    /// Set the default per-request deadline (seconds) for historical
    /// queries. Bounds every request that did not set its own deadline,
    /// so a live-but-silent stream resolves to a timeout instead of
    /// blocking forever. `0n` disables the default. Default `300n`
    /// (5 minutes). Seconds are taken as a `BigInt` for parity with the
    /// other `*Secs` knobs.
    #[napi(js_name = "setRequestTimeoutSecs")]
    pub fn set_request_timeout_secs(
        &self,
        secs: napi::bindgen_prelude::BigInt,
    ) -> napi::Result<()> {
        let value = bigint_to_u64("setRequestTimeoutSecs", &secs)?;
        let mut guard = self
            .inner
            .lock()
            .map_err(|_| napi::Error::from_reason("Config mutex poisoned"))?;
        guard.historical.request_timeout_secs = value;
        Ok(())
    }

    /// Current historical `request_timeout_secs` setting in seconds
    /// (default `300n`; `0n` = no default deadline).
    #[napi(getter, js_name = "requestTimeoutSecs")]
    pub fn request_timeout_secs(&self) -> napi::Result<napi::bindgen_prelude::BigInt> {
        let guard = self
            .inner
            .lock()
            .map_err(|_| napi::Error::from_reason("Config mutex poisoned"))?;
        Ok(napi::bindgen_prelude::BigInt::from(
            guard.historical.request_timeout_secs,
        ))
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
                return Err(crate::invalid_parameter_err(format!(
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
    /// auto-reconnect path. Default `30`. No effect unless the
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
    /// surface, which uses a 64-bit unsigned integer. JavaScript `Number` callers should wrap their
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
    /// Default `250`.
    ///
    /// Accepts a `bigint` for parity with the other bindings, which use a 64-bit unsigned integer.
    #[napi(js_name = "setReconnectWaitMs")]
    pub fn set_reconnect_wait_ms(&self, ms: napi::bindgen_prelude::BigInt) -> napi::Result<()> {
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

    /// Current reconnect `wait_ms` value (default `250`).
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

    /// Set the cap (ms) on the exponential generic-transient reconnect
    /// ladder. The ladder starts at `reconnectWaitMs` and doubles per
    /// consecutive attempt up to this value. Default `30_000n`.
    #[napi(js_name = "setReconnectWaitMaxMs")]
    pub fn set_reconnect_wait_max_ms(&self, ms: napi::bindgen_prelude::BigInt) -> napi::Result<()> {
        let value = bigint_to_u64("setReconnectWaitMaxMs", &ms)?;
        let mut guard = self
            .inner
            .lock()
            .map_err(|_| napi::Error::from_reason("Config mutex poisoned"))?;
        guard.reconnect.wait_max_ms = value;
        Ok(())
    }

    /// Current reconnect `wait_max_ms` value (default `30_000n`).
    #[napi(getter, js_name = "reconnectWaitMaxMs")]
    pub fn reconnect_wait_max_ms(&self) -> napi::Result<napi::bindgen_prelude::BigInt> {
        let guard = self
            .inner
            .lock()
            .map_err(|_| napi::Error::from_reason("Config mutex poisoned"))?;
        Ok(napi::bindgen_prelude::BigInt::from(
            guard.reconnect.wait_max_ms,
        ))
    }

    /// Set the flat reconnect cadence (ms) for `ServerRestarting`
    /// disconnects. Default `5_000n`.
    #[napi(js_name = "setReconnectWaitServerRestartMs")]
    pub fn set_reconnect_wait_server_restart_ms(
        &self,
        ms: napi::bindgen_prelude::BigInt,
    ) -> napi::Result<()> {
        let value = bigint_to_u64("setReconnectWaitServerRestartMs", &ms)?;
        let mut guard = self
            .inner
            .lock()
            .map_err(|_| napi::Error::from_reason("Config mutex poisoned"))?;
        guard.reconnect.wait_server_restart_ms = value;
        Ok(())
    }

    /// Current reconnect `wait_server_restart_ms` value (default `5_000n`).
    #[napi(getter, js_name = "reconnectWaitServerRestartMs")]
    pub fn reconnect_wait_server_restart_ms(&self) -> napi::Result<napi::bindgen_prelude::BigInt> {
        let guard = self
            .inner
            .lock()
            .map_err(|_| napi::Error::from_reason("Config mutex poisoned"))?;
        Ok(napi::bindgen_prelude::BigInt::from(
            guard.reconnect.wait_server_restart_ms,
        ))
    }

    /// Set the jitter strategy applied to every reconnect delay.
    /// Accepts `"full"` (default), `"equal"`, `"decorrelated"`, or
    /// `"none"` (case-insensitive).
    #[napi(js_name = "setReconnectJitter")]
    pub fn set_reconnect_jitter(&self, mode: String) -> napi::Result<()> {
        let parsed = config::JitterMode::parse(&mode).ok_or_else(|| {
            crate::invalid_parameter_err(format!(
                "setReconnectJitter: unknown mode {mode:?}; expected \"full\", \"equal\", \"decorrelated\", or \"none\""
            ))
        })?;
        let mut guard = self
            .inner
            .lock()
            .map_err(|_| napi::Error::from_reason("Config mutex poisoned"))?;
        guard.reconnect.jitter = parsed;
        Ok(())
    }

    /// Current reconnect jitter mode as a lowercase string.
    #[napi(getter, js_name = "reconnectJitter")]
    pub fn reconnect_jitter(&self) -> napi::Result<&'static str> {
        let guard = self
            .inner
            .lock()
            .map_err(|_| napi::Error::from_reason("Config mutex poisoned"))?;
        Ok(guard.reconnect.jitter.as_str())
    }

    /// Set the wall-clock reconnect envelope (seconds) for the
    /// generic-transient and server-restart classes, measured from the
    /// first attempt of a consecutive-reconnect sequence. `0n` disables
    /// the envelope (attempt budgets only). Default `300n`. No effect
    /// unless the reconnect policy is `Auto`.
    #[napi(js_name = "setReconnectMaxElapsedSecs")]
    pub fn set_reconnect_max_elapsed_secs(
        &self,
        secs: napi::bindgen_prelude::BigInt,
    ) -> napi::Result<()> {
        let value = bigint_to_u64("setReconnectMaxElapsedSecs", &secs)?;
        let mut guard = self
            .inner
            .lock()
            .map_err(|_| napi::Error::from_reason("Config mutex poisoned"))?;
        if let config::ReconnectPolicy::Auto(ref mut limits) = guard.reconnect.policy {
            limits.max_elapsed = std::time::Duration::from_secs(value);
        }
        Ok(())
    }

    /// Current wall-clock reconnect envelope in seconds (default
    /// `300n`; `0n` = disabled). Reads the default-limits value when
    /// the policy is not `Auto`.
    #[napi(getter, js_name = "reconnectMaxElapsedSecs")]
    pub fn reconnect_max_elapsed_secs(&self) -> napi::Result<napi::bindgen_prelude::BigInt> {
        let guard = self
            .inner
            .lock()
            .map_err(|_| napi::Error::from_reason("Config mutex poisoned"))?;
        let value = match &guard.reconnect.policy {
            config::ReconnectPolicy::Auto(limits) => limits.max_elapsed,
            _ => config::ReconnectAttemptLimits::default().max_elapsed,
        };
        Ok(napi::bindgen_prelude::BigInt::from(value.as_secs()))
    }

    /// Set the `ServerRestarting` reconnect attempt budget. Default
    /// `60`. No effect unless the reconnect policy is `Auto`.
    #[napi(js_name = "setReconnectMaxServerRestartAttempts")]
    pub fn set_reconnect_max_server_restart_attempts(&self, n: u32) -> napi::Result<()> {
        let mut guard = self
            .inner
            .lock()
            .map_err(|_| napi::Error::from_reason("Config mutex poisoned"))?;
        if let config::ReconnectPolicy::Auto(ref mut limits) = guard.reconnect.policy {
            limits.max_server_restart_attempts = n;
        }
        Ok(())
    }

    /// Current `ServerRestarting` reconnect attempt budget (default
    /// `60`). Reads the default-limits value when the policy is not
    /// `Auto`.
    #[napi(getter, js_name = "reconnectMaxServerRestartAttempts")]
    pub fn reconnect_max_server_restart_attempts(&self) -> napi::Result<u32> {
        let guard = self
            .inner
            .lock()
            .map_err(|_| napi::Error::from_reason("Config mutex poisoned"))?;
        Ok(match &guard.reconnect.policy {
            config::ReconnectPolicy::Auto(limits) => limits.max_server_restart_attempts,
            _ => config::ReconnectAttemptLimits::default().max_server_restart_attempts,
        })
    }

    /// Current reconnect policy as a string (`"auto"`, `"manual"`, or
    /// `"custom"`).
    #[napi(getter, js_name = "reconnectPolicy")]
    pub fn reconnect_policy(&self) -> napi::Result<&'static str> {
        let guard = self
            .inner
            .lock()
            .map_err(|_| napi::Error::from_reason("Config mutex poisoned"))?;
        Ok(match &guard.reconnect.policy {
            config::ReconnectPolicy::Auto(_) => "auto",
            config::ReconnectPolicy::Manual => "manual",
            _ => "custom",
        })
    }

    /// Current generic-transient reconnect attempt budget (default
    /// `30`). Reads the default-limits value when the policy is not
    /// `Auto`.
    #[napi(getter, js_name = "reconnectMaxAttempts")]
    pub fn reconnect_max_attempts(&self) -> napi::Result<u32> {
        let guard = self
            .inner
            .lock()
            .map_err(|_| napi::Error::from_reason("Config mutex poisoned"))?;
        Ok(match &guard.reconnect.policy {
            config::ReconnectPolicy::Auto(limits) => limits.max_attempts,
            _ => config::ReconnectAttemptLimits::default().max_attempts,
        })
    }

    /// Current rate-limited reconnect attempt budget (default `100`).
    /// Reads the default-limits value when the policy is not `Auto`.
    #[napi(getter, js_name = "reconnectMaxRateLimitedAttempts")]
    pub fn reconnect_max_rate_limited_attempts(&self) -> napi::Result<u32> {
        let guard = self
            .inner
            .lock()
            .map_err(|_| napi::Error::from_reason("Config mutex poisoned"))?;
        Ok(match &guard.reconnect.policy {
            config::ReconnectPolicy::Auto(limits) => limits.max_rate_limited_attempts,
            _ => config::ReconnectAttemptLimits::default().max_rate_limited_attempts,
        })
    }

    /// Current stable-window reset interval in seconds (default `60n`).
    /// Reads the default-limits value when the policy is not `Auto`.
    #[napi(getter, js_name = "reconnectStableWindowSecs")]
    pub fn reconnect_stable_window_secs(&self) -> napi::Result<napi::bindgen_prelude::BigInt> {
        let guard = self
            .inner
            .lock()
            .map_err(|_| napi::Error::from_reason("Config mutex poisoned"))?;
        let value = match &guard.reconnect.policy {
            config::ReconnectPolicy::Auto(limits) => limits.stable_window,
            _ => config::ReconnectAttemptLimits::default().stable_window,
        };
        Ok(napi::bindgen_prelude::BigInt::from(value.as_secs()))
    }

    /// Set the subscription-replay burst size used after an
    /// auto-reconnect: frames are written in bursts of this many, each
    /// burst flushed and followed by a jittered `replayPaceMs` pause.
    /// Minimum `1` (validated at connect). Default `50`.
    #[napi(js_name = "setReconnectReplayBurstSize")]
    pub fn set_reconnect_replay_burst_size(&self, n: u32) -> napi::Result<()> {
        let mut guard = self
            .inner
            .lock()
            .map_err(|_| napi::Error::from_reason("Config mutex poisoned"))?;
        guard.reconnect.replay_burst_size = n;
        Ok(())
    }

    /// Current `replay_burst_size` value (default `50`).
    #[napi(getter, js_name = "reconnectReplayBurstSize")]
    pub fn reconnect_replay_burst_size(&self) -> napi::Result<u32> {
        let guard = self
            .inner
            .lock()
            .map_err(|_| napi::Error::from_reason("Config mutex poisoned"))?;
        Ok(guard.reconnect.replay_burst_size)
    }

    /// Set the pause (ms) between subscription-replay bursts after an
    /// auto-reconnect. `0n` removes the pause. Default `5n`.
    #[napi(js_name = "setReconnectReplayPaceMs")]
    pub fn set_reconnect_replay_pace_ms(
        &self,
        ms: napi::bindgen_prelude::BigInt,
    ) -> napi::Result<()> {
        let value = bigint_to_u64("setReconnectReplayPaceMs", &ms)?;
        let mut guard = self
            .inner
            .lock()
            .map_err(|_| napi::Error::from_reason("Config mutex poisoned"))?;
        guard.reconnect.replay_pace_ms = value;
        Ok(())
    }

    /// Current `replay_pace_ms` value (default `5n`).
    #[napi(getter, js_name = "reconnectReplayPaceMs")]
    pub fn reconnect_replay_pace_ms(&self) -> napi::Result<napi::bindgen_prelude::BigInt> {
        let guard = self
            .inner
            .lock()
            .map_err(|_| napi::Error::from_reason("Config mutex poisoned"))?;
        Ok(napi::bindgen_prelude::BigInt::from(
            guard.reconnect.replay_pace_ms,
        ))
    }

    /// Install a custom reconnect policy driven by a JS callback.
    ///
    /// `callback(reason: number, attempt: number)` is invoked (on the
    /// Node main thread, queued from the streaming I/O thread) after
    /// each retriable involuntary disconnect. Return the reconnect
    /// delay in milliseconds, or `null` to stop reconnecting (the
    /// stream then emits the terminal `ReconnectsExhausted` event).
    /// Permanent disconnect reasons (bad credentials, account
    /// conflicts) never reach the callback. Pass `null` to restore the
    /// default `Auto` policy.
    ///
    /// The streaming I/O thread waits for the decision, so the
    /// callback should return promptly; if no decision arrives within
    /// 30 seconds (for example because the Node event loop is blocked)
    /// the stream stops reconnecting and emits the terminal event.
    #[napi(js_name = "setReconnectCallback")]
    pub fn set_reconnect_callback(
        &self,
        callback: Option<
            napi::threadsafe_function::ThreadsafeFunction<
                ReconnectDecisionArgs,
                Option<i64>,
                ReconnectDecisionArgs,
                napi::Status,
                false,
            >,
        >,
    ) -> napi::Result<()> {
        let mut guard = self
            .inner
            .lock()
            .map_err(|_| napi::Error::from_reason("Config mutex poisoned"))?;
        let Some(tsfn) = callback else {
            guard.reconnect.policy =
                config::ReconnectPolicy::Auto(config::ReconnectAttemptLimits::default());
            return Ok(());
        };
        guard.reconnect.policy =
            config::ReconnectPolicy::Custom(std::sync::Arc::new(move |reason, attempt| {
                let (tx, rx) = std::sync::mpsc::sync_channel::<Option<i64>>(1);
                let status = tsfn.call_with_return_value(
                    ReconnectDecisionArgs {
                        reason: reason as i32,
                        attempt,
                    },
                    napi::threadsafe_function::ThreadsafeFunctionCallMode::NonBlocking,
                    move |decision: napi::Result<Option<i64>>, _env| {
                        // A callback that throws (or returns a value
                        // that fails i64 extraction) cannot decide a
                        // delay — treat as "stop reconnecting".
                        let _ = tx.send(decision.unwrap_or(None));
                        Ok(())
                    },
                );
                if status != napi::Status::Ok {
                    // The JS environment is gone (or the queue is
                    // saturated) — no decision can be obtained; stop
                    // reconnecting so the terminal event fires.
                    return None;
                }
                match rx.recv_timeout(std::time::Duration::from_secs(30)) {
                    Ok(Some(ms)) if ms >= 0 => Some(std::time::Duration::from_millis(ms as u64)),
                    Ok(_) => None,
                    // No decision within the window (for example a
                    // blocked Node event loop) — stop reconnecting so
                    // the terminal event fires instead of wedging the
                    // I/O thread indefinitely.
                    Err(_) => None,
                }
            }));
        Ok(())
    }

    // ── streaming transport knobs — parity with Python / C++ / FFI ──────

    /// Set the streaming read timeout (ms): the no-frames deadline after which the streaming I/O loop declares the session dead and reconnects. Default `3_000n`; validated to `[100, 60_000]` at connect.
    #[napi(js_name = "setStreamingTimeoutMs")]
    pub fn set_streaming_timeout_ms(&self, ms: napi::bindgen_prelude::BigInt) -> napi::Result<()> {
        let value = bigint_to_u64("setStreamingTimeoutMs", &ms)?;
        let mut guard = self
            .inner
            .lock()
            .map_err(|_| napi::Error::from_reason("Config mutex poisoned"))?;
        guard.streaming.timeout_ms = value;
        Ok(())
    }

    /// Current `streaming.timeout_ms` value (default `3_000n`).
    #[napi(getter, js_name = "streamingTimeoutMs")]
    pub fn streaming_timeout_ms(&self) -> napi::Result<napi::bindgen_prelude::BigInt> {
        let guard = self
            .inner
            .lock()
            .map_err(|_| napi::Error::from_reason("Config mutex poisoned"))?;
        Ok(napi::bindgen_prelude::BigInt::from(
            guard.streaming.timeout_ms,
        ))
    }

    /// Set the per-server connect timeout (ms) for the streaming connection. Default `2_000n`; validated to `[1_000, 60_000]` at connect.
    #[napi(js_name = "setStreamingConnectTimeoutMs")]
    pub fn set_streaming_connect_timeout_ms(
        &self,
        ms: napi::bindgen_prelude::BigInt,
    ) -> napi::Result<()> {
        let value = bigint_to_u64("setStreamingConnectTimeoutMs", &ms)?;
        let mut guard = self
            .inner
            .lock()
            .map_err(|_| napi::Error::from_reason("Config mutex poisoned"))?;
        guard.streaming.connect_timeout_ms = value;
        Ok(())
    }

    /// Current `streaming.connect_timeout_ms` value (default `2_000n`).
    #[napi(getter, js_name = "streamingConnectTimeoutMs")]
    pub fn streaming_connect_timeout_ms(&self) -> napi::Result<napi::bindgen_prelude::BigInt> {
        let guard = self
            .inner
            .lock()
            .map_err(|_| napi::Error::from_reason("Config mutex poisoned"))?;
        Ok(napi::bindgen_prelude::BigInt::from(
            guard.streaming.connect_timeout_ms,
        ))
    }

    /// Set the streaming heartbeat ping interval (ms). Default `250n`; validated to `[100, 300_000]` at connect.
    #[napi(js_name = "setStreamingPingIntervalMs")]
    pub fn set_streaming_ping_interval_ms(
        &self,
        ms: napi::bindgen_prelude::BigInt,
    ) -> napi::Result<()> {
        let value = bigint_to_u64("setStreamingPingIntervalMs", &ms)?;
        let mut guard = self
            .inner
            .lock()
            .map_err(|_| napi::Error::from_reason("Config mutex poisoned"))?;
        guard.streaming.ping_interval_ms = value;
        Ok(())
    }

    /// Current `streaming.ping_interval_ms` value (default `250n`).
    #[napi(getter, js_name = "streamingPingIntervalMs")]
    pub fn streaming_ping_interval_ms(&self) -> napi::Result<napi::bindgen_prelude::BigInt> {
        let guard = self
            .inner
            .lock()
            .map_err(|_| napi::Error::from_reason("Config mutex poisoned"))?;
        Ok(napi::bindgen_prelude::BigInt::from(
            guard.streaming.ping_interval_ms,
        ))
    }

    /// Set the per-iteration blocking-read slice (ms) for the streaming I/O loop. Default `25n`; validated to `[10, 500]` at connect.
    #[napi(js_name = "setStreamingIoReadSliceMs")]
    pub fn set_streaming_io_read_slice_ms(
        &self,
        ms: napi::bindgen_prelude::BigInt,
    ) -> napi::Result<()> {
        let value = bigint_to_u64("setStreamingIoReadSliceMs", &ms)?;
        let mut guard = self
            .inner
            .lock()
            .map_err(|_| napi::Error::from_reason("Config mutex poisoned"))?;
        guard.streaming.io_read_slice_ms = value;
        Ok(())
    }

    /// Current `streaming.io_read_slice_ms` value (default `25n`).
    #[napi(getter, js_name = "streamingIoReadSliceMs")]
    pub fn streaming_io_read_slice_ms(&self) -> napi::Result<napi::bindgen_prelude::BigInt> {
        let guard = self
            .inner
            .lock()
            .map_err(|_| napi::Error::from_reason("Config mutex poisoned"))?;
        Ok(napi::bindgen_prelude::BigInt::from(
            guard.streaming.io_read_slice_ms,
        ))
    }

    /// Set the last-frame watchdog (ms): when no frame of any kind has arrived for this long the I/O loop force-reconnects. `0n` disables. Default `30_000n`.
    #[napi(js_name = "setStreamingDataWatchdogMs")]
    pub fn set_streaming_data_watchdog_ms(
        &self,
        ms: napi::bindgen_prelude::BigInt,
    ) -> napi::Result<()> {
        let value = bigint_to_u64("setStreamingDataWatchdogMs", &ms)?;
        let mut guard = self
            .inner
            .lock()
            .map_err(|_| napi::Error::from_reason("Config mutex poisoned"))?;
        guard.streaming.data_watchdog_ms = value;
        Ok(())
    }

    /// Current `streaming.data_watchdog_ms` value (default `30_000n`; `0n` = disabled).
    #[napi(getter, js_name = "streamingDataWatchdogMs")]
    pub fn streaming_data_watchdog_ms(&self) -> napi::Result<napi::bindgen_prelude::BigInt> {
        let guard = self
            .inner
            .lock()
            .map_err(|_| napi::Error::from_reason("Config mutex poisoned"))?;
        Ok(napi::bindgen_prelude::BigInt::from(
            guard.streaming.data_watchdog_ms,
        ))
    }

    /// Set the TCP keepalive idle time (seconds) before the first kernel probe on a silent streaming socket. Default `5n`; validated to `[1, 7_200]` at connect.
    #[napi(js_name = "setStreamingKeepaliveIdleSecs")]
    pub fn set_streaming_keepalive_idle_secs(
        &self,
        ms: napi::bindgen_prelude::BigInt,
    ) -> napi::Result<()> {
        let value = bigint_to_u64("setStreamingKeepaliveIdleSecs", &ms)?;
        let mut guard = self
            .inner
            .lock()
            .map_err(|_| napi::Error::from_reason("Config mutex poisoned"))?;
        guard.streaming.keepalive_idle_secs = value;
        Ok(())
    }

    /// Current `streaming.keepalive_idle_secs` value (default `5n`).
    #[napi(getter, js_name = "streamingKeepaliveIdleSecs")]
    pub fn streaming_keepalive_idle_secs(&self) -> napi::Result<napi::bindgen_prelude::BigInt> {
        let guard = self
            .inner
            .lock()
            .map_err(|_| napi::Error::from_reason("Config mutex poisoned"))?;
        Ok(napi::bindgen_prelude::BigInt::from(
            guard.streaming.keepalive_idle_secs,
        ))
    }

    /// Set the interval (seconds) between TCP keepalive probes. Default `2n`; validated to `[1, 75]` at connect.
    #[napi(js_name = "setStreamingKeepaliveIntervalSecs")]
    pub fn set_streaming_keepalive_interval_secs(
        &self,
        ms: napi::bindgen_prelude::BigInt,
    ) -> napi::Result<()> {
        let value = bigint_to_u64("setStreamingKeepaliveIntervalSecs", &ms)?;
        let mut guard = self
            .inner
            .lock()
            .map_err(|_| napi::Error::from_reason("Config mutex poisoned"))?;
        guard.streaming.keepalive_interval_secs = value;
        Ok(())
    }

    /// Current `streaming.keepalive_interval_secs` value (default `2n`).
    #[napi(getter, js_name = "streamingKeepaliveIntervalSecs")]
    pub fn streaming_keepalive_interval_secs(&self) -> napi::Result<napi::bindgen_prelude::BigInt> {
        let guard = self
            .inner
            .lock()
            .map_err(|_| napi::Error::from_reason("Config mutex poisoned"))?;
        Ok(napi::bindgen_prelude::BigInt::from(
            guard.streaming.keepalive_interval_secs,
        ))
    }

    /// Set the number of unanswered TCP keepalive probes after which
    /// the kernel declares the streaming connection dead (where the
    /// platform exposes the knob). Default `2`; validated to `[1, 10]`
    /// at connect.
    #[napi(js_name = "setStreamingKeepaliveRetries")]
    pub fn set_streaming_keepalive_retries(&self, n: u32) -> napi::Result<()> {
        let mut guard = self
            .inner
            .lock()
            .map_err(|_| napi::Error::from_reason("Config mutex poisoned"))?;
        guard.streaming.keepalive_retries = n;
        Ok(())
    }

    /// Current `streaming.keepalive_retries` value (default `2`).
    #[napi(getter, js_name = "streamingKeepaliveRetries")]
    pub fn streaming_keepalive_retries(&self) -> napi::Result<u32> {
        let guard = self
            .inner
            .lock()
            .map_err(|_| napi::Error::from_reason("Config mutex poisoned"))?;
        Ok(guard.streaming.keepalive_retries)
    }

    /// Set the streaming event ring buffer size (slots). Must be a power of
    /// two `>= 64`; invalid values are rejected immediately. The slot count
    /// is a pointer-width value in the core, so it marshals as a `BigInt`
    /// like the other wide streaming knobs: `setStreamingRingSize(BigInt(131072))`.
    /// Default `131_072`.
    #[napi(js_name = "setStreamingRingSize")]
    pub fn set_streaming_ring_size(
        &self,
        n: napi::bindgen_prelude::BigInt,
    ) -> napi::Result<()> {
        let value = bigint_to_u64("setStreamingRingSize", &n)?;
        let value = usize::try_from(value).map_err(|_| {
            crate::invalid_parameter_err(format!(
                "streaming_ring_size {value} exceeds the addressable range on this platform"
            ))
        })?;
        if value == 0 || !value.is_power_of_two() {
            return Err(crate::invalid_parameter_err(format!(
                "streaming_ring_size must be a power of two >= 64; got {value}"
            )));
        }
        if value < 64 {
            return Err(crate::invalid_parameter_err(format!(
                "streaming_ring_size must be >= 64; got {value}"
            )));
        }
        let mut guard = self
            .inner
            .lock()
            .map_err(|_| napi::Error::from_reason("Config mutex poisoned"))?;
        guard.streaming.ring_size = value;
        Ok(())
    }

    /// Current `streaming.ring_size` value (returned as a `BigInt`; default
    /// `131_072`).
    #[napi(getter, js_name = "streamingRingSize")]
    pub fn streaming_ring_size(&self) -> napi::Result<napi::bindgen_prelude::BigInt> {
        let guard = self
            .inner
            .lock()
            .map_err(|_| napi::Error::from_reason("Config mutex poisoned"))?;
        Ok(napi::bindgen_prelude::BigInt::from(
            guard.streaming.ring_size as u64,
        ))
    }

    /// Set the streaming host-selection policy. Accepts `"shuffled"`
    /// (default — fault-domain-aware per-client shuffle) or
    /// `"fixed_order"` (declared order verbatim), case-insensitive.
    #[napi(js_name = "setStreamingHostSelection")]
    pub fn set_streaming_host_selection(&self, policy: String) -> napi::Result<()> {
        let parsed = config::HostSelectionPolicy::parse(&policy).ok_or_else(|| {
            crate::invalid_parameter_err(format!(
                "setStreamingHostSelection: unknown policy {policy:?}; expected \"shuffled\" or \"fixed_order\""
            ))
        })?;
        let mut guard = self
            .inner
            .lock()
            .map_err(|_| napi::Error::from_reason("Config mutex poisoned"))?;
        guard.streaming.host_selection = parsed;
        Ok(())
    }

    /// Current streaming host-selection policy as a lowercase string.
    #[napi(getter, js_name = "streamingHostSelection")]
    pub fn streaming_host_selection(&self) -> napi::Result<&'static str> {
        let guard = self
            .inner
            .lock()
            .map_err(|_| napi::Error::from_reason("Config mutex poisoned"))?;
        Ok(guard.streaming.host_selection.as_str())
    }

    /// Set the streaming host-shuffle seed. `null` (default) derives a
    /// fresh per-client seed so a fleet shuffles independently; an
    /// explicit `bigint` makes the shuffled order deterministic —
    /// useful for fleet sharding and tests. Ignored under
    /// `"fixed_order"`.
    #[napi(js_name = "setStreamingHostShuffleSeed")]
    pub fn set_streaming_host_shuffle_seed(
        &self,
        seed: Option<napi::bindgen_prelude::BigInt>,
    ) -> napi::Result<()> {
        let resolved = match seed {
            Some(v) => Some(bigint_to_u64("setStreamingHostShuffleSeed", &v)?),
            None => None,
        };
        let mut guard = self
            .inner
            .lock()
            .map_err(|_| napi::Error::from_reason("Config mutex poisoned"))?;
        guard.streaming.host_shuffle_seed = resolved;
        Ok(())
    }

    /// Current `streaming.host_shuffle_seed` value (`null` = per-client
    /// entropy).
    #[napi(getter, js_name = "streamingHostShuffleSeed")]
    pub fn streaming_host_shuffle_seed(
        &self,
    ) -> napi::Result<Option<napi::bindgen_prelude::BigInt>> {
        let guard = self
            .inner
            .lock()
            .map_err(|_| napi::Error::from_reason("Config mutex poisoned"))?;
        Ok(guard
            .streaming
            .host_shuffle_seed
            .map(napi::bindgen_prelude::BigInt::from))
    }

    /// Set the wall-clock envelope (seconds) for one
    /// historical-channel retry sequence, measured from the first
    /// attempt. `0n` disables the envelope (attempt budget only).
    /// Default `300n`.
    #[napi(js_name = "setRetryMaxElapsedSecs")]
    pub fn set_retry_max_elapsed_secs(
        &self,
        secs: napi::bindgen_prelude::BigInt,
    ) -> napi::Result<()> {
        let value = bigint_to_u64("setRetryMaxElapsedSecs", &secs)?;
        let mut guard = self
            .inner
            .lock()
            .map_err(|_| napi::Error::from_reason("Config mutex poisoned"))?;
        guard.retry.max_elapsed = std::time::Duration::from_secs(value);
        Ok(())
    }

    /// Current `retry.max_elapsed` value in seconds (default `300n`;
    /// `0n` = disabled).
    #[napi(getter, js_name = "retryMaxElapsedSecs")]
    pub fn retry_max_elapsed_secs(&self) -> napi::Result<napi::bindgen_prelude::BigInt> {
        let guard = self
            .inner
            .lock()
            .map_err(|_| napi::Error::from_reason("Config mutex poisoned"))?;
        Ok(napi::bindgen_prelude::BigInt::from(
            guard.retry.max_elapsed.as_secs(),
        ))
    }

    /// Toggle AWS-style full jitter on the flatfile retry ladder.
    /// Default `true`; `false` gives the deterministic schedule,
    /// useful for tests that assert exact timings.
    #[napi(js_name = "setFlatfilesJitter")]
    pub fn set_flatfiles_jitter(&self, jitter: bool) -> napi::Result<()> {
        let mut guard = self
            .inner
            .lock()
            .map_err(|_| napi::Error::from_reason("Config mutex poisoned"))?;
        guard.flatfiles.jitter = jitter;
        Ok(())
    }

    /// Current `flatfiles.jitter` value (default `true`).
    #[napi(getter, js_name = "flatfilesJitter")]
    pub fn flatfiles_jitter(&self) -> napi::Result<bool> {
        let guard = self
            .inner
            .lock()
            .map_err(|_| napi::Error::from_reason("Config mutex poisoned"))?;
        Ok(guard.flatfiles.jitter)
    }

    /// Set the async worker-thread count for embedded runtimes.
    /// `hasValue=false` defers to the default sizing; `hasValue=true`
    /// pins worker count to `n` (with `n=0` preserved verbatim rather
    /// than treated as unset).
    ///
    /// The async worker pool is process-global: it is built once, from the
    /// config of the first client connected in the process. This setting
    /// is therefore honored when the first client in the process is
    /// created; clients connected later share the already-built pool, so
    /// setting it on a subsequent config has no effect.
    #[napi(js_name = "setWorkerThreads")]
    pub fn set_worker_threads(&self, has_value: bool, n: u32) -> napi::Result<()> {
        let mut guard = self
            .inner
            .lock()
            .map_err(|_| napi::Error::from_reason("Config mutex poisoned"))?;
        guard.runtime.tokio_worker_threads = if has_value { Some(n as usize) } else { None };
        Ok(())
    }

    /// Current `workerThreads` setting as `{ hasValue, n }`.
    /// `hasValue=false` encodes the unset (auto) sentinel.
    #[napi(getter, js_name = "workerThreads")]
    pub fn worker_threads(&self) -> napi::Result<WorkerThreadsSetting> {
        let guard = self
            .inner
            .lock()
            .map_err(|_| napi::Error::from_reason("Config mutex poisoned"))?;
        Ok(match guard.runtime.tokio_worker_threads {
            Some(n) => WorkerThreadsSetting {
                has_value: true,
                n: u32::try_from(n).unwrap_or(u32::MAX),
            },
            None => WorkerThreadsSetting {
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

    /// Set the upper-bound backoff delay (ms) for the historical retry
    /// policy. Default `30_000n` (30 s).
    #[napi(js_name = "setRetryMaxDelayMs")]
    pub fn set_retry_max_delay_ms(&self, ms: napi::bindgen_prelude::BigInt) -> napi::Result<()> {
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
    /// `maxAttempts - 1` after the initial call. Default `20`.
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
    /// Default `10`. Validated to the range `[1, 100]` at connect time.
    #[napi(js_name = "setFlatfilesMaxAttempts")]
    pub fn set_flatfiles_max_attempts(&self, n: u32) -> napi::Result<()> {
        let mut guard = self
            .inner
            .lock()
            .map_err(|_| napi::Error::from_reason("Config mutex poisoned"))?;
        guard.flatfiles.max_attempts = n;
        Ok(())
    }

    /// Current `flatfiles.max_attempts` value.
    #[napi(getter, js_name = "flatfilesMaxAttempts")]
    pub fn flatfiles_max_attempts(&self) -> napi::Result<u32> {
        let guard = self
            .inner
            .lock()
            .map_err(|_| napi::Error::from_reason("Config mutex poisoned"))?;
        Ok(guard.flatfiles.max_attempts)
    }

    /// Set the initial backoff delay (seconds) for the flatfile
    /// driver retry loop. Doubles per attempt up to
    /// `flatfilesMaxBackoffSecs`. Default `1n`.
    ///
    /// Accepts a `bigint` for parity with the other bindings, which
    /// use a 64-bit unsigned integer.
    #[napi(js_name = "setFlatfilesInitialBackoffSecs")]
    pub fn set_flatfiles_initial_backoff_secs(
        &self,
        secs: napi::bindgen_prelude::BigInt,
    ) -> napi::Result<()> {
        let (_signed, value, lossless) = secs.get_u64();
        if !lossless {
            return Err(napi::Error::from_reason(
                "setFlatfilesInitialBackoffSecs: BigInt magnitude must fit in u64",
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
    #[napi(getter, js_name = "flatfilesInitialBackoffSecs")]
    pub fn flatfiles_initial_backoff_secs(&self) -> napi::Result<napi::bindgen_prelude::BigInt> {
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
    /// value regardless of attempt number. Default `30n`. Must be
    /// greater than or equal to `flatfilesInitialBackoffSecs`
    /// (rejected at connect-time validate otherwise).
    ///
    /// Accepts a `bigint` for parity with the other bindings, which
    /// use a 64-bit unsigned integer.
    #[napi(js_name = "setFlatfilesMaxBackoffSecs")]
    pub fn set_flatfiles_max_backoff_secs(
        &self,
        secs: napi::bindgen_prelude::BigInt,
    ) -> napi::Result<()> {
        let (_signed, value, lossless) = secs.get_u64();
        if !lossless {
            return Err(napi::Error::from_reason(
                "setFlatfilesMaxBackoffSecs: BigInt magnitude must fit in u64",
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
    #[napi(getter, js_name = "flatfilesMaxBackoffSecs")]
    pub fn flatfiles_max_backoff_secs(&self) -> napi::Result<napi::bindgen_prelude::BigInt> {
        let guard = self
            .inner
            .lock()
            .map_err(|_| napi::Error::from_reason("Config mutex poisoned"))?;
        Ok(napi::bindgen_prelude::BigInt::from(
            guard.flatfiles.max_backoff.as_secs(),
        ))
    }

    /// Set the TCP + TLS connect timeout (seconds) for one flatfile-host
    /// attempt. Bounds the connect/auth handshake before the attempt is
    /// abandoned and the next host (or the retry ladder) takes over.
    /// Default `10n`.
    ///
    /// Accepts a `bigint` for parity with the other bindings, which
    /// use a 64-bit unsigned integer.
    #[napi(js_name = "setFlatfilesConnectTimeoutSecs")]
    pub fn set_flatfiles_connect_timeout_secs(
        &self,
        secs: napi::bindgen_prelude::BigInt,
    ) -> napi::Result<()> {
        let (_signed, value, lossless) = secs.get_u64();
        if !lossless {
            return Err(napi::Error::from_reason(
                "setFlatfilesConnectTimeoutSecs: BigInt magnitude must fit in u64",
            ));
        }
        let mut guard = self
            .inner
            .lock()
            .map_err(|_| napi::Error::from_reason("Config mutex poisoned"))?;
        guard.flatfiles.connect_timeout_secs = value;
        Ok(())
    }

    /// Current `flatfiles.connect_timeout_secs` value (seconds, returned as BigInt).
    #[napi(getter, js_name = "flatfilesConnectTimeoutSecs")]
    pub fn flatfiles_connect_timeout_secs(&self) -> napi::Result<napi::bindgen_prelude::BigInt> {
        let guard = self
            .inner
            .lock()
            .map_err(|_| napi::Error::from_reason("Config mutex poisoned"))?;
        Ok(napi::bindgen_prelude::BigInt::from(
            guard.flatfiles.connect_timeout_secs,
        ))
    }

    /// Set the read timeout (seconds) for a single flatfile response
    /// frame. Bounds the wait for the next chunk once streaming has begun
    /// so a mid-stream stall fails over instead of blocking forever.
    /// Default `60n`.
    ///
    /// Accepts a `bigint` for parity with the other bindings, which
    /// use a 64-bit unsigned integer.
    #[napi(js_name = "setFlatfilesReadTimeoutSecs")]
    pub fn set_flatfiles_read_timeout_secs(
        &self,
        secs: napi::bindgen_prelude::BigInt,
    ) -> napi::Result<()> {
        let (_signed, value, lossless) = secs.get_u64();
        if !lossless {
            return Err(napi::Error::from_reason(
                "setFlatfilesReadTimeoutSecs: BigInt magnitude must fit in u64",
            ));
        }
        let mut guard = self
            .inner
            .lock()
            .map_err(|_| napi::Error::from_reason("Config mutex poisoned"))?;
        guard.flatfiles.read_timeout_secs = value;
        Ok(())
    }

    /// Current `flatfiles.read_timeout_secs` value (seconds, returned as BigInt).
    #[napi(getter, js_name = "flatfilesReadTimeoutSecs")]
    pub fn flatfiles_read_timeout_secs(&self) -> napi::Result<napi::bindgen_prelude::BigInt> {
        let guard = self
            .inner
            .lock()
            .map_err(|_| napi::Error::from_reason("Config mutex poisoned"))?;
        Ok(napi::bindgen_prelude::BigInt::from(
            guard.flatfiles.read_timeout_secs,
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

    // ── HistoricalConfig advanced endpoint overrides ────────────────────
    //
    // `historical_host` / `historical_port` point the historical gRPC channel at an
    // explicit endpoint. Used by structural tests that need to aim the
    // historical channel at a known-refused endpoint to prove the
    // streaming-only surface never opens it; production code paths keep
    // the `Config.production()` default. Mirrors the Python
    // `Config.historical_host` / `.historical_port`, the C++ `set_historical_host` /
    // `set_historical_port`, and the C ABI `thetadatadx_config_set_historical_host` /
    // `thetadatadx_config_set_historical_port`.

    /// Override the historical gRPC host. Companion to `setHistoricalPort`.
    #[napi(js_name = "setHistoricalHost")]
    pub fn set_historical_host(&self, host: String) -> napi::Result<()> {
        let mut guard = self
            .inner
            .lock()
            .map_err(|_| napi::Error::from_reason("Config mutex poisoned"))?;
        guard.set_historical_host(host);
        Ok(())
    }

    /// Current historical gRPC host.
    #[napi(getter, js_name = "historicalHost")]
    pub fn historical_host(&self) -> napi::Result<String> {
        let guard = self
            .inner
            .lock()
            .map_err(|_| napi::Error::from_reason("Config mutex poisoned"))?;
        Ok(guard.historical_host().to_string())
    }

    /// Override the historical data port. Companion to `setHistoricalHost` —
    /// same test-only rationale. Rejects values outside the `0..=65535`
    /// port range.
    #[napi(js_name = "setHistoricalPort")]
    pub fn set_historical_port(&self, port: u32) -> napi::Result<()> {
        let resolved = u16::try_from(port).map_err(|_| {
            crate::invalid_parameter_err(format!(
                "setHistoricalPort: port must be in the u16 range 0..=65535; got {port}"
            ))
        })?;
        let mut guard = self
            .inner
            .lock()
            .map_err(|_| napi::Error::from_reason("Config mutex poisoned"))?;
        guard.historical.port = resolved;
        Ok(())
    }

    /// Current historical gRPC port.
    #[napi(getter, js_name = "historicalPort")]
    pub fn historical_port(&self) -> napi::Result<u32> {
        let guard = self
            .inner
            .lock()
            .map_err(|_| napi::Error::from_reason("Config mutex poisoned"))?;
        Ok(u32::from(guard.historical.port))
    }

    // ── MetricsConfig field setter/getter ─────────────────────────

    /// Set the Prometheus exporter port. Pass `null` or `undefined`
    /// to leave the exporter disabled (the default); pass a
    /// `number` to bind an HTTP listener on `0.0.0.0:<port>` when the
    /// `metrics-prometheus` feature is compiled in.
    ///
    /// Rejects values outside the `0..=65535` port range.
    #[napi(js_name = "setMetricsPort")]
    pub fn set_metrics_port(&self, port: Option<u32>) -> napi::Result<()> {
        let resolved = match port {
            Some(v) => Some(u16::try_from(v).map_err(|_| {
                crate::invalid_parameter_err(format!(
                    "setMetricsPort: port must be in 0..=65535; got {v}"
                ))
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

    /// Set the streaming write-flush policy.
    ///
    /// Accepts `"batched"` (default — flushes on the PING heartbeat,
    /// roughly every 100 ms — best throughput) or `"immediate"`
    /// (flushes after every wire write — lowest latency, higher
    /// per-frame syscall cost).
    #[napi(js_name = "setFlushMode")]
    pub fn set_flush_mode(&self, mode: String) -> napi::Result<()> {
        let parsed = match mode.to_ascii_lowercase().as_str() {
            "batched" => config::StreamingFlushMode::Batched,
            "immediate" => config::StreamingFlushMode::Immediate,
            other => {
                return Err(crate::invalid_parameter_err(format!(
                    "setFlushMode: mode must be \"batched\" or \"immediate\"; got {other:?}"
                )));
            }
        };
        let mut guard = self
            .inner
            .lock()
            .map_err(|_| napi::Error::from_reason("Config mutex poisoned"))?;
        guard.streaming.flush_mode = parsed;
        Ok(())
    }

    /// Current streaming write-flush policy (`"batched"` or
    /// `"immediate"`).
    #[napi(getter, js_name = "flushMode")]
    pub fn flush_mode(&self) -> napi::Result<&'static str> {
        let guard = self
            .inner
            .lock()
            .map_err(|_| napi::Error::from_reason("Config mutex poisoned"))?;
        Ok(match guard.streaming.flush_mode {
            config::StreamingFlushMode::Batched => "batched",
            config::StreamingFlushMode::Immediate => "immediate",
            _ => "unknown",
        })
    }

    /// Target server environment carried by this configuration: `"PROD"`
    /// for the production cluster, `"STAGE"` for staging. Set as a unit by
    /// `Config.production()` / `Config.stage()` (and by the
    /// `THETADATA_MDDS_TYPE` key on `Config.fromDotenv`); this is the
    /// readback of that selection. Mirrors the `mddsType` string the inline
    /// `Client.connectWith` factory accepts.
    #[napi(getter, js_name = "environment")]
    pub fn environment(&self) -> napi::Result<&'static str> {
        let guard = self
            .inner
            .lock()
            .map_err(|_| napi::Error::from_reason("Config mutex poisoned"))?;
        Ok(guard.environment().as_str())
    }

    /// Set the streaming event-ring consumer wait strategy — the
    /// latency-vs-CPU knob applied on each ring-empty poll.
    ///
    /// Accepts `"low_latency"` (default — never sleeps, lowest latency,
    /// highest idle CPU), `"balanced"` (brief park — low idle CPU),
    /// `"efficient"` (longer park — lowest idle CPU), or `"busy_spin"`
    /// (pure spin — pins a core). Tune the spin / yield / park counts via
    /// `setWaitSpinIters` / `setWaitYieldIters` / `setWaitParkUs`.
    #[napi(js_name = "setWaitStrategy")]
    pub fn set_wait_strategy(&self, strategy: String) -> napi::Result<()> {
        let parsed = config::StreamingWaitStrategy::parse(&strategy).ok_or_else(|| {
            crate::invalid_parameter_err(format!(
                "setWaitStrategy: strategy must be \"low_latency\", \"balanced\", \
                 \"efficient\", or \"busy_spin\"; got {strategy:?}"
            ))
        })?;
        let mut guard = self
            .inner
            .lock()
            .map_err(|_| napi::Error::from_reason("Config mutex poisoned"))?;
        guard.streaming.wait_strategy = parsed;
        Ok(())
    }

    /// Current streaming wait strategy (`"low_latency"`, `"balanced"`,
    /// `"efficient"`, or `"busy_spin"`).
    #[napi(getter, js_name = "waitStrategy")]
    pub fn wait_strategy(&self) -> napi::Result<&'static str> {
        let guard = self
            .inner
            .lock()
            .map_err(|_| napi::Error::from_reason("Config mutex poisoned"))?;
        Ok(guard.streaming.wait_strategy.as_str())
    }

    /// Set the wait-strategy spin iteration count.
    #[napi(js_name = "setWaitSpinIters")]
    pub fn set_wait_spin_iters(&self, iters: u32) -> napi::Result<()> {
        let mut guard = self
            .inner
            .lock()
            .map_err(|_| napi::Error::from_reason("Config mutex poisoned"))?;
        guard.streaming.wait_spin_iters = iters;
        Ok(())
    }

    /// Current wait-strategy spin iteration count.
    #[napi(getter, js_name = "waitSpinIters")]
    pub fn wait_spin_iters(&self) -> napi::Result<u32> {
        let guard = self
            .inner
            .lock()
            .map_err(|_| napi::Error::from_reason("Config mutex poisoned"))?;
        Ok(guard.streaming.wait_spin_iters)
    }

    /// Set the wait-strategy yield iteration count.
    #[napi(js_name = "setWaitYieldIters")]
    pub fn set_wait_yield_iters(&self, iters: u32) -> napi::Result<()> {
        let mut guard = self
            .inner
            .lock()
            .map_err(|_| napi::Error::from_reason("Config mutex poisoned"))?;
        guard.streaming.wait_yield_iters = iters;
        Ok(())
    }

    /// Current wait-strategy yield iteration count.
    #[napi(getter, js_name = "waitYieldIters")]
    pub fn wait_yield_iters(&self) -> napi::Result<u32> {
        let guard = self
            .inner
            .lock()
            .map_err(|_| napi::Error::from_reason("Config mutex poisoned"))?;
        Ok(guard.streaming.wait_yield_iters)
    }

    /// Set the wait-strategy park interval in microseconds (used by the
    /// `"balanced"` / `"efficient"` strategies). The interval is a `u64`
    /// microsecond value in the core, so it marshals as a `BigInt` like
    /// the other microsecond / second streaming and reconnect knobs:
    /// `setWaitParkUs(BigInt(50))`. The core clamps the effective park
    /// interval to its supported ceiling when the wait strategy is built.
    #[napi(js_name = "setWaitParkUs")]
    pub fn set_wait_park_us(&self, park_us: napi::bindgen_prelude::BigInt) -> napi::Result<()> {
        let value = bigint_to_u64("setWaitParkUs", &park_us)?;
        let mut guard = self
            .inner
            .lock()
            .map_err(|_| napi::Error::from_reason("Config mutex poisoned"))?;
        guard.streaming.wait_park_us = value;
        Ok(())
    }

    /// Current wait-strategy park interval in microseconds (returned as a
    /// `BigInt`).
    #[napi(getter, js_name = "waitParkUs")]
    pub fn wait_park_us(&self) -> napi::Result<napi::bindgen_prelude::BigInt> {
        let guard = self
            .inner
            .lock()
            .map_err(|_| napi::Error::from_reason("Config mutex poisoned"))?;
        Ok(napi::bindgen_prelude::BigInt::from(
            guard.streaming.wait_park_us,
        ))
    }

    /// Pin the streaming consumer thread to a CPU core, or `null` to
    /// leave it under the OS scheduler (default).
    ///
    /// Pinning the tick-consumer thread to an isolated core gives
    /// deterministic, low-jitter delivery. An out-of-range or offline
    /// core is a best-effort no-op rather than an error.
    #[napi(js_name = "setConsumerCpu")]
    pub fn set_consumer_cpu(&self, core: Option<u32>) -> napi::Result<()> {
        let mut guard = self
            .inner
            .lock()
            .map_err(|_| napi::Error::from_reason("Config mutex poisoned"))?;
        guard.streaming.consumer_cpu = core.map(|c| c as usize);
        Ok(())
    }

    /// Current streaming consumer-thread CPU pin, or `null` if unpinned.
    #[napi(getter, js_name = "consumerCpu")]
    pub fn consumer_cpu(&self) -> napi::Result<Option<u32>> {
        let guard = self
            .inner
            .lock()
            .map_err(|_| napi::Error::from_reason("Config mutex poisoned"))?;
        Ok(guard
            .streaming
            .consumer_cpu
            .map(|c| u32::try_from(c).unwrap_or(u32::MAX)))
    }

    /// Set whether to derive OHLCVC bars locally from trade events.
    /// When `false`, only server-sent OHLCVC frames are emitted,
    /// reducing per-trade throughput overhead. Default `true`.
    #[napi(js_name = "setDeriveOhlcvc")]
    pub fn set_derive_ohlcvc(&self, enabled: bool) -> napi::Result<()> {
        let mut guard = self
            .inner
            .lock()
            .map_err(|_| napi::Error::from_reason("Config mutex poisoned"))?;
        guard.streaming.derive_ohlcvc = enabled;
        Ok(())
    }

    /// Current OHLCVC derivation setting.
    #[napi(getter, js_name = "deriveOhlcvc")]
    pub fn derive_ohlcvc(&self) -> napi::Result<bool> {
        let guard = self
            .inner
            .lock()
            .map_err(|_| napi::Error::from_reason("Config mutex poisoned"))?;
        Ok(guard.streaming.derive_ohlcvc)
    }
}
