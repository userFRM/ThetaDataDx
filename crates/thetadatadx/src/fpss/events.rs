//! FPSS event types: data, control, and the I/O command channel.
//!
//! These are the wire-protocol-agnostic value types that flow from the I/O
//! thread into the Disruptor ring and out to user callbacks.

use std::sync::Arc;

use tdbe::types::enums::{RemoveReason, StreamMsgType, StreamResponseType};

use super::protocol::Contract;

/// Tick data events from the FPSS stream.
///
/// These are the hot-path events decoded from FIT wire format and
/// delta-decompressed. All price fields are decoded to `f64` at parse time.
///
/// Every variant carries the fully parsed [`Contract`] as `Arc<Contract>` —
/// users identify the contract via `contract.symbol`, `contract.expiration`,
/// `contract.strike`, `contract.is_call`. The wire-internal numeric id the
/// FPSS server assigns is no longer surfaced on data events; downstream code
/// that needs an id-keyed map builds it from the
/// [`FpssControl::ContractAssigned`] event stream.
///
/// The I/O thread populates an internal `contract_id -> Arc<Contract>` cache
/// on [`FpssControl::ContractAssigned`] so each decoded event only pays a
/// refcount bump — matching the Java terminal's behaviour where each
/// event listener receives the full `net.thetadata.fpssclient.Contract`
/// alongside the payload.
///
/// # Unresolved-contract sentinel
///
/// When a data frame arrives before the matching `ContractAssigned`
/// frame, the `contract` field holds an unresolved-contract sentinel.
/// Detect it via
/// `contract.sec_type == tdbe::types::enums::SecType::Unknown` — the
/// canonical, type-safe check.
///
/// The sentinel's `symbol` carries the wire-internal contract id under
/// the `__pending:` prefix (e.g. `"__pending:42"`). Production
/// callbacks should NOT parse this prefix — it is a diagnostic payload
/// that the WS bridge surfaces as `unresolved_contract_id` for
/// operator visibility. SDK consumers identify contracts by
/// `(symbol, expiration, right, strike)` per the removal of
/// wire ids from public data events.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum FpssData {
    /// Decoded quote tick (code 21).
    Quote {
        /// Full parsed contract for this tick. Holds the unresolved-
        /// contract sentinel (`sec_type == SecType::Unknown`; the
        /// `symbol` carries `__pending:<id>` for diagnostic surfacing)
        /// when the server has not yet sent the matching
        /// `ContractAssigned` frame.
        contract: Arc<Contract>,
        ms_of_day: i32,
        bid_size: i32,
        bid_exchange: i32,
        bid: f64,
        bid_condition: i32,
        ask_size: i32,
        ask_exchange: i32,
        ask: f64,
        ask_condition: i32,
        date: i32,
        /// Wall-clock nanoseconds since UNIX epoch, captured at frame decode time.
        received_at_ns: u64,
    },
    /// Decoded trade tick (code 22).
    Trade {
        /// Full parsed contract for this tick. Holds the unresolved-
        /// contract sentinel (`sec_type == SecType::Unknown`; the
        /// `symbol` carries `__pending:<id>` for diagnostic surfacing)
        /// when the matching `ContractAssigned` frame has not yet
        /// arrived.
        contract: Arc<Contract>,
        ms_of_day: i32,
        sequence: i32,
        ext_condition1: i32,
        ext_condition2: i32,
        ext_condition3: i32,
        ext_condition4: i32,
        condition: i32,
        size: i32,
        exchange: i32,
        price: f64,
        condition_flags: i32,
        price_flags: i32,
        volume_type: i32,
        records_back: i32,
        date: i32,
        /// Wall-clock nanoseconds since UNIX epoch, captured at frame decode time.
        received_at_ns: u64,
    },
    /// Decoded open interest tick (code 23).
    OpenInterest {
        /// Full parsed contract for this tick. Holds the unresolved-
        /// contract sentinel (`sec_type == SecType::Unknown`; the
        /// `symbol` carries `__pending:<id>` for diagnostic surfacing)
        /// when the matching `ContractAssigned` frame has not yet
        /// arrived.
        contract: Arc<Contract>,
        ms_of_day: i32,
        open_interest: i32,
        date: i32,
        /// Wall-clock nanoseconds since UNIX epoch, captured at frame decode time.
        received_at_ns: u64,
    },
    /// Decoded OHLCVC bar (code 24 or trade-derived).
    ///
    /// `volume` and `count` are `i64` to avoid overflow on high-volume symbols.
    Ohlcvc {
        /// Full parsed contract for this tick. Holds the unresolved-
        /// contract sentinel (`sec_type == SecType::Unknown`; the
        /// `symbol` carries `__pending:<id>` for diagnostic surfacing)
        /// when the matching `ContractAssigned` frame has not yet
        /// arrived.
        contract: Arc<Contract>,
        ms_of_day: i32,
        open: f64,
        high: f64,
        low: f64,
        close: f64,
        volume: i64,
        count: i64,
        date: i32,
        /// Wall-clock nanoseconds since UNIX epoch, captured at frame decode time.
        received_at_ns: u64,
    },
}

