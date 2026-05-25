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
"""

from __future__ import annotations

from typing import (
    Any,
    AsyncIterator,
    Awaitable,
    Callable,
    Iterator,
    List,
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
    # connect time with a warn. `decoder_ring_size` must be a power
    # of two >= 64; the setter raises ValueError otherwise.
    concurrent_requests: int
    decoder_ring_size: int
    # Byte ceiling above which a buffered (non-`.stream()`) historical
    # response emits a Rust-side `tracing::warn!` pointing the caller
    # at the streaming surface. `0` disables the warning; the default
    # is `100 * 1024 * 1024` (100 MiB). The data is still delivered.
    warn_on_buffered_threshold_bytes: int
    # MDDS two-stage decode pipeline. `decode_threads` sizes the
    # stage-2 prost-decode + Tick-build worker pool;
    # `decode_queue_depth` sizes the bounded MPSC queue between
    # stage-1 (per-channel zstd decompress) and stage-2. `None`
    # auto-sizes at connect time; `int` overrides. `0` clamps to
    # `1` internally so a zero-worker pool cannot deadlock stage-1.
    decode_threads: Optional[int]
    decode_queue_depth: Optional[int]
    # Reconnect tunables.
    reconnect_policy: str
    reconnect_max_attempts: int
    reconnect_max_rate_limited_attempts: int
    reconnect_stable_window_secs: int
    # Reconnect cadence (ms) per failure class. Default
    # `wait_ms=2_000` (generic transient) / `wait_rate_limited_ms=130_000`
    # (TooManyRequests). Plumbed through to the FPSS I/O loop at
    # connect time.
    reconnect_wait_ms: int
    reconnect_wait_rate_limited_ms: int
    # Tokio worker-thread count for embedded runtimes built via
    # `RuntimeConfig::build_runtime`. `None` defers to tokio's default
    # sizing; `int` (including `0`, which clamps to `1` inside the
    # builder) pins worker count.
    tokio_worker_threads: Optional[int]
    # RetryPolicy fields — per-field access on `DirectConfig.retry`.
    # Defaults: `initial=250ms`, `max=30s`, `attempts=5`, `jitter=True`.
    # Methods `delay_for_attempt` / `capped_backoff` stay Rust-only.
    retry_initial_delay_ms: int
    retry_max_delay_ms: int
    retry_max_attempts: int
    retry_jitter: bool
    # FPSS tunables.
    derive_ohlcvc: bool
    # REST-routing variant. Read-only -- write via
    # `with_rest_fallback`.
    fallback_variant: str

    def with_rest_fallback(self, policy: "FallbackPolicy") -> None: ...
    def __repr__(self) -> str: ...


# ─────────────────────────────────────────────────────────────────────
# FallbackPolicy — REST routing for the four historical-quote endpoints
# ─────────────────────────────────────────────────────────────────────


# Default Terminal REST base URL. Mirrors
# `thetadatadx::config::DEFAULT_REST_BASE_URL`.
DEFAULT_REST_BASE_URL: str = ...


@final
class FallbackPolicy:
    """REST-fallback policy for the four historical-quote endpoints.

    Construct via one of the two static factories. The Rust enum is
    `#[non_exhaustive]`; future variants land here behind new factories.
    """

    @staticmethod
    def disabled() -> "FallbackPolicy": ...
    @staticmethod
    def rest_always(base_url: str) -> "FallbackPolicy": ...

    base_url: str | None
    variant: str

    def __repr__(self) -> str: ...


# ─────────────────────────────────────────────────────────────────────
# Fluent: Contract / Subscription / SecType
# ─────────────────────────────────────────────────────────────────────


@final
class Contract:
    """Per-contract identity (stock or option) for FPSS subscriptions."""

    @staticmethod
    def stock(symbol: str) -> Contract: ...
    @staticmethod
    def option(
        symbol: str,
        expiration: str,
        strike: str,
        right: str,
    ) -> Contract: ...
    @property
    def symbol(self) -> str: ...
    @property
    def sec_type(self) -> SecType: ...

    def quote(self) -> Subscription: ...
    def trade(self) -> Subscription: ...
    def open_interest(self) -> Subscription: ...

    def __repr__(self) -> str: ...
    def __eq__(self, other: object) -> bool: ...


@final
class ContractRef:
    """Read-only contract identifier surfaced on every FPSS event.

    Distinct from the fluent `Contract` builder — `ContractRef` is what
    `event.contract` returns inside a streaming callback, with the
    resolved `symbol`, `sec_type`, `expiration`, `right`,
    `strike_dollars`, and the wire-level integer `strike`. The fluent
    `Contract` (above) is the one users instantiate to subscribe.
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
    def strike_dollars(self) -> Optional[float]: ...
    @property
    def strike(self) -> Optional[int]: ...

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
    """`"quote"` / `"trade"` / `"open_interest"` / `"full_trades"` / `"full_open_interest"`."""

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
# P4 closure: every field below was extracted from the
# `#[pyo3(get)]` declarations on the generated `_generated/fpss_event_classes.rs`
# at v10.0.0 + the post-v10 surface additions. Updating this stub
# without touching the matching pyclass attribute (or vice versa) is
# caught by `python -m mypy.stubtest thetadatadx --ignore-missing-stub`.
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


