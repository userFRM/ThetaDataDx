"""Type stubs for the `thetadatadx` native extension.

Hand-written stubs cover the load-bearing public surface:
client pyclasses, credentials, config, the fluent
`Contract` / `Subscription` / `SecType` value types, FPSS event
classes, and streaming context managers. Generator-emitted
historical builders and typed `<Tick>List` wrappers fall through
the module-level ``__getattr__`` to ``Any`` — there are 100+ of
them and they all share an identical structural shape (a fluent
builder chained to a typed-list terminal), so a hand-written
typed mirror would be high-maintenance noise.

Mypy / pyright pick this up via the ``py.typed`` marker (PEP 561)
shipped alongside this file.

Return types on PyO3-exported callables are NOT verified by the
stubtest gate: a compiled extension presents an opaque ``builtin``
descriptor whose annotations stubtest cannot read, so it never
compares the declared return against the concrete runtime object
(e.g. it cannot tell ``tuple[float, float]`` from ``float``). The
return annotation on every function and method below is therefore
hand-maintained against the live runtime and must be kept correct
by hand when the binding changes — the gate will not flag a wrong
one. The ``tests/test_typed_surface.py`` guard re-checks the
non-trivial offline-constructible returns at runtime to catch
drift the gate cannot.
"""

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

# ─────────────────────────────────────────────────────────────────────
# Credentials + Config
# ─────────────────────────────────────────────────────────────────────


@final
class Credentials:
    """ThetaData Nexus credentials (email + password)."""

    def __init__(self, email: str, password: str) -> None: ...
    @staticmethod
    def from_file(path: str) -> Credentials: ...
    def __repr__(self) -> str: ...


