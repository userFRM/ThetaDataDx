---
title: Historical Data
description: Overview of ThetaDataDx historical data endpoints for stocks, options, indices, rates, and calendars.
---

# Historical Data

ThetaDataDx provides 61 historical data endpoints across five asset categories. All historical data is accessed through the ThetaDataDx client, which communicates over gRPC with ThetaData's MDDS servers. Every call runs through compiled Rust - gRPC, protobuf parsing, zstd decompression, and FIT decoding all happen at native speed, regardless of which SDK you use.

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
| Stocks | 14 endpoints - list, snapshots, history, at-time, streaming | [Stock Endpoints](./stock) |
| Options | 34 endpoints - list, snapshots, history, Greeks, trade Greeks, at-time | [Option Endpoints](./option) |
| Indices | 9 endpoints - list, snapshots, history, at-time | [Index Endpoints](./index-data/) |
| Calendar | 3 endpoints - trading schedule, holidays, early closes | [Calendar](./calendar/) |
| Rates | 1 endpoint - interest rate EOD history | [Rates](./rate/) |

## Date Format

All dates are `YYYYMMDD` strings: `"20240315"` for March 15, 2024.

## Interval Format

Intervals are millisecond strings: `"60000"` for 1 minute, `"300000"` for 5 minutes, `"3600000"` for 1 hour.

## DataFrame Support (Python)

All data methods have `_df` variants that return pandas DataFrames directly:

```python
df = tdx.stock_history_eod_df("AAPL", "20240101", "20240301")
```

Or convert any result explicitly:

```python
from thetadatadx import to_dataframe

eod = tdx.stock_history_eod("AAPL", "20240101", "20240301")
df = to_dataframe(eod)
```

Requires `pip install thetadatadx[pandas]`.

## Time Reference

| Time (ET) | Milliseconds |
|-----------|-------------|
| 9:30 AM | `34200000` |
| 12:00 PM | `43200000` |
| 4:00 PM | `57600000` |

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
