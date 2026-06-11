---
title: Streaming Watchlist
description: Subscribe a basket of symbols in one call and fan events out by symbol.
---

# Streaming Watchlist

Install a basket of subscriptions in one `subscribe_many` call and route events by the contract on each event.

```python
import time
from collections import defaultdict

from thetadatadx import Config, Contract, Credentials, ThetaDataDxClient

WATCHLIST = ["AAPL", "MSFT", "NVDA", "SPY", "TSLA"]

creds = Credentials.from_file("creds.txt")
tdx = ThetaDataDxClient(creds, Config.production())

last_quote = {}
trade_count = defaultdict(int)

def on_event(event):
    if event.kind == "quote":
        last_quote[event.contract.symbol] = (event.bid, event.ask)
    elif event.kind == "trade":
        trade_count[event.contract.symbol] += 1

with tdx.streaming(on_event):
    tdx.subscribe_many(
        [Contract.stock(s).quote() for s in WATCHLIST]
        + [Contract.stock(s).trade() for s in WATCHLIST]
    )

    for _ in range(6):
        time.sleep(10)
        for sym in WATCHLIST:
            bid, ask = last_quote.get(sym, (None, None))
            print(f"{sym:6} bid={bid} ask={ask} trades={trade_count[sym]}")
        print("dropped:", tdx.dropped_event_count())
```

Keep the callback to dictionary writes and counters — heavy work belongs on another thread feeding from these structures. `dropped_event_count()` staying at zero is the proof your callback keeps up; see [Reconnection & Monitoring](/streaming/reliability).

Resize the basket live with further `subscribe` / `unsubscribe` calls — no restart needed. The same pattern in the other languages differs only in the loop syntax; see the per-stream [reference pages](/streaming/stocks/quote).
