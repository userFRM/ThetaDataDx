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
/// setter name in the failure diagnostic. `get_u64`'s `lossless` flag is
/// `false` for a negative (sign bit set) or an over-`u64` magnitude, so this
/// rejects both rather than passing a wrapped/truncated value.
pub(crate) fn bigint_to_u64(name: &str, v: &napi::bindgen_prelude::BigInt) -> napi::Result<u64> {
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

    /// Historical-staging config (historical staging cluster + auth marker; streaming
    /// stays on production). Unstable testing servers.
    #[napi(factory)]
    pub fn stage() -> Self {
        Self {
            inner: Arc::new(Mutex::new(config::DirectConfig::stage())),
        }
    }

    /// Source the target environment from a `.env`-format file.
    ///
    /// Starts from the production config and applies the cluster keys
    /// carried by the file: `THETADATA_HISTORICAL_TYPE` (`PROD` / `STAGE`,
    /// case-insensitive) selects the environment, and the optional
    /// `THETADATA_HISTORICAL_HOST` / `THETADATA_STREAMING_HOST` keys
    /// override the hosts (an explicit host wins over the environment
    /// default).
    ///
    /// Reads the same file format and keys as `Credentials.fromDotenv`, so
    /// a single `.env` file can carry both `THETADATA_API_KEY` and
    /// `THETADATA_HISTORICAL_TYPE`.
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

    /// Set the streaming event ring buffer size (slots). Must be a power of
    /// two `>= 64`; invalid values are rejected immediately. The slot count
    /// is a pointer-width value in the core, so it marshals as a `BigInt`
    /// like the other wide streaming knobs: `setStreamingRingSize(BigInt(131072))`.
    /// Default `131_072`.
    #[napi(js_name = "setStreamingRingSize")]
    pub fn set_streaming_ring_size(&self, n: napi::bindgen_prelude::BigInt) -> napi::Result<()> {
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

    // ── FlatFilesConfig field setters/getters ─────────────────────

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

    /// Target historical environment carried by this configuration:
    /// `"PROD"` for the production cluster or `"STAGE"` for staging. The
    /// historical and streaming channels are selected independently;
    /// `Config.production()` / `Config.stage()` (and the
    /// `THETADATA_HISTORICAL_TYPE` key on `Config.fromDotenv`) set the historical
    /// channel, and this is the readback of that selection. Mirrors the
    /// `historicalType` string the inline `Client.connectWith` factory accepts.
    #[napi(getter, js_name = "historicalEnvironment")]
    pub fn historical_environment(&self) -> napi::Result<&'static str> {
        let guard = self
            .inner
            .lock()
            .map_err(|_| napi::Error::from_reason("Config mutex poisoned"))?;
        Ok(guard.historical_environment().as_str())
    }

    /// Target streaming environment carried by this configuration:
    /// `"PROD"` for the production cluster or `"DEV"` for the dev cluster.
    /// The streaming and historical channels are selected independently;
    /// `Config.production()` / `Config.dev()` (and the
    /// `THETADATA_STREAMING_TYPE` key on `Config.fromDotenv`) set the streaming
    /// channel, and this is the readback of that selection. Mirrors the
    /// `streamingType` string the inline `Client.connectWith` factory accepts.
    #[napi(getter, js_name = "streamingEnvironment")]
    pub fn streaming_environment(&self) -> napi::Result<&'static str> {
        let guard = self
            .inner
            .lock()
            .map_err(|_| napi::Error::from_reason("Config mutex poisoned"))?;
        Ok(guard.streaming_environment().as_str())
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
}

// Mechanical scalar config setters/getters (`config_surface.toml`), in a
// second `#[napi] impl Config` block. The divergent accessors above (enum,
// string, `Option`, policy-aware) stay hand-written; only the assign/read
// pairs are projected from the SSOT.
include!("_generated/config_accessors.rs");

#[cfg(test)]
mod bigint_to_u64_tests {
    use super::bigint_to_u64;
    use napi::bindgen_prelude::BigInt;

    // The lossless u64 decode behind the BigInt setters (incl.
    // setSlowCallbackThresholdUs) must reject a negative or an over-u64
    // magnitude rather than passing a wrapped/truncated value.
    #[test]
    fn rejects_negative_bigint() {
        let neg = BigInt::from(-1i64);
        assert!(
            bigint_to_u64("test", &neg).is_err(),
            "a negative BigInt must be rejected, not wrapped to a large u64",
        );
    }

    #[test]
    fn rejects_over_u64_magnitude() {
        let huge = BigInt::from(u128::from(u64::MAX) + 1);
        assert!(
            bigint_to_u64("test", &huge).is_err(),
            "a magnitude beyond u64 must be rejected, not truncated",
        );
    }

    #[test]
    fn accepts_in_range_values() {
        assert_eq!(bigint_to_u64("test", &BigInt::from(0u64)).unwrap(), 0);
        assert_eq!(bigint_to_u64("test", &BigInt::from(50_000u64)).unwrap(), 50_000);
        assert_eq!(
            bigint_to_u64("test", &BigInt::from(u64::MAX)).unwrap(),
            u64::MAX,
        );
    }
}
