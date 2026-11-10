---
title: Bulk Downloads
description: Download large history fast with the flow-control window, opt-in sharding, and buffered or streaming delivery.
---

# Bulk Downloads

Pulling a lot of history at once (a full option chain for a day, months of minute bars, every tick a liquid symbol printed) should be fast without rewriting the query. The SDK makes one ordinary query run at the full rate your account allows. Two things do the work:

1. A large flow-control window, on by default, so each stream runs at full speed instead of stalling.
2. Sharding, which you turn on, so one big query runs in parallel across your concurrent-request pool.

You write the same query either way. This page explains what each lever does, how to receive a large result, and what to expect.

## The flow-control window

HTTP/2 gives every stream a flow-control window: how much data the server may send before the client acknowledges it. The protocol default is small, 64 KB. On a fast history stream that window drains quicker than the round trip needed to refill it, so the server spends most of its time waiting for the client to say "keep going," and the stream runs at a fraction of the link's capacity.

The SDK raises this to 8 MB per stream (16 MB per connection) by default, large enough that the server almost never stalls on an acknowledgement. The cost is a few megabytes of buffer per stream.

Two fields control it (see [Configuration](/articles/configuration)):

- `stream_window_size_kb`, the per-stream window (default 8192).
- `connection_window_size_kb`, the whole-connection window (default 16384).

Raise them for even fatter streams if you have memory to spare, or lower them when you run many streams at once and want to cap total buffering.

## Automatic sharding

A single large request does not have to be one stream. The SDK splits the requested time (or date) range into equal concurrent pieces. The split comes straight from the shape of the request, with no sizing round-trip. It then runs them in parallel across your concurrent-request pool and reassembles them into exactly the rows one request would have returned.

A one-day option chain at a bar interval, such as `1s` or `1m`, is split by contract instead of by time: each bar builds up from the start of the requested window, so a piece starting mid-day would return different bars. A chain over every expiration first lists the day's contracts in one small request, then fetches each expiration as its own request, calls and puts separately. A chain with a single expiration splits into calls and puts. Either way, every contract's bars cover the whole window, exactly as one request returns them. Quote ticks over every expiration split the same way, because the server prepares the whole chain for every piece of a time split; a trade chain keeps the time split, since it is too sparse for one request per expiration to pay. An OHLC chain over every expiration, and a Greeks chain over every expiration at an interval under a minute, are ones the server refuses as a single request, and the SDK returns that same error rather than working around it.

The number of pieces running at once is the request-pool size, which defaults to your account's allowance: the SDK sizes the pool to your subscription tier at connect time (Free 1 / Value 2 / Standard 4 / Pro 8), and setting `max_concurrent_requests` overrides that with no client-side cap, because the server enforces the real allowance. See [Concurrent Requests](/articles/concurrent-requests).

On Pro, one big query runs eight pieces at once; a boosted account with `max_concurrent_requests = 32` runs 32. How much that shortens the wall clock depends on how much of the fan-out the server actually runs in parallel under current conditions. It scales down cleanly too: four at once on Standard, two on Value, one on Free.

Sharding is opt-in. Which ways a request can be split depends on what the server accepts, and a change on the server side could break a split before the SDK catches up, so every query is one request until you turn it on:

```python
from thetadatadx import Client, Config, Credentials

cfg = Config.production()
cfg.bulk_fetch = "auto"
client = Client(Credentials.from_file("creds.txt"), cfg)
```

```rust
use thetadatadx::config::{BulkFetchPolicy, DirectConfig};

let mut cfg = DirectConfig::production();
cfg.market_data.bulk_fetch = BulkFetchPolicy::Auto;
```

```typescript
const cfg = Config.production();
cfg.setBulkFetch("auto");
```

In a TOML config file, set `bulk_fetch = "auto"` under `[market_data]`. Small pulls stay a single request even then, so there is nothing to tune for them. If you want control:

- `bulk_fetch = "off"` (the default) runs every query as one request, in the server's own order.
- `shard_concurrency` caps how many pieces run at once (default: the full pool). Lower it to leave concurrency free for other requests running at the same time.

Sharding only fills the pool you configured. It never exceeds it.

## Buffered or streaming

There are two ways to receive a large result. Both use the large window, and both shard when bulk fetch is on. The difference is whether you get one finished frame or a sequence of chunks.

### Buffered: one finished result

The buffered terminal (`.list()` in Python, `.await` in Rust) returns the whole result as one typed frame, a single ordered result you can save, convert, or index. Use it when you want the finished dataset in hand and it fits in memory.

```python
lst = (client.market_data
    .option_history_quote_builder("SPXW", "20260710").date("20260710")
    .strike("*").right("both").interval("tick")
    .list())

df = lst.to_polars()          # or .to_pandas() / .to_arrow() / .to_list()
```

```rust
let ticks = client
    .option_history_quote("SPXW", "20260710").date("20260710")
    .strike("*").right("both").interval("tick")
    .await?;
```

### Streaming: chunks as they arrive

The streaming terminal (`.stream(handler)`) hands you the result in chunks as each piece produces them, so you never hold the whole dataset in memory. It skips building and ordering the full frame, which makes it the path for results too large to hold whole.

```python
rows = 0
def on_chunk(chunk):
    global rows
    rows += len(chunk)

(client.market_data
    .option_history_quote_builder("SPXW", "20260710").date("20260710")
    .strike("*").right("both").interval("tick")
    .stream(on_chunk))
```

```rust
client
    .option_history_quote("SPXW", "20260710").date("20260710")
    .strike("*").right("both").interval("tick")
    .stream(|chunk| { /* handle chunk: it derefs to &[QuoteTick] */ })
    .await?;
```

Because pieces stream in parallel, chunks from different pieces interleave. Sort on your side if you need a specific order.

### Which to use

Both shard and window the same way. Buffered spends extra client-side work assembling the complete ordered frame; streaming skips that. Pick by the shape of the job:

- Buffered when you want the finished dataset, to save it, convert it, or work with it whole, and it fits in memory.
- Streaming when the result may not fit in memory, or you want to process rows as they arrive. This is the path for the largest pulls.

## The same rows either way

With bulk fetch on, a pull returns the same rows as one request. A concrete single contract comes back byte-for-byte identical to a single request. A full chain comes back in a deterministic canonical order (ascending expiration, then strike, then calls before puts, in trading-time order within each contract). Set `bulk_fetch = "off"` if you want the server's own ordering instead.

## Configuration reference

| Field | Default | What it does |
|---|---|---|
| `bulk_fetch` | `"off"` | `"auto"` splits large pulls across the request pool; `"off"` runs one request in server order. |
| `max_concurrent_requests` | your tier | Concurrent-request pool size. Defaults to your subscription tier's allowance; set it to a boosted allowance to go wider (the server enforces the real limit). |
| `shard_concurrency` | full pool | Caps pieces per pull. Lower to share concurrency with other requests. |
| `stream_window_size_kb` | `8192` | Per-stream HTTP/2 flow-control window, in KB. |
| `connection_window_size_kb` | `16384` | Whole-connection flow-control window, in KB. |

See [Configuration](/articles/configuration) for setting these on each language binding, and [Concurrent Requests](/articles/concurrent-requests) for running many separate requests in parallel.
