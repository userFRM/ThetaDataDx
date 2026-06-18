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

from datetime import date, datetime, time
from typing import (
    Any,
    Awaitable,
    Callable,
    List,
    Literal,
    Optional,
    Sequence,
    Tuple,
    Type,
    Union,
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

    @staticmethod
    def from_api_key(api_key: str) -> Credentials:
        """Authenticate with an API key instead of an email and password.

        Args:
            api_key: The API key. Trimmed and held as secret material.

        Returns:
            The built :class:`Credentials`.
        """
        ...

    @staticmethod
    def from_api_key_with_email(email: str, api_key: str) -> Credentials:
        """Authenticate with an API key paired with an account email.

        Args:
            email: Account email (lowercased and trimmed; an empty email
                is dropped).
            api_key: The API key. Trimmed and held as secret material.

        Returns:
            The built :class:`Credentials`.
        """
        ...

    @staticmethod
    def from_env(path: str) -> Credentials:
        """Source credentials from the environment, falling back to a file.

        When ``THETADATA_API_KEY`` is set and non-empty an API key is
        used; otherwise the two-line file at ``path`` is read.

        Args:
            path: Path to the credentials file used as the fallback.

        Returns:
            The sourced :class:`Credentials`.

        Raises:
            ThetaDataError: If the fallback file cannot be read or is
                malformed.
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
    warn_on_buffered_threshold_bytes: int
    """Byte ceiling above which a buffered (non-``.stream()``) historical response logs a warning pointing the caller at the streaming surface. ``0`` disables the warning; the default is ``100 * 1024 * 1024`` (100 MiB). The data is still delivered."""
    request_timeout_secs: int
    """Default per-request deadline, in seconds, for historical queries. Bounds every request that did not set its own deadline, so a live-but-silent stream resolves to a timeout instead of blocking forever. ``0`` disables the default; the default is ``300`` (5 minutes)."""
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
    flatfiles_connect_timeout_secs: int
    """TCP + TLS connect timeout, in seconds, for one flat-file host attempt (default 10)."""
    flatfiles_read_timeout_secs: int
    """Read timeout, in seconds, for a single flat-file response frame (default 60)."""
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
    def index(symbol: str) -> Contract:
        """Construct an index contract for ``symbol``."""
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

    def __str__(self) -> str:
        """Return the contract's wire-format string."""
        ...

    def __eq__(self, other: object) -> bool:
        """Return whether ``other`` is a contract with the same identity."""
        ...

    def __hash__(self) -> int:
        """Return a hash consistent with ``__eq__``, so equal contracts can share a dict key or set slot."""
        ...


@final
class ContractRef:
    """Read-only contract identifier surfaced on every streaming event.

    Distinct from the fluent `Contract` builder — `ContractRef` is what
    `event.contract` returns inside a streaming callback, with the
    resolved `symbol`, `sec_type`, `expiration`, `right`, `strike`
    (dollars, the same unit historical rows carry under the same name),
    and `strike_thousandths` (the exact wire integer). The fluent
    `Contract` (above) is the one users instantiate to subscribe.
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

    @property
    def strike_thousandths(self) -> Optional[int]:
        """Strike in thousandths of a dollar (a ``$550.00`` strike is
        ``550000``); the exact wire integer. ``None`` for non-options."""
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


# ─────────────────────────────────────────────────────────────────────
# String-valued parameter enums
# ─────────────────────────────────────────────────────────────────────
#
# Each class is a frozen, value-comparable handle whose members are the
# only valid instances. The backing ``value`` is the lowercase wire token
# the endpoints expect; ``str(member)`` returns that token and ``repr``
# returns ``"<Class>.<token>"``. Instances are hashable and compare by
# value, so they are usable as dict keys and in sets.


@final
class Right:
    """Option right accepted by the contract and request builders."""

    CALL: Right
    """Call options."""
    PUT: Right
    """Put options."""
    BOTH: Right
    """Both calls and puts."""

    @property
    def value(self) -> str:
        """The wire token for this right (e.g. ``"call"``)."""
        ...

    def __str__(self) -> str:
        """Return the wire token (e.g. ``"call"``)."""
        ...

    def __repr__(self) -> str:
        """Return a representation (e.g. ``"Right.call"``)."""
        ...

    def __eq__(self, other: object) -> bool:
        """Return whether ``other`` is the same right."""
        ...

    def __hash__(self) -> int:
        """Return a hash consistent with :meth:`__eq__`."""
        ...


@final
class Venue:
    """Quote-venue selector for venue-scoped quote requests."""

    NQB: Venue
    """The national best bid and offer composite venue."""
    UTP_CTA: Venue
    """The combined UTP / CTA tape venue."""

    @property
    def value(self) -> str:
        """The wire token for this venue (e.g. ``"nqb"``)."""
        ...

    def __str__(self) -> str:
        """Return the wire token (e.g. ``"nqb"``)."""
        ...

    def __repr__(self) -> str:
        """Return a representation (e.g. ``"Venue.nqb"``)."""
        ...

    def __eq__(self, other: object) -> bool:
        """Return whether ``other`` is the same venue."""
        ...

    def __hash__(self) -> int:
        """Return a hash consistent with :meth:`__eq__`."""
        ...


@final
class Interval:
    """Aggregation interval for bar / OHLC historical requests."""

    TICK: Interval
    """Per-tick, no aggregation."""
    MS_10: Interval
    """10-millisecond bars."""
    MS_100: Interval
    """100-millisecond bars."""
    MS_500: Interval
    """500-millisecond bars."""
    S_1: Interval
    """1-second bars."""
    S_5: Interval
    """5-second bars."""
    S_10: Interval
    """10-second bars."""
    S_15: Interval
    """15-second bars."""
    S_30: Interval
    """30-second bars."""
    M_1: Interval
    """1-minute bars."""
    M_5: Interval
    """5-minute bars."""
    M_10: Interval
    """10-minute bars."""
    M_15: Interval
    """15-minute bars."""
    M_30: Interval
    """30-minute bars."""
    H_1: Interval
    """1-hour bars."""

    @property
    def value(self) -> str:
        """The wire token for this interval (e.g. ``"1m"``)."""
        ...

    def __str__(self) -> str:
        """Return the wire token (e.g. ``"1m"``)."""
        ...

    def __repr__(self) -> str:
        """Return a representation (e.g. ``"Interval.1m"``)."""
        ...

    def __eq__(self, other: object) -> bool:
        """Return whether ``other`` is the same interval."""
        ...

    def __hash__(self) -> int:
        """Return a hash consistent with :meth:`__eq__`."""
        ...


@final
class RateType:
    """Reference-rate selector for interest-rate requests."""

    SOFR: RateType
    """The Secured Overnight Financing Rate."""
    TREASURY_M1: RateType
    """1-month Treasury rate."""
    TREASURY_M3: RateType
    """3-month Treasury rate."""
    TREASURY_M6: RateType
    """6-month Treasury rate."""
    TREASURY_Y1: RateType
    """1-year Treasury rate."""
    TREASURY_Y2: RateType
    """2-year Treasury rate."""
    TREASURY_Y3: RateType
    """3-year Treasury rate."""
    TREASURY_Y5: RateType
    """5-year Treasury rate."""
    TREASURY_Y7: RateType
    """7-year Treasury rate."""
    TREASURY_Y10: RateType
    """10-year Treasury rate."""
    TREASURY_Y20: RateType
    """20-year Treasury rate."""
    TREASURY_Y30: RateType
    """30-year Treasury rate."""

    @property
    def value(self) -> str:
        """The wire token for this rate (e.g. ``"sofr"``)."""
        ...

    def __str__(self) -> str:
        """Return the wire token (e.g. ``"sofr"``)."""
        ...

    def __repr__(self) -> str:
        """Return a representation (e.g. ``"RateType.sofr"``)."""
        ...

    def __eq__(self, other: object) -> bool:
        """Return whether ``other`` is the same rate type."""
        ...

    def __hash__(self) -> int:
        """Return a hash consistent with :meth:`__eq__`."""
        ...


@final
class RequestType:
    """Per-row request kind for the flat-file and bar request builders."""

    TRADE: RequestType
    """Trade rows."""
    QUOTE: RequestType
    """Quote rows."""
    EOD: RequestType
    """End-of-day summary rows."""
    OHLC: RequestType
    """Open / high / low / close bar rows."""

    @property
    def value(self) -> str:
        """The wire token for this request type (e.g. ``"trade"``)."""
        ...

    def __str__(self) -> str:
        """Return the wire token (e.g. ``"trade"``)."""
        ...

    def __repr__(self) -> str:
        """Return a representation (e.g. ``"RequestType.trade"``)."""
        ...

    def __eq__(self, other: object) -> bool:
        """Return whether ``other`` is the same request type."""
        ...

    def __hash__(self) -> int:
        """Return a hash consistent with :meth:`__eq__`."""
        ...


@final
class Version:
    """Endpoint schema-version selector."""

    LATEST: Version
    """The latest schema version the server serves."""
    V1: Version
    """The pinned first schema version."""

    @property
    def value(self) -> str:
        """The wire token for this version (e.g. ``"latest"``)."""
        ...

    def __str__(self) -> str:
        """Return the wire token (e.g. ``"latest"``)."""
        ...

    def __repr__(self) -> str:
        """Return a representation (e.g. ``"Version.latest"``)."""
        ...

    def __eq__(self, other: object) -> bool:
        """Return whether ``other`` is the same version."""
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
class MarketValue:
    """A real-time MarketValue tick — a theoretical market value derived from the live bid/ask."""

    contract: ContractRef
    """The contract this market-value tick is for."""
    ms_of_day: int
    """Milliseconds since midnight Eastern Time when the market value was recorded."""
    market_bid: float
    """Quote bid after a size-imbalance and spread-aware adjustment, in dollars."""
    market_ask: float
    """Quote ask after a size-imbalance and spread-aware adjustment, in dollars."""
    market_price: float
    """Midpoint of ``market_bid`` and ``market_ask``, in dollars."""
    date: int
    """Trading date as a ``YYYYMMDD`` integer."""
    received_at_ns: int
    """Wall-clock nanoseconds since the UNIX epoch, captured when the frame was decoded."""

    @property
    def kind(self) -> str:
        """Event kind discriminator (``"market_value"``)."""
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


# --- BEGIN GENERATED HISTORICAL VIEW (endpoint_surface.toml) ---
# Generated from endpoint_surface.toml; do not edit by hand. Run
# `cargo run -p thetadatadx --bin generate_sdk_surfaces` to refresh.
#
# The typed list wrappers and fluent builder classes returned below are
# registered at runtime by the compiled extension and resolve through
# the module-level `__getattr__`; they are aliased to `Any` here so the
# method signatures stay precise without re-enumerating every wrapper's
# converter surface.
StringList = Any
StockListSymbolsBuilder = Any
StockListDatesBuilder = Any
OhlcTick = Any
StockSnapshotOhlcBuilder = Any
TradeTick = Any
StockSnapshotTradeBuilder = Any
QuoteTick = Any
StockSnapshotQuoteBuilder = Any
MarketValueTick = Any
StockSnapshotMarketValueBuilder = Any
EodTickList = Any
StockHistoryEodBuilder = Any
OhlcTickList = Any
StockHistoryOhlcBuilder = Any
TradeTickList = Any
StockHistoryTradeBuilder = Any
QuoteTickList = Any
StockHistoryQuoteBuilder = Any
TradeQuoteTickList = Any
StockHistoryTradeQuoteBuilder = Any
StockAtTimeTradeBuilder = Any
StockAtTimeQuoteBuilder = Any
OptionListSymbolsBuilder = Any
OptionListDatesBuilder = Any
OptionListExpirationsBuilder = Any
OptionListStrikesBuilder = Any
OptionContractList = Any
OptionListContractsBuilder = Any
OptionSnapshotOhlcBuilder = Any
OptionSnapshotTradeBuilder = Any
OptionSnapshotQuoteBuilder = Any
OpenInterestTick = Any
OptionSnapshotOpenInterestBuilder = Any
OptionSnapshotMarketValueBuilder = Any
IvTick = Any
OptionSnapshotGreeksImpliedVolatilityBuilder = Any
GreeksAllTick = Any
OptionSnapshotGreeksAllBuilder = Any
GreeksFirstOrderTick = Any
OptionSnapshotGreeksFirstOrderBuilder = Any
GreeksSecondOrderTick = Any
OptionSnapshotGreeksSecondOrderBuilder = Any
GreeksThirdOrderTick = Any
OptionSnapshotGreeksThirdOrderBuilder = Any
OptionHistoryEodBuilder = Any
OptionHistoryOhlcBuilder = Any
OptionHistoryTradeBuilder = Any
OptionHistoryQuoteBuilder = Any
OptionHistoryTradeQuoteBuilder = Any
OpenInterestTickList = Any
OptionHistoryOpenInterestBuilder = Any
GreeksEodTickList = Any
OptionHistoryGreeksEodBuilder = Any
GreeksAllTickList = Any
OptionHistoryGreeksAllBuilder = Any
TradeGreeksAllTickList = Any
OptionHistoryTradeGreeksAllBuilder = Any
GreeksFirstOrderTickList = Any
OptionHistoryGreeksFirstOrderBuilder = Any
TradeGreeksFirstOrderTickList = Any
OptionHistoryTradeGreeksFirstOrderBuilder = Any
GreeksSecondOrderTickList = Any
OptionHistoryGreeksSecondOrderBuilder = Any
TradeGreeksSecondOrderTickList = Any
OptionHistoryTradeGreeksSecondOrderBuilder = Any
GreeksThirdOrderTickList = Any
OptionHistoryGreeksThirdOrderBuilder = Any
TradeGreeksThirdOrderTickList = Any
OptionHistoryTradeGreeksThirdOrderBuilder = Any
IvTickList = Any
OptionHistoryGreeksImpliedVolatilityBuilder = Any
TradeGreeksImpliedVolatilityTickList = Any
OptionHistoryTradeGreeksImpliedVolatilityBuilder = Any
OptionAtTimeTradeBuilder = Any
OptionAtTimeQuoteBuilder = Any
IndexListSymbolsBuilder = Any
IndexListDatesBuilder = Any
IndexSnapshotOhlcBuilder = Any
PriceTick = Any
IndexSnapshotPriceBuilder = Any
IndexSnapshotMarketValueBuilder = Any
IndexHistoryEodBuilder = Any
IndexHistoryOhlcBuilder = Any
PriceTickList = Any
IndexHistoryPriceBuilder = Any
IndexPriceAtTimeTickList = Any
IndexAtTimePriceBuilder = Any
CalendarDay = Any
CalendarOpenTodayBuilder = Any
CalendarOnDateBuilder = Any
CalendarYearBuilder = Any
InterestRateTickList = Any
InterestRateHistoryEodBuilder = Any
StockHistoryOhlcRangeBuilder = Any

@final
class HistoricalView:
    """Historical-data sub-namespace returned by :attr:`Client.historical`.

    Exposes every historical / list / snapshot / at-time endpoint as a
    method: the synchronous call, its ``*_async`` awaitable companion,
    and a ``*_builder`` fluent constructor. The surface is projected
    from the same endpoint definition that drives the runtime methods,
    so the stubs cannot drift from the installed extension.
    """

    def stock_list_symbols(
        self,
        *,
        timeout_ms: Optional[int] = None,
    ) -> StringList:
        """List all available stock ticker symbols.

        A symbol can be defined as a unique identifier for a stock / underlying asset. Common terms also include: root, ticker, and underlying. This endpoint returns all traded symbols for stocks. This endpoint is updated overnight.
        """
        ...

    def stock_list_symbols_async(
        self,
        *,
        timeout_ms: Optional[int] = None,
    ) -> Awaitable[StringList]:
        """List all available stock ticker symbols.

        A symbol can be defined as a unique identifier for a stock / underlying asset. Common terms also include: root, ticker, and underlying. This endpoint returns all traded symbols for stocks. This endpoint is updated overnight.


        Awaitable companion of the sync variant. The returned object resolves the request off the calling thread so a running event loop keeps servicing other coroutines.
        """
        ...

    def stock_list_symbols_builder(self) -> StockListSymbolsBuilder:
        """Fluent builder for `stock_list_symbols`. Chain the optional setters, then call `.list()` (or `.list_async()`) to execute; the returned typed list wrapper exposes `.to_list()` / `.to_arrow()` / `.to_pandas()` / `.to_polars()`."""
        ...

    def stock_list_dates(
        self,
        request_type: str,
        symbol: str,
        *,
        timeout_ms: Optional[int] = None,
    ) -> StringList:
        """List available dates for a stock by request type (EOD, TRADE, QUOTE, etc.).

        Lists all dates of data that are available for a stock with a given request type and symbol. This endpoint is updated overnight.
        """
        ...

    def stock_list_dates_async(
        self,
        request_type: str,
        symbol: str,
        *,
        timeout_ms: Optional[int] = None,
    ) -> Awaitable[StringList]:
        """List available dates for a stock by request type (EOD, TRADE, QUOTE, etc.).

        Lists all dates of data that are available for a stock with a given request type and symbol. This endpoint is updated overnight.


        Awaitable companion of the sync variant. The returned object resolves the request off the calling thread so a running event loop keeps servicing other coroutines.
        """
        ...

    def stock_list_dates_builder(
        self,
        request_type: str,
        symbol: str,
    ) -> StockListDatesBuilder:
        """Fluent builder for `stock_list_dates`. Chain the optional setters, then call `.list()` (or `.list_async()`) to execute; the returned typed list wrapper exposes `.to_list()` / `.to_arrow()` / `.to_pandas()` / `.to_polars()`."""
        ...

    def stock_snapshot_ohlc(
        self,
        symbols: Union[str, Sequence[str]],
        *,
        venue: Optional[str] = None,
        min_time: Optional[Union[str, time, datetime]] = None,
        timeout_ms: Optional[int] = None,
    ) -> List[OhlcTick]:
        """Get the latest OHLC snapshot for one or more stocks.

        Provides a real-time Open, High, Low, Close for the current day.
        * Returns a real-time session OHLC from the Nasdaq Basic feed if the account has a stocks standard or pro subscription.
        * Returns a 15-minute delayed session OHLC from the UTP & CTA feeds if the account has the stocks value subscription.
        * Theta Data resets its snapshot cache at midnight ET every day. This endpoint may not work on a weekend where there were no eligible messages sent over exchange feeds. We recommend using historic requests during the weekend.

        Defaults (upstream):
        - `venue`: `"nqb"`
        """
        ...

    def stock_snapshot_ohlc_async(
        self,
        symbols: Union[str, Sequence[str]],
        *,
        venue: Optional[str] = None,
        min_time: Optional[Union[str, time, datetime]] = None,
        timeout_ms: Optional[int] = None,
    ) -> Awaitable[List[OhlcTick]]:
        """Get the latest OHLC snapshot for one or more stocks.

        Provides a real-time Open, High, Low, Close for the current day.
        * Returns a real-time session OHLC from the Nasdaq Basic feed if the account has a stocks standard or pro subscription.
        * Returns a 15-minute delayed session OHLC from the UTP & CTA feeds if the account has the stocks value subscription.
        * Theta Data resets its snapshot cache at midnight ET every day. This endpoint may not work on a weekend where there were no eligible messages sent over exchange feeds. We recommend using historic requests during the weekend.

        Defaults (upstream):
        - `venue`: `"nqb"`


        Awaitable companion of the sync variant. The returned object resolves the request off the calling thread so a running event loop keeps servicing other coroutines.
        """
        ...

    def stock_snapshot_ohlc_builder(
        self,
        symbols: Union[str, Sequence[str]],
    ) -> StockSnapshotOhlcBuilder:
        """Fluent builder for `stock_snapshot_ohlc`. Chain the optional setters, then call `.list()` (or `.list_async()`) to execute; the returned typed list wrapper exposes `.to_list()` / `.to_arrow()` / `.to_pandas()` / `.to_polars()`."""
        ...

    def stock_snapshot_trade(
        self,
        symbols: Union[str, Sequence[str]],
        *,
        venue: Optional[str] = None,
        min_time: Optional[Union[str, time, datetime]] = None,
        timeout_ms: Optional[int] = None,
    ) -> List[TradeTick]:
        """Get the latest trade snapshot for one or more stocks.

        Returns a real-time last trade from the Nasdaq Basic feed if the account has a stocks standard or pro subscription.

        - Theta Data resets its snapshot cache at midnight ET every day. This endpoint may not work on a weekend where there were no eligible messages sent over exchange feeds. We recommend using historic requests during the weekend.

        Defaults (upstream):
        - `venue`: `"nqb"`
        """
        ...

    def stock_snapshot_trade_async(
        self,
        symbols: Union[str, Sequence[str]],
        *,
        venue: Optional[str] = None,
        min_time: Optional[Union[str, time, datetime]] = None,
        timeout_ms: Optional[int] = None,
    ) -> Awaitable[List[TradeTick]]:
        """Get the latest trade snapshot for one or more stocks.

        Returns a real-time last trade from the Nasdaq Basic feed if the account has a stocks standard or pro subscription.

        - Theta Data resets its snapshot cache at midnight ET every day. This endpoint may not work on a weekend where there were no eligible messages sent over exchange feeds. We recommend using historic requests during the weekend.

        Defaults (upstream):
        - `venue`: `"nqb"`


        Awaitable companion of the sync variant. The returned object resolves the request off the calling thread so a running event loop keeps servicing other coroutines.
        """
        ...

    def stock_snapshot_trade_builder(
        self,
        symbols: Union[str, Sequence[str]],
    ) -> StockSnapshotTradeBuilder:
        """Fluent builder for `stock_snapshot_trade`. Chain the optional setters, then call `.list()` (or `.list_async()`) to execute; the returned typed list wrapper exposes `.to_list()` / `.to_arrow()` / `.to_pandas()` / `.to_polars()`."""
        ...

    def stock_snapshot_quote(
        self,
        symbols: Union[str, Sequence[str]],
        *,
        venue: Optional[str] = None,
        min_time: Optional[Union[str, time, datetime]] = None,
        timeout_ms: Optional[int] = None,
    ) -> List[QuoteTick]:
        """Get the latest NBBO quote snapshot for one or more stocks.

        * Returns a real-time last BBO quote from the Nasdaq Basic feed if the account has a stocks standard or pro subscription.
        * Returns a 15-minute delayed NBBO quote from the UTP & CTA feeds account has the stocks value subscription subscription.
        - Theta Data resets its snapshot cache at midnight ET every day. This endpoint may not work on a weekend where there were no eligible messages sent over exchange feeds. We recommend using historic requests during the weekend.

        Defaults (upstream):
        - `venue`: `"nqb"`
        """
        ...

    def stock_snapshot_quote_async(
        self,
        symbols: Union[str, Sequence[str]],
        *,
        venue: Optional[str] = None,
        min_time: Optional[Union[str, time, datetime]] = None,
        timeout_ms: Optional[int] = None,
    ) -> Awaitable[List[QuoteTick]]:
        """Get the latest NBBO quote snapshot for one or more stocks.

        * Returns a real-time last BBO quote from the Nasdaq Basic feed if the account has a stocks standard or pro subscription.
        * Returns a 15-minute delayed NBBO quote from the UTP & CTA feeds account has the stocks value subscription subscription.
        - Theta Data resets its snapshot cache at midnight ET every day. This endpoint may not work on a weekend where there were no eligible messages sent over exchange feeds. We recommend using historic requests during the weekend.

        Defaults (upstream):
        - `venue`: `"nqb"`


        Awaitable companion of the sync variant. The returned object resolves the request off the calling thread so a running event loop keeps servicing other coroutines.
        """
        ...

    def stock_snapshot_quote_builder(
        self,
        symbols: Union[str, Sequence[str]],
    ) -> StockSnapshotQuoteBuilder:
        """Fluent builder for `stock_snapshot_quote`. Chain the optional setters, then call `.list()` (or `.list_async()`) to execute; the returned typed list wrapper exposes `.to_list()` / `.to_arrow()` / `.to_pandas()` / `.to_polars()`."""
        ...

    def stock_snapshot_market_value(
        self,
        symbols: Union[str, Sequence[str]],
        *,
        venue: Optional[str] = None,
        min_time: Optional[Union[str, time, datetime]] = None,
        timeout_ms: Optional[int] = None,
    ) -> List[MarketValueTick]:
        """Get the latest market value snapshot for one or more stocks.

        * Returns a real-time market value derived from the last BBO quote from the Nasdaq Basic feed if the account has a stocks standard or pro subscription.
        * Returns a 15-minute delayed market value derived from an NBBO quote from the UTP & CTA feeds if the account has the stocks value subscription subscription.
        - Theta Data resets its snapshot cache at midnight ET every day. This endpoint may not work on a weekend where there were no eligible messages sent over exchange feeds. We recommend using historic requests during the weekend.

        Defaults (upstream):
        - `venue`: `"nqb"`
        """
        ...

    def stock_snapshot_market_value_async(
        self,
        symbols: Union[str, Sequence[str]],
        *,
        venue: Optional[str] = None,
        min_time: Optional[Union[str, time, datetime]] = None,
        timeout_ms: Optional[int] = None,
    ) -> Awaitable[List[MarketValueTick]]:
        """Get the latest market value snapshot for one or more stocks.

        * Returns a real-time market value derived from the last BBO quote from the Nasdaq Basic feed if the account has a stocks standard or pro subscription.
        * Returns a 15-minute delayed market value derived from an NBBO quote from the UTP & CTA feeds if the account has the stocks value subscription subscription.
        - Theta Data resets its snapshot cache at midnight ET every day. This endpoint may not work on a weekend where there were no eligible messages sent over exchange feeds. We recommend using historic requests during the weekend.

        Defaults (upstream):
        - `venue`: `"nqb"`


        Awaitable companion of the sync variant. The returned object resolves the request off the calling thread so a running event loop keeps servicing other coroutines.
        """
        ...

    def stock_snapshot_market_value_builder(
        self,
        symbols: Union[str, Sequence[str]],
    ) -> StockSnapshotMarketValueBuilder:
        """Fluent builder for `stock_snapshot_market_value`. Chain the optional setters, then call `.list()` (or `.list_async()`) to execute; the returned typed list wrapper exposes `.to_list()` / `.to_arrow()` / `.to_pandas()` / `.to_polars()`."""
        ...

    def stock_history_eod(
        self,
        symbol: str,
        start_date: Union[str, date, datetime],
        end_date: Union[str, date, datetime],
        *,
        timeout_ms: Optional[int] = None,
    ) -> EodTickList:
        """Fetch end-of-day stock data for a date range. Returns OHLCV + bid/ask per trading day.

        Since the equity SIPs only generate a partial EOD report, Theta Data generates a national EOD report at 17:15 ET each day. ``created`` represents the datetime the report was generated and ``last_trade`` represents the datetime of the last trade. The quote in the response represents the last NBBO reported by CTA or UTP at the time of report generation. You can read more about EOD & OHLC data here. Theta Data plans to avail SIP EOD reports in the near future.
        """
        ...

    def stock_history_eod_async(
        self,
        symbol: str,
        start_date: Union[str, date, datetime],
        end_date: Union[str, date, datetime],
        *,
        timeout_ms: Optional[int] = None,
    ) -> Awaitable[EodTickList]:
        """Fetch end-of-day stock data for a date range. Returns OHLCV + bid/ask per trading day.

        Since the equity SIPs only generate a partial EOD report, Theta Data generates a national EOD report at 17:15 ET each day. ``created`` represents the datetime the report was generated and ``last_trade`` represents the datetime of the last trade. The quote in the response represents the last NBBO reported by CTA or UTP at the time of report generation. You can read more about EOD & OHLC data here. Theta Data plans to avail SIP EOD reports in the near future.


        Awaitable companion of the sync variant. The returned object resolves the request off the calling thread so a running event loop keeps servicing other coroutines.
        """
        ...

    def stock_history_eod_builder(
        self,
        symbol: str,
        start_date: Union[str, date, datetime],
        end_date: Union[str, date, datetime],
    ) -> StockHistoryEodBuilder:
        """Fluent builder for `stock_history_eod`. Chain the optional setters, then call `.list()` (or `.list_async()`) to execute; the returned typed list wrapper exposes `.to_list()` / `.to_arrow()` / `.to_pandas()` / `.to_polars()`."""
        ...

    def stock_history_ohlc(
        self,
        symbol: str,
        date: Union[str, date, datetime],
        *,
        interval: Optional[str] = None,
        start_time: Optional[Union[str, time, datetime]] = None,
        end_time: Optional[Union[str, time, datetime]] = None,
        venue: Optional[str] = None,
        start_date: Optional[Union[str, date, datetime]] = None,
        end_date: Optional[Union[str, date, datetime]] = None,
        timeout_ms: Optional[int] = None,
    ) -> OhlcTickList:
        """Fetch intraday OHLC bars for a stock on a single date.

        - Aggregated OHLC bars that use SIP rules for each bar. Time timestamp of the bar represents the opening time of the bar. For a trade to be part of the bar:  ``bar time`` <= ``trade time`` < ``bar timestamp + ivl``, where ivl is the specified interval size in milliseconds. 
        - Set the ``venue`` parameter to ``nqb`` to access current-day real-time historic data from the Nasdaq Basic feed if the account has a stocks standard or pro subscription.
        - Multi-day requests are limited to 1 month of data.

        Defaults (upstream):
        - `interval`: `"1s"`
        - `start_time`: `"09:30:00"`
        - `end_time`: `"16:00:00"`
        - `venue`: `"nqb"`
        """
        ...

    def stock_history_ohlc_async(
        self,
        symbol: str,
        date: Union[str, date, datetime],
        *,
        interval: Optional[str] = None,
        start_time: Optional[Union[str, time, datetime]] = None,
        end_time: Optional[Union[str, time, datetime]] = None,
        venue: Optional[str] = None,
        start_date: Optional[Union[str, date, datetime]] = None,
        end_date: Optional[Union[str, date, datetime]] = None,
        timeout_ms: Optional[int] = None,
    ) -> Awaitable[OhlcTickList]:
        """Fetch intraday OHLC bars for a stock on a single date.

        - Aggregated OHLC bars that use SIP rules for each bar. Time timestamp of the bar represents the opening time of the bar. For a trade to be part of the bar:  ``bar time`` <= ``trade time`` < ``bar timestamp + ivl``, where ivl is the specified interval size in milliseconds. 
        - Set the ``venue`` parameter to ``nqb`` to access current-day real-time historic data from the Nasdaq Basic feed if the account has a stocks standard or pro subscription.
        - Multi-day requests are limited to 1 month of data.

        Defaults (upstream):
        - `interval`: `"1s"`
        - `start_time`: `"09:30:00"`
        - `end_time`: `"16:00:00"`
        - `venue`: `"nqb"`


        Awaitable companion of the sync variant. The returned object resolves the request off the calling thread so a running event loop keeps servicing other coroutines.
        """
        ...

    def stock_history_ohlc_builder(
        self,
        symbol: str,
        date: Union[str, date, datetime],
    ) -> StockHistoryOhlcBuilder:
        """Fluent builder for `stock_history_ohlc`. Chain the optional setters, then call `.list()` (or `.list_async()`) to execute; the returned typed list wrapper exposes `.to_list()` / `.to_arrow()` / `.to_pandas()` / `.to_polars()`."""
        ...

    def stock_history_trade(
        self,
        symbol: str,
        date: Union[str, date, datetime],
        *,
        start_time: Optional[Union[str, time, datetime]] = None,
        end_time: Optional[Union[str, time, datetime]] = None,
        venue: Optional[str] = None,
        start_date: Optional[Union[str, date, datetime]] = None,
        end_date: Optional[Union[str, date, datetime]] = None,
        timeout_ms: Optional[int] = None,
    ) -> TradeTickList:
        """Fetch all trades for a stock on a given date.

        Returns every trade reported by UTP & CTA. Set the ``venue`` parameter to ``nqb`` to access current-day real-time historic data from the Nasdaq Basic feed if the account has a stocks standard or pro subscription.
        - Multi-day requests are limited to 1 month of data.

        Defaults (upstream):
        - `start_time`: `"09:30:00"`
        - `end_time`: `"16:00:00"`
        - `venue`: `"nqb"`
        """
        ...

    def stock_history_trade_async(
        self,
        symbol: str,
        date: Union[str, date, datetime],
        *,
        start_time: Optional[Union[str, time, datetime]] = None,
        end_time: Optional[Union[str, time, datetime]] = None,
        venue: Optional[str] = None,
        start_date: Optional[Union[str, date, datetime]] = None,
        end_date: Optional[Union[str, date, datetime]] = None,
        timeout_ms: Optional[int] = None,
    ) -> Awaitable[TradeTickList]:
        """Fetch all trades for a stock on a given date.

        Returns every trade reported by UTP & CTA. Set the ``venue`` parameter to ``nqb`` to access current-day real-time historic data from the Nasdaq Basic feed if the account has a stocks standard or pro subscription.
        - Multi-day requests are limited to 1 month of data.

        Defaults (upstream):
        - `start_time`: `"09:30:00"`
        - `end_time`: `"16:00:00"`
        - `venue`: `"nqb"`


        Awaitable companion of the sync variant. The returned object resolves the request off the calling thread so a running event loop keeps servicing other coroutines.
        """
        ...

    def stock_history_trade_builder(
        self,
        symbol: str,
        date: Union[str, date, datetime],
    ) -> StockHistoryTradeBuilder:
        """Fluent builder for `stock_history_trade`. Chain the optional setters, then call `.list()` (or `.list_async()`) to execute; the returned typed list wrapper exposes `.to_list()` / `.to_arrow()` / `.to_pandas()` / `.to_polars()`."""
        ...

    def stock_history_quote(
        self,
        symbol: str,
        date: Union[str, date, datetime],
        *,
        interval: Optional[str] = None,
        start_time: Optional[Union[str, time, datetime]] = None,
        end_time: Optional[Union[str, time, datetime]] = None,
        venue: Optional[str] = None,
        start_date: Optional[Union[str, date, datetime]] = None,
        end_date: Optional[Union[str, date, datetime]] = None,
        timeout_ms: Optional[int] = None,
    ) -> QuoteTickList:
        """Fetch NBBO quotes for a stock on a given date at a given interval.

        - Returns every NBBO quote reported by UTP and CTA. 
        - If the ``interval`` parameter is specified, the quote for each interval represents the last quote prior to the interval's timestamp. 
        - Set the ``venue`` parameter to ``nqb`` to access current-day real-time historic data from the Nasdaq Basic feed if the account has a stocks standard or pro subscription.
        - Multi-day requests are limited to 1 month of data.

        Defaults (upstream):
        - `interval`: `"1s"`
        - `start_time`: `"09:30:00"`
        - `end_time`: `"16:00:00"`
        - `venue`: `"nqb"`
        """
        ...

    def stock_history_quote_async(
        self,
        symbol: str,
        date: Union[str, date, datetime],
        *,
        interval: Optional[str] = None,
        start_time: Optional[Union[str, time, datetime]] = None,
        end_time: Optional[Union[str, time, datetime]] = None,
        venue: Optional[str] = None,
        start_date: Optional[Union[str, date, datetime]] = None,
        end_date: Optional[Union[str, date, datetime]] = None,
        timeout_ms: Optional[int] = None,
    ) -> Awaitable[QuoteTickList]:
        """Fetch NBBO quotes for a stock on a given date at a given interval.

        - Returns every NBBO quote reported by UTP and CTA. 
        - If the ``interval`` parameter is specified, the quote for each interval represents the last quote prior to the interval's timestamp. 
        - Set the ``venue`` parameter to ``nqb`` to access current-day real-time historic data from the Nasdaq Basic feed if the account has a stocks standard or pro subscription.
        - Multi-day requests are limited to 1 month of data.

        Defaults (upstream):
        - `interval`: `"1s"`
        - `start_time`: `"09:30:00"`
        - `end_time`: `"16:00:00"`
        - `venue`: `"nqb"`


        Awaitable companion of the sync variant. The returned object resolves the request off the calling thread so a running event loop keeps servicing other coroutines.
        """
        ...

    def stock_history_quote_builder(
        self,
        symbol: str,
        date: Union[str, date, datetime],
    ) -> StockHistoryQuoteBuilder:
        """Fluent builder for `stock_history_quote`. Chain the optional setters, then call `.list()` (or `.list_async()`) to execute; the returned typed list wrapper exposes `.to_list()` / `.to_arrow()` / `.to_pandas()` / `.to_polars()`."""
        ...

    def stock_history_trade_quote(
        self,
        symbol: str,
        date: Union[str, date, datetime],
        *,
        start_time: Optional[Union[str, time, datetime]] = None,
        end_time: Optional[Union[str, time, datetime]] = None,
        exclusive: Optional[bool] = False,
        venue: Optional[str] = None,
        start_date: Optional[Union[str, date, datetime]] = None,
        end_date: Optional[Union[str, date, datetime]] = None,
        timeout_ms: Optional[int] = None,
    ) -> TradeQuoteTickList:
        """Fetch combined trade + quote ticks for a stock on a given date. Returns raw DataTable.

        Returns every trade reported by UTP & CTA paired with the last BBO quote reported by UTP or CTA at the time of trade. A quote is matched with a trade if its timestamp ``<=`` the trade timestamp. If you prefer to match quotes with timestamps that are ``<`` the trade timestamp, specify the ``exclusive`` parameter to ``true``. Set the ``venue`` parameter to ``nqb`` to access current-day real-time historic data from the Nasdaq Basic feed if the account has a stocks standard or pro subscription.
        - Multi-day requests are limited to 1 month of data.

        Defaults (upstream):
        - `start_time`: `"09:30:00"`
        - `end_time`: `"16:00:00"`
        - `exclusive`: `false`
        - `venue`: `"nqb"`
        """
        ...

    def stock_history_trade_quote_async(
        self,
        symbol: str,
        date: Union[str, date, datetime],
        *,
        start_time: Optional[Union[str, time, datetime]] = None,
        end_time: Optional[Union[str, time, datetime]] = None,
        exclusive: Optional[bool] = None,
        venue: Optional[str] = None,
        start_date: Optional[Union[str, date, datetime]] = None,
        end_date: Optional[Union[str, date, datetime]] = None,
        timeout_ms: Optional[int] = None,
    ) -> Awaitable[TradeQuoteTickList]:
        """Fetch combined trade + quote ticks for a stock on a given date. Returns raw DataTable.

        Returns every trade reported by UTP & CTA paired with the last BBO quote reported by UTP or CTA at the time of trade. A quote is matched with a trade if its timestamp ``<=`` the trade timestamp. If you prefer to match quotes with timestamps that are ``<`` the trade timestamp, specify the ``exclusive`` parameter to ``true``. Set the ``venue`` parameter to ``nqb`` to access current-day real-time historic data from the Nasdaq Basic feed if the account has a stocks standard or pro subscription.
        - Multi-day requests are limited to 1 month of data.

        Defaults (upstream):
        - `start_time`: `"09:30:00"`
        - `end_time`: `"16:00:00"`
        - `exclusive`: `false`
        - `venue`: `"nqb"`


        Awaitable companion of the sync variant. The returned object resolves the request off the calling thread so a running event loop keeps servicing other coroutines.
        """
        ...

    def stock_history_trade_quote_builder(
        self,
        symbol: str,
        date: Union[str, date, datetime],
    ) -> StockHistoryTradeQuoteBuilder:
        """Fluent builder for `stock_history_trade_quote`. Chain the optional setters, then call `.list()` (or `.list_async()`) to execute; the returned typed list wrapper exposes `.to_list()` / `.to_arrow()` / `.to_pandas()` / `.to_polars()`."""
        ...

    def stock_at_time_trade(
        self,
        symbol: str,
        start_date: Union[str, date, datetime],
        end_date: Union[str, date, datetime],
        time_of_day: Union[str, time, datetime],
        *,
        venue: Optional[str] = None,
        timeout_ms: Optional[int] = None,
    ) -> TradeTickList:
        """Fetch the trade at a specific time of day across a date range.

        #### Real-time request:
        - Returns a real-time session from the Nasdaq Basic feed if the account has a stocks standard or pro subscription.
        - Returns a 15-minute delayed session from the UTP & CTA feeds account has the stocks value subscription subscription.

        #### Historical request:
        Returns the last trade reported by UTP & CTA feeds at a specified millisecond of the day.
        Trade condition mappings can be found here.

        Defaults (upstream):
        - `venue`: `"nqb"`
        """
        ...

    def stock_at_time_trade_async(
        self,
        symbol: str,
        start_date: Union[str, date, datetime],
        end_date: Union[str, date, datetime],
        time_of_day: Union[str, time, datetime],
        *,
        venue: Optional[str] = None,
        timeout_ms: Optional[int] = None,
    ) -> Awaitable[TradeTickList]:
        """Fetch the trade at a specific time of day across a date range.

        #### Real-time request:
        - Returns a real-time session from the Nasdaq Basic feed if the account has a stocks standard or pro subscription.
        - Returns a 15-minute delayed session from the UTP & CTA feeds account has the stocks value subscription subscription.

        #### Historical request:
        Returns the last trade reported by UTP & CTA feeds at a specified millisecond of the day.
        Trade condition mappings can be found here.

        Defaults (upstream):
        - `venue`: `"nqb"`


        Awaitable companion of the sync variant. The returned object resolves the request off the calling thread so a running event loop keeps servicing other coroutines.
        """
        ...

    def stock_at_time_trade_builder(
        self,
        symbol: str,
        start_date: Union[str, date, datetime],
        end_date: Union[str, date, datetime],
        time_of_day: Union[str, time, datetime],
    ) -> StockAtTimeTradeBuilder:
        """Fluent builder for `stock_at_time_trade`. Chain the optional setters, then call `.list()` (or `.list_async()`) to execute; the returned typed list wrapper exposes `.to_list()` / `.to_arrow()` / `.to_pandas()` / `.to_polars()`."""
        ...

    def stock_at_time_quote(
        self,
        symbol: str,
        start_date: Union[str, date, datetime],
        end_date: Union[str, date, datetime],
        time_of_day: Union[str, time, datetime],
        *,
        venue: Optional[str] = None,
        timeout_ms: Optional[int] = None,
    ) -> QuoteTickList:
        """Fetch the quote at a specific time of day across a date range.

        #### Real-time request:
          - Subscription tier standard or higher will default to NQB.
          - Real-time last BBO quote at-time_of_day-time from the Nasdaq Basic feed if the account has a stocks standard or pro subscription.
          - 15-minute delayed NBBO quote at-time_of_day-time from the UTP & CTA feeds account has the stocks value subscription subscription.

        #### Historical request:
          Returns the last NBBO quote reported by UTP & CTA feeds at a specified millisecond of the day.

        Defaults (upstream):
        - `venue`: `"nqb"`
        """
        ...

    def stock_at_time_quote_async(
        self,
        symbol: str,
        start_date: Union[str, date, datetime],
        end_date: Union[str, date, datetime],
        time_of_day: Union[str, time, datetime],
        *,
        venue: Optional[str] = None,
        timeout_ms: Optional[int] = None,
    ) -> Awaitable[QuoteTickList]:
        """Fetch the quote at a specific time of day across a date range.

        #### Real-time request:
          - Subscription tier standard or higher will default to NQB.
          - Real-time last BBO quote at-time_of_day-time from the Nasdaq Basic feed if the account has a stocks standard or pro subscription.
          - 15-minute delayed NBBO quote at-time_of_day-time from the UTP & CTA feeds account has the stocks value subscription subscription.

        #### Historical request:
          Returns the last NBBO quote reported by UTP & CTA feeds at a specified millisecond of the day.

        Defaults (upstream):
        - `venue`: `"nqb"`


        Awaitable companion of the sync variant. The returned object resolves the request off the calling thread so a running event loop keeps servicing other coroutines.
        """
        ...

    def stock_at_time_quote_builder(
        self,
        symbol: str,
        start_date: Union[str, date, datetime],
        end_date: Union[str, date, datetime],
        time_of_day: Union[str, time, datetime],
    ) -> StockAtTimeQuoteBuilder:
        """Fluent builder for `stock_at_time_quote`. Chain the optional setters, then call `.list()` (or `.list_async()`) to execute; the returned typed list wrapper exposes `.to_list()` / `.to_arrow()` / `.to_pandas()` / `.to_polars()`."""
        ...

    def option_list_symbols(
        self,
        *,
        timeout_ms: Optional[int] = None,
    ) -> StringList:
        """List all available option underlying symbols.

        A symbol can be defined as a unique identifier for a stock / underlying asset. Common terms also include: root, ticker, and underlying. This endpoint returns all traded symbols for options. This endpoint is updated overnight.
        """
        ...

    def option_list_symbols_async(
        self,
        *,
        timeout_ms: Optional[int] = None,
    ) -> Awaitable[StringList]:
        """List all available option underlying symbols.

        A symbol can be defined as a unique identifier for a stock / underlying asset. Common terms also include: root, ticker, and underlying. This endpoint returns all traded symbols for options. This endpoint is updated overnight.


        Awaitable companion of the sync variant. The returned object resolves the request off the calling thread so a running event loop keeps servicing other coroutines.
        """
        ...

    def option_list_symbols_builder(self) -> OptionListSymbolsBuilder:
        """Fluent builder for `option_list_symbols`. Chain the optional setters, then call `.list()` (or `.list_async()`) to execute; the returned typed list wrapper exposes `.to_list()` / `.to_arrow()` / `.to_pandas()` / `.to_polars()`."""
        ...

    def option_list_dates(
        self,
        request_type: str,
        symbol: str,
        expiration: Union[str, date, datetime],
        *,
        timeout_ms: Optional[int] = None,
    ) -> StringList:
        """List available dates for an option contract by request type.

        Lists all dates of data that are available for an option with a given symbol, request type, and expiration.
        This endpoint is updated overnight.

        Defaults (upstream):
        - `strike`: `"*"`
        - `right`: `"both"`
        """
        ...

    def option_list_dates_async(
        self,
        request_type: str,
        symbol: str,
        expiration: Union[str, date, datetime],
        *,
        timeout_ms: Optional[int] = None,
    ) -> Awaitable[StringList]:
        """List available dates for an option contract by request type.

        Lists all dates of data that are available for an option with a given symbol, request type, and expiration.
        This endpoint is updated overnight.

        Defaults (upstream):
        - `strike`: `"*"`
        - `right`: `"both"`


        Awaitable companion of the sync variant. The returned object resolves the request off the calling thread so a running event loop keeps servicing other coroutines.
        """
        ...

    def option_list_dates_builder(
        self,
        request_type: str,
        symbol: str,
        expiration: Union[str, date, datetime],
    ) -> OptionListDatesBuilder:
        """Fluent builder for `option_list_dates`. Chain the optional setters, then call `.list()` (or `.list_async()`) to execute; the returned typed list wrapper exposes `.to_list()` / `.to_arrow()` / `.to_pandas()` / `.to_polars()`."""
        ...

    def option_list_expirations(
        self,
        symbol: str,
        *,
        timeout_ms: Optional[int] = None,
    ) -> StringList:
        """List available expiration dates for an option underlying.

        Lists all dates of expirations that are available for an option with a given symbol.
        This endpoint is updated overnight.
        """
        ...

    def option_list_expirations_async(
        self,
        symbol: str,
        *,
        timeout_ms: Optional[int] = None,
    ) -> Awaitable[StringList]:
        """List available expiration dates for an option underlying.

        Lists all dates of expirations that are available for an option with a given symbol.
        This endpoint is updated overnight.


        Awaitable companion of the sync variant. The returned object resolves the request off the calling thread so a running event loop keeps servicing other coroutines.
        """
        ...

    def option_list_expirations_builder(
        self,
        symbol: str,
    ) -> OptionListExpirationsBuilder:
        """Fluent builder for `option_list_expirations`. Chain the optional setters, then call `.list()` (or `.list_async()`) to execute; the returned typed list wrapper exposes `.to_list()` / `.to_arrow()` / `.to_pandas()` / `.to_polars()`."""
        ...

    def option_list_strikes(
        self,
        symbol: str,
        expiration: Union[str, date, datetime],
        *,
        timeout_ms: Optional[int] = None,
    ) -> StringList:
        """List available strike prices for an option at a given expiration.

        Lists all strikes that are available for an option with a given symbol and expiration date.
        This endpoint is updated overnight.
        """
        ...

    def option_list_strikes_async(
        self,
        symbol: str,
        expiration: Union[str, date, datetime],
        *,
        timeout_ms: Optional[int] = None,
    ) -> Awaitable[StringList]:
        """List available strike prices for an option at a given expiration.

        Lists all strikes that are available for an option with a given symbol and expiration date.
        This endpoint is updated overnight.


        Awaitable companion of the sync variant. The returned object resolves the request off the calling thread so a running event loop keeps servicing other coroutines.
        """
        ...

    def option_list_strikes_builder(
        self,
        symbol: str,
        expiration: Union[str, date, datetime],
    ) -> OptionListStrikesBuilder:
        """Fluent builder for `option_list_strikes`. Chain the optional setters, then call `.list()` (or `.list_async()`) to execute; the returned typed list wrapper exposes `.to_list()` / `.to_arrow()` / `.to_pandas()` / `.to_polars()`."""
        ...

    def option_list_contracts(
        self,
        request_type: str,
        symbol: str,
        date: Union[str, date, datetime],
        *,
        max_dte: Optional[int] = None,
        timeout_ms: Optional[int] = None,
    ) -> OptionContractList:
        """List all option contracts for a symbol on a given date.

        Lists all contracts that were traded or quoted on a particular date.

        If the ``symbol`` parameter is specified, the returned contracts will be filtered to match the symbol.
        Multiple symbols can be specified by separating them with commas such as ``symbol=AAPL,SPY,AMD``
        This endpoint is updated real-time.
        """
        ...

    def option_list_contracts_async(
        self,
        request_type: str,
        symbol: str,
        date: Union[str, date, datetime],
        *,
        max_dte: Optional[int] = None,
        timeout_ms: Optional[int] = None,
    ) -> Awaitable[OptionContractList]:
        """List all option contracts for a symbol on a given date.

        Lists all contracts that were traded or quoted on a particular date.

        If the ``symbol`` parameter is specified, the returned contracts will be filtered to match the symbol.
        Multiple symbols can be specified by separating them with commas such as ``symbol=AAPL,SPY,AMD``
        This endpoint is updated real-time.


        Awaitable companion of the sync variant. The returned object resolves the request off the calling thread so a running event loop keeps servicing other coroutines.
        """
        ...

    def option_list_contracts_builder(
        self,
        request_type: str,
        symbol: str,
        date: Union[str, date, datetime],
    ) -> OptionListContractsBuilder:
        """Fluent builder for `option_list_contracts`. Chain the optional setters, then call `.list()` (or `.list_async()`) to execute; the returned typed list wrapper exposes `.to_list()` / `.to_arrow()` / `.to_pandas()` / `.to_polars()`."""
        ...

    def option_snapshot_ohlc(
        self,
        symbol: str,
        expiration: Union[str, date, datetime],
        *,
        strike: Optional[str] = None,
        right: Optional[str] = None,
        max_dte: Optional[int] = None,
        strike_range: Optional[int] = None,
        min_time: Optional[Union[str, time, datetime]] = None,
        timeout_ms: Optional[int] = None,
    ) -> List[OhlcTick]:
        """Get the latest OHLC snapshot for an option contract.

        - Retrieve a real-time last ohlc of an option contract for the trading day.

        Defaults (upstream):
        - `strike`: `"*"`
        - `right`: `"both"`
        """
        ...

    def option_snapshot_ohlc_async(
        self,
        symbol: str,
        expiration: Union[str, date, datetime],
        *,
        strike: Optional[str] = None,
        right: Optional[str] = None,
        max_dte: Optional[int] = None,
        strike_range: Optional[int] = None,
        min_time: Optional[Union[str, time, datetime]] = None,
        timeout_ms: Optional[int] = None,
    ) -> Awaitable[List[OhlcTick]]:
        """Get the latest OHLC snapshot for an option contract.

        - Retrieve a real-time last ohlc of an option contract for the trading day.

        Defaults (upstream):
        - `strike`: `"*"`
        - `right`: `"both"`


        Awaitable companion of the sync variant. The returned object resolves the request off the calling thread so a running event loop keeps servicing other coroutines.
        """
        ...

    def option_snapshot_ohlc_builder(
        self,
        symbol: str,
        expiration: Union[str, date, datetime],
    ) -> OptionSnapshotOhlcBuilder:
        """Fluent builder for `option_snapshot_ohlc`. Chain the optional setters, then call `.list()` (or `.list_async()`) to execute; the returned typed list wrapper exposes `.to_list()` / `.to_arrow()` / `.to_pandas()` / `.to_polars()`."""
        ...

    def option_snapshot_trade(
        self,
        symbol: str,
        expiration: Union[str, date, datetime],
        *,
        strike: Optional[str] = None,
        right: Optional[str] = None,
        strike_range: Optional[int] = None,
        min_time: Optional[Union[str, time, datetime]] = None,
        timeout_ms: Optional[int] = None,
    ) -> List[TradeTick]:
        """Get the latest trade snapshot for an option contract.

        - Retrieve the real-time last trade of an option contract.
        - This endpoint will return no data if the market was closed for the day. Theta Data resets the snapshot cache at midnight ET every night.

        Defaults (upstream):
        - `strike`: `"*"`
        - `right`: `"both"`
        """
        ...

    def option_snapshot_trade_async(
        self,
        symbol: str,
        expiration: Union[str, date, datetime],
        *,
        strike: Optional[str] = None,
        right: Optional[str] = None,
        strike_range: Optional[int] = None,
        min_time: Optional[Union[str, time, datetime]] = None,
        timeout_ms: Optional[int] = None,
    ) -> Awaitable[List[TradeTick]]:
        """Get the latest trade snapshot for an option contract.

        - Retrieve the real-time last trade of an option contract.
        - This endpoint will return no data if the market was closed for the day. Theta Data resets the snapshot cache at midnight ET every night.

        Defaults (upstream):
        - `strike`: `"*"`
        - `right`: `"both"`


        Awaitable companion of the sync variant. The returned object resolves the request off the calling thread so a running event loop keeps servicing other coroutines.
        """
        ...

    def option_snapshot_trade_builder(
        self,
        symbol: str,
        expiration: Union[str, date, datetime],
    ) -> OptionSnapshotTradeBuilder:
        """Fluent builder for `option_snapshot_trade`. Chain the optional setters, then call `.list()` (or `.list_async()`) to execute; the returned typed list wrapper exposes `.to_list()` / `.to_arrow()` / `.to_pandas()` / `.to_polars()`."""
        ...

    def option_snapshot_quote(
        self,
        symbol: str,
        expiration: Union[str, date, datetime],
        *,
        strike: Optional[str] = None,
        right: Optional[str] = None,
        max_dte: Optional[int] = None,
        strike_range: Optional[int] = None,
        min_time: Optional[Union[str, time, datetime]] = None,
        timeout_ms: Optional[int] = None,
    ) -> List[QuoteTick]:
        """Get the latest NBBO quote snapshot for an option contract.

        - Retrieve a real-time last NBBO quote of an option contract.
        - This endpoint will return no data if the market was closed for the day. Theta Data resets the snapshot cache at midnight ET every night.

        Defaults (upstream):
        - `strike`: `"*"`
        - `right`: `"both"`
        """
        ...

    def option_snapshot_quote_async(
        self,
        symbol: str,
        expiration: Union[str, date, datetime],
        *,
        strike: Optional[str] = None,
        right: Optional[str] = None,
        max_dte: Optional[int] = None,
        strike_range: Optional[int] = None,
        min_time: Optional[Union[str, time, datetime]] = None,
        timeout_ms: Optional[int] = None,
    ) -> Awaitable[List[QuoteTick]]:
        """Get the latest NBBO quote snapshot for an option contract.

        - Retrieve a real-time last NBBO quote of an option contract.
        - This endpoint will return no data if the market was closed for the day. Theta Data resets the snapshot cache at midnight ET every night.

        Defaults (upstream):
        - `strike`: `"*"`
        - `right`: `"both"`


        Awaitable companion of the sync variant. The returned object resolves the request off the calling thread so a running event loop keeps servicing other coroutines.
        """
        ...

    def option_snapshot_quote_builder(
        self,
        symbol: str,
        expiration: Union[str, date, datetime],
    ) -> OptionSnapshotQuoteBuilder:
        """Fluent builder for `option_snapshot_quote`. Chain the optional setters, then call `.list()` (or `.list_async()`) to execute; the returned typed list wrapper exposes `.to_list()` / `.to_arrow()` / `.to_pandas()` / `.to_polars()`."""
        ...

    def option_snapshot_open_interest(
        self,
        symbol: str,
        expiration: Union[str, date, datetime],
        *,
        strike: Optional[str] = None,
        right: Optional[str] = None,
        max_dte: Optional[int] = None,
        strike_range: Optional[int] = None,
        min_time: Optional[Union[str, time, datetime]] = None,
        timeout_ms: Optional[int] = None,
    ) -> List[OpenInterestTick]:
        """Get the latest open interest snapshot for an option contract.

        - Retrieve the last open interest message of an option contract.
        - Open interest is reported around 06:30 ET every morning by OPRA and reflects the open interest at the end of the previous trading day.
        - This endpoint will return no data if the market was closed for the day. Theta Data resets the snapshot cache at midnight ET every night.

        Defaults (upstream):
        - `strike`: `"*"`
        - `right`: `"both"`
        """
        ...

    def option_snapshot_open_interest_async(
        self,
        symbol: str,
        expiration: Union[str, date, datetime],
        *,
        strike: Optional[str] = None,
        right: Optional[str] = None,
        max_dte: Optional[int] = None,
        strike_range: Optional[int] = None,
        min_time: Optional[Union[str, time, datetime]] = None,
        timeout_ms: Optional[int] = None,
    ) -> Awaitable[List[OpenInterestTick]]:
        """Get the latest open interest snapshot for an option contract.

        - Retrieve the last open interest message of an option contract.
        - Open interest is reported around 06:30 ET every morning by OPRA and reflects the open interest at the end of the previous trading day.
        - This endpoint will return no data if the market was closed for the day. Theta Data resets the snapshot cache at midnight ET every night.

        Defaults (upstream):
        - `strike`: `"*"`
        - `right`: `"both"`


        Awaitable companion of the sync variant. The returned object resolves the request off the calling thread so a running event loop keeps servicing other coroutines.
        """
        ...

    def option_snapshot_open_interest_builder(
        self,
        symbol: str,
        expiration: Union[str, date, datetime],
    ) -> OptionSnapshotOpenInterestBuilder:
        """Fluent builder for `option_snapshot_open_interest`. Chain the optional setters, then call `.list()` (or `.list_async()`) to execute; the returned typed list wrapper exposes `.to_list()` / `.to_arrow()` / `.to_pandas()` / `.to_polars()`."""
        ...

    def option_snapshot_market_value(
        self,
        symbol: str,
        expiration: Union[str, date, datetime],
        *,
        strike: Optional[str] = None,
        right: Optional[str] = None,
        max_dte: Optional[int] = None,
        strike_range: Optional[int] = None,
        min_time: Optional[Union[str, time, datetime]] = None,
        timeout_ms: Optional[int] = None,
    ) -> List[MarketValueTick]:
        """Get the latest market value snapshot for an option contract.

        * Returns a real-time market value derived from the last NBBO quote of an option contract.

        Defaults (upstream):
        - `strike`: `"*"`
        - `right`: `"both"`
        """
        ...

    def option_snapshot_market_value_async(
        self,
        symbol: str,
        expiration: Union[str, date, datetime],
        *,
        strike: Optional[str] = None,
        right: Optional[str] = None,
        max_dte: Optional[int] = None,
        strike_range: Optional[int] = None,
        min_time: Optional[Union[str, time, datetime]] = None,
        timeout_ms: Optional[int] = None,
    ) -> Awaitable[List[MarketValueTick]]:
        """Get the latest market value snapshot for an option contract.

        * Returns a real-time market value derived from the last NBBO quote of an option contract.

        Defaults (upstream):
        - `strike`: `"*"`
        - `right`: `"both"`


        Awaitable companion of the sync variant. The returned object resolves the request off the calling thread so a running event loop keeps servicing other coroutines.
        """
        ...

    def option_snapshot_market_value_builder(
        self,
        symbol: str,
        expiration: Union[str, date, datetime],
    ) -> OptionSnapshotMarketValueBuilder:
        """Fluent builder for `option_snapshot_market_value`. Chain the optional setters, then call `.list()` (or `.list_async()`) to execute; the returned typed list wrapper exposes `.to_list()` / `.to_arrow()` / `.to_pandas()` / `.to_polars()`."""
        ...

    def option_snapshot_greeks_implied_volatility(
        self,
        symbol: str,
        expiration: Union[str, date, datetime],
        *,
        strike: Optional[str] = None,
        right: Optional[str] = None,
        annual_dividend: Optional[float] = None,
        rate_type: Optional[str] = None,
        rate_value: Optional[float] = None,
        stock_price: Optional[float] = None,
        version: Optional[str] = None,
        max_dte: Optional[int] = None,
        strike_range: Optional[int] = None,
        min_time: Optional[Union[str, time, datetime]] = None,
        use_market_value: Optional[bool] = False,
        timeout_ms: Optional[int] = None,
    ) -> List[IvTick]:
        """Get implied volatility snapshot for an option contract (from ThetaData server).

        Returns implied volatilies calculated using the national best bid, mid, and ask price
        of the option respectively. The underlying price represents whatever the last underlying price was at the
        ``underlying_timestamp`` field. You can read more about how Theta Data calculates greeks 
        here.

        Defaults (upstream):
        - `strike`: `"*"`
        - `right`: `"both"`
        - `rate_type`: `"sofr"`
        - `version`: `"latest"`
        - `use_market_value`: `false`
        """
        ...

    def option_snapshot_greeks_implied_volatility_async(
        self,
        symbol: str,
        expiration: Union[str, date, datetime],
        *,
        strike: Optional[str] = None,
        right: Optional[str] = None,
        annual_dividend: Optional[float] = None,
        rate_type: Optional[str] = None,
        rate_value: Optional[float] = None,
        stock_price: Optional[float] = None,
        version: Optional[str] = None,
        max_dte: Optional[int] = None,
        strike_range: Optional[int] = None,
        min_time: Optional[Union[str, time, datetime]] = None,
        use_market_value: Optional[bool] = None,
        timeout_ms: Optional[int] = None,
    ) -> Awaitable[List[IvTick]]:
        """Get implied volatility snapshot for an option contract (from ThetaData server).

        Returns implied volatilies calculated using the national best bid, mid, and ask price
        of the option respectively. The underlying price represents whatever the last underlying price was at the
        ``underlying_timestamp`` field. You can read more about how Theta Data calculates greeks 
        here.

        Defaults (upstream):
        - `strike`: `"*"`
        - `right`: `"both"`
        - `rate_type`: `"sofr"`
        - `version`: `"latest"`
        - `use_market_value`: `false`


        Awaitable companion of the sync variant. The returned object resolves the request off the calling thread so a running event loop keeps servicing other coroutines.
        """
        ...

    def option_snapshot_greeks_implied_volatility_builder(
        self,
        symbol: str,
        expiration: Union[str, date, datetime],
    ) -> OptionSnapshotGreeksImpliedVolatilityBuilder:
        """Fluent builder for `option_snapshot_greeks_implied_volatility`. Chain the optional setters, then call `.list()` (or `.list_async()`) to execute; the returned typed list wrapper exposes `.to_list()` / `.to_arrow()` / `.to_pandas()` / `.to_polars()`."""
        ...

    def option_snapshot_greeks_all(
        self,
        symbol: str,
        expiration: Union[str, date, datetime],
        *,
        strike: Optional[str] = None,
        right: Optional[str] = None,
        annual_dividend: Optional[float] = None,
        rate_type: Optional[str] = None,
        rate_value: Optional[float] = None,
        stock_price: Optional[float] = None,
        version: Optional[str] = None,
        max_dte: Optional[int] = None,
        strike_range: Optional[int] = None,
        min_time: Optional[Union[str, time, datetime]] = None,
        use_market_value: Optional[bool] = False,
        timeout_ms: Optional[int] = None,
    ) -> List[GreeksAllTick]:
        """Get all Greeks snapshot for an option contract (from ThetaData server).

        - Retrieve a real-time last greeks calculation for all option contracts that lie on a provided expiration.
        - Set `expiration` to `*` to snapshot every expiration for the underlying in a single request.
        > This endpoint will return no data if the market was closed for the day. Theta Data resets the snapshot cache at midnight ET every night.

        Defaults (upstream):
        - `strike`: `"*"`
        - `right`: `"both"`
        - `rate_type`: `"sofr"`
        - `version`: `"latest"`
        - `use_market_value`: `false`
        """
        ...

    def option_snapshot_greeks_all_async(
        self,
        symbol: str,
        expiration: Union[str, date, datetime],
        *,
        strike: Optional[str] = None,
        right: Optional[str] = None,
        annual_dividend: Optional[float] = None,
        rate_type: Optional[str] = None,
        rate_value: Optional[float] = None,
        stock_price: Optional[float] = None,
        version: Optional[str] = None,
        max_dte: Optional[int] = None,
        strike_range: Optional[int] = None,
        min_time: Optional[Union[str, time, datetime]] = None,
        use_market_value: Optional[bool] = None,
        timeout_ms: Optional[int] = None,
    ) -> Awaitable[List[GreeksAllTick]]:
        """Get all Greeks snapshot for an option contract (from ThetaData server).

        - Retrieve a real-time last greeks calculation for all option contracts that lie on a provided expiration.
        - Set `expiration` to `*` to snapshot every expiration for the underlying in a single request.
        > This endpoint will return no data if the market was closed for the day. Theta Data resets the snapshot cache at midnight ET every night.

        Defaults (upstream):
        - `strike`: `"*"`
        - `right`: `"both"`
        - `rate_type`: `"sofr"`
        - `version`: `"latest"`
        - `use_market_value`: `false`


        Awaitable companion of the sync variant. The returned object resolves the request off the calling thread so a running event loop keeps servicing other coroutines.
        """
        ...

    def option_snapshot_greeks_all_builder(
        self,
        symbol: str,
        expiration: Union[str, date, datetime],
    ) -> OptionSnapshotGreeksAllBuilder:
        """Fluent builder for `option_snapshot_greeks_all`. Chain the optional setters, then call `.list()` (or `.list_async()`) to execute; the returned typed list wrapper exposes `.to_list()` / `.to_arrow()` / `.to_pandas()` / `.to_polars()`."""
        ...

    def option_snapshot_greeks_first_order(
        self,
        symbol: str,
        expiration: Union[str, date, datetime],
        *,
        strike: Optional[str] = None,
        right: Optional[str] = None,
        annual_dividend: Optional[float] = None,
        rate_type: Optional[str] = None,
        rate_value: Optional[float] = None,
        stock_price: Optional[float] = None,
        version: Optional[str] = None,
        max_dte: Optional[int] = None,
        strike_range: Optional[int] = None,
        min_time: Optional[Union[str, time, datetime]] = None,
        use_market_value: Optional[bool] = False,
        timeout_ms: Optional[int] = None,
    ) -> List[GreeksFirstOrderTick]:
        """Get first-order Greeks snapshot (delta, theta, rho) for an option contract.

        - Retrieve a real-time last greeks calculation for all option contracts that lie on a provided expiration.
        - Set `expiration` to `*` to snapshot every expiration for the underlying in a single request.
        > This endpoint will return no data if the market was closed for the day. Theta Data resets the snapshot cache at midnight ET every night.

        Defaults (upstream):
        - `strike`: `"*"`
        - `right`: `"both"`
        - `rate_type`: `"sofr"`
        - `version`: `"latest"`
        - `use_market_value`: `false`
        """
        ...

    def option_snapshot_greeks_first_order_async(
        self,
        symbol: str,
        expiration: Union[str, date, datetime],
        *,
        strike: Optional[str] = None,
        right: Optional[str] = None,
        annual_dividend: Optional[float] = None,
        rate_type: Optional[str] = None,
        rate_value: Optional[float] = None,
        stock_price: Optional[float] = None,
        version: Optional[str] = None,
        max_dte: Optional[int] = None,
        strike_range: Optional[int] = None,
        min_time: Optional[Union[str, time, datetime]] = None,
        use_market_value: Optional[bool] = None,
        timeout_ms: Optional[int] = None,
    ) -> Awaitable[List[GreeksFirstOrderTick]]:
        """Get first-order Greeks snapshot (delta, theta, rho) for an option contract.

        - Retrieve a real-time last greeks calculation for all option contracts that lie on a provided expiration.
        - Set `expiration` to `*` to snapshot every expiration for the underlying in a single request.
        > This endpoint will return no data if the market was closed for the day. Theta Data resets the snapshot cache at midnight ET every night.

        Defaults (upstream):
        - `strike`: `"*"`
        - `right`: `"both"`
        - `rate_type`: `"sofr"`
        - `version`: `"latest"`
        - `use_market_value`: `false`


        Awaitable companion of the sync variant. The returned object resolves the request off the calling thread so a running event loop keeps servicing other coroutines.
        """
        ...

    def option_snapshot_greeks_first_order_builder(
        self,
        symbol: str,
        expiration: Union[str, date, datetime],
    ) -> OptionSnapshotGreeksFirstOrderBuilder:
        """Fluent builder for `option_snapshot_greeks_first_order`. Chain the optional setters, then call `.list()` (or `.list_async()`) to execute; the returned typed list wrapper exposes `.to_list()` / `.to_arrow()` / `.to_pandas()` / `.to_polars()`."""
        ...

    def option_snapshot_greeks_second_order(
        self,
        symbol: str,
        expiration: Union[str, date, datetime],
        *,
        strike: Optional[str] = None,
        right: Optional[str] = None,
        annual_dividend: Optional[float] = None,
        rate_type: Optional[str] = None,
        rate_value: Optional[float] = None,
        stock_price: Optional[float] = None,
        version: Optional[str] = None,
        max_dte: Optional[int] = None,
        strike_range: Optional[int] = None,
        min_time: Optional[Union[str, time, datetime]] = None,
        use_market_value: Optional[bool] = False,
        timeout_ms: Optional[int] = None,
    ) -> List[GreeksSecondOrderTick]:
        """Get second-order Greeks snapshot (gamma, vanna, charm) for an option contract.

        - Retrieve a real-time last second order greeks calculation for all option contracts that lie on a provided expiration.
        - Set `expiration` to `*` to snapshot every expiration for the underlying in a single request.
        > This endpoint will return no data if the market was closed for the day. Theta Data resets the snapshot cache at midnight ET every night.

        Defaults (upstream):
        - `strike`: `"*"`
        - `right`: `"both"`
        - `rate_type`: `"sofr"`
        - `version`: `"latest"`
        - `use_market_value`: `false`
        """
        ...

    def option_snapshot_greeks_second_order_async(
        self,
        symbol: str,
        expiration: Union[str, date, datetime],
        *,
        strike: Optional[str] = None,
        right: Optional[str] = None,
        annual_dividend: Optional[float] = None,
        rate_type: Optional[str] = None,
        rate_value: Optional[float] = None,
        stock_price: Optional[float] = None,
        version: Optional[str] = None,
        max_dte: Optional[int] = None,
        strike_range: Optional[int] = None,
        min_time: Optional[Union[str, time, datetime]] = None,
        use_market_value: Optional[bool] = None,
        timeout_ms: Optional[int] = None,
    ) -> Awaitable[List[GreeksSecondOrderTick]]:
        """Get second-order Greeks snapshot (gamma, vanna, charm) for an option contract.

        - Retrieve a real-time last second order greeks calculation for all option contracts that lie on a provided expiration.
        - Set `expiration` to `*` to snapshot every expiration for the underlying in a single request.
        > This endpoint will return no data if the market was closed for the day. Theta Data resets the snapshot cache at midnight ET every night.

        Defaults (upstream):
        - `strike`: `"*"`
        - `right`: `"both"`
        - `rate_type`: `"sofr"`
        - `version`: `"latest"`
        - `use_market_value`: `false`


        Awaitable companion of the sync variant. The returned object resolves the request off the calling thread so a running event loop keeps servicing other coroutines.
        """
        ...

    def option_snapshot_greeks_second_order_builder(
        self,
        symbol: str,
        expiration: Union[str, date, datetime],
    ) -> OptionSnapshotGreeksSecondOrderBuilder:
        """Fluent builder for `option_snapshot_greeks_second_order`. Chain the optional setters, then call `.list()` (or `.list_async()`) to execute; the returned typed list wrapper exposes `.to_list()` / `.to_arrow()` / `.to_pandas()` / `.to_polars()`."""
        ...

    def option_snapshot_greeks_third_order(
        self,
        symbol: str,
        expiration: Union[str, date, datetime],
        *,
        strike: Optional[str] = None,
        right: Optional[str] = None,
        annual_dividend: Optional[float] = None,
        rate_type: Optional[str] = None,
        rate_value: Optional[float] = None,
        stock_price: Optional[float] = None,
        version: Optional[str] = None,
        max_dte: Optional[int] = None,
        strike_range: Optional[int] = None,
        min_time: Optional[Union[str, time, datetime]] = None,
        use_market_value: Optional[bool] = False,
        timeout_ms: Optional[int] = None,
    ) -> List[GreeksThirdOrderTick]:
        """Get third-order Greeks snapshot (speed, color, ultima) for an option contract.

        - Retrieve a real-time last third order greeks calculation for all option contracts that lie on a provided expiration.
        - Set `expiration` to `*` to snapshot every expiration for the underlying in a single request.
        > This endpoint will return no data if the market was closed for the day. Theta Data resets the snapshot cache at midnight ET every night.

        Defaults (upstream):
        - `strike`: `"*"`
        - `right`: `"both"`
        - `rate_type`: `"sofr"`
        - `version`: `"latest"`
        - `use_market_value`: `false`
        """
        ...

    def option_snapshot_greeks_third_order_async(
        self,
        symbol: str,
        expiration: Union[str, date, datetime],
        *,
        strike: Optional[str] = None,
        right: Optional[str] = None,
        annual_dividend: Optional[float] = None,
        rate_type: Optional[str] = None,
        rate_value: Optional[float] = None,
        stock_price: Optional[float] = None,
        version: Optional[str] = None,
        max_dte: Optional[int] = None,
        strike_range: Optional[int] = None,
        min_time: Optional[Union[str, time, datetime]] = None,
        use_market_value: Optional[bool] = None,
        timeout_ms: Optional[int] = None,
    ) -> Awaitable[List[GreeksThirdOrderTick]]:
        """Get third-order Greeks snapshot (speed, color, ultima) for an option contract.

        - Retrieve a real-time last third order greeks calculation for all option contracts that lie on a provided expiration.
        - Set `expiration` to `*` to snapshot every expiration for the underlying in a single request.
        > This endpoint will return no data if the market was closed for the day. Theta Data resets the snapshot cache at midnight ET every night.

        Defaults (upstream):
        - `strike`: `"*"`
        - `right`: `"both"`
        - `rate_type`: `"sofr"`
        - `version`: `"latest"`
        - `use_market_value`: `false`


        Awaitable companion of the sync variant. The returned object resolves the request off the calling thread so a running event loop keeps servicing other coroutines.
        """
        ...

    def option_snapshot_greeks_third_order_builder(
        self,
        symbol: str,
        expiration: Union[str, date, datetime],
    ) -> OptionSnapshotGreeksThirdOrderBuilder:
        """Fluent builder for `option_snapshot_greeks_third_order`. Chain the optional setters, then call `.list()` (or `.list_async()`) to execute; the returned typed list wrapper exposes `.to_list()` / `.to_arrow()` / `.to_pandas()` / `.to_polars()`."""
        ...

    def option_history_eod(
        self,
        symbol: str,
        expiration: Union[str, date, datetime],
        start_date: Union[str, date, datetime],
        end_date: Union[str, date, datetime],
        *,
        strike: Optional[str] = None,
        right: Optional[str] = None,
        max_dte: Optional[int] = None,
        strike_range: Optional[int] = None,
        timeout_ms: Optional[int] = None,
    ) -> EodTickList:
        """Fetch end-of-day option data for a contract over a date range.

        - Since OPRA does not provide a national EOD report for options, Theta Data generates a national EOD report at 17:15 ET each day.
        - ``created`` represents the datetime the report was generated and ``last_trade`` represents the datetime of the last trade. 
        - The quote in the response represents the last NBBO reported by OPRA at the time of report generation. 
        - You can read more about EOD & OHLC data here.

        Defaults (upstream):
        - `strike`: `"*"`
        - `right`: `"both"`
        """
        ...

    def option_history_eod_async(
        self,
        symbol: str,
        expiration: Union[str, date, datetime],
        start_date: Union[str, date, datetime],
        end_date: Union[str, date, datetime],
        *,
        strike: Optional[str] = None,
        right: Optional[str] = None,
        max_dte: Optional[int] = None,
        strike_range: Optional[int] = None,
        timeout_ms: Optional[int] = None,
    ) -> Awaitable[EodTickList]:
        """Fetch end-of-day option data for a contract over a date range.

        - Since OPRA does not provide a national EOD report for options, Theta Data generates a national EOD report at 17:15 ET each day.
        - ``created`` represents the datetime the report was generated and ``last_trade`` represents the datetime of the last trade. 
        - The quote in the response represents the last NBBO reported by OPRA at the time of report generation. 
        - You can read more about EOD & OHLC data here.

        Defaults (upstream):
        - `strike`: `"*"`
        - `right`: `"both"`


        Awaitable companion of the sync variant. The returned object resolves the request off the calling thread so a running event loop keeps servicing other coroutines.
        """
        ...

    def option_history_eod_builder(
        self,
        symbol: str,
        expiration: Union[str, date, datetime],
        start_date: Union[str, date, datetime],
        end_date: Union[str, date, datetime],
    ) -> OptionHistoryEodBuilder:
        """Fluent builder for `option_history_eod`. Chain the optional setters, then call `.list()` (or `.list_async()`) to execute; the returned typed list wrapper exposes `.to_list()` / `.to_arrow()` / `.to_pandas()` / `.to_polars()`."""
        ...

    def option_history_ohlc(
        self,
        symbol: str,
        expiration: Union[str, date, datetime],
        date: Union[str, date, datetime],
        *,
        strike: Optional[str] = None,
        right: Optional[str] = None,
        interval: Optional[str] = None,
        start_time: Optional[Union[str, time, datetime]] = None,
        end_time: Optional[Union[str, time, datetime]] = None,
        strike_range: Optional[int] = None,
        start_date: Optional[Union[str, date, datetime]] = None,
        end_date: Optional[Union[str, date, datetime]] = None,
        timeout_ms: Optional[int] = None,
    ) -> OhlcTickList:
        """Fetch intraday OHLC bars for an option contract.

        - Aggregated OHLC bars that use SIP rules for each bar. 
        - Time timestamp of the bar represents the opening time of the bar. For a trade to be part of the bar:  ``bar timestamp`` <= ``trade time`` < ``bar timestamp + interval``.
        - Multi-day requests are limited to 1 month of data.

        Defaults (upstream):
        - `strike`: `"*"`
        - `right`: `"both"`
        - `interval`: `"1s"`
        - `start_time`: `"09:30:00"`
        - `end_time`: `"16:00:00"`
        """
        ...

    def option_history_ohlc_async(
        self,
        symbol: str,
        expiration: Union[str, date, datetime],
        date: Union[str, date, datetime],
        *,
        strike: Optional[str] = None,
        right: Optional[str] = None,
        interval: Optional[str] = None,
        start_time: Optional[Union[str, time, datetime]] = None,
        end_time: Optional[Union[str, time, datetime]] = None,
        strike_range: Optional[int] = None,
        start_date: Optional[Union[str, date, datetime]] = None,
        end_date: Optional[Union[str, date, datetime]] = None,
        timeout_ms: Optional[int] = None,
    ) -> Awaitable[OhlcTickList]:
        """Fetch intraday OHLC bars for an option contract.

        - Aggregated OHLC bars that use SIP rules for each bar. 
        - Time timestamp of the bar represents the opening time of the bar. For a trade to be part of the bar:  ``bar timestamp`` <= ``trade time`` < ``bar timestamp + interval``.
        - Multi-day requests are limited to 1 month of data.

        Defaults (upstream):
        - `strike`: `"*"`
        - `right`: `"both"`
        - `interval`: `"1s"`
        - `start_time`: `"09:30:00"`
        - `end_time`: `"16:00:00"`


        Awaitable companion of the sync variant. The returned object resolves the request off the calling thread so a running event loop keeps servicing other coroutines.
        """
        ...

    def option_history_ohlc_builder(
        self,
        symbol: str,
        expiration: Union[str, date, datetime],
        date: Union[str, date, datetime],
    ) -> OptionHistoryOhlcBuilder:
        """Fluent builder for `option_history_ohlc`. Chain the optional setters, then call `.list()` (or `.list_async()`) to execute; the returned typed list wrapper exposes `.to_list()` / `.to_arrow()` / `.to_pandas()` / `.to_polars()`."""
        ...

    def option_history_trade(
        self,
        symbol: str,
        expiration: Union[str, date, datetime],
        date: Union[str, date, datetime],
        *,
        strike: Optional[str] = None,
        right: Optional[str] = None,
        start_time: Optional[Union[str, time, datetime]] = None,
        end_time: Optional[Union[str, time, datetime]] = None,
        max_dte: Optional[int] = None,
        strike_range: Optional[int] = None,
        start_date: Optional[Union[str, date, datetime]] = None,
        end_date: Optional[Union[str, date, datetime]] = None,
        timeout_ms: Optional[int] = None,
    ) -> TradeTickList:
        """Fetch all trades for an option contract on a given date.

        - Returns every trade reported by OPRA. 
        - Trade condition mappings can be found here.
        - Extended trade conditions are not reported by OPRA for options, so they can be ignored.
        - Multi-day requests are limited to 1 month of data, and must specify an expiration.

        Defaults (upstream):
        - `strike`: `"*"`
        - `right`: `"both"`
        - `start_time`: `"09:30:00"`
        - `end_time`: `"16:00:00"`
        """
        ...

    def option_history_trade_async(
        self,
        symbol: str,
        expiration: Union[str, date, datetime],
        date: Union[str, date, datetime],
        *,
        strike: Optional[str] = None,
        right: Optional[str] = None,
        start_time: Optional[Union[str, time, datetime]] = None,
        end_time: Optional[Union[str, time, datetime]] = None,
        max_dte: Optional[int] = None,
        strike_range: Optional[int] = None,
        start_date: Optional[Union[str, date, datetime]] = None,
        end_date: Optional[Union[str, date, datetime]] = None,
        timeout_ms: Optional[int] = None,
    ) -> Awaitable[TradeTickList]:
        """Fetch all trades for an option contract on a given date.

        - Returns every trade reported by OPRA. 
        - Trade condition mappings can be found here.
        - Extended trade conditions are not reported by OPRA for options, so they can be ignored.
        - Multi-day requests are limited to 1 month of data, and must specify an expiration.

        Defaults (upstream):
        - `strike`: `"*"`
        - `right`: `"both"`
        - `start_time`: `"09:30:00"`
        - `end_time`: `"16:00:00"`


        Awaitable companion of the sync variant. The returned object resolves the request off the calling thread so a running event loop keeps servicing other coroutines.
        """
        ...

    def option_history_trade_builder(
        self,
        symbol: str,
        expiration: Union[str, date, datetime],
        date: Union[str, date, datetime],
    ) -> OptionHistoryTradeBuilder:
        """Fluent builder for `option_history_trade`. Chain the optional setters, then call `.list()` (or `.list_async()`) to execute; the returned typed list wrapper exposes `.to_list()` / `.to_arrow()` / `.to_pandas()` / `.to_polars()`."""
        ...

    def option_history_quote(
        self,
        symbol: str,
        expiration: Union[str, date, datetime],
        date: Union[str, date, datetime],
        *,
        strike: Optional[str] = None,
        right: Optional[str] = None,
        interval: Optional[str] = None,
        start_time: Optional[Union[str, time, datetime]] = None,
        end_time: Optional[Union[str, time, datetime]] = None,
        max_dte: Optional[int] = None,
        strike_range: Optional[int] = None,
        start_date: Optional[Union[str, date, datetime]] = None,
        end_date: Optional[Union[str, date, datetime]] = None,
        timeout_ms: Optional[int] = None,
    ) -> QuoteTickList:
        """Fetch NBBO quotes for an option contract on a given date.

        - Returns every NBBO quote reported by OPRA. 
        - If the ``interval`` parameter is specified, the quote for each interval represents the last quote at the interval's timestamp.
        - Multi-day requests are limited to 1 month of data, and must specify an expiration.

        Defaults (upstream):
        - `strike`: `"*"`
        - `right`: `"both"`
        - `interval`: `"1s"`
        - `start_time`: `"09:30:00"`
        - `end_time`: `"16:00:00"`
        """
        ...

    def option_history_quote_async(
        self,
        symbol: str,
        expiration: Union[str, date, datetime],
        date: Union[str, date, datetime],
        *,
        strike: Optional[str] = None,
        right: Optional[str] = None,
        interval: Optional[str] = None,
        start_time: Optional[Union[str, time, datetime]] = None,
        end_time: Optional[Union[str, time, datetime]] = None,
        max_dte: Optional[int] = None,
        strike_range: Optional[int] = None,
        start_date: Optional[Union[str, date, datetime]] = None,
        end_date: Optional[Union[str, date, datetime]] = None,
        timeout_ms: Optional[int] = None,
    ) -> Awaitable[QuoteTickList]:
        """Fetch NBBO quotes for an option contract on a given date.

        - Returns every NBBO quote reported by OPRA. 
        - If the ``interval`` parameter is specified, the quote for each interval represents the last quote at the interval's timestamp.
        - Multi-day requests are limited to 1 month of data, and must specify an expiration.

        Defaults (upstream):
        - `strike`: `"*"`
        - `right`: `"both"`
        - `interval`: `"1s"`
        - `start_time`: `"09:30:00"`
        - `end_time`: `"16:00:00"`


        Awaitable companion of the sync variant. The returned object resolves the request off the calling thread so a running event loop keeps servicing other coroutines.
        """
        ...

    def option_history_quote_builder(
        self,
        symbol: str,
        expiration: Union[str, date, datetime],
        date: Union[str, date, datetime],
    ) -> OptionHistoryQuoteBuilder:
        """Fluent builder for `option_history_quote`. Chain the optional setters, then call `.list()` (or `.list_async()`) to execute; the returned typed list wrapper exposes `.to_list()` / `.to_arrow()` / `.to_pandas()` / `.to_polars()`."""
        ...

    def option_history_trade_quote(
        self,
        symbol: str,
        expiration: Union[str, date, datetime],
        date: Union[str, date, datetime],
        *,
        strike: Optional[str] = None,
        right: Optional[str] = None,
        start_time: Optional[Union[str, time, datetime]] = None,
        end_time: Optional[Union[str, time, datetime]] = None,
        exclusive: Optional[bool] = False,
        max_dte: Optional[int] = None,
        strike_range: Optional[int] = None,
        start_date: Optional[Union[str, date, datetime]] = None,
        end_date: Optional[Union[str, date, datetime]] = None,
        timeout_ms: Optional[int] = None,
    ) -> TradeQuoteTickList:
        """Fetch combined trade + quote ticks for an option contract.

        - Returns every trade reported by OPRA paired with the last NBBO quote reported by OPRA at the time of trade.
        - A quote is matched with a trade if its timestamp ``<=`` the trade timestamp. 
        - To match trades with quotes timestamps that are ``<`` the trade timestamp, specify the ``exclusive``parameter to ``true``. After thorough testing, we have determined that using ``exclusive=true`` might yield better results for various applications.
        - Multi-day requests are limited to 1 month of data, and must specify an expiration.

        Defaults (upstream):
        - `strike`: `"*"`
        - `right`: `"both"`
        - `start_time`: `"09:30:00"`
        - `end_time`: `"16:00:00"`
        - `exclusive`: `false`
        """
        ...

    def option_history_trade_quote_async(
        self,
        symbol: str,
        expiration: Union[str, date, datetime],
        date: Union[str, date, datetime],
        *,
        strike: Optional[str] = None,
        right: Optional[str] = None,
        start_time: Optional[Union[str, time, datetime]] = None,
        end_time: Optional[Union[str, time, datetime]] = None,
        exclusive: Optional[bool] = None,
        max_dte: Optional[int] = None,
        strike_range: Optional[int] = None,
        start_date: Optional[Union[str, date, datetime]] = None,
        end_date: Optional[Union[str, date, datetime]] = None,
        timeout_ms: Optional[int] = None,
    ) -> Awaitable[TradeQuoteTickList]:
        """Fetch combined trade + quote ticks for an option contract.

        - Returns every trade reported by OPRA paired with the last NBBO quote reported by OPRA at the time of trade.
        - A quote is matched with a trade if its timestamp ``<=`` the trade timestamp. 
        - To match trades with quotes timestamps that are ``<`` the trade timestamp, specify the ``exclusive``parameter to ``true``. After thorough testing, we have determined that using ``exclusive=true`` might yield better results for various applications.
        - Multi-day requests are limited to 1 month of data, and must specify an expiration.

        Defaults (upstream):
        - `strike`: `"*"`
        - `right`: `"both"`
        - `start_time`: `"09:30:00"`
        - `end_time`: `"16:00:00"`
        - `exclusive`: `false`


        Awaitable companion of the sync variant. The returned object resolves the request off the calling thread so a running event loop keeps servicing other coroutines.
        """
        ...

    def option_history_trade_quote_builder(
        self,
        symbol: str,
        expiration: Union[str, date, datetime],
        date: Union[str, date, datetime],
    ) -> OptionHistoryTradeQuoteBuilder:
        """Fluent builder for `option_history_trade_quote`. Chain the optional setters, then call `.list()` (or `.list_async()`) to execute; the returned typed list wrapper exposes `.to_list()` / `.to_arrow()` / `.to_pandas()` / `.to_polars()`."""
        ...

    def option_history_open_interest(
        self,
        symbol: str,
        expiration: Union[str, date, datetime],
        date: Union[str, date, datetime],
        *,
        strike: Optional[str] = None,
        right: Optional[str] = None,
        max_dte: Optional[int] = None,
        strike_range: Optional[int] = None,
        start_date: Optional[Union[str, date, datetime]] = None,
        end_date: Optional[Union[str, date, datetime]] = None,
        timeout_ms: Optional[int] = None,
    ) -> OpenInterestTickList:
        """Fetch open interest history for an option contract.

        - Open Interest is normally reported once per day by OPRA at approximately 06:30 ET.
        - A new open interest message might not be sent by OPRA if there is no open interest for the option contract.
        - The reported open interest represents the open interest at the end of the previous trading day.

        Defaults (upstream):
        - `strike`: `"*"`
        - `right`: `"both"`
        """
        ...

    def option_history_open_interest_async(
        self,
        symbol: str,
        expiration: Union[str, date, datetime],
        date: Union[str, date, datetime],
        *,
        strike: Optional[str] = None,
        right: Optional[str] = None,
        max_dte: Optional[int] = None,
        strike_range: Optional[int] = None,
        start_date: Optional[Union[str, date, datetime]] = None,
        end_date: Optional[Union[str, date, datetime]] = None,
        timeout_ms: Optional[int] = None,
    ) -> Awaitable[OpenInterestTickList]:
        """Fetch open interest history for an option contract.

        - Open Interest is normally reported once per day by OPRA at approximately 06:30 ET.
        - A new open interest message might not be sent by OPRA if there is no open interest for the option contract.
        - The reported open interest represents the open interest at the end of the previous trading day.

        Defaults (upstream):
        - `strike`: `"*"`
        - `right`: `"both"`


        Awaitable companion of the sync variant. The returned object resolves the request off the calling thread so a running event loop keeps servicing other coroutines.
        """
        ...

    def option_history_open_interest_builder(
        self,
        symbol: str,
        expiration: Union[str, date, datetime],
        date: Union[str, date, datetime],
    ) -> OptionHistoryOpenInterestBuilder:
        """Fluent builder for `option_history_open_interest`. Chain the optional setters, then call `.list()` (or `.list_async()`) to execute; the returned typed list wrapper exposes `.to_list()` / `.to_arrow()` / `.to_pandas()` / `.to_polars()`."""
        ...

    def option_history_greeks_eod(
        self,
        symbol: str,
        expiration: Union[str, date, datetime],
        start_date: Union[str, date, datetime],
        end_date: Union[str, date, datetime],
        *,
        strike: Optional[str] = None,
        right: Optional[str] = None,
        annual_dividend: Optional[float] = None,
        rate_type: Optional[str] = None,
        rate_value: Optional[float] = None,
        version: Optional[str] = None,
        underlyer_use_nbbo: Optional[bool] = False,
        max_dte: Optional[int] = None,
        strike_range: Optional[int] = None,
        timeout_ms: Optional[int] = None,
    ) -> GreeksEodTickList:
        """Fetch end-of-day Greeks history for an option contract.

        - Returns the data for all contracts that share the same provided symbol and expiration.
        - Uses Theta Data's EOD reports that get generated at 17:15 ET each day. The closing option price and closing underlying price are used for the greeks calculation.
        - **Set `expiration` to ``*`` if you want to retrieve data for every option that shares the same ``symbol``. (note: Any ``expiration=*`` must be requested day by day)**

        Defaults (upstream):
        - `strike`: `"*"`
        - `right`: `"both"`
        - `rate_type`: `"sofr"`
        - `version`: `"latest"`
        - `underlyer_use_nbbo`: `false`
        """
        ...

    def option_history_greeks_eod_async(
        self,
        symbol: str,
        expiration: Union[str, date, datetime],
        start_date: Union[str, date, datetime],
        end_date: Union[str, date, datetime],
        *,
        strike: Optional[str] = None,
        right: Optional[str] = None,
        annual_dividend: Optional[float] = None,
        rate_type: Optional[str] = None,
        rate_value: Optional[float] = None,
        version: Optional[str] = None,
        underlyer_use_nbbo: Optional[bool] = None,
        max_dte: Optional[int] = None,
        strike_range: Optional[int] = None,
        timeout_ms: Optional[int] = None,
    ) -> Awaitable[GreeksEodTickList]:
        """Fetch end-of-day Greeks history for an option contract.

        - Returns the data for all contracts that share the same provided symbol and expiration.
        - Uses Theta Data's EOD reports that get generated at 17:15 ET each day. The closing option price and closing underlying price are used for the greeks calculation.
        - **Set `expiration` to ``*`` if you want to retrieve data for every option that shares the same ``symbol``. (note: Any ``expiration=*`` must be requested day by day)**

        Defaults (upstream):
        - `strike`: `"*"`
        - `right`: `"both"`
        - `rate_type`: `"sofr"`
        - `version`: `"latest"`
        - `underlyer_use_nbbo`: `false`


        Awaitable companion of the sync variant. The returned object resolves the request off the calling thread so a running event loop keeps servicing other coroutines.
        """
        ...

    def option_history_greeks_eod_builder(
        self,
        symbol: str,
        expiration: Union[str, date, datetime],
        start_date: Union[str, date, datetime],
        end_date: Union[str, date, datetime],
    ) -> OptionHistoryGreeksEodBuilder:
        """Fluent builder for `option_history_greeks_eod`. Chain the optional setters, then call `.list()` (or `.list_async()`) to execute; the returned typed list wrapper exposes `.to_list()` / `.to_arrow()` / `.to_pandas()` / `.to_polars()`."""
        ...

    def option_history_greeks_all(
        self,
        symbol: str,
        expiration: Union[str, date, datetime],
        date: Union[str, date, datetime],
        *,
        strike: Optional[str] = None,
        right: Optional[str] = None,
        interval: Optional[str] = None,
        start_time: Optional[Union[str, time, datetime]] = None,
        end_time: Optional[Union[str, time, datetime]] = None,
        annual_dividend: Optional[float] = None,
        rate_type: Optional[str] = None,
        rate_value: Optional[float] = None,
        version: Optional[str] = None,
        strike_range: Optional[int] = None,
        start_date: Optional[Union[str, date, datetime]] = None,
        end_date: Optional[Union[str, date, datetime]] = None,
        timeout_ms: Optional[int] = None,
    ) -> GreeksAllTickList:
        """Fetch all Greeks history for an option contract (intraday, sampled by interval).

        - Returns the data for all contracts that share the same provided symbol and expiration. 
        - Calculated using the option and underlying midpoint price. If an interval size is specified (*highly recommended*), the option quote used in the calculation follows the same rules as the quote endpoint. 
        - The underlying price represents whatever the last underlying price was at the ``timestamp`` field. You can read more about how Theta Data calculates greeks here.
        - Multi-day requests are limited to 1 month of data.

        Defaults (upstream):
        - `strike`: `"*"`
        - `right`: `"both"`
        - `interval`: `"1s"`
        - `start_time`: `"09:30:00"`
        - `end_time`: `"16:00:00"`
        - `rate_type`: `"sofr"`
        - `version`: `"latest"`
        """
        ...

    def option_history_greeks_all_async(
        self,
        symbol: str,
        expiration: Union[str, date, datetime],
        date: Union[str, date, datetime],
        *,
        strike: Optional[str] = None,
        right: Optional[str] = None,
        interval: Optional[str] = None,
        start_time: Optional[Union[str, time, datetime]] = None,
        end_time: Optional[Union[str, time, datetime]] = None,
        annual_dividend: Optional[float] = None,
        rate_type: Optional[str] = None,
        rate_value: Optional[float] = None,
        version: Optional[str] = None,
        strike_range: Optional[int] = None,
        start_date: Optional[Union[str, date, datetime]] = None,
        end_date: Optional[Union[str, date, datetime]] = None,
        timeout_ms: Optional[int] = None,
    ) -> Awaitable[GreeksAllTickList]:
        """Fetch all Greeks history for an option contract (intraday, sampled by interval).

        - Returns the data for all contracts that share the same provided symbol and expiration. 
        - Calculated using the option and underlying midpoint price. If an interval size is specified (*highly recommended*), the option quote used in the calculation follows the same rules as the quote endpoint. 
        - The underlying price represents whatever the last underlying price was at the ``timestamp`` field. You can read more about how Theta Data calculates greeks here.
        - Multi-day requests are limited to 1 month of data.

        Defaults (upstream):
        - `strike`: `"*"`
        - `right`: `"both"`
        - `interval`: `"1s"`
        - `start_time`: `"09:30:00"`
        - `end_time`: `"16:00:00"`
        - `rate_type`: `"sofr"`
        - `version`: `"latest"`


        Awaitable companion of the sync variant. The returned object resolves the request off the calling thread so a running event loop keeps servicing other coroutines.
        """
        ...

    def option_history_greeks_all_builder(
        self,
        symbol: str,
        expiration: Union[str, date, datetime],
        date: Union[str, date, datetime],
    ) -> OptionHistoryGreeksAllBuilder:
        """Fluent builder for `option_history_greeks_all`. Chain the optional setters, then call `.list()` (or `.list_async()`) to execute; the returned typed list wrapper exposes `.to_list()` / `.to_arrow()` / `.to_pandas()` / `.to_polars()`."""
        ...

    def option_history_trade_greeks_all(
        self,
        symbol: str,
        expiration: Union[str, date, datetime],
        date: Union[str, date, datetime],
        *,
        strike: Optional[str] = None,
        right: Optional[str] = None,
        start_time: Optional[Union[str, time, datetime]] = None,
        end_time: Optional[Union[str, time, datetime]] = None,
        annual_dividend: Optional[float] = None,
        rate_type: Optional[str] = None,
        rate_value: Optional[float] = None,
        version: Optional[str] = None,
        max_dte: Optional[int] = None,
        strike_range: Optional[int] = None,
        start_date: Optional[Union[str, date, datetime]] = None,
        end_date: Optional[Union[str, date, datetime]] = None,
        timeout_ms: Optional[int] = None,
    ) -> TradeGreeksAllTickList:
        """Fetch all Greeks on each trade for an option contract.

        - Returns the data for all contracts that share the same provided symbol and expiration. 
        - Calculates greeks for every trade reported by OPRA.
        - The underlying price represents whatever the last underlying price was at the ``timestamp`` field. You can read more about how Theta Data calculates greeks here.
        - Multi-day requests are limited to 1 month of data, and must specify an expiration.

        Defaults (upstream):
        - `strike`: `"*"`
        - `right`: `"both"`
        - `start_time`: `"09:30:00"`
        - `end_time`: `"16:00:00"`
        - `rate_type`: `"sofr"`
        - `version`: `"latest"`
        """
        ...

    def option_history_trade_greeks_all_async(
        self,
        symbol: str,
        expiration: Union[str, date, datetime],
        date: Union[str, date, datetime],
        *,
        strike: Optional[str] = None,
        right: Optional[str] = None,
        start_time: Optional[Union[str, time, datetime]] = None,
        end_time: Optional[Union[str, time, datetime]] = None,
        annual_dividend: Optional[float] = None,
        rate_type: Optional[str] = None,
        rate_value: Optional[float] = None,
        version: Optional[str] = None,
        max_dte: Optional[int] = None,
        strike_range: Optional[int] = None,
        start_date: Optional[Union[str, date, datetime]] = None,
        end_date: Optional[Union[str, date, datetime]] = None,
        timeout_ms: Optional[int] = None,
    ) -> Awaitable[TradeGreeksAllTickList]:
        """Fetch all Greeks on each trade for an option contract.

        - Returns the data for all contracts that share the same provided symbol and expiration. 
        - Calculates greeks for every trade reported by OPRA.
        - The underlying price represents whatever the last underlying price was at the ``timestamp`` field. You can read more about how Theta Data calculates greeks here.
        - Multi-day requests are limited to 1 month of data, and must specify an expiration.

        Defaults (upstream):
        - `strike`: `"*"`
        - `right`: `"both"`
        - `start_time`: `"09:30:00"`
        - `end_time`: `"16:00:00"`
        - `rate_type`: `"sofr"`
        - `version`: `"latest"`


        Awaitable companion of the sync variant. The returned object resolves the request off the calling thread so a running event loop keeps servicing other coroutines.
        """
        ...

    def option_history_trade_greeks_all_builder(
        self,
        symbol: str,
        expiration: Union[str, date, datetime],
        date: Union[str, date, datetime],
    ) -> OptionHistoryTradeGreeksAllBuilder:
        """Fluent builder for `option_history_trade_greeks_all`. Chain the optional setters, then call `.list()` (or `.list_async()`) to execute; the returned typed list wrapper exposes `.to_list()` / `.to_arrow()` / `.to_pandas()` / `.to_polars()`."""
        ...

    def option_history_greeks_first_order(
        self,
        symbol: str,
        expiration: Union[str, date, datetime],
        date: Union[str, date, datetime],
        *,
        strike: Optional[str] = None,
        right: Optional[str] = None,
        interval: Optional[str] = None,
        start_time: Optional[Union[str, time, datetime]] = None,
        end_time: Optional[Union[str, time, datetime]] = None,
        annual_dividend: Optional[float] = None,
        rate_type: Optional[str] = None,
        rate_value: Optional[float] = None,
        version: Optional[str] = None,
        strike_range: Optional[int] = None,
        start_date: Optional[Union[str, date, datetime]] = None,
        end_date: Optional[Union[str, date, datetime]] = None,
        timeout_ms: Optional[int] = None,
    ) -> GreeksFirstOrderTickList:
        """Fetch first-order Greeks history (intraday, sampled by interval).

        - Returns the data for all contracts that share the same provided symbol and expiration. 
        - Calculated using the option and underlying midpoint price. If an interval size is specified (*highly recommended*), the option quote used in the calculation follows the same rules as the quote endpoint. 
        - The underlying price represents whatever the last underlying price was at the ``timestamp`` field. You can read more about how Theta Data calculates greeks here.
        - Multi-day requests are limited to 1 month of data.

        Defaults (upstream):
        - `strike`: `"*"`
        - `right`: `"both"`
        - `interval`: `"1s"`
        - `start_time`: `"09:30:00"`
        - `end_time`: `"16:00:00"`
        - `rate_type`: `"sofr"`
        - `version`: `"latest"`
        """
        ...

    def option_history_greeks_first_order_async(
        self,
        symbol: str,
        expiration: Union[str, date, datetime],
        date: Union[str, date, datetime],
        *,
        strike: Optional[str] = None,
        right: Optional[str] = None,
        interval: Optional[str] = None,
        start_time: Optional[Union[str, time, datetime]] = None,
        end_time: Optional[Union[str, time, datetime]] = None,
        annual_dividend: Optional[float] = None,
        rate_type: Optional[str] = None,
        rate_value: Optional[float] = None,
        version: Optional[str] = None,
        strike_range: Optional[int] = None,
        start_date: Optional[Union[str, date, datetime]] = None,
        end_date: Optional[Union[str, date, datetime]] = None,
        timeout_ms: Optional[int] = None,
    ) -> Awaitable[GreeksFirstOrderTickList]:
        """Fetch first-order Greeks history (intraday, sampled by interval).

        - Returns the data for all contracts that share the same provided symbol and expiration. 
        - Calculated using the option and underlying midpoint price. If an interval size is specified (*highly recommended*), the option quote used in the calculation follows the same rules as the quote endpoint. 
        - The underlying price represents whatever the last underlying price was at the ``timestamp`` field. You can read more about how Theta Data calculates greeks here.
        - Multi-day requests are limited to 1 month of data.

        Defaults (upstream):
        - `strike`: `"*"`
        - `right`: `"both"`
        - `interval`: `"1s"`
        - `start_time`: `"09:30:00"`
        - `end_time`: `"16:00:00"`
        - `rate_type`: `"sofr"`
        - `version`: `"latest"`


        Awaitable companion of the sync variant. The returned object resolves the request off the calling thread so a running event loop keeps servicing other coroutines.
        """
        ...

    def option_history_greeks_first_order_builder(
        self,
        symbol: str,
        expiration: Union[str, date, datetime],
        date: Union[str, date, datetime],
    ) -> OptionHistoryGreeksFirstOrderBuilder:
        """Fluent builder for `option_history_greeks_first_order`. Chain the optional setters, then call `.list()` (or `.list_async()`) to execute; the returned typed list wrapper exposes `.to_list()` / `.to_arrow()` / `.to_pandas()` / `.to_polars()`."""
        ...

    def option_history_trade_greeks_first_order(
        self,
        symbol: str,
        expiration: Union[str, date, datetime],
        date: Union[str, date, datetime],
        *,
        strike: Optional[str] = None,
        right: Optional[str] = None,
        start_time: Optional[Union[str, time, datetime]] = None,
        end_time: Optional[Union[str, time, datetime]] = None,
        annual_dividend: Optional[float] = None,
        rate_type: Optional[str] = None,
        rate_value: Optional[float] = None,
        version: Optional[str] = None,
        max_dte: Optional[int] = None,
        strike_range: Optional[int] = None,
        start_date: Optional[Union[str, date, datetime]] = None,
        end_date: Optional[Union[str, date, datetime]] = None,
        timeout_ms: Optional[int] = None,
    ) -> TradeGreeksFirstOrderTickList:
        """Fetch first-order Greeks on each trade for an option contract.

        - Returns the data for all contracts that share the same provided symbol and expiration.
        - Calculates greeks for every trade reported by OPRA.
        - The underlying price represents whatever the last underlying price was at the ``timestamp`` field. You can read more about how Theta Data calculates greeks here.
        - Multi-day requests are limited to 1 month of data, and must specify an expiration.

        Defaults (upstream):
        - `strike`: `"*"`
        - `right`: `"both"`
        - `start_time`: `"09:30:00"`
        - `end_time`: `"16:00:00"`
        - `rate_type`: `"sofr"`
        - `version`: `"latest"`
        """
        ...

    def option_history_trade_greeks_first_order_async(
        self,
        symbol: str,
        expiration: Union[str, date, datetime],
        date: Union[str, date, datetime],
        *,
        strike: Optional[str] = None,
        right: Optional[str] = None,
        start_time: Optional[Union[str, time, datetime]] = None,
        end_time: Optional[Union[str, time, datetime]] = None,
        annual_dividend: Optional[float] = None,
        rate_type: Optional[str] = None,
        rate_value: Optional[float] = None,
        version: Optional[str] = None,
        max_dte: Optional[int] = None,
        strike_range: Optional[int] = None,
        start_date: Optional[Union[str, date, datetime]] = None,
        end_date: Optional[Union[str, date, datetime]] = None,
        timeout_ms: Optional[int] = None,
    ) -> Awaitable[TradeGreeksFirstOrderTickList]:
        """Fetch first-order Greeks on each trade for an option contract.

        - Returns the data for all contracts that share the same provided symbol and expiration.
        - Calculates greeks for every trade reported by OPRA.
        - The underlying price represents whatever the last underlying price was at the ``timestamp`` field. You can read more about how Theta Data calculates greeks here.
        - Multi-day requests are limited to 1 month of data, and must specify an expiration.

        Defaults (upstream):
        - `strike`: `"*"`
        - `right`: `"both"`
        - `start_time`: `"09:30:00"`
        - `end_time`: `"16:00:00"`
        - `rate_type`: `"sofr"`
        - `version`: `"latest"`


        Awaitable companion of the sync variant. The returned object resolves the request off the calling thread so a running event loop keeps servicing other coroutines.
        """
        ...

    def option_history_trade_greeks_first_order_builder(
        self,
        symbol: str,
        expiration: Union[str, date, datetime],
        date: Union[str, date, datetime],
    ) -> OptionHistoryTradeGreeksFirstOrderBuilder:
        """Fluent builder for `option_history_trade_greeks_first_order`. Chain the optional setters, then call `.list()` (or `.list_async()`) to execute; the returned typed list wrapper exposes `.to_list()` / `.to_arrow()` / `.to_pandas()` / `.to_polars()`."""
        ...

    def option_history_greeks_second_order(
        self,
        symbol: str,
        expiration: Union[str, date, datetime],
        date: Union[str, date, datetime],
        *,
        strike: Optional[str] = None,
        right: Optional[str] = None,
        interval: Optional[str] = None,
        start_time: Optional[Union[str, time, datetime]] = None,
        end_time: Optional[Union[str, time, datetime]] = None,
        annual_dividend: Optional[float] = None,
        rate_type: Optional[str] = None,
        rate_value: Optional[float] = None,
        version: Optional[str] = None,
        strike_range: Optional[int] = None,
        start_date: Optional[Union[str, date, datetime]] = None,
        end_date: Optional[Union[str, date, datetime]] = None,
        timeout_ms: Optional[int] = None,
    ) -> GreeksSecondOrderTickList:
        """Fetch second-order Greeks history (intraday, sampled by interval).

        - Returns the data for all contracts that share the same provided symbol and expiration. 
        - Calculated using the option and underlying midpoint price. If an interval size is specified (*highly recommended*), the option quote used in the calculation follows the same rules as the quote endpoint. 
        - The underlying price represents whatever the last underlying price was at the ``timestamp`` field. You can read more about how Theta Data calculates greeks here.
        - Multi-day requests are limited to 1 month of data.

        Defaults (upstream):
        - `strike`: `"*"`
        - `right`: `"both"`
        - `interval`: `"1s"`
        - `start_time`: `"09:30:00"`
        - `end_time`: `"16:00:00"`
        - `rate_type`: `"sofr"`
        - `version`: `"latest"`
        """
        ...

    def option_history_greeks_second_order_async(
        self,
        symbol: str,
        expiration: Union[str, date, datetime],
        date: Union[str, date, datetime],
        *,
        strike: Optional[str] = None,
        right: Optional[str] = None,
        interval: Optional[str] = None,
        start_time: Optional[Union[str, time, datetime]] = None,
        end_time: Optional[Union[str, time, datetime]] = None,
        annual_dividend: Optional[float] = None,
        rate_type: Optional[str] = None,
        rate_value: Optional[float] = None,
        version: Optional[str] = None,
        strike_range: Optional[int] = None,
        start_date: Optional[Union[str, date, datetime]] = None,
        end_date: Optional[Union[str, date, datetime]] = None,
        timeout_ms: Optional[int] = None,
    ) -> Awaitable[GreeksSecondOrderTickList]:
        """Fetch second-order Greeks history (intraday, sampled by interval).

        - Returns the data for all contracts that share the same provided symbol and expiration. 
        - Calculated using the option and underlying midpoint price. If an interval size is specified (*highly recommended*), the option quote used in the calculation follows the same rules as the quote endpoint. 
        - The underlying price represents whatever the last underlying price was at the ``timestamp`` field. You can read more about how Theta Data calculates greeks here.
        - Multi-day requests are limited to 1 month of data.

        Defaults (upstream):
        - `strike`: `"*"`
        - `right`: `"both"`
        - `interval`: `"1s"`
        - `start_time`: `"09:30:00"`
        - `end_time`: `"16:00:00"`
        - `rate_type`: `"sofr"`
        - `version`: `"latest"`


        Awaitable companion of the sync variant. The returned object resolves the request off the calling thread so a running event loop keeps servicing other coroutines.
        """
        ...

    def option_history_greeks_second_order_builder(
        self,
        symbol: str,
        expiration: Union[str, date, datetime],
        date: Union[str, date, datetime],
    ) -> OptionHistoryGreeksSecondOrderBuilder:
        """Fluent builder for `option_history_greeks_second_order`. Chain the optional setters, then call `.list()` (or `.list_async()`) to execute; the returned typed list wrapper exposes `.to_list()` / `.to_arrow()` / `.to_pandas()` / `.to_polars()`."""
        ...

    def option_history_trade_greeks_second_order(
        self,
        symbol: str,
        expiration: Union[str, date, datetime],
        date: Union[str, date, datetime],
        *,
        strike: Optional[str] = None,
        right: Optional[str] = None,
        start_time: Optional[Union[str, time, datetime]] = None,
        end_time: Optional[Union[str, time, datetime]] = None,
        annual_dividend: Optional[float] = None,
        rate_type: Optional[str] = None,
        rate_value: Optional[float] = None,
        version: Optional[str] = None,
        max_dte: Optional[int] = None,
        strike_range: Optional[int] = None,
        start_date: Optional[Union[str, date, datetime]] = None,
        end_date: Optional[Union[str, date, datetime]] = None,
        timeout_ms: Optional[int] = None,
    ) -> TradeGreeksSecondOrderTickList:
        """Fetch second-order Greeks on each trade for an option contract.

        - Returns the data for all contracts that share the same provided symbol and expiration.
        - Calculates greeks for every trade reported by OPRA.
        - The underlying price represents whatever the last underlying price was at the ``timestamp`` field. You can read more about how Theta Data calculates greeks here.
        - Multi-day requests are limited to 1 month of data, and must specify an expiration.

        Defaults (upstream):
        - `strike`: `"*"`
        - `right`: `"both"`
        - `start_time`: `"09:30:00"`
        - `end_time`: `"16:00:00"`
        - `rate_type`: `"sofr"`
        - `version`: `"latest"`
        """
        ...

    def option_history_trade_greeks_second_order_async(
        self,
        symbol: str,
        expiration: Union[str, date, datetime],
        date: Union[str, date, datetime],
        *,
        strike: Optional[str] = None,
        right: Optional[str] = None,
        start_time: Optional[Union[str, time, datetime]] = None,
        end_time: Optional[Union[str, time, datetime]] = None,
        annual_dividend: Optional[float] = None,
        rate_type: Optional[str] = None,
        rate_value: Optional[float] = None,
        version: Optional[str] = None,
        max_dte: Optional[int] = None,
        strike_range: Optional[int] = None,
        start_date: Optional[Union[str, date, datetime]] = None,
        end_date: Optional[Union[str, date, datetime]] = None,
        timeout_ms: Optional[int] = None,
    ) -> Awaitable[TradeGreeksSecondOrderTickList]:
        """Fetch second-order Greeks on each trade for an option contract.

        - Returns the data for all contracts that share the same provided symbol and expiration.
        - Calculates greeks for every trade reported by OPRA.
        - The underlying price represents whatever the last underlying price was at the ``timestamp`` field. You can read more about how Theta Data calculates greeks here.
        - Multi-day requests are limited to 1 month of data, and must specify an expiration.

        Defaults (upstream):
        - `strike`: `"*"`
        - `right`: `"both"`
        - `start_time`: `"09:30:00"`
        - `end_time`: `"16:00:00"`
        - `rate_type`: `"sofr"`
        - `version`: `"latest"`


        Awaitable companion of the sync variant. The returned object resolves the request off the calling thread so a running event loop keeps servicing other coroutines.
        """
        ...

    def option_history_trade_greeks_second_order_builder(
        self,
        symbol: str,
        expiration: Union[str, date, datetime],
        date: Union[str, date, datetime],
    ) -> OptionHistoryTradeGreeksSecondOrderBuilder:
        """Fluent builder for `option_history_trade_greeks_second_order`. Chain the optional setters, then call `.list()` (or `.list_async()`) to execute; the returned typed list wrapper exposes `.to_list()` / `.to_arrow()` / `.to_pandas()` / `.to_polars()`."""
        ...

    def option_history_greeks_third_order(
        self,
        symbol: str,
        expiration: Union[str, date, datetime],
        date: Union[str, date, datetime],
        *,
        strike: Optional[str] = None,
        right: Optional[str] = None,
        interval: Optional[str] = None,
        start_time: Optional[Union[str, time, datetime]] = None,
        end_time: Optional[Union[str, time, datetime]] = None,
        annual_dividend: Optional[float] = None,
        rate_type: Optional[str] = None,
        rate_value: Optional[float] = None,
        version: Optional[str] = None,
        strike_range: Optional[int] = None,
        start_date: Optional[Union[str, date, datetime]] = None,
        end_date: Optional[Union[str, date, datetime]] = None,
        timeout_ms: Optional[int] = None,
    ) -> GreeksThirdOrderTickList:
        """Fetch third-order Greeks history (intraday, sampled by interval).

        - Returns the data for all contracts that share the same provided symbol and expiration. 
        - Calculated using the option and underlying midpoint price. If an interval size is specified (*highly recommended*), the option quote used in the calculation follows the same rules as the quote endpoint. 
        - The underlying price represents whatever the last underlying price was at the ``timestamp`` field. You can read more about how Theta Data calculates greeks here.
        - Multi-day requests are limited to 1 month of data.

        Defaults (upstream):
        - `strike`: `"*"`
        - `right`: `"both"`
        - `interval`: `"1s"`
        - `start_time`: `"09:30:00"`
        - `end_time`: `"16:00:00"`
        - `rate_type`: `"sofr"`
        - `version`: `"latest"`
        """
        ...

    def option_history_greeks_third_order_async(
        self,
        symbol: str,
        expiration: Union[str, date, datetime],
        date: Union[str, date, datetime],
        *,
        strike: Optional[str] = None,
        right: Optional[str] = None,
        interval: Optional[str] = None,
        start_time: Optional[Union[str, time, datetime]] = None,
        end_time: Optional[Union[str, time, datetime]] = None,
        annual_dividend: Optional[float] = None,
        rate_type: Optional[str] = None,
        rate_value: Optional[float] = None,
        version: Optional[str] = None,
        strike_range: Optional[int] = None,
        start_date: Optional[Union[str, date, datetime]] = None,
        end_date: Optional[Union[str, date, datetime]] = None,
        timeout_ms: Optional[int] = None,
    ) -> Awaitable[GreeksThirdOrderTickList]:
        """Fetch third-order Greeks history (intraday, sampled by interval).

        - Returns the data for all contracts that share the same provided symbol and expiration. 
        - Calculated using the option and underlying midpoint price. If an interval size is specified (*highly recommended*), the option quote used in the calculation follows the same rules as the quote endpoint. 
        - The underlying price represents whatever the last underlying price was at the ``timestamp`` field. You can read more about how Theta Data calculates greeks here.
        - Multi-day requests are limited to 1 month of data.

        Defaults (upstream):
        - `strike`: `"*"`
        - `right`: `"both"`
        - `interval`: `"1s"`
        - `start_time`: `"09:30:00"`
        - `end_time`: `"16:00:00"`
        - `rate_type`: `"sofr"`
        - `version`: `"latest"`


        Awaitable companion of the sync variant. The returned object resolves the request off the calling thread so a running event loop keeps servicing other coroutines.
        """
        ...

    def option_history_greeks_third_order_builder(
        self,
        symbol: str,
        expiration: Union[str, date, datetime],
        date: Union[str, date, datetime],
    ) -> OptionHistoryGreeksThirdOrderBuilder:
        """Fluent builder for `option_history_greeks_third_order`. Chain the optional setters, then call `.list()` (or `.list_async()`) to execute; the returned typed list wrapper exposes `.to_list()` / `.to_arrow()` / `.to_pandas()` / `.to_polars()`."""
        ...

    def option_history_trade_greeks_third_order(
        self,
        symbol: str,
        expiration: Union[str, date, datetime],
        date: Union[str, date, datetime],
        *,
        strike: Optional[str] = None,
        right: Optional[str] = None,
        start_time: Optional[Union[str, time, datetime]] = None,
        end_time: Optional[Union[str, time, datetime]] = None,
        annual_dividend: Optional[float] = None,
        rate_type: Optional[str] = None,
        rate_value: Optional[float] = None,
        version: Optional[str] = None,
        max_dte: Optional[int] = None,
        strike_range: Optional[int] = None,
        start_date: Optional[Union[str, date, datetime]] = None,
        end_date: Optional[Union[str, date, datetime]] = None,
        timeout_ms: Optional[int] = None,
    ) -> TradeGreeksThirdOrderTickList:
        """Fetch third-order Greeks on each trade for an option contract.

        - Returns the data for all contracts that share the same provided symbol and expiration.
        - Calculates greeks for every trade reported by OPRA.
        - The underlying price represents whatever the last underlying price was at the ``timestamp`` field. You can read more about how Theta Data calculates greeks here.
        - Multi-day requests are limited to 1 month of data, and must specify an expiration.

        Defaults (upstream):
        - `strike`: `"*"`
        - `right`: `"both"`
        - `start_time`: `"09:30:00"`
        - `end_time`: `"16:00:00"`
        - `rate_type`: `"sofr"`
        - `version`: `"latest"`
        """
        ...

    def option_history_trade_greeks_third_order_async(
        self,
        symbol: str,
        expiration: Union[str, date, datetime],
        date: Union[str, date, datetime],
        *,
        strike: Optional[str] = None,
        right: Optional[str] = None,
        start_time: Optional[Union[str, time, datetime]] = None,
        end_time: Optional[Union[str, time, datetime]] = None,
        annual_dividend: Optional[float] = None,
        rate_type: Optional[str] = None,
        rate_value: Optional[float] = None,
        version: Optional[str] = None,
        max_dte: Optional[int] = None,
        strike_range: Optional[int] = None,
        start_date: Optional[Union[str, date, datetime]] = None,
        end_date: Optional[Union[str, date, datetime]] = None,
        timeout_ms: Optional[int] = None,
    ) -> Awaitable[TradeGreeksThirdOrderTickList]:
        """Fetch third-order Greeks on each trade for an option contract.

        - Returns the data for all contracts that share the same provided symbol and expiration.
        - Calculates greeks for every trade reported by OPRA.
        - The underlying price represents whatever the last underlying price was at the ``timestamp`` field. You can read more about how Theta Data calculates greeks here.
        - Multi-day requests are limited to 1 month of data, and must specify an expiration.

        Defaults (upstream):
        - `strike`: `"*"`
        - `right`: `"both"`
        - `start_time`: `"09:30:00"`
        - `end_time`: `"16:00:00"`
        - `rate_type`: `"sofr"`
        - `version`: `"latest"`


        Awaitable companion of the sync variant. The returned object resolves the request off the calling thread so a running event loop keeps servicing other coroutines.
        """
        ...

    def option_history_trade_greeks_third_order_builder(
        self,
        symbol: str,
        expiration: Union[str, date, datetime],
        date: Union[str, date, datetime],
    ) -> OptionHistoryTradeGreeksThirdOrderBuilder:
        """Fluent builder for `option_history_trade_greeks_third_order`. Chain the optional setters, then call `.list()` (or `.list_async()`) to execute; the returned typed list wrapper exposes `.to_list()` / `.to_arrow()` / `.to_pandas()` / `.to_polars()`."""
        ...

    def option_history_greeks_implied_volatility(
        self,
        symbol: str,
        expiration: Union[str, date, datetime],
        date: Union[str, date, datetime],
        *,
        strike: Optional[str] = None,
        right: Optional[str] = None,
        interval: Optional[str] = None,
        start_time: Optional[Union[str, time, datetime]] = None,
        end_time: Optional[Union[str, time, datetime]] = None,
        annual_dividend: Optional[float] = None,
        rate_type: Optional[str] = None,
        rate_value: Optional[float] = None,
        version: Optional[str] = None,
        strike_range: Optional[int] = None,
        start_date: Optional[Union[str, date, datetime]] = None,
        end_date: Optional[Union[str, date, datetime]] = None,
        timeout_ms: Optional[int] = None,
    ) -> IvTickList:
        """Fetch implied volatility history (intraday, sampled by interval).

        - Returns implied volatilies calculated using the national best bid, mid, and ask price of the option respectively. 
        - The underlying price represents whatever the last underlying price was at the ``timestamp`` field. You can read more about how Theta Data calculates greeks here.
        - Multi-day requests are limited to 1 month of data.

        Defaults (upstream):
        - `strike`: `"*"`
        - `right`: `"both"`
        - `interval`: `"1s"`
        - `start_time`: `"09:30:00"`
        - `end_time`: `"16:00:00"`
        - `rate_type`: `"sofr"`
        - `version`: `"latest"`
        """
        ...

    def option_history_greeks_implied_volatility_async(
        self,
        symbol: str,
        expiration: Union[str, date, datetime],
        date: Union[str, date, datetime],
        *,
        strike: Optional[str] = None,
        right: Optional[str] = None,
        interval: Optional[str] = None,
        start_time: Optional[Union[str, time, datetime]] = None,
        end_time: Optional[Union[str, time, datetime]] = None,
        annual_dividend: Optional[float] = None,
        rate_type: Optional[str] = None,
        rate_value: Optional[float] = None,
        version: Optional[str] = None,
        strike_range: Optional[int] = None,
        start_date: Optional[Union[str, date, datetime]] = None,
        end_date: Optional[Union[str, date, datetime]] = None,
        timeout_ms: Optional[int] = None,
    ) -> Awaitable[IvTickList]:
        """Fetch implied volatility history (intraday, sampled by interval).

        - Returns implied volatilies calculated using the national best bid, mid, and ask price of the option respectively. 
        - The underlying price represents whatever the last underlying price was at the ``timestamp`` field. You can read more about how Theta Data calculates greeks here.
        - Multi-day requests are limited to 1 month of data.

        Defaults (upstream):
        - `strike`: `"*"`
        - `right`: `"both"`
        - `interval`: `"1s"`
        - `start_time`: `"09:30:00"`
        - `end_time`: `"16:00:00"`
        - `rate_type`: `"sofr"`
        - `version`: `"latest"`


        Awaitable companion of the sync variant. The returned object resolves the request off the calling thread so a running event loop keeps servicing other coroutines.
        """
        ...

    def option_history_greeks_implied_volatility_builder(
        self,
        symbol: str,
        expiration: Union[str, date, datetime],
        date: Union[str, date, datetime],
    ) -> OptionHistoryGreeksImpliedVolatilityBuilder:
        """Fluent builder for `option_history_greeks_implied_volatility`. Chain the optional setters, then call `.list()` (or `.list_async()`) to execute; the returned typed list wrapper exposes `.to_list()` / `.to_arrow()` / `.to_pandas()` / `.to_polars()`."""
        ...

    def option_history_trade_greeks_implied_volatility(
        self,
        symbol: str,
        expiration: Union[str, date, datetime],
        date: Union[str, date, datetime],
        *,
        strike: Optional[str] = None,
        right: Optional[str] = None,
        start_time: Optional[Union[str, time, datetime]] = None,
        end_time: Optional[Union[str, time, datetime]] = None,
        annual_dividend: Optional[float] = None,
        rate_type: Optional[str] = None,
        rate_value: Optional[float] = None,
        version: Optional[str] = None,
        max_dte: Optional[int] = None,
        strike_range: Optional[int] = None,
        start_date: Optional[Union[str, date, datetime]] = None,
        end_date: Optional[Union[str, date, datetime]] = None,
        timeout_ms: Optional[int] = None,
    ) -> TradeGreeksImpliedVolatilityTickList:
        """Fetch implied volatility on each trade for an option contract.

        - Returns implied volatilies calculated using the trade reported by OPRA. 
        - The underlying price represents whatever the last underlying price was at the ``timestamp`` field. You can read more about how Theta Data calculates greeks here.
        - Multi-day requests are limited to 1 month of data, and must specify an expiration.

        Defaults (upstream):
        - `strike`: `"*"`
        - `right`: `"both"`
        - `start_time`: `"09:30:00"`
        - `end_time`: `"16:00:00"`
        - `rate_type`: `"sofr"`
        - `version`: `"latest"`
        """
        ...

    def option_history_trade_greeks_implied_volatility_async(
        self,
        symbol: str,
        expiration: Union[str, date, datetime],
        date: Union[str, date, datetime],
        *,
        strike: Optional[str] = None,
        right: Optional[str] = None,
        start_time: Optional[Union[str, time, datetime]] = None,
        end_time: Optional[Union[str, time, datetime]] = None,
        annual_dividend: Optional[float] = None,
        rate_type: Optional[str] = None,
        rate_value: Optional[float] = None,
        version: Optional[str] = None,
        max_dte: Optional[int] = None,
        strike_range: Optional[int] = None,
        start_date: Optional[Union[str, date, datetime]] = None,
        end_date: Optional[Union[str, date, datetime]] = None,
        timeout_ms: Optional[int] = None,
    ) -> Awaitable[TradeGreeksImpliedVolatilityTickList]:
        """Fetch implied volatility on each trade for an option contract.

        - Returns implied volatilies calculated using the trade reported by OPRA. 
        - The underlying price represents whatever the last underlying price was at the ``timestamp`` field. You can read more about how Theta Data calculates greeks here.
        - Multi-day requests are limited to 1 month of data, and must specify an expiration.

        Defaults (upstream):
        - `strike`: `"*"`
        - `right`: `"both"`
        - `start_time`: `"09:30:00"`
        - `end_time`: `"16:00:00"`
        - `rate_type`: `"sofr"`
        - `version`: `"latest"`


        Awaitable companion of the sync variant. The returned object resolves the request off the calling thread so a running event loop keeps servicing other coroutines.
        """
        ...

    def option_history_trade_greeks_implied_volatility_builder(
        self,
        symbol: str,
        expiration: Union[str, date, datetime],
        date: Union[str, date, datetime],
    ) -> OptionHistoryTradeGreeksImpliedVolatilityBuilder:
        """Fluent builder for `option_history_trade_greeks_implied_volatility`. Chain the optional setters, then call `.list()` (or `.list_async()`) to execute; the returned typed list wrapper exposes `.to_list()` / `.to_arrow()` / `.to_pandas()` / `.to_polars()`."""
        ...

    def option_at_time_trade(
        self,
        symbol: str,
        expiration: Union[str, date, datetime],
        start_date: Union[str, date, datetime],
        end_date: Union[str, date, datetime],
        time_of_day: Union[str, time, datetime],
        *,
        strike: Optional[str] = None,
        right: Optional[str] = None,
        max_dte: Optional[int] = None,
        strike_range: Optional[int] = None,
        timeout_ms: Optional[int] = None,
    ) -> TradeTickList:
        """Fetch the trade at a specific time of day across a date range for an option.

        - Returns the last trade reported by OPRA at a specified millisecond of the day.
        - Trade condition mappings can be found here.
        - Extended trade conditions are not reported by OPRA for options, so they can be ignored.
        - The ``time_of_day``parameter represents the 00:00:00.000 ET that the trade should be provided for.

        Defaults (upstream):
        - `strike`: `"*"`
        - `right`: `"both"`
        """
        ...

    def option_at_time_trade_async(
        self,
        symbol: str,
        expiration: Union[str, date, datetime],
        start_date: Union[str, date, datetime],
        end_date: Union[str, date, datetime],
        time_of_day: Union[str, time, datetime],
        *,
        strike: Optional[str] = None,
        right: Optional[str] = None,
        max_dte: Optional[int] = None,
        strike_range: Optional[int] = None,
        timeout_ms: Optional[int] = None,
    ) -> Awaitable[TradeTickList]:
        """Fetch the trade at a specific time of day across a date range for an option.

        - Returns the last trade reported by OPRA at a specified millisecond of the day.
        - Trade condition mappings can be found here.
        - Extended trade conditions are not reported by OPRA for options, so they can be ignored.
        - The ``time_of_day``parameter represents the 00:00:00.000 ET that the trade should be provided for.

        Defaults (upstream):
        - `strike`: `"*"`
        - `right`: `"both"`


        Awaitable companion of the sync variant. The returned object resolves the request off the calling thread so a running event loop keeps servicing other coroutines.
        """
        ...

    def option_at_time_trade_builder(
        self,
        symbol: str,
        expiration: Union[str, date, datetime],
        start_date: Union[str, date, datetime],
        end_date: Union[str, date, datetime],
        time_of_day: Union[str, time, datetime],
    ) -> OptionAtTimeTradeBuilder:
        """Fluent builder for `option_at_time_trade`. Chain the optional setters, then call `.list()` (or `.list_async()`) to execute; the returned typed list wrapper exposes `.to_list()` / `.to_arrow()` / `.to_pandas()` / `.to_polars()`."""
        ...

    def option_at_time_quote(
        self,
        symbol: str,
        expiration: Union[str, date, datetime],
        start_date: Union[str, date, datetime],
        end_date: Union[str, date, datetime],
        time_of_day: Union[str, time, datetime],
        *,
        strike: Optional[str] = None,
        right: Optional[str] = None,
        max_dte: Optional[int] = None,
        strike_range: Optional[int] = None,
        timeout_ms: Optional[int] = None,
    ) -> QuoteTickList:
        """Fetch the quote at a specific time of day across a date range for an option.

        - Returns the last NBBO quote reported by OPRA at a specified millisecond of the day.
        - The ``time_of_day``parameter represents the 00:00:00.000 ET that the quote should be provided for.

        Defaults (upstream):
        - `strike`: `"*"`
        - `right`: `"both"`
        """
        ...

    def option_at_time_quote_async(
        self,
        symbol: str,
        expiration: Union[str, date, datetime],
        start_date: Union[str, date, datetime],
        end_date: Union[str, date, datetime],
        time_of_day: Union[str, time, datetime],
        *,
        strike: Optional[str] = None,
        right: Optional[str] = None,
        max_dte: Optional[int] = None,
        strike_range: Optional[int] = None,
        timeout_ms: Optional[int] = None,
    ) -> Awaitable[QuoteTickList]:
        """Fetch the quote at a specific time of day across a date range for an option.

        - Returns the last NBBO quote reported by OPRA at a specified millisecond of the day.
        - The ``time_of_day``parameter represents the 00:00:00.000 ET that the quote should be provided for.

        Defaults (upstream):
        - `strike`: `"*"`
        - `right`: `"both"`


        Awaitable companion of the sync variant. The returned object resolves the request off the calling thread so a running event loop keeps servicing other coroutines.
        """
        ...

    def option_at_time_quote_builder(
        self,
        symbol: str,
        expiration: Union[str, date, datetime],
        start_date: Union[str, date, datetime],
        end_date: Union[str, date, datetime],
        time_of_day: Union[str, time, datetime],
    ) -> OptionAtTimeQuoteBuilder:
        """Fluent builder for `option_at_time_quote`. Chain the optional setters, then call `.list()` (or `.list_async()`) to execute; the returned typed list wrapper exposes `.to_list()` / `.to_arrow()` / `.to_pandas()` / `.to_polars()`."""
        ...

    def index_list_symbols(
        self,
        *,
        timeout_ms: Optional[int] = None,
    ) -> StringList:
        """List all available index symbols.

        A symbol can be defined as a unique identifier for a stock / underlying asset. Common terms also include: root, ticker, and underlying. This endpoint returns all traded symbols for options. This endpoint is updated overnight.
        """
        ...

    def index_list_symbols_async(
        self,
        *,
        timeout_ms: Optional[int] = None,
    ) -> Awaitable[StringList]:
        """List all available index symbols.

        A symbol can be defined as a unique identifier for a stock / underlying asset. Common terms also include: root, ticker, and underlying. This endpoint returns all traded symbols for options. This endpoint is updated overnight.


        Awaitable companion of the sync variant. The returned object resolves the request off the calling thread so a running event loop keeps servicing other coroutines.
        """
        ...

    def index_list_symbols_builder(self) -> IndexListSymbolsBuilder:
        """Fluent builder for `index_list_symbols`. Chain the optional setters, then call `.list()` (or `.list_async()`) to execute; the returned typed list wrapper exposes `.to_list()` / `.to_arrow()` / `.to_pandas()` / `.to_polars()`."""
        ...

    def index_list_dates(
        self,
        symbol: str,
        *,
        timeout_ms: Optional[int] = None,
    ) -> StringList:
        """List available dates for an index symbol.

        Lists all dates of data that are available for a index with a given request type and symbol. This endpoint is updated overnight.
        """
        ...

    def index_list_dates_async(
        self,
        symbol: str,
        *,
        timeout_ms: Optional[int] = None,
    ) -> Awaitable[StringList]:
        """List available dates for an index symbol.

        Lists all dates of data that are available for a index with a given request type and symbol. This endpoint is updated overnight.


        Awaitable companion of the sync variant. The returned object resolves the request off the calling thread so a running event loop keeps servicing other coroutines.
        """
        ...

    def index_list_dates_builder(
        self,
        symbol: str,
    ) -> IndexListDatesBuilder:
        """Fluent builder for `index_list_dates`. Chain the optional setters, then call `.list()` (or `.list_async()`) to execute; the returned typed list wrapper exposes `.to_list()` / `.to_arrow()` / `.to_pandas()` / `.to_polars()`."""
        ...

    def index_snapshot_ohlc(
        self,
        symbols: Union[str, Sequence[str]],
        *,
        min_time: Optional[Union[str, time, datetime]] = None,
        timeout_ms: Optional[int] = None,
    ) -> List[OhlcTick]:
        """Get the latest OHLC snapshot for one or more indices.

        - Retrieves the real-time current day OHLC.
        - Exchanges typically generate a price report every second for popular indices like SPX.
        """
        ...

    def index_snapshot_ohlc_async(
        self,
        symbols: Union[str, Sequence[str]],
        *,
        min_time: Optional[Union[str, time, datetime]] = None,
        timeout_ms: Optional[int] = None,
    ) -> Awaitable[List[OhlcTick]]:
        """Get the latest OHLC snapshot for one or more indices.

        - Retrieves the real-time current day OHLC.
        - Exchanges typically generate a price report every second for popular indices like SPX.


        Awaitable companion of the sync variant. The returned object resolves the request off the calling thread so a running event loop keeps servicing other coroutines.
        """
        ...

    def index_snapshot_ohlc_builder(
        self,
        symbols: Union[str, Sequence[str]],
    ) -> IndexSnapshotOhlcBuilder:
        """Fluent builder for `index_snapshot_ohlc`. Chain the optional setters, then call `.list()` (or `.list_async()`) to execute; the returned typed list wrapper exposes `.to_list()` / `.to_arrow()` / `.to_pandas()` / `.to_polars()`."""
        ...

    def index_snapshot_price(
        self,
        symbols: Union[str, Sequence[str]],
        *,
        min_time: Optional[Union[str, time, datetime]] = None,
        timeout_ms: Optional[int] = None,
    ) -> List[PriceTick]:
        """Get the latest price snapshot for one or more indices.

        - Retrieves a real-time last index price.
        - Exchanges typically generate a price report every second for popular indices like SPX.
        """
        ...

    def index_snapshot_price_async(
        self,
        symbols: Union[str, Sequence[str]],
        *,
        min_time: Optional[Union[str, time, datetime]] = None,
        timeout_ms: Optional[int] = None,
    ) -> Awaitable[List[PriceTick]]:
        """Get the latest price snapshot for one or more indices.

        - Retrieves a real-time last index price.
        - Exchanges typically generate a price report every second for popular indices like SPX.


        Awaitable companion of the sync variant. The returned object resolves the request off the calling thread so a running event loop keeps servicing other coroutines.
        """
        ...

    def index_snapshot_price_builder(
        self,
        symbols: Union[str, Sequence[str]],
    ) -> IndexSnapshotPriceBuilder:
        """Fluent builder for `index_snapshot_price`. Chain the optional setters, then call `.list()` (or `.list_async()`) to execute; the returned typed list wrapper exposes `.to_list()` / `.to_arrow()` / `.to_pandas()` / `.to_polars()`."""
        ...

    def index_snapshot_market_value(
        self,
        symbols: Union[str, Sequence[str]],
        *,
        min_time: Optional[Union[str, time, datetime]] = None,
        timeout_ms: Optional[int] = None,
    ) -> List[MarketValueTick]:
        """Get the latest market value snapshot for one or more indices.

        - Retrieves a real-time last index market value.
        - Exchanges typically generate a price report every second for popular indices like SPX.
        """
        ...

    def index_snapshot_market_value_async(
        self,
        symbols: Union[str, Sequence[str]],
        *,
        min_time: Optional[Union[str, time, datetime]] = None,
        timeout_ms: Optional[int] = None,
    ) -> Awaitable[List[MarketValueTick]]:
        """Get the latest market value snapshot for one or more indices.

        - Retrieves a real-time last index market value.
        - Exchanges typically generate a price report every second for popular indices like SPX.


        Awaitable companion of the sync variant. The returned object resolves the request off the calling thread so a running event loop keeps servicing other coroutines.
        """
        ...

    def index_snapshot_market_value_builder(
        self,
        symbols: Union[str, Sequence[str]],
    ) -> IndexSnapshotMarketValueBuilder:
        """Fluent builder for `index_snapshot_market_value`. Chain the optional setters, then call `.list()` (or `.list_async()`) to execute; the returned typed list wrapper exposes `.to_list()` / `.to_arrow()` / `.to_pandas()` / `.to_polars()`."""
        ...

    def index_history_eod(
        self,
        symbol: str,
        start_date: Union[str, date, datetime],
        end_date: Union[str, date, datetime],
        *,
        timeout_ms: Optional[int] = None,
    ) -> EodTickList:
        """Fetch end-of-day index data for a date range.

        - Since the indices feeds do not provide a national EOD report, Theta Data generates a national EOD report at 17:15 each day.
        """
        ...

    def index_history_eod_async(
        self,
        symbol: str,
        start_date: Union[str, date, datetime],
        end_date: Union[str, date, datetime],
        *,
        timeout_ms: Optional[int] = None,
    ) -> Awaitable[EodTickList]:
        """Fetch end-of-day index data for a date range.

        - Since the indices feeds do not provide a national EOD report, Theta Data generates a national EOD report at 17:15 each day.


        Awaitable companion of the sync variant. The returned object resolves the request off the calling thread so a running event loop keeps servicing other coroutines.
        """
        ...

    def index_history_eod_builder(
        self,
        symbol: str,
        start_date: Union[str, date, datetime],
        end_date: Union[str, date, datetime],
    ) -> IndexHistoryEodBuilder:
        """Fluent builder for `index_history_eod`. Chain the optional setters, then call `.list()` (or `.list_async()`) to execute; the returned typed list wrapper exposes `.to_list()` / `.to_arrow()` / `.to_pandas()` / `.to_polars()`."""
        ...

    def index_history_ohlc(
        self,
        symbol: str,
        start_date: Union[str, date, datetime],
        end_date: Union[str, date, datetime],
        *,
        interval: Optional[str] = None,
        start_time: Optional[Union[str, time, datetime]] = None,
        end_time: Optional[Union[str, time, datetime]] = None,
        timeout_ms: Optional[int] = None,
    ) -> OhlcTickList:
        """Fetch intraday OHLC bars for an index.

        - Aggregated OHLC bars that use SIP rules for each bar.
        - Time timestamp of the bar represents the opening time of the bar. For a trade to be part of the bar:  ``bar timestamp`` <= ``trade time`` < ``bar timestamp + interval``.
        - Exchanges typically generate a price report every second for popular indices like SPX.

        Defaults (upstream):
        - `interval`: `"1s"`
        - `start_time`: `"09:30:00"`
        - `end_time`: `"16:00:00"`
        """
        ...

    def index_history_ohlc_async(
        self,
        symbol: str,
        start_date: Union[str, date, datetime],
        end_date: Union[str, date, datetime],
        *,
        interval: Optional[str] = None,
        start_time: Optional[Union[str, time, datetime]] = None,
        end_time: Optional[Union[str, time, datetime]] = None,
        timeout_ms: Optional[int] = None,
    ) -> Awaitable[OhlcTickList]:
        """Fetch intraday OHLC bars for an index.

        - Aggregated OHLC bars that use SIP rules for each bar.
        - Time timestamp of the bar represents the opening time of the bar. For a trade to be part of the bar:  ``bar timestamp`` <= ``trade time`` < ``bar timestamp + interval``.
        - Exchanges typically generate a price report every second for popular indices like SPX.

        Defaults (upstream):
        - `interval`: `"1s"`
        - `start_time`: `"09:30:00"`
        - `end_time`: `"16:00:00"`


        Awaitable companion of the sync variant. The returned object resolves the request off the calling thread so a running event loop keeps servicing other coroutines.
        """
        ...

    def index_history_ohlc_builder(
        self,
        symbol: str,
        start_date: Union[str, date, datetime],
        end_date: Union[str, date, datetime],
    ) -> IndexHistoryOhlcBuilder:
        """Fluent builder for `index_history_ohlc`. Chain the optional setters, then call `.list()` (or `.list_async()`) to execute; the returned typed list wrapper exposes `.to_list()` / `.to_arrow()` / `.to_pandas()` / `.to_polars()`."""
        ...

    def index_history_price(
        self,
        symbol: str,
        date: Union[str, date, datetime],
        *,
        interval: Optional[str] = None,
        start_time: Optional[Union[str, time, datetime]] = None,
        end_time: Optional[Union[str, time, datetime]] = None,
        start_date: Optional[Union[str, date, datetime]] = None,
        end_date: Optional[Union[str, date, datetime]] = None,
        timeout_ms: Optional[int] = None,
    ) -> PriceTickList:
        """Fetch intraday price history for an index.

        - Retrieves historical indices price reports. Exchanges typically generate a price report every second for popular indices like SPX.
        - When the ``interval`` parameter is specified, the returned data represents the price at the exact time of each timestamp. If the timestamp in the response is 10:30:00, the price field represents the price at that exact time of the day.
        - A price update from the exchange is omitted if the price remained the same from the previous update.
        - Multi-day requests are limited to 1 month of data.

        Defaults (upstream):
        - `interval`: `"1s"`
        - `start_time`: `"09:30:00"`
        - `end_time`: `"16:00:00"`
        """
        ...

    def index_history_price_async(
        self,
        symbol: str,
        date: Union[str, date, datetime],
        *,
        interval: Optional[str] = None,
        start_time: Optional[Union[str, time, datetime]] = None,
        end_time: Optional[Union[str, time, datetime]] = None,
        start_date: Optional[Union[str, date, datetime]] = None,
        end_date: Optional[Union[str, date, datetime]] = None,
        timeout_ms: Optional[int] = None,
    ) -> Awaitable[PriceTickList]:
        """Fetch intraday price history for an index.

        - Retrieves historical indices price reports. Exchanges typically generate a price report every second for popular indices like SPX.
        - When the ``interval`` parameter is specified, the returned data represents the price at the exact time of each timestamp. If the timestamp in the response is 10:30:00, the price field represents the price at that exact time of the day.
        - A price update from the exchange is omitted if the price remained the same from the previous update.
        - Multi-day requests are limited to 1 month of data.

        Defaults (upstream):
        - `interval`: `"1s"`
        - `start_time`: `"09:30:00"`
        - `end_time`: `"16:00:00"`


        Awaitable companion of the sync variant. The returned object resolves the request off the calling thread so a running event loop keeps servicing other coroutines.
        """
        ...

    def index_history_price_builder(
        self,
        symbol: str,
        date: Union[str, date, datetime],
    ) -> IndexHistoryPriceBuilder:
        """Fluent builder for `index_history_price`. Chain the optional setters, then call `.list()` (or `.list_async()`) to execute; the returned typed list wrapper exposes `.to_list()` / `.to_arrow()` / `.to_pandas()` / `.to_polars()`."""
        ...

    def index_at_time_price(
        self,
        symbol: str,
        start_date: Union[str, date, datetime],
        end_date: Union[str, date, datetime],
        time_of_day: Union[str, time, datetime],
        *,
        timeout_ms: Optional[int] = None,
    ) -> IndexPriceAtTimeTickList:
        """Fetch the index price at a specific time of day across a date range.

        - Retrieves historical indices price reports. Exchanges typically generate a price report every second for popular indices like SPX.
        - The ``time_of_day`` parameter represents the 00:00:00.000 ET that the price should be provided for.
        """
        ...

    def index_at_time_price_async(
        self,
        symbol: str,
        start_date: Union[str, date, datetime],
        end_date: Union[str, date, datetime],
        time_of_day: Union[str, time, datetime],
        *,
        timeout_ms: Optional[int] = None,
    ) -> Awaitable[IndexPriceAtTimeTickList]:
        """Fetch the index price at a specific time of day across a date range.

        - Retrieves historical indices price reports. Exchanges typically generate a price report every second for popular indices like SPX.
        - The ``time_of_day`` parameter represents the 00:00:00.000 ET that the price should be provided for.


        Awaitable companion of the sync variant. The returned object resolves the request off the calling thread so a running event loop keeps servicing other coroutines.
        """
        ...

    def index_at_time_price_builder(
        self,
        symbol: str,
        start_date: Union[str, date, datetime],
        end_date: Union[str, date, datetime],
        time_of_day: Union[str, time, datetime],
    ) -> IndexAtTimePriceBuilder:
        """Fluent builder for `index_at_time_price`. Chain the optional setters, then call `.list()` (or `.list_async()`) to execute; the returned typed list wrapper exposes `.to_list()` / `.to_arrow()` / `.to_pandas()` / `.to_polars()`."""
        ...

    def calendar_open_today(
        self,
        *,
        timeout_ms: Optional[int] = None,
    ) -> List[CalendarDay]:
        """Check whether the market is open today.

        - Retrieves current day equity market schedule
        - *On days when the market closes early at 1:00 PM ET; eligible options will trade until 1:15 PM.
        - **Some NYSE exchanges will continue late trading until 5:00 PM ET on early close days.
        """
        ...

    def calendar_open_today_async(
        self,
        *,
        timeout_ms: Optional[int] = None,
    ) -> Awaitable[List[CalendarDay]]:
        """Check whether the market is open today.

        - Retrieves current day equity market schedule
        - *On days when the market closes early at 1:00 PM ET; eligible options will trade until 1:15 PM.
        - **Some NYSE exchanges will continue late trading until 5:00 PM ET on early close days.


        Awaitable companion of the sync variant. The returned object resolves the request off the calling thread so a running event loop keeps servicing other coroutines.
        """
        ...

    def calendar_open_today_builder(self) -> CalendarOpenTodayBuilder:
        """Fluent builder for `calendar_open_today`. Chain the optional setters, then call `.list()` (or `.list_async()`) to execute; the returned typed list wrapper exposes `.to_list()` / `.to_arrow()` / `.to_pandas()` / `.to_polars()`."""
        ...

    def calendar_on_date(
        self,
        date: Union[str, date, datetime],
        *,
        timeout_ms: Optional[int] = None,
    ) -> List[CalendarDay]:
        """Get calendar information for a specific date.

        - Retrieves equity market schedule for a given date
        - Note: Holiday data is available 01/01/2012 through the end of the calendar year that immediately follows the current year
        - *On days when the market closes early at 1:00 PM ET; eligible options will trade until 1:15 PM.
        - **Some NYSE exchanges will continue late trading until 5:00 PM ET on early close days.
        """
        ...

    def calendar_on_date_async(
        self,
        date: Union[str, date, datetime],
        *,
        timeout_ms: Optional[int] = None,
    ) -> Awaitable[List[CalendarDay]]:
        """Get calendar information for a specific date.

        - Retrieves equity market schedule for a given date
        - Note: Holiday data is available 01/01/2012 through the end of the calendar year that immediately follows the current year
        - *On days when the market closes early at 1:00 PM ET; eligible options will trade until 1:15 PM.
        - **Some NYSE exchanges will continue late trading until 5:00 PM ET on early close days.


        Awaitable companion of the sync variant. The returned object resolves the request off the calling thread so a running event loop keeps servicing other coroutines.
        """
        ...

    def calendar_on_date_builder(
        self,
        date: Union[str, date, datetime],
    ) -> CalendarOnDateBuilder:
        """Fluent builder for `calendar_on_date`. Chain the optional setters, then call `.list()` (or `.list_async()`) to execute; the returned typed list wrapper exposes `.to_list()` / `.to_arrow()` / `.to_pandas()` / `.to_polars()`."""
        ...

    def calendar_year(
        self,
        year: str,
        *,
        timeout_ms: Optional[int] = None,
    ) -> List[CalendarDay]:
        """Get equity market holidays and early-close days for a year (vendor `year_holidays` endpoint — only non-standard days, not every trading day).

        - Retrieves equity market holidays for a given year
        - Note: Holiday data is available 01/01/2012 through the end of the calendar year that immediately follows the current year
        - *On days when the market closes early at 1:00 PM ET; eligible options will trade until 1:15 PM.
        - **Some NYSE exchanges will continue late trading until 5:00 PM ET on early close days.
        """
        ...

    def calendar_year_async(
        self,
        year: str,
        *,
        timeout_ms: Optional[int] = None,
    ) -> Awaitable[List[CalendarDay]]:
        """Get equity market holidays and early-close days for a year (vendor `year_holidays` endpoint — only non-standard days, not every trading day).

        - Retrieves equity market holidays for a given year
        - Note: Holiday data is available 01/01/2012 through the end of the calendar year that immediately follows the current year
        - *On days when the market closes early at 1:00 PM ET; eligible options will trade until 1:15 PM.
        - **Some NYSE exchanges will continue late trading until 5:00 PM ET on early close days.


        Awaitable companion of the sync variant. The returned object resolves the request off the calling thread so a running event loop keeps servicing other coroutines.
        """
        ...

    def calendar_year_builder(
        self,
        year: str,
    ) -> CalendarYearBuilder:
        """Fluent builder for `calendar_year`. Chain the optional setters, then call `.list()` (or `.list_async()`) to execute; the returned typed list wrapper exposes `.to_list()` / `.to_arrow()` / `.to_pandas()` / `.to_polars()`."""
        ...

    def interest_rate_history_eod(
        self,
        symbol: str,
        start_date: Union[str, date, datetime],
        end_date: Union[str, date, datetime],
        *,
        timeout_ms: Optional[int] = None,
    ) -> InterestRateTickList:
        """Fetch end-of-day interest rate history.

        - Returns the interest rate reported. Depending on the rate, reports can occur in the morning or the afternoon.
        - Valid `symbol` values per upstream `RateType` enum:
          `SOFR`, `TREASURY_M1`, `TREASURY_M3`, `TREASURY_M6`,
          `TREASURY_Y1`, `TREASURY_Y2`, `TREASURY_Y3`, `TREASURY_Y5`,
          `TREASURY_Y7`, `TREASURY_Y10`, `TREASURY_Y20`, `TREASURY_Y30`.
        """
        ...

    def interest_rate_history_eod_async(
        self,
        symbol: str,
        start_date: Union[str, date, datetime],
        end_date: Union[str, date, datetime],
        *,
        timeout_ms: Optional[int] = None,
    ) -> Awaitable[InterestRateTickList]:
        """Fetch end-of-day interest rate history.

        - Returns the interest rate reported. Depending on the rate, reports can occur in the morning or the afternoon.
        - Valid `symbol` values per upstream `RateType` enum:
          `SOFR`, `TREASURY_M1`, `TREASURY_M3`, `TREASURY_M6`,
          `TREASURY_Y1`, `TREASURY_Y2`, `TREASURY_Y3`, `TREASURY_Y5`,
          `TREASURY_Y7`, `TREASURY_Y10`, `TREASURY_Y20`, `TREASURY_Y30`.


        Awaitable companion of the sync variant. The returned object resolves the request off the calling thread so a running event loop keeps servicing other coroutines.
        """
        ...

    def interest_rate_history_eod_builder(
        self,
        symbol: str,
        start_date: Union[str, date, datetime],
        end_date: Union[str, date, datetime],
    ) -> InterestRateHistoryEodBuilder:
        """Fluent builder for `interest_rate_history_eod`. Chain the optional setters, then call `.list()` (or `.list_async()`) to execute; the returned typed list wrapper exposes `.to_list()` / `.to_arrow()` / `.to_pandas()` / `.to_polars()`."""
        ...

    def stock_history_ohlc_range(
        self,
        symbol: str,
        start_date: Union[str, date, datetime],
        end_date: Union[str, date, datetime],
        *,
        interval: Optional[str] = None,
        start_time: Optional[Union[str, time, datetime]] = None,
        end_time: Optional[Union[str, time, datetime]] = None,
        venue: Optional[str] = None,
        timeout_ms: Optional[int] = None,
    ) -> OhlcTickList:
        """Fetch intraday OHLC bars across a date range (start_date..end_date). This is a dedicated upstream route, distinct from the single-date stock_history_ohlc; the `_range` suffix mirrors the vendor's separate `ohlc_range` route.

        Defaults (upstream):
        - `interval`: `"1s"`
        - `start_time`: `"09:30:00"`
        - `end_time`: `"16:00:00"`
        - `venue`: `"nqb"`
        """
        ...

    def stock_history_ohlc_range_async(
        self,
        symbol: str,
        start_date: Union[str, date, datetime],
        end_date: Union[str, date, datetime],
        *,
        interval: Optional[str] = None,
        start_time: Optional[Union[str, time, datetime]] = None,
        end_time: Optional[Union[str, time, datetime]] = None,
        venue: Optional[str] = None,
        timeout_ms: Optional[int] = None,
    ) -> Awaitable[OhlcTickList]:
        """Fetch intraday OHLC bars across a date range (start_date..end_date). This is a dedicated upstream route, distinct from the single-date stock_history_ohlc; the `_range` suffix mirrors the vendor's separate `ohlc_range` route.

        Defaults (upstream):
        - `interval`: `"1s"`
        - `start_time`: `"09:30:00"`
        - `end_time`: `"16:00:00"`
        - `venue`: `"nqb"`


        Awaitable companion of the sync variant. The returned object resolves the request off the calling thread so a running event loop keeps servicing other coroutines.
        """
        ...

    def stock_history_ohlc_range_builder(
        self,
        symbol: str,
        start_date: Union[str, date, datetime],
        end_date: Union[str, date, datetime],
    ) -> StockHistoryOhlcRangeBuilder:
        """Fluent builder for `stock_history_ohlc_range`. Chain the optional setters, then call `.list()` (or `.list_async()`) to execute; the returned typed list wrapper exposes `.to_list()` / `.to_arrow()` / `.to_pandas()` / `.to_polars()`."""
        ...

# --- END GENERATED HISTORICAL VIEW ---


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

    def is_authenticated(self) -> bool:
        """Return whether the live streaming session is authenticated.

        Distinct from :meth:`is_streaming`: the session can be live yet
        briefly unauthenticated mid-reconnect. ``False`` before streaming
        starts and after it stops.
        """
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

    def set_slow_callback_threshold_us(self, threshold_us: int) -> None:
        """Set the slow-callback wall-clock threshold in microseconds.

        When a callback invocation runs longer than ``threshold_us``,
        :meth:`slow_callback_count` increments and a rate-limited
        warning is logged. Pass ``0`` to disable the watchdog (the
        default). Observability only: the watchdog never cancels the
        callback. No-op when streaming has not started.
        """
        ...

    def slow_callback_count(self) -> int:
        """Cumulative count of user-callback invocations whose
        wall-clock duration exceeded the threshold set by
        :meth:`set_slow_callback_threshold_us`. 0 when the watchdog is
        disabled or streaming has not started.
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
    def flatfile_to_path_async(
        self,
        sec_type: str,
        req_type: str,
        date: str,
        path: str,
        format: Optional[str] = None,
    ) -> Awaitable[str]:
        """Awaitable twin of :py:meth:`flatfile_to_path`.

        Resolves the blob download off the calling thread so a running
        event loop keeps servicing other coroutines while the file streams
        to disk. Yields the final on-disk path.
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

        Runs the authentication and connection handshake to completion
        before returning. Use this when constructing outside a running
        event loop. Inside a coroutine, prefer
        :meth:`connect` so the handshake does not stall the event loop.

        Args:
            creds: Account credentials.
            config: Connection configuration.

        Raises:
            ThetaDataError: If authentication or the connection fails.
        """
        ...

    @staticmethod
    def connect(
        creds: Credentials,
        config: Config,
    ) -> Awaitable[AsyncClient]:
        """Connect without blocking the running event loop.

        The authentication and connection handshake resolves off the
        event loop, so other coroutines keep running while the connection
        is established. This is the preferred way to build an
        :class:`AsyncClient` from inside a coroutine::

            client = await AsyncClient.connect(creds, config)

        Args:
            creds: Account credentials.
            config: Connection configuration.

        Returns:
            An awaitable resolving to a connected :class:`AsyncClient`.

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

    @staticmethod
    def connect_from_file(
        path: str,
        config: Optional[Config] = None,
    ) -> Awaitable[AsyncClient]:
        """Connect from a credentials file without blocking the event loop.

        Loads credentials from a two-line file and connects off the event
        loop, defaulting to ``Config.production()`` when no ``config`` is
        supplied::

            client = await AsyncClient.connect_from_file("creds.txt")

        Args:
            path: Path to a two-line credentials file.
            config: Connection configuration; defaults to
                ``Config.production()`` when omitted.

        Returns:
            An awaitable resolving to a connected :class:`AsyncClient`.

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

    def set_slow_callback_threshold_us(self, threshold_us: int) -> None:
        """Set the slow-callback wall-clock threshold in microseconds.

        When a callback invocation runs longer than ``threshold_us``,
        :meth:`slow_callback_count` increments and a rate-limited
        warning is logged. Pass ``0`` to disable the watchdog (the
        default). Observability only: the watchdog never cancels the
        callback. No-op when no session is live.
        """
        ...

    def slow_callback_count(self) -> int:
        """Cumulative count of user-callback invocations whose
        wall-clock duration exceeded the threshold set by
        :meth:`set_slow_callback_threshold_us`. 0 when the watchdog is
        disabled or no session is live.
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

    def __bool__(self) -> bool:
        """Return whether the list holds at least one row."""
        ...

    def __repr__(self) -> str:
        """Return a representation (e.g. ``"FlatFileRowList(128 rows)"``)."""
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

    Every fetch carries an ``*_async`` twin returning an awaitable. A
    flat-file pull is a full-day blob download that takes seconds; the
    plain methods run that to completion on the calling thread, which is
    right for a :class:`Client` call but would stall a running event loop
    when reached through :py:attr:`AsyncClient.flat_files`. Inside a
    coroutine, ``await flat_files.option_eod_async(date)`` resolves the
    download without blocking the loop and yields the same
    :class:`FlatFileRowList`.
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

    def option_trade_quote_async(self, date: str) -> Awaitable[FlatFileRowList]:
        """Awaitable option-trade-quote flat file for ``date`` (``YYYYMMDD``)."""
        ...

    def option_open_interest_async(self, date: str) -> Awaitable[FlatFileRowList]:
        """Awaitable option-open-interest flat file for ``date`` (``YYYYMMDD``)."""
        ...

    def option_eod_async(self, date: str) -> Awaitable[FlatFileRowList]:
        """Awaitable option-EOD flat file for ``date`` (``YYYYMMDD``)."""
        ...

    def stock_trade_quote_async(self, date: str) -> Awaitable[FlatFileRowList]:
        """Awaitable stock-trade-quote flat file for ``date`` (``YYYYMMDD``)."""
        ...

    def stock_eod_async(self, date: str) -> Awaitable[FlatFileRowList]:
        """Awaitable stock-EOD flat file for ``date`` (``YYYYMMDD``)."""
        ...

    def request_async(
        self, sec_type: str, req_type: str, date: str
    ) -> Awaitable[FlatFileRowList]:
        """Awaitable twin of :py:meth:`request`, resolved off the event loop."""
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
