---
title: Flat Files
description: Whole-universe daily archives — every contract for a date in one call.
---

# Flat Files

<TierBadge tier="professional" />

Flat files deliver **the whole universe for one date in one call** — every option contract or every stock for a given (security type, request type, date). Use them for daily ETL and backtests that need everything; use the per-contract [reference endpoints](/reference/) when you need one contract fast; use [streaming](/streaming/) for live data.

## Datasets

The distribution serves a fixed set of five datasets. Each is one method on the `flat_files` namespace plus a generic `request(sec_type, req_type, date)` dispatcher; an unserved `(security, request)` pair is rejected with a typed invalid-parameter error before any network round-trip. Per-tick quotes, trades, and OHLC bars are served by the [reference endpoints](/reference/), not as flat files. Response times and file sizes are ThetaData's published typicals for a full day.

| Dataset | Method | What one file holds | Typical |
|---|---|---|---|
| Option trade-quote | `option_trade_quote` | Every OPRA trade paired with the NBBO quote in effect just before it, all contracts for the date. | ~3 min · ~1.2 GB |
| Option open-interest | `option_open_interest` | The prior session's closing open interest from OPRA (published ~06:30 ET), one row per contract. | ~1 min · ~50 MB |
| Option EOD | `option_eod` | Theta's 17:15 ET end-of-day summary (OPRA publishes no national EOD): `ms_of_day` is the report time, `ms_of_day2` the last trade, and the quote is the last NBBO at report time. | ~1 min · ~150 MB |
| Stock trade-quote | `stock_trade_quote` | Every trade paired with the NBBO quote preceding it, all symbols for the date. | ~30 min · ~14 GB |
| Stock EOD | `stock_eod` | The national 17:15 ET end-of-day summary (SIPs publish only partial EOD): `ms_of_day` report time, `ms_of_day2` last trade, and the last CTA/UTP NBBO. | ~1 sec · ~1.5 MB |

## Availability

The most recent **7 calendar days** are available; the prior day is generally ready between 00:30 and 01:00 ET. A request outside that window returns a typed no-data error, not empty rows — it is a window limit, not a failure. For deeper history, contact ThetaData.

## Pull a file

Flat files are account-authenticated market data, so `client` below can be either the unified `Client` or the standalone `MarketDataClient` — both expose the identical `flat_files` surface. A market-data-only workflow never needs the unified client just to pull archives.

<SdkTabs>

<template #rust>

```rust
use thetadatadx::flatfiles::{FlatFileFormat, ReqType, SecType};

// Decoded rows in memory, via the `flat_files` view:
let rows = client.flat_files().option_trade_quote("20250303").await?;

// Generic dispatcher (same view), for config-driven call shapes:
let rows = client.flat_files().request(SecType::Option, ReqType::TradeQuote, "20250303").await?;

// Vendor-format file straight to disk (bounded memory):
client.flat_files().to_path(SecType::Option, ReqType::TradeQuote, "20250303", "trade_quotes.csv", FlatFileFormat::Csv).await?;
```

The standalone `thetadatadx::flatfile_request*` free functions remain available as the lower-level API for callers passing credentials and config explicitly.

</template>

<template #python>

```python
rows = client.flat_files.option_trade_quote("20250303")
df = rows.to_polars()          # or .to_pandas() / .to_arrow() / .to_list()

# Or write the vendor-format file straight to disk (bounded memory):
client.flatfile_to_path("OPTION", "TRADE_QUOTE", "20250303", "trade_quotes.csv", "csv")
```

</template>

<template #typescript>

```typescript
const rows = await client.flatFiles.optionTradeQuote('20250303');
const ipc = rows.toArrowIpc();   // feed into apache-arrow `tableFromIPC`
```

</template>

<template #cpp>

```cpp
auto rows = client.flat_files().option_trade_quote("20250303");
auto ipc = rows.to_arrow_ipc();  // std::vector<uint8_t>
```

</template>

<template #http>

```bash
curl 'http://127.0.0.1:25503/v3/option/flat_file/trade_quote?date=20250303&format=csv' -o trade_quotes.csv
```

The server streams the response body in chunks, so downloads of any size run in bounded memory.

</template>

</SdkTabs>

## Parameters

| Parameter | Required | Description |
|---|---|---|
| `date` | yes | The archive date, `YYYYMMDD`. One date per call — flat files have no range form. |
| `format` | no | On-disk encoding for the to-disk / HTTP paths; one of the formats below. Defaults to `csv`. The decoded-rows path returns typed rows and ignores it. |

## Formats

The to-disk (`to_path` / `flatfile_to_path`) and HTTP paths write one of:

| Format | Value | Output |
|---|---|---|
| CSV | `csv` (default) | Vendor byte-format CSV — lowercase headers, byte-matches the terminal's own download. |
| JSON Lines | `jsonl` / `ndjson` | One JSON object per row. |
| JSON | `json` | A single JSON array of the same per-row objects. |
| HTML | `html` | An HTML `<table>`. |

The decoded-rows path (`option_trade_quote(...)`, `request(...)`) skips the file entirely and returns typed `FlatFileRow`s ready for Arrow, Polars, pandas, or your own pipeline.

## Columnar & DataFrames

The decoded-rows path hands back a typed row collection that converts straight to Arrow — and, on Python, to a Polars or pandas DataFrame — with no CSV round-trip. The Arrow schema is inferred from the file's own column header, so it always matches the dataset.

<SdkTabs>

<template #rust>

```rust
let rows = client.flat_files().option_trade_quote("20250303").await?;
let batch = thetadatadx::flatfiles::arrow::rows_to_arrow(&rows)?; // arrow_array::RecordBatch
```

</template>

<template #python>

```python
rows = client.flat_files.option_trade_quote("20250303")
pl_df = rows.to_polars()     # Polars DataFrame
pd_df = rows.to_pandas()     # pandas DataFrame
table = rows.to_arrow()      # pyarrow Table (zero-copy)
records = rows.to_list()     # list[dict]
```

</template>

<template #typescript>

```typescript
import { tableFromIPC } from "apache-arrow";

const rows = await client.flatFiles.optionTradeQuote('20250303');
const table = tableFromIPC(rows.toArrowIpc());   // apache-arrow Table
```

</template>

<template #cpp>

```cpp
auto rows = client.flat_files().option_trade_quote("20250303");
std::vector<uint8_t> ipc = rows.to_arrow_ipc();  // feed arrow::ipc::RecordBatchStreamReader
```

</template>

</SdkTabs>

## Column schema

Each dataset's columns are described by the server in the file's header and carried on the decoded `FlatFileRow` (and the CSV header) — the SDK does not hardcode a fixed column set, so a server-side schema addition flows through without an SDK change. For the authoritative per-dataset column list, see ThetaData's [flat-file reference](https://http-docs.thetadata.us/operations/get-v2-flat-file-getting-started.html).

## Size guidance

Flat files are large — a whole-universe option `trade_quote` day commonly exceeds 100 MB and tens of millions of rows.

- The decoded-rows path materializes everything in memory before returning; reserve it for machines with headroom.
- The to-disk path (`flatfile_to_path`, the HTTP route) keeps peak memory bounded and is the right default for ETL.
- Transient download failures retry automatically with backoff; tune attempts and backoff via the `flatfiles_*` [configuration](/articles/configuration) fields.
