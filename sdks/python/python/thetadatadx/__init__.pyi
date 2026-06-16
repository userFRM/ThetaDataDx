"""ThetaData market-data SDK.

`thetadatadx` provides direct access to ThetaData's real-time and
historical market data without a separate terminal process. The package
exposes:

- Connection types: :class:`Credentials` and :class:`Config`.
- Clients: :class:`Client` (historical plus on-demand
  streaming), :class:`AsyncClient` (``await``-based
  historical), :class:`StreamingClient` (streaming only), and
  :class:`HistoricalClient` (historical only).
- A fluent subscription surface: :class:`Contract`,
  :class:`Subscription`, and :class:`SecType`.
- Real-time event types delivered to the streaming callback
  (:class:`Quote`, :class:`Trade`, :class:`OpenInterest`,
  :class:`Ohlcvc`, and the connection / lifecycle events).
- A typed exception hierarchy rooted at :class:`ThetaDataError`.
- Analytics and utility entry points such as :func:`all_greeks`,
  :func:`implied_volatility`, and :func:`split_date_range`.

Type checkers discover these annotations through the ``py.typed`` marker
(PEP 561) shipped alongside this file.
"""

# Per-endpoint historical builders and typed `<Tick>List` wrappers share
# one structural shape and resolve through the module-level `__getattr__`
# to `Any`; only the load-bearing public surface is annotated explicitly
# here.

from __future__ import annotations

from typing import (
    Any,
    Callable,
    List,
    Literal,
    Optional,
    Tuple,
    Type,
    final,
)

# PEP 396 — package version string, resolved at import time from the
# installed wheel's metadata via `importlib.metadata.version`. Falls
# back to the in-source default when the wheel metadata is absent
# (editable installs, source-tree imports).
__version__: str
"""Installed package version string (PEP 396)."""

# ─────────────────────────────────────────────────────────────────────
# Credentials + Config
# ─────────────────────────────────────────────────────────────────────


@final
class Credentials:
    """ThetaData Nexus credentials (email + password)."""

    def __init__(self, email: str, password: str) -> None:
        """Create credentials from an account email and password."""
        ...

    @staticmethod
    def from_file(path: str) -> Credentials:
        """Load credentials from a two-line file (line 1 email, line 2 password).

        Args:
            path: Path to the credentials file.

        Returns:
            The loaded :class:`Credentials`.

        Raises:
            ThetaDataError: If the file cannot be read or is malformed.
        """
        ...

    def __repr__(self) -> str:
        """Return a representation with the email redacted."""
        ...


@final
class Config:
    """Connection configuration: historical host / streaming hosts / reconnect policy."""

    @staticmethod
    def production() -> Config:
        """Return the production configuration (ThetaData NJ datacenter)."""
        ...

    @staticmethod
    def dev() -> Config:
        """Return the dev configuration (port 20200, infinite historical replay)."""
        ...

    @staticmethod
    def stage() -> Config:
        """Return the stage configuration (port 20100, testing, unstable)."""
        ...

    historical_host: str
    """Hostname of the historical-data server."""
    historical_port: int
    """TCP port of the historical-data server."""
    concurrent_requests: int
    """Maximum in-flight historical requests. ``0`` auto-detects the cap from the subscription tier; explicit values above the tier cap are clamped at connect time with a warning."""
    warn_on_buffered_threshold_bytes: int
    """Byte ceiling above which a buffered (non-``.stream()``) historical response logs a warning pointing the caller at the streaming surface. ``0`` disables the warning; the default is ``100 * 1024 * 1024`` (100 MiB). The data is still delivered."""
    reconnect_policy: str
    """Active reconnect policy name: ``"auto"``, ``"manual"``, or ``"custom"`` (the last reported when a :attr:`reconnect_callback` is installed)."""
    reconnect_max_attempts: int
    """Maximum consecutive reconnect attempts on the generic-transient failure class before the session gives up (default 30)."""
    reconnect_max_rate_limited_attempts: int
    """Maximum consecutive reconnect attempts on the rate-limited (TooManyRequests) failure class, tracked independently of :attr:`reconnect_max_attempts`."""
    reconnect_max_server_restart_attempts: int
    """Maximum consecutive reconnect attempts on the ServerRestarting failure class, budgeting pool bounces (default 60)."""
    reconnect_max_elapsed_secs: int
    """Wall-clock envelope, in seconds, bounding a consecutive-reconnect sequence (default 300; ``0`` disables the envelope)."""
    reconnect_stable_window_secs: int
    """Connected duration, in seconds, after which a session is considered stable and the consecutive-reconnect counters reset."""
    reconnect_wait_ms: int
    """Initial delay, in milliseconds, of the generic-transient exponential back-off ladder (default 250), doubling up to :attr:`reconnect_wait_max_ms`."""
    reconnect_wait_max_ms: int
    """Ceiling, in milliseconds, of the generic-transient exponential back-off ladder (default 30_000)."""
    reconnect_wait_rate_limited_ms: int
    """Back-off floor, in milliseconds, applied to the rate-limited (TooManyRequests) failure class (default 130_000)."""
    reconnect_wait_server_restart_ms: int
    """Flat reconnect cadence, in milliseconds, applied to the ServerRestarting failure class (default 5_000)."""
    reconnect_jitter: Literal["full", "equal", "decorrelated", "none"]
    """Jitter strategy applied to every reconnect delay: ``"full"`` (default), ``"equal"``, ``"decorrelated"``, or ``"none"``."""
    reconnect_replay_burst_size: int
    """Number of subscription-replay frames sent per burst after an auto-reconnect (default 50, minimum 1)."""
    reconnect_replay_pace_ms: int
    """Jittered pause, in milliseconds, between subscription-replay bursts after an auto-reconnect (default 5; ``0`` removes the pause)."""
    reconnect_callback: Optional[Callable[[int, int], Optional[int]]]
    """Custom reconnect policy: a callable ``(reason: int, attempt: int) -> Optional[int]`` returning the reconnect delay in milliseconds, or ``None`` to stop (the stream then emits the terminal :class:`ReconnectsExhausted` event). Runs on the streaming I/O thread; permanent disconnect reasons never reach it. Assign ``None`` to restore the auto policy. Write-only: the configured callable cannot be read back (:attr:`reconnect_policy` then reports ``"custom"``)."""
    worker_threads: Optional[int]
    """Async worker-thread count for the embedded runtime. ``None`` defers to the default sizing; an ``int`` (including ``0``, which clamps to ``1``) pins the worker count."""
    retry_initial_delay_ms: int
    """Initial delay, in milliseconds, of the historical-request retry back-off (default 250)."""
    retry_max_delay_ms: int
    """Ceiling, in milliseconds, of the historical-request retry back-off (default 30_000)."""
    retry_max_attempts: int
    """Maximum historical-request retry attempts (default 20)."""
    retry_max_elapsed_secs: int
    """Wall-clock envelope, in seconds, bounding the historical-request retry loop (default 300; ``0`` disables the envelope)."""
    retry_jitter: bool
    """Whether jitter is applied to historical-request retry delays (default ``True``)."""
    flatfiles_max_attempts: int
    """Maximum retry attempts for the flat-file driver (default 10, validated ``1..=100``)."""
    flatfiles_initial_backoff_secs: int
    """Initial back-off, in seconds, of the flat-file driver retry loop (default 1)."""
    flatfiles_max_backoff_secs: int
    """Ceiling back-off, in seconds, of the flat-file driver retry loop (default 30)."""
    flatfiles_jitter: bool
    """Whether jitter is applied to flat-file driver retry delays (default ``True``)."""
    nexus_url: str
    """Authentication endpoint URL (defaults to the production endpoint)."""
    client_type: str
    """Client-type identifier sent during authentication (defaults to ``"rust-thetadatadx"``)."""
    metrics_port: Optional[int]
    """Prometheus exporter port. ``None`` (the default) leaves the exporter disabled even when the metrics feature is compiled in; an ``int`` binds an HTTP listener on ``0.0.0.0:<port>``. The setter raises ``ValueError`` for values outside ``0..=65535``."""
    streaming_timeout_ms: int
    """No-frames deadline, in milliseconds, for the streaming connection (default 3_000)."""
    streaming_connect_timeout_ms: int
    """Connect timeout, in milliseconds, for opening a streaming connection."""
    streaming_ping_interval_ms: int
    """Interval, in milliseconds, between client-side streaming heartbeats."""
    streaming_ring_size: int
    """Capacity, in slots, of the streaming event ring; must be a power of two and at least 64."""
    streaming_io_read_slice_ms: int
    """Time slice, in milliseconds, the streaming I/O loop spends reading per iteration."""
    streaming_data_watchdog_ms: int
    """Hard wall-clock backstop, in milliseconds, above :attr:`streaming_timeout_ms` that tears down a silent stream (default 30_000; ``0`` disables)."""
    streaming_keepalive_idle_secs: int
    """Idle time, in seconds, before kernel-side TCP keepalive probing begins on the streaming socket (default 5)."""
    streaming_keepalive_interval_secs: int
    """Interval, in seconds, between kernel-side TCP keepalive probes on the streaming socket (default 2)."""
    streaming_keepalive_retries: int
    """Number of unanswered kernel-side TCP keepalive probes before the streaming socket is declared dead (default 2)."""
    streaming_host_selection: Literal["shuffled", "fixed_order"]
    """Streaming host-selection order: ``"shuffled"`` (fault-domain-aware per-client shuffle, seedable via :attr:`streaming_host_shuffle_seed`) or ``"fixed_order"``."""
    streaming_host_shuffle_seed: Optional[int]
    """Seed for the per-client streaming host shuffle; ``None`` draws a fresh seed each connect."""
    derive_ohlcvc: bool
    """Whether OHLCVC bars are derived locally from the trade stream and delivered as :class:`Ohlcvc` events."""
    flush_mode: Literal["batched", "immediate"]
    """Streaming write-flush policy. ``"batched"`` (default) flushes on the heartbeat (~100 ms); ``"immediate"`` flushes after every wire write. The setter accepts the same two strings case-insensitively and raises ``ValueError`` otherwise."""
    wait_strategy: Literal["low_latency", "balanced", "efficient", "busy_spin"]
    """Streaming event-ring consumer wait strategy — the latency-vs-CPU knob applied on each ring-empty poll. ``"low_latency"`` (default) never sleeps; ``"balanced"`` parks briefly; ``"efficient"`` parks longer; ``"busy_spin"`` pure-spins and pins a core. The setter accepts the same strings case-insensitively and raises ``ValueError`` otherwise."""
    wait_spin_iters: int
    """Spin iterations the wait strategy busy-waits before yielding / parking."""
    wait_yield_iters: int
    """``yield_now`` iterations after the spin phase, before any park."""
    wait_park_us: int
    """Park interval (microseconds) for the ``"balanced"`` / ``"efficient"`` strategies; inert for the never-sleep strategies."""
    consumer_cpu: Optional[int]
    """CPU core to pin the streaming consumer thread to; ``None`` (default) leaves it under the OS scheduler. An out-of-range or offline core is a best-effort no-op."""

    def __repr__(self) -> str:
        """Return a representation with the host, port, and stream-host count."""
        ...


