---
title: Bulk Backfill
description: Pull months of tick data with bounded memory and full tier concurrency.
---

# Bulk Backfill

Two tools make large pulls civilized: per-day request splitting with your tier's [concurrency](/articles/concurrent-requests), and chunk-streaming decode so memory stays flat no matter the response size.

## Python — concurrent days, streamed decode

```python
import asyncio

from thetadatadx import Config, Credentials, Client, split_date_range

tdx = Client(Credentials.from_file("creds.txt"), Config.production())

def on_chunk(chunk):
    # chunk: list of TradeTick — write to your store, then it is freed.
    store.append(chunk)

async def pull(start, end):
    builder = tdx.stock_history_trade_builder("AAPL", start).end_date(end)
    await builder.stream_async(on_chunk)

windows = split_date_range("20250101", "20250331")

async def main():
    await asyncio.gather(*(pull(s, e) for s, e in windows))

asyncio.run(main())
```

Every endpoint has a `<endpoint>_builder(...)` factory whose `.stream(...)` / `.stream_async(...)` terminals hand each decoded chunk to your callback and free it before fetching the next.

## Rust — the same shape

```rust
let days = ["20250303", "20250304", "20250305"];
for day in days {
    tdx.stock_history_trade("AAPL", day)
        .stream(|chunk| {
            // &[TradeTick] — persist, then the chunk is dropped.
            write_parquet(chunk);
        })
        .await?;
}
```

Run several days concurrently with `futures::future::join_all` — the SDK's tier semaphore paces them.

## When to stop looping

A per-symbol loop over the whole market is the wrong tool past a handful of symbols — that's what [flat files](/articles/flat-files) are for: one request returns every contract for a date.
