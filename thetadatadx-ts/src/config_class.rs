//! `Config` napi class for the TypeScript SDK.
use std::sync::{Arc, Mutex};

use thetadatadx::config;

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
    /// carried by the file: `THETADATA_MARKET_DATA_TYPE` (`PROD` / `STAGE`,
    /// case-insensitive) selects the environment, and the optional
    /// `THETADATA_MARKET_DATA_HOST` / `THETADATA_STREAMING_HOST` keys
    /// override the hosts (an explicit host wins over the environment
    /// default).
    ///
    /// Reads the same file format and keys as `Credentials.fromDotenv`, so
    /// a single `.env` file can carry both `THETADATA_API_KEY` and
    /// `THETADATA_MARKET_DATA_TYPE`.
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
                // Weak: the reconnect callback is seated onto a `Config` that may
                // never drive a connection, so a strong threadsafe function would
                // hold the libuv loop open and a process that merely called
                // `setReconnectCallback` could never exit. The `Status != Ok`
                // path already treats a released function as "stop reconnecting",
                // so a weak reference that the runtime may drop on shutdown is
                // handled cleanly.
                true,
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

    /// Set the async worker-thread count for embedded runtimes. `null`
    /// (or omitted) defers to the default sizing; a number pins the worker
    /// count (with `0` preserved verbatim rather than treated as unset).
    ///
    /// The async worker pool is process-global: it is built once, from the
    /// config of the first client connected in the process. This setting
    /// is therefore honored when the first client in the process is
    /// created; clients connected later share the already-built pool, so
    /// setting it on a subsequent config has no effect.
    #[napi(js_name = "setWorkerThreads")]
    pub fn set_worker_threads(&self, n: Option<f64>) -> napi::Result<()> {
        // `0` is a valid, verbatim choice here (the core clamps it to 1),
        // so the plain `validate_optional_u32_arg` is used rather than the
        // `>= 1` floor — but a fractional / negative / over-u32 value is
        // still rejected instead of being silently rewritten by ToUint32.
        let n = crate::validate_optional_u32_arg("workerThreads", n)?;
        let mut guard = self
            .inner
            .lock()
            .map_err(|_| napi::Error::from_reason("Config mutex poisoned"))?;
        guard.runtime.tokio_worker_threads = n.map(|n| n as usize);
        Ok(())
    }

    /// Current `workerThreads` setting, or `null` for the unset (auto)
    /// sentinel. An explicit `0` is preserved verbatim.
    #[napi(getter, js_name = "workerThreads")]
    pub fn worker_threads(&self) -> napi::Result<Option<u32>> {
        let guard = self
            .inner
            .lock()
            .map_err(|_| napi::Error::from_reason("Config mutex poisoned"))?;
        Ok(guard
            .runtime
            .tokio_worker_threads
            .map(|n| u32::try_from(n).unwrap_or(u32::MAX)))
    }

    // `retry.initial_delay` / `retry.max_delay` (ms) getters, the
    // `auth.nexus_url` / `auth.client_type` string accessors, and the
    // `market_data_host` string accessor are generated from
    // config_surface.toml (the `ms` / `string` carve-out kinds).

    // `metrics.port` (`Option<number>` exporter port), the
    // `streaming.flushMode` enum, and the `reconnectJitter` /
    // `streamingHostSelection` enums are the generated `enum` / `option`
    // accessors from config_surface.toml.

    /// Target market-data environment carried by this configuration:
    /// `"PROD"` for the production cluster or `"STAGE"` for staging. The
    /// historical and streaming channels are selected independently;
    /// `Config.production()` / `Config.stage()` (and the
    /// `THETADATA_MARKET_DATA_TYPE` key on `Config.fromDotenv`) set the historical
    /// channel, and this is the readback of that selection. Mirrors the
    /// `marketDataType` string the inline `Client.connectWith` factory accepts.
    #[napi(getter, js_name = "marketDataEnvironment")]
    pub fn market_data_environment(&self) -> napi::Result<&'static str> {
        let guard = self
            .inner
            .lock()
            .map_err(|_| napi::Error::from_reason("Config mutex poisoned"))?;
        Ok(guard.market_data_environment().as_str())
    }

    /// Target streaming environment carried by this configuration:
    /// `"PROD"` for the production cluster or `"DEV"` for the dev cluster.
    /// The streaming and market-data channels are selected independently;
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

    /// Pin the streaming consumer thread to a CPU core, or `null` to
    /// leave it under the OS scheduler (default).
    ///
    /// Pinning the tick-consumer thread to an isolated core gives
    /// deterministic, low-jitter delivery. An out-of-range or offline
    /// core is a best-effort no-op rather than an error.
    #[napi(js_name = "setConsumerCpu")]
    pub fn set_consumer_cpu(&self, core: Option<f64>) -> napi::Result<()> {
        // Core index `0` is valid (pin to CPU 0); only reject a
        // non-finite / negative / fractional / over-u32 value.
        let core = crate::validate_optional_u32_arg("consumerCpu", core)?;
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

// Mechanical config setters/getters (`config_surface.toml`), in a second
// `#[napi] impl Config` block: the scalar / duration pairs plus the
// `policy_limit` (reconnect `Auto`-limit) and `string` carve-out kinds.
// The divergent accessors above (enum string labels, `Option`, policy
// selector) stay hand-written; only the assign/read pairs are projected
// from the SSOT.
include!("_generated/config_accessors.rs");

#[cfg(test)]
mod bigint_to_u64_tests {
    use super::bigint_to_u64;
    use napi::bindgen_prelude::BigInt;

    // The lossless u64 decode behind the BigInt config setters must reject
    // a negative or an over-u64 magnitude rather than passing a
    // wrapped/truncated value.
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
        assert_eq!(
            bigint_to_u64("test", &BigInt::from(50_000u64)).unwrap(),
            50_000
        );
        assert_eq!(
            bigint_to_u64("test", &BigInt::from(u64::MAX)).unwrap(),
            u64::MAX,
        );
    }
}
