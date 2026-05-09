# FLATFILES quickstart

Pull the whole-universe option-quote flat file for a single date in three lines, in every supported language. The shape is identical across bindings — the only difference is the call syntax.

## Python

```python
from thetadatadx import ThetaDataDxClient

with ThetaDataDxClient.connect_from_file("creds.txt") as tdx:
    rows = tdx.flat_files.option_quote(date="20260428")
    print(f"{len(rows)} rows")
    df = rows.to_polars()      # or .to_pandas() / .to_arrow() / .to_list()

# Write the vendor byte-format CSV directly to disk:
final_path = tdx.flatfile_to_path("OPTION", "QUOTE", "20260428",
                                  "/tmp/spy_quotes.csv", "csv")
```

## TypeScript / Node.js

```ts
import { ThetaDataDxClient } from 'thetadatadx';

const tdx = await ThetaDataDxClient.connectFromFile('creds.txt');
const rows = await tdx.flatFiles.optionQuote('20260428');
console.log(`${rows.len()} rows`);
const ipcBytes = rows.toArrowIpc();   // feed into apache-arrow `tableFromIPC`
```

## C++

```cpp
#include "thetadx.hpp"

int main() {
  auto creds = tdx::Credentials::from_file("creds.txt");
  auto client = tdx::Client::connect(creds);

  auto rows = client.flat_files().option_quote("20260428");
  auto ipc = rows.to_arrow_ipc();          // std::vector<uint8_t>
}
```

## `tdx` CLI

```bash
# Convenience subcommands — one per (sec_type, req_type) pair.
tdx flatfile quotes 20260428 --format csv -o spy_quotes.csv
tdx flatfile trades 20260428 --format jsonl
tdx flatfile ohlc   20260428 | head           # streams to stdout if -o omitted

# Generic dispatch:
tdx flatfile request --sec-type stock --req-type quote --date 20260428 --format csv
```

## REST server

```bash
# GET — convenience path
curl 'http://127.0.0.1:25503/v3/flatfile/option/quote?date=20260428&format=csv' \
     -o spy_quotes.csv

# POST — generic request
curl -X POST 'http://127.0.0.1:25503/v3/flatfile/request' \
     -H 'Content-Type: application/json' \
     -d '{"sec_type":"OPTION","req_type":"QUOTE","date":"20260428","format":"csv"}' \
     -o spy_quotes.csv
```

The response body is the file bytes themselves (`Content-Type: text/csv` for CSV, `application/x-ndjson` for JSONL), streamed in chunks so the server never pins the full blob in RAM.

## MCP server

LLM clients reach the same surface through the `tdx_flatfile_*` tool names:

```json
{
  "method": "tools/call",
  "params": {
    "name": "tdx_flatfile_option_quote",
    "arguments": { "date": "20260428", "format": "csv" }
  }
}
```

The tool writes the blob to a deterministic temp path (or the `output_path` you pass) and returns `{ "status": "ok", "path": "/tmp/...", ... }` so the LLM client can hand the file off to a downstream tool that knows how to read CSV / JSONL.
