# Getting Started with C++

## Prerequisites

- C++17 compiler
- CMake 3.16+
- Rust toolchain (for building the FFI library)

## Installation

First, build the Rust FFI library:

```bash
git clone https://github.com/userFRM/ThetaDataDx.git
cd ThetaDataDx
cargo build --release -p thetadatadx-ffi
```

This produces `target/release/libthetadatadx_ffi.so` (Linux) or `libthetadatadx_ffi.dylib` (macOS).

Then build the C++ SDK:

```bash
cd sdks/cpp
mkdir build && cd build
cmake ..
make
```

## Credentials

Create a `creds.txt` file with your ThetaData email on line 1 and password on line 2:

```text
your-email@example.com
your-password
```

## First Query

```cpp
#include "thetadatadx.hpp"
#include <iostream>
#include <iomanip>

int main() {
    // Load credentials
    auto creds = tdx::Credentials::from_file("creds.txt");

    // Connect
    auto client = tdx::Client::connect(creds, tdx::Config::production());

    // Fetch end-of-day data
    auto eod = client.stock_history_eod("AAPL", "20240101", "20240301");
    for (auto& tick : eod) {
        std::cout << tick.date << ": O=" << std::fixed << std::setprecision(2)
                  << tick.open << " H=" << tick.high
                  << " L=" << tick.low << " C=" << tick.close << std::endl;
    }

    // Compute Greeks (no server connection needed)
    auto g = tdx::all_greeks(450.0, 455.0, 0.05, 0.015, 30.0/365.0, 8.50, true);
    std::cout << "IV=" << g.iv << " Delta=" << g.delta
              << " Gamma=" << g.gamma << std::endl;
}
```

## Memory Management

The C++ SDK uses RAII wrappers around the C FFI handles. All objects automatically free their resources when they go out of scope. No manual memory management required.

```cpp
{
    auto client = tdx::Client::connect(creds, tdx::Config::production());
    // ... use client ...
}  // client automatically freed here
```

All methods throw `std::runtime_error` on failure.

## What's Next

- [Historical Data](historical.md) -- all endpoints with C++ examples
- [Real-Time Streaming](streaming.md) -- FPSS subscribe and next_event
- [API Reference](api-reference.md) -- complete type and method listing