# NOTE: The runtime exposes this class as `Error` — same name as the
# generic `Error` exception class declared near the bottom of this
# stub. The two have disjoint inheritance (FPSS-event payload vs
# exception type) and never share an instance, so the runtime
# resolution is unambiguous. We stub the FPSS-event variant under
# the suffix `FpssParseError` to avoid the mypy duplicate-class
# error, and add it to the stubtest allowlist as a documented alias.
@final
class FpssParseError:
    """FPSS protocol-level parse error. Mirrors `FpssControl::Error`.

    Runtime class name is ``Error`` (matches the Rust enum variant
    surface name). The stub uses ``FpssParseError`` to avoid colliding
    with the generic :class:`Error` exception class also named
    ``Error`` at runtime; both names point to the runtime ``Error``
    class via the module-level ``__getattr__`` fallback. See the
    stubtest allowlist for the documented alias.
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

    C6 closure: this stub no longer carries a per-class ``__getattr__``
    fallback. Every public method below is hand-listed so a new
    generator-emitted method shows up as a stubtest failure until the
    stub is regenerated. The module-level ``__getattr__`` at the
    bottom of this file routes the catch-all generator-emitted
    historical builders / list classes / endpoint factories without
    masking method-level drift on the load-bearing pyclasses.
    """

    def __init__(self, creds: Credentials, config: Config) -> None: ...

    # Streaming lifecycle.
    def start_streaming(self, callback: EventCallback) -> None: ...
    def start_streaming_iter(self) -> EventIterator: ...
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

    # Metrics.
    def dropped_event_count(self) -> int: ...

    # REST-routing surface for the four historical-quote endpoints.
    # Returns the typed tick-list wrappers; chain `.to_polars()` /
    # `.to_pandas()` / `.to_arrow()` for columnar consumers.
    def option_history_quote_with_fallback(
        self,
        symbol: str,
        expiration: str,
        start_date: str,
        end_date: Optional[str] = None,
        strike: Optional[str] = None,
        right: Optional[str] = None,
        interval: Optional[str] = None,
    ) -> Any:
        """Fetch option NBBO history per the configured FallbackPolicy.

        Raises:
            NetworkError: transport or REST-status failure.
            AuthenticationError: invalid credentials / unauthenticated.
            TimeoutError: SDK-side request deadline exceeded.
            SubscriptionError: tier does not permit the request.
            RateLimitError: backend signalled too-many-requests.
            NoDataFoundError: the request returned no rows.
            SchemaMismatchError: response failed strict schema decode.
        """
        ...

    def option_history_trade_quote_with_fallback(
        self,
        symbol: str,
        expiration: str,
        start_date: str,
        end_date: Optional[str] = None,
        strike: Optional[str] = None,
        right: Optional[str] = None,
    ) -> Any:
        """Fetch combined trade + NBBO history per the configured FallbackPolicy.

        Raises:
            NetworkError: transport or REST-status failure.
            AuthenticationError: invalid credentials / unauthenticated.
            TimeoutError: SDK-side request deadline exceeded.
            SubscriptionError: tier does not permit the request.
            RateLimitError: backend signalled too-many-requests.
            NoDataFoundError: the request returned no rows.
            SchemaMismatchError: response failed strict schema decode.
        """
        ...

    def option_history_greeks_implied_volatility_with_fallback(
        self,
        symbol: str,
        expiration: str,
        start_date: str,
        end_date: Optional[str] = None,
        strike: Optional[str] = None,
        right: Optional[str] = None,
        interval: Optional[str] = None,
    ) -> Any:
        """Fetch implied-volatility history per the configured FallbackPolicy.

        Raises:
            NetworkError: transport or REST-status failure.
            AuthenticationError: invalid credentials / unauthenticated.
            TimeoutError: SDK-side request deadline exceeded.
            SubscriptionError: tier does not permit the request.
            RateLimitError: backend signalled too-many-requests.
            NoDataFoundError: the request returned no rows.
            SchemaMismatchError: response failed strict schema decode.
        """
        ...

    def option_history_greeks_first_order_with_fallback(
        self,
        symbol: str,
        expiration: str,
        start_date: str,
        end_date: Optional[str] = None,
        strike: Optional[str] = None,
        right: Optional[str] = None,
        interval: Optional[str] = None,
    ) -> Any:
        """Fetch first-order Greeks history per the configured FallbackPolicy.

        Raises:
            NetworkError: transport or REST-status failure.
            AuthenticationError: invalid credentials / unauthenticated.
            TimeoutError: SDK-side request deadline exceeded.
            SubscriptionError: tier does not permit the request.
            RateLimitError: backend signalled too-many-requests.
            NoDataFoundError: the request returned no rows.
            SchemaMismatchError: response failed strict schema decode.
        """
        ...

    # Context managers.
    def streaming(self, callback: EventCallback) -> StreamingSession: ...
    def streaming_iter(self) -> StreamingIterSession: ...
    def streaming_async(
        self,
        *,
        max_queue_depth: int = ...,
        backpressure: BackpressurePolicy = ...,
    ) -> StreamingAsyncSession: ...
    def streaming_async_batches(
        self,
        *,
        max_queue_depth: int = ...,
        backpressure: BackpressurePolicy = ...,
    ) -> StreamingAsyncBatchesSession: ...

    # FLATFILES namespace getter.
    @property
    def flat_files(self) -> FlatFilesNamespace: ...

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
        / ``streaming_async`` / ``subscribe`` etc) → reaches the
        sync surface on the inner client; documented via
        :data:`ALLOWED_UNIFIED_PROXY_METHODS` in the binding source.

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
    def start_streaming_iter(self) -> EventIterator: ...
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

    def streaming(self, callback: EventCallback) -> StreamingSession: ...
    def streaming_iter(self) -> StreamingIterSession: ...
    def streaming_async(
        self,
        *,
        max_queue_depth: int = ...,
        backpressure: BackpressurePolicy = ...,
    ) -> StreamingAsyncSession: ...
    def streaming_async_batches(
        self,
        *,
        max_queue_depth: int = ...,
        backpressure: BackpressurePolicy = ...,
    ) -> StreamingAsyncBatchesSession: ...

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
class StreamingIterSession:
    """Context manager for pull-iter FPSS streaming.

    Like :class:`StreamingSession`, this is a PURE PROXY class with
    only the context-manager dunders + ``__getattr__`` physically on
    the pyclass; subscription / lifecycle methods proxy through to
    the bound client.
    """

    def __enter__(self) -> EventIterator: ...
    def __exit__(
        self,
        exc_type: Optional[Type[BaseException]],
        exc_value: Optional[BaseException],
        traceback: Optional[Any],
    ) -> bool: ...
    def __getattr__(self, name: str) -> Any: ...


