---
title: Quotes At a Time of Day
description: Sample the same wall-clock moment across a date range in one request.
---

# Quotes At a Time of Day

The `at_time` endpoints answer "what was the market at 10:30 every day last quarter?" in a single request — one row per trading day, each the last quote (or trade) at or before the requested moment.

```python
from thetadatadx import Config, Credentials, Client

creds = Credentials.from_file("creds.txt")
tdx = Client(creds, Config.production())

rows = tdx.historical.stock_at_time_quote("AAPL", "20250101", "20250331", "10:30:00.000")
for t in rows:
    mid = (t.bid + t.ask) / 2
    print(t.date, f"mid={mid:.2f}", f"spread={t.ask - t.bid:.2f}")
```

The same shape exists for [trades](/reference/stock/at-time/trade), [option contracts](/reference/option/at-time/quote), and [index values](/reference/index/at-time/price):

```python
opt = tdx.historical.option_at_time_quote("SPY", "20250321", "20250101", "20250331",
                               "10:30:00.000", strike="570", right="C")
```

Times are Eastern wall-clock `HH:MM:SS.SSS` — `"10:30:00.000"` means 10:30 ET on every day in the range, daylight-saving handled for you. This replaces the classic anti-pattern of pulling full tick days and searching each one for a single timestamp.
