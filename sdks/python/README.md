# thetadatadx (Python)

Python bindings over the Rust core. Every call crosses the PyO3 boundary into Rust: gRPC communication, protobuf parsing, zstd decompression, FIT tick decoding, and TCP streaming run inside the `thetadatadx` crate.

> **Surface coverage:** the Python binding exposes all three ThetaData surfaces — MDDS (historical), FPSS (streaming), and FLATFILES (whole-universe daily blobs). Flat files land via `tdx.flat_files.*()` with `.to_arrow()`, `.to_polars()`, `.to_pandas()`, and `.to_list()` terminals plus a `flatfile_to_path(...)` raw-bytes helper — see the [Flat Files](#flat-files) section for the full method list.
>
> **REST routing escape hatch:** `FallbackPolicy.rest_always` + `Config.with_rest_fallback` + four `option_history_*_with_fallback` methods route the historical-quote endpoints over a locally-running Terminal's REST surface when the caller wants a single transport for every quote-bearing call. See [channel pool design](../../docs-site/docs/channel-pool-design.md) for the connection-recovery story.

## Installation

```bash
pip install thetadatadx

# With pandas DataFrame support (Arrow-backed, zero-copy on pandas 2.x)
pip install thetadatadx[pandas]

# With polars DataFrame support
pip install thetadatadx[polars]

# Raw Arrow (pyarrow.Table for DuckDB / Arrow-Flight / cuDF / polars-arrow)
pip install thetadatadx[arrow]

# All optional adapters
pip install thetadatadx[all]
```

Or build from source (requires Rust toolchain):

```bash
pip install "maturin>=1.9.4,<2.0"
maturin develop --release
```

Binary wheels use CPython's stable ABI (`abi3`) with a minimum Python version of 3.12, so one wheel per platform supports Python 3.12+.

Separate per-version wheels for PEP 703 free-threaded interpreters (`python3.13t`, `python3.14t`) will be picked up by `pip` automatically once the next PyPI release ships. The wheels carry `gil_used = false` on the extension module, so the GIL stays disabled after `import thetadatadx` and CPU-bound Python threads run truly in parallel with the gRPC dispatcher. After the next release lands, install on a free-threaded interpreter and a `cp313-cp313t-*` or `cp314-cp314t-*` wheel will be picked up; install on a stock interpreter and the `cp312-abi3-*` wheel applies.

## Quick Start

```python
from thetadatadx import Credentials, Config, ThetaDataDxClient

# Authenticate and connect
creds = Credentials.from_file("creds.txt")
# Or inline: creds = Credentials("user@example.com", "your-password")
tdx = ThetaDataDxClient(creds, Config.production())

# Fetch end-of-day data
eod = tdx.stock_history_eod("AAPL", "20240101", "20240301")
for tick in eod:
    print(f"{tick.date}: O={tick.open:.2f} H={tick.high:.2f} "
          f"L={tick.low:.2f} C={tick.close:.2f} V={tick.volume}")

# Intraday 1-minute OHLC bars (shorthand or milliseconds)
bars = tdx.stock_history_ohlc("AAPL", "20240315", "1m")
print(f"{len(bars)} bars")

# Option chain
exps = tdx.option_list_expirations("SPY")
strikes = tdx.option_list_strikes("SPY", exps[0])
```

## Greeks Calculator

Full Black-Scholes calculator with 23 Greeks, running in Rust:

```python
from thetadatadx import all_greeks, implied_volatility

# All Greeks at once
g = all_greeks(
    spot=450.0, strike=455.0, rate=0.05, div_yield=0.015,
    tte=30/365, option_price=8.50, right="C"
)
print(f"IV={g.iv:.4f} Delta={g.delta:.4f} Gamma={g.gamma:.6f}")

# Just IV
iv, err = implied_volatility(450.0, 455.0, 0.05, 0.015, 30/365, 8.50, "C")
```

## API

### `Credentials`
- `Credentials(email, password)` - direct construction
- `Credentials.from_file(path)` - load from creds.txt

### `Config`
- `Config.production()` - ThetaData NJ production servers
- `Config.dev()` - Dev FPSS servers (port 20200, infinite historical replay)
- `Config.stage()` - Stage FPSS servers (port 20100, testing, unstable)

### `ThetaDataDxClient(creds, config)`

Every historical endpoint is available. The tick-returning endpoints
return typed `<TickName>List` wrappers (e.g. `EodTickList`,
`TradeTickList`, `QuoteTickList`, ...), each implementing the
Python sequence protocol (`len(...)`, `for tick in ...`,
`lst[0]`, negative indexing) with typed-pyclass elements exposed
via attribute access — `tick.close`, `tick.price` — with IDE
completion and typo-loud `AttributeError` on misuse. Every
wrapper also exposes the chainable DataFrame terminals covered
in [Chained DataFrame terminals](#chained-dataframe-terminals).
List-of-string endpoints (`*_list_symbols`, `*_list_dates`,
`option_list_expirations`, `option_list_strikes`) return a
`StringList` wrapper with the same chainable terminals; the
single output column is named by the endpoint metadata (`symbol`,
`date`, `expiration`, ...). `option_list_contracts` returns an
`OptionContractList`; `calendar_on_date` / `calendar_year`
return a `CalendarDayList`.

#### Stock Methods (14)

| Method | Description |
|--------|-------------|
| `stock_list_symbols()` | All stock symbols |
| `stock_list_dates(request_type, symbol)` | Available dates by request type |
| `stock_snapshot_ohlc(symbols)` | Latest OHLC snapshot |
| `stock_snapshot_trade(symbols)` | Latest trade snapshot |
| `stock_snapshot_quote(symbols)` | Latest NBBO quote snapshot |
| `stock_snapshot_market_value(symbols)` | Latest market value snapshot |
| `stock_history_eod(symbol, start_date, end_date)` | End-of-day data |
| `stock_history_ohlc(symbol, date, interval)` | Intraday OHLC bars. `interval` accepts ms (`"60000"`) or shorthand (`"1m"`). |
| `stock_history_ohlc_range(symbol, start_date, end_date, interval)` | OHLC bars across date range. `interval` accepts ms or shorthand. |
| `stock_history_trade(symbol, date)` | All trades for a date |
| `stock_history_quote(symbol, date, interval)` | NBBO quotes. `interval` accepts ms or shorthand. |
| `stock_history_trade_quote(symbol, date)` | Combined trade+quote ticks |
| `stock_at_time_trade(symbol, start_date, end_date, time)` | Trade at specific time across dates |
| `stock_at_time_quote(symbol, start_date, end_date, time)` | Quote at specific time across dates |

#### Option Methods (34)

| Method | Description |
|--------|-------------|
| `option_list_symbols()` | Option underlying symbols |
| `option_list_dates(request_type, symbol, expiration, strike, right)` | Available dates for a contract |
| `option_list_expirations(symbol)` | Expiration dates |
| `option_list_strikes(symbol, expiration)` | Strike prices |
| `option_list_contracts(request_type, symbol, date)` | All contracts for a date |
| `option_snapshot_ohlc(symbol, expiration, strike, right)` | Latest OHLC snapshot |
| `option_snapshot_trade(symbol, expiration, strike, right)` | Latest trade snapshot |
| `option_snapshot_quote(symbol, expiration, strike, right)` | Latest quote snapshot |
| `option_snapshot_open_interest(symbol, expiration, strike, right)` | Latest open interest |
| `option_snapshot_market_value(symbol, expiration, strike, right)` | Latest market value |
| `option_snapshot_greeks_implied_volatility(symbol, expiration, strike, right)` | IV snapshot |
| `option_snapshot_greeks_all(symbol, expiration, strike, right)` | All Greeks snapshot |
| `option_snapshot_greeks_first_order(symbol, expiration, strike, right)` | First-order Greeks |
| `option_snapshot_greeks_second_order(symbol, expiration, strike, right)` | Second-order Greeks |
| `option_snapshot_greeks_third_order(symbol, expiration, strike, right)` | Third-order Greeks |
| `option_history_eod(symbol, expiration, strike, right, start_date, end_date)` | EOD option data |
| `option_history_ohlc(symbol, expiration, strike, right, date, interval)` | Intraday OHLC bars |
| `option_history_trade(symbol, expiration, strike, right, date)` | All trades |
| `option_history_quote(symbol, expiration, strike, right, date, interval)` | NBBO quotes |
| `option_history_trade_quote(symbol, expiration, strike, right, date)` | Combined trade+quote |
| `option_history_open_interest(symbol, expiration, strike, right, date)` | Open interest history |
| `option_history_greeks_eod(symbol, expiration, strike, right, start_date, end_date, *, annual_dividend=None, rate_type=None, rate_value=None, version=None, underlyer_use_nbbo=None, max_dte=None, strike_range=None)` | EOD Greeks |
| `option_history_greeks_all(symbol, expiration, strike, right, date, interval)` | All Greeks history |
| `option_history_trade_greeks_all(symbol, expiration, strike, right, date)` | Greeks on each trade |
| `option_history_greeks_first_order(symbol, expiration, strike, right, date, interval)` | First-order Greeks history |
| `option_history_trade_greeks_first_order(symbol, expiration, strike, right, date)` | First-order on each trade |
| `option_history_greeks_second_order(symbol, expiration, strike, right, date, interval)` | Second-order Greeks history |
| `option_history_trade_greeks_second_order(symbol, expiration, strike, right, date)` | Second-order on each trade |
| `option_history_greeks_third_order(symbol, expiration, strike, right, date, interval)` | Third-order Greeks history |
| `option_history_trade_greeks_third_order(symbol, expiration, strike, right, date)` | Third-order on each trade |
| `option_history_greeks_implied_volatility(symbol, expiration, strike, right, date, interval)` | IV history |
| `option_history_trade_greeks_implied_volatility(symbol, expiration, strike, right, date)` | IV on each trade |
| `option_at_time_trade(symbol, expiration, strike, right, start_date, end_date, time)` | Trade at specific time |
| `option_at_time_quote(symbol, expiration, strike, right, start_date, end_date, time)` | Quote at specific time |

#### Index Methods (9)

| Method | Description |
|--------|-------------|
| `index_list_symbols()` | All index symbols |
| `index_list_dates(symbol)` | Available dates for an index |
| `index_snapshot_ohlc(symbols)` | Latest OHLC snapshot |
| `index_snapshot_price(symbols)` | Latest price snapshot |
| `index_snapshot_market_value(symbols)` | Latest market value snapshot |
| `index_history_eod(symbol, start_date, end_date)` | End-of-day index data |
| `index_history_ohlc(symbol, start_date, end_date, interval)` | Intraday OHLC bars |
| `index_history_price(symbol, date, interval)` | Intraday price history |
| `index_at_time_price(symbol, start_date, end_date, time)` | Price at specific time |

#### Calendar Methods (3)

| Method | Description |
|--------|-------------|
| `calendar_open_today()` | Is the market open today? |
| `calendar_on_date(date)` | Calendar info for a date |
| `calendar_year(year)` | Calendar for an entire year |

#### Rate Methods (1)

| Method | Description |
|--------|-------------|
| `interest_rate_history_eod(symbol, start_date, end_date)` | Interest rate EOD history |

### Streaming — fluent contract-first API (primary)

Real-time streaming is accessed through the same `ThetaDataDxClient` instance.
The primary surface is the fluent contract / subscription model:

```python
from thetadatadx import Contract, SecType

stock  = Contract.stock("AAPL")
option = Contract.option("SPY", expiration="20260620", strike="550", right="C")

with client.streaming(on_event) as session:
    session.subscribe(stock.quote())
    session.subscribe(option.trade())
    session.subscribe(option.open_interest())
    session.subscribe(SecType.OPTION.full_trades())

    # Bulk install for many contracts:
    session.subscribe_many([stock.quote(), option.quote()])
```

The `Subscription` value returned by `Contract.quote()` /
`Contract.trade()` / `Contract.open_interest()` is typed and
homogeneous — full-stream subscriptions returned by
`SecType.OPTION.full_trades()` mix into the same `subscribe_many([...])`
list, no string flags, no kwarg gymnastics.

#### Fluent surface

| Method | Description |
|--------|-------------|
| `Contract.stock(symbol)` | Stock contract |
| `Contract.option(symbol, *, expiration, strike, right)` | Option contract |
| `contract.quote()` / `.trade()` / `.open_interest()` | Per-contract `Subscription` |
| `SecType.OPTION.full_trades()` / `.full_open_interest()` | Full-stream `Subscription` |
| `client.subscribe(sub)` | Install one `Subscription` |
| `client.subscribe_many([sub, ...])` | Install many `Subscription` values |
| `client.unsubscribe(sub)` / `unsubscribe_many([...])` | Drop subscriptions |

**Full trade stream behavior:** when subscribed via `client.subscribe(SecType.OPTION.full_trades())`, the ThetaData FPSS server sends a **bundle** for every trade across ALL option contracts:

1. Pre-trade NBBO quote
2. OHLC bar for the traded contract
3. The trade itself
4. Two post-trade NBBO quotes

Events arrive as typed pyclass instances — `Quote`, `Trade`, `Ohlcvc`,
`OpenInterest` for market data; one typed class per control variant
(`LoginSuccess`, `ContractAssigned`, `ReqResponse`, `MarketOpen`,
`MarketClose`, `ServerError`, `Disconnected`, `Reconnecting`,
`Reconnected`, `Error`, `UnknownFrame`, `Connected`, `Ping`,
`ReconnectedServer`, `Restart`, `UnknownControl`). Each class mirrors
its `FpssControl::*` / `FpssData::*` Rust variant one-for-one, so
dispatch is a structural `match` — exactly the same shape Rust
consumers use. Truncated / unrecognised wire frames are filtered before
the callback fires and accounted on the
`thetadatadx.fpss.decode_failures` metric counter; they never surface
on the public event stream.

```python
from thetadatadx import (
    Quote, Trade, Ohlcvc, OpenInterest,
    LoginSuccess, ContractAssigned, Disconnected,
    Reconnecting, Reconnected, Restart,
)

# Build a contract ID -> symbol map as assignments arrive
contracts = {}


def on_event(event):
    match event:
        # Control / lifecycle events
        case LoginSuccess(permissions=p):
            print(f"logged in: bundle={p}")
        case ContractAssigned(id=cid, contract=c):
            contracts[cid] = c.symbol
        case Disconnected(reason=r):
            print(f"disconnected: reason={r}")
        case Reconnecting(attempt=n, delay_ms=ms):
            print(f"reconnect attempt {n} in {ms}ms")
        case Reconnected() | Restart():
            print("stream live again")

        # Market-data events — every variant carries its parsed `contract`.
        case Trade(price=px, size=sz, contract=c):
            print(f"[{c.symbol}] TRADE {px:.2f} x {sz}")
        case Quote(bid=b, ask=a, contract=c):
            print(f"[{c.symbol}] QUOTE bid={b:.2f} ask={a:.2f}")

        # Skip everything else (Ohlcvc bars, Ping heartbeats, ...)
        case _:
            pass


from thetadatadx import SecType

with tdx.streaming(on_event) as session:
    session.subscribe(SecType.OPTION.full_trades())

    # `on_event` runs on the LMAX Disruptor consumer thread under the
    # GIL, wrapped in `catch_unwind` so a Python exception is reported
    # via `tracing::error!` and `panic_count()` rather than tearing
    # down the consumer. Park the main thread while events flow.
    import time
    time.sleep(60)
# `__exit__` calls `stop_streaming()` and then blocks on
# `await_drain(5_000)` so the consumer thread has finished firing the
# callback before control returns. If the drain barrier times out, a
# `RuntimeWarning` is emitted but the `with` block still exits cleanly.
```

The `with tdx.streaming(callback)` context manager is the recommended
API. Subscription methods on the bound `session` proxy through to the
underlying `ThetaDataDxClient` via `StreamingSession.__getattr__` -- the
streaming surface is a single source of truth rooted in the Rust
crate, with no hand-listed wrapper to drift.

For the lower-level escape hatch (e.g. cross-process lifecycle
management, custom shutdown ordering), call the lifecycle methods
explicitly:

```python
tdx.start_streaming(callback=on_event)
tdx.subscribe(SecType.OPTION.full_trades())
import time
time.sleep(60)
tdx.stop_streaming()
# Drain barrier: by the time `await_drain(5000)` returns, the consumer
# thread is guaranteed to have finished firing `on_event`, so the
# closure stack the callback closed over can be released without a
# use-after-free race against the LMAX Disruptor consumer.
tdx.await_drain(5_000)
```

You can also subscribe to per-contract streams if you only need specific symbols rather than the full-stream subscription.

#### Pull-iter delivery — `for event in tdx.streaming_iter()` (high-throughput drain)

Push-callback (`tdx.streaming(callback)` above) is the recommended
default for low-latency single-event reaction. Pull-iter is the
sibling delivery mode for high-throughput batch processing where the
dominant cost is per-event Python work (tuple build, deque append,
DataFrame ingest) rather than per-event vendor latency:

```python
from thetadatadx import Contract, SecType

with tdx.streaming_iter() as iterator:
    iterator.subscribe(SecType.OPTION.full_trades())
    iterator.subscribe(Contract.stock("AAPL").quote())
    for event in iterator:
        match event:
            case Trade(price=p, size=s, contract=c):
                buf.append((c.symbol, p, s))
            case _:
                pass
```

`tdx.streaming_iter()` is a context manager that opens an FPSS
session in pull-iter mode on enter and pairs `stop_streaming()` +
`await_drain(5_000)` on exit, mirroring `tdx.streaming(callback)`.
The bound `iterator` is also an `EventIterator` you can drain with
`for event in iterator:`; `subscribe(...)` / `unsubscribe(...)`
forward to the underlying `ThetaDataDxClient` through the same
`__getattr__` proxy the callback session uses.

The Disruptor consumer thread pushes each event into a per-client
bounded queue; the `for event in iterator:` loop drains the queue
under one GIL acquire across the whole batch instead of one GIL
acquire per event. For the same per-event Python work (5-field
tuple build + `deque.append`), the included `streaming_throughput`
bench measures **~4.6 Melem/s for pull-iter** vs. **~1.1 Melem/s for
push-callback** — a 4.1× win.

Trade-off: pull-iter pays one queue hop of per-event latency for the
batched-GIL throughput win. Push-callback remains the recommended
default for sub-millisecond reaction loops; pick pull-iter when the
integrator's per-event work dominates.

Mode is chosen at start. Push and pull are mutually exclusive on a
given client; switch by calling `stop_streaming()` first. Backpressure
surfaces on the same `dropped_event_count()` counter as the callback
path.

#### Streaming buffering — Pattern A (`collections.deque`) vs Pattern B (`queue.Queue`)

Both patterns drain the SDK callback into a Python-native data
structure so business logic runs off the GIL-acquisition hot path.
Pattern A is the recommended default — it matches the
direct-callback shape used by every major market-data vendor's native
API and runs ~2-5x faster than Pattern B because `deque.append` /
`popleft` are GIL-atomic single ops with no condition-variable
wake-up. Pattern B is the right pick when the consumer needs
cross-thread blocking `get()` semantics.

```python
# Pattern A — collections.deque (lowest overhead, ring-buffer drop-oldest)
from collections import deque

buf = deque(maxlen=100_000)


def cb(event):
    buf.append(event)


tdx.start_streaming(cb)
# Consumer thread reads via buf.popleft() with retry-on-IndexError;
# `maxlen` enforces drop-oldest semantics inside the deque itself.
```

```python
# Pattern B — queue.Queue (cross-thread blocking get(), drop-newest backpressure)
import queue

q = queue.Queue(maxsize=100_000)


def cb(event):
    try:
        q.put_nowait(event)
    except queue.Full:
        pass  # explicit drop on overflow — the SDK's own dropped_event_count()
              # already counts ring-buffer overflow at the Rust layer.


tdx.start_streaming(cb)
# Consumer thread reads via q.get() (blocks until next event).
```

Trade-off: deque is the fastest queue in the standard library and
loses the oldest event on overflow; `queue.Queue` is slower but gives
you `get()` blocking semantics and an explicit drop-newest decision.
Pick deque by default; reach for `queue.Queue` only when you need
the blocking `get()`.

#### State & lifecycle

| Method | Description |
|--------|-------------|
| `active_subscriptions()` | Get list of active subscriptions (list of dicts with "kind" and "contract") |
| `streaming(callback)` | Open a context-managed streaming session. `with tdx.streaming(callback) as session:` registers `callback` via `start_streaming` on enter and pairs `stop_streaming()` + `await_drain(5_000)` on exit, mirroring the C++ RAII destructor. Subscription methods on the bound `session` proxy through to the underlying `ThetaDataDxClient` via `StreamingSession.__getattr__` -- single source of truth. |
| `streaming_iter()` | Open a context-managed pull-iter streaming session. `with tdx.streaming_iter() as it:` opens FPSS in pull-iter delivery mode on enter and pairs `stop_streaming()` + `await_drain(5_000)` on exit. The bound `it` is an `EventIterator` — `for event in it:` drains the per-client queue under one GIL acquire per batch, ~4.1× faster than push-callback on tuple-build / deque-append integrators. Mutually exclusive with `start_streaming(callback)` on the same client. |
| `start_streaming_iter()` | Lower-level pull-iter entry point. Returns a bare `EventIterator`; the caller is responsible for `stop_streaming()` + `await_drain()` on shutdown. Prefer `streaming_iter()` unless you need explicit lifecycle control. |
| `start_streaming(callback)` | Register a callable; the LMAX Disruptor consumer thread invokes `callback(event)` under the GIL for every typed FPSS event. `event` is a `Quote` / `Trade` / `Ohlcvc` / `OpenInterest` for market data or one of the typed control classes (`LoginSuccess`, `ContractAssigned`, `Disconnected`, `Reconnecting`, ...) for lifecycle events — dispatch via Python `match`. Truncated / unrecognised wire frames are filtered before the callback fires (counted on `thetadatadx.fpss.decode_failures`). Each invocation is wrapped in `catch_unwind`. `event.kind` carries the same discriminator tag as the TypeScript SDK's `FpssEvent.kind`. |
| `await_drain(timeout_ms)` | Block until the previous streaming session's consumer thread has finished firing the registered callback. Returns `True` if the drain completed within `timeout_ms`, `False` otherwise. Use after `stop_streaming()` or `reconnect()` from a thread other than the callback thread to confirm the callback closure can be safely released. The `with tdx.streaming(...)` context manager calls this for you with a 5_000 ms timeout. |
| `dropped_event_count()` | Cumulative count of events the FPSS reader could not publish into the Disruptor ring because the consumer fell behind and the ring was full. Resets to 0 on `reconnect()` (which rebuilds the FPSS client) and reads 0 after `stop_streaming()`. Snapshot the value before reconnect if you need to accumulate drops across session boundaries. |
| `reconnect()` | Reconnect streaming and re-register the previously installed callback; restores all subscriptions. |
| `shutdown()` | Graceful shutdown — drops the registered callback. |

### Streaming — `.stream(handler)` for large responses

Every historical builder exposes `.stream(handler)` and `.stream_async(handler)` alongside the buffered `.list()` / `.list_async()` terminals. The streaming variants drain the response chunk-by-chunk; the previous chunk is freed before the next is fetched. Peak resident memory stays at ~one chunk (≈64 KiB) regardless of total response size — eliminates the OOM mode reported on `option_history_quote(QQQ, 1DTE, interval=tick, strike_range=5)` at 32-permit concurrency (23 GiB RSS buffered → ~2 MiB streaming).

```python
# Sync: handler is called once per gRPC chunk with a typed list[QuoteTick].
def on_chunk(ticks):
    for t in ticks:
        # write to parquet / send to bus / accumulate stats
        pass

tdx.option_history_quote_builder("QQQ", "20260516", "20260516") \
    .interval("tick") \
    .strike_range(5) \
    .stream(on_chunk)

# Async: same handler shape, awaitable resolves to None on clean drain.
async def main():
    await tdx.option_history_quote_builder("QQQ", "20260516", "20260516") \
        .interval("tick") \
        .strike_range(5) \
        .stream_async(on_chunk)
```

The streaming variant is available on every historical builder regardless of tick type (`QuoteTick`, `TradeTick`, `OhlcTick`, `EodTick`, `GreeksAllTick`, `MarketValueTick`, `OptionContract`, `CalendarDay`, `InterestRateTick`, ...). Snapshot endpoints (≤10 rows) and the legacy `*_stream` endpoints don't expose the universal terminal — those keep their existing one-shot surface.

**Memory budget formula**:

```
peak_rss ≈ concurrency × rows × bytes_per_row × decode_factor
decode_factor: 3.0 buffered  /  1.0 streamed
```

For tick-interval requests across multi-day ranges or wide strike ranges, **always use `.stream()`** — the buffered path's `decode_factor=3.0` reflects the simultaneous residency of h2 frames + decompressed proto + decoded `Vec<T>` plus the `Vec::push` doubling transient.

**Buffered-size warning.** When the buffered `.list()` /
`.await` path returns a response whose estimated size exceeds the
configured threshold (default 100 MiB), the SDK emits a single
`tracing::warn!` event with `endpoint`, `row_count`, and `bytes_est`
fields suggesting `.stream(handler)` for the workload. Tune via
`MddsConfig::warn_on_buffered_threshold_bytes` (set to `0` to disable).
See [`docs-site/docs/channel-pool-design.md`](../../docs-site/docs/channel-pool-design.md)
for the gRPC channel-pool reconnect story.

### Chained DataFrame terminals

Every historical endpoint returns a typed list wrapper (`EodTickList`,
`OhlcTickList`, `StringList`, ...) whose terminals convert straight
into columnar form:

| Terminal | Returns | Extra |
|----------|---------|-------|
| `.to_list()` | Plain `list[TickClass]` | — |
| `.to_arrow()` | `pyarrow.Table` | `pip install thetadatadx[arrow]` |
| `.to_pandas()` | `pandas.DataFrame` | `pip install thetadatadx[pandas]` |
| `.to_polars()` | `polars.DataFrame` | `pip install thetadatadx[polars]` |

The `.to_arrow()` terminal hands the `RecordBatch` off to pyarrow via
the Arrow C Data Interface — zero-copy at the pyo3 boundary, the
underlying Arrow buffers are the same ones Rust just filled. `.to_pandas()`
and `.to_polars()` chain through the Arrow table for the final dtype.

```python
import duckdb
table = tdx.stock_history_eod("AAPL", "20240101", "20240301").to_arrow()
con = duckdb.connect()
con.register("eod", table)                  # zero-copy into DuckDB
con.sql("SELECT AVG(close) FROM eod").show()
```

The list wrapper itself behaves like a Python sequence:

```python
eod = tdx.stock_history_eod("AAPL", "20240101", "20240301")

len(eod)                                   # row count
bool(eod)                                  # False when empty
eod[0]                                     # first EodTick
eod[-1]                                    # last row, negative indexing supported
for tick in eod:                           # iterate in decode order
    process(tick)
```

On empty results the wrapper still has `__len__ == 0`, `bool(...) == False`,
and the `.to_arrow()` / `.to_pandas()` / `.to_polars()` terminals
produce a zero-row frame with the full typed column schema — no
`hint=` kwarg needed.

### `all_greeks(spot, strike, rate, div_yield, tte, option_price, right)`
`right` accepts `"C"`/`"P"` or `"call"`/`"put"` case-insensitively. Returns `AllGreeks` pyclass with 22 attribute fields (`g.iv`, `g.delta`, `g.gamma`, ...). All fields are `float` (f64). Consult `help(thetadatadx.AllGreeks)` for the full list.

### `implied_volatility(spot, strike, rate, div_yield, tte, option_price, right)`
`right` accepts `"C"`/`"P"` or `"call"`/`"put"` case-insensitively. Returns `(iv, error)` tuple.

## Flat Files

Whole-universe daily snapshots over the legacy MDDS port (port 12000).
Available for `(SecType, ReqType)` pairs spanning option / stock data
across `Quote`, `Trade`, `TradeQuote`, `Ohlc`, `OpenInterest`, `Eod`.

The decoded schema is determined at runtime by the request type, so
the binding follows the same `<List>` -> Arrow -> {pandas, polars}
pipeline as the typed historical endpoints, with the Arrow schema
inferred from the first row by `flatfiles::arrow::rows_to_arrow`.

```python
from thetadatadx import Credentials, Config, ThetaDataDxClient

creds = Credentials.from_file("creds.txt")
tdx = ThetaDataDxClient(creds, Config.production())

# Decoded -> polars / pandas / pyarrow
rows = tdx.flat_files.option_quote(date="20260428")
print(len(rows))
df = rows.to_polars()           # or .to_pandas() / .to_arrow() / .to_list()

# Generic dispatcher when sec_type / req_type come from config
oi = tdx.flat_files.request("OPTION", "OPEN_INTEREST", "20260428")

# Raw vendor CSV / JSONL straight to disk (no decode, no row materialise)
path = tdx.flatfile_to_path("OPTION", "QUOTE", "20260428",
                            "/tmp/option-quote", format="csv")
```

Available `flat_files.*` methods: `option_quote`, `option_trade`,
`option_trade_quote`, `option_ohlc`, `option_open_interest`,
`option_eod`, `stock_quote`, `stock_trade`, `stock_trade_quote`,
`stock_eod`, plus `request(sec_type, req_type, date)`.

## Architecture

```mermaid
graph TD
    A["Python code"] - "PyO3 FFI" --> B["thetadatadx Rust crate"]
    B - "in-house gRPC / TLS TCP" --> C["ThetaData servers"]
```

No HTTP middleware, no Java terminal, no subprocess. Direct wire-protocol access from the Rust core.

## FPSS Streaming

Real-time market data via ThetaData's FPSS servers:

```python
from thetadatadx import Credentials, Config, ThetaDataDxClient

creds = Credentials.from_file("creds.txt")
# Or inline: creds = Credentials("user@example.com", "your-password")
tdx = ThetaDataDxClient(creds, Config.production())

# Register a callback (typed pyclasses: `Quote`, `Trade`, `Ohlcvc`, ...).
# The LMAX Disruptor consumer thread acquires the GIL to invoke `on_event`,
# with each invocation wrapped in `catch_unwind` so a Python exception
# is counted on `panic_count()` rather than tearing down the consumer.
def on_event(event):
    if event.kind == "quote":
        print(f"Quote: {event.contract.symbol} bid={event.bid} ask={event.ask}")
    elif event.kind == "trade":
        print(f"Trade: {event.contract.symbol} price={event.price} size={event.size}")


with tdx.streaming(on_event) as session:
    session.subscribe(Contract.stock("AAPL").quote())
    session.subscribe(Contract.stock("SPY").trade())

    # Park the main thread while events flow.
    import time
    time.sleep(60)
# `__exit__` runs `stop_streaming()` + `await_drain(5_000)` so the
# consumer thread has finished firing `on_event` before this scope
# returns. Mirrors the C++ RAII destructor lifecycle.
```

## DataFrame Conversion (Arrow-Backed)

Every DataFrame entry point goes through a single Arrow columnar
pipeline: Rust `Vec<Tick>` -> `arrow::RecordBatch` -> `pyarrow.Table`
via the Arrow C Data Interface (zero-copy) -> pandas / polars / raw
Arrow. On pandas 2.x the numeric DataFrame columns alias the Arrow
buffers in place.

```python
from thetadatadx import Credentials, Config, ThetaDataDxClient

creds = Credentials.from_file("creds.txt")
tdx = ThetaDataDxClient(creds, Config.production())

# Pandas -- zero-copy numeric columns on pandas 2.x.
df  = tdx.stock_history_eod("AAPL", "20240101", "20240301").to_pandas()

# Polars via polars.from_arrow -- zero-copy at the Arrow boundary.
pdf = tdx.stock_history_eod("AAPL", "20240101", "20240301").to_polars()

# Raw Arrow -- plug straight into DuckDB / Arrow-Flight / cuDF.
tbl = tdx.stock_history_eod("AAPL", "20240101", "20240301").to_arrow()

# Same recipe for any tick-returning historical endpoint.
df  = tdx.stock_history_ohlc("AAPL", "20240315", "1m").to_pandas()
df  = tdx.option_history_trade("SPY", "20240620", "550", "C", "20240315").to_pandas()
```

List endpoints (`stock_list_symbols`, `option_list_expirations`, ...)
return a `StringList` wrapper with the same chainable terminals; the
DataFrame column header matches the semantic name:

```python
symbols = tdx.stock_list_symbols().to_polars()          # `symbol` column
expirations = tdx.option_list_expirations("AAPL").to_pandas()
```

See the Arrow project docs for the [C Data
Interface](https://arrow.apache.org/docs/format/CDataInterface.html)
(how the zero-copy handoff works) and [pyarrow Table
docs](https://arrow.apache.org/docs/python/generated/pyarrow.Table.html)
for consumer APIs.

Install with:
- `pip install thetadatadx[pandas]` — pandas + pyarrow
- `pip install thetadatadx[polars]` — polars + pyarrow
- `pip install thetadatadx[arrow]`  — pyarrow only
