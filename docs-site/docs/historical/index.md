---
title: Historical Data
description: Overview of ThetaDataDx historical data endpoints for stocks, options, indices, rates, and calendars.
---

# Historical Data

ThetaDataDx provides 61 historical data endpoints across five asset categories. All historical data is accessed through the ThetaDataDx client, which communicates over gRPC with ThetaData's MDDS servers. gRPC, protobuf parsing, zstd decompression, and FIT decoding run inside the `thetadatadx` Rust crate, regardless of which language binding you call.

## Connecting

::: code-group
```rust [Rust]
use thetadatadx::{ThetaDataDx, Credentials, DirectConfig};

let creds = Credentials::from_file("creds.txt")?;
let tdx = ThetaDataDx::connect(&creds, DirectConfig::production()).await?;
```
```python [Python]
from thetadatadx import Credentials, Config, ThetaDataDx

creds = Credentials.from_file("creds.txt")
tdx = ThetaDataDx(creds, Config.production())
```
```typescript [TypeScript]
import { ThetaDataDx } from 'thetadatadx';

const tdx = await ThetaDataDx.connectFromFile('creds.txt');
```
```go [Go]
creds, _ := thetadatadx.CredentialsFromFile("creds.txt")
defer creds.Close()

config := thetadatadx.ProductionConfig()
defer config.Close()

client, err := thetadatadx.Connect(creds, config)
if err != nil {
    log.Fatal(err)
}
defer client.Close()
```
```cpp [C++]
auto creds = tdx::Credentials::from_file("creds.txt");
auto client = tdx::Client::connect(creds, tdx::Config::production());
```
:::

## Endpoint Categories

| Category | Endpoints | Page |
|----------|-----------|------|
| Stocks | 14 endpoints - list, snapshots, history, at-time, streaming | [Stock Endpoints](./stock/) |
| Options | 34 endpoints - list, snapshots, history, Greeks, trade Greeks, at-time | [Option Endpoints](./option/) |
| Indices | 9 endpoints - list, snapshots, history, at-time | [Index Endpoints](./index-data/) |
| Calendar | 3 endpoints - trading schedule, holidays, early closes | [Calendar](./calendar/) |
| Rates | 1 endpoint - interest rate EOD history | [Rates](./rate/) |

## Date Format

All dates are `YYYYMMDD` strings: `"20240315"` for March 15, 2024.

## Interval Format

Intervals are millisecond strings: `"60000"` for 1 minute, `"300000"` for 5 minutes, `"3600000"` for 1 hour.

## DataFrame Support (Python)

Every historical method returns a typed list wrapper (`EodTickList`, `OhlcTickList`, `StringList`, ...). Chain `.to_polars()` / `.to_pandas()` / `.to_arrow()` / `.to_list()` on the return value for the matching representation:

```python
df  = tdx.stock_history_eod("AAPL", "20240101", "20240301").to_polars()
pdf = tdx.stock_history_eod("AAPL", "20240101", "20240301").to_pandas()
tbl = tdx.stock_history_eod("AAPL", "20240101", "20240301").to_arrow()
lst = tdx.stock_history_eod("AAPL", "20240101", "20240301").to_list()
```

The list wrapper itself behaves like a Python sequence (`len(ticks)`, `ticks[i]`, `for t in ticks:`), so most callers skip the terminal entirely.

Requires `pip install thetadatadx[pandas]` or `[polars]` / `[arrow]` depending on the terminal.

## Time Reference

| Time (ET) | `time_of_day` |
|-----------|---------------|
| 9:30 AM | `09:30:00.000` |
| 12:00 PM | `12:00:00.000` |
| 4:00 PM | `16:00:00.000` |

## Empty Responses

When a query returns no data (e.g., a non-trading date), the SDK returns an empty collection rather than an error. Check for emptiness using the appropriate idiom for your language:

::: code-group
```rust [Rust]
if eod.is_empty() {
    println!("No data for this date range");
}
```
```python [Python]
if not eod:
    print("No data for this date range")
```
```typescript [TypeScript]
if (eod.length === 0) {
    console.log('No data for this date range');
}
```
```go [Go]
if len(eod) == 0 {
    fmt.Println("No data for this date range")
}
```
```cpp [C++]
if (eod.empty()) {
    std::cout << "No data for this date range" << std::endl;
}
```
:::