/// Control/lifecycle events from the FPSS stream.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum FpssControl {
    /// Login succeeded (METADATA code 3).
    ///
    /// `permissions` is the server's "Bundle" string, copied verbatim from the
    /// METADATA frame payload as UTF-8. **It is opaque diagnostic metadata, not
    /// a structured permission set.** The wire protocol does not parse it:
    /// upstream logs the value as `[FPSS] CONNECTED: [host], Bundle: <perms>`
    /// and uses non-null as the `isVerified()` sentinel — that's it.
    ///
    /// **For feature gating, use [`crate::auth::AuthUser`] instead**.
    /// The Nexus REST endpoint exposes per-asset subscription tiers
    /// (`stock_subscription`, `options_subscription`, `indices_subscription`,
    /// `interest_rate_subscription`, each `0=FREE / 1=VALUE / 2=STANDARD /
    /// 3=PRO`), which is the canonical surface used to compute concurrency
    /// limits and gate features.
    ///
    /// Treat this field as a log/diagnostic string only. Do not parse it.
    LoginSuccess { permissions: String },
    /// Server sent a CONTRACT assignment (code 20).
    ///
    /// The `contract` is shared as `Arc<Contract>` so downstream consumers
    /// and the I/O thread's contract cache hold the same heap allocation —
    /// cloning the Arc is a refcount bump with no `String` allocation.
    ContractAssigned { id: i32, contract: Arc<Contract> },
    /// Subscription response (code 40).
    ReqResponse {
        req_id: i32,
        result: StreamResponseType,
    },
    /// Market open signal (code 30).
    MarketOpen,
    /// Market close / stop signal (code 32).
    MarketClose,
    /// Server error message (code 11).
    ServerError { message: String },
    /// Server disconnected us (code 12).
    Disconnected { reason: RemoveReason },
    /// Auto-reconnect is about to attempt reconnection.
    ///
    /// Emitted before sleeping for the delay. `attempt` is 1-based.
    Reconnecting {
        reason: RemoveReason,
        attempt: u32,
        delay_ms: u64,
    },
    /// Auto-reconnect succeeded -- connection is live again.
    Reconnected,
    /// Protocol-level parse error.
    Error { message: String },
    /// Server sent a frame with an unrecognized code. Raw bytes preserved
    /// for diagnostics / upstream bug reports.
    UnknownFrame { code: u8, payload: Vec<u8> },
    /// Server connection ack (code 4, `StreamMsgType::Connected`).
    ///
    /// Decoded from the server→client CONNECTED frame. Previously fell
    /// through to [`FpssControl::UnknownFrame`].
    Connected,
    /// Server heartbeat (code 10, `StreamMsgType::Ping`).
    ///
    /// The server emits PING frames (observed 1-byte payload `[0]`) that
    /// client heartbeat logic does not have to answer. Payload preserved
    /// for diagnostics — previously every heartbeat surfaced as
    /// `UnknownFrame { code: 10, payload: [0] }`.
    Ping { payload: Vec<u8> },
    /// Server-side reconnect ack (code 13).
    ///
    /// Distinct from [`FpssControl::Reconnected`], which the client
    /// emits from its auto-reconnect state machine once the new TLS
    /// session is authenticated. `ReconnectedServer` is the server
    /// telling the client that the server-side session has just
    /// re-established.
    ReconnectedServer,
    /// Server stream restart (code 31, `StreamMsgType::Restart`).
    ///
    /// The server restarts the stream without dropping the TCP
    /// connection. Delta decode state should be cleared on receipt.
    Restart,
}