@final
class EventIterator:
    """Iterator over FPSS events in pull-iter delivery mode."""

    def __iter__(self) -> EventIterator: ...
    def __next__(self) -> Any: ...
    def close(self) -> None: ...
    def __enter__(self) -> EventIterator: ...
    def __exit__(
        self,
        exc_type: Optional[Type[BaseException]],
        exc_value: Optional[BaseException],
        traceback: Optional[Any],
    ) -> bool: ...


# ─────────────────────────────────────────────────────────────────────
# FLATFILES surface
# ─────────────────────────────────────────────────────────────────────


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


@final
class StreamingAsyncSession:
    """Asyncio-native context manager for FPSS streaming.

    Bridges the Disruptor consumer thread to the asyncio event loop via
    FD-readiness signalling — the consumer writes a coalesced byte to a
    self-pipe on every successful queue push, and the loop's
    ``add_reader`` wakes the awaiting coroutine. No polling, no 100µs
    tick budget.

    Usage::

        async with client.streaming_async() as session:
            await session.subscribe(Contract.stock("QQQ").quote())
            async for batch in session:
                for ev in batch:
                    handle(ev)
    """

    def __aenter__(self) -> Awaitable[StreamingAsyncSession]: ...
    def __aexit__(
        self,
        exc_type: Optional[Type[BaseException]] = None,
        exc_value: Optional[BaseException] = None,
        traceback: Optional[Any] = None,
    ) -> Awaitable[None]: ...

    # Async iteration — yields ``list[FpssEvent]`` per OS wake.
    def __aiter__(self) -> AsyncIterator[List[Any]]: ...
    def __anext__(self) -> Awaitable[List[Any]]: ...

    # Awaitable subscription management. Resolves once the FPSS-protocol
    # round-trip lands.
    def subscribe(self, sub: Subscription) -> Awaitable[None]: ...
    def subscribe_many(self, subs: List[Subscription]) -> Awaitable[None]: ...
    def unsubscribe(self, sub: Subscription) -> Awaitable[None]: ...
    def unsubscribe_many(self, subs: List[Subscription]) -> Awaitable[None]: ...

    # Backpressure-aware drain. ``callback`` may be sync or
    # ``async def``; ``async def`` callbacks are awaited before the next
    # batch is drained so a slow consumer throttles upstream.
    def streaming_async_for_each(
        self, callback: Callable[[List[Any]], Any]
    ) -> Awaitable[None]: ...

    # Diagnostic — instantaneous queue depth between the Disruptor
    # consumer and this session.
    def queue_len(self) -> int: ...
    def queue_depth(self) -> int: ...
    def dropped_event_count(self) -> int: ...

    # Echoed-back configuration.
    @property
    def max_queue_depth(self) -> int: ...
    @property
    def backpressure(self) -> BackpressurePolicy: ...


