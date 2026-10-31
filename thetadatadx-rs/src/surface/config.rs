//! The config accessor table.
//!
//! Every `thetadatadx_config_{set,get}_*` accessor that reads or writes one
//! `DirectConfig` value is written once, in `__config!`. The C ABI expands it
//! into its entry points, Python into the `Config` properties, TypeScript
//! into the `Config` setters and getters, and `generate_sdk_surfaces` into the
//! C header's declarations, the C++ `Config` methods and the Python type
//! stub's `Config` properties. Accessors with their own logic (the ring size, the
//! worker threads, the consumer CPU, the reconnect policy and callback) are
//! written by hand in each binding.
//!
//! A consumer passes the name of one of its own macros and receives every
//! entry, in table order, after the tokens it passed:
//!
//! ```ignore
//! thetadatadx::__config! { my_consumer { leading tokens passed through } }
//! // expands to
//! my_consumer! { leading tokens passed through  <entry> <entry> ... }
//! ```
//!
//! # Entries
//!
//! ```ignore
//! set thetadatadx_config_set_retry_max_elapsed_secs(secs: u64) [retry.max_elapsed in secs] {
//!     param: secs,
//!     default: 300,
//!     doc: ["Set the wall-clock envelope (seconds) for one retry sequence."],
//! }
//! ```
//!
//! An entry is five token trees: `set` or `get`, the C symbol, the C
//! parameter and the value's type, the target in brackets, and the body.
//!
//! - The C parameter of a getter is its out-parameter; a `String` getter
//!   returns the string instead and leaves it unused. The type is `u64`,
//!   `u32`, `u16`, `usize`, `bool`, `String` or `Option<u32>`, or `i32` for
//!   an enum's code.
//! - The target is a field path (`[reconnect.wait_ms]`), a `Duration` field
//!   read and written in a unit (`[retry.initial_delay in ms]`), a field of
//!   the attempt limits the `Auto` reconnect policy carries
//!   (`[limits.max_attempts]`; it reads the `Auto` default under another
//!   policy, and a write under another policy changes nothing), or a method
//!   (`[market_data_host()]`).
//! - `param` is the Python and TypeScript name of a setter's argument.
//! - `default` is the value `DirectConfig::production_defaults` starts from,
//!   for a number, a flag or an enum (as its variant), optionally followed by
//!   the same value in other units (`default: 1000 "1 ms"`).
//! - `values` lists an enum accessor's codes and variants, with what each
//!   variant does on a setter, and `errors` an enum setter's rejections: the
//!   C error code for a null handle and the text each binding reports for a
//!   value outside the enum.
//! - `doc` describes the accessor, one string per paragraph with `""` between
//!   paragraphs, naming other settings by their field path.
//!
//! Each binding follows `doc` with what the table holds as data, in its own
//! spelling: the default (a `bigint` with its `n` in TypeScript, `True` in
//! Python, an enum by its lowercase name in Python and TypeScript), an enum's
//! values, and how its accessor treats a null handle, an unset value or a
//! rejected one. The Python stub writes code spans in reST.

/// Reads the value an entry's target names.
///
/// `__config_read!(receiver [target])` expands to the value the getter
/// returns: a unit's whole count for a `Duration` field (milliseconds
/// saturate at `u64::MAX`), and the method's return value for a method.
#[doc(hidden)]
#[macro_export]
macro_rules! __config_read {
    ($recv:ident $(.$r:ident)* [limits . $f:ident in secs]) => {
        match &$recv $(.$r)*.reconnect.policy {
            $crate::ReconnectPolicy::Auto(limits) => limits.$f.as_secs(),
            _ => $crate::ReconnectAttemptLimits::default().$f.as_secs(),
        }
    };
    ($recv:ident $(.$r:ident)* [limits . $f:ident]) => {
        match &$recv $(.$r)*.reconnect.policy {
            $crate::ReconnectPolicy::Auto(limits) => limits.$f,
            _ => $crate::ReconnectAttemptLimits::default().$f,
        }
    };
    ($recv:ident $(.$r:ident)* [$m:ident ()]) => {
        $recv $(.$r)*.$m()
    };
    ($recv:ident $(.$r:ident)* [$($p:ident).+ in ms]) => {
        u64::try_from($recv $(.$r)* $(.$p)+.as_millis()).unwrap_or(u64::MAX)
    };
    ($recv:ident $(.$r:ident)* [$($p:ident).+ in secs]) => {
        $recv $(.$r)* $(.$p)+.as_secs()
    };
    ($recv:ident $(.$r:ident)* [$($p:ident).+]) => {
        $recv $(.$r)* $(.$p)+
    };
}

