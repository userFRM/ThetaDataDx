# thetadatadx (Go)

Go SDK for ThetaData market data, powered by the `thetadatadx` Rust crate via CGo FFI.

**This is NOT a Go reimplementation.** Every call goes through compiled Rust via a C FFI layer. gRPC communication, protobuf parsing, zstd decompression, and TCP streaming all happen at native Rust speed. Go is just the interface.

## Prerequisites

- Go 1.21+
- Rust toolchain (for building the FFI library)
- C compiler (for CGo)

## Building

First, build the Rust FFI library:

```bash
# From the repository root
cargo build --release -p thetadatadx-ffi
```

This produces `target/release/libthetadatadx_ffi.so` (Linux) or `libthetadatadx_ffi.dylib` (macOS).

Then build or run your Go code:

```bash
cd sdks/go/examples
go run main.go
```

## Quick Start

```go
package main

import (
    "fmt"
    "log"

    thetadatadx "github.com/userFRM/thetadatadx/sdks/go"
)

func main() {
    creds, err := thetadatadx.CredentialsFromFile("creds.txt")
    // Or inline: creds, err := thetadatadx.NewCredentials("user@example.com", "your-password")
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

    eod, err := client.StockHistoryEOD("AAPL", "20240101", "20240301")
    if err != nil {
        log.Fatal(err)
    }
    for _, tick := range eod {
        fmt.Printf("%d: O=%.2f H=%.2f L=%.2f C=%.2f\n",
            tick.Date, tick.Open, tick.High, tick.Low, tick.Close)
    }
}
```

## API

### Credentials
- `NewCredentials(email, password)` - direct construction
- `CredentialsFromFile(path)` - load from creds.txt

### Config
- `ProductionConfig()` - ThetaData NJ production servers
- `DevConfig()` - dev servers with shorter timeouts

### Client (Historical Data)

All data methods return typed Go structs (received as native `#[repr(C)]` struct arrays over FFI).

```go
client, err := thetadatadx.Connect(creds, config)
defer client.Close()
```

#### Stock - List (2)

| Method | Returns | Description |
|--------|---------|-------------|
| `StockListSymbols()` | `([]string, error)` | All stock symbols |
| `StockListDates(requestType, symbol)` | `([]string, error)` | Available dates for a request type |

#### Stock - Snapshot (4)

| Method | Returns | Description |
|--------|---------|-------------|
| `StockSnapshotOHLC(symbols)` | `([]OhlcTick, error)` | Latest OHLC |
| `StockSnapshotTrade(symbols)` | `([]TradeTick, error)` | Latest trade |
| `StockSnapshotQuote(symbols)` | `([]QuoteTick, error)` | Latest quote |
| `StockSnapshotMarketValue(symbols)` | `([]MarketValueTick, error)` | Latest market value |

#### Stock - History (6)

| Method | Returns | Description |
|--------|---------|-------------|
| `StockHistoryEOD(symbol, start, end)` | `([]EodTick, error)` | EOD data |
| `StockHistoryOHLC(symbol, date, interval)` | `([]OhlcTick, error)` | Intraday OHLC. `interval` accepts ms (`"60000"`) or shorthand (`"1m"`). |
| `StockHistoryOHLCRange(symbol, start, end, interval)` | `([]OhlcTick, error)` | OHLC over date range. `interval` accepts ms or shorthand. |
| `StockHistoryTrade(symbol, date)` | `([]TradeTick, error)` | All trades |
| `StockHistoryQuote(symbol, date, interval)` | `([]QuoteTick, error)` | NBBO quotes. `interval` accepts ms or shorthand. |
| `StockHistoryTradeQuote(symbol, date)` | `([]TradeQuoteTick, error)` | Trade+quote combined |

#### Stock - At-Time (2)

| Method | Returns | Description |
|--------|---------|-------------|
| `StockAtTimeTrade(symbol, start, end, time)` | `([]TradeTick, error)` | Trade at specific time across dates |
| `StockAtTimeQuote(symbol, start, end, time)` | `([]QuoteTick, error)` | Quote at specific time across dates |

#### Option - List (5)