# ─────────────────────────────────────────────────────────────────────
# Fluent: Contract / Subscription / SecType
# ─────────────────────────────────────────────────────────────────────


@final
class Contract:
    """Per-contract identity (stock or option) for streaming subscriptions.

    ``strike`` is the price in dollars on both sides of the builder:
    ``option(strike=550)``, ``option(strike=550.0)``, and
    ``option(strike="550")`` are equivalent, and the ``strike``
    property reads the same dollar value back.
    """

    @staticmethod
    def stock(symbol: str) -> Contract:
        """Construct a stock contract for ``symbol``."""
        ...

    @staticmethod
    def option(
        symbol: str,
        *,
        expiration: str,
        strike: float | int | str,
        right: str,
    ) -> Contract:
        """Construct an option contract.

        Args:
            symbol: Underlying root symbol.
            expiration: Expiration date as a ``YYYYMMDD`` string.
            strike: Strike price in dollars; a number or string is
                accepted (``550``, ``550.0``, and ``"550"`` are equivalent).
            right: Option right, ``"C"`` (call) or ``"P"`` (put).

        Returns:
            The constructed option :class:`Contract`.

        Raises:
            ValueError: If any field fails validation.
        """
        ...

    @property
    def symbol(self) -> str:
        """The contract's symbol."""
        ...

    @property
    def sec_type(self) -> str:
        """Security type as an uppercase name (``"STOCK"`` / ``"OPTION"`` / ``"INDEX"`` / ``"RATE"``)."""
        ...

    @property
    def expiration(self) -> Optional[int]:
        """Expiration date as a ``YYYYMMDD`` integer; ``None`` for non-options."""
        ...

    @property
    def strike(self) -> Optional[float]:
        """Strike price in dollars; ``None`` for non-options."""
        ...

    @property
    def right(self) -> Optional[str]:
        """Option right (``"C"`` / ``"P"``); ``None`` for non-options."""
        ...

    def quote(self) -> Subscription:
        """Build a per-contract Quote subscription."""
        ...

    def trade(self) -> Subscription:
        """Build a per-contract Trade subscription."""
        ...

    def open_interest(self) -> Subscription:
        """Build a per-contract OpenInterest subscription."""
        ...

    def market_value(self) -> Subscription:
        """Build a per-contract market-value subscription."""
        ...

    def __repr__(self) -> str:
        """Return a representation of the contract."""
        ...

    def __eq__(self, other: object) -> bool:
        """Return whether ``other`` is a contract with the same identity."""
        ...


