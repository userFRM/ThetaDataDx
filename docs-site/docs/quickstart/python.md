---
title: Python Quickstart
description: Install, authenticate, run a historical call, subscribe to streaming, and handle errors with ThetaDataDx in Python.
---

# Python Quickstart

`pip install thetadatadx` — abi3 wheels for Python 3.9+. No Rust toolchain required on supported platforms.

## Install

```bash
pip install thetadatadx

# With DataFrame support:
pip install thetadatadx[pandas]   # pandas
pip install thetadatadx[polars]   # polars
pip install thetadatadx[arrow]    # pyarrow only
pip install thetadatadx[all]      # all three
```

## Authenticate

```python
from thetadatadx import Credentials

# From file
creds = Credentials.from_file("creds.txt")

# Or from env vars
import os
creds = Credentials(os.environ["THETA_EMAIL"], os.environ["THETA_PASS"])
```

## Connect

```python
from thetadatadx import Config, ThetaDataDx

tdx = ThetaDataDx(creds, Config.production())
```

## Historical call

```python
from thetadatadx import Credentials, Config, ThetaDataDx

creds = Credentials.from_file("creds.txt")
tdx = ThetaDataDx(creds, Config.production())

eod = tdx.stock_history_eod("AAPL", "20240101", "20240301")
for tick in eod:
    print(f"{tick.date}: O={tick.open:.2f} H={tick.high:.2f} "
          f"L={tick.low:.2f} C={tick.close:.2f} V={tick.volume}")
```

Convert to a DataFrame when you need one:

```python
from thetadatadx import to_polars, to_dataframe, to_arrow

pdf = to_polars(eod)     # polars.DataFrame
df  = to_dataframe(eod)  # pandas.DataFrame
tbl = to_arrow(eod)      # pyarrow.Table
```

See the [DataFrames page](../getting-started/dataframes) for the zero-copy scope.

## Streaming call

```python
from thetadatadx import Credentials, Config, ThetaDataDx

creds = Credentials.from_file("creds.txt")
tdx = ThetaDataDx(creds, Config.production())

tdx.start_streaming()
tdx.subscribe_quotes("AAPL")
tdx.subscribe_trades("MSFT")

try:
    while True:
        event = tdx.next_event(timeout_ms=1000)
        if event is None:
            continue
        if event.kind == "quote":
            print(f"Quote: {event.contract_id} "
                  f"{event.bid:.2f}/{event.ask:.2f}")
        elif event.kind == "trade":
            print(f"Trade: {event.contract_id} "
                  f"{event.price:.2f} x {event.size}")
        elif event.kind == "simple" and event.event_type == "disconnected":
            break
finally:
    tdx.stop_streaming()
```

## Error handling

```python
from thetadatadx import (
    ThetaDataError, AuthError, RateLimitError, SubscriptionError,
)
import time

try:
    ticks = tdx.option_history_greeks_all(
        "SPY", "20240419", "500", "C", "20240101", "20240301",
    )
except RateLimitError as e:
    time.sleep(e.wait_seconds)
    # retry
except SubscriptionError as e:
    log.error("endpoint %s requires %s", e.endpoint, e.required_tier)
except AuthError:
    refresh_credentials()
except ThetaDataError:
    raise
```

## Async variants

Every historical endpoint ships an `_async` variant that yields to the event loop:

```python
import asyncio

async def main():
    tasks = [
        tdx.stock_history_eod_async(sym, "20240101", "20240301")
        for sym in ["AAPL", "MSFT", "GOOGL", "AMZN"]
    ]
    results = await asyncio.gather(*tasks)

asyncio.run(main())
```

See [Async Python](../getting-started/async-python) for subscription-cap-safe concurrency patterns.

## Next

- [Historical data](../historical/) — 61 endpoints
- [DataFrames](../getting-started/dataframes) — Arrow / polars / pandas conversion
- [Streaming (FPSS)](../streaming/) — event types, reconnect, latency measurement
- [Migration from the ThetaData Python SDK](../migration/from-thetadata-python-sdk) — one-to-one call mapping