| Method | Returns | Description |
|--------|---------|-------------|
| `OptionListSymbols()` | `([]string, error)` | Option underlyings |
| `OptionListDates(reqType, sym, exp, strike, right)` | `([]string, error)` | Available dates |
| `OptionListExpirations(symbol)` | `([]string, error)` | Expiration dates |
| `OptionListStrikes(symbol, exp)` | `([]string, error)` | Strike prices |
| `OptionListContracts(reqType, symbol, date)` | `([]Contract, error)` | All contracts |

#### Option - Snapshot (10)

| Method | Returns | Description |
|--------|---------|-------------|
| `OptionSnapshotOHLC(sym, exp, strike, right)` | `([]OhlcTick, error)` | Latest OHLC |
| `OptionSnapshotTrade(sym, exp, strike, right)` | `([]TradeTick, error)` | Latest trade |
| `OptionSnapshotQuote(sym, exp, strike, right)` | `([]QuoteTick, error)` | Latest quote |
| `OptionSnapshotOpenInterest(sym, exp, strike, right)` | `([]OpenInterestTick, error)` | Latest OI |
| `OptionSnapshotMarketValue(sym, exp, strike, right)` | `([]MarketValueTick, error)` | Latest market value |
| `OptionSnapshotGreeksImpliedVolatility(sym, exp, strike, right)` | `([]IVTick, error)` | IV snapshot |
| `OptionSnapshotGreeksAll(sym, exp, strike, right)` | `([]GreeksTick, error)` | All Greeks snapshot |
| `OptionSnapshotGreeksFirstOrder(sym, exp, strike, right)` | `([]GreeksTick, error)` | First-order Greeks |
| `OptionSnapshotGreeksSecondOrder(sym, exp, strike, right)` | `([]GreeksTick, error)` | Second-order Greeks |
| `OptionSnapshotGreeksThirdOrder(sym, exp, strike, right)` | `([]GreeksTick, error)` | Third-order Greeks |

#### Option - History (6)

| Method | Returns | Description |
|--------|---------|-------------|
| `OptionHistoryEOD(sym, exp, strike, right, start, end)` | `([]EodTick, error)` | EOD data |
| `OptionHistoryOHLC(sym, exp, strike, right, date, interval)` | `([]OhlcTick, error)` | OHLC bars |
| `OptionHistoryTrade(sym, exp, strike, right, date)` | `([]TradeTick, error)` | Trades |
| `OptionHistoryQuote(sym, exp, strike, right, date, interval)` | `([]QuoteTick, error)` | Quotes |
| `OptionHistoryTradeQuote(sym, exp, strike, right, date)` | `([]TradeQuoteTick, error)` | Trade+quote combined |
| `OptionHistoryOpenInterest(sym, exp, strike, right, date)` | `([]OpenInterestTick, error)` | Open interest history |

#### Option - History Greeks (11)

| Method | Returns | Description |
|--------|---------|-------------|
| `OptionHistoryGreeksEOD(sym, exp, strike, right, start, end)` | `([]GreeksTick, error)` | EOD Greeks |
| `OptionHistoryGreeksAll(sym, exp, strike, right, date, interval)` | `([]GreeksTick, error)` | All Greeks history |
| `OptionHistoryTradeGreeksAll(sym, exp, strike, right, date)` | `([]GreeksTick, error)` | Greeks on each trade |
| `OptionHistoryGreeksFirstOrder(sym, exp, strike, right, date, interval)` | `([]GreeksTick, error)` | First-order Greeks history |
| `OptionHistoryTradeGreeksFirstOrder(sym, exp, strike, right, date)` | `([]GreeksTick, error)` | First-order on each trade |
| `OptionHistoryGreeksSecondOrder(sym, exp, strike, right, date, interval)` | `([]GreeksTick, error)` | Second-order Greeks history |
| `OptionHistoryTradeGreeksSecondOrder(sym, exp, strike, right, date)` | `([]GreeksTick, error)` | Second-order on each trade |
| `OptionHistoryGreeksThirdOrder(sym, exp, strike, right, date, interval)` | `([]GreeksTick, error)` | Third-order Greeks history |
| `OptionHistoryTradeGreeksThirdOrder(sym, exp, strike, right, date)` | `([]GreeksTick, error)` | Third-order on each trade |
| `OptionHistoryGreeksImpliedVolatility(sym, exp, strike, right, date, interval)` | `([]IVTick, error)` | IV history |
| `OptionHistoryTradeGreeksImpliedVolatility(sym, exp, strike, right, date)` | `([]IVTick, error)` | IV on each trade |