@final
class ContractRef:
    """Read-only contract identifier surfaced on every streaming event.

    Distinct from the fluent `Contract` builder — `ContractRef` is what
    `event.contract` returns inside a streaming callback, with the
    resolved `symbol`, `sec_type`, `expiration`, `right`, and `strike`
    (dollars — the same unit historical rows carry under the same
    name). The fluent `Contract` (above) is the one users instantiate
    to subscribe.
    """

    @property
    def symbol(self) -> str:
        """The resolved contract symbol."""
        ...

    @property
    def sec_type(self) -> str:
        """Security type as an uppercase name (``"STOCK"`` / ``"OPTION"`` / ``"INDEX"`` / ``"RATE"``)."""
        ...

    @property
    def expiration(self) -> Optional[int]:
        """Expiration date as a ``YYYYMMDD`` integer; ``None`` for non-options."""
        ...

    @property
    def right(self) -> Optional[str]:
        """Option right (``"C"`` / ``"P"``); ``None`` for non-options."""
        ...

    @property
    def strike(self) -> Optional[float]:
        """Strike price in dollars; ``None`` for non-options."""
        ...

    def __repr__(self) -> str:
        """Return a representation of the contract reference."""
        ...


@final
class SecType:
    """Security type — `STOCK` / `OPTION` / `INDEX` / `RATE`."""

    STOCK: SecType
    """The equity security type."""
    OPTION: SecType
    """The option security type."""
    INDEX: SecType
    """The index security type."""
    RATE: SecType
    """The interest-rate security type."""

    def full_trades(self) -> Subscription:
        """Build a full-stream Trade subscription for this security type."""
        ...

    def full_open_interest(self) -> Subscription:
        """Build a full-stream OpenInterest subscription for this security type."""
        ...

    def __repr__(self) -> str:
        """Return a representation of the security type (e.g. ``"SecType.OPTION"``)."""
        ...

    def __str__(self) -> str:
        """Return the uppercase name (e.g. ``"OPTION"``)."""
        ...

    def __eq__(self, other: object) -> bool:
        """Return whether ``other`` is the same security type."""
        ...

    def __hash__(self) -> int:
        """Return a hash consistent with :meth:`__eq__`."""
        ...


@final
class Subscription:
    """Typed market-data subscription (per-contract or full-stream)."""

    @property
    def kind(self) -> str:
        """The wire-level kind for this subscription.

        One of ``"quote"`` / ``"trade"`` / ``"open_interest"`` /
        ``"market_value"`` / ``"full_trades"`` / ``"full_open_interest"``.
        """
        ...

    @property
    def is_full(self) -> bool:
        """``True`` for full-stream (security-type-scoped) subscriptions."""
        ...

    @property
    def contract(self) -> Optional[Contract]:
        """The bound contract for per-contract subscriptions; ``None`` for full-stream."""
        ...

    @property
    def sec_type(self) -> Optional[SecType]:
        """The security type for full-stream subscriptions; ``None`` for per-contract."""
        ...

    def __repr__(self) -> str:
        """Return a representation of the subscription."""
        ...


# ─────────────────────────────────────────────────────────────────────
# Streaming event classes — delivered to the streaming callback. The
# dispatcher fires exactly one of these per event; narrow on the
# concrete class (`match event: case Quote(): ...`) or read `event.kind`.
# ─────────────────────────────────────────────────────────────────────


@final
class Quote:
    """A real-time Quote tick — top-of-book bid / ask for one contract."""

    contract: ContractRef
    """The contract this quote is for."""
    ms_of_day: int
    """Milliseconds since midnight Eastern Time when the quote was recorded."""
    bid_size: int
    """Number of contracts or shares resting at the bid."""
    bid_exchange: int
    """Exchange code posting the bid."""
    bid: float
    """Bid price in dollars."""
    bid_condition: int
    """Quote condition code for the bid."""
    ask_size: int
    """Number of contracts or shares resting at the ask."""
    ask_exchange: int
    """Exchange code posting the ask."""
    ask: float
    """Ask price in dollars."""
    ask_condition: int
    """Quote condition code for the ask."""
    date: int
    """Trading date as a ``YYYYMMDD`` integer."""
    received_at_ns: int
    """Wall-clock nanoseconds since the UNIX epoch, captured when the frame was decoded."""

    @property
    def kind(self) -> str:
        """Event kind discriminator (``"quote"``)."""
        ...

    def __repr__(self) -> str:
        """Return a representation of the event."""
        ...


@final
class Trade:
    """A real-time Trade tick — one executed print for a contract."""

    contract: ContractRef
    """The contract this trade is for."""
    ms_of_day: int
    """Milliseconds since midnight Eastern Time when the trade printed."""
    sequence: int
    """Exchange sequence number ordering trades within the day."""
    ext_condition1: int
    """Extended trade condition code 1."""
    ext_condition2: int
    """Extended trade condition code 2."""
    ext_condition3: int
    """Extended trade condition code 3."""
    ext_condition4: int
    """Extended trade condition code 4."""
    condition: int
    """Primary trade condition code."""
    size: int
    """Trade size in contracts or shares."""
    exchange: int
    """Exchange code where the trade printed."""
    price: float
    """Trade price in dollars."""
    condition_flags: int
    """Bit flags qualifying the trade conditions."""
    price_flags: int
    """Bit flags qualifying the trade price."""
    volume_type: int
    """Volume classification code for the trade."""
    records_back: int
    """Number of records back this trade was reported (out-of-order correction offset)."""
    date: int
    """Trading date as a ``YYYYMMDD`` integer."""
    received_at_ns: int
    """Wall-clock nanoseconds since the UNIX epoch, captured when the frame was decoded."""

    @property
    def kind(self) -> str:
        """Event kind discriminator (``"trade"``)."""
        ...

    def __repr__(self) -> str:
        """Return a representation of the event."""
        ...


@final
class OpenInterest:
    """A real-time OpenInterest tick — open-contract count for an option."""

    contract: ContractRef
    """The contract this open-interest tick is for."""
    ms_of_day: int
    """Milliseconds since midnight Eastern Time when the open interest was recorded."""
    open_interest: int
    """Number of outstanding open contracts."""
    date: int
    """Trading date as a ``YYYYMMDD`` integer."""
    received_at_ns: int
    """Wall-clock nanoseconds since the UNIX epoch, captured when the frame was decoded."""

    @property
    def kind(self) -> str:
        """Event kind discriminator (``"open_interest"``)."""
        ...

    def __repr__(self) -> str:
        """Return a representation of the event."""
        ...


@final
class Ohlcvc:
    """An OHLCVC bar, derived locally when ``Config.derive_ohlcvc`` is ``True``."""

    contract: ContractRef
    """The contract this bar is for."""
    ms_of_day: int
    """Milliseconds since midnight Eastern Time at the bar's open."""
    open: float
    """Opening price of the bar in dollars."""
    high: float
    """Highest traded price within the bar in dollars."""
    low: float
    """Lowest traded price within the bar in dollars."""
    close: float
    """Closing price of the bar in dollars."""
    volume: int
    """Total traded volume within the bar, in contracts or shares."""
    count: int
    """Number of trades aggregated into the bar."""
    date: int
    """Trading date as a ``YYYYMMDD`` integer."""
    received_at_ns: int
    """Wall-clock nanoseconds since the UNIX epoch, captured when the frame was decoded."""

    @property
    def kind(self) -> str:
        """Event kind discriminator (``"ohlcvc"``)."""
        ...

    def __repr__(self) -> str:
        """Return a representation of the event."""
        ...


