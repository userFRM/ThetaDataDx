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
thetadatadx = "13.0.0"
```

The market-data client is async; call it from your application's async runtime.

</template>

<template #python>

```bash
pip install thetadatadx

# Optional DataFrame adapters:
pip install "thetadatadx[pandas]"    # pandas
pip install "thetadatadx[polars]"    # polars
pip install "thetadatadx[arrow]"     # pyarrow only
```

Requires Python 3.12+. Pre-built `abi3` wheels for Linux x86_64, macOS, and Windows — no Rust toolchain needed. On other platforms, build from source with [maturin](https://www.maturin.rs/) (`maturin develop --release` in `thetadatadx-py`).

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

cd thetadatadx-cpp
cmake -B build && cmake --build build
```

Include `thetadatadx-cpp/include/thetadatadx.hpp`, compile it together with the implementation file `thetadatadx-cpp/src/thetadatadx.cpp`, and link the built `thetadatadx_ffi` library (the provided CMake target wires this up). Any other language with C interop can target the same C ABI directly.

</template>

<template #http>

```bash
cargo install thetadatadx-server --git https://github.com/userFRM/ThetaDataDx
```

The [server binary](/server/) exposes the full surface as local HTTP REST and WebSocket endpoints — no SDK required. Existing scripts written against the v3 route surface point at it unchanged.

</template>

</SdkTabs>

## 2. Authenticate

