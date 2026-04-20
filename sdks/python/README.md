# thetadatadx (Python)

Python SDK for ThetaData market data, powered by the `thetadatadx` Rust crate via PyO3.

**This is NOT a Python reimplementation.** Every call goes through compiled Rust - gRPC communication, protobuf parsing, zstd decompression, FIT tick decoding, and TCP streaming all happen at native speed. Python is just the interface.

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

Binary wheels use CPython's stable ABI (`abi3`) with a minimum Python version of 3.9, so one wheel per platform supports Python 3.9+.

## Quick Start

```python
from thetadatadx import Credentials, Config, ThetaDataDx

# Authenticate and connect
creds = Credentials.from_file("creds.txt")
# Or inline: creds = Credentials("user@example.com", "your-password")
tdx = ThetaDataDx(creds, Config.production())

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

Full Black-Scholes calculator with 22 Greeks, running in Rust:

```python
from thetadatadx import all_greeks, implied_volatility

# All Greeks at once
g = all_greeks(
    spot=450.0, strike=455.0, rate=0.05, div_yield=0.015,
    tte=30/365, option_price=8.50, right="C"
)
print(f"IV={g['iv']:.4f} Delta={g['delta']:.4f} Gamma={g['gamma']:.6f}")

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

### `ThetaDataDx(creds, config)`

All 61 endpoints are available. The 53 tick-returning endpoints
return lists of typed tick pyclass objects (e.g. `list[EodTick]`,
`list[TradeTick]`, `list[QuoteTick]`, ...). Field access is by
attribute — `tick.close`, `tick.price` — with IDE completion and
typo-loud `AttributeError` on misuse. The remaining list endpoints
(`*_list_symbols`, `*_list_dates`, `option_list_expirations`,
`option_list_strikes`) return `list[str]` / `list[int]` unchanged.

#### Stock Methods (14)

| Method | Description |
|--------|-------------|
| `stock_list_symbols()` | All stock symbols |
| `stock_list_dates(request_type, symbol)` | Available dates by request type |
| `stock_snapshot_ohlc(symbols)` | Latest OHLC snapshot |
| `stock_snapshot_trade(symbols)` | Latest trade snapshot |
| `stock_snapshot_quote(symbols)` | Latest NBBO quote snapshot |
| `stock_snapshot_market_value(symbols)` | Latest market value snapshot |
| `stock_history_eod(symbol, start, end)` | End-of-day data |
| `stock_history_ohlc(symbol, date, interval)` | Intraday OHLC bars. `interval` accepts ms (`"60000"`) or shorthand (`"1m"`). |
| `stock_history_ohlc_range(symbol, start, end, interval)` | OHLC bars across date range. `interval` accepts ms or shorthand. |
| `stock_history_trade(symbol, date)` | All trades for a date |
| `stock_history_quote(symbol, date, interval)` | NBBO quotes. `interval` accepts ms or shorthand. |
| `stock_history_trade_quote(symbol, date)` | Combined trade+quote ticks |
| `stock_at_time_trade(symbol, start, end, time)` | Trade at specific time across dates |
| `stock_at_time_quote(symbol, start, end, time)` | Quote at specific time across dates |

#### Option Methods (34)

