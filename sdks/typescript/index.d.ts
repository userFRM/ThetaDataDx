/* auto-generated — do not edit by hand */
/* eslint-disable */
export declare class Client {
  /**
   * Historical-data sub-namespace: `client.historical.stockHistoryEod(...)`.
   *
   * Returns a fresh [`HistoricalView`] over a cheap `Arc` clone of the
   * inner client. No auth round-trip, no streaming-state mutation.
   */
  get historical(): HistoricalView
  /**
   * Real-time-streaming sub-namespace: `client.stream.subscribe(...)`,
   * `client.stream.startStreaming(cb)`, …
   *
   * Returns a fresh [`StreamView`] sharing the inner client and the
   * parent's callback slot, so the streaming lifecycle observed through
   * the view is the one the unified client manages.
   */
  get stream(): StreamView
  /**
   * Connect to ThetaData with a `Credentials` handle. Pass an
   * optional `Config` (`dev` / `stage` / `production`, plus any
   * tuned setters) to override the production-default endpoint.
   * Historical only; call `client.stream.startStreaming(...)` to
   * begin FPSS real-time data.
   *
   * The config is snapshot at connect time: the `Config` handle may be
   * reused or mutated afterward without affecting this client.
   *
   * ```js
   * const creds = Credentials.fromFile("creds.txt");
   * const client = Client.connect(creds);
   * ```
   */
  static connect(creds: Credentials, config?: Config | undefined | null): Client
  /**
   * Connect with a credentials file (line 1 = email, line 2 =
   * password). Convenience wrapper over `Credentials.fromFile` +
   * `connect`. Pass an optional `Config` to override the
   * production-default endpoint.
   */
  static connectFromFile(path: string, config?: Config | undefined | null): Client
  /** FLATFILES namespace handle. Cheap — clones the inner Arc. */
  get flatFiles(): FlatFilesNamespace
  /**
   * Pull a flat-file blob and write the requested format to `path`.
   * Returns the final on-disk path with the format extension
   * auto-appended if missing.
   */
  flatFileToPath(secType: string, reqType: string, date: string, path: string, format?: string | undefined | null): string
}

/**
 * SDK configuration.
 *
 * Build a config via one of the three static factories
 * (`Config.production` / `Config.dev` / `Config.stage`), tune
 * it with the setters below, then pass it as the optional second
 * argument to `Client.connect(creds, config)` /
 * `Client.connectFromFile(path, config)`.
 *
 * Mutating methods follow JS convention and
 * return `void` (chain by calling `cfg.method(...)` then passing
 * `cfg` itself).
 *
 * The config is consumed at connect time, so once it has been used
 * to connect a client further mutations have no effect on that client.
 */
export declare class Config {
  /** Production config (`ThetaData` NJ datacenter). */
  static production(): Config
  /** Dev streaming config (port 20200, infinite historical replay). */
  static dev(): Config
  /** Stage streaming config (port 20100, unstable testing servers). */
  static stage(): Config
  /**
   * Set the number of concurrent in-flight gRPC requests.
   *
   * `0` (default) auto-detects from the Nexus subscription tier
   * (Free=1 / Value=2 / Standard=4 / Pro=8). Explicit values above
   * the tier cap are clamped at connect time with a warn.
   */
  setConcurrentRequests(n: number): void
  /** Current `concurrent_requests` setting (`0` = auto-detect). */
  get concurrentRequests(): number
  /**
   * Set the warning threshold (in bytes) for buffered (non-streaming)
   * historical responses. Endpoints whose decoded total exceeds this
   * value log a warning pointing the caller at the
   * matching `<endpoint>Stream(...)` method (e.g. `optionHistoryTradeStream`),
   * which delivers the same rows chunk-by-chunk through a callback with
   * memory bounded to a single chunk; the buffered data is still
   * delivered. `0n` disables the warning entirely. Default is
   * `100n * 1024n * 1024n` (100 MiB). Byte budgets can exceed the
   * 32-bit unsigned range, so the setter takes a `BigInt`.
   */
  setWarnOnBufferedThresholdBytes(n: bigint): void
  /**
   * Current `warn_on_buffered_threshold_bytes` setting (bytes,
   * returned as a `BigInt`).
   */
  get warnOnBufferedThresholdBytes(): bigint
  /**
   * Set the streaming reconnect policy.
   *
   * - `"auto"` (default): auto-reconnect with the per-class attempt
   *   budgets supplied by `Config.setReconnectMaxAttempts` and
   *   `Config.setReconnectMaxRateLimitedAttempts`.
   * - `"manual"`: no auto-reconnect; callers reconnect explicitly.
   */
  setReconnectPolicy(policy: string): void
  /**
   * Set the per-class transient-failure attempt budget for the
   * auto-reconnect path. Default `30`. No effect unless the
   * reconnect policy is `Auto`.
   */
  setReconnectMaxAttempts(maxAttempts: number): void
  /**
   * Set the per-class rate-limited (`TooManyRequests`) attempt
   * budget for the auto-reconnect path. Default `100`. No effect
   * unless the reconnect policy is `Auto`.
   */
  setReconnectMaxRateLimitedAttempts(maxRateLimitedAttempts: number): void
  /**
   * Set the continuous successful-data-flow window (in seconds)
   * after which the auto-reconnect attempt counters reset. Default
   * `60`. No effect unless the reconnect policy is `Auto`.
   *
   * Accepts a `bigint` for parity with the Python / C++ / FFI
   * surface, which uses a 64-bit unsigned integer. JavaScript `Number` callers should wrap their
   * value: `setReconnectStableWindowSecs(BigInt(60))`.
   */
  setReconnectStableWindowSecs(secs: bigint): void
  /**
   * Set the reconnect delay (ms) honoured for generic transient
   * disconnects (TimedOut, ServerRestarting, Unspecified, …).
   * Plumbed through to the streaming I/O loop at connect time.
   * Default `250`.
   *
   * Accepts a `bigint` for parity with the other bindings, which use a 64-bit unsigned integer.
   */
  setReconnectWaitMs(ms: bigint): void
  /** Current reconnect `wait_ms` value (default `250`). */
  get reconnectWaitMs(): bigint
  /**
   * Set the reconnect delay (ms) honoured for `TooManyRequests`
   * rate-limited disconnects. Default `130_000`.
   */
  setReconnectWaitRateLimitedMs(ms: bigint): void
  /** Current reconnect `wait_rate_limited_ms` value (default `130_000`). */
  get reconnectWaitRateLimitedMs(): bigint
  /**
   * Set the cap (ms) on the exponential generic-transient reconnect
   * ladder. The ladder starts at `reconnectWaitMs` and doubles per
   * consecutive attempt up to this value. Default `30_000n`.
   */
  setReconnectWaitMaxMs(ms: bigint): void
  /** Current reconnect `wait_max_ms` value (default `30_000n`). */
  get reconnectWaitMaxMs(): bigint
  /**
   * Set the flat reconnect cadence (ms) for `ServerRestarting`
   * disconnects. Default `5_000n`.
   */
  setReconnectWaitServerRestartMs(ms: bigint): void
  /** Current reconnect `wait_server_restart_ms` value (default `5_000n`). */
  get reconnectWaitServerRestartMs(): bigint
  /**
   * Set the jitter strategy applied to every reconnect delay.
   * Accepts `"full"` (default), `"equal"`, `"decorrelated"`, or
   * `"none"` (case-insensitive).
   */
  setReconnectJitter(mode: string): void
  /** Current reconnect jitter mode as a lowercase string. */
  get reconnectJitter(): string
  /**
   * Set the wall-clock reconnect envelope (seconds) for the
   * generic-transient and server-restart classes, measured from the
   * first attempt of a consecutive-reconnect sequence. `0n` disables
   * the envelope (attempt budgets only). Default `300n`. No effect
   * unless the reconnect policy is `Auto`.
   */
  setReconnectMaxElapsedSecs(secs: bigint): void
  /**
   * Current wall-clock reconnect envelope in seconds (default
   * `300n`; `0n` = disabled). Reads the default-limits value when
   * the policy is not `Auto`.
   */
  get reconnectMaxElapsedSecs(): bigint
  /**
   * Set the `ServerRestarting` reconnect attempt budget. Default
   * `60`. No effect unless the reconnect policy is `Auto`.
   */
  setReconnectMaxServerRestartAttempts(n: number): void
  /**
   * Current `ServerRestarting` reconnect attempt budget (default
   * `60`). Reads the default-limits value when the policy is not
   * `Auto`.
   */
  get reconnectMaxServerRestartAttempts(): number
  /**
   * Current reconnect policy as a string (`"auto"`, `"manual"`, or
   * `"custom"`).
   */
  get reconnectPolicy(): string
  /**
   * Current generic-transient reconnect attempt budget (default
   * `30`). Reads the default-limits value when the policy is not
   * `Auto`.
   */
  get reconnectMaxAttempts(): number
  /**
   * Current rate-limited reconnect attempt budget (default `100`).
   * Reads the default-limits value when the policy is not `Auto`.
   */
  get reconnectMaxRateLimitedAttempts(): number
  /**
   * Current stable-window reset interval in seconds (default `60n`).
   * Reads the default-limits value when the policy is not `Auto`.
   */
  get reconnectStableWindowSecs(): bigint
  /**
   * Set the subscription-replay burst size used after an
   * auto-reconnect: frames are written in bursts of this many, each
   * burst flushed and followed by a jittered `replayPaceMs` pause.
   * Minimum `1` (validated at connect). Default `50`.
   */
  setReconnectReplayBurstSize(n: number): void
  /** Current `replay_burst_size` value (default `50`). */
  get reconnectReplayBurstSize(): number
  /**
   * Set the pause (ms) between subscription-replay bursts after an
   * auto-reconnect. `0n` removes the pause. Default `5n`.
   */
  setReconnectReplayPaceMs(ms: bigint): void
  /** Current `replay_pace_ms` value (default `5n`). */
  get reconnectReplayPaceMs(): bigint
  /**
   * Install a custom reconnect policy driven by a JS callback.
   *
   * `callback(reason: number, attempt: number)` is invoked (on the
   * Node main thread, queued from the streaming I/O thread) after
   * each retriable involuntary disconnect. Return the reconnect
   * delay in milliseconds, or `null` to stop reconnecting (the
   * stream then emits the terminal `ReconnectsExhausted` event).
   * Permanent disconnect reasons (bad credentials, account
   * conflicts) never reach the callback. Pass `null` to restore the
   * default `Auto` policy.
   *
   * The streaming I/O thread waits for the decision, so the
   * callback should return promptly; if no decision arrives within
   * 30 seconds (for example because the Node event loop is blocked)
   * the stream stops reconnecting and emits the terminal event.
   */
  setReconnectCallback(callback?: (((arg: ReconnectDecisionArgs) => number | null)) | undefined | null): void
  /** Set the streaming read timeout (ms): the no-frames deadline after which the streaming I/O loop declares the session dead and reconnects. Default `3_000n`; validated to `[100, 60_000]` at connect. */
  setStreamingTimeoutMs(ms: bigint): void
  /** Current `streaming.timeout_ms` value (default `3_000n`). */
  get streamingTimeoutMs(): bigint
  /** Set the per-server connect timeout (ms) for the streaming connection. Default `2_000n`; validated to `[1_000, 60_000]` at connect. */
  setStreamingConnectTimeoutMs(ms: bigint): void
  /** Current `streaming.connect_timeout_ms` value (default `2_000n`). */
  get streamingConnectTimeoutMs(): bigint
  /** Set the streaming heartbeat ping interval (ms). Default `250n`; validated to `[100, 300_000]` at connect. */
  setStreamingPingIntervalMs(ms: bigint): void
  /** Current `streaming.ping_interval_ms` value (default `250n`). */
  get streamingPingIntervalMs(): bigint
  /** Set the per-iteration blocking-read slice (ms) for the streaming I/O loop. Default `25n`; validated to `[10, 500]` at connect. */
  setStreamingIoReadSliceMs(ms: bigint): void
  /** Current `streaming.io_read_slice_ms` value (default `25n`). */
  get streamingIoReadSliceMs(): bigint
  /** Set the last-frame watchdog (ms): when no frame of any kind has arrived for this long the I/O loop force-reconnects. `0n` disables. Default `30_000n`. */
  setStreamingDataWatchdogMs(ms: bigint): void
  /** Current `streaming.data_watchdog_ms` value (default `30_000n`; `0n` = disabled). */
  get streamingDataWatchdogMs(): bigint
  /** Set the TCP keepalive idle time (seconds) before the first kernel probe on a silent streaming socket. Default `5n`; validated to `[1, 7_200]` at connect. */
  setStreamingKeepaliveIdleSecs(ms: bigint): void
  /** Current `streaming.keepalive_idle_secs` value (default `5n`). */
  get streamingKeepaliveIdleSecs(): bigint
  /** Set the interval (seconds) between TCP keepalive probes. Default `2n`; validated to `[1, 75]` at connect. */
  setStreamingKeepaliveIntervalSecs(ms: bigint): void
  /** Current `streaming.keepalive_interval_secs` value (default `2n`). */
  get streamingKeepaliveIntervalSecs(): bigint
  /**
   * Set the number of unanswered TCP keepalive probes after which
   * the kernel declares the streaming connection dead (where the
   * platform exposes the knob). Default `2`; validated to `[1, 10]`
   * at connect.
   */
  setStreamingKeepaliveRetries(n: number): void
  /** Current `streaming.keepalive_retries` value (default `2`). */
  get streamingKeepaliveRetries(): number
  /**
   * Set the streaming event ring buffer size (slots). Must be a power of
   * two `>= 64`; invalid values are rejected immediately. Default
   * `131_072`.
   */
  setStreamingRingSize(n: number): void
  /** Current `streaming.ring_size` value (default `131_072`). */
  get streamingRingSize(): number
  /**
   * Set the streaming host-selection policy. Accepts `"shuffled"`
   * (default — fault-domain-aware per-client shuffle) or
   * `"fixed_order"` (declared order verbatim), case-insensitive.
   */
  setStreamingHostSelection(policy: string): void
  /** Current streaming host-selection policy as a lowercase string. */
  get streamingHostSelection(): string
  /**
   * Set the streaming host-shuffle seed. `null` (default) derives a
   * fresh per-client seed so a fleet shuffles independently; an
   * explicit `bigint` makes the shuffled order deterministic —
   * useful for fleet sharding and tests. Ignored under
   * `"fixed_order"`.
   */
  setStreamingHostShuffleSeed(seed?: bigint | undefined | null): void
  /**
   * Current `streaming.host_shuffle_seed` value (`null` = per-client
   * entropy).
   */
  get streamingHostShuffleSeed(): bigint | null
  /**
   * Set the wall-clock envelope (seconds) for one
   * historical-channel retry sequence, measured from the first
   * attempt. `0n` disables the envelope (attempt budget only).
   * Default `300n`.
   */
  setRetryMaxElapsedSecs(secs: bigint): void
  /**
   * Current `retry.max_elapsed` value in seconds (default `300n`;
   * `0n` = disabled).
   */
  get retryMaxElapsedSecs(): bigint
  /**
   * Toggle AWS-style full jitter on the flatfile retry ladder.
   * Default `true`; `false` gives the deterministic schedule,
   * useful for tests that assert exact timings.
   */
  setFlatfilesJitter(jitter: boolean): void
  /** Current `flatfiles.jitter` value (default `true`). */
  get flatfilesJitter(): boolean
  /**
   * Set the async worker-thread count for embedded runtimes.
   * `hasValue=false` defers to the default sizing; `hasValue=true`
   * pins worker count to `n` (with `n=0` preserved verbatim rather
   * than treated as unset).
   *
   * The async worker pool is process-global: it is built once, from the
   * config of the first client connected in the process. This setting
   * is therefore honored when the first client in the process is
   * created; clients connected later share the already-built pool, so
   * setting it on a subsequent config has no effect.
   */
  setWorkerThreads(hasValue: boolean, n: number): void
  /**
   * Current `workerThreads` setting as `{ hasValue, n }`.
   * `hasValue=false` encodes the unset (auto) sentinel.
   */
  get workerThreads(): WorkerThreadsSetting
  /**
   * Set the initial backoff delay (ms) for the historical-channel retry policy.
   * Default `250n`. Subsequent retries double from here, capped at
   * `retryMaxDelayMs`.
   */
  setRetryInitialDelayMs(ms: bigint): void
  /** Current `retry.initial_delay` value (ms, returned as BigInt). */
  get retryInitialDelayMs(): bigint
  /**
   * Set the upper-bound backoff delay (ms) for the historical retry
   * policy. Default `30_000n` (30 s).
   */
  setRetryMaxDelayMs(ms: bigint): void
  /** Current `retry.max_delay` value (ms, returned as BigInt). */
  get retryMaxDelayMs(): bigint
  /**
   * Set the total attempt budget for the historical-channel retry policy. `1`
   * disables retry; higher values permit retries up to
   * `maxAttempts - 1` after the initial call. Default `20`.
   */
  setRetryMaxAttempts(n: number): void
  /** Current `retry.max_attempts` value. */
  get retryMaxAttempts(): number
  /**
   * Toggle AWS-style full-jitter on the historical-channel retry policy. Default
   * `true`. `false` gives the deterministic backoff schedule
   * `min(max_delay, initial * 2^attempt)`, useful for tests that
   * need to assert exact timings.
   */
  setRetryJitter(jitter: boolean): void
  /** Current `retry.jitter` value. */
  get retryJitter(): boolean
  /**
   * Set the total attempt budget for the flatfile driver retry
   * loop. `1` disables retry (single call only); higher values
   * permit retries up to `maxAttempts - 1` after the initial call.
   * Default `10`. Validated to the range `[1, 100]` at connect time.
   */
  setFlatfilesMaxAttempts(n: number): void
  /** Current `flatfiles.max_attempts` value. */
  get flatfilesMaxAttempts(): number
  /**
   * Set the initial backoff delay (seconds) for the flatfile
   * driver retry loop. Doubles per attempt up to
   * `flatfilesMaxBackoffSecs`. Default `1n`.
   *
   * Accepts a `bigint` for parity with the other bindings, which
   * use a 64-bit unsigned integer.
   */
  setFlatfilesInitialBackoffSecs(secs: bigint): void
  /** Current `flatfiles.initial_backoff` value (seconds, returned as BigInt). */
  get flatfilesInitialBackoffSecs(): bigint
  /**
   * Set the upper-bound backoff delay (seconds) for the flatfile
   * driver retry loop. The doubling schedule never exceeds this
   * value regardless of attempt number. Default `30n`. Must be
   * greater than or equal to `flatfilesInitialBackoffSecs`
   * (rejected at connect-time validate otherwise).
   *
   * Accepts a `bigint` for parity with the other bindings, which
   * use a 64-bit unsigned integer.
   */
  setFlatfilesMaxBackoffSecs(secs: bigint): void
  /** Current `flatfiles.max_backoff` value (seconds, returned as BigInt). */
  get flatfilesMaxBackoffSecs(): bigint
  /**
   * Set the TCP + TLS connect timeout (seconds) for one flatfile-host
   * attempt. Bounds the connect/auth handshake before the attempt is
   * abandoned and the next host (or the retry ladder) takes over.
   * Default `10n`.
   *
   * Accepts a `bigint` for parity with the other bindings, which
   * use a 64-bit unsigned integer.
   */
  setFlatfilesConnectTimeoutSecs(secs: bigint): void
  /** Current `flatfiles.connect_timeout_secs` value (seconds, returned as BigInt). */
  get flatfilesConnectTimeoutSecs(): bigint
  /**
   * Set the read timeout (seconds) for a single flatfile response
   * frame. Bounds the wait for the next chunk once streaming has begun
   * so a mid-stream stall fails over instead of blocking forever.
   * Default `60n`.
   *
   * Accepts a `bigint` for parity with the other bindings, which
   * use a 64-bit unsigned integer.
   */
  setFlatfilesReadTimeoutSecs(secs: bigint): void
  /** Current `flatfiles.read_timeout_secs` value (seconds, returned as BigInt). */
  get flatfilesReadTimeoutSecs(): bigint
  /**
   * Set the Nexus auth URL. Default matches the upstream
   * production endpoint; override to redirect at a staging
   * cluster for testing.
   */
  setNexusUrl(url: string): void
  /** Current `auth.nexus_url` value. */
  get nexusUrl(): string
  /**
   * Set the `QueryInfo.client_type` identifier. Default is
   * `"rust-thetadatadx"`; override to identify a deployment fleet
   * in server-side dashboards.
   */
  setClientType(clientType: string): void
  /** Current `auth.client_type` value. */
  get clientType(): string
  /** Override the historical gRPC host. Companion to `setHistoricalPort`. */
  setHistoricalHost(host: string): void
  /** Current historical gRPC host. */
  get historicalHost(): string
  /**
   * Override the historical data port. Companion to `setHistoricalHost` —
   * same test-only rationale. Rejects values outside the `0..=65535`
   * port range.
   */
  setHistoricalPort(port: number): void
  /** Current historical gRPC port. */
  get historicalPort(): number
  /**
   * Set the Prometheus exporter port. Pass `null` or `undefined`
   * to leave the exporter disabled (the default); pass a
   * `number` to bind an HTTP listener on `0.0.0.0:<port>` when the
   * `metrics-prometheus` feature is compiled in.
   *
   * Rejects values outside the `0..=65535` port range.
   */
  setMetricsPort(port?: number | undefined | null): void
  /**
   * Current `metrics.port` setting. `null` means the exporter is
   * disabled; a `number` is the bound port.
   */
  get metricsPort(): number | null
  /**
   * Set the streaming write-flush policy.
   *
   * Accepts `"batched"` (default — flushes on the PING heartbeat,
   * roughly every 100 ms — best throughput) or `"immediate"`
   * (flushes after every wire write — lowest latency, higher
   * per-frame syscall cost).
   */
  setFlushMode(mode: string): void
  /**
   * Current streaming write-flush policy (`"batched"` or
   * `"immediate"`).
   */
  get flushMode(): string
  /**
   * Set the streaming event-ring consumer wait strategy — the
   * latency-vs-CPU knob applied on each ring-empty poll.
   *
   * Accepts `"low_latency"` (default — never sleeps, lowest latency,
   * highest idle CPU), `"balanced"` (brief park — low idle CPU),
   * `"efficient"` (longer park — lowest idle CPU), or `"busy_spin"`
   * (pure spin — pins a core). Tune the spin / yield / park counts via
   * `setWaitSpinIters` / `setWaitYieldIters` / `setWaitParkUs`.
   */
  setWaitStrategy(strategy: string): void
  /**
   * Current streaming wait strategy (`"low_latency"`, `"balanced"`,
   * `"efficient"`, or `"busy_spin"`).
   */
  get waitStrategy(): string
  /** Set the wait-strategy spin iteration count. */
  setWaitSpinIters(iters: number): void
  /** Current wait-strategy spin iteration count. */
  get waitSpinIters(): number
  /** Set the wait-strategy yield iteration count. */
  setWaitYieldIters(iters: number): void
  /** Current wait-strategy yield iteration count. */
  get waitYieldIters(): number
  /**
   * Set the wait-strategy park interval in microseconds (used by the
   * `"balanced"` / `"efficient"` strategies).
   */
  setWaitParkUs(parkUs: number): void
  /** Current wait-strategy park interval in microseconds. */
  get waitParkUs(): number
  /**
   * Pin the streaming consumer thread to a CPU core, or `null` to
   * leave it under the OS scheduler (default).
   *
   * Pinning the tick-consumer thread to an isolated core gives
   * deterministic, low-jitter delivery. An out-of-range or offline
   * core is a best-effort no-op rather than an error.
   */
  setConsumerCpu(core?: number | undefined | null): void
  /** Current streaming consumer-thread CPU pin, or `null` if unpinned. */
  get consumerCpu(): number | null
  /**
   * Set whether to derive OHLCVC bars locally from trade events.
   * When `false`, only server-sent OHLCVC frames are emitted,
   * reducing per-trade throughput overhead. Default `true`.
   */
  setDeriveOhlcvc(enabled: boolean): void
  /** Current OHLCVC derivation setting. */
  get deriveOhlcvc(): boolean
}

/**
 * Fluent contract identifier — stock or option.
 *
 * Use `Contract.stock("AAPL")` / `Contract.option(...)` to build one.
 * The class is also exported under the name `ContractRef`; `Contract`
 * is an alias for it, so the two names are interchangeable.
 */
export declare class ContractRef {
  /** Construct a stock contract. */
  static stock(symbol: string): ContractRef
  /** Construct an index contract. */
  static index(symbol: string): ContractRef
  /**
   * Construct an option contract. The expiration / strike / right
   * travel in a single `OptionLeg` object with named keys —
   * `Contract.option("SPY", { expiration: "20260620", strike: "550",
   * right: "C" })` — rather than as adjacent positional strings, so a
   * swapped expiration/strike/right pair cannot pass silently. `right`
   * accepts `"C"` / `"CALL"` / `"P"` / `"PUT"` (case-insensitive);
   * `strike` is the price in dollars as a number or string (`550`,
   * `550.5`, and `"550"` are equivalent).
   */
  static option(symbol: string, leg: OptionLeg): ContractRef
  /** Per-contract Quote subscription. */
  quote(): Subscription
  /** Per-contract Trade subscription. */
  trade(): Subscription
  /** Per-contract OpenInterest subscription. */
  openInterest(): Subscription
  /** Per-contract market-value subscription. */
  marketValue(): Subscription
  /** Underlying symbol (e.g. `"AAPL"`, `"SPY"`). */
  get symbol(): string
  /**
   * Security type as an upper-case string (`"STOCK"`, `"OPTION"`,
   * `"INDEX"`).
   */
  get secType(): string
  /** Expiration date as a `YYYYMMDD` integer; `null` for non-options. */
  get expiration(): number | null
  /**
   * Strike price in dollars; `null` for non-options. Reads back the
   * same notation `Contract.option(.., strike, ..)` takes, and joins
   * directly against historical-row `strike` columns.
   */
  get strike(): number | null
  /** Option right (`"C"` / `"P"`); `null` for non-options. */
  get right(): string | null
  /**
   * String rendering for `console.log` / template literals, e.g.
   * `"SPY OPTION 20260620 C 550"` or `"AAPL STOCK"`. The strike reads
   * in dollars, matching the `strike` getter. Delegates to
   * the same core rendering the Python `Contract` `__str__` uses, so
   * the two bindings print a contract identically. Without it a
   * `ContractRef` prints as an opaque `ContractRef {}` because its
   * getters do not surface on inspection.
   */
  toString(): string
}

/**
 * ThetaData login credentials.
 *
 * Build from an email + password pair (`new Credentials(email,
 * password)`) or load from a credentials file (`Credentials.fromFile`,
 * line 1 = email, line 2 = password), then pass the handle to a client
 * `connect(creds, config?)`.
 *
 * ```js
 * const { Credentials, Client } = require("@thetadatadx/sdk");
 * const creds = Credentials.fromFile("creds.txt");
 * const client = Client.connect(creds);
 * ```
 */
export declare class Credentials {
  /** Create credentials from an email and password. */
  constructor(email: string, password: string)
  /** Load credentials from a file (line 1 = email, line 2 = password). */
  static fromFile(path: string): Credentials
  /** Redacted string form — never exposes the email or password. */
  toString(): string
}

/**
 * JS class wrapping a decoded flat-file row vector. Created by every
 * method on `FlatFilesNamespace`; carries the typed
 * rows until the user picks a terminal.
 */
export declare class FlatFileRowList {
  /**
   * Number of decoded rows. Same value as `.length` on the JSON
   * representation, exposed as a method so the API stays stable if
   * the list later gains first-class iterator support.
   */
  len(): number
  /** Whether the decoded row vector is empty. */
  isEmpty(): boolean
  /**
   * Serialise the typed rows as Arrow IPC stream bytes. The dynamic
   * schema is inferred from the first row. Deserialise on
   * the JS side with `apache-arrow`'s `tableFromIPC`.
   */
  toArrowIpc(): Buffer
  /**
   * Return a JSON array of objects, one per row. Useful for quick
   * inspection, structured logging, or wiring into JS-side
   * dataframes that don't read Arrow IPC.
   */
  toJson(): string
}

/**
 * JS class returned from `client.flatFiles`. Each method maps to one
 * (security type, request type) pair and returns a `FlatFileRowList`.
 */
export declare class FlatFilesNamespace {
  /** Option trade-with-quote flat file for the given `YYYYMMDD` date. */
  optionTradeQuote(date: string): FlatFileRowList
  /** Option open-interest flat file for the given `YYYYMMDD` date. */
  optionOpenInterest(date: string): FlatFileRowList
  /** Option end-of-day flat file for the given `YYYYMMDD` date. */
  optionEod(date: string): FlatFileRowList
  /** Stock trade-with-quote flat file for the given `YYYYMMDD` date. */
  stockTradeQuote(date: string): FlatFileRowList
  /** Stock end-of-day flat file for the given `YYYYMMDD` date. */
  stockEod(date: string): FlatFileRowList
  /**
   * Generic dispatcher — `secType` and `reqType` accept `"OPTION"` /
   * `"QUOTE"` style strings.
   */
  request(secType: string, reqType: string, date: string): FlatFileRowList
}

/**
 * Standalone historical-only client.
 *
 * Opens ONLY the historical data channel and the Nexus authentication
 * flow — no real-time streaming connection or streaming state machine.
 * This lets a caller run a historical-only session alongside a parallel
 * streaming process without the unified `Client` taking over
 * the Nexus session at connect time.
 *
 * The full historical / list / snapshot / at-time / flat-files surface
 * is identical to the unified client, so `historicalClient.stockHistoryEod(...)`
 * behaves exactly like `client.stockHistoryEod(...)`. The streaming and
 * subscription methods are simply not present: there is no
 * `startStreaming` / `subscribe` on this class, so a historical-only handle
 * cannot open a streaming slot. Use `StreamingClient` for streaming, or the
 * unified `Client` when you need both surfaces.
 *
 * ```js
 * const { HistoricalClient, Config } = require("@thetadatadx/sdk");
 * const historical = HistoricalClient.connectFromFile("creds.txt");
 * const eod = await historical.stockHistoryEod("AAPL", "20240101", "20240301");
 * ```
 */