#### Option - At-Time (2)

| Method | Returns | Description |
|--------|---------|-------------|
| `OptionAtTimeTrade(sym, exp, strike, right, start, end, time)` | `([]TradeTick, error)` | Trade at specific time across dates |
| `OptionAtTimeQuote(sym, exp, strike, right, start, end, time)` | `([]QuoteTick, error)` | Quote at specific time across dates |

#### Index - List (2)

| Method | Returns | Description |
|--------|---------|-------------|
| `IndexListSymbols()` | `([]string, error)` | Index symbols |
| `IndexListDates(symbol)` | `([]string, error)` | Available dates |

#### Index - Snapshot (3)

| Method | Returns | Description |
|--------|---------|-------------|
| `IndexSnapshotOHLC(symbols)` | `([]OhlcTick, error)` | Latest OHLC |
| `IndexSnapshotPrice(symbols)` | `([]PriceTick, error)` | Latest price |
| `IndexSnapshotMarketValue(symbols)` | `([]MarketValueTick, error)` | Latest market value |

#### Index - History (3)

| Method | Returns | Description |
|--------|---------|-------------|
| `IndexHistoryEOD(symbol, start, end)` | `([]EodTick, error)` | EOD data |
| `IndexHistoryOHLC(symbol, start, end, interval)` | `([]OhlcTick, error)` | OHLC bars |
| `IndexHistoryPrice(symbol, date, interval)` | `([]PriceTick, error)` | Price history |

#### Index - At-Time (1)

| Method | Returns | Description |
|--------|---------|-------------|
| `IndexAtTimePrice(symbol, start, end, time)` | `([]PriceTick, error)` | Price at specific time across dates |

#### Calendar (3)

| Method | Returns | Description |
|--------|---------|-------------|
| `CalendarOpenToday()` | `([]CalendarDay, error)` | Is the market open today? |
| `CalendarOnDate(date)` | `([]CalendarDay, error)` | Market schedule for date |
| `CalendarYear(year)` | `([]CalendarDay, error)` | Full calendar for year |

#### Interest Rate (1)

| Method | Returns | Description |
|--------|---------|-------------|
| `InterestRateHistoryEOD(symbol, start, end)` | `([]InterestRate, error)` | Interest rate EOD history |

### Greeks (Standalone Functions)
- `AllGreeks(spot, strike, rate, divYield, tte, price, isCall)` - returns `(*Greeks, error)` with 22 fields
- `ImpliedVolatility(spot, strike, rate, divYield, tte, price, isCall)` - returns `(iv, errorBound, err)`

### Types

#### Core Tick Types

| Type | Fields | Description |
|------|--------|-------------|
| `EodTick` | MsOfDay, Open, High, Low, Close, Volume, Count, Bid, Ask, Date | End-of-day bar |
| `OhlcTick` | MsOfDay, Open, High, Low, Close, Volume, Count, Date | OHLC bar |
| `TradeTick` | MsOfDay, Sequence, Condition, Size, Exchange, Price, PriceRaw, PriceType, ConditionFlags, PriceFlags, VolumeType, RecordsBack, Date | Individual trade |
| `QuoteTick` | MsOfDay, BidSize, BidExchange, Bid, BidCondition, AskSize, AskExchange, Ask, AskCondition, Date | NBBO quote |
| `TradeQuoteTick` | All TradeTick fields + QuoteMsOfDay, BidSize, BidExchange, Bid, BidCondition, AskSize, AskExchange, Ask, AskCondition, Date | Combined trade+quote |

#### Derived Types

