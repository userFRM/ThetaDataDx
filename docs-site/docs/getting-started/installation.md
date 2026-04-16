---
title: Installation
description: Install ThetaDataDx for Rust, Python, Go, or C++.
---

# Installation

## SDK Installation

::: code-group
```toml [Rust]
# Add to your Cargo.toml
[dependencies]
thetadatadx = "7.2"
tokio = { version = "1", features = ["rt-multi-thread", "macros"] }
```
```bash [Python]
pip install thetadatadx

# With DataFrame support:
pip install thetadatadx[pandas]    # pandas DataFrames
pip install thetadatadx[polars]    # polars DataFrames
pip install thetadatadx[all]       # both

# Requires Python 3.9+. Pre-built wheels are provided - no Rust toolchain required.
```
```bash [Go]
# Prerequisites: Go 1.21+, Rust toolchain, C compiler (for CGo)

# First, build the Rust FFI library:
git clone https://github.com/userFRM/ThetaDataDx.git
cd ThetaDataDx
cargo build --release -p thetadatadx-ffi
# Produces target/release/libthetadatadx_ffi.so (Linux)
# or libthetadatadx_ffi.dylib (macOS)

# On Windows, the Go SDK links against the GNU Rust target instead:
rustup target add x86_64-pc-windows-gnu
cargo build --release --target x86_64-pc-windows-gnu -p thetadatadx-ffi
# Produces artifacts under target/x86_64-pc-windows-gnu/release/

# Then add the Go module:
go get github.com/userFRM/thetadatadx/sdks/go
```
```bash [C++]
# Prerequisites: C++17 compiler, CMake 3.16+, Rust toolchain

# First, build the Rust FFI library:
git clone https://github.com/userFRM/ThetaDataDx.git
cd ThetaDataDx
cargo build --release -p thetadatadx-ffi
# Produces target/release/libthetadatadx_ffi.so (Linux)
# or libthetadatadx_ffi.dylib (macOS)

# Then build the C++ SDK:
cd sdks/cpp
mkdir build && cd build
cmake ..
make
```
:::

## Building Python from Source

For unsupported platforms where pre-built wheels are not available:

```bash
pip install "maturin>=1.9.4,<2.0"
git clone https://github.com/userFRM/ThetaDataDx.git
cd ThetaDataDx/sdks/python
maturin develop --release
```

::: warning
Building from source requires a working Rust toolchain. Install it via [rustup.rs](https://rustup.rs) if you do not have one.
:::

## Memory Management

### Go

All Go SDK objects that wrap FFI handles must be closed when no longer needed:

```go
creds, _ := thetadatadx.CredentialsFromFile("creds.txt")
defer creds.Close()  // frees the Rust-side allocation

config := thetadatadx.ProductionConfig()
defer config.Close()

client, _ := thetadatadx.Connect(creds, config)
defer client.Close()
```

### C++

The C++ SDK uses RAII wrappers around the C FFI handles. All objects automatically free their resources when they go out of scope. No manual memory management required.

```cpp
{
    auto client = tdx::Client::connect(creds, tdx::Config::production());
    // ... use client ...
}  // client automatically freed here
```

All methods throw `std::runtime_error` on failure.

## Verify Installation

After installing, verify everything works by running a simple connectivity check:

::: code-group
```rust [Rust]
use thetadatadx::{ThetaDataDx, Credentials, DirectConfig};

#[tokio::main]
async fn main() -> Result<(), thetadatadx::Error> {
    let creds = Credentials::from_file("creds.txt")?;
    let client = ThetaDataDx::connect(&creds, DirectConfig::production()).await?;
    println!("Connected successfully");
    Ok(())
}
```
```python [Python]
from thetadatadx import Credentials, Config, ThetaDataDx

creds = Credentials.from_file("creds.txt")
client = ThetaDataDx(creds, Config.production())
print("Connected successfully")
```
```go [Go]
creds, err := thetadatadx.CredentialsFromFile("creds.txt")
if err != nil {
    log.Fatal(err)
}
defer creds.Close()

config := thetadatadx.ProductionConfig()
defer config.Close()

client, err := thetadatadx.Connect(creds, config)
if err != nil {
    log.Fatal(err)
}
defer client.Close()
fmt.Println("Connected successfully")
```
```cpp [C++]
auto creds = tdx::Credentials::from_file("creds.txt");
auto client = tdx::Client::connect(creds, tdx::Config::production());
std::cout << "Connected successfully" << std::endl;
```
:::
