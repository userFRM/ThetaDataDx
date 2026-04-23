---
title: Installation
description: Install ThetaDataDx for Rust, Python, TypeScript/Node.js, Go, or C++.
---

# Installation

ThetaDataDx ships five language surfaces from one Rust core. Pick your language and use the one-liner below.

## SDK installation

::: code-group
```toml [Rust]
# Add to your Cargo.toml
[dependencies]
thetadatadx = "7.3"
tokio = { version = "1", features = ["rt-multi-thread", "macros"] }
```
```bash [Python]
pip install thetadatadx

# With DataFrame adapters:
pip install thetadatadx[pandas]    # pandas
pip install thetadatadx[polars]    # polars
pip install thetadatadx[arrow]     # pyarrow only
pip install thetadatadx[all]       # all three

# Requires Python 3.9+. Pre-built abi3 wheels — no Rust toolchain required.
```
```bash [TypeScript]
npm install thetadatadx

# From source (requires Rust toolchain + Node.js 18+):
git clone https://github.com/userFRM/ThetaDataDx.git
cd ThetaDataDx/sdks/typescript
npm install
npm run build
```
```bash [Go]
# Prerequisites: Go 1.21+, Rust toolchain, C compiler (for CGo)

# Build the Rust FFI library:
git clone https://github.com/userFRM/ThetaDataDx.git
cd ThetaDataDx
cargo build --release -p thetadatadx-ffi
# Produces target/release/libthetadatadx_ffi.so (Linux)
# or libthetadatadx_ffi.dylib (macOS)

# On Windows, the Go SDK links against the GNU Rust target instead:
rustup target add x86_64-pc-windows-gnu
cargo build --release --target x86_64-pc-windows-gnu -p thetadatadx-ffi

# Then add the Go module:
go get github.com/userFRM/thetadatadx/sdks/go
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

The Python SDK ships a single `abi3` wheel per OS. One wheel built against Python 3.12 works on Python 3.9 → 3.14 without a per-version recompile, which is why the Rust toolchain is never needed on supported platforms.

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

## Memory management (Go / C++)

### Go

Every Go SDK handle that wraps an FFI pointer must be `Close()`d:

```go
creds, _ := thetadatadx.CredentialsFromFile("creds.txt")
defer creds.Close()

config := thetadatadx.ProductionConfig()
defer config.Close()

client, _ := thetadatadx.Connect(creds, config)
defer client.Close()
```

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
use thetadatadx::{ThetaDataDx, Credentials, DirectConfig};

#[tokio::main]
async fn main() -> Result<(), thetadatadx::Error> {
    let creds = Credentials::from_file("creds.txt")?;
    let _tdx = ThetaDataDx::connect(&creds, DirectConfig::production()).await?;
    println!("Connected successfully");
    Ok(())
}
```
```python [Python]
from thetadatadx import Credentials, Config, ThetaDataDx

creds = Credentials.from_file("creds.txt")
tdx = ThetaDataDx(creds, Config.production())
print("Connected successfully")
```
```typescript [TypeScript]
import { ThetaDataDx } from 'thetadatadx';

const tdx = await ThetaDataDx.connectFromFile('creds.txt');
console.log('Connected successfully');
```
```go [Go]
creds, err := thetadatadx.CredentialsFromFile("creds.txt")
if err != nil { log.Fatal(err) }
defer creds.Close()

config := thetadatadx.ProductionConfig()
defer config.Close()

client, err := thetadatadx.Connect(creds, config)
if err != nil { log.Fatal(err) }
defer client.Close()
fmt.Println("Connected successfully")
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