| Type | Fields | Description |
|------|--------|-------------|
| `OpenInterestTick` | MsOfDay, OpenInterest, Date | Open interest data point |
| `MarketValueTick` | MsOfDay, MarketCap, SharesOut, EntValue, BookValue, FreeFloat, Date | Market value data |
| `GreeksTick` | MsOfDay, Value, Delta, Gamma, Theta, Vega, Rho, IV, IVError, Vanna, Charm, Vomma, Veta, Speed, Zomma, Color, Ultima, D1, D2, DualDelta, DualGamma, Epsilon, Lambda, Date | Greeks time series |
| `IVTick` | MsOfDay, IV, IVError, Date | Implied volatility data point |
| `PriceTick` | MsOfDay, Price, Date | Price data point (indices) |
| `CalendarDay` | Date, IsOpen, OpenTime, CloseTime, Status | Market calendar day |
| `InterestRate` | Date, Rate | Interest rate data point |
| `Contract` | Symbol, Expiration, Strike, Right | Option contract identifier |

## FPSS Streaming

Real-time market data via ThetaData's FPSS servers. Streaming uses a separate `FpssClient` struct (not the historical `Client`).

```go
package main

import (
    "fmt"
    "log"

    thetadatadx "github.com/userFRM/thetadatadx/sdks/go"
)

func main() {
    creds, err := thetadatadx.CredentialsFromFile("creds.txt")
    // Or inline: creds, err := thetadatadx.NewCredentials("user@example.com", "your-password")
    if err != nil {
        log.Fatal(err)
    }
    defer creds.Close()

    config := thetadatadx.ProductionConfig()
    defer config.Close()

    fpss, err := thetadatadx.NewFpssClient(creds, config)
    if err != nil {
        log.Fatal(err)
    }
    defer fpss.Close()

    // Subscribe to real-time quotes
    reqID, err := fpss.SubscribeQuotes("AAPL")
    if err != nil {
        log.Fatal(err)
    }
    fmt.Printf("Subscribed (req_id=%d)\n", reqID)

    // Poll for events
    for {
        event, err := fpss.NextEvent(5000) // 5s timeout
        if err != nil {
            log.Println("Error:", err)
            break
        }
        if event == nil {
            continue // timeout, no event
        }
        fmt.Printf("Event: %s\n", event)
    }

    fpss.Shutdown()
}
```

### FpssClient API

| Method | Signature | Description |
|--------|-----------|-------------|
| `NewFpssClient(creds, config)` | `(*FpssClient, error)` | Connect to FPSS streaming servers |
| `SubscribeQuotes(symbol)` | `(int, error)` | Subscribe to quotes |
| `SubscribeTrades(symbol)` | `(int, error)` | Subscribe to trades |
| `SubscribeOpenInterest(symbol)` | `(int, error)` | Subscribe to open interest |
| `SubscribeFullTrades(secType)` | `(int, error)` | Subscribe to all trades for a security type |
| `UnsubscribeQuotes(symbol)` | `(int, error)` | Unsubscribe from quotes |
| `UnsubscribeTrades(symbol)` | `(int, error)` | Unsubscribe from trades |
| `UnsubscribeOpenInterest(symbol)` | `(int, error)` | Unsubscribe from open interest |
| `SubscribeFullOpenInterest(secType)` | `(int, error)` | Subscribe to all OI for a security type |
| `UnsubscribeFullTrades(secType)` | `(int, error)` | Unsubscribe from all trades for a security type |
| `UnsubscribeFullOpenInterest(secType)` | `(int, error)` | Unsubscribe from all OI for a security type |
| `IsAuthenticated()` | `bool` | Check if FPSS client is authenticated |
| `ContractLookup(id)` | `(string, error)` | Look up contract by server-assigned ID |
| `ActiveSubscriptions()` | `(json.RawMessage, error)` | List currently active subscriptions |
| `NextEvent(timeoutMs)` | `(json.RawMessage, error)` | Poll next event (nil on timeout) |
| `Shutdown()` | | Graceful shutdown of streaming |
| `Close()` | | Free the FPSS handle (call after Shutdown) |

Note: `NextEvent()` returns `json.RawMessage` because streaming events are polymorphic - different event types (trades, quotes, open interest, OHLC) arrive on the same channel. Inspect the `"kind"` field to determine the event type.

## Architecture

```
Go code
    |  (CGo FFI)
    v
libthetadatadx_ffi.so / .a
    |  (Rust FFI crate)
    v
thetadatadx Rust crate
    |  (tonic gRPC / tokio TCP)
    v
ThetaData servers
```
