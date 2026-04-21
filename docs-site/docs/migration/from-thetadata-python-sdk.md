---
title: Migration from the ThetaData Python SDK
description: Full migration guide from thetadata (pip) to thetadatadx — install, constructor, method mapping, DataFrame output, streaming, async, errors.
---

# Migration from the ThetaData Python SDK

This page covers the full migration path from `pip install thetadata` to `pip install thetadatadx`. For a pointer-sized summary, see [Migration quick-reference](../getting-started/migration).

## Install side by side

Both libraries publish pre-built wheels to PyPI and coexist in the same venv. Running them side-by-side is the easiest way to validate a migration — swap out one call site at a time and diff the result shapes:

```bash
pip install thetadata      # ThetaData Python SDK
pip install thetadatadx    # ThetaDataDx
```

Pin the versions in `requirements.txt` while you migrate so a `pip install -U` does not shift both at once.

## Constructor

```python
# Before
from thetadata import Client
client = Client(email="user@example.com", password="hunter2")

# After
from thetadatadx import Credentials, Config, ThetaDataDx

creds = Credentials.from_file("creds.txt")           # or Credentials(email, password)
tdx = ThetaDataDx(creds, Config.production())
```

Notable differences:

- ThetaDataDx splits credentials and configuration into two objects. The `Credentials` handle holds the email/password (both wrapped in `zeroize::Zeroizing` buffers); `Config` holds timeouts, server targets, and concurrency limits.
- ThetaDataDx defaults to the Nexus production auth endpoint and MDDS/FPSS production hosts. `Config.dev()` points at the dev-replay servers (infinite historical day replay, markets closed).
- ThetaDataDx does not require the Java terminal to be running. It connects directly to Nexus and MDDS/FPSS.

## Method mapping — stock

| ThetaData Python SDK | ThetaDataDx | Notes |
|----------------------|-------------|-------|
| `client.stock_list_symbols()` | `tdx.stock_list_symbols()` | Same shape. |
| `client.stock_list_dates(symbol)` | `tdx.stock_list_dates(symbol)` | |
| `client.stock_snapshot_ohlc(symbol)` | `tdx.stock_snapshot_ohlc(symbol)` | |
| `client.stock_snapshot_trade(symbol)` | `tdx.stock_snapshot_trade(symbol)` | |
| `client.stock_snapshot_quote(symbol)` | `tdx.stock_snapshot_quote(symbol)` | |
| `client.stock_history_eod(symbol, start=..., end=...)` | `tdx.stock_history_eod(symbol, start, end)` | Positional in ThetaDataDx. Dates are `"YYYYMMDD"` strings. |
| `client.stock_history_ohlc(symbol, start=..., end=..., interval=...)` | `tdx.stock_history_ohlc(symbol, start, end, interval)` | `interval` is ms in both. |
| `client.stock_history_trade(symbol, date=...)` | `tdx.stock_history_trade(symbol, date)` | |
| `client.stock_history_quote(symbol, date=..., interval=...)` | `tdx.stock_history_quote(symbol, date, interval)` | |
| `client.stock_at_time_trade(symbol, start=..., end=..., time_of_day=...)` | `tdx.stock_at_time_trade(symbol, start, end, time_of_day)` | `time_of_day` is `"HH:MM:SS.SSS"` ET wall-clock in ThetaDataDx. Legacy ms-strings (`"34200000"`) are also accepted. |

## Method mapping — option

| ThetaData Python SDK | ThetaDataDx | Notes |
|----------------------|-------------|-------|
| `client.option_list_roots()` | `tdx.option_list_roots()` | |
| `client.option_list_dates(symbol)` | `tdx.option_list_dates(symbol)` | |
| `client.option_list_expirations(symbol)` | `tdx.option_list_expirations(symbol)` | |
| `client.option_list_strikes(symbol, exp)` | `tdx.option_list_strikes(symbol, exp)` | |
| `client.option_list_contracts(date)` | `tdx.option_list_contracts(date)` | |
| `client.option_snapshot_quote(symbol, exp, strike, right)` | `tdx.option_snapshot_quote(symbol, exp, strike, right)` | `right` accepts `"C"`/`"P"`/`"call"`/`"put"`/`"both"`/`"*"`, case-insensitive. |
| `client.option_history_greeks_all(symbol, exp, strike, right, start, end)` | `tdx.option_history_greeks_all(symbol, exp, strike, right, start, end)` | |
| `client.option_history_eod(symbol, exp, strike, right, start, end)` | `tdx.option_history_eod(symbol, exp, strike, right, start, end)` | Wildcard `"0"` on `exp` or `strike` returns a chain. |

Full 61-endpoint listing on the [API Reference](../api-reference). The core surface is 1:1; where ThetaDataDx adds kwargs, the new kwargs default to the ThetaData Python SDK's behavior so existing callers migrate without changing arguments.

## DataFrame output

The biggest semantic difference. ThetaDataDx returns `list[TickClass]` by default; conversion to a DataFrame is an explicit call.

```python
# Before — returns polars.DataFrame by default, pandas with a kwarg
df = client.stock_history_eod("AAPL", start="20240101", end="20240301")
pdf = client.stock_history_eod("AAPL", start="20240101", end="20240301",
                                dataframe="pandas")

# After — returns list[StockEodTick] by default
eod = tdx.stock_history_eod("AAPL", "20240101", "20240301")

# Explicit conversion
from thetadatadx import to_polars, to_dataframe, to_arrow
pdf = to_polars(eod)     # polars.DataFrame
df  = to_dataframe(eod)  # pandas.DataFrame
tbl = to_arrow(eod)      # pyarrow.Table
```

