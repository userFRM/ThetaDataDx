---
title: Option Chain Snapshot
description: Discover a chain and pull live Greeks for every contract on an expiration.
---

# Option Chain Snapshot

Discover what trades, then pull the whole expiration's Greeks in one wildcard request.

```python
from thetadatadx import Config, Credentials, Client

creds = Credentials.from_file("creds.txt")
tdx = Client(creds, Config.production())

# 1. Nearest expiration for the underlying.
expiration = tdx.historical.option_list_expirations("SPY")[0]

# 2. All-Greeks snapshot for every contract on it (strike/right default to wildcard).
chain = tdx.historical.option_snapshot_greeks_all("SPY", expiration)

# 3. Closest-to-the-money calls by absolute delta.
calls = [t for t in chain if t.right == "C"]
for t in sorted(calls, key=lambda t: abs(t.delta - 0.5))[:5]:
    print(f"{t.strike:8.1f}  delta={t.delta:+.3f}  iv={t.implied_volatility:.4f}")
```

The same shape works in every language — wildcard the snapshot by omitting `strike` / `right` (Rust: skip the builder setters; C++: omit the options struct; TypeScript: pass `undefined`). See [All Greeks](/reference/option/snapshot/greeks/all) for the full field list.

To bound a wide chain, add `strike_range=10` (ten strikes either side of the money) or `max_dte=30` on [contract listing](/reference/option/list/contracts).
