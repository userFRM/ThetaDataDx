---
title: Async Python
description: asyncio.gather over ThetaDataDx async endpoint methods with subscription-cap concurrency.
---

# Async Python

The ThetaData Python SDK is synchronous-only. Every endpoint call blocks the calling thread until the response is decoded. ThetaDataDx exposes an `async def` variant of every historical endpoint (`_async` suffix) that shares the internal tokio runtime used by the sync path, so `asyncio.gather` over an `async`-awaitable fan-out is the idiomatic shape for dashboards, backtests, and multi-symbol pulls.

## Minimal gather

Pull four symbols concurrently under the STANDARD-tier stock cap (four concurrent requests):

```python
import asyncio
from thetadatadx import Credentials, Config, ThetaDataDx

async def main():
    creds = Credentials.from_file("creds.txt")
    tdx = ThetaDataDx(creds, Config.production())

    symbols = ["AAPL", "MSFT", "GOOGL", "AMZN"]
    tasks = [
        tdx.stock_history_eod_async(sym, "20240101", "20240301")
        for sym in symbols
    ]
    results = await asyncio.gather(*tasks)

    for sym, eod in zip(symbols, results):
        print(f"{sym}: {len(eod)} trading days")

asyncio.run(main())
```

The four calls execute concurrently on the shared tokio runtime. Network time overlaps, decode runs on Rust threads, and the Python coroutine wakes up when each result is ready.

## Respecting subscription caps

ThetaData enforces per-subscription concurrency caps at the server side. The MDDS gateway rejects requests beyond the cap with a `TooManyRequests` (code 12) error that costs a 130-second backoff. The defaults:

| Tier | Stock concurrent | Option concurrent |
|------|-----------------:|------------------:|
| Free | 1 | 1 |
| Value | 2 | 2 |
| Standard | 4 | 4 |
| Pro | 8 | 8 |

The SDK auto-detects the cap from your subscription tier and exposes it as `mdds_concurrent_requests` on `DirectConfig`. For large fan-outs, gate the gather with a semaphore:

```python
import asyncio
from thetadatadx import Credentials, Config, ThetaDataDx

async def pull_eod(tdx, sem, sym):
    async with sem:
        return await tdx.stock_history_eod_async(sym, "20240101", "20240301")

async def main():
    creds = Credentials.from_file("creds.txt")
    tdx = ThetaDataDx(creds, Config.production())

    sem = asyncio.Semaphore(4)   # STANDARD tier stock cap
    symbols = [s for s in load_universe()]   # say 500 names
    tasks = [pull_eod(tdx, sem, sym) for sym in symbols]
    results = await asyncio.gather(*tasks)

    # process results...
```

## Retries inside gather

Wrap individual calls in a retry context so one symbol's rate-limit or network blip does not fail the whole gather:

```python
import asyncio
from tenacity import retry, retry_if_exception_type, wait_exponential, stop_after_attempt
from thetadatadx import Credentials, Config, ThetaDataDx, RateLimitError, NetworkError

@retry(
    retry=retry_if_exception_type((RateLimitError, NetworkError)),
    wait=wait_exponential(multiplier=1, min=2, max=60),
    stop=stop_after_attempt(5),
)
async def pull(tdx, sym):
    return await tdx.stock_history_eod_async(sym, "20240101", "20240301")

async def main():
    creds = Credentials.from_file("creds.txt")
    tdx = ThetaDataDx(creds, Config.production())
    tasks = [pull(tdx, s) for s in load_universe()]
    results = await asyncio.gather(*tasks, return_exceptions=True)
    for sym, res in zip(load_universe(), results):
        if isinstance(res, Exception):
            log.error("%s failed: %s", sym, res)
```

## Mixing with streaming

`tdx.start_streaming()` returns synchronously and dispatches events on a dedicated I/O thread. The async historical calls share the same client — issue historical pulls from coroutines while events arrive on the polling channel:

```python
async def watch_stream(tdx, stop_event):
    loop = asyncio.get_running_loop()
    while not stop_event.is_set():
        event = await loop.run_in_executor(None, tdx.next_event, 1000)
        if event and event.kind == "trade":
            print(f"Trade: {event.contract_id} {event.price}")

async def main():
    creds = Credentials.from_file("creds.txt")
    tdx = ThetaDataDx(creds, Config.production())
    tdx.start_streaming()
    tdx.subscribe_trades("AAPL")

    stop = asyncio.Event()
    hist_task = asyncio.create_task(pull_history(tdx))
    stream_task = asyncio.create_task(watch_stream(tdx, stop))

    await hist_task
    stop.set()
    await stream_task
    tdx.stop_streaming()
```

## Under the hood

Each `_async` method is a thin wrapper emitted alongside the sync method from `endpoint_surface.toml`. The wrapper submits the gRPC future to the shared tokio runtime through `pyo3-async-runtimes` and yields to the Python event loop until the future resolves. No extra thread pool, no per-call runtime spin-up, no GIL contention while the decode runs in Rust.

## Next

- [Error handling](./errors) — retry patterns for `RateLimitError` inside a gather
- [Performance](./performance) — concurrency scaling expectations
