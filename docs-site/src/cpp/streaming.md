# Real-Time Streaming (C++)

Real-time market data via ThetaData's FPSS servers. The C++ SDK uses RAII wrappers with a polling model via `next_event()`.

## Connect

```cpp
auto creds = tdx::Credentials::from_file("creds.txt");
auto fpss = tdx::FpssClient::connect(creds, 1024);
```

## Subscribe

```cpp
// Stock quotes
int32_t req_id = fpss.subscribe_quotes("AAPL", tdx::SecType::Stock);
std::cout << "Subscribed (req_id=" << req_id << ")" << std::endl;

// Stock trades
fpss.subscribe_trades("MSFT", tdx::SecType::Stock);

// Open interest
fpss.subscribe_open_interest("AAPL", tdx::SecType::Stock);
```

### Security Type Enum

```cpp
tdx::SecType::Stock   // 0
tdx::SecType::Option  // 1
tdx::SecType::Index   // 2
tdx::SecType::Rate    // 3
```

## Receive Events

`next_event()` returns `nullptr` on timeout.

```cpp
while (auto event = fpss.next_event(5000)) {
    if (event->type() == tdx::FpssEventType::Quote) {
        std::cout << "Quote: " << event->contract()
                  << " bid=" << event->bid()
                  << " ask=" << event->ask() << std::endl;
    } else if (event->type() == tdx::FpssEventType::Trade) {
        std::cout << "Trade: " << event->contract()
                  << " price=" << event->price()
                  << " size=" << event->size() << std::endl;
    }
}
```

## Shutdown

```cpp
fpss.shutdown();
```

RAII also handles cleanup: the `FpssClient` destructor calls `shutdown()` automatically.

## FpssClient Methods

| Method | Signature | Description |
|--------|-----------|-------------|
| `connect` | `(creds, buf_size) -> FpssClient` | Static factory, connect + auth |
| `subscribe_quotes` | `(root, sec_type) -> int32_t` | Subscribe to quotes |
| `subscribe_trades` | `(root, sec_type) -> int32_t` | Subscribe to trades |
| `subscribe_open_interest` | `(root, sec_type) -> int32_t` | Subscribe to OI |
| `next_event` | `(timeout_ms) -> unique_ptr<FpssEvent>` | Poll next event (`nullptr` on timeout) |
| `shutdown` | `() -> void` | Graceful shutdown |

## Complete Example

```cpp
#include "thetadatadx.hpp"
#include <iostream>

int main() {
    auto creds = tdx::Credentials::from_file("creds.txt");
    auto fpss = tdx::FpssClient::connect(creds, 1024);

    // Subscribe to quotes and trades
    fpss.subscribe_quotes("AAPL", tdx::SecType::Stock);
    fpss.subscribe_trades("AAPL", tdx::SecType::Stock);
    fpss.subscribe_trades("MSFT", tdx::SecType::Stock);

    // Process events
    while (auto event = fpss.next_event(5000)) {
        std::cout << "Event type: " << event->type() << std::endl;

        if (event->type() == tdx::FpssEventType::Quote) {
            std::cout << "Quote: " << event->contract()
                      << " bid=" << event->bid()
                      << " ask=" << event->ask() << std::endl;
        } else if (event->type() == tdx::FpssEventType::Trade) {
            std::cout << "Trade: " << event->contract()
                      << " price=" << event->price()
                      << " size=" << event->size() << std::endl;
        } else if (event->type() == tdx::FpssEventType::Disconnected) {
            std::cerr << "Disconnected" << std::endl;
            break;
        }
    }

    fpss.shutdown();
}
```