export declare class HistoricalClient {
  /**
   * Connect to ThetaData with a `Credentials` handle and open the
   * historical data channel. Historical only — this client never
   * opens the FPSS streaming transport. Pass an optional `Config` to
   * override the production-default endpoint. Use `StreamingClient` for
   * real-time data.
   *
   * The config is snapshot at connect time: the `Config` handle may be
   * reused or mutated afterward without affecting this client.
   */
  static connect(creds: Credentials, config?: Config | undefined | null): HistoricalClient
  /**
   * Connect with a credentials file (line 1 = email, line 2 =
   * password). Convenience wrapper over `Credentials.fromFile` +
   * `connect`. Historical only. Pass an optional
   * `Config` to override the production-default endpoint.
   */
  static connectFromFile(path: string, config?: Config | undefined | null): HistoricalClient
  /**
   * List all available stock ticker symbols.
   *
   * A symbol can be defined as a unique identifier for a stock / underlying asset. Common terms also include: root, ticker, and underlying. This endpoint returns all traded symbols for stocks. This endpoint is updated overnight.
   */
  stockListSymbols(options?: StockListSymbolsOptions | undefined | null): Promise<Array<string>>
  /**
   * List available dates for a stock by request type (EOD, TRADE, QUOTE, etc.).
   *
   * Lists all dates of data that are available for a stock with a given request type and symbol. This endpoint is updated overnight.
   */
  stockListDates(requestType: string, symbol: string, options?: StockListDatesOptions | undefined | null): Promise<Array<string>>
  /**
   * Get the latest OHLC snapshot for one or more stocks.
   *
   * Provides a real-time Open, High, Low, Close for the current day.
   * * Returns a real-time session OHLC from the Nasdaq Basic feed if the account has a stocks standard or pro subscription.
   * * Returns a 15-minute delayed session OHLC from the UTP & CTA feeds if the account has the stocks value subscription.
   * * Theta Data resets its snapshot cache at midnight ET every day. This endpoint may not work on a weekend where there were no eligible messages sent over exchange feeds. We recommend using historic requests during the weekend.
   *
   * Defaults (upstream):
   * - `venue`: `"nqb"`
   */
  stockSnapshotOHLC(symbols: string | Array<string>, options?: StockSnapshotOhlcOptions | undefined | null): Promise<Array<OhlcTick>>
  /**
   * Get the latest trade snapshot for one or more stocks.
   *
   * Returns a real-time last trade from the Nasdaq Basic feed if the account has a stocks standard or pro subscription.
   *
   * - Theta Data resets its snapshot cache at midnight ET every day. This endpoint may not work on a weekend where there were no eligible messages sent over exchange feeds. We recommend using historic requests during the weekend.
   *
   * Defaults (upstream):
   * - `venue`: `"nqb"`
   */
  stockSnapshotTrade(symbols: string | Array<string>, options?: StockSnapshotTradeOptions | undefined | null): Promise<Array<TradeTick>>
  /**
   * Get the latest NBBO quote snapshot for one or more stocks.
   *
   * * Returns a real-time last BBO quote from the Nasdaq Basic feed if the account has a stocks standard or pro subscription.
   * * Returns a 15-minute delayed NBBO quote from the UTP & CTA feeds account has the stocks value subscription subscription.
   * - Theta Data resets its snapshot cache at midnight ET every day. This endpoint may not work on a weekend where there were no eligible messages sent over exchange feeds. We recommend using historic requests during the weekend.
   *
   * Defaults (upstream):
   * - `venue`: `"nqb"`
   */
  stockSnapshotQuote(symbols: string | Array<string>, options?: StockSnapshotQuoteOptions | undefined | null): Promise<Array<QuoteTick>>
  /**
   * Get the latest market value snapshot for one or more stocks.
   *
   * * Returns a real-time market value derived from the last BBO quote from the Nasdaq Basic feed if the account has a stocks standard or pro subscription.
   * * Returns a 15-minute delayed market value derived from an NBBO quote from the UTP & CTA feeds if the account has the stocks value subscription subscription.
   * - Theta Data resets its snapshot cache at midnight ET every day. This endpoint may not work on a weekend where there were no eligible messages sent over exchange feeds. We recommend using historic requests during the weekend.
   *
   * Defaults (upstream):
   * - `venue`: `"nqb"`
   */
  stockSnapshotMarketValue(symbols: string | Array<string>, options?: StockSnapshotMarketValueOptions | undefined | null): Promise<Array<MarketValueTick>>
  /**
   * Fetch end-of-day stock data for a date range. Returns OHLCV + bid/ask per trading day.
   *
   * Since the equity SIPs only generate a partial EOD report, Theta Data generates a national EOD report at 17:15 ET each day. ``created`` represents the datetime the report was generated and ``last_trade`` represents the datetime of the last trade. The quote in the response represents the last NBBO reported by CTA or UTP at the time of report generation. You can read more about EOD & OHLC data here. Theta Data plans to avail SIP EOD reports in the near future.
   */
  stockHistoryEOD(symbol: string, startDate: string | Date, endDate: string | Date, options?: StockHistoryEodOptions | undefined | null): Promise<Array<EodTick>>
  /** Stream `stock_history_eod` rows into `callback` without materialising the full response in memory. `callback(chunk: EodTick[]) => void` is invoked once per server chunk; the chunk is freed before the next is fetched, so peak memory tracks a single chunk rather than the whole result. This is the memory-bounded companion to the `stockHistoryEOD` method — prefer it for multi-day or full-universe pulls. The returned Promise resolves when the stream drains and rejects (typed like the buffered method) on a wire or decode error. Cancelling the Promise drops the in-flight request. `options` carries the same optional builder parameters and `timeoutMs` as the buffered method; the `callback` is the trailing argument. */
  stockHistoryEODStream(symbol: string, startDate: string | Date, endDate: string | Date, options: StockHistoryEodOptions | undefined | null, callback: ((arg: Array<EodTick>) => void)): Promise<void>
  /**
   * Fetch intraday OHLC bars for a stock on a single date.
   *
   * - Aggregated OHLC bars that use SIP rules for each bar. Time timestamp of the bar represents the opening time of the bar. For a trade to be part of the bar:  ``bar time`` <= ``trade time`` < ``bar timestamp + ivl``, where ivl is the specified interval size in milliseconds.
   * - Set the ``venue`` parameter to ``nqb`` to access current-day real-time historic data from the Nasdaq Basic feed if the account has a stocks standard or pro subscription.
   * - Multi-day requests are limited to 1 month of data.
   *
   * Defaults (upstream):
   * - `interval`: `"1s"`
   * - `start_time`: `"09:30:00"`
   * - `end_time`: `"16:00:00"`
   * - `venue`: `"nqb"`
   */
  stockHistoryOHLC(symbol: string, date: string | Date, options?: StockHistoryOhlcOptions | undefined | null): Promise<Array<OhlcTick>>
  /** Stream `stock_history_ohlc` rows into `callback` without materialising the full response in memory. `callback(chunk: OhlcTick[]) => void` is invoked once per server chunk; the chunk is freed before the next is fetched, so peak memory tracks a single chunk rather than the whole result. This is the memory-bounded companion to the `stockHistoryOHLC` method — prefer it for multi-day or full-universe pulls. The returned Promise resolves when the stream drains and rejects (typed like the buffered method) on a wire or decode error. Cancelling the Promise drops the in-flight request. `options` carries the same optional builder parameters and `timeoutMs` as the buffered method; the `callback` is the trailing argument. */
  stockHistoryOHLCStream(symbol: string, date: string | Date, options: StockHistoryOhlcOptions | undefined | null, callback: ((arg: Array<OhlcTick>) => void)): Promise<void>
  /**
   * Fetch all trades for a stock on a given date.
   *
   * Returns every trade reported by UTP & CTA. Set the ``venue`` parameter to ``nqb`` to access current-day real-time historic data from the Nasdaq Basic feed if the account has a stocks standard or pro subscription.
   * - Multi-day requests are limited to 1 month of data.
   *
   * Defaults (upstream):
   * - `start_time`: `"09:30:00"`
   * - `end_time`: `"16:00:00"`
   * - `venue`: `"nqb"`
   */
  stockHistoryTrade(symbol: string, date: string | Date, options?: StockHistoryTradeOptions | undefined | null): Promise<Array<TradeTick>>
  /** Stream `stock_history_trade` rows into `callback` without materialising the full response in memory. `callback(chunk: TradeTick[]) => void` is invoked once per server chunk; the chunk is freed before the next is fetched, so peak memory tracks a single chunk rather than the whole result. This is the memory-bounded companion to the `stockHistoryTrade` method — prefer it for multi-day or full-universe pulls. The returned Promise resolves when the stream drains and rejects (typed like the buffered method) on a wire or decode error. Cancelling the Promise drops the in-flight request. `options` carries the same optional builder parameters and `timeoutMs` as the buffered method; the `callback` is the trailing argument. */
  stockHistoryTradeStream(symbol: string, date: string | Date, options: StockHistoryTradeOptions | undefined | null, callback: ((arg: Array<TradeTick>) => void)): Promise<void>
  /**
   * Fetch NBBO quotes for a stock on a given date at a given interval.
   *
   * - Returns every NBBO quote reported by UTP and CTA.
   * - If the ``interval`` parameter is specified, the quote for each interval represents the last quote prior to the interval's timestamp.
   * - Set the ``venue`` parameter to ``nqb`` to access current-day real-time historic data from the Nasdaq Basic feed if the account has a stocks standard or pro subscription.
   * - Multi-day requests are limited to 1 month of data.
   *
   * Defaults (upstream):
   * - `interval`: `"1s"`
   * - `start_time`: `"09:30:00"`
   * - `end_time`: `"16:00:00"`
   * - `venue`: `"nqb"`
   */
  stockHistoryQuote(symbol: string, date: string | Date, options?: StockHistoryQuoteOptions | undefined | null): Promise<Array<QuoteTick>>
  /** Stream `stock_history_quote` rows into `callback` without materialising the full response in memory. `callback(chunk: QuoteTick[]) => void` is invoked once per server chunk; the chunk is freed before the next is fetched, so peak memory tracks a single chunk rather than the whole result. This is the memory-bounded companion to the `stockHistoryQuote` method — prefer it for multi-day or full-universe pulls. The returned Promise resolves when the stream drains and rejects (typed like the buffered method) on a wire or decode error. Cancelling the Promise drops the in-flight request. `options` carries the same optional builder parameters and `timeoutMs` as the buffered method; the `callback` is the trailing argument. */
  stockHistoryQuoteStream(symbol: string, date: string | Date, options: StockHistoryQuoteOptions | undefined | null, callback: ((arg: Array<QuoteTick>) => void)): Promise<void>
  /**
   * Fetch combined trade + quote ticks for a stock on a given date. Returns raw DataTable.
   *
   * Returns every trade reported by UTP & CTA paired with the last BBO quote reported by UTP or CTA at the time of trade. A quote is matched with a trade if its timestamp ``<=`` the trade timestamp. If you prefer to match quotes with timestamps that are ``<`` the trade timestamp, specify the ``exclusive`` parameter to ``true``. Set the ``venue`` parameter to ``nqb`` to access current-day real-time historic data from the Nasdaq Basic feed if the account has a stocks standard or pro subscription.
   * - Multi-day requests are limited to 1 month of data.
   *
   * Defaults (upstream):
   * - `start_time`: `"09:30:00"`
   * - `end_time`: `"16:00:00"`
   * - `exclusive`: `true`
   * - `venue`: `"nqb"`
   */
  stockHistoryTradeQuote(symbol: string, date: string | Date, options?: StockHistoryTradeQuoteOptions | undefined | null): Promise<Array<TradeQuoteTick>>
  /** Stream `stock_history_trade_quote` rows into `callback` without materialising the full response in memory. `callback(chunk: TradeQuoteTick[]) => void` is invoked once per server chunk; the chunk is freed before the next is fetched, so peak memory tracks a single chunk rather than the whole result. This is the memory-bounded companion to the `stockHistoryTradeQuote` method — prefer it for multi-day or full-universe pulls. The returned Promise resolves when the stream drains and rejects (typed like the buffered method) on a wire or decode error. Cancelling the Promise drops the in-flight request. `options` carries the same optional builder parameters and `timeoutMs` as the buffered method; the `callback` is the trailing argument. */
  stockHistoryTradeQuoteStream(symbol: string, date: string | Date, options: StockHistoryTradeQuoteOptions | undefined | null, callback: ((arg: Array<TradeQuoteTick>) => void)): Promise<void>
  /**
   * Fetch the trade at a specific time of day across a date range.
   *
   * #### Real-time request:
   * - Returns a real-time session from the Nasdaq Basic feed if the account has a stocks standard or pro subscription.
   * - Returns a 15-minute delayed session from the UTP & CTA feeds account has the stocks value subscription subscription.
   *
   * #### Historical request:
   * Returns the last trade reported by UTP & CTA feeds at a specified millisecond of the day.
   * Trade condition mappings can be found here.
   *
   * Defaults (upstream):
   * - `venue`: `"nqb"`
   */
  stockAtTimeTrade(symbol: string, startDate: string | Date, endDate: string | Date, timeOfDay: string | Date, options?: StockAtTimeTradeOptions | undefined | null): Promise<Array<TradeTick>>
  /** Stream `stock_at_time_trade` rows into `callback` without materialising the full response in memory. `callback(chunk: TradeTick[]) => void` is invoked once per server chunk; the chunk is freed before the next is fetched, so peak memory tracks a single chunk rather than the whole result. This is the memory-bounded companion to the `stockAtTimeTrade` method — prefer it for multi-day or full-universe pulls. The returned Promise resolves when the stream drains and rejects (typed like the buffered method) on a wire or decode error. Cancelling the Promise drops the in-flight request. `options` carries the same optional builder parameters and `timeoutMs` as the buffered method; the `callback` is the trailing argument. */
  stockAtTimeTradeStream(symbol: string, startDate: string | Date, endDate: string | Date, timeOfDay: string | Date, options: StockAtTimeTradeOptions | undefined | null, callback: ((arg: Array<TradeTick>) => void)): Promise<void>
  /**
   * Fetch the quote at a specific time of day across a date range.
   *
   * #### Real-time request:
   *   - Subscription tier standard or higher will default to NQB.
   *   - Real-time last BBO quote at-time_of_day-time from the Nasdaq Basic feed if the account has a stocks standard or pro subscription.
   *   - 15-minute delayed NBBO quote at-time_of_day-time from the UTP & CTA feeds account has the stocks value subscription subscription.
   *
   * #### Historical request:
   *   Returns the last NBBO quote reported by UTP & CTA feeds at a specified millisecond of the day.
   *
   * Defaults (upstream):
   * - `venue`: `"nqb"`
   */
  stockAtTimeQuote(symbol: string, startDate: string | Date, endDate: string | Date, timeOfDay: string | Date, options?: StockAtTimeQuoteOptions | undefined | null): Promise<Array<QuoteTick>>
  /** Stream `stock_at_time_quote` rows into `callback` without materialising the full response in memory. `callback(chunk: QuoteTick[]) => void` is invoked once per server chunk; the chunk is freed before the next is fetched, so peak memory tracks a single chunk rather than the whole result. This is the memory-bounded companion to the `stockAtTimeQuote` method — prefer it for multi-day or full-universe pulls. The returned Promise resolves when the stream drains and rejects (typed like the buffered method) on a wire or decode error. Cancelling the Promise drops the in-flight request. `options` carries the same optional builder parameters and `timeoutMs` as the buffered method; the `callback` is the trailing argument. */
  stockAtTimeQuoteStream(symbol: string, startDate: string | Date, endDate: string | Date, timeOfDay: string | Date, options: StockAtTimeQuoteOptions | undefined | null, callback: ((arg: Array<QuoteTick>) => void)): Promise<void>
  /**
   * List all available option underlying symbols.
   *
   * A symbol can be defined as a unique identifier for a stock / underlying asset. Common terms also include: root, ticker, and underlying. This endpoint returns all traded symbols for options. This endpoint is updated overnight.
   */
  optionListSymbols(options?: OptionListSymbolsOptions | undefined | null): Promise<Array<string>>
  /**
   * List available dates for an option contract by request type.
   *
   * Lists all dates of data that are available for an option with a given symbol, request type, and expiration.
   * This endpoint is updated overnight.
   *
   * Defaults (upstream):
   * - `strike`: `"*"`
   * - `right`: `"both"`
   */
  optionListDates(requestType: string, symbol: string, expiration: string | Date, options?: OptionListDatesOptions | undefined | null): Promise<Array<string>>
  /**
   * List available expiration dates for an option underlying.
   *
   * Lists all dates of expirations that are available for an option with a given symbol.
   * This endpoint is updated overnight.
   */
  optionListExpirations(symbol: string, options?: OptionListExpirationsOptions | undefined | null): Promise<Array<string>>
  /**
   * List available strike prices for an option at a given expiration.
   *
   * Lists all strikes that are available for an option with a given symbol and expiration date.
   * This endpoint is updated overnight.
   */
  optionListStrikes(symbol: string, expiration: string | Date, options?: OptionListStrikesOptions | undefined | null): Promise<Array<string>>
  /**
   * List all option contracts for a symbol on a given date.
   *
   * Lists all contracts that were traded or quoted on a particular date.
   *
   * If the ``symbol`` parameter is specified, the returned contracts will be filtered to match the symbol.
   * Multiple symbols can be specified by separating them with commas such as ``symbol=AAPL,SPY,AMD``
   * This endpoint is updated real-time.
   */
  optionListContracts(requestType: string, symbol: string, date: string | Date, options?: OptionListContractsOptions | undefined | null): Promise<Array<OptionContract>>
  /** Stream `option_list_contracts` rows into `callback` without materialising the full response in memory. `callback(chunk: OptionContract[]) => void` is invoked once per server chunk; the chunk is freed before the next is fetched, so peak memory tracks a single chunk rather than the whole result. This is the memory-bounded companion to the `optionListContracts` method — prefer it for multi-day or full-universe pulls. The returned Promise resolves when the stream drains and rejects (typed like the buffered method) on a wire or decode error. Cancelling the Promise drops the in-flight request. `options` carries the same optional builder parameters and `timeoutMs` as the buffered method; the `callback` is the trailing argument. */
  optionListContractsStream(requestType: string, symbol: string, date: string | Date, options: OptionListContractsOptions | undefined | null, callback: ((arg: Array<OptionContract>) => void)): Promise<void>
  /**
   * Get the latest OHLC snapshot for an option contract.
   *
   * - Retrieve a real-time last ohlc of an option contract for the trading day.
   * - You might need to change the default expiration date to a different date if it is past the current date.
   *
   * Defaults (upstream):
   * - `strike`: `"*"`
   * - `right`: `"both"`
   */
  optionSnapshotOHLC(symbol: string, expiration: string | Date, options?: OptionSnapshotOhlcOptions | undefined | null): Promise<Array<OhlcTick>>
  /**
   * Get the latest trade snapshot for an option contract.
   *
   * - Retrieve the real-time last trade of an option contract.
   * - You might need to change the default expiration date to a different date if it is past the current date.
   * - This endpoint will return no data if the market was closed for the day. Theta Data resets the snapshot cache at midnight ET every night.
   *
   * Defaults (upstream):
   * - `strike`: `"*"`
   * - `right`: `"both"`
   */
  optionSnapshotTrade(symbol: string, expiration: string | Date, options?: OptionSnapshotTradeOptions | undefined | null): Promise<Array<TradeTick>>
  /**
   * Get the latest NBBO quote snapshot for an option contract.
   *
   * - Retrieve a real-time last NBBO quote of an option contract.
   * - You might need to change the default expiration date to a different date if it is past the current date.
   * - This endpoint will return no data if the market was closed for the day. Theta Data resets the snapshot cache at midnight ET every night.
   *
   * Defaults (upstream):
   * - `strike`: `"*"`
   * - `right`: `"both"`
   */
  optionSnapshotQuote(symbol: string, expiration: string | Date, options?: OptionSnapshotQuoteOptions | undefined | null): Promise<Array<QuoteTick>>
  /**
   * Get the latest open interest snapshot for an option contract.
   *
   * - Retrieve the last open interest message of an option contract.
   * - Open interest is reported around 06:30 ET every morning by OPRA and reflects the open interest at the end of the previous trading day.
   * - You might need to change the default expiration date to a different date if it is past the current date.
   * - This endpoint will return no data if the market was closed for the day. Theta Data resets the snapshot cache at midnight ET every night.
   *
   * Defaults (upstream):
   * - `strike`: `"*"`
   * - `right`: `"both"`
   */
  optionSnapshotOpenInterest(symbol: string, expiration: string | Date, options?: OptionSnapshotOpenInterestOptions | undefined | null): Promise<Array<OpenInterestTick>>
  /**
   * Get the latest market value snapshot for an option contract.
   *
   * * Returns a real-time market value derived from the last NBBO quote of an option contract.
   *
   * Defaults (upstream):
   * - `strike`: `"*"`
   * - `right`: `"both"`
   */
  optionSnapshotMarketValue(symbol: string, expiration: string | Date, options?: OptionSnapshotMarketValueOptions | undefined | null): Promise<Array<MarketValueTick>>
  /**
   * Get implied volatility snapshot for an option contract (from ThetaData server).
   *
   * Returns implied volatilies calculated using the national best bid, mid, and ask price
   * of the option respectively. The underlying price represents whatever the last underlying price was at the
   * ``underlying_timestamp`` field. You can read more about how Theta Data calculates greeks
   * here.
   *
   * Defaults (upstream):
   * - `strike`: `"*"`
   * - `right`: `"both"`
   * - `rate_type`: `"sofr"`
   * - `version`: `"latest"`
   * - `use_market_value`: `false`
   */
  optionSnapshotGreeksImpliedVolatility(symbol: string, expiration: string | Date, options?: OptionSnapshotGreeksImpliedVolatilityOptions | undefined | null): Promise<Array<IvTick>>
  /**
   * Get all Greeks snapshot for an option contract (from ThetaData server).
   *
   * - Retrieve a real-time last greeks calculation for all option contracts that lie on a provided expiration.
   * - You might need to change the default expiration date to a different date if it is past the current date. Some quotes are omitted in the example to reduce the space of the sample output.
   * - Make `expiration` * if you want to get the snapshot for every expiration chain for the underlying.
   * > This endpoint will return no data if the market was closed for the day. Theta Data resets the snapshot cache at midnight ET every night.
   *
   * Defaults (upstream):
   * - `strike`: `"*"`
   * - `right`: `"both"`
   * - `rate_type`: `"sofr"`
   * - `version`: `"latest"`
   * - `use_market_value`: `false`
   */
  optionSnapshotGreeksAll(symbol: string, expiration: string | Date, options?: OptionSnapshotGreeksAllOptions | undefined | null): Promise<Array<GreeksAllTick>>
  /**
   * Get first-order Greeks snapshot (delta, theta, rho) for an option contract.
   *
   * - Retrieve a real-time last greeks calculation for all option contracts that lie on a provided expiration.
   * - You might need to change the default expiration date to a different date if it is past the current date. Some quotes are omitted in the example to reduce the space of the sample output.
   * - Make `expiration` * if you want to get the snapshot for every expiration chain for the underlying.
   * > This endpoint will return no data if the market was closed for the day. Theta Data resets the snapshot cache at midnight ET every night.
   *
   * Defaults (upstream):
   * - `strike`: `"*"`
   * - `right`: `"both"`
   * - `rate_type`: `"sofr"`
   * - `version`: `"latest"`
   * - `use_market_value`: `false`
   */
  optionSnapshotGreeksFirstOrder(symbol: string, expiration: string | Date, options?: OptionSnapshotGreeksFirstOrderOptions | undefined | null): Promise<Array<GreeksFirstOrderTick>>
  /**
   * Get second-order Greeks snapshot (gamma, vanna, charm) for an option contract.
   *
   * - Retrieve a real-time last second order greeks calculation for all option contracts that lie on a provided expiration.
   * - You might need to change the default expiration date to a different date if it is past the current date. Some quotes are omitted in the example to reduce the space of the sample output.
   * - Make `expiration` * if you want to get the snapshot for every expiration chain for the underlying.
   * > This endpoint will return no data if the market was closed for the day. Theta Data resets the snapshot cache at midnight ET every night.
   *
   * Defaults (upstream):
   * - `strike`: `"*"`
   * - `right`: `"both"`
   * - `rate_type`: `"sofr"`
   * - `version`: `"latest"`
   * - `use_market_value`: `false`
   */
  optionSnapshotGreeksSecondOrder(symbol: string, expiration: string | Date, options?: OptionSnapshotGreeksSecondOrderOptions | undefined | null): Promise<Array<GreeksSecondOrderTick>>
  /**
   * Get third-order Greeks snapshot (speed, color, ultima) for an option contract.
   *
   * - Retrieve a real-time last third order greeks calculation for all option contracts that lie on a provided expiration.
   * - You might need to change the default expiration date to a different date if it is past the current date. Some quotes are omitted in the example to reduce the space of the sample output.
   * - Make `expiration` * if you want to get the snapshot for every expiration chain for the underlying.
   * > This endpoint will return no data if the market was closed for the day. Theta Data resets the snapshot cache at midnight ET every night.
   *
   * Defaults (upstream):
   * - `strike`: `"*"`
   * - `right`: `"both"`
   * - `rate_type`: `"sofr"`
   * - `version`: `"latest"`
   * - `use_market_value`: `false`
   */
  optionSnapshotGreeksThirdOrder(symbol: string, expiration: string | Date, options?: OptionSnapshotGreeksThirdOrderOptions | undefined | null): Promise<Array<GreeksThirdOrderTick>>
  /**
   * Fetch end-of-day option data for a contract over a date range.
   *
   * - Since OPRA does not provide a national EOD report for options, Theta Data generates a national EOD report at 17:15 ET each day.
   * - ``created`` represents the datetime the report was generated and ``last_trade`` represents the datetime of the last trade.
   * - The quote in the response represents the last NBBO reported by OPRA at the time of report generation.
   * - You can read more about EOD & OHLC data here.
   *
   * Defaults (upstream):
   * - `strike`: `"*"`
   * - `right`: `"both"`
   */
  optionHistoryEOD(symbol: string, expiration: string | Date, startDate: string | Date, endDate: string | Date, options?: OptionHistoryEodOptions | undefined | null): Promise<Array<EodTick>>
  /** Stream `option_history_eod` rows into `callback` without materialising the full response in memory. `callback(chunk: EodTick[]) => void` is invoked once per server chunk; the chunk is freed before the next is fetched, so peak memory tracks a single chunk rather than the whole result. This is the memory-bounded companion to the `optionHistoryEOD` method — prefer it for multi-day or full-universe pulls. The returned Promise resolves when the stream drains and rejects (typed like the buffered method) on a wire or decode error. Cancelling the Promise drops the in-flight request. `options` carries the same optional builder parameters and `timeoutMs` as the buffered method; the `callback` is the trailing argument. */
  optionHistoryEODStream(symbol: string, expiration: string | Date, startDate: string | Date, endDate: string | Date, options: OptionHistoryEodOptions | undefined | null, callback: ((arg: Array<EodTick>) => void)): Promise<void>
  /**
   * Fetch intraday OHLC bars for an option contract.
   *
   * - Aggregated OHLC bars that use SIP rules for each bar.
   * - Time timestamp of the bar represents the opening time of the bar. For a trade to be part of the bar:  ``bar timestamp`` <= ``trade time`` < ``bar timestamp + interval``.
   * - Multi-day requests are limited to 1 month of data.
   *
   * Defaults (upstream):
   * - `strike`: `"*"`
   * - `right`: `"both"`
   * - `interval`: `"1s"`
   * - `start_time`: `"09:30:00"`
   * - `end_time`: `"16:00:00"`
   */
  optionHistoryOHLC(symbol: string, expiration: string | Date, date: string | Date, options?: OptionHistoryOhlcOptions | undefined | null): Promise<Array<OhlcTick>>
  /** Stream `option_history_ohlc` rows into `callback` without materialising the full response in memory. `callback(chunk: OhlcTick[]) => void` is invoked once per server chunk; the chunk is freed before the next is fetched, so peak memory tracks a single chunk rather than the whole result. This is the memory-bounded companion to the `optionHistoryOHLC` method — prefer it for multi-day or full-universe pulls. The returned Promise resolves when the stream drains and rejects (typed like the buffered method) on a wire or decode error. Cancelling the Promise drops the in-flight request. `options` carries the same optional builder parameters and `timeoutMs` as the buffered method; the `callback` is the trailing argument. */
  optionHistoryOHLCStream(symbol: string, expiration: string | Date, date: string | Date, options: OptionHistoryOhlcOptions | undefined | null, callback: ((arg: Array<OhlcTick>) => void)): Promise<void>
  /**
   * Fetch all trades for an option contract on a given date.
   *
   * - Returns every trade reported by OPRA.
   * - Trade condition mappings can be found here.
   * - Extended trade conditions are not reported by OPRA for options, so they can be ignored.
   * - Multi-day requests are limited to 1 month of data, and must specify an expiration.
   *
   * Defaults (upstream):
   * - `strike`: `"*"`
   * - `right`: `"both"`
   * - `start_time`: `"09:30:00"`
   * - `end_time`: `"16:00:00"`
   */
  optionHistoryTrade(symbol: string, expiration: string | Date, date: string | Date, options?: OptionHistoryTradeOptions | undefined | null): Promise<Array<TradeTick>>
  /** Stream `option_history_trade` rows into `callback` without materialising the full response in memory. `callback(chunk: TradeTick[]) => void` is invoked once per server chunk; the chunk is freed before the next is fetched, so peak memory tracks a single chunk rather than the whole result. This is the memory-bounded companion to the `optionHistoryTrade` method — prefer it for multi-day or full-universe pulls. The returned Promise resolves when the stream drains and rejects (typed like the buffered method) on a wire or decode error. Cancelling the Promise drops the in-flight request. `options` carries the same optional builder parameters and `timeoutMs` as the buffered method; the `callback` is the trailing argument. */
  optionHistoryTradeStream(symbol: string, expiration: string | Date, date: string | Date, options: OptionHistoryTradeOptions | undefined | null, callback: ((arg: Array<TradeTick>) => void)): Promise<void>
  /**
   * Fetch NBBO quotes for an option contract on a given date.
   *
   * - Returns every NBBO quote reported by OPRA.
   * - If the ``interval`` parameter is specified, the quote for each interval represents the last quote at the interval's timestamp.
   * - Multi-day requests are limited to 1 month of data, and must specify an expiration.
   *
   * Defaults (upstream):
   * - `strike`: `"*"`
   * - `right`: `"both"`
   * - `interval`: `"1s"`
   * - `start_time`: `"09:30:00"`
   * - `end_time`: `"16:00:00"`
   */
  optionHistoryQuote(symbol: string, expiration: string | Date, date: string | Date, options?: OptionHistoryQuoteOptions | undefined | null): Promise<Array<QuoteTick>>
  /** Stream `option_history_quote` rows into `callback` without materialising the full response in memory. `callback(chunk: QuoteTick[]) => void` is invoked once per server chunk; the chunk is freed before the next is fetched, so peak memory tracks a single chunk rather than the whole result. This is the memory-bounded companion to the `optionHistoryQuote` method — prefer it for multi-day or full-universe pulls. The returned Promise resolves when the stream drains and rejects (typed like the buffered method) on a wire or decode error. Cancelling the Promise drops the in-flight request. `options` carries the same optional builder parameters and `timeoutMs` as the buffered method; the `callback` is the trailing argument. */
  optionHistoryQuoteStream(symbol: string, expiration: string | Date, date: string | Date, options: OptionHistoryQuoteOptions | undefined | null, callback: ((arg: Array<QuoteTick>) => void)): Promise<void>
  /**
   * Fetch combined trade + quote ticks for an option contract.
   *
   * - Returns every trade reported by OPRA paired with the last NBBO quote reported by OPRA at the time of trade.
   * - A quote is matched with a trade if its timestamp ``<=`` the trade timestamp.
   * - To match trades with quotes timestamps that are ``<`` the trade timestamp, specify the ``exclusive``parameter to ``true``. After thorough testing, we have determined that using ``exclusive=true`` might yield better results for various applications.
   * - Multi-day requests are limited to 1 month of data, and must specify an expiration.
   *
   * Defaults (upstream):
   * - `strike`: `"*"`
   * - `right`: `"both"`
   * - `start_time`: `"09:30:00"`
   * - `end_time`: `"16:00:00"`
   * - `exclusive`: `true`
   */
  optionHistoryTradeQuote(symbol: string, expiration: string | Date, date: string | Date, options?: OptionHistoryTradeQuoteOptions | undefined | null): Promise<Array<TradeQuoteTick>>
  /** Stream `option_history_trade_quote` rows into `callback` without materialising the full response in memory. `callback(chunk: TradeQuoteTick[]) => void` is invoked once per server chunk; the chunk is freed before the next is fetched, so peak memory tracks a single chunk rather than the whole result. This is the memory-bounded companion to the `optionHistoryTradeQuote` method — prefer it for multi-day or full-universe pulls. The returned Promise resolves when the stream drains and rejects (typed like the buffered method) on a wire or decode error. Cancelling the Promise drops the in-flight request. `options` carries the same optional builder parameters and `timeoutMs` as the buffered method; the `callback` is the trailing argument. */
  optionHistoryTradeQuoteStream(symbol: string, expiration: string | Date, date: string | Date, options: OptionHistoryTradeQuoteOptions | undefined | null, callback: ((arg: Array<TradeQuoteTick>) => void)): Promise<void>
  /**
   * Fetch open interest history for an option contract.
   *
   * - Open Interest is normally reported once per day by OPRA at approximately 06:30 ET.
   * - A new open interest message might not be sent by OPRA if there is no open interest for the option contract.
   * - The reported open interest represents the open interest at the end of the previous trading day.
   *
   * Defaults (upstream):
   * - `strike`: `"*"`
   * - `right`: `"both"`
   */
  optionHistoryOpenInterest(symbol: string, expiration: string | Date, date: string | Date, options?: OptionHistoryOpenInterestOptions | undefined | null): Promise<Array<OpenInterestTick>>
  /** Stream `option_history_open_interest` rows into `callback` without materialising the full response in memory. `callback(chunk: OpenInterestTick[]) => void` is invoked once per server chunk; the chunk is freed before the next is fetched, so peak memory tracks a single chunk rather than the whole result. This is the memory-bounded companion to the `optionHistoryOpenInterest` method — prefer it for multi-day or full-universe pulls. The returned Promise resolves when the stream drains and rejects (typed like the buffered method) on a wire or decode error. Cancelling the Promise drops the in-flight request. `options` carries the same optional builder parameters and `timeoutMs` as the buffered method; the `callback` is the trailing argument. */
  optionHistoryOpenInterestStream(symbol: string, expiration: string | Date, date: string | Date, options: OptionHistoryOpenInterestOptions | undefined | null, callback: ((arg: Array<OpenInterestTick>) => void)): Promise<void>
  /**
   * Fetch end-of-day Greeks history for an option contract.
   *
   * - Returns the data for all contracts that share the same provided symbol and expiration.
   * - Uses Theta Data's EOD reports that get generated at 17:15 ET each day. The closing option price and closing underlying price are used for the greeks calculation.
   * - **Set `expiration` to ``*`` if you want to retrieve data for every option that shares the same ``symbol``. (note: Any ``expiration=*`` must be requested day by day)**
   *
   * Defaults (upstream):
   * - `strike`: `"*"`
   * - `right`: `"both"`
   * - `rate_type`: `"sofr"`
   * - `version`: `"latest"`
   * - `underlyer_use_nbbo`: `false`
   */
  optionHistoryGreeksEOD(symbol: string, expiration: string | Date, startDate: string | Date, endDate: string | Date, options?: OptionHistoryGreeksEodOptions | undefined | null): Promise<Array<GreeksEodTick>>
  /** Stream `option_history_greeks_eod` rows into `callback` without materialising the full response in memory. `callback(chunk: GreeksEodTick[]) => void` is invoked once per server chunk; the chunk is freed before the next is fetched, so peak memory tracks a single chunk rather than the whole result. This is the memory-bounded companion to the `optionHistoryGreeksEOD` method — prefer it for multi-day or full-universe pulls. The returned Promise resolves when the stream drains and rejects (typed like the buffered method) on a wire or decode error. Cancelling the Promise drops the in-flight request. `options` carries the same optional builder parameters and `timeoutMs` as the buffered method; the `callback` is the trailing argument. */
  optionHistoryGreeksEODStream(symbol: string, expiration: string | Date, startDate: string | Date, endDate: string | Date, options: OptionHistoryGreeksEodOptions | undefined | null, callback: ((arg: Array<GreeksEodTick>) => void)): Promise<void>
  /**
   * Fetch all Greeks history for an option contract (intraday, sampled by interval).
   *
   * - Returns the data for all contracts that share the same provided symbol and expiration.
   * - Calculated using the option and underlying midpoint price. If an interval size is specified (*highly recommended*), the option quote used in the calculation follows the same rules as the quote endpoint.
   * - The underlying price represents whatever the last underlying price was at the ``timestamp`` field. You can read more about how Theta Data calculates greeks here.
   * - Multi-day requests are limited to 1 month of data.
   *
   * Defaults (upstream):
   * - `strike`: `"*"`
   * - `right`: `"both"`
   * - `interval`: `"1s"`
   * - `start_time`: `"09:30:00"`
   * - `end_time`: `"16:00:00"`
   * - `rate_type`: `"sofr"`
   * - `version`: `"latest"`
   */
  optionHistoryGreeksAll(symbol: string, expiration: string | Date, date: string | Date, options?: OptionHistoryGreeksAllOptions | undefined | null): Promise<Array<GreeksAllTick>>
  /** Stream `option_history_greeks_all` rows into `callback` without materialising the full response in memory. `callback(chunk: GreeksAllTick[]) => void` is invoked once per server chunk; the chunk is freed before the next is fetched, so peak memory tracks a single chunk rather than the whole result. This is the memory-bounded companion to the `optionHistoryGreeksAll` method — prefer it for multi-day or full-universe pulls. The returned Promise resolves when the stream drains and rejects (typed like the buffered method) on a wire or decode error. Cancelling the Promise drops the in-flight request. `options` carries the same optional builder parameters and `timeoutMs` as the buffered method; the `callback` is the trailing argument. */
  optionHistoryGreeksAllStream(symbol: string, expiration: string | Date, date: string | Date, options: OptionHistoryGreeksAllOptions | undefined | null, callback: ((arg: Array<GreeksAllTick>) => void)): Promise<void>
  /**
   * Fetch all Greeks on each trade for an option contract.
   *
   * - Returns the data for all contracts that share the same provided symbol and expiration.
   * - Calculates greeks for every trade reported by OPRA.
   * - The underlying price represents whatever the last underlying price was at the ``timestamp`` field. You can read more about how Theta Data calculates greeks here.
   * - Multi-day requests are limited to 1 month of data, and must specify an expiration.
   *
   * Defaults (upstream):
   * - `strike`: `"*"`
   * - `right`: `"both"`
   * - `start_time`: `"09:30:00"`
   * - `end_time`: `"16:00:00"`
   * - `rate_type`: `"sofr"`
   * - `version`: `"latest"`
   */
  optionHistoryTradeGreeksAll(symbol: string, expiration: string | Date, date: string | Date, options?: OptionHistoryTradeGreeksAllOptions | undefined | null): Promise<Array<TradeGreeksAllTick>>
  /** Stream `option_history_trade_greeks_all` rows into `callback` without materialising the full response in memory. `callback(chunk: TradeGreeksAllTick[]) => void` is invoked once per server chunk; the chunk is freed before the next is fetched, so peak memory tracks a single chunk rather than the whole result. This is the memory-bounded companion to the `optionHistoryTradeGreeksAll` method — prefer it for multi-day or full-universe pulls. The returned Promise resolves when the stream drains and rejects (typed like the buffered method) on a wire or decode error. Cancelling the Promise drops the in-flight request. `options` carries the same optional builder parameters and `timeoutMs` as the buffered method; the `callback` is the trailing argument. */
  optionHistoryTradeGreeksAllStream(symbol: string, expiration: string | Date, date: string | Date, options: OptionHistoryTradeGreeksAllOptions | undefined | null, callback: ((arg: Array<TradeGreeksAllTick>) => void)): Promise<void>
  /**
   * Fetch first-order Greeks history (intraday, sampled by interval).
   *
   * - Returns the data for all contracts that share the same provided symbol and expiration.
   * - Calculated using the option and underlying midpoint price. If an interval size is specified (*highly recommended*), the option quote used in the calculation follows the same rules as the quote endpoint.
   * - The underlying price represents whatever the last underlying price was at the ``timestamp`` field. You can read more about how Theta Data calculates greeks here.
   * - Multi-day requests are limited to 1 month of data.
   *
   * Defaults (upstream):
   * - `strike`: `"*"`
   * - `right`: `"both"`
   * - `interval`: `"1s"`
   * - `start_time`: `"09:30:00"`
   * - `end_time`: `"16:00:00"`
   * - `rate_type`: `"sofr"`
   * - `version`: `"latest"`
   */
  optionHistoryGreeksFirstOrder(symbol: string, expiration: string | Date, date: string | Date, options?: OptionHistoryGreeksFirstOrderOptions | undefined | null): Promise<Array<GreeksFirstOrderTick>>
  /** Stream `option_history_greeks_first_order` rows into `callback` without materialising the full response in memory. `callback(chunk: GreeksFirstOrderTick[]) => void` is invoked once per server chunk; the chunk is freed before the next is fetched, so peak memory tracks a single chunk rather than the whole result. This is the memory-bounded companion to the `optionHistoryGreeksFirstOrder` method — prefer it for multi-day or full-universe pulls. The returned Promise resolves when the stream drains and rejects (typed like the buffered method) on a wire or decode error. Cancelling the Promise drops the in-flight request. `options` carries the same optional builder parameters and `timeoutMs` as the buffered method; the `callback` is the trailing argument. */
  optionHistoryGreeksFirstOrderStream(symbol: string, expiration: string | Date, date: string | Date, options: OptionHistoryGreeksFirstOrderOptions | undefined | null, callback: ((arg: Array<GreeksFirstOrderTick>) => void)): Promise<void>
  /**
   * Fetch first-order Greeks on each trade for an option contract.
   *
   * - Returns the data for all contracts that share the same provided symbol and expiration.
   * - Calculates greeks for every trade reported by OPRA.
   * - The underlying price represents whatever the last underlying price was at the ``timestamp`` field. You can read more about how Theta Data calculates greeks here.
   * - Multi-day requests are limited to 1 month of data, and must specify an expiration.
   *
   * Defaults (upstream):
   * - `strike`: `"*"`
   * - `right`: `"both"`
   * - `start_time`: `"09:30:00"`
   * - `end_time`: `"16:00:00"`
   * - `rate_type`: `"sofr"`
   * - `version`: `"latest"`
   */
  optionHistoryTradeGreeksFirstOrder(symbol: string, expiration: string | Date, date: string | Date, options?: OptionHistoryTradeGreeksFirstOrderOptions | undefined | null): Promise<Array<TradeGreeksFirstOrderTick>>
  /** Stream `option_history_trade_greeks_first_order` rows into `callback` without materialising the full response in memory. `callback(chunk: TradeGreeksFirstOrderTick[]) => void` is invoked once per server chunk; the chunk is freed before the next is fetched, so peak memory tracks a single chunk rather than the whole result. This is the memory-bounded companion to the `optionHistoryTradeGreeksFirstOrder` method — prefer it for multi-day or full-universe pulls. The returned Promise resolves when the stream drains and rejects (typed like the buffered method) on a wire or decode error. Cancelling the Promise drops the in-flight request. `options` carries the same optional builder parameters and `timeoutMs` as the buffered method; the `callback` is the trailing argument. */
  optionHistoryTradeGreeksFirstOrderStream(symbol: string, expiration: string | Date, date: string | Date, options: OptionHistoryTradeGreeksFirstOrderOptions | undefined | null, callback: ((arg: Array<TradeGreeksFirstOrderTick>) => void)): Promise<void>
  /**
   * Fetch second-order Greeks history (intraday, sampled by interval).
   *
   * - Returns the data for all contracts that share the same provided symbol and expiration.
   * - Calculated using the option and underlying midpoint price. If an interval size is specified (*highly recommended*), the option quote used in the calculation follows the same rules as the quote endpoint.
   * - The underlying price represents whatever the last underlying price was at the ``timestamp`` field. You can read more about how Theta Data calculates greeks here.
   * - Multi-day requests are limited to 1 month of data.
   *
   * Defaults (upstream):
   * - `strike`: `"*"`
   * - `right`: `"both"`
   * - `interval`: `"1s"`
   * - `start_time`: `"09:30:00"`
   * - `end_time`: `"16:00:00"`
   * - `rate_type`: `"sofr"`
   * - `version`: `"latest"`
   */
  optionHistoryGreeksSecondOrder(symbol: string, expiration: string | Date, date: string | Date, options?: OptionHistoryGreeksSecondOrderOptions | undefined | null): Promise<Array<GreeksSecondOrderTick>>
  /** Stream `option_history_greeks_second_order` rows into `callback` without materialising the full response in memory. `callback(chunk: GreeksSecondOrderTick[]) => void` is invoked once per server chunk; the chunk is freed before the next is fetched, so peak memory tracks a single chunk rather than the whole result. This is the memory-bounded companion to the `optionHistoryGreeksSecondOrder` method — prefer it for multi-day or full-universe pulls. The returned Promise resolves when the stream drains and rejects (typed like the buffered method) on a wire or decode error. Cancelling the Promise drops the in-flight request. `options` carries the same optional builder parameters and `timeoutMs` as the buffered method; the `callback` is the trailing argument. */
  optionHistoryGreeksSecondOrderStream(symbol: string, expiration: string | Date, date: string | Date, options: OptionHistoryGreeksSecondOrderOptions | undefined | null, callback: ((arg: Array<GreeksSecondOrderTick>) => void)): Promise<void>
  /**
   * Fetch second-order Greeks on each trade for an option contract.
   *
   * - Returns the data for all contracts that share the same provided symbol and expiration.
   * - Calculates greeks for every trade reported by OPRA.
   * - The underlying price represents whatever the last underlying price was at the ``timestamp`` field. You can read more about how Theta Data calculates greeks here.
   * - Multi-day requests are limited to 1 month of data, and must specify an expiration.
   *
   * Defaults (upstream):
   * - `strike`: `"*"`
   * - `right`: `"both"`
   * - `start_time`: `"09:30:00"`
   * - `end_time`: `"16:00:00"`
   * - `rate_type`: `"sofr"`
   * - `version`: `"latest"`
   */
  optionHistoryTradeGreeksSecondOrder(symbol: string, expiration: string | Date, date: string | Date, options?: OptionHistoryTradeGreeksSecondOrderOptions | undefined | null): Promise<Array<TradeGreeksSecondOrderTick>>
  /** Stream `option_history_trade_greeks_second_order` rows into `callback` without materialising the full response in memory. `callback(chunk: TradeGreeksSecondOrderTick[]) => void` is invoked once per server chunk; the chunk is freed before the next is fetched, so peak memory tracks a single chunk rather than the whole result. This is the memory-bounded companion to the `optionHistoryTradeGreeksSecondOrder` method — prefer it for multi-day or full-universe pulls. The returned Promise resolves when the stream drains and rejects (typed like the buffered method) on a wire or decode error. Cancelling the Promise drops the in-flight request. `options` carries the same optional builder parameters and `timeoutMs` as the buffered method; the `callback` is the trailing argument. */
  optionHistoryTradeGreeksSecondOrderStream(symbol: string, expiration: string | Date, date: string | Date, options: OptionHistoryTradeGreeksSecondOrderOptions | undefined | null, callback: ((arg: Array<TradeGreeksSecondOrderTick>) => void)): Promise<void>
  /**
   * Fetch third-order Greeks history (intraday, sampled by interval).
   *
   * - Returns the data for all contracts that share the same provided symbol and expiration.
   * - Calculated using the option and underlying midpoint price. If an interval size is specified (*highly recommended*), the option quote used in the calculation follows the same rules as the quote endpoint.
   * - The underlying price represents whatever the last underlying price was at the ``timestamp`` field. You can read more about how Theta Data calculates greeks here.
   * - Multi-day requests are limited to 1 month of data.
   *
   * Defaults (upstream):
   * - `strike`: `"*"`
   * - `right`: `"both"`
   * - `interval`: `"1s"`
   * - `start_time`: `"09:30:00"`
   * - `end_time`: `"16:00:00"`
   * - `rate_type`: `"sofr"`
   * - `version`: `"latest"`
   */
  optionHistoryGreeksThirdOrder(symbol: string, expiration: string | Date, date: string | Date, options?: OptionHistoryGreeksThirdOrderOptions | undefined | null): Promise<Array<GreeksThirdOrderTick>>
  /** Stream `option_history_greeks_third_order` rows into `callback` without materialising the full response in memory. `callback(chunk: GreeksThirdOrderTick[]) => void` is invoked once per server chunk; the chunk is freed before the next is fetched, so peak memory tracks a single chunk rather than the whole result. This is the memory-bounded companion to the `optionHistoryGreeksThirdOrder` method — prefer it for multi-day or full-universe pulls. The returned Promise resolves when the stream drains and rejects (typed like the buffered method) on a wire or decode error. Cancelling the Promise drops the in-flight request. `options` carries the same optional builder parameters and `timeoutMs` as the buffered method; the `callback` is the trailing argument. */
  optionHistoryGreeksThirdOrderStream(symbol: string, expiration: string | Date, date: string | Date, options: OptionHistoryGreeksThirdOrderOptions | undefined | null, callback: ((arg: Array<GreeksThirdOrderTick>) => void)): Promise<void>
  /**
   * Fetch third-order Greeks on each trade for an option contract.
   *
   * - Returns the data for all contracts that share the same provided symbol and expiration.
   * - Calculates greeks for every trade reported by OPRA.
   * - The underlying price represents whatever the last underlying price was at the ``timestamp`` field. You can read more about how Theta Data calculates greeks here.
   * - Multi-day requests are limited to 1 month of data, and must specify an expiration.
   *
   * Defaults (upstream):
   * - `strike`: `"*"`
   * - `right`: `"both"`
   * - `start_time`: `"09:30:00"`
   * - `end_time`: `"16:00:00"`
   * - `rate_type`: `"sofr"`
   * - `version`: `"latest"`
   */
  optionHistoryTradeGreeksThirdOrder(symbol: string, expiration: string | Date, date: string | Date, options?: OptionHistoryTradeGreeksThirdOrderOptions | undefined | null): Promise<Array<TradeGreeksThirdOrderTick>>
  /** Stream `option_history_trade_greeks_third_order` rows into `callback` without materialising the full response in memory. `callback(chunk: TradeGreeksThirdOrderTick[]) => void` is invoked once per server chunk; the chunk is freed before the next is fetched, so peak memory tracks a single chunk rather than the whole result. This is the memory-bounded companion to the `optionHistoryTradeGreeksThirdOrder` method — prefer it for multi-day or full-universe pulls. The returned Promise resolves when the stream drains and rejects (typed like the buffered method) on a wire or decode error. Cancelling the Promise drops the in-flight request. `options` carries the same optional builder parameters and `timeoutMs` as the buffered method; the `callback` is the trailing argument. */
  optionHistoryTradeGreeksThirdOrderStream(symbol: string, expiration: string | Date, date: string | Date, options: OptionHistoryTradeGreeksThirdOrderOptions | undefined | null, callback: ((arg: Array<TradeGreeksThirdOrderTick>) => void)): Promise<void>
  /**
   * Fetch implied volatility history (intraday, sampled by interval).
   *
   * - Returns implied volatilies calculated using the national best bid, mid, and ask price of the option respectively.
   * - The underlying price represents whatever the last underlying price was at the ``timestamp`` field. You can read more about how Theta Data calculates greeks here.
   * - Multi-day requests are limited to 1 month of data.
   *
   * Defaults (upstream):
   * - `strike`: `"*"`
   * - `right`: `"both"`
   * - `interval`: `"1s"`
   * - `start_time`: `"09:30:00"`
   * - `end_time`: `"16:00:00"`
   * - `rate_type`: `"sofr"`
   * - `version`: `"latest"`
   */
  optionHistoryGreeksImpliedVolatility(symbol: string, expiration: string | Date, date: string | Date, options?: OptionHistoryGreeksImpliedVolatilityOptions | undefined | null): Promise<Array<IvTick>>
  /** Stream `option_history_greeks_implied_volatility` rows into `callback` without materialising the full response in memory. `callback(chunk: IvTick[]) => void` is invoked once per server chunk; the chunk is freed before the next is fetched, so peak memory tracks a single chunk rather than the whole result. This is the memory-bounded companion to the `optionHistoryGreeksImpliedVolatility` method — prefer it for multi-day or full-universe pulls. The returned Promise resolves when the stream drains and rejects (typed like the buffered method) on a wire or decode error. Cancelling the Promise drops the in-flight request. `options` carries the same optional builder parameters and `timeoutMs` as the buffered method; the `callback` is the trailing argument. */
  optionHistoryGreeksImpliedVolatilityStream(symbol: string, expiration: string | Date, date: string | Date, options: OptionHistoryGreeksImpliedVolatilityOptions | undefined | null, callback: ((arg: Array<IvTick>) => void)): Promise<void>
  /**
   * Fetch implied volatility on each trade for an option contract.
   *
   * - Returns implied volatilies calculated using the trade reported by OPRA.
   * - The underlying price represents whatever the last underlying price was at the ``timestamp`` field. You can read more about how Theta Data calculates greeks here.
   * - Multi-day requests are limited to 1 month of data, and must specify an expiration.
   *
   * Defaults (upstream):
   * - `strike`: `"*"`
   * - `right`: `"both"`
   * - `start_time`: `"09:30:00"`
   * - `end_time`: `"16:00:00"`
   * - `rate_type`: `"sofr"`
   * - `version`: `"latest"`
   */
  optionHistoryTradeGreeksImpliedVolatility(symbol: string, expiration: string | Date, date: string | Date, options?: OptionHistoryTradeGreeksImpliedVolatilityOptions | undefined | null): Promise<Array<TradeGreeksImpliedVolatilityTick>>
  /** Stream `option_history_trade_greeks_implied_volatility` rows into `callback` without materialising the full response in memory. `callback(chunk: TradeGreeksImpliedVolatilityTick[]) => void` is invoked once per server chunk; the chunk is freed before the next is fetched, so peak memory tracks a single chunk rather than the whole result. This is the memory-bounded companion to the `optionHistoryTradeGreeksImpliedVolatility` method — prefer it for multi-day or full-universe pulls. The returned Promise resolves when the stream drains and rejects (typed like the buffered method) on a wire or decode error. Cancelling the Promise drops the in-flight request. `options` carries the same optional builder parameters and `timeoutMs` as the buffered method; the `callback` is the trailing argument. */
  optionHistoryTradeGreeksImpliedVolatilityStream(symbol: string, expiration: string | Date, date: string | Date, options: OptionHistoryTradeGreeksImpliedVolatilityOptions | undefined | null, callback: ((arg: Array<TradeGreeksImpliedVolatilityTick>) => void)): Promise<void>
  /**
   * Fetch the trade at a specific time of day across a date range for an option.
   *
   * - Returns the last trade reported by OPRA at a specified millisecond of the day.
   * - Trade condition mappings can be found here.
   * - Extended trade conditions are not reported by OPRA for options, so they can be ignored.
   * - The ``time_of_day``parameter represents the 00:00:00.000 ET that the trade should be provided for.
   *
   * Defaults (upstream):
   * - `strike`: `"*"`
   * - `right`: `"both"`
   */
  optionAtTimeTrade(symbol: string, expiration: string | Date, startDate: string | Date, endDate: string | Date, timeOfDay: string | Date, options?: OptionAtTimeTradeOptions | undefined | null): Promise<Array<TradeTick>>
  /** Stream `option_at_time_trade` rows into `callback` without materialising the full response in memory. `callback(chunk: TradeTick[]) => void` is invoked once per server chunk; the chunk is freed before the next is fetched, so peak memory tracks a single chunk rather than the whole result. This is the memory-bounded companion to the `optionAtTimeTrade` method — prefer it for multi-day or full-universe pulls. The returned Promise resolves when the stream drains and rejects (typed like the buffered method) on a wire or decode error. Cancelling the Promise drops the in-flight request. `options` carries the same optional builder parameters and `timeoutMs` as the buffered method; the `callback` is the trailing argument. */
  optionAtTimeTradeStream(symbol: string, expiration: string | Date, startDate: string | Date, endDate: string | Date, timeOfDay: string | Date, options: OptionAtTimeTradeOptions | undefined | null, callback: ((arg: Array<TradeTick>) => void)): Promise<void>
  /**
   * Fetch the quote at a specific time of day across a date range for an option.
   *
   * - Returns the last NBBO quote reported by OPRA at a specified millisecond of the day.
   * - The ``time_of_day``parameter represents the 00:00:00.000 ET that the quote should be provided for.
   *
   * Defaults (upstream):
   * - `strike`: `"*"`
   * - `right`: `"both"`
   */
  optionAtTimeQuote(symbol: string, expiration: string | Date, startDate: string | Date, endDate: string | Date, timeOfDay: string | Date, options?: OptionAtTimeQuoteOptions | undefined | null): Promise<Array<QuoteTick>>
  /** Stream `option_at_time_quote` rows into `callback` without materialising the full response in memory. `callback(chunk: QuoteTick[]) => void` is invoked once per server chunk; the chunk is freed before the next is fetched, so peak memory tracks a single chunk rather than the whole result. This is the memory-bounded companion to the `optionAtTimeQuote` method — prefer it for multi-day or full-universe pulls. The returned Promise resolves when the stream drains and rejects (typed like the buffered method) on a wire or decode error. Cancelling the Promise drops the in-flight request. `options` carries the same optional builder parameters and `timeoutMs` as the buffered method; the `callback` is the trailing argument. */
  optionAtTimeQuoteStream(symbol: string, expiration: string | Date, startDate: string | Date, endDate: string | Date, timeOfDay: string | Date, options: OptionAtTimeQuoteOptions | undefined | null, callback: ((arg: Array<QuoteTick>) => void)): Promise<void>
  /**
   * List all available index symbols.
   *
   * A symbol can be defined as a unique identifier for a stock / underlying asset. Common terms also include: root, ticker, and underlying. This endpoint returns all traded symbols for options. This endpoint is updated overnight.
   */
  indexListSymbols(options?: IndexListSymbolsOptions | undefined | null): Promise<Array<string>>
  /**
   * List available dates for an index symbol.
   *
   * Lists all dates of data that are available for a index with a given request type and symbol. This endpoint is updated overnight.
   */
  indexListDates(symbol: string, options?: IndexListDatesOptions | undefined | null): Promise<Array<string>>
  /**
   * Get the latest OHLC snapshot for one or more indices.
   *
   * - Retrieves the real-time current day OHLC.
   * - Exchanges typically generate a price report every second for popular indices like SPX.
   */
  indexSnapshotOHLC(symbols: string | Array<string>, options?: IndexSnapshotOhlcOptions | undefined | null): Promise<Array<OhlcTick>>
  /**
   * Get the latest price snapshot for one or more indices.
   *
   * - Retrieves a real-time last index price.
   * - Exchanges typically generate a price report every second for popular indices like SPX.
   */
  indexSnapshotPrice(symbols: string | Array<string>, options?: IndexSnapshotPriceOptions | undefined | null): Promise<Array<PriceTick>>
  /**
   * Get the latest market value snapshot for one or more indices.
   *
   * - Retrieves a real-time last index market value.
   * - Exchanges typically generate a price report every second for popular indices like SPX.
   */
  indexSnapshotMarketValue(symbols: string | Array<string>, options?: IndexSnapshotMarketValueOptions | undefined | null): Promise<Array<MarketValueTick>>
  /**
   * Fetch end-of-day index data for a date range.
   *
   * - Since the indices feeds do not provide a national EOD report, Theta Data generates a national EOD report at 17:15 each day.
   */
  indexHistoryEOD(symbol: string, startDate: string | Date, endDate: string | Date, options?: IndexHistoryEodOptions | undefined | null): Promise<Array<EodTick>>
  /** Stream `index_history_eod` rows into `callback` without materialising the full response in memory. `callback(chunk: EodTick[]) => void` is invoked once per server chunk; the chunk is freed before the next is fetched, so peak memory tracks a single chunk rather than the whole result. This is the memory-bounded companion to the `indexHistoryEOD` method — prefer it for multi-day or full-universe pulls. The returned Promise resolves when the stream drains and rejects (typed like the buffered method) on a wire or decode error. Cancelling the Promise drops the in-flight request. `options` carries the same optional builder parameters and `timeoutMs` as the buffered method; the `callback` is the trailing argument. */
  indexHistoryEODStream(symbol: string, startDate: string | Date, endDate: string | Date, options: IndexHistoryEodOptions | undefined | null, callback: ((arg: Array<EodTick>) => void)): Promise<void>
  /**
   * Fetch intraday OHLC bars for an index.
   *
   * - Aggregated OHLC bars that use SIP rules for each bar.
   * - Time timestamp of the bar represents the opening time of the bar. For a trade to be part of the bar:  ``bar timestamp`` <= ``trade time`` < ``bar timestamp + interval``.
   * - Exchanges typically generate a price report every second for popular indices like SPX.
   *
   * Defaults (upstream):
   * - `interval`: `"1s"`
   * - `start_time`: `"09:30:00"`
   * - `end_time`: `"16:00:00"`
   */
  indexHistoryOHLC(symbol: string, startDate: string | Date, endDate: string | Date, options?: IndexHistoryOhlcOptions | undefined | null): Promise<Array<OhlcTick>>
  /** Stream `index_history_ohlc` rows into `callback` without materialising the full response in memory. `callback(chunk: OhlcTick[]) => void` is invoked once per server chunk; the chunk is freed before the next is fetched, so peak memory tracks a single chunk rather than the whole result. This is the memory-bounded companion to the `indexHistoryOHLC` method — prefer it for multi-day or full-universe pulls. The returned Promise resolves when the stream drains and rejects (typed like the buffered method) on a wire or decode error. Cancelling the Promise drops the in-flight request. `options` carries the same optional builder parameters and `timeoutMs` as the buffered method; the `callback` is the trailing argument. */
  indexHistoryOHLCStream(symbol: string, startDate: string | Date, endDate: string | Date, options: IndexHistoryOhlcOptions | undefined | null, callback: ((arg: Array<OhlcTick>) => void)): Promise<void>
  /**
   * Fetch intraday price history for an index.
   *
   * - Retrieves historical indices price reports. Exchanges typically generate a price report every second for popular indices like SPX.
   * - When the ``interval`` parameter is specified, the returned data represents the price at the exact time of each timestamp. If the timestamp in the response is 10:30:00, the price field represents the price at that exact time of the day.
   * - A price update from the exchange is omitted if the price remained the same from the previous update.
   * - Multi-day requests are limited to 1 month of data.
   *
   * Defaults (upstream):
   * - `interval`: `"1s"`
   * - `start_time`: `"09:30:00"`
   * - `end_time`: `"16:00:00"`
   */
  indexHistoryPrice(symbol: string, date: string | Date, options?: IndexHistoryPriceOptions | undefined | null): Promise<Array<PriceTick>>
  /** Stream `index_history_price` rows into `callback` without materialising the full response in memory. `callback(chunk: PriceTick[]) => void` is invoked once per server chunk; the chunk is freed before the next is fetched, so peak memory tracks a single chunk rather than the whole result. This is the memory-bounded companion to the `indexHistoryPrice` method — prefer it for multi-day or full-universe pulls. The returned Promise resolves when the stream drains and rejects (typed like the buffered method) on a wire or decode error. Cancelling the Promise drops the in-flight request. `options` carries the same optional builder parameters and `timeoutMs` as the buffered method; the `callback` is the trailing argument. */
  indexHistoryPriceStream(symbol: string, date: string | Date, options: IndexHistoryPriceOptions | undefined | null, callback: ((arg: Array<PriceTick>) => void)): Promise<void>
  /**
   * Fetch the index price at a specific time of day across a date range.
   *
   * - Retrieves historical indices price reports. Exchanges typically generate a price report every second for popular indices like SPX.
   * - The ``time_of_day`` parameter represents the 00:00:00.000 ET that the price should be provided for.
   */
  indexAtTimePrice(symbol: string, startDate: string | Date, endDate: string | Date, timeOfDay: string | Date, options?: IndexAtTimePriceOptions | undefined | null): Promise<Array<IndexPriceAtTimeTick>>
  /** Stream `index_at_time_price` rows into `callback` without materialising the full response in memory. `callback(chunk: IndexPriceAtTimeTick[]) => void` is invoked once per server chunk; the chunk is freed before the next is fetched, so peak memory tracks a single chunk rather than the whole result. This is the memory-bounded companion to the `indexAtTimePrice` method — prefer it for multi-day or full-universe pulls. The returned Promise resolves when the stream drains and rejects (typed like the buffered method) on a wire or decode error. Cancelling the Promise drops the in-flight request. `options` carries the same optional builder parameters and `timeoutMs` as the buffered method; the `callback` is the trailing argument. */
  indexAtTimePriceStream(symbol: string, startDate: string | Date, endDate: string | Date, timeOfDay: string | Date, options: IndexAtTimePriceOptions | undefined | null, callback: ((arg: Array<IndexPriceAtTimeTick>) => void)): Promise<void>
  /**
   * Check whether the market is open today.
   *
   * - Retrieves current day equity market schedule
   * - *On days when the market closes early at 1:00 PM ET; eligible options will trade until 1:15 PM.
   * - **Some NYSE exchanges will continue late trading until 5:00 PM ET on early close days.
   */
  calendarOpenToday(options?: CalendarOpenTodayOptions | undefined | null): Promise<Array<CalendarDay>>
  /**
   * Get calendar information for a specific date.
   *
   * - Retrieves equity market schedule for a given date
   * - Note: Holiday data is available 01/01/2012 through the end of the calendar year that immediately follows the current year
   * - *On days when the market closes early at 1:00 PM ET; eligible options will trade until 1:15 PM.
   * - **Some NYSE exchanges will continue late trading until 5:00 PM ET on early close days.
   */
  calendarOnDate(date: string | Date, options?: CalendarOnDateOptions | undefined | null): Promise<Array<CalendarDay>>
  /**
   * Get equity market holidays and early-close days for a year (vendor `year_holidays` endpoint — only non-standard days, not every trading day).
   *
   * - Retrieves equity market holidays for a given year
   * - Note: Holiday data is available 01/01/2012 through the end of the calendar year that immediately follows the current year
   * - *On days when the market closes early at 1:00 PM ET; eligible options will trade until 1:15 PM.
   * - **Some NYSE exchanges will continue late trading until 5:00 PM ET on early close days.
   */
  calendarYear(year: string, options?: CalendarYearOptions | undefined | null): Promise<Array<CalendarDay>>
  /**
   * Fetch end-of-day interest rate history.
   *
   * - Returns the interest rate reported. Depending on the rate, reports can occur in the morning or the afternoon.
   * - Valid `symbol` values per upstream `RateType` enum:
   *   `SOFR`, `TREASURY_M1`, `TREASURY_M3`, `TREASURY_M6`,
   *   `TREASURY_Y1`, `TREASURY_Y2`, `TREASURY_Y3`, `TREASURY_Y5`,
   *   `TREASURY_Y7`, `TREASURY_Y10`, `TREASURY_Y20`, `TREASURY_Y30`.
   */
  interestRateHistoryEOD(symbol: string, startDate: string | Date, endDate: string | Date, options?: InterestRateHistoryEodOptions | undefined | null): Promise<Array<InterestRateTick>>
  /** Stream `interest_rate_history_eod` rows into `callback` without materialising the full response in memory. `callback(chunk: InterestRateTick[]) => void` is invoked once per server chunk; the chunk is freed before the next is fetched, so peak memory tracks a single chunk rather than the whole result. This is the memory-bounded companion to the `interestRateHistoryEOD` method — prefer it for multi-day or full-universe pulls. The returned Promise resolves when the stream drains and rejects (typed like the buffered method) on a wire or decode error. Cancelling the Promise drops the in-flight request. `options` carries the same optional builder parameters and `timeoutMs` as the buffered method; the `callback` is the trailing argument. */
  interestRateHistoryEODStream(symbol: string, startDate: string | Date, endDate: string | Date, options: InterestRateHistoryEodOptions | undefined | null, callback: ((arg: Array<InterestRateTick>) => void)): Promise<void>
  /**
   * Fetch intraday OHLC bars across a date range (start_date..end_date). This is a dedicated upstream route, distinct from the single-date stock_history_ohlc; the `_range` suffix mirrors the vendor's separate `ohlc_range` route.
   *
   * Defaults (upstream):
   * - `interval`: `"1s"`
   * - `start_time`: `"09:30:00"`
   * - `end_time`: `"16:00:00"`
   * - `venue`: `"nqb"`
   */
  stockHistoryOHLCRange(symbol: string, startDate: string | Date, endDate: string | Date, options?: StockHistoryOhlcRangeOptions | undefined | null): Promise<Array<OhlcTick>>
  /** Stream `stock_history_ohlc_range` rows into `callback` without materialising the full response in memory. `callback(chunk: OhlcTick[]) => void` is invoked once per server chunk; the chunk is freed before the next is fetched, so peak memory tracks a single chunk rather than the whole result. This is the memory-bounded companion to the `stockHistoryOHLCRange` method — prefer it for multi-day or full-universe pulls. The returned Promise resolves when the stream drains and rejects (typed like the buffered method) on a wire or decode error. Cancelling the Promise drops the in-flight request. `options` carries the same optional builder parameters and `timeoutMs` as the buffered method; the `callback` is the trailing argument. */
  stockHistoryOHLCRangeStream(symbol: string, startDate: string | Date, endDate: string | Date, options: StockHistoryOhlcRangeOptions | undefined | null, callback: ((arg: Array<OhlcTick>) => void)): Promise<void>
}

