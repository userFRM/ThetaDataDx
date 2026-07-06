---
title: Request Sizing
description: How to split large market-data requests.
---

# Request Sizing

Keep individual historical responses comfortably small. Intraday endpoints cap multi-day requests at one month of data; tick-interval responses for liquid symbols can still run to millions of rows per day.

Rules of thumb:

- Use `start_date` / `end_date` windows of one day for tick-level data, one month for minute bars, and as wide as you like for EOD.
- Pin `strike` and `right` on option requests unless you genuinely need the whole chain; wildcard chains multiply row counts by the contract count.
- For bulk pulls, split the range and run requests [concurrently](/articles/concurrent-requests) rather than growing one giant response. Python's `split_date_range(start, end)` helper yields ready-to-use windows.
- In Rust and Python, switch from the buffered call to the chunk-streaming variant (`.stream(handler)`) when a response may exceed a few hundred thousand rows — it caps memory regardless of response size.
- Need the entire market for a day? [Flat files](/articles/flat-files) deliver whole-universe daily archives more efficiently than any per-symbol loop.