@final
class Config:
    """Connection configuration: MDDS host / FPSS hosts / reconnect policy."""

    @staticmethod
    def production() -> Config: ...
    @staticmethod
    def dev() -> Config: ...
    @staticmethod
    def stage() -> Config: ...
    # MDDS host / port.
    mdds_host: str
    mdds_port: int
    # MDDS pool sizing. `concurrent_requests = 0` auto-detects from
    # the tier; explicit values above the tier cap are clamped at
    # connect time with a warn.
    concurrent_requests: int
    # Byte ceiling above which a buffered (non-`.stream()`) historical
    # response emits a Rust-side `tracing::warn!` pointing the caller
    # at the streaming surface. `0` disables the warning; the default
    # is `100 * 1024 * 1024` (100 MiB). The data is still delivered.
    warn_on_buffered_threshold_bytes: int
    # Reconnect tunables. `reconnect_max_attempts` (default 30) and
    # `reconnect_max_elapsed_secs` (default 300; 0 disables) bound a
    # consecutive-reconnect sequence on the generic-transient class;
    # the rate-limited class rides `reconnect_max_rate_limited_attempts`
    # alone and `reconnect_max_server_restart_attempts` (default 60)
    # budgets pool bounces.
    reconnect_policy: str
    reconnect_max_attempts: int
    reconnect_max_rate_limited_attempts: int
    reconnect_max_server_restart_attempts: int
    reconnect_max_elapsed_secs: int
    reconnect_stable_window_secs: int
    # Reconnect cadence (ms) per failure class. `wait_ms` (default 250)
    # is the initial delay of the generic-transient exponential ladder,
    # doubling up to `wait_max_ms` (default 30_000);
    # `wait_rate_limited_ms` (default 130_000) is the TooManyRequests
    # floor; `wait_server_restart_ms` (default 5_000) is the flat
    # ServerRestarting cadence. Every delay is jittered per
    # `reconnect_jitter` ("full" default / "equal" / "decorrelated" /
    # "none").
    reconnect_wait_ms: int
    reconnect_wait_max_ms: int
    reconnect_wait_rate_limited_ms: int
    reconnect_wait_server_restart_ms: int
    reconnect_jitter: Literal["full", "equal", "decorrelated", "none"]
    # Subscription replay pacing after auto-reconnect: frames per burst
    # (default 50, minimum 1) and the jittered pause between bursts in
    # ms (default 5; 0 removes the pause).
    reconnect_replay_burst_size: int
    reconnect_replay_pace_ms: int
    # Custom reconnect policy: a callable `(reason: int, attempt: int)
    # -> Optional[int]` returning the reconnect delay in ms, or `None`
    # to stop (the stream then emits the terminal ReconnectsExhausted
    # event). Runs on the streaming I/O thread; permanent disconnect
    # reasons never reach it. Assign `None` to restore the Auto policy.
    # Write-only: reading the configured callable back is not
    # supported (`reconnect_policy` reports "custom").
    reconnect_callback: Optional[Callable[[int, int], Optional[int]]]
    # Async worker-thread count for embedded runtimes. `None` defers to
    # the default sizing; `int` (including `0`, which clamps to `1`)
    # pins worker count.
    worker_threads: Optional[int]
    # RetryPolicy fields — per-field access on `DirectConfig.retry`.
    # Defaults: `initial=250ms`, `max=30s`, `attempts=20`,
    # `max_elapsed_secs=300` (0 disables the wall-clock envelope),
    # `jitter=True`. Methods `delay_for_attempt` / `capped_backoff`
    # stay Rust-only.
    retry_initial_delay_ms: int
    retry_max_delay_ms: int
    retry_max_attempts: int
    retry_max_elapsed_secs: int
    retry_jitter: bool
    # FlatFilesConfig fields — per-field access on
    # `DirectConfig.flatfiles`. Tunes the legacy flatfile driver's
    # retry loop. Defaults: `max_attempts=10` (validated 1..=100),
    # `initial_backoff_secs=1`, `max_backoff_secs=30`, `jitter=True`.
    flatfiles_max_attempts: int
    flatfiles_initial_backoff_secs: int
    flatfiles_max_backoff_secs: int
    flatfiles_jitter: bool
    # AuthConfig fields — per-field access on `DirectConfig.auth`.
    # `nexus_url` defaults to the upstream production endpoint;
    # `client_type` defaults to `"rust-thetadatadx"`.
    nexus_url: str
    client_type: str
    # MetricsConfig field — Prometheus exporter port on
    # `DirectConfig.metrics`. `None` (the default) leaves the exporter
    # disabled even when the `metrics-prometheus` feature is compiled
    # in; an `int` binds an HTTP listener on `0.0.0.0:<port>`. The
    # setter raises ValueError for values outside `0..=65535`.
    metrics_port: Optional[int]
    # FPSS tunables. `fpss_timeout_ms` (default 3_000) is the
    # no-frames deadline; `fpss_data_watchdog_ms` (default 30_000; 0
    # disables) is the hard wall-clock backstop above it. The keepalive
    # trio arms kernel-side half-open detection (defaults 5 s idle /
    # 2 s interval / 2 probes). `fpss_host_selection` is "shuffled"
    # (fault-domain-aware per-client shuffle, seedable via
    # `fpss_host_shuffle_seed`) or "fixed_order". `fpss_ring_size`
    # must be a power of two >= 64.
    fpss_timeout_ms: int
    fpss_connect_timeout_ms: int
    fpss_ping_interval_ms: int
    fpss_ring_size: int
    fpss_io_read_slice_ms: int
    fpss_data_watchdog_ms: int
    fpss_keepalive_idle_secs: int
    fpss_keepalive_interval_secs: int
    fpss_keepalive_retries: int
    fpss_host_selection: Literal["shuffled", "fixed_order"]
    fpss_host_shuffle_seed: Optional[int]
    derive_ohlcvc: bool
    # Streaming write-flush policy. `"batched"` (default) flushes on the
    # PING heartbeat (~100 ms); `"immediate"` flushes after every wire
    # write. Setter accepts the same two strings case-insensitively and
    # raises ValueError otherwise.
    flush_mode: Literal["batched", "immediate"]

    def __repr__(self) -> str: ...


# ─────────────────────────────────────────────────────────────────────
# Fluent: Contract / Subscription / SecType
# ─────────────────────────────────────────────────────────────────────