/// Writes a setter's value to the target an entry names.
///
/// `__config_write!(receiver [target] = value)` converts a unit count to a
/// `Duration`, passes the value to a method, and writes an attempt limit only
/// while the reconnect policy is `Auto`.
#[doc(hidden)]
#[macro_export]
macro_rules! __config_write {
    ($recv:ident $(.$r:ident)* [limits . $f:ident $(in $unit:ident)?] = $v:expr) => {
        if let $crate::ReconnectPolicy::Auto(ref mut limits) = $recv $(.$r)*.reconnect.policy {
            limits.$f = $crate::__config_write!(@value $v $(, $unit)?);
        }
    };
    ($recv:ident $(.$r:ident)* [$m:ident ()] = $v:expr) => {
        $recv $(.$r)*.$m($v)
    };
    ($recv:ident $(.$r:ident)* [$($p:ident).+ $(in $unit:ident)?] = $v:expr) => {
        $recv $(.$r)* $(.$p)+ = $crate::__config_write!(@value $v $(, $unit)?)
    };
    (@value $v:expr) => {
        $v
    };
    (@value $v:expr, ms) => {
        ::std::time::Duration::from_millis($v)
    };
    (@value $v:expr, secs) => {
        ::std::time::Duration::from_secs($v)
    };
}

