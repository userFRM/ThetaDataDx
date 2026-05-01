# thetadatadx (Python)

Python bindings over the Rust core. Every call crosses the PyO3 boundary into Rust: gRPC communication, protobuf parsing, zstd decompression, FIT tick decoding, and TCP streaming run inside the `thetadatadx` crate.

> **FLATFILES coverage:** the Python binding currently exposes the MDDS (historical) and FPSS (streaming) surfaces only. The third surface — FLATFILES whole-universe daily blobs — is shipped in the Rust core (v8.0.17+) and is being wired into Python under issue [#435](https://github.com/userFRM/ThetaDataDx/issues/435). See [`ROADMAP.md`](../../ROADMAP.md#flatfiles--binding-coverage) for the per-binding status.

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

### `ThetaDataDx(creds, config)`

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

## Architecture

```mermaid
graph TD
    A["Python code"] - "PyO3 FFI" --> B["thetadatadx Rust crate"]
    B - "tonic gRPC / TLS TCP" --> C["ThetaData servers"]
```

No HTTP middleware, no Java terminal, no subprocess. Direct wire-protocol access from the Rust core.

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
pipeline: Rust `Vec<Tick>` -> `arrow::RecordBatch` -> `pyarrow.Table`
via the Arrow C Data Interface (zero-copy) -> pandas / polars / raw
Arrow. On pandas 2.x the numeric DataFrame columns alias the Arrow
buffers in place.

```python
from thetadatadx import Credentials, Config, ThetaDataDx

creds = Credentials.from_file("creds.txt")
tdx = ThetaDataDx(creds, Config.production())

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