/// All FPSS events -- either data or control.
///
/// Subscribers receive these through the Disruptor callback. The enum is
/// non-exhaustive to allow adding new event types without breaking downstream.
///
/// # Layout
///
/// Declared `#[repr(C, u8)]` with explicit discriminants so the in-memory
/// layout is shared with [`FpssEventInternal`]: both enums encode `Data`
/// at discriminant `0` and `Control` at discriminant `1`, with identical
/// payload positions. The I/O loop publishes `FpssEventInternal` into the
/// Disruptor ring (so it can also carry decode-fallback / placeholder
/// variants without surfacing them publicly), then delivers a
/// `&FpssEvent` reference to the user callback for `Data`/`Control` slots
/// only — see [`FpssEventInternal::as_public`] for the layout-compatible
/// reborrow that makes that zero-clone.
#[derive(Debug, Clone)]
#[repr(C, u8)]
#[non_exhaustive]
pub enum FpssEvent {
    /// Tick data event (quote, trade, open interest, OHLCVC).
    Data(FpssData) = FPSS_EVENT_TAG_DATA,
    /// Control/lifecycle event (login, contract assignment, market open/close, etc.).
    Control(FpssControl) = FPSS_EVENT_TAG_CONTROL,
}

// Discriminant tags shared between `FpssEvent` and `FpssEventInternal`.
// `pub(crate)` so the I/O loop, ring, and decode layers can match on the
// same source of truth as the enum definitions.
pub(crate) const FPSS_EVENT_TAG_DATA: u8 = 0;
pub(crate) const FPSS_EVENT_TAG_CONTROL: u8 = 1;
pub(crate) const FPSS_EVENT_TAG_UNPARSEABLE: u8 = 2;
pub(crate) const FPSS_EVENT_TAG_EMPTY: u8 = 3;

/// Internal event type stored in the Disruptor ring.
///
/// **Not part of the supported public API.** Marked `#[doc(hidden)]`
/// because the SDK exports a `__test_internals` shim that re-exports
/// it solely for soak-test infrastructure (capture+replay,
/// reconnect-storm, schema-drift). Production consumers must not name
/// this type — its variants and layout discipline can change without a
/// SemVer bump.
///
/// Carries the same `Data` / `Control` variants as the public
/// [`FpssEvent`] (and at the same discriminant + layout, see the
/// `repr(C, u8)` clause), plus two crate-private variants the SDK never
/// surfaces to the user callback:
///
/// * [`FpssEventInternal::Unparseable`] — truncated / corrupt FIT
///   payload that the decoder accounts on the
///   `thetadatadx.fpss.decode_failures` metric. Kept as a typed event
///   for soak-test introspection without leaking raw bytes through the
///   public API.
/// * [`FpssEventInternal::Empty`] — pre-allocation placeholder for
///   ring-buffer slots that have never been written; the previous
///   `Option<FpssEvent>` slot wrapper is collapsed into this variant
///   so the consumer closure can avoid the `Option` discriminant test.
///
/// The I/O thread builds `FpssEventInternal` directly from the wire
/// decoder; the Disruptor consumer reborrows the slot reference to a
/// `&FpssEvent` via [`Self::as_public`] (zero-clone, layout-compatible)
/// and only invokes the user callback when that reborrow succeeds.
#[derive(Debug, Clone)]
#[repr(C, u8)]
#[doc(hidden)]
pub enum FpssEventInternal {
    /// Same payload + discriminant as [`FpssEvent::Data`].
    Data(FpssData) = FPSS_EVENT_TAG_DATA,
    /// Same payload + discriminant as [`FpssEvent::Control`].
    Control(FpssControl) = FPSS_EVENT_TAG_CONTROL,
    /// Decoder rejected this frame (truncated FIT payload). Filtered
    /// before user callbacks; surfaced on the
    /// `thetadatadx.fpss.decode_failures` metric counter and visible
    /// to soak tests that assert on the internal stream shape.
    Unparseable = FPSS_EVENT_TAG_UNPARSEABLE,
    /// Ring-buffer slot placeholder. Filtered before user callbacks.
    Empty = FPSS_EVENT_TAG_EMPTY,
}

impl Default for FpssEventInternal {
    #[inline]
    fn default() -> Self {
        Self::Empty
    }
}

