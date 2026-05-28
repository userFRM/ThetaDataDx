---
layout: home

hero:
  name: "ThetaDataDx"
  text: "Rust SDK for ThetaData market data"
  tagline: "Three public surfaces — historical request/response (MDDS gRPC), real-time streaming (FPSS), and whole-universe daily blobs (FLATFILES) — plus a local Greeks calculator, exposed in Rust, Python, TypeScript, and C++ from a single Rust core."
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
    title: "Four language surfaces"
    details: "One Rust core, four bindings: Rust, Python (PyO3, abi3), TypeScript/Node.js (napi-rs), C++ (RAII header-only). Same API shape, typed results in each language's idiom. The C ABI in `ffi/` is also the supported integration path for any third-party Go/C consumer that wants to roll their own wrapper."
  - icon:
      src: /icons/bolt.svg
    title: "Real-time streaming"
    details: "FPSS client with SPKI certificate pinning, SPSC ring buffer (131,072 slots), configurable reconnect policy. Subscribes to quotes, trades, open interest, and full-stream subscriptions."
  - icon:
      src: /icons/chart.svg
    title: "Local Greeks"
    details: "23 Black-Scholes Greeks plus an IV solver, computed in Rust with no server round-trip. The server-computed Greeks endpoints are also exposed."
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

```bash [C++]
# Build the FFI library once, then include sdks/cpp/include/thetadx.hpp
```

:::

### Minimal example

::: code-group

```rust [Rust]
use thetadatadx::{ThetaDataDxClient, Credentials, DirectConfig};

#[tokio::main]
async fn main() -> Result<(), thetadatadx::Error> {
    let creds = Credentials::from_file("creds.txt")?;
    let client = ThetaDataDxClient::connect(&creds, DirectConfig::production()).await?;

    let quotes = client.stock_history_quote("AAPL", "20250115", "60000").await?;
    for q in &quotes {
        println!("{}: bid={} ask={}", q.date, q.bid, q.ask);
    }
    Ok(())
}
```

```python [Python]
from thetadatadx import ThetaDataDxClient, Credentials, Config

creds = Credentials.from_file("creds.txt")
client = ThetaDataDxClient(creds, Config.production())

quotes = client.stock_history_quote("AAPL", "20250115", "60000")
for q in quotes:
    print(f"{q.date}: bid={q.bid:.2f} ask={q.ask:.2f}")
```

```typescript [TypeScript]
import { ThetaDataDxClient } from 'thetadatadx';

const client = await ThetaDataDxClient.connectFromFile('creds.txt');

const quotes = client.stockHistoryQuote('AAPL', '20250115', '60000');
for (const q of quotes) {
    console.log(`${q.date}: bid=${q.bid} ask=${q.ask}`);
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
| Languages | Rust, Python, TypeScript, C++ |
| Historical endpoints | Full typed historical surface (plus 4 `_stream` SDK-only variants) |
| Real-time streaming | FPSS with SPKI pinning, SPSC ring, reconnect policy |
| Local Greeks calculator | 23 Greeks + IV solver in Rust |
| Async Python surface | `*_async` variant of every endpoint |
| DataFrame output | Arrow / polars / pandas via explicit conversion |

Historical decode runs in Rust and materialises typed structs (or an Arrow `RecordBatch`, zero-copy at the PyO3 boundary).

</div>
