---
title: C++ Quickstart
description: Install, authenticate, run a historical call, subscribe to streaming, and handle errors with ThetaDataDx in C++.
---

# C++ Quickstart

C++17 header + RAII wrappers around the same Rust FFI core the Go SDK uses. Scope-based cleanup, no manual memory management.

## Install

```bash
# Prerequisites: C++17 compiler, CMake 3.16+, Rust toolchain

# Build the FFI library once:
git clone https://github.com/userFRM/ThetaDataDx.git
cd ThetaDataDx
cargo build --release -p thetadatadx-ffi

# Build the C++ SDK:
cd sdks/cpp
mkdir build && cd build
cmake ..
make
```

Include path: `sdks/cpp/include`. Link against `target/release/libthetadatadx_ffi.{so|dylib}`.

## Authenticate and connect

```cpp
#include "thetadx.hpp"

// From file
auto creds = tdx::Credentials::from_file("creds.txt");

// Or from env vars
auto creds = tdx::Credentials(
    std::getenv("THETA_EMAIL"),
    std::getenv("THETA_PASS")
);

auto client = tdx::Client::connect(creds, tdx::Config::production());
```

RAII: each handle frees its FFI-side resources on scope exit. No `Close()` needed.

## Historical call

```cpp
#include "thetadx.hpp"
#include <iomanip>
#include <iostream>

int main() {
    auto creds = tdx::Credentials::from_file("creds.txt");
    auto client = tdx::Client::connect(creds, tdx::Config::production());

    auto eod = client.stock_history_eod("AAPL", "20240101", "20240301");
    for (const auto& tick : eod) {
        std::cout << tick.date
                  << std::fixed << std::setprecision(2)
                  << ": O=" << tick.open << " H=" << tick.high
                  << " L=" << tick.low  << " C=" << tick.close
                  << " V=" << tick.volume << std::endl;
    }
}
```

Ticks are typed `#[repr(C)]` structs with price fields decoded to `double` at parse time — no `Price{value, type}` wrappers.

## Streaming call

Like Go, the C++ streaming client (`tdx::FpssClient`) is a separate type from the historical `tdx::Client`.

```cpp
#include "thetadx.hpp"
#include <iostream>

int main() {
    auto creds = tdx::Credentials::from_file("creds.txt");
    auto config = tdx::Config::production();
    tdx::FpssClient fpss(creds, config);

    fpss.subscribe_quotes("AAPL");
    fpss.subscribe_trades("MSFT");

    while (true) {
        auto event = fpss.next_event(1000);
        if (!event) continue;

        switch (event->kind) {
        case TDX_FPSS_QUOTE: {
            const auto& q = event->quote;
            std::cout << "Quote: " << q.contract_id
                      << " " << q.bid << "/" << q.ask << std::endl;
            break;
        }
        case TDX_FPSS_TRADE: {
            const auto& t = event->trade;
            std::cout << "Trade: " << t.contract_id
                      << " " << t.price << " x " << t.size << std::endl;
            break;
        }
        default: break;
        }
    }

    fpss.shutdown();
}
```

`next_event` returns a `std::unique_ptr<TdxFpssEvent>` so the event lifetime is scope-bound.

## Error handling

Every method throws a subclass of `tdx::ThetaDataError` on failure:

```cpp
try {
    auto ticks = client.option_history_greeks_all(
        "SPY", "20240419", "500", "C", "20240101", "20240301"
    );
} catch (const tdx::RateLimitError& e) {
    std::this_thread::sleep_for(std::chrono::milliseconds(e.wait_ms()));
    // retry
} catch (const tdx::SubscriptionError& e) {
    std::cerr << e.endpoint() << " requires " << e.required_tier() << std::endl;
} catch (const tdx::AuthError&) {
    refresh_credentials();
} catch (const tdx::ThetaDataError& e) {
    std::cerr << "SDK error: " << e.what() << std::endl;
    throw;
}
```

## Next

- [Historical data](../historical/) — 61 endpoints
- [Streaming (FPSS)](../streaming/) — polling model, event types, reconnect
- [Options & Greeks](../options) — wildcard chain queries, local Greeks calculator
- [Error handling](../getting-started/errors) — full `ThetaDataError` hierarchy
