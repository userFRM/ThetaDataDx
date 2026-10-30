---
title: Flat Files
description: "Whole-universe files for one date: every contract or stock of a dataset in one call."
---

# Flat Files

<TierBadge tier="professional" />

A flat file is one dataset for the whole universe on one date: every option contract, every stock or every index. Use them for daily ETL and backtests that need everything; use the per-contract [reference endpoints](/reference/) when you need one contract fast; use [streaming](/streaming/) for live data.

## Datasets

Each flat file is an endpoint like any other, on every binding and on the local server, served by the same market-data service as the rest of the reference. Each takes one `date`.

| Dataset | Endpoint | One row per | Rows |
|---|---|---|---|
| [Option EOD](/reference/option/flat-file/eod) | `option_flat_file_eod` | Contract: the end-of-day report generated at 17:15 ET, with the last NBBO at report time. | `EodTick` |
| [Option trade-quote](/reference/option/flat-file/trade-quote) | `option_flat_file_trade_quote` | OPRA trade, paired with the NBBO quote at the time of the trade. | `TradeQuoteTick` |
| [Option open interest](/reference/option/flat-file/open-interest) | `option_flat_file_open_interest` | Contract: the open interest OPRA reports in the morning for the prior session. | `OpenInterestTick` |
| [Option quote](/reference/option/flat-file/quote) | `option_flat_file_quote` | Contract and minute in which a new NBBO quote was reported. | `IntervalQuoteTick` |
| [Stock EOD](/reference/stock/flat-file/eod) | `stock_flat_file_eod` | Stock: the national end-of-day report generated at 17:15 ET, with the last CTA / UTP NBBO. | `EodTick` |
| [Stock trade-quote](/reference/stock/flat-file/trade-quote) | `stock_flat_file_trade_quote` | Trade, paired with the NBBO quote at the time of the trade. | `TradeQuoteTick` |
| [Stock quote](/reference/stock/flat-file/quote) | `stock_flat_file_quote` | Stock and minute in which a new quote was reported, from the merged UTP and CTA NBBO. | `IntervalQuoteTick` |
| [Index EOD](/reference/index/flat-file/eod) | `index_flat_file_eod` | Index: the end-of-day report. | `EodTick` |

Every row names its contract: the root on every file (the `symbol` column of a DataFrame), plus expiration, strike and right on the option files. On an EOD row `created_ms_of_day` is the report time and `last_trade_ms_of_day` the day's last trade; on a quote row `interval_ms_of_day` is the minute the row samples and `quote_ms_of_day` the time the quote in effect was reported.

## Availability

The most recent **7 calendar days** are available; the prior day is generally ready between 00:30 and 01:00 ET. A date outside that window fails with the error the vendor returns for it: a window limit, not a fault. For deeper history, contact ThetaData.

## Pull a file

A small file (stock or index EOD) comes back whole like any other endpoint. The trade-quote, quote and option files are far larger: stream them chunk by chunk, and lift the per-request deadline, which defaults to 300 seconds and would end a long download early.

<SdkTabs>

<template #rust>

```rust
use std::time::Duration;

// A small file, whole:
let eod = client.market_data().stock_flat_file_eod("20261021").await?;
let df = eod.to_polars()?;  // leads with the `symbol` column

// A large one, chunk by chunk:
client
    .market_data()
    .option_flat_file_trade_quote("20261021")
    .with_deadline(Duration::ZERO)
    .stream(|chunk| {
        // `chunk` derefs to `&[TradeQuoteTick]`; `chunk.columns()` names each row's root
    })
    .await?;
```

</template>

<template #python>

```python
eod = client.market_data.stock_flat_file_eod("20261021")
df = eod.to_polars()          # or .to_pandas() / .to_arrow() / .to_list()

def on_chunk(chunk):          # TradeQuoteTickList, like a whole result
    df = chunk.to_polars()    # leads with the `symbol` column

(client.market_data
    .option_flat_file_trade_quote_builder("20261021")
    .timeout_ms(0)
    .stream(on_chunk))
```

</template>

<template #typescript>

```typescript
const eod = await client.marketData.stockFlatFileEOD('20261021');

await client.marketData.optionFlatFileTradeQuoteStream('20261021', { timeoutMs: 0 }, (rows, columns) => {
  // rows: TradeQuoteTick[]; columns names each row's root
});
```

</template>

<template #cpp>

```cpp
auto eod = client.market_data().stock_flat_file_eod("20261021");

client.market_data().option_flat_file_trade_quote_stream("20261021",
    [](auto rows, const thetadatadx::ColumnPresence& columns) {
        // rows: Span<const TradeQuoteTick>; columns names each row's root
    },
    thetadatadx::EndpointRequestOptions{}.with_timeout_ms(0));
```