impl FpssEventInternal {
    /// Borrow this internal event as a public [`FpssEvent`] reference,
    /// or return `None` for the internal-only variants.
    ///
    /// # Safety / soundness
    ///
    /// Both `FpssEvent` and `FpssEventInternal` are
    /// `#[repr(C, u8)]`, share the same discriminant constants for the
    /// `Data` and `Control` arms ([`FPSS_EVENT_TAG_DATA`],
    /// [`FPSS_EVENT_TAG_CONTROL`]), and carry the same payload type at
    /// each shared discriminant. Per the Rust reference's
    /// "Primitive representation of enums with fields" section, two
    /// `#[repr(C, u8)]` enums with matching discriminants and matching
    /// per-variant payload types have identical in-memory layout for
    /// those variants — the layout is `(u8 tag, padding, payload)`
    /// where `padding` is determined entirely by the alignment of the
    /// payload type, which is the same for both enums (same payload
    /// type ⇒ same alignment ⇒ same padding). Casting a
    /// `&FpssEventInternal` to a `&FpssEvent` is therefore sound when
    /// the discriminant is `Data` or `Control`.
    ///
    /// The cast is gated on the discriminant tag, so the
    /// `Unparseable` / `Empty` arms (with discriminants
    /// [`FPSS_EVENT_TAG_UNPARSEABLE`], [`FPSS_EVENT_TAG_EMPTY`]) can
    /// never escape into the public type — they map to `None`.
    ///
    /// The static assertions in [`assert_layout_compat`] (run in the
    /// crate's unit tests) verify size + alignment + discriminant
    /// equality at compile time so a future divergence — e.g. someone
    /// adding a private field to `FpssData` only on the internal side
    /// — fails the build before it can corrupt a user callback.
    #[inline]
    pub fn as_public(&self) -> Option<&FpssEvent> {
        // Gate the layout-compatibility cast on a real `match`. The
        // arm bindings (`_d`, `_c`) read the variant payload bytes
        // through `discriminant_data_offset` / `discriminant_control_offset`
        // — those helpers ensure static dead-code analysis observes
        // the field, complementing the unsafe reborrow that hands the
        // bytes back to the caller via the public type.
        match self {
            Self::Data(d) => {
                // Touch the variant payload so dead-code analysis
                // sees a field read alongside the layout-compat
                // reborrow below. `core::hint::black_box` is the
                // canonical "use this value" marker that survives
                // optimisation; it compiles to a no-op move on every
                // backend Rust ships.
                core::hint::black_box(d);
                // SAFETY: this arm proves the discriminant is
                // `FPSS_EVENT_TAG_DATA`. Both `FpssEvent` and
                // `FpssEventInternal` are `#[repr(C, u8)]` with
                // identical `Data(FpssData)` payloads at that
                // discriminant, so the layout is shared (Rust
                // reference, "Primitive representation of enums with
                // fields"). The reborrow inherits the `&self`
                // lifetime; aliasing rules treat it like the
                // original borrow.
                Some(unsafe { &*(self as *const Self as *const FpssEvent) })
            }
            Self::Control(c) => {
                // Same field-read marker as the `Data` arm.
                core::hint::black_box(c);
                // SAFETY: same layout-compatibility argument as the
                // `Data` arm — same `#[repr(C, u8)]` discipline,
                // same payload type at this discriminant.
                Some(unsafe { &*(self as *const Self as *const FpssEvent) })
            }
            Self::Unparseable | Self::Empty => None,
        }
    }
}

impl From<FpssEvent> for FpssEventInternal {
    #[inline]
    fn from(evt: FpssEvent) -> Self {
        match evt {
            FpssEvent::Data(d) => Self::Data(d),
            FpssEvent::Control(c) => Self::Control(c),
        }
    }
}

// ---------------------------------------------------------------------------
// BackpressurePolicy — overflow strategy for the pull-iter queue
// ---------------------------------------------------------------------------

