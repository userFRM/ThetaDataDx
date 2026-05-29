---
title: Installation
description: Install ThetaDataDx for Rust, Python, TypeScript/Node.js, or C++.
---

# Installation

ThetaDataDx ships four language surfaces from one Rust core. Pick your language and use the one-liner below. Go consumers can build their own cgo wrapper against the unchanged C ABI in [`ffi/`](https://github.com/userFRM/ThetaDataDx/tree/main/ffi).

## SDK installation

::: code-group
```toml [Rust]
# Add to your Cargo.toml
[dependencies]
thetadatadx = "11"
tokio = { version = "1", features = ["rt-multi-thread", "macros"] }
```
```bash [Python]
pip install thetadatadx

# With DataFrame adapters:
pip install thetadatadx[pandas]    # pandas
pip install thetadatadx[polars]    # polars
pip install thetadatadx[arrow]     # pyarrow only
pip install thetadatadx[all]       # all three

# Requires Python 3.12+. Pre-built abi3 wheels — no Rust toolchain required.
```
```bash [TypeScript]
npm install thetadatadx

# From source (requires Rust toolchain + Node.js 18+):
git clone https://github.com/userFRM/ThetaDataDx.git
cd ThetaDataDx/sdks/typescript
npm install
npm run build
```
```bash [C++]
# Prerequisites: C++17 compiler, CMake 3.16+, Rust toolchain

# Build the Rust FFI library:
git clone https://github.com/userFRM/ThetaDataDx.git
cd ThetaDataDx
cargo build --release -p thetadatadx-ffi

# Build the C++ SDK:
cd sdks/cpp
mkdir build && cd build
cmake ..
make

# The C++ header lives at sdks/cpp/include/thetadx.hpp and pulls in
# endpoint_options.hpp.inc alongside the generated C header.
```
:::

## Python abi3 wheels

The Python SDK ships a single `abi3` wheel per OS. One wheel built against Python 3.12 works on Python 3.12 → 3.14 without a per-version recompile, which is why the Rust toolchain is never needed on supported platforms.

| Platform | Wheel |
|----------|-------|
| Linux x86_64 (`manylinux2014`) | pre-built |
| macOS (universal2) | pre-built |
| Windows x86_64 | pre-built |
| Anything else | build from source (`maturin develop --release`) |

### Building Python from source

```bash
pip install "maturin>=1.9.4,<2.0"
git clone https://github.com/userFRM/ThetaDataDx.git
cd ThetaDataDx/sdks/python
maturin develop --release
```

::: warning
Building from source requires a working Rust toolchain. Install it via [rustup.rs](https://rustup.rs).
:::

## Memory management (C++)

### C++

The C++ SDK uses RAII wrappers around the C FFI handles. Resources free on scope exit; no manual cleanup.

```cpp
{
    auto client = tdx::Client::connect(creds, tdx::Config::production());
    // use client
}  // freed here
```

Methods throw `std::runtime_error` on failure.

## Verify the install

::: code-group
```rust [Rust]
use thetadatadx::{ThetaDataDxClient, Credentials, DirectConfig};

#[tokio::main]
async fn main() -> Result<(), thetadatadx::Error> {
    let creds = Credentials::from_file("creds.txt")?;
    let _tdx = ThetaDataDxClient::connect(&creds, DirectConfig::production()).await?;
    println!("Connected successfully");
    Ok(())
}
```
```python [Python]
from thetadatadx import Credentials, Config, ThetaDataDxClient

creds = Credentials.from_file("creds.txt")
tdx = ThetaDataDxClient(creds, Config.production())
print("Connected successfully")
```
```typescript [TypeScript]
import { ThetaDataDxClient } from 'thetadatadx';

const tdx = await ThetaDataDxClient.connectFromFile('creds.txt');
console.log('Connected successfully');
```
```cpp [C++]
auto creds = tdx::Credentials::from_file("creds.txt");
auto client = tdx::Client::connect(creds, tdx::Config::production());
std::cout << "Connected successfully" << std::endl;
```
:::

## Next

- [Authentication](./authentication) — credentials file, env vars, session lifecycle
- [First query](./first-query) — one historical call in every language
