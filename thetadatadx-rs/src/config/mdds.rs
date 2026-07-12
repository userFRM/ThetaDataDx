//! Market-data (gRPC) sub-configuration.
//!
//! Holds the per-channel gRPC tuning for the market-data transport:
//! message-size ceiling, keepalive cadence, HTTP/2 flow-control
//! windows, connect/request deadlines, and the buffered-response warn
//! threshold.
//!
//! Channel-pool concurrency is **not** a tuning knob here: it is
//! resolved internally from the subscription tier returned by Nexus
//! auth at connect time, so the live pool always stays inside the
//! server-side per-tier ceiling without any caller input.
//!
//! See `docs-site/docs/configuration.md` for the per-binding setter
//! samples.

/// Default per-request market-data deadline in seconds (5 min).
///
/// The floor the effective-deadline resolver applies when a caller leaves the
/// per-request deadline unset AND the configured `request_timeout_secs` is `0`:
/// a `0` there would disable the gRPC hang guard for every deadline-less
/// request, so a live-but-silent server could hang the client forever. Sits
/// above the slowest realistic bulk pull. The per-call
/// `with_deadline(Duration::ZERO)` opt-out is a separate, explicit path and is
/// unaffected by this floor.
pub(crate) const DEFAULT_REQUEST_TIMEOUT_SECS: u64 = 300;

/// Market-data client tuning.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct MarketDataConfig {
    /// Market-data hostname (v3 path).
    ///
    /// Set through [`DirectConfig::set_market_data_host`] so the write is
    /// recorded as an explicit override that survives environment
    /// selection; read through [`DirectConfig::market_data_host`]. The field
    /// is crate-private so the only way to point the market-data channel at a
    /// host is the tracked setter — there is no untracked direct-write path
    /// for environment selection to second-guess.
    ///
    /// [`DirectConfig::set_market_data_host`]: crate::config::DirectConfig::set_market_data_host
    /// [`DirectConfig::market_data_host`]: crate::config::DirectConfig::market_data_host
    pub(crate) host: String,

    /// Market-data port (443 for TLS in production).
    pub port: u16,

    /// Whether to use TLS for the market-data connection.
    /// Always `true` in production (standard gRPC-over-TLS on port 443).
    pub tls: bool,

    /// Max inbound gRPC message size, in bytes.
    ///
    /// Caps the size of a single inbound gRPC message. Default
    /// `4 * 1024 * 1024` (4 MiB); validation bounds it to `[1 B, 64 MiB]`,
    /// the same ceiling the `[grpc] max_message_size_mb` TOML spelling enforces.
    pub max_message_size: usize,

    /// gRPC keepalive interval in seconds (`keepAliveTime(30, SECONDS)`).
    ///
    /// Sets the HTTP/2 keepalive PING cadence on every market-data channel.
    pub keepalive_secs: u64,

    /// gRPC keepalive timeout in seconds (`keepAliveTimeout(10, SECONDS)`).
    ///
    /// How long an unanswered keepalive PING is tolerated before the
    /// connection is declared dead and recycled.
    pub keepalive_timeout_secs: u64,

    /// gRPC flow control: initial stream window size in KB.
    ///
    /// Sets the per-stream HTTP/2 flow-control window on every market-data
    /// channel. Default 1024 KB (1 MiB) — well above the 64 KiB HTTP/2 spec
    /// window, which throttles bulk pulls; validation clamps to
    /// `[64, 2_097_151]`.
    pub stream_window_size_kb: usize,

    /// gRPC flow control: initial connection window size in KB.
    ///
    /// Sets the connection-level HTTP/2 flow-control window on every
    /// market-data channel. Default 8192 KB (8 MiB); validation clamps to
    /// `[64, 2_097_151]`. Increase for high-throughput bulk queries.
    pub connection_window_size_kb: usize,

    /// TCP connect timeout for the market-data channel, in seconds.
    ///
    /// Bounds the time the transport will spend establishing a TCP +
    /// TLS handshake before failing fast. Default `10s` matches the upper
    /// bound observed on the wire; production deployments behind NAT / VPN
    /// can raise this to absorb slow handshakes without altering keepalive
    /// cadence.
    pub connect_timeout_secs: u64,

    /// Default per-request deadline for market-data (gRPC) queries, in
    /// seconds.
    ///
    /// A server that holds the HTTP/2 stream open while sending no
    /// chunks would otherwise hang `collect_stream` / `stream(...)`
    /// indefinitely: the gRPC keepalive PING only detects a fully dead
    /// peer, not a live-but-silent one. This default bounds every
    /// request that did not call `with_deadline(...)`, so a stalled
    /// stream resolves to `Error::Timeout` instead of blocking forever.
    ///
    /// Configuring `0` here does **not** disable the guard: the effective-
    /// deadline resolver every market-data request routes through floors a `0`
    /// to the production default (`300s`) so a deadline-less request can never
    /// hang the client forever, regardless of whether the config was validated.
    /// Opt a single request out with the per-call escape hatch instead.
    ///
    /// Per-call control overrides this: `with_deadline(Duration)` sets a
    /// shorter or longer bound, and `with_deadline(Duration::ZERO)`
    /// opts a single request out of any deadline.
    ///
    /// Default `300s` (5 min) — comfortably above the slowest realistic
    /// multi-million-row bulk pull while still bounding a wedged stream.
    pub request_timeout_secs: u64,

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
}

