---
layout: home

hero:
  name: "ThetaDataDx"
  text: "Rust SDK for ThetaData market data"
  tagline: "Historical (MDDS gRPC) and real-time (FPSS) surfaces plus a local Greeks calculator, exposed in Rust, Python, TypeScript, Go, and C++ from a single Rust core."
  actions:
    - theme: brand
      text: Get Started
      link: /getting-started/
    - theme: alt
      text: Quick Start
      link: /getting-started/quickstart
    - theme: alt
      text: GitHub
      link: https://github.com/userFRM/ThetaDataDx

features:
  - icon:
      src: /icons/globe.svg
    title: "Five language surfaces"
    details: "One Rust core, five bindings: Rust, Python (PyO3, abi3), TypeScript/Node.js (napi-rs), Go (CGo), C++ (RAII header-only). Same API shape, typed results in each language's idiom."
  - icon:
      src: /icons/bolt.svg
    title: "Real-time streaming"
    details: "FPSS client with SPKI certificate pinning, SPSC ring buffer (131,072 slots), configurable reconnect policy. Subscribes to quotes, trades, open interest, and full-stream firehoses."
  - icon:
      src: /icons/chart.svg
    title: "Local Greeks"
    details: "22 Black-Scholes Greeks plus an IV solver, computed in Rust with no server round-trip. The server-computed Greeks endpoints are also exposed."
  - icon:
      src: /icons/terminal.svg
    title: "CLI, MCP, REST server"
    details: "Standalone CLI for one-off queries, an MCP server that exposes every generated historical endpoint plus three offline tools to any MCP-compatible client, and a REST + WebSocket server on port 25503."
---

<div class="install-section">

## Quick install

::: code-group

```bash [Rust]
cargo add thetadatadx tokio --features tokio/rt-multi-thread,tokio/macros
```

```bash [Python]
pip install thetadatadx

# With DataFrame adapters
pip install thetadatadx[pandas]
```

```bash [TypeScript]
npm install thetadatadx
```

```bash [Go]
# Build the FFI library once, then:
go get github.com/userFRM/thetadatadx/sdks/go
```

```bash [C++]
# Build the FFI library once, then include sdks/cpp/include/thetadx.hpp
```

:::

### Minimal example

::: code-group

```rust [Rust]
use thetadatadx::{ThetaDataDx, Credentials, DirectConfig};

#[tokio::main]
async fn main() -> Result<(), thetadatadx::Error> {
    let creds = Credentials::from_file("creds.txt")?;
    let tdx = ThetaDataDx::connect(&creds, DirectConfig::production()).await?;

    let quotes = tdx.stock_history_quote("AAPL", "20250115", "60000").await?;
    for q in &quotes {
        println!("{}: bid={} ask={}", q.date, q.bid, q.ask);
    }
    Ok(())
}
```

```python [Python]
from thetadatadx import ThetaDataDx, Credentials, Config

creds = Credentials.from_file("creds.txt")
tdx = ThetaDataDx(creds, Config.production())

quotes = tdx.stock_history_quote("AAPL", "20250115", "60000")
for q in quotes:
    print(f"{q.date}: bid={q.bid:.2f} ask={q.ask:.2f}")
```

```typescript [TypeScript]
import { ThetaDataDx } from 'thetadatadx';

const tdx = await ThetaDataDx.connectFromFile('creds.txt');

const quotes = tdx.stockHistoryQuote('AAPL', '20250115', '60000');
for (const q of quotes) {
    console.log(`${q.date}: bid=${q.bid} ask=${q.ask}`);
}
```

```go [Go]
creds, _ := thetadatadx.CredentialsFromFile("creds.txt")
defer creds.Close()

config := thetadatadx.ProductionConfig()
defer config.Close()

client, _ := thetadatadx.Connect(creds, config)
defer client.Close()

quotes, _ := client.StockHistoryQuote("AAPL", "20250115", "60000")
for _, q := range quotes {
    fmt.Printf("%d: bid=%.2f ask=%.2f\n", q.Date, q.Bid, q.Ask)
}
```

```cpp [C++]
#include "thetadx.hpp"

auto creds = tdx::Credentials::from_file("creds.txt");
auto client = tdx::Client::connect(creds, tdx::Config::production());

auto quotes = client.stock_history_quote("AAPL", "20250115", "60000");
for (const auto& q : quotes) {
    std::cout << q.date << ": bid=" << q.bid << " ask=" << q.ask << std::endl;
}
```

:::

### What ships

| Axis | ThetaDataDx |
|------|-------------|
| Languages | Rust, Python, TypeScript, Go, C++ |
| Historical endpoints | Full typed historical surface (plus 4 `_stream` SDK-only variants) |
| Real-time streaming | FPSS with SPKI pinning, SPSC ring, reconnect policy |
| Local Greeks calculator | 22 Greeks + IV solver in Rust |
| Async Python surface | `*_async` variant of every endpoint |
| DataFrame output | Arrow / polars / pandas via explicit conversion |

Historical decode runs in Rust and materialises typed structs (or an Arrow `RecordBatch`, zero-copy at the PyO3 boundary).

</div>
