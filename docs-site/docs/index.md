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
      link: https://github.com/userFRM/ThetaDataDx

features:
  - icon:
      src: /icons/globe.svg
    title: "Multi-Language SDKs"
    details: "Native clients for Rust, Python, Go, and C++. Each SDK returns fully typed structures in its language's native idiom."
  - icon:
      src: /icons/bolt.svg
    title: "Real-Time Streaming"
    details: "FPSS streaming via LMAX Disruptor ring buffer (Rust) or polling (Python/Go/C++). Automatic heartbeat, reconnection logic built in."
  - icon:
      src: /icons/chart.svg
    title: "Options & Greeks"
    details: "Full option chain retrieval with 22 Black-Scholes Greeks, IV solver, open interest, and volatility surface construction."
  - icon:
      src: /icons/terminal.svg
    title: "CLI & Server Tools"
    details: "Standalone CLI for quick queries, an MCP server for AI-assisted workflows, and a REST+WS server as a drop-in Java terminal replacement."
---

<div class="install-section">

## Quick Install

::: code-group

```bash [Rust]
# Add to Cargo.toml
cargo add thetadatadx tokio --features tokio/rt-multi-thread,tokio/macros
```

```bash [Python]
pip install thetadatadx

# With pandas DataFrame support
pip install thetadatadx[pandas]
```

```bash [Go]
# Build the Rust FFI library first
cargo build --release -p thetadatadx-ffi
```

```bash [C++]
# Build the Rust FFI library first
cargo build --release -p thetadatadx-ffi

# Then link against libthetadatadx_ffi.so/.dylib
```

:::

### Minimal Example

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
    print(f"{q['date']}: bid={q['bid']:.2f} ask={q['ask']:.2f}")
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
for (auto& q : quotes) {
    std::cout << q.date << ": bid=" << q.bid
              << " ask=" << q.ask << std::endl;
}
```

:::

</div>