@final
class ContractAssigned:
    """The server assigned a numeric id to a subscribed contract."""

    id: int
    """Wire-internal numeric id the server assigned to this contract."""
    contract: ContractRef
    """The contract associated with the assigned id."""

    @property
    def kind(self) -> str:
        """Event kind discriminator (``"contract_assigned"``)."""
        ...

    def __repr__(self) -> str:
        """Return a representation of the event."""
        ...


@final
class Connected:
    """The streaming connection has been established."""

    @property
    def kind(self) -> str:
        """Event kind discriminator (``"connected"``)."""
        ...

    def __repr__(self) -> str:
        """Return a representation of the event."""
        ...


@final
class Disconnected:
    """The server disconnected the client."""

    reason: int
    """Numeric disconnect reason code; see :attr:`reason_name` for the symbolic form."""

    @property
    def kind(self) -> str:
        """Event kind discriminator (``"disconnected"``)."""
        ...

    @property
    def reason_name(self) -> str:
        """The disconnect reason as a symbolic name (e.g. ``"TooManyRequests"``)."""
        ...

    def __repr__(self) -> str:
        """Return a representation of the event."""
        ...


@final
class ParseError:
    """A streaming protocol-level parse error.

    Named ``ParseError`` so it never collides with the :class:`Error`
    exception class.
    """

    message: str
    """Human-readable description of the parse failure."""

    @property
    def kind(self) -> str:
        """Event kind discriminator (``"parse_error"``)."""
        ...

    def __repr__(self) -> str:
        """Return a representation of the event."""
        ...


@final
class LoginSuccess:
    """A successful login acknowledgement carrying the granted permissions."""

    permissions: str
    """Server-supplied entitlement string from the login acknowledgement; opaque diagnostic metadata, not a structured permission set."""

    @property
    def kind(self) -> str:
        """Event kind discriminator (``"login_success"``)."""
        ...

    def __repr__(self) -> str:
        """Return a representation of the event."""
        ...


@final
class MarketClose:
    """A market-close signal. Carries no payload."""

    @property
    def kind(self) -> str:
        """Event kind discriminator (``"market_close"``)."""
        ...

    def __repr__(self) -> str:
        """Return a representation of the event."""
        ...


@final
class MarketOpen:
    """A market-open signal. Carries no payload."""

    @property
    def kind(self) -> str:
        """Event kind discriminator (``"market_open"``)."""
        ...

    def __repr__(self) -> str:
        """Return a representation of the event."""
        ...


@final
class Ping:
    """A server heartbeat carrying an opaque payload."""

    payload: bytes
    """Raw heartbeat payload bytes, preserved for diagnostics."""

    @property
    def kind(self) -> str:
        """Event kind discriminator (``"ping"``)."""
        ...

    def __repr__(self) -> str:
        """Return a representation of the event."""
        ...


@final
class Reconnected:
    """Auto-reconnect succeeded — the connection is live again."""

    @property
    def kind(self) -> str:
        """Event kind discriminator (``"reconnected"``)."""
        ...

    def __repr__(self) -> str:
        """Return a representation of the event."""
        ...


@final
class ReconnectedServer:
    """A server-side reconnect acknowledgement."""

    @property
    def kind(self) -> str:
        """Event kind discriminator (``"reconnected_server"``)."""
        ...

    def __repr__(self) -> str:
        """Return a representation of the event."""
        ...


@final
class Reconnecting:
    """Auto-reconnect is about to attempt a reconnection."""

    reason: int
    """Numeric disconnect reason code that triggered the reconnect attempt; see :attr:`reason_name` for the symbolic form."""
    attempt: int
    """One-based index of this reconnect attempt."""
    delay_ms: int
    """Delay, in milliseconds, before this reconnect attempt fires."""

    @property
    def kind(self) -> str:
        """Event kind discriminator (``"reconnecting"``)."""
        ...

    @property
    def reason_name(self) -> str:
        """The disconnect reason as a symbolic name."""
        ...

    def __repr__(self) -> str:
        """Return a representation of the event."""
        ...


@final
class ReconnectsExhausted:
    """Auto-reconnect stopped without a user-initiated shutdown —
    terminal for the session. Emitted on budget / wall-clock-envelope
    exhaustion, a permanent disconnect reason, a manual policy, or a
    custom policy returning ``None``. ``attempts`` is the number of
    consecutive reconnect attempts consumed (0 when no reconnect was
    attempted)."""

    reason: int
    """Numeric disconnect reason code of the final drop before recovery was abandoned; see :attr:`reason_name` for the symbolic form."""
    attempts: int
    """Number of consecutive reconnect attempts consumed before giving up (``0`` when no reconnect was attempted)."""

    @property
    def kind(self) -> str:
        """Event kind discriminator (``"reconnects_exhausted"``)."""
        ...

    @property
    def reason_name(self) -> str:
        """The disconnect reason as a symbolic name."""
        ...

    def __repr__(self) -> str:
        """Return a representation of the event."""
        ...


@final
class ReqResponse:
    """A response to a subscription request, identified by ``req_id``."""

    req_id: int
    """Identifier of the subscription request this response answers."""
    result: int
    """Numeric outcome code of the subscription request."""

    @property
    def kind(self) -> str:
        """Event kind discriminator (``"req_response"``)."""
        ...

    def __repr__(self) -> str:
        """Return a representation of the event."""
        ...


@final
class Restart:
    """A server-initiated stream restart."""

    @property
    def kind(self) -> str:
        """Event kind discriminator (``"restart"``)."""
        ...

    def __repr__(self) -> str:
        """Return a representation of the event."""
        ...


@final
class ServerError:
    """A server-error message carrying a human-readable description."""

    message: str
    """Human-readable error text from the server."""

    @property
    def kind(self) -> str:
        """Event kind discriminator (``"server_error"``)."""
        ...

    def __repr__(self) -> str:
        """Return a representation of the event."""
        ...


@final
class UnknownControl:
    """A control event the SDK does not yet recognise."""

    @property
    def kind(self) -> str:
        """Event kind discriminator (``"unknown_control"``)."""
        ...

    def __repr__(self) -> str:
        """Return a representation of the event."""
        ...


@final
class UnknownFrame:
    """A frame with an unrecognised wire code, surfaced with its raw bytes."""

    code: int
    """Unrecognised wire frame code reported by the server."""
    payload: bytes
    """Raw frame payload bytes, preserved for diagnostics."""

    @property
    def kind(self) -> str:
        """Event kind discriminator (``"unknown_frame"``)."""
        ...

    def __repr__(self) -> str:
        """Return a representation of the event."""
        ...


# Union of every streaming event class delivered to the callback. Opaque
# to type checkers; narrow at runtime via `match` / `isinstance`.
StreamEvent = Any


# ─────────────────────────────────────────────────────────────────────
# Streaming clients
# ─────────────────────────────────────────────────────────────────────

EventCallback = Callable[[Any], None]


@final
class HistoricalView:
    """Historical-data sub-namespace returned by :attr:`Client.historical`.

    Exposes every historical / list / snapshot / at-time endpoint as a
    method (sync, ``*_async`` coroutine, and ``*_builder`` fluent
    constructor). The endpoint surface is generated, so individual
    method signatures are resolved at runtime rather than enumerated
    here.
    """


