---
title: Concurrent Requests
description: How many market-data requests run in parallel, and how to size the SDK's request pool to your account's allowance.
---

# Concurrent Requests

Your requests are not rate-limited, but the number of **concurrent** market-data requests is capped by your account's allowance, enforced server-side. The allowance is account-wide and set by your **highest** subscription tier across asset classes:

| Tier | Concurrent requests |
|---|---:|
| Free | 1 |
| Value | 2 |
| Standard | 4 |
| Pro | 8 |

The SDK sizes its request pool to that allowance automatically: at connect time it reads your tier from the auth response and defaults the pool to the matching width, so a Pro account runs 8 concurrent requests and a Value account runs 2 with nothing configured.

Accounts can also carry a **boosted** allowance above their base tier (for example 32 concurrent on a boosted Pro account). The SDK does not cap concurrency on its own — the server enforces the real allowance — so set `max_concurrent_requests` to the boosted number and the pool actually runs that wide:

```python
from thetadatadx import Client, Credentials, Config

cfg = Config.production()
cfg.max_concurrent_requests = 32   # boosted allowance; leave unset to match your tier

client = Client(Credentials.from_file("creds.txt"), cfg)
```

The same knob is `config.market_data.max_concurrent_requests` in Rust, `cfg.setMaxConcurrentRequests(32)` in TypeScript, and `cfg.set_max_concurrent_requests(32)` in C++. An explicit value always wins over the tier default, in either direction — you can also set it below your allowance to reserve concurrency for another process. Requests past your account's real allowance are rejected by the server and retried with backoff before surfacing an error.

## Fire your whole batch

You can issue more requests than your pool holds. The extra requests are **queued and run in order**, so a burst completes as fast as your allowance permits without you managing anything. The idiomatic pattern is to launch the whole batch and let it run:

```python
import asyncio
from thetadatadx import AsyncClient

client = AsyncClient.from_file("creds.txt")

async def pull(day):
    return await client.stock_history_trade_async("AAPL", day)

results = asyncio.run(asyncio.gather(*(pull(d) for d in days)))
```

With a pool of eight, eight of those requests run concurrently and the rest wait their turn. With a pool of one, they run one at a time. Same code either way.

### Any client, same concurrency

The batch pattern is not exclusive to `AsyncClient`. Every market-data endpoint has an `*_async` companion on every client that serves market data: `AsyncClient` is the all-async ergonomic wrapper, while `MarketDataClient` is the leaner market-data-only path (it never opens a streaming channel) and the unified `Client` carries both surfaces. On the latter two the same awaitables live on the `.market_data` view, and `asyncio.gather` works identically because all of them dispatch through the same request pool:

```python
from thetadatadx import MarketDataClient

mdc = MarketDataClient.from_file("creds.txt")

quotes = await asyncio.gather(*(
    mdc.market_data.option_history_quote_async("SPXW", exp, date=day)
    for exp in expirations
))
```

Pick the client by what else you need — streaming, sync calls, a smaller surface — not by whether you want concurrency. You always have it.

## One giant request, split for you

The pattern above parallelizes work you have already split into many requests. The SDK also does the reverse for you: a **single** large history request has its time or date range split into equal pieces, run in parallel across the request pool, and reassembled into exactly the rows one request would have returned. You write one ordinary query and it runs at your configured concurrency. This is on by default.

See [Bulk Downloads](/articles/bulk-downloads) for the full picture: how the split works, buffered versus streaming delivery, the `bulk_fetch` and `shard_concurrency` knobs, API examples, and measured performance.

## When parallelism pays

Concurrency multiplies throughput on multi-request workloads: per-day backfills, per-contract chain pulls, anything you can split with `split_date_range`. For a single large request you no longer have to split it yourself, since `bulk_fetch` does it automatically ([above](#one-giant-request-split-for-you)); splitting manually still works when you want direct control over the pieces.

If the service reports exhaustion — because a burst exceeded your account's allowance, or during peak hours — the SDK retries with backoff before surfacing an error. Long-running bulk jobs should expect occasional retries at peak; see [Data Issues?](/articles/data-issues) if a job stalls beyond that.
