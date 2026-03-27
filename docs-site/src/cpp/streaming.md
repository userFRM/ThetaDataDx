# Real-Time Streaming (C++)

Real-time market data via ThetaData's FPSS servers. The C++ SDK uses RAII wrappers with a polling model via `next_event()`.

## Connect

```cpp
auto creds = tdx::Credentials::from_file("creds.txt");
auto config = tdx::Config::production();
tdx::FpssClient fpss(creds, config);
```

## Subscribe

```cpp
// Stock quotes
int32_t req_id = fpss.subscribe_quotes("AAPL");
std::cout << "Subscribed (req_id=" << req_id << ")" << std::endl;

// Stock trades
fpss.subscribe_trades("MSFT");

// Open interest
fpss.subscribe_open_interest("AAPL");

// All trades for a security type
fpss.subscribe_full_trades("STOCK");
```

## Receive Events

`next_event()` returns JSON as a string, or empty string on timeout.

```cpp
while (true) {
    auto event = fpss.next_event(5000); // 5s timeout
    if (event.empty()) {
        continue; // timeout
    }
    std::cout << "Event: " << event << std::endl;
}
```

## Stop Streaming

```cpp
fpss.shutdown();
```

RAII also handles cleanup: the `FpssClient` destructor calls `shutdown()` automatically.

## Streaming Methods (on FpssClient)

| Method | Signature | Description |
|--------|-----------|-------------|
| `subscribe_quotes` | `(symbol) -> int32_t` | Subscribe to quotes |
| `subscribe_trades` | `(symbol) -> int32_t` | Subscribe to trades |
| `subscribe_open_interest` | `(symbol) -> int32_t` | Subscribe to OI |
| `subscribe_full_trades` | `(sec_type) -> int32_t` | Subscribe to all trades for a security type |
| `unsubscribe_trades` | `(symbol) -> int32_t` | Unsubscribe from trades |
| `unsubscribe_open_interest` | `(symbol) -> int32_t` | Unsubscribe from OI |
| `next_event` | `(timeout_ms) -> std::string` | Poll next event (empty on timeout) |
| `is_authenticated` | `() -> bool` | Check FPSS auth status |
| `contract_lookup` | `(id) -> std::optional<std::string>` | Look up contract by server-assigned ID |
| `active_subscriptions` | `() -> std::string` | Get active subscriptions as JSON |
| `shutdown` | `() -> void` | Graceful shutdown |

## Complete Example

```cpp
#include "thetadatadx.hpp"
#include <iostream>

int main() {
    auto creds = tdx::Credentials::from_file("creds.txt");
    auto config = tdx::Config::production();

    // Historical client
    auto client = tdx::Client::connect(creds, config);

    // Streaming client (separate connection, same credentials)
    tdx::FpssClient fpss(creds, config);

    // Subscribe to quotes and trades
    fpss.subscribe_quotes("AAPL");
    fpss.subscribe_trades("AAPL");
    fpss.subscribe_trades("MSFT");

    // Process events
    while (true) {
        auto event = fpss.next_event(5000);
        if (event.empty()) {
            continue;
        }
        std::cout << "Event: " << event << std::endl;
    }

    fpss.shutdown();
}
```