impl MarketDataConfig {
    /// Upper ceiling for [`Self::max_message_size`], in megabytes.
    ///
    /// The inbound message size is a pre-allocated decode budget, so an
    /// out-of-range value is a footgun in both directions: an absurd value
    /// commits the channel to a buffer far beyond any legitimate response, and
    /// the MB→byte conversion (`mb * 1024 * 1024`) overflows `usize` for the
    /// largest inputs. The production default is 4 MB; 64 MB leaves generous
    /// headroom for the largest bulk market-data chunk while keeping the budget
    /// bounded. This is the single source of truth both the byte-denominated
    /// [`crate::config::DirectConfig::validate`] check and the
    /// `[grpc] max_message_size_mb` TOML ceiling read, so the two spellings
    /// cannot drift.
    pub(crate) const MAX_MESSAGE_SIZE_MB: usize = 64;

    /// Market-data hostname.
    ///
    /// Read accessor for the crate-private [`Self::host`] field. The host is
    /// written through [`DirectConfig::set_market_data_host`] so a
    /// caller-supplied value is recorded as a tracked override; this getter
    /// is the supported way to read it back (including from the SDK
    /// bindings, which snapshot a [`MarketDataConfig`]).
    ///
    /// [`DirectConfig::set_market_data_host`]: crate::config::DirectConfig::set_market_data_host
    #[must_use]
    pub fn host(&self) -> &str {
        &self.host
    }

    /// Production defaults.
    #[must_use]
    pub fn production_defaults() -> Self {
        Self {
            host: "mdds-01.thetadata.us".to_string(),
            port: 443,
            tls: true,
            max_message_size: 4 * 1024 * 1024,
            keepalive_secs: 30,
            keepalive_timeout_secs: 10,
            stream_window_size_kb: 1_024,
            connection_window_size_kb: 8_192,
            connect_timeout_secs: 10,
            // 5 min — bounds a server that holds the stream open while
            // sending no chunks (h2 keepalive only catches a fully dead
            // peer). Sits above the slowest realistic bulk pull;
            // `with_deadline(Duration::ZERO)` opts a single request out.
            request_timeout_secs: DEFAULT_REQUEST_TIMEOUT_SECS,
            // 100 MiB — empirically catches bulk pulls (multi-million
            // row option-chain or multi-day backfill responses) while
            // staying silent on ad-hoc single-day quote / OHLC pulls
            // that fit in a single h2 frame. Issue #576 sets the
            // operator-visible "you are on the wrong API for this
            // workload" signal at this boundary.
            warn_on_buffered_threshold_bytes: 100 * 1024 * 1024,
        }
    }
}

impl Default for MarketDataConfig {
    fn default() -> Self {
        Self::production_defaults()
    }
}
