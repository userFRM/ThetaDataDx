---
layout: home

hero:
  name: "ThetaDataDx"
  text: "Direct-Wire Market Data SDK"
  tagline: "No terminal. No overhead. Pure speed. Connect directly to ThetaData from Rust, Python, Go, and C++."
  actions:
    - theme: brand
      text: Get Started
      link: /getting-started/
    - theme: alt
      text: View on GitHub
      link: https://github.com/userFRM/thetadatadx

features:
  - icon: "🌐"
    title: "Multi-Language SDKs"
    details: "Native clients for Rust, Python, Go, and C++. Each SDK is idiomatic to its language with full type safety and zero-copy where possible."
  - icon: "⚡"
    title: "Real-Time Streaming"
    details: "Sub-millisecond WebSocket streaming for quotes, trades, and OHLC bars. Automatic reconnection and backpressure handling built in."
  - icon: "📊"
    title: "Options & Greeks"
    details: "Full option chain retrieval with Greeks, implied volatility, open interest, and volatility surface construction out of the box."
  - icon: "🛠️"
    title: "CLI & Server Tools"
    details: "Standalone CLI for quick queries, an MCP server for AI-assisted workflows, and a REST proxy for language-agnostic access."
---

<div class="install-section">

## Quick Install

::: code-group

```bash [Rust]
# Add to Cargo.toml
cargo add thetadatadx
```

```bash [Python]
pip install thetadatadx
```

```bash [Go]
go get github.com/userFRM/thetadatadx/sdks/go
```

```bash [C++]
# Via vcpkg
vcpkg install thetadatadx

# Or via CMake FetchContent
# See the C++ Getting Started guide
```

:::

### Minimal Example

::: code-group

```rust [Rust]
use thetadatadx::Client;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let client = Client::new().await?;

    let quotes = client
        .stock_quotes("AAPL")
        .date("2025-01-15")
        .send()
        .await?;

    for quote in &quotes {
        println!("{} bid={} ask={}", quote.timestamp, quote.bid, quote.ask);
    }
    Ok(())
}
```

```python [Python]
from thetadatadx import Client

client = Client()

quotes = client.stock_quotes(
    symbol="AAPL",
    date="2025-01-15"
)

for quote in quotes:
    print(f"{quote.timestamp} bid={quote.bid} ask={quote.ask}")
```

```go [Go]
package main

import (
    "fmt"
    "log"

    tdx "github.com/userFRM/thetadatadx/sdks/go"
)

func main() {
    client, err := tdx.NewClient()
    if err != nil {
        log.Fatal(err)
    }

    quotes, err := client.StockQuotes("AAPL", tdx.Date("2025-01-15"))
    if err != nil {
        log.Fatal(err)
    }

    for _, q := range quotes {
        fmt.Printf("%s bid=%f ask=%f\n", q.Timestamp, q.Bid, q.Ask)
    }
}
```

```cpp [C++]
#include <thetadatadx/client.hpp>
#include <iostream>

int main() {
    auto client = tdx::Client::create();

    auto quotes = client.stock_quotes("AAPL")
        .date("2025-01-15")
        .send();

    for (const auto& q : quotes) {
        std::cout << q.timestamp << " bid=" << q.bid
                  << " ask=" << q.ask << "\n";
    }
    return 0;
}
```

:::

</div>
