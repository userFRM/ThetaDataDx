# FLATFILES API reference

The FLATFILES surface accepts three orthogonal inputs:

| Parameter | Values | Notes |
|---|---|---|
| `sec_type` | `OPTION`, `STOCK`, `INDEX` | Security class. |
| `req_type` | `EOD`, `QUOTE`, `OPEN_INTEREST`, `OHLC`, `TRADE`, `TRADE_QUOTE` | The kind of data delivered. |
| `date` | `YYYYMMDD` string | Single trading date. Flat files are **per-day** blobs; date ranges are not supported. |
| `format` | `csv`, `jsonl` | On-disk encoding when writing bytes to a file. |

Not every `(sec_type, req_type)` combination is supported by ThetaData — see the [ROADMAP](https://github.com/userFRM/ThetaDataDx/blob/main/docs/ROADMAP.md#flatfiles--surface-status) for the verified subset. Unsupported combinations surface as a typed `Error::FlatFilesUnavailable` in Rust (or its language-specific equivalent).

## Output formats

### CSV

Vendor byte-format CSV. Lowercase column headers, comma-separated values, no quoting, Unix line endings. Byte-matches the legacy ThetaData terminal's CSV output. Suitable for archival and for piping into existing CSV-based pipelines.

### JSONL

JSON Lines — one JSON object per row. Column names match the CSV column headers. Integer columns stay numeric (no stringification). Suitable for streaming ETL and for tools that prefer per-line parsing.

For columnar formats (Parquet, Arrow IPC, Polars / Pandas DataFrames), use the in-memory typed-row entry point and drive your own writer — this keeps the SDK free of heavy columnar dependencies. The [Quickstart](./quickstart) shows the `to_arrow()` / `to_polars()` / `to_pandas()` Python terminals.

## Method signatures

### Rust

```rust
use thetadatadx::flatfiles::{FlatFileFormat, ReqType, SecType};

// Convenience — one method per (sec, req) pair on `ThetaDataDxClient`:
client.flatfile_option_quote(date, output_path, FlatFileFormat::Csv).await?;
client.flatfile_option_trade(date, output_path, FlatFileFormat::Csv).await?;
client.flatfile_option_trade_quote(date, output_path, FlatFileFormat::Csv).await?;
client.flatfile_option_open_interest(date, output_path, FlatFileFormat::Csv).await?;
client.flatfile_option_eod(date, output_path, FlatFileFormat::Csv).await?;
client.flatfile_stock_quote(date, output_path, FlatFileFormat::Csv).await?;
client.flatfile_stock_trade(date, output_path, FlatFileFormat::Csv).await?;
client.flatfile_stock_trade_quote(date, output_path, FlatFileFormat::Csv).await?;
client.flatfile_stock_eod(date, output_path, FlatFileFormat::Csv).await?;

// Generic dispatch:
client.flatfile_request(SecType::Option, ReqType::Quote, date, output_path, FlatFileFormat::Csv).await?;
client.flatfile_request_decoded(SecType::Option, ReqType::Quote, date).await?;
```

### Python

```python
# Decoded rows in memory:
tdx.flat_files.option_quote(date)
tdx.flat_files.option_trade(date)
tdx.flat_files.option_trade_quote(date)
tdx.flat_files.option_ohlc(date)
tdx.flat_files.option_open_interest(date)
tdx.flat_files.option_eod(date)
tdx.flat_files.stock_quote(date)
tdx.flat_files.stock_trade(date)
tdx.flat_files.stock_trade_quote(date)
tdx.flat_files.stock_eod(date)

# Generic dispatch:
tdx.flat_files.request(sec_type, req_type, date)

# Bytes on disk:
tdx.flatfile_to_path(sec_type, req_type, date, path, format="csv")
```

`FlatFileRowList` returned by every typed terminal exposes `.to_arrow()`, `.to_pandas()`, `.to_polars()`, `.to_list()`, `len()`, and `bool()`.

### TypeScript

```ts
// Each typed shortcut returns Promise<FlatFileRowList>:
await tdx.flatFiles.optionQuote(date);
await tdx.flatFiles.optionTrade(date);
await tdx.flatFiles.optionOhlc(date);
// ...

// Generic dispatch:
await tdx.flatFiles.request(secType, reqType, date);

// FlatFileRowList terminals:
rows.len();
rows.isEmpty();
rows.toArrowIpc();      // -> Buffer (apache-arrow tableFromIPC)
rows.toJson();          // -> string (JSONL-style)
```

### C++

```cpp
auto rows = client.flat_files().option_quote(date);
auto ipc  = rows.to_arrow_ipc();   // std::vector<uint8_t>

// Bytes on disk via the FFI direct path:
client.flatfile_to_path(sec_type, req_type, date, path, "csv");
```

### REST server

```
GET  /v3/flatfile/{sec_type}/{req_type}?date=YYYYMMDD&format=csv|jsonl
POST /v3/flatfile/request
```

`GET` path segments are case-insensitive (`option` and `OPTION` are equivalent). `POST` body is JSON:

```json
{
  "sec_type": "OPTION",
  "req_type": "QUOTE",
  "date": "20260428",
  "format": "csv"
}
```

Response: `200 OK` with the file bytes streamed via chunked transfer encoding. `Content-Type` is `text/csv; charset=utf-8` (CSV) or `application/x-ndjson; charset=utf-8` (JSONL). On failure: standard error envelope (`error_type` + `error_msg`).

### MCP server

| Tool | Description |
|---|---|
| `tdx_flatfile_option_quote` | Whole-universe option-quote flat file for one date. |
| `tdx_flatfile_option_trade` | Whole-universe option-trade flat file. |
| `tdx_flatfile_option_trade_quote` | Combined trade-quote stream. |
| `tdx_flatfile_option_ohlc` | OHLC flat file. |
| `tdx_flatfile_option_open_interest` | Open-interest flat file. |
| `tdx_flatfile_option_eod` | End-of-day flat file. |
| `tdx_flatfile_stock_quote` / `_trade` / `_trade_quote` / `_eod` | Stock equivalents. |
| `tdx_flatfile_request` | Generic dispatch with explicit `sec_type` / `req_type` / `date` / `output_path` / `format` arguments. |

Each tool returns `{ "status": "ok", "path": "...", "sec_type": "...", "req_type": "...", "format": "csv", "date": "..." }`. The LLM client hands the path to a downstream tool that already knows how to read CSV / JSONL.