@final
class Contract:
    """Per-contract identity (stock or option) for FPSS subscriptions.

    ``strike`` is the price in dollars on both sides of the builder:
    ``option(strike=550)``, ``option(strike=550.0)``, and
    ``option(strike="550")`` are equivalent, and the ``strike``
    property reads the same dollar value back.
    """

    @staticmethod
    def stock(symbol: str) -> Contract: ...
    @staticmethod
    def option(
        symbol: str,
        *,
        expiration: str,
        strike: float | int | str,
        right: str,
    ) -> Contract: ...
    @property
    def symbol(self) -> str: ...
    @property
    def sec_type(self) -> str: ...
    @property
    def expiration(self) -> Optional[int]: ...
    @property
    def strike(self) -> Optional[float]: ...
    @property
    def right(self) -> Optional[str]: ...

    def quote(self) -> Subscription: ...
    def trade(self) -> Subscription: ...
    def open_interest(self) -> Subscription: ...
    def market_value(self) -> Subscription: ...

    def __repr__(self) -> str: ...
    def __eq__(self, other: object) -> bool: ...


@final
class ContractRef:
    """Read-only contract identifier surfaced on every FPSS event.

    Distinct from the fluent `Contract` builder — `ContractRef` is what
    `event.contract` returns inside a streaming callback, with the
    resolved `symbol`, `sec_type`, `expiration`, `right`, and `strike`
    (dollars — the same unit historical rows carry under the same
    name). The fluent `Contract` (above) is the one users instantiate
    to subscribe.
    """

    @property
    def symbol(self) -> str: ...
    @property
    def sec_type(self) -> str: ...
    @property
    def expiration(self) -> Optional[int]: ...
    @property
    def right(self) -> Optional[str]: ...
    @property
    def strike(self) -> Optional[float]: ...

    def __repr__(self) -> str: ...


@final
class SecType:
    """Security type — `STOCK` / `OPTION` / `INDEX` / `RATE`."""

    STOCK: SecType
    OPTION: SecType
    INDEX: SecType
    RATE: SecType

    def full_trades(self) -> Subscription: ...
    def full_open_interest(self) -> Subscription: ...

    def __repr__(self) -> str: ...
    def __str__(self) -> str: ...
    def __eq__(self, other: object) -> bool: ...
    def __hash__(self) -> int: ...


@final
class Subscription:
    """Typed market-data subscription (per-contract or full-stream)."""

    @property
    def kind(self) -> str: ...
    """`"quote"` / `"trade"` / `"open_interest"` / `"market_value"` / `"full_trades"` / `"full_open_interest"`."""

    @property
    def is_full(self) -> bool: ...
    @property
    def contract(self) -> Optional[Contract]: ...
    @property
    def sec_type(self) -> Optional[SecType]: ...

    def __repr__(self) -> str: ...


# ─────────────────────────────────────────────────────────────────────
# FPSS event classes — emitted to the streaming callback
#
# Every field below was extracted from the `#[pyo3(get)]` declarations
# on the generated `_generated/fpss_event_classes.rs` plus
# subsequent surface additions. Updating this stub without touching
# the matching pyclass attribute (or vice versa) is caught by
# `python -m mypy.stubtest thetadatadx --ignore-missing-stub`.
# ─────────────────────────────────────────────────────────────────────


@final
class Quote:
    """FPSS Quote tick. Mirrors `FpssData::Quote`."""

    contract: ContractRef
    ms_of_day: int
    bid_size: int
    bid_exchange: int
    bid: float
    bid_condition: int
    ask_size: int
    ask_exchange: int
    ask: float
    ask_condition: int
    date: int
    received_at_ns: int

    @property
    def kind(self) -> str: ...
    def __repr__(self) -> str: ...


@final
class Trade:
    """FPSS Trade tick. Mirrors `FpssData::Trade`."""

    contract: ContractRef
    ms_of_day: int
    sequence: int
    ext_condition1: int
    ext_condition2: int
    ext_condition3: int
    ext_condition4: int
    condition: int
    size: int
    exchange: int
    price: float
    condition_flags: int
    price_flags: int
    volume_type: int
    records_back: int
    date: int
    received_at_ns: int

    @property
    def kind(self) -> str: ...
    def __repr__(self) -> str: ...


@final
class OpenInterest:
    """FPSS OpenInterest tick. Mirrors `FpssData::OpenInterest`."""

    contract: ContractRef
    ms_of_day: int
    open_interest: int
    date: int
    received_at_ns: int

    @property
    def kind(self) -> str: ...
    def __repr__(self) -> str: ...


