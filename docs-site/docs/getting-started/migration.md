---
title: Migration from the ThetaData Python SDK
description: Quick-reference side-by-side mapping from thetadata to thetadatadx — constructor, methods, DataFrame output, errors, streaming, async.
---

# Migration from the ThetaData Python SDK

Dropping into ThetaDataDx from `pip install thetadata` is mostly a one-line change at the constructor plus a single wrapper at the DataFrame boundary. This page covers the most common mappings. For the full guide — every endpoint kwarg delta, streaming setup, async migration — see [migration/from-thetadata-python-sdk](../migration/from-thetadata-python-sdk).

## Install side by side

Both libraries publish pre-built wheels to PyPI and coexist in the same venv:

```bash
pip install thetadata      # ThetaData Python SDK — keep for comparison runs
pip install thetadatadx    # ThetaDataDx — new primary client
```

## Constructor mapping

```python
# Before
from thetadata import Client
client = Client(email="user@example.com", password="hunter2")

# After
from thetadatadx import Credentials, Config, ThetaDataDx
creds = Credentials.from_file("creds.txt")   # or Credentials(email, password)
tdx = ThetaDataDx(creds, Config.production())
```

## Method mapping (representative)

| ThetaData Python SDK | ThetaDataDx |
|----------------------|-------------|
| `client.stock_history_eod("AAPL", start="20240101", end="20240301")` | `tdx.stock_history_eod("AAPL", "20240101", "20240301")` |
| `client.option_history_greeks_all(symbol, exp, strike, right, start, end)` | `tdx.option_history_greeks_all(symbol, exp, strike, right, start, end)` |
| `client.option_snapshot_quote(symbol, exp, strike, right)` | `tdx.option_snapshot_quote(symbol, exp, strike, right)` |
| `client.option_list_expirations(symbol)` | `tdx.option_list_expirations(symbol)` |

Most endpoint names match exactly. Known kwarg deltas (default venue / start-time / interval) are documented on the [full migration guide](../migration/from-thetadata-python-sdk).

## DataFrame output

```python
# Before — returns polars.DataFrame by default
df = client.stock_history_eod("AAPL", start="20240101", end="20240301")

# After — returns list[StockEodTick] by default
eod = tdx.stock_history_eod("AAPL", "20240101", "20240301")

# Add an explicit conversion if you want a DataFrame
from thetadatadx import to_polars, to_dataframe, to_arrow
pdf = to_polars(eod)     # polars.DataFrame
df  = to_dataframe(eod)  # pandas.DataFrame
tbl = to_arrow(eod)      # pyarrow.Table
```

The list-first default is a deliberate choice: callers who do not need columnar compute skip the Arrow allocation entirely, which matters at 176k+ rows (see [DataFrames](./dataframes) for the memory trade-off).

## Errors

```python
# Before
from thetadata import AuthenticationError, NoDataFoundError

try:
    df = client.stock_history_eod("AAPL", start=..., end=...)
except AuthenticationError:
    ...
except NoDataFoundError:
    ...

# After — more granular hierarchy
from thetadatadx import ThetaDataError, AuthError, RateLimitError, SubscriptionError

try:
    eod = tdx.stock_history_eod("AAPL", "20240101", "20240301")
except RateLimitError as e:
    time.sleep(e.wait_seconds)
except SubscriptionError as e:
    log.error("need tier %s for %s", e.required_tier, e.endpoint)
except AuthError:
    refresh_credentials()
except ThetaDataError:
    raise
```

See [Error handling](./errors) for every subclass.

## Streaming

The ThetaData Python SDK has no streaming. ThetaDataDx ships FPSS:

```python
tdx.start_streaming()
tdx.subscribe_quotes("AAPL")
while True:
    event = tdx.next_event(timeout_ms=1000)
    if event and event.kind == "quote":
        print(event.bid, event.ask)
```

See [Streaming](./streaming) for the full callback / polling / reconnect model.

## Async

The ThetaData Python SDK has no async. ThetaDataDx ships an `_async` variant of every historical endpoint:

```python
results = await asyncio.gather(
    tdx.stock_history_eod_async("AAPL", "20240101", "20240301"),
    tdx.stock_history_eod_async("MSFT", "20240101", "20240301"),
)
```

See [Async Python](./async-python).

## Next

- [Full migration guide](../migration/from-thetadata-python-sdk) — per-endpoint kwarg deltas, streaming / async migration, gotchas
- [Performance](./performance) — benchmark delta vs the ThetaData Python SDK