/**
 * User-facing historical-data sub-namespace returned by the
 * `client.historical` getter.
 *
 * Holds a cheap `Arc` clone of the inner unified client; constructing it
 * performs no auth round-trip and mutates no streaming state. Every
 * historical endpoint method is generated onto this view from
 * `endpoint_surface.toml`, so the surface stays a single generated
 * source of truth.
 */
export declare class HistoricalView {
  /**
   * List all available stock ticker symbols.
   *
   * A symbol can be defined as a unique identifier for a stock / underlying asset. Common terms also include: root, ticker, and underlying. This endpoint returns all traded symbols for stocks. This endpoint is updated overnight.
   */
  stockListSymbols(options?: StockListSymbolsOptions | undefined | null): Promise<Array<string>>
  /**
   * List available dates for a stock by request type (EOD, TRADE, QUOTE, etc.).
   *
   * Lists all dates of data that are available for a stock with a given request type and symbol. This endpoint is updated overnight.
   */
  stockListDates(requestType: string, symbol: string, options?: StockListDatesOptions | undefined | null): Promise<Array<string>>
  /**
   * Get the latest OHLC snapshot for one or more stocks.
   *
   * Provides a real-time Open, High, Low, Close for the current day.
   * * Returns a real-time session OHLC from the Nasdaq Basic feed if the account has a stocks standard or pro subscription.
   * * Returns a 15-minute delayed session OHLC from the UTP & CTA feeds if the account has the stocks value subscription.
   * * Theta Data resets its snapshot cache at midnight ET every day. This endpoint may not work on a weekend where there were no eligible messages sent over exchange feeds. We recommend using historic requests during the weekend.
   *
   * Defaults (upstream):
   * - `venue`: `"nqb"`
   */
  stockSnapshotOHLC(symbols: string | Array<string>, options?: StockSnapshotOhlcOptions | undefined | null): Promise<Array<OhlcTick>>
  /**
   * Get the latest trade snapshot for one or more stocks.
   *
   * Returns a real-time last trade from the Nasdaq Basic feed if the account has a stocks standard or pro subscription.
   *
   * - Theta Data resets its snapshot cache at midnight ET every day. This endpoint may not work on a weekend where there were no eligible messages sent over exchange feeds. We recommend using historic requests during the weekend.
   *
   * Defaults (upstream):
   * - `venue`: `"nqb"`
   */
  stockSnapshotTrade(symbols: string | Array<string>, options?: StockSnapshotTradeOptions | undefined | null): Promise<Array<TradeTick>>
  /**
   * Get the latest NBBO quote snapshot for one or more stocks.
   *
   * * Returns a real-time last BBO quote from the Nasdaq Basic feed if the account has a stocks standard or pro subscription.
   * * Returns a 15-minute delayed NBBO quote from the UTP & CTA feeds account has the stocks value subscription subscription.
   * - Theta Data resets its snapshot cache at midnight ET every day. This endpoint may not work on a weekend where there were no eligible messages sent over exchange feeds. We recommend using historic requests during the weekend.
   *
   * Defaults (upstream):
   * - `venue`: `"nqb"`
   */
  stockSnapshotQuote(symbols: string | Array<string>, options?: StockSnapshotQuoteOptions | undefined | null): Promise<Array<QuoteTick>>
  /**
   * Get the latest market value snapshot for one or more stocks.
   *
   * * Returns a real-time market value derived from the last BBO quote from the Nasdaq Basic feed if the account has a stocks standard or pro subscription.
   * * Returns a 15-minute delayed market value derived from an NBBO quote from the UTP & CTA feeds if the account has the stocks value subscription subscription.
   * - Theta Data resets its snapshot cache at midnight ET every day. This endpoint may not work on a weekend where there were no eligible messages sent over exchange feeds. We recommend using historic requests during the weekend.
   *
   * Defaults (upstream):
   * - `venue`: `"nqb"`
   */
  stockSnapshotMarketValue(symbols: string | Array<string>, options?: StockSnapshotMarketValueOptions | undefined | null): Promise<Array<MarketValueTick>>
  /**
   * Fetch end-of-day stock data for a date range. Returns OHLCV + bid/ask per trading day.
   *
   * Since the equity SIPs only generate a partial EOD report, Theta Data generates a national EOD report at 17:15 ET each day. ``created`` represents the datetime the report was generated and ``last_trade`` represents the datetime of the last trade. The quote in the response represents the last NBBO reported by CTA or UTP at the time of report generation. You can read more about EOD & OHLC data here. Theta Data plans to avail SIP EOD reports in the near future.
   */
  stockHistoryEOD(symbol: string, startDate: string | Date, endDate: string | Date, options?: StockHistoryEodOptions | undefined | null): Promise<Array<EodTick>>
  /** Stream `stock_history_eod` rows into `callback` without materialising the full response in memory. `callback(chunk: EodTick[]) => void` is invoked once per server chunk; the chunk is freed before the next is fetched, so peak memory tracks a single chunk rather than the whole result. This is the memory-bounded companion to the `stockHistoryEOD` method — prefer it for multi-day or full-universe pulls. The returned Promise resolves when the stream drains and rejects (typed like the buffered method) on a wire or decode error. Cancelling the Promise drops the in-flight request. `options` carries the same optional builder parameters and `timeoutMs` as the buffered method; the `callback` is the trailing argument. */
  stockHistoryEODStream(symbol: string, startDate: string | Date, endDate: string | Date, options: StockHistoryEodOptions | undefined | null, callback: ((arg: Array<EodTick>) => void)): Promise<void>
  /**
   * Fetch intraday OHLC bars for a stock on a single date.
   *
   * - Aggregated OHLC bars that use SIP rules for each bar. Time timestamp of the bar represents the opening time of the bar. For a trade to be part of the bar:  ``bar time`` <= ``trade time`` < ``bar timestamp + ivl``, where ivl is the specified interval size in milliseconds.
   * - Set the ``venue`` parameter to ``nqb`` to access current-day real-time historic data from the Nasdaq Basic feed if the account has a stocks standard or pro subscription.
   * - Multi-day requests are limited to 1 month of data.
   *
   * Defaults (upstream):
   * - `interval`: `"1s"`
   * - `start_time`: `"09:30:00"`
   * - `end_time`: `"16:00:00"`
   * - `venue`: `"nqb"`
   */
  stockHistoryOHLC(symbol: string, date: string | Date, options?: StockHistoryOhlcOptions | undefined | null): Promise<Array<OhlcTick>>
  /** Stream `stock_history_ohlc` rows into `callback` without materialising the full response in memory. `callback(chunk: OhlcTick[]) => void` is invoked once per server chunk; the chunk is freed before the next is fetched, so peak memory tracks a single chunk rather than the whole result. This is the memory-bounded companion to the `stockHistoryOHLC` method — prefer it for multi-day or full-universe pulls. The returned Promise resolves when the stream drains and rejects (typed like the buffered method) on a wire or decode error. Cancelling the Promise drops the in-flight request. `options` carries the same optional builder parameters and `timeoutMs` as the buffered method; the `callback` is the trailing argument. */
  stockHistoryOHLCStream(symbol: string, date: string | Date, options: StockHistoryOhlcOptions | undefined | null, callback: ((arg: Array<OhlcTick>) => void)): Promise<void>
  /**
   * Fetch all trades for a stock on a given date.
   *
   * Returns every trade reported by UTP & CTA. Set the ``venue`` parameter to ``nqb`` to access current-day real-time historic data from the Nasdaq Basic feed if the account has a stocks standard or pro subscription.
   * - Multi-day requests are limited to 1 month of data.
   *
   * Defaults (upstream):
   * - `start_time`: `"09:30:00"`
   * - `end_time`: `"16:00:00"`
   * - `venue`: `"nqb"`
   */
  stockHistoryTrade(symbol: string, date: string | Date, options?: StockHistoryTradeOptions | undefined | null): Promise<Array<TradeTick>>
  /** Stream `stock_history_trade` rows into `callback` without materialising the full response in memory. `callback(chunk: TradeTick[]) => void` is invoked once per server chunk; the chunk is freed before the next is fetched, so peak memory tracks a single chunk rather than the whole result. This is the memory-bounded companion to the `stockHistoryTrade` method — prefer it for multi-day or full-universe pulls. The returned Promise resolves when the stream drains and rejects (typed like the buffered method) on a wire or decode error. Cancelling the Promise drops the in-flight request. `options` carries the same optional builder parameters and `timeoutMs` as the buffered method; the `callback` is the trailing argument. */
  stockHistoryTradeStream(symbol: string, date: string | Date, options: StockHistoryTradeOptions | undefined | null, callback: ((arg: Array<TradeTick>) => void)): Promise<void>
  /**
   * Fetch NBBO quotes for a stock on a given date at a given interval.
   *
   * - Returns every NBBO quote reported by UTP and CTA.
   * - If the ``interval`` parameter is specified, the quote for each interval represents the last quote prior to the interval's timestamp.
   * - Set the ``venue`` parameter to ``nqb`` to access current-day real-time historic data from the Nasdaq Basic feed if the account has a stocks standard or pro subscription.
   * - Multi-day requests are limited to 1 month of data.
   *
   * Defaults (upstream):
   * - `interval`: `"1s"`
   * - `start_time`: `"09:30:00"`
   * - `end_time`: `"16:00:00"`
   * - `venue`: `"nqb"`
   */
  stockHistoryQuote(symbol: string, date: string | Date, options?: StockHistoryQuoteOptions | undefined | null): Promise<Array<QuoteTick>>
  /** Stream `stock_history_quote` rows into `callback` without materialising the full response in memory. `callback(chunk: QuoteTick[]) => void` is invoked once per server chunk; the chunk is freed before the next is fetched, so peak memory tracks a single chunk rather than the whole result. This is the memory-bounded companion to the `stockHistoryQuote` method — prefer it for multi-day or full-universe pulls. The returned Promise resolves when the stream drains and rejects (typed like the buffered method) on a wire or decode error. Cancelling the Promise drops the in-flight request. `options` carries the same optional builder parameters and `timeoutMs` as the buffered method; the `callback` is the trailing argument. */
  stockHistoryQuoteStream(symbol: string, date: string | Date, options: StockHistoryQuoteOptions | undefined | null, callback: ((arg: Array<QuoteTick>) => void)): Promise<void>
  /**
   * Fetch combined trade + quote ticks for a stock on a given date. Returns raw DataTable.
   *
   * Returns every trade reported by UTP & CTA paired with the last BBO quote reported by UTP or CTA at the time of trade. A quote is matched with a trade if its timestamp ``<=`` the trade timestamp. If you prefer to match quotes with timestamps that are ``<`` the trade timestamp, specify the ``exclusive`` parameter to ``true``. Set the ``venue`` parameter to ``nqb`` to access current-day real-time historic data from the Nasdaq Basic feed if the account has a stocks standard or pro subscription.
   * - Multi-day requests are limited to 1 month of data.
   *
   * Defaults (upstream):
   * - `start_time`: `"09:30:00"`
   * - `end_time`: `"16:00:00"`
   * - `exclusive`: `true`
   * - `venue`: `"nqb"`
   */
  stockHistoryTradeQuote(symbol: string, date: string | Date, options?: StockHistoryTradeQuoteOptions | undefined | null): Promise<Array<TradeQuoteTick>>
  /** Stream `stock_history_trade_quote` rows into `callback` without materialising the full response in memory. `callback(chunk: TradeQuoteTick[]) => void` is invoked once per server chunk; the chunk is freed before the next is fetched, so peak memory tracks a single chunk rather than the whole result. This is the memory-bounded companion to the `stockHistoryTradeQuote` method — prefer it for multi-day or full-universe pulls. The returned Promise resolves when the stream drains and rejects (typed like the buffered method) on a wire or decode error. Cancelling the Promise drops the in-flight request. `options` carries the same optional builder parameters and `timeoutMs` as the buffered method; the `callback` is the trailing argument. */
  stockHistoryTradeQuoteStream(symbol: string, date: string | Date, options: StockHistoryTradeQuoteOptions | undefined | null, callback: ((arg: Array<TradeQuoteTick>) => void)): Promise<void>
  /**
   * Fetch the trade at a specific time of day across a date range.
   *
   * #### Real-time request:
   * - Returns a real-time session from the Nasdaq Basic feed if the account has a stocks standard or pro subscription.
   * - Returns a 15-minute delayed session from the UTP & CTA feeds account has the stocks value subscription subscription.
   *
   * #### Historical request:
   * Returns the last trade reported by UTP & CTA feeds at a specified millisecond of the day.
   * Trade condition mappings can be found here.
   *
   * Defaults (upstream):
   * - `venue`: `"nqb"`
   */
  stockAtTimeTrade(symbol: string, startDate: string | Date, endDate: string | Date, timeOfDay: string | Date, options?: StockAtTimeTradeOptions | undefined | null): Promise<Array<TradeTick>>
  /** Stream `stock_at_time_trade` rows into `callback` without materialising the full response in memory. `callback(chunk: TradeTick[]) => void` is invoked once per server chunk; the chunk is freed before the next is fetched, so peak memory tracks a single chunk rather than the whole result. This is the memory-bounded companion to the `stockAtTimeTrade` method — prefer it for multi-day or full-universe pulls. The returned Promise resolves when the stream drains and rejects (typed like the buffered method) on a wire or decode error. Cancelling the Promise drops the in-flight request. `options` carries the same optional builder parameters and `timeoutMs` as the buffered method; the `callback` is the trailing argument. */
  stockAtTimeTradeStream(symbol: string, startDate: string | Date, endDate: string | Date, timeOfDay: string | Date, options: StockAtTimeTradeOptions | undefined | null, callback: ((arg: Array<TradeTick>) => void)): Promise<void>
  /**
   * Fetch the quote at a specific time of day across a date range.
   *
   * #### Real-time request:
   *   - Subscription tier standard or higher will default to NQB.
   *   - Real-time last BBO quote at-time_of_day-time from the Nasdaq Basic feed if the account has a stocks standard or pro subscription.
   *   - 15-minute delayed NBBO quote at-time_of_day-time from the UTP & CTA feeds account has the stocks value subscription subscription.
   *
   * #### Historical request:
   *   Returns the last NBBO quote reported by UTP & CTA feeds at a specified millisecond of the day.
   *
   * Defaults (upstream):
   * - `venue`: `"nqb"`
   */
  stockAtTimeQuote(symbol: string, startDate: string | Date, endDate: string | Date, timeOfDay: string | Date, options?: StockAtTimeQuoteOptions | undefined | null): Promise<Array<QuoteTick>>
  /** Stream `stock_at_time_quote` rows into `callback` without materialising the full response in memory. `callback(chunk: QuoteTick[]) => void` is invoked once per server chunk; the chunk is freed before the next is fetched, so peak memory tracks a single chunk rather than the whole result. This is the memory-bounded companion to the `stockAtTimeQuote` method — prefer it for multi-day or full-universe pulls. The returned Promise resolves when the stream drains and rejects (typed like the buffered method) on a wire or decode error. Cancelling the Promise drops the in-flight request. `options` carries the same optional builder parameters and `timeoutMs` as the buffered method; the `callback` is the trailing argument. */
  stockAtTimeQuoteStream(symbol: string, startDate: string | Date, endDate: string | Date, timeOfDay: string | Date, options: StockAtTimeQuoteOptions | undefined | null, callback: ((arg: Array<QuoteTick>) => void)): Promise<void>
  /**
   * List all available option underlying symbols.
   *
   * A symbol can be defined as a unique identifier for a stock / underlying asset. Common terms also include: root, ticker, and underlying. This endpoint returns all traded symbols for options. This endpoint is updated overnight.
   */
  optionListSymbols(options?: OptionListSymbolsOptions | undefined | null): Promise<Array<string>>
  /**
   * List available dates for an option contract by request type.
   *
   * Lists all dates of data that are available for an option with a given symbol, request type, and expiration.
   * This endpoint is updated overnight.
   *
   * Defaults (upstream):
   * - `strike`: `"*"`
   * - `right`: `"both"`
   */
  optionListDates(requestType: string, symbol: string, expiration: string | Date, options?: OptionListDatesOptions | undefined | null): Promise<Array<string>>
  /**
   * List available expiration dates for an option underlying.
   *
   * Lists all dates of expirations that are available for an option with a given symbol.
   * This endpoint is updated overnight.
   */
  optionListExpirations(symbol: string, options?: OptionListExpirationsOptions | undefined | null): Promise<Array<string>>
  /**
   * List available strike prices for an option at a given expiration.
   *
   * Lists all strikes that are available for an option with a given symbol and expiration date.
   * This endpoint is updated overnight.
   */
  optionListStrikes(symbol: string, expiration: string | Date, options?: OptionListStrikesOptions | undefined | null): Promise<Array<string>>
  /**
   * List all option contracts for a symbol on a given date.
   *
   * Lists all contracts that were traded or quoted on a particular date.
   *
   * If the ``symbol`` parameter is specified, the returned contracts will be filtered to match the symbol.
   * Multiple symbols can be specified by separating them with commas such as ``symbol=AAPL,SPY,AMD``
   * This endpoint is updated real-time.
   */
  optionListContracts(requestType: string, symbol: string, date: string | Date, options?: OptionListContractsOptions | undefined | null): Promise<Array<OptionContract>>
  /** Stream `option_list_contracts` rows into `callback` without materialising the full response in memory. `callback(chunk: OptionContract[]) => void` is invoked once per server chunk; the chunk is freed before the next is fetched, so peak memory tracks a single chunk rather than the whole result. This is the memory-bounded companion to the `optionListContracts` method — prefer it for multi-day or full-universe pulls. The returned Promise resolves when the stream drains and rejects (typed like the buffered method) on a wire or decode error. Cancelling the Promise drops the in-flight request. `options` carries the same optional builder parameters and `timeoutMs` as the buffered method; the `callback` is the trailing argument. */
  optionListContractsStream(requestType: string, symbol: string, date: string | Date, options: OptionListContractsOptions | undefined | null, callback: ((arg: Array<OptionContract>) => void)): Promise<void>
  /**
   * Get the latest OHLC snapshot for an option contract.
   *
   * - Retrieve a real-time last ohlc of an option contract for the trading day.
   * - You might need to change the default expiration date to a different date if it is past the current date.
   *
   * Defaults (upstream):
   * - `strike`: `"*"`
   * - `right`: `"both"`
   */
  optionSnapshotOHLC(symbol: string, expiration: string | Date, options?: OptionSnapshotOhlcOptions | undefined | null): Promise<Array<OhlcTick>>
  /**
   * Get the latest trade snapshot for an option contract.
   *
   * - Retrieve the real-time last trade of an option contract.
   * - You might need to change the default expiration date to a different date if it is past the current date.
   * - This endpoint will return no data if the market was closed for the day. Theta Data resets the snapshot cache at midnight ET every night.
   *
   * Defaults (upstream):
   * - `strike`: `"*"`
   * - `right`: `"both"`
   */
  optionSnapshotTrade(symbol: string, expiration: string | Date, options?: OptionSnapshotTradeOptions | undefined | null): Promise<Array<TradeTick>>
  /**
   * Get the latest NBBO quote snapshot for an option contract.
   *
   * - Retrieve a real-time last NBBO quote of an option contract.
   * - You might need to change the default expiration date to a different date if it is past the current date.
   * - This endpoint will return no data if the market was closed for the day. Theta Data resets the snapshot cache at midnight ET every night.
   *
   * Defaults (upstream):
   * - `strike`: `"*"`
   * - `right`: `"both"`
   */
  optionSnapshotQuote(symbol: string, expiration: string | Date, options?: OptionSnapshotQuoteOptions | undefined | null): Promise<Array<QuoteTick>>
  /**
   * Get the latest open interest snapshot for an option contract.
   *
   * - Retrieve the last open interest message of an option contract.
   * - Open interest is reported around 06:30 ET every morning by OPRA and reflects the open interest at the end of the previous trading day.
   * - You might need to change the default expiration date to a different date if it is past the current date.
   * - This endpoint will return no data if the market was closed for the day. Theta Data resets the snapshot cache at midnight ET every night.
   *
   * Defaults (upstream):
   * - `strike`: `"*"`
   * - `right`: `"both"`
   */
  optionSnapshotOpenInterest(symbol: string, expiration: string | Date, options?: OptionSnapshotOpenInterestOptions | undefined | null): Promise<Array<OpenInterestTick>>
  /**
   * Get the latest market value snapshot for an option contract.
   *
   * * Returns a real-time market value derived from the last NBBO quote of an option contract.
   *
   * Defaults (upstream):
   * - `strike`: `"*"`
   * - `right`: `"both"`
   */
  optionSnapshotMarketValue(symbol: string, expiration: string | Date, options?: OptionSnapshotMarketValueOptions | undefined | null): Promise<Array<MarketValueTick>>
  /**
   * Get implied volatility snapshot for an option contract (from ThetaData server).
   *
   * Returns implied volatilies calculated using the national best bid, mid, and ask price
   * of the option respectively. The underlying price represents whatever the last underlying price was at the
   * ``underlying_timestamp`` field. You can read more about how Theta Data calculates greeks
   * here.
   *
   * Defaults (upstream):
   * - `strike`: `"*"`
   * - `right`: `"both"`
   * - `rate_type`: `"sofr"`
   * - `version`: `"latest"`
   * - `use_market_value`: `false`
   */
  optionSnapshotGreeksImpliedVolatility(symbol: string, expiration: string | Date, options?: OptionSnapshotGreeksImpliedVolatilityOptions | undefined | null): Promise<Array<IvTick>>
  /**
   * Get all Greeks snapshot for an option contract (from ThetaData server).
   *
   * - Retrieve a real-time last greeks calculation for all option contracts that lie on a provided expiration.
   * - You might need to change the default expiration date to a different date if it is past the current date. Some quotes are omitted in the example to reduce the space of the sample output.
   * - Make `expiration` * if you want to get the snapshot for every expiration chain for the underlying.
   * > This endpoint will return no data if the market was closed for the day. Theta Data resets the snapshot cache at midnight ET every night.
   *
   * Defaults (upstream):
   * - `strike`: `"*"`
   * - `right`: `"both"`
   * - `rate_type`: `"sofr"`
   * - `version`: `"latest"`
   * - `use_market_value`: `false`
   */
  optionSnapshotGreeksAll(symbol: string, expiration: string | Date, options?: OptionSnapshotGreeksAllOptions | undefined | null): Promise<Array<GreeksAllTick>>
  /**
   * Get first-order Greeks snapshot (delta, theta, rho) for an option contract.
   *
   * - Retrieve a real-time last greeks calculation for all option contracts that lie on a provided expiration.
   * - You might need to change the default expiration date to a different date if it is past the current date. Some quotes are omitted in the example to reduce the space of the sample output.
   * - Make `expiration` * if you want to get the snapshot for every expiration chain for the underlying.
   * > This endpoint will return no data if the market was closed for the day. Theta Data resets the snapshot cache at midnight ET every night.
   *
   * Defaults (upstream):
   * - `strike`: `"*"`
   * - `right`: `"both"`
   * - `rate_type`: `"sofr"`
   * - `version`: `"latest"`
   * - `use_market_value`: `false`
   */
  optionSnapshotGreeksFirstOrder(symbol: string, expiration: string | Date, options?: OptionSnapshotGreeksFirstOrderOptions | undefined | null): Promise<Array<GreeksFirstOrderTick>>
  /**
   * Get second-order Greeks snapshot (gamma, vanna, charm) for an option contract.
   *
   * - Retrieve a real-time last second order greeks calculation for all option contracts that lie on a provided expiration.
   * - You might need to change the default expiration date to a different date if it is past the current date. Some quotes are omitted in the example to reduce the space of the sample output.
   * - Make `expiration` * if you want to get the snapshot for every expiration chain for the underlying.
   * > This endpoint will return no data if the market was closed for the day. Theta Data resets the snapshot cache at midnight ET every night.
   *
   * Defaults (upstream):
   * - `strike`: `"*"`
   * - `right`: `"both"`
   * - `rate_type`: `"sofr"`
   * - `version`: `"latest"`
   * - `use_market_value`: `false`
   */
  optionSnapshotGreeksSecondOrder(symbol: string, expiration: string | Date, options?: OptionSnapshotGreeksSecondOrderOptions | undefined | null): Promise<Array<GreeksSecondOrderTick>>
  /**
   * Get third-order Greeks snapshot (speed, color, ultima) for an option contract.
   *
   * - Retrieve a real-time last third order greeks calculation for all option contracts that lie on a provided expiration.
   * - You might need to change the default expiration date to a different date if it is past the current date. Some quotes are omitted in the example to reduce the space of the sample output.
   * - Make `expiration` * if you want to get the snapshot for every expiration chain for the underlying.
   * > This endpoint will return no data if the market was closed for the day. Theta Data resets the snapshot cache at midnight ET every night.
   *
   * Defaults (upstream):
   * - `strike`: `"*"`
   * - `right`: `"both"`
   * - `rate_type`: `"sofr"`
   * - `version`: `"latest"`
   * - `use_market_value`: `false`
   */
  optionSnapshotGreeksThirdOrder(symbol: string, expiration: string | Date, options?: OptionSnapshotGreeksThirdOrderOptions | undefined | null): Promise<Array<GreeksThirdOrderTick>>
  /**
   * Fetch end-of-day option data for a contract over a date range.
   *
   * - Since OPRA does not provide a national EOD report for options, Theta Data generates a national EOD report at 17:15 ET each day.
   * - ``created`` represents the datetime the report was generated and ``last_trade`` represents the datetime of the last trade.
   * - The quote in the response represents the last NBBO reported by OPRA at the time of report generation.
   * - You can read more about EOD & OHLC data here.
   *
   * Defaults (upstream):
   * - `strike`: `"*"`
   * - `right`: `"both"`
   */
  optionHistoryEOD(symbol: string, expiration: string | Date, startDate: string | Date, endDate: string | Date, options?: OptionHistoryEodOptions | undefined | null): Promise<Array<EodTick>>
  /** Stream `option_history_eod` rows into `callback` without materialising the full response in memory. `callback(chunk: EodTick[]) => void` is invoked once per server chunk; the chunk is freed before the next is fetched, so peak memory tracks a single chunk rather than the whole result. This is the memory-bounded companion to the `optionHistoryEOD` method — prefer it for multi-day or full-universe pulls. The returned Promise resolves when the stream drains and rejects (typed like the buffered method) on a wire or decode error. Cancelling the Promise drops the in-flight request. `options` carries the same optional builder parameters and `timeoutMs` as the buffered method; the `callback` is the trailing argument. */
  optionHistoryEODStream(symbol: string, expiration: string | Date, startDate: string | Date, endDate: string | Date, options: OptionHistoryEodOptions | undefined | null, callback: ((arg: Array<EodTick>) => void)): Promise<void>
  /**
   * Fetch intraday OHLC bars for an option contract.
   *
   * - Aggregated OHLC bars that use SIP rules for each bar.
   * - Time timestamp of the bar represents the opening time of the bar. For a trade to be part of the bar:  ``bar timestamp`` <= ``trade time`` < ``bar timestamp + interval``.
   * - Multi-day requests are limited to 1 month of data.
   *
   * Defaults (upstream):
   * - `strike`: `"*"`
   * - `right`: `"both"`
   * - `interval`: `"1s"`
   * - `start_time`: `"09:30:00"`
   * - `end_time`: `"16:00:00"`
   */
  optionHistoryOHLC(symbol: string, expiration: string | Date, date: string | Date, options?: OptionHistoryOhlcOptions | undefined | null): Promise<Array<OhlcTick>>
  /** Stream `option_history_ohlc` rows into `callback` without materialising the full response in memory. `callback(chunk: OhlcTick[]) => void` is invoked once per server chunk; the chunk is freed before the next is fetched, so peak memory tracks a single chunk rather than the whole result. This is the memory-bounded companion to the `optionHistoryOHLC` method — prefer it for multi-day or full-universe pulls. The returned Promise resolves when the stream drains and rejects (typed like the buffered method) on a wire or decode error. Cancelling the Promise drops the in-flight request. `options` carries the same optional builder parameters and `timeoutMs` as the buffered method; the `callback` is the trailing argument. */
  optionHistoryOHLCStream(symbol: string, expiration: string | Date, date: string | Date, options: OptionHistoryOhlcOptions | undefined | null, callback: ((arg: Array<OhlcTick>) => void)): Promise<void>
  /**
   * Fetch all trades for an option contract on a given date.
   *
   * - Returns every trade reported by OPRA.
   * - Trade condition mappings can be found here.
   * - Extended trade conditions are not reported by OPRA for options, so they can be ignored.
   * - Multi-day requests are limited to 1 month of data, and must specify an expiration.
   *
   * Defaults (upstream):
   * - `strike`: `"*"`
   * - `right`: `"both"`
   * - `start_time`: `"09:30:00"`
   * - `end_time`: `"16:00:00"`
   */
  optionHistoryTrade(symbol: string, expiration: string | Date, date: string | Date, options?: OptionHistoryTradeOptions | undefined | null): Promise<Array<TradeTick>>
  /** Stream `option_history_trade` rows into `callback` without materialising the full response in memory. `callback(chunk: TradeTick[]) => void` is invoked once per server chunk; the chunk is freed before the next is fetched, so peak memory tracks a single chunk rather than the whole result. This is the memory-bounded companion to the `optionHistoryTrade` method — prefer it for multi-day or full-universe pulls. The returned Promise resolves when the stream drains and rejects (typed like the buffered method) on a wire or decode error. Cancelling the Promise drops the in-flight request. `options` carries the same optional builder parameters and `timeoutMs` as the buffered method; the `callback` is the trailing argument. */
  optionHistoryTradeStream(symbol: string, expiration: string | Date, date: string | Date, options: OptionHistoryTradeOptions | undefined | null, callback: ((arg: Array<TradeTick>) => void)): Promise<void>
  /**
   * Fetch NBBO quotes for an option contract on a given date.
   *
   * - Returns every NBBO quote reported by OPRA.
   * - If the ``interval`` parameter is specified, the quote for each interval represents the last quote at the interval's timestamp.
   * - Multi-day requests are limited to 1 month of data, and must specify an expiration.
   *
   * Defaults (upstream):
   * - `strike`: `"*"`
   * - `right`: `"both"`
   * - `interval`: `"1s"`
   * - `start_time`: `"09:30:00"`
   * - `end_time`: `"16:00:00"`
   */
  optionHistoryQuote(symbol: string, expiration: string | Date, date: string | Date, options?: OptionHistoryQuoteOptions | undefined | null): Promise<Array<QuoteTick>>
  /** Stream `option_history_quote` rows into `callback` without materialising the full response in memory. `callback(chunk: QuoteTick[]) => void` is invoked once per server chunk; the chunk is freed before the next is fetched, so peak memory tracks a single chunk rather than the whole result. This is the memory-bounded companion to the `optionHistoryQuote` method — prefer it for multi-day or full-universe pulls. The returned Promise resolves when the stream drains and rejects (typed like the buffered method) on a wire or decode error. Cancelling the Promise drops the in-flight request. `options` carries the same optional builder parameters and `timeoutMs` as the buffered method; the `callback` is the trailing argument. */
  optionHistoryQuoteStream(symbol: string, expiration: string | Date, date: string | Date, options: OptionHistoryQuoteOptions | undefined | null, callback: ((arg: Array<QuoteTick>) => void)): Promise<void>
  /**
   * Fetch combined trade + quote ticks for an option contract.
   *
   * - Returns every trade reported by OPRA paired with the last NBBO quote reported by OPRA at the time of trade.
   * - A quote is matched with a trade if its timestamp ``<=`` the trade timestamp.
   * - To match trades with quotes timestamps that are ``<`` the trade timestamp, specify the ``exclusive``parameter to ``true``. After thorough testing, we have determined that using ``exclusive=true`` might yield better results for various applications.
   * - Multi-day requests are limited to 1 month of data, and must specify an expiration.
   *
   * Defaults (upstream):
   * - `strike`: `"*"`
   * - `right`: `"both"`
   * - `start_time`: `"09:30:00"`
   * - `end_time`: `"16:00:00"`
   * - `exclusive`: `true`
   */
  optionHistoryTradeQuote(symbol: string, expiration: string | Date, date: string | Date, options?: OptionHistoryTradeQuoteOptions | undefined | null): Promise<Array<TradeQuoteTick>>
  /** Stream `option_history_trade_quote` rows into `callback` without materialising the full response in memory. `callback(chunk: TradeQuoteTick[]) => void` is invoked once per server chunk; the chunk is freed before the next is fetched, so peak memory tracks a single chunk rather than the whole result. This is the memory-bounded companion to the `optionHistoryTradeQuote` method — prefer it for multi-day or full-universe pulls. The returned Promise resolves when the stream drains and rejects (typed like the buffered method) on a wire or decode error. Cancelling the Promise drops the in-flight request. `options` carries the same optional builder parameters and `timeoutMs` as the buffered method; the `callback` is the trailing argument. */
  optionHistoryTradeQuoteStream(symbol: string, expiration: string | Date, date: string | Date, options: OptionHistoryTradeQuoteOptions | undefined | null, callback: ((arg: Array<TradeQuoteTick>) => void)): Promise<void>
  /**
   * Fetch open interest history for an option contract.
   *
   * - Open Interest is normally reported once per day by OPRA at approximately 06:30 ET.
   * - A new open interest message might not be sent by OPRA if there is no open interest for the option contract.
   * - The reported open interest represents the open interest at the end of the previous trading day.
   *
   * Defaults (upstream):
   * - `strike`: `"*"`
   * - `right`: `"both"`
   */
  optionHistoryOpenInterest(symbol: string, expiration: string | Date, date: string | Date, options?: OptionHistoryOpenInterestOptions | undefined | null): Promise<Array<OpenInterestTick>>
  /** Stream `option_history_open_interest` rows into `callback` without materialising the full response in memory. `callback(chunk: OpenInterestTick[]) => void` is invoked once per server chunk; the chunk is freed before the next is fetched, so peak memory tracks a single chunk rather than the whole result. This is the memory-bounded companion to the `optionHistoryOpenInterest` method — prefer it for multi-day or full-universe pulls. The returned Promise resolves when the stream drains and rejects (typed like the buffered method) on a wire or decode error. Cancelling the Promise drops the in-flight request. `options` carries the same optional builder parameters and `timeoutMs` as the buffered method; the `callback` is the trailing argument. */
  optionHistoryOpenInterestStream(symbol: string, expiration: string | Date, date: string | Date, options: OptionHistoryOpenInterestOptions | undefined | null, callback: ((arg: Array<OpenInterestTick>) => void)): Promise<void>
  /**
   * Fetch end-of-day Greeks history for an option contract.
   *
   * - Returns the data for all contracts that share the same provided symbol and expiration.
   * - Uses Theta Data's EOD reports that get generated at 17:15 ET each day. The closing option price and closing underlying price are used for the greeks calculation.
   * - **Set `expiration` to ``*`` if you want to retrieve data for every option that shares the same ``symbol``. (note: Any ``expiration=*`` must be requested day by day)**
   *
   * Defaults (upstream):
   * - `strike`: `"*"`
   * - `right`: `"both"`
   * - `rate_type`: `"sofr"`
   * - `version`: `"latest"`
   * - `underlyer_use_nbbo`: `false`
   */
  optionHistoryGreeksEOD(symbol: string, expiration: string | Date, startDate: string | Date, endDate: string | Date, options?: OptionHistoryGreeksEodOptions | undefined | null): Promise<Array<GreeksEodTick>>
  /** Stream `option_history_greeks_eod` rows into `callback` without materialising the full response in memory. `callback(chunk: GreeksEodTick[]) => void` is invoked once per server chunk; the chunk is freed before the next is fetched, so peak memory tracks a single chunk rather than the whole result. This is the memory-bounded companion to the `optionHistoryGreeksEOD` method — prefer it for multi-day or full-universe pulls. The returned Promise resolves when the stream drains and rejects (typed like the buffered method) on a wire or decode error. Cancelling the Promise drops the in-flight request. `options` carries the same optional builder parameters and `timeoutMs` as the buffered method; the `callback` is the trailing argument. */
  optionHistoryGreeksEODStream(symbol: string, expiration: string | Date, startDate: string | Date, endDate: string | Date, options: OptionHistoryGreeksEodOptions | undefined | null, callback: ((arg: Array<GreeksEodTick>) => void)): Promise<void>
  /**
   * Fetch all Greeks history for an option contract (intraday, sampled by interval).
   *
   * - Returns the data for all contracts that share the same provided symbol and expiration.
   * - Calculated using the option and underlying midpoint price. If an interval size is specified (*highly recommended*), the option quote used in the calculation follows the same rules as the quote endpoint.
   * - The underlying price represents whatever the last underlying price was at the ``timestamp`` field. You can read more about how Theta Data calculates greeks here.
   * - Multi-day requests are limited to 1 month of data.
   *
   * Defaults (upstream):
   * - `strike`: `"*"`
   * - `right`: `"both"`
   * - `interval`: `"1s"`
   * - `start_time`: `"09:30:00"`
   * - `end_time`: `"16:00:00"`
   * - `rate_type`: `"sofr"`
   * - `version`: `"latest"`
   */
  optionHistoryGreeksAll(symbol: string, expiration: string | Date, date: string | Date, options?: OptionHistoryGreeksAllOptions | undefined | null): Promise<Array<GreeksAllTick>>
  /** Stream `option_history_greeks_all` rows into `callback` without materialising the full response in memory. `callback(chunk: GreeksAllTick[]) => void` is invoked once per server chunk; the chunk is freed before the next is fetched, so peak memory tracks a single chunk rather than the whole result. This is the memory-bounded companion to the `optionHistoryGreeksAll` method — prefer it for multi-day or full-universe pulls. The returned Promise resolves when the stream drains and rejects (typed like the buffered method) on a wire or decode error. Cancelling the Promise drops the in-flight request. `options` carries the same optional builder parameters and `timeoutMs` as the buffered method; the `callback` is the trailing argument. */
  optionHistoryGreeksAllStream(symbol: string, expiration: string | Date, date: string | Date, options: OptionHistoryGreeksAllOptions | undefined | null, callback: ((arg: Array<GreeksAllTick>) => void)): Promise<void>
  /**
   * Fetch all Greeks on each trade for an option contract.
   *
   * - Returns the data for all contracts that share the same provided symbol and expiration.
   * - Calculates greeks for every trade reported by OPRA.
   * - The underlying price represents whatever the last underlying price was at the ``timestamp`` field. You can read more about how Theta Data calculates greeks here.
   * - Multi-day requests are limited to 1 month of data, and must specify an expiration.
   *
   * Defaults (upstream):
   * - `strike`: `"*"`
   * - `right`: `"both"`
   * - `start_time`: `"09:30:00"`
   * - `end_time`: `"16:00:00"`
   * - `rate_type`: `"sofr"`
   * - `version`: `"latest"`
   */
  optionHistoryTradeGreeksAll(symbol: string, expiration: string | Date, date: string | Date, options?: OptionHistoryTradeGreeksAllOptions | undefined | null): Promise<Array<TradeGreeksAllTick>>
  /** Stream `option_history_trade_greeks_all` rows into `callback` without materialising the full response in memory. `callback(chunk: TradeGreeksAllTick[]) => void` is invoked once per server chunk; the chunk is freed before the next is fetched, so peak memory tracks a single chunk rather than the whole result. This is the memory-bounded companion to the `optionHistoryTradeGreeksAll` method — prefer it for multi-day or full-universe pulls. The returned Promise resolves when the stream drains and rejects (typed like the buffered method) on a wire or decode error. Cancelling the Promise drops the in-flight request. `options` carries the same optional builder parameters and `timeoutMs` as the buffered method; the `callback` is the trailing argument. */
  optionHistoryTradeGreeksAllStream(symbol: string, expiration: string | Date, date: string | Date, options: OptionHistoryTradeGreeksAllOptions | undefined | null, callback: ((arg: Array<TradeGreeksAllTick>) => void)): Promise<void>
  /**
   * Fetch first-order Greeks history (intraday, sampled by interval).
   *
   * - Returns the data for all contracts that share the same provided symbol and expiration.
   * - Calculated using the option and underlying midpoint price. If an interval size is specified (*highly recommended*), the option quote used in the calculation follows the same rules as the quote endpoint.
   * - The underlying price represents whatever the last underlying price was at the ``timestamp`` field. You can read more about how Theta Data calculates greeks here.
   * - Multi-day requests are limited to 1 month of data.
   *
   * Defaults (upstream):
   * - `strike`: `"*"`
   * - `right`: `"both"`
   * - `interval`: `"1s"`
   * - `start_time`: `"09:30:00"`
   * - `end_time`: `"16:00:00"`
   * - `rate_type`: `"sofr"`
   * - `version`: `"latest"`
   */
  optionHistoryGreeksFirstOrder(symbol: string, expiration: string | Date, date: string | Date, options?: OptionHistoryGreeksFirstOrderOptions | undefined | null): Promise<Array<GreeksFirstOrderTick>>
  /** Stream `option_history_greeks_first_order` rows into `callback` without materialising the full response in memory. `callback(chunk: GreeksFirstOrderTick[]) => void` is invoked once per server chunk; the chunk is freed before the next is fetched, so peak memory tracks a single chunk rather than the whole result. This is the memory-bounded companion to the `optionHistoryGreeksFirstOrder` method — prefer it for multi-day or full-universe pulls. The returned Promise resolves when the stream drains and rejects (typed like the buffered method) on a wire or decode error. Cancelling the Promise drops the in-flight request. `options` carries the same optional builder parameters and `timeoutMs` as the buffered method; the `callback` is the trailing argument. */
  optionHistoryGreeksFirstOrderStream(symbol: string, expiration: string | Date, date: string | Date, options: OptionHistoryGreeksFirstOrderOptions | undefined | null, callback: ((arg: Array<GreeksFirstOrderTick>) => void)): Promise<void>
  /**
   * Fetch first-order Greeks on each trade for an option contract.
   *
   * - Returns the data for all contracts that share the same provided symbol and expiration.
   * - Calculates greeks for every trade reported by OPRA.
   * - The underlying price represents whatever the last underlying price was at the ``timestamp`` field. You can read more about how Theta Data calculates greeks here.
   * - Multi-day requests are limited to 1 month of data, and must specify an expiration.
   *
   * Defaults (upstream):
   * - `strike`: `"*"`
   * - `right`: `"both"`
   * - `start_time`: `"09:30:00"`
   * - `end_time`: `"16:00:00"`
   * - `rate_type`: `"sofr"`
   * - `version`: `"latest"`
   */
  optionHistoryTradeGreeksFirstOrder(symbol: string, expiration: string | Date, date: string | Date, options?: OptionHistoryTradeGreeksFirstOrderOptions | undefined | null): Promise<Array<TradeGreeksFirstOrderTick>>
  /** Stream `option_history_trade_greeks_first_order` rows into `callback` without materialising the full response in memory. `callback(chunk: TradeGreeksFirstOrderTick[]) => void` is invoked once per server chunk; the chunk is freed before the next is fetched, so peak memory tracks a single chunk rather than the whole result. This is the memory-bounded companion to the `optionHistoryTradeGreeksFirstOrder` method — prefer it for multi-day or full-universe pulls. The returned Promise resolves when the stream drains and rejects (typed like the buffered method) on a wire or decode error. Cancelling the Promise drops the in-flight request. `options` carries the same optional builder parameters and `timeoutMs` as the buffered method; the `callback` is the trailing argument. */
  optionHistoryTradeGreeksFirstOrderStream(symbol: string, expiration: string | Date, date: string | Date, options: OptionHistoryTradeGreeksFirstOrderOptions | undefined | null, callback: ((arg: Array<TradeGreeksFirstOrderTick>) => void)): Promise<void>
  /**
   * Fetch second-order Greeks history (intraday, sampled by interval).
   *
   * - Returns the data for all contracts that share the same provided symbol and expiration.
   * - Calculated using the option and underlying midpoint price. If an interval size is specified (*highly recommended*), the option quote used in the calculation follows the same rules as the quote endpoint.
   * - The underlying price represents whatever the last underlying price was at the ``timestamp`` field. You can read more about how Theta Data calculates greeks here.
   * - Multi-day requests are limited to 1 month of data.
   *
   * Defaults (upstream):
   * - `strike`: `"*"`
   * - `right`: `"both"`
   * - `interval`: `"1s"`
   * - `start_time`: `"09:30:00"`
   * - `end_time`: `"16:00:00"`
   * - `rate_type`: `"sofr"`
   * - `version`: `"latest"`
   */
  optionHistoryGreeksSecondOrder(symbol: string, expiration: string | Date, date: string | Date, options?: OptionHistoryGreeksSecondOrderOptions | undefined | null): Promise<Array<GreeksSecondOrderTick>>
  /** Stream `option_history_greeks_second_order` rows into `callback` without materialising the full response in memory. `callback(chunk: GreeksSecondOrderTick[]) => void` is invoked once per server chunk; the chunk is freed before the next is fetched, so peak memory tracks a single chunk rather than the whole result. This is the memory-bounded companion to the `optionHistoryGreeksSecondOrder` method — prefer it for multi-day or full-universe pulls. The returned Promise resolves when the stream drains and rejects (typed like the buffered method) on a wire or decode error. Cancelling the Promise drops the in-flight request. `options` carries the same optional builder parameters and `timeoutMs` as the buffered method; the `callback` is the trailing argument. */
  optionHistoryGreeksSecondOrderStream(symbol: string, expiration: string | Date, date: string | Date, options: OptionHistoryGreeksSecondOrderOptions | undefined | null, callback: ((arg: Array<GreeksSecondOrderTick>) => void)): Promise<void>
  /**
   * Fetch second-order Greeks on each trade for an option contract.
   *
   * - Returns the data for all contracts that share the same provided symbol and expiration.
   * - Calculates greeks for every trade reported by OPRA.
   * - The underlying price represents whatever the last underlying price was at the ``timestamp`` field. You can read more about how Theta Data calculates greeks here.
   * - Multi-day requests are limited to 1 month of data, and must specify an expiration.
   *
   * Defaults (upstream):
   * - `strike`: `"*"`
   * - `right`: `"both"`
   * - `start_time`: `"09:30:00"`
   * - `end_time`: `"16:00:00"`
   * - `rate_type`: `"sofr"`
   * - `version`: `"latest"`
   */
  optionHistoryTradeGreeksSecondOrder(symbol: string, expiration: string | Date, date: string | Date, options?: OptionHistoryTradeGreeksSecondOrderOptions | undefined | null): Promise<Array<TradeGreeksSecondOrderTick>>
  /** Stream `option_history_trade_greeks_second_order` rows into `callback` without materialising the full response in memory. `callback(chunk: TradeGreeksSecondOrderTick[]) => void` is invoked once per server chunk; the chunk is freed before the next is fetched, so peak memory tracks a single chunk rather than the whole result. This is the memory-bounded companion to the `optionHistoryTradeGreeksSecondOrder` method — prefer it for multi-day or full-universe pulls. The returned Promise resolves when the stream drains and rejects (typed like the buffered method) on a wire or decode error. Cancelling the Promise drops the in-flight request. `options` carries the same optional builder parameters and `timeoutMs` as the buffered method; the `callback` is the trailing argument. */
  optionHistoryTradeGreeksSecondOrderStream(symbol: string, expiration: string | Date, date: string | Date, options: OptionHistoryTradeGreeksSecondOrderOptions | undefined | null, callback: ((arg: Array<TradeGreeksSecondOrderTick>) => void)): Promise<void>
  /**
   * Fetch third-order Greeks history (intraday, sampled by interval).
   *
   * - Returns the data for all contracts that share the same provided symbol and expiration.
   * - Calculated using the option and underlying midpoint price. If an interval size is specified (*highly recommended*), the option quote used in the calculation follows the same rules as the quote endpoint.
   * - The underlying price represents whatever the last underlying price was at the ``timestamp`` field. You can read more about how Theta Data calculates greeks here.
   * - Multi-day requests are limited to 1 month of data.
   *
   * Defaults (upstream):
   * - `strike`: `"*"`
   * - `right`: `"both"`
   * - `interval`: `"1s"`
   * - `start_time`: `"09:30:00"`
   * - `end_time`: `"16:00:00"`
   * - `rate_type`: `"sofr"`
   * - `version`: `"latest"`
   */
  optionHistoryGreeksThirdOrder(symbol: string, expiration: string | Date, date: string | Date, options?: OptionHistoryGreeksThirdOrderOptions | undefined | null): Promise<Array<GreeksThirdOrderTick>>
  /** Stream `option_history_greeks_third_order` rows into `callback` without materialising the full response in memory. `callback(chunk: GreeksThirdOrderTick[]) => void` is invoked once per server chunk; the chunk is freed before the next is fetched, so peak memory tracks a single chunk rather than the whole result. This is the memory-bounded companion to the `optionHistoryGreeksThirdOrder` method — prefer it for multi-day or full-universe pulls. The returned Promise resolves when the stream drains and rejects (typed like the buffered method) on a wire or decode error. Cancelling the Promise drops the in-flight request. `options` carries the same optional builder parameters and `timeoutMs` as the buffered method; the `callback` is the trailing argument. */
  optionHistoryGreeksThirdOrderStream(symbol: string, expiration: string | Date, date: string | Date, options: OptionHistoryGreeksThirdOrderOptions | undefined | null, callback: ((arg: Array<GreeksThirdOrderTick>) => void)): Promise<void>
  /**
   * Fetch third-order Greeks on each trade for an option contract.
   *
   * - Returns the data for all contracts that share the same provided symbol and expiration.
   * - Calculates greeks for every trade reported by OPRA.
   * - The underlying price represents whatever the last underlying price was at the ``timestamp`` field. You can read more about how Theta Data calculates greeks here.
   * - Multi-day requests are limited to 1 month of data, and must specify an expiration.
   *
   * Defaults (upstream):
   * - `strike`: `"*"`
   * - `right`: `"both"`
   * - `start_time`: `"09:30:00"`
   * - `end_time`: `"16:00:00"`
   * - `rate_type`: `"sofr"`
   * - `version`: `"latest"`
   */
  optionHistoryTradeGreeksThirdOrder(symbol: string, expiration: string | Date, date: string | Date, options?: OptionHistoryTradeGreeksThirdOrderOptions | undefined | null): Promise<Array<TradeGreeksThirdOrderTick>>
  /** Stream `option_history_trade_greeks_third_order` rows into `callback` without materialising the full response in memory. `callback(chunk: TradeGreeksThirdOrderTick[]) => void` is invoked once per server chunk; the chunk is freed before the next is fetched, so peak memory tracks a single chunk rather than the whole result. This is the memory-bounded companion to the `optionHistoryTradeGreeksThirdOrder` method — prefer it for multi-day or full-universe pulls. The returned Promise resolves when the stream drains and rejects (typed like the buffered method) on a wire or decode error. Cancelling the Promise drops the in-flight request. `options` carries the same optional builder parameters and `timeoutMs` as the buffered method; the `callback` is the trailing argument. */
  optionHistoryTradeGreeksThirdOrderStream(symbol: string, expiration: string | Date, date: string | Date, options: OptionHistoryTradeGreeksThirdOrderOptions | undefined | null, callback: ((arg: Array<TradeGreeksThirdOrderTick>) => void)): Promise<void>
  /**
   * Fetch implied volatility history (intraday, sampled by interval).
   *
   * - Returns implied volatilies calculated using the national best bid, mid, and ask price of the option respectively.
   * - The underlying price represents whatever the last underlying price was at the ``timestamp`` field. You can read more about how Theta Data calculates greeks here.
   * - Multi-day requests are limited to 1 month of data.
   *
   * Defaults (upstream):
   * - `strike`: `"*"`
   * - `right`: `"both"`
   * - `interval`: `"1s"`
   * - `start_time`: `"09:30:00"`
   * - `end_time`: `"16:00:00"`
   * - `rate_type`: `"sofr"`
   * - `version`: `"latest"`
   */
  optionHistoryGreeksImpliedVolatility(symbol: string, expiration: string | Date, date: string | Date, options?: OptionHistoryGreeksImpliedVolatilityOptions | undefined | null): Promise<Array<IvTick>>
  /** Stream `option_history_greeks_implied_volatility` rows into `callback` without materialising the full response in memory. `callback(chunk: IvTick[]) => void` is invoked once per server chunk; the chunk is freed before the next is fetched, so peak memory tracks a single chunk rather than the whole result. This is the memory-bounded companion to the `optionHistoryGreeksImpliedVolatility` method — prefer it for multi-day or full-universe pulls. The returned Promise resolves when the stream drains and rejects (typed like the buffered method) on a wire or decode error. Cancelling the Promise drops the in-flight request. `options` carries the same optional builder parameters and `timeoutMs` as the buffered method; the `callback` is the trailing argument. */
  optionHistoryGreeksImpliedVolatilityStream(symbol: string, expiration: string | Date, date: string | Date, options: OptionHistoryGreeksImpliedVolatilityOptions | undefined | null, callback: ((arg: Array<IvTick>) => void)): Promise<void>
  /**
   * Fetch implied volatility on each trade for an option contract.
   *
   * - Returns implied volatilies calculated using the trade reported by OPRA.
   * - The underlying price represents whatever the last underlying price was at the ``timestamp`` field. You can read more about how Theta Data calculates greeks here.
   * - Multi-day requests are limited to 1 month of data, and must specify an expiration.
   *
   * Defaults (upstream):
   * - `strike`: `"*"`
   * - `right`: `"both"`
   * - `start_time`: `"09:30:00"`
   * - `end_time`: `"16:00:00"`
   * - `rate_type`: `"sofr"`
   * - `version`: `"latest"`
   */
  optionHistoryTradeGreeksImpliedVolatility(symbol: string, expiration: string | Date, date: string | Date, options?: OptionHistoryTradeGreeksImpliedVolatilityOptions | undefined | null): Promise<Array<TradeGreeksImpliedVolatilityTick>>
  /** Stream `option_history_trade_greeks_implied_volatility` rows into `callback` without materialising the full response in memory. `callback(chunk: TradeGreeksImpliedVolatilityTick[]) => void` is invoked once per server chunk; the chunk is freed before the next is fetched, so peak memory tracks a single chunk rather than the whole result. This is the memory-bounded companion to the `optionHistoryTradeGreeksImpliedVolatility` method — prefer it for multi-day or full-universe pulls. The returned Promise resolves when the stream drains and rejects (typed like the buffered method) on a wire or decode error. Cancelling the Promise drops the in-flight request. `options` carries the same optional builder parameters and `timeoutMs` as the buffered method; the `callback` is the trailing argument. */
  optionHistoryTradeGreeksImpliedVolatilityStream(symbol: string, expiration: string | Date, date: string | Date, options: OptionHistoryTradeGreeksImpliedVolatilityOptions | undefined | null, callback: ((arg: Array<TradeGreeksImpliedVolatilityTick>) => void)): Promise<void>
  /**
   * Fetch the trade at a specific time of day across a date range for an option.
   *
   * - Returns the last trade reported by OPRA at a specified millisecond of the day.
   * - Trade condition mappings can be found here.
   * - Extended trade conditions are not reported by OPRA for options, so they can be ignored.
   * - The ``time_of_day``parameter represents the 00:00:00.000 ET that the trade should be provided for.
   *
   * Defaults (upstream):
   * - `strike`: `"*"`
   * - `right`: `"both"`
   */
  optionAtTimeTrade(symbol: string, expiration: string | Date, startDate: string | Date, endDate: string | Date, timeOfDay: string | Date, options?: OptionAtTimeTradeOptions | undefined | null): Promise<Array<TradeTick>>
  /** Stream `option_at_time_trade` rows into `callback` without materialising the full response in memory. `callback(chunk: TradeTick[]) => void` is invoked once per server chunk; the chunk is freed before the next is fetched, so peak memory tracks a single chunk rather than the whole result. This is the memory-bounded companion to the `optionAtTimeTrade` method — prefer it for multi-day or full-universe pulls. The returned Promise resolves when the stream drains and rejects (typed like the buffered method) on a wire or decode error. Cancelling the Promise drops the in-flight request. `options` carries the same optional builder parameters and `timeoutMs` as the buffered method; the `callback` is the trailing argument. */
  optionAtTimeTradeStream(symbol: string, expiration: string | Date, startDate: string | Date, endDate: string | Date, timeOfDay: string | Date, options: OptionAtTimeTradeOptions | undefined | null, callback: ((arg: Array<TradeTick>) => void)): Promise<void>
  /**
   * Fetch the quote at a specific time of day across a date range for an option.
   *
   * - Returns the last NBBO quote reported by OPRA at a specified millisecond of the day.
   * - The ``time_of_day``parameter represents the 00:00:00.000 ET that the quote should be provided for.
   *
   * Defaults (upstream):
   * - `strike`: `"*"`
   * - `right`: `"both"`
   */
  optionAtTimeQuote(symbol: string, expiration: string | Date, startDate: string | Date, endDate: string | Date, timeOfDay: string | Date, options?: OptionAtTimeQuoteOptions | undefined | null): Promise<Array<QuoteTick>>
  /** Stream `option_at_time_quote` rows into `callback` without materialising the full response in memory. `callback(chunk: QuoteTick[]) => void` is invoked once per server chunk; the chunk is freed before the next is fetched, so peak memory tracks a single chunk rather than the whole result. This is the memory-bounded companion to the `optionAtTimeQuote` method — prefer it for multi-day or full-universe pulls. The returned Promise resolves when the stream drains and rejects (typed like the buffered method) on a wire or decode error. Cancelling the Promise drops the in-flight request. `options` carries the same optional builder parameters and `timeoutMs` as the buffered method; the `callback` is the trailing argument. */
  optionAtTimeQuoteStream(symbol: string, expiration: string | Date, startDate: string | Date, endDate: string | Date, timeOfDay: string | Date, options: OptionAtTimeQuoteOptions | undefined | null, callback: ((arg: Array<QuoteTick>) => void)): Promise<void>
  /**
   * List all available index symbols.
   *
   * A symbol can be defined as a unique identifier for a stock / underlying asset. Common terms also include: root, ticker, and underlying. This endpoint returns all traded symbols for options. This endpoint is updated overnight.
   */
  indexListSymbols(options?: IndexListSymbolsOptions | undefined | null): Promise<Array<string>>
  /**
   * List available dates for an index symbol.
   *
   * Lists all dates of data that are available for a index with a given request type and symbol. This endpoint is updated overnight.
   */
  indexListDates(symbol: string, options?: IndexListDatesOptions | undefined | null): Promise<Array<string>>
  /**
   * Get the latest OHLC snapshot for one or more indices.
   *
   * - Retrieves the real-time current day OHLC.
   * - Exchanges typically generate a price report every second for popular indices like SPX.
   */
  indexSnapshotOHLC(symbols: string | Array<string>, options?: IndexSnapshotOhlcOptions | undefined | null): Promise<Array<OhlcTick>>
  /**
   * Get the latest price snapshot for one or more indices.
   *
   * - Retrieves a real-time last index price.
   * - Exchanges typically generate a price report every second for popular indices like SPX.
   */
  indexSnapshotPrice(symbols: string | Array<string>, options?: IndexSnapshotPriceOptions | undefined | null): Promise<Array<PriceTick>>
  /**
   * Get the latest market value snapshot for one or more indices.
   *
   * - Retrieves a real-time last index market value.
   * - Exchanges typically generate a price report every second for popular indices like SPX.
   */
  indexSnapshotMarketValue(symbols: string | Array<string>, options?: IndexSnapshotMarketValueOptions | undefined | null): Promise<Array<MarketValueTick>>
  /**
   * Fetch end-of-day index data for a date range.
   *
   * - Since the indices feeds do not provide a national EOD report, Theta Data generates a national EOD report at 17:15 each day.
   */
  indexHistoryEOD(symbol: string, startDate: string | Date, endDate: string | Date, options?: IndexHistoryEodOptions | undefined | null): Promise<Array<EodTick>>
  /** Stream `index_history_eod` rows into `callback` without materialising the full response in memory. `callback(chunk: EodTick[]) => void` is invoked once per server chunk; the chunk is freed before the next is fetched, so peak memory tracks a single chunk rather than the whole result. This is the memory-bounded companion to the `indexHistoryEOD` method — prefer it for multi-day or full-universe pulls. The returned Promise resolves when the stream drains and rejects (typed like the buffered method) on a wire or decode error. Cancelling the Promise drops the in-flight request. `options` carries the same optional builder parameters and `timeoutMs` as the buffered method; the `callback` is the trailing argument. */
  indexHistoryEODStream(symbol: string, startDate: string | Date, endDate: string | Date, options: IndexHistoryEodOptions | undefined | null, callback: ((arg: Array<EodTick>) => void)): Promise<void>
  /**
   * Fetch intraday OHLC bars for an index.
   *
   * - Aggregated OHLC bars that use SIP rules for each bar.
   * - Time timestamp of the bar represents the opening time of the bar. For a trade to be part of the bar:  ``bar timestamp`` <= ``trade time`` < ``bar timestamp + interval``.
   * - Exchanges typically generate a price report every second for popular indices like SPX.
   *
   * Defaults (upstream):
   * - `interval`: `"1s"`
   * - `start_time`: `"09:30:00"`
   * - `end_time`: `"16:00:00"`
   */
  indexHistoryOHLC(symbol: string, startDate: string | Date, endDate: string | Date, options?: IndexHistoryOhlcOptions | undefined | null): Promise<Array<OhlcTick>>
  /** Stream `index_history_ohlc` rows into `callback` without materialising the full response in memory. `callback(chunk: OhlcTick[]) => void` is invoked once per server chunk; the chunk is freed before the next is fetched, so peak memory tracks a single chunk rather than the whole result. This is the memory-bounded companion to the `indexHistoryOHLC` method — prefer it for multi-day or full-universe pulls. The returned Promise resolves when the stream drains and rejects (typed like the buffered method) on a wire or decode error. Cancelling the Promise drops the in-flight request. `options` carries the same optional builder parameters and `timeoutMs` as the buffered method; the `callback` is the trailing argument. */
  indexHistoryOHLCStream(symbol: string, startDate: string | Date, endDate: string | Date, options: IndexHistoryOhlcOptions | undefined | null, callback: ((arg: Array<OhlcTick>) => void)): Promise<void>
  /**
   * Fetch intraday price history for an index.
   *
   * - Retrieves historical indices price reports. Exchanges typically generate a price report every second for popular indices like SPX.
   * - When the ``interval`` parameter is specified, the returned data represents the price at the exact time of each timestamp. If the timestamp in the response is 10:30:00, the price field represents the price at that exact time of the day.
   * - A price update from the exchange is omitted if the price remained the same from the previous update.
   * - Multi-day requests are limited to 1 month of data.
   *
   * Defaults (upstream):
   * - `interval`: `"1s"`
   * - `start_time`: `"09:30:00"`
   * - `end_time`: `"16:00:00"`
   */
  indexHistoryPrice(symbol: string, date: string | Date, options?: IndexHistoryPriceOptions | undefined | null): Promise<Array<PriceTick>>
  /** Stream `index_history_price` rows into `callback` without materialising the full response in memory. `callback(chunk: PriceTick[]) => void` is invoked once per server chunk; the chunk is freed before the next is fetched, so peak memory tracks a single chunk rather than the whole result. This is the memory-bounded companion to the `indexHistoryPrice` method — prefer it for multi-day or full-universe pulls. The returned Promise resolves when the stream drains and rejects (typed like the buffered method) on a wire or decode error. Cancelling the Promise drops the in-flight request. `options` carries the same optional builder parameters and `timeoutMs` as the buffered method; the `callback` is the trailing argument. */
  indexHistoryPriceStream(symbol: string, date: string | Date, options: IndexHistoryPriceOptions | undefined | null, callback: ((arg: Array<PriceTick>) => void)): Promise<void>
  /**
   * Fetch the index price at a specific time of day across a date range.
   *
   * - Retrieves historical indices price reports. Exchanges typically generate a price report every second for popular indices like SPX.
   * - The ``time_of_day`` parameter represents the 00:00:00.000 ET that the price should be provided for.
   */
  indexAtTimePrice(symbol: string, startDate: string | Date, endDate: string | Date, timeOfDay: string | Date, options?: IndexAtTimePriceOptions | undefined | null): Promise<Array<IndexPriceAtTimeTick>>
  /** Stream `index_at_time_price` rows into `callback` without materialising the full response in memory. `callback(chunk: IndexPriceAtTimeTick[]) => void` is invoked once per server chunk; the chunk is freed before the next is fetched, so peak memory tracks a single chunk rather than the whole result. This is the memory-bounded companion to the `indexAtTimePrice` method — prefer it for multi-day or full-universe pulls. The returned Promise resolves when the stream drains and rejects (typed like the buffered method) on a wire or decode error. Cancelling the Promise drops the in-flight request. `options` carries the same optional builder parameters and `timeoutMs` as the buffered method; the `callback` is the trailing argument. */
  indexAtTimePriceStream(symbol: string, startDate: string | Date, endDate: string | Date, timeOfDay: string | Date, options: IndexAtTimePriceOptions | undefined | null, callback: ((arg: Array<IndexPriceAtTimeTick>) => void)): Promise<void>
  /**
   * Check whether the market is open today.
   *
   * - Retrieves current day equity market schedule
   * - *On days when the market closes early at 1:00 PM ET; eligible options will trade until 1:15 PM.
   * - **Some NYSE exchanges will continue late trading until 5:00 PM ET on early close days.
   */
  calendarOpenToday(options?: CalendarOpenTodayOptions | undefined | null): Promise<Array<CalendarDay>>
  /**
   * Get calendar information for a specific date.
   *
   * - Retrieves equity market schedule for a given date
   * - Note: Holiday data is available 01/01/2012 through the end of the calendar year that immediately follows the current year
   * - *On days when the market closes early at 1:00 PM ET; eligible options will trade until 1:15 PM.
   * - **Some NYSE exchanges will continue late trading until 5:00 PM ET on early close days.
   */
  calendarOnDate(date: string | Date, options?: CalendarOnDateOptions | undefined | null): Promise<Array<CalendarDay>>
  /**
   * Get equity market holidays and early-close days for a year (vendor `year_holidays` endpoint — only non-standard days, not every trading day).
   *
   * - Retrieves equity market holidays for a given year
   * - Note: Holiday data is available 01/01/2012 through the end of the calendar year that immediately follows the current year
   * - *On days when the market closes early at 1:00 PM ET; eligible options will trade until 1:15 PM.
   * - **Some NYSE exchanges will continue late trading until 5:00 PM ET on early close days.
   */
  calendarYear(year: string, options?: CalendarYearOptions | undefined | null): Promise<Array<CalendarDay>>
  /**
   * Fetch end-of-day interest rate history.
   *
   * - Returns the interest rate reported. Depending on the rate, reports can occur in the morning or the afternoon.
   * - Valid `symbol` values per upstream `RateType` enum:
   *   `SOFR`, `TREASURY_M1`, `TREASURY_M3`, `TREASURY_M6`,
   *   `TREASURY_Y1`, `TREASURY_Y2`, `TREASURY_Y3`, `TREASURY_Y5`,
   *   `TREASURY_Y7`, `TREASURY_Y10`, `TREASURY_Y20`, `TREASURY_Y30`.
   */
  interestRateHistoryEOD(symbol: string, startDate: string | Date, endDate: string | Date, options?: InterestRateHistoryEodOptions | undefined | null): Promise<Array<InterestRateTick>>
  /** Stream `interest_rate_history_eod` rows into `callback` without materialising the full response in memory. `callback(chunk: InterestRateTick[]) => void` is invoked once per server chunk; the chunk is freed before the next is fetched, so peak memory tracks a single chunk rather than the whole result. This is the memory-bounded companion to the `interestRateHistoryEOD` method — prefer it for multi-day or full-universe pulls. The returned Promise resolves when the stream drains and rejects (typed like the buffered method) on a wire or decode error. Cancelling the Promise drops the in-flight request. `options` carries the same optional builder parameters and `timeoutMs` as the buffered method; the `callback` is the trailing argument. */
  interestRateHistoryEODStream(symbol: string, startDate: string | Date, endDate: string | Date, options: InterestRateHistoryEodOptions | undefined | null, callback: ((arg: Array<InterestRateTick>) => void)): Promise<void>
  /**
   * Fetch intraday OHLC bars across a date range (start_date..end_date). This is a dedicated upstream route, distinct from the single-date stock_history_ohlc; the `_range` suffix mirrors the vendor's separate `ohlc_range` route.
   *
   * Defaults (upstream):
   * - `interval`: `"1s"`
   * - `start_time`: `"09:30:00"`
   * - `end_time`: `"16:00:00"`
   * - `venue`: `"nqb"`
   */
  stockHistoryOHLCRange(symbol: string, startDate: string | Date, endDate: string | Date, options?: StockHistoryOhlcRangeOptions | undefined | null): Promise<Array<OhlcTick>>
  /** Stream `stock_history_ohlc_range` rows into `callback` without materialising the full response in memory. `callback(chunk: OhlcTick[]) => void` is invoked once per server chunk; the chunk is freed before the next is fetched, so peak memory tracks a single chunk rather than the whole result. This is the memory-bounded companion to the `stockHistoryOHLCRange` method — prefer it for multi-day or full-universe pulls. The returned Promise resolves when the stream drains and rejects (typed like the buffered method) on a wire or decode error. Cancelling the Promise drops the in-flight request. `options` carries the same optional builder parameters and `timeoutMs` as the buffered method; the `callback` is the trailing argument. */
  stockHistoryOHLCRangeStream(symbol: string, startDate: string | Date, endDate: string | Date, options: StockHistoryOhlcRangeOptions | undefined | null, callback: ((arg: Array<OhlcTick>) => void)): Promise<void>
}