</template>

<template #http>

```bash
curl 'http://127.0.0.1:25503/v3/option/flat_file/trade_quote?date=20261021&format=csv' -o trade_quotes.csv
```

</template>

</SdkTabs>

A streamed chunk carries the root of each of its rows, as a whole result does: in Rust on the chunk's `columns()`, in Python on the chunk's frames, in TypeScript on the callback's `columns` argument and in C++ on the handler's `ColumnPresence`. When the chunk's rows span several roots, `symbols` holds one root per row; when every row in it shares one, `symbol` holds it.

## Write a file to disk

Each flat file has a companion call that writes it to disk exactly as the terminal writes it: `<endpoint>_to_path`, such as `option_flat_file_trade_quote_to_path`. It takes the file's `date`, the path to write and a format, and returns the path written. The file streams to disk chunk by chunk, so memory stays flat whatever its size, `json_legacy` aside: its one array per column cannot close before the last row, so the file is held until then, as the terminal holds it. It is written under a temporary name beside the path and takes the path's name only once complete, so a failed or cancelled download never leaves a partial file under that name. A path without an extension gets the format's.

| Format | Layout |
|---|---|
| `csv` (default) | A header line of the column names, then one line per row: text and price cells quoted, integers bare, a missing value empty. |
| `json` | One object, `{"response": [...]}`, holding an object per row. |
| `json_legacy` | One object holding an array per column. |
| `ndjson` (alias `jsonl`) | One object per row, one row per line. |
| `html` | The terminal's HTML page: one table, a header row and a row per row. |

<SdkTabs>

<template #rust>

```rust
use thetadatadx::flatfiles::FlatFileFormat;

let path = client
    .market_data()
    .option_flat_file_trade_quote_to_path("20261021", "trade_quotes.csv", FlatFileFormat::Csv)
    .await?;
```

</template>

<template #python>

```python
path = client.market_data.option_flat_file_trade_quote_to_path("20261021", "trade_quotes.csv")

# In a coroutine:
path = await client.market_data.option_flat_file_trade_quote_to_path_async("20261021", "trade_quotes.csv")
```

</template>

<template #typescript>

```typescript
const path = await client.marketData.optionFlatFileTradeQuoteToPath('20261021', 'trade_quotes.csv');
```

</template>

<template #cpp>

```cpp
// Throws on failure.
client.market_data().option_flat_file_trade_quote_to_path("20261021", "trade_quotes.csv");
```

</template>

<template #http>

```bash
curl 'http://127.0.0.1:25503/v3/option/flat_file/trade_quote?date=20261021&format=csv' -o trade_quotes.csv
```

</template>

</SdkTabs>

The C ABI exposes the same call as `thetadatadx_option_flat_file_trade_quote_to_path(market_data, date, path, format)` on a market-data handle, where a NULL `format` means `csv` and the return is 0 on success. The [MCP server](/mcp) offers it as one `<endpoint>_to_path` tool per flat file, its way to hand a model a file past the 50,000 rows one tool call returns.

## Over HTTP

The [local server](/server/) answers `/v3/{option,stock,index}/flat_file/{dataset}` the way the terminal does. It writes each chunk as it arrives, so the first bytes leave at once and memory stays flat whatever the file's size. The columns carry the terminal's names: `root`, then `expiration`, `strike` and `right` on option files, then the dataset's columns, where the EOD report time is `ms_of_day` and the second time column `ms_of_day2`. Times are integer milliseconds since midnight ET and dates `YYYYMMDD` integers.

| Parameter | Required | Description |
|---|---|---|
| `date` | yes | The file's date, `YYYYMMDD` or `YYYY-MM-DD`. |
| `format` | no | `csv` (default), `json`, `json_legacy`, `ndjson` (alias `jsonl`) or `html`. |

In CSV every text and price cell is quoted and a price keeps its decimal point (`"SPY",20261030,"808.0","P",...`); `json` wraps the rows in a `response` array with prices as strings. The body is byte for byte the file the `_to_path` calls write. A refusal before any data, such as a date outside the window, answers with its own status, like every other route. A download that fails after the first bytes ends in a broken transfer, never a short file that reads as complete.

## The `flat_files` API

The `flat_files` view (`client.flat_files()` in Rust and C++, `client.flat_files` in Python, `client.flatFiles` in TypeScript) and the `flatfile_*` functions request flat files from the vendor's older flat-file service. That service serves five of the datasets above: option trade-quote, open interest and EOD, and stock trade-quote and EOD. It does not serve the quote files or index EOD, and it writes its own CSV layout. The endpoints above are the complete set.