@final
class StreamingAsyncBatchesSession:
    """Arrow IPC zero-copy batched streaming context manager.

    Sibling of :class:`StreamingAsyncSession` that yields one
    ``pyarrow.RecordBatch`` per OS wake instead of ``list[FpssEvent]``.
    Same wake-FD plumbing, same backpressure semantics — different
    drain shape optimised for vectorised downstream processing
    (pandas / polars / datafusion).

    Usage::

        async with client.streaming_async_batches() as session:
            await session.subscribe(Contract.stock("QQQ").quote())
            async for batch in session:
                df = batch.to_pandas()
                # ... vectorised processing ...

    The emitted schema is a union of every ``FpssData`` variant's
    columns plus a ``kind`` discriminator (``"Quote"`` / ``"Trade"``
    / ``"OpenInterest"`` / ``"Ohlcvc"``). Variant-specific columns
    are nullable and null-filled on rows that do not populate them.
    """

    def __aenter__(self) -> Awaitable[StreamingAsyncBatchesSession]: ...
    def __aexit__(
        self,
        exc_type: Optional[Type[BaseException]] = None,
        exc_value: Optional[BaseException] = None,
        traceback: Optional[Any] = None,
    ) -> Awaitable[None]: ...

    # Async iteration — yields ``pyarrow.RecordBatch`` per OS wake.
    # Annotated as ``Any`` because the pyarrow dep is optional at
    # install time; consumers gate on the extras (``pip install
    # thetadatadx[arrow]``).
    def __aiter__(self) -> AsyncIterator[Any]: ...
    def __anext__(self) -> Awaitable[Any]: ...

    def subscribe(self, sub: Subscription) -> Awaitable[None]: ...
    def subscribe_many(self, subs: List[Subscription]) -> Awaitable[None]: ...
    def unsubscribe(self, sub: Subscription) -> Awaitable[None]: ...
    def unsubscribe_many(self, subs: List[Subscription]) -> Awaitable[None]: ...

    def queue_len(self) -> int: ...
    def queue_depth(self) -> int: ...
    def dropped_event_count(self) -> int: ...
    def schema(self) -> Any: ...

    @property
    def max_queue_depth(self) -> int: ...
    @property
    def backpressure(self) -> BackpressurePolicy: ...