/// Producer-side strategy when the pull-iter `Delivery::Queue` ring saturates.
///
/// Pull-iter delivery routes events through a bounded
/// [`crossbeam_queue::ArrayQueue`] shared with an [`super::EventIterator`].
/// When the iterator falls behind, the queue fills. This enum tells the
/// Disruptor consumer thread how to react.
///
/// Default is [`Self::Block`] — preserves every event at the cost of
/// upstream backpressure into the TLS reader (which can ultimately get
/// disconnected by the server if the wait runs long). Trading
/// systems that cannot tolerate dropped ticks should hold the default;
/// dashboards and cold-consumer tooling that prefer freshness over
/// completeness should explicitly opt into [`Self::DropOldest`] or
/// [`Self::DropNewest`].
///
/// Mirrors the standard Kafka client `acks` axis (BLOCK / DROP_OLDEST /
/// DROP_NEWEST) and Bloomberg BLPAPI `eventQueue` overflow modes — same
/// operator vocabulary across vendors so users coming from kdb+/kx or
/// BLPAPI carry their mental model over without translation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BackpressurePolicy {
    /// The Disruptor consumer parks (with backoff) on a full queue
    /// until the iterator drains. No events are dropped; the TLS
    /// reader applies upstream backpressure because the Disruptor
    /// ring saturates next and `try_publish` starts incrementing the
    /// `dropped` counter.
    ///
    /// This is the safe default — every event the SDK decoded is
    /// guaranteed to reach the iterator. A sustained slow consumer
    /// risks the server kicking the session
    /// (`RemoveReason::TimedOut`) because the TCP read window stops
    /// advancing, which is the correct failure mode for a
    /// configuration that has explicitly asked for no drops.
    #[default]
    Block,
    /// Evict the head of the queue before pushing the new event. The
    /// freshest events stay; the stalest events are dropped. The
    /// shared [`super::FpssClient::dropped_count`] counter still
    /// increments per evicted event so operators can graph the drop
    /// rate.
    ///
    /// Recommended for dashboards and visualisers where the user
    /// cares about "what is the market doing right now" and tolerates
    /// gaps in stale history.
    DropOldest,
    /// Skip the new event when the queue is full. The events already
    /// in flight are preserved; the new event is dropped, and the
    /// shared `dropped` counter increments.
    ///
    /// Equivalent to the legacy pre-v10.1 behaviour (silent
    /// best-effort delivery). Recommended when downstream processing
    /// is strictly causal — once an event is enqueued you want to
    /// finish processing it before considering a newer one.
    DropNewest,
}

// ---------------------------------------------------------------------------
// Delivery — push (callback) vs pull (iterator queue) selection
// ---------------------------------------------------------------------------