/**
 * JS-visible `SecType` (frozen security-type enum). Construction
 * happens via the four named factories: `SecType.stock()`,
 * `SecType.option()`, `SecType.index()`, `SecType.rate()`. Returns
 * flow into `secType.fullTrades()` /
 * `secType.fullOpenInterest()` to build a full-stream
 * `Subscription`.
 */
export declare class SecType {
  /** `SecType.stock()` — equity-side full-stream constructor. */
  static stock(): SecType
  /** `SecType.option()` — option-side full-stream constructor. */
  static option(): SecType
  /** `SecType.index()` — index-side full-stream constructor. */
  static index(): SecType
  /** `SecType.rate()` — rate-side full-stream constructor. */
  static rate(): SecType
  /** Full-stream Trade subscription for this security type. */
  fullTrades(): Subscription
  /** Full-stream OpenInterest subscription for this security type. */
  fullOpenInterest(): Subscription
  /** Symbolic name (`"STOCK"`, `"OPTION"`, `"INDEX"`, `"RATE"`). */
  get name(): string
  /**
   * String rendering for `console.log` / template literals. Returns
   * the symbolic name (`"OPTION"`), matching the Python `SecType`
   * `__str__`. Without it a `SecType` instance prints as an opaque
   * `SecType {}` because its getters do not surface on inspection.
   */
  toString(): string
}