/// Every config accessor, in the order each binding declares them.
#[doc(hidden)]
#[macro_export]
macro_rules! __config {
    ($m:ident { $($ctx:tt)* }) => {
        $m! {
            $($ctx)*
            set thetadatadx_config_set_reconnect_wait_ms(ms: u64) [reconnect.wait_ms] {
                param: ms,
                default: 250,
                doc: ["Set the reconnect delay (ms) honoured for generic transient disconnects (TimedOut, ServerRestarting, Unspecified, …), applied to the streaming session at connect time."],
            }
            get thetadatadx_config_get_reconnect_wait_ms(out_ms: u64) [reconnect.wait_ms] {
                default: 250,
                doc: ["Current `reconnect.wait_ms` value."],
            }
            set thetadatadx_config_set_reconnect_wait_rate_limited_ms(ms: u64) [reconnect.wait_rate_limited_ms] {
                param: ms,
                default: 130_000,
                doc: ["Set the reconnect delay (ms) honoured for `TooManyRequests` rate-limited disconnects, applied to the streaming session at connect time. The default matches the JVM terminal's 130 s rate-limit cooldown."],
            }
            get thetadatadx_config_get_reconnect_wait_rate_limited_ms(out_ms: u64) [reconnect.wait_rate_limited_ms] {
                default: 130_000,
                doc: ["Current `reconnect.wait_rate_limited_ms` value."],
            }
            set thetadatadx_config_set_reconnect_wait_max_ms(v: u64) [reconnect.wait_max_ms] {
                param: ms,
                default: 30_000,
                doc: ["Set the cap (ms) on the exponential generic-transient reconnect ladder. The ladder starts at `reconnect.wait_ms` and doubles per consecutive attempt up to this value."],
            }
            get thetadatadx_config_get_reconnect_wait_max_ms(out: u64) [reconnect.wait_max_ms] {
                default: 30_000,
                doc: ["Current `reconnect.wait_max_ms` value."],
            }
            set thetadatadx_config_set_reconnect_wait_server_restart_ms(v: u64) [reconnect.wait_server_restart_ms] {
                param: ms,
                default: 5_000,
                doc: ["Set the flat reconnect cadence (ms) for `ServerRestarting` disconnects."],
            }
            get thetadatadx_config_get_reconnect_wait_server_restart_ms(out: u64) [reconnect.wait_server_restart_ms] {
                default: 5_000,
                doc: ["Current `reconnect.wait_server_restart_ms` value."],
            }
            set thetadatadx_config_set_reconnect_replay_burst_size(n: u32) [reconnect.replay_burst_size] {
                param: n,
                default: 50,
                doc: ["Set the subscription-replay burst size used after an auto-reconnect: frames are written in bursts of this many, each burst flushed and followed by a jittered `reconnect.replay_pace_ms` pause. Minimum `1`, validated at connect."],
            }
            get thetadatadx_config_get_reconnect_replay_burst_size(out: u32) [reconnect.replay_burst_size] {
                default: 50,
                doc: ["Current `reconnect.replay_burst_size` value."],
            }
            set thetadatadx_config_set_reconnect_replay_pace_ms(v: u64) [reconnect.replay_pace_ms] {
                param: ms,
                default: 5,
                doc: ["Set the pause (ms) between subscription-replay bursts after an auto-reconnect. Zero removes the pause."],
            }
            get thetadatadx_config_get_reconnect_replay_pace_ms(out: u64) [reconnect.replay_pace_ms] {
                default: 5,
                doc: ["Current `reconnect.replay_pace_ms` value."],
            }
            set thetadatadx_config_set_streaming_timeout_ms(v: u64) [streaming.timeout_ms] {
                param: ms,
                default: 10_000,
                doc: ["Set the streaming read timeout (ms): the no-frames deadline after which the streaming session is declared dead and reconnects. Validated to `[100, 60_000]` at connect."],
            }
            get thetadatadx_config_get_streaming_timeout_ms(out: u64) [streaming.timeout_ms] {
                default: 10_000,
                doc: ["Current `streaming.timeout_ms` value."],
            }
            set thetadatadx_config_set_streaming_connect_timeout_ms(v: u64) [streaming.connect_timeout_ms] {
                param: ms,
                default: 2_000,
                doc: ["Set the per-server TCP connect timeout (ms) for the streaming connection. Validated to `[1_000, 60_000]` at connect."],
            }
            get thetadatadx_config_get_streaming_connect_timeout_ms(out: u64) [streaming.connect_timeout_ms] {
                default: 2_000,
                doc: ["Current `streaming.connect_timeout_ms` value."],
            }
            set thetadatadx_config_set_streaming_ping_interval_ms(v: u64) [streaming.ping_interval_ms] {
                param: ms,
                default: 100,
                doc: ["Set the streaming heartbeat ping interval (ms). Validated to `[100, 300_000]` at connect."],
            }
            get thetadatadx_config_get_streaming_ping_interval_ms(out: u64) [streaming.ping_interval_ms] {
                default: 100,
                doc: ["Current `streaming.ping_interval_ms` value."],
            }
            set thetadatadx_config_set_streaming_io_read_slice_ms(v: u64) [streaming.io_read_slice_ms] {
                param: ms,
                default: 25,
                doc: ["Set the per-iteration blocking-read slice (ms) for the streaming I/O loop. Shorter slices service outbound commands more promptly at slightly higher idle CPU. Validated to `[10, 500]` at connect."],
            }
            get thetadatadx_config_get_streaming_io_read_slice_ms(out: u64) [streaming.io_read_slice_ms] {
                default: 25,
                doc: ["Current `streaming.io_read_slice_ms` value."],
            }
            set thetadatadx_config_set_streaming_keepalive_idle_secs(v: u64) [streaming.keepalive_idle_secs] {
                param: secs,
                default: 5,
                doc: ["Set the TCP keepalive idle time (seconds) before the kernel sends the first probe on a silent streaming socket. Validated to `[1, 7_200]` at connect."],
            }
            get thetadatadx_config_get_streaming_keepalive_idle_secs(out: u64) [streaming.keepalive_idle_secs] {
                default: 5,
                doc: ["Current `streaming.keepalive_idle_secs` value."],
            }
            set thetadatadx_config_set_streaming_keepalive_interval_secs(v: u64) [streaming.keepalive_interval_secs] {
                param: secs,
                default: 2,
                doc: ["Set the interval (seconds) between TCP keepalive probes. Validated to `[1, 75]` at connect."],
            }
            get thetadatadx_config_get_streaming_keepalive_interval_secs(out: u64) [streaming.keepalive_interval_secs] {
                default: 2,
                doc: ["Current `streaming.keepalive_interval_secs` value."],
            }
            set thetadatadx_config_set_streaming_keepalive_retries(v: u32) [streaming.keepalive_retries] {
                param: n,
                default: 2,
                doc: ["Set the number of unanswered TCP keepalive probes after which the kernel declares the streaming connection dead (where the platform exposes the knob). Validated to `[1, 10]` at connect."],
            }
            get thetadatadx_config_get_streaming_keepalive_retries(out: u32) [streaming.keepalive_retries] {
                default: 2,
                doc: ["Current `streaming.keepalive_retries` value."],
            }
            get thetadatadx_config_get_streaming_ring_size(out: usize) [streaming.ring_size] {
                default: 131_072,
                doc: ["Current `streaming.ring_size` value."],
            }
            set thetadatadx_config_set_wait_mode(mode: i32) [streaming.wait_mode] {
                param: mode,
                default: Spin,
                values: WaitMode {
                    0 Spin "adaptive spin then a yield ramp; never sleeps, so it holds ~100% of one core.",
                    1 BusySpin "pure spin, no yield, never sleeps; ~100% of one core like the adaptive spin, with the lowest jitter.",
                    2 Park "spin and yield ramp, then sleep the park interval; low idle CPU.",
                    3 Backoff "spins while events flow, sleeps the park interval once idle, resumes; low idle CPU.",
                },
                errors: {
                    null: THETADATADX_ERR_OTHER,
                    ffi: "expected 0 (Spin), 1 (BusySpin), 2 (Park) or 3 (Backoff)",
                    py: "unknown wait_mode: {mode:?} (expected \"spin\", \"busyspin\", \"park\" or \"backoff\")",
                    ts: "setWaitMode: unknown mode {mode:?}; expected \"spin\", \"busyspin\", \"park\" or \"backoff\"",
                },
                doc: ["Set how the streaming consumer waits when the event ring is momentarily empty. Only the two modes that sleep lower idle CPU; their sleep length is `streaming.park_interval_us`."],
            }
            get thetadatadx_config_get_wait_mode(out_mode: i32) [streaming.wait_mode] {
                default: Spin,
                values: WaitMode { 0 Spin, 1 BusySpin, 2 Park, 3 Backoff },
                doc: ["Current streaming consumer wait mode."],
            }
            set thetadatadx_config_set_park_interval_us(us: u64) [streaming.park_interval_us] {
                param: us,
                default: 1000 "1 ms",
                doc: ["Set the park / backoff idle sleep length in MICROSECONDS for the streaming consumer. Used only by the two wait modes that sleep, park and backoff; ignored by the spinning ones. This is the worst-case delivery latency added to an event that arrives while the consumer is parked. The OS timer honours sleeps down to ~50 us (below that, kernel timer slack dominates, so 50 is the floor); a 100 us park is a valid low-latency option (a few percent of a core in live premarket). The client pings every ~100 ms, so a value beyond the ping cadence saves no extra CPU and only adds latency. Validated to `[50, 1_000_000]` us at connect time."],
            }
            get thetadatadx_config_get_park_interval_us(out_us: u64) [streaming.park_interval_us] {
                default: 1000 "1 ms",
                doc: ["Current `streaming.park_interval_us` value, in microseconds."],
            }
            set thetadatadx_config_set_retry_max_elapsed_secs(secs: u64) [retry.max_elapsed in secs] {
                param: secs,
                default: 300,
                doc: ["Set the wall-clock envelope (seconds) for one market-data-channel retry sequence, measured from the first attempt. Zero disables the envelope (attempt budget only)."],
            }
            get thetadatadx_config_get_retry_max_elapsed_secs(out_secs: u64) [retry.max_elapsed in secs] {
                default: 300,
                doc: ["Current `retry.max_elapsed` value in seconds; zero means disabled."],
            }
            set thetadatadx_config_set_retry_initial_delay_ms(ms: u64) [retry.initial_delay in ms] {
                param: ms,
                default: 250,
                doc: ["Set the initial backoff delay (ms) for the market-data-channel retry policy. Subsequent retries double from here, capped at `retry.max_delay`."],
            }
            set thetadatadx_config_set_retry_max_delay_ms(ms: u64) [retry.max_delay in ms] {
                param: ms,
                default: 30_000 "30 s",
                doc: ["Set the upper-bound backoff delay (ms) for the market-data-channel retry policy. The exponential schedule never exceeds this value regardless of attempt number."],
            }
            set thetadatadx_config_set_retry_max_attempts(n: u32) [retry.max_attempts] {
                param: n,
                default: 20,
                doc: ["Set the total attempt budget for the market-data-channel retry policy. `1` disables retry (single call only); higher values permit retries up to `max_attempts - 1` after the initial call."],
            }
            get thetadatadx_config_get_retry_max_attempts(out_n: u32) [retry.max_attempts] {
                default: 20,
                doc: ["Current `retry.max_attempts` value."],
            }
            set thetadatadx_config_set_retry_jitter(jitter: bool) [retry.jitter] {
                param: jitter,
                default: true,
                doc: ["Toggle AWS-style full jitter on the market-data-channel retry policy. Without it the backoff schedule is deterministic (`min(max_delay, initial * 2^attempt)`), which is useful for tests that need to assert exact timings."],
            }
            get thetadatadx_config_get_retry_jitter(out_jitter: bool) [retry.jitter] {
                default: true,
                doc: ["Current `retry.jitter` value."],
            }
            set thetadatadx_config_set_market_data_port(port: u16) [market_data.port] {
                param: port,
                doc: ["Set the market-data gRPC port. The environment factories set it; override it together with the market-data host to reach the market-data service at another address, such as a test server. A port outside `0..=65535` is rejected."],
            }
            get thetadatadx_config_get_market_data_port(out_port: u16) [market_data.port] {
                doc: ["Current market-data gRPC port."],
            }
            set thetadatadx_config_set_warn_on_buffered_threshold_bytes(n: usize) [market_data.warn_on_buffered_threshold_bytes] {
                param: n,
                default: 104_857_600 "100 MiB",
                doc: ["Set the warning threshold, in bytes, for buffered (non-streaming) historical responses. An endpoint whose decoded response exceeds it logs a warning pointing the caller at the endpoint's streaming variant, which delivers the same rows chunk by chunk with memory bounded to one chunk; the buffered data is still delivered. Zero disables the warning entirely."],
            }
            get thetadatadx_config_get_warn_on_buffered_threshold_bytes(out_n: usize) [market_data.warn_on_buffered_threshold_bytes] {
                default: 104_857_600 "100 MiB",
                doc: ["Current `market_data.warn_on_buffered_threshold_bytes` value, in bytes."],
            }
            set thetadatadx_config_set_request_timeout_secs(secs: u64) [market_data.request_timeout_secs] {
                param: secs,
                default: 300 "5 minutes",
                doc: [
                    "Set the default per-request deadline (seconds) for market-data queries.",
                    "",
                    "It bounds every request that did not set its own deadline, so a live-but-silent stream resolves to a timeout instead of blocking forever (the gRPC keepalive PING only detects a fully dead peer). Zero does not disable the deadline: it is floored to the 300-second default at request time, so the deadline always applies.",
                ],
            }
            get thetadatadx_config_get_request_timeout_secs(out: u64) [market_data.request_timeout_secs] {
                default: 300 "5 minutes",
                doc: ["Current `market_data.request_timeout_secs` value. A stored zero is floored to the 300-second default at request time rather than disabling the deadline."],
            }
            set thetadatadx_config_set_market_data_stream_window_size_kb(kb: usize) [market_data.stream_window_size_kb] {
                param: kb,
                default: 8192 "8 MiB",
                doc: ["Set the initial per-stream HTTP/2 flow-control window, in KB, for the market-data gRPC channel. A larger window raises the throughput ceiling on bulk streaming pulls before HTTP/2 backpressure kicks in. The value is clamped into `[64, 2_097_151]` KB at validate/connect time."],
            }
            get thetadatadx_config_get_market_data_stream_window_size_kb(out_kb: usize) [market_data.stream_window_size_kb] {
                default: 8192 "8 MiB",
                doc: ["Current per-stream HTTP/2 flow-control window (KB) for the market-data gRPC channel."],
            }
            set thetadatadx_config_set_market_data_connection_window_size_kb(kb: usize) [market_data.connection_window_size_kb] {
                param: kb,
                default: 16384 "16 MiB",
                doc: ["Set the initial connection-level HTTP/2 flow-control window, in KB, for the market-data gRPC channel. A larger window raises the throughput ceiling on bulk streaming pulls before HTTP/2 backpressure kicks in. The value is clamped into `[64, 2_097_151]` KB at validate/connect time."],
            }
            get thetadatadx_config_get_market_data_connection_window_size_kb(out_kb: usize) [market_data.connection_window_size_kb] {
                default: 16384 "16 MiB",
                doc: ["Current connection-level HTTP/2 flow-control window (KB) for the market-data gRPC channel."],
            }
            set thetadatadx_config_set_bulk_fetch(policy: i32) [market_data.bulk_fetch] {
                param: policy,
                default: Auto,
                values: BulkFetchPolicy {
                    0 Auto "a large history pull's requested time or date range is split into equal concurrent bands across the account's concurrent-request budget, decided from the request shape alone (no sizing request is issued). Buffered pulls merge the shards back into exactly the rows of the single-stream response: single-contract, stock, and index pulls keep the exact single-stream row order, while option-chain pulls come back in a deterministic canonical order (expiration, strike, right ascending; calls before puts; time-ascending within each contract). Streaming pulls forward each band's chunks to the handler as they arrive: every chunk exactly once, but chunks from different bands interleave in arrival order rather than the single stream's order. Small pulls and non-history endpoints are never sharded.",
                    1 Off "every query runs as a single stream, in the server's own row and chunk order.",
                },
                errors: {
                    null: THETADATADX_ERR_CONFIG,
                    ffi: "expected 0 (Auto) or 1 (Off)",
                    py: "unknown bulk_fetch: {policy:?} (expected \"auto\" or \"off\")",
                    ts: "setBulkFetch: unknown policy {policy:?}; expected \"auto\" or \"off\"",
                },
                doc: ["Set the automatic bulk-fetch sharding policy for history pulls, covering both the buffered and the chunk-streaming call paths."],
            }
            get thetadatadx_config_get_bulk_fetch(out_policy: i32) [market_data.bulk_fetch] {
                default: Auto,
                values: BulkFetchPolicy { 0 Auto, 1 Off },
                doc: ["Current bulk-fetch sharding policy."],
            }
            set thetadatadx_config_set_shard_concurrency(n: Option<u32>) [market_data.shard_concurrency] {
                param: n,
                doc: ["Set the upper bound on concurrent sub-requests per sharded bulk fetch. Unset, the default, a sharded pull uses the full concurrent-request budget: the channel-pool size resolved from `market_data.max_concurrent_requests`. Set, a sharded pull spreads across at most that many sub-requests; the applied value is clamped into `[1, pool_size]` when a plan is built, and validation floors an explicit `0` to `1`."],
            }
            get thetadatadx_config_get_shard_concurrency(out_n: Option<u32>) [market_data.shard_concurrency] {
                doc: ["Current `market_data.shard_concurrency` value. Unset means a sharded pull uses the full concurrent-request budget."],
            }
            set thetadatadx_config_set_max_concurrent_requests(n: Option<u32>) [market_data.max_concurrent_requests] {
                param: n,
                doc: [
                    "Set the number of concurrent in-flight market-data requests: the size of the gRPC channel pool and of the request semaphore that admits requests onto it, resolved at connect time.",
                    "",
                    "Unset, the default, the pool is sized to the account's subscription tier from the auth response (Free 1 / Value 2 / Standard 4 / Pro 8). Set, the pool is that size verbatim, with no client-side cap: the allowance is enforced server-side, so an account boosted above its base tier sets the boosted value and runs that wide. Requests past the server's allowance surface as `ResourceExhausted` and are retried with backoff. Validation floors an explicit `0` to `1`.",
                ],
            }
            get thetadatadx_config_get_max_concurrent_requests(out_n: Option<u32>) [market_data.max_concurrent_requests] {
                doc: ["Current `market_data.max_concurrent_requests` value. Unset means the pool is sized to the account's subscription tier at connect time."],
            }
            get thetadatadx_config_get_retry_initial_delay_ms(out_ms: u64) [retry.initial_delay in ms] {
                default: 250,
                doc: ["Current `retry.initial_delay` value in ms."],
            }
            get thetadatadx_config_get_retry_max_delay_ms(out_ms: u64) [retry.max_delay in ms] {
                default: 30_000 "30 s",
                doc: ["Current `retry.max_delay` value in ms."],
            }
            set thetadatadx_config_set_reconnect_max_attempts(max_attempts: u32) [limits.max_attempts] {
                param: max_attempts,
                default: 30,
                doc: ["Set the per-class transient-failure attempt budget for the auto-reconnect path. No effect unless the reconnect policy is `Auto`."],
            }
            get thetadatadx_config_get_reconnect_max_attempts(out: u32) [limits.max_attempts] {
                default: 30,
                doc: ["Current generic-transient reconnect attempt budget. Under a reconnect policy other than `Auto` it reads the default limit, since the budgets apply only under `Auto`."],
            }
            set thetadatadx_config_set_reconnect_max_rate_limited_attempts(max_rate_limited_attempts: u32) [limits.max_rate_limited_attempts] {
                param: max_rate_limited_attempts,
                default: 100,
                doc: ["Set the per-class rate-limited (`TooManyRequests`) attempt budget for the auto-reconnect path. No effect unless the reconnect policy is `Auto`."],
            }
            get thetadatadx_config_get_reconnect_max_rate_limited_attempts(out: u32) [limits.max_rate_limited_attempts] {
                default: 100,
                doc: ["Current rate-limited (`TooManyRequests`) reconnect attempt budget. Under a reconnect policy other than `Auto` it reads the default limit, since the budgets apply only under `Auto`."],
            }
            set thetadatadx_config_set_reconnect_max_server_restart_attempts(n: u32) [limits.max_server_restart_attempts] {
                param: n,
                default: 60,
                doc: ["Set the `ServerRestarting` reconnect attempt budget. No effect unless the reconnect policy is `Auto`."],
            }
            get thetadatadx_config_get_reconnect_max_server_restart_attempts(out: u32) [limits.max_server_restart_attempts] {
                default: 60,
                doc: ["Current `ServerRestarting` reconnect attempt budget. Under a reconnect policy other than `Auto` it reads the default limit, since the budgets apply only under `Auto`."],
            }
            set thetadatadx_config_set_reconnect_max_elapsed_secs(secs: u64) [limits.max_elapsed in secs] {
                param: secs,
                default: 300,
                doc: ["Set the wall-clock reconnect envelope (seconds) for the generic-transient and server-restart classes, measured from the first attempt of a consecutive-reconnect sequence. Zero disables the envelope (attempt budgets only). No effect unless the reconnect policy is `Auto`."],
            }
            get thetadatadx_config_get_reconnect_max_elapsed_secs(out: u64) [limits.max_elapsed in secs] {
                default: 300,
                doc: ["Current wall-clock reconnect envelope in seconds; zero means disabled. Under a reconnect policy other than `Auto` it reads the default limit, since the budgets apply only under `Auto`."],
            }
            set thetadatadx_config_set_reconnect_stable_window_secs(secs: u64) [limits.stable_window in secs] {
                param: secs,
                default: 60,
                doc: ["Set the continuous successful-data-flow window (seconds) after which the auto-reconnect attempt counters reset. No effect unless the reconnect policy is `Auto`."],
            }
            get thetadatadx_config_get_reconnect_stable_window_secs(out: u64) [limits.stable_window in secs] {
                default: 60,
                doc: ["Current stable-window reset interval in seconds. Under a reconnect policy other than `Auto` it reads the default limit, since the budgets apply only under `Auto`."],
            }
            set thetadatadx_config_set_nexus_url(url: String) [auth.nexus_url] {
                param: url,
                doc: ["Set the Nexus auth URL. The default is the upstream production endpoint; override it to point at a staging cluster for testing."],
            }
            get thetadatadx_config_get_nexus_url(out: String) [auth.nexus_url] {
                doc: ["Current `auth.nexus_url` value."],
            }
            set thetadatadx_config_set_client_type(client_type: String) [auth.client_type] {
                param: client_type,
                doc: ["Set the `QueryInfo.client_type` identifier. The default is `\"rust-thetadatadx\"`; override it to identify a deployment fleet in server-side dashboards."],
            }
            get thetadatadx_config_get_client_type(out: String) [auth.client_type] {
                doc: ["Current `auth.client_type` value."],
            }
            set thetadatadx_config_set_market_data_host(host: String) [set_market_data_host()] {
                param: host,
                doc: ["Set the market-data gRPC host. The environment factories set it; override it together with the market-data port to reach the market-data service at another address, such as a test server."],
            }
            get thetadatadx_config_get_market_data_host(out: String) [market_data_host()] {
                doc: ["Current market-data gRPC host."],
            }
            set thetadatadx_config_set_reconnect_jitter(mode: i32) [reconnect.jitter] {
                param: mode,
                default: Full,
                values: JitterMode {
                    0 Full "sample uniformly from `[0, delay]`.",
                    1 None "deterministic delays, for tests only.",
                },
                errors: {
                    null: THETADATADX_ERR_OTHER,
                    ffi: "expected 0 (Full) or 1 (None)",
                    py: "unknown reconnect_jitter: {mode:?} (expected \"full\" or \"none\")",
                    ts: "setReconnectJitter: unknown mode {mode:?}; expected \"full\" or \"none\"",
                },
                doc: ["Set the jitter strategy applied to every reconnect delay."],
            }
            get thetadatadx_config_get_reconnect_jitter(out_mode: i32) [reconnect.jitter] {
                default: Full,
                values: JitterMode { 0 Full, 1 None },
                doc: ["Current reconnect jitter mode."],
            }
        }
    };
}