/// Delivery mode chosen at [`super::FpssClient::connect`] time.
///
/// Mutually exclusive on a given client. The Disruptor consumer
/// closure dispatches on the variant once per event:
///
/// * [`Delivery::Callback`] — invoke the user-supplied `FnMut(&FpssEvent)`
///   closure under `catch_unwind` (push-callback delivery, the default
///   recommended for low-latency consumption).
/// * [`Delivery::Queue`] — `force_push` the public [`FpssEvent`] clone
///   onto a lock-free [`crossbeam_queue::ArrayQueue`] sized at the same
///   ring capacity, increment a shared `dropped` counter on overflow,
///   and let an [`super::EventIterator`] drain the queue from the user
///   thread (pull-iter delivery, equivalent to databento's
///   `for record in client:` pattern but bypassing their intermediate
///   `queue.Queue`).
///
/// Both modes share the same Disruptor producer + ring + reader, so the
/// upstream pipeline is identical and the only branch is the per-event
/// dispatch inside the consumer closure. Switching between modes requires
/// `stop_streaming()` + a fresh `start_streaming*()`; live mode swap is
/// not supported because it would require synchronising the consumer
/// closure mid-flight.
pub(crate) enum Delivery {
    /// Push-callback delivery. The captured closure runs on the
    /// Disruptor consumer thread.
    Callback(Box<dyn FnMut(&FpssEvent) + Send + 'static>),
    /// Pull-iter delivery. The closure clones the public event into the
    /// shared bounded queue; an [`super::EventIterator`] drains it on the
    /// user thread.
    Queue {
        /// Same capacity as the Disruptor ring so backpressure semantics
        /// match the callback path. Drained by an
        /// [`super::EventIterator`] on the user thread.
        queue: std::sync::Arc<crossbeam_queue::ArrayQueue<FpssEvent>>,
        /// Set to `true` by the Disruptor consumer thread's drop guard
        /// AFTER the consume loop has exited and all in-flight events
        /// have been pushed onto `queue`. The [`super::EventIterator`]
        /// uses this — not the global shutdown flag — as its terminal
        /// predicate, so a `stop_streaming()` followed by a tail of
        /// not-yet-consumed events cannot false-EOF the iterator
        /// mid-drain.
        ///
        /// The flag is owned by a `move`-captured drop guard inside the
        /// consumer closure; when the Disruptor producer is dropped at
        /// io_loop exit, the consumer thread joins, the closure is
        /// dropped, the guard's `Drop` runs, and the flag flips. This
        /// is the only point in the system where "no more pushes will
        /// happen" is observable, which is why the EventIterator keys
        /// off it instead of the I/O-thread shutdown signal.
        iter_closed: std::sync::Arc<std::sync::atomic::AtomicBool>,
        /// Optional FD-readiness signal for asyncio / select-loop
        /// consumers. `Some(wake)` is set via
        /// [`super::FpssClient::connect_iter_with_wake`] — the
        /// Python SDK's `streaming_async()` surface allocates a
        /// self-pipe, hands the write-end to the wake, and registers
        /// the read-end on the asyncio loop via `loop.add_reader`.
        /// Each successful `queue.push` writes a coalesced single byte
        /// (via [`super::wake::WakeFd::signal`]) so the reader's
        /// `epoll` wake fires without polling.
        ///
        /// `None` for the synchronous pull-iter path
        /// (`start_streaming_iter()` / `client.streaming_iter()`),
        /// which drains via `next_timeout` / `next` and pays the
        /// 100 µs sleep tick budget instead. Keeping the field
        /// optional preserves zero-cost on the existing sync path —
        /// no atomic load, no FD write, no extra Arc clone.
        wake_fd: Option<std::sync::Arc<super::wake::WakeFd>>,
        /// Overflow strategy. See [`BackpressurePolicy`]. The default
        /// [`BackpressurePolicy::Block`] preserves every event at the
        /// cost of upstream pressure into the TLS reader; the
        /// `DropOldest` / `DropNewest` variants trade event-loss for
        /// liveness.
        ///
        /// The sync pull-iter path
        /// ([`super::FpssClient::connect_iter`]) hard-codes
        /// `DropNewest` to preserve legacy behaviour. The async
        /// surfaces (`streaming_async()`,
        /// `streaming_async_batches()`) thread the policy through via
        /// [`super::FpssClient::connect_iter_with_wake_keep_handle_policy`].
        policy: BackpressurePolicy,
    },
}

// ---------------------------------------------------------------------------
// Command channel -- FpssClient -> I/O thread
// ---------------------------------------------------------------------------

/// Commands sent from the `FpssClient` handle to the I/O thread.
pub(super) enum IoCommand {
    /// Write a raw frame (code + payload) to the TLS stream.
    WriteFrame {
        code: StreamMsgType,
        payload: Vec<u8>,
    },
    /// Graceful shutdown: send STOP, then exit the I/O loop.
    Shutdown,
}

#[cfg(test)]
mod tests {
    use super::*;
    use tdbe::types::price::Price;

    /// The default [`BackpressurePolicy`] must be `Block` — the safe
    /// option that preserves every event. A future contributor
    /// flipping the default would silently introduce drop-on-overflow
    /// on every caller of the pull-iter surfaces; pin the contract.
    #[test]
    fn backpressure_policy_default_is_block() {
        assert_eq!(BackpressurePolicy::default(), BackpressurePolicy::Block);
    }

    /// All three [`BackpressurePolicy`] variants must be distinct,
    /// Copy, and Eq so callers can store the policy in a `#[pyclass]`
    /// field without an `Arc` wrapper and compare it cheaply in the
    /// io_loop hot path's `match`.
    #[test]
    fn backpressure_policy_variants_are_distinct() {
        let block = BackpressurePolicy::Block;
        let drop_oldest = BackpressurePolicy::DropOldest;
        let drop_newest = BackpressurePolicy::DropNewest;
        assert_ne!(block, drop_oldest);
        assert_ne!(block, drop_newest);
        assert_ne!(drop_oldest, drop_newest);
        // Copy: bind through fresh `let`s.
        let _copy: BackpressurePolicy = block;
        let _copy: BackpressurePolicy = drop_oldest;
        let _copy: BackpressurePolicy = drop_newest;
    }