@final
class StreamView:
    """Real-time-streaming sub-namespace returned by :attr:`Client.stream`.

    Owns the streaming lifecycle, subscription management, and feed
    diagnostics for the unified client. Shares the parent client's
    callback registration, so starting / stopping / reconnecting through
    this view drives the same session the client manages.
    """

    # Streaming lifecycle.
    def start_streaming(self, callback: EventCallback) -> None:
        """Start real-time streaming and register ``callback`` for events.

        ``callback`` is invoked with exactly one argument — a typed
        event instance (:class:`Quote`, :class:`Trade`, :class:`Ohlcvc`,
        :class:`OpenInterest`, and the lifecycle / control events).
        Narrow on the concrete class or read ``event.kind``. Exceptions
        raised inside the callback are caught and reported through the
        unraisable hook; each one increments the panic count.

        Args:
            callback: Single-argument callable receiving each event.

        Raises:
            RuntimeError: If streaming is already started.
            ThetaDataError: If the streaming connection cannot be opened.
        """
        ...

    def is_streaming(self) -> bool:
        """Return whether the streaming connection is currently active."""
        ...

    def stop_streaming(self) -> None:
        """Stop streaming while keeping the historical client usable.

        Clears the registered callback. To resume, call
        :meth:`start_streaming` again with a freshly bound callable;
        :meth:`reconnect` raises until a callback is re-registered.
        """
        ...

    def shutdown(self) -> None:
        """Shut down the streaming connection and clear the callback.

        Equivalent to :meth:`stop_streaming` for callback lifetime; a
        subsequent :meth:`reconnect` fails until :meth:`start_streaming`
        is called again.
        """
        ...

    def reconnect(self) -> None:
        """Reconnect streaming and re-register the previous callback.

        Restores all active subscriptions on the new connection.

        Raises:
            RuntimeError: If no callback is registered (i.e. after
                :meth:`stop_streaming` / :meth:`shutdown`).
        """
        ...

    def await_drain(self, timeout_ms: int) -> bool:
        """Block until the streaming consumer thread finishes firing the callback.

        Args:
            timeout_ms: Maximum time to wait, in milliseconds.

        Returns:
            ``True`` if the drain completed within the timeout, else
            ``False``.
        """
        ...

    # Subscriptions.
    def subscribe(self, sub: Subscription) -> None:
        """Subscribe to a single :class:`Subscription`.

        Args:
            sub: A subscription from ``Contract.quote()`` / ``.trade()``
                / ``.open_interest()`` or ``SecType.OPTION.full_trades()``.

        Raises:
            ThetaDataError: If the subscription is rejected.
        """
        ...

    def subscribe_many(self, subs: List[Subscription]) -> None:
        """Subscribe to several subscriptions.

        Stops at the first error and re-raises it; previously installed
        subscriptions are not rolled back.

        Args:
            subs: An iterable of :class:`Subscription` values.

        Raises:
            ThetaDataError: If any subscription is rejected.
        """
        ...

    def unsubscribe(self, sub: Subscription) -> None:
        """Cancel a single :class:`Subscription`.

        Raises:
            ThetaDataError: If the request is rejected.
        """
        ...

    def unsubscribe_many(self, subs: List[Subscription]) -> None:
        """Cancel several subscriptions.

        Raises:
            ThetaDataError: If any request is rejected.
        """
        ...

    def active_subscriptions(self) -> List[Subscription]:
        """Return a snapshot of the active per-contract subscriptions.

        Empty when streaming has not started.
        """
        ...

    # Metrics + connection observability.
    def dropped_event_count(self) -> int:
        """Cumulative count of streaming events dropped because the
        consumer fell behind and the event ring was full.

        Returns 0 before :meth:`start_streaming` and after
        :meth:`stop_streaming`. :meth:`reconnect` resets the counter;
        snapshot it beforehand to accumulate across sessions.
        """
        ...

    def ring_occupancy(self) -> int:
        """Point-in-time count of events published into the streaming
        event ring but not yet drained into the callback. Rising
        occupancy approaching ``ring_capacity()`` predicts drops;
        sampling never blocks the feed. 0 when streaming is not
        active."""
        ...
    def ring_capacity(self) -> int:
        """Configured streaming event-ring capacity in slots — the
        fixed denominator for ``ring_occupancy()``. 0 when streaming
        is not active."""
        ...
    def millis_since_last_event(self) -> Optional[int]:
        """Milliseconds since the most recent inbound streaming frame
        of any kind, or ``None`` before streaming starts. A steadily
        growing value is the earliest external signal of a dead or
        wedged connection."""
        ...
    def last_event_received_at_unix_nanos(self) -> int:
        """UNIX-nanosecond receive timestamp of the most recent inbound
        streaming frame of any kind.

        Returns 0 before streaming starts or before any frame arrives.
        """
        ...

    def last_connected_addr(self) -> Optional[str]:
        """``host:port`` of the live streaming server, following the
        session across auto-reconnects."""
        ...

    def active_full_subscriptions(self) -> List[Subscription]:
        """Return a snapshot of the active full-stream subscriptions
        (e.g. ``SecType.OPTION.full_trades()``).

        Returns the same typed :class:`Subscription` values passed to
        :meth:`subscribe`. Empty when streaming has not started.
        """
        ...

    def panic_count(self) -> int:
        """Cumulative count of user-callback panics caught by the
        streaming consumer's panic boundary.

        Each exception raised inside the registered callback is caught,
        reported through the unraisable hook, and counted here.
        """
        ...