@final
class Ohlcvc:
    """FPSS OHLCVC bar (derived in the SDK when `Config.derive_ohlcvc=True`)."""

    contract: ContractRef
    ms_of_day: int
    open: float
    high: float
    low: float
    close: float
    volume: int
    count: int
    date: int
    received_at_ns: int

    @property
    def kind(self) -> str: ...
    def __repr__(self) -> str: ...


@final
class ContractAssigned:
    """FPSS server assigned a contract id (`FpssControl::ContractAssigned`)."""

    id: int
    contract: ContractRef

    @property
    def kind(self) -> str: ...
    def __repr__(self) -> str: ...


@final
class Connected:
    """FPSS server connection ack (wire code 4)."""

    @property
    def kind(self) -> str: ...
    def __repr__(self) -> str: ...


@final
class Disconnected:
    """FPSS server disconnected the client (wire code 12)."""

    reason: int

    @property
    def kind(self) -> str: ...

    @property
    def reason_name(self) -> str:
        """Resolved `RemoveReason` variant name (e.g. `"TooManyRequests"`)."""
        ...

    def __repr__(self) -> str: ...


@final
class ParseError:
    """FPSS protocol-level parse error event. Mirrors
    `FpssControl::Error` on the Rust core. Named ``ParseError`` so it
    never collides with the :class:`Error` exception class.
    """

    message: str

    @property
    def kind(self) -> str: ...
    def __repr__(self) -> str: ...


@final
class LoginSuccess:
    """FPSS login-success ack. Carries the server-side permissions string."""

    permissions: str

    @property
    def kind(self) -> str: ...
    def __repr__(self) -> str: ...


@final
class MarketClose:
    """FPSS market-close signal (wire code 32). Carries no payload."""

    @property
    def kind(self) -> str: ...
    def __repr__(self) -> str: ...


@final
class MarketOpen:
    """FPSS market-open signal (wire code 30). Carries no payload."""

    @property
    def kind(self) -> str: ...
    def __repr__(self) -> str: ...


@final
class Ping:
    """FPSS server heartbeat (wire code 10)."""

    payload: bytes

    @property
    def kind(self) -> str: ...
    def __repr__(self) -> str: ...


@final
class Reconnected:
    """FPSS auto-reconnect succeeded — connection is live again."""

    @property
    def kind(self) -> str: ...
    def __repr__(self) -> str: ...


@final
class ReconnectedServer:
    """FPSS server-side reconnect ack (wire code 13)."""

    @property
    def kind(self) -> str: ...
    def __repr__(self) -> str: ...


@final
class Reconnecting:
    """FPSS auto-reconnect is about to attempt reconnection."""

    reason: int
    attempt: int
    delay_ms: int

    @property
    def kind(self) -> str: ...

    @property
    def reason_name(self) -> str:
        """Resolved `RemoveReason` variant name."""
        ...

    def __repr__(self) -> str: ...


@final
class ReconnectsExhausted:
    """Auto-reconnect stopped without a user-initiated shutdown —
    terminal for the session. Emitted on budget / wall-clock-envelope
    exhaustion, a permanent disconnect reason, a manual policy, or a
    custom policy returning ``None``. ``attempts`` is the number of
    consecutive reconnect attempts consumed (0 when no reconnect was
    attempted)."""

    reason: int
    attempts: int

    @property
    def kind(self) -> str: ...

    @property
    def reason_name(self) -> str:
        """Resolved `RemoveReason` variant name."""
        ...

    def __repr__(self) -> str: ...


@final
class ReqResponse:
    """FPSS subscription response (wire code 40)."""

    req_id: int
    result: int

    @property
    def kind(self) -> str: ...
    def __repr__(self) -> str: ...


@final
class Restart:
    """FPSS server stream restart (wire code 31)."""

    @property
    def kind(self) -> str: ...
    def __repr__(self) -> str: ...


@final
class ServerError:
    """FPSS server-error message (wire code 11)."""

    message: str

    @property
    def kind(self) -> str: ...
    def __repr__(self) -> str: ...


@final
class UnknownControl:
    """FPSS control variant the SDK does not yet recognise."""

    @property
    def kind(self) -> str: ...
    def __repr__(self) -> str: ...


@final
class UnknownFrame:
    """FPSS server sent a frame with an unrecognised wire code."""

    code: int
    payload: bytes

    @property
    def kind(self) -> str: ...
    def __repr__(self) -> str: ...


