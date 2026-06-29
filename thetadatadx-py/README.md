<p align="center">
  <img src="../assets/logo.svg" alt="ThetaDataDx" width="100%" />
</p>

# thetadatadx (Python)

The Python SDK for [ThetaData](https://thetadata.us) market data. Pull US stock, option, index, and rate data three ways — point-in-time **history**, real-time **streaming**, and whole-universe **flat files** — all from a single authenticated client. Connects straight to ThetaData; nothing to install and run locally, no local proxy.

[![PyPI](https://img.shields.io/pypi/v/thetadatadx?logo=python&logoColor=white)](https://pypi.org/project/thetadatadx)
[![License](https://img.shields.io/badge/license-Apache--2.0-blue.svg)](https://github.com/userFRM/ThetaDataDx/blob/main/LICENSE)
[![Python](https://img.shields.io/badge/python-3.12%2B-blue.svg?logo=python&logoColor=white)](https://www.python.org)
[![Discord](https://img.shields.io/badge/Discord-community-5865F2.svg?logo=discord&logoColor=white)](https://discord.thetadata.us/)

> [!IMPORTANT]
> A valid [ThetaData](https://thetadata.us) subscription is required. The SDK
> authenticates against ThetaData's Nexus API using your account credentials.

## Features

- **Complete coverage** — stocks, options, indices, and rates across 65 typed endpoints.
- **Three access modes, one client** — point-in-time history, real-time streaming, and bulk flat-file downloads.
- **DataFrames built in** — every result chains straight to Polars, pandas, or Arrow over a zero-copy boundary.
- **Typed all the way down** — every tick is a typed object with attribute access and IDE completion, not a dict.
- **No terminal to run** — a direct connection to ThetaData; nothing to install and babysit locally.

## Install

```bash
pip install thetadatadx

pip install thetadatadx[polars]   # Polars DataFrames
pip install thetadatadx[pandas]   # pandas DataFrames (Arrow-backed)
pip install thetadatadx[arrow]    # raw pyarrow.Table
pip install thetadatadx[all]      # every optional adapter
```

Binary wheels ship for Linux, macOS, and Windows and require no Rust toolchain. Wheels use CPython's stable ABI (`abi3`), so one wheel per platform covers Python 3.12 and up; the free-threaded (`3.14t`) build runs with the GIL disabled and is selected automatically by `pip`.

## Quick start

> [!TIP]
> Pass your API key directly to the client and you are one line from a live connection. Generate a key from your [ThetaData user portal](https://www.thetadata.net/), then construct `Client(api_key="td1_...")`. The key can also come from the environment with `Client.from_env()` (reading `THETADATA_API_KEY`) or a `.env` file with `Client.from_dotenv(".env")`. Email and password is also supported: `Client(email="you@example.com", password="your_password")` inline, or a `creds.txt` file (email on line 1, password on line 2) via `Credentials.from_file`. Target staging with `historical_type="STAGE"`. For full control over hosts and timeouts, build a typed `Credentials` + `Config` and pass both to `Client(...)`.

```python
from thetadatadx import Client

# Pass your API key directly. Use historical_type="STAGE" to target staging.
client = Client(api_key="td1_...")

# First-order Greeks for every strike on SPY's 2026-06-19 expiry, as of 2024-03-15
greeks = client.historical.option_history_greeks_first_order("SPY", "20260619", "20240315")

df = greeks.to_polars()
print(df.select(["strike", "right", "delta", "gamma", "theta", "vega"]).head())
```

Other ways to construct the client:

```python
from thetadatadx import Client, Credentials, Config

# API key from the THETADATA_API_KEY environment variable, or from a .env file
client = Client.from_env()
client = Client.from_dotenv(".env")

# Email and password, inline
client = Client(email="you@example.com", password="your_password")

# Full control: build a typed Credentials + Config (custom hosts, timeouts)
client = Client(Credentials.from_file("creds.txt"), Config.production())
```

Every historical method returns a typed list — iterate it, index it, or convert it to a dataframe:

```python
eod = client.historical.stock_history_eod("AAPL", "20240101", "20240301")
for tick in eod:
    print(f"{tick.date}: O={tick.open:.2f} H={tick.high:.2f} "
          f"L={tick.low:.2f} C={tick.close:.2f} V={tick.volume}")

bars = client.historical.stock_history_ohlc("AAPL", "20240315", interval="1m")   # 1-minute bars
exps = client.historical.option_list_expirations("SPY")
strikes = client.historical.option_list_strikes("SPY", exps[0])
```

## DataFrames

Every result converts directly to a dataframe — no row-by-row iteration:

```python
greeks.to_polars()   # polars.DataFrame
greeks.to_pandas()   # pandas.DataFrame   (pip install thetadatadx[pandas])
greeks.to_arrow()    # pyarrow.Table      (zero-copy)
greeks.to_list()     # list[GreeksTick]
```

The `.to_arrow()` terminal hands the underlying Arrow buffers to pyarrow over the [Arrow C Data Interface](https://arrow.apache.org/docs/format/CDataInterface.html) — zero-copy at the boundary — so the table plugs straight into DuckDB, polars, cuDF, or Arrow Flight:

```python
import duckdb

table = client.historical.stock_history_eod("AAPL", "20240101", "20240301").to_arrow()
con = duckdb.connect()
con.register("eod", table)                 # zero-copy into DuckDB
con.sql("SELECT AVG(close) FROM eod").show()
```

List endpoints (`stock_list_symbols`, `option_list_expirations`, …) return a `StringList` with the same terminals; the single column is named by the endpoint (`symbol`, `expiration`, …). Empty results still convert to a zero-row frame with the full typed schema.

For multi-day backfills, stream the response instead of buffering it. Every historical builder exposes `.stream(handler)` / `.stream_async(handler)` alongside the buffered `.list()` / `.list_async()` terminals; the handler is called once per chunk with a typed list, and the previous chunk is freed before the next is fetched, so peak memory stays flat regardless of total size:

```python
def on_chunk(ticks):
    for t in ticks:
        ...   # write to Parquet, push to a bus, accumulate stats

(client.historical.option_history_quote_builder("QQQ", "20260516", "20260516")
    .interval("tick")
    .strike_range(5)
    .stream(on_chunk))
```

## Streaming

Real-time quotes and trades flow through the same client. Register a callback and match on typed event classes — `Trade`, `Quote`, `Ohlcvc`, `OpenInterest` for market data, plus one typed class per lifecycle event (`Connected`, `LoginSuccess`, `Disconnected`, `Reconnecting`, …):

```python
import time
from thetadatadx import Contract, Quote, Trade

def on_event(event):
    match event:
        case Trade(price=px, size=sz, exchange=ex, ms_of_day=ms, sequence=seq, condition=cond, contract=c):
            print(
                f"{c.symbol} {c.expiration} {c.strike:g} {c.right} trade price={px:.2f} size={sz} "
                f"exchange={ex} ms_of_day={ms} sequence={seq} condition={cond}"
            )
        case Quote(bid=b, ask=a, bid_size=bs, ask_size=asz, bid_exchange=bx, ask_exchange=ax, ms_of_day=ms, contract=c):
            print(
                f"{c.symbol} {c.expiration} {c.strike:g} {c.right} quote bid={b:.2f} ask={a:.2f} "
                f"bid_size={bs} ask_size={asz} bid_exchange={bx} "
                f"ask_exchange={ax} ms_of_day={ms}"
            )

spy_call = Contract.option("SPY", expiration="20260620", strike="550", right="C")

with client.streaming(on_event) as session:
    session.subscribe_many([spy_call.quote(), spy_call.trade()])
    time.sleep(60)   # park the main thread while events flow into on_event
```

Build subscriptions with the fluent `Contract` API and pass them — one at a time or in bulk — to `subscribe` / `subscribe_many`. Every subscription is the same typed value, so quotes, trades, open interest, and market value across contracts mix freely in one list:

```python
from thetadatadx import Contract, SecType

stock  = Contract.stock("AAPL")
option = Contract.option("SPY", expiration="20260620", strike="550", right="C")

with client.streaming(on_event) as session:
    session.subscribe(stock.quote())
    session.subscribe_many([option.quote(), option.trade(), option.open_interest(), option.market_value()])
```

The option constructor is `Contract.option(symbol, *, expiration, strike, right)` — the leg parameters are keyword-only, so the call site always reads `expiration=…, strike=…, right=…` and never depends on argument order. Pair it with `Contract.stock(symbol)` for equities.

Or take a whole-market feed — every option trade across the universe, no per-contract setup. The full-trade feed sends a quote and an OHLC bar before each trade, so add an `Ohlcvc` case to the callback to handle the bars:

```python
from thetadatadx import Ohlcvc

def on_full_trade(event):
    match event:
        case Ohlcvc(open=o, high=h, low=lo, close=cl, volume=v, contract=c):
            print(
                f"{c.symbol} {c.expiration} {c.strike:g} {c.right} bar "
                f"o={o:.2f} h={h:.2f} l={lo:.2f} c={cl:.2f} volume={v}"
            )
        case _:
            on_event(event)   # reuse the quote/trade handling above

with client.streaming(on_full_trade) as session:
    session.subscribe(SecType.OPTION.full_trades())
    time.sleep(60)   # the callback runs on the streaming thread — keep it fast
```

Watch feed health from the main thread without touching the callback. The session resolves the client's observability getters directly: `millis_since_last_event()` is the staleness clock (a steadily growing value is the earliest sign of a wedged link), `ring_occupancy()` against `ring_capacity()` shows how close the consumer is to falling behind, and `dropped_event_count()` is the cumulative tally of events shed on a full ring:

```python
with client.streaming(on_event) as session:
    session.subscribe(SecType.OPTION.full_trades())
    while True:
        time.sleep(5)
        stale_ms = session.millis_since_last_event()   # None until the first frame
        print(
            f"stale={stale_ms}ms "
            f"ring={session.ring_occupancy()}/{session.ring_capacity()} "
            f"dropped={session.dropped_event_count()}"
        )
```

> [!TIP]
> The `with client.streaming(callback)` block opens the session on entry and drains
> it cleanly on exit, so the callback has stopped firing by the time the block
> returns. On an involuntary disconnect the client recovers on its own —
> exponential backoff with jitter, host failover, then a paced re-subscribe of
> every active contract.

Prefer columns? `client.stream.batches(...)` is a sibling to the callback — the same subscriptions, delivered as `pyarrow.RecordBatch` values under a fixed schema. The reader is iterable (sync, releasing the GIL on the blocking pull) and async-iterable, and closes on context-manager exit:

```python
# `batches(...)` starts the streaming session, so open it first, then subscribe.
with client.stream.batches(batch_size=8192) as batches:
    client.stream.subscribe(Contract.stock("AAPL").trade())
    for batch in batches:        # or: async for batch in batches
        print(batch.num_rows)
```

## Flat files

Whole-universe daily snapshots for one `(security type, request type, date)` at a time. The decoded schema follows the request type, so flat-file results chain through the same DataFrame terminals as history:

```python
rows = client.flat_files.option_trade_quote(date="20260428")
print(len(rows))
df = rows.to_polars()                       # or .to_pandas() / .to_arrow() / .to_list()

# Generic dispatcher when security type / request type come from config
oi = client.flat_files.request("OPTION", "OPEN_INTEREST", "20260428")

# Or write the raw vendor file straight to disk — no decode, no row materialise
path = client.flatfile_to_path("OPTION", "TRADE_QUOTE", "20260428",
                            "/tmp/option-trade-quote", format="csv")
```

The flat-file distribution serves a fixed set of datasets: option `trade_quote` / `open_interest` / `eod` and stock `trade_quote` / `eod`. Available `flat_files.*` methods: `option_trade_quote`, `option_open_interest`, `option_eod`, `stock_trade_quote`, `stock_eod`, plus `request(sec_type, req_type, date)`. The generic `request(...)` and `flatfile_to_path(...)` paths reject an unserved `(security, request)` pair with a typed invalid-parameter error.

## Endpoint coverage

65 typed endpoints across stocks, options, indices, the market calendar, and interest rates, plus real-time streaming.

| Category | Endpoints | Examples |
|---|---|---|
| Stock | 16 | EOD, OHLC, trades, quotes, snapshots, at-time |
| Option | 36 | Every stock surface plus five Greeks tiers, open interest, contract lists |
| Index | 9 | EOD, OHLC, price, snapshots |
| Calendar | 3 | Market open/close, holidays, early closes |
| Interest rate | 1 | EOD rate history |

Every endpoint is a method on `Client`. The full per-method list with signatures lives in the [API reference](https://userfrm.github.io/ThetaDataDx/reference/); `Config.dev()` and `Config.stage()` target the non-production environments.

## Errors

Every call raises a typed exception under a common `ThetaDataError` base — `AuthenticationError`, `RateLimitError`, `NotFoundError`, `DeadlineExceededError`, `InvalidParameterError`, and the rest — so the same cases are catchable here exactly as they are in every other binding.

## Documentation

- [Documentation site](https://userfrm.github.io/ThetaDataDx/) — getting started, API reference, streaming, flat files
- [Repository, issues, contributing](https://github.com/userFRM/ThetaDataDx)
- Community discussion on the [ThetaData Discord](https://discord.thetadata.us/)

## License

Licensed under the Apache License, Version 2.0.