@final
class Client:
    """Unified client for historical data and real-time streaming.

    Connects to ThetaData at construction (a single authentication
    covers both historical access and streaming). Historical endpoints
    are available immediately; real-time streaming starts on demand via
    :meth:`start_streaming`. This is the recommended entry point.
    """

    def __init__(self, creds: Credentials, config: Config) -> None:
        """Connect to ThetaData with ``creds`` and ``config``.

        Authenticates and opens the historical channel; streaming is not
        started. The call is interruptible with ``Ctrl+C`` if the
        handshake stalls.

        Args:
            creds: Account credentials.
            config: Connection configuration (e.g. ``Config.production()``).

        Raises:
            ThetaDataError: If authentication or the connection fails.
        """
        ...

    @staticmethod
    def from_file(
        path: str,
        config: Optional[Config] = None,
    ) -> Client:
        """Construct a client from a credentials file and connect.

        Args:
            path: Path to a two-line credentials file.
            config: Connection configuration; defaults to
                ``Config.production()`` when omitted.

        Returns:
            A connected :class:`Client`.

        Raises:
            ThetaDataError: If the file cannot be read or the connection
                fails.
        """
        ...

    # Data sub-namespaces.
    @property
    def historical(self) -> HistoricalView:
        """Historical-data sub-namespace.

        Every historical / list / snapshot / at-time endpoint is reached
        through this view, e.g. ``client.historical.stock_eod(...)`` and
        the ``*_async`` / ``*_builder`` companions. Returns a fresh view
        over a cheap handle clone on each access.
        """
        ...

    @property
    def stream(self) -> StreamView:
        """Real-time-streaming sub-namespace.

        The streaming lifecycle, subscription management, and feed
        diagnostics are reached through this view, e.g.
        ``client.stream.start_streaming(cb)`` and
        ``client.stream.subscribe(...)``. Shares the unified client's
        callback registration so the lifecycle observed through the view
        is the one the client manages.
        """
        ...

    # Session identity + subscription tier.
    def session_uuid(self) -> str:
        """Server-assigned session UUID for the live streaming connection."""
        ...
    def subscription_info(self) -> List[Tuple[str, str]]:
        """Subscription-tier snapshot captured at authentication time.

        One ``(asset_class, tier)`` tuple per asset class the Nexus auth
        payload carries, in stable declaration order: ``stock`` /
        ``options`` / ``indices`` / ``interest_rate``.
        """
        ...

    # Context managers.
    def streaming(self, callback: EventCallback) -> StreamingSession:
        """Open a streaming session bound to ``callback`` as a context manager.

        Entering the ``with`` block starts streaming; exiting stops it
        and drains pending events, so the callback is never invoked after
        the block closes.

        Args:
            callback: Single-argument callable receiving each event.

        Returns:
            A :class:`StreamingSession` context manager.
        """
        ...

    # FLATFILES namespace getter + direct-to-disk helper.
    @property
    def flat_files(self) -> FlatFilesNamespace:
        """The flat-files namespace for bulk per-day file requests."""
        ...
    def flatfile_to_path(
        self,
        sec_type: str,
        req_type: str,
        date: str,
        path: str,
        format: Optional[str] = None,
    ) -> str:
        """Pull a flat-file blob and write it to ``path`` without decoding
        rows.

        ``sec_type`` / ``req_type`` accept the same strings as
        ``flat_files.request(...)``; ``format`` is ``"csv"`` (default) or
        ``"jsonl"``. Returns the final on-disk path (extension
        auto-appended if absent).
        """
        ...

    def __repr__(self) -> str:
        """Return a representation including historical and streaming state."""
        ...


@final
class AsyncClient:
    """Async client exposing ``await``-based historical methods.

    Each historical endpoint is available as an ``*_async`` coroutine
    (e.g. ``await client.stock_history_eod_async(...)``). The streaming
    lifecycle and subscription methods (``start_streaming``,
    ``stop_streaming``, ``subscribe``, ``streaming`` etc.) mirror those
    on :class:`Client`. Accessing the synchronous historical
    methods on this class raises ``AttributeError`` — use
    :class:`Client` for those.
    """

    def __init__(self, creds: Credentials, config: Config) -> None:
        """Connect to ThetaData with ``creds`` and ``config``.

        Args:
            creds: Account credentials.
            config: Connection configuration.

        Raises:
            ThetaDataError: If authentication or the connection fails.
        """
        ...

    @staticmethod
    def from_file(
        path: str,
        config: Optional[Config] = None,
    ) -> AsyncClient:
        """Construct an async client from a credentials file and connect.

        Args:
            path: Path to a two-line credentials file.
            config: Connection configuration; defaults to
                ``Config.production()`` when omitted.

        Returns:
            A connected :class:`AsyncClient`.

        Raises:
            ThetaDataError: If the file cannot be read or the connection
                fails.
        """
        ...

    def __repr__(self) -> str:
        """Return a representation including historical and streaming state."""
        ...

    def __getattr__(self, name: str) -> Any:
        """Resolve an ``*_async`` historical method or a streaming method.

        Raises:
            AttributeError: If ``name`` is not part of the async surface.
        """
        ...


@final
class StreamingClient:
    """Streaming-only client — opens the real-time feed and never the
    historical channel."""

    def __init__(self, creds: Credentials, config: Config) -> None:
        """Create a streaming-only client with ``creds`` and ``config``.

        Args:
            creds: Account credentials.
            config: Connection configuration.

        Raises:
            ThetaDataError: If construction fails.
        """
        ...

    @staticmethod
    def from_file(
        path: str,
        config: Optional[Config] = None,
    ) -> StreamingClient:
        """Construct a streaming-only client from a credentials file.

        Args:
            path: Path to a two-line credentials file.
            config: Connection configuration; defaults to
                ``Config.production()`` when omitted.

        Returns:
            A :class:`StreamingClient`.

        Raises:
            ThetaDataError: If the file cannot be read.
        """
        ...

    def start_streaming(self, callback: EventCallback) -> None:
        """Start streaming and register ``callback`` for incoming events.

        See :meth:`Client.start_streaming` for the callback
        contract.

        Raises:
            RuntimeError: If streaming is already started.
            ThetaDataError: If the connection cannot be opened.
        """
        ...

    def is_streaming(self) -> bool:
        """Return whether the streaming connection is currently open.

        Returns ``False`` if the dispatcher thread has failed.
        """
        ...

    def is_authenticated(self) -> bool:
        """Return whether the streaming session is currently authenticated.

        Distinct from :meth:`is_streaming`: the connection slot can
        remain populated with the authenticated flag cleared after a
        server disconnect, before :meth:`reconnect` is issued.
        """
        ...

    def stop_streaming(self) -> None:
        """Stop streaming and clear the registered callback."""
        ...

    def shutdown(self) -> None:
        """Shut down the streaming connection and clear the callback."""
        ...

    def reconnect(self) -> None:
        """Reconnect and re-register the previous callback, restoring subscriptions.

        Raises:
            RuntimeError: If no callback is registered.
        """
        ...

    def await_drain(self, timeout_ms: int) -> bool:
        """Block until the streaming consumer thread finishes firing the callback.

        Args:
            timeout_ms: Maximum time to wait, in milliseconds.

        Returns:
            ``True`` if the drain completed within the timeout, else
            ``False``.
        """
        ...

    def subscribe(self, sub: Subscription) -> None:
        """Subscribe to a single :class:`Subscription`.

        Raises:
            ThetaDataError: If the subscription is rejected.
        """
        ...

    def subscribe_many(self, subs: List[Subscription]) -> None:
        """Subscribe to several subscriptions.

        Stops at the first error and re-raises it.

        Raises:
            ThetaDataError: If any subscription is rejected.
        """
        ...

    def unsubscribe(self, sub: Subscription) -> None:
        """Cancel a single :class:`Subscription`.

        Raises:
            ThetaDataError: If the request is rejected.
        """
        ...

    def unsubscribe_many(self, subs: List[Subscription]) -> None:
        """Cancel several subscriptions.

        Raises:
            ThetaDataError: If any request is rejected.
        """
        ...

    def active_subscriptions(self) -> List[Subscription]:
        """Return a snapshot of the active per-contract subscriptions.

        Empty when streaming has not started.
        """
        ...

    def active_full_subscriptions(self) -> List[Subscription]:
        """Return a snapshot of the active full-stream subscriptions.

        Empty when streaming has not started.
        """
        ...

    def dropped_event_count(self) -> int:
        """Cumulative count of streaming events dropped on a full event ring.

        Returns 0 before :meth:`start_streaming` and after
        :meth:`stop_streaming`.
        """
        ...

    def panic_count(self) -> int:
        """Cumulative count of exceptions raised by the user callback."""
        ...

    def ring_occupancy(self) -> int:
        """Point-in-time count of events queued but not yet delivered to the callback.

        Returns 0 when streaming is not active.
        """
        ...

    def ring_capacity(self) -> int:
        """Configured streaming event-ring capacity in slots.

        The fixed denominator for :meth:`ring_occupancy`; 0 when
        streaming is not active.
        """
        ...

    def millis_since_last_event(self) -> Optional[int]:
        """Milliseconds since the most recent inbound frame, or ``None`` before streaming starts.

        A steadily growing value is the earliest signal of a dead or
        wedged connection.
        """
        ...

    def last_event_received_at_unix_nanos(self) -> int:
        """UNIX-nanosecond receive timestamp of the most recent inbound frame.

        Returns 0 before streaming starts or before any frame arrives.
        """
        ...

    def last_connected_addr(self) -> Optional[str]:
        """``host:port`` of the live streaming server, or ``None`` before streaming starts."""
        ...

    def streaming(self, callback: EventCallback) -> StreamingSession:
        """Open a streaming session bound to ``callback`` as a context manager.

        Args:
            callback: Single-argument callable receiving each event.

        Returns:
            A :class:`StreamingSession` context manager.
        """
        ...

    def __repr__(self) -> str:
        """Return a representation of the streaming client."""
        ...