Pass your API key directly to the client and you are one line from a live connection. Generate a key from your [ThetaData user portal](https://www.thetadata.net/), hand it to the client, and you have a connected client ready to make requests.

<SdkTabs>

<template #rust>

```rust
// API key inline, production by default.
let client = thetadatadx::Client::builder()
    .api_key("your_api_key")
    .connect()
    .await?;
```

</template>

<template #python>

```python
# API key inline, production by default.
client = Client(api_key="your_api_key")
```

</template>

<template #typescript>

```typescript
// API key inline, production by default.
const client = await Client.connectWith({ apiKey: 'your_api_key' });
```

</template>

<template #cpp>

```cpp
// API key inline, production by default.
auto client = thetadatadx::Client::builder()
    .api_key("your_api_key")
    .connect();
```

</template>

<template #http>

```bash
thetadatadx-server --api-key "your_api_key" &
```

</template>

</SdkTabs>

The same one-step construction takes the key from the environment or a `.env` file, takes an email and password instead, and selects the staging cluster. Email and password is also supported; it is shown alongside the api-key forms below.

<SdkTabs>

<template #rust>

```rust
// Source the key from THETADATA_API_KEY, or from a .env file.
let client = thetadatadx::Client::builder().api_key_from_env().connect().await?;
let client = thetadatadx::Client::builder().api_key_from_dotenv(".env").connect().await?;

// Email and password inline, staging environment.
let client = thetadatadx::Client::builder()
    .email_password("you@example.com", "your-password")
    .stage()
    .connect()
    .await?;
```

</template>

<template #python>

```python
# Source the key from THETADATA_API_KEY, or from a .env file.
client = Client.from_env()
client = Client.from_dotenv(".env")

# Email and password inline, staging environment.
client = Client(email="you@example.com", password="your-password", market_data_type="STAGE")
```

</template>

<template #typescript>

```typescript
// Source the key from THETADATA_API_KEY, or from a .env file.
const fromEnv = await Client.connectWith({ apiKeyFromEnv: true });
const fromDotenv = await Client.connectWith({ apiKeyFromDotenv: '.env' });

// Email and password inline, staging environment.
const withLogin = await Client.connectWith({
  email: 'you@example.com',
  password: 'your-password',
  marketDataType: 'STAGE',
});
```

</template>

<template #cpp>

```cpp
// Source the key from THETADATA_API_KEY, or from a .env file.
auto fromEnv = thetadatadx::Client::builder().api_key_from_env().connect();
auto fromDotenv = thetadatadx::Client::builder().api_key_from_dotenv(".env").connect();

// Email and password inline, staging environment.
auto staged = thetadatadx::Client::builder()
    .email_password("you@example.com", "your-password")
    .stage()
    .connect();
```

</template>

<template #http>

```bash
# The server takes the same inputs as flags.
thetadatadx-server --api-key "your_api_key" &
```

</template>

</SdkTabs>

### Credential sources

The client resolves any of the credential sources below. They are the building blocks behind the one-step construction above: an API key supplied inline, from the environment, or from a `.env` file; or an email and password supplied from a file, inline, or at a custom path. Each one also produces a standalone `Credentials` value you can hold and pass to the lower-level `connect` when you want full control over hosts and tuning (see "Full control" at the end of this section).

#### API key

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

#### Email and password

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

### Full control

The one-step construction at the top of this section is a convenience over the typed path: build a `Credentials` and a `Config` yourself and pass both to the lower-level `connect`. Reach for this when you need to override hosts, timeouts, or other tuning knobs on the `Config`.

<SdkTabs>

<template #rust>

```rust
let creds = thetadatadx::Credentials::from_file("creds.txt")?;
let client = thetadatadx::Client::connect(&creds, thetadatadx::DirectConfig::production()).await?;
```

</template>

<template #python>

```python
client = Client(Credentials.from_file("creds.txt"), Config.production())
```

</template>

<template #typescript>

```typescript
const client = await Client.connect(Credentials.fromFile('creds.txt'));
```

</template>

<template #cpp>

```cpp
auto creds = thetadatadx::Credentials::from_file("creds.txt");
auto client = thetadatadx::Client::connect(creds, thetadatadx::Config::production());
```

</template>

<template #http>

```bash
thetadatadx-server --creds creds.txt &
```

</template>

</SdkTabs>

## 3. First request

<SdkTabs>

<template #rust>

```rust
use thetadatadx::Client;

async fn run() -> Result<(), thetadatadx::Error> {
    // Pass your API key directly. Add .stage() before .connect() for staging.
    let client = Client::builder().api_key("your_api_key").connect().await?;

    let rows = client.market_data().stock_history_eod("AAPL", "20250303", "20250306").await?;
    for t in &rows {
        println!("{}: open={} close={} volume={}", t.date, t.open, t.close, t.volume);
    }
    Ok(())
}
```

</template>

<template #python>

```python
from thetadatadx import Client

# Pass your API key directly. Use market_data_type="STAGE" to target staging.
client = Client(api_key="your_api_key")

rows = client.market_data.stock_history_eod("AAPL", "20250303", "20250306")
for t in rows:
    print(t.date, t.open, t.close, t.volume)
```

</template>

<template #typescript>

```typescript
import { Client } from 'thetadatadx';

// Pass your API key directly. Add marketDataType: 'STAGE' to target staging.
const client = await Client.connectWith({ apiKey: 'your_api_key' });

const rows = await client.marketData.stockHistoryEOD('AAPL', '20250303', '20250306');
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
    // Pass your API key directly. Add .stage() before .connect() for staging.
    auto client = thetadatadx::Client::builder()
        .api_key("your_api_key")
        .connect();

    auto rows = client.market_data().stock_history_eod("AAPL", "20250303", "20250306");
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
- **Connect once, reuse the client.** One client multiplexes any number of market-data requests and an optional [streaming](/streaming/) session; per-request connections waste the authentication round trip.
- **Markets closed?** Connect with `Config.dev()` / `DirectConfig::dev()` to stream a replayed market-data session, and prefer market-data endpoints over snapshots on weekends.
- **Targeting staging or dev?** The market-data and streaming environments are selected independently. Pick the market-data staging cluster with `DirectConfig::production().with_market_data_environment(MarketDataEnvironment::Stage)` (or the `Config.stage()` / `DirectConfig::stage()` preset), the `THETADATA_MARKET_DATA_TYPE=STAGE` environment variable, or a `.env` file. Pick the streaming dev-replay cluster with `with_streaming_environment(StreamingEnvironment::Dev)` (or the `dev()` preset) or `THETADATA_STREAMING_TYPE=DEV`. The market-data environment also sets the authentication marker; the streaming environment does not. All paths work with either credential type, and one `.env` file can hold `THETADATA_API_KEY`, `THETADATA_MARKET_DATA_TYPE`, and `THETADATA_STREAMING_TYPE` together. See [Configuration](/articles/configuration).
