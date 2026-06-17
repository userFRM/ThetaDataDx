---
title: Concurrent Requests
description: How many historical requests run in parallel per subscription tier.
---

# Concurrent Requests

Your requests are not rate-limited, but the number of historical requests **in flight at once** is capped by your subscription tier on each asset class:

| Tier | Concurrent requests |
|---|---:|
| Free | 1 |
| Value | 2 |
| Standard | 4 |
| Pro | 8 |

## It's automatic

There is no knob to set. At connect time the SDK reads your tier from authentication and sizes its historical connection pool to match. A Pro account gets eight in-flight slots, Free gets one, and so on. You don't pass a value, you don't tune anything, and there's nothing to get wrong.

Every historical call quietly takes a slot from that pool. Fire as many calls in parallel as you like, whether that's async tasks in Rust and Python, threads, or a process-wide backfill loop. Anything beyond your tier's slot count **queues in order and drains as slots free**. You never get a rejection for over-parallelism; the burst is absorbed as latency, not errors.

So the idiomatic pattern is simply to launch your whole batch and let the pool pace it:

```python
import asyncio
from thetadatadx import AsyncClient

client = AsyncClient.from_file("creds.txt")

async def pull(day):
    return await client.stock_history_trade_async("AAPL", day)

results = asyncio.run(asyncio.gather(*(pull(d) for d in days)))
```

With a Pro subscription, eight of those requests run concurrently and the rest wait their turn. On Free, they run one at a time. Same code either way.

## When parallelism pays

Concurrency multiplies throughput on multi-request workloads: per-day backfills, per-contract chain pulls, anything you can split with `split_date_range`. It does nothing for one giant request, so split the request first ([Request Sizing](/articles/request-sizing)), then let your tier's slots work through the pieces.

One more queue exists upstream: if the servers themselves report exhaustion, the SDK retries with backoff before surfacing an error. Long-running bulk jobs should still expect occasional retries during peak hours; see [Data Issues?](/articles/data-issues) if a job stalls beyond that.
</content>
</invoke>
