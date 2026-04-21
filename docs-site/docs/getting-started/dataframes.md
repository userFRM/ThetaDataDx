---
title: DataFrames
description: Convert ThetaDataDx tick lists to Arrow, Polars, or Pandas DataFrames, with the zero-copy scope explicitly documented.
---

# DataFrames

The Python SDK returns every historical endpoint as a `list[TickClass]` by default. Three single-step adapters convert that list to columnar form for pandas, polars, or Arrow-based downstreams (DuckDB, cuDF, Arrow Flight).

```python
from thetadatadx import Credentials, Config, ThetaDataDx
from thetadatadx import to_dataframe, to_polars, to_arrow

creds = Credentials.from_file("creds.txt")
tdx = ThetaDataDx(creds, Config.production())

eod = tdx.stock_history_eod("AAPL", "20240101", "20240301")
df  = to_dataframe(eod)   # pandas.DataFrame
pdf = to_polars(eod)      # polars.DataFrame
tbl = to_arrow(eod)       # pyarrow.Table
```

The three adapters share one Rust implementation that walks the `Vec<Tick>` produced by the decoder, builds Arrow columnar buffers, and hands the `RecordBatch` off to `pyarrow.Table` via the Arrow C Data Interface. `to_dataframe` and `to_polars` then call through `pyarrow.Table` for the final dtype.

Install with the extra that matches your stack:

```bash
pip install thetadatadx[pandas]   # pandas adapter
pip install thetadatadx[polars]   # polars adapter
pip install thetadatadx[arrow]    # pyarrow only
pip install thetadatadx[all]      # all three
```

## Zero-copy scope

The Arrow / polars / pandas handoff is zero-copy at the `pyarrow.Table` boundary — the `RecordBatch` memory is shared, not copied, when it crosses into Python. Upstream of that boundary, the `list[TickClass]` step still allocates Python objects; the Rust decoder writes the typed slice, then the Arrow builder reads from the slice to populate the buffers.

In concrete terms, the benchmark matrix (see [performance page](../performance/benchmark)) measured the following peak RSS for `option_history_greeks_all` (176,732 rows × 31 columns):

- Python SDK `list[dict]` output: 731 MB
- ThetaDataDx `list[TickClass]` output: 19 MB
- ThetaDataDx `to_arrow(ticks)` output: 61 MB

The arrow path holds both the pyclass list and the Arrow buffers alive at peak, which is why RSS runs higher than the list-only path. If bulk RAM is the priority, take the list output and skip the Arrow conversion. If columnar compute is the priority, take the Arrow path and accept the peak RSS overhead; downstream libraries will still see real zero-copy semantics.

Endpoint-direct Arrow variants (skip the pyclass list entirely) are tracked as a future work item — they would bring the arrow-path RSS down to pyclass-path levels or below.

## Arrow consumers

Because `to_arrow(ticks)` returns a `pyarrow.Table`, every Arrow-native tool reads it with zero copy:

```python
import duckdb
tbl = to_arrow(chain)
duckdb.sql("SELECT strike, avg(close) FROM tbl GROUP BY strike").show()
```

```python
import polars as pl
pdf = pl.from_arrow(to_arrow(chain))   # alternative polars path; shares buffers
```

```python
import cudf
gdf = cudf.DataFrame.from_arrow(to_arrow(chain))  # GPU-side DataFrame
```

## When to use which

| Priority | Pick |
|----------|------|
| Bulk RAM (millions of rows, no columnar ops needed) | Keep the `list[TickClass]` output — do not call `to_*`. |
| Columnar compute (aggregations, joins, scenario sweeps) | `to_polars` or `to_arrow` + downstream library. |
| Pandas-only stack | `to_dataframe` (pandas). |
| GPU pipelines | `to_arrow` then `cudf.DataFrame.from_arrow`. |
| Simple dict iteration | The tick list already iterates — no conversion needed. |

## Next

- [Performance](./performance) — benchmark headline
- [Error handling](./errors) — `ThetaDataError` hierarchy for `to_*` converters
