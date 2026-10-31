"""ThetaData market-data SDK.

`thetadatadx` provides direct access to ThetaData's real-time and
market-data market data without a separate terminal process. The package
exposes:

- Connection types: :class:`Credentials` and :class:`Config`.
- Clients: :class:`Client` (market-data plus on-demand
  streaming), :class:`AsyncClient` (``await``-based
  market-data), :class:`StreamingClient` (streaming only), and
  :class:`MarketDataClient` (market-data only).
- A fluent subscription surface: :class:`Contract`,
  :class:`Subscription`, and :class:`SecType`.
- Real-time event types delivered to the streaming callback
  (:class:`Quote`, :class:`Trade`, :class:`OpenInterest`,
  :class:`Ohlcvc`, and the connection / lifecycle events).
- A typed exception hierarchy rooted at :class:`ThetaDataError`.
- Utility entry points such as :func:`split_date_range`.

Type checkers discover these annotations through the ``py.typed`` marker
(PEP 561) shipped alongside this file.
"""

# The event classes, the `Config` properties, the tick classes, the
# market-data methods and their builders are generated from the tables the
# runtime is built from.

from __future__ import annotations

from datetime import date, datetime, time
# A builder's `date` setter shadows `date` in its class, so the builders'
# setters name the type `_date`.
from datetime import date as _date
from os import PathLike
from typing import (
    Any,
    Awaitable,
    Callable,
    Generic,
    Iterator,
    List,
    Literal,
    Optional,
    Self,
    Sequence,
    Tuple,
    Type,
    TypeVar,
    Union,
    final,
    overload,
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
    def from_env() -> Credentials:
        """Source credentials strictly from the ``THETADATA_API_KEY`` env var.

        Strict: an unset or whitespace-only value raises rather than
        falling back, and there is no ``creds.txt`` file fallback. Use
        :meth:`from_env_or_file` when a file fallback is wanted instead.

        Returns:
            The sourced :class:`Credentials`.

        Raises:
            InvalidParameterError: If ``THETADATA_API_KEY`` is unset or empty.
        """
        ...

    @staticmethod
    def from_env_or_file(path: str) -> Credentials:
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

    @staticmethod
    def from_dotenv(path: str) -> Credentials:
        """Source credentials from a ``.env``-format file.

        The file uses the common ``.env`` grammar (one ``KEY=VALUE`` per
        line, optional ``export`` prefix, ``#`` comments, optional
        quotes). ``THETADATA_API_KEY`` selects an API key; otherwise
        ``THETADATA_EMAIL`` + ``THETADATA_PASSWORD`` build email +
        password credentials.

        Args:
            path: Path to the ``.env`` file to read.

        Returns:
            The sourced :class:`Credentials`.

        Raises:
            ThetaDataError: If the file cannot be read or defines none of
                the recognized keys.
        """
        ...

    def __repr__(self) -> str:
        """Return a representation with the email redacted."""
        ...


@final
class Config:
    """Connection configuration: market-data host / streaming hosts / reconnect policy."""

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
        """Return the market-data-staging configuration (market-data staging cluster + auth marker; streaming stays on production). Testing, unstable."""
        ...

    @staticmethod
    def from_dotenv(path: str) -> Config:
        """Source the target environment from a ``.env``-format file.

        ``THETADATA_MARKET_DATA_TYPE`` (``PROD`` / ``STAGE``) selects the
        environment; ``THETADATA_MARKET_DATA_HOST`` /
        ``THETADATA_STREAMING_HOST`` override the hosts. Reads the same
        file and keys as :meth:`Credentials.from_dotenv`, so one ``.env``
        can carry both ``THETADATA_API_KEY`` and ``THETADATA_MARKET_DATA_TYPE``.
        """
        ...

    # --- BEGIN GENERATED CONFIG PROPERTIES (config table) ---
    # Generated from the config table; do not edit by hand.
    reconnect_wait_ms: int
    """Set the reconnect delay (ms) honoured for generic transient
    disconnects (TimedOut, ServerRestarting, Unspecified, …).
    Applied to the streaming session at connect time.
    Default ``250``.
    """
    reconnect_wait_rate_limited_ms: int
    """Set the reconnect delay (ms) honoured for ``TooManyRequests``
    rate-limited disconnects. Default ``130_000`` (matches the
    JVM terminal's 130 s rate-limit cooldown).
    """
    reconnect_wait_max_ms: int
    """Set the cap (ms) on the exponential generic-transient reconnect
    ladder. The ladder starts at ``reconnect_wait_ms`` and doubles
    per consecutive attempt up to this value. Default ``30_000``.
    """
    reconnect_wait_server_restart_ms: int
    """Set the flat reconnect cadence (ms) for ``ServerRestarting``
    disconnects. Default ``5_000``.
    """
    reconnect_replay_burst_size: int
    """Set the subscription-replay burst size used after an
    auto-reconnect: frames are written in bursts of this many, each
    burst flushed and followed by a jittered ``replay_pace_ms``
    pause. Minimum ``1`` (validated at connect). Default ``50``.
    """
    reconnect_replay_pace_ms: int
    """Set the pause (ms) between subscription-replay bursts after an
    auto-reconnect. ``0`` removes the pause. Default ``5``.
    """
    streaming_timeout_ms: int
    """Set the streaming read timeout (ms): the no-frames deadline after
    which the streaming session is declared dead and reconnects.
    Default ``10_000``; validated to ``[100, 60_000]``.
    """
    streaming_connect_timeout_ms: int
    """Set the per-server connect timeout (ms) for the streaming
    connection. Default
    ``2_000``; validated to ``[1_000, 60_000]``.
    """
    streaming_ping_interval_ms: int
    """Set the streaming heartbeat ping interval (ms). Default ``100``;
    validated to ``[100, 300_000]``.
    """
    streaming_io_read_slice_ms: int
    """Set the per-iteration blocking-read slice (ms) for the
    streaming I/O loop. Default ``25``; validated to ``[10, 500]``.
    """
    streaming_keepalive_idle_secs: int
    """Set the TCP keepalive idle time (seconds) before the first
    kernel probe on a silent streaming socket. Default ``5``;
    validated to ``[1, 7_200]``.
    """
    streaming_keepalive_interval_secs: int
    """Set the interval (seconds) between TCP keepalive probes.
    Default ``2``; validated to ``[1, 75]``.
    """
    streaming_keepalive_retries: int
    """Set the number of unanswered TCP keepalive probes after which
    the kernel declares the streaming connection dead (where the
    platform exposes the knob). Default ``2``; validated to
    ``[1, 10]``.
    """
    wait_mode: Literal["spin", "busyspin", "park", "backoff"]
    """Set how the streaming consumer waits when the event ring is empty.
    Accepts ``"spin"`` (default), ``"busyspin"``, ``"park"``, or
    ``"backoff"`` (case-insensitive). ``"spin"`` and ``"busyspin"`` both hold
    ~100% of one core and differ only in jitter; only ``"park"`` and
    ``"backoff"`` lower idle CPU. The park / backoff sleep length is
    ``park_interval_us``.
    """
    park_interval_us: int
    """Set the park / backoff idle sleep length in microseconds for the streaming
    consumer. Used only when the wait mode is ``"park"`` or ``"backoff"``.
    Validated ``[50, 1000000]`` at connect time. Default ``1000`` (= 1 ms).
    """
    retry_max_elapsed_secs: int
    """Set the wall-clock envelope (seconds) for one
    market-data-channel retry sequence, measured from the first
    attempt. ``0`` disables the envelope (attempt budget only).
    Default ``300``.
    """
    retry_initial_delay_ms: int
    """Set the initial backoff delay (ms) for the market-data-channel retry policy.
    Default ``250``. Subsequent retries double from here, capped
    at :attr:`retry_max_delay_ms`.
    """
    retry_max_delay_ms: int
    """Set the upper-bound backoff delay (ms) for the
    market-data-channel retry policy. Default ``30_000`` (30 s).
    """
    retry_max_attempts: int
    """Set the total attempt budget for the market-data-channel retry policy. ``1``
    disables retry (single call only); higher values permit retries
    up to ``max_attempts - 1`` after the initial call. Default ``20``.
    """
    retry_jitter: bool
    """Toggle AWS-style full-jitter on the market-data-channel retry policy. Default
    ``True``. ``False`` gives the deterministic backoff schedule
    ``min(max_delay, initial * 2^attempt)``, useful for tests that
    need to assert exact timings.
    """
    market_data_port: int
    """Set the market-data gRPC port. The environment factories set it;
    override it together with ``market_data_host`` to reach the
    market-data service at another address.
    """
    warn_on_buffered_threshold_bytes: int
    """Set the warning threshold (in bytes) for buffered (non-streaming)
    historical responses. Endpoints whose decoded total exceeds this
    value log a warning pointing the caller at the
    ``.stream()`` surface; the data is still delivered. ``0``
    disables the warning entirely. Default is ``100 * 1024 * 1024``
    (100 MiB).

    Examples
    --------
        cfg = Config.production()
        cfg.warn_on_buffered_threshold_bytes = 50 * 1024 * 1024
    """
    request_timeout_secs: int
    """Set the default per-request deadline (in seconds) for market-data
    queries. Bounds every request that did not set its own
    ``timeout_ms``, so a live-but-silent stream resolves to a
    timeout instead of blocking forever. ``0`` no longer disables the
    default: it is floored to the ``300``-second default at request time,
    so the deadline always applies. Default is ``300`` (5 minutes).

    Examples
    --------
        cfg = Config.production()
        cfg.request_timeout_secs = 120
    """
    market_data_stream_window_size_kb: int
    """Set the initial per-stream HTTP/2 flow-control window (in KB) for the
    market-data gRPC channel. A larger window raises the throughput ceiling
    on bulk streaming pulls before HTTP/2 backpressure kicks in. The value
    is clamped into ``[64, 2_097_151]`` KB at validate/connect time.
    Default is ``8192`` (8 MiB).

    Examples
    --------
        cfg = Config.production()
        cfg.market_data_stream_window_size_kb = 4096
    """
    market_data_connection_window_size_kb: int
    """Set the initial connection-level HTTP/2 flow-control window (in KB) for
    the market-data gRPC channel. A larger window raises the throughput
    ceiling on bulk streaming pulls before HTTP/2 backpressure kicks in.
    The value is clamped into ``[64, 2_097_151]`` KB at validate/connect
    time. Default is ``16384`` (16 MiB).

    Examples
    --------
        cfg = Config.production()
        cfg.market_data_connection_window_size_kb = 16384
    """
    bulk_fetch: Literal["auto", "off"]
    """Automatic bulk-fetch sharding policy for history pulls (buffered and
    chunk-streaming alike). Accepts ``"auto"`` (default: a large history
    pull's requested time or date range is split into equal concurrent
    bands across the account's concurrent-request budget; buffered
    pulls are merged back
    into exactly the rows of the single-stream response, single-contract,
    stock, and index pulls in the exact single-stream order and
    option-chain pulls in a deterministic canonical order grouped by
    expiration, strike, and right; streaming pulls forward each band's
    chunks as they arrive, every chunk exactly once but interleaved
    across bands in arrival order; small pulls are never sharded) or
    ``"off"`` (every query runs as a single stream, in the server's own
    row and chunk order), case-insensitive.
    """
    shard_concurrency: Optional[int]
    """Upper bound on concurrent sub-requests per sharded bulk fetch.
    ``None`` (the default) uses the full concurrent-request budget (the
    channel-pool size resolved from ``max_concurrent_requests``); an
    ``int`` caps the fan-out. The applied value is clamped into
    ``[1, pool_size]`` when a plan is built, and validation floors an
    explicit ``0`` to ``1``.
    """
    max_concurrent_requests: Optional[int]
    """Number of concurrent in-flight market-data requests (gRPC channel-pool
    + request-semaphore size, resolved at connect time). ``None`` (the
    default) sizes the pool to the account's subscription tier from the
    auth response (Free 1 / Value 2 / Standard 4 / Pro 8); an ``int`` is
    used verbatim with no client-side cap; the allowance is enforced
    server-side, so a boosted account sets its boosted value. Requests
    past the allowance are retried with backoff. Validation floors an
    explicit ``0`` to ``1``.

    Examples
    --------
        cfg = Config.production()
        cfg.max_concurrent_requests = 32
    """
    reconnect_max_attempts: int
    """Set the per-class transient-failure attempt budget for the
    auto-reconnect path. Default `30`. No effect unless the
    reconnect policy is `Auto`.
    """
    reconnect_max_rate_limited_attempts: int
    """Set the per-class rate-limited (`TooManyRequests`) attempt budget
    for the auto-reconnect path. Default `100`. No effect unless the
    reconnect policy is `Auto`.
    """
    reconnect_max_server_restart_attempts: int
    """Set the ``ServerRestarting`` reconnect attempt budget. Default
    ``60``. No effect unless the reconnect policy is ``Auto``.
    """
    reconnect_max_elapsed_secs: int
    """Set the wall-clock reconnect envelope (seconds) for the
    generic-transient and server-restart classes, measured from the
    first attempt of a consecutive-reconnect sequence. ``0``
    disables the envelope (attempt budgets only). Default ``300``.
    No effect unless the reconnect policy is ``Auto``.
    """
    reconnect_stable_window_secs: int
    """Set the continuous successful-data-flow window (in seconds)
    after which the auto-reconnect attempt counters reset. Default
    ``60``. No effect unless the reconnect policy is ``Auto``.
    """
    nexus_url: str
    """Set the Nexus auth URL. Default matches the upstream production
    endpoint; override to redirect at a staging cluster.
    """
    client_type: str
    """Set the ``QueryInfo.client_type`` identifier. Default is
    ``"rust-thetadatadx"``; override to identify a deployment fleet
    in server-side dashboards.
    """
    market_data_host: str
    """Set the market-data gRPC host. The environment factories set it;
    override it to reach the market-data service at another address.
    """
    reconnect_jitter: Literal["full", "none"]
    """Set the jitter strategy applied to every reconnect delay.
    Accepts ``"full"`` (default) or ``"none"`` (case-insensitive).
    """
    # --- END GENERATED CONFIG PROPERTIES ---
    reconnect_policy: str
    """Active reconnect policy name: ``"auto"``, ``"manual"``, or ``"custom"`` (the last reported when a :attr:`reconnect_callback` is installed)."""
    reconnect_callback: Optional[Callable[[int, int], Optional[int]]]
    """Custom reconnect policy: a callable ``(reason: int, attempt: int) -> Optional[int]`` returning the reconnect delay in milliseconds, or ``None`` to stop (the stream then emits the terminal :class:`ReconnectsExhausted` event). Runs on an SDK streaming thread; permanent disconnect reasons never reach it. Assign ``None`` to restore the auto policy. Write-only: the configured callable cannot be read back (:attr:`reconnect_policy` then reports ``"custom"``)."""
    worker_threads: Optional[int]
    """Async worker-thread count for the embedded runtime. ``None`` defers to the default sizing; an ``int`` (including ``0``, which clamps to ``1``) pins the worker count."""
    streaming_ring_size: int
    """Capacity, in slots, of the streaming event ring; must be a power of two from 64 to ``2**24``, otherwise connecting raises."""
    @property
    def market_data_environment(self) -> Literal["PROD", "STAGE"]:
        """Target market-data environment carried by this configuration: ``"PROD"`` for the production cluster or ``"STAGE"`` for staging. The market-data and streaming channels are selected independently; :meth:`Config.production` / :meth:`Config.stage` (and the ``THETADATA_MARKET_DATA_TYPE`` key on :meth:`Config.from_dotenv`) set the market-data channel, and this is the readback of that selection. Read-only: the selector is chosen by the environment-tier factories, not assigned directly. Mirrors the ``market_data_type`` string the inline :class:`Client` constructor accepts."""
    @property
    def streaming_environment(self) -> Literal["PROD", "STAGE", "DEV"]:
        """Target streaming environment carried by this configuration: ``"PROD"`` for the production cluster, ``"STAGE"`` for the staging cluster or ``"DEV"`` for the dev cluster. The streaming and market-data channels are selected independently; :meth:`Config.production` / :meth:`Config.dev` (and the ``THETADATA_STREAMING_TYPE`` key on :meth:`Config.from_dotenv`) set the streaming channel, and this is the readback of that selection. Read-only: the selector is chosen by the environment-tier factories, not assigned directly. Mirrors the ``streaming_type`` string the inline :class:`Client` constructor accepts."""
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
        right: str | Right,
    ) -> Contract:
        """Construct an option contract.

        Args:
            symbol: Underlying root symbol.
            expiration: Expiration date as a ``YYYYMMDD`` string.
            strike: Strike price in dollars; a number or string is
                accepted (``550``, ``550.0``, and ``"550"`` are equivalent).
            right: Option right — ``"C"`` (call) or ``"P"`` (put), or a
                :class:`Right`.

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
    """Aggregation interval for bar / OHLC market-data requests."""

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

# --- BEGIN GENERATED STREAMING EVENTS (event table) ---
# Generated from the streaming event table; do not edit by hand. Run
# `cargo run -p xtask --bin generate_sdk_surfaces` to refresh.


@final
class IndexMarketValue:
    """Streaming index MarketValue tick (wire code 25, index contracts): `ms_of_day`, `date` and `market_price` as the feed sent them; an index has no bid/ask. Per-contract only (no full-stream variant)."""

    contract: ContractRef
    """Contract this event refers to."""
    ms_of_day: int
    """Milliseconds since midnight Eastern Time when the event was recorded."""
    market_price: float
    """Index market value (dollars): the vendor's indicative figure, the index level moved by a random 1 to 5 cents. The exact level is the index price."""
    date: int
    """Trading date as `YYYYMMDD`."""
    received_at_ns: int
    """Wall-clock nanoseconds since UNIX epoch, captured at frame decode time."""

    @property
    def kind(self) -> str:
        """Event kind discriminator (``"index_market_value"``)."""
        ...

    def __repr__(self) -> str:
        """Return a representation of the event."""
        ...


@final
class MarketValue:
    """Streaming MarketValue tick (wire code 25). A calculated theoretical market value derived from the real-time bid/ask; `market_bid` / `market_ask` are the quote bid/ask after a size-imbalance + spread-aware nudge, `market_price` is their integer midpoint. Per-contract only (no full-stream variant)."""

    contract: ContractRef
    """Contract this event refers to."""
    ms_of_day: int
    """Milliseconds since midnight Eastern Time when the event was recorded."""
    market_bid: float
    """Calculated market bid (dollars), nudged from the quote bid."""
    market_ask: float
    """Calculated market ask (dollars), nudged from the quote ask."""
    market_price: float
    """Integer midpoint of `market_bid` / `market_ask` (dollars)."""
    date: int
    """Trading date as `YYYYMMDD`."""
    received_at_ns: int
    """Wall-clock nanoseconds since UNIX epoch, captured at frame decode time."""

    @property
    def kind(self) -> str:
        """Event kind discriminator (``"market_value"``)."""
        ...

    def __repr__(self) -> str:
        """Return a representation of the event."""
        ...


@final
class Ohlcvc:
    """Streaming OHLCVC bar."""

    contract: ContractRef
    """Contract this event refers to."""
    ms_of_day: int
    """Milliseconds since midnight Eastern Time when the event was recorded."""
    open: float
    """Opening price of the bar."""
    high: float
    """Highest traded price within the bar."""
    low: float
    """Lowest traded price within the bar."""
    close: float
    """Closing price of the bar."""
    volume: int
    """Total traded volume within the bar, in contracts/shares."""
    count: int
    """Number of trades aggregated into the bar."""
    date: int
    """Trading date as `YYYYMMDD`."""
    received_at_ns: int
    """Wall-clock nanoseconds since UNIX epoch, captured at frame decode time."""

    @property
    def kind(self) -> str:
        """Event kind discriminator (``"ohlcvc"``)."""
        ...

    def __repr__(self) -> str:
        """Return a representation of the event."""
        ...


@final
class OpenInterest:
    """Streaming OpenInterest tick."""

    contract: ContractRef
    """Contract this event refers to."""
    ms_of_day: int
    """Milliseconds since midnight Eastern Time when the event was recorded."""
    open_interest: int
    """Number of outstanding open contracts."""
    date: int
    """Trading date as `YYYYMMDD`."""
    received_at_ns: int
    """Wall-clock nanoseconds since UNIX epoch, captured at frame decode time."""

    @property
    def kind(self) -> str:
        """Event kind discriminator (``"open_interest"``)."""
        ...

    def __repr__(self) -> str:
        """Return a representation of the event."""
        ...


@final
class Quote:
    """Streaming Quote tick."""

    contract: ContractRef
    """Contract this event refers to."""
    ms_of_day: int
    """Milliseconds since midnight Eastern Time when the event was recorded."""
    bid_size: int
    """Number of contracts/shares resting at the bid."""
    bid_exchange: int
    """Exchange code posting the bid."""
    bid: float
    """Bid price."""
    bid_condition: int
    """Quote condition code for the bid."""
    ask_size: int
    """Number of contracts/shares resting at the ask."""
    ask_exchange: int
    """Exchange code posting the ask."""
    ask: float
    """Ask price."""
    ask_condition: int
    """Quote condition code for the ask."""
    date: int
    """Trading date as `YYYYMMDD`."""
    received_at_ns: int
    """Wall-clock nanoseconds since UNIX epoch, captured at frame decode time."""

    @property
    def kind(self) -> str:
        """Event kind discriminator (``"quote"``)."""
        ...

    def __repr__(self) -> str:
        """Return a representation of the event."""
        ...


@final
class Trade:
    """Streaming Trade tick."""

    contract: ContractRef
    """Contract this event refers to."""
    ms_of_day: int
    """Milliseconds since midnight Eastern Time when the event was recorded."""
    sequence: int
    """Exchange sequence number for ordering trades within the day."""
    condition: int
    """Primary trade condition code."""
    size: int
    """Trade size in contracts/shares."""
    exchange: int
    """Exchange code where the trade printed."""
    price: float
    """Trade price."""
    date: int
    """Trading date as `YYYYMMDD`."""
    received_at_ns: int
    """Wall-clock nanoseconds since UNIX epoch, captured at frame decode time."""

    @property
    def kind(self) -> str:
        """Event kind discriminator (``"trade"``)."""
        ...

    def __repr__(self) -> str:
        """Return a representation of the event."""
        ...


@final
class Connected:
    """Streaming server connection ack (wire code 4). Carries no payload."""

    @property
    def kind(self) -> str:
        """Event kind discriminator (``"connected"``)."""
        ...

    def __repr__(self) -> str:
        """Return a representation of the event."""
        ...


@final
class ContractAssigned:
    """Streaming server assigned a contract id. The `contract` payload carries the full resolved contract (root, sec_type, expiration / strike / right for options)."""

    id: int
    """Wire-internal contract id the streaming server assigns to this contract."""
    contract: ContractRef
    """Contract this event refers to."""

    @property
    def kind(self) -> str:
        """Event kind discriminator (``"contract_assigned"``)."""
        ...

    def __repr__(self) -> str:
        """Return a representation of the event."""
        ...


@final
class Disconnected:
    """Streaming connection ended. `reason` is the integer disconnect code: the server's own when the server sent a disconnect message (wire code 12), and otherwise the code the client has always reported for the way the connection ended (4 TimedOut for its own read deadline, -1 Unspecified for a socket that closed or failed). Read the resolved reason-name field for its symbolic name, and the cause field for where the disconnect came from."""

    reason: int
    """Disconnect code: the server's own when it sent a disconnect message, and otherwise the code the client reports for the way the connection ended."""
    cause: int
    """Where the disconnect came from: 0 the server sent a disconnect message, 1 the server ended the stream without one, 2 a read failed, 3 a write failed, 4 nothing arrived inside the client read deadline."""
    cause_io_error_kind: str
    """Name of the I/O error kind behind a read or write failure (`"ConnectionReset"`, `"BrokenPipe"`, ...). Empty for every other cause."""
    cause_timeout_ms: int
    """Client read deadline that expired, in milliseconds. Zero for every other cause."""

    @property
    def kind(self) -> str:
        """Event kind discriminator (``"disconnected"``)."""
        ...

    @property
    def reason_name(self) -> str:
        """Resolved disconnect-reason name (e.g. `"TooManyRequests"`, `"InvalidCredentials"`, `"Unspecified"` for unknown codes). Derived from the wire-level `reason` integer."""
        ...

    @property
    def cause_name(self) -> str:
        """Resolved disconnect-cause name: `"ServerSent"`, `"ClosedByServer"`, `"ReadFailed"`, `"WriteFailed"` or `"ReadTimeout"`. Derived from the `cause` integer."""
        ...

    def __repr__(self) -> str:
        """Return a representation of the event."""
        ...


@final
class LoginSuccess:
    """Streaming login succeeded. `permissions` is the server's opaque bundle string; diagnostic metadata only; for feature gating use the Nexus REST subscription tiers."""

    permissions: str
    """Server "Bundle" string copied verbatim from the METADATA frame; opaque diagnostic metadata."""

    @property
    def kind(self) -> str:
        """Event kind discriminator (``"login_success"``)."""
        ...

    def __repr__(self) -> str:
        """Return a representation of the event."""
        ...


@final
class MarketClose:
    """Streaming market-close signal (wire code 32). Carries no payload."""

    @property
    def kind(self) -> str:
        """Event kind discriminator (``"market_close"``)."""
        ...

    def __repr__(self) -> str:
        """Return a representation of the event."""
        ...


@final
class MarketOpen:
    """Streaming market-open signal (wire code 30). Carries no payload."""

    @property
    def kind(self) -> str:
        """Event kind discriminator (``"market_open"``)."""
        ...

    def __repr__(self) -> str:
        """Return a representation of the event."""
        ...


@final
class ParseError:
    """Streaming protocol-level parse error. Named `ParseError` on every binding so it never collides with the language's own error types (Python's exception classes, the JS global `Error`)."""

    message: str
    """Human-readable error text from the server."""

    @property
    def kind(self) -> str:
        """Event kind discriminator (``"parse_error"``)."""
        ...

    def __repr__(self) -> str:
        """Return a representation of the event."""
        ...


@final
class Ping:
    """Streaming server heartbeat (wire code 10). The server emits PING frames (observed 1-byte payload `[0]`) the client heartbeat logic does not have to answer; payload preserved for diagnostics."""

    payload: bytes
    """Raw frame payload bytes, preserved for diagnostics."""

    @property
    def kind(self) -> str:
        """Event kind discriminator (``"ping"``)."""
        ...

    def __repr__(self) -> str:
        """Return a representation of the event."""
        ...


@final
class Reconnected:
    """Streaming auto-reconnect succeeded; connection is live again. Carries no payload."""

    @property
    def kind(self) -> str:
        """Event kind discriminator (``"reconnected"``)."""
        ...

    def __repr__(self) -> str:
        """Return a representation of the event."""
        ...


@final
class ReconnectedServer:
    """Streaming server-side reconnect ack (wire code 13). Distinct from `Reconnected`, which the client emits from its auto-reconnect state machine once the new TLS session is authenticated."""

    @property
    def kind(self) -> str:
        """Event kind discriminator (``"reconnected_server"``)."""
        ...

    def __repr__(self) -> str:
        """Return a representation of the event."""
        ...


@final
class Reconnecting:
    """Streaming auto-reconnect is about to attempt reconnection. Emitted before sleeping for `delay_ms` milliseconds. `attempt` is 1-based and saturates at the maximum 32-bit signed value if the reconnect loop exceeds 2^31 attempts. `reason` and the cause fields describe the disconnect that triggered the attempt."""

    reason: int
    """Disconnect code: the server's own when it sent a disconnect message, and otherwise the code the client reports for the way the connection ended."""
    cause: int
    """Where the disconnect came from: 0 the server sent a disconnect message, 1 the server ended the stream without one, 2 a read failed, 3 a write failed, 4 nothing arrived inside the client read deadline."""
    cause_io_error_kind: str
    """Name of the I/O error kind behind a read or write failure (`"ConnectionReset"`, `"BrokenPipe"`, ...). Empty for every other cause."""
    cause_timeout_ms: int
    """Client read deadline that expired, in milliseconds. Zero for every other cause."""
    attempt: int
    """1-based index of this reconnect attempt."""
    delay_ms: int
    """Delay, in milliseconds, before the attempt fires."""

    @property
    def kind(self) -> str:
        """Event kind discriminator (``"reconnecting"``)."""
        ...

    @property
    def reason_name(self) -> str:
        """Resolved disconnect-reason name (e.g. `"TooManyRequests"`, `"InvalidCredentials"`, `"Unspecified"` for unknown codes). Derived from the wire-level `reason` integer."""
        ...

    @property
    def cause_name(self) -> str:
        """Resolved disconnect-cause name: `"ServerSent"`, `"ClosedByServer"`, `"ReadFailed"`, `"WriteFailed"` or `"ReadTimeout"`. Derived from the `cause` integer."""
        ...

    def __repr__(self) -> str:
        """Return a representation of the event."""
        ...


@final
class ReconnectsExhausted:
    """Streaming auto-reconnect stopped without a user-initiated shutdown; terminal for the session. Emitted when the reconnect budget (attempt count or wall-clock envelope) is exhausted, a permanent disconnect reason short-circuits recovery, a manual policy declines to reconnect, or a custom policy returns no delay. `reason` is the integer disconnect code of the final drop; read the resolved reason-name field for the symbolic name and the cause field for where that drop came from. `attempts` is the number of consecutive reconnect attempts consumed before giving up (0 when no reconnect was attempted)."""

    reason: int
    """Disconnect code: the server's own when it sent a disconnect message, and otherwise the code the client reports for the way the connection ended."""
    cause: int
    """Where the disconnect came from: 0 the server sent a disconnect message, 1 the server ended the stream without one, 2 a read failed, 3 a write failed, 4 nothing arrived inside the client read deadline."""
    cause_io_error_kind: str
    """Name of the I/O error kind behind a read or write failure (`"ConnectionReset"`, `"BrokenPipe"`, ...). Empty for every other cause."""
    cause_timeout_ms: int
    """Client read deadline that expired, in milliseconds. Zero for every other cause."""
    attempts: int
    """Number of consecutive reconnect attempts consumed before giving up."""

    @property
    def kind(self) -> str:
        """Event kind discriminator (``"reconnects_exhausted"``)."""
        ...

    @property
    def reason_name(self) -> str:
        """Resolved disconnect-reason name (e.g. `"TooManyRequests"`, `"InvalidCredentials"`, `"Unspecified"` for unknown codes). Derived from the wire-level `reason` integer."""
        ...

    @property
    def cause_name(self) -> str:
        """Resolved disconnect-cause name: `"ServerSent"`, `"ClosedByServer"`, `"ReadFailed"`, `"WriteFailed"` or `"ReadTimeout"`. Derived from the `cause` integer."""
        ...

    def __repr__(self) -> str:
        """Return a representation of the event."""
        ...


@final
class ReqResponse:
    """Streaming subscription response (wire code 40). `result` is an integer status code (0=Subscribed, 1=Error, 2=MaxStreamsReached, 3=InvalidPerms)."""

    req_id: int
    """Identifier of the subscription request this response answers."""
    result: int
    """Outcome of the subscription request."""

    @property
    def kind(self) -> str:
        """Event kind discriminator (``"req_response"``)."""
        ...

    def __repr__(self) -> str:
        """Return a representation of the event."""
        ...


@final
class Restart:
    """Streaming server stream restart (wire code 31). The server restarts the stream without dropping the TCP connection; delta decode state should be cleared on receipt."""

    @property
    def kind(self) -> str:
        """Event kind discriminator (``"restart"``)."""
        ...

    def __repr__(self) -> str:
        """Return a representation of the event."""
        ...


@final
class ServerError:
    """Streaming server-error message (wire code 11)."""

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
    """Streaming control variant the SDK does not yet recognise. Surfaced when a newer protocol revision adds a control event this build predates; keep dispatch logic forward-compatible by handling this variant. Carries no payload."""

    @property
    def kind(self) -> str:
        """Event kind discriminator (``"unknown_control"``)."""
        ...

    def __repr__(self) -> str:
        """Return a representation of the event."""
        ...


@final
class UnknownFrame:
    """Streaming server sent a frame with an unrecognised wire code. Raw bytes preserved for diagnostics / upstream bug reports."""

    code: int
    """Unrecognized frame code reported by the server."""
    payload: bytes
    """Raw frame payload bytes, preserved for diagnostics."""

    @property
    def kind(self) -> str:
        """Event kind discriminator (``"unknown_frame"``)."""
        ...

    def __repr__(self) -> str:
        """Return a representation of the event."""
        ...


# --- END GENERATED STREAMING EVENTS ---


# Union of every streaming event class delivered to the callback. Opaque
# to type checkers; narrow at runtime via `match` / `isinstance`.
StreamEvent = Any


# ─────────────────────────────────────────────────────────────────────
# Tick rows and their typed lists, returned by the market-data methods
# ─────────────────────────────────────────────────────────────────────

# --- BEGIN GENERATED TICK CLASSES (tick table) ---
# Generated from the tick table; do not edit by hand. Run
# `cargo run -p xtask --bin generate_sdk_surfaces` to refresh.

_Row = TypeVar("_Row")


class _TickList(Generic[_Row]):
    """Supports `len()`, indexing, slicing and iteration, and converts with
    `.to_list()`, `.to_arrow()`, `.to_pandas()` and `.to_polars()`.
    """

    def __new__(cls, ticks: Sequence[_Row] = ...) -> Self: ...
    def __len__(self) -> int: ...
    def __bool__(self) -> bool: ...
    def __repr__(self) -> str: ...
    @overload
    def __getitem__(self, key: int) -> _Row: ...
    @overload
    def __getitem__(self, key: slice) -> Self: ...
    def __iter__(self) -> Iterator[_Row]: ...
    def to_list(self) -> List[_Row]:
        """Return a plain Python `list` of the rows."""
        ...

    def to_arrow(self) -> Any:
        """Return a `pyarrow.Table` of the rows, carrying only the columns the response sent."""
        ...

    def to_pandas(self) -> Any:
        """Return a `pandas.DataFrame` via `pyarrow.Table.to_pandas()`. Requires pandas and pyarrow:
        `pip install thetadatadx-py[pandas]`.
        """
        ...

    def to_polars(self) -> Any:
        """Return a `polars.DataFrame` via `polars.from_arrow`. Requires polars and pyarrow:
        `pip install thetadatadx-py[polars]`.
        """
        ...


@final
class CalendarDay:
    """Calendar day. Market open/close schedule.

    `status` carries the vendor's own day-type vocabulary (`open` / `early_close` / `full_close` / `weekend`); unknown wire text fails decode loudly rather than degrading to a sentinel. `date` is `0` on the single-row `calendar_on_date` / `calendar_open_today` responses, where the server omits the date column.
    """

    def __new__(
        cls,
        *,
        date: int = ...,
        open_time: int = ...,
        close_time: int = ...,
        status: str,
    ) -> CalendarDay: ...

    date: int
    """Calendar date as a YYYYMMDD integer. 0 when the server omits the column (single-day endpoints)."""
    open_time: int
    """Market open time, milliseconds since midnight ET."""
    close_time: int
    """Market close time, milliseconds since midnight ET."""
    status: str
    """Vendor day classification: open, early_close, full_close, or weekend."""

    def __repr__(self) -> str: ...


@final
class CalendarDayList(_TickList[CalendarDay]):
    """Typed list of `CalendarDay` rows, returned by every market-data endpoint whose
    response is `CalendarDay`.
    """


@final
class EodTick:
    """End-of-day tick. Full EOD snapshot with OHLC + quote.

    The two time columns carry the vendor's v3 field semantics under their v3 names: `created` is when the EOD report was generated (~17:15 ET, NOT a trade time) and `last_trade` is the time of the day's final trade. On a day with no trades the report zero-fills the trade-derived columns: `open` / `high` / `low` / `close` are `0.0` and `last_trade_ms_of_day` is `0`; the quote-side columns still carry the closing NBBO snapshot.
    """

    def __new__(
        cls,
        *,
        created_ms_of_day: int = ...,
        last_trade_ms_of_day: int = ...,
        open: float = ...,
        high: float = ...,
        low: float = ...,
        close: float = ...,
        volume: int = ...,
        count: int = ...,
        bid_size: int = ...,
        bid_exchange: Optional[int] = ...,
        bid: float = ...,
        bid_condition: Optional[int] = ...,
        ask_size: int = ...,
        ask_exchange: Optional[int] = ...,
        ask: float = ...,
        ask_condition: Optional[int] = ...,
        date: int = ...,
        expiration: Optional[int] = ...,
        strike: Optional[float] = ...,
        right: Optional[str] = ...,
    ) -> EodTick: ...

    created_ms_of_day: int
    """EOD report creation time (NOT a trade time), milliseconds since midnight ET."""
    last_trade_ms_of_day: int
    """Time of the day's last trade, milliseconds since midnight ET. 0 when no trades printed that day."""
    open: float
    """Opening trade price. 0.0 when no trades printed that day."""
    high: float
    """Highest traded price. 0.0 when no trades printed that day."""
    low: float
    """Lowest traded price. 0.0 when no trades printed that day."""
    close: float
    """Closing traded price. 0.0 when no trades printed that day."""
    volume: int
    """Number of contracts or shares traded."""
    count: int
    """Number of trades."""
    bid_size: int
    """Last NBBO bid size."""
    bid_exchange: Optional[int]
    """Exchange code of the NBBO bid. `None` when the response carried no such column."""
    bid: float
    """Last NBBO bid price."""
    bid_condition: Optional[int]
    """Quote condition code on the bid side. `None` when the response carried no such column."""
    ask_size: int
    """Last NBBO ask size."""
    ask_exchange: Optional[int]
    """Exchange code of the NBBO ask. `None` when the response carried no such column."""
    ask: float
    """Last NBBO ask price."""
    ask_condition: Optional[int]
    """Quote condition code on the ask side. `None` when the response carried no such column."""
    date: int
    """Trading date as a YYYYMMDD integer."""
    expiration: Optional[int]
    """Expiration of the row's contract as a YYYYMMDD integer on a wildcard option response; `None` when the response names a single contract."""
    strike: Optional[float]
    """Strike of the row's contract in dollars on a wildcard option response; `None` when the response names a single contract."""
    right: Optional[str]
    """Right of the row's contract on a wildcard option response, `"C"` or `"P"`; `None` when the response names a single contract."""

    @property
    def created_timestamp_ms(self) -> Optional[int]:
        """Unix epoch milliseconds (UTC, DST-aware) combining `date` with `created_ms_of_day` (Eastern-Time milliseconds-of-day). `None` when `date` is absent (`0`)."""
        ...

    @property
    def last_trade_timestamp_ms(self) -> Optional[int]:
        """Unix epoch milliseconds (UTC, DST-aware) combining `date` with `last_trade_ms_of_day` (Eastern-Time milliseconds-of-day). `None` when `date` is absent (`0`)."""
        ...

    def __repr__(self) -> str: ...


@final
class EodTickList(_TickList[EodTick]):
    """Typed list of `EodTick` rows, returned by every market-data endpoint whose
    response is `EodTick`.
    """


@final
class GreeksAllTick:
    """Full union Greeks tick -- every Greek the v3 server publishes on the `option_*_greeks_all` endpoints.

    The vendor's per-order endpoints (`option_*_greeks_first_order`, `_second_order`, `_third_order`) emit strict subsets of these columns and bind to `GreeksFirstOrderTick`, `GreeksSecondOrderTick` and `GreeksThirdOrderTick` respectively. `option_*_greeks_implied_volatility` binds to `IvTick`, and `option_*_greeks_eod` to `GreeksEodTick`.
    """

    def __new__(
        cls,
        *,
        ms_of_day: int = ...,
        bid: float = ...,
        ask: float = ...,
        implied_volatility: float = ...,
        delta: float = ...,
        gamma: float = ...,
        theta: float = ...,
        vega: float = ...,
        rho: float = ...,
        iv_error: float = ...,
        vanna: float = ...,
        charm: float = ...,
        vomma: float = ...,
        veta: float = ...,
        speed: float = ...,
        zomma: float = ...,
        color: float = ...,
        ultima: float = ...,
        d1: float = ...,
        d2: float = ...,
        dual_delta: float = ...,
        dual_gamma: float = ...,
        epsilon: float = ...,
        lambda_: float = ...,
        vera: float = ...,
        underlying_ms_of_day: int = ...,
        underlying_price: float = ...,
        date: int = ...,
        underlying_date: int = ...,
        expiration: Optional[int] = ...,
        strike: Optional[float] = ...,
        right: Optional[str] = ...,
    ) -> GreeksAllTick: ...

    ms_of_day: int
    """Milliseconds since midnight Eastern Time."""
    bid: float
    """Last NBBO bid price."""
    ask: float
    """Last NBBO ask price."""
    implied_volatility: float
    """Implied volatility solved from the option price. The vendor names this column `IMPLIED_VOL`."""
    delta: float
    """Sensitivity of the option value to a one-dollar move in the underlying."""
    gamma: float
    """Rate of change of delta with respect to the underlying price."""
    theta: float
    """Sensitivity of the option value to the passage of time."""
    vega: float
    """Sensitivity of the option value to a change in implied volatility."""
    rho: float
    """Sensitivity of the option value to the risk-free rate."""
    iv_error: float
    """Residual pricing error of the implied-volatility solve."""
    vanna: float
    """Sensitivity of delta to a change in implied volatility."""
    charm: float
    """Rate of change of delta over time."""
    vomma: float
    """Sensitivity of vega to a change in implied volatility."""
    veta: float
    """Rate of change of vega over time."""
    speed: float
    """Rate of change of gamma with respect to the underlying price."""
    zomma: float
    """Sensitivity of gamma to a change in implied volatility."""
    color: float
    """Rate of change of gamma over time."""
    ultima: float
    """Sensitivity of vomma to a change in implied volatility."""
    d1: float
    """Black-Scholes d1 intermediate term."""
    d2: float
    """Black-Scholes d2 intermediate term."""
    dual_delta: float
    """Sensitivity of the option value to the strike price."""
    dual_gamma: float
    """Second derivative of the option value with respect to the strike."""
    epsilon: float
    """Sensitivity of the option value to the dividend yield."""
    lambda_: float
    """Percent change in the option value per percent change in the underlying."""
    vera: float
    """Sensitivity of rho to a change in implied volatility."""
    underlying_ms_of_day: int
    """Timestamp of the underlying price, milliseconds since midnight ET. The vendor names this column `MS_OF_DAY2`."""
    underlying_price: float
    """Underlying price used in the calculation (midpoint of the underlying)."""
    date: int
    """Trading date as a YYYYMMDD integer."""
    underlying_date: int
    """Trading date of the underlying price as a YYYYMMDD integer, decoded from the same wire column as `underlying_ms_of_day`. It differs from `date` when the underlying price and the option quote come from different sessions."""
    expiration: Optional[int]
    """Expiration of the row's contract as a YYYYMMDD integer on a wildcard option response; `None` when the response names a single contract."""
    strike: Optional[float]
    """Strike of the row's contract in dollars on a wildcard option response; `None` when the response names a single contract."""
    right: Optional[str]
    """Right of the row's contract on a wildcard option response, `"C"` or `"P"`; `None` when the response names a single contract."""

    @property
    def timestamp_ms(self) -> Optional[int]:
        """Unix epoch milliseconds (UTC, DST-aware) combining `date` with `ms_of_day` (Eastern-Time milliseconds-of-day). `None` when `date` is absent (`0`)."""
        ...

    @property
    def underlying_timestamp_ms(self) -> Optional[int]:
        """Unix epoch milliseconds (UTC, DST-aware) combining `underlying_date` with `underlying_ms_of_day` (Eastern-Time milliseconds-of-day). `None` when `underlying_date` is absent (`0`)."""
        ...

    def __repr__(self) -> str: ...


@final
class GreeksAllTickList(_TickList[GreeksAllTick]):
    """Typed list of `GreeksAllTick` rows, returned by every market-data endpoint whose
    response is `GreeksAllTick`.
    """


@final
class GreeksEodTick:
    """End-of-day union Greeks tick -- every Greek the v3 server publishes on `option_history_greeks_eod`, paired with the twelve EOD trade/quote context columns (`open`, `high`, `low`, `close`, `volume`, `count`, `bid_size`, `bid_exchange`, `bid_condition`, `ask_size`, `ask_exchange`, `ask_condition`) that identify the daily bar + closing NBBO snapshot the Greeks were calculated against."""

    def __new__(
        cls,
        *,
        ms_of_day: int = ...,
        open: float = ...,
        high: float = ...,
        low: float = ...,
        close: float = ...,
        volume: int = ...,
        count: int = ...,
        bid_size: int = ...,
        bid_exchange: Optional[int] = ...,
        bid: float = ...,
        bid_condition: Optional[int] = ...,
        ask_size: int = ...,
        ask_exchange: Optional[int] = ...,
        ask: float = ...,
        ask_condition: Optional[int] = ...,
        delta: float = ...,
        theta: float = ...,
        vega: float = ...,
        rho: float = ...,
        epsilon: float = ...,
        lambda_: float = ...,
        gamma: float = ...,
        vanna: float = ...,
        charm: float = ...,
        vomma: float = ...,
        veta: float = ...,
        vera: float = ...,
        speed: float = ...,
        zomma: float = ...,
        color: float = ...,
        ultima: float = ...,
        d1: float = ...,
        d2: float = ...,
        dual_delta: float = ...,
        dual_gamma: float = ...,
        implied_volatility: float = ...,
        iv_error: float = ...,
        underlying_ms_of_day: int = ...,
        underlying_price: float = ...,
        date: int = ...,
        underlying_date: int = ...,
        expiration: Optional[int] = ...,
        strike: Optional[float] = ...,
        right: Optional[str] = ...,
    ) -> GreeksEodTick: ...

    ms_of_day: int
    """EOD report creation time, milliseconds since midnight ET."""
    open: float
    """Opening trade price."""
    high: float
    """Highest traded price."""
    low: float
    """Lowest traded price."""
    close: float
    """Closing traded price."""
    volume: int
    """Number of contracts or shares traded."""
    count: int
    """Number of trades."""
    bid_size: int
    """Last NBBO bid size."""
    bid_exchange: Optional[int]
    """Exchange code of the NBBO bid. `None` when the response carried no such column."""
    bid: float
    """Last NBBO bid price."""
    bid_condition: Optional[int]
    """Quote condition code on the bid side. `None` when the response carried no such column."""
    ask_size: int
    """Last NBBO ask size."""
    ask_exchange: Optional[int]
    """Exchange code of the NBBO ask. `None` when the response carried no such column."""
    ask: float
    """Last NBBO ask price."""
    ask_condition: Optional[int]
    """Quote condition code on the ask side. `None` when the response carried no such column."""
    delta: float
    """Sensitivity of the option value to a one-dollar move in the underlying."""
    theta: float
    """Sensitivity of the option value to the passage of time."""
    vega: float
    """Sensitivity of the option value to a change in implied volatility."""
    rho: float
    """Sensitivity of the option value to the risk-free rate."""
    epsilon: float
    """Sensitivity of the option value to the dividend yield."""
    lambda_: float
    """Percent change in the option value per percent change in the underlying."""
    gamma: float
    """Rate of change of delta with respect to the underlying price."""
    vanna: float
    """Sensitivity of delta to a change in implied volatility."""
    charm: float
    """Rate of change of delta over time."""
    vomma: float
    """Sensitivity of vega to a change in implied volatility."""
    veta: float
    """Rate of change of vega over time."""
    vera: float
    """Sensitivity of rho to a change in implied volatility."""
    speed: float
    """Rate of change of gamma with respect to the underlying price."""
    zomma: float
    """Sensitivity of gamma to a change in implied volatility."""
    color: float
    """Rate of change of gamma over time."""
    ultima: float
    """Sensitivity of vomma to a change in implied volatility."""
    d1: float
    """Black-Scholes d1 intermediate term."""
    d2: float
    """Black-Scholes d2 intermediate term."""
    dual_delta: float
    """Sensitivity of the option value to the strike price."""
    dual_gamma: float
    """Second derivative of the option value with respect to the strike."""
    implied_volatility: float
    """Implied volatility solved from the option price. The vendor names this column `IMPLIED_VOL`."""
    iv_error: float
    """Residual pricing error of the implied-volatility solve."""
    underlying_ms_of_day: int
    """Timestamp of the underlying price, milliseconds since midnight ET. The vendor names this column `MS_OF_DAY2`."""
    underlying_price: float
    """Underlying price used in the calculation (midpoint of the underlying)."""
    date: int
    """Trading date as a YYYYMMDD integer."""
    underlying_date: int
    """Trading date of the underlying price as a YYYYMMDD integer, decoded from the same wire column as `underlying_ms_of_day`. It differs from `date` when the underlying price and the option quote come from different sessions."""
    expiration: Optional[int]
    """Expiration of the row's contract as a YYYYMMDD integer on a wildcard option response; `None` when the response names a single contract."""
    strike: Optional[float]
    """Strike of the row's contract in dollars on a wildcard option response; `None` when the response names a single contract."""
    right: Optional[str]
    """Right of the row's contract on a wildcard option response, `"C"` or `"P"`; `None` when the response names a single contract."""

    @property
    def timestamp_ms(self) -> Optional[int]:
        """Unix epoch milliseconds (UTC, DST-aware) combining `date` with `ms_of_day` (Eastern-Time milliseconds-of-day). `None` when `date` is absent (`0`)."""
        ...

    @property
    def underlying_timestamp_ms(self) -> Optional[int]:
        """Unix epoch milliseconds (UTC, DST-aware) combining `underlying_date` with `underlying_ms_of_day` (Eastern-Time milliseconds-of-day). `None` when `underlying_date` is absent (`0`)."""
        ...

    def __repr__(self) -> str: ...


@final
class GreeksEodTickList(_TickList[GreeksEodTick]):
    """Typed list of `GreeksEodTick` rows, returned by every market-data endpoint whose
    response is `GreeksEodTick`.
    """


@final
class GreeksFirstOrderTick:
    """First-order Greeks tick -- the strict column subset emitted by the vendor's `option_*_greeks_first_order` endpoints (delta / theta / vega / rho / epsilon / lambda) plus the bid/ask quote pair, the IV pair, and the underlying snapshot used to derive each Greek."""

    def __new__(
        cls,
        *,
        ms_of_day: int = ...,
        bid: float = ...,
        ask: float = ...,
        delta: float = ...,
        theta: float = ...,
        vega: float = ...,
        rho: float = ...,
        epsilon: float = ...,
        lambda_: float = ...,
        implied_volatility: float = ...,
        iv_error: float = ...,
        underlying_ms_of_day: int = ...,
        underlying_price: float = ...,
        date: int = ...,
        underlying_date: int = ...,
        expiration: Optional[int] = ...,
        strike: Optional[float] = ...,
        right: Optional[str] = ...,
    ) -> GreeksFirstOrderTick: ...

    ms_of_day: int
    """Milliseconds since midnight Eastern Time."""
    bid: float
    """Last NBBO bid price."""
    ask: float
    """Last NBBO ask price."""
    delta: float
    """Sensitivity of the option value to a one-dollar move in the underlying."""
    theta: float
    """Sensitivity of the option value to the passage of time."""
    vega: float
    """Sensitivity of the option value to a change in implied volatility."""
    rho: float
    """Sensitivity of the option value to the risk-free rate."""
    epsilon: float
    """Sensitivity of the option value to the dividend yield."""
    lambda_: float
    """Percent change in the option value per percent change in the underlying."""
    implied_volatility: float
    """Implied volatility solved from the option price. The vendor names this column `IMPLIED_VOL`."""
    iv_error: float
    """Residual pricing error of the implied-volatility solve."""
    underlying_ms_of_day: int
    """Timestamp of the underlying price, milliseconds since midnight ET. The vendor names this column `MS_OF_DAY2`."""
    underlying_price: float
    """Underlying price used in the calculation (midpoint of the underlying)."""
    date: int
    """Trading date as a YYYYMMDD integer."""
    underlying_date: int
    """Trading date of the underlying price as a YYYYMMDD integer, decoded from the same wire column as `underlying_ms_of_day`. It differs from `date` when the underlying price and the option quote come from different sessions."""
    expiration: Optional[int]
    """Expiration of the row's contract as a YYYYMMDD integer on a wildcard option response; `None` when the response names a single contract."""
    strike: Optional[float]
    """Strike of the row's contract in dollars on a wildcard option response; `None` when the response names a single contract."""
    right: Optional[str]
    """Right of the row's contract on a wildcard option response, `"C"` or `"P"`; `None` when the response names a single contract."""

    @property
    def timestamp_ms(self) -> Optional[int]:
        """Unix epoch milliseconds (UTC, DST-aware) combining `date` with `ms_of_day` (Eastern-Time milliseconds-of-day). `None` when `date` is absent (`0`)."""
        ...

    @property
    def underlying_timestamp_ms(self) -> Optional[int]:
        """Unix epoch milliseconds (UTC, DST-aware) combining `underlying_date` with `underlying_ms_of_day` (Eastern-Time milliseconds-of-day). `None` when `underlying_date` is absent (`0`)."""
        ...

    def __repr__(self) -> str: ...


@final
class GreeksFirstOrderTickList(_TickList[GreeksFirstOrderTick]):
    """Typed list of `GreeksFirstOrderTick` rows, returned by every market-data endpoint whose
    response is `GreeksFirstOrderTick`.
    """


@final
class GreeksSecondOrderTick:
    """Second-order Greeks tick -- the strict column subset emitted by the vendor's `option_*_greeks_second_order` endpoints (gamma / vanna / charm / vomma / veta) plus the bid/ask quote pair, the IV pair, and the underlying snapshot."""

    def __new__(
        cls,
        *,
        ms_of_day: int = ...,
        bid: float = ...,
        ask: float = ...,
        gamma: float = ...,
        vanna: float = ...,
        charm: float = ...,
        vomma: float = ...,
        veta: float = ...,
        implied_volatility: float = ...,
        iv_error: float = ...,
        underlying_ms_of_day: int = ...,
        underlying_price: float = ...,
        date: int = ...,
        underlying_date: int = ...,
        expiration: Optional[int] = ...,
        strike: Optional[float] = ...,
        right: Optional[str] = ...,
    ) -> GreeksSecondOrderTick: ...

    ms_of_day: int
    """Milliseconds since midnight Eastern Time."""
    bid: float
    """Last NBBO bid price."""
    ask: float
    """Last NBBO ask price."""
    gamma: float
    """Rate of change of delta with respect to the underlying price."""
    vanna: float
    """Sensitivity of delta to a change in implied volatility."""
    charm: float
    """Rate of change of delta over time."""
    vomma: float
    """Sensitivity of vega to a change in implied volatility."""
    veta: float
    """Rate of change of vega over time."""
    implied_volatility: float
    """Implied volatility solved from the option price. The vendor names this column `IMPLIED_VOL`."""
    iv_error: float
    """Residual pricing error of the implied-volatility solve."""
    underlying_ms_of_day: int
    """Timestamp of the underlying price, milliseconds since midnight ET. The vendor names this column `MS_OF_DAY2`."""
    underlying_price: float
    """Underlying price used in the calculation (midpoint of the underlying)."""
    date: int
    """Trading date as a YYYYMMDD integer."""
    underlying_date: int
    """Trading date of the underlying price as a YYYYMMDD integer, decoded from the same wire column as `underlying_ms_of_day`. It differs from `date` when the underlying price and the option quote come from different sessions."""
    expiration: Optional[int]
    """Expiration of the row's contract as a YYYYMMDD integer on a wildcard option response; `None` when the response names a single contract."""
    strike: Optional[float]
    """Strike of the row's contract in dollars on a wildcard option response; `None` when the response names a single contract."""
    right: Optional[str]
    """Right of the row's contract on a wildcard option response, `"C"` or `"P"`; `None` when the response names a single contract."""

    @property
    def timestamp_ms(self) -> Optional[int]:
        """Unix epoch milliseconds (UTC, DST-aware) combining `date` with `ms_of_day` (Eastern-Time milliseconds-of-day). `None` when `date` is absent (`0`)."""
        ...

    @property
    def underlying_timestamp_ms(self) -> Optional[int]:
        """Unix epoch milliseconds (UTC, DST-aware) combining `underlying_date` with `underlying_ms_of_day` (Eastern-Time milliseconds-of-day). `None` when `underlying_date` is absent (`0`)."""
        ...

    def __repr__(self) -> str: ...


@final
class GreeksSecondOrderTickList(_TickList[GreeksSecondOrderTick]):
    """Typed list of `GreeksSecondOrderTick` rows, returned by every market-data endpoint whose
    response is `GreeksSecondOrderTick`.
    """


@final
class GreeksThirdOrderTick:
    """Third-order Greeks tick -- the strict column subset emitted by the vendor's `option_*_greeks_third_order` endpoints (speed / zomma / color / ultima) plus the bid/ask quote pair, the IV pair, and the underlying snapshot. The vendor's third-order schema does not publish `vera`."""

    def __new__(
        cls,
        *,
        ms_of_day: int = ...,
        bid: float = ...,
        ask: float = ...,
        speed: float = ...,
        zomma: float = ...,
        color: float = ...,
        ultima: float = ...,
        implied_volatility: float = ...,
        iv_error: float = ...,
        underlying_ms_of_day: int = ...,
        underlying_price: float = ...,
        date: int = ...,
        underlying_date: int = ...,
        expiration: Optional[int] = ...,
        strike: Optional[float] = ...,
        right: Optional[str] = ...,
    ) -> GreeksThirdOrderTick: ...

    ms_of_day: int
    """Milliseconds since midnight Eastern Time."""
    bid: float
    """Last NBBO bid price."""
    ask: float
    """Last NBBO ask price."""
    speed: float
    """Rate of change of gamma with respect to the underlying price."""
    zomma: float
    """Sensitivity of gamma to a change in implied volatility."""
    color: float
    """Rate of change of gamma over time."""
    ultima: float
    """Sensitivity of vomma to a change in implied volatility."""
    implied_volatility: float
    """Implied volatility solved from the option price. The vendor names this column `IMPLIED_VOL`."""
    iv_error: float
    """Residual pricing error of the implied-volatility solve."""
    underlying_ms_of_day: int
    """Timestamp of the underlying price, milliseconds since midnight ET. The vendor names this column `MS_OF_DAY2`."""
    underlying_price: float
    """Underlying price used in the calculation (midpoint of the underlying)."""
    date: int
    """Trading date as a YYYYMMDD integer."""
    underlying_date: int
    """Trading date of the underlying price as a YYYYMMDD integer, decoded from the same wire column as `underlying_ms_of_day`. It differs from `date` when the underlying price and the option quote come from different sessions."""
    expiration: Optional[int]
    """Expiration of the row's contract as a YYYYMMDD integer on a wildcard option response; `None` when the response names a single contract."""
    strike: Optional[float]
    """Strike of the row's contract in dollars on a wildcard option response; `None` when the response names a single contract."""
    right: Optional[str]
    """Right of the row's contract on a wildcard option response, `"C"` or `"P"`; `None` when the response names a single contract."""

    @property
    def timestamp_ms(self) -> Optional[int]:
        """Unix epoch milliseconds (UTC, DST-aware) combining `date` with `ms_of_day` (Eastern-Time milliseconds-of-day). `None` when `date` is absent (`0`)."""
        ...

    @property
    def underlying_timestamp_ms(self) -> Optional[int]:
        """Unix epoch milliseconds (UTC, DST-aware) combining `underlying_date` with `underlying_ms_of_day` (Eastern-Time milliseconds-of-day). `None` when `underlying_date` is absent (`0`)."""
        ...

    def __repr__(self) -> str: ...


@final
class GreeksThirdOrderTickList(_TickList[GreeksThirdOrderTick]):
    """Typed list of `GreeksThirdOrderTick` rows, returned by every market-data endpoint whose
    response is `GreeksThirdOrderTick`.
    """


@final
class IndexPriceAtTimeTick:
    """Index price-at-time tick -- the trade-shaped row the v3 server publishes on `index_at_time_price`."""

    def __new__(
        cls,
        *,
        ms_of_day: int = ...,
        sequence: int = ...,
        ext_condition1: Optional[int] = ...,
        ext_condition2: Optional[int] = ...,
        ext_condition3: Optional[int] = ...,
        ext_condition4: Optional[int] = ...,
        condition: Optional[int] = ...,
        size: int = ...,
        exchange: Optional[int] = ...,
        price: float = ...,
        date: int = ...,
    ) -> IndexPriceAtTimeTick: ...

    ms_of_day: int
    """Milliseconds since midnight Eastern Time."""
    sequence: int
    """Exchange-assigned trade sequence number."""
    ext_condition1: Optional[int]
    """Additional trade condition code. `None` when the response carried no such column."""
    ext_condition2: Optional[int]
    """Additional trade condition code. `None` when the response carried no such column."""
    ext_condition3: Optional[int]
    """Additional trade condition code. `None` when the response carried no such column."""
    ext_condition4: Optional[int]
    """Additional trade condition code. `None` when the response carried no such column."""
    condition: Optional[int]
    """Trade condition code. `None` when the response carried no such column."""
    size: int
    """Number of contracts or shares traded."""
    exchange: Optional[int]
    """Exchange code where the trade executed. `None` when the response carried no such column."""
    price: float
    """Index value at the requested time."""
    date: int
    """Trading date as a YYYYMMDD integer."""

    @property
    def timestamp_ms(self) -> Optional[int]:
        """Unix epoch milliseconds (UTC, DST-aware) combining `date` with `ms_of_day` (Eastern-Time milliseconds-of-day). `None` when `date` is absent (`0`)."""
        ...

    def __repr__(self) -> str: ...


@final
class IndexPriceAtTimeTickList(_TickList[IndexPriceAtTimeTick]):
    """Typed list of `IndexPriceAtTimeTick` rows, returned by every market-data endpoint whose
    response is `IndexPriceAtTimeTick`.
    """


@final
class InterestRateTick:
    """Interest rate tick. End-of-day interest rate (percent)."""

    def __new__(
        cls,
        *,
        date: int = ...,
        rate: float = ...,
    ) -> InterestRateTick: ...

    date: int
    """Report creation date as a YYYYMMDD integer."""
    rate: float
    """Interest rate expressed as a percent (1.25 means 1.25%)."""

    def __repr__(self) -> str: ...


@final
class InterestRateTickList(_TickList[InterestRateTick]):
    """Typed list of `InterestRateTick` rows, returned by every market-data endpoint whose
    response is `InterestRateTick`.
    """


@final
class IntervalQuoteTick:
    """Interval quote tick. The NBBO in effect at one interval.

    Returned by the quote flat files (`*_flat_file_quote`), which carry one row per interval in which a new quote was reported. Each row holds two times: `interval_ms_of_day`, the interval the row samples, and `quote_ms_of_day`, when the quote in effect at that interval was reported.
    """

    def __new__(
        cls,
        *,
        interval_ms_of_day: int = ...,
        quote_ms_of_day: int = ...,
        bid_size: int = ...,
        bid_exchange: Optional[int] = ...,
        bid: float = ...,
        bid_condition: Optional[int] = ...,
        ask_size: int = ...,
        ask_exchange: Optional[int] = ...,
        ask: float = ...,
        ask_condition: Optional[int] = ...,
        date: int = ...,
        expiration: Optional[int] = ...,
        strike: Optional[float] = ...,
        right: Optional[str] = ...,
    ) -> IntervalQuoteTick: ...

    interval_ms_of_day: int
    """Time of the interval the row samples, milliseconds since midnight ET."""
    quote_ms_of_day: int
    """Time the quote in effect at the interval was reported, milliseconds since midnight ET."""
    bid_size: int
    """NBBO bid size."""
    bid_exchange: Optional[int]
    """Exchange code of the NBBO bid. `None` when the response carried no such column."""
    bid: float
    """NBBO bid price."""
    bid_condition: Optional[int]
    """Quote condition code on the bid side. `None` when the response carried no such column."""
    ask_size: int
    """NBBO ask size."""
    ask_exchange: Optional[int]
    """Exchange code of the NBBO ask. `None` when the response carried no such column."""
    ask: float
    """NBBO ask price."""
    ask_condition: Optional[int]
    """Quote condition code on the ask side. `None` when the response carried no such column."""
    date: int
    """Trading date as a YYYYMMDD integer."""
    expiration: Optional[int]
    """Expiration of the row's contract as a YYYYMMDD integer on a wildcard option response; `None` when the response names a single contract."""
    strike: Optional[float]
    """Strike of the row's contract in dollars on a wildcard option response; `None` when the response names a single contract."""
    right: Optional[str]
    """Right of the row's contract on a wildcard option response, `"C"` or `"P"`; `None` when the response names a single contract."""

    @property
    def interval_timestamp_ms(self) -> Optional[int]:
        """Unix epoch milliseconds (UTC, DST-aware) combining `date` with `interval_ms_of_day` (Eastern-Time milliseconds-of-day). `None` when `date` is absent (`0`)."""
        ...

    @property
    def quote_timestamp_ms(self) -> Optional[int]:
        """Unix epoch milliseconds (UTC, DST-aware) combining `date` with `quote_ms_of_day` (Eastern-Time milliseconds-of-day). `None` when `date` is absent (`0`)."""
        ...

    def __repr__(self) -> str: ...


@final
class IntervalQuoteTickList(_TickList[IntervalQuoteTick]):
    """Typed list of `IntervalQuoteTick` rows, returned by every market-data endpoint whose
    response is `IntervalQuoteTick`.
    """


@final
class IvTick:
    """Implied volatility tick.

    The snapshot variant (`option_snapshot_greeks_implied_volatility`) carries only `ms_of_day`, `implied_volatility`, `iv_error` and `date`; the other fields then read zero.
    """

    def __new__(
        cls,
        *,
        ms_of_day: int = ...,
        bid: float = ...,
        bid_implied_volatility: float = ...,
        midpoint: float = ...,
        implied_volatility: float = ...,
        ask: float = ...,
        ask_implied_volatility: float = ...,
        iv_error: float = ...,
        underlying_ms_of_day: int = ...,
        underlying_price: float = ...,
        date: int = ...,
        underlying_date: int = ...,
        expiration: Optional[int] = ...,
        strike: Optional[float] = ...,
        right: Optional[str] = ...,
    ) -> IvTick: ...

    ms_of_day: int
    """Milliseconds since midnight Eastern Time."""
    bid: float
    """Last NBBO bid price."""
    bid_implied_volatility: float
    """Implied volatility solved at the bid price. The vendor names this column `BID_IMPLIED_VOL`."""
    midpoint: float
    """Midpoint of the bid and ask prices."""
    implied_volatility: float
    """Implied volatility solved from the option price. The vendor names this column `IMPLIED_VOL`."""
    ask: float
    """Last NBBO ask price."""
    ask_implied_volatility: float
    """Implied volatility solved at the ask price. The vendor names this column `ASK_IMPLIED_VOL`."""
    iv_error: float
    """Residual pricing error of the implied-volatility solve."""
    underlying_ms_of_day: int
    """Timestamp of the underlying price, milliseconds since midnight ET. The vendor names this column `MS_OF_DAY2`."""
    underlying_price: float
    """Underlying price used in the calculation (midpoint of the underlying)."""
    date: int
    """Trading date as a YYYYMMDD integer."""
    underlying_date: int
    """Trading date of the underlying price as a YYYYMMDD integer, decoded from the same wire column as `underlying_ms_of_day`. It differs from `date` when the underlying price and the option quote come from different sessions."""
    expiration: Optional[int]
    """Expiration of the row's contract as a YYYYMMDD integer on a wildcard option response; `None` when the response names a single contract."""
    strike: Optional[float]
    """Strike of the row's contract in dollars on a wildcard option response; `None` when the response names a single contract."""
    right: Optional[str]
    """Right of the row's contract on a wildcard option response, `"C"` or `"P"`; `None` when the response names a single contract."""

    @property
    def timestamp_ms(self) -> Optional[int]:
        """Unix epoch milliseconds (UTC, DST-aware) combining `date` with `ms_of_day` (Eastern-Time milliseconds-of-day). `None` when `date` is absent (`0`)."""
        ...

    @property
    def underlying_timestamp_ms(self) -> Optional[int]:
        """Unix epoch milliseconds (UTC, DST-aware) combining `underlying_date` with `underlying_ms_of_day` (Eastern-Time milliseconds-of-day). `None` when `underlying_date` is absent (`0`)."""
        ...

    def __repr__(self) -> str: ...


@final
class IvTickList(_TickList[IvTick]):
    """Typed list of `IvTick` rows, returned by every market-data endpoint whose
    response is `IvTick`.
    """


@final
class MarketValueTick:
    """Market value tick -- quoted bid/ask/price for a symbol."""

    def __new__(
        cls,
        *,
        ms_of_day: int = ...,
        market_bid: float = ...,
        market_ask: float = ...,
        market_price: float = ...,
        date: int = ...,
        expiration: Optional[int] = ...,
        strike: Optional[float] = ...,
        right: Optional[str] = ...,
    ) -> MarketValueTick: ...

    ms_of_day: int
    """Milliseconds since midnight Eastern Time."""
    market_bid: float
    """Last market bid."""
    market_ask: float
    """Last market ask."""
    market_price: float
    """Market value derived from the last NBBO."""
    date: int
    """Trading date as a YYYYMMDD integer."""
    expiration: Optional[int]
    """Expiration of the row's contract as a YYYYMMDD integer on a wildcard option response; `None` when the response names a single contract."""
    strike: Optional[float]
    """Strike of the row's contract in dollars on a wildcard option response; `None` when the response names a single contract."""
    right: Optional[str]
    """Right of the row's contract on a wildcard option response, `"C"` or `"P"`; `None` when the response names a single contract."""

    @property
    def timestamp_ms(self) -> Optional[int]:
        """Unix epoch milliseconds (UTC, DST-aware) combining `date` with `ms_of_day` (Eastern-Time milliseconds-of-day). `None` when `date` is absent (`0`)."""
        ...

    def __repr__(self) -> str: ...


@final
class MarketValueTickList(_TickList[MarketValueTick]):
    """Typed list of `MarketValueTick` rows, returned by every market-data endpoint whose
    response is `MarketValueTick`.
    """


@final
class OhlcTick:
    """OHLC tick. Aggregated bar data including SIP-rule VWAP.

    `stock_history_ohlc`, `option_history_ohlc` and `index_history_ohlc` carry every column. The snapshot variants (`*_snapshot_ohlc`) omit `vwap`, which then reads `0.0`, as `volume` and `count` read `0` on quote-only intraday bars.
    """

    def __new__(
        cls,
        *,
        ms_of_day: int = ...,
        open: float = ...,
        high: float = ...,
        low: float = ...,
        close: float = ...,
        volume: int = ...,
        count: int = ...,
        vwap: float = ...,
        date: int = ...,
        expiration: Optional[int] = ...,
        strike: Optional[float] = ...,
        right: Optional[str] = ...,
    ) -> OhlcTick: ...

    ms_of_day: int
    """Opening time of the bar, milliseconds since midnight ET."""
    open: float
    """Opening trade price."""
    high: float
    """Highest traded price."""
    low: float
    """Lowest traded price."""
    close: float
    """Closing traded price."""
    volume: int
    """Number of contracts or shares traded."""
    count: int
    """Number of trades."""
    vwap: float
    """Volume-weighted average price of the session."""
    date: int
    """Trading date as a YYYYMMDD integer."""
    expiration: Optional[int]
    """Expiration of the row's contract as a YYYYMMDD integer on a wildcard option response; `None` when the response names a single contract."""
    strike: Optional[float]
    """Strike of the row's contract in dollars on a wildcard option response; `None` when the response names a single contract."""
    right: Optional[str]
    """Right of the row's contract on a wildcard option response, `"C"` or `"P"`; `None` when the response names a single contract."""

    @property
    def timestamp_ms(self) -> Optional[int]:
        """Unix epoch milliseconds (UTC, DST-aware) combining `date` with `ms_of_day` (Eastern-Time milliseconds-of-day). `None` when `date` is absent (`0`)."""
        ...

    def __repr__(self) -> str: ...


@final
class OhlcTickList(_TickList[OhlcTick]):
    """Typed list of `OhlcTick` rows, returned by every market-data endpoint whose
    response is `OhlcTick`.
    """


@final
class OpenInterestTick:
    """Open interest tick."""

    def __new__(
        cls,
        *,
        ms_of_day: int = ...,
        open_interest: int = ...,
        date: int = ...,
        expiration: Optional[int] = ...,
        strike: Optional[float] = ...,
        right: Optional[str] = ...,
    ) -> OpenInterestTick: ...

    ms_of_day: int
    """Milliseconds since midnight Eastern Time."""
    open_interest: int
    """Total outstanding contracts."""
    date: int
    """Trading date as a YYYYMMDD integer."""
    expiration: Optional[int]
    """Expiration of the row's contract as a YYYYMMDD integer on a wildcard option response; `None` when the response names a single contract."""
    strike: Optional[float]
    """Strike of the row's contract in dollars on a wildcard option response; `None` when the response names a single contract."""
    right: Optional[str]
    """Right of the row's contract on a wildcard option response, `"C"` or `"P"`; `None` when the response names a single contract."""

    @property
    def timestamp_ms(self) -> Optional[int]:
        """Unix epoch milliseconds (UTC, DST-aware) combining `date` with `ms_of_day` (Eastern-Time milliseconds-of-day). `None` when `date` is absent (`0`)."""
        ...

    def __repr__(self) -> str: ...


@final
class OpenInterestTickList(_TickList[OpenInterestTick]):
    """Typed list of `OpenInterestTick` rows, returned by every market-data endpoint whose
    response is `OpenInterestTick`.
    """


@final
class OptionContract:
    """Option contract. Contract specification."""

    def __new__(
        cls,
        *,
        symbol: str = ...,
        expiration: int = ...,
        strike: float = ...,
        right: str = ...,
    ) -> OptionContract: ...

    symbol: str
    """Underlying symbol of the contract."""
    expiration: int
    """Expiration date of the contract as a YYYYMMDD integer."""
    strike: float
    """Strike price of the contract in dollars."""
    right: str
    """Contract right: 'C' for a call, 'P' for a put."""

    def __repr__(self) -> str: ...


@final
class OptionContractList(_TickList[OptionContract]):
    """Typed list of `OptionContract` rows, returned by every market-data endpoint whose
    response is `OptionContract`.
    """


@final
class PriceTick:
    """Price tick. Generic price data point."""

    def __new__(
        cls,
        *,
        ms_of_day: int = ...,
        price: float = ...,
        date: int = ...,
    ) -> PriceTick: ...

    ms_of_day: int
    """Milliseconds since midnight Eastern Time."""
    price: float
    """Index value."""
    date: int
    """Trading date as a YYYYMMDD integer."""

    @property
    def timestamp_ms(self) -> Optional[int]:
        """Unix epoch milliseconds (UTC, DST-aware) combining `date` with `ms_of_day` (Eastern-Time milliseconds-of-day). `None` when `date` is absent (`0`)."""
        ...

    def __repr__(self) -> str: ...


@final
class PriceTickList(_TickList[PriceTick]):
    """Typed list of `PriceTick` rows, returned by every market-data endpoint whose
    response is `PriceTick`.
    """


@final
class QuoteTick:
    """Quote tick. NBBO quote data.

    Some storage tiers return only `ms_of_day`, `bid_size`, `bid`, `ask_size`, `ask` and `date`; the exchange and condition fields are then absent.
    """

    def __new__(
        cls,
        *,
        ms_of_day: int = ...,
        bid_size: int = ...,
        bid_exchange: Optional[int] = ...,
        bid: float = ...,
        bid_condition: Optional[int] = ...,
        ask_size: int = ...,
        ask_exchange: Optional[int] = ...,
        ask: float = ...,
        ask_condition: Optional[int] = ...,
        date: int = ...,
        expiration: Optional[int] = ...,
        strike: Optional[float] = ...,
        right: Optional[str] = ...,
    ) -> QuoteTick: ...

    ms_of_day: int
    """Milliseconds since midnight Eastern Time."""
    bid_size: int
    """Last NBBO bid size."""
    bid_exchange: Optional[int]
    """Exchange code of the NBBO bid. `None` when the response carried no such column."""
    bid: float
    """Last NBBO bid price."""
    bid_condition: Optional[int]
    """Quote condition code on the bid side. `None` when the response carried no such column."""
    ask_size: int
    """Last NBBO ask size."""
    ask_exchange: Optional[int]
    """Exchange code of the NBBO ask. `None` when the response carried no such column."""
    ask: float
    """Last NBBO ask price."""
    ask_condition: Optional[int]
    """Quote condition code on the ask side. `None` when the response carried no such column."""
    date: int
    """Trading date as a YYYYMMDD integer."""
    expiration: Optional[int]
    """Expiration of the row's contract as a YYYYMMDD integer on a wildcard option response; `None` when the response names a single contract."""
    strike: Optional[float]
    """Strike of the row's contract in dollars on a wildcard option response; `None` when the response names a single contract."""
    right: Optional[str]
    """Right of the row's contract on a wildcard option response, `"C"` or `"P"`; `None` when the response names a single contract."""

    @property
    def timestamp_ms(self) -> Optional[int]:
        """Unix epoch milliseconds (UTC, DST-aware) combining `date` with `ms_of_day` (Eastern-Time milliseconds-of-day). `None` when `date` is absent (`0`)."""
        ...

    def __repr__(self) -> str: ...


@final
class QuoteTickList(_TickList[QuoteTick]):
    """Typed list of `QuoteTick` rows, returned by every market-data endpoint whose
    response is `QuoteTick`.
    """


@final
class TradeGreeksAllTick:
    """Per-trade union Greeks tick -- every Greek the v3 server publishes on `option_history_trade_greeks_all`, paired with the trade-side execution columns (`sequence`, `ext_condition1..4`, `condition`, `size`, `exchange`, `price`) that identify which OPRA print each Greek was calculated against."""

    def __new__(
        cls,
        *,
        ms_of_day: int = ...,
        sequence: int = ...,
        ext_condition1: Optional[int] = ...,
        ext_condition2: Optional[int] = ...,
        ext_condition3: Optional[int] = ...,
        ext_condition4: Optional[int] = ...,
        condition: Optional[int] = ...,
        size: int = ...,
        exchange: Optional[int] = ...,
        price: float = ...,
        delta: float = ...,
        theta: float = ...,
        vega: float = ...,
        rho: float = ...,
        epsilon: float = ...,
        lambda_: float = ...,
        gamma: float = ...,
        vanna: float = ...,
        charm: float = ...,
        vomma: float = ...,
        veta: float = ...,
        vera: float = ...,
        speed: float = ...,
        zomma: float = ...,
        color: float = ...,
        ultima: float = ...,
        d1: float = ...,
        d2: float = ...,
        dual_delta: float = ...,
        dual_gamma: float = ...,
        implied_volatility: float = ...,
        iv_error: float = ...,
        underlying_ms_of_day: int = ...,
        underlying_price: float = ...,
        date: int = ...,
        underlying_date: int = ...,
        expiration: Optional[int] = ...,
        strike: Optional[float] = ...,
        right: Optional[str] = ...,
    ) -> TradeGreeksAllTick: ...

    ms_of_day: int
    """Milliseconds since midnight Eastern Time."""
    sequence: int
    """Exchange-assigned trade sequence number."""
    ext_condition1: Optional[int]
    """Additional trade condition code. `None` when the response carried no such column."""
    ext_condition2: Optional[int]
    """Additional trade condition code. `None` when the response carried no such column."""
    ext_condition3: Optional[int]
    """Additional trade condition code. `None` when the response carried no such column."""
    ext_condition4: Optional[int]
    """Additional trade condition code. `None` when the response carried no such column."""
    condition: Optional[int]
    """Trade condition code. `None` when the response carried no such column."""
    size: int
    """Number of contracts or shares traded."""
    exchange: Optional[int]
    """Exchange code where the trade executed. `None` when the response carried no such column."""
    price: float
    """Trade price."""
    delta: float
    """Sensitivity of the option value to a one-dollar move in the underlying."""
    theta: float
    """Sensitivity of the option value to the passage of time."""
    vega: float
    """Sensitivity of the option value to a change in implied volatility."""
    rho: float
    """Sensitivity of the option value to the risk-free rate."""
    epsilon: float
    """Sensitivity of the option value to the dividend yield."""
    lambda_: float
    """Percent change in the option value per percent change in the underlying."""
    gamma: float
    """Rate of change of delta with respect to the underlying price."""
    vanna: float
    """Sensitivity of delta to a change in implied volatility."""
    charm: float
    """Rate of change of delta over time."""
    vomma: float
    """Sensitivity of vega to a change in implied volatility."""
    veta: float
    """Rate of change of vega over time."""
    vera: float
    """Sensitivity of rho to a change in implied volatility."""
    speed: float
    """Rate of change of gamma with respect to the underlying price."""
    zomma: float
    """Sensitivity of gamma to a change in implied volatility."""
    color: float
    """Rate of change of gamma over time."""
    ultima: float
    """Sensitivity of vomma to a change in implied volatility."""
    d1: float
    """Black-Scholes d1 intermediate term."""
    d2: float
    """Black-Scholes d2 intermediate term."""
    dual_delta: float
    """Sensitivity of the option value to the strike price."""
    dual_gamma: float
    """Second derivative of the option value with respect to the strike."""
    implied_volatility: float
    """Implied volatility solved from the option price. The vendor names this column `IMPLIED_VOL`."""
    iv_error: float
    """Residual pricing error of the implied-volatility solve."""
    underlying_ms_of_day: int
    """Timestamp of the underlying price, milliseconds since midnight ET. The vendor names this column `MS_OF_DAY2`."""
    underlying_price: float
    """Underlying price used in the calculation (midpoint of the underlying)."""
    date: int
    """Trading date as a YYYYMMDD integer."""
    underlying_date: int
    """Trading date of the underlying price as a YYYYMMDD integer, decoded from the same wire column as `underlying_ms_of_day`. It differs from `date` when the underlying price and the option quote come from different sessions."""
    expiration: Optional[int]
    """Expiration of the row's contract as a YYYYMMDD integer on a wildcard option response; `None` when the response names a single contract."""
    strike: Optional[float]
    """Strike of the row's contract in dollars on a wildcard option response; `None` when the response names a single contract."""
    right: Optional[str]
    """Right of the row's contract on a wildcard option response, `"C"` or `"P"`; `None` when the response names a single contract."""

    @property
    def timestamp_ms(self) -> Optional[int]:
        """Unix epoch milliseconds (UTC, DST-aware) combining `date` with `ms_of_day` (Eastern-Time milliseconds-of-day). `None` when `date` is absent (`0`)."""
        ...

    @property
    def underlying_timestamp_ms(self) -> Optional[int]:
        """Unix epoch milliseconds (UTC, DST-aware) combining `underlying_date` with `underlying_ms_of_day` (Eastern-Time milliseconds-of-day). `None` when `underlying_date` is absent (`0`)."""
        ...

    def __repr__(self) -> str: ...


@final
class TradeGreeksAllTickList(_TickList[TradeGreeksAllTick]):
    """Typed list of `TradeGreeksAllTick` rows, returned by every market-data endpoint whose
    response is `TradeGreeksAllTick`.
    """


@final
class TradeGreeksFirstOrderTick:
    """Per-trade first-order Greeks tick (delta / theta / vega / rho / epsilon / lambda) paired with the trade-side execution columns identifying the OPRA print each Greek was calculated against."""

    def __new__(
        cls,
        *,
        ms_of_day: int = ...,
        sequence: int = ...,
        ext_condition1: Optional[int] = ...,
        ext_condition2: Optional[int] = ...,
        ext_condition3: Optional[int] = ...,
        ext_condition4: Optional[int] = ...,
        condition: Optional[int] = ...,
        size: int = ...,
        exchange: Optional[int] = ...,
        price: float = ...,
        delta: float = ...,
        theta: float = ...,
        vega: float = ...,
        rho: float = ...,
        epsilon: float = ...,
        lambda_: float = ...,
        implied_volatility: float = ...,
        iv_error: float = ...,
        underlying_ms_of_day: int = ...,
        underlying_price: float = ...,
        date: int = ...,
        underlying_date: int = ...,
        expiration: Optional[int] = ...,
        strike: Optional[float] = ...,
        right: Optional[str] = ...,
    ) -> TradeGreeksFirstOrderTick: ...

    ms_of_day: int
    """Milliseconds since midnight Eastern Time."""
    sequence: int
    """Exchange-assigned trade sequence number."""
    ext_condition1: Optional[int]
    """Additional trade condition code. `None` when the response carried no such column."""
    ext_condition2: Optional[int]
    """Additional trade condition code. `None` when the response carried no such column."""
    ext_condition3: Optional[int]
    """Additional trade condition code. `None` when the response carried no such column."""
    ext_condition4: Optional[int]
    """Additional trade condition code. `None` when the response carried no such column."""
    condition: Optional[int]
    """Trade condition code. `None` when the response carried no such column."""
    size: int
    """Number of contracts or shares traded."""
    exchange: Optional[int]
    """Exchange code where the trade executed. `None` when the response carried no such column."""
    price: float
    """Trade price."""
    delta: float
    """Sensitivity of the option value to a one-dollar move in the underlying."""
    theta: float
    """Sensitivity of the option value to the passage of time."""
    vega: float
    """Sensitivity of the option value to a change in implied volatility."""
    rho: float
    """Sensitivity of the option value to the risk-free rate."""
    epsilon: float
    """Sensitivity of the option value to the dividend yield."""
    lambda_: float
    """Percent change in the option value per percent change in the underlying."""
    implied_volatility: float
    """Implied volatility solved from the option price. The vendor names this column `IMPLIED_VOL`."""
    iv_error: float
    """Residual pricing error of the implied-volatility solve."""
    underlying_ms_of_day: int
    """Timestamp of the underlying price, milliseconds since midnight ET. The vendor names this column `MS_OF_DAY2`."""
    underlying_price: float
    """Underlying price used in the calculation (midpoint of the underlying)."""
    date: int
    """Trading date as a YYYYMMDD integer."""
    underlying_date: int
    """Trading date of the underlying price as a YYYYMMDD integer, decoded from the same wire column as `underlying_ms_of_day`. It differs from `date` when the underlying price and the option quote come from different sessions."""
    expiration: Optional[int]
    """Expiration of the row's contract as a YYYYMMDD integer on a wildcard option response; `None` when the response names a single contract."""
    strike: Optional[float]
    """Strike of the row's contract in dollars on a wildcard option response; `None` when the response names a single contract."""
    right: Optional[str]
    """Right of the row's contract on a wildcard option response, `"C"` or `"P"`; `None` when the response names a single contract."""

    @property
    def timestamp_ms(self) -> Optional[int]:
        """Unix epoch milliseconds (UTC, DST-aware) combining `date` with `ms_of_day` (Eastern-Time milliseconds-of-day). `None` when `date` is absent (`0`)."""
        ...

    @property
    def underlying_timestamp_ms(self) -> Optional[int]:
        """Unix epoch milliseconds (UTC, DST-aware) combining `underlying_date` with `underlying_ms_of_day` (Eastern-Time milliseconds-of-day). `None` when `underlying_date` is absent (`0`)."""
        ...

    def __repr__(self) -> str: ...


@final
class TradeGreeksFirstOrderTickList(_TickList[TradeGreeksFirstOrderTick]):
    """Typed list of `TradeGreeksFirstOrderTick` rows, returned by every market-data endpoint whose
    response is `TradeGreeksFirstOrderTick`.
    """


@final
class TradeGreeksImpliedVolatilityTick:
    """Per-trade implied-volatility tick (single `implied_volatility` + `iv_error` pair, NOT the bid/mid/ask IV triple of the interval-sampled `IvTick`) paired with the trade-side execution columns identifying the OPRA print the IV was calculated against."""

    def __new__(
        cls,
        *,
        ms_of_day: int = ...,
        sequence: int = ...,
        ext_condition1: Optional[int] = ...,
        ext_condition2: Optional[int] = ...,
        ext_condition3: Optional[int] = ...,
        ext_condition4: Optional[int] = ...,
        condition: Optional[int] = ...,
        size: int = ...,
        exchange: Optional[int] = ...,
        price: float = ...,
        implied_volatility: float = ...,
        iv_error: float = ...,
        underlying_ms_of_day: int = ...,
        underlying_price: float = ...,
        date: int = ...,
        underlying_date: int = ...,
        expiration: Optional[int] = ...,
        strike: Optional[float] = ...,
        right: Optional[str] = ...,
    ) -> TradeGreeksImpliedVolatilityTick: ...

    ms_of_day: int
    """Milliseconds since midnight Eastern Time."""
    sequence: int
    """Exchange-assigned trade sequence number."""
    ext_condition1: Optional[int]
    """Additional trade condition code. `None` when the response carried no such column."""
    ext_condition2: Optional[int]
    """Additional trade condition code. `None` when the response carried no such column."""
    ext_condition3: Optional[int]
    """Additional trade condition code. `None` when the response carried no such column."""
    ext_condition4: Optional[int]
    """Additional trade condition code. `None` when the response carried no such column."""
    condition: Optional[int]
    """Trade condition code. `None` when the response carried no such column."""
    size: int
    """Number of contracts or shares traded."""
    exchange: Optional[int]
    """Exchange code where the trade executed. `None` when the response carried no such column."""
    price: float
    """Trade price."""
    implied_volatility: float
    """Implied volatility solved from the option price. The vendor names this column `IMPLIED_VOL`."""
    iv_error: float
    """Residual pricing error of the implied-volatility solve."""
    underlying_ms_of_day: int
    """Timestamp of the underlying price, milliseconds since midnight ET. The vendor names this column `MS_OF_DAY2`."""
    underlying_price: float
    """Underlying price used in the calculation (midpoint of the underlying)."""
    date: int
    """Trading date as a YYYYMMDD integer."""
    underlying_date: int
    """Trading date of the underlying price as a YYYYMMDD integer, decoded from the same wire column as `underlying_ms_of_day`. It differs from `date` when the underlying price and the option quote come from different sessions."""
    expiration: Optional[int]
    """Expiration of the row's contract as a YYYYMMDD integer on a wildcard option response; `None` when the response names a single contract."""
    strike: Optional[float]
    """Strike of the row's contract in dollars on a wildcard option response; `None` when the response names a single contract."""
    right: Optional[str]
    """Right of the row's contract on a wildcard option response, `"C"` or `"P"`; `None` when the response names a single contract."""

    @property
    def timestamp_ms(self) -> Optional[int]:
        """Unix epoch milliseconds (UTC, DST-aware) combining `date` with `ms_of_day` (Eastern-Time milliseconds-of-day). `None` when `date` is absent (`0`)."""
        ...

    @property
    def underlying_timestamp_ms(self) -> Optional[int]:
        """Unix epoch milliseconds (UTC, DST-aware) combining `underlying_date` with `underlying_ms_of_day` (Eastern-Time milliseconds-of-day). `None` when `underlying_date` is absent (`0`)."""
        ...

    def __repr__(self) -> str: ...


@final
class TradeGreeksImpliedVolatilityTickList(_TickList[TradeGreeksImpliedVolatilityTick]):
    """Typed list of `TradeGreeksImpliedVolatilityTick` rows, returned by every market-data endpoint whose
    response is `TradeGreeksImpliedVolatilityTick`.
    """


@final
class TradeGreeksSecondOrderTick:
    """Per-trade second-order Greeks tick (gamma / vanna / charm / vomma / veta) paired with the trade-side execution columns identifying the OPRA print each Greek was calculated against."""

    def __new__(
        cls,
        *,
        ms_of_day: int = ...,
        sequence: int = ...,
        ext_condition1: Optional[int] = ...,
        ext_condition2: Optional[int] = ...,
        ext_condition3: Optional[int] = ...,
        ext_condition4: Optional[int] = ...,
        condition: Optional[int] = ...,
        size: int = ...,
        exchange: Optional[int] = ...,
        price: float = ...,
        gamma: float = ...,
        vanna: float = ...,
        charm: float = ...,
        vomma: float = ...,
        veta: float = ...,
        implied_volatility: float = ...,
        iv_error: float = ...,
        underlying_ms_of_day: int = ...,
        underlying_price: float = ...,
        date: int = ...,
        underlying_date: int = ...,
        expiration: Optional[int] = ...,
        strike: Optional[float] = ...,
        right: Optional[str] = ...,
    ) -> TradeGreeksSecondOrderTick: ...

    ms_of_day: int
    """Milliseconds since midnight Eastern Time."""
    sequence: int
    """Exchange-assigned trade sequence number."""
    ext_condition1: Optional[int]
    """Additional trade condition code. `None` when the response carried no such column."""
    ext_condition2: Optional[int]
    """Additional trade condition code. `None` when the response carried no such column."""
    ext_condition3: Optional[int]
    """Additional trade condition code. `None` when the response carried no such column."""
    ext_condition4: Optional[int]
    """Additional trade condition code. `None` when the response carried no such column."""
    condition: Optional[int]
    """Trade condition code. `None` when the response carried no such column."""
    size: int
    """Number of contracts or shares traded."""
    exchange: Optional[int]
    """Exchange code where the trade executed. `None` when the response carried no such column."""
    price: float
    """Trade price."""
    gamma: float
    """Rate of change of delta with respect to the underlying price."""
    vanna: float
    """Sensitivity of delta to a change in implied volatility."""
    charm: float
    """Rate of change of delta over time."""
    vomma: float
    """Sensitivity of vega to a change in implied volatility."""
    veta: float
    """Rate of change of vega over time."""
    implied_volatility: float
    """Implied volatility solved from the option price. The vendor names this column `IMPLIED_VOL`."""
    iv_error: float
    """Residual pricing error of the implied-volatility solve."""
    underlying_ms_of_day: int
    """Timestamp of the underlying price, milliseconds since midnight ET. The vendor names this column `MS_OF_DAY2`."""
    underlying_price: float
    """Underlying price used in the calculation (midpoint of the underlying)."""
    date: int
    """Trading date as a YYYYMMDD integer."""
    underlying_date: int
    """Trading date of the underlying price as a YYYYMMDD integer, decoded from the same wire column as `underlying_ms_of_day`. It differs from `date` when the underlying price and the option quote come from different sessions."""
    expiration: Optional[int]
    """Expiration of the row's contract as a YYYYMMDD integer on a wildcard option response; `None` when the response names a single contract."""
    strike: Optional[float]
    """Strike of the row's contract in dollars on a wildcard option response; `None` when the response names a single contract."""
    right: Optional[str]
    """Right of the row's contract on a wildcard option response, `"C"` or `"P"`; `None` when the response names a single contract."""

    @property
    def timestamp_ms(self) -> Optional[int]:
        """Unix epoch milliseconds (UTC, DST-aware) combining `date` with `ms_of_day` (Eastern-Time milliseconds-of-day). `None` when `date` is absent (`0`)."""
        ...

    @property
    def underlying_timestamp_ms(self) -> Optional[int]:
        """Unix epoch milliseconds (UTC, DST-aware) combining `underlying_date` with `underlying_ms_of_day` (Eastern-Time milliseconds-of-day). `None` when `underlying_date` is absent (`0`)."""
        ...

    def __repr__(self) -> str: ...


@final
class TradeGreeksSecondOrderTickList(_TickList[TradeGreeksSecondOrderTick]):
    """Typed list of `TradeGreeksSecondOrderTick` rows, returned by every market-data endpoint whose
    response is `TradeGreeksSecondOrderTick`.
    """


@final
class TradeGreeksThirdOrderTick:
    """Per-trade third-order Greeks tick (speed / zomma / color / ultima) paired with the trade-side execution columns identifying the OPRA print each Greek was calculated against. The vendor's third-order schema does not publish `vera`."""

    def __new__(
        cls,
        *,
        ms_of_day: int = ...,
        sequence: int = ...,
        ext_condition1: Optional[int] = ...,
        ext_condition2: Optional[int] = ...,
        ext_condition3: Optional[int] = ...,
        ext_condition4: Optional[int] = ...,
        condition: Optional[int] = ...,
        size: int = ...,
        exchange: Optional[int] = ...,
        price: float = ...,
        speed: float = ...,
        zomma: float = ...,
        color: float = ...,
        ultima: float = ...,
        implied_volatility: float = ...,
        iv_error: float = ...,
        underlying_ms_of_day: int = ...,
        underlying_price: float = ...,
        date: int = ...,
        underlying_date: int = ...,
        expiration: Optional[int] = ...,
        strike: Optional[float] = ...,
        right: Optional[str] = ...,
    ) -> TradeGreeksThirdOrderTick: ...

    ms_of_day: int
    """Milliseconds since midnight Eastern Time."""
    sequence: int
    """Exchange-assigned trade sequence number."""
    ext_condition1: Optional[int]
    """Additional trade condition code. `None` when the response carried no such column."""
    ext_condition2: Optional[int]
    """Additional trade condition code. `None` when the response carried no such column."""
    ext_condition3: Optional[int]
    """Additional trade condition code. `None` when the response carried no such column."""
    ext_condition4: Optional[int]
    """Additional trade condition code. `None` when the response carried no such column."""
    condition: Optional[int]
    """Trade condition code. `None` when the response carried no such column."""
    size: int
    """Number of contracts or shares traded."""
    exchange: Optional[int]
    """Exchange code where the trade executed. `None` when the response carried no such column."""
    price: float
    """Trade price."""
    speed: float
    """Rate of change of gamma with respect to the underlying price."""
    zomma: float
    """Sensitivity of gamma to a change in implied volatility."""
    color: float
    """Rate of change of gamma over time."""
    ultima: float
    """Sensitivity of vomma to a change in implied volatility."""
    implied_volatility: float
    """Implied volatility solved from the option price. The vendor names this column `IMPLIED_VOL`."""
    iv_error: float
    """Residual pricing error of the implied-volatility solve."""
    underlying_ms_of_day: int
    """Timestamp of the underlying price, milliseconds since midnight ET. The vendor names this column `MS_OF_DAY2`."""
    underlying_price: float
    """Underlying price used in the calculation (midpoint of the underlying)."""
    date: int
    """Trading date as a YYYYMMDD integer."""
    underlying_date: int
    """Trading date of the underlying price as a YYYYMMDD integer, decoded from the same wire column as `underlying_ms_of_day`. It differs from `date` when the underlying price and the option quote come from different sessions."""
    expiration: Optional[int]
    """Expiration of the row's contract as a YYYYMMDD integer on a wildcard option response; `None` when the response names a single contract."""
    strike: Optional[float]
    """Strike of the row's contract in dollars on a wildcard option response; `None` when the response names a single contract."""
    right: Optional[str]
    """Right of the row's contract on a wildcard option response, `"C"` or `"P"`; `None` when the response names a single contract."""

    @property
    def timestamp_ms(self) -> Optional[int]:
        """Unix epoch milliseconds (UTC, DST-aware) combining `date` with `ms_of_day` (Eastern-Time milliseconds-of-day). `None` when `date` is absent (`0`)."""
        ...

    @property
    def underlying_timestamp_ms(self) -> Optional[int]:
        """Unix epoch milliseconds (UTC, DST-aware) combining `underlying_date` with `underlying_ms_of_day` (Eastern-Time milliseconds-of-day). `None` when `underlying_date` is absent (`0`)."""
        ...

    def __repr__(self) -> str: ...


@final
class TradeGreeksThirdOrderTickList(_TickList[TradeGreeksThirdOrderTick]):
    """Typed list of `TradeGreeksThirdOrderTick` rows, returned by every market-data endpoint whose
    response is `TradeGreeksThirdOrderTick`.
    """


@final
class TradeQuoteTick:
    """Combined trade + quote tick."""

    def __new__(
        cls,
        *,
        ms_of_day: int = ...,
        sequence: int = ...,
        ext_condition1: Optional[int] = ...,
        ext_condition2: Optional[int] = ...,
        ext_condition3: Optional[int] = ...,
        ext_condition4: Optional[int] = ...,
        condition: Optional[int] = ...,
        size: int = ...,
        exchange: Optional[int] = ...,
        price: float = ...,
        condition_flags: int = ...,
        price_flags: int = ...,
        volume_type: int = ...,
        records_back: int = ...,
        quote_ms_of_day: int = ...,
        bid_size: int = ...,
        bid_exchange: Optional[int] = ...,
        bid: float = ...,
        bid_condition: Optional[int] = ...,
        ask_size: int = ...,
        ask_exchange: Optional[int] = ...,
        ask: float = ...,
        ask_condition: Optional[int] = ...,
        date: int = ...,
        expiration: Optional[int] = ...,
        strike: Optional[float] = ...,
        right: Optional[str] = ...,
    ) -> TradeQuoteTick: ...

    ms_of_day: int
    """Milliseconds since midnight Eastern Time."""
    sequence: int
    """Exchange-assigned trade sequence number."""
    ext_condition1: Optional[int]
    """Additional trade condition code. `None` when the response carried no such column."""
    ext_condition2: Optional[int]
    """Additional trade condition code. `None` when the response carried no such column."""
    ext_condition3: Optional[int]
    """Additional trade condition code. `None` when the response carried no such column."""
    ext_condition4: Optional[int]
    """Additional trade condition code. `None` when the response carried no such column."""
    condition: Optional[int]
    """Trade condition code. `None` when the response carried no such column."""
    size: int
    """Number of contracts or shares traded."""
    exchange: Optional[int]
    """Exchange code where the trade executed. `None` when the response carried no such column."""
    price: float
    """Trade price."""
    condition_flags: int
    """Trade condition flags bitmap."""
    price_flags: int
    """Trade price flags bitmap."""
    volume_type: int
    """Volume reporting mode, as sent. The vendor does not define this column."""
    records_back: int
    """Offset of this record behind the most recent record."""
    quote_ms_of_day: int
    """Timestamp of the paired quote, milliseconds since midnight ET. The vendor names this column `MS_OF_DAY2`."""
    bid_size: int
    """Last NBBO bid size."""
    bid_exchange: Optional[int]
    """Exchange code of the NBBO bid. `None` when the response carried no such column."""
    bid: float
    """NBBO bid at the time of the trade."""
    bid_condition: Optional[int]
    """Quote condition code on the bid side. `None` when the response carried no such column."""
    ask_size: int
    """Last NBBO ask size."""
    ask_exchange: Optional[int]
    """Exchange code of the NBBO ask. `None` when the response carried no such column."""
    ask: float
    """NBBO ask at the time of the trade."""
    ask_condition: Optional[int]
    """Quote condition code on the ask side. `None` when the response carried no such column."""
    date: int
    """Trading date as a YYYYMMDD integer."""
    expiration: Optional[int]
    """Expiration of the row's contract as a YYYYMMDD integer on a wildcard option response; `None` when the response names a single contract."""
    strike: Optional[float]
    """Strike of the row's contract in dollars on a wildcard option response; `None` when the response names a single contract."""
    right: Optional[str]
    """Right of the row's contract on a wildcard option response, `"C"` or `"P"`; `None` when the response names a single contract."""

    @property
    def timestamp_ms(self) -> Optional[int]:
        """Unix epoch milliseconds (UTC, DST-aware) combining `date` with `ms_of_day` (Eastern-Time milliseconds-of-day). `None` when `date` is absent (`0`)."""
        ...

    @property
    def quote_timestamp_ms(self) -> Optional[int]:
        """Unix epoch milliseconds (UTC, DST-aware) combining `date` with `quote_ms_of_day` (Eastern-Time milliseconds-of-day). `None` when `date` is absent (`0`)."""
        ...

    def __repr__(self) -> str: ...


@final
class TradeQuoteTickList(_TickList[TradeQuoteTick]):
    """Typed list of `TradeQuoteTick` rows, returned by every market-data endpoint whose
    response is `TradeQuoteTick`.
    """


@final
class TradeTick:
    """Trade tick. Core unit of trade data."""

    def __new__(
        cls,
        *,
        ms_of_day: int = ...,
        sequence: int = ...,
        ext_condition1: Optional[int] = ...,
        ext_condition2: Optional[int] = ...,
        ext_condition3: Optional[int] = ...,
        ext_condition4: Optional[int] = ...,
        condition: Optional[int] = ...,
        size: int = ...,
        exchange: Optional[int] = ...,
        price: float = ...,
        condition_flags: int = ...,
        price_flags: int = ...,
        volume_type: int = ...,
        records_back: int = ...,
        date: int = ...,
        expiration: Optional[int] = ...,
        strike: Optional[float] = ...,
        right: Optional[str] = ...,
    ) -> TradeTick: ...

    ms_of_day: int
    """Milliseconds since midnight Eastern Time."""
    sequence: int
    """Exchange-assigned trade sequence number."""
    ext_condition1: Optional[int]
    """Additional trade condition code. `None` when the response carried no such column."""
    ext_condition2: Optional[int]
    """Additional trade condition code. `None` when the response carried no such column."""
    ext_condition3: Optional[int]
    """Additional trade condition code. `None` when the response carried no such column."""
    ext_condition4: Optional[int]
    """Additional trade condition code. `None` when the response carried no such column."""
    condition: Optional[int]
    """Trade condition code. `None` when the response carried no such column."""
    size: int
    """Number of contracts or shares traded."""
    exchange: Optional[int]
    """Exchange code where the trade executed. `None` when the response carried no such column."""
    price: float
    """Trade price."""
    condition_flags: int
    """Trade condition flags bitmap."""
    price_flags: int
    """Trade price flags bitmap."""
    volume_type: int
    """Volume reporting mode, as sent. The vendor does not define this column."""
    records_back: int
    """Offset of this record behind the most recent record."""
    date: int
    """Trading date as a YYYYMMDD integer."""
    expiration: Optional[int]
    """Expiration of the row's contract as a YYYYMMDD integer on a wildcard option response; `None` when the response names a single contract."""
    strike: Optional[float]
    """Strike of the row's contract in dollars on a wildcard option response; `None` when the response names a single contract."""
    right: Optional[str]
    """Right of the row's contract on a wildcard option response, `"C"` or `"P"`; `None` when the response names a single contract."""

    @property
    def timestamp_ms(self) -> Optional[int]:
        """Unix epoch milliseconds (UTC, DST-aware) combining `date` with `ms_of_day` (Eastern-Time milliseconds-of-day). `None` when `date` is absent (`0`)."""
        ...

    @property
    def is_cancelled(self) -> bool:
        """True when the trade carries a cancelled-trade condition (codes 40-44)."""
        ...

    def __repr__(self) -> str: ...


@final
class TradeTickList(_TickList[TradeTick]):
    """Typed list of `TradeTick` rows, returned by every market-data endpoint whose
    response is `TradeTick`.
    """


# --- END GENERATED TICK CLASSES ---


# ─────────────────────────────────────────────────────────────────────
# Streaming clients
# ─────────────────────────────────────────────────────────────────────

EventCallback = Callable[[Any], None]


# --- BEGIN GENERATED MARKET DATA VIEW (endpoint table) ---
# Generated from the endpoint table; do not edit by hand. Run
# `cargo run -p xtask --bin generate_sdk_surfaces` to refresh.

@final
class StringList:
    """The strings a list endpoint returns. Supports `len()`, indexing, slicing and
    iteration, and converts with `.to_list()`, `.to_arrow()`, `.to_pandas()` and
    `.to_polars()` into one column named for what the list holds.
    """

    def __len__(self) -> int: ...
    def __bool__(self) -> bool: ...
    def __repr__(self) -> str: ...
    @overload
    def __getitem__(self, key: int) -> str: ...
    @overload
    def __getitem__(self, key: slice) -> StringList: ...
    def __iter__(self) -> Iterator[str]: ...
    def to_list(self) -> List[str]:
        """Return a plain Python `list` of the strings."""
        ...

    def to_arrow(self) -> Any:
        """Return a one-column `pyarrow.Table` of the strings."""
        ...

    def to_pandas(self) -> Any:
        """Return a one-column `pandas.DataFrame` of the strings. Requires pandas and pyarrow."""
        ...

    def to_polars(self) -> Any:
        """Return a one-column `polars.DataFrame` of the strings. Requires polars and pyarrow."""
        ...


_Result = TypeVar("_Result")


class _Builder(Generic[_Result]):
    """Chain the setters, then run the request with `.list()` or `.list_async()`."""

    def timeout_ms(self, value: int) -> Self:
        """Apply a per-request deadline in milliseconds."""
        ...

    def list(self) -> _Result:
        """Run the request and return its result."""
        ...

    def list_async(self) -> Awaitable[_Result]:
        """Awaitable companion of `list()`."""
        ...


class _ChunkedBuilder(_Builder[_Result]):
    """A builder whose response can also be streamed chunk by chunk."""

    def stream(self, handler: Callable[[_Result], object]) -> None:
        """Call `handler` once per response chunk, with the chunk's rows in the typed
        list a whole result returns, without holding the full response in memory.
        An exception raised by `handler` stops the stream and is raised from this call.
        """
        ...

    def stream_async(self, handler: Callable[[_Result], object]) -> Awaitable[None]:
        """Awaitable companion of `stream()`, resolving to `None` once the response
        finishes. `handler` runs on a worker thread, one chunk at a time; cancelling
        the awaitable fetches no further chunks.
        """
        ...


@final
class StockListSymbolsBuilder(_Builder[StringList]):
    """Fluent builder for :meth:`MarketDataView.stock_list_symbols`."""


@final
class StockListDatesBuilder(_Builder[StringList]):
    """Fluent builder for :meth:`MarketDataView.stock_list_dates`."""

    def request_type(self, value: Union[str, RequestType]) -> Self:
        """Request type. Accepted values: `trade`, `quote`, `eod`, `ohlc`."""
        ...

    def symbol(self, value: str) -> Self:
        """Ticker symbol (e.g. AAPL)"""
        ...


@final
class StockSnapshotOhlcBuilder(_Builder[OhlcTickList]):
    """Fluent builder for :meth:`MarketDataView.stock_snapshot_ohlc`."""

    def symbols(self, value: Union[str, Sequence[str]]) -> Self:
        """Comma-separated ticker symbols (e.g. AAPL,MSFT)"""
        ...

    def venue(self, value: Union[str, Venue]) -> Self:
        """Venue/exchange filter. Accepted values: `nqb`, `utp_cta`."""
        ...

    def min_time(self, value: Union[str, time, datetime]) -> Self:
        """Minimum time filter"""
        ...

    def current_day_only(self, value: bool) -> Self:
        """When true, only snapshot data from the current trading day (New York date) is returned. Applied before `min_time`. Defaults to false, matching the terminal, which sends false when the value is omitted."""
        ...


@final
class StockSnapshotTradeBuilder(_Builder[TradeTickList]):
    """Fluent builder for :meth:`MarketDataView.stock_snapshot_trade`."""

    def symbols(self, value: Union[str, Sequence[str]]) -> Self:
        """Comma-separated ticker symbols (e.g. AAPL,MSFT)"""
        ...

    def venue(self, value: Union[str, Venue]) -> Self:
        """Venue/exchange filter. Accepted values: `nqb`, `utp_cta`."""
        ...

    def min_time(self, value: Union[str, time, datetime]) -> Self:
        """Minimum time filter"""
        ...

    def current_day_only(self, value: bool) -> Self:
        """When true, only snapshot data from the current trading day (New York date) is returned. Applied before `min_time`. Defaults to false, matching the terminal, which sends false when the value is omitted."""
        ...


@final
class StockSnapshotQuoteBuilder(_Builder[QuoteTickList]):
    """Fluent builder for :meth:`MarketDataView.stock_snapshot_quote`."""

    def symbols(self, value: Union[str, Sequence[str]]) -> Self:
        """Comma-separated ticker symbols (e.g. AAPL,MSFT)"""
        ...

    def venue(self, value: Union[str, Venue]) -> Self:
        """Venue/exchange filter. Accepted values: `nqb`, `utp_cta`."""
        ...

    def min_time(self, value: Union[str, time, datetime]) -> Self:
        """Minimum time filter"""
        ...

    def current_day_only(self, value: bool) -> Self:
        """When true, only snapshot data from the current trading day (New York date) is returned. Applied before `min_time`. Defaults to false, matching the terminal, which sends false when the value is omitted."""
        ...


@final
class StockSnapshotMarketValueBuilder(_Builder[MarketValueTickList]):
    """Fluent builder for :meth:`MarketDataView.stock_snapshot_market_value`."""

    def symbols(self, value: Union[str, Sequence[str]]) -> Self:
        """Comma-separated ticker symbols (e.g. AAPL,MSFT)"""
        ...

    def venue(self, value: Union[str, Venue]) -> Self:
        """Venue/exchange filter. Accepted values: `nqb`, `utp_cta`."""
        ...

    def min_time(self, value: Union[str, time, datetime]) -> Self:
        """Minimum time filter"""
        ...

    def current_day_only(self, value: bool) -> Self:
        """When true, only snapshot data from the current trading day (New York date) is returned. Applied before `min_time`. Defaults to false, matching the terminal, which sends false when the value is omitted."""
        ...


@final
class StockHistoryEodBuilder(_ChunkedBuilder[EodTickList]):
    """Fluent builder for :meth:`MarketDataView.stock_history_eod`."""

    def symbol(self, value: str) -> Self:
        """Ticker symbol (e.g. AAPL)"""
        ...

    def start_date(self, value: Union[str, _date, datetime]) -> Self:
        """Start date YYYYMMDD"""
        ...

    def end_date(self, value: Union[str, _date, datetime]) -> Self:
        """End date YYYYMMDD"""
        ...


@final
class StockHistoryOhlcBuilder(_ChunkedBuilder[OhlcTickList]):
    """Fluent builder for :meth:`MarketDataView.stock_history_ohlc`."""

    def symbol(self, value: str) -> Self:
        """Ticker symbol (e.g. AAPL)"""
        ...

    def date(self, value: Union[str, _date, datetime]) -> Self:
        """Single date YYYYMMDD. Supply this for a single-day pull, or supply `start_date`/`end_date` for a range. When present, `date` takes precedence over the range."""
        ...

    def interval(self, value: Union[str, Interval]) -> Self:
        """Interval preset. Defaults to `1s` when omitted, matching the upstream ThetaData Python library. Accepted values: `tick`, `10ms`, `100ms`, `500ms`, `1s`, `5s`, `10s`, `15s`, `30s`, `1m`, `5m`, `10m`, `15m`, `30m`, `1h`."""
        ...

    def start_time(self, value: Union[str, time, datetime]) -> Self:
        """Start time filter"""
        ...

    def end_time(self, value: Union[str, time, datetime]) -> Self:
        """End time filter"""
        ...

    def venue(self, value: Union[str, Venue]) -> Self:
        """Venue/exchange filter. Accepted values: `nqb`, `utp_cta`."""
        ...

    def start_date(self, value: Union[str, _date, datetime]) -> Self:
        """Start date YYYYMMDD"""
        ...

    def end_date(self, value: Union[str, _date, datetime]) -> Self:
        """End date YYYYMMDD"""
        ...


@final
class StockHistoryTradeBuilder(_ChunkedBuilder[TradeTickList]):
    """Fluent builder for :meth:`MarketDataView.stock_history_trade`."""

    def symbol(self, value: str) -> Self:
        """Ticker symbol (e.g. AAPL)"""
        ...

    def date(self, value: Union[str, _date, datetime]) -> Self:
        """Single date YYYYMMDD. Supply this for a single-day pull, or supply `start_date`/`end_date` for a range. When present, `date` takes precedence over the range."""
        ...

    def start_time(self, value: Union[str, time, datetime]) -> Self:
        """Start time filter"""
        ...

    def end_time(self, value: Union[str, time, datetime]) -> Self:
        """End time filter"""
        ...

    def venue(self, value: Union[str, Venue]) -> Self:
        """Venue/exchange filter. Accepted values: `nqb`, `utp_cta`."""
        ...

    def start_date(self, value: Union[str, _date, datetime]) -> Self:
        """Start date YYYYMMDD"""
        ...

    def end_date(self, value: Union[str, _date, datetime]) -> Self:
        """End date YYYYMMDD"""
        ...


@final
class StockHistoryQuoteBuilder(_ChunkedBuilder[QuoteTickList]):
    """Fluent builder for :meth:`MarketDataView.stock_history_quote`."""

    def symbol(self, value: str) -> Self:
        """Ticker symbol (e.g. AAPL)"""
        ...

    def date(self, value: Union[str, _date, datetime]) -> Self:
        """Single date YYYYMMDD. Supply this for a single-day pull, or supply `start_date`/`end_date` for a range. When present, `date` takes precedence over the range."""
        ...

    def interval(self, value: Union[str, Interval]) -> Self:
        """Interval preset. Defaults to `1s` when omitted, matching the upstream ThetaData Python library. Accepted values: `tick`, `10ms`, `100ms`, `500ms`, `1s`, `5s`, `10s`, `15s`, `30s`, `1m`, `5m`, `10m`, `15m`, `30m`, `1h`."""
        ...

    def start_time(self, value: Union[str, time, datetime]) -> Self:
        """Start time filter"""
        ...

    def end_time(self, value: Union[str, time, datetime]) -> Self:
        """End time filter"""
        ...

    def venue(self, value: Union[str, Venue]) -> Self:
        """Venue/exchange filter. Accepted values: `nqb`, `utp_cta`."""
        ...

    def start_date(self, value: Union[str, _date, datetime]) -> Self:
        """Start date YYYYMMDD"""
        ...

    def end_date(self, value: Union[str, _date, datetime]) -> Self:
        """End date YYYYMMDD"""
        ...


@final
class StockHistoryTradeQuoteBuilder(_ChunkedBuilder[TradeQuoteTickList]):
    """Fluent builder for :meth:`MarketDataView.stock_history_trade_quote`."""

    def symbol(self, value: str) -> Self:
        """Ticker symbol (e.g. AAPL)"""
        ...

    def date(self, value: Union[str, _date, datetime]) -> Self:
        """Single date YYYYMMDD. Supply this for a single-day pull, or supply `start_date`/`end_date` for a range. When present, `date` takes precedence over the range."""
        ...

    def start_time(self, value: Union[str, time, datetime]) -> Self:
        """Start time filter"""
        ...

    def end_time(self, value: Union[str, time, datetime]) -> Self:
        """End time filter"""
        ...

    def exclusive(self, value: bool) -> Self:
        """When true, quotes whose timestamp equals the trade timestamp are excluded; only quotes strictly before the trade are paired. Defaults to true, matching the terminal, which injects exclusive=true when the value is omitted."""
        ...

    def venue(self, value: Union[str, Venue]) -> Self:
        """Venue/exchange filter. Accepted values: `nqb`, `utp_cta`."""
        ...

    def start_date(self, value: Union[str, _date, datetime]) -> Self:
        """Start date YYYYMMDD"""
        ...

    def end_date(self, value: Union[str, _date, datetime]) -> Self:
        """End date YYYYMMDD"""
        ...

    def use_time_based_trade_quote(self, value: bool) -> Self:
        """Affects requests for current-day data only. When true, each trade is matched with a quote by millisecond timestamp. Defaults to true, matching the terminal, which sends true when the value is omitted."""
        ...


@final
class StockAtTimeTradeBuilder(_ChunkedBuilder[TradeTickList]):
    """Fluent builder for :meth:`MarketDataView.stock_at_time_trade`."""

    def symbol(self, value: str) -> Self:
        """Ticker symbol (e.g. AAPL)"""
        ...

    def start_date(self, value: Union[str, _date, datetime]) -> Self:
        """Start date YYYYMMDD"""
        ...

    def end_date(self, value: Union[str, _date, datetime]) -> Self:
        """End date YYYYMMDD"""
        ...

    def time_of_day(self, value: Union[str, time, datetime]) -> Self:
        """ET wall-clock time in HH:MM:SS.SSS (e.g. 09:30:00.000 for 9:30 AM ET; legacy 34200000 is also accepted)"""
        ...

    def venue(self, value: Union[str, Venue]) -> Self:
        """Venue/exchange filter. Accepted values: `nqb`, `utp_cta`."""
        ...


@final
class StockAtTimeQuoteBuilder(_ChunkedBuilder[QuoteTickList]):
    """Fluent builder for :meth:`MarketDataView.stock_at_time_quote`."""

    def symbol(self, value: str) -> Self:
        """Ticker symbol (e.g. AAPL)"""
        ...

    def start_date(self, value: Union[str, _date, datetime]) -> Self:
        """Start date YYYYMMDD"""
        ...

    def end_date(self, value: Union[str, _date, datetime]) -> Self:
        """End date YYYYMMDD"""
        ...

    def time_of_day(self, value: Union[str, time, datetime]) -> Self:
        """ET wall-clock time in HH:MM:SS.SSS (e.g. 09:30:00.000 for 9:30 AM ET; legacy 34200000 is also accepted)"""
        ...

    def venue(self, value: Union[str, Venue]) -> Self:
        """Venue/exchange filter. Accepted values: `nqb`, `utp_cta`."""
        ...


@final
class StockFlatFileEodBuilder(_ChunkedBuilder[EodTickList]):
    """Fluent builder for :meth:`MarketDataView.stock_flat_file_eod`."""

    def date(self, value: Union[str, _date, datetime]) -> Self:
        """Date YYYYMMDD"""
        ...


@final
class StockFlatFileTradeQuoteBuilder(_ChunkedBuilder[TradeQuoteTickList]):
    """Fluent builder for :meth:`MarketDataView.stock_flat_file_trade_quote`."""

    def date(self, value: Union[str, _date, datetime]) -> Self:
        """Date YYYYMMDD"""
        ...


@final
class StockFlatFileQuoteBuilder(_ChunkedBuilder[IntervalQuoteTickList]):
    """Fluent builder for :meth:`MarketDataView.stock_flat_file_quote`."""

    def date(self, value: Union[str, _date, datetime]) -> Self:
        """Date YYYYMMDD"""
        ...


@final
class OptionListSymbolsBuilder(_Builder[StringList]):
    """Fluent builder for :meth:`MarketDataView.option_list_symbols`."""


@final
class OptionListDatesBuilder(_Builder[StringList]):
    """Fluent builder for :meth:`MarketDataView.option_list_dates`."""

    def request_type(self, value: Union[str, RequestType]) -> Self:
        """Request type. Accepted values: `trade`, `quote`, `eod`, `ohlc`."""
        ...

    def symbol(self, value: str) -> Self:
        """Ticker symbol (e.g. AAPL)"""
        ...

    def expiration(self, value: Union[str, _date, datetime]) -> Self:
        """Expiration date YYYYMMDD"""
        ...


@final
class OptionListExpirationsBuilder(_Builder[StringList]):
    """Fluent builder for :meth:`MarketDataView.option_list_expirations`."""

    def symbol(self, value: str) -> Self:
        """Ticker symbol (e.g. AAPL)"""
        ...


@final
class OptionListStrikesBuilder(_Builder[StringList]):
    """Fluent builder for :meth:`MarketDataView.option_list_strikes`."""

    def symbol(self, value: str) -> Self:
        """Ticker symbol (e.g. AAPL)"""
        ...

    def expiration(self, value: Union[str, _date, datetime]) -> Self:
        """Expiration date YYYYMMDD"""
        ...


@final
class OptionListContractsBuilder(_ChunkedBuilder[OptionContractList]):
    """Fluent builder for :meth:`MarketDataView.option_list_contracts`."""

    def request_type(self, value: Union[str, RequestType]) -> Self:
        """Request type. Accepted values: `trade`, `quote`, `eod`, `ohlc`."""
        ...

    def symbol(self, value: str) -> Self:
        """Ticker symbol to filter by (e.g. AAPL), or a comma-separated list (AAPL,SPY) for the contracts of each. Omit to list every contract for the date."""
        ...

    def date(self, value: Union[str, _date, datetime]) -> Self:
        """Date YYYYMMDD"""
        ...

    def max_dte(self, value: int) -> Self:
        """Maximum days to expiration"""
        ...


@final
class OptionSnapshotOhlcBuilder(_Builder[OhlcTickList]):
    """Fluent builder for :meth:`MarketDataView.option_snapshot_ohlc`."""

    def symbol(self, value: str) -> Self:
        """Ticker symbol (e.g. AAPL)"""
        ...

    def expiration(self, value: Union[str, _date, datetime]) -> Self:
        """Expiration date YYYYMMDD"""
        ...

    def strike(self, value: str) -> Self:
        """Strike price in dollars as a string (e.g. 500 or 17.5). Use `*` for wildcard selection."""
        ...

    def right(self, value: Union[str, Right]) -> Self:
        """Option side. Use `both` or `*` (alias) for calls and puts. Accepted values: `call`, `put`, `both`, `*`."""
        ...

    def max_dte(self, value: int) -> Self:
        """Maximum days to expiration"""
        ...

    def strike_range(self, value: int) -> Self:
        """Strike range filter"""
        ...

    def min_time(self, value: Union[str, time, datetime]) -> Self:
        """Minimum time filter"""
        ...

    def current_day_only(self, value: bool) -> Self:
        """When true, only snapshot data from the current trading day (New York date) is returned. Applied before `min_time`. Defaults to false, matching the terminal, which sends false when the value is omitted."""
        ...


@final
class OptionSnapshotTradeBuilder(_Builder[TradeTickList]):
    """Fluent builder for :meth:`MarketDataView.option_snapshot_trade`."""

    def symbol(self, value: str) -> Self:
        """Ticker symbol (e.g. AAPL)"""
        ...

    def expiration(self, value: Union[str, _date, datetime]) -> Self:
        """Expiration date YYYYMMDD"""
        ...

    def strike(self, value: str) -> Self:
        """Strike price in dollars as a string (e.g. 500 or 17.5). Use `*` for wildcard selection."""
        ...

    def right(self, value: Union[str, Right]) -> Self:
        """Option side. Use `both` or `*` (alias) for calls and puts. Accepted values: `call`, `put`, `both`, `*`."""
        ...

    def strike_range(self, value: int) -> Self:
        """Strike range filter"""
        ...

    def min_time(self, value: Union[str, time, datetime]) -> Self:
        """Minimum time filter"""
        ...

    def current_day_only(self, value: bool) -> Self:
        """When true, only snapshot data from the current trading day (New York date) is returned. Applied before `min_time`. Defaults to false, matching the terminal, which sends false when the value is omitted."""
        ...


@final
class OptionSnapshotQuoteBuilder(_Builder[QuoteTickList]):
    """Fluent builder for :meth:`MarketDataView.option_snapshot_quote`."""

    def symbol(self, value: str) -> Self:
        """Ticker symbol (e.g. AAPL)"""
        ...

    def expiration(self, value: Union[str, _date, datetime]) -> Self:
        """Expiration date YYYYMMDD"""
        ...

    def strike(self, value: str) -> Self:
        """Strike price in dollars as a string (e.g. 500 or 17.5). Use `*` for wildcard selection."""
        ...

    def right(self, value: Union[str, Right]) -> Self:
        """Option side. Use `both` or `*` (alias) for calls and puts. Accepted values: `call`, `put`, `both`, `*`."""
        ...

    def max_dte(self, value: int) -> Self:
        """Maximum days to expiration"""
        ...

    def strike_range(self, value: int) -> Self:
        """Strike range filter"""
        ...

    def min_time(self, value: Union[str, time, datetime]) -> Self:
        """Minimum time filter"""
        ...

    def current_day_only(self, value: bool) -> Self:
        """When true, only snapshot data from the current trading day (New York date) is returned. Applied before `min_time`. Defaults to false, matching the terminal, which sends false when the value is omitted."""
        ...


@final
class OptionSnapshotOpenInterestBuilder(_Builder[OpenInterestTickList]):
    """Fluent builder for :meth:`MarketDataView.option_snapshot_open_interest`."""

    def symbol(self, value: str) -> Self:
        """Ticker symbol (e.g. AAPL)"""
        ...

    def expiration(self, value: Union[str, _date, datetime]) -> Self:
        """Expiration date YYYYMMDD"""
        ...

    def strike(self, value: str) -> Self:
        """Strike price in dollars as a string (e.g. 500 or 17.5). Use `*` for wildcard selection."""
        ...

    def right(self, value: Union[str, Right]) -> Self:
        """Option side. Use `both` or `*` (alias) for calls and puts. Accepted values: `call`, `put`, `both`, `*`."""
        ...

    def max_dte(self, value: int) -> Self:
        """Maximum days to expiration"""
        ...

    def strike_range(self, value: int) -> Self:
        """Strike range filter"""
        ...

    def min_time(self, value: Union[str, time, datetime]) -> Self:
        """Minimum time filter"""
        ...

    def current_day_only(self, value: bool) -> Self:
        """When true, only snapshot data from the current trading day (New York date) is returned. Applied before `min_time`. Defaults to false, matching the terminal, which sends false when the value is omitted."""
        ...


@final
class OptionSnapshotMarketValueBuilder(_Builder[MarketValueTickList]):
    """Fluent builder for :meth:`MarketDataView.option_snapshot_market_value`."""

    def symbol(self, value: str) -> Self:
        """Ticker symbol (e.g. AAPL)"""
        ...

    def expiration(self, value: Union[str, _date, datetime]) -> Self:
        """Expiration date YYYYMMDD"""
        ...

    def strike(self, value: str) -> Self:
        """Strike price in dollars as a string (e.g. 500 or 17.5). Use `*` for wildcard selection."""
        ...

    def right(self, value: Union[str, Right]) -> Self:
        """Option side. Use `both` or `*` (alias) for calls and puts. Accepted values: `call`, `put`, `both`, `*`."""
        ...

    def max_dte(self, value: int) -> Self:
        """Maximum days to expiration"""
        ...

    def strike_range(self, value: int) -> Self:
        """Strike range filter"""
        ...

    def min_time(self, value: Union[str, time, datetime]) -> Self:
        """Minimum time filter"""
        ...

    def current_day_only(self, value: bool) -> Self:
        """When true, only snapshot data from the current trading day (New York date) is returned. Applied before `min_time`. Defaults to false, matching the terminal, which sends false when the value is omitted."""
        ...


@final
class OptionSnapshotGreeksImpliedVolatilityBuilder(_Builder[IvTickList]):
    """Fluent builder for :meth:`MarketDataView.option_snapshot_greeks_implied_volatility`."""

    def symbol(self, value: str) -> Self:
        """Ticker symbol (e.g. AAPL)"""
        ...

    def expiration(self, value: Union[str, _date, datetime]) -> Self:
        """Expiration date YYYYMMDD"""
        ...

    def strike(self, value: str) -> Self:
        """Strike price in dollars as a string (e.g. 500 or 17.5). Use `*` for wildcard selection."""
        ...

    def right(self, value: Union[str, Right]) -> Self:
        """Option side. Use `both` or `*` (alias) for calls and puts. Accepted values: `call`, `put`, `both`, `*`."""
        ...

    def annual_dividend(self, value: float) -> Self:
        """Annualized continuous dividend yield used in the Greeks calculation, as a decimal (e.g. 0.0042 for a 0.42% yield). Must be non-negative; to pass a dollar amount per share, use `annual_dividend_amount`."""
        ...

    def annual_dividend_amount(self, value: float) -> Self:
        """Annual dividend per share, in dollars, used in the Greeks calculation (e.g. 0.96 for $0.96 per year). Must be non-negative, and the server refuses it beside a non-zero `annual_dividend`."""
        ...

    def rate_type(self, value: Union[str, RateType]) -> Self:
        """Risk-free-rate source used in the Greeks calculation. Accepted values: `sofr`, `treasury_m1`, `treasury_m3`, `treasury_m6`, `treasury_y1`, `treasury_y2`, `treasury_y3`, `treasury_y5`, `treasury_y7`, `treasury_y10`, `treasury_y20`, `treasury_y30`."""
        ...

    def rate_value(self, value: float) -> Self:
        """Interest rate as a percent (4.36 means 4.36%, matching the InterestRateTick.rate convention) used in the Greeks calculation. Applied when rate_type selects a manual rate."""
        ...

    def stock_price(self, value: float) -> Self:
        """Underlying price in dollars used in the Greeks calculation, overriding the observed underlying when set."""
        ...

    def version(self, value: Union[str, Version]) -> Self:
        """Greeks model version. Accepted values: `latest`, `1`."""
        ...

    def max_dte(self, value: int) -> Self:
        """Maximum days to expiration"""
        ...

    def strike_range(self, value: int) -> Self:
        """Strike range filter"""
        ...

    def min_time(self, value: Union[str, time, datetime]) -> Self:
        """Minimum time filter"""
        ...

    def use_market_value(self, value: bool) -> Self:
        """When true, calculate Greeks against the option market value (mid-price) instead of the NBBO bid/ask pair."""
        ...

    def current_day_only(self, value: bool) -> Self:
        """When true, only snapshot data from the current trading day (New York date) is returned. Applied before `min_time`. Defaults to false, matching the terminal, which sends false when the value is omitted."""
        ...


@final
class OptionSnapshotGreeksAllBuilder(_Builder[GreeksAllTickList]):
    """Fluent builder for :meth:`MarketDataView.option_snapshot_greeks_all`."""

    def symbol(self, value: str) -> Self:
        """Ticker symbol (e.g. AAPL)"""
        ...

    def expiration(self, value: Union[str, _date, datetime]) -> Self:
        """Expiration date YYYYMMDD"""
        ...

    def strike(self, value: str) -> Self:
        """Strike price in dollars as a string (e.g. 500 or 17.5). Use `*` for wildcard selection."""
        ...

    def right(self, value: Union[str, Right]) -> Self:
        """Option side. Use `both` or `*` (alias) for calls and puts. Accepted values: `call`, `put`, `both`, `*`."""
        ...

    def annual_dividend(self, value: float) -> Self:
        """Annualized continuous dividend yield used in the Greeks calculation, as a decimal (e.g. 0.0042 for a 0.42% yield). Must be non-negative; to pass a dollar amount per share, use `annual_dividend_amount`."""
        ...

    def annual_dividend_amount(self, value: float) -> Self:
        """Annual dividend per share, in dollars, used in the Greeks calculation (e.g. 0.96 for $0.96 per year). Must be non-negative, and the server refuses it beside a non-zero `annual_dividend`."""
        ...

    def rate_type(self, value: Union[str, RateType]) -> Self:
        """Risk-free-rate source used in the Greeks calculation. Accepted values: `sofr`, `treasury_m1`, `treasury_m3`, `treasury_m6`, `treasury_y1`, `treasury_y2`, `treasury_y3`, `treasury_y5`, `treasury_y7`, `treasury_y10`, `treasury_y20`, `treasury_y30`."""
        ...

    def rate_value(self, value: float) -> Self:
        """Interest rate as a percent (4.36 means 4.36%, matching the InterestRateTick.rate convention) used in the Greeks calculation. Applied when rate_type selects a manual rate."""
        ...

    def stock_price(self, value: float) -> Self:
        """Underlying price in dollars used in the Greeks calculation, overriding the observed underlying when set."""
        ...

    def version(self, value: Union[str, Version]) -> Self:
        """Greeks model version. Accepted values: `latest`, `1`."""
        ...

    def max_dte(self, value: int) -> Self:
        """Maximum days to expiration"""
        ...

    def strike_range(self, value: int) -> Self:
        """Strike range filter"""
        ...

    def min_time(self, value: Union[str, time, datetime]) -> Self:
        """Minimum time filter"""
        ...

    def use_market_value(self, value: bool) -> Self:
        """When true, calculate Greeks against the option market value (mid-price) instead of the NBBO bid/ask pair."""
        ...

    def current_day_only(self, value: bool) -> Self:
        """When true, only snapshot data from the current trading day (New York date) is returned. Applied before `min_time`. Defaults to false, matching the terminal, which sends false when the value is omitted."""
        ...


@final
class OptionSnapshotBinomialGreeksAllBuilder(_Builder[GreeksAllTickList]):
    """Fluent builder for :meth:`MarketDataView.option_snapshot_binomial_greeks_all`."""

    def symbol(self, value: str) -> Self:
        """Ticker symbol (e.g. AAPL)"""
        ...

    def expiration(self, value: Union[str, _date, datetime]) -> Self:
        """Expiration date YYYYMMDD"""
        ...

    def strike(self, value: str) -> Self:
        """Strike price in dollars as a string (e.g. 500 or 17.5). Use `*` for wildcard selection."""
        ...

    def right(self, value: Union[str, Right]) -> Self:
        """Option side. Use `both` or `*` (alias) for calls and puts. Accepted values: `call`, `put`, `both`, `*`."""
        ...

    def annual_dividend(self, value: float) -> Self:
        """Annualized continuous dividend yield used in the Greeks calculation, as a decimal (e.g. 0.0042 for a 0.42% yield). Must be non-negative; to pass a dollar amount per share, use `annual_dividend_amount`."""
        ...

    def annual_dividend_amount(self, value: float) -> Self:
        """Annual dividend per share, in dollars, used in the Greeks calculation (e.g. 0.96 for $0.96 per year). Must be non-negative, and the server refuses it beside a non-zero `annual_dividend`."""
        ...

    def rate_type(self, value: Union[str, RateType]) -> Self:
        """Risk-free-rate source used in the Greeks calculation. Accepted values: `sofr`, `treasury_m1`, `treasury_m3`, `treasury_m6`, `treasury_y1`, `treasury_y2`, `treasury_y3`, `treasury_y5`, `treasury_y7`, `treasury_y10`, `treasury_y20`, `treasury_y30`."""
        ...

    def rate_value(self, value: float) -> Self:
        """Interest rate as a percent (4.36 means 4.36%, matching the InterestRateTick.rate convention) used in the Greeks calculation. Applied when rate_type selects a manual rate."""
        ...

    def stock_price(self, value: float) -> Self:
        """Underlying price in dollars used in the Greeks calculation, overriding the observed underlying when set."""
        ...

    def binomial_steps(self, value: int) -> Self:
        """Number of steps in the binomial tree used for the Greeks calculation. Accurate at higher values and cheaper at lower ones; the server's cost grows with the square of the value. An even value is rounded up to the next odd one, and an `expiration=*` request is capped at 101 steps."""
        ...

    def version(self, value: Union[str, Version]) -> Self:
        """Greeks model version. Accepted values: `latest`, `1`."""
        ...

    def max_dte(self, value: int) -> Self:
        """Maximum days to expiration"""
        ...

    def strike_range(self, value: int) -> Self:
        """Strike range filter"""
        ...

    def min_time(self, value: Union[str, time, datetime]) -> Self:
        """Minimum time filter"""
        ...

    def use_market_value(self, value: bool) -> Self:
        """When true, calculate Greeks against the option market value (mid-price) instead of the NBBO bid/ask pair."""
        ...

    def current_day_only(self, value: bool) -> Self:
        """When true, only snapshot data from the current trading day (New York date) is returned. Applied before `min_time`. Defaults to false, matching the terminal, which sends false when the value is omitted."""
        ...


@final
class OptionSnapshotGreeksFirstOrderBuilder(_Builder[GreeksFirstOrderTickList]):
    """Fluent builder for :meth:`MarketDataView.option_snapshot_greeks_first_order`."""

    def symbol(self, value: str) -> Self:
        """Ticker symbol (e.g. AAPL)"""
        ...

    def expiration(self, value: Union[str, _date, datetime]) -> Self:
        """Expiration date YYYYMMDD"""
        ...

    def strike(self, value: str) -> Self:
        """Strike price in dollars as a string (e.g. 500 or 17.5). Use `*` for wildcard selection."""
        ...

    def right(self, value: Union[str, Right]) -> Self:
        """Option side. Use `both` or `*` (alias) for calls and puts. Accepted values: `call`, `put`, `both`, `*`."""
        ...

    def annual_dividend(self, value: float) -> Self:
        """Annualized continuous dividend yield used in the Greeks calculation, as a decimal (e.g. 0.0042 for a 0.42% yield). Must be non-negative; to pass a dollar amount per share, use `annual_dividend_amount`."""
        ...

    def annual_dividend_amount(self, value: float) -> Self:
        """Annual dividend per share, in dollars, used in the Greeks calculation (e.g. 0.96 for $0.96 per year). Must be non-negative, and the server refuses it beside a non-zero `annual_dividend`."""
        ...

    def rate_type(self, value: Union[str, RateType]) -> Self:
        """Risk-free-rate source used in the Greeks calculation. Accepted values: `sofr`, `treasury_m1`, `treasury_m3`, `treasury_m6`, `treasury_y1`, `treasury_y2`, `treasury_y3`, `treasury_y5`, `treasury_y7`, `treasury_y10`, `treasury_y20`, `treasury_y30`."""
        ...

    def rate_value(self, value: float) -> Self:
        """Interest rate as a percent (4.36 means 4.36%, matching the InterestRateTick.rate convention) used in the Greeks calculation. Applied when rate_type selects a manual rate."""
        ...

    def stock_price(self, value: float) -> Self:
        """Underlying price in dollars used in the Greeks calculation, overriding the observed underlying when set."""
        ...

    def version(self, value: Union[str, Version]) -> Self:
        """Greeks model version. Accepted values: `latest`, `1`."""
        ...

    def max_dte(self, value: int) -> Self:
        """Maximum days to expiration"""
        ...

    def strike_range(self, value: int) -> Self:
        """Strike range filter"""
        ...

    def min_time(self, value: Union[str, time, datetime]) -> Self:
        """Minimum time filter"""
        ...

    def use_market_value(self, value: bool) -> Self:
        """When true, calculate Greeks against the option market value (mid-price) instead of the NBBO bid/ask pair."""
        ...

    def current_day_only(self, value: bool) -> Self:
        """When true, only snapshot data from the current trading day (New York date) is returned. Applied before `min_time`. Defaults to false, matching the terminal, which sends false when the value is omitted."""
        ...


@final
class OptionSnapshotBinomialGreeksFirstOrderBuilder(_Builder[GreeksFirstOrderTickList]):
    """Fluent builder for :meth:`MarketDataView.option_snapshot_binomial_greeks_first_order`."""

    def symbol(self, value: str) -> Self:
        """Ticker symbol (e.g. AAPL)"""
        ...

    def expiration(self, value: Union[str, _date, datetime]) -> Self:
        """Expiration date YYYYMMDD"""
        ...

    def strike(self, value: str) -> Self:
        """Strike price in dollars as a string (e.g. 500 or 17.5). Use `*` for wildcard selection."""
        ...

    def right(self, value: Union[str, Right]) -> Self:
        """Option side. Use `both` or `*` (alias) for calls and puts. Accepted values: `call`, `put`, `both`, `*`."""
        ...

    def annual_dividend(self, value: float) -> Self:
        """Annualized continuous dividend yield used in the Greeks calculation, as a decimal (e.g. 0.0042 for a 0.42% yield). Must be non-negative; to pass a dollar amount per share, use `annual_dividend_amount`."""
        ...

    def annual_dividend_amount(self, value: float) -> Self:
        """Annual dividend per share, in dollars, used in the Greeks calculation (e.g. 0.96 for $0.96 per year). Must be non-negative, and the server refuses it beside a non-zero `annual_dividend`."""
        ...

    def rate_type(self, value: Union[str, RateType]) -> Self:
        """Risk-free-rate source used in the Greeks calculation. Accepted values: `sofr`, `treasury_m1`, `treasury_m3`, `treasury_m6`, `treasury_y1`, `treasury_y2`, `treasury_y3`, `treasury_y5`, `treasury_y7`, `treasury_y10`, `treasury_y20`, `treasury_y30`."""
        ...

    def rate_value(self, value: float) -> Self:
        """Interest rate as a percent (4.36 means 4.36%, matching the InterestRateTick.rate convention) used in the Greeks calculation. Applied when rate_type selects a manual rate."""
        ...

    def stock_price(self, value: float) -> Self:
        """Underlying price in dollars used in the Greeks calculation, overriding the observed underlying when set."""
        ...

    def binomial_steps(self, value: int) -> Self:
        """Number of steps in the binomial tree used for the Greeks calculation. Accurate at higher values and cheaper at lower ones; the server's cost grows with the square of the value. An even value is rounded up to the next odd one, and an `expiration=*` request is capped at 101 steps."""
        ...

    def version(self, value: Union[str, Version]) -> Self:
        """Greeks model version. Accepted values: `latest`, `1`."""
        ...

    def max_dte(self, value: int) -> Self:
        """Maximum days to expiration"""
        ...

    def strike_range(self, value: int) -> Self:
        """Strike range filter"""
        ...

    def min_time(self, value: Union[str, time, datetime]) -> Self:
        """Minimum time filter"""
        ...

    def use_market_value(self, value: bool) -> Self:
        """When true, calculate Greeks against the option market value (mid-price) instead of the NBBO bid/ask pair."""
        ...

    def current_day_only(self, value: bool) -> Self:
        """When true, only snapshot data from the current trading day (New York date) is returned. Applied before `min_time`. Defaults to false, matching the terminal, which sends false when the value is omitted."""
        ...


@final
class OptionSnapshotGreeksSecondOrderBuilder(_Builder[GreeksSecondOrderTickList]):
    """Fluent builder for :meth:`MarketDataView.option_snapshot_greeks_second_order`."""

    def symbol(self, value: str) -> Self:
        """Ticker symbol (e.g. AAPL)"""
        ...

    def expiration(self, value: Union[str, _date, datetime]) -> Self:
        """Expiration date YYYYMMDD"""
        ...

    def strike(self, value: str) -> Self:
        """Strike price in dollars as a string (e.g. 500 or 17.5). Use `*` for wildcard selection."""
        ...

    def right(self, value: Union[str, Right]) -> Self:
        """Option side. Use `both` or `*` (alias) for calls and puts. Accepted values: `call`, `put`, `both`, `*`."""
        ...

    def annual_dividend(self, value: float) -> Self:
        """Annualized continuous dividend yield used in the Greeks calculation, as a decimal (e.g. 0.0042 for a 0.42% yield). Must be non-negative; to pass a dollar amount per share, use `annual_dividend_amount`."""
        ...

    def annual_dividend_amount(self, value: float) -> Self:
        """Annual dividend per share, in dollars, used in the Greeks calculation (e.g. 0.96 for $0.96 per year). Must be non-negative, and the server refuses it beside a non-zero `annual_dividend`."""
        ...

    def rate_type(self, value: Union[str, RateType]) -> Self:
        """Risk-free-rate source used in the Greeks calculation. Accepted values: `sofr`, `treasury_m1`, `treasury_m3`, `treasury_m6`, `treasury_y1`, `treasury_y2`, `treasury_y3`, `treasury_y5`, `treasury_y7`, `treasury_y10`, `treasury_y20`, `treasury_y30`."""
        ...

    def rate_value(self, value: float) -> Self:
        """Interest rate as a percent (4.36 means 4.36%, matching the InterestRateTick.rate convention) used in the Greeks calculation. Applied when rate_type selects a manual rate."""
        ...

    def stock_price(self, value: float) -> Self:
        """Underlying price in dollars used in the Greeks calculation, overriding the observed underlying when set."""
        ...

    def version(self, value: Union[str, Version]) -> Self:
        """Greeks model version. Accepted values: `latest`, `1`."""
        ...

    def max_dte(self, value: int) -> Self:
        """Maximum days to expiration"""
        ...

    def strike_range(self, value: int) -> Self:
        """Strike range filter"""
        ...

    def min_time(self, value: Union[str, time, datetime]) -> Self:
        """Minimum time filter"""
        ...

    def use_market_value(self, value: bool) -> Self:
        """When true, calculate Greeks against the option market value (mid-price) instead of the NBBO bid/ask pair."""
        ...

    def current_day_only(self, value: bool) -> Self:
        """When true, only snapshot data from the current trading day (New York date) is returned. Applied before `min_time`. Defaults to false, matching the terminal, which sends false when the value is omitted."""
        ...


@final
class OptionSnapshotBinomialGreeksSecondOrderBuilder(_Builder[GreeksSecondOrderTickList]):
    """Fluent builder for :meth:`MarketDataView.option_snapshot_binomial_greeks_second_order`."""

    def symbol(self, value: str) -> Self:
        """Ticker symbol (e.g. AAPL)"""
        ...

    def expiration(self, value: Union[str, _date, datetime]) -> Self:
        """Expiration date YYYYMMDD"""
        ...

    def strike(self, value: str) -> Self:
        """Strike price in dollars as a string (e.g. 500 or 17.5). Use `*` for wildcard selection."""
        ...

    def right(self, value: Union[str, Right]) -> Self:
        """Option side. Use `both` or `*` (alias) for calls and puts. Accepted values: `call`, `put`, `both`, `*`."""
        ...

    def annual_dividend(self, value: float) -> Self:
        """Annualized continuous dividend yield used in the Greeks calculation, as a decimal (e.g. 0.0042 for a 0.42% yield). Must be non-negative; to pass a dollar amount per share, use `annual_dividend_amount`."""
        ...

    def annual_dividend_amount(self, value: float) -> Self:
        """Annual dividend per share, in dollars, used in the Greeks calculation (e.g. 0.96 for $0.96 per year). Must be non-negative, and the server refuses it beside a non-zero `annual_dividend`."""
        ...

    def rate_type(self, value: Union[str, RateType]) -> Self:
        """Risk-free-rate source used in the Greeks calculation. Accepted values: `sofr`, `treasury_m1`, `treasury_m3`, `treasury_m6`, `treasury_y1`, `treasury_y2`, `treasury_y3`, `treasury_y5`, `treasury_y7`, `treasury_y10`, `treasury_y20`, `treasury_y30`."""
        ...

    def rate_value(self, value: float) -> Self:
        """Interest rate as a percent (4.36 means 4.36%, matching the InterestRateTick.rate convention) used in the Greeks calculation. Applied when rate_type selects a manual rate."""
        ...

    def stock_price(self, value: float) -> Self:
        """Underlying price in dollars used in the Greeks calculation, overriding the observed underlying when set."""
        ...

    def binomial_steps(self, value: int) -> Self:
        """Number of steps in the binomial tree used for the Greeks calculation. Accurate at higher values and cheaper at lower ones; the server's cost grows with the square of the value. An even value is rounded up to the next odd one, and an `expiration=*` request is capped at 101 steps."""
        ...

    def version(self, value: Union[str, Version]) -> Self:
        """Greeks model version. Accepted values: `latest`, `1`."""
        ...

    def max_dte(self, value: int) -> Self:
        """Maximum days to expiration"""
        ...

    def strike_range(self, value: int) -> Self:
        """Strike range filter"""
        ...

    def min_time(self, value: Union[str, time, datetime]) -> Self:
        """Minimum time filter"""
        ...

    def use_market_value(self, value: bool) -> Self:
        """When true, calculate Greeks against the option market value (mid-price) instead of the NBBO bid/ask pair."""
        ...

    def current_day_only(self, value: bool) -> Self:
        """When true, only snapshot data from the current trading day (New York date) is returned. Applied before `min_time`. Defaults to false, matching the terminal, which sends false when the value is omitted."""
        ...


@final
class OptionSnapshotGreeksThirdOrderBuilder(_Builder[GreeksThirdOrderTickList]):
    """Fluent builder for :meth:`MarketDataView.option_snapshot_greeks_third_order`."""

    def symbol(self, value: str) -> Self:
        """Ticker symbol (e.g. AAPL)"""
        ...

    def expiration(self, value: Union[str, _date, datetime]) -> Self:
        """Expiration date YYYYMMDD"""
        ...

    def strike(self, value: str) -> Self:
        """Strike price in dollars as a string (e.g. 500 or 17.5). Use `*` for wildcard selection."""
        ...

    def right(self, value: Union[str, Right]) -> Self:
        """Option side. Use `both` or `*` (alias) for calls and puts. Accepted values: `call`, `put`, `both`, `*`."""
        ...

    def annual_dividend(self, value: float) -> Self:
        """Annualized continuous dividend yield used in the Greeks calculation, as a decimal (e.g. 0.0042 for a 0.42% yield). Must be non-negative; to pass a dollar amount per share, use `annual_dividend_amount`."""
        ...

    def annual_dividend_amount(self, value: float) -> Self:
        """Annual dividend per share, in dollars, used in the Greeks calculation (e.g. 0.96 for $0.96 per year). Must be non-negative, and the server refuses it beside a non-zero `annual_dividend`."""
        ...

    def rate_type(self, value: Union[str, RateType]) -> Self:
        """Risk-free-rate source used in the Greeks calculation. Accepted values: `sofr`, `treasury_m1`, `treasury_m3`, `treasury_m6`, `treasury_y1`, `treasury_y2`, `treasury_y3`, `treasury_y5`, `treasury_y7`, `treasury_y10`, `treasury_y20`, `treasury_y30`."""
        ...

    def rate_value(self, value: float) -> Self:
        """Interest rate as a percent (4.36 means 4.36%, matching the InterestRateTick.rate convention) used in the Greeks calculation. Applied when rate_type selects a manual rate."""
        ...

    def stock_price(self, value: float) -> Self:
        """Underlying price in dollars used in the Greeks calculation, overriding the observed underlying when set."""
        ...

    def version(self, value: Union[str, Version]) -> Self:
        """Greeks model version. Accepted values: `latest`, `1`."""
        ...

    def max_dte(self, value: int) -> Self:
        """Maximum days to expiration"""
        ...

    def strike_range(self, value: int) -> Self:
        """Strike range filter"""
        ...

    def min_time(self, value: Union[str, time, datetime]) -> Self:
        """Minimum time filter"""
        ...

    def use_market_value(self, value: bool) -> Self:
        """When true, calculate Greeks against the option market value (mid-price) instead of the NBBO bid/ask pair."""
        ...

    def current_day_only(self, value: bool) -> Self:
        """When true, only snapshot data from the current trading day (New York date) is returned. Applied before `min_time`. Defaults to false, matching the terminal, which sends false when the value is omitted."""
        ...


@final
class OptionSnapshotBinomialGreeksThirdOrderBuilder(_Builder[GreeksThirdOrderTickList]):
    """Fluent builder for :meth:`MarketDataView.option_snapshot_binomial_greeks_third_order`."""

    def symbol(self, value: str) -> Self:
        """Ticker symbol (e.g. AAPL)"""
        ...

    def expiration(self, value: Union[str, _date, datetime]) -> Self:
        """Expiration date YYYYMMDD"""
        ...

    def strike(self, value: str) -> Self:
        """Strike price in dollars as a string (e.g. 500 or 17.5). Use `*` for wildcard selection."""
        ...

    def right(self, value: Union[str, Right]) -> Self:
        """Option side. Use `both` or `*` (alias) for calls and puts. Accepted values: `call`, `put`, `both`, `*`."""
        ...

    def annual_dividend(self, value: float) -> Self:
        """Annualized continuous dividend yield used in the Greeks calculation, as a decimal (e.g. 0.0042 for a 0.42% yield). Must be non-negative; to pass a dollar amount per share, use `annual_dividend_amount`."""
        ...

    def annual_dividend_amount(self, value: float) -> Self:
        """Annual dividend per share, in dollars, used in the Greeks calculation (e.g. 0.96 for $0.96 per year). Must be non-negative, and the server refuses it beside a non-zero `annual_dividend`."""
        ...

    def rate_type(self, value: Union[str, RateType]) -> Self:
        """Risk-free-rate source used in the Greeks calculation. Accepted values: `sofr`, `treasury_m1`, `treasury_m3`, `treasury_m6`, `treasury_y1`, `treasury_y2`, `treasury_y3`, `treasury_y5`, `treasury_y7`, `treasury_y10`, `treasury_y20`, `treasury_y30`."""
        ...

    def rate_value(self, value: float) -> Self:
        """Interest rate as a percent (4.36 means 4.36%, matching the InterestRateTick.rate convention) used in the Greeks calculation. Applied when rate_type selects a manual rate."""
        ...

    def stock_price(self, value: float) -> Self:
        """Underlying price in dollars used in the Greeks calculation, overriding the observed underlying when set."""
        ...

    def binomial_steps(self, value: int) -> Self:
        """Number of steps in the binomial tree used for the Greeks calculation. Accurate at higher values and cheaper at lower ones; the server's cost grows with the square of the value. An even value is rounded up to the next odd one, and an `expiration=*` request is capped at 101 steps."""
        ...

    def version(self, value: Union[str, Version]) -> Self:
        """Greeks model version. Accepted values: `latest`, `1`."""
        ...

    def max_dte(self, value: int) -> Self:
        """Maximum days to expiration"""
        ...

    def strike_range(self, value: int) -> Self:
        """Strike range filter"""
        ...

    def min_time(self, value: Union[str, time, datetime]) -> Self:
        """Minimum time filter"""
        ...

    def use_market_value(self, value: bool) -> Self:
        """When true, calculate Greeks against the option market value (mid-price) instead of the NBBO bid/ask pair."""
        ...

    def current_day_only(self, value: bool) -> Self:
        """When true, only snapshot data from the current trading day (New York date) is returned. Applied before `min_time`. Defaults to false, matching the terminal, which sends false when the value is omitted."""
        ...


@final
class OptionHistoryEodBuilder(_ChunkedBuilder[EodTickList]):
    """Fluent builder for :meth:`MarketDataView.option_history_eod`."""

    def symbol(self, value: str) -> Self:
        """Ticker symbol (e.g. AAPL)"""
        ...

    def expiration(self, value: Union[str, _date, datetime]) -> Self:
        """Expiration date YYYYMMDD"""
        ...

    def strike(self, value: str) -> Self:
        """Strike price in dollars as a string (e.g. 500 or 17.5). Use `*` for wildcard selection."""
        ...

    def right(self, value: Union[str, Right]) -> Self:
        """Option side. Use `both` or `*` (alias) for calls and puts. Accepted values: `call`, `put`, `both`, `*`."""
        ...

    def start_date(self, value: Union[str, _date, datetime]) -> Self:
        """Start date YYYYMMDD"""
        ...

    def end_date(self, value: Union[str, _date, datetime]) -> Self:
        """End date YYYYMMDD"""
        ...

    def max_dte(self, value: int) -> Self:
        """Maximum days to expiration"""
        ...

    def strike_range(self, value: int) -> Self:
        """Strike range filter"""
        ...


@final
class OptionHistoryOhlcBuilder(_ChunkedBuilder[OhlcTickList]):
    """Fluent builder for :meth:`MarketDataView.option_history_ohlc`."""

    def symbol(self, value: str) -> Self:
        """Ticker symbol (e.g. AAPL)"""
        ...

    def expiration(self, value: Union[str, _date, datetime]) -> Self:
        """Expiration date YYYYMMDD"""
        ...

    def strike(self, value: str) -> Self:
        """Strike price in dollars as a string (e.g. 500 or 17.5). Use `*` for wildcard selection."""
        ...

    def right(self, value: Union[str, Right]) -> Self:
        """Option side. Use `both` or `*` (alias) for calls and puts. Accepted values: `call`, `put`, `both`, `*`."""
        ...

    def date(self, value: Union[str, _date, datetime]) -> Self:
        """Single date YYYYMMDD. Supply this for a single-day pull, or supply `start_date`/`end_date` for a range. When present, `date` takes precedence over the range."""
        ...

    def interval(self, value: Union[str, Interval]) -> Self:
        """Interval preset. Defaults to `1s` when omitted, matching the upstream ThetaData Python library. Accepted values: `tick`, `10ms`, `100ms`, `500ms`, `1s`, `5s`, `10s`, `15s`, `30s`, `1m`, `5m`, `10m`, `15m`, `30m`, `1h`."""
        ...

    def start_time(self, value: Union[str, time, datetime]) -> Self:
        """Start time filter"""
        ...

    def end_time(self, value: Union[str, time, datetime]) -> Self:
        """End time filter"""
        ...

    def strike_range(self, value: int) -> Self:
        """Strike range filter"""
        ...

    def start_date(self, value: Union[str, _date, datetime]) -> Self:
        """Start date YYYYMMDD"""
        ...

    def end_date(self, value: Union[str, _date, datetime]) -> Self:
        """End date YYYYMMDD"""
        ...


@final
class OptionHistoryTradeBuilder(_ChunkedBuilder[TradeTickList]):
    """Fluent builder for :meth:`MarketDataView.option_history_trade`."""

    def symbol(self, value: str) -> Self:
        """Ticker symbol (e.g. AAPL)"""
        ...

    def expiration(self, value: Union[str, _date, datetime]) -> Self:
        """Expiration date YYYYMMDD"""
        ...

    def strike(self, value: str) -> Self:
        """Strike price in dollars as a string (e.g. 500 or 17.5). Use `*` for wildcard selection."""
        ...

    def right(self, value: Union[str, Right]) -> Self:
        """Option side. Use `both` or `*` (alias) for calls and puts. Accepted values: `call`, `put`, `both`, `*`."""
        ...

    def date(self, value: Union[str, _date, datetime]) -> Self:
        """Single date YYYYMMDD. Supply this for a single-day pull, or supply `start_date`/`end_date` for a range. When present, `date` takes precedence over the range."""
        ...

    def start_time(self, value: Union[str, time, datetime]) -> Self:
        """Start time filter"""
        ...

    def end_time(self, value: Union[str, time, datetime]) -> Self:
        """End time filter"""
        ...

    def max_dte(self, value: int) -> Self:
        """Maximum days to expiration"""
        ...

    def strike_range(self, value: int) -> Self:
        """Strike range filter"""
        ...

    def start_date(self, value: Union[str, _date, datetime]) -> Self:
        """Start date YYYYMMDD"""
        ...

    def end_date(self, value: Union[str, _date, datetime]) -> Self:
        """End date YYYYMMDD"""
        ...


@final
class OptionHistoryQuoteBuilder(_ChunkedBuilder[QuoteTickList]):
    """Fluent builder for :meth:`MarketDataView.option_history_quote`."""

    def symbol(self, value: str) -> Self:
        """Ticker symbol (e.g. AAPL)"""
        ...

    def expiration(self, value: Union[str, _date, datetime]) -> Self:
        """Expiration date YYYYMMDD"""
        ...

    def strike(self, value: str) -> Self:
        """Strike price in dollars as a string (e.g. 500 or 17.5). Use `*` for wildcard selection."""
        ...

    def right(self, value: Union[str, Right]) -> Self:
        """Option side. Use `both` or `*` (alias) for calls and puts. Accepted values: `call`, `put`, `both`, `*`."""
        ...

    def date(self, value: Union[str, _date, datetime]) -> Self:
        """Single date YYYYMMDD. Supply this for a single-day pull, or supply `start_date`/`end_date` for a range. When present, `date` takes precedence over the range."""
        ...

    def interval(self, value: Union[str, Interval]) -> Self:
        """Interval preset. Defaults to `1s` when omitted, matching the upstream ThetaData Python library. Accepted values: `tick`, `10ms`, `100ms`, `500ms`, `1s`, `5s`, `10s`, `15s`, `30s`, `1m`, `5m`, `10m`, `15m`, `30m`, `1h`."""
        ...

    def start_time(self, value: Union[str, time, datetime]) -> Self:
        """Start time filter"""
        ...

    def end_time(self, value: Union[str, time, datetime]) -> Self:
        """End time filter"""
        ...

    def max_dte(self, value: int) -> Self:
        """Maximum days to expiration"""
        ...

    def strike_range(self, value: int) -> Self:
        """Strike range filter"""
        ...

    def start_date(self, value: Union[str, _date, datetime]) -> Self:
        """Start date YYYYMMDD"""
        ...

    def end_date(self, value: Union[str, _date, datetime]) -> Self:
        """End date YYYYMMDD"""
        ...


@final
class OptionHistoryTradeQuoteBuilder(_ChunkedBuilder[TradeQuoteTickList]):
    """Fluent builder for :meth:`MarketDataView.option_history_trade_quote`."""

    def symbol(self, value: str) -> Self:
        """Ticker symbol (e.g. AAPL)"""
        ...

    def expiration(self, value: Union[str, _date, datetime]) -> Self:
        """Expiration date YYYYMMDD"""
        ...

    def strike(self, value: str) -> Self:
        """Strike price in dollars as a string (e.g. 500 or 17.5). Use `*` for wildcard selection."""
        ...

    def right(self, value: Union[str, Right]) -> Self:
        """Option side. Use `both` or `*` (alias) for calls and puts. Accepted values: `call`, `put`, `both`, `*`."""
        ...

    def date(self, value: Union[str, _date, datetime]) -> Self:
        """Single date YYYYMMDD. Supply this for a single-day pull, or supply `start_date`/`end_date` for a range. When present, `date` takes precedence over the range."""
        ...

    def start_time(self, value: Union[str, time, datetime]) -> Self:
        """Start time filter"""
        ...

    def end_time(self, value: Union[str, time, datetime]) -> Self:
        """End time filter"""
        ...

    def exclusive(self, value: bool) -> Self:
        """When true, quotes whose timestamp equals the trade timestamp are excluded; only quotes strictly before the trade are paired. Defaults to true, matching the terminal, which injects exclusive=true when the value is omitted."""
        ...

    def max_dte(self, value: int) -> Self:
        """Maximum days to expiration"""
        ...

    def strike_range(self, value: int) -> Self:
        """Strike range filter"""
        ...

    def start_date(self, value: Union[str, _date, datetime]) -> Self:
        """Start date YYYYMMDD"""
        ...

    def end_date(self, value: Union[str, _date, datetime]) -> Self:
        """End date YYYYMMDD"""
        ...

    def use_time_based_trade_quote(self, value: bool) -> Self:
        """Affects requests for current-day data only. When true, each trade is matched with a quote by millisecond timestamp. Defaults to true, matching the terminal, which sends true when the value is omitted."""
        ...


@final
class OptionHistoryOpenInterestBuilder(_ChunkedBuilder[OpenInterestTickList]):
    """Fluent builder for :meth:`MarketDataView.option_history_open_interest`."""

    def symbol(self, value: str) -> Self:
        """Ticker symbol (e.g. AAPL)"""
        ...

    def expiration(self, value: Union[str, _date, datetime]) -> Self:
        """Expiration date YYYYMMDD"""
        ...

    def strike(self, value: str) -> Self:
        """Strike price in dollars as a string (e.g. 500 or 17.5). Use `*` for wildcard selection."""
        ...

    def right(self, value: Union[str, Right]) -> Self:
        """Option side. Use `both` or `*` (alias) for calls and puts. Accepted values: `call`, `put`, `both`, `*`."""
        ...

    def date(self, value: Union[str, _date, datetime]) -> Self:
        """Single date YYYYMMDD. Supply this for a single-day pull, or supply `start_date`/`end_date` for a range. When present, `date` takes precedence over the range."""
        ...

    def max_dte(self, value: int) -> Self:
        """Maximum days to expiration"""
        ...

    def strike_range(self, value: int) -> Self:
        """Strike range filter"""
        ...

    def start_date(self, value: Union[str, _date, datetime]) -> Self:
        """Start date YYYYMMDD"""
        ...

    def end_date(self, value: Union[str, _date, datetime]) -> Self:
        """End date YYYYMMDD"""
        ...


@final
class OptionHistoryGreeksEodBuilder(_ChunkedBuilder[GreeksEodTickList]):
    """Fluent builder for :meth:`MarketDataView.option_history_greeks_eod`."""

    def symbol(self, value: str) -> Self:
        """Ticker symbol (e.g. AAPL)"""
        ...

    def expiration(self, value: Union[str, _date, datetime]) -> Self:
        """Expiration date YYYYMMDD"""
        ...

    def strike(self, value: str) -> Self:
        """Strike price in dollars as a string (e.g. 500 or 17.5). Use `*` for wildcard selection."""
        ...

    def right(self, value: Union[str, Right]) -> Self:
        """Option side. Use `both` or `*` (alias) for calls and puts. Accepted values: `call`, `put`, `both`, `*`."""
        ...

    def start_date(self, value: Union[str, _date, datetime]) -> Self:
        """Start date YYYYMMDD"""
        ...

    def end_date(self, value: Union[str, _date, datetime]) -> Self:
        """End date YYYYMMDD"""
        ...

    def annual_dividend(self, value: float) -> Self:
        """Annualized continuous dividend yield used in the Greeks calculation, as a decimal (e.g. 0.0042 for a 0.42% yield). Must be non-negative; to pass a dollar amount per share, use `annual_dividend_amount`."""
        ...

    def annual_dividend_amount(self, value: float) -> Self:
        """Annual dividend per share, in dollars, used in the Greeks calculation (e.g. 0.96 for $0.96 per year). Must be non-negative, and the server refuses it beside a non-zero `annual_dividend`."""
        ...

    def rate_type(self, value: Union[str, RateType]) -> Self:
        """Risk-free-rate source used in the Greeks calculation. Accepted values: `sofr`, `treasury_m1`, `treasury_m3`, `treasury_m6`, `treasury_y1`, `treasury_y2`, `treasury_y3`, `treasury_y5`, `treasury_y7`, `treasury_y10`, `treasury_y20`, `treasury_y30`."""
        ...

    def rate_value(self, value: float) -> Self:
        """Interest rate as a percent (4.36 means 4.36%, matching the InterestRateTick.rate convention) used in the Greeks calculation. Applied when rate_type selects a manual rate."""
        ...

    def version(self, value: Union[str, Version]) -> Self:
        """Greeks model version. Accepted values: `latest`, `1`."""
        ...

    def underlyer_use_nbbo(self, value: bool) -> Self:
        """When true, use the NBBO-derived underlyer price as the Greeks input instead of the last trade."""
        ...

    def max_dte(self, value: int) -> Self:
        """Maximum days to expiration"""
        ...

    def strike_range(self, value: int) -> Self:
        """Strike range filter"""
        ...

    def option_use_nbbo(self, value: bool) -> Self:
        """Selects the option price used in the Greeks calculation: true uses the midpoint of the closing NBBO, false uses the last trade price. Defaults to true, matching the terminal, which sends true when the value is omitted."""
        ...


@final
class OptionHistoryBinomialGreeksEodBuilder(_ChunkedBuilder[GreeksEodTickList]):
    """Fluent builder for :meth:`MarketDataView.option_history_binomial_greeks_eod`."""

    def symbol(self, value: str) -> Self:
        """Ticker symbol (e.g. AAPL)"""
        ...

    def expiration(self, value: Union[str, _date, datetime]) -> Self:
        """Expiration date YYYYMMDD"""
        ...

    def strike(self, value: str) -> Self:
        """Strike price in dollars as a string (e.g. 500 or 17.5). Use `*` for wildcard selection."""
        ...

    def right(self, value: Union[str, Right]) -> Self:
        """Option side. Use `both` or `*` (alias) for calls and puts. Accepted values: `call`, `put`, `both`, `*`."""
        ...

    def start_date(self, value: Union[str, _date, datetime]) -> Self:
        """Start date YYYYMMDD"""
        ...

    def end_date(self, value: Union[str, _date, datetime]) -> Self:
        """End date YYYYMMDD"""
        ...

    def annual_dividend(self, value: float) -> Self:
        """Annualized continuous dividend yield used in the Greeks calculation, as a decimal (e.g. 0.0042 for a 0.42% yield). Must be non-negative; to pass a dollar amount per share, use `annual_dividend_amount`."""
        ...

    def annual_dividend_amount(self, value: float) -> Self:
        """Annual dividend per share, in dollars, used in the Greeks calculation (e.g. 0.96 for $0.96 per year). Must be non-negative, and the server refuses it beside a non-zero `annual_dividend`."""
        ...

    def rate_type(self, value: Union[str, RateType]) -> Self:
        """Risk-free-rate source used in the Greeks calculation. Accepted values: `sofr`, `treasury_m1`, `treasury_m3`, `treasury_m6`, `treasury_y1`, `treasury_y2`, `treasury_y3`, `treasury_y5`, `treasury_y7`, `treasury_y10`, `treasury_y20`, `treasury_y30`."""
        ...

    def rate_value(self, value: float) -> Self:
        """Interest rate as a percent (4.36 means 4.36%, matching the InterestRateTick.rate convention) used in the Greeks calculation. Applied when rate_type selects a manual rate."""
        ...

    def binomial_steps(self, value: int) -> Self:
        """Number of steps in the binomial tree used for the Greeks calculation. Accurate at higher values and cheaper at lower ones; the server's cost grows with the square of the value. An even value is rounded up to the next odd one, and an `expiration=*` request is capped at 101 steps."""
        ...

    def version(self, value: Union[str, Version]) -> Self:
        """Greeks model version. Accepted values: `latest`, `1`."""
        ...

    def underlyer_use_nbbo(self, value: bool) -> Self:
        """When true, use the NBBO-derived underlyer price as the Greeks input instead of the last trade."""
        ...

    def max_dte(self, value: int) -> Self:
        """Maximum days to expiration"""
        ...

    def strike_range(self, value: int) -> Self:
        """Strike range filter"""
        ...

    def option_use_nbbo(self, value: bool) -> Self:
        """Selects the option price used in the Greeks calculation: true uses the midpoint of the closing NBBO, false uses the last trade price. Defaults to true, matching the terminal, which sends true when the value is omitted."""
        ...


@final
class OptionHistoryGreeksAllBuilder(_ChunkedBuilder[GreeksAllTickList]):
    """Fluent builder for :meth:`MarketDataView.option_history_greeks_all`."""

    def symbol(self, value: str) -> Self:
        """Ticker symbol (e.g. AAPL)"""
        ...

    def expiration(self, value: Union[str, _date, datetime]) -> Self:
        """Expiration date YYYYMMDD"""
        ...

    def strike(self, value: str) -> Self:
        """Strike price in dollars as a string (e.g. 500 or 17.5). Use `*` for wildcard selection."""
        ...

    def right(self, value: Union[str, Right]) -> Self:
        """Option side. Use `both` or `*` (alias) for calls and puts. Accepted values: `call`, `put`, `both`, `*`."""
        ...

    def date(self, value: Union[str, _date, datetime]) -> Self:
        """Single date YYYYMMDD. Supply this for a single-day pull, or supply `start_date`/`end_date` for a range. When present, `date` takes precedence over the range."""
        ...

    def interval(self, value: Union[str, Interval]) -> Self:
        """Interval preset. Defaults to `1s` when omitted, matching the upstream ThetaData Python library. Accepted values: `tick`, `10ms`, `100ms`, `500ms`, `1s`, `5s`, `10s`, `15s`, `30s`, `1m`, `5m`, `10m`, `15m`, `30m`, `1h`."""
        ...

    def start_time(self, value: Union[str, time, datetime]) -> Self:
        """Start time filter"""
        ...

    def end_time(self, value: Union[str, time, datetime]) -> Self:
        """End time filter"""
        ...

    def annual_dividend(self, value: float) -> Self:
        """Annualized continuous dividend yield used in the Greeks calculation, as a decimal (e.g. 0.0042 for a 0.42% yield). Must be non-negative; to pass a dollar amount per share, use `annual_dividend_amount`."""
        ...

    def annual_dividend_amount(self, value: float) -> Self:
        """Annual dividend per share, in dollars, used in the Greeks calculation (e.g. 0.96 for $0.96 per year). Must be non-negative, and the server refuses it beside a non-zero `annual_dividend`."""
        ...

    def rate_type(self, value: Union[str, RateType]) -> Self:
        """Risk-free-rate source used in the Greeks calculation. Accepted values: `sofr`, `treasury_m1`, `treasury_m3`, `treasury_m6`, `treasury_y1`, `treasury_y2`, `treasury_y3`, `treasury_y5`, `treasury_y7`, `treasury_y10`, `treasury_y20`, `treasury_y30`."""
        ...

    def rate_value(self, value: float) -> Self:
        """Interest rate as a percent (4.36 means 4.36%, matching the InterestRateTick.rate convention) used in the Greeks calculation. Applied when rate_type selects a manual rate."""
        ...

    def version(self, value: Union[str, Version]) -> Self:
        """Greeks model version. Accepted values: `latest`, `1`."""
        ...

    def strike_range(self, value: int) -> Self:
        """Strike range filter"""
        ...

    def start_date(self, value: Union[str, _date, datetime]) -> Self:
        """Start date YYYYMMDD"""
        ...

    def end_date(self, value: Union[str, _date, datetime]) -> Self:
        """End date YYYYMMDD"""
        ...


@final
class OptionHistoryBinomialGreeksAllBuilder(_ChunkedBuilder[GreeksAllTickList]):
    """Fluent builder for :meth:`MarketDataView.option_history_binomial_greeks_all`."""

    def symbol(self, value: str) -> Self:
        """Ticker symbol (e.g. AAPL)"""
        ...

    def expiration(self, value: Union[str, _date, datetime]) -> Self:
        """Expiration date YYYYMMDD"""
        ...

    def strike(self, value: str) -> Self:
        """Strike price in dollars as a string (e.g. 500 or 17.5). Use `*` for wildcard selection."""
        ...

    def right(self, value: Union[str, Right]) -> Self:
        """Option side. Use `both` or `*` (alias) for calls and puts. Accepted values: `call`, `put`, `both`, `*`."""
        ...

    def date(self, value: Union[str, _date, datetime]) -> Self:
        """Single date YYYYMMDD. Supply this for a single-day pull, or supply `start_date`/`end_date` for a range. When present, `date` takes precedence over the range."""
        ...

    def interval(self, value: Union[str, Interval]) -> Self:
        """Interval preset. Defaults to `1s` when omitted, matching the upstream ThetaData Python library. Accepted values: `tick`, `10ms`, `100ms`, `500ms`, `1s`, `5s`, `10s`, `15s`, `30s`, `1m`, `5m`, `10m`, `15m`, `30m`, `1h`."""
        ...

    def start_time(self, value: Union[str, time, datetime]) -> Self:
        """Start time filter"""
        ...

    def end_time(self, value: Union[str, time, datetime]) -> Self:
        """End time filter"""
        ...

    def annual_dividend(self, value: float) -> Self:
        """Annualized continuous dividend yield used in the Greeks calculation, as a decimal (e.g. 0.0042 for a 0.42% yield). Must be non-negative; to pass a dollar amount per share, use `annual_dividend_amount`."""
        ...

    def annual_dividend_amount(self, value: float) -> Self:
        """Annual dividend per share, in dollars, used in the Greeks calculation (e.g. 0.96 for $0.96 per year). Must be non-negative, and the server refuses it beside a non-zero `annual_dividend`."""
        ...

    def rate_type(self, value: Union[str, RateType]) -> Self:
        """Risk-free-rate source used in the Greeks calculation. Accepted values: `sofr`, `treasury_m1`, `treasury_m3`, `treasury_m6`, `treasury_y1`, `treasury_y2`, `treasury_y3`, `treasury_y5`, `treasury_y7`, `treasury_y10`, `treasury_y20`, `treasury_y30`."""
        ...

    def rate_value(self, value: float) -> Self:
        """Interest rate as a percent (4.36 means 4.36%, matching the InterestRateTick.rate convention) used in the Greeks calculation. Applied when rate_type selects a manual rate."""
        ...

    def binomial_steps(self, value: int) -> Self:
        """Number of steps in the binomial tree used for the Greeks calculation. Accurate at higher values and cheaper at lower ones; the server's cost grows with the square of the value. An even value is rounded up to the next odd one, and an `expiration=*` request is capped at 101 steps."""
        ...

    def version(self, value: Union[str, Version]) -> Self:
        """Greeks model version. Accepted values: `latest`, `1`."""
        ...

    def strike_range(self, value: int) -> Self:
        """Strike range filter"""
        ...

    def start_date(self, value: Union[str, _date, datetime]) -> Self:
        """Start date YYYYMMDD"""
        ...

    def end_date(self, value: Union[str, _date, datetime]) -> Self:
        """End date YYYYMMDD"""
        ...


@final
class OptionHistoryTradeGreeksAllBuilder(_ChunkedBuilder[TradeGreeksAllTickList]):
    """Fluent builder for :meth:`MarketDataView.option_history_trade_greeks_all`."""

    def symbol(self, value: str) -> Self:
        """Ticker symbol (e.g. AAPL)"""
        ...

    def expiration(self, value: Union[str, _date, datetime]) -> Self:
        """Expiration date YYYYMMDD"""
        ...

    def strike(self, value: str) -> Self:
        """Strike price in dollars as a string (e.g. 500 or 17.5). Use `*` for wildcard selection."""
        ...

    def right(self, value: Union[str, Right]) -> Self:
        """Option side. Use `both` or `*` (alias) for calls and puts. Accepted values: `call`, `put`, `both`, `*`."""
        ...

    def date(self, value: Union[str, _date, datetime]) -> Self:
        """Single date YYYYMMDD. Supply this for a single-day pull, or supply `start_date`/`end_date` for a range. When present, `date` takes precedence over the range."""
        ...

    def start_time(self, value: Union[str, time, datetime]) -> Self:
        """Start time filter"""
        ...

    def end_time(self, value: Union[str, time, datetime]) -> Self:
        """End time filter"""
        ...

    def annual_dividend(self, value: float) -> Self:
        """Annualized continuous dividend yield used in the Greeks calculation, as a decimal (e.g. 0.0042 for a 0.42% yield). Must be non-negative; to pass a dollar amount per share, use `annual_dividend_amount`."""
        ...

    def annual_dividend_amount(self, value: float) -> Self:
        """Annual dividend per share, in dollars, used in the Greeks calculation (e.g. 0.96 for $0.96 per year). Must be non-negative, and the server refuses it beside a non-zero `annual_dividend`."""
        ...

    def rate_type(self, value: Union[str, RateType]) -> Self:
        """Risk-free-rate source used in the Greeks calculation. Accepted values: `sofr`, `treasury_m1`, `treasury_m3`, `treasury_m6`, `treasury_y1`, `treasury_y2`, `treasury_y3`, `treasury_y5`, `treasury_y7`, `treasury_y10`, `treasury_y20`, `treasury_y30`."""
        ...

    def rate_value(self, value: float) -> Self:
        """Interest rate as a percent (4.36 means 4.36%, matching the InterestRateTick.rate convention) used in the Greeks calculation. Applied when rate_type selects a manual rate."""
        ...

    def version(self, value: Union[str, Version]) -> Self:
        """Greeks model version. Accepted values: `latest`, `1`."""
        ...

    def max_dte(self, value: int) -> Self:
        """Maximum days to expiration"""
        ...

    def strike_range(self, value: int) -> Self:
        """Strike range filter"""
        ...

    def start_date(self, value: Union[str, _date, datetime]) -> Self:
        """Start date YYYYMMDD"""
        ...

    def end_date(self, value: Union[str, _date, datetime]) -> Self:
        """End date YYYYMMDD"""
        ...

    def perf_boost_intraday(self, value: bool) -> Self:
        """When true, the underlying price for a current-day query is looked up with 1-second precision instead of millisecond precision, which answers faster. Defaults to false, matching the terminal, which sends false when the value is omitted."""
        ...


@final
class OptionHistoryBinomialTradeGreeksAllBuilder(_ChunkedBuilder[TradeGreeksAllTickList]):
    """Fluent builder for :meth:`MarketDataView.option_history_binomial_trade_greeks_all`."""

    def symbol(self, value: str) -> Self:
        """Ticker symbol (e.g. AAPL)"""
        ...

    def expiration(self, value: Union[str, _date, datetime]) -> Self:
        """Expiration date YYYYMMDD"""
        ...

    def strike(self, value: str) -> Self:
        """Strike price in dollars as a string (e.g. 500 or 17.5). Use `*` for wildcard selection."""
        ...

    def right(self, value: Union[str, Right]) -> Self:
        """Option side. Use `both` or `*` (alias) for calls and puts. Accepted values: `call`, `put`, `both`, `*`."""
        ...

    def date(self, value: Union[str, _date, datetime]) -> Self:
        """Single date YYYYMMDD. Supply this for a single-day pull, or supply `start_date`/`end_date` for a range. When present, `date` takes precedence over the range."""
        ...

    def start_time(self, value: Union[str, time, datetime]) -> Self:
        """Start time filter"""
        ...

    def end_time(self, value: Union[str, time, datetime]) -> Self:
        """End time filter"""
        ...

    def annual_dividend(self, value: float) -> Self:
        """Annualized continuous dividend yield used in the Greeks calculation, as a decimal (e.g. 0.0042 for a 0.42% yield). Must be non-negative; to pass a dollar amount per share, use `annual_dividend_amount`."""
        ...

    def annual_dividend_amount(self, value: float) -> Self:
        """Annual dividend per share, in dollars, used in the Greeks calculation (e.g. 0.96 for $0.96 per year). Must be non-negative, and the server refuses it beside a non-zero `annual_dividend`."""
        ...

    def rate_type(self, value: Union[str, RateType]) -> Self:
        """Risk-free-rate source used in the Greeks calculation. Accepted values: `sofr`, `treasury_m1`, `treasury_m3`, `treasury_m6`, `treasury_y1`, `treasury_y2`, `treasury_y3`, `treasury_y5`, `treasury_y7`, `treasury_y10`, `treasury_y20`, `treasury_y30`."""
        ...

    def rate_value(self, value: float) -> Self:
        """Interest rate as a percent (4.36 means 4.36%, matching the InterestRateTick.rate convention) used in the Greeks calculation. Applied when rate_type selects a manual rate."""
        ...

    def binomial_steps(self, value: int) -> Self:
        """Number of steps in the binomial tree used for the Greeks calculation. Accurate at higher values and cheaper at lower ones; the server's cost grows with the square of the value. An even value is rounded up to the next odd one, and an `expiration=*` request is capped at 101 steps."""
        ...

    def version(self, value: Union[str, Version]) -> Self:
        """Greeks model version. Accepted values: `latest`, `1`."""
        ...

    def max_dte(self, value: int) -> Self:
        """Maximum days to expiration"""
        ...

    def strike_range(self, value: int) -> Self:
        """Strike range filter"""
        ...

    def start_date(self, value: Union[str, _date, datetime]) -> Self:
        """Start date YYYYMMDD"""
        ...

    def end_date(self, value: Union[str, _date, datetime]) -> Self:
        """End date YYYYMMDD"""
        ...

    def perf_boost_intraday(self, value: bool) -> Self:
        """When true, the underlying price for a current-day query is looked up with 1-second precision instead of millisecond precision, which answers faster. Defaults to false, matching the terminal, which sends false when the value is omitted."""
        ...


@final
class OptionHistoryGreeksFirstOrderBuilder(_ChunkedBuilder[GreeksFirstOrderTickList]):
    """Fluent builder for :meth:`MarketDataView.option_history_greeks_first_order`."""

    def symbol(self, value: str) -> Self:
        """Ticker symbol (e.g. AAPL)"""
        ...

    def expiration(self, value: Union[str, _date, datetime]) -> Self:
        """Expiration date YYYYMMDD"""
        ...

    def strike(self, value: str) -> Self:
        """Strike price in dollars as a string (e.g. 500 or 17.5). Use `*` for wildcard selection."""
        ...

    def right(self, value: Union[str, Right]) -> Self:
        """Option side. Use `both` or `*` (alias) for calls and puts. Accepted values: `call`, `put`, `both`, `*`."""
        ...

    def date(self, value: Union[str, _date, datetime]) -> Self:
        """Single date YYYYMMDD. Supply this for a single-day pull, or supply `start_date`/`end_date` for a range. When present, `date` takes precedence over the range."""
        ...

    def interval(self, value: Union[str, Interval]) -> Self:
        """Interval preset. Defaults to `1s` when omitted, matching the upstream ThetaData Python library. Accepted values: `tick`, `10ms`, `100ms`, `500ms`, `1s`, `5s`, `10s`, `15s`, `30s`, `1m`, `5m`, `10m`, `15m`, `30m`, `1h`."""
        ...

    def start_time(self, value: Union[str, time, datetime]) -> Self:
        """Start time filter"""
        ...

    def end_time(self, value: Union[str, time, datetime]) -> Self:
        """End time filter"""
        ...

    def annual_dividend(self, value: float) -> Self:
        """Annualized continuous dividend yield used in the Greeks calculation, as a decimal (e.g. 0.0042 for a 0.42% yield). Must be non-negative; to pass a dollar amount per share, use `annual_dividend_amount`."""
        ...

    def annual_dividend_amount(self, value: float) -> Self:
        """Annual dividend per share, in dollars, used in the Greeks calculation (e.g. 0.96 for $0.96 per year). Must be non-negative, and the server refuses it beside a non-zero `annual_dividend`."""
        ...

    def rate_type(self, value: Union[str, RateType]) -> Self:
        """Risk-free-rate source used in the Greeks calculation. Accepted values: `sofr`, `treasury_m1`, `treasury_m3`, `treasury_m6`, `treasury_y1`, `treasury_y2`, `treasury_y3`, `treasury_y5`, `treasury_y7`, `treasury_y10`, `treasury_y20`, `treasury_y30`."""
        ...

    def rate_value(self, value: float) -> Self:
        """Interest rate as a percent (4.36 means 4.36%, matching the InterestRateTick.rate convention) used in the Greeks calculation. Applied when rate_type selects a manual rate."""
        ...

    def version(self, value: Union[str, Version]) -> Self:
        """Greeks model version. Accepted values: `latest`, `1`."""
        ...

    def strike_range(self, value: int) -> Self:
        """Strike range filter"""
        ...

    def start_date(self, value: Union[str, _date, datetime]) -> Self:
        """Start date YYYYMMDD"""
        ...

    def end_date(self, value: Union[str, _date, datetime]) -> Self:
        """End date YYYYMMDD"""
        ...


@final
class OptionHistoryBinomialGreeksFirstOrderBuilder(_ChunkedBuilder[GreeksFirstOrderTickList]):
    """Fluent builder for :meth:`MarketDataView.option_history_binomial_greeks_first_order`."""

    def symbol(self, value: str) -> Self:
        """Ticker symbol (e.g. AAPL)"""
        ...

    def expiration(self, value: Union[str, _date, datetime]) -> Self:
        """Expiration date YYYYMMDD"""
        ...

    def strike(self, value: str) -> Self:
        """Strike price in dollars as a string (e.g. 500 or 17.5). Use `*` for wildcard selection."""
        ...

    def right(self, value: Union[str, Right]) -> Self:
        """Option side. Use `both` or `*` (alias) for calls and puts. Accepted values: `call`, `put`, `both`, `*`."""
        ...

    def date(self, value: Union[str, _date, datetime]) -> Self:
        """Single date YYYYMMDD. Supply this for a single-day pull, or supply `start_date`/`end_date` for a range. When present, `date` takes precedence over the range."""
        ...

    def interval(self, value: Union[str, Interval]) -> Self:
        """Interval preset. Defaults to `1s` when omitted, matching the upstream ThetaData Python library. Accepted values: `tick`, `10ms`, `100ms`, `500ms`, `1s`, `5s`, `10s`, `15s`, `30s`, `1m`, `5m`, `10m`, `15m`, `30m`, `1h`."""
        ...

    def start_time(self, value: Union[str, time, datetime]) -> Self:
        """Start time filter"""
        ...

    def end_time(self, value: Union[str, time, datetime]) -> Self:
        """End time filter"""
        ...

    def annual_dividend(self, value: float) -> Self:
        """Annualized continuous dividend yield used in the Greeks calculation, as a decimal (e.g. 0.0042 for a 0.42% yield). Must be non-negative; to pass a dollar amount per share, use `annual_dividend_amount`."""
        ...

    def annual_dividend_amount(self, value: float) -> Self:
        """Annual dividend per share, in dollars, used in the Greeks calculation (e.g. 0.96 for $0.96 per year). Must be non-negative, and the server refuses it beside a non-zero `annual_dividend`."""
        ...

    def rate_type(self, value: Union[str, RateType]) -> Self:
        """Risk-free-rate source used in the Greeks calculation. Accepted values: `sofr`, `treasury_m1`, `treasury_m3`, `treasury_m6`, `treasury_y1`, `treasury_y2`, `treasury_y3`, `treasury_y5`, `treasury_y7`, `treasury_y10`, `treasury_y20`, `treasury_y30`."""
        ...

    def rate_value(self, value: float) -> Self:
        """Interest rate as a percent (4.36 means 4.36%, matching the InterestRateTick.rate convention) used in the Greeks calculation. Applied when rate_type selects a manual rate."""
        ...

    def binomial_steps(self, value: int) -> Self:
        """Number of steps in the binomial tree used for the Greeks calculation. Accurate at higher values and cheaper at lower ones; the server's cost grows with the square of the value. An even value is rounded up to the next odd one, and an `expiration=*` request is capped at 101 steps."""
        ...

    def version(self, value: Union[str, Version]) -> Self:
        """Greeks model version. Accepted values: `latest`, `1`."""
        ...

    def strike_range(self, value: int) -> Self:
        """Strike range filter"""
        ...

    def start_date(self, value: Union[str, _date, datetime]) -> Self:
        """Start date YYYYMMDD"""
        ...

    def end_date(self, value: Union[str, _date, datetime]) -> Self:
        """End date YYYYMMDD"""
        ...


@final
class OptionHistoryTradeGreeksFirstOrderBuilder(_ChunkedBuilder[TradeGreeksFirstOrderTickList]):
    """Fluent builder for :meth:`MarketDataView.option_history_trade_greeks_first_order`."""

    def symbol(self, value: str) -> Self:
        """Ticker symbol (e.g. AAPL)"""
        ...

    def expiration(self, value: Union[str, _date, datetime]) -> Self:
        """Expiration date YYYYMMDD"""
        ...

    def strike(self, value: str) -> Self:
        """Strike price in dollars as a string (e.g. 500 or 17.5). Use `*` for wildcard selection."""
        ...

    def right(self, value: Union[str, Right]) -> Self:
        """Option side. Use `both` or `*` (alias) for calls and puts. Accepted values: `call`, `put`, `both`, `*`."""
        ...

    def date(self, value: Union[str, _date, datetime]) -> Self:
        """Single date YYYYMMDD. Supply this for a single-day pull, or supply `start_date`/`end_date` for a range. When present, `date` takes precedence over the range."""
        ...

    def start_time(self, value: Union[str, time, datetime]) -> Self:
        """Start time filter"""
        ...

    def end_time(self, value: Union[str, time, datetime]) -> Self:
        """End time filter"""
        ...

    def annual_dividend(self, value: float) -> Self:
        """Annualized continuous dividend yield used in the Greeks calculation, as a decimal (e.g. 0.0042 for a 0.42% yield). Must be non-negative; to pass a dollar amount per share, use `annual_dividend_amount`."""
        ...

    def annual_dividend_amount(self, value: float) -> Self:
        """Annual dividend per share, in dollars, used in the Greeks calculation (e.g. 0.96 for $0.96 per year). Must be non-negative, and the server refuses it beside a non-zero `annual_dividend`."""
        ...

    def rate_type(self, value: Union[str, RateType]) -> Self:
        """Risk-free-rate source used in the Greeks calculation. Accepted values: `sofr`, `treasury_m1`, `treasury_m3`, `treasury_m6`, `treasury_y1`, `treasury_y2`, `treasury_y3`, `treasury_y5`, `treasury_y7`, `treasury_y10`, `treasury_y20`, `treasury_y30`."""
        ...

    def rate_value(self, value: float) -> Self:
        """Interest rate as a percent (4.36 means 4.36%, matching the InterestRateTick.rate convention) used in the Greeks calculation. Applied when rate_type selects a manual rate."""
        ...

    def version(self, value: Union[str, Version]) -> Self:
        """Greeks model version. Accepted values: `latest`, `1`."""
        ...

    def max_dte(self, value: int) -> Self:
        """Maximum days to expiration"""
        ...

    def strike_range(self, value: int) -> Self:
        """Strike range filter"""
        ...

    def start_date(self, value: Union[str, _date, datetime]) -> Self:
        """Start date YYYYMMDD"""
        ...

    def end_date(self, value: Union[str, _date, datetime]) -> Self:
        """End date YYYYMMDD"""
        ...

    def perf_boost_intraday(self, value: bool) -> Self:
        """When true, the underlying price for a current-day query is looked up with 1-second precision instead of millisecond precision, which answers faster. Defaults to false, matching the terminal, which sends false when the value is omitted."""
        ...


@final
class OptionHistoryBinomialTradeGreeksFirstOrderBuilder(_ChunkedBuilder[TradeGreeksFirstOrderTickList]):
    """Fluent builder for :meth:`MarketDataView.option_history_binomial_trade_greeks_first_order`."""

    def symbol(self, value: str) -> Self:
        """Ticker symbol (e.g. AAPL)"""
        ...

    def expiration(self, value: Union[str, _date, datetime]) -> Self:
        """Expiration date YYYYMMDD"""
        ...

    def strike(self, value: str) -> Self:
        """Strike price in dollars as a string (e.g. 500 or 17.5). Use `*` for wildcard selection."""
        ...

    def right(self, value: Union[str, Right]) -> Self:
        """Option side. Use `both` or `*` (alias) for calls and puts. Accepted values: `call`, `put`, `both`, `*`."""
        ...

    def date(self, value: Union[str, _date, datetime]) -> Self:
        """Single date YYYYMMDD. Supply this for a single-day pull, or supply `start_date`/`end_date` for a range. When present, `date` takes precedence over the range."""
        ...

    def start_time(self, value: Union[str, time, datetime]) -> Self:
        """Start time filter"""
        ...

    def end_time(self, value: Union[str, time, datetime]) -> Self:
        """End time filter"""
        ...

    def annual_dividend(self, value: float) -> Self:
        """Annualized continuous dividend yield used in the Greeks calculation, as a decimal (e.g. 0.0042 for a 0.42% yield). Must be non-negative; to pass a dollar amount per share, use `annual_dividend_amount`."""
        ...

    def annual_dividend_amount(self, value: float) -> Self:
        """Annual dividend per share, in dollars, used in the Greeks calculation (e.g. 0.96 for $0.96 per year). Must be non-negative, and the server refuses it beside a non-zero `annual_dividend`."""
        ...

    def rate_type(self, value: Union[str, RateType]) -> Self:
        """Risk-free-rate source used in the Greeks calculation. Accepted values: `sofr`, `treasury_m1`, `treasury_m3`, `treasury_m6`, `treasury_y1`, `treasury_y2`, `treasury_y3`, `treasury_y5`, `treasury_y7`, `treasury_y10`, `treasury_y20`, `treasury_y30`."""
        ...

    def rate_value(self, value: float) -> Self:
        """Interest rate as a percent (4.36 means 4.36%, matching the InterestRateTick.rate convention) used in the Greeks calculation. Applied when rate_type selects a manual rate."""
        ...

    def binomial_steps(self, value: int) -> Self:
        """Number of steps in the binomial tree used for the Greeks calculation. Accurate at higher values and cheaper at lower ones; the server's cost grows with the square of the value. An even value is rounded up to the next odd one, and an `expiration=*` request is capped at 101 steps."""
        ...

    def version(self, value: Union[str, Version]) -> Self:
        """Greeks model version. Accepted values: `latest`, `1`."""
        ...

    def max_dte(self, value: int) -> Self:
        """Maximum days to expiration"""
        ...

    def strike_range(self, value: int) -> Self:
        """Strike range filter"""
        ...

    def start_date(self, value: Union[str, _date, datetime]) -> Self:
        """Start date YYYYMMDD"""
        ...

    def end_date(self, value: Union[str, _date, datetime]) -> Self:
        """End date YYYYMMDD"""
        ...

    def perf_boost_intraday(self, value: bool) -> Self:
        """When true, the underlying price for a current-day query is looked up with 1-second precision instead of millisecond precision, which answers faster. Defaults to false, matching the terminal, which sends false when the value is omitted."""
        ...


@final
class OptionHistoryGreeksSecondOrderBuilder(_ChunkedBuilder[GreeksSecondOrderTickList]):
    """Fluent builder for :meth:`MarketDataView.option_history_greeks_second_order`."""

    def symbol(self, value: str) -> Self:
        """Ticker symbol (e.g. AAPL)"""
        ...

    def expiration(self, value: Union[str, _date, datetime]) -> Self:
        """Expiration date YYYYMMDD"""
        ...

    def strike(self, value: str) -> Self:
        """Strike price in dollars as a string (e.g. 500 or 17.5). Use `*` for wildcard selection."""
        ...

    def right(self, value: Union[str, Right]) -> Self:
        """Option side. Use `both` or `*` (alias) for calls and puts. Accepted values: `call`, `put`, `both`, `*`."""
        ...

    def date(self, value: Union[str, _date, datetime]) -> Self:
        """Single date YYYYMMDD. Supply this for a single-day pull, or supply `start_date`/`end_date` for a range. When present, `date` takes precedence over the range."""
        ...

    def interval(self, value: Union[str, Interval]) -> Self:
        """Interval preset. Defaults to `1s` when omitted, matching the upstream ThetaData Python library. Accepted values: `tick`, `10ms`, `100ms`, `500ms`, `1s`, `5s`, `10s`, `15s`, `30s`, `1m`, `5m`, `10m`, `15m`, `30m`, `1h`."""
        ...

    def start_time(self, value: Union[str, time, datetime]) -> Self:
        """Start time filter"""
        ...

    def end_time(self, value: Union[str, time, datetime]) -> Self:
        """End time filter"""
        ...

    def annual_dividend(self, value: float) -> Self:
        """Annualized continuous dividend yield used in the Greeks calculation, as a decimal (e.g. 0.0042 for a 0.42% yield). Must be non-negative; to pass a dollar amount per share, use `annual_dividend_amount`."""
        ...

    def annual_dividend_amount(self, value: float) -> Self:
        """Annual dividend per share, in dollars, used in the Greeks calculation (e.g. 0.96 for $0.96 per year). Must be non-negative, and the server refuses it beside a non-zero `annual_dividend`."""
        ...

    def rate_type(self, value: Union[str, RateType]) -> Self:
        """Risk-free-rate source used in the Greeks calculation. Accepted values: `sofr`, `treasury_m1`, `treasury_m3`, `treasury_m6`, `treasury_y1`, `treasury_y2`, `treasury_y3`, `treasury_y5`, `treasury_y7`, `treasury_y10`, `treasury_y20`, `treasury_y30`."""
        ...

    def rate_value(self, value: float) -> Self:
        """Interest rate as a percent (4.36 means 4.36%, matching the InterestRateTick.rate convention) used in the Greeks calculation. Applied when rate_type selects a manual rate."""
        ...

    def version(self, value: Union[str, Version]) -> Self:
        """Greeks model version. Accepted values: `latest`, `1`."""
        ...

    def strike_range(self, value: int) -> Self:
        """Strike range filter"""
        ...

    def start_date(self, value: Union[str, _date, datetime]) -> Self:
        """Start date YYYYMMDD"""
        ...

    def end_date(self, value: Union[str, _date, datetime]) -> Self:
        """End date YYYYMMDD"""
        ...


@final
class OptionHistoryBinomialGreeksSecondOrderBuilder(_ChunkedBuilder[GreeksSecondOrderTickList]):
    """Fluent builder for :meth:`MarketDataView.option_history_binomial_greeks_second_order`."""

    def symbol(self, value: str) -> Self:
        """Ticker symbol (e.g. AAPL)"""
        ...

    def expiration(self, value: Union[str, _date, datetime]) -> Self:
        """Expiration date YYYYMMDD"""
        ...

    def strike(self, value: str) -> Self:
        """Strike price in dollars as a string (e.g. 500 or 17.5). Use `*` for wildcard selection."""
        ...

    def right(self, value: Union[str, Right]) -> Self:
        """Option side. Use `both` or `*` (alias) for calls and puts. Accepted values: `call`, `put`, `both`, `*`."""
        ...

    def date(self, value: Union[str, _date, datetime]) -> Self:
        """Single date YYYYMMDD. Supply this for a single-day pull, or supply `start_date`/`end_date` for a range. When present, `date` takes precedence over the range."""
        ...

    def interval(self, value: Union[str, Interval]) -> Self:
        """Interval preset. Defaults to `1s` when omitted, matching the upstream ThetaData Python library. Accepted values: `tick`, `10ms`, `100ms`, `500ms`, `1s`, `5s`, `10s`, `15s`, `30s`, `1m`, `5m`, `10m`, `15m`, `30m`, `1h`."""
        ...

    def start_time(self, value: Union[str, time, datetime]) -> Self:
        """Start time filter"""
        ...

    def end_time(self, value: Union[str, time, datetime]) -> Self:
        """End time filter"""
        ...

    def annual_dividend(self, value: float) -> Self:
        """Annualized continuous dividend yield used in the Greeks calculation, as a decimal (e.g. 0.0042 for a 0.42% yield). Must be non-negative; to pass a dollar amount per share, use `annual_dividend_amount`."""
        ...

    def annual_dividend_amount(self, value: float) -> Self:
        """Annual dividend per share, in dollars, used in the Greeks calculation (e.g. 0.96 for $0.96 per year). Must be non-negative, and the server refuses it beside a non-zero `annual_dividend`."""
        ...

    def rate_type(self, value: Union[str, RateType]) -> Self:
        """Risk-free-rate source used in the Greeks calculation. Accepted values: `sofr`, `treasury_m1`, `treasury_m3`, `treasury_m6`, `treasury_y1`, `treasury_y2`, `treasury_y3`, `treasury_y5`, `treasury_y7`, `treasury_y10`, `treasury_y20`, `treasury_y30`."""
        ...

    def rate_value(self, value: float) -> Self:
        """Interest rate as a percent (4.36 means 4.36%, matching the InterestRateTick.rate convention) used in the Greeks calculation. Applied when rate_type selects a manual rate."""
        ...

    def binomial_steps(self, value: int) -> Self:
        """Number of steps in the binomial tree used for the Greeks calculation. Accurate at higher values and cheaper at lower ones; the server's cost grows with the square of the value. An even value is rounded up to the next odd one, and an `expiration=*` request is capped at 101 steps."""
        ...

    def version(self, value: Union[str, Version]) -> Self:
        """Greeks model version. Accepted values: `latest`, `1`."""
        ...

    def strike_range(self, value: int) -> Self:
        """Strike range filter"""
        ...

    def start_date(self, value: Union[str, _date, datetime]) -> Self:
        """Start date YYYYMMDD"""
        ...

    def end_date(self, value: Union[str, _date, datetime]) -> Self:
        """End date YYYYMMDD"""
        ...


@final
class OptionHistoryTradeGreeksSecondOrderBuilder(_ChunkedBuilder[TradeGreeksSecondOrderTickList]):
    """Fluent builder for :meth:`MarketDataView.option_history_trade_greeks_second_order`."""

    def symbol(self, value: str) -> Self:
        """Ticker symbol (e.g. AAPL)"""
        ...

    def expiration(self, value: Union[str, _date, datetime]) -> Self:
        """Expiration date YYYYMMDD"""
        ...

    def strike(self, value: str) -> Self:
        """Strike price in dollars as a string (e.g. 500 or 17.5). Use `*` for wildcard selection."""
        ...

    def right(self, value: Union[str, Right]) -> Self:
        """Option side. Use `both` or `*` (alias) for calls and puts. Accepted values: `call`, `put`, `both`, `*`."""
        ...

    def date(self, value: Union[str, _date, datetime]) -> Self:
        """Single date YYYYMMDD. Supply this for a single-day pull, or supply `start_date`/`end_date` for a range. When present, `date` takes precedence over the range."""
        ...

    def start_time(self, value: Union[str, time, datetime]) -> Self:
        """Start time filter"""
        ...

    def end_time(self, value: Union[str, time, datetime]) -> Self:
        """End time filter"""
        ...

    def annual_dividend(self, value: float) -> Self:
        """Annualized continuous dividend yield used in the Greeks calculation, as a decimal (e.g. 0.0042 for a 0.42% yield). Must be non-negative; to pass a dollar amount per share, use `annual_dividend_amount`."""
        ...

    def annual_dividend_amount(self, value: float) -> Self:
        """Annual dividend per share, in dollars, used in the Greeks calculation (e.g. 0.96 for $0.96 per year). Must be non-negative, and the server refuses it beside a non-zero `annual_dividend`."""
        ...

    def rate_type(self, value: Union[str, RateType]) -> Self:
        """Risk-free-rate source used in the Greeks calculation. Accepted values: `sofr`, `treasury_m1`, `treasury_m3`, `treasury_m6`, `treasury_y1`, `treasury_y2`, `treasury_y3`, `treasury_y5`, `treasury_y7`, `treasury_y10`, `treasury_y20`, `treasury_y30`."""
        ...

    def rate_value(self, value: float) -> Self:
        """Interest rate as a percent (4.36 means 4.36%, matching the InterestRateTick.rate convention) used in the Greeks calculation. Applied when rate_type selects a manual rate."""
        ...

    def version(self, value: Union[str, Version]) -> Self:
        """Greeks model version. Accepted values: `latest`, `1`."""
        ...

    def max_dte(self, value: int) -> Self:
        """Maximum days to expiration"""
        ...

    def strike_range(self, value: int) -> Self:
        """Strike range filter"""
        ...

    def start_date(self, value: Union[str, _date, datetime]) -> Self:
        """Start date YYYYMMDD"""
        ...

    def end_date(self, value: Union[str, _date, datetime]) -> Self:
        """End date YYYYMMDD"""
        ...

    def perf_boost_intraday(self, value: bool) -> Self:
        """When true, the underlying price for a current-day query is looked up with 1-second precision instead of millisecond precision, which answers faster. Defaults to false, matching the terminal, which sends false when the value is omitted."""
        ...


@final
class OptionHistoryBinomialTradeGreeksSecondOrderBuilder(_ChunkedBuilder[TradeGreeksSecondOrderTickList]):
    """Fluent builder for :meth:`MarketDataView.option_history_binomial_trade_greeks_second_order`."""

    def symbol(self, value: str) -> Self:
        """Ticker symbol (e.g. AAPL)"""
        ...

    def expiration(self, value: Union[str, _date, datetime]) -> Self:
        """Expiration date YYYYMMDD"""
        ...

    def strike(self, value: str) -> Self:
        """Strike price in dollars as a string (e.g. 500 or 17.5). Use `*` for wildcard selection."""
        ...

    def right(self, value: Union[str, Right]) -> Self:
        """Option side. Use `both` or `*` (alias) for calls and puts. Accepted values: `call`, `put`, `both`, `*`."""
        ...

    def date(self, value: Union[str, _date, datetime]) -> Self:
        """Single date YYYYMMDD. Supply this for a single-day pull, or supply `start_date`/`end_date` for a range. When present, `date` takes precedence over the range."""
        ...

    def start_time(self, value: Union[str, time, datetime]) -> Self:
        """Start time filter"""
        ...

    def end_time(self, value: Union[str, time, datetime]) -> Self:
        """End time filter"""
        ...

    def annual_dividend(self, value: float) -> Self:
        """Annualized continuous dividend yield used in the Greeks calculation, as a decimal (e.g. 0.0042 for a 0.42% yield). Must be non-negative; to pass a dollar amount per share, use `annual_dividend_amount`."""
        ...

    def annual_dividend_amount(self, value: float) -> Self:
        """Annual dividend per share, in dollars, used in the Greeks calculation (e.g. 0.96 for $0.96 per year). Must be non-negative, and the server refuses it beside a non-zero `annual_dividend`."""
        ...

    def rate_type(self, value: Union[str, RateType]) -> Self:
        """Risk-free-rate source used in the Greeks calculation. Accepted values: `sofr`, `treasury_m1`, `treasury_m3`, `treasury_m6`, `treasury_y1`, `treasury_y2`, `treasury_y3`, `treasury_y5`, `treasury_y7`, `treasury_y10`, `treasury_y20`, `treasury_y30`."""
        ...

    def rate_value(self, value: float) -> Self:
        """Interest rate as a percent (4.36 means 4.36%, matching the InterestRateTick.rate convention) used in the Greeks calculation. Applied when rate_type selects a manual rate."""
        ...

    def binomial_steps(self, value: int) -> Self:
        """Number of steps in the binomial tree used for the Greeks calculation. Accurate at higher values and cheaper at lower ones; the server's cost grows with the square of the value. An even value is rounded up to the next odd one, and an `expiration=*` request is capped at 101 steps."""
        ...

    def version(self, value: Union[str, Version]) -> Self:
        """Greeks model version. Accepted values: `latest`, `1`."""
        ...

    def max_dte(self, value: int) -> Self:
        """Maximum days to expiration"""
        ...

    def strike_range(self, value: int) -> Self:
        """Strike range filter"""
        ...

    def start_date(self, value: Union[str, _date, datetime]) -> Self:
        """Start date YYYYMMDD"""
        ...

    def end_date(self, value: Union[str, _date, datetime]) -> Self:
        """End date YYYYMMDD"""
        ...

    def perf_boost_intraday(self, value: bool) -> Self:
        """When true, the underlying price for a current-day query is looked up with 1-second precision instead of millisecond precision, which answers faster. Defaults to false, matching the terminal, which sends false when the value is omitted."""
        ...


@final
class OptionHistoryGreeksThirdOrderBuilder(_ChunkedBuilder[GreeksThirdOrderTickList]):
    """Fluent builder for :meth:`MarketDataView.option_history_greeks_third_order`."""

    def symbol(self, value: str) -> Self:
        """Ticker symbol (e.g. AAPL)"""
        ...

    def expiration(self, value: Union[str, _date, datetime]) -> Self:
        """Expiration date YYYYMMDD"""
        ...

    def strike(self, value: str) -> Self:
        """Strike price in dollars as a string (e.g. 500 or 17.5). Use `*` for wildcard selection."""
        ...

    def right(self, value: Union[str, Right]) -> Self:
        """Option side. Use `both` or `*` (alias) for calls and puts. Accepted values: `call`, `put`, `both`, `*`."""
        ...

    def date(self, value: Union[str, _date, datetime]) -> Self:
        """Single date YYYYMMDD. Supply this for a single-day pull, or supply `start_date`/`end_date` for a range. When present, `date` takes precedence over the range."""
        ...

    def interval(self, value: Union[str, Interval]) -> Self:
        """Interval preset. Defaults to `1s` when omitted, matching the upstream ThetaData Python library. Accepted values: `tick`, `10ms`, `100ms`, `500ms`, `1s`, `5s`, `10s`, `15s`, `30s`, `1m`, `5m`, `10m`, `15m`, `30m`, `1h`."""
        ...

    def start_time(self, value: Union[str, time, datetime]) -> Self:
        """Start time filter"""
        ...

    def end_time(self, value: Union[str, time, datetime]) -> Self:
        """End time filter"""
        ...

    def annual_dividend(self, value: float) -> Self:
        """Annualized continuous dividend yield used in the Greeks calculation, as a decimal (e.g. 0.0042 for a 0.42% yield). Must be non-negative; to pass a dollar amount per share, use `annual_dividend_amount`."""
        ...

    def annual_dividend_amount(self, value: float) -> Self:
        """Annual dividend per share, in dollars, used in the Greeks calculation (e.g. 0.96 for $0.96 per year). Must be non-negative, and the server refuses it beside a non-zero `annual_dividend`."""
        ...

    def rate_type(self, value: Union[str, RateType]) -> Self:
        """Risk-free-rate source used in the Greeks calculation. Accepted values: `sofr`, `treasury_m1`, `treasury_m3`, `treasury_m6`, `treasury_y1`, `treasury_y2`, `treasury_y3`, `treasury_y5`, `treasury_y7`, `treasury_y10`, `treasury_y20`, `treasury_y30`."""
        ...

    def rate_value(self, value: float) -> Self:
        """Interest rate as a percent (4.36 means 4.36%, matching the InterestRateTick.rate convention) used in the Greeks calculation. Applied when rate_type selects a manual rate."""
        ...

    def version(self, value: Union[str, Version]) -> Self:
        """Greeks model version. Accepted values: `latest`, `1`."""
        ...

    def strike_range(self, value: int) -> Self:
        """Strike range filter"""
        ...

    def start_date(self, value: Union[str, _date, datetime]) -> Self:
        """Start date YYYYMMDD"""
        ...

    def end_date(self, value: Union[str, _date, datetime]) -> Self:
        """End date YYYYMMDD"""
        ...


@final
class OptionHistoryBinomialGreeksThirdOrderBuilder(_ChunkedBuilder[GreeksThirdOrderTickList]):
    """Fluent builder for :meth:`MarketDataView.option_history_binomial_greeks_third_order`."""

    def symbol(self, value: str) -> Self:
        """Ticker symbol (e.g. AAPL)"""
        ...

    def expiration(self, value: Union[str, _date, datetime]) -> Self:
        """Expiration date YYYYMMDD"""
        ...

    def strike(self, value: str) -> Self:
        """Strike price in dollars as a string (e.g. 500 or 17.5). Use `*` for wildcard selection."""
        ...

    def right(self, value: Union[str, Right]) -> Self:
        """Option side. Use `both` or `*` (alias) for calls and puts. Accepted values: `call`, `put`, `both`, `*`."""
        ...

    def date(self, value: Union[str, _date, datetime]) -> Self:
        """Single date YYYYMMDD. Supply this for a single-day pull, or supply `start_date`/`end_date` for a range. When present, `date` takes precedence over the range."""
        ...

    def interval(self, value: Union[str, Interval]) -> Self:
        """Interval preset. Defaults to `1s` when omitted, matching the upstream ThetaData Python library. Accepted values: `tick`, `10ms`, `100ms`, `500ms`, `1s`, `5s`, `10s`, `15s`, `30s`, `1m`, `5m`, `10m`, `15m`, `30m`, `1h`."""
        ...

    def start_time(self, value: Union[str, time, datetime]) -> Self:
        """Start time filter"""
        ...

    def end_time(self, value: Union[str, time, datetime]) -> Self:
        """End time filter"""
        ...

    def annual_dividend(self, value: float) -> Self:
        """Annualized continuous dividend yield used in the Greeks calculation, as a decimal (e.g. 0.0042 for a 0.42% yield). Must be non-negative; to pass a dollar amount per share, use `annual_dividend_amount`."""
        ...

    def annual_dividend_amount(self, value: float) -> Self:
        """Annual dividend per share, in dollars, used in the Greeks calculation (e.g. 0.96 for $0.96 per year). Must be non-negative, and the server refuses it beside a non-zero `annual_dividend`."""
        ...

    def rate_type(self, value: Union[str, RateType]) -> Self:
        """Risk-free-rate source used in the Greeks calculation. Accepted values: `sofr`, `treasury_m1`, `treasury_m3`, `treasury_m6`, `treasury_y1`, `treasury_y2`, `treasury_y3`, `treasury_y5`, `treasury_y7`, `treasury_y10`, `treasury_y20`, `treasury_y30`."""
        ...

    def rate_value(self, value: float) -> Self:
        """Interest rate as a percent (4.36 means 4.36%, matching the InterestRateTick.rate convention) used in the Greeks calculation. Applied when rate_type selects a manual rate."""
        ...

    def binomial_steps(self, value: int) -> Self:
        """Number of steps in the binomial tree used for the Greeks calculation. Accurate at higher values and cheaper at lower ones; the server's cost grows with the square of the value. An even value is rounded up to the next odd one, and an `expiration=*` request is capped at 101 steps."""
        ...

    def version(self, value: Union[str, Version]) -> Self:
        """Greeks model version. Accepted values: `latest`, `1`."""
        ...

    def strike_range(self, value: int) -> Self:
        """Strike range filter"""
        ...

    def start_date(self, value: Union[str, _date, datetime]) -> Self:
        """Start date YYYYMMDD"""
        ...

    def end_date(self, value: Union[str, _date, datetime]) -> Self:
        """End date YYYYMMDD"""
        ...


@final
class OptionHistoryTradeGreeksThirdOrderBuilder(_ChunkedBuilder[TradeGreeksThirdOrderTickList]):
    """Fluent builder for :meth:`MarketDataView.option_history_trade_greeks_third_order`."""

    def symbol(self, value: str) -> Self:
        """Ticker symbol (e.g. AAPL)"""
        ...

    def expiration(self, value: Union[str, _date, datetime]) -> Self:
        """Expiration date YYYYMMDD"""
        ...

    def strike(self, value: str) -> Self:
        """Strike price in dollars as a string (e.g. 500 or 17.5). Use `*` for wildcard selection."""
        ...

    def right(self, value: Union[str, Right]) -> Self:
        """Option side. Use `both` or `*` (alias) for calls and puts. Accepted values: `call`, `put`, `both`, `*`."""
        ...

    def date(self, value: Union[str, _date, datetime]) -> Self:
        """Single date YYYYMMDD. Supply this for a single-day pull, or supply `start_date`/`end_date` for a range. When present, `date` takes precedence over the range."""
        ...

    def start_time(self, value: Union[str, time, datetime]) -> Self:
        """Start time filter"""
        ...

    def end_time(self, value: Union[str, time, datetime]) -> Self:
        """End time filter"""
        ...

    def annual_dividend(self, value: float) -> Self:
        """Annualized continuous dividend yield used in the Greeks calculation, as a decimal (e.g. 0.0042 for a 0.42% yield). Must be non-negative; to pass a dollar amount per share, use `annual_dividend_amount`."""
        ...

    def annual_dividend_amount(self, value: float) -> Self:
        """Annual dividend per share, in dollars, used in the Greeks calculation (e.g. 0.96 for $0.96 per year). Must be non-negative, and the server refuses it beside a non-zero `annual_dividend`."""
        ...

    def rate_type(self, value: Union[str, RateType]) -> Self:
        """Risk-free-rate source used in the Greeks calculation. Accepted values: `sofr`, `treasury_m1`, `treasury_m3`, `treasury_m6`, `treasury_y1`, `treasury_y2`, `treasury_y3`, `treasury_y5`, `treasury_y7`, `treasury_y10`, `treasury_y20`, `treasury_y30`."""
        ...

    def rate_value(self, value: float) -> Self:
        """Interest rate as a percent (4.36 means 4.36%, matching the InterestRateTick.rate convention) used in the Greeks calculation. Applied when rate_type selects a manual rate."""
        ...

    def version(self, value: Union[str, Version]) -> Self:
        """Greeks model version. Accepted values: `latest`, `1`."""
        ...

    def max_dte(self, value: int) -> Self:
        """Maximum days to expiration"""
        ...

    def strike_range(self, value: int) -> Self:
        """Strike range filter"""
        ...

    def start_date(self, value: Union[str, _date, datetime]) -> Self:
        """Start date YYYYMMDD"""
        ...

    def end_date(self, value: Union[str, _date, datetime]) -> Self:
        """End date YYYYMMDD"""
        ...

    def perf_boost_intraday(self, value: bool) -> Self:
        """When true, the underlying price for a current-day query is looked up with 1-second precision instead of millisecond precision, which answers faster. Defaults to false, matching the terminal, which sends false when the value is omitted."""
        ...


@final
class OptionHistoryBinomialTradeGreeksThirdOrderBuilder(_ChunkedBuilder[TradeGreeksThirdOrderTickList]):
    """Fluent builder for :meth:`MarketDataView.option_history_binomial_trade_greeks_third_order`."""

    def symbol(self, value: str) -> Self:
        """Ticker symbol (e.g. AAPL)"""
        ...

    def expiration(self, value: Union[str, _date, datetime]) -> Self:
        """Expiration date YYYYMMDD"""
        ...

    def strike(self, value: str) -> Self:
        """Strike price in dollars as a string (e.g. 500 or 17.5). Use `*` for wildcard selection."""
        ...

    def right(self, value: Union[str, Right]) -> Self:
        """Option side. Use `both` or `*` (alias) for calls and puts. Accepted values: `call`, `put`, `both`, `*`."""
        ...

    def date(self, value: Union[str, _date, datetime]) -> Self:
        """Single date YYYYMMDD. Supply this for a single-day pull, or supply `start_date`/`end_date` for a range. When present, `date` takes precedence over the range."""
        ...

    def start_time(self, value: Union[str, time, datetime]) -> Self:
        """Start time filter"""
        ...

    def end_time(self, value: Union[str, time, datetime]) -> Self:
        """End time filter"""
        ...

    def annual_dividend(self, value: float) -> Self:
        """Annualized continuous dividend yield used in the Greeks calculation, as a decimal (e.g. 0.0042 for a 0.42% yield). Must be non-negative; to pass a dollar amount per share, use `annual_dividend_amount`."""
        ...

    def annual_dividend_amount(self, value: float) -> Self:
        """Annual dividend per share, in dollars, used in the Greeks calculation (e.g. 0.96 for $0.96 per year). Must be non-negative, and the server refuses it beside a non-zero `annual_dividend`."""
        ...

    def rate_type(self, value: Union[str, RateType]) -> Self:
        """Risk-free-rate source used in the Greeks calculation. Accepted values: `sofr`, `treasury_m1`, `treasury_m3`, `treasury_m6`, `treasury_y1`, `treasury_y2`, `treasury_y3`, `treasury_y5`, `treasury_y7`, `treasury_y10`, `treasury_y20`, `treasury_y30`."""
        ...

    def rate_value(self, value: float) -> Self:
        """Interest rate as a percent (4.36 means 4.36%, matching the InterestRateTick.rate convention) used in the Greeks calculation. Applied when rate_type selects a manual rate."""
        ...

    def binomial_steps(self, value: int) -> Self:
        """Number of steps in the binomial tree used for the Greeks calculation. Accurate at higher values and cheaper at lower ones; the server's cost grows with the square of the value. An even value is rounded up to the next odd one, and an `expiration=*` request is capped at 101 steps."""
        ...

    def version(self, value: Union[str, Version]) -> Self:
        """Greeks model version. Accepted values: `latest`, `1`."""
        ...

    def max_dte(self, value: int) -> Self:
        """Maximum days to expiration"""
        ...

    def strike_range(self, value: int) -> Self:
        """Strike range filter"""
        ...

    def start_date(self, value: Union[str, _date, datetime]) -> Self:
        """Start date YYYYMMDD"""
        ...

    def end_date(self, value: Union[str, _date, datetime]) -> Self:
        """End date YYYYMMDD"""
        ...

    def perf_boost_intraday(self, value: bool) -> Self:
        """When true, the underlying price for a current-day query is looked up with 1-second precision instead of millisecond precision, which answers faster. Defaults to false, matching the terminal, which sends false when the value is omitted."""
        ...


@final
class OptionHistoryGreeksImpliedVolatilityBuilder(_ChunkedBuilder[IvTickList]):
    """Fluent builder for :meth:`MarketDataView.option_history_greeks_implied_volatility`."""

    def symbol(self, value: str) -> Self:
        """Ticker symbol (e.g. AAPL)"""
        ...

    def expiration(self, value: Union[str, _date, datetime]) -> Self:
        """Expiration date YYYYMMDD"""
        ...

    def strike(self, value: str) -> Self:
        """Strike price in dollars as a string (e.g. 500 or 17.5). Use `*` for wildcard selection."""
        ...

    def right(self, value: Union[str, Right]) -> Self:
        """Option side. Use `both` or `*` (alias) for calls and puts. Accepted values: `call`, `put`, `both`, `*`."""
        ...

    def date(self, value: Union[str, _date, datetime]) -> Self:
        """Single date YYYYMMDD. Supply this for a single-day pull, or supply `start_date`/`end_date` for a range. When present, `date` takes precedence over the range."""
        ...

    def interval(self, value: Union[str, Interval]) -> Self:
        """Interval preset. Defaults to `1s` when omitted, matching the upstream ThetaData Python library. Accepted values: `tick`, `10ms`, `100ms`, `500ms`, `1s`, `5s`, `10s`, `15s`, `30s`, `1m`, `5m`, `10m`, `15m`, `30m`, `1h`."""
        ...

    def start_time(self, value: Union[str, time, datetime]) -> Self:
        """Start time filter"""
        ...

    def end_time(self, value: Union[str, time, datetime]) -> Self:
        """End time filter"""
        ...

    def annual_dividend(self, value: float) -> Self:
        """Annualized continuous dividend yield used in the Greeks calculation, as a decimal (e.g. 0.0042 for a 0.42% yield). Must be non-negative; to pass a dollar amount per share, use `annual_dividend_amount`."""
        ...

    def annual_dividend_amount(self, value: float) -> Self:
        """Annual dividend per share, in dollars, used in the Greeks calculation (e.g. 0.96 for $0.96 per year). Must be non-negative, and the server refuses it beside a non-zero `annual_dividend`."""
        ...

    def rate_type(self, value: Union[str, RateType]) -> Self:
        """Risk-free-rate source used in the Greeks calculation. Accepted values: `sofr`, `treasury_m1`, `treasury_m3`, `treasury_m6`, `treasury_y1`, `treasury_y2`, `treasury_y3`, `treasury_y5`, `treasury_y7`, `treasury_y10`, `treasury_y20`, `treasury_y30`."""
        ...

    def rate_value(self, value: float) -> Self:
        """Interest rate as a percent (4.36 means 4.36%, matching the InterestRateTick.rate convention) used in the Greeks calculation. Applied when rate_type selects a manual rate."""
        ...

    def version(self, value: Union[str, Version]) -> Self:
        """Greeks model version. Accepted values: `latest`, `1`."""
        ...

    def strike_range(self, value: int) -> Self:
        """Strike range filter"""
        ...

    def start_date(self, value: Union[str, _date, datetime]) -> Self:
        """Start date YYYYMMDD"""
        ...

    def end_date(self, value: Union[str, _date, datetime]) -> Self:
        """End date YYYYMMDD"""
        ...


@final
class OptionHistoryTradeGreeksImpliedVolatilityBuilder(_ChunkedBuilder[TradeGreeksImpliedVolatilityTickList]):
    """Fluent builder for :meth:`MarketDataView.option_history_trade_greeks_implied_volatility`."""

    def symbol(self, value: str) -> Self:
        """Ticker symbol (e.g. AAPL)"""
        ...

    def expiration(self, value: Union[str, _date, datetime]) -> Self:
        """Expiration date YYYYMMDD"""
        ...

    def strike(self, value: str) -> Self:
        """Strike price in dollars as a string (e.g. 500 or 17.5). Use `*` for wildcard selection."""
        ...

    def right(self, value: Union[str, Right]) -> Self:
        """Option side. Use `both` or `*` (alias) for calls and puts. Accepted values: `call`, `put`, `both`, `*`."""
        ...

    def date(self, value: Union[str, _date, datetime]) -> Self:
        """Single date YYYYMMDD. Supply this for a single-day pull, or supply `start_date`/`end_date` for a range. When present, `date` takes precedence over the range."""
        ...

    def start_time(self, value: Union[str, time, datetime]) -> Self:
        """Start time filter"""
        ...

    def end_time(self, value: Union[str, time, datetime]) -> Self:
        """End time filter"""
        ...

    def annual_dividend(self, value: float) -> Self:
        """Annualized continuous dividend yield used in the Greeks calculation, as a decimal (e.g. 0.0042 for a 0.42% yield). Must be non-negative; to pass a dollar amount per share, use `annual_dividend_amount`."""
        ...

    def annual_dividend_amount(self, value: float) -> Self:
        """Annual dividend per share, in dollars, used in the Greeks calculation (e.g. 0.96 for $0.96 per year). Must be non-negative, and the server refuses it beside a non-zero `annual_dividend`."""
        ...

    def rate_type(self, value: Union[str, RateType]) -> Self:
        """Risk-free-rate source used in the Greeks calculation. Accepted values: `sofr`, `treasury_m1`, `treasury_m3`, `treasury_m6`, `treasury_y1`, `treasury_y2`, `treasury_y3`, `treasury_y5`, `treasury_y7`, `treasury_y10`, `treasury_y20`, `treasury_y30`."""
        ...

    def rate_value(self, value: float) -> Self:
        """Interest rate as a percent (4.36 means 4.36%, matching the InterestRateTick.rate convention) used in the Greeks calculation. Applied when rate_type selects a manual rate."""
        ...

    def version(self, value: Union[str, Version]) -> Self:
        """Greeks model version. Accepted values: `latest`, `1`."""
        ...

    def max_dte(self, value: int) -> Self:
        """Maximum days to expiration"""
        ...

    def strike_range(self, value: int) -> Self:
        """Strike range filter"""
        ...

    def start_date(self, value: Union[str, _date, datetime]) -> Self:
        """Start date YYYYMMDD"""
        ...

    def end_date(self, value: Union[str, _date, datetime]) -> Self:
        """End date YYYYMMDD"""
        ...

    def perf_boost_intraday(self, value: bool) -> Self:
        """When true, the underlying price for a current-day query is looked up with 1-second precision instead of millisecond precision, which answers faster. Defaults to false, matching the terminal, which sends false when the value is omitted."""
        ...


@final
class OptionAtTimeTradeBuilder(_ChunkedBuilder[TradeTickList]):
    """Fluent builder for :meth:`MarketDataView.option_at_time_trade`."""

    def symbol(self, value: str) -> Self:
        """Ticker symbol (e.g. AAPL)"""
        ...

    def expiration(self, value: Union[str, _date, datetime]) -> Self:
        """Expiration date YYYYMMDD"""
        ...

    def strike(self, value: str) -> Self:
        """Strike price in dollars as a string (e.g. 500 or 17.5). Use `*` for wildcard selection."""
        ...

    def right(self, value: Union[str, Right]) -> Self:
        """Option side. Use `both` or `*` (alias) for calls and puts. Accepted values: `call`, `put`, `both`, `*`."""
        ...

    def start_date(self, value: Union[str, _date, datetime]) -> Self:
        """Start date YYYYMMDD"""
        ...

    def end_date(self, value: Union[str, _date, datetime]) -> Self:
        """End date YYYYMMDD"""
        ...

    def time_of_day(self, value: Union[str, time, datetime]) -> Self:
        """ET wall-clock time in HH:MM:SS.SSS (e.g. 09:30:00.000 for 9:30 AM ET; legacy 34200000 is also accepted)"""
        ...

    def max_dte(self, value: int) -> Self:
        """Maximum days to expiration"""
        ...

    def strike_range(self, value: int) -> Self:
        """Strike range filter"""
        ...


@final
class OptionAtTimeQuoteBuilder(_ChunkedBuilder[QuoteTickList]):
    """Fluent builder for :meth:`MarketDataView.option_at_time_quote`."""

    def symbol(self, value: str) -> Self:
        """Ticker symbol (e.g. AAPL)"""
        ...

    def expiration(self, value: Union[str, _date, datetime]) -> Self:
        """Expiration date YYYYMMDD"""
        ...

    def strike(self, value: str) -> Self:
        """Strike price in dollars as a string (e.g. 500 or 17.5). Use `*` for wildcard selection."""
        ...

    def right(self, value: Union[str, Right]) -> Self:
        """Option side. Use `both` or `*` (alias) for calls and puts. Accepted values: `call`, `put`, `both`, `*`."""
        ...

    def start_date(self, value: Union[str, _date, datetime]) -> Self:
        """Start date YYYYMMDD"""
        ...

    def end_date(self, value: Union[str, _date, datetime]) -> Self:
        """End date YYYYMMDD"""
        ...

    def time_of_day(self, value: Union[str, time, datetime]) -> Self:
        """ET wall-clock time in HH:MM:SS.SSS (e.g. 09:30:00.000 for 9:30 AM ET; legacy 34200000 is also accepted)"""
        ...

    def max_dte(self, value: int) -> Self:
        """Maximum days to expiration"""
        ...

    def strike_range(self, value: int) -> Self:
        """Strike range filter"""
        ...


@final
class OptionFlatFileEodBuilder(_ChunkedBuilder[EodTickList]):
    """Fluent builder for :meth:`MarketDataView.option_flat_file_eod`."""

    def date(self, value: Union[str, _date, datetime]) -> Self:
        """Date YYYYMMDD"""
        ...


@final
class OptionFlatFileTradeQuoteBuilder(_ChunkedBuilder[TradeQuoteTickList]):
    """Fluent builder for :meth:`MarketDataView.option_flat_file_trade_quote`."""

    def date(self, value: Union[str, _date, datetime]) -> Self:
        """Date YYYYMMDD"""
        ...


@final
class OptionFlatFileOpenInterestBuilder(_ChunkedBuilder[OpenInterestTickList]):
    """Fluent builder for :meth:`MarketDataView.option_flat_file_open_interest`."""

    def date(self, value: Union[str, _date, datetime]) -> Self:
        """Date YYYYMMDD"""
        ...


@final
class OptionFlatFileQuoteBuilder(_ChunkedBuilder[IntervalQuoteTickList]):
    """Fluent builder for :meth:`MarketDataView.option_flat_file_quote`."""

    def date(self, value: Union[str, _date, datetime]) -> Self:
        """Date YYYYMMDD"""
        ...


@final
class IndexListSymbolsBuilder(_Builder[StringList]):
    """Fluent builder for :meth:`MarketDataView.index_list_symbols`."""


@final
class IndexListDatesBuilder(_Builder[StringList]):
    """Fluent builder for :meth:`MarketDataView.index_list_dates`."""

    def symbol(self, value: str) -> Self:
        """Ticker symbol (e.g. AAPL)"""
        ...


@final
class IndexSnapshotOhlcBuilder(_Builder[OhlcTickList]):
    """Fluent builder for :meth:`MarketDataView.index_snapshot_ohlc`."""

    def symbols(self, value: Union[str, Sequence[str]]) -> Self:
        """Comma-separated ticker symbols (e.g. AAPL,MSFT)"""
        ...

    def min_time(self, value: Union[str, time, datetime]) -> Self:
        """Minimum time filter"""
        ...

    def current_day_only(self, value: bool) -> Self:
        """When true, only snapshot data from the current trading day (New York date) is returned. Applied before `min_time`. Defaults to false, matching the terminal, which sends false when the value is omitted."""
        ...


@final
class IndexSnapshotPriceBuilder(_Builder[PriceTickList]):
    """Fluent builder for :meth:`MarketDataView.index_snapshot_price`."""

    def symbols(self, value: Union[str, Sequence[str]]) -> Self:
        """Comma-separated ticker symbols (e.g. AAPL,MSFT)"""
        ...

    def min_time(self, value: Union[str, time, datetime]) -> Self:
        """Minimum time filter"""
        ...

    def current_day_only(self, value: bool) -> Self:
        """When true, only snapshot data from the current trading day (New York date) is returned. Applied before `min_time`. Defaults to false, matching the terminal, which sends false when the value is omitted."""
        ...


@final
class IndexSnapshotMarketValueBuilder(_Builder[MarketValueTickList]):
    """Fluent builder for :meth:`MarketDataView.index_snapshot_market_value`."""

    def symbols(self, value: Union[str, Sequence[str]]) -> Self:
        """Comma-separated ticker symbols (e.g. AAPL,MSFT)"""
        ...

    def min_time(self, value: Union[str, time, datetime]) -> Self:
        """Minimum time filter"""
        ...

    def current_day_only(self, value: bool) -> Self:
        """When true, only snapshot data from the current trading day (New York date) is returned. Applied before `min_time`. Defaults to false, matching the terminal, which sends false when the value is omitted."""
        ...


@final
class IndexHistoryEodBuilder(_ChunkedBuilder[EodTickList]):
    """Fluent builder for :meth:`MarketDataView.index_history_eod`."""

    def symbol(self, value: str) -> Self:
        """Ticker symbol (e.g. AAPL)"""
        ...

    def start_date(self, value: Union[str, _date, datetime]) -> Self:
        """Start date YYYYMMDD"""
        ...

    def end_date(self, value: Union[str, _date, datetime]) -> Self:
        """End date YYYYMMDD"""
        ...


@final
class IndexHistoryOhlcBuilder(_ChunkedBuilder[OhlcTickList]):
    """Fluent builder for :meth:`MarketDataView.index_history_ohlc`."""

    def symbol(self, value: str) -> Self:
        """Ticker symbol (e.g. AAPL)"""
        ...

    def start_date(self, value: Union[str, _date, datetime]) -> Self:
        """Start date YYYYMMDD"""
        ...

    def end_date(self, value: Union[str, _date, datetime]) -> Self:
        """End date YYYYMMDD"""
        ...

    def interval(self, value: Union[str, Interval]) -> Self:
        """Interval preset. Defaults to `1s` when omitted, matching the upstream ThetaData Python library. Accepted values: `tick`, `10ms`, `100ms`, `500ms`, `1s`, `5s`, `10s`, `15s`, `30s`, `1m`, `5m`, `10m`, `15m`, `30m`, `1h`."""
        ...

    def start_time(self, value: Union[str, time, datetime]) -> Self:
        """Start time filter"""
        ...

    def end_time(self, value: Union[str, time, datetime]) -> Self:
        """End time filter"""
        ...


@final
class IndexHistoryPriceBuilder(_ChunkedBuilder[PriceTickList]):
    """Fluent builder for :meth:`MarketDataView.index_history_price`."""

    def symbol(self, value: str) -> Self:
        """Ticker symbol (e.g. AAPL)"""
        ...

    def date(self, value: Union[str, _date, datetime]) -> Self:
        """Single date YYYYMMDD. Supply this for a single-day pull, or supply `start_date`/`end_date` for a range. When present, `date` takes precedence over the range."""
        ...

    def interval(self, value: Union[str, Interval]) -> Self:
        """Interval preset. Defaults to `1s` when omitted, matching the upstream ThetaData Python library. Accepted values: `tick`, `10ms`, `100ms`, `500ms`, `1s`, `5s`, `10s`, `15s`, `30s`, `1m`, `5m`, `10m`, `15m`, `30m`, `1h`."""
        ...

    def start_time(self, value: Union[str, time, datetime]) -> Self:
        """Start time filter"""
        ...

    def end_time(self, value: Union[str, time, datetime]) -> Self:
        """End time filter"""
        ...

    def start_date(self, value: Union[str, _date, datetime]) -> Self:
        """Start date YYYYMMDD"""
        ...

    def end_date(self, value: Union[str, _date, datetime]) -> Self:
        """End date YYYYMMDD"""
        ...


@final
class IndexAtTimePriceBuilder(_ChunkedBuilder[IndexPriceAtTimeTickList]):
    """Fluent builder for :meth:`MarketDataView.index_at_time_price`."""

    def symbol(self, value: str) -> Self:
        """Ticker symbol (e.g. AAPL)"""
        ...

    def start_date(self, value: Union[str, _date, datetime]) -> Self:
        """Start date YYYYMMDD"""
        ...

    def end_date(self, value: Union[str, _date, datetime]) -> Self:
        """End date YYYYMMDD"""
        ...

    def time_of_day(self, value: Union[str, time, datetime]) -> Self:
        """ET wall-clock time in HH:MM:SS.SSS (e.g. 09:30:00.000 for 9:30 AM ET; legacy 34200000 is also accepted)"""
        ...


@final
class IndexFlatFileEodBuilder(_ChunkedBuilder[EodTickList]):
    """Fluent builder for :meth:`MarketDataView.index_flat_file_eod`."""

    def date(self, value: Union[str, _date, datetime]) -> Self:
        """Date YYYYMMDD"""
        ...


@final
class CalendarOpenTodayBuilder(_Builder[CalendarDayList]):
    """Fluent builder for :meth:`MarketDataView.calendar_open_today`."""


@final
class CalendarOnDateBuilder(_Builder[CalendarDayList]):
    """Fluent builder for :meth:`MarketDataView.calendar_on_date`."""

    def date(self, value: Union[str, _date, datetime]) -> Self:
        """Date YYYYMMDD"""
        ...


@final
class CalendarYearBuilder(_Builder[CalendarDayList]):
    """Fluent builder for :meth:`MarketDataView.calendar_year`."""

    def year(self, value: str) -> Self:
        """4-digit year (e.g. 2024)"""
        ...


@final
class InterestRateHistoryEodBuilder(_ChunkedBuilder[InterestRateTickList]):
    """Fluent builder for :meth:`MarketDataView.interest_rate_history_eod`."""

    def symbol(self, value: str) -> Self:
        """Ticker symbol (e.g. AAPL)"""
        ...

    def start_date(self, value: Union[str, _date, datetime]) -> Self:
        """Start date YYYYMMDD"""
        ...

    def end_date(self, value: Union[str, _date, datetime]) -> Self:
        """End date YYYYMMDD"""
        ...


class _AsyncMarketData:
    """The awaitable market-data methods of :class:`MarketDataView`, which
    :class:`AsyncClient` forwards. It does not exist at runtime.
    """

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

    def stock_list_dates_async(
        self,
        request_type: Union[str, RequestType],
        symbol: str,
        *,
        timeout_ms: Optional[int] = None,
    ) -> Awaitable[StringList]:
        """List available dates for a stock by request type (EOD, TRADE, QUOTE, etc.).

        Lists all dates of data that are available for a stock with a given request type and symbol. This endpoint is updated overnight.


        Awaitable companion of the sync variant. The returned object resolves the request off the calling thread so a running event loop keeps servicing other coroutines.
        """
        ...

    def stock_snapshot_ohlc_async(
        self,
        symbols: Union[str, Sequence[str]],
        *,
        venue: Optional[Union[str, Venue]] = None,
        min_time: Optional[Union[str, time, datetime]] = None,
        current_day_only: Optional[bool] = False,
        timeout_ms: Optional[int] = None,
    ) -> Awaitable[OhlcTickList]:
        """Get the latest OHLC snapshot for one or more stocks.

        Provides a real-time Open, High, Low, Close for the current day.
        * Returns a real-time session OHLC from the Nasdaq Basic feed if the account has a stocks standard or pro subscription.
        * Returns a 15-minute delayed session OHLC from the UTP & CTA feeds if the account has the stocks value subscription.
        * Theta Data resets its snapshot cache at midnight ET every day. This endpoint may not work on a weekend where there were no eligible messages sent over exchange feeds. We recommend using historic requests during the weekend.

        Defaults (upstream):
        - `venue`: `"nqb"`
        - `current_day_only`: `false`


        Awaitable companion of the sync variant. The returned object resolves the request off the calling thread so a running event loop keeps servicing other coroutines.
        """
        ...

    def stock_snapshot_trade_async(
        self,
        symbols: Union[str, Sequence[str]],
        *,
        venue: Optional[Union[str, Venue]] = None,
        min_time: Optional[Union[str, time, datetime]] = None,
        current_day_only: Optional[bool] = False,
        timeout_ms: Optional[int] = None,
    ) -> Awaitable[TradeTickList]:
        """Get the latest trade snapshot for one or more stocks.

        Returns a real-time last trade from the Nasdaq Basic feed if the account has a stocks standard or pro subscription.

        - Theta Data resets its snapshot cache at midnight ET every day. This endpoint may not work on a weekend where there were no eligible messages sent over exchange feeds. We recommend using historic requests during the weekend.

        Defaults (upstream):
        - `venue`: `"nqb"`
        - `current_day_only`: `false`


        Awaitable companion of the sync variant. The returned object resolves the request off the calling thread so a running event loop keeps servicing other coroutines.
        """
        ...

    def stock_snapshot_quote_async(
        self,
        symbols: Union[str, Sequence[str]],
        *,
        venue: Optional[Union[str, Venue]] = None,
        min_time: Optional[Union[str, time, datetime]] = None,
        current_day_only: Optional[bool] = False,
        timeout_ms: Optional[int] = None,
    ) -> Awaitable[QuoteTickList]:
        """Get the latest NBBO quote snapshot for one or more stocks.

        * Returns a real-time last BBO quote from the Nasdaq Basic feed if the account has a stocks standard or pro subscription.
        * Returns a 15-minute delayed NBBO quote from the UTP & CTA feeds account has the stocks value subscription subscription.
        - Theta Data resets its snapshot cache at midnight ET every day. This endpoint may not work on a weekend where there were no eligible messages sent over exchange feeds. We recommend using historic requests during the weekend.

        Defaults (upstream):
        - `venue`: `"nqb"`
        - `current_day_only`: `false`


        Awaitable companion of the sync variant. The returned object resolves the request off the calling thread so a running event loop keeps servicing other coroutines.
        """
        ...

    def stock_snapshot_market_value_async(
        self,
        symbols: Union[str, Sequence[str]],
        *,
        venue: Optional[Union[str, Venue]] = None,
        min_time: Optional[Union[str, time, datetime]] = None,
        current_day_only: Optional[bool] = False,
        timeout_ms: Optional[int] = None,
    ) -> Awaitable[MarketValueTickList]:
        """Get the latest market value snapshot for one or more stocks.

        * Returns a real-time market value derived from the last BBO quote from the Nasdaq Basic feed if the account has a stocks standard or pro subscription.
        * Returns a 15-minute delayed market value derived from an NBBO quote from the UTP & CTA feeds if the account has the stocks value subscription subscription.
        - Theta Data resets its snapshot cache at midnight ET every day. This endpoint may not work on a weekend where there were no eligible messages sent over exchange feeds. We recommend using historic requests during the weekend.

        Defaults (upstream):
        - `venue`: `"nqb"`
        - `current_day_only`: `false`


        Awaitable companion of the sync variant. The returned object resolves the request off the calling thread so a running event loop keeps servicing other coroutines.
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

    def stock_history_ohlc_async(
        self,
        symbol: str,
        *,
        date: Optional[Union[str, date, datetime]] = None,
        interval: Optional[Union[str, Interval]] = None,
        start_time: Optional[Union[str, time, datetime]] = None,
        end_time: Optional[Union[str, time, datetime]] = None,
        venue: Optional[Union[str, Venue]] = None,
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

    def stock_history_trade_async(
        self,
        symbol: str,
        *,
        date: Optional[Union[str, date, datetime]] = None,
        start_time: Optional[Union[str, time, datetime]] = None,
        end_time: Optional[Union[str, time, datetime]] = None,
        venue: Optional[Union[str, Venue]] = None,
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

    def stock_history_quote_async(
        self,
        symbol: str,
        *,
        date: Optional[Union[str, date, datetime]] = None,
        interval: Optional[Union[str, Interval]] = None,
        start_time: Optional[Union[str, time, datetime]] = None,
        end_time: Optional[Union[str, time, datetime]] = None,
        venue: Optional[Union[str, Venue]] = None,
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

    def stock_history_trade_quote_async(
        self,
        symbol: str,
        *,
        date: Optional[Union[str, date, datetime]] = None,
        start_time: Optional[Union[str, time, datetime]] = None,
        end_time: Optional[Union[str, time, datetime]] = None,
        exclusive: Optional[bool] = True,
        venue: Optional[Union[str, Venue]] = None,
        start_date: Optional[Union[str, date, datetime]] = None,
        end_date: Optional[Union[str, date, datetime]] = None,
        use_time_based_trade_quote: Optional[bool] = True,
        timeout_ms: Optional[int] = None,
    ) -> Awaitable[TradeQuoteTickList]:
        """Fetch combined trade + quote ticks for a stock on a given date.

        Returns every trade reported by UTP & CTA paired with the last BBO quote reported by UTP or CTA at the time of trade. A quote is matched with a trade if its timestamp ``<=`` the trade timestamp. If you prefer to match quotes with timestamps that are ``<`` the trade timestamp, specify the ``exclusive`` parameter to ``true``. Set the ``venue`` parameter to ``nqb`` to access current-day real-time historic data from the Nasdaq Basic feed if the account has a stocks standard or pro subscription.
        - Multi-day requests are limited to 1 month of data.

        Defaults (upstream):
        - `start_time`: `"09:30:00"`
        - `end_time`: `"16:00:00"`
        - `exclusive`: `true`
        - `venue`: `"nqb"`
        - `use_time_based_trade_quote`: `true`


        Awaitable companion of the sync variant. The returned object resolves the request off the calling thread so a running event loop keeps servicing other coroutines.
        """
        ...

    def stock_at_time_trade_async(
        self,
        symbol: str,
        start_date: Union[str, date, datetime],
        end_date: Union[str, date, datetime],
        time_of_day: Union[str, time, datetime],
        *,
        venue: Optional[Union[str, Venue]] = None,
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

    def stock_at_time_quote_async(
        self,
        symbol: str,
        start_date: Union[str, date, datetime],
        end_date: Union[str, date, datetime],
        time_of_day: Union[str, time, datetime],
        *,
        venue: Optional[Union[str, Venue]] = None,
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

    def stock_flat_file_eod_async(
        self,
        date: Union[str, date, datetime],
        *,
        timeout_ms: Optional[int] = None,
    ) -> Awaitable[EodTickList]:
        """Fetch the end-of-day report of every stock for one date. The vendor keeps the most recent 7 calendar days.

        Awaitable companion of the sync variant. The returned object resolves the request off the calling thread so a running event loop keeps servicing other coroutines.
        """
        ...

    def stock_flat_file_eod_to_path_async(
        self,
        date: Union[str, date, datetime],
        path: Union[str, PathLike[str]],
        format: Optional[str] = None,
    ) -> Awaitable[str]:
        """Write the `stock_flat_file_eod` file for `date` to `path` as the terminal writes it, and return the path written.

        ``format`` is ``"csv"`` (the default), ``"json"``, ``"json_legacy"``, ``"ndjson"`` (or ``"jsonl"``) or ``"html"``. The file streams to disk chunk by chunk and takes its name only once complete; a path without an extension gets the format's.

        Awaitable companion of the sync variant. The returned object resolves to the path written, so a running event loop keeps servicing other coroutines while the file streams to disk.
        """
        ...

    def stock_flat_file_trade_quote_async(
        self,
        date: Union[str, date, datetime],
        *,
        timeout_ms: Optional[int] = None,
    ) -> Awaitable[TradeQuoteTickList]:
        """Fetch every stock trade of one date, each paired with the NBBO quote at the time of the trade. The vendor keeps the most recent 7 calendar days.

        Awaitable companion of the sync variant. The returned object resolves the request off the calling thread so a running event loop keeps servicing other coroutines.
        """
        ...

    def stock_flat_file_trade_quote_to_path_async(
        self,
        date: Union[str, date, datetime],
        path: Union[str, PathLike[str]],
        format: Optional[str] = None,
    ) -> Awaitable[str]:
        """Write the `stock_flat_file_trade_quote` file for `date` to `path` as the terminal writes it, and return the path written.

        ``format`` is ``"csv"`` (the default), ``"json"``, ``"json_legacy"``, ``"ndjson"`` (or ``"jsonl"``) or ``"html"``. The file streams to disk chunk by chunk and takes its name only once complete; a path without an extension gets the format's.

        Awaitable companion of the sync variant. The returned object resolves to the path written, so a running event loop keeps servicing other coroutines while the file streams to disk.
        """
        ...

    def stock_flat_file_quote_async(
        self,
        date: Union[str, date, datetime],
        *,
        timeout_ms: Optional[int] = None,
    ) -> Awaitable[IntervalQuoteTickList]:
        """Fetch every stock's 1-minute NBBO quote intervals for one date: one row per minute in which a new quote was reported. The vendor keeps the most recent 7 calendar days.

        Awaitable companion of the sync variant. The returned object resolves the request off the calling thread so a running event loop keeps servicing other coroutines.
        """
        ...

    def stock_flat_file_quote_to_path_async(
        self,
        date: Union[str, date, datetime],
        path: Union[str, PathLike[str]],
        format: Optional[str] = None,
    ) -> Awaitable[str]:
        """Write the `stock_flat_file_quote` file for `date` to `path` as the terminal writes it, and return the path written.

        ``format`` is ``"csv"`` (the default), ``"json"``, ``"json_legacy"``, ``"ndjson"`` (or ``"jsonl"``) or ``"html"``. The file streams to disk chunk by chunk and takes its name only once complete; a path without an extension gets the format's.

        Awaitable companion of the sync variant. The returned object resolves to the path written, so a running event loop keeps servicing other coroutines while the file streams to disk.
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

    def option_list_dates_async(
        self,
        request_type: Union[str, RequestType],
        symbol: str,
        expiration: Union[str, date, datetime],
        *,
        strike: Optional[str] = None,
        right: Optional[Union[str, Right]] = None,
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

    def option_list_contracts_async(
        self,
        request_type: Union[str, RequestType],
        date: Union[str, date, datetime],
        *,
        symbol: Optional[str] = None,
        max_dte: Optional[int] = None,
        timeout_ms: Optional[int] = None,
    ) -> Awaitable[OptionContractList]:
        """List all option contracts traded or quoted on a given date, optionally filtered to a symbol.

        Lists all contracts that were traded or quoted on a particular date.

        If the ``symbol`` parameter is specified, the returned contracts will be filtered to match the symbol.
        When ``symbol`` is omitted the full universe of contracts for that date is returned.
        This endpoint is updated real-time.


        Awaitable companion of the sync variant. The returned object resolves the request off the calling thread so a running event loop keeps servicing other coroutines.
        """
        ...

    def option_snapshot_ohlc_async(
        self,
        symbol: str,
        expiration: Union[str, date, datetime],
        *,
        strike: Optional[str] = None,
        right: Optional[Union[str, Right]] = None,
        max_dte: Optional[int] = None,
        strike_range: Optional[int] = None,
        min_time: Optional[Union[str, time, datetime]] = None,
        current_day_only: Optional[bool] = False,
        timeout_ms: Optional[int] = None,
    ) -> Awaitable[List[OhlcTick]]:
        """Get the latest OHLC snapshot for an option contract.

        - Retrieve a real-time last ohlc of an option contract for the trading day.

        Defaults (upstream):
        - `strike`: `"*"`
        - `right`: `"both"`
        - `current_day_only`: `false`


        Awaitable companion of the sync variant. The returned object resolves the request off the calling thread so a running event loop keeps servicing other coroutines.
        """
        ...

    def option_snapshot_trade_async(
        self,
        symbol: str,
        expiration: Union[str, date, datetime],
        *,
        strike: Optional[str] = None,
        right: Optional[Union[str, Right]] = None,
        strike_range: Optional[int] = None,
        min_time: Optional[Union[str, time, datetime]] = None,
        current_day_only: Optional[bool] = False,
        timeout_ms: Optional[int] = None,
    ) -> Awaitable[List[TradeTick]]:
        """Get the latest trade snapshot for an option contract.

        - Retrieve the real-time last trade of an option contract.
        - This endpoint will return no data if the market was closed for the day. Theta Data resets the snapshot cache at midnight ET every night.

        Defaults (upstream):
        - `strike`: `"*"`
        - `right`: `"both"`
        - `current_day_only`: `false`


        Awaitable companion of the sync variant. The returned object resolves the request off the calling thread so a running event loop keeps servicing other coroutines.
        """
        ...

    def option_snapshot_quote_async(
        self,
        symbol: str,
        expiration: Union[str, date, datetime],
        *,
        strike: Optional[str] = None,
        right: Optional[Union[str, Right]] = None,
        max_dte: Optional[int] = None,
        strike_range: Optional[int] = None,
        min_time: Optional[Union[str, time, datetime]] = None,
        current_day_only: Optional[bool] = False,
        timeout_ms: Optional[int] = None,
    ) -> Awaitable[List[QuoteTick]]:
        """Get the latest NBBO quote snapshot for an option contract.

        - Retrieve a real-time last NBBO quote of an option contract.
        - This endpoint will return no data if the market was closed for the day. Theta Data resets the snapshot cache at midnight ET every night.

        Defaults (upstream):
        - `strike`: `"*"`
        - `right`: `"both"`
        - `current_day_only`: `false`


        Awaitable companion of the sync variant. The returned object resolves the request off the calling thread so a running event loop keeps servicing other coroutines.
        """
        ...

    def option_snapshot_open_interest_async(
        self,
        symbol: str,
        expiration: Union[str, date, datetime],
        *,
        strike: Optional[str] = None,
        right: Optional[Union[str, Right]] = None,
        max_dte: Optional[int] = None,
        strike_range: Optional[int] = None,
        min_time: Optional[Union[str, time, datetime]] = None,
        current_day_only: Optional[bool] = False,
        timeout_ms: Optional[int] = None,
    ) -> Awaitable[List[OpenInterestTick]]:
        """Get the latest open interest snapshot for an option contract.

        - Retrieve the last open interest message of an option contract.
        - Open interest is reported around 06:30 ET every morning by OPRA and reflects the open interest at the end of the previous trading day.
        - This endpoint will return no data if the market was closed for the day. Theta Data resets the snapshot cache at midnight ET every night.

        Defaults (upstream):
        - `strike`: `"*"`
        - `right`: `"both"`
        - `current_day_only`: `false`


        Awaitable companion of the sync variant. The returned object resolves the request off the calling thread so a running event loop keeps servicing other coroutines.
        """
        ...

    def option_snapshot_market_value_async(
        self,
        symbol: str,
        expiration: Union[str, date, datetime],
        *,
        strike: Optional[str] = None,
        right: Optional[Union[str, Right]] = None,
        max_dte: Optional[int] = None,
        strike_range: Optional[int] = None,
        min_time: Optional[Union[str, time, datetime]] = None,
        current_day_only: Optional[bool] = False,
        timeout_ms: Optional[int] = None,
    ) -> Awaitable[List[MarketValueTick]]:
        """Get the latest market value snapshot for an option contract.

        * Returns a real-time market value derived from the last NBBO quote of an option contract.

        Defaults (upstream):
        - `strike`: `"*"`
        - `right`: `"both"`
        - `current_day_only`: `false`


        Awaitable companion of the sync variant. The returned object resolves the request off the calling thread so a running event loop keeps servicing other coroutines.
        """
        ...

    def option_snapshot_greeks_implied_volatility_async(
        self,
        symbol: str,
        expiration: Union[str, date, datetime],
        *,
        strike: Optional[str] = None,
        right: Optional[Union[str, Right]] = None,
        annual_dividend: Optional[float] = None,
        annual_dividend_amount: Optional[float] = None,
        rate_type: Optional[Union[str, RateType]] = None,
        rate_value: Optional[float] = None,
        stock_price: Optional[float] = None,
        version: Optional[Union[str, Version]] = None,
        max_dte: Optional[int] = None,
        strike_range: Optional[int] = None,
        min_time: Optional[Union[str, time, datetime]] = None,
        use_market_value: Optional[bool] = False,
        current_day_only: Optional[bool] = False,
        timeout_ms: Optional[int] = None,
    ) -> Awaitable[List[IvTick]]:
        """Get implied volatility snapshot for an option contract (from ThetaData server).

        Returns implied volatilies calculated using the national best bid, mid, and ask price
        of the option respectively. The underlying price represents whatever the last underlying price was at the
        ``underlying_timestamp`` field. You can read more about how Theta Data calculates greeks [here](/articles/option-greeks).

        Defaults (upstream):
        - `strike`: `"*"`
        - `right`: `"both"`
        - `rate_type`: `"sofr"`
        - `version`: `"latest"`
        - `use_market_value`: `false`
        - `current_day_only`: `false`


        Awaitable companion of the sync variant. The returned object resolves the request off the calling thread so a running event loop keeps servicing other coroutines.
        """
        ...

    def option_snapshot_greeks_all_async(
        self,
        symbol: str,
        expiration: Union[str, date, datetime],
        *,
        strike: Optional[str] = None,
        right: Optional[Union[str, Right]] = None,
        annual_dividend: Optional[float] = None,
        annual_dividend_amount: Optional[float] = None,
        rate_type: Optional[Union[str, RateType]] = None,
        rate_value: Optional[float] = None,
        stock_price: Optional[float] = None,
        version: Optional[Union[str, Version]] = None,
        max_dte: Optional[int] = None,
        strike_range: Optional[int] = None,
        min_time: Optional[Union[str, time, datetime]] = None,
        use_market_value: Optional[bool] = False,
        current_day_only: Optional[bool] = False,
        timeout_ms: Optional[int] = None,
    ) -> Awaitable[List[GreeksAllTick]]:
        """Get all Greeks snapshot for an option contract (from ThetaData server).

        - Retrieve a real-time last greeks calculation for all option contracts that lie on a provided expiration.
        > This endpoint will return no data if the market was closed for the day. Theta Data resets the snapshot cache at midnight ET every night.

        Defaults (upstream):
        - `strike`: `"*"`
        - `right`: `"both"`
        - `rate_type`: `"sofr"`
        - `version`: `"latest"`
        - `use_market_value`: `false`
        - `current_day_only`: `false`


        Awaitable companion of the sync variant. The returned object resolves the request off the calling thread so a running event loop keeps servicing other coroutines.
        """
        ...

    def option_snapshot_binomial_greeks_all_async(
        self,
        symbol: str,
        expiration: Union[str, date, datetime],
        *,
        strike: Optional[str] = None,
        right: Optional[Union[str, Right]] = None,
        annual_dividend: Optional[float] = None,
        annual_dividend_amount: Optional[float] = None,
        rate_type: Optional[Union[str, RateType]] = None,
        rate_value: Optional[float] = None,
        stock_price: Optional[float] = None,
        binomial_steps: Optional[int] = 101,
        version: Optional[Union[str, Version]] = None,
        max_dte: Optional[int] = None,
        strike_range: Optional[int] = None,
        min_time: Optional[Union[str, time, datetime]] = None,
        use_market_value: Optional[bool] = False,
        current_day_only: Optional[bool] = False,
        timeout_ms: Optional[int] = None,
    ) -> Awaitable[List[GreeksAllTick]]:
        """Get all binomial Greeks snapshot for an option contract (from ThetaData server).

        - Retrieve a real-time last greeks calculation for all option contracts that lie on a provided expiration.
        - Greeks are calculated with a Leisen-Reimer binomial tree that allows early exercise at every node, so the values are American-style.
        > This endpoint will return no data if the market was closed for the day. Theta Data resets the snapshot cache at midnight ET every night.

        Defaults (upstream):
        - `strike`: `"*"`
        - `right`: `"both"`
        - `rate_type`: `"sofr"`
        - `binomial_steps`: `101`
        - `version`: `"latest"`
        - `use_market_value`: `false`
        - `current_day_only`: `false`


        Awaitable companion of the sync variant. The returned object resolves the request off the calling thread so a running event loop keeps servicing other coroutines.
        """
        ...

    def option_snapshot_greeks_first_order_async(
        self,
        symbol: str,
        expiration: Union[str, date, datetime],
        *,
        strike: Optional[str] = None,
        right: Optional[Union[str, Right]] = None,
        annual_dividend: Optional[float] = None,
        annual_dividend_amount: Optional[float] = None,
        rate_type: Optional[Union[str, RateType]] = None,
        rate_value: Optional[float] = None,
        stock_price: Optional[float] = None,
        version: Optional[Union[str, Version]] = None,
        max_dte: Optional[int] = None,
        strike_range: Optional[int] = None,
        min_time: Optional[Union[str, time, datetime]] = None,
        use_market_value: Optional[bool] = False,
        current_day_only: Optional[bool] = False,
        timeout_ms: Optional[int] = None,
    ) -> Awaitable[List[GreeksFirstOrderTick]]:
        """Get first-order Greeks snapshot (delta, theta, rho) for an option contract.

        - Retrieve a real-time last greeks calculation for all option contracts that lie on a provided expiration.
        > This endpoint will return no data if the market was closed for the day. Theta Data resets the snapshot cache at midnight ET every night.

        Defaults (upstream):
        - `strike`: `"*"`
        - `right`: `"both"`
        - `rate_type`: `"sofr"`
        - `version`: `"latest"`
        - `use_market_value`: `false`
        - `current_day_only`: `false`


        Awaitable companion of the sync variant. The returned object resolves the request off the calling thread so a running event loop keeps servicing other coroutines.
        """
        ...

    def option_snapshot_binomial_greeks_first_order_async(
        self,
        symbol: str,
        expiration: Union[str, date, datetime],
        *,
        strike: Optional[str] = None,
        right: Optional[Union[str, Right]] = None,
        annual_dividend: Optional[float] = None,
        annual_dividend_amount: Optional[float] = None,
        rate_type: Optional[Union[str, RateType]] = None,
        rate_value: Optional[float] = None,
        stock_price: Optional[float] = None,
        binomial_steps: Optional[int] = 101,
        version: Optional[Union[str, Version]] = None,
        max_dte: Optional[int] = None,
        strike_range: Optional[int] = None,
        min_time: Optional[Union[str, time, datetime]] = None,
        use_market_value: Optional[bool] = False,
        current_day_only: Optional[bool] = False,
        timeout_ms: Optional[int] = None,
    ) -> Awaitable[List[GreeksFirstOrderTick]]:
        """Get first-order binomial Greeks snapshot (delta, theta, rho) for an option contract.

        - Retrieve a real-time last greeks calculation for all option contracts that lie on a provided expiration.
        - Greeks are calculated with a Leisen-Reimer binomial tree that allows early exercise at every node, so the values are American-style.
        > This endpoint will return no data if the market was closed for the day. Theta Data resets the snapshot cache at midnight ET every night.

        Defaults (upstream):
        - `strike`: `"*"`
        - `right`: `"both"`
        - `rate_type`: `"sofr"`
        - `binomial_steps`: `101`
        - `version`: `"latest"`
        - `use_market_value`: `false`
        - `current_day_only`: `false`


        Awaitable companion of the sync variant. The returned object resolves the request off the calling thread so a running event loop keeps servicing other coroutines.
        """
        ...

    def option_snapshot_greeks_second_order_async(
        self,
        symbol: str,
        expiration: Union[str, date, datetime],
        *,
        strike: Optional[str] = None,
        right: Optional[Union[str, Right]] = None,
        annual_dividend: Optional[float] = None,
        annual_dividend_amount: Optional[float] = None,
        rate_type: Optional[Union[str, RateType]] = None,
        rate_value: Optional[float] = None,
        stock_price: Optional[float] = None,
        version: Optional[Union[str, Version]] = None,
        max_dte: Optional[int] = None,
        strike_range: Optional[int] = None,
        min_time: Optional[Union[str, time, datetime]] = None,
        use_market_value: Optional[bool] = False,
        current_day_only: Optional[bool] = False,
        timeout_ms: Optional[int] = None,
    ) -> Awaitable[List[GreeksSecondOrderTick]]:
        """Get second-order Greeks snapshot (gamma, vanna, charm) for an option contract.

        - Retrieve a real-time last second order greeks calculation for all option contracts that lie on a provided expiration.
        > This endpoint will return no data if the market was closed for the day. Theta Data resets the snapshot cache at midnight ET every night.

        Defaults (upstream):
        - `strike`: `"*"`
        - `right`: `"both"`
        - `rate_type`: `"sofr"`
        - `version`: `"latest"`
        - `use_market_value`: `false`
        - `current_day_only`: `false`


        Awaitable companion of the sync variant. The returned object resolves the request off the calling thread so a running event loop keeps servicing other coroutines.
        """
        ...

    def option_snapshot_binomial_greeks_second_order_async(
        self,
        symbol: str,
        expiration: Union[str, date, datetime],
        *,
        strike: Optional[str] = None,
        right: Optional[Union[str, Right]] = None,
        annual_dividend: Optional[float] = None,
        annual_dividend_amount: Optional[float] = None,
        rate_type: Optional[Union[str, RateType]] = None,
        rate_value: Optional[float] = None,
        stock_price: Optional[float] = None,
        binomial_steps: Optional[int] = 101,
        version: Optional[Union[str, Version]] = None,
        max_dte: Optional[int] = None,
        strike_range: Optional[int] = None,
        min_time: Optional[Union[str, time, datetime]] = None,
        use_market_value: Optional[bool] = False,
        current_day_only: Optional[bool] = False,
        timeout_ms: Optional[int] = None,
    ) -> Awaitable[List[GreeksSecondOrderTick]]:
        """Get second-order binomial Greeks snapshot (gamma, vanna, charm) for an option contract.

        - Retrieve a real-time last second order binomial greeks calculation for all option contracts that lie on a provided expiration.
        - Greeks are calculated with a Leisen-Reimer binomial tree that allows early exercise at every node, so the values are American-style.
        > This endpoint will return no data if the market was closed for the day. Theta Data resets the snapshot cache at midnight ET every night.

        Defaults (upstream):
        - `strike`: `"*"`
        - `right`: `"both"`
        - `rate_type`: `"sofr"`
        - `binomial_steps`: `101`
        - `version`: `"latest"`
        - `use_market_value`: `false`
        - `current_day_only`: `false`


        Awaitable companion of the sync variant. The returned object resolves the request off the calling thread so a running event loop keeps servicing other coroutines.
        """
        ...

    def option_snapshot_greeks_third_order_async(
        self,
        symbol: str,
        expiration: Union[str, date, datetime],
        *,
        strike: Optional[str] = None,
        right: Optional[Union[str, Right]] = None,
        annual_dividend: Optional[float] = None,
        annual_dividend_amount: Optional[float] = None,
        rate_type: Optional[Union[str, RateType]] = None,
        rate_value: Optional[float] = None,
        stock_price: Optional[float] = None,
        version: Optional[Union[str, Version]] = None,
        max_dte: Optional[int] = None,
        strike_range: Optional[int] = None,
        min_time: Optional[Union[str, time, datetime]] = None,
        use_market_value: Optional[bool] = False,
        current_day_only: Optional[bool] = False,
        timeout_ms: Optional[int] = None,
    ) -> Awaitable[List[GreeksThirdOrderTick]]:
        """Get third-order Greeks snapshot (speed, color, ultima) for an option contract.

        - Retrieve a real-time last third order greeks calculation for all option contracts that lie on a provided expiration.
        > This endpoint will return no data if the market was closed for the day. Theta Data resets the snapshot cache at midnight ET every night.

        Defaults (upstream):
        - `strike`: `"*"`
        - `right`: `"both"`
        - `rate_type`: `"sofr"`
        - `version`: `"latest"`
        - `use_market_value`: `false`
        - `current_day_only`: `false`


        Awaitable companion of the sync variant. The returned object resolves the request off the calling thread so a running event loop keeps servicing other coroutines.
        """
        ...

    def option_snapshot_binomial_greeks_third_order_async(
        self,
        symbol: str,
        expiration: Union[str, date, datetime],
        *,
        strike: Optional[str] = None,
        right: Optional[Union[str, Right]] = None,
        annual_dividend: Optional[float] = None,
        annual_dividend_amount: Optional[float] = None,
        rate_type: Optional[Union[str, RateType]] = None,
        rate_value: Optional[float] = None,
        stock_price: Optional[float] = None,
        binomial_steps: Optional[int] = 101,
        version: Optional[Union[str, Version]] = None,
        max_dte: Optional[int] = None,
        strike_range: Optional[int] = None,
        min_time: Optional[Union[str, time, datetime]] = None,
        use_market_value: Optional[bool] = False,
        current_day_only: Optional[bool] = False,
        timeout_ms: Optional[int] = None,
    ) -> Awaitable[List[GreeksThirdOrderTick]]:
        """Get third-order binomial Greeks snapshot (speed, color, ultima) for an option contract.

        - Retrieve a real-time last third order binomial greeks calculation for all option contracts that lie on a provided expiration.
        - Greeks are calculated with a Leisen-Reimer binomial tree that allows early exercise at every node, so the values are American-style.
        > This endpoint will return no data if the market was closed for the day. Theta Data resets the snapshot cache at midnight ET every night.

        Defaults (upstream):
        - `strike`: `"*"`
        - `right`: `"both"`
        - `rate_type`: `"sofr"`
        - `binomial_steps`: `101`
        - `version`: `"latest"`
        - `use_market_value`: `false`
        - `current_day_only`: `false`


        Awaitable companion of the sync variant. The returned object resolves the request off the calling thread so a running event loop keeps servicing other coroutines.
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
        right: Optional[Union[str, Right]] = None,
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

    def option_history_ohlc_async(
        self,
        symbol: str,
        expiration: Union[str, date, datetime],
        *,
        strike: Optional[str] = None,
        right: Optional[Union[str, Right]] = None,
        date: Optional[Union[str, date, datetime]] = None,
        interval: Optional[Union[str, Interval]] = None,
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

    def option_history_trade_async(
        self,
        symbol: str,
        expiration: Union[str, date, datetime],
        *,
        strike: Optional[str] = None,
        right: Optional[Union[str, Right]] = None,
        date: Optional[Union[str, date, datetime]] = None,
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

    def option_history_quote_async(
        self,
        symbol: str,
        expiration: Union[str, date, datetime],
        *,
        strike: Optional[str] = None,
        right: Optional[Union[str, Right]] = None,
        date: Optional[Union[str, date, datetime]] = None,
        interval: Optional[Union[str, Interval]] = None,
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

    def option_history_trade_quote_async(
        self,
        symbol: str,
        expiration: Union[str, date, datetime],
        *,
        strike: Optional[str] = None,
        right: Optional[Union[str, Right]] = None,
        date: Optional[Union[str, date, datetime]] = None,
        start_time: Optional[Union[str, time, datetime]] = None,
        end_time: Optional[Union[str, time, datetime]] = None,
        exclusive: Optional[bool] = True,
        max_dte: Optional[int] = None,
        strike_range: Optional[int] = None,
        start_date: Optional[Union[str, date, datetime]] = None,
        end_date: Optional[Union[str, date, datetime]] = None,
        use_time_based_trade_quote: Optional[bool] = True,
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
        - `exclusive`: `true`
        - `use_time_based_trade_quote`: `true`


        Awaitable companion of the sync variant. The returned object resolves the request off the calling thread so a running event loop keeps servicing other coroutines.
        """
        ...

    def option_history_open_interest_async(
        self,
        symbol: str,
        expiration: Union[str, date, datetime],
        *,
        strike: Optional[str] = None,
        right: Optional[Union[str, Right]] = None,
        date: Optional[Union[str, date, datetime]] = None,
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

    def option_history_greeks_eod_async(
        self,
        symbol: str,
        expiration: Union[str, date, datetime],
        start_date: Union[str, date, datetime],
        end_date: Union[str, date, datetime],
        *,
        strike: Optional[str] = None,
        right: Optional[Union[str, Right]] = None,
        annual_dividend: Optional[float] = None,
        annual_dividend_amount: Optional[float] = None,
        rate_type: Optional[Union[str, RateType]] = None,
        rate_value: Optional[float] = None,
        version: Optional[Union[str, Version]] = None,
        underlyer_use_nbbo: Optional[bool] = False,
        max_dte: Optional[int] = None,
        strike_range: Optional[int] = None,
        option_use_nbbo: Optional[bool] = True,
        timeout_ms: Optional[int] = None,
    ) -> Awaitable[GreeksEodTickList]:
        """Fetch end-of-day Greeks history for an option contract.

        - Returns the data for all contracts that share the same provided symbol and expiration.
        - Uses Theta Data's EOD reports that get generated at 17:15 ET each day. The closing option price and closing underlying price are used for the greeks calculation.
        - **Any ``expiration=*`` request must be made day by day.**

        Defaults (upstream):
        - `strike`: `"*"`
        - `right`: `"both"`
        - `rate_type`: `"sofr"`
        - `version`: `"latest"`
        - `underlyer_use_nbbo`: `false`
        - `option_use_nbbo`: `true`


        Awaitable companion of the sync variant. The returned object resolves the request off the calling thread so a running event loop keeps servicing other coroutines.
        """
        ...

    def option_history_binomial_greeks_eod_async(
        self,
        symbol: str,
        expiration: Union[str, date, datetime],
        start_date: Union[str, date, datetime],
        end_date: Union[str, date, datetime],
        *,
        strike: Optional[str] = None,
        right: Optional[Union[str, Right]] = None,
        annual_dividend: Optional[float] = None,
        annual_dividend_amount: Optional[float] = None,
        rate_type: Optional[Union[str, RateType]] = None,
        rate_value: Optional[float] = None,
        binomial_steps: Optional[int] = 101,
        version: Optional[Union[str, Version]] = None,
        underlyer_use_nbbo: Optional[bool] = False,
        max_dte: Optional[int] = None,
        strike_range: Optional[int] = None,
        option_use_nbbo: Optional[bool] = True,
        timeout_ms: Optional[int] = None,
    ) -> Awaitable[GreeksEodTickList]:
        """Fetch end-of-day binomial Greeks history for an option contract.

        - Returns the data for all contracts that share the same provided symbol and expiration.
        - Greeks are calculated with a Leisen-Reimer binomial tree that allows early exercise at every node, so the values are American-style.
        - Uses Theta Data's EOD reports that get generated at 17:15 ET each day. The closing option price and closing underlying price are used for the greeks calculation.
        - **Any ``expiration=*`` request must be made day by day.**

        Defaults (upstream):
        - `strike`: `"*"`
        - `right`: `"both"`
        - `rate_type`: `"sofr"`
        - `binomial_steps`: `101`
        - `version`: `"latest"`
        - `underlyer_use_nbbo`: `false`
        - `option_use_nbbo`: `true`


        Awaitable companion of the sync variant. The returned object resolves the request off the calling thread so a running event loop keeps servicing other coroutines.
        """
        ...

    def option_history_greeks_all_async(
        self,
        symbol: str,
        expiration: Union[str, date, datetime],
        *,
        strike: Optional[str] = None,
        right: Optional[Union[str, Right]] = None,
        date: Optional[Union[str, date, datetime]] = None,
        interval: Optional[Union[str, Interval]] = None,
        start_time: Optional[Union[str, time, datetime]] = None,
        end_time: Optional[Union[str, time, datetime]] = None,
        annual_dividend: Optional[float] = None,
        annual_dividend_amount: Optional[float] = None,
        rate_type: Optional[Union[str, RateType]] = None,
        rate_value: Optional[float] = None,
        version: Optional[Union[str, Version]] = None,
        strike_range: Optional[int] = None,
        start_date: Optional[Union[str, date, datetime]] = None,
        end_date: Optional[Union[str, date, datetime]] = None,
        timeout_ms: Optional[int] = None,
    ) -> Awaitable[GreeksAllTickList]:
        """Fetch all Greeks history for an option contract (intraday, sampled by interval).

        - Returns the data for all contracts that share the same provided symbol and expiration. 
        - Calculated using the option and underlying midpoint price. If an interval size is specified (*highly recommended*), the option quote used in the calculation follows the same rules as the quote endpoint. 
        - The underlying price represents whatever the last underlying price was at the ``timestamp`` field. You can read more about how Theta Data calculates greeks [here](/articles/option-greeks).
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

    def option_history_binomial_greeks_all_async(
        self,
        symbol: str,
        expiration: Union[str, date, datetime],
        *,
        strike: Optional[str] = None,
        right: Optional[Union[str, Right]] = None,
        date: Optional[Union[str, date, datetime]] = None,
        interval: Optional[Union[str, Interval]] = None,
        start_time: Optional[Union[str, time, datetime]] = None,
        end_time: Optional[Union[str, time, datetime]] = None,
        annual_dividend: Optional[float] = None,
        annual_dividend_amount: Optional[float] = None,
        rate_type: Optional[Union[str, RateType]] = None,
        rate_value: Optional[float] = None,
        binomial_steps: Optional[int] = 101,
        version: Optional[Union[str, Version]] = None,
        strike_range: Optional[int] = None,
        start_date: Optional[Union[str, date, datetime]] = None,
        end_date: Optional[Union[str, date, datetime]] = None,
        timeout_ms: Optional[int] = None,
    ) -> Awaitable[GreeksAllTickList]:
        """Fetch all binomial Greeks history for an option contract (intraday, sampled by interval).

        - Returns the data for all contracts that share the same provided symbol and expiration. 
        - Greeks are calculated with a Leisen-Reimer binomial tree that allows early exercise at every node, so the values are American-style.
        - Calculated using the option and underlying midpoint price. If an interval size is specified (*highly recommended*), the option quote used in the calculation follows the same rules as the quote endpoint. 
        - The underlying price represents whatever the last underlying price was at the ``timestamp`` field. You can read more about how Theta Data calculates greeks [here](/articles/option-greeks).
        - Multi-day requests are limited to 1 month of data.

        Defaults (upstream):
        - `strike`: `"*"`
        - `right`: `"both"`
        - `interval`: `"1s"`
        - `start_time`: `"09:30:00"`
        - `end_time`: `"16:00:00"`
        - `rate_type`: `"sofr"`
        - `binomial_steps`: `101`
        - `version`: `"latest"`


        Awaitable companion of the sync variant. The returned object resolves the request off the calling thread so a running event loop keeps servicing other coroutines.
        """
        ...

    def option_history_trade_greeks_all_async(
        self,
        symbol: str,
        expiration: Union[str, date, datetime],
        *,
        strike: Optional[str] = None,
        right: Optional[Union[str, Right]] = None,
        date: Optional[Union[str, date, datetime]] = None,
        start_time: Optional[Union[str, time, datetime]] = None,
        end_time: Optional[Union[str, time, datetime]] = None,
        annual_dividend: Optional[float] = None,
        annual_dividend_amount: Optional[float] = None,
        rate_type: Optional[Union[str, RateType]] = None,
        rate_value: Optional[float] = None,
        version: Optional[Union[str, Version]] = None,
        max_dte: Optional[int] = None,
        strike_range: Optional[int] = None,
        start_date: Optional[Union[str, date, datetime]] = None,
        end_date: Optional[Union[str, date, datetime]] = None,
        perf_boost_intraday: Optional[bool] = False,
        timeout_ms: Optional[int] = None,
    ) -> Awaitable[TradeGreeksAllTickList]:
        """Fetch all Greeks on each trade for an option contract.

        - Returns the data for all contracts that share the same provided symbol and expiration. 
        - Calculates greeks for every trade reported by OPRA.
        - The underlying price represents whatever the last underlying price was at the ``timestamp`` field. You can read more about how Theta Data calculates greeks [here](/articles/option-greeks).
        - Multi-day requests are limited to 1 month of data, and must specify an expiration.

        Defaults (upstream):
        - `strike`: `"*"`
        - `right`: `"both"`
        - `start_time`: `"09:30:00"`
        - `end_time`: `"16:00:00"`
        - `rate_type`: `"sofr"`
        - `version`: `"latest"`
        - `perf_boost_intraday`: `false`


        Awaitable companion of the sync variant. The returned object resolves the request off the calling thread so a running event loop keeps servicing other coroutines.
        """
        ...

    def option_history_binomial_trade_greeks_all_async(
        self,
        symbol: str,
        expiration: Union[str, date, datetime],
        *,
        strike: Optional[str] = None,
        right: Optional[Union[str, Right]] = None,
        date: Optional[Union[str, date, datetime]] = None,
        start_time: Optional[Union[str, time, datetime]] = None,
        end_time: Optional[Union[str, time, datetime]] = None,
        annual_dividend: Optional[float] = None,
        annual_dividend_amount: Optional[float] = None,
        rate_type: Optional[Union[str, RateType]] = None,
        rate_value: Optional[float] = None,
        binomial_steps: Optional[int] = 101,
        version: Optional[Union[str, Version]] = None,
        max_dte: Optional[int] = None,
        strike_range: Optional[int] = None,
        start_date: Optional[Union[str, date, datetime]] = None,
        end_date: Optional[Union[str, date, datetime]] = None,
        perf_boost_intraday: Optional[bool] = False,
        timeout_ms: Optional[int] = None,
    ) -> Awaitable[TradeGreeksAllTickList]:
        """Fetch all binomial Greeks on each trade for an option contract.

        - Returns the data for all contracts that share the same provided symbol and expiration.
        - Greeks are calculated with a Leisen-Reimer binomial tree that allows early exercise at every node, so the values are American-style.
        - Calculates greeks for every trade reported by OPRA.
        - The underlying price represents whatever the last underlying price was at the ``timestamp`` field. You can read more about how Theta Data calculates greeks [here](/articles/option-greeks).
        - Multi-day requests are limited to 1 month of data, and must specify an expiration.

        Defaults (upstream):
        - `strike`: `"*"`
        - `right`: `"both"`
        - `start_time`: `"09:30:00"`
        - `end_time`: `"16:00:00"`
        - `rate_type`: `"sofr"`
        - `binomial_steps`: `101`
        - `version`: `"latest"`
        - `perf_boost_intraday`: `false`


        Awaitable companion of the sync variant. The returned object resolves the request off the calling thread so a running event loop keeps servicing other coroutines.
        """
        ...

    def option_history_greeks_first_order_async(
        self,
        symbol: str,
        expiration: Union[str, date, datetime],
        *,
        strike: Optional[str] = None,
        right: Optional[Union[str, Right]] = None,
        date: Optional[Union[str, date, datetime]] = None,
        interval: Optional[Union[str, Interval]] = None,
        start_time: Optional[Union[str, time, datetime]] = None,
        end_time: Optional[Union[str, time, datetime]] = None,
        annual_dividend: Optional[float] = None,
        annual_dividend_amount: Optional[float] = None,
        rate_type: Optional[Union[str, RateType]] = None,
        rate_value: Optional[float] = None,
        version: Optional[Union[str, Version]] = None,
        strike_range: Optional[int] = None,
        start_date: Optional[Union[str, date, datetime]] = None,
        end_date: Optional[Union[str, date, datetime]] = None,
        timeout_ms: Optional[int] = None,
    ) -> Awaitable[GreeksFirstOrderTickList]:
        """Fetch first-order Greeks history (intraday, sampled by interval).

        - Returns the data for all contracts that share the same provided symbol and expiration. 
        - Calculated using the option and underlying midpoint price. If an interval size is specified (*highly recommended*), the option quote used in the calculation follows the same rules as the quote endpoint. 
        - The underlying price represents whatever the last underlying price was at the ``timestamp`` field. You can read more about how Theta Data calculates greeks [here](/articles/option-greeks).
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

    def option_history_binomial_greeks_first_order_async(
        self,
        symbol: str,
        expiration: Union[str, date, datetime],
        *,
        strike: Optional[str] = None,
        right: Optional[Union[str, Right]] = None,
        date: Optional[Union[str, date, datetime]] = None,
        interval: Optional[Union[str, Interval]] = None,
        start_time: Optional[Union[str, time, datetime]] = None,
        end_time: Optional[Union[str, time, datetime]] = None,
        annual_dividend: Optional[float] = None,
        annual_dividend_amount: Optional[float] = None,
        rate_type: Optional[Union[str, RateType]] = None,
        rate_value: Optional[float] = None,
        binomial_steps: Optional[int] = 101,
        version: Optional[Union[str, Version]] = None,
        strike_range: Optional[int] = None,
        start_date: Optional[Union[str, date, datetime]] = None,
        end_date: Optional[Union[str, date, datetime]] = None,
        timeout_ms: Optional[int] = None,
    ) -> Awaitable[GreeksFirstOrderTickList]:
        """Fetch first-order binomial Greeks history (intraday, sampled by interval).

        - Returns the data for all contracts that share the same provided symbol and expiration. 
        - Greeks are calculated with a Leisen-Reimer binomial tree that allows early exercise at every node, so the values are American-style.
        - Calculated using the option and underlying midpoint price. If an interval size is specified (*highly recommended*), the option quote used in the calculation follows the same rules as the quote endpoint. 
        - The underlying price represents whatever the last underlying price was at the ``timestamp`` field. You can read more about how Theta Data calculates greeks [here](/articles/option-greeks).
        - Multi-day requests are limited to 1 month of data.

        Defaults (upstream):
        - `strike`: `"*"`
        - `right`: `"both"`
        - `interval`: `"1s"`
        - `start_time`: `"09:30:00"`
        - `end_time`: `"16:00:00"`
        - `rate_type`: `"sofr"`
        - `binomial_steps`: `101`
        - `version`: `"latest"`


        Awaitable companion of the sync variant. The returned object resolves the request off the calling thread so a running event loop keeps servicing other coroutines.
        """
        ...

    def option_history_trade_greeks_first_order_async(
        self,
        symbol: str,
        expiration: Union[str, date, datetime],
        *,
        strike: Optional[str] = None,
        right: Optional[Union[str, Right]] = None,
        date: Optional[Union[str, date, datetime]] = None,
        start_time: Optional[Union[str, time, datetime]] = None,
        end_time: Optional[Union[str, time, datetime]] = None,
        annual_dividend: Optional[float] = None,
        annual_dividend_amount: Optional[float] = None,
        rate_type: Optional[Union[str, RateType]] = None,
        rate_value: Optional[float] = None,
        version: Optional[Union[str, Version]] = None,
        max_dte: Optional[int] = None,
        strike_range: Optional[int] = None,
        start_date: Optional[Union[str, date, datetime]] = None,
        end_date: Optional[Union[str, date, datetime]] = None,
        perf_boost_intraday: Optional[bool] = False,
        timeout_ms: Optional[int] = None,
    ) -> Awaitable[TradeGreeksFirstOrderTickList]:
        """Fetch first-order Greeks on each trade for an option contract.

        - Returns the data for all contracts that share the same provided symbol and expiration.
        - Calculates greeks for every trade reported by OPRA.
        - The underlying price represents whatever the last underlying price was at the ``timestamp`` field. You can read more about how Theta Data calculates greeks [here](/articles/option-greeks).
        - Multi-day requests are limited to 1 month of data, and must specify an expiration.

        Defaults (upstream):
        - `strike`: `"*"`
        - `right`: `"both"`
        - `start_time`: `"09:30:00"`
        - `end_time`: `"16:00:00"`
        - `rate_type`: `"sofr"`
        - `version`: `"latest"`
        - `perf_boost_intraday`: `false`


        Awaitable companion of the sync variant. The returned object resolves the request off the calling thread so a running event loop keeps servicing other coroutines.
        """
        ...

    def option_history_binomial_trade_greeks_first_order_async(
        self,
        symbol: str,
        expiration: Union[str, date, datetime],
        *,
        strike: Optional[str] = None,
        right: Optional[Union[str, Right]] = None,
        date: Optional[Union[str, date, datetime]] = None,
        start_time: Optional[Union[str, time, datetime]] = None,
        end_time: Optional[Union[str, time, datetime]] = None,
        annual_dividend: Optional[float] = None,
        annual_dividend_amount: Optional[float] = None,
        rate_type: Optional[Union[str, RateType]] = None,
        rate_value: Optional[float] = None,
        binomial_steps: Optional[int] = 101,
        version: Optional[Union[str, Version]] = None,
        max_dte: Optional[int] = None,
        strike_range: Optional[int] = None,
        start_date: Optional[Union[str, date, datetime]] = None,
        end_date: Optional[Union[str, date, datetime]] = None,
        perf_boost_intraday: Optional[bool] = False,
        timeout_ms: Optional[int] = None,
    ) -> Awaitable[TradeGreeksFirstOrderTickList]:
        """Fetch first-order binomial Greeks on each trade for an option contract.

        - Returns the data for all contracts that share the same provided symbol and expiration.
        - Greeks are calculated with a Leisen-Reimer binomial tree that allows early exercise at every node, so the values are American-style.
        - Calculates greeks for every trade reported by OPRA.
        - The underlying price represents whatever the last underlying price was at the ``timestamp`` field. You can read more about how Theta Data calculates greeks [here](/articles/option-greeks).
        - Multi-day requests are limited to 1 month of data, and must specify an expiration.

        Defaults (upstream):
        - `strike`: `"*"`
        - `right`: `"both"`
        - `start_time`: `"09:30:00"`
        - `end_time`: `"16:00:00"`
        - `rate_type`: `"sofr"`
        - `binomial_steps`: `101`
        - `version`: `"latest"`
        - `perf_boost_intraday`: `false`


        Awaitable companion of the sync variant. The returned object resolves the request off the calling thread so a running event loop keeps servicing other coroutines.
        """
        ...

    def option_history_greeks_second_order_async(
        self,
        symbol: str,
        expiration: Union[str, date, datetime],
        *,
        strike: Optional[str] = None,
        right: Optional[Union[str, Right]] = None,
        date: Optional[Union[str, date, datetime]] = None,
        interval: Optional[Union[str, Interval]] = None,
        start_time: Optional[Union[str, time, datetime]] = None,
        end_time: Optional[Union[str, time, datetime]] = None,
        annual_dividend: Optional[float] = None,
        annual_dividend_amount: Optional[float] = None,
        rate_type: Optional[Union[str, RateType]] = None,
        rate_value: Optional[float] = None,
        version: Optional[Union[str, Version]] = None,
        strike_range: Optional[int] = None,
        start_date: Optional[Union[str, date, datetime]] = None,
        end_date: Optional[Union[str, date, datetime]] = None,
        timeout_ms: Optional[int] = None,
    ) -> Awaitable[GreeksSecondOrderTickList]:
        """Fetch second-order Greeks history (intraday, sampled by interval).

        - Returns the data for all contracts that share the same provided symbol and expiration. 
        - Calculated using the option and underlying midpoint price. If an interval size is specified (*highly recommended*), the option quote used in the calculation follows the same rules as the quote endpoint. 
        - The underlying price represents whatever the last underlying price was at the ``timestamp`` field. You can read more about how Theta Data calculates greeks [here](/articles/option-greeks).
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

    def option_history_binomial_greeks_second_order_async(
        self,
        symbol: str,
        expiration: Union[str, date, datetime],
        *,
        strike: Optional[str] = None,
        right: Optional[Union[str, Right]] = None,
        date: Optional[Union[str, date, datetime]] = None,
        interval: Optional[Union[str, Interval]] = None,
        start_time: Optional[Union[str, time, datetime]] = None,
        end_time: Optional[Union[str, time, datetime]] = None,
        annual_dividend: Optional[float] = None,
        annual_dividend_amount: Optional[float] = None,
        rate_type: Optional[Union[str, RateType]] = None,
        rate_value: Optional[float] = None,
        binomial_steps: Optional[int] = 101,
        version: Optional[Union[str, Version]] = None,
        strike_range: Optional[int] = None,
        start_date: Optional[Union[str, date, datetime]] = None,
        end_date: Optional[Union[str, date, datetime]] = None,
        timeout_ms: Optional[int] = None,
    ) -> Awaitable[GreeksSecondOrderTickList]:
        """Fetch second-order binomial Greeks history (intraday, sampled by interval).

        - Returns the data for all contracts that share the same provided symbol and expiration. 
        - Greeks are calculated with a Leisen-Reimer binomial tree that allows early exercise at every node, so the values are American-style.
        - Calculated using the option and underlying midpoint price. If an interval size is specified (*highly recommended*), the option quote used in the calculation follows the same rules as the quote endpoint. 
        - The underlying price represents whatever the last underlying price was at the ``timestamp`` field. You can read more about how Theta Data calculates greeks [here](/articles/option-greeks).
        - Multi-day requests are limited to 1 month of data.

        Defaults (upstream):
        - `strike`: `"*"`
        - `right`: `"both"`
        - `interval`: `"1s"`
        - `start_time`: `"09:30:00"`
        - `end_time`: `"16:00:00"`
        - `rate_type`: `"sofr"`
        - `binomial_steps`: `101`
        - `version`: `"latest"`


        Awaitable companion of the sync variant. The returned object resolves the request off the calling thread so a running event loop keeps servicing other coroutines.
        """
        ...

    def option_history_trade_greeks_second_order_async(
        self,
        symbol: str,
        expiration: Union[str, date, datetime],
        *,
        strike: Optional[str] = None,
        right: Optional[Union[str, Right]] = None,
        date: Optional[Union[str, date, datetime]] = None,
        start_time: Optional[Union[str, time, datetime]] = None,
        end_time: Optional[Union[str, time, datetime]] = None,
        annual_dividend: Optional[float] = None,
        annual_dividend_amount: Optional[float] = None,
        rate_type: Optional[Union[str, RateType]] = None,
        rate_value: Optional[float] = None,
        version: Optional[Union[str, Version]] = None,
        max_dte: Optional[int] = None,
        strike_range: Optional[int] = None,
        start_date: Optional[Union[str, date, datetime]] = None,
        end_date: Optional[Union[str, date, datetime]] = None,
        perf_boost_intraday: Optional[bool] = False,
        timeout_ms: Optional[int] = None,
    ) -> Awaitable[TradeGreeksSecondOrderTickList]:
        """Fetch second-order Greeks on each trade for an option contract.

        - Returns the data for all contracts that share the same provided symbol and expiration.
        - Calculates greeks for every trade reported by OPRA.
        - The underlying price represents whatever the last underlying price was at the ``timestamp`` field. You can read more about how Theta Data calculates greeks [here](/articles/option-greeks).
        - Multi-day requests are limited to 1 month of data, and must specify an expiration.

        Defaults (upstream):
        - `strike`: `"*"`
        - `right`: `"both"`
        - `start_time`: `"09:30:00"`
        - `end_time`: `"16:00:00"`
        - `rate_type`: `"sofr"`
        - `version`: `"latest"`
        - `perf_boost_intraday`: `false`


        Awaitable companion of the sync variant. The returned object resolves the request off the calling thread so a running event loop keeps servicing other coroutines.
        """
        ...

    def option_history_binomial_trade_greeks_second_order_async(
        self,
        symbol: str,
        expiration: Union[str, date, datetime],
        *,
        strike: Optional[str] = None,
        right: Optional[Union[str, Right]] = None,
        date: Optional[Union[str, date, datetime]] = None,
        start_time: Optional[Union[str, time, datetime]] = None,
        end_time: Optional[Union[str, time, datetime]] = None,
        annual_dividend: Optional[float] = None,
        annual_dividend_amount: Optional[float] = None,
        rate_type: Optional[Union[str, RateType]] = None,
        rate_value: Optional[float] = None,
        binomial_steps: Optional[int] = 101,
        version: Optional[Union[str, Version]] = None,
        max_dte: Optional[int] = None,
        strike_range: Optional[int] = None,
        start_date: Optional[Union[str, date, datetime]] = None,
        end_date: Optional[Union[str, date, datetime]] = None,
        perf_boost_intraday: Optional[bool] = False,
        timeout_ms: Optional[int] = None,
    ) -> Awaitable[TradeGreeksSecondOrderTickList]:
        """Fetch second-order binomial Greeks on each trade for an option contract.

        - Returns the data for all contracts that share the same provided symbol and expiration.
        - Greeks are calculated with a Leisen-Reimer binomial tree that allows early exercise at every node, so the values are American-style.
        - Calculates greeks for every trade reported by OPRA.
        - The underlying price represents whatever the last underlying price was at the ``timestamp`` field. You can read more about how Theta Data calculates greeks [here](/articles/option-greeks).
        - Multi-day requests are limited to 1 month of data, and must specify an expiration.

        Defaults (upstream):
        - `strike`: `"*"`
        - `right`: `"both"`
        - `start_time`: `"09:30:00"`
        - `end_time`: `"16:00:00"`
        - `rate_type`: `"sofr"`
        - `binomial_steps`: `101`
        - `version`: `"latest"`
        - `perf_boost_intraday`: `false`


        Awaitable companion of the sync variant. The returned object resolves the request off the calling thread so a running event loop keeps servicing other coroutines.
        """
        ...

    def option_history_greeks_third_order_async(
        self,
        symbol: str,
        expiration: Union[str, date, datetime],
        *,
        strike: Optional[str] = None,
        right: Optional[Union[str, Right]] = None,
        date: Optional[Union[str, date, datetime]] = None,
        interval: Optional[Union[str, Interval]] = None,
        start_time: Optional[Union[str, time, datetime]] = None,
        end_time: Optional[Union[str, time, datetime]] = None,
        annual_dividend: Optional[float] = None,
        annual_dividend_amount: Optional[float] = None,
        rate_type: Optional[Union[str, RateType]] = None,
        rate_value: Optional[float] = None,
        version: Optional[Union[str, Version]] = None,
        strike_range: Optional[int] = None,
        start_date: Optional[Union[str, date, datetime]] = None,
        end_date: Optional[Union[str, date, datetime]] = None,
        timeout_ms: Optional[int] = None,
    ) -> Awaitable[GreeksThirdOrderTickList]:
        """Fetch third-order Greeks history (intraday, sampled by interval).

        - Returns the data for all contracts that share the same provided symbol and expiration. 
        - Calculated using the option and underlying midpoint price. If an interval size is specified (*highly recommended*), the option quote used in the calculation follows the same rules as the quote endpoint. 
        - The underlying price represents whatever the last underlying price was at the ``timestamp`` field. You can read more about how Theta Data calculates greeks [here](/articles/option-greeks).
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

    def option_history_binomial_greeks_third_order_async(
        self,
        symbol: str,
        expiration: Union[str, date, datetime],
        *,
        strike: Optional[str] = None,
        right: Optional[Union[str, Right]] = None,
        date: Optional[Union[str, date, datetime]] = None,
        interval: Optional[Union[str, Interval]] = None,
        start_time: Optional[Union[str, time, datetime]] = None,
        end_time: Optional[Union[str, time, datetime]] = None,
        annual_dividend: Optional[float] = None,
        annual_dividend_amount: Optional[float] = None,
        rate_type: Optional[Union[str, RateType]] = None,
        rate_value: Optional[float] = None,
        binomial_steps: Optional[int] = 101,
        version: Optional[Union[str, Version]] = None,
        strike_range: Optional[int] = None,
        start_date: Optional[Union[str, date, datetime]] = None,
        end_date: Optional[Union[str, date, datetime]] = None,
        timeout_ms: Optional[int] = None,
    ) -> Awaitable[GreeksThirdOrderTickList]:
        """Fetch third-order binomial Greeks history (intraday, sampled by interval).

        - Returns the data for all contracts that share the same provided symbol and expiration. 
        - Greeks are calculated with a Leisen-Reimer binomial tree that allows early exercise at every node, so the values are American-style.
        - Calculated using the option and underlying midpoint price. If an interval size is specified (*highly recommended*), the option quote used in the calculation follows the same rules as the quote endpoint. 
        - The underlying price represents whatever the last underlying price was at the ``timestamp`` field. You can read more about how Theta Data calculates greeks [here](/articles/option-greeks).
        - Multi-day requests are limited to 1 month of data.

        Defaults (upstream):
        - `strike`: `"*"`
        - `right`: `"both"`
        - `interval`: `"1s"`
        - `start_time`: `"09:30:00"`
        - `end_time`: `"16:00:00"`
        - `rate_type`: `"sofr"`
        - `binomial_steps`: `101`
        - `version`: `"latest"`


        Awaitable companion of the sync variant. The returned object resolves the request off the calling thread so a running event loop keeps servicing other coroutines.
        """
        ...

    def option_history_trade_greeks_third_order_async(
        self,
        symbol: str,
        expiration: Union[str, date, datetime],
        *,
        strike: Optional[str] = None,
        right: Optional[Union[str, Right]] = None,
        date: Optional[Union[str, date, datetime]] = None,
        start_time: Optional[Union[str, time, datetime]] = None,
        end_time: Optional[Union[str, time, datetime]] = None,
        annual_dividend: Optional[float] = None,
        annual_dividend_amount: Optional[float] = None,
        rate_type: Optional[Union[str, RateType]] = None,
        rate_value: Optional[float] = None,
        version: Optional[Union[str, Version]] = None,
        max_dte: Optional[int] = None,
        strike_range: Optional[int] = None,
        start_date: Optional[Union[str, date, datetime]] = None,
        end_date: Optional[Union[str, date, datetime]] = None,
        perf_boost_intraday: Optional[bool] = False,
        timeout_ms: Optional[int] = None,
    ) -> Awaitable[TradeGreeksThirdOrderTickList]:
        """Fetch third-order Greeks on each trade for an option contract.

        - Returns the data for all contracts that share the same provided symbol and expiration.
        - Calculates greeks for every trade reported by OPRA.
        - The underlying price represents whatever the last underlying price was at the ``timestamp`` field. You can read more about how Theta Data calculates greeks [here](/articles/option-greeks).
        - Multi-day requests are limited to 1 month of data, and must specify an expiration.

        Defaults (upstream):
        - `strike`: `"*"`
        - `right`: `"both"`
        - `start_time`: `"09:30:00"`
        - `end_time`: `"16:00:00"`
        - `rate_type`: `"sofr"`
        - `version`: `"latest"`
        - `perf_boost_intraday`: `false`


        Awaitable companion of the sync variant. The returned object resolves the request off the calling thread so a running event loop keeps servicing other coroutines.
        """
        ...

    def option_history_binomial_trade_greeks_third_order_async(
        self,
        symbol: str,
        expiration: Union[str, date, datetime],
        *,
        strike: Optional[str] = None,
        right: Optional[Union[str, Right]] = None,
        date: Optional[Union[str, date, datetime]] = None,
        start_time: Optional[Union[str, time, datetime]] = None,
        end_time: Optional[Union[str, time, datetime]] = None,
        annual_dividend: Optional[float] = None,
        annual_dividend_amount: Optional[float] = None,
        rate_type: Optional[Union[str, RateType]] = None,
        rate_value: Optional[float] = None,
        binomial_steps: Optional[int] = 101,
        version: Optional[Union[str, Version]] = None,
        max_dte: Optional[int] = None,
        strike_range: Optional[int] = None,
        start_date: Optional[Union[str, date, datetime]] = None,
        end_date: Optional[Union[str, date, datetime]] = None,
        perf_boost_intraday: Optional[bool] = False,
        timeout_ms: Optional[int] = None,
    ) -> Awaitable[TradeGreeksThirdOrderTickList]:
        """Fetch third-order binomial Greeks on each trade for an option contract.

        - Returns the data for all contracts that share the same provided symbol and expiration.
        - Greeks are calculated with a Leisen-Reimer binomial tree that allows early exercise at every node, so the values are American-style.
        - Calculates greeks for every trade reported by OPRA.
        - The underlying price represents whatever the last underlying price was at the ``timestamp`` field. You can read more about how Theta Data calculates greeks [here](/articles/option-greeks).
        - Multi-day requests are limited to 1 month of data, and must specify an expiration.

        Defaults (upstream):
        - `strike`: `"*"`
        - `right`: `"both"`
        - `start_time`: `"09:30:00"`
        - `end_time`: `"16:00:00"`
        - `rate_type`: `"sofr"`
        - `binomial_steps`: `101`
        - `version`: `"latest"`
        - `perf_boost_intraday`: `false`


        Awaitable companion of the sync variant. The returned object resolves the request off the calling thread so a running event loop keeps servicing other coroutines.
        """
        ...

    def option_history_greeks_implied_volatility_async(
        self,
        symbol: str,
        expiration: Union[str, date, datetime],
        *,
        strike: Optional[str] = None,
        right: Optional[Union[str, Right]] = None,
        date: Optional[Union[str, date, datetime]] = None,
        interval: Optional[Union[str, Interval]] = None,
        start_time: Optional[Union[str, time, datetime]] = None,
        end_time: Optional[Union[str, time, datetime]] = None,
        annual_dividend: Optional[float] = None,
        annual_dividend_amount: Optional[float] = None,
        rate_type: Optional[Union[str, RateType]] = None,
        rate_value: Optional[float] = None,
        version: Optional[Union[str, Version]] = None,
        strike_range: Optional[int] = None,
        start_date: Optional[Union[str, date, datetime]] = None,
        end_date: Optional[Union[str, date, datetime]] = None,
        timeout_ms: Optional[int] = None,
    ) -> Awaitable[IvTickList]:
        """Fetch implied volatility history (intraday, sampled by interval).

        - Returns implied volatilies calculated using the national best bid, mid, and ask price of the option respectively. 
        - The underlying price represents whatever the last underlying price was at the ``timestamp`` field. You can read more about how Theta Data calculates greeks [here](/articles/option-greeks).
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

    def option_history_trade_greeks_implied_volatility_async(
        self,
        symbol: str,
        expiration: Union[str, date, datetime],
        *,
        strike: Optional[str] = None,
        right: Optional[Union[str, Right]] = None,
        date: Optional[Union[str, date, datetime]] = None,
        start_time: Optional[Union[str, time, datetime]] = None,
        end_time: Optional[Union[str, time, datetime]] = None,
        annual_dividend: Optional[float] = None,
        annual_dividend_amount: Optional[float] = None,
        rate_type: Optional[Union[str, RateType]] = None,
        rate_value: Optional[float] = None,
        version: Optional[Union[str, Version]] = None,
        max_dte: Optional[int] = None,
        strike_range: Optional[int] = None,
        start_date: Optional[Union[str, date, datetime]] = None,
        end_date: Optional[Union[str, date, datetime]] = None,
        perf_boost_intraday: Optional[bool] = False,
        timeout_ms: Optional[int] = None,
    ) -> Awaitable[TradeGreeksImpliedVolatilityTickList]:
        """Fetch implied volatility on each trade for an option contract.

        - Returns implied volatilies calculated using the trade reported by OPRA. 
        - The underlying price represents whatever the last underlying price was at the ``timestamp`` field. You can read more about how Theta Data calculates greeks [here](/articles/option-greeks).
        - Multi-day requests are limited to 1 month of data, and must specify an expiration.

        Defaults (upstream):
        - `strike`: `"*"`
        - `right`: `"both"`
        - `start_time`: `"09:30:00"`
        - `end_time`: `"16:00:00"`
        - `rate_type`: `"sofr"`
        - `version`: `"latest"`
        - `perf_boost_intraday`: `false`


        Awaitable companion of the sync variant. The returned object resolves the request off the calling thread so a running event loop keeps servicing other coroutines.
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
        right: Optional[Union[str, Right]] = None,
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

    def option_at_time_quote_async(
        self,
        symbol: str,
        expiration: Union[str, date, datetime],
        start_date: Union[str, date, datetime],
        end_date: Union[str, date, datetime],
        time_of_day: Union[str, time, datetime],
        *,
        strike: Optional[str] = None,
        right: Optional[Union[str, Right]] = None,
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

    def option_flat_file_eod_async(
        self,
        date: Union[str, date, datetime],
        *,
        timeout_ms: Optional[int] = None,
    ) -> Awaitable[EodTickList]:
        """Fetch the end-of-day report of every option contract for one date. The vendor keeps the most recent 7 calendar days.

        Awaitable companion of the sync variant. The returned object resolves the request off the calling thread so a running event loop keeps servicing other coroutines.
        """
        ...

    def option_flat_file_eod_to_path_async(
        self,
        date: Union[str, date, datetime],
        path: Union[str, PathLike[str]],
        format: Optional[str] = None,
    ) -> Awaitable[str]:
        """Write the `option_flat_file_eod` file for `date` to `path` as the terminal writes it, and return the path written.

        ``format`` is ``"csv"`` (the default), ``"json"``, ``"json_legacy"``, ``"ndjson"`` (or ``"jsonl"``) or ``"html"``. The file streams to disk chunk by chunk and takes its name only once complete; a path without an extension gets the format's.

        Awaitable companion of the sync variant. The returned object resolves to the path written, so a running event loop keeps servicing other coroutines while the file streams to disk.
        """
        ...

    def option_flat_file_trade_quote_async(
        self,
        date: Union[str, date, datetime],
        *,
        timeout_ms: Optional[int] = None,
    ) -> Awaitable[TradeQuoteTickList]:
        """Fetch every option trade of one date, each paired with the NBBO quote at the time of the trade. The vendor keeps the most recent 7 calendar days.

        Awaitable companion of the sync variant. The returned object resolves the request off the calling thread so a running event loop keeps servicing other coroutines.
        """
        ...

    def option_flat_file_trade_quote_to_path_async(
        self,
        date: Union[str, date, datetime],
        path: Union[str, PathLike[str]],
        format: Optional[str] = None,
    ) -> Awaitable[str]:
        """Write the `option_flat_file_trade_quote` file for `date` to `path` as the terminal writes it, and return the path written.

        ``format`` is ``"csv"`` (the default), ``"json"``, ``"json_legacy"``, ``"ndjson"`` (or ``"jsonl"``) or ``"html"``. The file streams to disk chunk by chunk and takes its name only once complete; a path without an extension gets the format's.

        Awaitable companion of the sync variant. The returned object resolves to the path written, so a running event loop keeps servicing other coroutines while the file streams to disk.
        """
        ...

    def option_flat_file_open_interest_async(
        self,
        date: Union[str, date, datetime],
        *,
        timeout_ms: Optional[int] = None,
    ) -> Awaitable[OpenInterestTickList]:
        """Fetch the open interest of every option contract for one date. The vendor keeps the most recent 7 calendar days.

        Awaitable companion of the sync variant. The returned object resolves the request off the calling thread so a running event loop keeps servicing other coroutines.
        """
        ...

    def option_flat_file_open_interest_to_path_async(
        self,
        date: Union[str, date, datetime],
        path: Union[str, PathLike[str]],
        format: Optional[str] = None,
    ) -> Awaitable[str]:
        """Write the `option_flat_file_open_interest` file for `date` to `path` as the terminal writes it, and return the path written.

        ``format`` is ``"csv"`` (the default), ``"json"``, ``"json_legacy"``, ``"ndjson"`` (or ``"jsonl"``) or ``"html"``. The file streams to disk chunk by chunk and takes its name only once complete; a path without an extension gets the format's.

        Awaitable companion of the sync variant. The returned object resolves to the path written, so a running event loop keeps servicing other coroutines while the file streams to disk.
        """
        ...

    def option_flat_file_quote_async(
        self,
        date: Union[str, date, datetime],
        *,
        timeout_ms: Optional[int] = None,
    ) -> Awaitable[IntervalQuoteTickList]:
        """Fetch every option contract's 1-minute NBBO quote intervals for one date: one row per minute in which a new quote was reported. The vendor keeps the most recent 7 calendar days.

        Awaitable companion of the sync variant. The returned object resolves the request off the calling thread so a running event loop keeps servicing other coroutines.
        """
        ...

    def option_flat_file_quote_to_path_async(
        self,
        date: Union[str, date, datetime],
        path: Union[str, PathLike[str]],
        format: Optional[str] = None,
    ) -> Awaitable[str]:
        """Write the `option_flat_file_quote` file for `date` to `path` as the terminal writes it, and return the path written.

        ``format`` is ``"csv"`` (the default), ``"json"``, ``"json_legacy"``, ``"ndjson"`` (or ``"jsonl"``) or ``"html"``. The file streams to disk chunk by chunk and takes its name only once complete; a path without an extension gets the format's.

        Awaitable companion of the sync variant. The returned object resolves to the path written, so a running event loop keeps servicing other coroutines while the file streams to disk.
        """
        ...

    def index_list_symbols_async(
        self,
        *,
        timeout_ms: Optional[int] = None,
    ) -> Awaitable[StringList]:
        """List all available index symbols.

        A symbol can be defined as a unique identifier for a stock / underlying asset. Common terms also include: root, ticker, and underlying. This endpoint returns all traded index symbols. This endpoint is updated overnight.


        Awaitable companion of the sync variant. The returned object resolves the request off the calling thread so a running event loop keeps servicing other coroutines.
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

    def index_snapshot_ohlc_async(
        self,
        symbols: Union[str, Sequence[str]],
        *,
        min_time: Optional[Union[str, time, datetime]] = None,
        current_day_only: Optional[bool] = False,
        timeout_ms: Optional[int] = None,
    ) -> Awaitable[OhlcTickList]:
        """Get the latest OHLC snapshot for one or more indices.

        - Retrieves the real-time current day OHLC.
        - Exchanges typically generate a price report every second for popular indices like SPX.

        Defaults (upstream):
        - `current_day_only`: `false`


        Awaitable companion of the sync variant. The returned object resolves the request off the calling thread so a running event loop keeps servicing other coroutines.
        """
        ...

    def index_snapshot_price_async(
        self,
        symbols: Union[str, Sequence[str]],
        *,
        min_time: Optional[Union[str, time, datetime]] = None,
        current_day_only: Optional[bool] = False,
        timeout_ms: Optional[int] = None,
    ) -> Awaitable[PriceTickList]:
        """Get the latest price snapshot for one or more indices.

        - Retrieves a real-time last index price.
        - Exchanges typically generate a price report every second for popular indices like SPX.

        Defaults (upstream):
        - `current_day_only`: `false`


        Awaitable companion of the sync variant. The returned object resolves the request off the calling thread so a running event loop keeps servicing other coroutines.
        """
        ...

    def index_snapshot_market_value_async(
        self,
        symbols: Union[str, Sequence[str]],
        *,
        min_time: Optional[Union[str, time, datetime]] = None,
        current_day_only: Optional[bool] = False,
        timeout_ms: Optional[int] = None,
    ) -> Awaitable[MarketValueTickList]:
        """Get the latest market value snapshot for one or more indices.

        - Retrieves a real-time last index market value.
        - Exchanges typically generate a price report every second for popular indices like SPX.

        Defaults (upstream):
        - `current_day_only`: `false`


        Awaitable companion of the sync variant. The returned object resolves the request off the calling thread so a running event loop keeps servicing other coroutines.
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

    def index_history_ohlc_async(
        self,
        symbol: str,
        start_date: Union[str, date, datetime],
        end_date: Union[str, date, datetime],
        *,
        interval: Optional[Union[str, Interval]] = None,
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

    def index_history_price_async(
        self,
        symbol: str,
        *,
        date: Optional[Union[str, date, datetime]] = None,
        interval: Optional[Union[str, Interval]] = None,
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

    def index_flat_file_eod_async(
        self,
        date: Union[str, date, datetime],
        *,
        timeout_ms: Optional[int] = None,
    ) -> Awaitable[EodTickList]:
        """Fetch the end-of-day report of every index for one date. The vendor keeps the most recent 7 calendar days.

        Awaitable companion of the sync variant. The returned object resolves the request off the calling thread so a running event loop keeps servicing other coroutines.
        """
        ...

    def index_flat_file_eod_to_path_async(
        self,
        date: Union[str, date, datetime],
        path: Union[str, PathLike[str]],
        format: Optional[str] = None,
    ) -> Awaitable[str]:
        """Write the `index_flat_file_eod` file for `date` to `path` as the terminal writes it, and return the path written.

        ``format`` is ``"csv"`` (the default), ``"json"``, ``"json_legacy"``, ``"ndjson"`` (or ``"jsonl"``) or ``"html"``. The file streams to disk chunk by chunk and takes its name only once complete; a path without an extension gets the format's.

        Awaitable companion of the sync variant. The returned object resolves to the path written, so a running event loop keeps servicing other coroutines while the file streams to disk.
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

    def calendar_year_async(
        self,
        year: str,
        *,
        timeout_ms: Optional[int] = None,
    ) -> Awaitable[List[CalendarDay]]:
        """Get equity market holidays and early-close days for a year (vendor `year_holidays` endpoint: only non-standard days, not every trading day).

        - Retrieves equity market holidays for a given year
        - Note: Holiday data is available 01/01/2012 through the end of the calendar year that immediately follows the current year
        - *On days when the market closes early at 1:00 PM ET; eligible options will trade until 1:15 PM.
        - **Some NYSE exchanges will continue late trading until 5:00 PM ET on early close days.


        Awaitable companion of the sync variant. The returned object resolves the request off the calling thread so a running event loop keeps servicing other coroutines.
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


@final
class MarketDataView(_AsyncMarketData):
    """Market-data sub-namespace returned by :attr:`Client.market_data`.

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

    def stock_list_symbols_builder(self) -> StockListSymbolsBuilder:
        """Fluent builder for `stock_list_symbols`. Chain the optional setters, then call `.list()` (or `.list_async()`) to execute; the returned typed list wrapper exposes `.to_list()` / `.to_arrow()` / `.to_pandas()` / `.to_polars()`."""
        ...

    def stock_list_dates(
        self,
        request_type: Union[str, RequestType],
        symbol: str,
        *,
        timeout_ms: Optional[int] = None,
    ) -> StringList:
        """List available dates for a stock by request type (EOD, TRADE, QUOTE, etc.).

        Lists all dates of data that are available for a stock with a given request type and symbol. This endpoint is updated overnight.
        """
        ...

    def stock_list_dates_builder(
        self,
        request_type: Union[str, RequestType],
        symbol: str,
    ) -> StockListDatesBuilder:
        """Fluent builder for `stock_list_dates`. Chain the optional setters, then call `.list()` (or `.list_async()`) to execute; the returned typed list wrapper exposes `.to_list()` / `.to_arrow()` / `.to_pandas()` / `.to_polars()`."""
        ...

    def stock_snapshot_ohlc(
        self,
        symbols: Union[str, Sequence[str]],
        *,
        venue: Optional[Union[str, Venue]] = None,
        min_time: Optional[Union[str, time, datetime]] = None,
        current_day_only: Optional[bool] = False,
        timeout_ms: Optional[int] = None,
    ) -> OhlcTickList:
        """Get the latest OHLC snapshot for one or more stocks.

        Provides a real-time Open, High, Low, Close for the current day.
        * Returns a real-time session OHLC from the Nasdaq Basic feed if the account has a stocks standard or pro subscription.
        * Returns a 15-minute delayed session OHLC from the UTP & CTA feeds if the account has the stocks value subscription.
        * Theta Data resets its snapshot cache at midnight ET every day. This endpoint may not work on a weekend where there were no eligible messages sent over exchange feeds. We recommend using historic requests during the weekend.

        Defaults (upstream):
        - `venue`: `"nqb"`
        - `current_day_only`: `false`
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
        venue: Optional[Union[str, Venue]] = None,
        min_time: Optional[Union[str, time, datetime]] = None,
        current_day_only: Optional[bool] = False,
        timeout_ms: Optional[int] = None,
    ) -> TradeTickList:
        """Get the latest trade snapshot for one or more stocks.

        Returns a real-time last trade from the Nasdaq Basic feed if the account has a stocks standard or pro subscription.

        - Theta Data resets its snapshot cache at midnight ET every day. This endpoint may not work on a weekend where there were no eligible messages sent over exchange feeds. We recommend using historic requests during the weekend.

        Defaults (upstream):
        - `venue`: `"nqb"`
        - `current_day_only`: `false`
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
        venue: Optional[Union[str, Venue]] = None,
        min_time: Optional[Union[str, time, datetime]] = None,
        current_day_only: Optional[bool] = False,
        timeout_ms: Optional[int] = None,
    ) -> QuoteTickList:
        """Get the latest NBBO quote snapshot for one or more stocks.

        * Returns a real-time last BBO quote from the Nasdaq Basic feed if the account has a stocks standard or pro subscription.
        * Returns a 15-minute delayed NBBO quote from the UTP & CTA feeds account has the stocks value subscription subscription.
        - Theta Data resets its snapshot cache at midnight ET every day. This endpoint may not work on a weekend where there were no eligible messages sent over exchange feeds. We recommend using historic requests during the weekend.

        Defaults (upstream):
        - `venue`: `"nqb"`
        - `current_day_only`: `false`
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
        venue: Optional[Union[str, Venue]] = None,
        min_time: Optional[Union[str, time, datetime]] = None,
        current_day_only: Optional[bool] = False,
        timeout_ms: Optional[int] = None,
    ) -> MarketValueTickList:
        """Get the latest market value snapshot for one or more stocks.

        * Returns a real-time market value derived from the last BBO quote from the Nasdaq Basic feed if the account has a stocks standard or pro subscription.
        * Returns a 15-minute delayed market value derived from an NBBO quote from the UTP & CTA feeds if the account has the stocks value subscription subscription.
        - Theta Data resets its snapshot cache at midnight ET every day. This endpoint may not work on a weekend where there were no eligible messages sent over exchange feeds. We recommend using historic requests during the weekend.

        Defaults (upstream):
        - `venue`: `"nqb"`
        - `current_day_only`: `false`
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
        *,
        date: Optional[Union[str, date, datetime]] = None,
        interval: Optional[Union[str, Interval]] = None,
        start_time: Optional[Union[str, time, datetime]] = None,
        end_time: Optional[Union[str, time, datetime]] = None,
        venue: Optional[Union[str, Venue]] = None,
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

    def stock_history_ohlc_builder(
        self,
        symbol: str,
    ) -> StockHistoryOhlcBuilder:
        """Fluent builder for `stock_history_ohlc`. Chain the optional setters, then call `.list()` (or `.list_async()`) to execute; the returned typed list wrapper exposes `.to_list()` / `.to_arrow()` / `.to_pandas()` / `.to_polars()`."""
        ...

    def stock_history_trade(
        self,
        symbol: str,
        *,
        date: Optional[Union[str, date, datetime]] = None,
        start_time: Optional[Union[str, time, datetime]] = None,
        end_time: Optional[Union[str, time, datetime]] = None,
        venue: Optional[Union[str, Venue]] = None,
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

    def stock_history_trade_builder(
        self,
        symbol: str,
    ) -> StockHistoryTradeBuilder:
        """Fluent builder for `stock_history_trade`. Chain the optional setters, then call `.list()` (or `.list_async()`) to execute; the returned typed list wrapper exposes `.to_list()` / `.to_arrow()` / `.to_pandas()` / `.to_polars()`."""
        ...

    def stock_history_quote(
        self,
        symbol: str,
        *,
        date: Optional[Union[str, date, datetime]] = None,
        interval: Optional[Union[str, Interval]] = None,
        start_time: Optional[Union[str, time, datetime]] = None,
        end_time: Optional[Union[str, time, datetime]] = None,
        venue: Optional[Union[str, Venue]] = None,
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

    def stock_history_quote_builder(
        self,
        symbol: str,
    ) -> StockHistoryQuoteBuilder:
        """Fluent builder for `stock_history_quote`. Chain the optional setters, then call `.list()` (or `.list_async()`) to execute; the returned typed list wrapper exposes `.to_list()` / `.to_arrow()` / `.to_pandas()` / `.to_polars()`."""
        ...

    def stock_history_trade_quote(
        self,
        symbol: str,
        *,
        date: Optional[Union[str, date, datetime]] = None,
        start_time: Optional[Union[str, time, datetime]] = None,
        end_time: Optional[Union[str, time, datetime]] = None,
        exclusive: Optional[bool] = True,
        venue: Optional[Union[str, Venue]] = None,
        start_date: Optional[Union[str, date, datetime]] = None,
        end_date: Optional[Union[str, date, datetime]] = None,
        use_time_based_trade_quote: Optional[bool] = True,
        timeout_ms: Optional[int] = None,
    ) -> TradeQuoteTickList:
        """Fetch combined trade + quote ticks for a stock on a given date.

        Returns every trade reported by UTP & CTA paired with the last BBO quote reported by UTP or CTA at the time of trade. A quote is matched with a trade if its timestamp ``<=`` the trade timestamp. If you prefer to match quotes with timestamps that are ``<`` the trade timestamp, specify the ``exclusive`` parameter to ``true``. Set the ``venue`` parameter to ``nqb`` to access current-day real-time historic data from the Nasdaq Basic feed if the account has a stocks standard or pro subscription.
        - Multi-day requests are limited to 1 month of data.

        Defaults (upstream):
        - `start_time`: `"09:30:00"`
        - `end_time`: `"16:00:00"`
        - `exclusive`: `true`
        - `venue`: `"nqb"`
        - `use_time_based_trade_quote`: `true`
        """
        ...

    def stock_history_trade_quote_builder(
        self,
        symbol: str,
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
        venue: Optional[Union[str, Venue]] = None,
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
        venue: Optional[Union[str, Venue]] = None,
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

    def stock_at_time_quote_builder(
        self,
        symbol: str,
        start_date: Union[str, date, datetime],
        end_date: Union[str, date, datetime],
        time_of_day: Union[str, time, datetime],
    ) -> StockAtTimeQuoteBuilder:
        """Fluent builder for `stock_at_time_quote`. Chain the optional setters, then call `.list()` (or `.list_async()`) to execute; the returned typed list wrapper exposes `.to_list()` / `.to_arrow()` / `.to_pandas()` / `.to_polars()`."""
        ...

    def stock_flat_file_eod(
        self,
        date: Union[str, date, datetime],
        *,
        timeout_ms: Optional[int] = None,
    ) -> EodTickList:
        """Fetch the end-of-day report of every stock for one date. The vendor keeps the most recent 7 calendar days."""
        ...

    def stock_flat_file_eod_builder(
        self,
        date: Union[str, date, datetime],
    ) -> StockFlatFileEodBuilder:
        """Fluent builder for `stock_flat_file_eod`. Chain the optional setters, then call `.list()` (or `.list_async()`) to execute; the returned typed list wrapper exposes `.to_list()` / `.to_arrow()` / `.to_pandas()` / `.to_polars()`."""
        ...

    def stock_flat_file_eod_to_path(
        self,
        date: Union[str, date, datetime],
        path: Union[str, PathLike[str]],
        format: Optional[str] = None,
    ) -> str:
        """Write the `stock_flat_file_eod` file for `date` to `path` as the terminal writes it, and return the path written.

        ``format`` is ``"csv"`` (the default), ``"json"``, ``"json_legacy"``, ``"ndjson"`` (or ``"jsonl"``) or ``"html"``. The file streams to disk chunk by chunk and takes its name only once complete; a path without an extension gets the format's.
        """
        ...

    def stock_flat_file_trade_quote(
        self,
        date: Union[str, date, datetime],
        *,
        timeout_ms: Optional[int] = None,
    ) -> TradeQuoteTickList:
        """Fetch every stock trade of one date, each paired with the NBBO quote at the time of the trade. The vendor keeps the most recent 7 calendar days."""
        ...

    def stock_flat_file_trade_quote_builder(
        self,
        date: Union[str, date, datetime],
    ) -> StockFlatFileTradeQuoteBuilder:
        """Fluent builder for `stock_flat_file_trade_quote`. Chain the optional setters, then call `.list()` (or `.list_async()`) to execute; the returned typed list wrapper exposes `.to_list()` / `.to_arrow()` / `.to_pandas()` / `.to_polars()`."""
        ...

    def stock_flat_file_trade_quote_to_path(
        self,
        date: Union[str, date, datetime],
        path: Union[str, PathLike[str]],
        format: Optional[str] = None,
    ) -> str:
        """Write the `stock_flat_file_trade_quote` file for `date` to `path` as the terminal writes it, and return the path written.

        ``format`` is ``"csv"`` (the default), ``"json"``, ``"json_legacy"``, ``"ndjson"`` (or ``"jsonl"``) or ``"html"``. The file streams to disk chunk by chunk and takes its name only once complete; a path without an extension gets the format's.
        """
        ...

    def stock_flat_file_quote(
        self,
        date: Union[str, date, datetime],
        *,
        timeout_ms: Optional[int] = None,
    ) -> IntervalQuoteTickList:
        """Fetch every stock's 1-minute NBBO quote intervals for one date: one row per minute in which a new quote was reported. The vendor keeps the most recent 7 calendar days."""
        ...

    def stock_flat_file_quote_builder(
        self,
        date: Union[str, date, datetime],
    ) -> StockFlatFileQuoteBuilder:
        """Fluent builder for `stock_flat_file_quote`. Chain the optional setters, then call `.list()` (or `.list_async()`) to execute; the returned typed list wrapper exposes `.to_list()` / `.to_arrow()` / `.to_pandas()` / `.to_polars()`."""
        ...

    def stock_flat_file_quote_to_path(
        self,
        date: Union[str, date, datetime],
        path: Union[str, PathLike[str]],
        format: Optional[str] = None,
    ) -> str:
        """Write the `stock_flat_file_quote` file for `date` to `path` as the terminal writes it, and return the path written.

        ``format`` is ``"csv"`` (the default), ``"json"``, ``"json_legacy"``, ``"ndjson"`` (or ``"jsonl"``) or ``"html"``. The file streams to disk chunk by chunk and takes its name only once complete; a path without an extension gets the format's.
        """
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

    def option_list_symbols_builder(self) -> OptionListSymbolsBuilder:
        """Fluent builder for `option_list_symbols`. Chain the optional setters, then call `.list()` (or `.list_async()`) to execute; the returned typed list wrapper exposes `.to_list()` / `.to_arrow()` / `.to_pandas()` / `.to_polars()`."""
        ...

    def option_list_dates(
        self,
        request_type: Union[str, RequestType],
        symbol: str,
        expiration: Union[str, date, datetime],
        *,
        strike: Optional[str] = None,
        right: Optional[Union[str, Right]] = None,
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

    def option_list_dates_builder(
        self,
        request_type: Union[str, RequestType],
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

    def option_list_strikes_builder(
        self,
        symbol: str,
        expiration: Union[str, date, datetime],
    ) -> OptionListStrikesBuilder:
        """Fluent builder for `option_list_strikes`. Chain the optional setters, then call `.list()` (or `.list_async()`) to execute; the returned typed list wrapper exposes `.to_list()` / `.to_arrow()` / `.to_pandas()` / `.to_polars()`."""
        ...

    def option_list_contracts(
        self,
        request_type: Union[str, RequestType],
        date: Union[str, date, datetime],
        *,
        symbol: Optional[str] = None,
        max_dte: Optional[int] = None,
        timeout_ms: Optional[int] = None,
    ) -> OptionContractList:
        """List all option contracts traded or quoted on a given date, optionally filtered to a symbol.

        Lists all contracts that were traded or quoted on a particular date.

        If the ``symbol`` parameter is specified, the returned contracts will be filtered to match the symbol.
        When ``symbol`` is omitted the full universe of contracts for that date is returned.
        This endpoint is updated real-time.
        """
        ...

    def option_list_contracts_builder(
        self,
        request_type: Union[str, RequestType],
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
        right: Optional[Union[str, Right]] = None,
        max_dte: Optional[int] = None,
        strike_range: Optional[int] = None,
        min_time: Optional[Union[str, time, datetime]] = None,
        current_day_only: Optional[bool] = False,
        timeout_ms: Optional[int] = None,
    ) -> List[OhlcTick]:
        """Get the latest OHLC snapshot for an option contract.

        - Retrieve a real-time last ohlc of an option contract for the trading day.

        Defaults (upstream):
        - `strike`: `"*"`
        - `right`: `"both"`
        - `current_day_only`: `false`
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
        right: Optional[Union[str, Right]] = None,
        strike_range: Optional[int] = None,
        min_time: Optional[Union[str, time, datetime]] = None,
        current_day_only: Optional[bool] = False,
        timeout_ms: Optional[int] = None,
    ) -> List[TradeTick]:
        """Get the latest trade snapshot for an option contract.

        - Retrieve the real-time last trade of an option contract.
        - This endpoint will return no data if the market was closed for the day. Theta Data resets the snapshot cache at midnight ET every night.

        Defaults (upstream):
        - `strike`: `"*"`
        - `right`: `"both"`
        - `current_day_only`: `false`
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
        right: Optional[Union[str, Right]] = None,
        max_dte: Optional[int] = None,
        strike_range: Optional[int] = None,
        min_time: Optional[Union[str, time, datetime]] = None,
        current_day_only: Optional[bool] = False,
        timeout_ms: Optional[int] = None,
    ) -> List[QuoteTick]:
        """Get the latest NBBO quote snapshot for an option contract.

        - Retrieve a real-time last NBBO quote of an option contract.
        - This endpoint will return no data if the market was closed for the day. Theta Data resets the snapshot cache at midnight ET every night.

        Defaults (upstream):
        - `strike`: `"*"`
        - `right`: `"both"`
        - `current_day_only`: `false`
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
        right: Optional[Union[str, Right]] = None,
        max_dte: Optional[int] = None,
        strike_range: Optional[int] = None,
        min_time: Optional[Union[str, time, datetime]] = None,
        current_day_only: Optional[bool] = False,
        timeout_ms: Optional[int] = None,
    ) -> List[OpenInterestTick]:
        """Get the latest open interest snapshot for an option contract.

        - Retrieve the last open interest message of an option contract.
        - Open interest is reported around 06:30 ET every morning by OPRA and reflects the open interest at the end of the previous trading day.
        - This endpoint will return no data if the market was closed for the day. Theta Data resets the snapshot cache at midnight ET every night.

        Defaults (upstream):
        - `strike`: `"*"`
        - `right`: `"both"`
        - `current_day_only`: `false`
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
        right: Optional[Union[str, Right]] = None,
        max_dte: Optional[int] = None,
        strike_range: Optional[int] = None,
        min_time: Optional[Union[str, time, datetime]] = None,
        current_day_only: Optional[bool] = False,
        timeout_ms: Optional[int] = None,
    ) -> List[MarketValueTick]:
        """Get the latest market value snapshot for an option contract.

        * Returns a real-time market value derived from the last NBBO quote of an option contract.

        Defaults (upstream):
        - `strike`: `"*"`
        - `right`: `"both"`
        - `current_day_only`: `false`
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
        right: Optional[Union[str, Right]] = None,
        annual_dividend: Optional[float] = None,
        annual_dividend_amount: Optional[float] = None,
        rate_type: Optional[Union[str, RateType]] = None,
        rate_value: Optional[float] = None,
        stock_price: Optional[float] = None,
        version: Optional[Union[str, Version]] = None,
        max_dte: Optional[int] = None,
        strike_range: Optional[int] = None,
        min_time: Optional[Union[str, time, datetime]] = None,
        use_market_value: Optional[bool] = False,
        current_day_only: Optional[bool] = False,
        timeout_ms: Optional[int] = None,
    ) -> List[IvTick]:
        """Get implied volatility snapshot for an option contract (from ThetaData server).

        Returns implied volatilies calculated using the national best bid, mid, and ask price
        of the option respectively. The underlying price represents whatever the last underlying price was at the
        ``underlying_timestamp`` field. You can read more about how Theta Data calculates greeks [here](/articles/option-greeks).

        Defaults (upstream):
        - `strike`: `"*"`
        - `right`: `"both"`
        - `rate_type`: `"sofr"`
        - `version`: `"latest"`
        - `use_market_value`: `false`
        - `current_day_only`: `false`
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
        right: Optional[Union[str, Right]] = None,
        annual_dividend: Optional[float] = None,
        annual_dividend_amount: Optional[float] = None,
        rate_type: Optional[Union[str, RateType]] = None,
        rate_value: Optional[float] = None,
        stock_price: Optional[float] = None,
        version: Optional[Union[str, Version]] = None,
        max_dte: Optional[int] = None,
        strike_range: Optional[int] = None,
        min_time: Optional[Union[str, time, datetime]] = None,
        use_market_value: Optional[bool] = False,
        current_day_only: Optional[bool] = False,
        timeout_ms: Optional[int] = None,
    ) -> List[GreeksAllTick]:
        """Get all Greeks snapshot for an option contract (from ThetaData server).

        - Retrieve a real-time last greeks calculation for all option contracts that lie on a provided expiration.
        > This endpoint will return no data if the market was closed for the day. Theta Data resets the snapshot cache at midnight ET every night.

        Defaults (upstream):
        - `strike`: `"*"`
        - `right`: `"both"`
        - `rate_type`: `"sofr"`
        - `version`: `"latest"`
        - `use_market_value`: `false`
        - `current_day_only`: `false`
        """
        ...

    def option_snapshot_greeks_all_builder(
        self,
        symbol: str,
        expiration: Union[str, date, datetime],
    ) -> OptionSnapshotGreeksAllBuilder:
        """Fluent builder for `option_snapshot_greeks_all`. Chain the optional setters, then call `.list()` (or `.list_async()`) to execute; the returned typed list wrapper exposes `.to_list()` / `.to_arrow()` / `.to_pandas()` / `.to_polars()`."""
        ...

    def option_snapshot_binomial_greeks_all(
        self,
        symbol: str,
        expiration: Union[str, date, datetime],
        *,
        strike: Optional[str] = None,
        right: Optional[Union[str, Right]] = None,
        annual_dividend: Optional[float] = None,
        annual_dividend_amount: Optional[float] = None,
        rate_type: Optional[Union[str, RateType]] = None,
        rate_value: Optional[float] = None,
        stock_price: Optional[float] = None,
        binomial_steps: Optional[int] = 101,
        version: Optional[Union[str, Version]] = None,
        max_dte: Optional[int] = None,
        strike_range: Optional[int] = None,
        min_time: Optional[Union[str, time, datetime]] = None,
        use_market_value: Optional[bool] = False,
        current_day_only: Optional[bool] = False,
        timeout_ms: Optional[int] = None,
    ) -> List[GreeksAllTick]:
        """Get all binomial Greeks snapshot for an option contract (from ThetaData server).

        - Retrieve a real-time last greeks calculation for all option contracts that lie on a provided expiration.
        - Greeks are calculated with a Leisen-Reimer binomial tree that allows early exercise at every node, so the values are American-style.
        > This endpoint will return no data if the market was closed for the day. Theta Data resets the snapshot cache at midnight ET every night.

        Defaults (upstream):
        - `strike`: `"*"`
        - `right`: `"both"`
        - `rate_type`: `"sofr"`
        - `binomial_steps`: `101`
        - `version`: `"latest"`
        - `use_market_value`: `false`
        - `current_day_only`: `false`
        """
        ...

    def option_snapshot_binomial_greeks_all_builder(
        self,
        symbol: str,
        expiration: Union[str, date, datetime],
    ) -> OptionSnapshotBinomialGreeksAllBuilder:
        """Fluent builder for `option_snapshot_binomial_greeks_all`. Chain the optional setters, then call `.list()` (or `.list_async()`) to execute; the returned typed list wrapper exposes `.to_list()` / `.to_arrow()` / `.to_pandas()` / `.to_polars()`."""
        ...

    def option_snapshot_greeks_first_order(
        self,
        symbol: str,
        expiration: Union[str, date, datetime],
        *,
        strike: Optional[str] = None,
        right: Optional[Union[str, Right]] = None,
        annual_dividend: Optional[float] = None,
        annual_dividend_amount: Optional[float] = None,
        rate_type: Optional[Union[str, RateType]] = None,
        rate_value: Optional[float] = None,
        stock_price: Optional[float] = None,
        version: Optional[Union[str, Version]] = None,
        max_dte: Optional[int] = None,
        strike_range: Optional[int] = None,
        min_time: Optional[Union[str, time, datetime]] = None,
        use_market_value: Optional[bool] = False,
        current_day_only: Optional[bool] = False,
        timeout_ms: Optional[int] = None,
    ) -> List[GreeksFirstOrderTick]:
        """Get first-order Greeks snapshot (delta, theta, rho) for an option contract.

        - Retrieve a real-time last greeks calculation for all option contracts that lie on a provided expiration.
        > This endpoint will return no data if the market was closed for the day. Theta Data resets the snapshot cache at midnight ET every night.

        Defaults (upstream):
        - `strike`: `"*"`
        - `right`: `"both"`
        - `rate_type`: `"sofr"`
        - `version`: `"latest"`
        - `use_market_value`: `false`
        - `current_day_only`: `false`
        """
        ...

    def option_snapshot_greeks_first_order_builder(
        self,
        symbol: str,
        expiration: Union[str, date, datetime],
    ) -> OptionSnapshotGreeksFirstOrderBuilder:
        """Fluent builder for `option_snapshot_greeks_first_order`. Chain the optional setters, then call `.list()` (or `.list_async()`) to execute; the returned typed list wrapper exposes `.to_list()` / `.to_arrow()` / `.to_pandas()` / `.to_polars()`."""
        ...

    def option_snapshot_binomial_greeks_first_order(
        self,
        symbol: str,
        expiration: Union[str, date, datetime],
        *,
        strike: Optional[str] = None,
        right: Optional[Union[str, Right]] = None,
        annual_dividend: Optional[float] = None,
        annual_dividend_amount: Optional[float] = None,
        rate_type: Optional[Union[str, RateType]] = None,
        rate_value: Optional[float] = None,
        stock_price: Optional[float] = None,
        binomial_steps: Optional[int] = 101,
        version: Optional[Union[str, Version]] = None,
        max_dte: Optional[int] = None,
        strike_range: Optional[int] = None,
        min_time: Optional[Union[str, time, datetime]] = None,
        use_market_value: Optional[bool] = False,
        current_day_only: Optional[bool] = False,
        timeout_ms: Optional[int] = None,
    ) -> List[GreeksFirstOrderTick]:
        """Get first-order binomial Greeks snapshot (delta, theta, rho) for an option contract.

        - Retrieve a real-time last greeks calculation for all option contracts that lie on a provided expiration.
        - Greeks are calculated with a Leisen-Reimer binomial tree that allows early exercise at every node, so the values are American-style.
        > This endpoint will return no data if the market was closed for the day. Theta Data resets the snapshot cache at midnight ET every night.

        Defaults (upstream):
        - `strike`: `"*"`
        - `right`: `"both"`
        - `rate_type`: `"sofr"`
        - `binomial_steps`: `101`
        - `version`: `"latest"`
        - `use_market_value`: `false`
        - `current_day_only`: `false`
        """
        ...

    def option_snapshot_binomial_greeks_first_order_builder(
        self,
        symbol: str,
        expiration: Union[str, date, datetime],
    ) -> OptionSnapshotBinomialGreeksFirstOrderBuilder:
        """Fluent builder for `option_snapshot_binomial_greeks_first_order`. Chain the optional setters, then call `.list()` (or `.list_async()`) to execute; the returned typed list wrapper exposes `.to_list()` / `.to_arrow()` / `.to_pandas()` / `.to_polars()`."""
        ...

    def option_snapshot_greeks_second_order(
        self,
        symbol: str,
        expiration: Union[str, date, datetime],
        *,
        strike: Optional[str] = None,
        right: Optional[Union[str, Right]] = None,
        annual_dividend: Optional[float] = None,
        annual_dividend_amount: Optional[float] = None,
        rate_type: Optional[Union[str, RateType]] = None,
        rate_value: Optional[float] = None,
        stock_price: Optional[float] = None,
        version: Optional[Union[str, Version]] = None,
        max_dte: Optional[int] = None,
        strike_range: Optional[int] = None,
        min_time: Optional[Union[str, time, datetime]] = None,
        use_market_value: Optional[bool] = False,
        current_day_only: Optional[bool] = False,
        timeout_ms: Optional[int] = None,
    ) -> List[GreeksSecondOrderTick]:
        """Get second-order Greeks snapshot (gamma, vanna, charm) for an option contract.

        - Retrieve a real-time last second order greeks calculation for all option contracts that lie on a provided expiration.
        > This endpoint will return no data if the market was closed for the day. Theta Data resets the snapshot cache at midnight ET every night.

        Defaults (upstream):
        - `strike`: `"*"`
        - `right`: `"both"`
        - `rate_type`: `"sofr"`
        - `version`: `"latest"`
        - `use_market_value`: `false`
        - `current_day_only`: `false`
        """
        ...

    def option_snapshot_greeks_second_order_builder(
        self,
        symbol: str,
        expiration: Union[str, date, datetime],
    ) -> OptionSnapshotGreeksSecondOrderBuilder:
        """Fluent builder for `option_snapshot_greeks_second_order`. Chain the optional setters, then call `.list()` (or `.list_async()`) to execute; the returned typed list wrapper exposes `.to_list()` / `.to_arrow()` / `.to_pandas()` / `.to_polars()`."""
        ...

    def option_snapshot_binomial_greeks_second_order(
        self,
        symbol: str,
        expiration: Union[str, date, datetime],
        *,
        strike: Optional[str] = None,
        right: Optional[Union[str, Right]] = None,
        annual_dividend: Optional[float] = None,
        annual_dividend_amount: Optional[float] = None,
        rate_type: Optional[Union[str, RateType]] = None,
        rate_value: Optional[float] = None,
        stock_price: Optional[float] = None,
        binomial_steps: Optional[int] = 101,
        version: Optional[Union[str, Version]] = None,
        max_dte: Optional[int] = None,
        strike_range: Optional[int] = None,
        min_time: Optional[Union[str, time, datetime]] = None,
        use_market_value: Optional[bool] = False,
        current_day_only: Optional[bool] = False,
        timeout_ms: Optional[int] = None,
    ) -> List[GreeksSecondOrderTick]:
        """Get second-order binomial Greeks snapshot (gamma, vanna, charm) for an option contract.

        - Retrieve a real-time last second order binomial greeks calculation for all option contracts that lie on a provided expiration.
        - Greeks are calculated with a Leisen-Reimer binomial tree that allows early exercise at every node, so the values are American-style.
        > This endpoint will return no data if the market was closed for the day. Theta Data resets the snapshot cache at midnight ET every night.

        Defaults (upstream):
        - `strike`: `"*"`
        - `right`: `"both"`
        - `rate_type`: `"sofr"`
        - `binomial_steps`: `101`
        - `version`: `"latest"`
        - `use_market_value`: `false`
        - `current_day_only`: `false`
        """
        ...

    def option_snapshot_binomial_greeks_second_order_builder(
        self,
        symbol: str,
        expiration: Union[str, date, datetime],
    ) -> OptionSnapshotBinomialGreeksSecondOrderBuilder:
        """Fluent builder for `option_snapshot_binomial_greeks_second_order`. Chain the optional setters, then call `.list()` (or `.list_async()`) to execute; the returned typed list wrapper exposes `.to_list()` / `.to_arrow()` / `.to_pandas()` / `.to_polars()`."""
        ...

    def option_snapshot_greeks_third_order(
        self,
        symbol: str,
        expiration: Union[str, date, datetime],
        *,
        strike: Optional[str] = None,
        right: Optional[Union[str, Right]] = None,
        annual_dividend: Optional[float] = None,
        annual_dividend_amount: Optional[float] = None,
        rate_type: Optional[Union[str, RateType]] = None,
        rate_value: Optional[float] = None,
        stock_price: Optional[float] = None,
        version: Optional[Union[str, Version]] = None,
        max_dte: Optional[int] = None,
        strike_range: Optional[int] = None,
        min_time: Optional[Union[str, time, datetime]] = None,
        use_market_value: Optional[bool] = False,
        current_day_only: Optional[bool] = False,
        timeout_ms: Optional[int] = None,
    ) -> List[GreeksThirdOrderTick]:
        """Get third-order Greeks snapshot (speed, color, ultima) for an option contract.

        - Retrieve a real-time last third order greeks calculation for all option contracts that lie on a provided expiration.
        > This endpoint will return no data if the market was closed for the day. Theta Data resets the snapshot cache at midnight ET every night.

        Defaults (upstream):
        - `strike`: `"*"`
        - `right`: `"both"`
        - `rate_type`: `"sofr"`
        - `version`: `"latest"`
        - `use_market_value`: `false`
        - `current_day_only`: `false`
        """
        ...

    def option_snapshot_greeks_third_order_builder(
        self,
        symbol: str,
        expiration: Union[str, date, datetime],
    ) -> OptionSnapshotGreeksThirdOrderBuilder:
        """Fluent builder for `option_snapshot_greeks_third_order`. Chain the optional setters, then call `.list()` (or `.list_async()`) to execute; the returned typed list wrapper exposes `.to_list()` / `.to_arrow()` / `.to_pandas()` / `.to_polars()`."""
        ...

    def option_snapshot_binomial_greeks_third_order(
        self,
        symbol: str,
        expiration: Union[str, date, datetime],
        *,
        strike: Optional[str] = None,
        right: Optional[Union[str, Right]] = None,
        annual_dividend: Optional[float] = None,
        annual_dividend_amount: Optional[float] = None,
        rate_type: Optional[Union[str, RateType]] = None,
        rate_value: Optional[float] = None,
        stock_price: Optional[float] = None,
        binomial_steps: Optional[int] = 101,
        version: Optional[Union[str, Version]] = None,
        max_dte: Optional[int] = None,
        strike_range: Optional[int] = None,
        min_time: Optional[Union[str, time, datetime]] = None,
        use_market_value: Optional[bool] = False,
        current_day_only: Optional[bool] = False,
        timeout_ms: Optional[int] = None,
    ) -> List[GreeksThirdOrderTick]:
        """Get third-order binomial Greeks snapshot (speed, color, ultima) for an option contract.

        - Retrieve a real-time last third order binomial greeks calculation for all option contracts that lie on a provided expiration.
        - Greeks are calculated with a Leisen-Reimer binomial tree that allows early exercise at every node, so the values are American-style.
        > This endpoint will return no data if the market was closed for the day. Theta Data resets the snapshot cache at midnight ET every night.

        Defaults (upstream):
        - `strike`: `"*"`
        - `right`: `"both"`
        - `rate_type`: `"sofr"`
        - `binomial_steps`: `101`
        - `version`: `"latest"`
        - `use_market_value`: `false`
        - `current_day_only`: `false`
        """
        ...

    def option_snapshot_binomial_greeks_third_order_builder(
        self,
        symbol: str,
        expiration: Union[str, date, datetime],
    ) -> OptionSnapshotBinomialGreeksThirdOrderBuilder:
        """Fluent builder for `option_snapshot_binomial_greeks_third_order`. Chain the optional setters, then call `.list()` (or `.list_async()`) to execute; the returned typed list wrapper exposes `.to_list()` / `.to_arrow()` / `.to_pandas()` / `.to_polars()`."""
        ...

    def option_history_eod(
        self,
        symbol: str,
        expiration: Union[str, date, datetime],
        start_date: Union[str, date, datetime],
        end_date: Union[str, date, datetime],
        *,
        strike: Optional[str] = None,
        right: Optional[Union[str, Right]] = None,
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
        *,
        strike: Optional[str] = None,
        right: Optional[Union[str, Right]] = None,
        date: Optional[Union[str, date, datetime]] = None,
        interval: Optional[Union[str, Interval]] = None,
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

    def option_history_ohlc_builder(
        self,
        symbol: str,
        expiration: Union[str, date, datetime],
    ) -> OptionHistoryOhlcBuilder:
        """Fluent builder for `option_history_ohlc`. Chain the optional setters, then call `.list()` (or `.list_async()`) to execute; the returned typed list wrapper exposes `.to_list()` / `.to_arrow()` / `.to_pandas()` / `.to_polars()`."""
        ...

    def option_history_trade(
        self,
        symbol: str,
        expiration: Union[str, date, datetime],
        *,
        strike: Optional[str] = None,
        right: Optional[Union[str, Right]] = None,
        date: Optional[Union[str, date, datetime]] = None,
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

    def option_history_trade_builder(
        self,
        symbol: str,
        expiration: Union[str, date, datetime],
    ) -> OptionHistoryTradeBuilder:
        """Fluent builder for `option_history_trade`. Chain the optional setters, then call `.list()` (or `.list_async()`) to execute; the returned typed list wrapper exposes `.to_list()` / `.to_arrow()` / `.to_pandas()` / `.to_polars()`."""
        ...

    def option_history_quote(
        self,
        symbol: str,
        expiration: Union[str, date, datetime],
        *,
        strike: Optional[str] = None,
        right: Optional[Union[str, Right]] = None,
        date: Optional[Union[str, date, datetime]] = None,
        interval: Optional[Union[str, Interval]] = None,
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

    def option_history_quote_builder(
        self,
        symbol: str,
        expiration: Union[str, date, datetime],
    ) -> OptionHistoryQuoteBuilder:
        """Fluent builder for `option_history_quote`. Chain the optional setters, then call `.list()` (or `.list_async()`) to execute; the returned typed list wrapper exposes `.to_list()` / `.to_arrow()` / `.to_pandas()` / `.to_polars()`."""
        ...

    def option_history_trade_quote(
        self,
        symbol: str,
        expiration: Union[str, date, datetime],
        *,
        strike: Optional[str] = None,
        right: Optional[Union[str, Right]] = None,
        date: Optional[Union[str, date, datetime]] = None,
        start_time: Optional[Union[str, time, datetime]] = None,
        end_time: Optional[Union[str, time, datetime]] = None,
        exclusive: Optional[bool] = True,
        max_dte: Optional[int] = None,
        strike_range: Optional[int] = None,
        start_date: Optional[Union[str, date, datetime]] = None,
        end_date: Optional[Union[str, date, datetime]] = None,
        use_time_based_trade_quote: Optional[bool] = True,
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
        - `exclusive`: `true`
        - `use_time_based_trade_quote`: `true`
        """
        ...

    def option_history_trade_quote_builder(
        self,
        symbol: str,
        expiration: Union[str, date, datetime],
    ) -> OptionHistoryTradeQuoteBuilder:
        """Fluent builder for `option_history_trade_quote`. Chain the optional setters, then call `.list()` (or `.list_async()`) to execute; the returned typed list wrapper exposes `.to_list()` / `.to_arrow()` / `.to_pandas()` / `.to_polars()`."""
        ...

    def option_history_open_interest(
        self,
        symbol: str,
        expiration: Union[str, date, datetime],
        *,
        strike: Optional[str] = None,
        right: Optional[Union[str, Right]] = None,
        date: Optional[Union[str, date, datetime]] = None,
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

    def option_history_open_interest_builder(
        self,
        symbol: str,
        expiration: Union[str, date, datetime],
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
        right: Optional[Union[str, Right]] = None,
        annual_dividend: Optional[float] = None,
        annual_dividend_amount: Optional[float] = None,
        rate_type: Optional[Union[str, RateType]] = None,
        rate_value: Optional[float] = None,
        version: Optional[Union[str, Version]] = None,
        underlyer_use_nbbo: Optional[bool] = False,
        max_dte: Optional[int] = None,
        strike_range: Optional[int] = None,
        option_use_nbbo: Optional[bool] = True,
        timeout_ms: Optional[int] = None,
    ) -> GreeksEodTickList:
        """Fetch end-of-day Greeks history for an option contract.

        - Returns the data for all contracts that share the same provided symbol and expiration.
        - Uses Theta Data's EOD reports that get generated at 17:15 ET each day. The closing option price and closing underlying price are used for the greeks calculation.
        - **Any ``expiration=*`` request must be made day by day.**

        Defaults (upstream):
        - `strike`: `"*"`
        - `right`: `"both"`
        - `rate_type`: `"sofr"`
        - `version`: `"latest"`
        - `underlyer_use_nbbo`: `false`
        - `option_use_nbbo`: `true`
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

    def option_history_binomial_greeks_eod(
        self,
        symbol: str,
        expiration: Union[str, date, datetime],
        start_date: Union[str, date, datetime],
        end_date: Union[str, date, datetime],
        *,
        strike: Optional[str] = None,
        right: Optional[Union[str, Right]] = None,
        annual_dividend: Optional[float] = None,
        annual_dividend_amount: Optional[float] = None,
        rate_type: Optional[Union[str, RateType]] = None,
        rate_value: Optional[float] = None,
        binomial_steps: Optional[int] = 101,
        version: Optional[Union[str, Version]] = None,
        underlyer_use_nbbo: Optional[bool] = False,
        max_dte: Optional[int] = None,
        strike_range: Optional[int] = None,
        option_use_nbbo: Optional[bool] = True,
        timeout_ms: Optional[int] = None,
    ) -> GreeksEodTickList:
        """Fetch end-of-day binomial Greeks history for an option contract.

        - Returns the data for all contracts that share the same provided symbol and expiration.
        - Greeks are calculated with a Leisen-Reimer binomial tree that allows early exercise at every node, so the values are American-style.
        - Uses Theta Data's EOD reports that get generated at 17:15 ET each day. The closing option price and closing underlying price are used for the greeks calculation.
        - **Any ``expiration=*`` request must be made day by day.**

        Defaults (upstream):
        - `strike`: `"*"`
        - `right`: `"both"`
        - `rate_type`: `"sofr"`
        - `binomial_steps`: `101`
        - `version`: `"latest"`
        - `underlyer_use_nbbo`: `false`
        - `option_use_nbbo`: `true`
        """
        ...

    def option_history_binomial_greeks_eod_builder(
        self,
        symbol: str,
        expiration: Union[str, date, datetime],
        start_date: Union[str, date, datetime],
        end_date: Union[str, date, datetime],
    ) -> OptionHistoryBinomialGreeksEodBuilder:
        """Fluent builder for `option_history_binomial_greeks_eod`. Chain the optional setters, then call `.list()` (or `.list_async()`) to execute; the returned typed list wrapper exposes `.to_list()` / `.to_arrow()` / `.to_pandas()` / `.to_polars()`."""
        ...

    def option_history_greeks_all(
        self,
        symbol: str,
        expiration: Union[str, date, datetime],
        *,
        strike: Optional[str] = None,
        right: Optional[Union[str, Right]] = None,
        date: Optional[Union[str, date, datetime]] = None,
        interval: Optional[Union[str, Interval]] = None,
        start_time: Optional[Union[str, time, datetime]] = None,
        end_time: Optional[Union[str, time, datetime]] = None,
        annual_dividend: Optional[float] = None,
        annual_dividend_amount: Optional[float] = None,
        rate_type: Optional[Union[str, RateType]] = None,
        rate_value: Optional[float] = None,
        version: Optional[Union[str, Version]] = None,
        strike_range: Optional[int] = None,
        start_date: Optional[Union[str, date, datetime]] = None,
        end_date: Optional[Union[str, date, datetime]] = None,
        timeout_ms: Optional[int] = None,
    ) -> GreeksAllTickList:
        """Fetch all Greeks history for an option contract (intraday, sampled by interval).

        - Returns the data for all contracts that share the same provided symbol and expiration. 
        - Calculated using the option and underlying midpoint price. If an interval size is specified (*highly recommended*), the option quote used in the calculation follows the same rules as the quote endpoint. 
        - The underlying price represents whatever the last underlying price was at the ``timestamp`` field. You can read more about how Theta Data calculates greeks [here](/articles/option-greeks).
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

    def option_history_greeks_all_builder(
        self,
        symbol: str,
        expiration: Union[str, date, datetime],
    ) -> OptionHistoryGreeksAllBuilder:
        """Fluent builder for `option_history_greeks_all`. Chain the optional setters, then call `.list()` (or `.list_async()`) to execute; the returned typed list wrapper exposes `.to_list()` / `.to_arrow()` / `.to_pandas()` / `.to_polars()`."""
        ...

    def option_history_binomial_greeks_all(
        self,
        symbol: str,
        expiration: Union[str, date, datetime],
        *,
        strike: Optional[str] = None,
        right: Optional[Union[str, Right]] = None,
        date: Optional[Union[str, date, datetime]] = None,
        interval: Optional[Union[str, Interval]] = None,
        start_time: Optional[Union[str, time, datetime]] = None,
        end_time: Optional[Union[str, time, datetime]] = None,
        annual_dividend: Optional[float] = None,
        annual_dividend_amount: Optional[float] = None,
        rate_type: Optional[Union[str, RateType]] = None,
        rate_value: Optional[float] = None,
        binomial_steps: Optional[int] = 101,
        version: Optional[Union[str, Version]] = None,
        strike_range: Optional[int] = None,
        start_date: Optional[Union[str, date, datetime]] = None,
        end_date: Optional[Union[str, date, datetime]] = None,
        timeout_ms: Optional[int] = None,
    ) -> GreeksAllTickList:
        """Fetch all binomial Greeks history for an option contract (intraday, sampled by interval).

        - Returns the data for all contracts that share the same provided symbol and expiration. 
        - Greeks are calculated with a Leisen-Reimer binomial tree that allows early exercise at every node, so the values are American-style.
        - Calculated using the option and underlying midpoint price. If an interval size is specified (*highly recommended*), the option quote used in the calculation follows the same rules as the quote endpoint. 
        - The underlying price represents whatever the last underlying price was at the ``timestamp`` field. You can read more about how Theta Data calculates greeks [here](/articles/option-greeks).
        - Multi-day requests are limited to 1 month of data.

        Defaults (upstream):
        - `strike`: `"*"`
        - `right`: `"both"`
        - `interval`: `"1s"`
        - `start_time`: `"09:30:00"`
        - `end_time`: `"16:00:00"`
        - `rate_type`: `"sofr"`
        - `binomial_steps`: `101`
        - `version`: `"latest"`
        """
        ...

    def option_history_binomial_greeks_all_builder(
        self,
        symbol: str,
        expiration: Union[str, date, datetime],
    ) -> OptionHistoryBinomialGreeksAllBuilder:
        """Fluent builder for `option_history_binomial_greeks_all`. Chain the optional setters, then call `.list()` (or `.list_async()`) to execute; the returned typed list wrapper exposes `.to_list()` / `.to_arrow()` / `.to_pandas()` / `.to_polars()`."""
        ...

    def option_history_trade_greeks_all(
        self,
        symbol: str,
        expiration: Union[str, date, datetime],
        *,
        strike: Optional[str] = None,
        right: Optional[Union[str, Right]] = None,
        date: Optional[Union[str, date, datetime]] = None,
        start_time: Optional[Union[str, time, datetime]] = None,
        end_time: Optional[Union[str, time, datetime]] = None,
        annual_dividend: Optional[float] = None,
        annual_dividend_amount: Optional[float] = None,
        rate_type: Optional[Union[str, RateType]] = None,
        rate_value: Optional[float] = None,
        version: Optional[Union[str, Version]] = None,
        max_dte: Optional[int] = None,
        strike_range: Optional[int] = None,
        start_date: Optional[Union[str, date, datetime]] = None,
        end_date: Optional[Union[str, date, datetime]] = None,
        perf_boost_intraday: Optional[bool] = False,
        timeout_ms: Optional[int] = None,
    ) -> TradeGreeksAllTickList:
        """Fetch all Greeks on each trade for an option contract.

        - Returns the data for all contracts that share the same provided symbol and expiration. 
        - Calculates greeks for every trade reported by OPRA.
        - The underlying price represents whatever the last underlying price was at the ``timestamp`` field. You can read more about how Theta Data calculates greeks [here](/articles/option-greeks).
        - Multi-day requests are limited to 1 month of data, and must specify an expiration.

        Defaults (upstream):
        - `strike`: `"*"`
        - `right`: `"both"`
        - `start_time`: `"09:30:00"`
        - `end_time`: `"16:00:00"`
        - `rate_type`: `"sofr"`
        - `version`: `"latest"`
        - `perf_boost_intraday`: `false`
        """
        ...

    def option_history_trade_greeks_all_builder(
        self,
        symbol: str,
        expiration: Union[str, date, datetime],
    ) -> OptionHistoryTradeGreeksAllBuilder:
        """Fluent builder for `option_history_trade_greeks_all`. Chain the optional setters, then call `.list()` (or `.list_async()`) to execute; the returned typed list wrapper exposes `.to_list()` / `.to_arrow()` / `.to_pandas()` / `.to_polars()`."""
        ...

    def option_history_binomial_trade_greeks_all(
        self,
        symbol: str,
        expiration: Union[str, date, datetime],
        *,
        strike: Optional[str] = None,
        right: Optional[Union[str, Right]] = None,
        date: Optional[Union[str, date, datetime]] = None,
        start_time: Optional[Union[str, time, datetime]] = None,
        end_time: Optional[Union[str, time, datetime]] = None,
        annual_dividend: Optional[float] = None,
        annual_dividend_amount: Optional[float] = None,
        rate_type: Optional[Union[str, RateType]] = None,
        rate_value: Optional[float] = None,
        binomial_steps: Optional[int] = 101,
        version: Optional[Union[str, Version]] = None,
        max_dte: Optional[int] = None,
        strike_range: Optional[int] = None,
        start_date: Optional[Union[str, date, datetime]] = None,
        end_date: Optional[Union[str, date, datetime]] = None,
        perf_boost_intraday: Optional[bool] = False,
        timeout_ms: Optional[int] = None,
    ) -> TradeGreeksAllTickList:
        """Fetch all binomial Greeks on each trade for an option contract.

        - Returns the data for all contracts that share the same provided symbol and expiration.
        - Greeks are calculated with a Leisen-Reimer binomial tree that allows early exercise at every node, so the values are American-style.
        - Calculates greeks for every trade reported by OPRA.
        - The underlying price represents whatever the last underlying price was at the ``timestamp`` field. You can read more about how Theta Data calculates greeks [here](/articles/option-greeks).
        - Multi-day requests are limited to 1 month of data, and must specify an expiration.

        Defaults (upstream):
        - `strike`: `"*"`
        - `right`: `"both"`
        - `start_time`: `"09:30:00"`
        - `end_time`: `"16:00:00"`
        - `rate_type`: `"sofr"`
        - `binomial_steps`: `101`
        - `version`: `"latest"`
        - `perf_boost_intraday`: `false`
        """
        ...

    def option_history_binomial_trade_greeks_all_builder(
        self,
        symbol: str,
        expiration: Union[str, date, datetime],
    ) -> OptionHistoryBinomialTradeGreeksAllBuilder:
        """Fluent builder for `option_history_binomial_trade_greeks_all`. Chain the optional setters, then call `.list()` (or `.list_async()`) to execute; the returned typed list wrapper exposes `.to_list()` / `.to_arrow()` / `.to_pandas()` / `.to_polars()`."""
        ...

    def option_history_greeks_first_order(
        self,
        symbol: str,
        expiration: Union[str, date, datetime],
        *,
        strike: Optional[str] = None,
        right: Optional[Union[str, Right]] = None,
        date: Optional[Union[str, date, datetime]] = None,
        interval: Optional[Union[str, Interval]] = None,
        start_time: Optional[Union[str, time, datetime]] = None,
        end_time: Optional[Union[str, time, datetime]] = None,
        annual_dividend: Optional[float] = None,
        annual_dividend_amount: Optional[float] = None,
        rate_type: Optional[Union[str, RateType]] = None,
        rate_value: Optional[float] = None,
        version: Optional[Union[str, Version]] = None,
        strike_range: Optional[int] = None,
        start_date: Optional[Union[str, date, datetime]] = None,
        end_date: Optional[Union[str, date, datetime]] = None,
        timeout_ms: Optional[int] = None,
    ) -> GreeksFirstOrderTickList:
        """Fetch first-order Greeks history (intraday, sampled by interval).

        - Returns the data for all contracts that share the same provided symbol and expiration. 
        - Calculated using the option and underlying midpoint price. If an interval size is specified (*highly recommended*), the option quote used in the calculation follows the same rules as the quote endpoint. 
        - The underlying price represents whatever the last underlying price was at the ``timestamp`` field. You can read more about how Theta Data calculates greeks [here](/articles/option-greeks).
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

    def option_history_greeks_first_order_builder(
        self,
        symbol: str,
        expiration: Union[str, date, datetime],
    ) -> OptionHistoryGreeksFirstOrderBuilder:
        """Fluent builder for `option_history_greeks_first_order`. Chain the optional setters, then call `.list()` (or `.list_async()`) to execute; the returned typed list wrapper exposes `.to_list()` / `.to_arrow()` / `.to_pandas()` / `.to_polars()`."""
        ...

    def option_history_binomial_greeks_first_order(
        self,
        symbol: str,
        expiration: Union[str, date, datetime],
        *,
        strike: Optional[str] = None,
        right: Optional[Union[str, Right]] = None,
        date: Optional[Union[str, date, datetime]] = None,
        interval: Optional[Union[str, Interval]] = None,
        start_time: Optional[Union[str, time, datetime]] = None,
        end_time: Optional[Union[str, time, datetime]] = None,
        annual_dividend: Optional[float] = None,
        annual_dividend_amount: Optional[float] = None,
        rate_type: Optional[Union[str, RateType]] = None,
        rate_value: Optional[float] = None,
        binomial_steps: Optional[int] = 101,
        version: Optional[Union[str, Version]] = None,
        strike_range: Optional[int] = None,
        start_date: Optional[Union[str, date, datetime]] = None,
        end_date: Optional[Union[str, date, datetime]] = None,
        timeout_ms: Optional[int] = None,
    ) -> GreeksFirstOrderTickList:
        """Fetch first-order binomial Greeks history (intraday, sampled by interval).

        - Returns the data for all contracts that share the same provided symbol and expiration. 
        - Greeks are calculated with a Leisen-Reimer binomial tree that allows early exercise at every node, so the values are American-style.
        - Calculated using the option and underlying midpoint price. If an interval size is specified (*highly recommended*), the option quote used in the calculation follows the same rules as the quote endpoint. 
        - The underlying price represents whatever the last underlying price was at the ``timestamp`` field. You can read more about how Theta Data calculates greeks [here](/articles/option-greeks).
        - Multi-day requests are limited to 1 month of data.

        Defaults (upstream):
        - `strike`: `"*"`
        - `right`: `"both"`
        - `interval`: `"1s"`
        - `start_time`: `"09:30:00"`
        - `end_time`: `"16:00:00"`
        - `rate_type`: `"sofr"`
        - `binomial_steps`: `101`
        - `version`: `"latest"`
        """
        ...

    def option_history_binomial_greeks_first_order_builder(
        self,
        symbol: str,
        expiration: Union[str, date, datetime],
    ) -> OptionHistoryBinomialGreeksFirstOrderBuilder:
        """Fluent builder for `option_history_binomial_greeks_first_order`. Chain the optional setters, then call `.list()` (or `.list_async()`) to execute; the returned typed list wrapper exposes `.to_list()` / `.to_arrow()` / `.to_pandas()` / `.to_polars()`."""
        ...

    def option_history_trade_greeks_first_order(
        self,
        symbol: str,
        expiration: Union[str, date, datetime],
        *,
        strike: Optional[str] = None,
        right: Optional[Union[str, Right]] = None,
        date: Optional[Union[str, date, datetime]] = None,
        start_time: Optional[Union[str, time, datetime]] = None,
        end_time: Optional[Union[str, time, datetime]] = None,
        annual_dividend: Optional[float] = None,
        annual_dividend_amount: Optional[float] = None,
        rate_type: Optional[Union[str, RateType]] = None,
        rate_value: Optional[float] = None,
        version: Optional[Union[str, Version]] = None,
        max_dte: Optional[int] = None,
        strike_range: Optional[int] = None,
        start_date: Optional[Union[str, date, datetime]] = None,
        end_date: Optional[Union[str, date, datetime]] = None,
        perf_boost_intraday: Optional[bool] = False,
        timeout_ms: Optional[int] = None,
    ) -> TradeGreeksFirstOrderTickList:
        """Fetch first-order Greeks on each trade for an option contract.

        - Returns the data for all contracts that share the same provided symbol and expiration.
        - Calculates greeks for every trade reported by OPRA.
        - The underlying price represents whatever the last underlying price was at the ``timestamp`` field. You can read more about how Theta Data calculates greeks [here](/articles/option-greeks).
        - Multi-day requests are limited to 1 month of data, and must specify an expiration.

        Defaults (upstream):
        - `strike`: `"*"`
        - `right`: `"both"`
        - `start_time`: `"09:30:00"`
        - `end_time`: `"16:00:00"`
        - `rate_type`: `"sofr"`
        - `version`: `"latest"`
        - `perf_boost_intraday`: `false`
        """
        ...

    def option_history_trade_greeks_first_order_builder(
        self,
        symbol: str,
        expiration: Union[str, date, datetime],
    ) -> OptionHistoryTradeGreeksFirstOrderBuilder:
        """Fluent builder for `option_history_trade_greeks_first_order`. Chain the optional setters, then call `.list()` (or `.list_async()`) to execute; the returned typed list wrapper exposes `.to_list()` / `.to_arrow()` / `.to_pandas()` / `.to_polars()`."""
        ...

    def option_history_binomial_trade_greeks_first_order(
        self,
        symbol: str,
        expiration: Union[str, date, datetime],
        *,
        strike: Optional[str] = None,
        right: Optional[Union[str, Right]] = None,
        date: Optional[Union[str, date, datetime]] = None,
        start_time: Optional[Union[str, time, datetime]] = None,
        end_time: Optional[Union[str, time, datetime]] = None,
        annual_dividend: Optional[float] = None,
        annual_dividend_amount: Optional[float] = None,
        rate_type: Optional[Union[str, RateType]] = None,
        rate_value: Optional[float] = None,
        binomial_steps: Optional[int] = 101,
        version: Optional[Union[str, Version]] = None,
        max_dte: Optional[int] = None,
        strike_range: Optional[int] = None,
        start_date: Optional[Union[str, date, datetime]] = None,
        end_date: Optional[Union[str, date, datetime]] = None,
        perf_boost_intraday: Optional[bool] = False,
        timeout_ms: Optional[int] = None,
    ) -> TradeGreeksFirstOrderTickList:
        """Fetch first-order binomial Greeks on each trade for an option contract.

        - Returns the data for all contracts that share the same provided symbol and expiration.
        - Greeks are calculated with a Leisen-Reimer binomial tree that allows early exercise at every node, so the values are American-style.
        - Calculates greeks for every trade reported by OPRA.
        - The underlying price represents whatever the last underlying price was at the ``timestamp`` field. You can read more about how Theta Data calculates greeks [here](/articles/option-greeks).
        - Multi-day requests are limited to 1 month of data, and must specify an expiration.

        Defaults (upstream):
        - `strike`: `"*"`
        - `right`: `"both"`
        - `start_time`: `"09:30:00"`
        - `end_time`: `"16:00:00"`
        - `rate_type`: `"sofr"`
        - `binomial_steps`: `101`
        - `version`: `"latest"`
        - `perf_boost_intraday`: `false`
        """
        ...

    def option_history_binomial_trade_greeks_first_order_builder(
        self,
        symbol: str,
        expiration: Union[str, date, datetime],
    ) -> OptionHistoryBinomialTradeGreeksFirstOrderBuilder:
        """Fluent builder for `option_history_binomial_trade_greeks_first_order`. Chain the optional setters, then call `.list()` (or `.list_async()`) to execute; the returned typed list wrapper exposes `.to_list()` / `.to_arrow()` / `.to_pandas()` / `.to_polars()`."""
        ...

    def option_history_greeks_second_order(
        self,
        symbol: str,
        expiration: Union[str, date, datetime],
        *,
        strike: Optional[str] = None,
        right: Optional[Union[str, Right]] = None,
        date: Optional[Union[str, date, datetime]] = None,
        interval: Optional[Union[str, Interval]] = None,
        start_time: Optional[Union[str, time, datetime]] = None,
        end_time: Optional[Union[str, time, datetime]] = None,
        annual_dividend: Optional[float] = None,
        annual_dividend_amount: Optional[float] = None,
        rate_type: Optional[Union[str, RateType]] = None,
        rate_value: Optional[float] = None,
        version: Optional[Union[str, Version]] = None,
        strike_range: Optional[int] = None,
        start_date: Optional[Union[str, date, datetime]] = None,
        end_date: Optional[Union[str, date, datetime]] = None,
        timeout_ms: Optional[int] = None,
    ) -> GreeksSecondOrderTickList:
        """Fetch second-order Greeks history (intraday, sampled by interval).

        - Returns the data for all contracts that share the same provided symbol and expiration. 
        - Calculated using the option and underlying midpoint price. If an interval size is specified (*highly recommended*), the option quote used in the calculation follows the same rules as the quote endpoint. 
        - The underlying price represents whatever the last underlying price was at the ``timestamp`` field. You can read more about how Theta Data calculates greeks [here](/articles/option-greeks).
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

    def option_history_greeks_second_order_builder(
        self,
        symbol: str,
        expiration: Union[str, date, datetime],
    ) -> OptionHistoryGreeksSecondOrderBuilder:
        """Fluent builder for `option_history_greeks_second_order`. Chain the optional setters, then call `.list()` (or `.list_async()`) to execute; the returned typed list wrapper exposes `.to_list()` / `.to_arrow()` / `.to_pandas()` / `.to_polars()`."""
        ...

    def option_history_binomial_greeks_second_order(
        self,
        symbol: str,
        expiration: Union[str, date, datetime],
        *,
        strike: Optional[str] = None,
        right: Optional[Union[str, Right]] = None,
        date: Optional[Union[str, date, datetime]] = None,
        interval: Optional[Union[str, Interval]] = None,
        start_time: Optional[Union[str, time, datetime]] = None,
        end_time: Optional[Union[str, time, datetime]] = None,
        annual_dividend: Optional[float] = None,
        annual_dividend_amount: Optional[float] = None,
        rate_type: Optional[Union[str, RateType]] = None,
        rate_value: Optional[float] = None,
        binomial_steps: Optional[int] = 101,
        version: Optional[Union[str, Version]] = None,
        strike_range: Optional[int] = None,
        start_date: Optional[Union[str, date, datetime]] = None,
        end_date: Optional[Union[str, date, datetime]] = None,
        timeout_ms: Optional[int] = None,
    ) -> GreeksSecondOrderTickList:
        """Fetch second-order binomial Greeks history (intraday, sampled by interval).

        - Returns the data for all contracts that share the same provided symbol and expiration. 
        - Greeks are calculated with a Leisen-Reimer binomial tree that allows early exercise at every node, so the values are American-style.
        - Calculated using the option and underlying midpoint price. If an interval size is specified (*highly recommended*), the option quote used in the calculation follows the same rules as the quote endpoint. 
        - The underlying price represents whatever the last underlying price was at the ``timestamp`` field. You can read more about how Theta Data calculates greeks [here](/articles/option-greeks).
        - Multi-day requests are limited to 1 month of data.

        Defaults (upstream):
        - `strike`: `"*"`
        - `right`: `"both"`
        - `interval`: `"1s"`
        - `start_time`: `"09:30:00"`
        - `end_time`: `"16:00:00"`
        - `rate_type`: `"sofr"`
        - `binomial_steps`: `101`
        - `version`: `"latest"`
        """
        ...

    def option_history_binomial_greeks_second_order_builder(
        self,
        symbol: str,
        expiration: Union[str, date, datetime],
    ) -> OptionHistoryBinomialGreeksSecondOrderBuilder:
        """Fluent builder for `option_history_binomial_greeks_second_order`. Chain the optional setters, then call `.list()` (or `.list_async()`) to execute; the returned typed list wrapper exposes `.to_list()` / `.to_arrow()` / `.to_pandas()` / `.to_polars()`."""
        ...

    def option_history_trade_greeks_second_order(
        self,
        symbol: str,
        expiration: Union[str, date, datetime],
        *,
        strike: Optional[str] = None,
        right: Optional[Union[str, Right]] = None,
        date: Optional[Union[str, date, datetime]] = None,
        start_time: Optional[Union[str, time, datetime]] = None,
        end_time: Optional[Union[str, time, datetime]] = None,
        annual_dividend: Optional[float] = None,
        annual_dividend_amount: Optional[float] = None,
        rate_type: Optional[Union[str, RateType]] = None,
        rate_value: Optional[float] = None,
        version: Optional[Union[str, Version]] = None,
        max_dte: Optional[int] = None,
        strike_range: Optional[int] = None,
        start_date: Optional[Union[str, date, datetime]] = None,
        end_date: Optional[Union[str, date, datetime]] = None,
        perf_boost_intraday: Optional[bool] = False,
        timeout_ms: Optional[int] = None,
    ) -> TradeGreeksSecondOrderTickList:
        """Fetch second-order Greeks on each trade for an option contract.

        - Returns the data for all contracts that share the same provided symbol and expiration.
        - Calculates greeks for every trade reported by OPRA.
        - The underlying price represents whatever the last underlying price was at the ``timestamp`` field. You can read more about how Theta Data calculates greeks [here](/articles/option-greeks).
        - Multi-day requests are limited to 1 month of data, and must specify an expiration.

        Defaults (upstream):
        - `strike`: `"*"`
        - `right`: `"both"`
        - `start_time`: `"09:30:00"`
        - `end_time`: `"16:00:00"`
        - `rate_type`: `"sofr"`
        - `version`: `"latest"`
        - `perf_boost_intraday`: `false`
        """
        ...

    def option_history_trade_greeks_second_order_builder(
        self,
        symbol: str,
        expiration: Union[str, date, datetime],
    ) -> OptionHistoryTradeGreeksSecondOrderBuilder:
        """Fluent builder for `option_history_trade_greeks_second_order`. Chain the optional setters, then call `.list()` (or `.list_async()`) to execute; the returned typed list wrapper exposes `.to_list()` / `.to_arrow()` / `.to_pandas()` / `.to_polars()`."""
        ...

    def option_history_binomial_trade_greeks_second_order(
        self,
        symbol: str,
        expiration: Union[str, date, datetime],
        *,
        strike: Optional[str] = None,
        right: Optional[Union[str, Right]] = None,
        date: Optional[Union[str, date, datetime]] = None,
        start_time: Optional[Union[str, time, datetime]] = None,
        end_time: Optional[Union[str, time, datetime]] = None,
        annual_dividend: Optional[float] = None,
        annual_dividend_amount: Optional[float] = None,
        rate_type: Optional[Union[str, RateType]] = None,
        rate_value: Optional[float] = None,
        binomial_steps: Optional[int] = 101,
        version: Optional[Union[str, Version]] = None,
        max_dte: Optional[int] = None,
        strike_range: Optional[int] = None,
        start_date: Optional[Union[str, date, datetime]] = None,
        end_date: Optional[Union[str, date, datetime]] = None,
        perf_boost_intraday: Optional[bool] = False,
        timeout_ms: Optional[int] = None,
    ) -> TradeGreeksSecondOrderTickList:
        """Fetch second-order binomial Greeks on each trade for an option contract.

        - Returns the data for all contracts that share the same provided symbol and expiration.
        - Greeks are calculated with a Leisen-Reimer binomial tree that allows early exercise at every node, so the values are American-style.
        - Calculates greeks for every trade reported by OPRA.
        - The underlying price represents whatever the last underlying price was at the ``timestamp`` field. You can read more about how Theta Data calculates greeks [here](/articles/option-greeks).
        - Multi-day requests are limited to 1 month of data, and must specify an expiration.

        Defaults (upstream):
        - `strike`: `"*"`
        - `right`: `"both"`
        - `start_time`: `"09:30:00"`
        - `end_time`: `"16:00:00"`
        - `rate_type`: `"sofr"`
        - `binomial_steps`: `101`
        - `version`: `"latest"`
        - `perf_boost_intraday`: `false`
        """
        ...

    def option_history_binomial_trade_greeks_second_order_builder(
        self,
        symbol: str,
        expiration: Union[str, date, datetime],
    ) -> OptionHistoryBinomialTradeGreeksSecondOrderBuilder:
        """Fluent builder for `option_history_binomial_trade_greeks_second_order`. Chain the optional setters, then call `.list()` (or `.list_async()`) to execute; the returned typed list wrapper exposes `.to_list()` / `.to_arrow()` / `.to_pandas()` / `.to_polars()`."""
        ...

    def option_history_greeks_third_order(
        self,
        symbol: str,
        expiration: Union[str, date, datetime],
        *,
        strike: Optional[str] = None,
        right: Optional[Union[str, Right]] = None,
        date: Optional[Union[str, date, datetime]] = None,
        interval: Optional[Union[str, Interval]] = None,
        start_time: Optional[Union[str, time, datetime]] = None,
        end_time: Optional[Union[str, time, datetime]] = None,
        annual_dividend: Optional[float] = None,
        annual_dividend_amount: Optional[float] = None,
        rate_type: Optional[Union[str, RateType]] = None,
        rate_value: Optional[float] = None,
        version: Optional[Union[str, Version]] = None,
        strike_range: Optional[int] = None,
        start_date: Optional[Union[str, date, datetime]] = None,
        end_date: Optional[Union[str, date, datetime]] = None,
        timeout_ms: Optional[int] = None,
    ) -> GreeksThirdOrderTickList:
        """Fetch third-order Greeks history (intraday, sampled by interval).

        - Returns the data for all contracts that share the same provided symbol and expiration. 
        - Calculated using the option and underlying midpoint price. If an interval size is specified (*highly recommended*), the option quote used in the calculation follows the same rules as the quote endpoint. 
        - The underlying price represents whatever the last underlying price was at the ``timestamp`` field. You can read more about how Theta Data calculates greeks [here](/articles/option-greeks).
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

    def option_history_greeks_third_order_builder(
        self,
        symbol: str,
        expiration: Union[str, date, datetime],
    ) -> OptionHistoryGreeksThirdOrderBuilder:
        """Fluent builder for `option_history_greeks_third_order`. Chain the optional setters, then call `.list()` (or `.list_async()`) to execute; the returned typed list wrapper exposes `.to_list()` / `.to_arrow()` / `.to_pandas()` / `.to_polars()`."""
        ...

    def option_history_binomial_greeks_third_order(
        self,
        symbol: str,
        expiration: Union[str, date, datetime],
        *,
        strike: Optional[str] = None,
        right: Optional[Union[str, Right]] = None,
        date: Optional[Union[str, date, datetime]] = None,
        interval: Optional[Union[str, Interval]] = None,
        start_time: Optional[Union[str, time, datetime]] = None,
        end_time: Optional[Union[str, time, datetime]] = None,
        annual_dividend: Optional[float] = None,
        annual_dividend_amount: Optional[float] = None,
        rate_type: Optional[Union[str, RateType]] = None,
        rate_value: Optional[float] = None,
        binomial_steps: Optional[int] = 101,
        version: Optional[Union[str, Version]] = None,
        strike_range: Optional[int] = None,
        start_date: Optional[Union[str, date, datetime]] = None,
        end_date: Optional[Union[str, date, datetime]] = None,
        timeout_ms: Optional[int] = None,
    ) -> GreeksThirdOrderTickList:
        """Fetch third-order binomial Greeks history (intraday, sampled by interval).

        - Returns the data for all contracts that share the same provided symbol and expiration. 
        - Greeks are calculated with a Leisen-Reimer binomial tree that allows early exercise at every node, so the values are American-style.
        - Calculated using the option and underlying midpoint price. If an interval size is specified (*highly recommended*), the option quote used in the calculation follows the same rules as the quote endpoint. 
        - The underlying price represents whatever the last underlying price was at the ``timestamp`` field. You can read more about how Theta Data calculates greeks [here](/articles/option-greeks).
        - Multi-day requests are limited to 1 month of data.

        Defaults (upstream):
        - `strike`: `"*"`
        - `right`: `"both"`
        - `interval`: `"1s"`
        - `start_time`: `"09:30:00"`
        - `end_time`: `"16:00:00"`
        - `rate_type`: `"sofr"`
        - `binomial_steps`: `101`
        - `version`: `"latest"`
        """
        ...

    def option_history_binomial_greeks_third_order_builder(
        self,
        symbol: str,
        expiration: Union[str, date, datetime],
    ) -> OptionHistoryBinomialGreeksThirdOrderBuilder:
        """Fluent builder for `option_history_binomial_greeks_third_order`. Chain the optional setters, then call `.list()` (or `.list_async()`) to execute; the returned typed list wrapper exposes `.to_list()` / `.to_arrow()` / `.to_pandas()` / `.to_polars()`."""
        ...

    def option_history_trade_greeks_third_order(
        self,
        symbol: str,
        expiration: Union[str, date, datetime],
        *,
        strike: Optional[str] = None,
        right: Optional[Union[str, Right]] = None,
        date: Optional[Union[str, date, datetime]] = None,
        start_time: Optional[Union[str, time, datetime]] = None,
        end_time: Optional[Union[str, time, datetime]] = None,
        annual_dividend: Optional[float] = None,
        annual_dividend_amount: Optional[float] = None,
        rate_type: Optional[Union[str, RateType]] = None,
        rate_value: Optional[float] = None,
        version: Optional[Union[str, Version]] = None,
        max_dte: Optional[int] = None,
        strike_range: Optional[int] = None,
        start_date: Optional[Union[str, date, datetime]] = None,
        end_date: Optional[Union[str, date, datetime]] = None,
        perf_boost_intraday: Optional[bool] = False,
        timeout_ms: Optional[int] = None,
    ) -> TradeGreeksThirdOrderTickList:
        """Fetch third-order Greeks on each trade for an option contract.

        - Returns the data for all contracts that share the same provided symbol and expiration.
        - Calculates greeks for every trade reported by OPRA.
        - The underlying price represents whatever the last underlying price was at the ``timestamp`` field. You can read more about how Theta Data calculates greeks [here](/articles/option-greeks).
        - Multi-day requests are limited to 1 month of data, and must specify an expiration.

        Defaults (upstream):
        - `strike`: `"*"`
        - `right`: `"both"`
        - `start_time`: `"09:30:00"`
        - `end_time`: `"16:00:00"`
        - `rate_type`: `"sofr"`
        - `version`: `"latest"`
        - `perf_boost_intraday`: `false`
        """
        ...

    def option_history_trade_greeks_third_order_builder(
        self,
        symbol: str,
        expiration: Union[str, date, datetime],
    ) -> OptionHistoryTradeGreeksThirdOrderBuilder:
        """Fluent builder for `option_history_trade_greeks_third_order`. Chain the optional setters, then call `.list()` (or `.list_async()`) to execute; the returned typed list wrapper exposes `.to_list()` / `.to_arrow()` / `.to_pandas()` / `.to_polars()`."""
        ...

    def option_history_binomial_trade_greeks_third_order(
        self,
        symbol: str,
        expiration: Union[str, date, datetime],
        *,
        strike: Optional[str] = None,
        right: Optional[Union[str, Right]] = None,
        date: Optional[Union[str, date, datetime]] = None,
        start_time: Optional[Union[str, time, datetime]] = None,
        end_time: Optional[Union[str, time, datetime]] = None,
        annual_dividend: Optional[float] = None,
        annual_dividend_amount: Optional[float] = None,
        rate_type: Optional[Union[str, RateType]] = None,
        rate_value: Optional[float] = None,
        binomial_steps: Optional[int] = 101,
        version: Optional[Union[str, Version]] = None,
        max_dte: Optional[int] = None,
        strike_range: Optional[int] = None,
        start_date: Optional[Union[str, date, datetime]] = None,
        end_date: Optional[Union[str, date, datetime]] = None,
        perf_boost_intraday: Optional[bool] = False,
        timeout_ms: Optional[int] = None,
    ) -> TradeGreeksThirdOrderTickList:
        """Fetch third-order binomial Greeks on each trade for an option contract.

        - Returns the data for all contracts that share the same provided symbol and expiration.
        - Greeks are calculated with a Leisen-Reimer binomial tree that allows early exercise at every node, so the values are American-style.
        - Calculates greeks for every trade reported by OPRA.
        - The underlying price represents whatever the last underlying price was at the ``timestamp`` field. You can read more about how Theta Data calculates greeks [here](/articles/option-greeks).
        - Multi-day requests are limited to 1 month of data, and must specify an expiration.

        Defaults (upstream):
        - `strike`: `"*"`
        - `right`: `"both"`
        - `start_time`: `"09:30:00"`
        - `end_time`: `"16:00:00"`
        - `rate_type`: `"sofr"`
        - `binomial_steps`: `101`
        - `version`: `"latest"`
        - `perf_boost_intraday`: `false`
        """
        ...

    def option_history_binomial_trade_greeks_third_order_builder(
        self,
        symbol: str,
        expiration: Union[str, date, datetime],
    ) -> OptionHistoryBinomialTradeGreeksThirdOrderBuilder:
        """Fluent builder for `option_history_binomial_trade_greeks_third_order`. Chain the optional setters, then call `.list()` (or `.list_async()`) to execute; the returned typed list wrapper exposes `.to_list()` / `.to_arrow()` / `.to_pandas()` / `.to_polars()`."""
        ...

    def option_history_greeks_implied_volatility(
        self,
        symbol: str,
        expiration: Union[str, date, datetime],
        *,
        strike: Optional[str] = None,
        right: Optional[Union[str, Right]] = None,
        date: Optional[Union[str, date, datetime]] = None,
        interval: Optional[Union[str, Interval]] = None,
        start_time: Optional[Union[str, time, datetime]] = None,
        end_time: Optional[Union[str, time, datetime]] = None,
        annual_dividend: Optional[float] = None,
        annual_dividend_amount: Optional[float] = None,
        rate_type: Optional[Union[str, RateType]] = None,
        rate_value: Optional[float] = None,
        version: Optional[Union[str, Version]] = None,
        strike_range: Optional[int] = None,
        start_date: Optional[Union[str, date, datetime]] = None,
        end_date: Optional[Union[str, date, datetime]] = None,
        timeout_ms: Optional[int] = None,
    ) -> IvTickList:
        """Fetch implied volatility history (intraday, sampled by interval).

        - Returns implied volatilies calculated using the national best bid, mid, and ask price of the option respectively. 
        - The underlying price represents whatever the last underlying price was at the ``timestamp`` field. You can read more about how Theta Data calculates greeks [here](/articles/option-greeks).
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

    def option_history_greeks_implied_volatility_builder(
        self,
        symbol: str,
        expiration: Union[str, date, datetime],
    ) -> OptionHistoryGreeksImpliedVolatilityBuilder:
        """Fluent builder for `option_history_greeks_implied_volatility`. Chain the optional setters, then call `.list()` (or `.list_async()`) to execute; the returned typed list wrapper exposes `.to_list()` / `.to_arrow()` / `.to_pandas()` / `.to_polars()`."""
        ...

    def option_history_trade_greeks_implied_volatility(
        self,
        symbol: str,
        expiration: Union[str, date, datetime],
        *,
        strike: Optional[str] = None,
        right: Optional[Union[str, Right]] = None,
        date: Optional[Union[str, date, datetime]] = None,
        start_time: Optional[Union[str, time, datetime]] = None,
        end_time: Optional[Union[str, time, datetime]] = None,
        annual_dividend: Optional[float] = None,
        annual_dividend_amount: Optional[float] = None,
        rate_type: Optional[Union[str, RateType]] = None,
        rate_value: Optional[float] = None,
        version: Optional[Union[str, Version]] = None,
        max_dte: Optional[int] = None,
        strike_range: Optional[int] = None,
        start_date: Optional[Union[str, date, datetime]] = None,
        end_date: Optional[Union[str, date, datetime]] = None,
        perf_boost_intraday: Optional[bool] = False,
        timeout_ms: Optional[int] = None,
    ) -> TradeGreeksImpliedVolatilityTickList:
        """Fetch implied volatility on each trade for an option contract.

        - Returns implied volatilies calculated using the trade reported by OPRA. 
        - The underlying price represents whatever the last underlying price was at the ``timestamp`` field. You can read more about how Theta Data calculates greeks [here](/articles/option-greeks).
        - Multi-day requests are limited to 1 month of data, and must specify an expiration.

        Defaults (upstream):
        - `strike`: `"*"`
        - `right`: `"both"`
        - `start_time`: `"09:30:00"`
        - `end_time`: `"16:00:00"`
        - `rate_type`: `"sofr"`
        - `version`: `"latest"`
        - `perf_boost_intraday`: `false`
        """
        ...

    def option_history_trade_greeks_implied_volatility_builder(
        self,
        symbol: str,
        expiration: Union[str, date, datetime],
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
        right: Optional[Union[str, Right]] = None,
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
        right: Optional[Union[str, Right]] = None,
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

    def option_flat_file_eod(
        self,
        date: Union[str, date, datetime],
        *,
        timeout_ms: Optional[int] = None,
    ) -> EodTickList:
        """Fetch the end-of-day report of every option contract for one date. The vendor keeps the most recent 7 calendar days."""
        ...

    def option_flat_file_eod_builder(
        self,
        date: Union[str, date, datetime],
    ) -> OptionFlatFileEodBuilder:
        """Fluent builder for `option_flat_file_eod`. Chain the optional setters, then call `.list()` (or `.list_async()`) to execute; the returned typed list wrapper exposes `.to_list()` / `.to_arrow()` / `.to_pandas()` / `.to_polars()`."""
        ...

    def option_flat_file_eod_to_path(
        self,
        date: Union[str, date, datetime],
        path: Union[str, PathLike[str]],
        format: Optional[str] = None,
    ) -> str:
        """Write the `option_flat_file_eod` file for `date` to `path` as the terminal writes it, and return the path written.

        ``format`` is ``"csv"`` (the default), ``"json"``, ``"json_legacy"``, ``"ndjson"`` (or ``"jsonl"``) or ``"html"``. The file streams to disk chunk by chunk and takes its name only once complete; a path without an extension gets the format's.
        """
        ...

    def option_flat_file_trade_quote(
        self,
        date: Union[str, date, datetime],
        *,
        timeout_ms: Optional[int] = None,
    ) -> TradeQuoteTickList:
        """Fetch every option trade of one date, each paired with the NBBO quote at the time of the trade. The vendor keeps the most recent 7 calendar days."""
        ...

    def option_flat_file_trade_quote_builder(
        self,
        date: Union[str, date, datetime],
    ) -> OptionFlatFileTradeQuoteBuilder:
        """Fluent builder for `option_flat_file_trade_quote`. Chain the optional setters, then call `.list()` (or `.list_async()`) to execute; the returned typed list wrapper exposes `.to_list()` / `.to_arrow()` / `.to_pandas()` / `.to_polars()`."""
        ...

    def option_flat_file_trade_quote_to_path(
        self,
        date: Union[str, date, datetime],
        path: Union[str, PathLike[str]],
        format: Optional[str] = None,
    ) -> str:
        """Write the `option_flat_file_trade_quote` file for `date` to `path` as the terminal writes it, and return the path written.

        ``format`` is ``"csv"`` (the default), ``"json"``, ``"json_legacy"``, ``"ndjson"`` (or ``"jsonl"``) or ``"html"``. The file streams to disk chunk by chunk and takes its name only once complete; a path without an extension gets the format's.
        """
        ...

    def option_flat_file_open_interest(
        self,
        date: Union[str, date, datetime],
        *,
        timeout_ms: Optional[int] = None,
    ) -> OpenInterestTickList:
        """Fetch the open interest of every option contract for one date. The vendor keeps the most recent 7 calendar days."""
        ...

    def option_flat_file_open_interest_builder(
        self,
        date: Union[str, date, datetime],
    ) -> OptionFlatFileOpenInterestBuilder:
        """Fluent builder for `option_flat_file_open_interest`. Chain the optional setters, then call `.list()` (or `.list_async()`) to execute; the returned typed list wrapper exposes `.to_list()` / `.to_arrow()` / `.to_pandas()` / `.to_polars()`."""
        ...

    def option_flat_file_open_interest_to_path(
        self,
        date: Union[str, date, datetime],
        path: Union[str, PathLike[str]],
        format: Optional[str] = None,
    ) -> str:
        """Write the `option_flat_file_open_interest` file for `date` to `path` as the terminal writes it, and return the path written.

        ``format`` is ``"csv"`` (the default), ``"json"``, ``"json_legacy"``, ``"ndjson"`` (or ``"jsonl"``) or ``"html"``. The file streams to disk chunk by chunk and takes its name only once complete; a path without an extension gets the format's.
        """
        ...

    def option_flat_file_quote(
        self,
        date: Union[str, date, datetime],
        *,
        timeout_ms: Optional[int] = None,
    ) -> IntervalQuoteTickList:
        """Fetch every option contract's 1-minute NBBO quote intervals for one date: one row per minute in which a new quote was reported. The vendor keeps the most recent 7 calendar days."""
        ...

    def option_flat_file_quote_builder(
        self,
        date: Union[str, date, datetime],
    ) -> OptionFlatFileQuoteBuilder:
        """Fluent builder for `option_flat_file_quote`. Chain the optional setters, then call `.list()` (or `.list_async()`) to execute; the returned typed list wrapper exposes `.to_list()` / `.to_arrow()` / `.to_pandas()` / `.to_polars()`."""
        ...

    def option_flat_file_quote_to_path(
        self,
        date: Union[str, date, datetime],
        path: Union[str, PathLike[str]],
        format: Optional[str] = None,
    ) -> str:
        """Write the `option_flat_file_quote` file for `date` to `path` as the terminal writes it, and return the path written.

        ``format`` is ``"csv"`` (the default), ``"json"``, ``"json_legacy"``, ``"ndjson"`` (or ``"jsonl"``) or ``"html"``. The file streams to disk chunk by chunk and takes its name only once complete; a path without an extension gets the format's.
        """
        ...

    def index_list_symbols(
        self,
        *,
        timeout_ms: Optional[int] = None,
    ) -> StringList:
        """List all available index symbols.

        A symbol can be defined as a unique identifier for a stock / underlying asset. Common terms also include: root, ticker, and underlying. This endpoint returns all traded index symbols. This endpoint is updated overnight.
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
        current_day_only: Optional[bool] = False,
        timeout_ms: Optional[int] = None,
    ) -> OhlcTickList:
        """Get the latest OHLC snapshot for one or more indices.

        - Retrieves the real-time current day OHLC.
        - Exchanges typically generate a price report every second for popular indices like SPX.

        Defaults (upstream):
        - `current_day_only`: `false`
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
        current_day_only: Optional[bool] = False,
        timeout_ms: Optional[int] = None,
    ) -> PriceTickList:
        """Get the latest price snapshot for one or more indices.

        - Retrieves a real-time last index price.
        - Exchanges typically generate a price report every second for popular indices like SPX.

        Defaults (upstream):
        - `current_day_only`: `false`
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
        current_day_only: Optional[bool] = False,
        timeout_ms: Optional[int] = None,
    ) -> MarketValueTickList:
        """Get the latest market value snapshot for one or more indices.

        - Retrieves a real-time last index market value.
        - Exchanges typically generate a price report every second for popular indices like SPX.

        Defaults (upstream):
        - `current_day_only`: `false`
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
        interval: Optional[Union[str, Interval]] = None,
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
        *,
        date: Optional[Union[str, date, datetime]] = None,
        interval: Optional[Union[str, Interval]] = None,
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

    def index_history_price_builder(
        self,
        symbol: str,
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

    def index_at_time_price_builder(
        self,
        symbol: str,
        start_date: Union[str, date, datetime],
        end_date: Union[str, date, datetime],
        time_of_day: Union[str, time, datetime],
    ) -> IndexAtTimePriceBuilder:
        """Fluent builder for `index_at_time_price`. Chain the optional setters, then call `.list()` (or `.list_async()`) to execute; the returned typed list wrapper exposes `.to_list()` / `.to_arrow()` / `.to_pandas()` / `.to_polars()`."""
        ...

    def index_flat_file_eod(
        self,
        date: Union[str, date, datetime],
        *,
        timeout_ms: Optional[int] = None,
    ) -> EodTickList:
        """Fetch the end-of-day report of every index for one date. The vendor keeps the most recent 7 calendar days."""
        ...

    def index_flat_file_eod_builder(
        self,
        date: Union[str, date, datetime],
    ) -> IndexFlatFileEodBuilder:
        """Fluent builder for `index_flat_file_eod`. Chain the optional setters, then call `.list()` (or `.list_async()`) to execute; the returned typed list wrapper exposes `.to_list()` / `.to_arrow()` / `.to_pandas()` / `.to_polars()`."""
        ...

    def index_flat_file_eod_to_path(
        self,
        date: Union[str, date, datetime],
        path: Union[str, PathLike[str]],
        format: Optional[str] = None,
    ) -> str:
        """Write the `index_flat_file_eod` file for `date` to `path` as the terminal writes it, and return the path written.

        ``format`` is ``"csv"`` (the default), ``"json"``, ``"json_legacy"``, ``"ndjson"`` (or ``"jsonl"``) or ``"html"``. The file streams to disk chunk by chunk and takes its name only once complete; a path without an extension gets the format's.
        """
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
        """Get equity market holidays and early-close days for a year (vendor `year_holidays` endpoint: only non-standard days, not every trading day).

        - Retrieves equity market holidays for a given year
        - Note: Holiday data is available 01/01/2012 through the end of the calendar year that immediately follows the current year
        - *On days when the market closes early at 1:00 PM ET; eligible options will trade until 1:15 PM.
        - **Some NYSE exchanges will continue late trading until 5:00 PM ET on early close days.
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

    def interest_rate_history_eod_builder(
        self,
        symbol: str,
        start_date: Union[str, date, datetime],
        end_date: Union[str, date, datetime],
    ) -> InterestRateHistoryEodBuilder:
        """Fluent builder for `interest_rate_history_eod`. Chain the optional setters, then call `.list()` (or `.list_async()`) to execute; the returned typed list wrapper exposes `.to_list()` / `.to_arrow()` / `.to_pandas()` / `.to_polars()`."""
        ...

# --- END GENERATED MARKET DATA VIEW ---


class _StreamMethods:
    """The methods of :class:`StreamView`, which :class:`AsyncClient`
    forwards. It does not exist at runtime.
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

    def batches(
        self,
        *,
        batch_size: Optional[int] = None,
        linger_ms: Optional[int] = None,
        backpressure: Optional[str] = None,
        capacity: Optional[int] = None,
    ) -> RecordBatchStream:
        """Open a pull-based columnar reader over the live stream.

        Returns a :class:`RecordBatchStream` yielding ``pyarrow.RecordBatch``
        objects. ``backpressure`` selects ``"block"`` (the default; no
        queue-side drops, though a sustained stall can overflow the upstream
        event ring) or ``"drop_oldest"`` (bounded by ``capacity`` buffered
        batches).
        """
        ...

    def stop_streaming(self) -> None:
        """Stop streaming while keeping the market-data client usable.

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

        Raises:
            NetworkError: If streaming has not started.
        """
        ...

    # Metrics + connection observability.
    def dropped_event_count(self) -> int:
        """Count of streaming events dropped because the consumer fell
        behind and the event ring was full, across every session this
        client has run.

        Returns 0 before :meth:`start_streaming`, and the running total
        from then on: :meth:`stop_streaming` and :meth:`reconnect` retire
        the session, and the drops it recorded stay in the total.
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
        :meth:`subscribe`.

        Raises:
            NetworkError: If streaming has not started.
        """
        ...

    def panic_count(self) -> int:
        """Count of user-callback panics caught by the streaming consumer's
        panic boundary, across every session this client has run.

        Each exception raised inside the registered callback is caught,
        reported through the unraisable hook, and counted here. A session
        that has been stopped keeps the faults it recorded.
        """
        ...


@final
class StreamView(_StreamMethods):
    """Real-time-streaming sub-namespace returned by :attr:`Client.stream`.

    Owns the streaming lifecycle, subscription management, and feed
    diagnostics for the unified client. Shares the parent client's
    callback registration, so starting / stopping / reconnecting through
    this view drives the same session the client manages.
    """


@final
class Client:
    """Unified client for historical data and real-time streaming.

    Connects to ThetaData at construction (a single authentication
    covers both market-data access and streaming). Market-data endpoints
    are available immediately; real-time streaming starts on demand via
    :meth:`start_streaming`. This is the recommended entry point.
    """

    # pyo3 routes the constructor through `__new__`; this stub mirrors the
    # exact runtime `#[pyo3(signature = ...)]` so stubtest validates the
    # parameter list against the compiled extension (the `Client.__new__`
    # allowlist entry is intentionally dropped, so a renamed / removed
    # constructor kwarg is caught). The documented `__init__` twin below is
    # what `help()` / IDEs / the rendered docs surface; stubtest flags the
    # documented-`__init__`-vs-real-`__new__` swap, which the
    # `Client.__init__` allowlist entry covers.
    def __new__(
        cls,
        credentials: Optional[Credentials] = None,
        config: Optional[Config] = None,
        *,
        api_key: Optional[str] = None,
        email: Optional[str] = None,
        password: Optional[str] = None,
        market_data_type: Optional[str] = None,
        streaming_type: Optional[str] = None,
    ) -> Client: ...

    def __init__(
        self,
        credentials: Optional[Credentials] = None,
        config: Optional[Config] = None,
        *,
        api_key: Optional[str] = None,
        email: Optional[str] = None,
        password: Optional[str] = None,
        market_data_type: Optional[str] = None,
        streaming_type: Optional[str] = None,
    ) -> None:
        """Connect to ThetaData and open the market-data channel.

        The API key is first-class and directly passed:
        ``Client(api_key="td1_...")`` (optionally with
        ``market_data_type="STAGE"``). Email + password is the parallel inline
        form: ``Client(email="user@example.com", password="secret")``. The
        lower-level typed path stays a superset:
        ``Client(credentials=creds, config=cfg)`` (and the positional
        ``Client(creds, config)``) still work.

        Exactly one authentication argument must be supplied — ``api_key``,
        the ``email`` + ``password`` pair, or ``credentials``. Passing
        none, or two different ones, raises ``ConfigError`` before any
        network round-trip. ``market_data_type`` (``"PROD"`` / ``"STAGE"``,
        case-insensitive) selects the market-data environment and
        ``streaming_type`` (``"PROD"`` / ``"STAGE"`` / ``"DEV"``, case-insensitive) the
        streaming environment, independently; ``config`` supplies a full
        :class:`Config` whose environments and hosts win. Streaming is not
        started. The call is interruptible with ``Ctrl+C`` if the
        handshake stalls.

        Args:
            credentials: Pre-built account credentials.
            config: Connection configuration (e.g. ``Config.production()``).
            api_key: Inline API key.
            email: Inline account email, paired with ``password``.
            password: Inline account password, paired with ``email``.
            market_data_type: Market-data environment selector (``"PROD"`` / ``"STAGE"``).
            streaming_type: Streaming environment selector (``"PROD"`` / ``"STAGE"`` / ``"DEV"``).

        Raises:
            ConfigError: If no authentication argument is given, two
                different ones are given, or ``market_data_type`` / ``streaming_type``
                is invalid.
            ThetaDataError: If authentication or the connection fails.
        """
        ...

    @staticmethod
    def from_env(
        config: Optional[Config] = None,
        *,
        market_data_type: Optional[str] = None,
        streaming_type: Optional[str] = None,
    ) -> Client:
        """Connect with the API key sourced strictly from the environment.

        Reads ``THETADATA_API_KEY`` and connects. Strict, with no file
        fallback: an unset or whitespace-only ``THETADATA_API_KEY`` raises
        ``InvalidParameterError`` before any network round-trip. For the
        env-or-file convenience read a ``.env`` file with :meth:`from_dotenv`
        instead.

        Args:
            config: Connection configuration; defaults to
                ``Config.production()`` when omitted.
            market_data_type: Market-data environment selector (``"PROD"`` / ``"STAGE"``).
            streaming_type: Streaming environment selector (``"PROD"`` / ``"STAGE"`` / ``"DEV"``).

        Returns:
            A connected :class:`Client`.

        Raises:
            InvalidParameterError: If ``THETADATA_API_KEY`` is unset or empty.
            ConfigError: If ``market_data_type`` / ``streaming_type`` is invalid.
            ThetaDataError: If the connection fails.
        """
        ...

    @staticmethod
    def from_dotenv(
        path: str,
        config: Optional[Config] = None,
        *,
        market_data_type: Optional[str] = None,
        streaming_type: Optional[str] = None,
    ) -> Client:
        """Connect with the credential (and optionally the environments)
        sourced from a ``.env``-format file.

        ``THETADATA_API_KEY`` selects an API key; otherwise
        ``THETADATA_EMAIL`` + ``THETADATA_PASSWORD`` build email +
        password credentials. When ``config`` is omitted the same file is
        also read for ``THETADATA_MARKET_DATA_TYPE`` and ``THETADATA_STREAMING_TYPE``,
        so one ``.env`` can carry both the credential and the
        environments. An explicit ``config``, ``market_data_type``, or
        ``streaming_type`` overrides the file's environment selection.

        Args:
            path: Path to the ``.env`` file to read.
            config: Connection configuration overriding the file's
                environment selection.
            market_data_type: Market-data environment selector (``"PROD"`` /
                ``"STAGE"``) overriding the file's selection.
            streaming_type: Streaming environment selector (``"PROD"`` /
                ``"DEV"``) overriding the file's selection.

        Returns:
            A connected :class:`Client`.

        Raises:
            ThetaDataError: If the file cannot be read or defines none of
                the recognized keys, or the connection fails.
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
    def market_data(self) -> MarketDataView:
        """Market-data sub-namespace.

        Every historical / list / snapshot / at-time endpoint is reached
        through this view, e.g. ``client.market_data.stock_eod(...)`` and
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

    def close(self) -> None:
        """Deterministically close the client.

        Stops streaming if it is live (idempotent), waits for the streaming
        consumer thread to finish firing the registered callback, and drops
        the stored callback. Safe to call more than once and safe on a
        client that only ran market-data queries. Prefer the context manager
        (``with Client(...) as c:``) so this runs on block exit.
        """
        ...

    def __enter__(self) -> "Client": ...
    def __exit__(
        self,
        exc_type: Optional[type] = None,
        exc_value: Optional[BaseException] = None,
        traceback: Optional[object] = None,
    ) -> bool: ...
    async def __aenter__(self) -> "Client": ...
    async def __aexit__(
        self,
        exc_type: Optional[type] = None,
        exc_value: Optional[BaseException] = None,
        traceback: Optional[object] = None,
    ) -> bool: ...

    def __repr__(self) -> str:
        """Return a representation including market-data and streaming state."""
        ...


@final
class AsyncClient(_AsyncMarketData, _StreamMethods):
    """Async client exposing ``await``-based market-data methods.

    Each market-data endpoint is available as the ``*_async`` coroutine
    of :attr:`Client.market_data` (e.g.
    ``await client.stock_history_eod_async(...)``). The streaming methods
    of :attr:`Client.stream` (``start_streaming``, ``subscribe`` and the
    rest) and :meth:`Client.streaming` are available directly. Accessing
    the synchronous market-data methods on this class raises
    ``AttributeError``; use :class:`Client` for those.
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
        """Return a representation including market-data and streaming state."""
        ...

    async def close(self) -> None:
        """Deterministically close the async client.

        Stops streaming if live, drains the consumer, and releases the
        callback. Awaitable: the blocking stop and drain run on a worker
        thread, never on the event loop. Prefer
        ``async with await AsyncClient.connect(...) as c:`` so this runs on
        scope exit.
        """
        ...

    async def __aenter__(self) -> "AsyncClient": ...
    async def __aexit__(
        self,
        exc_type: Optional[type] = None,
        exc_value: Optional[BaseException] = None,
        traceback: Optional[object] = None,
    ) -> bool: ...

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


@final
class StreamingClient:
    """Streaming-only client — opens the real-time feed and never the
    market-data channel."""

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

        Raises:
            NetworkError: If streaming has not started.
        """
        ...

    def active_full_subscriptions(self) -> List[Subscription]:
        """Return a snapshot of the active full-stream subscriptions.

        Raises:
            NetworkError: If streaming has not started.
        """
        ...

    def dropped_event_count(self) -> int:
        """Count of streaming events dropped on a full event ring, across
        every session this client has run.

        Returns 0 before :meth:`start_streaming`, and the running total
        from then on, including after :meth:`stop_streaming`.
        """
        ...

    def panic_count(self) -> int:
        """Count of exceptions raised by the user callback, across every
        session this client has run, including sessions already stopped."""
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

    def __enter__(self) -> "StreamingClient": ...
    def __exit__(
        self,
        exc_type: Optional[type] = None,
        exc_value: Optional[BaseException] = None,
        traceback: Optional[object] = None,
    ) -> bool:
        """Stop streaming and drain on scope exit."""
        ...
    async def __aenter__(self) -> "StreamingClient": ...
    async def __aexit__(
        self,
        exc_type: Optional[type] = None,
        exc_value: Optional[BaseException] = None,
        traceback: Optional[object] = None,
    ) -> bool:
        """Stop streaming and drain on scope exit."""
        ...


@final
class MarketDataClient:
    """Market-data-only client — the streaming surface is blocked.

    Exposes the same market-data endpoints as :class:`Client`
    and never opens the real-time feed. Any streaming method (e.g.
    ``start_streaming`` / ``subscribe``) raises ``AttributeError``; use
    :class:`StreamingClient` or :class:`Client` for streaming.
    """

    def __init__(self, creds: Credentials, config: Config) -> None:
        """Connect a market-data-only client with ``creds`` and ``config``.

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
    ) -> MarketDataClient:
        """Construct a market-data-only client from a credentials file.

        Args:
            path: Path to a two-line credentials file.
            config: Connection configuration; defaults to
                ``Config.production()`` when omitted.

        Returns:
            A connected :class:`MarketDataClient`.

        Raises:
            ThetaDataError: If the file cannot be read or the connection
                fails.
        """
        ...

    def __repr__(self) -> str:
        """Return a representation of the market-data client."""
        ...

    def close(self) -> None:
        """Deterministically close the market-data client.

        The market-data-only surface never opens streaming, so this releases
        the gRPC channel pool with no drain. Idempotent. Prefer the context
        manager (``with MarketDataClient(...) as c:``) so this runs on block
        exit.
        """
        ...

    def __enter__(self) -> "MarketDataClient": ...
    def __exit__(
        self,
        exc_type: Optional[type] = None,
        exc_value: Optional[BaseException] = None,
        traceback: Optional[object] = None,
    ) -> bool: ...
    async def __aenter__(self) -> "MarketDataClient": ...
    async def __aexit__(
        self,
        exc_type: Optional[type] = None,
        exc_value: Optional[BaseException] = None,
        traceback: Optional[object] = None,
    ) -> bool: ...

    def __getattr__(self, name: str) -> Any:
        """Resolve a market-data method.

        Raises:
            AttributeError: If ``name`` is a streaming method (blocked on
                this client).
        """
        ...


# ─────────────────────────────────────────────────────────────────────
# Streaming context managers + iterator
# ─────────────────────────────────────────────────────────────────────


@final
class RecordBatchStream:
    """Pull-based columnar reader returned by :meth:`StreamView.batches`.

    Synchronous and asynchronous iterable, and a context manager on both
    protocols; each iteration yields a ``pyarrow.RecordBatch``. Closing the
    reader (explicitly, or on block exit) releases the underlying stream.
    """

    def __iter__(self) -> RecordBatchStream:
        """Return ``self`` as a synchronous iterator."""
        ...

    def __next__(self) -> Any:
        """Return the next ``pyarrow.RecordBatch``; raise ``StopIteration`` at end."""
        ...

    def __aiter__(self) -> RecordBatchStream:
        """Return ``self`` as an asynchronous iterator."""
        ...

    async def __anext__(self) -> Any:
        """Return the next ``pyarrow.RecordBatch``; raise ``StopAsyncIteration`` at end."""
        ...

    def __enter__(self) -> RecordBatchStream:
        """Return the reader for use as a context manager."""
        ...

    def __exit__(
        self,
        exc_type: Optional[Type[BaseException]],
        exc_value: Optional[BaseException],
        traceback: Optional[Any],
    ) -> bool:
        """Close the reader; never suppresses exceptions."""
        ...

    async def __aenter__(self) -> RecordBatchStream:
        """Return the reader for use as an async context manager."""
        ...

    async def __aexit__(
        self,
        exc_type: Optional[Type[BaseException]],
        exc_value: Optional[BaseException],
        traceback: Optional[Any],
    ) -> bool:
        """Close the reader; never suppresses exceptions."""
        ...

    def close(self) -> None:
        """Release the underlying stream. Idempotent."""
        ...

    @property
    def schema(self) -> Any:
        """The fixed ``pyarrow.Schema`` every batch carries."""
        ...

    @property
    def dropped(self) -> int:
        """Count of batches dropped under back-pressure."""
        ...


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

    ``chunks`` are the wire-frame byte buffers for one market-data
    response in order; ``endpoint`` selects the decoder and names a
    market-data endpoint that returns typed rows, which is every endpoint
    except those returning a list of strings. Returns the endpoint's typed
    ``<Tick>List`` wrapper.
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