/**
 * Standalone FPSS-only streaming client.
 *
 * Opens ONLY the FPSS TLS transport — no historical data channel, no
 * Nexus HTTP authentication. Use when a parallel historical process is
 * already running in the same environment and you need to stream FPSS
 * without the bundled `Client` taking over the Nexus session
 * at connect time.
 *
 * ```js
 * const { StreamingClient, Config, Contract } = require("@thetadatadx/sdk");
 * const fpss = StreamingClient.connectFromFile("creds.txt");
 * fpss.startStreaming((event) => console.log(event.kind, event));
 * fpss.subscribe(Contract.stock("AAPL").quote());
 * // ... events arrive on the Node main thread ...
 * fpss.stopStreaming();
 * ```
 */
export declare class StreamingClient {
  /**
   * Allocate a standalone FPSS handle with a `Credentials` handle.
   * Streaming only — opens no historical data channel and issues no
   * Nexus request. Pass an optional `Config` (`dev` / `stage` /
   * `production`, plus any tuned FPSS / reconnect setters) to override the
   * production-default endpoint. The FPSS TLS connection opens on the
   * first `startStreaming` call.
   *
   * The config is snapshot at construction time: the `Config` handle
   * may be reused or mutated afterward without affecting this client.
   */
  static connect(creds: Credentials, config?: Config | undefined | null): StreamingClient
  /**
   * Allocate a standalone FPSS handle with a credentials file (line 1 =
   * email, line 2 = password). Convenience wrapper over
   * `Credentials.fromFile` + `connect`. Pass an optional `Config` to
   * override the production-default endpoint.
   */
  static connectFromFile(path: string, config?: Config | undefined | null): StreamingClient
  /**
   * Start FPSS streaming and register a JS callback for incoming events.
   *
   * Opens the FPSS connection and begins delivering events. Each typed
   * FPSS event is delivered to your `callback(event)` on the Node main
   * thread, so the callback may use any JS API safely. A callback that
   * panics or throws is isolated and does not interrupt the stream.
   *
   * Backpressure: a slow callback causes incoming events to queue and,
   * once the buffer is full, newly arriving events are dropped, observable
   * via `droppedEventCount()`. The receive path is never blocked by a
   * slow callback, so the upstream connection stays healthy regardless
   * of callback speed.
   */
  startStreaming(callback: TsfnCallback): void
  /**
   * Whether the FPSS TLS connection is currently open. Returns `false`
   * when the dispatcher thread has panicked — no events are arriving
   * even though the TLS slot is still populated.
   */
  isStreaming(): boolean
  /**
   * Whether the FPSS session is currently authenticated. Distinct from
   * `isStreaming()`: the TLS slot can hold a client whose authenticated
   * flag has flipped to `false` after a server disconnect, before the
   * application has issued `reconnect()`. A panicked dispatcher also
   * folds back to `false` here.
   */
  isAuthenticated(): boolean
  /**
   * Polymorphic subscribe — primary fluent entry point. Accepts the
   * `Subscription` value returned by `Contract.quote()` /
   * `Contract.trade()` / `Contract.openInterest()` (per-contract scope)
   * or by `SecType.option().fullTrades()` /
   * `SecType.option().fullOpenInterest()` (full-stream scope).
   */
  subscribe(sub: Subscription): void
  /**
   * Bulk-subscribe an array of `Subscription` values. Stops at the first
   * error and returns it; previously-installed subscriptions are NOT
   * rolled back.
   */
  subscribeMany(subs: Array<Subscription>): void
  /** Polymorphic unsubscribe — fluent counterpart to `subscribe(sub)`. */
  unsubscribe(sub: Subscription): void
  /** Bulk-unsubscribe an array of `Subscription` values. */
  unsubscribeMany(subs: Array<Subscription>): void
  /**
   * Snapshot of per-contract subscriptions on the live session as an
   * array of `{ kind, contract }` objects (matching the unified
   * client's `activeSubscriptions()` projection). Empty array when
   * streaming has not started.
   */
  activeSubscriptions(): any
  /**
   * Snapshot of full-stream subscriptions (e.g. `OPTION` /
   * `full_trades`). Each entry has the same `{ kind, contract }` shape
   * as the unified client's `activeFullSubscriptions()`, where `kind` is
   * `"full_trades"` / `"full_open_interest"` and `contract` carries the
   * wire-level security type. Quote is never a valid full-stream kind,
   * so any such row is dropped. Empty array when streaming has not
   * started.
   */
  activeFullSubscriptions(): any
  /**
   * Cumulative count of FPSS events the TLS reader could not publish into
   * the event ring because the consumer fell behind. Snapshot the value
   * BEFORE `reconnect()` if you need to accumulate drops across session
   * boundaries — `reconnect` rebuilds the inner client and the counter
   * resets. Returned as `bigint` for the full 64-bit unsigned range.
   */
  droppedEventCount(): bigint
  /**
   * Point-in-time count of events published into the ring but not yet
   * drained into your callback — the in-flight depth between the I/O
   * thread and the dispatcher. The leading back-pressure signal: rises
   * before `droppedEventCount()` moves. Returns `0n` when no session is
   * live.
   */
  ringOccupancy(): bigint
  /**
   * Configured capacity of the event ring in slots (a power of two) —
   * the fixed denominator for `ringOccupancy()`. Returns `0n` when no
   * session is live.
   */
  ringCapacity(): bigint
  /**
   * Cumulative count of user-callback panics caught at the per-event
   * isolation boundary. A panic is caught, recorded here, and does not
   * stop event delivery. Returned as `bigint` for the full 64-bit unsigned range.
   */
  panicCount(): bigint
  /**
   * Set the slow-callback wall-clock threshold in microseconds. When a
   * callback invocation runs longer than `thresholdUs`,
   * `slowCallbackCount()` increments and a rate-limited warning is
   * logged. Pass `0n` to disable the watchdog (the default).
   * Observability only: the watchdog never cancels the callback. No-op
   * when no session is live. Accepts `bigint` for the full 64-bit
   * unsigned range.
   */
  setSlowCallbackThresholdUs(thresholdUs: bigint): void
  /**
   * Cumulative count of user-callback invocations whose wall-clock
   * duration exceeded the threshold set by `setSlowCallbackThresholdUs()`.
   * Returns `0n` when the watchdog is disabled or no session is live.
   * Returned as `bigint` for the full 64-bit unsigned range.
   */
  slowCallbackCount(): bigint
  /**
   * Milliseconds since the most recent inbound streaming frame of any
   * kind (data tick, heartbeat, control), or `null` when no session is
   * live or no frame has been received yet. The operator-facing
   * staleness clock.
   */
  millisSinceLastEvent(): bigint | null
  /**
   * UNIX-nanosecond receive timestamp of the most recent inbound
   * streaming frame of any kind. Returns `0n` when no session is live or
   * no frame has been received yet.
   */
  lastEventReceivedAtUnixNanos(): bigint
  /**
   * Address (`host:port`) of the streaming server the current session is
   * connected to, following the session across auto-reconnects. `null`
   * when no session is live.
   */
  lastConnectedAddr(): string | null
  /**
   * Stop streaming and clear the registered callback. Same
   * explicit-handoff semantics as the unified client: to resume after
   * this returns, call `startStreaming(callback)` again with a freshly
   * bound function; `reconnect()` throws because no callback is held.
   *
   * Lock ordering: `callback` BEFORE `inner`, matching `startStreaming`.
   */
  stopStreaming(): void
  /**
   * Alias for `stopStreaming`. Mirrors the unified client's split surface
   * where `shutdown` is documented as the terminal stop — on the
   * standalone client both names are equivalent.
   */
  shutdown(): void
  /**
   * Re-open the FPSS connection and re-register the previously installed
   * callback. Requires a prior `startStreaming(callback)`; throws
   * otherwise.
   *
   * Saves the active per-contract and full-stream subscriptions against
   * the old session, opens a fresh FPSS connection under the previously
   * installed callback, and re-applies the saved subscriptions through
   * the core's paced replay engine. Per-subscription failures surface as
   * a single error naming every contract that did not re-subscribe — the
   * streaming session itself is already up at that point.
   */
  reconnect(): void
  /**
   * Block until every superseded streaming session's event-ring consumer
   * has finished firing the registered callback. Resolves `true` once
   * all retired generations have drained, `false` on timeout. Polls at
   * 1 ms cadence on a worker so the Node event loop stays free.
   */
  awaitDrain(timeoutMs: number): Promise<boolean>
}

/**
 * User-facing real-time-streaming sub-namespace returned by the
 * `client.stream` getter.
 *
 * Shares the parent client's `Arc<thetadatadx::Client>` and its
 * `Arc<Mutex<Option<Arc<TsfnCallback>>>>` callback slot, so
 * `startStreaming`, `stopStreaming`, `reconnect`, and the subscription
 * methods observe the same registration the unified client does.
 */
export declare class StreamView {
  /**
   * Cumulative count of FPSS events that were dropped because the
   * callback fell behind and the in-flight buffer was full.
   *
   * The value matches every other binding (C ABI, Python, C++). The
   * counter resets when the session is recreated -- that happens on
   * `stopStreaming()` and `reconnect()`. Snapshot the value before
   * reconnect if you need to accumulate drops across session
   * boundaries.
   *
   * Returned as `bigint` so it can represent the full 64-bit unsigned range
   * (Number would top out at 2^53).
   */
  droppedEventCount(): bigint
  /**
   * Point-in-time count of streaming events published into the
   * event ring but not yet drained into your callback — the
   * in-flight depth between the I/O thread and the dispatcher.
   *
   * The leading back-pressure signal: `droppedEventCount()` only
   * moves AFTER data has been lost, while a rising occupancy that
   * approaches `ringCapacity()` predicts those drops while there
   * is still time to react. Sampling never blocks the feed; poll
   * it from your own code at any cadence.
   *
   * The value matches every other binding (C ABI, Python, C++).
   * Returns `0n` before `startStreaming` and after `stopStreaming`.
   * Returned as `bigint` for shape-consistency with the other
   * streaming counters.
   */
  ringOccupancy(): bigint
  /**
   * Configured capacity of the streaming event ring in slots (the
   * `streamingRingSize` setting, a power of two).
   *
   * The fixed denominator for `ringOccupancy()`: when the
   * occupancy sample approaches this value the ring is saturating
   * and further events will be dropped (counted by
   * `droppedEventCount()`). Returns `0n` before `startStreaming`
   * and after `stopStreaming`. Returned as `bigint` for
   * shape-consistency with the other streaming counters.
   */
  ringCapacity(): bigint
  /**
   * Milliseconds since the most recent inbound streaming frame of
   * any kind (data tick, heartbeat, control), or `null` when
   * streaming has not started or no frame has been received yet.
   *
   * The operator-facing staleness clock: a healthy session stays in
   * the low hundreds of milliseconds (the upstream heartbeats even
   * when no market data flows), so a steadily growing value is the
   * earliest external signal of a dead or wedged connection.
   */
  millisSinceLastEvent(): bigint | null
  /**
   * UNIX-nanosecond receive timestamp of the most recent inbound
   * streaming frame of any kind. Returns `0n` when streaming has
   * not started or no frame has been received yet. Raw feed for
   * `millisSinceLastEvent`, exposed for callers correlating against
   * their own pipeline timestamps.
   */
  lastEventReceivedAtUnixNanos(): bigint
  /**
   * Address (`host:port`) of the streaming server the current
   * session is connected to, following the session across
   * auto-reconnects. `null` when streaming has not started.
   */
  lastConnectedAddr(): string | null
  /**
   * Cumulative count of user-callback panics caught at the per-event
   * isolation boundary since the current stream started.
   *
   * A panic in the callback is caught, recorded here, and does not
   * stop event delivery — the next event continues normally. The
   * value matches every other binding (C ABI, Python, C++).
   *
   * Returned as `bigint` so it can represent the full 64-bit unsigned range
   * (Number would top out at 2^53).
   */
  panicCount(): bigint
  /**
   * Set the slow-callback wall-clock threshold in microseconds. When a
   * callback invocation runs longer than `thresholdUs`,
   * `slowCallbackCount()` increments and a rate-limited warning is
   * logged. Pass `0n` to disable the watchdog (the default).
   * Observability only: the watchdog never cancels the callback. No-op
   * before `startStreaming`. Accepts `bigint` for the full 64-bit
   * unsigned range.
   */
  setSlowCallbackThresholdUs(thresholdUs: bigint): void
  /**
   * Cumulative count of user-callback invocations whose wall-clock
   * duration exceeded the threshold set by `setSlowCallbackThresholdUs()`.
   * Returns `0n` when the watchdog is disabled or before `startStreaming`.
   * The value matches every other binding (C ABI, Python, C++). Returned
   * as `bigint` for the full 64-bit unsigned range.
   */
  slowCallbackCount(): bigint
  /**
   * Snapshot of full-stream subscriptions (e.g. `OPTION` /
   * `full_trades`, `OPTION` / `full_open_interest`).
   *
   * Each entry has the same `{ kind, contract }` shape returned by
   * `activeSubscriptions()`, where `kind` is one of
   * `"full_trades"` / `"full_open_interest"` and `contract` carries
   * the wire-level security type (`"OPTION"`, `"STOCK"`, ...).
   * Quote is never a valid full-stream kind on the FPSS wire, so
   * any such row from the core is dropped from the projection.
   * Empty array when streaming has not started.
   */
  activeFullSubscriptions(): any
  /**
   * Start FPSS streaming and register a JS callback for incoming events.
   *
   * Each typed FPSS event is delivered to your
   * `callback(event)` on the Node main thread, so the
   * callback may use any JS API safely. A callback that
   * panics or throws is isolated and does not interrupt
   * the stream.
   *
   * Backpressure: a slow callback causes incoming events
   * to queue and, once the buffer is full, newly arriving
   * events are dropped. The dropped count is observable
   * via `droppedEventCount()`. The receive path is never
   * blocked by a slow callback, so the upstream connection
   * stays healthy regardless of callback speed.
   */
  startStreaming(callback: ((arg: StreamEvent) => void)): void
  /** Whether the streaming connection is active. */
  isStreaming(): boolean
  /** Get a snapshot of currently active subscriptions. */
  activeSubscriptions(): any
  /**
   * Reconnect FPSS streaming and re-register the previously installed callback.
   *
   * Requires a prior `startStreaming(callback)`; throws if
   * no callback is registered. All active subscriptions are
   * restored on the new connection. If some subscriptions
   * cannot be restored, the reconnect still completes for
   * the rest and the failures are reported through the
   * callback.
   *
   * # Callback lifetime across `stopStreaming`
   *
   * `stopStreaming()` and `shutdown()` clear the registered
   * callback. To resume streaming on this client after
   * `stopStreaming()`, you MUST call `startStreaming(callback)`
   * again with a freshly bound function; `reconnect()` throws
   * because no callback is held.
   *
   * This explicit-handoff model matches the C++ wrapper's RAII
   * destructor and the Python `with` block's `__exit__`: the
   * resource (the JS callback handle) is cleared at the same
   * scope boundary the application observes. The unified C API
   * preserves the callback across stop/reconnect, but the
   * TypeScript and Python bindings deliberately diverge to enforce
   * the explicit handoff and avoid retaining captured references
   * past a teardown the caller has already observed.
   */
  reconnect(): void
  /**
   * Stop streaming while keeping the historical client usable.
   *
   * Clears the registered callback. To resume streaming, start streaming again with a freshly bound callback -- reconnect will fail because no callback is held. See the reconnect docs for the rationale: the callback is released at the same scope boundary the application observes, so a stopped session never retains a captured reference past a teardown the caller has already seen.
   */
  stopStreaming(): void
  /**
   * Shut down the FPSS streaming connection.
   *
   * On the Python and TypeScript bindings, this clears the registered callback (same explicit-handoff semantics as stopping the stream); reconnect will then fail until the caller starts streaming again with a freshly bound callback. The C++ binding preserves the underlying connection's behaviour.
   */
  shutdown(): void
  /** Block until the previous streaming session's consumer thread has finished firing the registered callback. Returns true if the drain completed within the timeout, false otherwise. */
  awaitDrain(timeoutMs: number): Promise<boolean>
  /**
   * Polymorphic subscribe — primary fluent entry point. Accepts the
   * `Subscription` value returned by `Contract.quote()` /
   * `Contract.trade()` / `Contract.openInterest()` (per-contract
   * scope) or by `SecType.option().fullTrades()` /
   * `SecType.option().fullOpenInterest()` (full-stream scope).
   */
  subscribe(sub: Subscription): void
  /**
   * Bulk-subscribe an array of `Subscription` values. Stops at the
   * first error and returns it; previously-installed subscriptions
   * are NOT rolled back.
   */
  subscribeMany(subs: Array<Subscription>): void
  /** Polymorphic unsubscribe — fluent counterpart to `subscribe(sub)`. */
  unsubscribe(sub: Subscription): void
  /** Bulk-unsubscribe an array of `Subscription` values. */
  unsubscribeMany(subs: Array<Subscription>): void
}

/**
 * Typed market-data subscription.
 *
 * Returned by `Contract.quote()` / `.trade()` / `.openInterest()`
 * (per-contract scope) and by `SecType.option().fullTrades()` /
 * `.fullOpenInterest()` (full-stream scope). Pass to
 * `client.subscribe(sub)` or `client.subscribeMany([...])`.
 */
export declare class Subscription {
  /**
   * One of `"quote"`, `"trade"`, `"open_interest"`,
   * `"market_value"`, `"full_trades"`, `"full_open_interest"` — the
   * wire-level kind.
   */
  get kind(): string
  /** `true` for full-stream (security-type-scoped) subscriptions. */
  get isFull(): boolean
  /**
   * The bound contract for per-contract subscriptions, `null` for
   * full-stream subscriptions.
   */
  get contract(): ContractRef | null
  /**
   * The security type for full-stream subscriptions, `null` for
   * per-contract subscriptions.
   */
  get secType(): SecType | null
  /**
   * String rendering for `console.log` / template literals, e.g.
   * `"Subscription(Trade, SPY OPTION 20260620 C 550)"` or
   * `"Subscription(full Trades, OPTION)"`. Mirrors the Python
   * `Subscription` `__repr__`. Without it a `Subscription` prints as
   * an opaque `Subscription {}` because its getters do not surface on
   * inspection.
   */
  toString(): string
}

