---
title: Request Sizing
description: How to split large market-data requests.
---

# Request Sizing

Keep individual historical responses comfortably small. Intraday endpoints cap multi-day requests at one month of data; tick-interval responses for liquid symbols can still run to millions of rows per day.

Rules of thumb:

- Use `start_date` / `end_date` windows of one day for tick-level data, one month for minute bars, and as wide as you like for EOD.
- Pin `strike` and `right` on option requests unless you genuinely need the whole chain; wildcard chains multiply row counts by the contract count.
- For a large single request you no longer have to split it yourself: the SDK sizes it and runs it in parallel across your tier (see [Bulk Downloads](/articles/bulk-downloads)). You can still split the range manually with Python's `split_date_range(start, end)` helper when you want direct control over the pieces.
- In Rust and Python, switch from the buffered call to the chunk-streaming variant (`.stream(handler)`) when a response may exceed a few hundred thousand rows. It caps memory regardless of response size, and it is the fastest path for very large pulls. See [Bulk Downloads](/articles/bulk-downloads) for buffered versus streaming.
- Large responses stream faster with a bigger HTTP/2 flow-control window. The SDK defaults it to 8 MB per stream, well above the 64 KB protocol default that throttles big streams; see [Bulk Downloads](/articles/bulk-downloads) for what it does and [Configuration](/articles/configuration) to tune it.
- Need the entire market for a day? [Flat files](/articles/flat-files) deliver whole-universe daily archives more efficiently than any per-symbol loop.
