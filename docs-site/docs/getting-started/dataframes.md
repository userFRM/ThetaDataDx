---
title: DataFrames
description: Convert ThetaDataDx endpoint results to Arrow, Polars, or Pandas DataFrames via chained terminals on the typed list wrapper, with the zero-copy scope explicitly documented.
---

# DataFrames

Every historical endpoint on the Python SDK returns a typed list wrapper (`EodTickList`, `OhlcTickList`, `StringList`, ...). DataFrame conversion happens via chained terminals on the wrapper — no free-function round-trip.

```python
from thetadatadx import Credentials, Config, ThetaDataDx

creds = Credentials.from_file("creds.txt")
tdx = ThetaDataDx(creds, Config.production())

df  = tdx.stock_history_eod("AAPL", "20240101", "20240301").to_polars()
pdf = tdx.stock_history_eod("AAPL", "20240101", "20240301").to_pandas()
tbl = tdx.stock_history_eod("AAPL", "20240101", "20240301").to_arrow()
lst = tdx.stock_history_eod("AAPL", "20240101", "20240301").to_list()
```

The terminals share one Rust implementation that walks the decoder-owned `Vec<Tick>` directly into Arrow columnar buffers, then hands the `RecordBatch` off to `pyarrow.Table` via the Arrow C Data Interface. `.to_pandas()` and `.to_polars()` then call through `pyarrow.Table` for the final dtype.

Install with the extra that matches your stack:

```bash
pip install thetadatadx[pandas]   # pandas terminal
pip install thetadatadx[polars]   # polars terminal
pip install thetadatadx[arrow]    # pyarrow only
pip install thetadatadx[all]      # all three
```

## The list wrapper

Before a terminal is called the list wrapper itself behaves like a regular Python sequence:

```python
ticks = tdx.stock_history_eod("AAPL", "20240101", "20240301")

len(ticks)                # row count
bool(ticks)               # False when empty
ticks[0]                  # first row as EodTick
ticks[-1]                 # last row, negative indexing supported
for tick in ticks:        # iterate in decode order
    process(tick)
```

`.to_list()` returns a plain `list[TickClass]` for callers that want a mutable container or need to feed an API that insists on `list`.

## Zero-copy scope

The Arrow / polars / pandas handoff is zero-copy at the `pyarrow.Table` boundary — the `RecordBatch` memory is shared, not copied, when it crosses into Python. Upstream of that boundary, the list wrapper holds the decoder-owned `Vec<Tick>` without allocating a Python object per row; the Arrow builder reads from that slice directly to populate the buffers.

This makes the chained path strictly lighter than the `list[TickClass]` + free-function path in the previous SDK release: there is no double-buffering peak where both the pyclass list and the Arrow columns live simultaneously.

## Arrow consumers

Because `.to_arrow()` returns a `pyarrow.Table`, every Arrow-native tool reads it with zero copy:

```python
import duckdb
tbl = tdx.option_history_quote(...).to_arrow()
duckdb.sql("SELECT strike, avg(bid) FROM tbl GROUP BY strike").show()
```

```python
import polars as pl
pdf = pl.from_arrow(tdx.option_history_quote(...).to_arrow())
```

```python
import cudf
gdf = cudf.DataFrame.from_arrow(tdx.option_history_quote(...).to_arrow())
```

## List endpoints

Endpoints that return a `Vec<String>` (`stock_list_symbols`, `option_list_expirations`, `stock_list_dates`, ...) surface a `StringList` wrapper with the same chainable terminals. The DataFrame column header matches the semantic name on the wrapper (`symbol`, `expiration`, `date`, ...):

```python
symbols = tdx.stock_list_symbols().to_polars()   # single-column `symbol` frame
expirations = tdx.option_list_expirations("AAPL").to_pandas()
```

## When to use which

| Priority | Pick |
|----------|------|
| Bulk RAM, row-at-a-time iteration | Keep the list wrapper — iterate directly, no terminal. |
| Columnar compute (aggregations, joins, scenario sweeps) | `.to_polars()` or `.to_arrow()` + downstream library. |
| Pandas-only stack | `.to_pandas()`. |
| GPU pipelines | `.to_arrow()` then `cudf.DataFrame.from_arrow`. |
| Plain list (mutability, API compatibility) | `.to_list()`. |

## Next

- [Quick Start](./quickstart) — install, authenticate, first call, tabbed per language
- [Error handling](./errors) — `ThetaDataError` hierarchy for endpoint calls