/**
 * Cross-language lookup-table namespace. Exposes the static condition,
 * exchange, calendar, timestamp, and sequence helpers as `Util.*` static
 * methods so the JS surface mirrors the Python / C++ / C ABI utility sets.
 */
export declare class Util {
  /**
   * Symbolic name for a trade `condition` code (e.g. `0` -> `"REGULAR"`).
   * Returns `"UNKNOWN"` for codes outside the table.
   */
  static conditionName(code: number): string
  /** Human-readable description for a trade `condition` code. */
  static conditionDescription(code: number): string
  /** Whether a trade `condition` code marks a trade cancellation. */
  static isCancel(code: number): boolean
  /**
   * Whether a trade with this `condition` code contributes to the
   * running session volume.
   */
  static updatesVolume(code: number): boolean
  /** Symbolic name for a quote `condition` code. */
  static quoteConditionName(code: number): string
  /** Human-readable description for a quote `condition` code. */
  static quoteConditionDescription(code: number): string
  /** Whether a quote `condition` code marks a firm (binding) quote. */
  static isFirm(code: number): boolean
  /** Whether a quote `condition` code marks a trading halt. */
  static isHalted(code: number): boolean
  /**
   * Symbolic name for an `exchange` code (e.g. `3` ->
   * `"NewYorkStockExchange"`).
   */
  static exchangeName(code: number): string
  /**
   * Short ticker-tape symbol for an `exchange` code (e.g. `3` ->
   * `"NYSE"`).
   */
  static exchangeSymbol(code: number): string
  /**
   * Vendor vocabulary text for a calendar-day `status` code (`0` ->
   * `"open"`, `1` -> `"early_close"`, `2` -> `"full_close"`, `3` ->
   * `"weekend"`). Returns the literal `"UNKNOWN"` for codes outside
   * the table.
   */
  static calendarStatusName(code: number): string
  /**
   * Combine an Eastern-Time `YYYYMMDD` date and milliseconds-of-day
   * into Unix epoch milliseconds (UTC, DST-aware) as a JS BigInt.
   * Usable with any `(date, *_ms_of_day)` pair on the tick structs.
   * Returns `null` when `date` is absent (`0`) or either input is out
   * of domain. BigInt matches the `*TimestampMs` tick accessors so the
   * epoch domain is uniform.
   */
  static timestampMs(date: number, msOfDay: number): bigint | null
  /**
   * Convert a signed wire-encoded trade-sequence value to its unsigned
   * monotonic form. Accepts a JS BigInt in the **32-bit signed wire
   * range** (`-2_147_483_648 ..= 2_147_483_647`) — the upstream feed
   * encodes trade sequences as a 32-bit signed integer. Returns a JS
   * BigInt because the unsigned monotonic sequence id can exceed
   * `Number.MAX_SAFE_INTEGER`. Inputs outside the wire range throw so
   * silent coercion cannot produce a look-correct-but-wrong sequence id
   * downstream.
   */
  static sequenceSignedToUnsigned(signedValue: bigint): bigint
  /**
   * Convert an unsigned monotonic trade-sequence value back to its
   * signed wire encoding. Accepts a JS BigInt in the unsigned wire
   * range (`0 ..= 2^32 - 1`); returns a JS BigInt for symmetry with
   * `sequenceSignedToUnsigned`. Negative inputs and inputs above the
   * wire range throw — the unsigned monotonic sequence id is always
   * non-negative and never wider than the 32-bit wire range.
   */
  static sequenceUnsignedToSigned(unsignedValue: bigint): bigint
}

/** Compute all 23 Black-Scholes Greeks + IV in one call. */
export declare function allGreeks(spot: number, strike: number, rate: number, divYield: number, tte: number, optionPrice: number, right: string): AllGreeks

/**
 * All 23 Black-Scholes Greeks + IV in a single typed object.
 * Returned by `allGreeks(...)`.
 */
export interface AllGreeks {
  value: number
  iv: number
  ivError: number
  delta: number
  gamma: number
  theta: number
  vega: number
  rho: number
  vanna: number
  charm: number
  vomma: number
  veta: number
  vera: number
  speed: number
  zomma: number
  color: number
  ultima: number
  d1: number
  d2: number
  dualDelta: number
  dualGamma: number
  epsilon: number
  lambda: number
}

/** Calendar day. Market open/close schedule. */
export interface CalendarDay {
  date: number
  isOpen: boolean
  openTime: number
  closeTime: number
  status: string
}

/**
 * Serialise a `CalendarDay` history result to an Arrow IPC stream
 * (the `apache-arrow` wire form). Mirrors the FlatFiles
 * `FlatFileRowList.toArrowIpc()` exit and the Python
 * `<TickName>List.to_arrow()` terminal so every binding can reach a
 * dataframe from an in-band history result.
 */
export declare function calendarDayToArrowIpc(rows: Array<CalendarDay>): Buffer

/**
 * Optional parameters for the `calendarOnDate` method. Keys are
 * the camelCase parameter names; absent keys behave exactly like an
 * omitted parameter. `timeoutMs` bounds the whole call: on expiry the
 * returned Promise rejects and the underlying request is cancelled.
 */
export interface CalendarOnDateOptions {
  /**
   * Per-call deadline as a non-negative whole number of milliseconds;
   * on expiry the returned Promise rejects and the underlying request
   * is cancelled. A non-finite, negative, or fractional value is
   * rejected with `InvalidParameterError` rather than coerced.
   */
  timeoutMs?: number
}

/**
 * Optional parameters for the `calendarOpenToday` method. Keys are
 * the camelCase parameter names; absent keys behave exactly like an
 * omitted parameter. `timeoutMs` bounds the whole call: on expiry the
 * returned Promise rejects and the underlying request is cancelled.
 */
export interface CalendarOpenTodayOptions {
  /**
   * Per-call deadline as a non-negative whole number of milliseconds;
   * on expiry the returned Promise rejects and the underlying request
   * is cancelled. A non-finite, negative, or fractional value is
   * rejected with `InvalidParameterError` rather than coerced.
   */
  timeoutMs?: number
}

/**
 * Optional parameters for the `calendarYear` method. Keys are
 * the camelCase parameter names; absent keys behave exactly like an
 * omitted parameter. `timeoutMs` bounds the whole call: on expiry the
 * returned Promise rejects and the underlying request is cancelled.
 */
export interface CalendarYearOptions {
  /**
   * Per-call deadline as a non-negative whole number of milliseconds;
   * on expiry the returned Promise rejects and the underlying request
   * is cancelled. A non-finite, negative, or fractional value is
   * rejected with `InvalidParameterError` rather than coerced.
   */
  timeoutMs?: number
}

/** FPSS server connection ack (wire code 4). Carries no payload. */
export interface Connected {

}

/**
 * FPSS contract identifier. Surfaced on every decoded FPSS data
 * event as `event.quote.contract` / `event.trade.contract` / etc.
 * `secType` is the symbolic uppercase name (`"STOCK"` / `"OPTION"` /
 * `"INDEX"` / `"RATE"`); `right` is `"C"` / `"P"` / `null`;
 * `strike` is the option strike in dollars — the same unit historical
 * rows carry under the same name, so streaming contracts join against
 * historical data directly. `expiration` is a `YYYYMMDD` integer.
 */
export interface Contract {
  symbol: string
  secType: string
  expiration?: number
  right?: string
  strike?: number
}

/** FPSS server assigned a contract id. The `contract` payload carries the full resolved contract (root, sec_type, expiration / strike / right for options). */
export interface ContractAssigned {
  id: number
  contract: Contract
}

/** FPSS server disconnected the client (wire code 12). `reason` is the integer disconnect code; read the resolved reason-name field for the symbolic name. */
export interface Disconnected {
  reason: number
  /**
   * Resolved disconnect-reason name (e.g. `"TooManyRequests"`,
   * `"InvalidCredentials"`, `"Unspecified"` for unknown codes).
   * Derived from the wire-level `reason` integer.
   */
  reasonName: string
}

/** End-of-day tick. Full EOD snapshot with OHLC + quote. */
export interface EodTick {
  createdMsOfDay: number
  lastTradeMsOfDay: number
  open: number
  high: number
  low: number
  close: number
  volume: bigint
  count: bigint
  bidSize: number
  bidExchange: number
  bid: number
  bidCondition: number
  askSize: number
  askExchange: number
  ask: number
  askCondition: number
  date: number
  expiration?: number
  strike?: number
  right?: string
  /**
   * Unix epoch milliseconds (UTC, DST-aware) combining `date` with
   * `created_ms_of_day` (Eastern-Time milliseconds-of-day). `undefined` when
   * `date` is absent (`0`).
   */
  createdTimestampMs?: bigint
  /**
   * Unix epoch milliseconds (UTC, DST-aware) combining `date` with
   * `last_trade_ms_of_day` (Eastern-Time milliseconds-of-day). `undefined` when
   * `date` is absent (`0`).
   */
  lastTradeTimestampMs?: bigint
}

/**
 * Serialise a `EodTick` history result to an Arrow IPC stream
 * (the `apache-arrow` wire form). Mirrors the FlatFiles
 * `FlatFileRowList.toArrowIpc()` exit and the Python
 * `<TickName>List.to_arrow()` terminal so every binding can reach a
 * dataframe from an in-band history result.
 */
export declare function eodTickToArrowIpc(rows: Array<EodTick>): Buffer

/** Full union Greeks tick -- every Greek the v3 server publishes on the */
export interface GreeksAllTick {
  msOfDay: number
  bid: number
  ask: number
  impliedVolatility: number
  delta: number
  gamma: number
  theta: number
  vega: number
  rho: number
  ivError: number
  vanna: number
  charm: number
  vomma: number
  veta: number
  speed: number
  zomma: number
  color: number
  ultima: number
  d1: number
  d2: number
  dualDelta: number
  dualGamma: number
  epsilon: number
  lambda: number
  vera: number
  underlyingMsOfDay: number
  underlyingPrice: number
  date: number
  expiration?: number
  strike?: number
  right?: string
  /**
   * Unix epoch milliseconds (UTC, DST-aware) combining `date` with
   * `ms_of_day` (Eastern-Time milliseconds-of-day). `undefined` when
   * `date` is absent (`0`).
   */
  timestampMs?: bigint
  /**
   * Unix epoch milliseconds (UTC, DST-aware) combining `date` with
   * `underlying_ms_of_day` (Eastern-Time milliseconds-of-day). `undefined` when
   * `date` is absent (`0`).
   */
  underlyingTimestampMs?: bigint
}

/**
 * Serialise a `GreeksAllTick` history result to an Arrow IPC stream
 * (the `apache-arrow` wire form). Mirrors the FlatFiles
 * `FlatFileRowList.toArrowIpc()` exit and the Python
 * `<TickName>List.to_arrow()` terminal so every binding can reach a
 * dataframe from an in-band history result.
 */
export declare function greeksAllTickToArrowIpc(rows: Array<GreeksAllTick>): Buffer

/** End-of-day union Greeks tick -- every Greek the v3 server publishes on */
export interface GreeksEodTick {
  msOfDay: number
  open: number
  high: number
  low: number
  close: number
  volume: bigint
  count: bigint
  bidSize: number
  bidExchange: number
  bid: number
  bidCondition: number
  askSize: number
  askExchange: number
  ask: number
  askCondition: number
  delta: number
  theta: number
  vega: number
  rho: number
  epsilon: number
  lambda: number
  gamma: number
  vanna: number
  charm: number
  vomma: number
  veta: number
  vera: number
  speed: number
  zomma: number
  color: number
  ultima: number
  d1: number
  d2: number
  dualDelta: number
  dualGamma: number
  impliedVolatility: number
  ivError: number
  underlyingMsOfDay: number
  underlyingPrice: number
  date: number
  expiration?: number
  strike?: number
  right?: string
  /**
   * Unix epoch milliseconds (UTC, DST-aware) combining `date` with
   * `ms_of_day` (Eastern-Time milliseconds-of-day). `undefined` when
   * `date` is absent (`0`).
   */
  timestampMs?: bigint
  /**
   * Unix epoch milliseconds (UTC, DST-aware) combining `date` with
   * `underlying_ms_of_day` (Eastern-Time milliseconds-of-day). `undefined` when
   * `date` is absent (`0`).
   */
  underlyingTimestampMs?: bigint
}

/**
 * Serialise a `GreeksEodTick` history result to an Arrow IPC stream
 * (the `apache-arrow` wire form). Mirrors the FlatFiles
 * `FlatFileRowList.toArrowIpc()` exit and the Python
 * `<TickName>List.to_arrow()` terminal so every binding can reach a
 * dataframe from an in-band history result.
 */
export declare function greeksEodTickToArrowIpc(rows: Array<GreeksEodTick>): Buffer

/** First-order Greeks tick -- the strict column subset emitted by the */
export interface GreeksFirstOrderTick {
  msOfDay: number
  bid: number
  ask: number
  delta: number
  theta: number
  vega: number
  rho: number
  epsilon: number
  lambda: number
  impliedVolatility: number
  ivError: number
  underlyingMsOfDay: number
  underlyingPrice: number
  date: number
  expiration?: number
  strike?: number
  right?: string
  /**
   * Unix epoch milliseconds (UTC, DST-aware) combining `date` with
   * `ms_of_day` (Eastern-Time milliseconds-of-day). `undefined` when
   * `date` is absent (`0`).
   */
  timestampMs?: bigint
  /**
   * Unix epoch milliseconds (UTC, DST-aware) combining `date` with
   * `underlying_ms_of_day` (Eastern-Time milliseconds-of-day). `undefined` when
   * `date` is absent (`0`).
   */
  underlyingTimestampMs?: bigint
}

/**
 * Serialise a `GreeksFirstOrderTick` history result to an Arrow IPC stream
 * (the `apache-arrow` wire form). Mirrors the FlatFiles
 * `FlatFileRowList.toArrowIpc()` exit and the Python
 * `<TickName>List.to_arrow()` terminal so every binding can reach a
 * dataframe from an in-band history result.
 */
export declare function greeksFirstOrderTickToArrowIpc(rows: Array<GreeksFirstOrderTick>): Buffer

/** Second-order Greeks tick -- the strict column subset emitted by the */
export interface GreeksSecondOrderTick {
  msOfDay: number
  bid: number
  ask: number
  gamma: number
  vanna: number
  charm: number
  vomma: number
  veta: number
  impliedVolatility: number
  ivError: number
  underlyingMsOfDay: number
  underlyingPrice: number
  date: number
  expiration?: number
  strike?: number
  right?: string
  /**
   * Unix epoch milliseconds (UTC, DST-aware) combining `date` with
   * `ms_of_day` (Eastern-Time milliseconds-of-day). `undefined` when
   * `date` is absent (`0`).
   */
  timestampMs?: bigint
  /**
   * Unix epoch milliseconds (UTC, DST-aware) combining `date` with
   * `underlying_ms_of_day` (Eastern-Time milliseconds-of-day). `undefined` when
   * `date` is absent (`0`).
   */
  underlyingTimestampMs?: bigint
}

/**
 * Serialise a `GreeksSecondOrderTick` history result to an Arrow IPC stream
 * (the `apache-arrow` wire form). Mirrors the FlatFiles
 * `FlatFileRowList.toArrowIpc()` exit and the Python
 * `<TickName>List.to_arrow()` terminal so every binding can reach a
 * dataframe from an in-band history result.
 */
export declare function greeksSecondOrderTickToArrowIpc(rows: Array<GreeksSecondOrderTick>): Buffer

/** Third-order Greeks tick -- the strict column subset emitted by the */
export interface GreeksThirdOrderTick {
  msOfDay: number
  bid: number
  ask: number
  speed: number
  zomma: number
  color: number
  ultima: number
  impliedVolatility: number
  ivError: number
  underlyingMsOfDay: number
  underlyingPrice: number
  date: number
  expiration?: number
  strike?: number
  right?: string
  /**
   * Unix epoch milliseconds (UTC, DST-aware) combining `date` with
   * `ms_of_day` (Eastern-Time milliseconds-of-day). `undefined` when
   * `date` is absent (`0`).
   */
  timestampMs?: bigint
  /**
   * Unix epoch milliseconds (UTC, DST-aware) combining `date` with
   * `underlying_ms_of_day` (Eastern-Time milliseconds-of-day). `undefined` when
   * `date` is absent (`0`).
   */
  underlyingTimestampMs?: bigint
}

/**
 * Serialise a `GreeksThirdOrderTick` history result to an Arrow IPC stream
 * (the `apache-arrow` wire form). Mirrors the FlatFiles
 * `FlatFileRowList.toArrowIpc()` exit and the Python
 * `<TickName>List.to_arrow()` terminal so every binding can reach a
 * dataframe from an in-band history result.
 */
export declare function greeksThirdOrderTickToArrowIpc(rows: Array<GreeksThirdOrderTick>): Buffer

/** Compute implied volatility via bisection. */
export declare function impliedVolatility(spot: number, strike: number, rate: number, divYield: number, tte: number, optionPrice: number, right: string): [number, number]

/**
 * Optional parameters for the `indexAtTimePrice` method. Keys are
 * the camelCase parameter names; absent keys behave exactly like an
 * omitted parameter. `timeoutMs` bounds the whole call: on expiry the
 * returned Promise rejects and the underlying request is cancelled.
 */
export interface IndexAtTimePriceOptions {
  /**
   * Per-call deadline as a non-negative whole number of milliseconds;
   * on expiry the returned Promise rejects and the underlying request
   * is cancelled. A non-finite, negative, or fractional value is
   * rejected with `InvalidParameterError` rather than coerced.
   */
  timeoutMs?: number
}

/**
 * Optional parameters for the `indexHistoryEOD` method. Keys are
 * the camelCase parameter names; absent keys behave exactly like an
 * omitted parameter. `timeoutMs` bounds the whole call: on expiry the
 * returned Promise rejects and the underlying request is cancelled.
 */
export interface IndexHistoryEodOptions {
  /**
   * Per-call deadline as a non-negative whole number of milliseconds;
   * on expiry the returned Promise rejects and the underlying request
   * is cancelled. A non-finite, negative, or fractional value is
   * rejected with `InvalidParameterError` rather than coerced.
   */
  timeoutMs?: number
}

/**
 * Optional parameters for the `indexHistoryOHLC` method. Keys are
 * the camelCase parameter names; absent keys behave exactly like an
 * omitted parameter. `timeoutMs` bounds the whole call: on expiry the
 * returned Promise rejects and the underlying request is cancelled.
 */
export interface IndexHistoryOhlcOptions {
  /** Interval preset or millisecond string. Defaults to `1s` when omitted — matching the upstream ThetaData Python library. Accepted values: `tick`, `10ms`, `100ms`, `500ms`, `1s`, `5s`, `10s`, `15s`, `30s`, `1m`, `5m`, `10m`, `15m`, `30m`, `1h`. */
  interval?: string
  /** Start time filter */
  startTime?: string | Date
  /** End time filter */
  endTime?: string | Date
  /**
   * Per-call deadline as a non-negative whole number of milliseconds;
   * on expiry the returned Promise rejects and the underlying request
   * is cancelled. A non-finite, negative, or fractional value is
   * rejected with `InvalidParameterError` rather than coerced.
   */
  timeoutMs?: number
}

/**
 * Optional parameters for the `indexHistoryPrice` method. Keys are
 * the camelCase parameter names; absent keys behave exactly like an
 * omitted parameter. `timeoutMs` bounds the whole call: on expiry the
 * returned Promise rejects and the underlying request is cancelled.
 */
export interface IndexHistoryPriceOptions {
  /** Interval preset or millisecond string. Defaults to `1s` when omitted — matching the upstream ThetaData Python library. Accepted values: `tick`, `10ms`, `100ms`, `500ms`, `1s`, `5s`, `10s`, `15s`, `30s`, `1m`, `5m`, `10m`, `15m`, `30m`, `1h`. */
  interval?: string
  /** Start time filter */
  startTime?: string | Date
  /** End time filter */
  endTime?: string | Date
  /** Start date YYYYMMDD */
  startDate?: string | Date
  /** End date YYYYMMDD */
  endDate?: string | Date
  /**
   * Per-call deadline as a non-negative whole number of milliseconds;
   * on expiry the returned Promise rejects and the underlying request
   * is cancelled. A non-finite, negative, or fractional value is
   * rejected with `InvalidParameterError` rather than coerced.
   */
  timeoutMs?: number
}

/**
 * Optional parameters for the `indexListDates` method. Keys are
 * the camelCase parameter names; absent keys behave exactly like an
 * omitted parameter. `timeoutMs` bounds the whole call: on expiry the
 * returned Promise rejects and the underlying request is cancelled.
 */
export interface IndexListDatesOptions {
  /**
   * Per-call deadline as a non-negative whole number of milliseconds;
   * on expiry the returned Promise rejects and the underlying request
   * is cancelled. A non-finite, negative, or fractional value is
   * rejected with `InvalidParameterError` rather than coerced.
   */
  timeoutMs?: number
}

/**
 * Optional parameters for the `indexListSymbols` method. Keys are
 * the camelCase parameter names; absent keys behave exactly like an
 * omitted parameter. `timeoutMs` bounds the whole call: on expiry the
 * returned Promise rejects and the underlying request is cancelled.
 */
export interface IndexListSymbolsOptions {
  /**
   * Per-call deadline as a non-negative whole number of milliseconds;
   * on expiry the returned Promise rejects and the underlying request
   * is cancelled. A non-finite, negative, or fractional value is
   * rejected with `InvalidParameterError` rather than coerced.
   */
  timeoutMs?: number
}

/** Index price-at-time tick -- the trade-shaped row the v3 server */
export interface IndexPriceAtTimeTick {
  msOfDay: number
  sequence: number
  extCondition1: number
  extCondition2: number
  extCondition3: number
  extCondition4: number
  condition: number
  size: number
  exchange: number
  price: number
  date: number
  /**
   * Unix epoch milliseconds (UTC, DST-aware) combining `date` with
   * `ms_of_day` (Eastern-Time milliseconds-of-day). `undefined` when
   * `date` is absent (`0`).
   */
  timestampMs?: bigint
}

/**
 * Serialise a `IndexPriceAtTimeTick` history result to an Arrow IPC stream
 * (the `apache-arrow` wire form). Mirrors the FlatFiles
 * `FlatFileRowList.toArrowIpc()` exit and the Python
 * `<TickName>List.to_arrow()` terminal so every binding can reach a
 * dataframe from an in-band history result.
 */
export declare function indexPriceAtTimeTickToArrowIpc(rows: Array<IndexPriceAtTimeTick>): Buffer

/**
 * Optional parameters for the `indexSnapshotMarketValue` method. Keys are
 * the camelCase parameter names; absent keys behave exactly like an
 * omitted parameter. `timeoutMs` bounds the whole call: on expiry the
 * returned Promise rejects and the underlying request is cancelled.
 */
export interface IndexSnapshotMarketValueOptions {
  /** Minimum time filter */
  minTime?: string | Date
  /**
   * Per-call deadline as a non-negative whole number of milliseconds;
   * on expiry the returned Promise rejects and the underlying request
   * is cancelled. A non-finite, negative, or fractional value is
   * rejected with `InvalidParameterError` rather than coerced.
   */
  timeoutMs?: number
}

/**
 * Optional parameters for the `indexSnapshotOHLC` method. Keys are
 * the camelCase parameter names; absent keys behave exactly like an
 * omitted parameter. `timeoutMs` bounds the whole call: on expiry the
 * returned Promise rejects and the underlying request is cancelled.
 */
export interface IndexSnapshotOhlcOptions {
  /** Minimum time filter */
  minTime?: string | Date
  /**
   * Per-call deadline as a non-negative whole number of milliseconds;
   * on expiry the returned Promise rejects and the underlying request
   * is cancelled. A non-finite, negative, or fractional value is
   * rejected with `InvalidParameterError` rather than coerced.
   */
  timeoutMs?: number
}

/**
 * Optional parameters for the `indexSnapshotPrice` method. Keys are
 * the camelCase parameter names; absent keys behave exactly like an
 * omitted parameter. `timeoutMs` bounds the whole call: on expiry the
 * returned Promise rejects and the underlying request is cancelled.
 */
export interface IndexSnapshotPriceOptions {
  /** Minimum time filter */
  minTime?: string | Date
  /**
   * Per-call deadline as a non-negative whole number of milliseconds;
   * on expiry the returned Promise rejects and the underlying request
   * is cancelled. A non-finite, negative, or fractional value is
   * rejected with `InvalidParameterError` rather than coerced.
   */
  timeoutMs?: number
}

/**
 * Optional parameters for the `interestRateHistoryEOD` method. Keys are
 * the camelCase parameter names; absent keys behave exactly like an
 * omitted parameter. `timeoutMs` bounds the whole call: on expiry the
 * returned Promise rejects and the underlying request is cancelled.
 */
export interface InterestRateHistoryEodOptions {
  /**
   * Per-call deadline as a non-negative whole number of milliseconds;
   * on expiry the returned Promise rejects and the underlying request
   * is cancelled. A non-finite, negative, or fractional value is
   * rejected with `InvalidParameterError` rather than coerced.
   */
  timeoutMs?: number
}

/** Interest rate tick. End-of-day interest rate (percent). */
export interface InterestRateTick {
  date: number
  rate: number
}

/**
 * Serialise a `InterestRateTick` history result to an Arrow IPC stream
 * (the `apache-arrow` wire form). Mirrors the FlatFiles
 * `FlatFileRowList.toArrowIpc()` exit and the Python
 * `<TickName>List.to_arrow()` terminal so every binding can reach a
 * dataframe from an in-band history result.
 */
export declare function interestRateTickToArrowIpc(rows: Array<InterestRateTick>): Buffer

/** Wire string enum `Interval`. */
export declare const enum Interval {
  Tick = 'tick',
  Ms10 = '10ms',
  Ms100 = '100ms',
  Ms500 = '500ms',
  S1 = '1s',
  S5 = '5s',
  S10 = '10s',
  S15 = '15s',
  S30 = '30s',
  M1 = '1m',
  M5 = '5m',
  M10 = '10m',
  M15 = '15m',
  M30 = '30m',
  H1 = '1h'
}

/** Implied volatility tick. */
export interface IvTick {
  msOfDay: number
  bid: number
  bidImpliedVolatility: number
  midpoint: number
  impliedVolatility: number
  ask: number
  askImpliedVolatility: number
  ivError: number
  underlyingMsOfDay: number
  underlyingPrice: number
  date: number
  expiration?: number
  strike?: number
  right?: string
  /**
   * Unix epoch milliseconds (UTC, DST-aware) combining `date` with
   * `ms_of_day` (Eastern-Time milliseconds-of-day). `undefined` when
   * `date` is absent (`0`).
   */
  timestampMs?: bigint
  /**
   * Unix epoch milliseconds (UTC, DST-aware) combining `date` with
   * `underlying_ms_of_day` (Eastern-Time milliseconds-of-day). `undefined` when
   * `date` is absent (`0`).
   */
  underlyingTimestampMs?: bigint
}

/**
 * Serialise a `IvTick` history result to an Arrow IPC stream
 * (the `apache-arrow` wire form). Mirrors the FlatFiles
 * `FlatFileRowList.toArrowIpc()` exit and the Python
 * `<TickName>List.to_arrow()` terminal so every binding can reach a
 * dataframe from an in-band history result.
 */
export declare function ivTickToArrowIpc(rows: Array<IvTick>): Buffer

/** FPSS login succeeded. `permissions` is the server's opaque bundle string — diagnostic metadata only; for feature gating use the Nexus REST subscription tiers. */
export interface LoginSuccess {
  permissions: string
}

/** FPSS market-close signal (wire code 32). Carries no payload. */
export interface MarketClose {

}

/** FPSS market-open signal (wire code 30). Carries no payload. */
export interface MarketOpen {

}

/** FPSS MarketValue tick (wire code 25). A calculated theoretical market value derived from the real-time bid/ask — `market_bid` / `market_ask` are the quote bid/ask after a size-imbalance + spread-aware nudge, `market_price` is their integer midpoint. Per-contract only (no full-stream variant). */
export interface MarketValue {
  contract: Contract
  msOfDay: number
  marketBid: number
  marketAsk: number
  marketPrice: number
  date: number
  receivedAtNs: bigint
}

/** Market value tick -- quoted bid/ask/price for a symbol. */
export interface MarketValueTick {
  msOfDay: number
  marketBid: number
  marketAsk: number
  marketPrice: number
  date: number
  expiration?: number
  strike?: number
  right?: string
  /**
   * Unix epoch milliseconds (UTC, DST-aware) combining `date` with
   * `ms_of_day` (Eastern-Time milliseconds-of-day). `undefined` when
   * `date` is absent (`0`).
   */
  timestampMs?: bigint
}

/**
 * Serialise a `MarketValueTick` history result to an Arrow IPC stream
 * (the `apache-arrow` wire form). Mirrors the FlatFiles
 * `FlatFileRowList.toArrowIpc()` exit and the Python
 * `<TickName>List.to_arrow()` terminal so every binding can reach a
 * dataframe from an in-band history result.
 */
export declare function marketValueTickToArrowIpc(rows: Array<MarketValueTick>): Buffer

/** OHLC tick. Aggregated bar data including SIP-rule VWAP. */
export interface OhlcTick {
  msOfDay: number
  open: number
  high: number
  low: number
  close: number
  volume: bigint
  count: bigint
  vwap: number
  date: number
  expiration?: number
  strike?: number
  right?: string
  /**
   * Unix epoch milliseconds (UTC, DST-aware) combining `date` with
   * `ms_of_day` (Eastern-Time milliseconds-of-day). `undefined` when
   * `date` is absent (`0`).
   */
  timestampMs?: bigint
}

/**
 * Serialise a `OhlcTick` history result to an Arrow IPC stream
 * (the `apache-arrow` wire form). Mirrors the FlatFiles
 * `FlatFileRowList.toArrowIpc()` exit and the Python
 * `<TickName>List.to_arrow()` terminal so every binding can reach a
 * dataframe from an in-band history result.
 */
export declare function ohlcTickToArrowIpc(rows: Array<OhlcTick>): Buffer

/** FPSS OHLCVC bar. */
export interface Ohlcvc {
  contract: Contract
  msOfDay: number
  open: number
  high: number
  low: number
  close: number
  volume: bigint
  count: bigint
  date: number
  receivedAtNs: bigint
}

/** FPSS OpenInterest tick. */
export interface OpenInterest {
  contract: Contract
  msOfDay: number
  openInterest: number
  date: number
  receivedAtNs: bigint
}

/** Open interest tick. */
export interface OpenInterestTick {
  msOfDay: number
  openInterest: number
  date: number
  expiration?: number
  strike?: number
  right?: string
  /**
   * Unix epoch milliseconds (UTC, DST-aware) combining `date` with
   * `ms_of_day` (Eastern-Time milliseconds-of-day). `undefined` when
   * `date` is absent (`0`).
   */
  timestampMs?: bigint
}

/**
 * Serialise a `OpenInterestTick` history result to an Arrow IPC stream
 * (the `apache-arrow` wire form). Mirrors the FlatFiles
 * `FlatFileRowList.toArrowIpc()` exit and the Python
 * `<TickName>List.to_arrow()` terminal so every binding can reach a
 * dataframe from an in-band history result.
 */
export declare function openInterestTickToArrowIpc(rows: Array<OpenInterestTick>): Buffer

/**
 * Optional parameters for the `optionAtTimeQuote` method. Keys are
 * the camelCase parameter names; absent keys behave exactly like an
 * omitted parameter. `timeoutMs` bounds the whole call: on expiry the
 * returned Promise rejects and the underlying request is cancelled.
 */
export interface OptionAtTimeQuoteOptions {
  /** Strike price in dollars as a string (e.g. 500 or 17.5). Use `*` for wildcard selection. */
  strike?: string
  /** Option side. Accepted values: `call`, `put`, `both`. */
  right?: string
  /** Maximum days to expiration */
  maxDte?: number
  /** Strike range filter */
  strikeRange?: number
  /**
   * Per-call deadline as a non-negative whole number of milliseconds;
   * on expiry the returned Promise rejects and the underlying request
   * is cancelled. A non-finite, negative, or fractional value is
   * rejected with `InvalidParameterError` rather than coerced.
   */
  timeoutMs?: number
}

/**
 * Optional parameters for the `optionAtTimeTrade` method. Keys are
 * the camelCase parameter names; absent keys behave exactly like an
 * omitted parameter. `timeoutMs` bounds the whole call: on expiry the
 * returned Promise rejects and the underlying request is cancelled.
 */
export interface OptionAtTimeTradeOptions {
  /** Strike price in dollars as a string (e.g. 500 or 17.5). Use `*` for wildcard selection. */
  strike?: string
  /** Option side. Accepted values: `call`, `put`, `both`. */
  right?: string
  /** Maximum days to expiration */
  maxDte?: number
  /** Strike range filter */
  strikeRange?: number
  /**
   * Per-call deadline as a non-negative whole number of milliseconds;
   * on expiry the returned Promise rejects and the underlying request
   * is cancelled. A non-finite, negative, or fractional value is
   * rejected with `InvalidParameterError` rather than coerced.
   */
  timeoutMs?: number
}

/** Option contract. Contract specification. */
export interface OptionContract {
  symbol: string
  expiration: number
  strike: number
  right: string
}

/**
 * Optional parameters for the `optionHistoryEOD` method. Keys are
 * the camelCase parameter names; absent keys behave exactly like an
 * omitted parameter. `timeoutMs` bounds the whole call: on expiry the
 * returned Promise rejects and the underlying request is cancelled.
 */
export interface OptionHistoryEodOptions {
  /** Strike price in dollars as a string (e.g. 500 or 17.5). Use `*` for wildcard selection. */
  strike?: string
  /** Option side. Accepted values: `call`, `put`, `both`. */
  right?: string
  /** Maximum days to expiration */
  maxDte?: number
  /** Strike range filter */
  strikeRange?: number
  /**
   * Per-call deadline as a non-negative whole number of milliseconds;
   * on expiry the returned Promise rejects and the underlying request
   * is cancelled. A non-finite, negative, or fractional value is
   * rejected with `InvalidParameterError` rather than coerced.
   */
  timeoutMs?: number
}

/**
 * Optional parameters for the `optionHistoryGreeksAll` method. Keys are
 * the camelCase parameter names; absent keys behave exactly like an
 * omitted parameter. `timeoutMs` bounds the whole call: on expiry the
 * returned Promise rejects and the underlying request is cancelled.
 */
export interface OptionHistoryGreeksAllOptions {
  /** Strike price in dollars as a string (e.g. 500 or 17.5). Use `*` for wildcard selection. */
  strike?: string
  /** Option side. Accepted values: `call`, `put`, `both`. */
  right?: string
  /** Interval preset or millisecond string. Defaults to `1s` when omitted — matching the upstream ThetaData Python library. Accepted values: `tick`, `10ms`, `100ms`, `500ms`, `1s`, `5s`, `10s`, `15s`, `30s`, `1m`, `5m`, `10m`, `15m`, `30m`, `1h`. */
  interval?: string
  /** Start time filter */
  startTime?: string | Date
  /** End time filter */
  endTime?: string | Date
  /** Annualized expected dividend amount, in dollars per share, used in the Greeks calculation (e.g. 2.5 is $2.50 per share per year). */
  annualDividend?: number
  /** Risk-free-rate source used in the Greeks calculation. Accepted values: `sofr`, `treasury_m1`, `treasury_m3`, `treasury_m6`, `treasury_y1`, `treasury_y2`, `treasury_y3`, `treasury_y5`, `treasury_y7`, `treasury_y10`, `treasury_y20`, `treasury_y30`. */
  rateType?: string
  /** Interest rate as a percent (4.36 means 4.36%, matching the InterestRateTick.rate convention) used in the Greeks calculation. Applied when rate_type selects a manual rate. */
  rateValue?: number
  /** Greeks model version. Accepted values: `latest`, `1`. */
  version?: string
  /** Strike range filter */
  strikeRange?: number
  /** Start date YYYYMMDD */
  startDate?: string | Date
  /** End date YYYYMMDD */
  endDate?: string | Date
  /**
   * Per-call deadline as a non-negative whole number of milliseconds;
   * on expiry the returned Promise rejects and the underlying request
   * is cancelled. A non-finite, negative, or fractional value is
   * rejected with `InvalidParameterError` rather than coerced.
   */
  timeoutMs?: number
}

/**
 * Optional parameters for the `optionHistoryGreeksEOD` method. Keys are
 * the camelCase parameter names; absent keys behave exactly like an
 * omitted parameter. `timeoutMs` bounds the whole call: on expiry the
 * returned Promise rejects and the underlying request is cancelled.
 */
export interface OptionHistoryGreeksEodOptions {
  /** Strike price in dollars as a string (e.g. 500 or 17.5). Use `*` for wildcard selection. */
  strike?: string
  /** Option side. Accepted values: `call`, `put`, `both`. */
  right?: string
  /** Annualized expected dividend amount, in dollars per share, used in the Greeks calculation (e.g. 2.5 is $2.50 per share per year). */
  annualDividend?: number
  /** Risk-free-rate source used in the Greeks calculation. Accepted values: `sofr`, `treasury_m1`, `treasury_m3`, `treasury_m6`, `treasury_y1`, `treasury_y2`, `treasury_y3`, `treasury_y5`, `treasury_y7`, `treasury_y10`, `treasury_y20`, `treasury_y30`. */
  rateType?: string
  /** Interest rate as a percent (4.36 means 4.36%, matching the InterestRateTick.rate convention) used in the Greeks calculation. Applied when rate_type selects a manual rate. */
  rateValue?: number
  /** Greeks model version. Accepted values: `latest`, `1`. */
  version?: string
  /** When true, use the NBBO-derived underlyer price as the Greeks input instead of the last trade. */
  underlyerUseNbbo?: boolean
  /** Maximum days to expiration */
  maxDte?: number
  /** Strike range filter */
  strikeRange?: number
  /**
   * Per-call deadline as a non-negative whole number of milliseconds;
   * on expiry the returned Promise rejects and the underlying request
   * is cancelled. A non-finite, negative, or fractional value is
   * rejected with `InvalidParameterError` rather than coerced.
   */
  timeoutMs?: number
}