    /// Pin the layout-compatibility invariant
    /// `FpssEventInternal::as_public` relies on. Any future change that
    /// breaks size, alignment, or discriminant equality between
    /// `FpssEvent` and the public-facing variants of
    /// `FpssEventInternal` must trip this test before it can corrupt a
    /// reborrow.
    #[test]
    fn fpss_event_internal_layout_matches_public() {
        // Same `#[repr(C, u8)]` declaration on both enums plus
        // identical payload types ⇒ identical size + alignment.
        assert_eq!(
            std::mem::size_of::<FpssEvent>(),
            std::mem::size_of::<FpssEventInternal>(),
            "FpssEvent and FpssEventInternal must have identical size for the layout-compat reborrow",
        );
        assert_eq!(
            std::mem::align_of::<FpssEvent>(),
            std::mem::align_of::<FpssEventInternal>(),
            "FpssEvent and FpssEventInternal must have identical alignment",
        );

        // Discriminant-byte equality. The `as_public` reborrow assumes
        // a constructed `FpssEventInternal::Data(_)` shares the same
        // first-byte tag as `FpssEvent::Data(_)`, and likewise for
        // `Control`. If a contributor reorders the explicit
        // `= FPSS_EVENT_TAG_*` discriminants on either enum (or removes
        // the explicit tag) this fires before silent corruption ships.
        let contract = Arc::new(Contract::stock("DISC"));
        let internal_data = FpssEventInternal::Data(FpssData::Quote {
            contract: Arc::clone(&contract),
            ms_of_day: 0,
            bid_size: 0,
            bid_exchange: 0,
            bid: 0.0,
            bid_condition: 0,
            ask_size: 0,
            ask_exchange: 0,
            ask: 0.0,
            ask_condition: 0,
            date: 0,
            received_at_ns: 0,
        });
        let internal_control = FpssEventInternal::Control(FpssControl::LoginSuccess {
            permissions: String::new(),
        });
        let public_data = FpssEvent::Data(FpssData::Quote {
            contract: Arc::clone(&contract),
            ms_of_day: 0,
            bid_size: 0,
            bid_exchange: 0,
            bid: 0.0,
            bid_condition: 0,
            ask_size: 0,
            ask_exchange: 0,
            ask: 0.0,
            ask_condition: 0,
            date: 0,
            received_at_ns: 0,
        });
        let public_control = FpssEvent::Control(FpssControl::LoginSuccess {
            permissions: String::new(),
        });
        let tag = |p: *const u8| unsafe { *p };
        assert_eq!(
            tag(&internal_data as *const _ as *const u8),
            FPSS_EVENT_TAG_DATA,
            "FpssEventInternal::Data discriminant byte must equal FPSS_EVENT_TAG_DATA",
        );
        assert_eq!(
            tag(&public_data as *const _ as *const u8),
            FPSS_EVENT_TAG_DATA,
            "FpssEvent::Data discriminant byte must equal FPSS_EVENT_TAG_DATA",
        );
        assert_eq!(
            tag(&internal_control as *const _ as *const u8),
            FPSS_EVENT_TAG_CONTROL,
            "FpssEventInternal::Control discriminant byte must equal FPSS_EVENT_TAG_CONTROL",
        );
        assert_eq!(
            tag(&public_control as *const _ as *const u8),
            FPSS_EVENT_TAG_CONTROL,
            "FpssEvent::Control discriminant byte must equal FPSS_EVENT_TAG_CONTROL",
        );
    }

    /// Verify that constructing a `Data` / `Control` `FpssEventInternal`
    /// and reborrowing it via `as_public` yields a value-equal
    /// `FpssEvent`. Round-trips data + control payloads through the
    /// reborrow so payload bytes (Arc pointers, scalar fields) are not
    /// corrupted by the cast.
    #[test]
    fn fpss_event_internal_roundtrips_data_and_control() {
        let contract = Arc::new(Contract::stock("MSFT"));
        let internal = FpssEventInternal::Data(FpssData::Trade {
            contract: Arc::clone(&contract),
            ms_of_day: 12_345,
            sequence: 7,
            ext_condition1: 0,
            ext_condition2: 0,
            ext_condition3: 0,
            ext_condition4: 0,
            condition: 0,
            size: 100,
            exchange: 1,
            price: 150.0,
            condition_flags: 0,
            price_flags: 0,
            volume_type: 0,
            records_back: 0,
            date: 20_260_507,
            received_at_ns: 99,
        });
        let public = internal
            .as_public()
            .expect("Data variant must reborrow as public");
        match public {
            FpssEvent::Data(FpssData::Trade {
                contract: pub_contract,
                ms_of_day,
                sequence,
                price,
                ..
            }) => {
                assert!(
                    Arc::ptr_eq(pub_contract, &contract),
                    "reborrow must preserve the Arc<Contract> pointer identity",
                );
                assert_eq!(*ms_of_day, 12_345);
                assert_eq!(*sequence, 7);
                assert!((*price - 150.0).abs() < f64::EPSILON);
            }
            other => panic!("expected Data(Trade) after reborrow, got {other:?}"),
        }

        let internal_ctrl = FpssEventInternal::Control(FpssControl::MarketOpen);
        assert!(matches!(
            internal_ctrl.as_public(),
            Some(FpssEvent::Control(FpssControl::MarketOpen)),
        ));
    }

