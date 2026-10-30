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
//!     doc: ["Set the retry deadline in seconds."],
//!     cpp_doc: [..],
//!     py_doc: [..],
//!     ts_doc: [..],
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
//! - `values` lists an enum accessor's codes and variants, and `errors` an
//!   enum setter's rejections: the C error code for a null handle and the
//!   text each binding reports for a value outside the enum.
//! - `doc`, `cpp_doc`, `py_doc` and `ts_doc` are the documentation of the C,
//!   C++, Python and TypeScript accessor, one string per line, kept verbatim.

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
                doc: [
                    "Set the reconnect delay (ms) honoured for generic transient",
                    "disconnects (TimedOut, ServerRestarting, Unspecified, …). Applied to",
                    "the streaming session at connect time. Default `250`.",
                ],
                cpp_doc: [
                    "Set the reconnect delay (ms) honoured for generic transient",
                    "disconnects (TimedOut, ServerRestarting, Unspecified, ...).",
                    "Default 250.",
                ],
                py_doc: [
                    "Set the reconnect delay (ms) honoured for generic transient",
                    "disconnects (TimedOut, ServerRestarting, Unspecified, …).",
                    "Applied to the streaming session at connect time.",
                    "Default ``250``.",
                ],
                ts_doc: [
                    "Set the reconnect delay (ms) honoured for generic transient",
                    "disconnects (TimedOut, ServerRestarting, Unspecified, …).",
                    "Plumbed through to the streaming I/O loop at connect time.",
                    "Default `250`.",
                    "",
                    "Accepts a `bigint` for parity with the other bindings, which use a 64-bit unsigned integer.",
                ],
            }
            get thetadatadx_config_get_reconnect_wait_ms(out_ms: u64) [reconnect.wait_ms] {
                doc: [
                    "Read the current reconnect `wait_ms` setting.",
                    "",
                    "Writes the configured millisecond delay into `*out_ms`. Returns",
                    "`0` on success, `-1` if either pointer is null.",
                ],
                cpp_doc: [
                    "Current reconnect wait_ms (default 250). Returns the default on",
                    "a null config handle.",
                ],
                py_doc: [
                    "Current reconnect ``wait_ms`` value (default ``250``).",
                ],
                ts_doc: [
                    "Current reconnect `wait_ms` value (default `250`).",
                ],
            }
            set thetadatadx_config_set_reconnect_wait_rate_limited_ms(ms: u64) [reconnect.wait_rate_limited_ms] {
                param: ms,
                doc: [
                    "Set the reconnect delay (ms) honoured for `TooManyRequests`",
                    "rate-limited disconnects. Applied to the streaming session at connect",
                    "time. Default `130_000` (matches the JVM terminal's 130 s rate-limit",
                    "cooldown).",
                ],
                cpp_doc: [
                    "Set the reconnect delay (ms) honoured for `TooManyRequests`",
                    "rate-limited disconnects. Default 130_000.",
                ],
                py_doc: [
                    "Set the reconnect delay (ms) honoured for ``TooManyRequests``",
                    "rate-limited disconnects. Default ``130_000`` (matches the",
                    "JVM terminal's 130 s rate-limit cooldown).",
                ],
                ts_doc: [
                    "Set the reconnect delay (ms) honoured for `TooManyRequests`",
                    "rate-limited disconnects. Default `130_000`.",
                ],
            }
            get thetadatadx_config_get_reconnect_wait_rate_limited_ms(out_ms: u64) [reconnect.wait_rate_limited_ms] {
                doc: [
                    "Read the current reconnect `wait_rate_limited_ms` setting. Same",
                    "shape as `thetadatadx_config_get_reconnect_wait_ms`.",
                ],
                cpp_doc: [
                    "Current reconnect wait_rate_limited_ms (default 130_000).",
                ],
                py_doc: [
                    "Current reconnect ``wait_rate_limited_ms`` value (default ``130_000``).",
                ],
                ts_doc: [
                    "Current reconnect `wait_rate_limited_ms` value (default `130_000`).",
                ],
            }
            set thetadatadx_config_set_reconnect_wait_max_ms(v: u64) [reconnect.wait_max_ms] {
                param: ms,
                doc: [
                    "Set the cap (ms) on the exponential generic-transient reconnect ladder. The ladder starts at `reconnect_wait_ms` and doubles per consecutive attempt up to this value. Default `30_000`.",
                ],
                cpp_doc: [
                    "Set the cap (ms) on the exponential generic-transient reconnect",
                    "ladder. Default 30_000.",
                ],
                py_doc: [
                    "Set the cap (ms) on the exponential generic-transient reconnect",
                    "ladder. The ladder starts at ``reconnect_wait_ms`` and doubles",
                    "per consecutive attempt up to this value. Default ``30_000``.",
                ],
                ts_doc: [
                    "Set the cap (ms) on the exponential generic-transient reconnect",
                    "ladder. The ladder starts at `reconnectWaitMs` and doubles per",
                    "consecutive attempt up to this value. Default `30_000n`.",
                ],
            }
            get thetadatadx_config_get_reconnect_wait_max_ms(out: u64) [reconnect.wait_max_ms] {
                doc: [
                    "Read the current reconnect `wait_max_ms` setting (default `30_000`).",
                    "",
                    "Writes the configured value into `*out`. Returns `0` on success,",
                    "`-1` if either pointer is null.",
                ],
                cpp_doc: [
                    "Current reconnect wait_max_ms (default 30_000).",
                ],
                py_doc: [
                    "Current reconnect ``wait_max_ms`` value (default ``30_000``).",
                ],
                ts_doc: [
                    "Current reconnect `wait_max_ms` value (default `30_000n`).",
                ],
            }
            set thetadatadx_config_set_reconnect_wait_server_restart_ms(v: u64) [reconnect.wait_server_restart_ms] {
                param: ms,
                doc: [
                    "Set the flat reconnect cadence (ms) for `ServerRestarting` disconnects. Default `5_000`.",
                ],
                cpp_doc: [
                    "Set the flat reconnect cadence (ms) for ServerRestarting",
                    "disconnects. Default 5_000.",
                ],
                py_doc: [
                    "Set the flat reconnect cadence (ms) for ``ServerRestarting``",
                    "disconnects. Default ``5_000``.",
                ],
                ts_doc: [
                    "Set the flat reconnect cadence (ms) for `ServerRestarting`",
                    "disconnects. Default `5_000n`.",
                ],
            }
            get thetadatadx_config_get_reconnect_wait_server_restart_ms(out: u64) [reconnect.wait_server_restart_ms] {
                doc: [
                    "Read the current reconnect `wait_server_restart_ms` setting (default `5_000`).",
                    "",
                    "Writes the configured value into `*out`. Returns `0` on success,",
                    "`-1` if either pointer is null.",
                ],
                cpp_doc: [
                    "Current reconnect wait_server_restart_ms (default 5_000).",
                ],
                py_doc: [
                    "Current reconnect ``wait_server_restart_ms`` value (default ``5_000``).",
                ],
                ts_doc: [
                    "Current reconnect `wait_server_restart_ms` value (default `5_000n`).",
                ],
            }
            set thetadatadx_config_set_reconnect_replay_burst_size(n: u32) [reconnect.replay_burst_size] {
                param: n,
                doc: [
                    "Set the subscription-replay burst size used after an",
                    "auto-reconnect: frames are written in bursts of this many, each",
                    "burst flushed and followed by a jittered `replay_pace_ms` pause.",
                    "Minimum `1` (validated at connect). Default `50`.",
                ],
                cpp_doc: [
                    "Set the subscription-replay burst size used after an",
                    "auto-reconnect. Minimum 1 (validated at connect). Default 50.",
                ],
                py_doc: [
                    "Set the subscription-replay burst size used after an",
                    "auto-reconnect: frames are written in bursts of this many, each",
                    "burst flushed and followed by a jittered ``replay_pace_ms``",
                    "pause. Minimum ``1`` (validated at connect). Default ``50``.",
                ],
                ts_doc: [
                    "Set the subscription-replay burst size used after an",
                    "auto-reconnect: frames are written in bursts of this many, each",
                    "burst flushed and followed by a jittered `replayPaceMs` pause.",
                    "Minimum `1` (validated at connect). Default `50`.",
                ],
            }
            get thetadatadx_config_get_reconnect_replay_burst_size(out: u32) [reconnect.replay_burst_size] {
                doc: [
                    "Read the current `replay_burst_size` setting (default `50`).",
                    "",
                    "Writes the configured value into `*out`. Returns `0` on success,",
                    "`-1` if either pointer is null.",
                ],
                cpp_doc: [
                    "Current replay_burst_size (default 50).",
                ],
                py_doc: [
                    "Current ``replay_burst_size`` value (default ``50``).",
                ],
                ts_doc: [
                    "Current `replay_burst_size` value (default `50`).",
                ],
            }
            set thetadatadx_config_set_reconnect_replay_pace_ms(v: u64) [reconnect.replay_pace_ms] {
                param: ms,
                doc: [
                    "Set the pause (ms) between subscription-replay bursts after an auto-reconnect. `0` removes the pause. Default `5`.",
                ],
                cpp_doc: [
                    "Set the pause (ms) between subscription-replay bursts. 0",
                    "removes the pause. Default 5.",
                ],
                py_doc: [
                    "Set the pause (ms) between subscription-replay bursts after an",
                    "auto-reconnect. ``0`` removes the pause. Default ``5``.",
                ],
                ts_doc: [
                    "Set the pause (ms) between subscription-replay bursts after an",
                    "auto-reconnect. `0n` removes the pause. Default `5n`.",
                ],
            }
            get thetadatadx_config_get_reconnect_replay_pace_ms(out: u64) [reconnect.replay_pace_ms] {
                doc: [
                    "Read the current `replay_pace_ms` setting (default `5`).",
                    "",
                    "Writes the configured value into `*out`. Returns `0` on success,",
                    "`-1` if either pointer is null.",
                ],
                cpp_doc: [
                    "Current replay_pace_ms (default 5).",
                ],
                py_doc: [
                    "Current ``replay_pace_ms`` value (default ``5``).",
                ],
                ts_doc: [
                    "Current `replay_pace_ms` value (default `5n`).",
                ],
            }
            set thetadatadx_config_set_streaming_timeout_ms(v: u64) [streaming.timeout_ms] {
                param: ms,
                doc: [
                    "Set the streaming read timeout (ms): the no-frames deadline after which the streaming session is declared dead and reconnects. Default `10_000`; validated to `[100, 60_000]` at connect.",
                ],
                cpp_doc: [
                    "Set the streaming read timeout (ms): the no-frames deadline after",
                    "which the streaming I/O loop reconnects. Default 10_000;",
                    "validated to [100, 60_000] at connect.",
                ],
                py_doc: [
                    "Set the streaming read timeout (ms): the no-frames deadline after",
                    "which the streaming session is declared dead and reconnects.",
                    "Default ``10_000``; validated to ``[100, 60_000]``.",
                ],
                ts_doc: [
                    "Set the streaming read timeout (ms): the no-frames deadline after which the streaming I/O loop declares the session dead and reconnects. Default `10_000n`; validated to `[100, 60_000]` at connect.",
                ],
            }
            get thetadatadx_config_get_streaming_timeout_ms(out: u64) [streaming.timeout_ms] {
                doc: [
                    "Read the current streaming `timeout_ms` setting (default `10_000`).",
                    "",
                    "Writes the configured value into `*out`. Returns `0` on success,",
                    "`-1` if either pointer is null.",
                ],
                cpp_doc: [
                    "Current streaming timeout_ms (default 10_000).",
                ],
                py_doc: [
                    "Current ``streaming.timeout_ms`` value (default ``10_000``).",
                ],
                ts_doc: [
                    "Current `streaming.timeout_ms` value (default `10_000n`).",
                ],
            }
            set thetadatadx_config_set_streaming_connect_timeout_ms(v: u64) [streaming.connect_timeout_ms] {
                param: ms,
                doc: [
                    "Set the per-server streaming TCP connect timeout (ms). Default `2_000`; validated to `[1_000, 60_000]` at connect.",
                ],
                cpp_doc: [
                    "Set the per-server connect timeout (ms) for the streaming",
                    "connection. Default 2_000; validated to [1_000, 60_000] at",
                    "connect.",
                ],
                py_doc: [
                    "Set the per-server connect timeout (ms) for the streaming",
                    "connection. Default",
                    "``2_000``; validated to ``[1_000, 60_000]``.",
                ],
                ts_doc: [
                    "Set the per-server connect timeout (ms) for the streaming connection. Default `2_000n`; validated to `[1_000, 60_000]` at connect.",
                ],
            }
            get thetadatadx_config_get_streaming_connect_timeout_ms(out: u64) [streaming.connect_timeout_ms] {
                doc: [
                    "Read the current streaming `connect_timeout_ms` setting (default `2_000`).",
                    "",
                    "Writes the configured value into `*out`. Returns `0` on success,",
                    "`-1` if either pointer is null.",
                ],
                cpp_doc: [
                    "Current streaming connect_timeout_ms (default 2_000).",
                ],
                py_doc: [
                    "Current ``streaming.connect_timeout_ms`` value (default ``2_000``).",
                ],
                ts_doc: [
                    "Current `streaming.connect_timeout_ms` value (default `2_000n`).",
                ],
            }
            set thetadatadx_config_set_streaming_ping_interval_ms(v: u64) [streaming.ping_interval_ms] {
                param: ms,
                doc: [
                    "Set the streaming heartbeat ping interval (ms). Default `100`; validated to `[100, 300_000]` at connect.",
                ],
                cpp_doc: [
                    "Set the streaming heartbeat ping interval (ms). Default 100;",
                    "validated to [100, 300_000] at connect.",
                ],
                py_doc: [
                    "Set the streaming heartbeat ping interval (ms). Default ``100``;",
                    "validated to ``[100, 300_000]``.",
                ],
                ts_doc: [
                    "Set the streaming heartbeat ping interval (ms). Default `100n`; validated to `[100, 300_000]` at connect.",
                ],
            }
            get thetadatadx_config_get_streaming_ping_interval_ms(out: u64) [streaming.ping_interval_ms] {
                doc: [
                    "Read the current streaming `ping_interval_ms` setting (default `100`).",
                    "",
                    "Writes the configured value into `*out`. Returns `0` on success,",
                    "`-1` if either pointer is null.",
                ],
                cpp_doc: [
                    "Current streaming ping_interval_ms (default 100).",
                ],
                py_doc: [
                    "Current ``streaming.ping_interval_ms`` value (default ``100``).",
                ],
                ts_doc: [
                    "Current `streaming.ping_interval_ms` value (default `100n`).",
                ],
            }
            set thetadatadx_config_set_streaming_io_read_slice_ms(v: u64) [streaming.io_read_slice_ms] {
                param: ms,
                doc: [
                    "Set the per-iteration blocking-read slice (ms) for the streaming I/O loop. Shorter slices service outbound commands more promptly at slightly higher idle CPU. Default `25`; validated to `[10, 500]` at connect.",
                ],
                cpp_doc: [
                    "Set the per-iteration blocking-read slice (ms) for the",
                    "streaming I/O loop. Default 25; validated to [10, 500].",
                ],
                py_doc: [
                    "Set the per-iteration blocking-read slice (ms) for the",
                    "streaming I/O loop. Default ``25``; validated to ``[10, 500]``.",
                ],
                ts_doc: [
                    "Set the per-iteration blocking-read slice (ms) for the streaming I/O loop. Default `25n`; validated to `[10, 500]` at connect.",
                ],
            }
            get thetadatadx_config_get_streaming_io_read_slice_ms(out: u64) [streaming.io_read_slice_ms] {
                doc: [
                    "Read the current streaming `io_read_slice_ms` setting (default `25`).",
                    "",
                    "Writes the configured value into `*out`. Returns `0` on success,",
                    "`-1` if either pointer is null.",
                ],
                cpp_doc: [
                    "Current streaming io_read_slice_ms (default 25).",
                ],
                py_doc: [
                    "Current ``streaming.io_read_slice_ms`` value (default ``25``).",
                ],
                ts_doc: [
                    "Current `streaming.io_read_slice_ms` value (default `25n`).",
                ],
            }
            set thetadatadx_config_set_streaming_keepalive_idle_secs(v: u64) [streaming.keepalive_idle_secs] {
                param: secs,
                doc: [
                    "Set the TCP keepalive idle time (seconds) before the kernel sends the first probe on a silent streaming socket. Default `5`; validated to `[1, 7_200]` at connect.",
                ],
                cpp_doc: [
                    "Set the TCP keepalive idle time (seconds). Default 5; validated",
                    "to [1, 7_200] at connect.",
                ],
                py_doc: [
                    "Set the TCP keepalive idle time (seconds) before the first",
                    "kernel probe on a silent streaming socket. Default ``5``;",
                    "validated to ``[1, 7_200]``.",
                ],
                ts_doc: [
                    "Set the TCP keepalive idle time (seconds) before the first kernel probe on a silent streaming socket. Default `5n`; validated to `[1, 7_200]` at connect.",
                ],
            }
            get thetadatadx_config_get_streaming_keepalive_idle_secs(out: u64) [streaming.keepalive_idle_secs] {
                doc: [
                    "Read the current streaming `keepalive_idle_secs` setting (default `5`).",
                    "",
                    "Writes the configured value into `*out`. Returns `0` on success,",
                    "`-1` if either pointer is null.",
                ],
                cpp_doc: [
                    "Current streaming keepalive_idle_secs (default 5).",
                ],
                py_doc: [
                    "Current ``streaming.keepalive_idle_secs`` value (default ``5``).",
                ],
                ts_doc: [
                    "Current `streaming.keepalive_idle_secs` value (default `5n`).",
                ],
            }
            set thetadatadx_config_set_streaming_keepalive_interval_secs(v: u64) [streaming.keepalive_interval_secs] {
                param: secs,
                doc: [
                    "Set the interval (seconds) between TCP keepalive probes. Default `2`; validated to `[1, 75]` at connect.",
                ],
                cpp_doc: [
                    "Set the TCP keepalive probe interval (seconds). Default 2;",
                    "validated to [1, 75] at connect.",
                ],
                py_doc: [
                    "Set the interval (seconds) between TCP keepalive probes.",
                    "Default ``2``; validated to ``[1, 75]``.",
                ],
                ts_doc: [
                    "Set the interval (seconds) between TCP keepalive probes. Default `2n`; validated to `[1, 75]` at connect.",
                ],
            }
            get thetadatadx_config_get_streaming_keepalive_interval_secs(out: u64) [streaming.keepalive_interval_secs] {
                doc: [
                    "Read the current streaming `keepalive_interval_secs` setting (default `2`).",
                    "",
                    "Writes the configured value into `*out`. Returns `0` on success,",
                    "`-1` if either pointer is null.",
                ],
                cpp_doc: [
                    "Current streaming keepalive_interval_secs (default 2).",
                ],
                py_doc: [
                    "Current ``streaming.keepalive_interval_secs`` value (default ``2``).",
                ],
                ts_doc: [
                    "Current `streaming.keepalive_interval_secs` value (default `2n`).",
                ],
            }
            set thetadatadx_config_set_streaming_keepalive_retries(v: u32) [streaming.keepalive_retries] {
                param: n,
                doc: [
                    "Set the number of unanswered TCP keepalive probes after which the kernel declares the streaming connection dead (where the platform exposes the knob). Default `2`; validated to `[1, 10]` at connect.",
                ],
                cpp_doc: [
                    "Set the TCP keepalive probe count before the kernel declares",
                    "the peer dead. Default 2; validated to [1, 10] at connect.",
                ],
                py_doc: [
                    "Set the number of unanswered TCP keepalive probes after which",
                    "the kernel declares the streaming connection dead (where the",
                    "platform exposes the knob). Default ``2``; validated to",
                    "``[1, 10]``.",
                ],
                ts_doc: [
                    "Set the number of unanswered TCP keepalive probes after which",
                    "the kernel declares the streaming connection dead (where the",
                    "platform exposes the knob). Default `2`; validated to `[1, 10]`",
                    "at connect.",
                ],
            }
            get thetadatadx_config_get_streaming_keepalive_retries(out: u32) [streaming.keepalive_retries] {
                doc: [
                    "Read the current streaming `keepalive_retries` setting (default `2`).",
                    "",
                    "Writes the configured value into `*out`. Returns `0` on success,",
                    "`-1` if either pointer is null.",
                ],
                cpp_doc: [
                    "Current streaming keepalive_retries (default 2).",
                ],
                py_doc: [
                    "Current ``streaming.keepalive_retries`` value (default ``2``).",
                ],
                ts_doc: [
                    "Current `streaming.keepalive_retries` value (default `2`).",
                ],
            }
            get thetadatadx_config_get_streaming_ring_size(out: usize) [streaming.ring_size] {
                doc: [
                    "Read the current streaming `ring_size` setting (default `131_072`).",
                    "",
                    "Writes the configured value into `*out`. Returns `0` on success,",
                    "`-1` if either pointer is null.",
                ],
                cpp_doc: [
                    "Current streaming ring_size (default 131_072).",
                ],
                py_doc: [
                    "Current ``streaming.ring_size`` value (default ``131_072``).",
                ],
                ts_doc: [
                    "Current `streaming.ring_size` value (returned as a `BigInt`; default",
                    "`131_072`).",
                ],
            }
            set thetadatadx_config_set_wait_mode(mode: i32) [streaming.wait_mode] {
                param: mode,
                values: WaitMode { 0 Spin, 1 BusySpin, 2 Park, 3 Backoff },
                errors: {
                    null: THETADATADX_ERR_OTHER,
                    ffi: "expected 0 (Spin), 1 (BusySpin), 2 (Park) or 3 (Backoff)",
                    py: "unknown wait_mode: {mode:?} (expected \"spin\", \"busyspin\", \"park\" or \"backoff\")",
                    ts: "setWaitMode: unknown mode {mode:?}; expected \"spin\", \"busyspin\", \"park\" or \"backoff\"",
                },
                doc: [
                    "Set how the streaming consumer waits when the event ring is momentarily",
                    "empty.",
                    "",
                    "- `mode = 0`: Spin (default); adaptive spin then a yield ramp; never sleeps.",
                    "- `mode = 1`: BusySpin; pure spin, no yield, never sleeps. Lowest jitter.",
                    "- `mode = 2`: Park; spin/yield ramp then sleep the park interval. Low CPU.",
                    "- `mode = 3`: Backoff; spins while events flow, sleeps once idle, resumes.",
                    "",
                    "`Spin` and `BusySpin` both hold ~100% of one core and differ only in",
                    "jitter; only `Park` and `Backoff` lower idle CPU. The park / backoff sleep",
                    "length is `thetadatadx_config_set_park_interval_us`.",
                    "",
                    "Returns `0` on success. Returns `-1` and sets `thetadatadx_last_error` when",
                    "`mode` is outside the documented `{0, 1, 2, 3}` set or `config` is null. A",
                    "rejected `mode` carries `thetadatadx_last_error_code = THETADATADX_ERR_INVALID_PARAMETER`",
                    "so an out-of-domain enum int surfaces the same typed class across every",
                    "binding.",
                ],
                cpp_doc: [
                    "Set the streaming consumer wait mode: 0=Spin (default), 1=BusySpin,",
                    "2=Park, 3=Backoff. Spin and BusySpin both hold ~100% of one core (differ",
                    "only in jitter); only Park and Backoff lower idle CPU. Throws @c",
                    "thetadatadx::InvalidParameterError on an out-of-domain mode (and @c",
                    "thetadatadx::ThetaDataError on a null handle).",
                ],
                py_doc: [
                    "Set how the streaming consumer waits when the event ring is empty.",
                    "Accepts ``\"spin\"`` (default), ``\"busyspin\"``, ``\"park\"``, or",
                    "``\"backoff\"`` (case-insensitive). ``\"spin\"`` and ``\"busyspin\"`` both hold",
                    "~100% of one core and differ only in jitter; only ``\"park\"`` and",
                    "``\"backoff\"`` lower idle CPU. The park / backoff sleep length is",
                    "``park_interval_us``.",
                ],
                ts_doc: [
                    "Set how the streaming consumer waits when the event ring is empty.",
                    "Accepts `\"spin\"` (default), `\"busyspin\"`, `\"park\"`, or `\"backoff\"`",
                    "(case-insensitive). `\"spin\"` and `\"busyspin\"` both hold ~100% of one core",
                    "and differ only in jitter; only `\"park\"` and `\"backoff\"` lower idle CPU.",
                    "The park / backoff sleep length is `parkIntervalUs`.",
                ],
            }
            get thetadatadx_config_get_wait_mode(out_mode: i32) [streaming.wait_mode] {
                values: WaitMode { 0 Spin, 1 BusySpin, 2 Park, 3 Backoff },
                doc: [
                    "Read the configured streaming consumer wait mode. Same encoding as",
                    "`thetadatadx_config_set_wait_mode` (0=Spin, 1=BusySpin, 2=Park, 3=Backoff).",
                    "Returns `0` on success, `-1` if either pointer is null.",
                ],
                cpp_doc: [
                    "Current streaming consumer wait mode (same encoding as the setter).",
                ],
                py_doc: [
                    "Current streaming consumer wait mode as a lowercase string.",
                ],
                ts_doc: [
                    "Current streaming consumer wait mode as a lowercase string.",
                ],
            }
            set thetadatadx_config_set_park_interval_us(us: u64) [streaming.park_interval_us] {
                param: us,
                doc: [
                    "Set the park / backoff idle sleep length in MICROSECONDS for the streaming",
                    "consumer. Used only when the wait mode is `Park` or `Backoff`; ignored by",
                    "`Spin` / `BusySpin`. This is the worst-case delivery latency added to an",
                    "event that arrives while the consumer is parked. The OS timer honours sleeps",
                    "down to ~50 us (below that, kernel timer slack dominates, so 50 is the",
                    "floor); a 100 us park is a valid low-latency option (a few percent of a core in live",
                    "premarket). The client pings every ~100 ms, so a value beyond the ping",
                    "cadence saves no extra CPU and only adds latency. Validated to",
                    "`[50, 1_000_000]` us at connect time. Default `1000` (= 1 ms).",
                ],
                cpp_doc: [
                    "Set the park / backoff idle sleep length in microseconds for the streaming",
                    "consumer. Used only by the Park / Backoff wait modes. Validated",
                    "[50, 1000000]. Default 1000 (= 1 ms).",
                ],
                py_doc: [
                    "Set the park / backoff idle sleep length in microseconds for the streaming",
                    "consumer. Used only when the wait mode is ``\"park\"`` or ``\"backoff\"``.",
                    "Validated ``[50, 1000000]`` at connect time. Default ``1000`` (= 1 ms).",
                ],
                ts_doc: [
                    "Set the park / backoff idle sleep length in microseconds for the streaming",
                    "consumer. Used only when the wait mode is `\"park\"` or `\"backoff\"`. Validated",
                    "`[50, 1000000]` at connect time. Default `1000` (= 1 ms).",
                    "",
                    "Accepts a `bigint` for parity with the other bindings, which use a 64-bit unsigned integer.",
                ],
            }
            get thetadatadx_config_get_park_interval_us(out_us: u64) [streaming.park_interval_us] {
                doc: [
                    "Read the current streaming `park_interval_us` setting (default `1000`,",
                    "= 1 ms).",
                    "",
                    "Writes the configured microsecond sleep length into `*out_us`. Returns",
                    "`0` on success, `-1` if either pointer is null.",
                ],
                cpp_doc: [
                    "Current streaming park_interval_us (default 1000, = 1 ms). Returns the",
                    "default on a null config handle.",
                ],
                py_doc: [
                    "Current streaming ``park_interval_us`` value in microseconds (default",
                    "``1000``, = 1 ms).",
                ],
                ts_doc: [
                    "Current streaming `park_interval_us` value in microseconds (returned as a",
                    "`BigInt`; default `1000`, = 1 ms).",
                ],
            }
            set thetadatadx_config_set_retry_max_elapsed_secs(secs: u64) [retry.max_elapsed in secs] {
                param: secs,
                doc: [
                    "Set the wall-clock envelope (seconds) for one market-data-channel",
                    "retry sequence, measured from the first attempt. `0` disables the",
                    "envelope (attempt budget only). Default `300`.",
                ],
                cpp_doc: [
                    "Set the wall-clock envelope (seconds) for one",
                    "market-data-channel retry sequence. 0 disables. Default 300.",
                ],
                py_doc: [
                    "Set the wall-clock envelope (seconds) for one",
                    "market-data-channel retry sequence, measured from the first",
                    "attempt. ``0`` disables the envelope (attempt budget only).",
                    "Default ``300``.",
                ],
                ts_doc: [
                    "Set the wall-clock envelope (seconds) for one",
                    "market-data-channel retry sequence, measured from the first",
                    "attempt. `0n` disables the envelope (attempt budget only).",
                    "Default `300n`.",
                ],
            }
            get thetadatadx_config_get_retry_max_elapsed_secs(out_secs: u64) [retry.max_elapsed in secs] {
                doc: [
                    "Read the current `retry.max_elapsed` value in seconds (default",
                    "`300`; `0` = disabled). Returns `0` on success, `-1` if either",
                    "pointer is null.",
                ],
                cpp_doc: [
                    "Current retry max_elapsed in seconds (default 300; 0 = disabled).",
                ],
                py_doc: [
                    "Current ``retry.max_elapsed`` value in seconds (default ``300``;",
                    "``0`` = disabled).",
                ],
                ts_doc: [
                    "Current `retry.max_elapsed` value in seconds (default `300n`;",
                    "`0n` = disabled).",
                ],
            }
            set thetadatadx_config_set_retry_initial_delay_ms(ms: u64) [retry.initial_delay in ms] {
                param: ms,
                doc: [
                    "Set the initial backoff delay (ms) for the market-data retry policy.",
                    "Default `250`. Subsequent retries double from here, capped at",
                    "`thetadatadx_config_set_retry_max_delay_ms`.",
                ],
                cpp_doc: [
                    "Initial backoff delay (ms) for the market-data retry policy. Default 250.",
                ],
                py_doc: [
                    "Set the initial backoff delay (ms) for the market-data-channel retry policy.",
                    "Default ``250``. Subsequent retries double from here, capped",
                    "at :attr:`retry_max_delay_ms`.",
                ],
                ts_doc: [
                    "Set the initial backoff delay (ms) for the market-data-channel retry policy.",
                    "Default `250n`. Subsequent retries double from here, capped at",
                    "`retryMaxDelayMs`.",
                ],
            }
            set thetadatadx_config_set_retry_max_delay_ms(ms: u64) [retry.max_delay in ms] {
                param: ms,
                doc: [
                    "Set the upper-bound backoff delay (ms) for the market-data retry policy.",
                    "Default `30_000` (30 s). The exponential schedule never exceeds",
                    "this value regardless of attempt number.",
                ],
                cpp_doc: [
                    "Upper-bound backoff delay (ms). Default 30_000 (30 s).",
                ],
                py_doc: [
                    "Set the upper-bound backoff delay (ms) for the",
                    "market-data-channel retry policy. Default ``30_000`` (30 s).",
                ],
                ts_doc: [
                    "Set the upper-bound backoff delay (ms) for the market-data retry",
                    "policy. Default `30_000n` (30 s).",
                ],
            }
            set thetadatadx_config_set_retry_max_attempts(n: u32) [retry.max_attempts] {
                param: n,
                doc: [
                    "Set the total attempt budget for the market-data retry policy. `1`",
                    "disables retry (single call only); higher values permit",
                    "retries up to `max_attempts - 1` after the initial call. Default",
                    "`20`.",
                ],
                cpp_doc: [
                    "Total attempt budget. 1 disables retry. Default 20.",
                ],
                py_doc: [
                    "Set the total attempt budget for the market-data-channel retry policy. ``1``",
                    "disables retry (single call only); higher values permit retries",
                    "up to ``max_attempts - 1`` after the initial call. Default ``20``.",
                ],
                ts_doc: [
                    "Set the total attempt budget for the market-data-channel retry policy. `1`",
                    "disables retry; higher values permit retries up to",
                    "`maxAttempts - 1` after the initial call. Default `20`.",
                ],
            }
            get thetadatadx_config_get_retry_max_attempts(out_n: u32) [retry.max_attempts] {
                doc: [
                    "Read the current `retry.max_attempts` setting.",
                ],
                cpp_doc: [
                    "",
                ],
                py_doc: [
                    "Current ``retry.max_attempts`` value.",
                ],
                ts_doc: [
                    "Current `retry.max_attempts` value.",
                ],
            }
            set thetadatadx_config_set_retry_jitter(jitter: bool) [retry.jitter] {
                param: jitter,
                doc: [
                    "Toggle AWS-style full-jitter on the market-data retry policy. Default",
                    "`true`. With `jitter=false` the backoff schedule is deterministic",
                    "(`min(max_delay, initial * 2^attempt)`), which is useful for tests",
                    "that need to assert exact timings.",
                ],
                cpp_doc: [
                    "AWS-style full jitter toggle. Default true.",
                ],
                py_doc: [
                    "Toggle AWS-style full-jitter on the market-data-channel retry policy. Default",
                    "``True``. ``False`` gives the deterministic backoff schedule",
                    "``min(max_delay, initial * 2^attempt)``, useful for tests that",
                    "need to assert exact timings.",
                ],
                ts_doc: [
                    "Toggle AWS-style full-jitter on the market-data-channel retry policy. Default",
                    "`true`. `false` gives the deterministic backoff schedule",
                    "`min(max_delay, initial * 2^attempt)`, useful for tests that",
                    "need to assert exact timings.",
                ],
            }
            get thetadatadx_config_get_retry_jitter(out_jitter: bool) [retry.jitter] {
                doc: [
                    "Read the current `retry.jitter` setting.",
                ],
                cpp_doc: [
                    "",
                ],
                py_doc: [
                    "Current ``retry.jitter`` value.",
                ],
                ts_doc: [
                    "Current `retry.jitter` value.",
                ],
            }
            set thetadatadx_config_set_market_data_port(port: u16) [market_data.port] {
                param: port,
                doc: [
                    "Set the market-data gRPC port on a config handle.",
                ],
                cpp_doc: [
                    "Set the market-data gRPC port. Companion to",
                    "@c set_market_data_host.",
                ],
                py_doc: [
                    "Set the market-data gRPC port. The environment factories set it;",
                    "override it together with ``market_data_host`` to reach the",
                    "market-data service at another address.",
                ],
                ts_doc: [
                    "Override the market-data gRPC port. Companion to `setMarketDataHost`:",
                    "same test-only rationale. Rejects values outside the `0..=65535`",
                    "port range.",
                ],
            }
            get thetadatadx_config_get_market_data_port(out_port: u16) [market_data.port] {
                doc: [
                    "Read the configured market-data gRPC port. Writes the value",
                    "into `*out_port`. Returns `0` on success, `-1` if either pointer is",
                    "null.",
                ],
                cpp_doc: [
                    "Current market-data gRPC port. Returns 0 on a null handle.",
                ],
                py_doc: [
                    "Current market-data gRPC port.",
                ],
                ts_doc: [
                    "Current market-data gRPC port.",
                ],
            }
            set thetadatadx_config_set_warn_on_buffered_threshold_bytes(n: usize) [market_data.warn_on_buffered_threshold_bytes] {
                param: n,
                doc: [
                    "Set the `warn_on_buffered_threshold_bytes` ceiling on a config",
                    "handle. A buffered (non-streaming) endpoint logs a warning when a",
                    "response's decoded total size exceeds this threshold (default",
                    "100 MiB), pointing large pulls at the streaming variant; the data is",
                    "still delivered.",
                    "",
                    "`n = 0` disables the warning entirely.",
                ],
                cpp_doc: [
                    "Set the warn_on_buffered_threshold_bytes ceiling.",
                    "",
                    "Buffered (non-streaming) endpoints log a warning when a response's",
                    "decoded total exceeds this threshold, guiding users to the streaming",
                    "variant. The payload is still delivered.",
                    "",
                    "@p n = 0 disables the warning entirely.",
                    "Default is `100 * 1024 * 1024` (100 MiB).",
                ],
                py_doc: [
                    "Set the warning threshold (in bytes) for buffered (non-streaming)",
                    "historical responses. Endpoints whose decoded total exceeds this",
                    "value log a warning pointing the caller at the",
                    "``.stream()`` surface; the data is still delivered. ``0``",
                    "disables the warning entirely. Default is ``100 * 1024 * 1024``",
                    "(100 MiB).",
                    "",
                    "Examples",
                    "--------",
                    "    cfg = Config.production()",
                    "    cfg.warn_on_buffered_threshold_bytes = 50 * 1024 * 1024",
                ],
                ts_doc: [
                    "Set the warning threshold (in bytes) for buffered (non-streaming)",
                    "historical responses. Endpoints whose decoded total exceeds this",
                    "value log a warning pointing the caller at the",
                    "matching `<endpoint>Stream(...)` method (e.g. `optionHistoryTradeStream`),",
                    "which delivers the same rows chunk-by-chunk through a callback with",
                    "memory bounded to a single chunk; the buffered data is still",
                    "delivered. `0n` disables the warning entirely. Default is",
                    "`100n * 1024n * 1024n` (100 MiB). Byte budgets can exceed the",
                    "32-bit unsigned range, so the setter takes a `BigInt`.",
                ],
            }
            get thetadatadx_config_get_warn_on_buffered_threshold_bytes(out_n: usize) [market_data.warn_on_buffered_threshold_bytes] {
                doc: [
                    "Read the current `warn_on_buffered_threshold_bytes` setting.",
                    "",
                    "Writes the configured byte count into `*out_n`. Returns `0` on",
                    "success, `-1` if either pointer is null.",
                ],
                cpp_doc: [
                    "Read the current `warn_on_buffered_threshold_bytes` setting.",
                    "",
                    "Returns the configured byte count, or `0` on a null handle",
                    "(matching the C ABI's `-1` failure mapping at the boundary).",
                ],
                py_doc: [
                    "Current ``warn_on_buffered_threshold_bytes`` setting (bytes).",
                ],
                ts_doc: [
                    "Current `warn_on_buffered_threshold_bytes` setting (bytes,",
                    "returned as a `BigInt`).",
                ],
            }
            set thetadatadx_config_set_request_timeout_secs(secs: u64) [market_data.request_timeout_secs] {
                param: secs,
                doc: [
                    "Set the default per-request deadline (seconds) for market-data queries",
                    "on a config handle.",
                    "",
                    "Bounds every request that did not set its own deadline, so a",
                    "live-but-silent stream resolves to a timeout instead of blocking",
                    "forever (the gRPC keepalive PING only detects a fully dead peer).",
                    "`secs = 0` no longer disables the default: it is floored to the",
                    "`300`-second default at request time, so the deadline always applies.",
                    "Default `300`.",
                ],
                cpp_doc: [
                    "Set the default per-request deadline (seconds) for market-data queries.",
                    "",
                    "Bounds every request that did not set its own deadline, so a",
                    "live-but-silent stream resolves to a timeout instead of blocking",
                    "forever. @p secs = 0 no longer disables the default; it is floored to",
                    "the `300`-second default at request time. Default is `300`.",
                ],
                py_doc: [
                    "Set the default per-request deadline (in seconds) for market-data",
                    "queries. Bounds every request that did not set its own",
                    "``timeout_ms``, so a live-but-silent stream resolves to a",
                    "timeout instead of blocking forever. ``0`` no longer disables the",
                    "default: it is floored to the ``300``-second default at request time,",
                    "so the deadline always applies. Default is ``300`` (5 minutes).",
                    "",
                    "Examples",
                    "--------",
                    "    cfg = Config.production()",
                    "    cfg.request_timeout_secs = 120",
                ],
                ts_doc: [
                    "Set the default per-request deadline (seconds) for market-data",
                    "queries. Bounds every request that did not set its own deadline,",
                    "so a live-but-silent stream resolves to a timeout instead of",
                    "blocking forever. `0n` no longer disables the default; it is floored",
                    "to the `300n`-second default at request time. Default `300n`",
                    "(5 minutes). Seconds are taken as a `BigInt` for parity with the",
                    "other `*Secs` knobs.",
                ],
            }
            get thetadatadx_config_get_request_timeout_secs(out: u64) [market_data.request_timeout_secs] {
                doc: [
                    "Read the current market-data `request_timeout_secs` setting (default",
                    "`300`). A stored `0` is floored to the `300`-second default at request",
                    "time rather than disabling the deadline.",
                    "",
                    "Writes the configured value into `*out`. Returns `0` on success,",
                    "`-1` if either pointer is null.",
                ],
                cpp_doc: [
                    "Read the current market-data `request_timeout_secs` setting.",
                    "",
                    "Returns the configured seconds (a stored `0` is floored to the",
                    "`300`-second default at request time), or `0` on a null handle",
                    "(matching the C ABI's `-1` failure mapping at the boundary).",
                ],
                py_doc: [
                    "Current market-data ``request_timeout_secs`` setting. A stored ``0``",
                    "is floored to the ``300``-second default at request time rather than",
                    "disabling the deadline.",
                ],
                ts_doc: [
                    "Current market-data `request_timeout_secs` setting in seconds",
                    "(default `300n`). A stored `0n` is floored to the `300n`-second",
                    "default at request time rather than disabling the deadline.",
                ],
            }
            set thetadatadx_config_set_market_data_stream_window_size_kb(kb: usize) [market_data.stream_window_size_kb] {
                param: kb,
                doc: [
                    "Set the initial per-stream HTTP/2 flow-control window, in KB, for the",
                    "market-data gRPC channel on a config handle. A larger window raises the",
                    "throughput ceiling on bulk streaming pulls before HTTP/2 backpressure",
                    "kicks in. Default `8192` (8 MiB); the value is clamped into",
                    "`[64, 2_097_151]` KB at connect.",
                ],
                cpp_doc: [
                    "Set the initial per-stream HTTP/2 flow-control window, in KB, for the",
                    "market-data gRPC channel. A larger window raises the throughput ceiling",
                    "on bulk streaming pulls before HTTP/2 backpressure kicks in.",
                    "@p kb is clamped into `[64, 2_097_151]` KB at validate/connect time.",
                    "Default is `8192` (8 MiB).",
                ],
                py_doc: [
                    "Set the initial per-stream HTTP/2 flow-control window (in KB) for the",
                    "market-data gRPC channel. A larger window raises the throughput ceiling",
                    "on bulk streaming pulls before HTTP/2 backpressure kicks in. The value",
                    "is clamped into ``[64, 2_097_151]`` KB at validate/connect time.",
                    "Default is ``8192`` (8 MiB).",
                    "",
                    "Examples",
                    "--------",
                    "    cfg = Config.production()",
                    "    cfg.market_data_stream_window_size_kb = 4096",
                ],
                ts_doc: [
                    "Set the initial per-stream HTTP/2 flow-control window (in KB) for the",
                    "market-data gRPC channel. A larger window raises the throughput ceiling",
                    "on bulk streaming pulls before HTTP/2 backpressure kicks in. The value",
                    "is clamped into `[64, 2_097_151]` KB at validate/connect time. Default",
                    "`8192n` (8 MiB). KB are taken as a `BigInt` for parity with the other",
                    "byte/KB-denominated knobs.",
                ],
            }
            get thetadatadx_config_get_market_data_stream_window_size_kb(out_kb: usize) [market_data.stream_window_size_kb] {
                doc: [
                    "Read the current per-stream HTTP/2 flow-control window (KB) for the",
                    "market-data gRPC channel (default `8192`).",
                    "",
                    "Writes the configured value into `*out_kb`. Returns `0` on success,",
                    "`-1` if either pointer is null.",
                ],
                cpp_doc: [
                    "Read the current per-stream HTTP/2 flow-control window (KB) for the",
                    "market-data gRPC channel.",
                    "",
                    "Returns the configured KB value, or `0` on a null handle (matching the",
                    "C ABI's `-1` failure mapping at the boundary).",
                ],
                py_doc: [
                    "Current per-stream HTTP/2 flow-control window (KB) for the market-data",
                    "gRPC channel (default ``8192``).",
                ],
                ts_doc: [
                    "Current per-stream HTTP/2 flow-control window (KB) for the market-data",
                    "gRPC channel (default `8192n`, returned as a `BigInt`).",
                ],
            }
            set thetadatadx_config_set_market_data_connection_window_size_kb(kb: usize) [market_data.connection_window_size_kb] {
                param: kb,
                doc: [
                    "Set the initial connection-level HTTP/2 flow-control window, in KB, for",
                    "the market-data gRPC channel on a config handle. A larger window raises",
                    "the throughput ceiling on bulk streaming pulls before HTTP/2",
                    "backpressure kicks in. Default `16384` (16 MiB); the value is clamped",
                    "into `[64, 2_097_151]` KB at connect.",
                ],
                cpp_doc: [
                    "Set the initial connection-level HTTP/2 flow-control window, in KB, for",
                    "the market-data gRPC channel. A larger window raises the throughput",
                    "ceiling on bulk streaming pulls before HTTP/2 backpressure kicks in.",
                    "@p kb is clamped into `[64, 2_097_151]` KB at validate/connect time.",
                    "Default is `16384` (16 MiB).",
                ],
                py_doc: [
                    "Set the initial connection-level HTTP/2 flow-control window (in KB) for",
                    "the market-data gRPC channel. A larger window raises the throughput",
                    "ceiling on bulk streaming pulls before HTTP/2 backpressure kicks in.",
                    "The value is clamped into ``[64, 2_097_151]`` KB at validate/connect",
                    "time. Default is ``16384`` (16 MiB).",
                    "",
                    "Examples",
                    "--------",
                    "    cfg = Config.production()",
                    "    cfg.market_data_connection_window_size_kb = 16384",
                ],
                ts_doc: [
                    "Set the initial connection-level HTTP/2 flow-control window (in KB) for",
                    "the market-data gRPC channel. A larger window raises the throughput",
                    "ceiling on bulk streaming pulls before HTTP/2 backpressure kicks in.",
                    "The value is clamped into `[64, 2_097_151]` KB at validate/connect",
                    "time. Default `16384n` (16 MiB). KB are taken as a `BigInt` for parity",
                    "with the other byte/KB-denominated knobs.",
                ],
            }
            get thetadatadx_config_get_market_data_connection_window_size_kb(out_kb: usize) [market_data.connection_window_size_kb] {
                doc: [
                    "Read the current connection-level HTTP/2 flow-control window (KB) for",
                    "the market-data gRPC channel (default `16384`).",
                    "",
                    "Writes the configured value into `*out_kb`. Returns `0` on success,",
                    "`-1` if either pointer is null.",
                ],
                cpp_doc: [
                    "Read the current connection-level HTTP/2 flow-control window (KB) for",
                    "the market-data gRPC channel.",
                    "",
                    "Returns the configured KB value, or `0` on a null handle (matching the",
                    "C ABI's `-1` failure mapping at the boundary).",
                ],
                py_doc: [
                    "Current connection-level HTTP/2 flow-control window (KB) for the",
                    "market-data gRPC channel (default ``16384``).",
                ],
                ts_doc: [
                    "Current connection-level HTTP/2 flow-control window (KB) for the",
                    "market-data gRPC channel (default `16384n`, returned as a `BigInt`).",
                ],
            }
            set thetadatadx_config_set_bulk_fetch(policy: i32) [market_data.bulk_fetch] {
                param: policy,
                values: BulkFetchPolicy { 0 Auto, 1 Off },
                errors: {
                    null: THETADATADX_ERR_CONFIG,
                    ffi: "expected 0 (Auto) or 1 (Off)",
                    py: "unknown bulk_fetch: {policy:?} (expected \"auto\" or \"off\")",
                    ts: "setBulkFetch: unknown policy {policy:?}; expected \"auto\" or \"off\"",
                },
                doc: [
                    "Set the automatic bulk-fetch sharding policy for history pulls, covering",
                    "both the buffered and the chunk-streaming call paths.",
                    "",
                    "- `policy = 0`: Auto (default); a large history pull's requested time",
                    "  or date range is split into equal concurrent bands across the",
                    "  account's concurrent-request budget, decided from the request shape",
                    "  alone (no sizing request is issued). Buffered",
                    "  pulls merge the shards back into exactly the rows of the single-stream",
                    "  response: single-contract, stock, and index pulls keep the exact",
                    "  single-stream row order, while option-chain pulls come back in a",
                    "  deterministic canonical order (expiration, strike, right ascending;",
                    "  calls before puts; time-ascending within each contract). Streaming",
                    "  pulls forward each band's chunks to the handler as they arrive: every",
                    "  chunk exactly once, but chunks from different bands interleave in",
                    "  arrival order rather than the single stream's order. Small pulls and",
                    "  non-history endpoints are never sharded.",
                    "- `policy = 1`: Off; every query runs as a single stream, in the",
                    "  server's own row and chunk order.",
                    "",
                    "Returns `0` on success. Returns `-1` and sets `thetadatadx_last_error` when",
                    "`policy` is outside the documented `{0, 1}` set or `config` is null. A",
                    "rejected `policy` value carries",
                    "`thetadatadx_last_error_code = THETADATADX_ERR_INVALID_PARAMETER` so an",
                    "out-of-domain enum int surfaces the same typed class across every binding.",
                ],
                cpp_doc: [
                    "Set the automatic bulk-fetch sharding policy for history pulls (buffered",
                    "and chunk-streaming alike): 0=Auto (default; a large pull's requested",
                    "time or date range is split into equal concurrent bands; buffered",
                    "pulls merge back into",
                    "exactly the single-stream rows, with option-chain pulls in a",
                    "deterministic canonical contract order and every other pull in the",
                    "exact single-stream row order; streaming pulls forward each band's",
                    "chunks as they arrive, exactly once but interleaved across bands in",
                    "arrival order), 1=Off (every query runs as a single stream, in the",
                    "server's own row and chunk order). Throws",
                    "@c thetadatadx::InvalidParameterError on an out-of-domain policy (and",
                    "@c thetadatadx::ConfigError on a null handle), routing through the typed",
                    "leaf the FFI error code selects.",
                ],
                py_doc: [
                    "Automatic bulk-fetch sharding policy for history pulls (buffered and",
                    "chunk-streaming alike). Accepts ``\"auto\"`` (default: a large history",
                    "pull's requested time or date range is split into equal concurrent",
                    "bands across the account's concurrent-request budget; buffered",
                    "pulls are merged back",
                    "into exactly the rows of the single-stream response, single-contract,",
                    "stock, and index pulls in the exact single-stream order and",
                    "option-chain pulls in a deterministic canonical order grouped by",
                    "expiration, strike, and right; streaming pulls forward each band's",
                    "chunks as they arrive, every chunk exactly once but interleaved",
                    "across bands in arrival order; small pulls are never sharded) or",
                    "``\"off\"`` (every query runs as a single stream, in the server's own",
                    "row and chunk order), case-insensitive.",
                ],
                ts_doc: [
                    "Automatic bulk-fetch sharding policy for history pulls (buffered and",
                    "chunk-streaming alike). Accepts `\"auto\"` (default: a large history",
                    "pull's requested time or date range is split into equal concurrent",
                    "bands across the account's concurrent-request budget; buffered pulls",
                    "are merged back into exactly",
                    "the rows of the single-stream response, single-contract, stock, and",
                    "index pulls in the exact single-stream order and option-chain pulls in",
                    "a deterministic canonical order grouped by expiration, strike, and",
                    "right; streaming pulls forward each band's chunks as they arrive, every",
                    "chunk exactly once but interleaved across bands in arrival order; small",
                    "pulls are never sharded) or `\"off\"` (every query runs as a single",
                    "stream, in the server's own row and chunk order), case-insensitive.",
                ],
            }
            get thetadatadx_config_get_bulk_fetch(out_policy: i32) [market_data.bulk_fetch] {
                values: BulkFetchPolicy { 0 Auto, 1 Off },
                doc: [
                    "Read the configured bulk-fetch sharding policy. Same encoding as",
                    "`thetadatadx_config_set_bulk_fetch`. Returns `0` on success, `-1` if",
                    "either pointer is null.",
                ],
                cpp_doc: [
                    "Current bulk-fetch sharding policy (same encoding as the setter).",
                ],
                py_doc: [
                    "Current bulk-fetch sharding policy as a lowercase string.",
                ],
                ts_doc: [
                    "Current bulk-fetch sharding policy as a lowercase string.",
                ],
            }
            set thetadatadx_config_set_shard_concurrency(n: Option<u32>) [market_data.shard_concurrency] {
                param: n,
                doc: [
                    "Set the upper bound on concurrent sub-requests per sharded bulk fetch.",
                    "",
                    "* `has_value = false` encodes `None` (the default): a sharded pull uses",
                    "  the full concurrent-request budget (the channel-pool size resolved",
                    "  from `max_concurrent_requests`). `n` is ignored.",
                    "* `has_value = true` encodes `Some(n)`: a sharded pull spreads across at",
                    "  most `n` sub-requests. The applied value is clamped into",
                    "  `[1, pool_size]` when a plan is built; the pool size is resolved",
                    "  from `max_concurrent_requests`; and validation floors an explicit",
                    "  `0` to `1`.",
                    "",
                    "Returns `0` on success, `-1` if `config` is null.",
                ],
                cpp_doc: [
                    "Set the upper bound on concurrent sub-requests per sharded bulk fetch.",
                    "Pass @c std::nullopt (the @c None default) to use the full",
                    "concurrent-request budget; pass an explicit @c std::uint32_t to cap the",
                    "fan-out. The applied value is clamped into `[1, pool_size]` when a plan",
                    "is built (the pool size is resolved from `max_concurrent_requests`).",
                    "",
                    "Throws a @c thetadatadx::ThetaDataError leaf on a null-handle FFI failure,",
                    "routing through the typed class the FFI error code selects.",
                ],
                py_doc: [
                    "Upper bound on concurrent sub-requests per sharded bulk fetch.",
                    "``None`` (the default) uses the full concurrent-request budget (the",
                    "channel-pool size resolved from ``max_concurrent_requests``); an",
                    "``int`` caps the fan-out. The applied value is clamped into",
                    "``[1, pool_size]`` when a plan is built, and validation floors an",
                    "explicit ``0`` to ``1``.",
                ],
                ts_doc: [
                    "Set the upper bound on concurrent sub-requests per sharded bulk fetch.",
                    "Pass `null` or `undefined` (the default) to use the full",
                    "concurrent-request budget (the channel-pool size resolved from",
                    "`maxConcurrentRequests`); pass a `number` to cap the fan-out. The",
                    "applied value is clamped into `[1, pool_size]` when a plan is built",
                    "(the pool size is resolved from `maxConcurrentRequests`).",
                ],
            }
            get thetadatadx_config_get_shard_concurrency(out_n: Option<u32>) [market_data.shard_concurrency] {
                doc: [
                    "Read the current `market_data.shard_concurrency` setting.",
                    "",
                    "* `*out_has_value = false` → the config holds `None` (full pool",
                    "  budget). `*out_n` is left as `0`.",
                    "* `*out_has_value = true` → the config holds `Some(*out_n)`.",
                    "",
                    "Returns `0` on success, `-1` if any pointer is null.",
                ],
                cpp_doc: [
                    "Read the current @c market_data.shard_concurrency setting. Returns",
                    "@c std::nullopt for the full-pool-budget sentinel; returns the wrapped",
                    "@c std::uint32_t when an explicit cap is set.",
                    "",
                    "Throws a @c thetadatadx::ThetaDataError leaf on a null-handle FFI failure,",
                    "routing through the typed class the FFI error code selects.",
                ],
                py_doc: [
                    "Current ``shard_concurrency`` setting. ``None`` means a sharded pull",
                    "uses the full concurrent-request budget; an ``int`` is the",
                    "configured cap.",
                ],
                ts_doc: [
                    "Current `shardConcurrency` setting. `null` means a sharded pull uses",
                    "the full concurrent-request budget; a `number` is the configured cap.",
                ],
            }
            set thetadatadx_config_set_max_concurrent_requests(n: Option<u32>) [market_data.max_concurrent_requests] {
                param: n,
                doc: [
                    "Set the number of concurrent in-flight market-data requests: the size",
                    "of the gRPC channel pool and of the request semaphore that admits",
                    "requests onto it, resolved at connect time.",
                    "",
                    "* `has_value = false` encodes `None` (the default): the pool is sized",
                    "  to the account's subscription tier from the auth response (Free 1 /",
                    "  Value 2 / Standard 4 / Pro 8). `n` is ignored.",
                    "* `has_value = true` encodes `Some(n)`: the pool is `n` verbatim, with",
                    "  no client-side cap; the allowance is enforced server-side, so an",
                    "  account boosted above its base tier sets the boosted value and runs",
                    "  that wide. Requests past the server's allowance surface as",
                    "  `ResourceExhausted` and are retried with backoff. Validation floors",
                    "  an explicit `0` to `1`.",
                    "",
                    "Returns `0` on success, `-1` if `config` is null.",
                ],
                cpp_doc: [
                    "Set the number of concurrent in-flight market-data requests (gRPC",
                    "channel-pool + request-semaphore size, resolved at connect time). Pass",
                    "@c std::nullopt (the @c None default) to size the pool to the account's",
                    "subscription tier from the auth response (Free 1 / Value 2 / Standard 4",
                    "/ Pro 8); pass an explicit @c std::uint32_t to use it verbatim; no",
                    "client-side cap, so a server-boosted account sets its boosted",
                    "allowance. Requests past the server-enforced allowance are rejected",
                    "upstream and retried with backoff. Validation floors an explicit `0`",
                    "to `1`.",
                    "",
                    "Throws a @c thetadatadx::ThetaDataError leaf on a null-handle FFI failure,",
                    "routing through the typed class the FFI error code selects.",
                ],
                py_doc: [
                    "Number of concurrent in-flight market-data requests (gRPC channel-pool",
                    "+ request-semaphore size, resolved at connect time). ``None`` (the",
                    "default) sizes the pool to the account's subscription tier from the",
                    "auth response (Free 1 / Value 2 / Standard 4 / Pro 8); an ``int`` is",
                    "used verbatim with no client-side cap; the allowance is enforced",
                    "server-side, so a boosted account sets its boosted value. Requests",
                    "past the allowance are retried with backoff. Validation floors an",
                    "explicit ``0`` to ``1``.",
                    "",
                    "Examples",
                    "--------",
                    "    cfg = Config.production()",
                    "    cfg.max_concurrent_requests = 32",
                ],
                ts_doc: [
                    "Set the number of concurrent in-flight market-data requests (gRPC",
                    "channel-pool + request-semaphore size, resolved at connect time). Pass",
                    "`null` or `undefined` (the default) to size the pool to the account's",
                    "subscription tier from the auth response (Free 1 / Value 2 / Standard 4",
                    "/ Pro 8); pass a `number` to use it verbatim; no client-side cap, so",
                    "a server-boosted account sets its boosted allowance. Requests past the",
                    "server-enforced allowance are retried with backoff. Validation floors",
                    "an explicit `0` to `1`.",
                ],
            }
            get thetadatadx_config_get_max_concurrent_requests(out_n: Option<u32>) [market_data.max_concurrent_requests] {
                doc: [
                    "Read the current `market_data.max_concurrent_requests` setting.",
                    "",
                    "* `*out_has_value = false` → the config holds `None` (size the pool to",
                    "  the account's subscription tier at connect time). `*out_n` is left",
                    "  as `0`.",
                    "* `*out_has_value = true` → the config holds `Some(*out_n)`.",
                    "",
                    "Returns `0` on success, `-1` if any pointer is null.",
                ],
                cpp_doc: [
                    "Read the current @c market_data.max_concurrent_requests setting.",
                    "Returns @c std::nullopt for the size-to-tier default; returns the",
                    "wrapped @c std::uint32_t when an explicit pool size is set.",
                    "",
                    "Throws a @c thetadatadx::ThetaDataError leaf on a null-handle FFI failure,",
                    "routing through the typed class the FFI error code selects.",
                ],
                py_doc: [
                    "Current ``max_concurrent_requests`` setting. ``None`` means the pool",
                    "is sized to the account's subscription tier at connect time; an",
                    "``int`` is the configured pool size.",
                ],
                ts_doc: [
                    "Current `maxConcurrentRequests` setting. `null` means the pool is",
                    "sized to the account's subscription tier at connect time; a `number`",
                    "is the configured pool size.",
                ],
            }
            get thetadatadx_config_get_retry_initial_delay_ms(out_ms: u64) [retry.initial_delay in ms] {
                doc: [
                    "Read the current `retry.initial_delay` setting (ms). Returns `0` on",
                    "success, `-1` if either pointer is null.",
                ],
                cpp_doc: [
                    "",
                ],
                py_doc: [
                    "Current ``retry.initial_delay`` value in ms.",
                ],
                ts_doc: [
                    "Current `retry.initial_delay` value (ms, returned as BigInt).",
                ],
            }
            get thetadatadx_config_get_retry_max_delay_ms(out_ms: u64) [retry.max_delay in ms] {
                doc: [
                    "Read the current `retry.max_delay` setting (ms).",
                ],
                cpp_doc: [
                    "",
                ],
                py_doc: [
                    "Current ``retry.max_delay`` value in ms.",
                ],
                ts_doc: [
                    "Current `retry.max_delay` value (ms, returned as BigInt).",
                ],
            }
            set thetadatadx_config_set_reconnect_max_attempts(max_attempts: u32) [limits.max_attempts] {
                param: max_attempts,
                doc: [
                    "Set the per-class transient-failure attempt budget for the",
                    "auto-reconnect path. Default `30`. No effect unless the reconnect",
                    "policy is `Auto`.",
                ],
                cpp_doc: [
                    "Set the per-class transient-failure attempt budget. Default 30.",
                ],
                py_doc: [
                    "Set the per-class transient-failure attempt budget for the",
                    "auto-reconnect path. Default `30`. No effect unless the",
                    "reconnect policy is `Auto`.",
                ],
                ts_doc: [
                    "Set the per-class transient-failure attempt budget for the",
                    "auto-reconnect path. Default `30`. No effect unless the",
                    "reconnect policy is `Auto`.",
                ],
            }
            get thetadatadx_config_get_reconnect_max_attempts(out: u32) [limits.max_attempts] {
                doc: [
                    "Read the generic-transient reconnect attempt budget. Default `30`.",
                    "",
                    "Writes the configured value into `*out`. When the reconnect policy",
                    "is not `Auto`, writes the default-limits value (the budgets only",
                    "apply under the `Auto` policy). Returns `0` on success, `-1` if",
                    "either pointer is null.",
                ],
                cpp_doc: [
                    "Current generic-transient reconnect attempt budget (default 30).",
                ],
                py_doc: [
                    "Current generic-transient reconnect attempt budget (default",
                    "`30`). Reads the default-limits value when the policy is not",
                    "`Auto`.",
                ],
                ts_doc: [
                    "Current generic-transient reconnect attempt budget (default",
                    "`30`). Reads the default-limits value when the policy is not",
                    "`Auto`.",
                ],
            }
            set thetadatadx_config_set_reconnect_max_rate_limited_attempts(max_rate_limited_attempts: u32) [limits.max_rate_limited_attempts] {
                param: max_rate_limited_attempts,
                doc: [
                    "Set the per-class rate-limited (`TooManyRequests`) attempt budget",
                    "for the auto-reconnect path. Default `100`. No effect unless the",
                    "reconnect policy is `Auto`.",
                ],
                cpp_doc: [
                    "Set the rate-limited (TooManyRequests) attempt budget. Default 100.",
                ],
                py_doc: [
                    "Set the per-class rate-limited (`TooManyRequests`) attempt budget",
                    "for the auto-reconnect path. Default `100`. No effect unless the",
                    "reconnect policy is `Auto`.",
                ],
                ts_doc: [
                    "Set the per-class rate-limited (`TooManyRequests`) attempt",
                    "budget for the auto-reconnect path. Default `100`. No effect",
                    "unless the reconnect policy is `Auto`.",
                ],
            }
            get thetadatadx_config_get_reconnect_max_rate_limited_attempts(out: u32) [limits.max_rate_limited_attempts] {
                doc: [
                    "Read the rate-limited (`TooManyRequests`) reconnect attempt budget. Default `100`.",
                    "",
                    "Writes the configured value into `*out`. When the reconnect policy",
                    "is not `Auto`, writes the default-limits value (the budgets only",
                    "apply under the `Auto` policy). Returns `0` on success, `-1` if",
                    "either pointer is null.",
                ],
                cpp_doc: [
                    "Current rate-limited reconnect attempt budget (default 100).",
                ],
                py_doc: [
                    "Current rate-limited reconnect attempt budget (default `100`).",
                    "Reads the default-limits value when the policy is not `Auto`.",
                ],
                ts_doc: [
                    "Current rate-limited reconnect attempt budget (default `100`).",
                    "Reads the default-limits value when the policy is not `Auto`.",
                ],
            }
            set thetadatadx_config_set_reconnect_max_server_restart_attempts(n: u32) [limits.max_server_restart_attempts] {
                param: n,
                doc: [
                    "Set the `ServerRestarting` reconnect attempt budget. Default `60`. No effect unless the reconnect policy is `Auto`.",
                ],
                cpp_doc: [
                    "Set the ServerRestarting reconnect attempt budget. Default 60.",
                ],
                py_doc: [
                    "Set the ``ServerRestarting`` reconnect attempt budget. Default",
                    "``60``. No effect unless the reconnect policy is ``Auto``.",
                ],
                ts_doc: [
                    "Set the `ServerRestarting` reconnect attempt budget. Default",
                    "`60`. No effect unless the reconnect policy is `Auto`.",
                ],
            }
            get thetadatadx_config_get_reconnect_max_server_restart_attempts(out: u32) [limits.max_server_restart_attempts] {
                doc: [
                    "Read the `ServerRestarting` reconnect attempt budget. Default `60`.",
                    "",
                    "Writes the configured value into `*out`. When the reconnect policy",
                    "is not `Auto`, writes the default-limits value (the budgets only",
                    "apply under the `Auto` policy). Returns `0` on success, `-1` if",
                    "either pointer is null.",
                ],
                cpp_doc: [
                    "Current ServerRestarting reconnect attempt budget (default 60).",
                ],
                py_doc: [
                    "Current ``ServerRestarting`` reconnect attempt budget (default",
                    "``60``). Reads the default-limits value when the policy is not",
                    "``Auto``.",
                ],
                ts_doc: [
                    "Current `ServerRestarting` reconnect attempt budget (default",
                    "`60`). Reads the default-limits value when the policy is not",
                    "`Auto`.",
                ],
            }
            set thetadatadx_config_set_reconnect_max_elapsed_secs(secs: u64) [limits.max_elapsed in secs] {
                param: secs,
                doc: [
                    "Set the wall-clock reconnect envelope (seconds) for the generic-transient and server-restart classes, measured from the first attempt of a consecutive-reconnect sequence. `0` disables the envelope (attempt budgets only). Default `300`. No effect unless the reconnect policy is `Auto`.",
                ],
                cpp_doc: [
                    "Set the wall-clock reconnect envelope (seconds) for the",
                    "generic-transient and server-restart classes. 0 disables the",
                    "envelope (attempt budgets only). Default 300.",
                ],
                py_doc: [
                    "Set the wall-clock reconnect envelope (seconds) for the",
                    "generic-transient and server-restart classes, measured from the",
                    "first attempt of a consecutive-reconnect sequence. ``0``",
                    "disables the envelope (attempt budgets only). Default ``300``.",
                    "No effect unless the reconnect policy is ``Auto``.",
                ],
                ts_doc: [
                    "Set the wall-clock reconnect envelope (seconds) for the",
                    "generic-transient and server-restart classes, measured from the",
                    "first attempt of a consecutive-reconnect sequence. `0n` disables",
                    "the envelope (attempt budgets only). Default `300n`. No effect",
                    "unless the reconnect policy is `Auto`.",
                ],
            }
            get thetadatadx_config_get_reconnect_max_elapsed_secs(out: u64) [limits.max_elapsed in secs] {
                doc: [
                    "Read the wall-clock reconnect envelope (seconds). `0` means the envelope is disabled. Default `300`.",
                    "",
                    "Writes the configured value into `*out`. When the reconnect policy",
                    "is not `Auto`, writes the default-limits value (the budgets only",
                    "apply under the `Auto` policy). Returns `0` on success, `-1` if",
                    "either pointer is null.",
                ],
                cpp_doc: [
                    "Current wall-clock reconnect envelope in seconds (default 300;",
                    "0 = disabled).",
                ],
                py_doc: [
                    "Current wall-clock reconnect envelope in seconds (default",
                    "``300``; ``0`` = disabled). Reads the default-limits value when",
                    "the policy is not ``Auto``.",
                ],
                ts_doc: [
                    "Current wall-clock reconnect envelope in seconds (default",
                    "`300n`; `0n` = disabled). Reads the default-limits value when",
                    "the policy is not `Auto`.",
                ],
            }
            set thetadatadx_config_set_reconnect_stable_window_secs(secs: u64) [limits.stable_window in secs] {
                param: secs,
                doc: [
                    "Set the continuous successful-data-flow window (in seconds) after",
                    "which the auto-reconnect attempt counters reset. Default `60`. No",
                    "effect unless the reconnect policy is `Auto`.",
                ],
                cpp_doc: [
                    "Set the stable-window timer (seconds) after which the",
                    "auto-reconnect attempt counters reset. Default 60.",
                ],
                py_doc: [
                    "Set the continuous successful-data-flow window (in seconds)",
                    "after which the auto-reconnect attempt counters reset. Default",
                    "``60``. No effect unless the reconnect policy is ``Auto``.",
                ],
                ts_doc: [
                    "Set the continuous successful-data-flow window (in seconds)",
                    "after which the auto-reconnect attempt counters reset. Default",
                    "`60`. No effect unless the reconnect policy is `Auto`.",
                    "",
                    "Accepts a `bigint` for parity with the Python / C++ / FFI",
                    "surface, which uses a 64-bit unsigned integer. JavaScript `Number` callers should wrap their",
                    "value: `setReconnectStableWindowSecs(BigInt(60))`.",
                ],
            }
            get thetadatadx_config_get_reconnect_stable_window_secs(out: u64) [limits.stable_window in secs] {
                doc: [
                    "Read the stable-window reset interval (seconds). Default `60`.",
                    "",
                    "Writes the configured value into `*out`. When the reconnect policy",
                    "is not `Auto`, writes the default-limits value (the budgets only",
                    "apply under the `Auto` policy). Returns `0` on success, `-1` if",
                    "either pointer is null.",
                ],
                cpp_doc: [
                    "Current stable-window reset interval in seconds (default 60).",
                ],
                py_doc: [
                    "Current stable-window reset interval in seconds (default `60`).",
                    "Reads the default-limits value when the policy is not `Auto`.",
                ],
                ts_doc: [
                    "Current stable-window reset interval in seconds (default `60n`).",
                    "Reads the default-limits value when the policy is not `Auto`.",
                ],
            }
            set thetadatadx_config_set_nexus_url(url: String) [auth.nexus_url] {
                param: url,
                doc: [
                    "Set the Nexus auth URL on a config handle.",
                    "",
                    "`url` must be a non-null, NUL-terminated, valid-UTF-8 C string.",
                    "Returns `0` on success, `-1` if `config` is null or `url` is",
                    "null / not valid UTF-8 (the diagnostic is written to thread-local",
                    "storage retrievable via `thetadatadx_last_error()`).",
                ],
                cpp_doc: [
                    "Set the Nexus auth URL. Default is the upstream production",
                    "endpoint; redirect at a staging cluster for testing.",
                    "",
                    "Throws a @c thetadatadx::ThetaDataError leaf if the FFI rejects the value",
                    "(null handle or non-UTF-8 input), routing through the typed class",
                    "the FFI error code selects.",
                ],
                py_doc: [
                    "Set the Nexus auth URL. Default matches the upstream production",
                    "endpoint; override to redirect at a staging cluster.",
                ],
                ts_doc: [
                    "Set the Nexus auth URL. Default matches the upstream",
                    "production endpoint; override to redirect at a staging",
                    "cluster for testing.",
                ],
            }
            get thetadatadx_config_get_nexus_url(out: String) [auth.nexus_url] {
                doc: [
                    "Read the current `auth.nexus_url` setting.",
                    "",
                    "On success, returns a heap-owned NUL-terminated C string the",
                    "caller MUST release with `thetadatadx_string_free`. Returns null if",
                    "`config` is null or the stored value contains an interior NUL",
                    "(the diagnostic is written to `thetadatadx_last_error()`).",
                ],
                cpp_doc: [
                    "Current @c auth.nexus_url. Returns an empty string if the FFI",
                    "getter returns null (null handle or interior-NUL value).",
                ],
                py_doc: [
                    "Current ``auth.nexus_url`` value.",
                ],
                ts_doc: [
                    "Current `auth.nexus_url` value.",
                ],
            }
            set thetadatadx_config_set_client_type(client_type: String) [auth.client_type] {
                param: client_type,
                doc: [
                    "Set the `QueryInfo.client_type` identifier on a config handle.",
                    "",
                    "`client_type` must be a non-null, NUL-terminated, valid-UTF-8 C",
                    "string. Returns `0` on success, `-1` if `config` is null or",
                    "`client_type` is null / not valid UTF-8 (the diagnostic is written",
                    "to thread-local storage retrievable via `thetadatadx_last_error()`).",
                ],
                cpp_doc: [
                    "Set the QueryInfo.client_type identifier. Default is",
                    "@c \"rust-thetadatadx\"; override to identify a deployment fleet",
                    "in server-side dashboards.",
                    "",
                    "Throws a @c thetadatadx::ThetaDataError leaf if the FFI rejects the value",
                    "(null handle or non-UTF-8 input), routing through the typed class",
                    "the FFI error code selects.",
                ],
                py_doc: [
                    "Set the ``QueryInfo.client_type`` identifier. Default is",
                    "``\"rust-thetadatadx\"``; override to identify a deployment fleet",
                    "in server-side dashboards.",
                ],
                ts_doc: [
                    "Set the `QueryInfo.client_type` identifier. Default is",
                    "`\"rust-thetadatadx\"`; override to identify a deployment fleet",
                    "in server-side dashboards.",
                ],
            }
            get thetadatadx_config_get_client_type(out: String) [auth.client_type] {
                doc: [
                    "Read the current `auth.client_type` setting.",
                    "",
                    "On success, returns a heap-owned NUL-terminated C string the",
                    "caller MUST release with `thetadatadx_string_free`. Returns null if",
                    "`config` is null or the stored value contains an interior NUL",
                    "(the diagnostic is written to `thetadatadx_last_error()`).",
                ],
                cpp_doc: [
                    "Current @c auth.client_type. Returns an empty string if the FFI",
                    "getter returns null (null handle or interior-NUL value).",
                ],
                py_doc: [
                    "Current ``auth.client_type`` value.",
                ],
                ts_doc: [
                    "Current `auth.client_type` value.",
                ],
            }
            set thetadatadx_config_set_market_data_host(host: String) [set_market_data_host()] {
                param: host,
                doc: [
                    "Set the market-data gRPC host on a config handle.",
                    "",
                    "`host` must be a non-null, NUL-terminated, valid-UTF-8 C string.",
                    "Returns `0` on success, `-1` if `config` is null or `host` is",
                    "null / not valid UTF-8 (the diagnostic is written to thread-local",
                    "storage retrievable via `thetadatadx_last_error()`).",
                ],
                cpp_doc: [
                    "Set the market-data gRPC host. Defaults to the upstream",
                    "production endpoint; redirect the market-data channel at a known",
                    "host for testing. Throws a @c thetadatadx::ThetaDataError leaf if the FFI",
                    "rejects the value (null handle or non-UTF-8 input).",
                ],
                py_doc: [
                    "Set the market-data gRPC host. The environment factories set it;",
                    "override it to reach the market-data service at another address.",
                ],
                ts_doc: [
                    "Override the market-data gRPC host. Companion to `setMarketDataPort`.",
                ],
            }
            get thetadatadx_config_get_market_data_host(out: String) [market_data_host()] {
                doc: [
                    "Read the current market-data gRPC host.",
                    "",
                    "On success, returns a heap-owned NUL-terminated C string the caller",
                    "MUST release with `thetadatadx_string_free`. Returns null if `config` is",
                    "null or the stored value contains an interior NUL (the diagnostic is",
                    "written to `thetadatadx_last_error()`).",
                ],
                cpp_doc: [
                    "Current market-data gRPC host. Returns an empty string if",
                    "the FFI getter returns null (null handle or interior-NUL value).",
                ],
                py_doc: [
                    "Current market-data gRPC host.",
                ],
                ts_doc: [
                    "Current market-data gRPC host.",
                ],
            }
            set thetadatadx_config_set_reconnect_jitter(mode: i32) [reconnect.jitter] {
                param: mode,
                values: JitterMode { 0 Full, 1 None },
                errors: {
                    null: THETADATADX_ERR_OTHER,
                    ffi: "expected 0 (Full) or 1 (None)",
                    py: "unknown reconnect_jitter: {mode:?} (expected \"full\" or \"none\")",
                    ts: "setReconnectJitter: unknown mode {mode:?}; expected \"full\" or \"none\"",
                },
                doc: [
                    "Set the jitter strategy applied to every reconnect delay.",
                    "",
                    "- `mode = 0`: Full (default); sample uniformly from `[0, delay]`.",
                    "- `mode = 1`: None; deterministic delays (tests only).",
                    "",
                    "Returns `0` on success. Returns `-1` and sets `thetadatadx_last_error` when",
                    "`mode` is outside the documented `{0, 1}` set or `config` is",
                    "null. A rejected `mode` value carries",
                    "`thetadatadx_last_error_code = THETADATADX_ERR_INVALID_PARAMETER` so an out-of-domain",
                    "enum int surfaces the same typed class across every binding.",
                ],
                cpp_doc: [
                    "Set the reconnect jitter mode: 0=Full (default), 1=None. Throws @c thetadatadx::InvalidParameterError on",
                    "an out-of-domain mode (and @c thetadatadx::ThetaDataError on a null",
                    "handle), routing through the typed leaf the FFI error code",
                    "selects.",
                ],
                py_doc: [
                    "Set the jitter strategy applied to every reconnect delay.",
                    "Accepts ``\"full\"`` (default) or ``\"none\"`` (case-insensitive).",
                ],
                ts_doc: [
                    "Set the jitter strategy applied to every reconnect delay.",
                    "Accepts `\"full\"` (default) or `\"none\"` (case-insensitive).",
                ],
            }
            get thetadatadx_config_get_reconnect_jitter(out_mode: i32) [reconnect.jitter] {
                values: JitterMode { 0 Full, 1 None },
                doc: [
                    "Read the configured reconnect jitter mode. Same encoding as",
                    "`thetadatadx_config_set_reconnect_jitter`. Returns `0` on success, `-1` if",
                    "either pointer is null.",
                ],
                cpp_doc: [
                    "Current reconnect jitter mode (same encoding as the setter).",
                ],
                py_doc: [
                    "Current reconnect jitter mode as a lowercase string.",
                ],
                ts_doc: [
                    "Current reconnect jitter mode as a lowercase string.",
                ],
            }
        }
    };
}