| Method | Description |
|--------|-------------|
| `option_list_symbols()` | Option underlying symbols |
| `option_list_dates(request_type, symbol, exp, strike, right)` | Available dates for a contract |
| `option_list_expirations(symbol)` | Expiration dates |
| `option_list_strikes(symbol, exp)` | Strike prices |
| `option_list_contracts(request_type, symbol, date)` | All contracts for a date |
| `option_snapshot_ohlc(symbol, exp, strike, right)` | Latest OHLC snapshot |
| `option_snapshot_trade(symbol, exp, strike, right)` | Latest trade snapshot |
| `option_snapshot_quote(symbol, exp, strike, right)` | Latest quote snapshot |
| `option_snapshot_open_interest(symbol, exp, strike, right)` | Latest open interest |
| `option_snapshot_market_value(symbol, exp, strike, right)` | Latest market value |
| `option_snapshot_greeks_implied_volatility(symbol, exp, strike, right)` | IV snapshot |
| `option_snapshot_greeks_all(symbol, exp, strike, right)` | All Greeks snapshot |
| `option_snapshot_greeks_first_order(symbol, exp, strike, right)` | First-order Greeks |
| `option_snapshot_greeks_second_order(symbol, exp, strike, right)` | Second-order Greeks |
| `option_snapshot_greeks_third_order(symbol, exp, strike, right)` | Third-order Greeks |
| `option_history_eod(symbol, exp, strike, right, start, end)` | EOD option data |
| `option_history_ohlc(symbol, exp, strike, right, date, interval)` | Intraday OHLC bars |
| `option_history_trade(symbol, exp, strike, right, date)` | All trades |
| `option_history_quote(symbol, exp, strike, right, date, interval)` | NBBO quotes |
| `option_history_trade_quote(symbol, exp, strike, right, date)` | Combined trade+quote |
| `option_history_open_interest(symbol, exp, strike, right, date)` | Open interest history |
| `option_history_greeks_eod(symbol, exp, strike, right, start, end, *, annual_dividend=None, rate_type=None, rate_value=None, version=None, underlyer_use_nbbo=None, max_dte=None, strike_range=None)` | EOD Greeks |
| `option_history_greeks_all(symbol, exp, strike, right, date, interval)` | All Greeks history |
| `option_history_trade_greeks_all(symbol, exp, strike, right, date)` | Greeks on each trade |
| `option_history_greeks_first_order(symbol, exp, strike, right, date, interval)` | First-order Greeks history |
| `option_history_trade_greeks_first_order(symbol, exp, strike, right, date)` | First-order on each trade |
| `option_history_greeks_second_order(symbol, exp, strike, right, date, interval)` | Second-order Greeks history |
| `option_history_trade_greeks_second_order(symbol, exp, strike, right, date)` | Second-order on each trade |
| `option_history_greeks_third_order(symbol, exp, strike, right, date, interval)` | Third-order Greeks history |
| `option_history_trade_greeks_third_order(symbol, exp, strike, right, date)` | Third-order on each trade |
| `option_history_greeks_implied_volatility(symbol, exp, strike, right, date, interval)` | IV history |
| `option_history_trade_greeks_implied_volatility(symbol, exp, strike, right, date)` | IV on each trade |
| `option_at_time_trade(symbol, exp, strike, right, start, end, time)` | Trade at specific time |
| `option_at_time_quote(symbol, exp, strike, right, start, end, time)` | Quote at specific time |

#### Index Methods (9)

| Method | Description |
|--------|-------------|
| `index_list_symbols()` | All index symbols |
| `index_list_dates(symbol)` | Available dates for an index |
| `index_snapshot_ohlc(symbols)` | Latest OHLC snapshot |
| `index_snapshot_price(symbols)` | Latest price snapshot |
| `index_snapshot_market_value(symbols)` | Latest market value snapshot |
| `index_history_eod(symbol, start, end)` | End-of-day index data |
| `index_history_ohlc(symbol, start, end, interval)` | Intraday OHLC bars |
| `index_history_price(symbol, date, interval)` | Intraday price history |
| `index_at_time_price(symbol, start, end, time)` | Price at specific time |

#### Calendar Methods (3)

| Method | Description |
|--------|-------------|
| `calendar_open_today()` | Is the market open today? |
| `calendar_on_date(date)` | Calendar info for a date |
| `calendar_year(year)` | Calendar for an entire year |

#### Rate Methods (1)

| Method | Description |
|--------|-------------|
| `interest_rate_history_eod(symbol, start, end)` | Interest rate EOD history |

### Streaming (via `ThetaDataDx`)
Real-time streaming is accessed through the same `ThetaDataDx` instance.

#### Per-contract subscriptions (stocks)

| Method | Description |
|--------|-------------|
| `subscribe_quotes(symbol)` | Subscribe to quote data for a stock |
| `subscribe_trades(symbol)` | Subscribe to trade data for a stock |
| `subscribe_open_interest(symbol)` | Subscribe to open interest data for a stock |
| `unsubscribe_quotes(symbol)` | Unsubscribe from quote data for a stock |
| `unsubscribe_trades(symbol)` | Unsubscribe from trade data for a stock |
| `unsubscribe_open_interest(symbol)` | Unsubscribe from open interest data for a stock |

#### Per-contract subscriptions (options)

| Method | Description |
|--------|-------------|
| `subscribe_option_quotes(symbol, expiration, strike, right)` | Subscribe to option quote data. `right`: accepts `"call"`, `"put"`, `"C"`, `"P"` (case-insensitive). `strike`: dollar string e.g. `"550"`. |
| `subscribe_option_trades(symbol, expiration, strike, right)` | Subscribe to option trade data |
| `subscribe_option_open_interest(symbol, expiration, strike, right)` | Subscribe to option OI data |
| `unsubscribe_option_quotes(symbol, expiration, strike, right)` | Unsubscribe from option quotes |
| `unsubscribe_option_trades(symbol, expiration, strike, right)` | Unsubscribe from option trades |
| `unsubscribe_option_open_interest(symbol, expiration, strike, right)` | Unsubscribe from option OI |

