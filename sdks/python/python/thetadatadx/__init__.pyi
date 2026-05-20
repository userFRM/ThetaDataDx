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

from typing import Any, Callable, Iterator, List, Optional, Tuple, Type

# ─────────────────────────────────────────────────────────────────────
# Credentials + Config
# ─────────────────────────────────────────────────────────────────────


class Credentials:
    """ThetaData Nexus credentials (email + password)."""

    def __init__(self, email: str, password: str) -> None: ...
    @staticmethod
    def from_file(path: str) -> Credentials: ...
    def __repr__(self) -> str: ...


class Config:
    """Connection configuration: MDDS host / FPSS hosts / reconnect policy."""

    @staticmethod
    def production() -> Config: ...
    @staticmethod
    def dev() -> Config: ...
    @staticmethod
    def stage() -> Config: ...
    # MDDS host / port — settable so structural tests can point at a
    # known-refused endpoint.
    mdds_host: str
    mdds_port: int
    # Reconnect tunables.
    reconnect_policy: str
    reconnect_max_attempts: int
    reconnect_max_rate_limited_attempts: int
    reconnect_stable_window_secs: int
    # FPSS tunables.
    derive_ohlcvc: bool

    def __repr__(self) -> str: ...


# ─────────────────────────────────────────────────────────────────────
# Fluent: Contract / Subscription / SecType
# ─────────────────────────────────────────────────────────────────────


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
# ─────────────────────────────────────────────────────────────────────


class Quote:
    """FPSS per-contract quote event."""

    contract: Contract
    bid_price: float
    bid_size: int
    ask_price: float
    ask_size: int
    timestamp_ns: int


class Trade:
    """FPSS per-contract trade event."""

    contract: Contract
    price: float
    size: int
    timestamp_ns: int


class OpenInterest:
    """FPSS open-interest event (per-contract or full-stream)."""

    contract: Contract
    open_interest: int
    timestamp_ns: int


class Ohlcvc:
    """FPSS OHLCVC bar (derived in the SDK when `Config.derive_ohlcvc=True`)."""

    contract: Contract
    open: float
    high: float
    low: float
    close: float
    volume: int
    count: int


# ─────────────────────────────────────────────────────────────────────
# Streaming clients
# ─────────────────────────────────────────────────────────────────────

EventCallback = Callable[[Any], None]


class ThetaDataDxClient:
    """Unified client: opens MDDS + Nexus at construction, FPSS on demand."""

    def __init__(self, creds: Credentials, config: Config) -> None: ...
    # Historical endpoints are generator-emitted and resolved through
    # ``__getattr__`` — they appear on the instance but are not listed
    # exhaustively here.

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
    def panic_count(self) -> int: ...

    # Context managers.
    def streaming(self, callback: EventCallback) -> StreamingSession: ...
    def streaming_iter(self) -> StreamingIterSession: ...

    # Session / subscription metadata.
    def session_uuid(self) -> str: ...
    def subscription_info(self) -> List[Tuple[str, str]]: ...

    def __repr__(self) -> str: ...
    def __getattr__(self, name: str) -> Any: ...


class AsyncThetaDataDxClient:
    """Async surface: ``*_async`` historical methods plus streaming helpers."""

    def __init__(self, creds: Credentials, config: Config) -> None: ...
    @staticmethod
    def from_file(path: str) -> AsyncThetaDataDxClient: ...
    def __repr__(self) -> str: ...
    def __getattr__(self, name: str) -> Any: ...


class FpssClient:
    """Standalone FPSS-only streaming client — never opens MDDS / Nexus."""

    def __init__(self, creds: Credentials, config: Config) -> None: ...

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

    def __repr__(self) -> str: ...


class MddsClient:
    """Standalone MDDS-only historical client — FPSS surface is blocked."""

    def __init__(self, creds: Credentials, config: Config) -> None: ...
    @staticmethod
    def from_file(path: str) -> MddsClient: ...
    def __repr__(self) -> str: ...
    # Historical endpoints reach through ``__getattr__``; FPSS-touching
    # method names raise ``AttributeError`` (see
    # ``mdds_client::FPSS_TOUCHING_METHODS`` for the inventory).
    def __getattr__(self, name: str) -> Any: ...


# ─────────────────────────────────────────────────────────────────────
# Streaming context managers + iterator
# ─────────────────────────────────────────────────────────────────────


class StreamingSession:
    """Context manager for push-callback FPSS streaming."""

    def __enter__(self) -> StreamingSession: ...
    def __exit__(
        self,
        exc_type: Optional[Type[BaseException]],
        exc_value: Optional[BaseException],
        traceback: Optional[Any],
    ) -> bool: ...
    def __getattr__(self, name: str) -> Any: ...


class StreamingIterSession:
    """Context manager for pull-iter FPSS streaming."""

    def __enter__(self) -> EventIterator: ...
    def __exit__(
        self,
        exc_type: Optional[Type[BaseException]],
        exc_value: Optional[BaseException],
        traceback: Optional[Any],
    ) -> bool: ...
    def __getattr__(self, name: str) -> Any: ...


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
# Exception hierarchy
# ─────────────────────────────────────────────────────────────────────


class ThetaDataError(Exception):
    """Base exception for every typed error this binding raises."""

    ...


class AuthenticationError(ThetaDataError): ...


class InvalidCredentialsError(AuthenticationError): ...


class NetworkError(ThetaDataError): ...


class NoDataFoundError(ThetaDataError): ...


class Error(ThetaDataError):
    """Generic untyped error — fallback when no typed variant matches."""

    ...


# ─────────────────────────────────────────────────────────────────────
# Utility module-level entry points
# ─────────────────────────────────────────────────────────────────────


def decode_response_bytes(endpoint: str, chunks: List[bytes]) -> Any: ...


def split_date_range(
    start_date: str,
    end_date: str,
    *,
    days_per_chunk: int = ...,
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
# Catch-all: generator-emitted builders, typed `<Tick>List` wrappers,
# FPSS event variants (`LoginSuccess`, `ContractAssigned`, …), and any
# other module-level attribute not listed above resolve to `Any`. The
# Rust binding owns the SSOT; mirroring 100+ shape-identical builders
# here would be high-maintenance noise.
# ─────────────────────────────────────────────────────────────────────


def __getattr__(name: str) -> Any: ...
