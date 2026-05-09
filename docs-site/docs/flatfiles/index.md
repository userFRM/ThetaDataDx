# FLATFILES

ThetaData ships **whole-universe daily blobs** through a third surface called FLATFILES, alongside MDDS (request/response history) and FPSS (real-time streaming). A single FLATFILES call returns *every contract* the server knows about for a given (security type, request type, date) tuple — so a single request can return tens of millions of rows.

ThetaDataDx exposes this surface in every public binding: Rust, Python, TypeScript, C, C++, the `tdx` CLI, the REST server, and the MCP server.

## When to use FLATFILES

Choose FLATFILES when you need the whole universe at once and you don't care about the per-contract latency the request-response endpoints optimise for.

| Scenario | Use |
|---|---|
| Backtest needs every option contract for one date | **FLATFILES** |
| Backtest needs one option contract for one date | MDDS history |
| Real-time pipeline needs trades / quotes as they happen | FPSS streaming |
| Daily ETL writes one Parquet file per security type | **FLATFILES** |
| Latency-sensitive lookup (single contract, single field) | MDDS history |

The vendor reference for the wire path is at <https://http-docs.thetadata.us/operations/get-v2-flat-file-getting-started.html>. ThetaDataDx negotiates that path on your behalf — you call one method, the SDK pulls and decodes the bytes, and you get either an in-memory typed-row vector or a CSV / JSONL file on disk.

## Three call shapes

Every binding exposes the same three call shapes, so you can pick the one that matches your downstream pipeline:

1. **Decoded rows in memory.** `tdx.flat_files.option_quote(date)` (Python) → typed list of rows. Best when you want to feed the data straight into Polars / Pandas / Arrow / a backtester.
2. **Bytes on disk.** `tdx.flatfile_to_path("OPTION", "QUOTE", date, "out.csv", "csv")` (Python) → CSV or JSONL written directly to the path you provide. Best when you want the vendor byte-format CSV for archival or for piping into another tool that already speaks CSV.
3. **Generic dispatch.** `tdx.flat_files.request(sec_type, req_type, date)` (Python) → identical to the typed shortcuts but with the `(SecType, ReqType)` tuple driven by config. Useful when the call shape comes from a config file rather than from code.

## Bandwidth caveats

Flat files are large. A single option-quote daily blob can exceed 100 MB. Plan accordingly:

- The decoded-rows path materialises every row in memory before returning. Whole-universe option blobs commonly hold tens of millions of rows; do not call this on a constrained machine without a row cap.
- The on-disk path is the safer default for ETL pipelines: bytes flow through a single async write, peak memory stays bounded, and the file is ready for downstream tools without any additional buffering.
- The REST server route streams bytes via a chunked response body so the server never pins more than a few KB of buffered output per request.

## What's covered

| Surface | Coverage |
|---|---|
| Rust | All ten convenience methods on `ThetaDataDxClient` plus the generic `flatfile_request*` calls. |
| Python | `tdx.flat_files.{option,stock}_*` typed terminals plus `tdx.flatfile_to_path(...)`. |
| TypeScript | Same shape as Python via napi-rs. |
| C | `tdx_flatfile_*` C ABI calls. |
| C++ | `tdx::FlatFiles` namespace + Arrow IPC bytes terminal. |
| `tdx` CLI | `tdx flatfile {quotes,trades,trade_quote,ohlc,open_interest,eod,stock_*,request}` |
| REST server | `GET /v3/flatfile/{sec_type}/{req_type}?date=YYYYMMDD&format=csv\|jsonl` and `POST /v3/flatfile/request`. |
| MCP server | `tdx_flatfile_*` tools mirroring the Rust convenience methods. |

The full per-binding matrix lives in [`docs/ROADMAP.md`](https://github.com/userFRM/ThetaDataDx/blob/main/docs/ROADMAP.md#binding-coverage-matrix).

Continue to [Quickstart](./quickstart) for code samples, then [API reference](./api-reference) for the full method signatures.