#### Full-type subscriptions

| Method | Description |
|--------|-------------|
| `subscribe_full_trades(sec_type)` | Subscribe to ALL trades for a security type (`"STOCK"`, `"OPTION"`, `"INDEX"`) |
| `subscribe_full_open_interest(sec_type)` | Subscribe to ALL OI for a security type |
| `unsubscribe_full_trades(sec_type)` | Unsubscribe from ALL trades for a security type |
| `unsubscribe_full_open_interest(sec_type)` | Unsubscribe from ALL OI for a security type |

**Full trade stream behavior:** When subscribed via `subscribe_full_trades("OPTION")`, the ThetaData FPSS server sends a **bundle** for every trade across ALL option contracts:

1. Pre-trade NBBO quote
2. OHLC bar for the traded contract
3. The trade itself
4. Two post-trade NBBO quotes

Events arrive as typed objects — `Quote`, `Trade`, `Ohlcvc`, `OpenInterest`
for market data; `Simple` for control / diagnostic events
(login_success, contract_assigned, disconnected, market_open/close, ...);
`RawData` for unrecognized wire frames. Every variant carries a `.kind`
discriminator matching the TypeScript SDK's `FpssEvent.kind` tag exactly
(`"quote"`, `"trade"`, `"ohlcvc"`, `"open_interest"`, `"simple"`,
`"raw_data"`). Concrete control-event names (`"login_success"`,
`"contract_assigned"`, ...) live on `Simple.event_type` — mirroring
`FpssSimplePayload.eventType` on the TS side. Filter on `event.kind` to
route, then read attributes directly:

```python
tdx.start_streaming()
tdx.subscribe_full_trades("OPTION")

# Build a contract ID -> symbol map as assignments arrive
contracts = {}

while True:
    event = tdx.next_event(timeout_ms=100)
    if event is None:
        continue

    # Track contract assignments (control events go through `Simple`)
    if event.kind == "simple" and event.event_type == "contract_assigned":
        contracts[event.id] = event.detail
        continue

    contract = contracts.get(getattr(event, "contract_id", None), "unknown")

    # Filter by type - you choose what you want
    if event.kind == "trade":
        print(f"[{contract}] TRADE {event.price:.2f} x {event.size}")
    elif event.kind == "quote":
        print(f"[{contract}] QUOTE bid={event.bid:.2f} ask={event.ask:.2f}")
    # Skip ohlcvc if you don't need bars

tdx.stop_streaming()
```

You can also subscribe to per-contract streams if you only need specific symbols rather than the full firehose.

#### State & lifecycle

| Method | Description |
|--------|-------------|
| `contract_map()` | Get dict mapping contract IDs to string descriptions |
| `contract_lookup(id)` | Look up a single contract by ID (returns str or None) |
| `active_subscriptions()` | Get list of active subscriptions (list of dicts with "kind" and "contract") |
| `next_event(timeout_ms=5000)` | Poll for the next event (returns a typed `Quote` / `Trade` / `Ohlcvc` / `OpenInterest` / `Simple` / `RawData` pyclass, or `None` on timeout). `event.kind` carries the same discriminator tag as the TypeScript SDK's `FpssEvent.kind`. |
| `next_event_typed(timeout_ms=5000)` | Alias — same return type and shape as `next_event`. |
| `reconnect()` | Reconnect streaming and restore subscriptions |
| `shutdown()` | Graceful shutdown |

### `to_arrow(ticks)`
Convert a `list[TickClass]` to a `pyarrow.Table` with a zero-copy
handoff via the Arrow C Data Interface. The underlying Arrow buffers
are the same ones Rust just filled -- nothing is copied at the
pyo3 boundary. Requires `pip install thetadatadx[arrow]`.

Use this to feed DuckDB, Arrow-Flight, cuDF, polars-arrow, or any
other Arrow-native tool without an intermediate pandas step:

```python
import duckdb
table = thetadatadx.to_arrow(eod)          # pyarrow.Table
con = duckdb.connect()
con.register("eod", table)                  # zero-copy into DuckDB
con.sql("SELECT AVG(close) FROM eod").show()
```