/**
 * Optional parameters for the `optionHistoryGreeksFirstOrder` method. Keys are
 * the camelCase parameter names; absent keys behave exactly like an
 * omitted parameter. `timeoutMs` bounds the whole call: on expiry the
 * returned Promise rejects and the underlying request is cancelled.
 */
export interface OptionHistoryGreeksFirstOrderOptions {
  /** Strike price in dollars as a string (e.g. 500 or 17.5). Use `*` for wildcard selection. */
  strike?: string
  /** Option side. Accepted values: `call`, `put`, `both`. */
  right?: string
  /** Interval preset or millisecond string. Defaults to `1s` when omitted — matching the upstream ThetaData Python library. Accepted values: `tick`, `10ms`, `100ms`, `500ms`, `1s`, `5s`, `10s`, `15s`, `30s`, `1m`, `5m`, `10m`, `15m`, `30m`, `1h`. */
  interval?: string
  /** Start time filter */
  startTime?: string | Date
  /** End time filter */
  endTime?: string | Date
  /** Annualized expected dividend amount, in dollars per share, used in the Greeks calculation (e.g. 2.5 is $2.50 per share per year). */
  annualDividend?: number
  /** Risk-free-rate source used in the Greeks calculation. Accepted values: `sofr`, `treasury_m1`, `treasury_m3`, `treasury_m6`, `treasury_y1`, `treasury_y2`, `treasury_y3`, `treasury_y5`, `treasury_y7`, `treasury_y10`, `treasury_y20`, `treasury_y30`. */
  rateType?: string
  /** Interest rate as a percent (4.36 means 4.36%, matching the InterestRateTick.rate convention) used in the Greeks calculation. Applied when rate_type selects a manual rate. */
  rateValue?: number
  /** Greeks model version. Accepted values: `latest`, `1`. */
  version?: string
  /** Strike range filter */
  strikeRange?: number
  /** Start date YYYYMMDD */
  startDate?: string | Date
  /** End date YYYYMMDD */
  endDate?: string | Date
  /**
   * Per-call deadline as a non-negative whole number of milliseconds;
   * on expiry the returned Promise rejects and the underlying request
   * is cancelled. A non-finite, negative, or fractional value is
   * rejected with `InvalidParameterError` rather than coerced.
   */
  timeoutMs?: number
}

/**
 * Optional parameters for the `optionHistoryGreeksImpliedVolatility` method. Keys are
 * the camelCase parameter names; absent keys behave exactly like an
 * omitted parameter. `timeoutMs` bounds the whole call: on expiry the
 * returned Promise rejects and the underlying request is cancelled.
 */
export interface OptionHistoryGreeksImpliedVolatilityOptions {
  /** Strike price in dollars as a string (e.g. 500 or 17.5). Use `*` for wildcard selection. */
  strike?: string
  /** Option side. Accepted values: `call`, `put`, `both`. */
  right?: string
  /** Interval preset or millisecond string. Defaults to `1s` when omitted — matching the upstream ThetaData Python library. Accepted values: `tick`, `10ms`, `100ms`, `500ms`, `1s`, `5s`, `10s`, `15s`, `30s`, `1m`, `5m`, `10m`, `15m`, `30m`, `1h`. */
  interval?: string
  /** Start time filter */
  startTime?: string | Date
  /** End time filter */
  endTime?: string | Date
  /** Annualized expected dividend amount, in dollars per share, used in the Greeks calculation (e.g. 2.5 is $2.50 per share per year). */
  annualDividend?: number
  /** Risk-free-rate source used in the Greeks calculation. Accepted values: `sofr`, `treasury_m1`, `treasury_m3`, `treasury_m6`, `treasury_y1`, `treasury_y2`, `treasury_y3`, `treasury_y5`, `treasury_y7`, `treasury_y10`, `treasury_y20`, `treasury_y30`. */
  rateType?: string
  /** Interest rate as a percent (4.36 means 4.36%, matching the InterestRateTick.rate convention) used in the Greeks calculation. Applied when rate_type selects a manual rate. */
  rateValue?: number
  /** Greeks model version. Accepted values: `latest`, `1`. */
  version?: string
  /** Strike range filter */
  strikeRange?: number
  /** Start date YYYYMMDD */
  startDate?: string | Date
  /** End date YYYYMMDD */
  endDate?: string | Date
  /**
   * Per-call deadline as a non-negative whole number of milliseconds;
   * on expiry the returned Promise rejects and the underlying request
   * is cancelled. A non-finite, negative, or fractional value is
   * rejected with `InvalidParameterError` rather than coerced.
   */
  timeoutMs?: number
}

/**
 * Optional parameters for the `optionHistoryGreeksSecondOrder` method. Keys are
 * the camelCase parameter names; absent keys behave exactly like an
 * omitted parameter. `timeoutMs` bounds the whole call: on expiry the
 * returned Promise rejects and the underlying request is cancelled.
 */
export interface OptionHistoryGreeksSecondOrderOptions {
  /** Strike price in dollars as a string (e.g. 500 or 17.5). Use `*` for wildcard selection. */
  strike?: string
  /** Option side. Accepted values: `call`, `put`, `both`. */
  right?: string
  /** Interval preset or millisecond string. Defaults to `1s` when omitted — matching the upstream ThetaData Python library. Accepted values: `tick`, `10ms`, `100ms`, `500ms`, `1s`, `5s`, `10s`, `15s`, `30s`, `1m`, `5m`, `10m`, `15m`, `30m`, `1h`. */
  interval?: string
  /** Start time filter */
  startTime?: string | Date
  /** End time filter */
  endTime?: string | Date
  /** Annualized expected dividend amount, in dollars per share, used in the Greeks calculation (e.g. 2.5 is $2.50 per share per year). */
  annualDividend?: number
  /** Risk-free-rate source used in the Greeks calculation. Accepted values: `sofr`, `treasury_m1`, `treasury_m3`, `treasury_m6`, `treasury_y1`, `treasury_y2`, `treasury_y3`, `treasury_y5`, `treasury_y7`, `treasury_y10`, `treasury_y20`, `treasury_y30`. */
  rateType?: string
  /** Interest rate as a percent (4.36 means 4.36%, matching the InterestRateTick.rate convention) used in the Greeks calculation. Applied when rate_type selects a manual rate. */
  rateValue?: number
  /** Greeks model version. Accepted values: `latest`, `1`. */
  version?: string
  /** Strike range filter */
  strikeRange?: number
  /** Start date YYYYMMDD */
  startDate?: string | Date
  /** End date YYYYMMDD */
  endDate?: string | Date
  /**
   * Per-call deadline as a non-negative whole number of milliseconds;
   * on expiry the returned Promise rejects and the underlying request
   * is cancelled. A non-finite, negative, or fractional value is
   * rejected with `InvalidParameterError` rather than coerced.
   */
  timeoutMs?: number
}

/**
 * Optional parameters for the `optionHistoryGreeksThirdOrder` method. Keys are
 * the camelCase parameter names; absent keys behave exactly like an
 * omitted parameter. `timeoutMs` bounds the whole call: on expiry the
 * returned Promise rejects and the underlying request is cancelled.
 */
export interface OptionHistoryGreeksThirdOrderOptions {
  /** Strike price in dollars as a string (e.g. 500 or 17.5). Use `*` for wildcard selection. */
  strike?: string
  /** Option side. Accepted values: `call`, `put`, `both`. */
  right?: string
  /** Interval preset or millisecond string. Defaults to `1s` when omitted — matching the upstream ThetaData Python library. Accepted values: `tick`, `10ms`, `100ms`, `500ms`, `1s`, `5s`, `10s`, `15s`, `30s`, `1m`, `5m`, `10m`, `15m`, `30m`, `1h`. */
  interval?: string
  /** Start time filter */
  startTime?: string | Date
  /** End time filter */
  endTime?: string | Date
  /** Annualized expected dividend amount, in dollars per share, used in the Greeks calculation (e.g. 2.5 is $2.50 per share per year). */
  annualDividend?: number
  /** Risk-free-rate source used in the Greeks calculation. Accepted values: `sofr`, `treasury_m1`, `treasury_m3`, `treasury_m6`, `treasury_y1`, `treasury_y2`, `treasury_y3`, `treasury_y5`, `treasury_y7`, `treasury_y10`, `treasury_y20`, `treasury_y30`. */
  rateType?: string
  /** Interest rate as a percent (4.36 means 4.36%, matching the InterestRateTick.rate convention) used in the Greeks calculation. Applied when rate_type selects a manual rate. */
  rateValue?: number
  /** Greeks model version. Accepted values: `latest`, `1`. */
  version?: string
  /** Strike range filter */
  strikeRange?: number
  /** Start date YYYYMMDD */
  startDate?: string | Date
  /** End date YYYYMMDD */
  endDate?: string | Date
  /**
   * Per-call deadline as a non-negative whole number of milliseconds;
   * on expiry the returned Promise rejects and the underlying request
   * is cancelled. A non-finite, negative, or fractional value is
   * rejected with `InvalidParameterError` rather than coerced.
   */
  timeoutMs?: number
}

/**
 * Optional parameters for the `optionHistoryOHLC` method. Keys are
 * the camelCase parameter names; absent keys behave exactly like an
 * omitted parameter. `timeoutMs` bounds the whole call: on expiry the
 * returned Promise rejects and the underlying request is cancelled.
 */
export interface OptionHistoryOhlcOptions {
  /** Strike price in dollars as a string (e.g. 500 or 17.5). Use `*` for wildcard selection. */
  strike?: string
  /** Option side. Accepted values: `call`, `put`, `both`. */
  right?: string
  /** Interval preset or millisecond string. Defaults to `1s` when omitted — matching the upstream ThetaData Python library. Accepted values: `tick`, `10ms`, `100ms`, `500ms`, `1s`, `5s`, `10s`, `15s`, `30s`, `1m`, `5m`, `10m`, `15m`, `30m`, `1h`. */
  interval?: string
  /** Start time filter */
  startTime?: string | Date
  /** End time filter */
  endTime?: string | Date
  /** Strike range filter */
  strikeRange?: number
  /** Start date YYYYMMDD */
  startDate?: string | Date
  /** End date YYYYMMDD */
  endDate?: string | Date
  /**
   * Per-call deadline as a non-negative whole number of milliseconds;
   * on expiry the returned Promise rejects and the underlying request
   * is cancelled. A non-finite, negative, or fractional value is
   * rejected with `InvalidParameterError` rather than coerced.
   */
  timeoutMs?: number
}

/**
 * Optional parameters for the `optionHistoryOpenInterest` method. Keys are
 * the camelCase parameter names; absent keys behave exactly like an
 * omitted parameter. `timeoutMs` bounds the whole call: on expiry the
 * returned Promise rejects and the underlying request is cancelled.
 */
export interface OptionHistoryOpenInterestOptions {
  /** Strike price in dollars as a string (e.g. 500 or 17.5). Use `*` for wildcard selection. */
  strike?: string
  /** Option side. Accepted values: `call`, `put`, `both`. */
  right?: string
  /** Maximum days to expiration */
  maxDte?: number
  /** Strike range filter */
  strikeRange?: number
  /** Start date YYYYMMDD */
  startDate?: string | Date
  /** End date YYYYMMDD */
  endDate?: string | Date
  /**
   * Per-call deadline as a non-negative whole number of milliseconds;
   * on expiry the returned Promise rejects and the underlying request
   * is cancelled. A non-finite, negative, or fractional value is
   * rejected with `InvalidParameterError` rather than coerced.
   */
  timeoutMs?: number
}

/**
 * Optional parameters for the `optionHistoryQuote` method. Keys are
 * the camelCase parameter names; absent keys behave exactly like an
 * omitted parameter. `timeoutMs` bounds the whole call: on expiry the
 * returned Promise rejects and the underlying request is cancelled.
 */
export interface OptionHistoryQuoteOptions {
  /** Strike price in dollars as a string (e.g. 500 or 17.5). Use `*` for wildcard selection. */
  strike?: string
  /** Option side. Accepted values: `call`, `put`, `both`. */
  right?: string
  /** Interval preset or millisecond string. Defaults to `1s` when omitted — matching the upstream ThetaData Python library. Accepted values: `tick`, `10ms`, `100ms`, `500ms`, `1s`, `5s`, `10s`, `15s`, `30s`, `1m`, `5m`, `10m`, `15m`, `30m`, `1h`. */
  interval?: string
  /** Start time filter */
  startTime?: string | Date
  /** End time filter */
  endTime?: string | Date
  /** Maximum days to expiration */
  maxDte?: number
  /** Strike range filter */
  strikeRange?: number
  /** Start date YYYYMMDD */
  startDate?: string | Date
  /** End date YYYYMMDD */
  endDate?: string | Date
  /**
   * Per-call deadline as a non-negative whole number of milliseconds;
   * on expiry the returned Promise rejects and the underlying request
   * is cancelled. A non-finite, negative, or fractional value is
   * rejected with `InvalidParameterError` rather than coerced.
   */
  timeoutMs?: number
}

/**
 * Optional parameters for the `optionHistoryTradeGreeksAll` method. Keys are
 * the camelCase parameter names; absent keys behave exactly like an
 * omitted parameter. `timeoutMs` bounds the whole call: on expiry the
 * returned Promise rejects and the underlying request is cancelled.
 */
export interface OptionHistoryTradeGreeksAllOptions {
  /** Strike price in dollars as a string (e.g. 500 or 17.5). Use `*` for wildcard selection. */
  strike?: string
  /** Option side. Accepted values: `call`, `put`, `both`. */
  right?: string
  /** Start time filter */
  startTime?: string | Date
  /** End time filter */
  endTime?: string | Date
  /** Annualized expected dividend amount, in dollars per share, used in the Greeks calculation (e.g. 2.5 is $2.50 per share per year). */
  annualDividend?: number
  /** Risk-free-rate source used in the Greeks calculation. Accepted values: `sofr`, `treasury_m1`, `treasury_m3`, `treasury_m6`, `treasury_y1`, `treasury_y2`, `treasury_y3`, `treasury_y5`, `treasury_y7`, `treasury_y10`, `treasury_y20`, `treasury_y30`. */
  rateType?: string
  /** Interest rate as a percent (4.36 means 4.36%, matching the InterestRateTick.rate convention) used in the Greeks calculation. Applied when rate_type selects a manual rate. */
  rateValue?: number
  /** Greeks model version. Accepted values: `latest`, `1`. */
  version?: string
  /** Maximum days to expiration */
  maxDte?: number
  /** Strike range filter */
  strikeRange?: number
  /** Start date YYYYMMDD */
  startDate?: string | Date
  /** End date YYYYMMDD */
  endDate?: string | Date
  /**
   * Per-call deadline as a non-negative whole number of milliseconds;
   * on expiry the returned Promise rejects and the underlying request
   * is cancelled. A non-finite, negative, or fractional value is
   * rejected with `InvalidParameterError` rather than coerced.
   */
  timeoutMs?: number
}

/**
 * Optional parameters for the `optionHistoryTradeGreeksFirstOrder` method. Keys are
 * the camelCase parameter names; absent keys behave exactly like an
 * omitted parameter. `timeoutMs` bounds the whole call: on expiry the
 * returned Promise rejects and the underlying request is cancelled.
 */
export interface OptionHistoryTradeGreeksFirstOrderOptions {
  /** Strike price in dollars as a string (e.g. 500 or 17.5). Use `*` for wildcard selection. */
  strike?: string
  /** Option side. Accepted values: `call`, `put`, `both`. */
  right?: string
  /** Start time filter */
  startTime?: string | Date
  /** End time filter */
  endTime?: string | Date
  /** Annualized expected dividend amount, in dollars per share, used in the Greeks calculation (e.g. 2.5 is $2.50 per share per year). */
  annualDividend?: number
  /** Risk-free-rate source used in the Greeks calculation. Accepted values: `sofr`, `treasury_m1`, `treasury_m3`, `treasury_m6`, `treasury_y1`, `treasury_y2`, `treasury_y3`, `treasury_y5`, `treasury_y7`, `treasury_y10`, `treasury_y20`, `treasury_y30`. */
  rateType?: string
  /** Interest rate as a percent (4.36 means 4.36%, matching the InterestRateTick.rate convention) used in the Greeks calculation. Applied when rate_type selects a manual rate. */
  rateValue?: number
  /** Greeks model version. Accepted values: `latest`, `1`. */
  version?: string
  /** Maximum days to expiration */
  maxDte?: number
  /** Strike range filter */
  strikeRange?: number
  /** Start date YYYYMMDD */
  startDate?: string | Date
  /** End date YYYYMMDD */
  endDate?: string | Date
  /**
   * Per-call deadline as a non-negative whole number of milliseconds;
   * on expiry the returned Promise rejects and the underlying request
   * is cancelled. A non-finite, negative, or fractional value is
   * rejected with `InvalidParameterError` rather than coerced.
   */
  timeoutMs?: number
}

/**
 * Optional parameters for the `optionHistoryTradeGreeksImpliedVolatility` method. Keys are
 * the camelCase parameter names; absent keys behave exactly like an
 * omitted parameter. `timeoutMs` bounds the whole call: on expiry the
 * returned Promise rejects and the underlying request is cancelled.
 */
export interface OptionHistoryTradeGreeksImpliedVolatilityOptions {
  /** Strike price in dollars as a string (e.g. 500 or 17.5). Use `*` for wildcard selection. */
  strike?: string
  /** Option side. Accepted values: `call`, `put`, `both`. */
  right?: string
  /** Start time filter */
  startTime?: string | Date
  /** End time filter */
  endTime?: string | Date
  /** Annualized expected dividend amount, in dollars per share, used in the Greeks calculation (e.g. 2.5 is $2.50 per share per year). */
  annualDividend?: number
  /** Risk-free-rate source used in the Greeks calculation. Accepted values: `sofr`, `treasury_m1`, `treasury_m3`, `treasury_m6`, `treasury_y1`, `treasury_y2`, `treasury_y3`, `treasury_y5`, `treasury_y7`, `treasury_y10`, `treasury_y20`, `treasury_y30`. */
  rateType?: string
  /** Interest rate as a percent (4.36 means 4.36%, matching the InterestRateTick.rate convention) used in the Greeks calculation. Applied when rate_type selects a manual rate. */
  rateValue?: number
  /** Greeks model version. Accepted values: `latest`, `1`. */
  version?: string
  /** Maximum days to expiration */
  maxDte?: number
  /** Strike range filter */
  strikeRange?: number
  /** Start date YYYYMMDD */
  startDate?: string | Date
  /** End date YYYYMMDD */
  endDate?: string | Date
  /**
   * Per-call deadline as a non-negative whole number of milliseconds;
   * on expiry the returned Promise rejects and the underlying request
   * is cancelled. A non-finite, negative, or fractional value is
   * rejected with `InvalidParameterError` rather than coerced.
   */
  timeoutMs?: number
}

/**
 * Optional parameters for the `optionHistoryTradeGreeksSecondOrder` method. Keys are
 * the camelCase parameter names; absent keys behave exactly like an
 * omitted parameter. `timeoutMs` bounds the whole call: on expiry the
 * returned Promise rejects and the underlying request is cancelled.
 */
export interface OptionHistoryTradeGreeksSecondOrderOptions {
  /** Strike price in dollars as a string (e.g. 500 or 17.5). Use `*` for wildcard selection. */
  strike?: string
  /** Option side. Accepted values: `call`, `put`, `both`. */
  right?: string
  /** Start time filter */
  startTime?: string | Date
  /** End time filter */
  endTime?: string | Date
  /** Annualized expected dividend amount, in dollars per share, used in the Greeks calculation (e.g. 2.5 is $2.50 per share per year). */
  annualDividend?: number
  /** Risk-free-rate source used in the Greeks calculation. Accepted values: `sofr`, `treasury_m1`, `treasury_m3`, `treasury_m6`, `treasury_y1`, `treasury_y2`, `treasury_y3`, `treasury_y5`, `treasury_y7`, `treasury_y10`, `treasury_y20`, `treasury_y30`. */
  rateType?: string
  /** Interest rate as a percent (4.36 means 4.36%, matching the InterestRateTick.rate convention) used in the Greeks calculation. Applied when rate_type selects a manual rate. */
  rateValue?: number
  /** Greeks model version. Accepted values: `latest`, `1`. */
  version?: string
  /** Maximum days to expiration */
  maxDte?: number
  /** Strike range filter */
  strikeRange?: number
  /** Start date YYYYMMDD */
  startDate?: string | Date
  /** End date YYYYMMDD */
  endDate?: string | Date
  /**
   * Per-call deadline as a non-negative whole number of milliseconds;
   * on expiry the returned Promise rejects and the underlying request
   * is cancelled. A non-finite, negative, or fractional value is
   * rejected with `InvalidParameterError` rather than coerced.
   */
  timeoutMs?: number
}

/**
 * Optional parameters for the `optionHistoryTradeGreeksThirdOrder` method. Keys are
 * the camelCase parameter names; absent keys behave exactly like an
 * omitted parameter. `timeoutMs` bounds the whole call: on expiry the
 * returned Promise rejects and the underlying request is cancelled.
 */
export interface OptionHistoryTradeGreeksThirdOrderOptions {
  /** Strike price in dollars as a string (e.g. 500 or 17.5). Use `*` for wildcard selection. */
  strike?: string
  /** Option side. Accepted values: `call`, `put`, `both`. */
  right?: string
  /** Start time filter */
  startTime?: string | Date
  /** End time filter */
  endTime?: string | Date
  /** Annualized expected dividend amount, in dollars per share, used in the Greeks calculation (e.g. 2.5 is $2.50 per share per year). */
  annualDividend?: number
  /** Risk-free-rate source used in the Greeks calculation. Accepted values: `sofr`, `treasury_m1`, `treasury_m3`, `treasury_m6`, `treasury_y1`, `treasury_y2`, `treasury_y3`, `treasury_y5`, `treasury_y7`, `treasury_y10`, `treasury_y20`, `treasury_y30`. */
  rateType?: string
  /** Interest rate as a percent (4.36 means 4.36%, matching the InterestRateTick.rate convention) used in the Greeks calculation. Applied when rate_type selects a manual rate. */
  rateValue?: number
  /** Greeks model version. Accepted values: `latest`, `1`. */
  version?: string
  /** Maximum days to expiration */
  maxDte?: number
  /** Strike range filter */
  strikeRange?: number
  /** Start date YYYYMMDD */
  startDate?: string | Date
  /** End date YYYYMMDD */
  endDate?: string | Date
  /**
   * Per-call deadline as a non-negative whole number of milliseconds;
   * on expiry the returned Promise rejects and the underlying request
   * is cancelled. A non-finite, negative, or fractional value is
   * rejected with `InvalidParameterError` rather than coerced.
   */
  timeoutMs?: number
}

/**
 * Optional parameters for the `optionHistoryTrade` method. Keys are
 * the camelCase parameter names; absent keys behave exactly like an
 * omitted parameter. `timeoutMs` bounds the whole call: on expiry the
 * returned Promise rejects and the underlying request is cancelled.
 */
export interface OptionHistoryTradeOptions {
  /** Strike price in dollars as a string (e.g. 500 or 17.5). Use `*` for wildcard selection. */
  strike?: string
  /** Option side. Accepted values: `call`, `put`, `both`. */
  right?: string
  /** Start time filter */
  startTime?: string | Date
  /** End time filter */
  endTime?: string | Date
  /** Maximum days to expiration */
  maxDte?: number
  /** Strike range filter */
  strikeRange?: number
  /** Start date YYYYMMDD */
  startDate?: string | Date
  /** End date YYYYMMDD */
  endDate?: string | Date
  /**
   * Per-call deadline as a non-negative whole number of milliseconds;
   * on expiry the returned Promise rejects and the underlying request
   * is cancelled. A non-finite, negative, or fractional value is
   * rejected with `InvalidParameterError` rather than coerced.
   */
  timeoutMs?: number
}

/**
 * Optional parameters for the `optionHistoryTradeQuote` method. Keys are
 * the camelCase parameter names; absent keys behave exactly like an
 * omitted parameter. `timeoutMs` bounds the whole call: on expiry the
 * returned Promise rejects and the underlying request is cancelled.
 */
export interface OptionHistoryTradeQuoteOptions {
  /** Strike price in dollars as a string (e.g. 500 or 17.5). Use `*` for wildcard selection. */
  strike?: string
  /** Option side. Accepted values: `call`, `put`, `both`. */
  right?: string
  /** Start time filter */
  startTime?: string | Date
  /** End time filter */
  endTime?: string | Date
  /** When true, quotes whose timestamp equals the trade timestamp are excluded; only quotes strictly before the trade are paired. */
  exclusive?: boolean
  /** Maximum days to expiration */
  maxDte?: number
  /** Strike range filter */
  strikeRange?: number
  /** Start date YYYYMMDD */
  startDate?: string | Date
  /** End date YYYYMMDD */
  endDate?: string | Date
  /**
   * Per-call deadline as a non-negative whole number of milliseconds;
   * on expiry the returned Promise rejects and the underlying request
   * is cancelled. A non-finite, negative, or fractional value is
   * rejected with `InvalidParameterError` rather than coerced.
   */
  timeoutMs?: number
}

/**
 * The expiration / strike / right of an option leg, passed to
 * `Contract.option(symbol, leg)` as a single object with named keys.
 *
 * Naming the three values — all of which are strings — keeps the
 * contract identity non-transposable: `{ expiration, strike, right }`
 * cannot silently accept a swapped pair the way three adjacent
 * positional string arguments could.
 */
export interface OptionLeg {
  /** Expiration date as `YYYYMMDD` (e.g. `"20260620"`). */
  expiration: string
  /**
   * Strike price in dollars, as a number or string (`550`, `550.5`,
   * `"550"` are equivalent).
   */
  strike: number | string
  /**
   * Option right: `"C"` / `"CALL"` / `"P"` / `"PUT"`
   * (case-insensitive).
   */
  right: string
}

/**
 * Optional parameters for the `optionListContracts` method. Keys are
 * the camelCase parameter names; absent keys behave exactly like an
 * omitted parameter. `timeoutMs` bounds the whole call: on expiry the
 * returned Promise rejects and the underlying request is cancelled.
 */
export interface OptionListContractsOptions {
  /** Maximum days to expiration */
  maxDte?: number
  /**
   * Per-call deadline as a non-negative whole number of milliseconds;
   * on expiry the returned Promise rejects and the underlying request
   * is cancelled. A non-finite, negative, or fractional value is
   * rejected with `InvalidParameterError` rather than coerced.
   */
  timeoutMs?: number
}

/**
 * Optional parameters for the `optionListDates` method. Keys are
 * the camelCase parameter names; absent keys behave exactly like an
 * omitted parameter. `timeoutMs` bounds the whole call: on expiry the
 * returned Promise rejects and the underlying request is cancelled.
 */
export interface OptionListDatesOptions {
  /**
   * Per-call deadline as a non-negative whole number of milliseconds;
   * on expiry the returned Promise rejects and the underlying request
   * is cancelled. A non-finite, negative, or fractional value is
   * rejected with `InvalidParameterError` rather than coerced.
   */
  timeoutMs?: number
}

/**
 * Optional parameters for the `optionListExpirations` method. Keys are
 * the camelCase parameter names; absent keys behave exactly like an
 * omitted parameter. `timeoutMs` bounds the whole call: on expiry the
 * returned Promise rejects and the underlying request is cancelled.
 */
export interface OptionListExpirationsOptions {
  /**
   * Per-call deadline as a non-negative whole number of milliseconds;
   * on expiry the returned Promise rejects and the underlying request
   * is cancelled. A non-finite, negative, or fractional value is
   * rejected with `InvalidParameterError` rather than coerced.
   */
  timeoutMs?: number
}

/**
 * Optional parameters for the `optionListStrikes` method. Keys are
 * the camelCase parameter names; absent keys behave exactly like an
 * omitted parameter. `timeoutMs` bounds the whole call: on expiry the
 * returned Promise rejects and the underlying request is cancelled.
 */
export interface OptionListStrikesOptions {
  /**
   * Per-call deadline as a non-negative whole number of milliseconds;
   * on expiry the returned Promise rejects and the underlying request
   * is cancelled. A non-finite, negative, or fractional value is
   * rejected with `InvalidParameterError` rather than coerced.
   */
  timeoutMs?: number
}

/**
 * Optional parameters for the `optionListSymbols` method. Keys are
 * the camelCase parameter names; absent keys behave exactly like an
 * omitted parameter. `timeoutMs` bounds the whole call: on expiry the
 * returned Promise rejects and the underlying request is cancelled.
 */
export interface OptionListSymbolsOptions {
  /**
   * Per-call deadline as a non-negative whole number of milliseconds;
   * on expiry the returned Promise rejects and the underlying request
   * is cancelled. A non-finite, negative, or fractional value is
   * rejected with `InvalidParameterError` rather than coerced.
   */
  timeoutMs?: number
}

/**
 * Optional parameters for the `optionSnapshotGreeksAll` method. Keys are
 * the camelCase parameter names; absent keys behave exactly like an
 * omitted parameter. `timeoutMs` bounds the whole call: on expiry the
 * returned Promise rejects and the underlying request is cancelled.
 */
export interface OptionSnapshotGreeksAllOptions {
  /** Strike price in dollars as a string (e.g. 500 or 17.5). Use `*` for wildcard selection. */
  strike?: string
  /** Option side. Accepted values: `call`, `put`, `both`. */
  right?: string
  /** Annualized expected dividend amount, in dollars per share, used in the Greeks calculation (e.g. 2.5 is $2.50 per share per year). */
  annualDividend?: number
  /** Risk-free-rate source used in the Greeks calculation. Accepted values: `sofr`, `treasury_m1`, `treasury_m3`, `treasury_m6`, `treasury_y1`, `treasury_y2`, `treasury_y3`, `treasury_y5`, `treasury_y7`, `treasury_y10`, `treasury_y20`, `treasury_y30`. */
  rateType?: string
  /** Interest rate as a percent (4.36 means 4.36%, matching the InterestRateTick.rate convention) used in the Greeks calculation. Applied when rate_type selects a manual rate. */
  rateValue?: number
  /** Underlying price in dollars used in the Greeks calculation, overriding the observed underlying when set. */
  stockPrice?: number
  /** Greeks model version. Accepted values: `latest`, `1`. */
  version?: string
  /** Maximum days to expiration */
  maxDte?: number
  /** Strike range filter */
  strikeRange?: number
  /** Minimum time filter */
  minTime?: string | Date
  /** When true, calculate Greeks against the option market value (mid-price) instead of the NBBO bid/ask pair. */
  useMarketValue?: boolean
  /**
   * Per-call deadline as a non-negative whole number of milliseconds;
   * on expiry the returned Promise rejects and the underlying request
   * is cancelled. A non-finite, negative, or fractional value is
   * rejected with `InvalidParameterError` rather than coerced.
   */
  timeoutMs?: number
}

/**
 * Optional parameters for the `optionSnapshotGreeksFirstOrder` method. Keys are
 * the camelCase parameter names; absent keys behave exactly like an
 * omitted parameter. `timeoutMs` bounds the whole call: on expiry the
 * returned Promise rejects and the underlying request is cancelled.
 */
export interface OptionSnapshotGreeksFirstOrderOptions {
  /** Strike price in dollars as a string (e.g. 500 or 17.5). Use `*` for wildcard selection. */
  strike?: string
  /** Option side. Accepted values: `call`, `put`, `both`. */
  right?: string
  /** Annualized expected dividend amount, in dollars per share, used in the Greeks calculation (e.g. 2.5 is $2.50 per share per year). */
  annualDividend?: number
  /** Risk-free-rate source used in the Greeks calculation. Accepted values: `sofr`, `treasury_m1`, `treasury_m3`, `treasury_m6`, `treasury_y1`, `treasury_y2`, `treasury_y3`, `treasury_y5`, `treasury_y7`, `treasury_y10`, `treasury_y20`, `treasury_y30`. */
  rateType?: string
  /** Interest rate as a percent (4.36 means 4.36%, matching the InterestRateTick.rate convention) used in the Greeks calculation. Applied when rate_type selects a manual rate. */
  rateValue?: number
  /** Underlying price in dollars used in the Greeks calculation, overriding the observed underlying when set. */
  stockPrice?: number
  /** Greeks model version. Accepted values: `latest`, `1`. */
  version?: string
  /** Maximum days to expiration */
  maxDte?: number
  /** Strike range filter */
  strikeRange?: number
  /** Minimum time filter */
  minTime?: string | Date
  /** When true, calculate Greeks against the option market value (mid-price) instead of the NBBO bid/ask pair. */
  useMarketValue?: boolean
  /**
   * Per-call deadline as a non-negative whole number of milliseconds;
   * on expiry the returned Promise rejects and the underlying request
   * is cancelled. A non-finite, negative, or fractional value is
   * rejected with `InvalidParameterError` rather than coerced.
   */
  timeoutMs?: number
}

/**
 * Optional parameters for the `optionSnapshotGreeksImpliedVolatility` method. Keys are
 * the camelCase parameter names; absent keys behave exactly like an
 * omitted parameter. `timeoutMs` bounds the whole call: on expiry the
 * returned Promise rejects and the underlying request is cancelled.
 */
export interface OptionSnapshotGreeksImpliedVolatilityOptions {
  /** Strike price in dollars as a string (e.g. 500 or 17.5). Use `*` for wildcard selection. */
  strike?: string
  /** Option side. Accepted values: `call`, `put`, `both`. */
  right?: string
  /** Annualized expected dividend amount, in dollars per share, used in the Greeks calculation (e.g. 2.5 is $2.50 per share per year). */
  annualDividend?: number
  /** Risk-free-rate source used in the Greeks calculation. Accepted values: `sofr`, `treasury_m1`, `treasury_m3`, `treasury_m6`, `treasury_y1`, `treasury_y2`, `treasury_y3`, `treasury_y5`, `treasury_y7`, `treasury_y10`, `treasury_y20`, `treasury_y30`. */
  rateType?: string
  /** Interest rate as a percent (4.36 means 4.36%, matching the InterestRateTick.rate convention) used in the Greeks calculation. Applied when rate_type selects a manual rate. */
  rateValue?: number
  /** Underlying price in dollars used in the Greeks calculation, overriding the observed underlying when set. */
  stockPrice?: number
  /** Greeks model version. Accepted values: `latest`, `1`. */
  version?: string
  /** Maximum days to expiration */
  maxDte?: number
  /** Strike range filter */
  strikeRange?: number
  /** Minimum time filter */
  minTime?: string | Date
  /** When true, calculate Greeks against the option market value (mid-price) instead of the NBBO bid/ask pair. */
  useMarketValue?: boolean
  /**
   * Per-call deadline as a non-negative whole number of milliseconds;
   * on expiry the returned Promise rejects and the underlying request
   * is cancelled. A non-finite, negative, or fractional value is
   * rejected with `InvalidParameterError` rather than coerced.
   */
  timeoutMs?: number
}

/**
 * Optional parameters for the `optionSnapshotGreeksSecondOrder` method. Keys are
 * the camelCase parameter names; absent keys behave exactly like an
 * omitted parameter. `timeoutMs` bounds the whole call: on expiry the
 * returned Promise rejects and the underlying request is cancelled.
 */
export interface OptionSnapshotGreeksSecondOrderOptions {
  /** Strike price in dollars as a string (e.g. 500 or 17.5). Use `*` for wildcard selection. */
  strike?: string
  /** Option side. Accepted values: `call`, `put`, `both`. */
  right?: string
  /** Annualized expected dividend amount, in dollars per share, used in the Greeks calculation (e.g. 2.5 is $2.50 per share per year). */
  annualDividend?: number
  /** Risk-free-rate source used in the Greeks calculation. Accepted values: `sofr`, `treasury_m1`, `treasury_m3`, `treasury_m6`, `treasury_y1`, `treasury_y2`, `treasury_y3`, `treasury_y5`, `treasury_y7`, `treasury_y10`, `treasury_y20`, `treasury_y30`. */
  rateType?: string
  /** Interest rate as a percent (4.36 means 4.36%, matching the InterestRateTick.rate convention) used in the Greeks calculation. Applied when rate_type selects a manual rate. */
  rateValue?: number
  /** Underlying price in dollars used in the Greeks calculation, overriding the observed underlying when set. */
  stockPrice?: number
  /** Greeks model version. Accepted values: `latest`, `1`. */
  version?: string
  /** Maximum days to expiration */
  maxDte?: number
  /** Strike range filter */
  strikeRange?: number
  /** Minimum time filter */
  minTime?: string | Date
  /** When true, calculate Greeks against the option market value (mid-price) instead of the NBBO bid/ask pair. */
  useMarketValue?: boolean
  /**
   * Per-call deadline as a non-negative whole number of milliseconds;
   * on expiry the returned Promise rejects and the underlying request
   * is cancelled. A non-finite, negative, or fractional value is
   * rejected with `InvalidParameterError` rather than coerced.
   */
  timeoutMs?: number
}

/**
 * Optional parameters for the `optionSnapshotGreeksThirdOrder` method. Keys are
 * the camelCase parameter names; absent keys behave exactly like an
 * omitted parameter. `timeoutMs` bounds the whole call: on expiry the
 * returned Promise rejects and the underlying request is cancelled.
 */