@final
class HistoricalClient:
    """Historical-only client — the streaming surface is blocked.

    Exposes the same historical endpoints as :class:`Client`
    and never opens the real-time feed. Any streaming method (e.g.
    ``start_streaming`` / ``subscribe``) raises ``AttributeError``; use
    :class:`StreamingClient` or :class:`Client` for streaming.
    """

    def __init__(self, creds: Credentials, config: Config) -> None:
        """Connect a historical-only client with ``creds`` and ``config``.

        Args:
            creds: Account credentials.
            config: Connection configuration.

        Raises:
            ThetaDataError: If authentication or the connection fails.
        """
        ...

    @staticmethod
    def from_file(
        path: str,
        config: Optional[Config] = None,
    ) -> HistoricalClient:
        """Construct a historical-only client from a credentials file.

        Args:
            path: Path to a two-line credentials file.
            config: Connection configuration; defaults to
                ``Config.production()`` when omitted.

        Returns:
            A connected :class:`HistoricalClient`.

        Raises:
            ThetaDataError: If the file cannot be read or the connection
                fails.
        """
        ...

    def __repr__(self) -> str:
        """Return a representation of the historical client."""
        ...

    def __getattr__(self, name: str) -> Any:
        """Resolve a historical method.

        Raises:
            AttributeError: If ``name`` is a streaming method (blocked on
                this client).
        """
        ...


# ─────────────────────────────────────────────────────────────────────
# Streaming context managers + iterator
# ─────────────────────────────────────────────────────────────────────


@final
class StreamingSession:
    """Context manager for callback-driven streaming.

    Acquired via :py:meth:`Client.streaming` /
    :py:meth:`StreamingClient.streaming`. Entering the ``with`` block starts
    streaming; exiting stops it and drains pending events. Subscription
    and lifecycle methods of the bound client are reachable directly on
    the session.
    """

    def __enter__(self) -> StreamingSession:
        """Start streaming and return the session."""
        ...

    def __exit__(
        self,
        exc_type: Optional[Type[BaseException]],
        exc_value: Optional[BaseException],
        traceback: Optional[Any],
    ) -> bool:
        """Stop streaming and drain pending events; never suppresses exceptions."""
        ...

    def __getattr__(self, name: str) -> Any:
        """Resolve a subscription or lifecycle method on the bound client.

        Raises:
            AttributeError: If ``name`` is not available on the bound
                client.
        """
        ...


@final
class FlatFileRowList:
    """Typed list of flat-file rows. One row per `(symbol, date, ...)`."""

    def __len__(self) -> int:
        """Return the number of rows."""
        ...

    def to_list(self) -> List[Any]:
        """Return the rows as a list of dicts, one dict per row."""
        ...

    def to_arrow(self) -> Any:
        """Return the rows as a ``pyarrow.Table``."""
        ...

    def to_pandas(self) -> Any:
        """Return the rows as a ``pandas.DataFrame``. Requires pandas and pyarrow."""
        ...

    def to_polars(self) -> Any:
        """Return the rows as a ``polars.DataFrame``. Requires polars and pyarrow."""
        ...


@final
class FlatFilesNamespace:
    """Namespace accessor returned by :py:attr:`Client.flat_files`.

    Each method maps one ``(SecType, ReqType)`` pair to a
    :class:`FlatFileRowList`. The wildcard :py:meth:`request` dispatches
    dynamically by string identifiers.
    """

    def option_trade_quote(self, date: str) -> FlatFileRowList:
        """Return the decoded option-trade-quote flat file for ``date`` (``YYYYMMDD``)."""
        ...

    def option_open_interest(self, date: str) -> FlatFileRowList:
        """Return the decoded option-open-interest flat file for ``date`` (``YYYYMMDD``)."""
        ...

    def option_eod(self, date: str) -> FlatFileRowList:
        """Return the decoded option-EOD flat file for ``date`` (``YYYYMMDD``)."""
        ...

    def stock_trade_quote(self, date: str) -> FlatFileRowList:
        """Return the decoded stock-trade-quote flat file for ``date`` (``YYYYMMDD``)."""
        ...

    def stock_eod(self, date: str) -> FlatFileRowList:
        """Return the decoded stock-EOD flat file for ``date`` (``YYYYMMDD``)."""
        ...

    def request(self, sec_type: str, req_type: str, date: str) -> FlatFileRowList:
        """Return a decoded flat file selected by string identifiers.

        Args:
            sec_type: Security type, e.g. ``"OPTION"`` / ``"STOCK"``.
            req_type: Request type, e.g. ``"TRADE_QUOTE"`` / ``"EOD"``.
            date: The trading day as a ``YYYYMMDD`` string.

        Returns:
            The decoded :class:`FlatFileRowList`.

        Raises:
            InvalidParameterError: If the ``(sec_type, req_type)`` pair is
                not one the flat-file distribution serves.
        """
        ...


class ThetaDataError(Exception):
    """Base exception for every typed error this binding raises."""

    ...


class AuthenticationError(ThetaDataError):
    """Authentication failed. Parent of :class:`InvalidCredentialsError`."""

    ...


@final
class InvalidCredentialsError(AuthenticationError):
    """The supplied email or password was rejected by the server."""

    ...


@final
class SubscriptionError(ThetaDataError):
    """Tier / plan does not cover the request (gRPC ``PermissionDenied``)."""

    ...


@final
class RateLimitError(ThetaDataError):
    """Rate limit / quota exhausted (gRPC ``ResourceExhausted``, HTTP 429).

    ``retry_after`` is the server-supplied minimum back-off in seconds
    when the upstream attached a ``google.rpc.RetryInfo`` detail, or
    ``None`` when no hint was supplied. The attribute is always present
    so callers can read it unconditionally.
    """

    retry_after: Optional[float]
    """Server-supplied minimum back-off in seconds, or ``None`` when the upstream attached no retry hint. Always present so callers can read it unconditionally."""