Why the list default:

- 176k-row result: the pyclass list uses 19 MB RSS. The arrow conversion allocates another 61 MB.
- Many callers iterate the result and never materialize a DataFrame — paying the Arrow allocation on every call wastes RAM.
- Callers who do want columnar compute pay exactly once, at a point they control.

See the [DataFrames page](../getting-started/dataframes) for the zero-copy scope.

### Row-level access differences

```python
# ThetaData Python SDK — index by column name on a polars.DataFrame
for row in df.iter_rows(named=True):
    print(row["date"], row["close"])

# ThetaDataDx — attribute access on a typed tick object
for tick in eod:
    print(tick.date, tick.close)

# ThetaDataDx + DataFrame — same polars access
pdf = to_polars(eod)
for row in pdf.iter_rows(named=True):
    print(row["date"], row["close"])
```

## Streaming

The ThetaData Python SDK has no streaming. If you are polling `*_snapshot_*` endpoints in a loop to approximate real-time, switch to FPSS:

```python
# Before — polling snapshot in a loop
while True:
    q = client.stock_snapshot_quote("AAPL")
    print(q["bid"], q["ask"])
    time.sleep(0.5)

# After — native FPSS streaming with SPKI pinning
tdx.start_streaming()
tdx.subscribe_quotes("AAPL")
while True:
    event = tdx.next_event(timeout_ms=1000)
    if event and event.kind == "quote":
        print(event.bid, event.ask)
```

See [Streaming (FPSS)](../streaming/) for the full model.

## Async

The ThetaData Python SDK has no async. Every endpoint in ThetaDataDx has an `_async` variant:

```python
# Before — sequential blocking calls
results = [client.stock_history_eod(s, start=..., end=...) for s in symbols]

# After — concurrent async calls
import asyncio
results = await asyncio.gather(*[
    tdx.stock_history_eod_async(s, "20240101", "20240301") for s in symbols
])
```

See [Async Python](../getting-started/async-python) for subscription-cap-safe concurrency patterns.

## Error mapping

```python
# Before
from thetadata import AuthenticationError, NoDataFoundError

try:
    df = client.stock_history_eod("AAPL", start=..., end=...)
except AuthenticationError:
    refresh_credentials()
except NoDataFoundError:
    df = polars.DataFrame()
```

```python
# After
from thetadatadx import (
    ThetaDataError, AuthError, RateLimitError, SubscriptionError,
    EndpointNotFoundError, SchemaMismatchError, NetworkError,
)

try:
    eod = tdx.stock_history_eod("AAPL", "20240101", "20240301")
except AuthError:
    refresh_credentials()
except EndpointNotFoundError:
    eod = []
except RateLimitError as e:
    time.sleep(e.wait_seconds)
    # retry
except SubscriptionError as e:
    log.error("need tier %s for %s", e.required_tier, e.endpoint)
except ThetaDataError:
    raise
```

Mapping between the two exception sets:

| ThetaData Python SDK | ThetaDataDx | Notes |
|----------------------|-------------|-------|
| `AuthenticationError` | `AuthError` | Same semantics: bad credentials or expired session. |
| `NoDataFoundError` | `EndpointNotFoundError` **or** empty list | ThetaDataDx returns an empty list when the server returns zero rows on an otherwise-valid query; it raises `EndpointNotFoundError` only when the server returns `NotFound`. |
| *(none)* | `RateLimitError` | Explicit surface on `TooManyRequests` (code 12). |
| *(none)* | `SubscriptionError` | Raised on `PermissionDenied`. |
| *(none)* | `SchemaMismatchError` | Raised when the decoder cannot unpack the response. |

## Gotchas

- **`right` parameter on option endpoints.** Accepts `"C"`, `"P"`, `"call"`, `"put"`, `"both"`, `"*"` (case-insensitive). Unlike `exp`/`strike`, it does **not** accept `"0"` as a wildcard — use `"both"` or `"*"`.
- **Strikes are dollar values.** ThetaDataDx v3 emits strikes as `float` dollars (e.g. `500.0`), not scaled integers. If your downstream code multiplied by 1000, drop the multiplication.
- **Dates as strings.** `"YYYYMMDD"` everywhere — no `datetime.date` coercion. Pass `"20240101"`, not `date(2024, 1, 1)`.
- **Time-of-day format.** `"HH:MM:SS.SSS"` ET wall-clock. Legacy ms strings (`"34200000"`) also work but are deprecated.
- **Strike range semantics.** `strike="500"` + `strike_range=5` still targets the single 500-strike contract. `strike="0"` + `strike_range=5` returns a spot-relative range. Read [Options & Greeks — wildcard queries](../options#wildcard-queries-with-contract-identification) before porting any chain-scan code.

## Validation checklist

After migrating, run both libraries side-by-side on a representative workload and compare:

1. Row counts match (intersect columns by name first — column sets differ by design).
2. Numeric columns match within `atol=1e-6`.
3. Peak RSS is smaller on ThetaDataDx (expect 3–75× depending on endpoint).
4. Wall time is smaller on ThetaDataDx for bulk endpoints (expect 1.5–9× depending on endpoint).

The external benchmark harness at `userFRM/thetadata-bench-v2` ships a `harness/integrity.py` doing exactly this comparison — point it at your own subscription and endpoints.

## Next

- [Performance](../performance/benchmark) — full benchmark matrix
- [Error handling](../getting-started/errors) — retry patterns for `ThetaDataError` subclasses
- [Async Python](../getting-started/async-python) — fan-out under subscription caps