### `to_dataframe(ticks)`
Convert a `list[TickClass]` to a pandas DataFrame. Backed by the
Arrow columnar pipeline -- on pandas 2.x the numeric columns alias
the Arrow buffers in place (zero copy). Benchmarks at 100k rows /
20 columns show ~8ms wall-clock (vs ~300-500ms for the legacy
dict-of-lists path). Requires `pip install thetadatadx[pandas]`.

### `to_polars(ticks)`
Convert to a polars DataFrame via `polars.from_arrow` -- zero-copy
at the Arrow boundary. Requires `pip install thetadatadx[polars]`.

### Unified DataFrame path
No per-endpoint `_df` / `_arrow` / `_polars` convenience wrappers.
Every historical endpoint returns `list[TickClass]`; chain
`to_dataframe(ticks)` / `to_polars(ticks)` / `to_arrow(ticks)` for
the Arrow-backed conversion. One code path, one schema, one place
to audit. See the "DataFrame Conversion (Arrow-Backed)" section
below for the recipe.

### `all_greeks(spot, strike, rate, div_yield, tte, option_price, right)`
`right` accepts `"C"`/`"P"` or `"call"`/`"put"` case-insensitively. Returns dict with 22 Greeks: delta, gamma, theta, vega, rho, iv, vanna, charm, vomma, veta, speed, zomma, color, ultima, d1, d2, dual_delta, dual_gamma, epsilon, lambda.

### `implied_volatility(spot, strike, rate, div_yield, tte, option_price, right)`
`right` accepts `"C"`/`"P"` or `"call"`/`"put"` case-insensitively. Returns `(iv, error)` tuple.

## Architecture

```mermaid
graph TD
    A["Python code"] - "PyO3 FFI" --> B["thetadatadx Rust crate"]
    B - "tonic gRPC / TLS TCP" --> C["ThetaData servers"]
```

No HTTP middleware, no Java terminal, no subprocess. Direct wire protocol access at Rust speed.

## FPSS Streaming

Real-time market data via ThetaData's FPSS servers:

```python
from thetadatadx import Credentials, Config, ThetaDataDx

creds = Credentials.from_file("creds.txt")
# Or inline: creds = Credentials("user@example.com", "your-password")
tdx = ThetaDataDx(creds, Config.production())

# Start streaming and subscribe to real-time data
tdx.start_streaming()
tdx.subscribe_quotes("AAPL")
tdx.subscribe_trades("SPY")

# Poll for events (typed pyclasses: `Quote`, `Trade`, `Ohlcvc`, ...)
while True:
    event = tdx.next_event(timeout_ms=5000)
    if event is None:
        break  # timeout, no event received
    if event.kind == "quote":
        print(f"Quote: contract_id={event.contract_id} bid={event.bid} ask={event.ask}")
    elif event.kind == "trade":
        print(f"Trade: contract_id={event.contract_id} price={event.price} size={event.size}")

tdx.stop_streaming()
```

## DataFrame Conversion (Arrow-Backed)

Every DataFrame entry point goes through a single Arrow columnar
pipeline: Rust -> `arrow::RecordBatch` -> `pyarrow.Table` via the
Arrow C Data Interface (zero-copy) -> pandas / polars / raw Arrow.
On pandas 2.x the numeric DataFrame columns alias the Arrow
buffers in place, so a 100k x 20 result set converts in ~8ms
(vs ~300-500ms for the old dict-of-lists path).

```python
from thetadatadx import (
    Credentials, Config, ThetaDataDx,
    to_arrow, to_dataframe, to_polars,
)

creds = Credentials.from_file("creds.txt")
tdx = ThetaDataDx(creds, Config.production())

# One typed path: historical endpoints return `list[TickClass]`,
# then chain the Arrow-backed adapter for pandas / polars / raw Arrow.

eod = tdx.stock_history_eod("AAPL", "20240101", "20240301")

# Pandas -- zero-copy numeric columns on pandas 2.x
df = to_dataframe(eod)

# Polars via polars.from_arrow -- zero-copy
pdf = to_polars(eod)

# Raw Arrow -- plug straight into DuckDB / Arrow-Flight / cuDF
table = to_arrow(eod)

# Same recipe for any of the 44 tick-returning historical endpoints:
ohlc = tdx.stock_history_ohlc("AAPL", "20240315", "1m")
df   = to_dataframe(ohlc)

trd  = tdx.option_history_trade("SPY", "20240620", "550", "C", "20240315")
df   = to_dataframe(trd)
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
