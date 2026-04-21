---
layout: home

hero:
  name: "ThetaDataDx"
  text: "Direct-wire SDK for ThetaData"
  tagline: "Up to 9× faster and 75× less RAM than the ThetaData Python SDK, across Rust, Python, TypeScript, Go, and C++."
  actions:
    - theme: brand
      text: Get Started
      link: /getting-started/
    - theme: alt
      text: Benchmark
      link: /performance/benchmark
    - theme: alt
      text: GitHub
      link: https://github.com/userFRM/ThetaDataDx

features:
  - icon:
      src: /icons/globe.svg
    title: "Five native SDKs"
    details: "One Rust core, five native bindings: Rust, Python (PyO3, abi3), TypeScript/Node.js (napi-rs), Go (CGo), C++ (RAII). Same API shape, typed results in each language's idiom."
  - icon:
      src: /icons/bolt.svg
    title: "Real-time streaming"
    details: "FPSS client with SPKI certificate pinning, LMAX Disruptor SPSC ring (131,072 slots), configurable reconnect policy. The ThetaData Python SDK has no streaming at all."
  - icon:
      src: /icons/chart.svg
    title: "Local Greeks"
    details: "22 Black-Scholes Greeks plus an IV solver, computed in Rust with no server round-trip. ThetaData routes Greeks through the server; ThetaDataDx also ships the server-computed endpoints."
  - icon:
      src: /icons/terminal.svg
    title: "CLI, MCP, REST server"
    details: "Standalone CLI for quick queries, an MCP server for AI-assisted workflows, and a REST+WS server that drop-in replaces the Java terminal on port 25503."
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

### What you get

| Axis | ThetaData Python SDK | ThetaDataDx |
|------|----------------------|-------------|
| Languages | Python only | Rust, Python, TypeScript, Go, C++ |
| Historical endpoints | 61 | 61 (same coverage) |
| Real-time streaming | Not available | FPSS with SPKI pinning, SPSC ring, reconnect policy |
| Local Greeks calculator | Server round-trip | 22 Greeks + IV solver in Rust |
| Async Python surface | None | `*_async` variant of every endpoint |
| DataFrame output | polars default | Arrow / polars / pandas via explicit conversion |

Headline number: **5.60× faster wall-clock and up to 75× less peak RSS** on `option_history_greeks_all` (176k rows × 31 cols). [Full matrix](./performance/benchmark).

</div>