@final
class BackpressurePolicy:
    """Producer-side overflow strategy on the pull-iter queue.

    Mirrors the core ``thetadatadx::fpss::BackpressurePolicy`` enum.
    Pass one of the variants as the ``backpressure`` kwarg on
    ``streaming_async(...)``.

    * ``Block`` — preserve every event; producer parks when the queue
      saturates. Default.
    * ``DropOldest`` — evict queue head on full insert; preserve
      recency. Increments ``dropped_event_count`` per eviction.
    * ``DropNewest`` — skip new event on full insert; preserve
      in-flight history. Increments ``dropped_event_count`` per skip.
    """

    Block: BackpressurePolicy
    DropOldest: BackpressurePolicy
    DropNewest: BackpressurePolicy

    def __repr__(self) -> str: ...
    def __eq__(self, other: object) -> bool: ...


# ─────────────────────────────────────────────────────────────────────
# Exception hierarchy
# ─────────────────────────────────────────────────────────────────────


class ThetaDataError(Exception):
    """Base exception for every typed error this binding raises."""

    ...


class AuthenticationError(ThetaDataError): ...


@final
class InvalidCredentialsError(AuthenticationError): ...


@final
class NetworkError(ThetaDataError): ...


@final
class NoDataFoundError(ThetaDataError): ...


@final
class Error(ThetaDataError):
    """Generic untyped error — fallback when no typed variant matches."""

    ...


# ─────────────────────────────────────────────────────────────────────
# Utility module-level entry points
# ─────────────────────────────────────────────────────────────────────


def decode_response_bytes(endpoint: str, chunks: List[bytes]) -> Any: ...


def split_date_range(
    start: str,
    end: str,
) -> List[Tuple[str, str]]: ...


def all_greeks(
    spot: float,
    strike: float,
    rate: float,
    div_yield: float,
    tte: float,
    option_price: float,
    right: str,
) -> AllGreeks: ...


def implied_volatility(
    spot: float,
    strike: float,
    rate: float,
    div_yield: float,
    tte: float,
    option_price: float,
    right: str,
) -> float: ...


@final
class AllGreeks:
    """Greeks bundle returned by ``all_greeks(...)``."""

    delta: float
    gamma: float
    theta: float
    vega: float
    rho: float
    epsilon: float
    speed: float
    charm: float
    color: float
    vanna: float
    veta: float
    vomma: float
    ultima: float
    implied_volatility: float


# ─────────────────────────────────────────────────────────────────────
# Catch-all: generator-emitted builders + typed `<Tick>List` wrappers
# resolve to `Any` at the module level. The Rust binding owns the
# SSOT for ~100 shape-identical per-endpoint builders / list classes;
# hand-mirroring every one here is high-maintenance noise that mypy
# would re-emit on every endpoint addition.
#
# C6 closure: this catch-all is scoped to MODULE-LEVEL attribute
# lookup ONLY. Every load-bearing pyclass (`ThetaDataDxClient`,
# `FpssClient`, `MddsClient`, `AsyncThetaDataDxClient`,
# `StreamingSession`, `StreamingIterSession`, `StreamingAsyncSession`,
# `EventIterator`) explicitly removed its per-class `__getattr__`
# fallback so stubtest catches method-level drift on those classes.
# Adding a new public method to any of them is a stubtest failure
# until the matching stub is updated.
# ─────────────────────────────────────────────────────────────────────


def __getattr__(name: str) -> Any: ...