# Discriminated union of every FPSS event class fired through the
# streaming callback. The dispatcher fires one of these per event;
# narrow via `match event: case Quote(): ...`.
FpssEvent = Any  # opaque to mypy; runtime narrowing via `match` / `isinstance`


# ─────────────────────────────────────────────────────────────────────
# Streaming clients
# ─────────────────────────────────────────────────────────────────────

EventCallback = Callable[[Any], None]


@final
class ThetaDataDxClient:
    """Unified client: opens MDDS + Nexus at construction, FPSS on demand.

    This stub does not carry a per-class ``__getattr__`` fallback.
    Every public method below is hand-listed so a new generator-emitted
    method shows up as a stubtest failure until the stub is
    regenerated. The module-level ``__getattr__`` at the bottom of
    this file routes the catch-all generator-emitted historical
    builders / list classes / endpoint factories without masking
    method-level drift on the load-bearing pyclasses.
    """

    def __init__(self, creds: Credentials, config: Config) -> None: ...
    @staticmethod
    def from_file(
        path: str,
        config: Optional[Config] = None,
    ) -> ThetaDataDxClient: ...

    # Streaming lifecycle.
    def start_streaming(self, callback: EventCallback) -> None: ...
    def is_streaming(self) -> bool: ...
    def stop_streaming(self) -> None: ...
    def shutdown(self) -> None: ...
    def reconnect(self) -> None: ...
    def await_drain(self, timeout_ms: int) -> bool: ...

    # Subscriptions.
    def subscribe(self, sub: Subscription) -> None: ...
    def subscribe_many(self, subs: List[Subscription]) -> None: ...
    def unsubscribe(self, sub: Subscription) -> None: ...
    def unsubscribe_many(self, subs: List[Subscription]) -> None: ...
    def active_subscriptions(self) -> List[Subscription]: ...
    def active_full_subscriptions(self) -> List[Subscription]: ...

    # Metrics + connection observability.
    def dropped_event_count(self) -> int: ...
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
    def last_event_received_at_unix_nanos(self) -> int: ...
    def last_connected_addr(self) -> Optional[str]:
        """``host:port`` of the live streaming server, following the
        session across auto-reconnects."""
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
    def streaming(self, callback: EventCallback) -> StreamingSession: ...

    # FLATFILES namespace getter + direct-to-disk helper.
    @property
    def flat_files(self) -> FlatFilesNamespace: ...
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

    def __repr__(self) -> str: ...


@final
class AsyncThetaDataDxClient:
    """Async surface: ``*_async`` historical methods plus streaming helpers.

    AsyncThetaDataDxClient is a PURE PROXY class — its public surface
    is dynamic, dispatched through ``__getattr__`` against an inner
    :class:`ThetaDataDxClient`. The only physical methods on this
    pyclass are the constructor, ``from_file``, ``__repr__``, and
    ``__getattr__`` itself. Every other attribute resolves dynamically:

      - ``*_async`` historical methods → 60+ generator-emitted async
        terminals on the inner unified client.
      - Streaming lifecycle (``start_streaming`` / ``stop_streaming``
        / ``subscribe`` etc) → reaches the sync surface on the inner
        client; documented via :data:`ALLOWED_UNIFIED_PROXY_METHODS`
        in the binding source.

    The per-class ``__getattr__`` stub below is retained
    intentionally: hand-stubbing every proxied attribute would
    duplicate the inner :class:`ThetaDataDxClient` stub and drift on
    every endpoint addition. The compile-time assertion in the
    Rust binding (``const _:() = { ... }`` on
    ``ALLOWED_UNIFIED_PROXY_METHODS``) pins the safelisted names so a
    confused-deputy proxy promise (e.g. ``is_authenticated`` on
    AsyncThetaDataDxClient when it only exists on ``FpssClient``) is
    a build failure rather than a runtime ``AttributeError``.
    """

    def __init__(self, creds: Credentials, config: Config) -> None: ...
    @staticmethod
    def from_file(
        path: str,
        config: Optional[Config] = None,
    ) -> AsyncThetaDataDxClient: ...

    def __repr__(self) -> str: ...
    def __getattr__(self, name: str) -> Any: ...


