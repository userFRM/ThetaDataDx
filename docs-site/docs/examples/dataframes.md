---
title: DataFrames
description: Convert any historical response to pandas, polars, or Arrow.
---

# DataFrames

Every Python historical response converts in one chained call — the rows cross into pyarrow via the Arrow C Data Interface, so the conversion itself is zero-copy.

```python
from thetadatadx import Config, Credentials, Client

creds = Credentials.from_file("creds.txt")
client = Client(creds, Config.production())

quotes = client.historical.option_history_quote("SPY", "20250321", "20250303",
                                  strike="570", right="C", interval="1m")

df = quotes.to_pandas()      # pip install thetadatadx[pandas]
lf = quotes.to_polars()      # pip install thetadatadx[polars]
tbl = quotes.to_arrow()      # pip install thetadatadx[arrow]
rows = quotes.to_list()      # plain list of typed objects, no extra dependency

print(df[["ms_of_day", "bid", "ask"]].describe())
```

A typical resample to one-minute midpoints:

```python
import pandas as pd

df["mid"] = (df["bid"] + df["ask"]) / 2
df["ts"] = pd.to_datetime(df["date"], format="%Y%m%d") + pd.to_timedelta(df["ms_of_day"], unit="ms")
minute_mid = df.set_index("ts")["mid"].resample("1min").last()
```

In Rust, the optional `frames` feature adds `.to_polars()` / `.to_arrow()` on tick slices:

```toml
thetadatadx = { version = "13.0.0-rc.5", features = ["frames"] }
```

```rust
use thetadatadx::frames::TicksPolarsExt;

let ticks = client.historical().stock_history_eod("AAPL", "20250303", "20250306").await?;
let df = ticks.to_polars()?;
```

TypeScript and C++ return typed arrays/vectors directly; for columnar pipelines, the flat-files surface offers Arrow IPC bytes (`rows.toArrowIpc()` / `rows.to_arrow_ipc()`).
