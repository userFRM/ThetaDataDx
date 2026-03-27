# Real-Time Streaming (Go)

Real-time market data via ThetaData's FPSS servers. The Go SDK uses a polling model with `NextEvent()`.

## Connect

```go
creds, _ := thetadatadx.CredentialsFromFile("creds.txt")
defer creds.Close()

config := thetadatadx.ProductionConfig()
defer config.Close()

fpss, _ := thetadatadx.NewFpssClient(creds, config)
defer fpss.Close()
```

## Subscribe

```go
// Stock quotes
reqID, _ := fpss.SubscribeQuotes("AAPL")
fmt.Printf("Subscribed (req_id=%d)\n", reqID)

// Stock trades
fpss.SubscribeTrades("MSFT")

// Open interest
fpss.SubscribeOpenInterest("AAPL")

// All trades for a security type
fpss.SubscribeFullTrades("STOCK")
```

## Receive Events

`NextEvent()` returns `nil` on timeout.

```go
for {
    event, err := fpss.NextEvent(5000) // 5s timeout
    if err != nil {
        log.Println("Error:", err)
        break
    }
    if event == nil {
        continue // timeout
    }
    fmt.Printf("Event: %s\n", string(event))
}
```

## Stop Streaming

```go
fpss.Shutdown()
```

## Streaming Methods (on FpssClient)

| Method | Signature | Description |
|--------|-----------|-------------|
| `SubscribeQuotes` | `(symbol string) (int, error)` | Subscribe to quotes |
| `SubscribeTrades` | `(symbol string) (int, error)` | Subscribe to trades |
| `SubscribeOpenInterest` | `(symbol string) (int, error)` | Subscribe to OI |
| `SubscribeFullTrades` | `(secType string) (int, error)` | Subscribe to all trades for a security type |
| `UnsubscribeQuotes` | `(symbol string) (int, error)` | Unsubscribe from quotes |
| `UnsubscribeTrades` | `(symbol string) (int, error)` | Unsubscribe from trades |
| `UnsubscribeOpenInterest` | `(symbol string) (int, error)` | Unsubscribe from OI |
| `NextEvent` | `(timeoutMs uint64) (json.RawMessage, error)` | Poll next event |
| `IsAuthenticated` | `() bool` | Check FPSS auth status |
| `ContractLookup` | `(id int) (string, error)` | Look up contract by server-assigned ID |
| `ActiveSubscriptions` | `() (json.RawMessage, error)` | Get active subscriptions |
| `Shutdown` | `()` | Graceful shutdown |

## Complete Example

```go
package main

import (
    "fmt"
    "log"

    thetadatadx "github.com/userFRM/ThetaDataDx/sdks/go"
)

func main() {
    creds, _ := thetadatadx.CredentialsFromFile("creds.txt")
    defer creds.Close()

    config := thetadatadx.ProductionConfig()
    defer config.Close()

    // Historical client
    client, err := thetadatadx.Connect(creds, config)
    if err != nil {
        log.Fatal(err)
    }
    defer client.Close()

    // Streaming client (separate connection, same credentials)
    fpss, err := thetadatadx.NewFpssClient(creds, config)
    if err != nil {
        log.Fatal(err)
    }
    defer fpss.Close()

    // Subscribe to real-time data
    fpss.SubscribeQuotes("AAPL")
    fpss.SubscribeTrades("AAPL")

    // Process events
    for {
        event, err := fpss.NextEvent(5000)
        if err != nil {
            log.Println("Error:", err)
            break
        }
        if event == nil {
            continue
        }
        fmt.Printf("Event: %s\n", string(event))
    }

    fpss.Shutdown()
}
```
