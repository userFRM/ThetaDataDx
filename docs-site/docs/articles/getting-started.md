---
title: Getting Started
description: Install the SDK, save your credentials, and make your first request in Rust, Python, TypeScript, or C++.
---

# Getting Started

ThetaDataDx connects directly to ThetaData's servers — nothing to install and babysit locally. Pick a language, install, save your credentials, and make a request.

## 1. Install

<SdkTabs>

<template #rust>

```toml
# Cargo.toml
[dependencies]
thetadatadx = "13.0.0-rc.1"
tokio = { version = "1", features = ["rt-multi-thread", "macros"] }
```

The historical client is async; `tokio` provides the runtime.

</template>

<template #python>

```bash
pip install thetadatadx

# Optional DataFrame adapters:
pip install thetadatadx[pandas]    # pandas
pip install thetadatadx[polars]    # polars
pip install thetadatadx[arrow]     # pyarrow only
```

Requires Python 3.12+. Pre-built `abi3` wheels for Linux x86_64, macOS, and Windows — no Rust toolchain needed. On other platforms, build from source with [maturin](https://www.maturin.rs/) (`maturin develop --release` in `sdks/python`).

</template>

<template #typescript>

```bash
npm install thetadatadx
```

Requires Node.js 20+. Pre-built native binaries install automatically per platform.

</template>

<template #cpp>

```bash
# Prerequisites: C++17 compiler, CMake 3.16+, Rust toolchain
git clone https://github.com/userFRM/ThetaDataDx.git
cd ThetaDataDx
cargo build --release -p thetadatadx-ffi

cd sdks/cpp
cmake -B build && cmake --build build
```

Include `sdks/cpp/include/thetadatadx.hpp`, compile it together with the implementation file `sdks/cpp/src/thetadatadx.cpp`, and link the built `thetadatadx_ffi` library (the provided CMake target wires this up). Any other language with C interop can target the same C ABI directly.

</template>

<template #http>

```bash
cargo install thetadatadx-server --git https://github.com/userFRM/ThetaDataDx
```

The [server binary](/server/) exposes the full surface as local HTTP REST and WebSocket endpoints — no SDK required. Existing scripts written against the v3 route surface point at it unchanged.

</template>

</SdkTabs>

## 2. Save credentials

Create a `creds.txt` in your working directory: your ThetaData account email on line 1, password on line 2.

```
you@example.com
your-password
```

No subscription yet? Create an account at [thetadata.net](https://www.thetadata.net/) — several endpoints work on the free tier (look for the Free badge on [reference pages](/reference/)).

## 3. First request

<SdkTabs>

<template #rust>

```rust
use thetadatadx::{Credentials, DirectConfig, Client};

#[tokio::main]
async fn main() -> Result<(), thetadatadx::Error> {
    let creds = Credentials::from_file("creds.txt")?;
    let client = Client::connect(&creds, DirectConfig::production()).await?;

    let rows = client.historical().stock_history_eod("AAPL", "20250303", "20250306").await?;
    for t in &rows {
        println!("{}: open={} close={} volume={}", t.date, t.open, t.close, t.volume);
    }
    Ok(())
}
```

</template>

<template #python>

```python
from thetadatadx import Config, Credentials, Client

creds = Credentials.from_file("creds.txt")
client = Client(creds, Config.production())

rows = client.historical.stock_history_eod("AAPL", "20250303", "20250306")
for t in rows:
    print(t.date, t.open, t.close, t.volume)
```

</template>

<template #typescript>

```typescript
import { Client } from 'thetadatadx';

const client = await Client.connectFromFile('creds.txt');

const rows = await client.historical.stockHistoryEOD('AAPL', '20250303', '20250306');
for (const t of rows) {
  console.log(t.date, t.open, t.close, t.volume);
}
```

</template>

<template #cpp>

```cpp
#include "thetadatadx.hpp"
#include <iostream>

int main() {
    auto creds = thetadatadx::Credentials::from_file("creds.txt");
    auto client = thetadatadx::HistoricalClient::connect(creds, thetadatadx::Config::production());

    auto rows = client.stock_history_eod("AAPL", "20250303", "20250306");
    for (const auto& t : rows) {
        std::cout << t.date << ": open=" << t.open
                  << " close=" << t.close << " volume=" << t.volume << "\n";
    }
}
```

</template>

<template #http>

```bash
thetadatadx-server --creds creds.txt &

curl 'http://127.0.0.1:25503/v3/stock/history/eod?symbol=AAPL&start_date=2025-03-03&end_date=2025-03-06'
```

</template>

</SdkTabs>

Every endpoint follows this shape. Browse the [API Reference](/reference/) — each page carries the signature and a runnable sample in all five surfaces.

## Good to knows

- **Dates are `YYYYMMDD` strings** in the SDKs (`"20250303"`); the HTTP server also accepts ISO `YYYY-MM-DD`. Timestamps come back as milliseconds since midnight Eastern Time — see [Symbology & Contract Identity](/articles/symbology).
- **Connect once, reuse the client.** One client multiplexes any number of historical requests and an optional [streaming](/streaming/) session; per-request connections waste the authentication round trip.
- **Markets closed?** Connect with `Config.dev()` / `DirectConfig::dev()` to stream a replayed historical session, and prefer historical endpoints over snapshots on weekends.
