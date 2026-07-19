---
title: Bulk Downloads
description: Download large history fast with the flow-control window, automatic sharding, and buffered or streaming delivery.
---

# Bulk Downloads

Pulling a lot of history at once (a full option chain for a day, months of minute bars, every tick a liquid symbol printed) should be fast without tuning anything. The SDK makes one ordinary query run at the full rate your subscription allows. Two things do the work, both on by default:

1. A large flow-control window, so each stream runs at full speed instead of stalling.
2. Automatic sharding, so one big query runs in parallel across your tier's concurrency.

You write the same query either way. This page explains what each lever does, how to receive a large result, and what to expect.

## The flow-control window

HTTP/2 gives every stream a flow-control window: how much data the server may send before the client acknowledges it. The protocol default is small, 64 KB. On a fast history stream that window drains quicker than the round trip needed to refill it, so the server spends most of its time waiting for the client to say "keep going," and the stream runs at a fraction of the link's capacity.

The SDK raises this to 8 MB per stream (16 MB per connection) by default, large enough that the server almost never stalls on an acknowledgement. On a full-day options chain this one change is about a 1.6x speedup over the protocol default, for a few megabytes of buffer per stream.

Two fields control it (see [Configuration](/articles/configuration)):

- `stream_window_size_kb`, the per-stream window (default 8192).
- `connection_window_size_kb`, the whole-connection window (default 16384).

Raise them for even fatter streams if you have memory to spare, or lower them when you run many streams at once and want to cap total buffering.

## Automatic sharding

A single large request does not have to be one stream. The SDK splits the requested time (or date) range into equal concurrent pieces — straight from the shape of the request, with no sizing round-trip — runs them in parallel across your tier's concurrent-request budget, and reassembles them into exactly the rows one request would have returned.

The number of pieces is your tier's concurrency:

| Tier | Pieces |
|---|---:|
| Free | 1 |
| Value | 2 |
| Standard | 4 |
| Pro | 8 |

On Pro, one big query becomes eight pieces fetching at once. How much that shortens the wall clock depends on how much of the fan-out the server actually runs in parallel under current conditions; the table below shows one measured day. It scales down cleanly with tier: four pieces on Standard, two on Value, one on Free.

Sharding is on by default. Small pulls stay a single request, so there is nothing to tune for them. If you want control:

- `bulk_fetch = "off"` runs every query as one request, in the server's own order.
- `shard_concurrency` caps how many pieces run at once (default: your full tier budget). Lower it to leave concurrency free for other requests running at the same time.

Sharding only fills the concurrency your tier already grants. It never exceeds it.

## Buffered or streaming

There are two ways to receive a large result, and both are sharded and windowed. The difference is whether you get one finished frame or a sequence of chunks.

### Buffered: one finished result

The buffered terminal (`.list()` in Python, `.await` in Rust) returns the whole result as one typed frame, a single ordered result you can save, convert, or index. Use it when you want the finished dataset in hand and it fits in memory.

```python
from thetadatadx import Client, Credentials, Config

client = Client(Credentials.from_file("creds.txt"), Config.production())

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
    .stream(|chunk: &[QuoteTick]| { /* handle chunk */ })
    .await?;
```

Because pieces stream in parallel, chunks from different pieces interleave. Sort on your side if you need a specific order.

### Which to use

Both shard and window the same way. Buffered spends extra client-side work assembling the complete ordered frame; streaming skips that. Pick by the shape of the job:

- Buffered when you want the finished dataset, to save it, convert it, or work with it whole, and it fits in memory.
- Streaming when the result may not fit in memory, or you want to process rows as they arrive. This is the path for the largest pulls.

## What it adds up to

Measured on a full-day SPXW options chain (125,849,342 rows, `strike="*"`, `right="both"`, tick interval) on a Pro account, using the buffered path. Absolute times depend on your tier, your distance to the server, current load, and how much of the shard fan-out the server ran in parallel that day, so read them as one box on one day, not a guarantee.

| Setup | Full-day chain | vs default |
|---|---:|---:|
| 64 KB window, single stream (protocol default) | 872.7 s | 1x |
| 8 MB window, single stream | 546.3 s | 1.60x |
| 8 MB window + 8-way sharding, buffered | 183.8 s | 4.75x |

Streaming shards the same way, and its realized gain rides on the same server-side parallelism, so we quote no wall clock for it.

Every setup returns the same rows. A concrete single contract comes back byte-for-byte identical to a single request. A full chain comes back in a deterministic canonical order (ascending expiration, then strike, then calls before puts, in trading-time order within each contract). Set `bulk_fetch = "off"` if you want the server's own ordering instead.

## Configuration reference

| Field | Default | What it does |
|---|---|---|
| `bulk_fetch` | `"auto"` | `"auto"` splits large pulls across your tier; `"off"` runs one request in server order. |
| `shard_concurrency` | tier budget | Caps pieces per pull. Lower to share concurrency with other requests. |
| `stream_window_size_kb` | `8192` | Per-stream HTTP/2 flow-control window, in KB. |
| `connection_window_size_kb` | `16384` | Whole-connection flow-control window, in KB. |

See [Configuration](/articles/configuration) for setting these on each language binding, and [Concurrent Requests](/articles/concurrent-requests) for running many separate requests in parallel.