@final
class FpssClient:
    """Standalone FPSS-only streaming client — never opens MDDS / Nexus."""

    def __init__(self, creds: Credentials, config: Config) -> None: ...
    @staticmethod
    def from_file(
        path: str,
        config: Optional[Config] = None,
    ) -> FpssClient: ...

    def start_streaming(self, callback: EventCallback) -> None: ...
    def is_streaming(self) -> bool: ...
    def is_authenticated(self) -> bool: ...
    def stop_streaming(self) -> None: ...
    def shutdown(self) -> None: ...
    def reconnect(self) -> None: ...
    def await_drain(self, timeout_ms: int) -> bool: ...

    def subscribe(self, sub: Subscription) -> None: ...
    def subscribe_many(self, subs: List[Subscription]) -> None: ...
    def unsubscribe(self, sub: Subscription) -> None: ...
    def unsubscribe_many(self, subs: List[Subscription]) -> None: ...
    def active_subscriptions(self) -> List[Subscription]: ...
    def active_full_subscriptions(self) -> List[Subscription]: ...

    def dropped_event_count(self) -> int: ...
    def panic_count(self) -> int: ...
    def ring_occupancy(self) -> int: ...
    def ring_capacity(self) -> int: ...
    def millis_since_last_event(self) -> Optional[int]: ...
    def last_event_received_at_unix_nanos(self) -> int: ...
    def last_connected_addr(self) -> Optional[str]: ...

    def streaming(self, callback: EventCallback) -> StreamingSession: ...

    def __repr__(self) -> str: ...


@final
class MddsClient:
    """Standalone MDDS-only historical client — FPSS surface is blocked.

    Like :class:`AsyncThetaDataDxClient`, ``MddsClient`` is a PURE
    PROXY class — its public surface is dynamic, dispatched through
    ``__getattr__`` against an inner :class:`ThetaDataDxClient`.
    Every FPSS-touching method name (see
    ``mdds_client::FPSS_TOUCHING_METHODS`` in the binding source +
    the matching ``BLOCKED_FPSS_METHODS`` mirror in
    ``tests/test_standalone_clients.py``) raises ``AttributeError``;
    every other attribute reaches the unified client transparently.

    The per-class ``__getattr__`` stub below is retained
    intentionally — hand-stubbing 60+ generator-emitted historical
    builders would duplicate the inner :class:`ThetaDataDxClient`
    stub and drift on every endpoint addition. The
    runtime block-list invariant is enforced via an offline pytest
    coverage check (``test_mdds_client_block_list_offline``) plus a
    compile-time guard in the Rust source.
    """

    def __init__(self, creds: Credentials, config: Config) -> None: ...
    @staticmethod
    def from_file(
        path: str,
        config: Optional[Config] = None,
    ) -> MddsClient: ...
    def __repr__(self) -> str: ...
    def __getattr__(self, name: str) -> Any: ...


# ─────────────────────────────────────────────────────────────────────
# Streaming context managers + iterator
# ─────────────────────────────────────────────────────────────────────


@final
class StreamingSession:
    """Context manager for push-callback FPSS streaming.

    Acquired via :py:meth:`ThetaDataDxClient.streaming` /
    :py:meth:`FpssClient.streaming`. StreamingSession is a PURE
    PROXY class — its public surface is dynamic, dispatched through
    ``__getattr__`` against the bound client. Only the context-
    manager dunders and ``__getattr__`` itself are physical methods
    on the pyclass; subscription / lifecycle methods reach the bound
    client transparently.
    """

    def __enter__(self) -> StreamingSession: ...
    def __exit__(
        self,
        exc_type: Optional[Type[BaseException]],
        exc_value: Optional[BaseException],
        traceback: Optional[Any],
    ) -> bool: ...
    def __getattr__(self, name: str) -> Any: ...


@final
class FlatFileRowList:
    """Typed list of FLATFILES rows. One row per `(symbol, date, ...)`."""

    def __len__(self) -> int: ...
    def to_list(self) -> List[Any]: ...
    def to_arrow(self) -> Any: ...
    def to_pandas(self) -> Any: ...
    def to_polars(self) -> Any: ...