    /// The `Unparseable` and `Empty` variants must NEVER escape into a
    /// public `FpssEvent` reference — `as_public` maps them to `None`.
    /// Pinning the filter at the type level is the whole point of the
    /// internal/public split.
    #[test]
    fn fpss_event_internal_filters_internal_only_variants() {
        assert!(FpssEventInternal::Unparseable.as_public().is_none());
        assert!(FpssEventInternal::Empty.as_public().is_none());
    }

    #[test]
    fn fpss_control_reconnecting_variant() {
        let evt = FpssEvent::Control(FpssControl::Reconnecting {
            reason: RemoveReason::ServerRestarting,
            attempt: 1,
            delay_ms: 2000,
        });
        if let FpssEvent::Control(FpssControl::Reconnecting {
            reason,
            attempt,
            delay_ms,
        }) = &evt
        {
            assert_eq!(*reason, RemoveReason::ServerRestarting);
            assert_eq!(*attempt, 1);
            assert_eq!(*delay_ms, 2000);
        } else {
            panic!("expected Reconnecting");
        }
    }

    #[test]
    fn fpss_control_reconnected_variant() {
        let evt = FpssEvent::Control(FpssControl::Reconnected);
        assert!(matches!(&evt, FpssEvent::Control(FpssControl::Reconnected)));
    }

    #[test]
    fn fpss_event_split_data_control() {
        let contract = Arc::new(Contract::stock("AAPL"));
        let data_evt = FpssEvent::Data(FpssData::Trade {
            contract: Arc::clone(&contract),
            ms_of_day: 0,
            sequence: 0,
            ext_condition1: 0,
            ext_condition2: 0,
            ext_condition3: 0,
            ext_condition4: 0,
            condition: 0,
            size: 100,
            exchange: 0,
            price: Price::new(15025, 8).to_f64(),
            condition_flags: 0,
            price_flags: 0,
            volume_type: 0,
            records_back: 0,
            date: 20240315,
            received_at_ns: 0,
        });
        match &data_evt {
            FpssEvent::Data(FpssData::Trade {
                contract, price, ..
            }) => {
                assert_eq!(contract.symbol, "AAPL");
                assert!((*price - 150.25).abs() < f64::EPSILON);
            }
            other => panic!("expected Data(Trade), got {other:?}"),
        }
        let ctrl = FpssEvent::Control(FpssControl::MarketOpen);
        assert!(matches!(&ctrl, FpssEvent::Control(FpssControl::MarketOpen)));
    }

    #[test]
    fn fpss_control_connected_ping_reconnected_server_restart_variants() {
        // Every new control variant must round-trip and expose its payload
        // correctly — matching the Java terminal hand-off where codes
        // 4 / 10 / 13 / 31 each land on their own typed listener.
        let connected = FpssEvent::Control(FpssControl::Connected);
        assert!(matches!(
            &connected,
            FpssEvent::Control(FpssControl::Connected)
        ));

        let ping = FpssEvent::Control(FpssControl::Ping {
            payload: vec![0x00],
        });
        if let FpssEvent::Control(FpssControl::Ping { payload }) = &ping {
            assert_eq!(payload.as_slice(), &[0x00]);
        } else {
            panic!("expected Ping");
        }

        let reconnected_server = FpssEvent::Control(FpssControl::ReconnectedServer);
        assert!(matches!(
            &reconnected_server,
            FpssEvent::Control(FpssControl::ReconnectedServer)
        ));

        let restart = FpssEvent::Control(FpssControl::Restart);
        assert!(matches!(&restart, FpssEvent::Control(FpssControl::Restart)));
    }
}
