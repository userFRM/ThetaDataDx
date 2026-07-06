---
title: Concurrent Requests
description: How many historical requests run in parallel per subscription tier.
---

# Concurrent Requests

Your requests are not rate-limited, but the number of **concurrent** historical requests is capped by your subscription tier. Concurrency is account-wide and set by your **highest** subscription tier across asset classes:

| Tier | Concurrent requests |
|---|---:|
| Free | 1 |
| Value | 2 |
| Standard | 4 |
| Pro | 8 |

## Fire your whole batch

You can issue more requests than your tier allows. The extra requests are **queued and run in order**, so a burst completes as fast as your tier permits without you managing anything. There is nothing to configure. The idiomatic pattern is to launch the whole batch and let it run at your tier's rate:

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

Concurrency multiplies throughput on multi-request workloads: per-day backfills, per-contract chain pulls, anything you can split with `split_date_range`. It does nothing for one giant request, so split the request first ([Request Sizing](/articles/request-sizing)), then let your tier's capacity work through the pieces.

If the service reports exhaustion during peak hours, the SDK retries with backoff before surfacing an error. Long-running bulk jobs should expect occasional retries at peak; see [Data Issues?](/articles/data-issues) if a job stalls beyond that.