#[cfg(test)]
mod tests {
    use crate::DirectConfig;

    /// Asserts that each entry's default is the production config's value.
    macro_rules! check_defaults {
        ({} $($dir:ident $sym:ident $sig:tt $target:tt { $($body:tt)* })*) => {
            let config = DirectConfig::production_defaults();
            $(check_defaults!(@entry config $sym $target $($body)*);)*
        };
        (@entry $c:ident $sym:ident $target:tt $(param: $p:ident,)?
            default: $v:ident, values: $enum:ident $($rest:tt)*) => {
            assert!(
                matches!($crate::__config_read!($c $target), crate::$enum::$v),
                concat!(stringify!($sym), " documents the default ", stringify!($v)),
            );
        };
        (@entry $c:ident $sym:ident $target:tt $(param: $p:ident,)?
            default: $v:literal $($gloss:literal)?, $($rest:tt)*) => {
            assert_eq!(
                $crate::__config_read!($c $target),
                $v,
                concat!(stringify!($sym), " documents the default ", stringify!($v)),
            );
        };
        (@entry $c:ident $sym:ident $target:tt $(param: $p:ident,)? doc: $doc:tt $(,)?) => {};
    }

    /// The default each binding documents is the one `DirectConfig` starts
    /// from.
    #[test]
    fn documented_defaults_match_the_production_config() {
        crate::__config! { check_defaults { {} } }
    }
}
