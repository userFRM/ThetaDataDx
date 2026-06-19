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

## 2. Authenticate

You can sign in with an API key or with your account email and password. An API key is the simpler option: a cleaner sign-in that does not require storing your account password. Email and password still works. Each form has three ways to supply the credential, so you can pick whichever fits your setup.

### API key

Generate an API key from your [ThetaData user portal](https://www.thetadata.net/), then supply it one of three ways.

**1. Pass it directly.** Hand the key straight to the api-key constructor.

<SdkTabs>

<template #rust>

```rust
let creds = thetadatadx::Credentials::api_key("your_api_key");
```

</template>

<template #python>

```python
creds = Credentials.from_api_key("your_api_key")
```

</template>

<template #typescript>

```typescript
const creds = Credentials.fromApiKey('your_api_key');
```

</template>

<template #cpp>

```cpp
auto creds = thetadatadx::Credentials::from_api_key("your_api_key");
```

</template>

<template #http>

```bash
thetadatadx-server --api-key "your_api_key" &
```

</template>

</SdkTabs>

**2. Environment variable.** Set `THETADATA_API_KEY` and let the SDK read it. `from_env_or_file` reads the variable when it is set and falls back to a `creds.txt` file otherwise, so the same code works in both setups.

```bash
export THETADATA_API_KEY="your_api_key"
```

<SdkTabs>

<template #rust>

```rust
let creds = thetadatadx::Credentials::from_env_or_file("creds.txt")?;
```

</template>

<template #python>

```python
creds = Credentials.from_env_or_file("creds.txt")
```

</template>

<template #typescript>

```typescript
const creds = Credentials.fromEnvOrFile('creds.txt');
```

</template>

<template #cpp>

```cpp
auto creds = thetadatadx::Credentials::from_env_or_file("creds.txt");
```

</template>

<template #http>

```bash
export THETADATA_API_KEY="your_api_key"
thetadatadx-server &
```

</template>

</SdkTabs>

**3. `.env` file.** Keep the key in a `.env` file (one `KEY=VALUE` per line) and point the SDK at it.

```
THETADATA_API_KEY="your_api_key"
```

<SdkTabs>

<template #rust>

```rust
let creds = thetadatadx::Credentials::from_dotenv(".env")?;
```

</template>

<template #python>

```python
creds = Credentials.from_dotenv(".env")
```

</template>

<template #typescript>

```typescript
const creds = Credentials.fromDotenv('.env');
```

</template>

<template #cpp>

```cpp
auto creds = thetadatadx::Credentials::from_dotenv(".env");
```

</template>

<template #http>

```bash
# Export the .env into the process environment first, then start the server.
export $(grep -v '^#' .env | xargs)
thetadatadx-server &
```

</template>

</SdkTabs>

### Email and password

Supply your account email and password one of three ways.

**1. Credentials file.** Create a `creds.txt` in your working directory: your ThetaData account email on line 1, password on line 2.

```
you@example.com
your-password
```

<SdkTabs>

<template #rust>

```rust
let creds = thetadatadx::Credentials::from_file("creds.txt")?;
```

</template>

<template #python>

```python
creds = Credentials.from_file("creds.txt")
```

</template>

<template #typescript>

```typescript
const creds = Credentials.fromFile('creds.txt');
```

</template>

<template #cpp>

```cpp
auto creds = thetadatadx::Credentials::from_file("creds.txt");
```

</template>

<template #http>

```bash
thetadatadx-server --creds creds.txt &
```

</template>

</SdkTabs>

**2. Pass them directly.** Hand the email and password straight to the constructor.

<SdkTabs>

<template #rust>

```rust
let creds = thetadatadx::Credentials::new("you@example.com", "your-password");
```

</template>

<template #python>

```python
creds = Credentials("you@example.com", "your-password")
```

</template>

<template #typescript>

```typescript
const creds = new Credentials('you@example.com', 'your-password');
```

</template>

<template #cpp>

```cpp
auto creds = thetadatadx::Credentials::from_email("you@example.com", "your-password");
```

</template>

<template #http>

```bash
# The HTTP server reads the pair from a credentials file.
thetadatadx-server --creds creds.txt &
```

</template>

</SdkTabs>

**3. Custom file path.** Point `from_file` at any path, not just `creds.txt` in the working directory.

<SdkTabs>

<template #rust>

```rust
let creds = thetadatadx::Credentials::from_file("/path/to/creds.txt")?;
```

</template>

<template #python>

```python
creds = Credentials.from_file("/path/to/creds.txt")
```

</template>

<template #typescript>

```typescript
const creds = Credentials.fromFile('/path/to/creds.txt');
```

</template>

<template #cpp>

```cpp
auto creds = thetadatadx::Credentials::from_file("/path/to/creds.txt");
```

</template>

<template #http>

```bash
thetadatadx-server --creds /path/to/creds.txt &
```

</template>

</SdkTabs>

No subscription yet? Create an account at [thetadata.net](https://www.thetadata.net/) — several endpoints work on the free tier (look for the Free badge on [reference pages](/reference/)).

## 3. First request

<SdkTabs>

<template #rust>

```rust
use thetadatadx::{Credentials, DirectConfig, Client};

#[tokio::main]
async fn main() -> Result<(), thetadatadx::Error> {
    // API key: reads THETADATA_API_KEY when set, else falls back to creds.txt.
    let creds = Credentials::from_env_or_file("creds.txt")?;
    // Or pass the key directly: Credentials::api_key(std::env::var("THETADATA_API_KEY")?)
    // Email + password: Credentials::from_file("creds.txt")?
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
import os

from thetadatadx import Config, Credentials, Client

# API key: reads THETADATA_API_KEY when set, else falls back to creds.txt.
creds = Credentials.from_env_or_file("creds.txt")
# Or pass the key directly: Credentials.from_api_key(os.environ["THETADATA_API_KEY"])
# Email + password: Credentials.from_file("creds.txt")
client = Client(creds, Config.production())

rows = client.historical.stock_history_eod("AAPL", "20250303", "20250306")
for t in rows:
    print(t.date, t.open, t.close, t.volume)
```

</template>

<template #typescript>

```typescript
import { Client, Credentials } from 'thetadatadx';

// API key: reads THETADATA_API_KEY when set, else falls back to creds.txt.
const client = await Client.connect(Credentials.fromEnvOrFile('creds.txt'));
// Or pass the key directly: Credentials.fromApiKey(process.env.THETADATA_API_KEY!)
// Email + password: Client.connectFromFile('creds.txt')

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
    // API key: reads THETADATA_API_KEY when set, else falls back to creds.txt.
    auto creds = thetadatadx::Credentials::from_env_or_file("creds.txt");
    // Or pass the key directly: thetadatadx::Credentials::from_api_key(std::getenv("THETADATA_API_KEY"))
    // Email + password: thetadatadx::Credentials::from_file("creds.txt")
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
# API key: reads THETADATA_API_KEY when set, or pass it with --api-key.
export THETADATA_API_KEY="your_api_key"
thetadatadx-server &
# Or pass the key directly: thetadatadx-server --api-key "$THETADATA_API_KEY" &
# Email + password: thetadatadx-server --creds creds.txt &

curl 'http://127.0.0.1:25503/v3/stock/history/eod?symbol=AAPL&start_date=2025-03-03&end_date=2025-03-06'
```

</template>

</SdkTabs>

Every endpoint follows this shape. Browse the [API Reference](/reference/) — each page carries the signature and a runnable sample in all five surfaces.

## Good to knows

- **Dates are `YYYYMMDD` strings** in the SDKs (`"20250303"`); the HTTP server also accepts ISO `YYYY-MM-DD`. Timestamps come back as milliseconds since midnight Eastern Time — see [Symbology & Contract Identity](/articles/symbology).
- **Connect once, reuse the client.** One client multiplexes any number of historical requests and an optional [streaming](/streaming/) session; per-request connections waste the authentication round trip.
- **Markets closed?** Connect with `Config.dev()` / `DirectConfig::dev()` to stream a replayed historical session, and prefer historical endpoints over snapshots on weekends.
- **Targeting staging?** Build the config from the staging preset (`Config.stage()` / `DirectConfig::stage()`) to point authentication, historical, and streaming all at the staging cluster. See [Configuration](/articles/configuration).