@final
class InvalidParameterError(ThetaDataError):
    """A client-side parameter was rejected by input validation.

    Distinct from the root :class:`ThetaDataError` so a malformed-but-
    rejected argument (bad value, out-of-range number, missing required
    field) is distinguishable by class from an unrelated configuration
    fault (config-file I/O, TOML parse), which is raised as
    :class:`ConfigError`.
    """

    ...


@final
class ConfigError(ThetaDataError):
    """An environmental configuration fault.

    Raised on a config-file read failure, a TOML parse error, or an
    internal config invariant. Distinct from :class:`InvalidParameterError`
    (a rejected user-supplied argument): a :class:`ConfigError` is the
    environment, not the call site.
    """

    ...


@final
class SchemaMismatchError(ThetaDataError):
    """Decoder schema mismatch — usually a server proto bump."""

    ...


@final
class NetworkError(ThetaDataError):
    """Transport-layer failure (TCP / TLS / IO) other than ``Unavailable``."""

    ...


@final
class UnavailableError(ThetaDataError):
    """Upstream unavailable (gRPC ``Unavailable``, often retryable)."""

    ...


@final
class DeadlineExceededError(ThetaDataError):
    """Per-request deadline elapsed (``timeout_ms`` / gRPC ``DeadlineExceeded``)."""

    ...


@final
class NotFoundError(ThetaDataError):
    """Empty result / unknown contract (gRPC ``NotFound``)."""

    ...


@final
class StreamError(ThetaDataError):
    """Streaming protocol / state-machine failure."""

    ...


# ── Back-compatibility aliases ────────────────────────────────────────
#
# `NoDataFoundError` and `TimeoutError` are registered as assignment
# aliases of their canonical replacements (`NotFoundError` /
# `DeadlineExceededError`) — the same class object under both names — so
# existing `except thetadatadx.NoDataFoundError` / `except
# thetadatadx.TimeoutError` clauses keep catching the dispatched
# canonical class. New code should use the canonical names. Typed here
# as the canonical class so `except` narrowing matches runtime identity.
NoDataFoundError: Type[NotFoundError]
"""Back-compatibility alias of :class:`NotFoundError` — the same class object under both names."""
TimeoutError: Type[DeadlineExceededError]
"""Back-compatibility alias of :class:`DeadlineExceededError` — the same class object under both names."""


# ─────────────────────────────────────────────────────────────────────
# Utility module-level entry points
# ─────────────────────────────────────────────────────────────────────


def decode_response_bytes(endpoint: str, chunks: List[bytes]) -> Any:
    """Decode raw response chunks for ``endpoint`` into the typed result.

    ``chunks`` are the wire-frame byte buffers for one historical
    response in order; ``endpoint`` selects the decoder. Returns the
    endpoint's typed ``<Tick>List`` wrapper.
    """
    ...


def split_date_range(
    start: str,
    end: str,
) -> List[Tuple[str, str]]:
    """Split an inclusive ``start``..``end`` date range into per-request
    ``(start, end)`` sub-ranges sized to the server's per-call window.

    Dates are ``YYYYMMDD`` strings; the returned pairs cover the range
    contiguously in chronological order.
    """
    ...


def all_greeks(
    spot: float,
    strike: float,
    rate: float,
    div_yield: float,
    tte: float,
    option_price: float,
    right: str,
) -> AllGreeks:
    """Compute all Black-Scholes Greeks plus implied volatility for one option.

    ``right`` is ``"C"`` / ``"P"``; ``tte`` is time to expiry in years.
    The implied volatility is solved from ``option_price`` and seeds the
    Greek derivatives. Returns an :class:`AllGreeks` with every field
    populated.
    """
    ...


def implied_volatility(
    spot: float,
    strike: float,
    rate: float,
    div_yield: float,
    tte: float,
    option_price: float,
    right: str,
) -> tuple[float, float]:
    """Solve the Black-Scholes implied volatility for one option.

    Args:
        spot: Underlying spot price.
        strike: Option strike price.
        rate: Risk-free interest rate (annualised, decimal).
        div_yield: Continuous dividend yield (annualised, decimal).
        tte: Time to expiry in years.
        option_price: Observed option price to invert.
        right: Option right, ``"C"`` (call) or ``"P"`` (put).

    Returns:
        A ``(iv, iv_error)`` pair: the implied volatility and the
        residual ``(model_price - option_price) / option_price`` at that
        volatility.
    """
    ...


@final
class AllGreeks:
    """All 23 Black-Scholes Greeks plus IV returned by :func:`all_greeks`.

    Fields are grouped as the model value and IV pair first, then
    first-, second-, and third-order Greeks, then the ``d1`` / ``d2``
    auxiliaries. Every attribute is a plain ``float``.
    """

    value: float
    """Black-Scholes theoretical option value."""
    iv: float
    """Implied volatility solved from the observed option price (annualised, decimal)."""
    iv_error: float
    """Relative residual of the implied-volatility solve, ``(model_price - option_price) / option_price``."""
    delta: float
    """First derivative of value with respect to spot."""
    gamma: float
    """Second derivative of value with respect to spot (rate of change of delta)."""
    theta: float
    """Sensitivity of value to the passage of time, expressed per calendar day."""
    vega: float
    """Sensitivity of value to volatility."""
    rho: float
    """Sensitivity of value to the risk-free rate."""
    vanna: float
    """Sensitivity of delta to volatility (equivalently, of vega to spot)."""
    charm: float
    """Sensitivity of delta to the passage of time (delta decay)."""
    vomma: float
    """Sensitivity of vega to volatility (vega convexity)."""
    veta: float
    """Sensitivity of vega to the passage of time."""
    vera: float
    """Sensitivity of vega to the risk-free rate."""
    speed: float
    """Sensitivity of gamma to spot (third derivative of value in spot)."""
    zomma: float
    """Sensitivity of gamma to volatility."""
    color: float
    """Sensitivity of gamma to the passage of time (gamma decay)."""
    ultima: float
    """Sensitivity of vomma to volatility (third-order volatility sensitivity)."""
    d1: float
    """The Black-Scholes ``d1`` term."""
    d2: float
    """The Black-Scholes ``d2`` term (``d1 - sigma * sqrt(t)``)."""
    dual_delta: float
    """Sensitivity of value to the strike."""
    dual_gamma: float
    """Sensitivity of dual delta to the strike."""
    epsilon: float
    """Sensitivity of value to the dividend yield."""
    lambda_: float
    """Option elasticity: percentage change in value per percentage change in spot (``delta * spot / value``). Carries the PEP 8 trailing-underscore escape because ``lambda`` is a reserved Python keyword."""


# ─────────────────────────────────────────────────────────────────────
# Module-level fallback: the per-endpoint historical builders and typed
# `<Tick>List` wrappers share one structural shape and resolve to `Any`
# at the module level rather than being annotated individually.
# ─────────────────────────────────────────────────────────────────────


def __getattr__(name: str) -> Any:
    """Resolve a per-endpoint historical builder or typed result wrapper."""
    ...
