---
title: Flat Files
description: Whole-universe daily archives — every contract for a date in one call.
---

# Flat Files

Flat files deliver **the whole universe for one date in one call** — every option contract or every stock for a given (security type, request type, date). Use them for daily ETL and backtests that need everything; use the per-contract [reference endpoints](/reference/) when you need one contract fast; use [streaming](/streaming/) for live data.

## Pull a file

The flat-file distribution serves a fixed set of datasets: option `trade_quote` / `open_interest` / `eod` and stock `trade_quote` / `eod`. The `flat_files` namespace exposes one method per served pair: `option_trade_quote`, `option_open_interest`, `option_eod`, `stock_trade_quote`, `stock_eod` — plus a generic `request(sec_type, req_type, date)` that rejects an unserved `(security, request)` pair with a typed invalid-parameter error before any network round-trip. Per-tick quotes, trades, and OHLC bars are served by the [reference endpoints](/reference/), not as flat files.

<SdkTabs>

<template #rust>

```rust
use thetadatadx::flatfiles::{FlatFileFormat, ReqType, SecType};

// Vendor-format file straight to disk (bounded memory):
client.flatfile_option_trade_quote("20250303", "trade_quotes.csv", FlatFileFormat::Csv).await?;

// Decoded rows in memory:
let rows = client.flatfile_request_decoded(SecType::Option, ReqType::TradeQuote, "20250303").await?;
```

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
curl 'http://127.0.0.1:25503/v3/flatfile/option/trade_quote?date=20250303&format=csv' -o trade_quotes.csv
```

The server streams the response body in chunks, so downloads of any size run in bounded memory. The same surface is available from the CLI: `thetadatadx flatfile trade_quote 20250303 --format csv -o trade_quotes.csv`.

</template>

</SdkTabs>

## Size guidance

Flat files are large — a whole-universe option `trade_quote` day commonly exceeds 100 MB and tens of millions of rows.

- The decoded-rows path materializes everything in memory before returning; reserve it for machines with headroom.
- The to-disk path (`flatfile_to_path`, the HTTP route, the CLI) keeps peak memory bounded and is the right default for ETL.
- Transient download failures retry automatically with backoff; tune attempts and backoff via the `flatfiles_*` [configuration](/articles/configuration) fields.
