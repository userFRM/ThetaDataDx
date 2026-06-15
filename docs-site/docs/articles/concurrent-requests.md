---
title: Concurrent Requests
description: How many historical requests run in parallel per subscription tier, and how to use that budget.
---

# Concurrent Requests

Your requests are not rate-limited, but the number of requests **in flight at once** is capped by your subscription tier on each asset class:

| Tier | Concurrent requests |
|---|---:|
| Free | 1 |
| Value | 2 |
| Standard | 4 |
| Pro | 8 |

## What the SDK does for you

Every historical call acquires a permit from a client-side semaphore sized to your tier's cap, detected at connect time. Fire as many calls in parallel as you like — async tasks in Rust and Python, threads, a process-wide backfill loop — and anything beyond the cap **queues in order and drains as slots free**. You never receive a rejection for over-parallelism; the burst is absorbed as latency, not errors.

That means the idiomatic pattern is simply to launch your whole batch:

```python
import asyncio
from thetadatadx import AsyncClient

tdx = AsyncClient.from_file("creds.txt")

async def pull(day):
    return await tdx.historical.stock_history_trade_async("AAPL", day)

results = asyncio.run(asyncio.gather(*(pull(d) for d in days)))
```

With a Pro subscription, eight of those requests run concurrently and the rest wait their turn.

## Tuning the cap down (or trying to exceed it)

`concurrent_requests` on the configuration object overrides the auto-detected value — useful for capping a shared account's footprint from one process. Values above your tier cap are clamped back to the cap at connect time, with a single warning log naming both numbers, because the servers reject the excess anyway.

```python
cfg = Config.production()
cfg.concurrent_requests = 2   # be a polite tenant on a shared account
```

## When parallelism pays

Concurrency multiplies throughput on multi-request workloads: per-day backfills, per-contract chain pulls, anything you can split with `split_date_range`. It does nothing for one giant request — split the request first ([Request Sizing](/articles/request-sizing)), then spend your tier's slots on the pieces.

One more queue exists upstream: if the servers themselves report exhaustion, the SDK retries with backoff before surfacing an error. Long-running bulk jobs should still expect occasional retries during peak hours; see [Data Issues?](/articles/data-issues) if a job stalls beyond that.