@final
class FlatFilesNamespace:
    """Namespace accessor returned by :py:attr:`ThetaDataDxClient.flat_files`.

    Each method maps one ``(SecType, ReqType)`` pair to a
    :class:`FlatFileRowList`. The wildcard :py:meth:`request` dispatches
    dynamically by string identifiers.
    """

    def option_quote(self, date: str) -> FlatFileRowList: ...
    def option_trade(self, date: str) -> FlatFileRowList: ...
    def option_trade_quote(self, date: str) -> FlatFileRowList: ...
    def option_ohlc(self, date: str) -> FlatFileRowList: ...
    def option_open_interest(self, date: str) -> FlatFileRowList: ...
    def option_eod(self, date: str) -> FlatFileRowList: ...
    def stock_quote(self, date: str) -> FlatFileRowList: ...
    def stock_trade(self, date: str) -> FlatFileRowList: ...
    def stock_trade_quote(self, date: str) -> FlatFileRowList: ...
    def stock_eod(self, date: str) -> FlatFileRowList: ...
    def request(self, sec_type: str, req_type: str, date: str) -> FlatFileRowList: ...


class ThetaDataError(Exception):
    """Base exception for every typed error this binding raises."""

    ...


class AuthenticationError(ThetaDataError): ...


@final
class InvalidCredentialsError(AuthenticationError): ...


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


@final
class InvalidParameterError(ThetaDataError):
    """A client-side parameter was rejected by input validation.

    Distinct from the root :class:`ThetaDataError` so a malformed-but-
    rejected argument (bad value, out-of-range number, missing required
    field) is distinguishable by class from an unrelated configuration
    fault (config-file I/O, TOML parse), which stays on the root class.
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
    """FPSS streaming protocol / state-machine failure."""

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
TimeoutError: Type[DeadlineExceededError]


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


# Returns a ``(iv, iv_error)`` pair: the implied volatility from the
# bisection solver and the residual `(model_price - option_price) /
# option_price` at that vol. The runtime hands back a 2-tuple, not a
# bare float — stubtest cannot see the difference, so this annotation
# is the only thing keeping callers correct.
def implied_volatility(
    spot: float,
    strike: float,
    rate: float,
    div_yield: float,
    tte: float,
    option_price: float,
    right: str,
) -> tuple[float, float]: ...


@final
class AllGreeks:
    """All 23 Black-Scholes Greeks plus IV returned by ``all_greeks(...)``.

    Field order mirrors ``thetadatadx::GreeksResult`` (the Rust
    single-source-of-truth the binding wraps): the model value and IV
    pair first, then first-, second-, and third-order Greeks, then the
    ``d1`` / ``d2`` auxiliaries. Every attribute is a plain ``float``.
    """

    # Model price + implied-volatility pair.
    value: float
    iv: float
    iv_error: float
    # First-order Greeks.
    delta: float
    gamma: float
    theta: float
    vega: float
    rho: float
    # Second-order Greeks.
    vanna: float
    charm: float
    vomma: float
    veta: float
    vera: float
    # Third-order Greeks.
    speed: float
    zomma: float
    color: float
    ultima: float
    # Auxiliary quantities.
    d1: float
    d2: float
    dual_delta: float
    dual_gamma: float
    epsilon: float
    # Option elasticity (``delta * spot / value``). Carries the PEP 8
    # trailing-underscore keyword escape because ``lambda`` is a reserved
    # Python keyword — same spelling as the ``GreeksAllTick.lambda_`` tick
    # attribute, so the field stays reachable with ordinary attribute
    # syntax (``result.lambda_``) across the calculator and tick surfaces.
    lambda_: float


# ─────────────────────────────────────────────────────────────────────
# Catch-all: generator-emitted builders + typed `<Tick>List` wrappers
# resolve to `Any` at the module level. The Rust binding owns the
# SSOT for ~100 shape-identical per-endpoint builders / list classes;
# hand-mirroring every one here is high-maintenance noise that mypy
# would re-emit on every endpoint addition.
#
# This catch-all is scoped to MODULE-LEVEL attribute lookup ONLY.
# Every load-bearing pyclass (`ThetaDataDxClient`, `FpssClient`,
# `MddsClient`, `AsyncThetaDataDxClient`, `StreamingSession`)
# explicitly omits a per-class `__getattr__` fallback so stubtest
# catches method-level drift on those classes. Adding a new public
# method to any of them is a stubtest failure until the matching stub
# is updated.
# ─────────────────────────────────────────────────────────────────────


def __getattr__(name: str) -> Any: ...
