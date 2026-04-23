---
title: First Query
description: Run your first ThetaDataDx historical call in Rust, Python, TypeScript, Go, or C++.
---

# First Query

One historical call, every SDK, side by side. Each snippet assumes you have a `creds.txt` file (see [Authentication](./authentication)) and an active ThetaData subscription with stock EOD access (the free tier is sufficient).

## End-of-day stock history

::: code-group
```rust [Rust]
use thetadatadx::{ThetaDataDx, Credentials, DirectConfig};

#[tokio::main]
async fn main() -> Result<(), thetadatadx::Error> {
    let creds = Credentials::from_file("creds.txt")?;
    let tdx = ThetaDataDx::connect(&creds, DirectConfig::production()).await?;

    let eod = tdx.stock_history_eod("AAPL", "20240101", "20240301").await?;
    for tick in &eod {
        println!("{}: O={:.2} H={:.2} L={:.2} C={:.2} V={}",
            tick.date, tick.open, tick.high, tick.low, tick.close, tick.volume);
    }
    Ok(())
}
```
```python [Python]
from thetadatadx import Credentials, Config, ThetaDataDx

creds = Credentials.from_file("creds.txt")
tdx = ThetaDataDx(creds, Config.production())

eod = tdx.stock_history_eod("AAPL", "20240101", "20240301")
for tick in eod:
    print(f"{tick.date}: O={tick.open:.2f} H={tick.high:.2f} "
          f"L={tick.low:.2f} C={tick.close:.2f} V={tick.volume}")
```
```typescript [TypeScript]
import { ThetaDataDx } from 'thetadatadx';

const tdx = await ThetaDataDx.connectFromFile('creds.txt');

const eod = tdx.stockHistoryEOD('AAPL', '20240101', '20240301');
for (const tick of eod) {
    console.log(`${tick.date}: O=${tick.open} H=${tick.high} L=${tick.low} C=${tick.close} V=${tick.volume}`);
}
```
```go [Go]
package main

import (
    "fmt"
    "log"

    thetadatadx "github.com/userFRM/thetadatadx/sdks/go"
)

func main() {
    creds, err := thetadatadx.CredentialsFromFile("creds.txt")
    if err != nil { log.Fatal(err) }
    defer creds.Close()

    config := thetadatadx.ProductionConfig()
    defer config.Close()

    client, err := thetadatadx.Connect(creds, config)
    if err != nil { log.Fatal(err) }
    defer client.Close()

    eod, err := client.StockHistoryEOD("AAPL", "20240101", "20240301")
    if err != nil { log.Fatal(err) }
    for _, tick := range eod {
        fmt.Printf("%d: O=%.2f H=%.2f L=%.2f C=%.2f V=%d\n",
            tick.Date, tick.Open, tick.High, tick.Low, tick.Close, tick.Volume)
    }
}
```
```cpp [C++]
#include "thetadx.hpp"
#include <iomanip>
#include <iostream>

int main() {
    auto creds = tdx::Credentials::from_file("creds.txt");
    auto client = tdx::Client::connect(creds, tdx::Config::production());

    auto eod = client.stock_history_eod("AAPL", "20240101", "20240301");
    for (const auto& tick : eod) {
        std::cout << tick.date
                  << std::fixed << std::setprecision(2)
                  << ": O=" << tick.open << " H=" << tick.high
                  << " L=" << tick.low  << " C=" << tick.close
                  << " V=" << tick.volume << std::endl;
    }
}
```
:::

### Sample response

```json
[
  {"date": 20240102, "open": 187.15, "high": 188.44, "low": 183.89, "close": 185.64, "volume": 82488682},
  {"date": 20240103, "open": 184.22, "high": 185.88, "low": 183.43, "close": 184.25, "volume": 58414500},
  {"date": 20240104, "open": 182.15, "high": 183.09, "low": 180.88, "close": 181.91, "volume": 71983600}
]
```

Every historical endpoint returns a typed tick list with the same JSON-like shape across all five SDKs. Numeric fields are decoded to `f64` / `double` at parse time — no `Price{value, type}` objects to unpack.

## What runs under the hood

1. `connect()` loads credentials and authenticates against ThetaData's Nexus endpoint, retrieving a session UUID.
2. The session UUID is attached to an HTTP/2 (`tonic` / `gRPC`) channel to the MDDS datacenter.
3. `stock_history_eod(...)` streams a compressed protobuf `DataTable` response.
4. A Rust decoder turns the `DataTable` into a `Vec<StockEodTick>` (~86k rows/sec per core on wide-schema data). The Python / TypeScript / Go / C++ bindings expose this slice as the language's native collection.

## Next

- [Streaming (FPSS)](./streaming) — live quotes, trades, open interest, OHLC with SPKI pinning
- [DataFrames](./dataframes) — convert tick lists to Arrow / Polars / Pandas
- [Greeks calculator](./greeks) — 22 local Greeks + IV solver
