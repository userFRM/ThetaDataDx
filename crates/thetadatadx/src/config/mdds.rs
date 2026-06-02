//! MDDS (gRPC) sub-configuration.
//!
//! Three knobs control SDK-side throughput on large historical pulls
//! (multi-day backfills, wide `strike_range`, `interval = 1s` /
//! `tick`):
//!
//! | Workload                                        | `concurrent_requests` | `decode_threads` | `decoder_ring_size` |
//! |-------------------------------------------------|-----------------------|------------------|---------------------|
//! | One-shot single-day single-strike query         | `1`                   | auto             | default (256)       |
//! | Multi-day backfill, narrow strike scope (sr<10) | `4` (PRO)             | auto             | default (256)       |
//! | Wide `strike_range` or `1s`/`tick` interval bulk| `8` (PRO max)         | `8`              | default (256)       |
//! | Server-side tier caps                           | FREE=1 / VALUE=2 / STANDARD=4 / PRO=8 |  |                     |
//!
//! `concurrent_requests` is clamped to the resolved subscription
//! tier cap at connect time (a setting of `32` on a PRO tier opens
//! 8 channels and emits a `tracing::warn!`).
//!
//! See `docs-site/docs/configuration.md` for the per-binding setter
//! samples.

/// MDDS client tuning.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct MddsConfig {
    /// MDDS hostname (v3 path).
    pub host: String,

    /// MDDS port (443 for TLS in production).
    pub port: u16,

    /// Whether to use TLS for the MDDS connection.
    /// Always `true` in production (standard gRPC-over-TLS on port 443).
    pub tls: bool,

    /// Max concurrent in-flight gRPC requests.
    ///
    /// JVM equivalent: `2^subscription_tier` (Free=1, Value=2, Standard=4, Pro=8).
    /// Set to 0 to auto-detect from the subscription tier returned by Nexus auth.
    pub concurrent_requests: usize,

    /// Max inbound gRPC message size in bytes.
    ///
    /// JVM equivalent: `maxInboundMessageSize(0x100000 * config.messageSize())`,
    /// default 4MB, max 10MB.
    pub max_message_size: usize,

    /// gRPC keepalive interval in seconds (`keepAliveTime(30, SECONDS)`).
    pub keepalive_secs: u64,

    /// gRPC keepalive timeout in seconds (`keepAliveTimeout(10, SECONDS)`).
    pub keepalive_timeout_secs: u64,

    /// gRPC flow control: initial stream window size in KB.
    ///
    /// Maps to the underlying transport's initial_stream_window_size`.
    /// Default 64 KB matches HTTP/2 spec default.
    pub window_size_kb: usize,

    /// gRPC flow control: initial connection window size in KB.
    ///
    /// Maps to the underlying transport's initial_connection_window_size`.
    /// Default 64 KB. Increase for high-throughput bulk queries.
    pub connection_window_size_kb: usize,

    /// TCP connect timeout for the MDDS channel, in seconds.
    ///
    /// Bounds the time the transport will spend establishing a TCP +
    /// TLS handshake before failing fast. Default `10s` matches the upper
    /// bound observed on the wire; production deployments behind NAT / VPN
    /// can raise this to absorb slow handshakes without altering keepalive
    /// cadence.
    pub connect_timeout_secs: u64,

    /// Stage-2 worker thread count for the two-stage decode
    /// pipeline. Stage-2 runs `prost::Message::decode` and the
    /// downstream Tick build off a bounded MPSC queue fed by the
    /// stage-1 decoder threads.
    ///
    /// `None` (the default) sizes the pool to
    /// [`std::thread::available_parallelism`] with a minimum of `1`,
    /// fanning parsing work across every logical core when the
    /// workload is parser-bound rather than IO-bound. `Some(0)` clamps
    /// to `1` (a zero-worker
    /// pool would deadlock stage-1 on the first push). `Some(n)`
    /// pins the worker count to `n` regardless of the available
    /// core count — useful on shared hosts where
    /// `available_parallelism` reads the wrong number from `/proc`.
    pub decode_threads: Option<usize>,

    /// Bounded queue depth between stage-1 (zstd decompress) and
    /// stage-2 (prost decode + Tick build). When stage-2 cannot
    /// keep up, stage-1's `send()` parks the decoder thread rather
    /// than dropping the payload — silent drops on a market data
    /// feed are unacceptable, so the queue prefers backpressure.
    ///
    /// `None` (the default) sizes the queue to
    /// `concurrent_requests * 64`, picked so a 64-way burst on
    /// every configured channel pool has a chunk-worth of headroom
    /// without leaving stage-2 starved. `Some(n)` pins the depth to
    /// `n` slots; `Some(0)` clamps to `1` (a zero-slot queue
    /// degenerates to a rendezvous channel — still backpressure-
    /// preserving but no buffer).
    pub decode_queue_depth: Option<usize>,

    /// Per-thread decoder ring size. Must be a power of two `>= 64`.
    ///
    /// Larger rings absorb burstier IO without back-pressuring the
    /// h2 receive task; smaller rings reduce memory footprint. `256`
    /// is the production default — enough headroom for a 64-way
    /// burst across 4 channels to land on the same decoder thread
    /// without queue-full back-pressure.
    pub decoder_ring_size: usize,

    /// Estimated-bytes threshold above which the buffered `.await`
    /// path on a `parsed_endpoint!` builder emits a single
    /// `tracing::warn!` event suggesting `.stream(handler)` for the
    /// workload.
    ///
    /// The buffered path materializes the full response as
    /// `Vec<Tick>` before returning; the streaming path drops each
    /// chunk after the user callback consumes it. When
    /// `row_count * size_of::<Tick>() > threshold`, the SDK logs an
    /// `endpoint = ..., row_count = ..., bytes_est = ...` warn once
    /// at the end of the buffered collect — enough signal for an
    /// operator running `RUST_LOG=warn` to notice that this workload
    /// is on the wrong API, with zero impact on the value returned
    /// to the caller.
    ///
    /// Default `100 * 1024 * 1024` (100 MiB) — catches bulk pulls
    /// (multi-million-row option chains, multi-day backfills) while
    /// staying silent on ad-hoc single-day queries.
    ///
    /// Set to `0` to disable the warn entirely. `usize::MAX`
    /// effectively disables it too (no realistic response reaches
    /// that size).
    pub warn_on_buffered_threshold_bytes: usize,

    /// Bypass the subscription-tier clamp on `concurrent_requests`.
    ///
    /// **Test-only escape hatch.** ThetaData enforces a server-side
    /// cap on concurrent in-flight gRPC requests per subscription
    /// tier (Free=1 / Value=2 / Standard=4 / Pro=8). The SDK normally
    /// clamps `concurrent_requests` to this cap at connect time so
    /// the user gets a clear `tracing::warn!` rather than opaque
    /// upstream rejections on the (N+1)-th channel. Setting this to
    /// `true` skips the clamp — only useful for tests that need to
    /// reproduce the over-provisioning failure mode against a stubbed
    /// auth response.
    ///
    /// **Do not enable in production.** The server will reject
    /// channels above the tier cap; the SDK's clamp is the friendly
    /// boundary that surfaces the problem locally instead of letting
    /// it leak into per-RPC retry storms.
    #[doc(hidden)]
    pub override_tier_clamp: bool,
}

impl MddsConfig {
    /// Production defaults.
    #[must_use]
    pub fn production_defaults() -> Self {
        Self {
            host: "mdds-01.thetadata.us".to_string(),
            port: 443,
            tls: true,
            concurrent_requests: 0,
            max_message_size: 4 * 1024 * 1024,
            keepalive_secs: 30,
            keepalive_timeout_secs: 10,
            window_size_kb: 64,
            connection_window_size_kb: 64,
            connect_timeout_secs: 10,
            decode_threads: None,
            decode_queue_depth: None,
            decoder_ring_size: 256,
            // 100 MiB — empirically catches bulk pulls (multi-million
            // row option-chain or multi-day backfill responses) while
            // staying silent on ad-hoc single-day quote / OHLC pulls
            // that fit in a single h2 frame. Issue #576 sets the
            // operator-visible "you are on the wrong API for this
            // workload" signal at this boundary.
            warn_on_buffered_threshold_bytes: 100 * 1024 * 1024,
            override_tier_clamp: false,
        }
    }
}

impl Default for MddsConfig {
    fn default() -> Self {
        Self::production_defaults()
    }
}