export interface OptionSnapshotGreeksThirdOrderOptions {
  /** Strike price in dollars as a string (e.g. 500 or 17.5). Use `*` for wildcard selection. */
  strike?: string
  /** Option side. Accepted values: `call`, `put`, `both`. */
  right?: string
  /** Annualized expected dividend amount, in dollars per share, used in the Greeks calculation (e.g. 2.5 is $2.50 per share per year). */
  annualDividend?: number
  /** Risk-free-rate source used in the Greeks calculation. Accepted values: `sofr`, `treasury_m1`, `treasury_m3`, `treasury_m6`, `treasury_y1`, `treasury_y2`, `treasury_y3`, `treasury_y5`, `treasury_y7`, `treasury_y10`, `treasury_y20`, `treasury_y30`. */
  rateType?: string
  /** Interest rate as a percent (4.36 means 4.36%, matching the InterestRateTick.rate convention) used in the Greeks calculation. Applied when rate_type selects a manual rate. */
  rateValue?: number
  /** Underlying price in dollars used in the Greeks calculation, overriding the observed underlying when set. */
  stockPrice?: number
  /** Greeks model version. Accepted values: `latest`, `1`. */
  version?: string
  /** Maximum days to expiration */
  maxDte?: number
  /** Strike range filter */
  strikeRange?: number
  /** Minimum time filter */
  minTime?: string | Date
  /** When true, calculate Greeks against the option market value (mid-price) instead of the NBBO bid/ask pair. */
  useMarketValue?: boolean
  /**
   * Per-call deadline as a non-negative whole number of milliseconds;
   * on expiry the returned Promise rejects and the underlying request
   * is cancelled. A non-finite, negative, or fractional value is
   * rejected with `InvalidParameterError` rather than coerced.
   */
  timeoutMs?: number
}

/**
 * Optional parameters for the `optionSnapshotMarketValue` method. Keys are
 * the camelCase parameter names; absent keys behave exactly like an
 * omitted parameter. `timeoutMs` bounds the whole call: on expiry the
 * returned Promise rejects and the underlying request is cancelled.
 */
export interface OptionSnapshotMarketValueOptions {
  /** Strike price in dollars as a string (e.g. 500 or 17.5). Use `*` for wildcard selection. */
  strike?: string
  /** Option side. Accepted values: `call`, `put`, `both`. */
  right?: string
  /** Maximum days to expiration */
  maxDte?: number
  /** Strike range filter */
  strikeRange?: number
  /** Minimum time filter */
  minTime?: string | Date
  /**
   * Per-call deadline as a non-negative whole number of milliseconds;
   * on expiry the returned Promise rejects and the underlying request
   * is cancelled. A non-finite, negative, or fractional value is
   * rejected with `InvalidParameterError` rather than coerced.
   */
  timeoutMs?: number
}

/**
 * Optional parameters for the `optionSnapshotOHLC` method. Keys are
 * the camelCase parameter names; absent keys behave exactly like an
 * omitted parameter. `timeoutMs` bounds the whole call: on expiry the
 * returned Promise rejects and the underlying request is cancelled.
 */
export interface OptionSnapshotOhlcOptions {
  /** Strike price in dollars as a string (e.g. 500 or 17.5). Use `*` for wildcard selection. */
  strike?: string
  /** Option side. Accepted values: `call`, `put`, `both`. */
  right?: string
  /** Maximum days to expiration */
  maxDte?: number
  /** Strike range filter */
  strikeRange?: number
  /** Minimum time filter */
  minTime?: string | Date
  /**
   * Per-call deadline as a non-negative whole number of milliseconds;
   * on expiry the returned Promise rejects and the underlying request
   * is cancelled. A non-finite, negative, or fractional value is
   * rejected with `InvalidParameterError` rather than coerced.
   */
  timeoutMs?: number
}

/**
 * Optional parameters for the `optionSnapshotOpenInterest` method. Keys are
 * the camelCase parameter names; absent keys behave exactly like an
 * omitted parameter. `timeoutMs` bounds the whole call: on expiry the
 * returned Promise rejects and the underlying request is cancelled.
 */
export interface OptionSnapshotOpenInterestOptions {
  /** Strike price in dollars as a string (e.g. 500 or 17.5). Use `*` for wildcard selection. */
  strike?: string
  /** Option side. Accepted values: `call`, `put`, `both`. */
  right?: string
  /** Maximum days to expiration */
  maxDte?: number
  /** Strike range filter */
  strikeRange?: number
  /** Minimum time filter */
  minTime?: string | Date
  /**
   * Per-call deadline as a non-negative whole number of milliseconds;
   * on expiry the returned Promise rejects and the underlying request
   * is cancelled. A non-finite, negative, or fractional value is
   * rejected with `InvalidParameterError` rather than coerced.
   */
  timeoutMs?: number
}

/**
 * Optional parameters for the `optionSnapshotQuote` method. Keys are
 * the camelCase parameter names; absent keys behave exactly like an
 * omitted parameter. `timeoutMs` bounds the whole call: on expiry the
 * returned Promise rejects and the underlying request is cancelled.
 */
export interface OptionSnapshotQuoteOptions {
  /** Strike price in dollars as a string (e.g. 500 or 17.5). Use `*` for wildcard selection. */
  strike?: string
  /** Option side. Accepted values: `call`, `put`, `both`. */
  right?: string
  /** Maximum days to expiration */
  maxDte?: number
  /** Strike range filter */
  strikeRange?: number
  /** Minimum time filter */
  minTime?: string | Date
  /**
   * Per-call deadline as a non-negative whole number of milliseconds;
   * on expiry the returned Promise rejects and the underlying request
   * is cancelled. A non-finite, negative, or fractional value is
   * rejected with `InvalidParameterError` rather than coerced.
   */
  timeoutMs?: number
}

/**
 * Optional parameters for the `optionSnapshotTrade` method. Keys are
 * the camelCase parameter names; absent keys behave exactly like an
 * omitted parameter. `timeoutMs` bounds the whole call: on expiry the
 * returned Promise rejects and the underlying request is cancelled.
 */
export interface OptionSnapshotTradeOptions {
  /** Strike price in dollars as a string (e.g. 500 or 17.5). Use `*` for wildcard selection. */
  strike?: string
  /** Option side. Accepted values: `call`, `put`, `both`. */
  right?: string
  /** Strike range filter */
  strikeRange?: number
  /** Minimum time filter */
  minTime?: string | Date
  /**
   * Per-call deadline as a non-negative whole number of milliseconds;
   * on expiry the returned Promise rejects and the underlying request
   * is cancelled. A non-finite, negative, or fractional value is
   * rejected with `InvalidParameterError` rather than coerced.
   */
  timeoutMs?: number
}

/** FPSS protocol-level parse error. Named `ParseError` on every binding so it never collides with the language's own error types (Python's exception classes, the JS global `Error`). */
export interface ParseError {
  message: string
}

/** FPSS server heartbeat (wire code 10). The server emits PING frames (observed 1-byte payload `[0]`) the client heartbeat logic does not have to answer; payload preserved for diagnostics. */
export interface Ping {
  payload: Array<number>
}

/** Price tick. Generic price data point. */
export interface PriceTick {
  msOfDay: number
  price: number
  date: number
  /**
   * Unix epoch milliseconds (UTC, DST-aware) combining `date` with
   * `ms_of_day` (Eastern-Time milliseconds-of-day). `undefined` when
   * `date` is absent (`0`).
   */
  timestampMs?: bigint
}

/**
 * Serialise a `PriceTick` history result to an Arrow IPC stream
 * (the `apache-arrow` wire form). Mirrors the FlatFiles
 * `FlatFileRowList.toArrowIpc()` exit and the Python
 * `<TickName>List.to_arrow()` terminal so every binding can reach a
 * dataframe from an in-band history result.
 */
export declare function priceTickToArrowIpc(rows: Array<PriceTick>): Buffer

/** FPSS Quote tick. */
export interface Quote {
  contract: Contract
  msOfDay: number
  bidSize: number
  bidExchange: number
  bid: number
  bidCondition: number
  askSize: number
  askExchange: number
  ask: number
  askCondition: number
  date: number
  receivedAtNs: bigint
}

/** Quote tick. NBBO quote data. */
export interface QuoteTick {
  msOfDay: number
  bidSize: number
  bidExchange: number
  bid: number
  bidCondition: number
  askSize: number
  askExchange: number
  ask: number
  askCondition: number
  date: number
  midpoint: number
  expiration?: number
  strike?: number
  right?: string
  /**
   * Unix epoch milliseconds (UTC, DST-aware) combining `date` with
   * `ms_of_day` (Eastern-Time milliseconds-of-day). `undefined` when
   * `date` is absent (`0`).
   */
  timestampMs?: bigint
}

/**
 * Serialise a `QuoteTick` history result to an Arrow IPC stream
 * (the `apache-arrow` wire form). Mirrors the FlatFiles
 * `FlatFileRowList.toArrowIpc()` exit and the Python
 * `<TickName>List.to_arrow()` terminal so every binding can reach a
 * dataframe from an in-band history result.
 */
export declare function quoteTickToArrowIpc(rows: Array<QuoteTick>): Buffer

/** Wire string enum `RateType`. */
export declare const enum RateType {
  Sofr = 'sofr',
  TreasuryM1 = 'treasury_m1',
  TreasuryM3 = 'treasury_m3',
  TreasuryM6 = 'treasury_m6',
  TreasuryY1 = 'treasury_y1',
  TreasuryY2 = 'treasury_y2',
  TreasuryY3 = 'treasury_y3',
  TreasuryY5 = 'treasury_y5',
  TreasuryY7 = 'treasury_y7',
  TreasuryY10 = 'treasury_y10',
  TreasuryY20 = 'treasury_y20',
  TreasuryY30 = 'treasury_y30'
}

/**
 * `(reason, attempt)` argument object handed to the JS reconnect
 * callback registered via `Config.setReconnectCallback`. `reason` is
 * the integer disconnect code; `attempt` is the
 * 1-based consecutive-reconnect counter.
 */
export interface ReconnectDecisionArgs {
  reason: number
  attempt: number
}

/** FPSS auto-reconnect succeeded — connection is live again. Carries no payload. */
export interface Reconnected {

}

/** FPSS server-side reconnect ack (wire code 13). Distinct from `Reconnected`, which the client emits from its auto-reconnect state machine once the new TLS session is authenticated. */
export interface ReconnectedServer {

}

/** FPSS auto-reconnect is about to attempt reconnection. Emitted before sleeping for `delay_ms` milliseconds. `attempt` is 1-based and saturates at the maximum 32-bit signed value if the reconnect loop exceeds 2^31 attempts. */
export interface Reconnecting {
  reason: number
  attempt: number
  delayMs: bigint
  /**
   * Resolved disconnect-reason name (e.g. `"TooManyRequests"`,
   * `"InvalidCredentials"`, `"Unspecified"` for unknown codes).
   * Derived from the wire-level `reason` integer.
   */
  reasonName: string
}

/** FPSS auto-reconnect stopped without a user-initiated shutdown — terminal for the session. Emitted when the reconnect budget (attempt count or wall-clock envelope) is exhausted, a permanent disconnect reason short-circuits recovery, a manual policy declines to reconnect, or a custom policy returns no delay. `reason` is the integer disconnect code of the final drop; read the resolved reason-name field for the symbolic name. `attempts` is the number of consecutive reconnect attempts consumed before giving up (0 when no reconnect was attempted). */
export interface ReconnectsExhausted {
  reason: number
  attempts: number
  /**
   * Resolved disconnect-reason name (e.g. `"TooManyRequests"`,
   * `"InvalidCredentials"`, `"Unspecified"` for unknown codes).
   * Derived from the wire-level `reason` integer.
   */
  reasonName: string
}

/** FPSS subscription response (wire code 40). `result` is an integer status code (0=Subscribed, 1=Error, 2=MaxStreamsReached, 3=InvalidPerms). */
export interface ReqResponse {
  reqId: number
  result: number
}

/** Wire string enum `RequestType`. */
export declare const enum RequestType {
  Trade = 'trade',
  Quote = 'quote',
  Eod = 'eod',
  Ohlc = 'ohlc'
}

/** FPSS server stream restart (wire code 31). The server restarts the stream without dropping the TCP connection; delta decode state should be cleared on receipt. */
export interface Restart {

}

/** Wire string enum `Right`. */
export declare const enum Right {
  Call = 'call',
  Put = 'put',
  Both = 'both'
}

/** FPSS server-error message (wire code 11). */
export interface ServerError {
  message: string
}

/**
 * Optional parameters for the `stockAtTimeQuote` method. Keys are
 * the camelCase parameter names; absent keys behave exactly like an
 * omitted parameter. `timeoutMs` bounds the whole call: on expiry the
 * returned Promise rejects and the underlying request is cancelled.
 */
export interface StockAtTimeQuoteOptions {
  /** Venue/exchange filter. Accepted values: `nqb`, `utp_cta`. */
  venue?: string
  /**
   * Per-call deadline as a non-negative whole number of milliseconds;
   * on expiry the returned Promise rejects and the underlying request
   * is cancelled. A non-finite, negative, or fractional value is
   * rejected with `InvalidParameterError` rather than coerced.
   */
  timeoutMs?: number
}

/**
 * Optional parameters for the `stockAtTimeTrade` method. Keys are
 * the camelCase parameter names; absent keys behave exactly like an
 * omitted parameter. `timeoutMs` bounds the whole call: on expiry the
 * returned Promise rejects and the underlying request is cancelled.
 */
export interface StockAtTimeTradeOptions {
  /** Venue/exchange filter. Accepted values: `nqb`, `utp_cta`. */
  venue?: string
  /**
   * Per-call deadline as a non-negative whole number of milliseconds;
   * on expiry the returned Promise rejects and the underlying request
   * is cancelled. A non-finite, negative, or fractional value is
   * rejected with `InvalidParameterError` rather than coerced.
   */
  timeoutMs?: number
}

/**
 * Optional parameters for the `stockHistoryEOD` method. Keys are
 * the camelCase parameter names; absent keys behave exactly like an
 * omitted parameter. `timeoutMs` bounds the whole call: on expiry the
 * returned Promise rejects and the underlying request is cancelled.
 */
export interface StockHistoryEodOptions {
  /**
   * Per-call deadline as a non-negative whole number of milliseconds;
   * on expiry the returned Promise rejects and the underlying request
   * is cancelled. A non-finite, negative, or fractional value is
   * rejected with `InvalidParameterError` rather than coerced.
   */
  timeoutMs?: number
}

/**
 * Optional parameters for the `stockHistoryOHLC` method. Keys are
 * the camelCase parameter names; absent keys behave exactly like an
 * omitted parameter. `timeoutMs` bounds the whole call: on expiry the
 * returned Promise rejects and the underlying request is cancelled.
 */
export interface StockHistoryOhlcOptions {
  /** Interval preset or millisecond string. Defaults to `1s` when omitted — matching the upstream ThetaData Python library. Accepted values: `tick`, `10ms`, `100ms`, `500ms`, `1s`, `5s`, `10s`, `15s`, `30s`, `1m`, `5m`, `10m`, `15m`, `30m`, `1h`. */
  interval?: string
  /** Start time filter */
  startTime?: string | Date
  /** End time filter */
  endTime?: string | Date
  /** Venue/exchange filter. Accepted values: `nqb`, `utp_cta`. */
  venue?: string
  /** Start date YYYYMMDD */
  startDate?: string | Date
  /** End date YYYYMMDD */
  endDate?: string | Date
  /**
   * Per-call deadline as a non-negative whole number of milliseconds;
   * on expiry the returned Promise rejects and the underlying request
   * is cancelled. A non-finite, negative, or fractional value is
   * rejected with `InvalidParameterError` rather than coerced.
   */
  timeoutMs?: number
}

/**
 * Optional parameters for the `stockHistoryOHLCRange` method. Keys are
 * the camelCase parameter names; absent keys behave exactly like an
 * omitted parameter. `timeoutMs` bounds the whole call: on expiry the
 * returned Promise rejects and the underlying request is cancelled.
 */
export interface StockHistoryOhlcRangeOptions {
  /** Interval preset or millisecond string. Defaults to `1s` when omitted — matching the upstream ThetaData Python library. Accepted values: `tick`, `10ms`, `100ms`, `500ms`, `1s`, `5s`, `10s`, `15s`, `30s`, `1m`, `5m`, `10m`, `15m`, `30m`, `1h`. */
  interval?: string
  /** Start time filter */
  startTime?: string | Date
  /** End time filter */
  endTime?: string | Date
  /** Venue/exchange filter. Accepted values: `nqb`, `utp_cta`. */
  venue?: string
  /**
   * Per-call deadline as a non-negative whole number of milliseconds;
   * on expiry the returned Promise rejects and the underlying request
   * is cancelled. A non-finite, negative, or fractional value is
   * rejected with `InvalidParameterError` rather than coerced.
   */
  timeoutMs?: number
}

/**
 * Optional parameters for the `stockHistoryQuote` method. Keys are
 * the camelCase parameter names; absent keys behave exactly like an
 * omitted parameter. `timeoutMs` bounds the whole call: on expiry the
 * returned Promise rejects and the underlying request is cancelled.
 */
export interface StockHistoryQuoteOptions {
  /** Interval preset or millisecond string. Defaults to `1s` when omitted — matching the upstream ThetaData Python library. Accepted values: `tick`, `10ms`, `100ms`, `500ms`, `1s`, `5s`, `10s`, `15s`, `30s`, `1m`, `5m`, `10m`, `15m`, `30m`, `1h`. */
  interval?: string
  /** Start time filter */
  startTime?: string | Date
  /** End time filter */
  endTime?: string | Date
  /** Venue/exchange filter. Accepted values: `nqb`, `utp_cta`. */
  venue?: string
  /** Start date YYYYMMDD */
  startDate?: string | Date
  /** End date YYYYMMDD */
  endDate?: string | Date
  /**
   * Per-call deadline as a non-negative whole number of milliseconds;
   * on expiry the returned Promise rejects and the underlying request
   * is cancelled. A non-finite, negative, or fractional value is
   * rejected with `InvalidParameterError` rather than coerced.
   */
  timeoutMs?: number
}

/**
 * Optional parameters for the `stockHistoryTrade` method. Keys are
 * the camelCase parameter names; absent keys behave exactly like an
 * omitted parameter. `timeoutMs` bounds the whole call: on expiry the
 * returned Promise rejects and the underlying request is cancelled.
 */
export interface StockHistoryTradeOptions {
  /** Start time filter */
  startTime?: string | Date
  /** End time filter */
  endTime?: string | Date
  /** Venue/exchange filter. Accepted values: `nqb`, `utp_cta`. */
  venue?: string
  /** Start date YYYYMMDD */
  startDate?: string | Date
  /** End date YYYYMMDD */
  endDate?: string | Date
  /**
   * Per-call deadline as a non-negative whole number of milliseconds;
   * on expiry the returned Promise rejects and the underlying request
   * is cancelled. A non-finite, negative, or fractional value is
   * rejected with `InvalidParameterError` rather than coerced.
   */
  timeoutMs?: number
}

/**
 * Optional parameters for the `stockHistoryTradeQuote` method. Keys are
 * the camelCase parameter names; absent keys behave exactly like an
 * omitted parameter. `timeoutMs` bounds the whole call: on expiry the
 * returned Promise rejects and the underlying request is cancelled.
 */
export interface StockHistoryTradeQuoteOptions {
  /** Start time filter */
  startTime?: string | Date
  /** End time filter */
  endTime?: string | Date
  /** When true, quotes whose timestamp equals the trade timestamp are excluded; only quotes strictly before the trade are paired. */
  exclusive?: boolean
  /** Venue/exchange filter. Accepted values: `nqb`, `utp_cta`. */
  venue?: string
  /** Start date YYYYMMDD */
  startDate?: string | Date
  /** End date YYYYMMDD */
  endDate?: string | Date
  /**
   * Per-call deadline as a non-negative whole number of milliseconds;
   * on expiry the returned Promise rejects and the underlying request
   * is cancelled. A non-finite, negative, or fractional value is
   * rejected with `InvalidParameterError` rather than coerced.
   */
  timeoutMs?: number
}

/**
 * Optional parameters for the `stockListDates` method. Keys are
 * the camelCase parameter names; absent keys behave exactly like an
 * omitted parameter. `timeoutMs` bounds the whole call: on expiry the
 * returned Promise rejects and the underlying request is cancelled.
 */
export interface StockListDatesOptions {
  /**
   * Per-call deadline as a non-negative whole number of milliseconds;
   * on expiry the returned Promise rejects and the underlying request
   * is cancelled. A non-finite, negative, or fractional value is
   * rejected with `InvalidParameterError` rather than coerced.
   */
  timeoutMs?: number
}

/**
 * Optional parameters for the `stockListSymbols` method. Keys are
 * the camelCase parameter names; absent keys behave exactly like an
 * omitted parameter. `timeoutMs` bounds the whole call: on expiry the
 * returned Promise rejects and the underlying request is cancelled.
 */
export interface StockListSymbolsOptions {
  /**
   * Per-call deadline as a non-negative whole number of milliseconds;
   * on expiry the returned Promise rejects and the underlying request
   * is cancelled. A non-finite, negative, or fractional value is
   * rejected with `InvalidParameterError` rather than coerced.
   */
  timeoutMs?: number
}

/**
 * Optional parameters for the `stockSnapshotMarketValue` method. Keys are
 * the camelCase parameter names; absent keys behave exactly like an
 * omitted parameter. `timeoutMs` bounds the whole call: on expiry the
 * returned Promise rejects and the underlying request is cancelled.
 */
export interface StockSnapshotMarketValueOptions {
  /** Venue/exchange filter. Accepted values: `nqb`, `utp_cta`. */
  venue?: string
  /** Minimum time filter */
  minTime?: string | Date
  /**
   * Per-call deadline as a non-negative whole number of milliseconds;
   * on expiry the returned Promise rejects and the underlying request
   * is cancelled. A non-finite, negative, or fractional value is
   * rejected with `InvalidParameterError` rather than coerced.
   */
  timeoutMs?: number
}

/**
 * Optional parameters for the `stockSnapshotOHLC` method. Keys are
 * the camelCase parameter names; absent keys behave exactly like an
 * omitted parameter. `timeoutMs` bounds the whole call: on expiry the
 * returned Promise rejects and the underlying request is cancelled.
 */
export interface StockSnapshotOhlcOptions {
  /** Venue/exchange filter. Accepted values: `nqb`, `utp_cta`. */
  venue?: string
  /** Minimum time filter */
  minTime?: string | Date
  /**
   * Per-call deadline as a non-negative whole number of milliseconds;
   * on expiry the returned Promise rejects and the underlying request
   * is cancelled. A non-finite, negative, or fractional value is
   * rejected with `InvalidParameterError` rather than coerced.
   */
  timeoutMs?: number
}

/**
 * Optional parameters for the `stockSnapshotQuote` method. Keys are
 * the camelCase parameter names; absent keys behave exactly like an
 * omitted parameter. `timeoutMs` bounds the whole call: on expiry the
 * returned Promise rejects and the underlying request is cancelled.
 */
export interface StockSnapshotQuoteOptions {
  /** Venue/exchange filter. Accepted values: `nqb`, `utp_cta`. */
  venue?: string
  /** Minimum time filter */
  minTime?: string | Date
  /**
   * Per-call deadline as a non-negative whole number of milliseconds;
   * on expiry the returned Promise rejects and the underlying request
   * is cancelled. A non-finite, negative, or fractional value is
   * rejected with `InvalidParameterError` rather than coerced.
   */
  timeoutMs?: number
}

/**
 * Optional parameters for the `stockSnapshotTrade` method. Keys are
 * the camelCase parameter names; absent keys behave exactly like an
 * omitted parameter. `timeoutMs` bounds the whole call: on expiry the
 * returned Promise rejects and the underlying request is cancelled.
 */
export interface StockSnapshotTradeOptions {
  /** Venue/exchange filter. Accepted values: `nqb`, `utp_cta`. */
  venue?: string
  /** Minimum time filter */
  minTime?: string | Date
  /**
   * Per-call deadline as a non-negative whole number of milliseconds;
   * on expiry the returned Promise rejects and the underlying request
   * is cancelled. A non-finite, negative, or fractional value is
   * rejected with `InvalidParameterError` rather than coerced.
   */
  timeoutMs?: number
}

/**
 * A single FPSS event surfaced to JS/TS.
 *
 * `kind` is the discriminator — switch on it and read the matching
 * payload field. The shape is stable and every payload is typed, so
 * consumers never fall back to untyped `any`.
 */
export interface StreamEvent {
  /**
   * Discriminator matching one of the typed payload fields below.
   * Narrowed to a literal union in TS so `switch (event.kind)`
   * correctly narrows the optional payload fields.
   */
  kind: 'connected' | 'contract_assigned' | 'disconnected' | 'login_success' | 'market_close' | 'market_open' | 'market_value' | 'ohlcvc' | 'open_interest' | 'parse_error' | 'ping' | 'quote' | 'reconnected' | 'reconnected_server' | 'reconnecting' | 'reconnects_exhausted' | 'req_response' | 'restart' | 'server_error' | 'trade' | 'unknown_control' | 'unknown_frame'
  marketValue?: MarketValue
  ohlcvc?: Ohlcvc
  openInterest?: OpenInterest
  quote?: Quote
  trade?: Trade
  connected?: Connected
  contractAssigned?: ContractAssigned
  disconnected?: Disconnected
  loginSuccess?: LoginSuccess
  marketClose?: MarketClose
  marketOpen?: MarketOpen
  parseError?: ParseError
  ping?: Ping
  reconnected?: Reconnected
  reconnectedServer?: ReconnectedServer
  reconnecting?: Reconnecting
  reconnectsExhausted?: ReconnectsExhausted
  reqResponse?: ReqResponse
  restart?: Restart
  serverError?: ServerError
  unknownControl?: UnknownControl
  unknownFrame?: UnknownFrame
}

/** FPSS Trade tick. */
export interface Trade {
  contract: Contract
  msOfDay: number
  sequence: number
  extCondition1: number
  extCondition2: number
  extCondition3: number
  extCondition4: number
  condition: number
  size: number
  exchange: number
  price: number
  conditionFlags: number
  priceFlags: number
  volumeType: number
  recordsBack: number
  date: number
  receivedAtNs: bigint
}

/** Per-trade union Greeks tick -- every Greek the v3 server publishes on */
export interface TradeGreeksAllTick {
  msOfDay: number
  sequence: number
  extCondition1: number
  extCondition2: number
  extCondition3: number
  extCondition4: number
  condition: number
  size: number
  exchange: number
  price: number
  delta: number
  theta: number
  vega: number
  rho: number
  epsilon: number
  lambda: number
  gamma: number
  vanna: number
  charm: number
  vomma: number
  veta: number
  vera: number
  speed: number
  zomma: number
  color: number
  ultima: number
  d1: number
  d2: number
  dualDelta: number
  dualGamma: number
  impliedVolatility: number
  ivError: number
  underlyingMsOfDay: number
  underlyingPrice: number
  date: number
  expiration?: number
  strike?: number
  right?: string
  /**
   * Unix epoch milliseconds (UTC, DST-aware) combining `date` with
   * `ms_of_day` (Eastern-Time milliseconds-of-day). `undefined` when
   * `date` is absent (`0`).
   */
  timestampMs?: bigint
  /**
   * Unix epoch milliseconds (UTC, DST-aware) combining `date` with
   * `underlying_ms_of_day` (Eastern-Time milliseconds-of-day). `undefined` when
   * `date` is absent (`0`).
   */
  underlyingTimestampMs?: bigint
}

/**
 * Serialise a `TradeGreeksAllTick` history result to an Arrow IPC stream
 * (the `apache-arrow` wire form). Mirrors the FlatFiles
 * `FlatFileRowList.toArrowIpc()` exit and the Python
 * `<TickName>List.to_arrow()` terminal so every binding can reach a
 * dataframe from an in-band history result.
 */
export declare function tradeGreeksAllTickToArrowIpc(rows: Array<TradeGreeksAllTick>): Buffer

/** Per-trade first-order Greeks tick (delta / theta / vega / rho / epsilon */
export interface TradeGreeksFirstOrderTick {
  msOfDay: number
  sequence: number
  extCondition1: number
  extCondition2: number
  extCondition3: number
  extCondition4: number
  condition: number
  size: number
  exchange: number
  price: number
  delta: number
  theta: number
  vega: number
  rho: number
  epsilon: number
  lambda: number
  impliedVolatility: number
  ivError: number
  underlyingMsOfDay: number
  underlyingPrice: number
  date: number
  expiration?: number
  strike?: number
  right?: string
  /**
   * Unix epoch milliseconds (UTC, DST-aware) combining `date` with
   * `ms_of_day` (Eastern-Time milliseconds-of-day). `undefined` when
   * `date` is absent (`0`).
   */
  timestampMs?: bigint
  /**
   * Unix epoch milliseconds (UTC, DST-aware) combining `date` with
   * `underlying_ms_of_day` (Eastern-Time milliseconds-of-day). `undefined` when
   * `date` is absent (`0`).
   */
  underlyingTimestampMs?: bigint
}

/**
 * Serialise a `TradeGreeksFirstOrderTick` history result to an Arrow IPC stream
 * (the `apache-arrow` wire form). Mirrors the FlatFiles
 * `FlatFileRowList.toArrowIpc()` exit and the Python
 * `<TickName>List.to_arrow()` terminal so every binding can reach a
 * dataframe from an in-band history result.
 */
export declare function tradeGreeksFirstOrderTickToArrowIpc(rows: Array<TradeGreeksFirstOrderTick>): Buffer

/** Per-trade implied-volatility tick (single `implied_volatility` + */
export interface TradeGreeksImpliedVolatilityTick {
  msOfDay: number
  sequence: number
  extCondition1: number
  extCondition2: number
  extCondition3: number
  extCondition4: number
  condition: number
  size: number
  exchange: number
  price: number
  impliedVolatility: number
  ivError: number
  underlyingMsOfDay: number
  underlyingPrice: number
  date: number
  expiration?: number
  strike?: number
  right?: string
  /**
   * Unix epoch milliseconds (UTC, DST-aware) combining `date` with
   * `ms_of_day` (Eastern-Time milliseconds-of-day). `undefined` when
   * `date` is absent (`0`).
   */
  timestampMs?: bigint
  /**
   * Unix epoch milliseconds (UTC, DST-aware) combining `date` with
   * `underlying_ms_of_day` (Eastern-Time milliseconds-of-day). `undefined` when
   * `date` is absent (`0`).
   */
  underlyingTimestampMs?: bigint
}

/**
 * Serialise a `TradeGreeksImpliedVolatilityTick` history result to an Arrow IPC stream
 * (the `apache-arrow` wire form). Mirrors the FlatFiles
 * `FlatFileRowList.toArrowIpc()` exit and the Python
 * `<TickName>List.to_arrow()` terminal so every binding can reach a
 * dataframe from an in-band history result.
 */
export declare function tradeGreeksImpliedVolatilityTickToArrowIpc(rows: Array<TradeGreeksImpliedVolatilityTick>): Buffer

/** Per-trade second-order Greeks tick (gamma / vanna / charm / vomma / */
export interface TradeGreeksSecondOrderTick {
  msOfDay: number
  sequence: number
  extCondition1: number
  extCondition2: number
  extCondition3: number
  extCondition4: number
  condition: number
  size: number
  exchange: number
  price: number
  gamma: number
  vanna: number
  charm: number
  vomma: number
  veta: number
  impliedVolatility: number
  ivError: number
  underlyingMsOfDay: number
  underlyingPrice: number
  date: number
  expiration?: number
  strike?: number
  right?: string
  /**
   * Unix epoch milliseconds (UTC, DST-aware) combining `date` with
   * `ms_of_day` (Eastern-Time milliseconds-of-day). `undefined` when
   * `date` is absent (`0`).
   */
  timestampMs?: bigint
  /**
   * Unix epoch milliseconds (UTC, DST-aware) combining `date` with
   * `underlying_ms_of_day` (Eastern-Time milliseconds-of-day). `undefined` when
   * `date` is absent (`0`).
   */
  underlyingTimestampMs?: bigint
}

/**
 * Serialise a `TradeGreeksSecondOrderTick` history result to an Arrow IPC stream
 * (the `apache-arrow` wire form). Mirrors the FlatFiles
 * `FlatFileRowList.toArrowIpc()` exit and the Python
 * `<TickName>List.to_arrow()` terminal so every binding can reach a
 * dataframe from an in-band history result.
 */
export declare function tradeGreeksSecondOrderTickToArrowIpc(rows: Array<TradeGreeksSecondOrderTick>): Buffer

/** Per-trade third-order Greeks tick (speed / zomma / color / ultima) */
export interface TradeGreeksThirdOrderTick {
  msOfDay: number
  sequence: number
  extCondition1: number
  extCondition2: number
  extCondition3: number
  extCondition4: number
  condition: number
  size: number
  exchange: number
  price: number
  speed: number
  zomma: number
  color: number
  ultima: number
  impliedVolatility: number
  ivError: number
  underlyingMsOfDay: number
  underlyingPrice: number
  date: number
  expiration?: number
  strike?: number
  right?: string
  /**
   * Unix epoch milliseconds (UTC, DST-aware) combining `date` with
   * `ms_of_day` (Eastern-Time milliseconds-of-day). `undefined` when
   * `date` is absent (`0`).
   */
  timestampMs?: bigint
  /**
   * Unix epoch milliseconds (UTC, DST-aware) combining `date` with
   * `underlying_ms_of_day` (Eastern-Time milliseconds-of-day). `undefined` when
   * `date` is absent (`0`).
   */
  underlyingTimestampMs?: bigint
}

/**
 * Serialise a `TradeGreeksThirdOrderTick` history result to an Arrow IPC stream
 * (the `apache-arrow` wire form). Mirrors the FlatFiles
 * `FlatFileRowList.toArrowIpc()` exit and the Python
 * `<TickName>List.to_arrow()` terminal so every binding can reach a
 * dataframe from an in-band history result.
 */
export declare function tradeGreeksThirdOrderTickToArrowIpc(rows: Array<TradeGreeksThirdOrderTick>): Buffer

/** Combined trade + quote tick. */
export interface TradeQuoteTick {
  msOfDay: number
  sequence: number
  extCondition1: number
  extCondition2: number
  extCondition3: number
  extCondition4: number
  condition: number
  size: number
  exchange: number
  price: number
  conditionFlags: number
  priceFlags: number
  volumeType: number
  recordsBack: number
  quoteMsOfDay: number
  bidSize: number
  bidExchange: number
  bid: number
  bidCondition: number
  askSize: number
  askExchange: number
  ask: number
  askCondition: number
  date: number
  expiration?: number
  strike?: number
  right?: string
  /**
   * Unix epoch milliseconds (UTC, DST-aware) combining `date` with
   * `ms_of_day` (Eastern-Time milliseconds-of-day). `undefined` when
   * `date` is absent (`0`).
   */
  timestampMs?: bigint
  /**
   * Unix epoch milliseconds (UTC, DST-aware) combining `date` with
   * `quote_ms_of_day` (Eastern-Time milliseconds-of-day). `undefined` when
   * `date` is absent (`0`).
   */
  quoteTimestampMs?: bigint
}

/**
 * Serialise a `TradeQuoteTick` history result to an Arrow IPC stream
 * (the `apache-arrow` wire form). Mirrors the FlatFiles
 * `FlatFileRowList.toArrowIpc()` exit and the Python
 * `<TickName>List.to_arrow()` terminal so every binding can reach a
 * dataframe from an in-band history result.
 */
export declare function tradeQuoteTickToArrowIpc(rows: Array<TradeQuoteTick>): Buffer

/** Trade tick. Core unit of trade data. */
export interface TradeTick {
  msOfDay: number
  sequence: number
  extCondition1: number
  extCondition2: number
  extCondition3: number
  extCondition4: number
  condition: number
  size: number
  exchange: number
  price: number
  conditionFlags: number
  priceFlags: number
  volumeType: number
  recordsBack: number
  date: number
  expiration?: number
  strike?: number
  right?: string
  /** True when the trade carries a cancelled-trade condition (codes 40-44). */
  isCancelled: boolean
  /** True when the trade condition flags set the 'no last' bit (this trade must not update the last price). */
  tradeConditionNoLast: boolean
  /** True when the price flags set the 'set last' bit (this trade sets the last price). */
  priceConditionSetLast: boolean
  /** True when volume is reported incrementally (each trade adds to the daily total) rather than cumulatively. */
  isIncrementalVolume: boolean
  /** True when the trade occurred during regular trading hours (9:30 AM - 4:00 PM ET). */
  regularTradingHours: boolean
  /** True when the trade is seller-initiated (ext_condition1 == 12). */
  isSeller: boolean
  /**
   * Unix epoch milliseconds (UTC, DST-aware) combining `date` with
   * `ms_of_day` (Eastern-Time milliseconds-of-day). `undefined` when
   * `date` is absent (`0`).
   */
  timestampMs?: bigint
}

/**
 * Serialise a `TradeTick` history result to an Arrow IPC stream
 * (the `apache-arrow` wire form). Mirrors the FlatFiles
 * `FlatFileRowList.toArrowIpc()` exit and the Python
 * `<TickName>List.to_arrow()` terminal so every binding can reach a
 * dataframe from an in-band history result.
 */
export declare function tradeTickToArrowIpc(rows: Array<TradeTick>): Buffer

/** FPSS control variant the SDK does not yet recognise. Surfaced when a newer protocol revision adds a control event this build predates — keep dispatch logic forward-compatible by handling this variant. Carries no payload. */
export interface UnknownControl {

}

/** FPSS server sent a frame with an unrecognised wire code. Raw bytes preserved for diagnostics / upstream bug reports. */
export interface UnknownFrame {
  code: number
  payload: Array<number>
}

/** Wire string enum `Venue`. */
export declare const enum Venue {
  Nqb = 'nqb',
  UtpCta = 'utp_cta'
}

/** Wire string enum `Version`. */
export declare const enum Version {
  Latest = 'latest',
  V1 = '1'
}

/**
 * `(hasValue, n)` shape for the worker-threads setting. `hasValue=false`
 * encodes the unset sentinel; `hasValue=true` carries the explicit
 * worker count (with `n=0` preserved verbatim).
 */
export interface WorkerThreadsSetting {
  hasValue: boolean
  n: number
}

// `Contract` is the public name for the fluent contract builder; it
// is an alias for the `ContractRef` class, so the two are
// interchangeable.
export const Contract: typeof ContractRef;
