# thetadatadx (Go)

Go SDK for ThetaData market data, powered by the `thetadatadx` Rust crate via CGo FFI.

**This is NOT a Go reimplementation.** Every call goes through compiled Rust via a C FFI layer. gRPC communication, protobuf parsing, zstd decompression, and TCP streaming all happen at native Rust speed. Go is just the interface.

## Prerequisites

- Go 1.21+
- Rust toolchain (for building the FFI library)
- C compiler (for CGo)

## Platform Support

- Linux: CI-validated
- macOS: CI-validated
- Windows: CI-validated via a GNU-targeted Rust FFI build (`x86_64-pc-windows-gnu`)

## Building

First, build the Rust FFI library:

```bash
# From the repository root
cargo build --release -p thetadatadx-ffi
```

This produces `target/release/libthetadatadx_ffi.so` (Linux) or `target/release/libthetadatadx_ffi.dylib` (macOS).

On Windows, build the GNU-targeted FFI for the Go SDK:

```powershell
rustup target add x86_64-pc-windows-gnu
cargo build --release --target x86_64-pc-windows-gnu -p thetadatadx-ffi
```

That produces the Windows Go-linkable artifacts under `target/x86_64-pc-windows-gnu/release/`.

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
        // Prices are pre-decoded to float64 -- no manual conversion needed
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
- `DevConfig()` - Dev FPSS servers (port 20200, infinite historical replay)
- `StageConfig()` - Stage FPSS servers (port 20100, testing, unstable)

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
| `OptionHistoryGreeksEODWithOptions(sym, exp, strike, right, start, end, opts)` | `([]GreeksTick, error)` | EOD Greeks with `EndpointRequestOptions`, including builder parameters such as `StrikeRange` |
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
- `AllGreeks(spot, strike, rate, divYield, tte, price, right)` - returns `(*Greeks, error)` with 22 fields. `right` accepts `"C"`/`"P"` or `"call"`/`"put"` case-insensitively.
- `ImpliedVolatility(spot, strike, rate, divYield, tte, price, right)` - returns `(iv, errorBound, err)`. Same `right` vocabulary.

### Types

#### Core Tick Types

| Type | Fields | Description |
|------|--------|-------------|
| `EodTick` | MsOfDay, Open, High, Low, Close, Volume, Count, Bid, Ask, Date, **Expiration, Strike (float64), Right (string)** | End-of-day bar |
| `OhlcTick` | MsOfDay, Open, High, Low, Close, Volume, Count, Date, **Expiration, Strike (float64), Right (string)** | OHLC bar |
| `TradeTick` | MsOfDay, Sequence, Condition, Size, Exchange, Price (float64), ConditionFlags, PriceFlags, VolumeType, RecordsBack, Date, **Expiration, Strike (float64), Right (string)** | Individual trade |
| `QuoteTick` | MsOfDay, BidSize, BidExchange, Bid, BidCondition, AskSize, AskExchange, Ask, AskCondition, Midpoint, Date, **Expiration, Strike (float64), Right (string)** | NBBO quote |
| `TradeQuoteTick` | All TradeTick fields + QuoteMsOfDay, BidSize, BidExchange, Bid, BidCondition, AskSize, AskExchange, Ask, AskCondition, Date, **Expiration, Strike (float64), Right (string)** | Combined trade+quote |

#### Derived Types

| Type | Fields | Description |
|------|--------|-------------|
| `OpenInterestTick` | MsOfDay, OpenInterest, Date, **Expiration, Strike (float64), Right (string)** | Open interest data point |
| `MarketValueTick` | MsOfDay, MarketBid, MarketAsk, MarketPrice, Date, **Expiration, Strike (float64), Right (string)** | Market value data |
| `GreeksTick` | MsOfDay, ImpliedVolatility, Delta, Gamma, Theta, Vega, Rho, IVError, Vanna, Charm, Vomma, Veta, Speed, Zomma, Color, Ultima, D1, D2, DualDelta, DualGamma, Epsilon, Lambda, Vera, Date, **Expiration, Strike (float64), Right (string)** | Greeks time series |
| `IVTick` | MsOfDay, ImpliedVolatility, IVError, Date, **Expiration, Strike (float64), Right (string)** | Implied volatility data point |
| `PriceTick` | MsOfDay, Price (float64), Date | Price data point (indices) |
| `CalendarDay` | Date, IsOpen, OpenTime, CloseTime, Status | Market calendar day |
| `InterestRate` | Date, Rate | Interest rate data point |
| `Contract` | Symbol, Expiration, Strike (float64), Right (string) | Option contract identifier |

**Contract identification fields** (bold above): `Expiration`, `Strike` (float64), `Right` (string) are populated by the server on wildcard queries (pass `"0"` for expiration/strike). On single-contract queries these fields are zero/empty.

**Right field**: In the Go SDK, `Right` is a human-readable `string` (`"C"` for call, `"P"` for put, `""` if not set). On **input**, `right` parameters accept `"call"`, `"put"`, `"C"`, `"P"` (case-insensitive); the server output stays `"C"`/`"P"`.

## FPSS Streaming

Real-time market data via ThetaData's FPSS servers. Streaming uses a separate `FpssClient` struct (not the historical `Client`). Events are returned as typed Go structs -- no JSON parsing on the hot path.

```go
package main

import (
    "fmt"
    "log"

    thetadatadx "github.com/userFRM/thetadatadx/sdks/go"
)

func main() {
    creds, err := thetadatadx.CredentialsFromFile("creds.txt")
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

    // Poll for events (returns typed *FpssEvent)
    for {
        event, err := fpss.NextEvent(5000) // 5s timeout
        if err != nil {
            log.Println("Error:", err)
            break
        }
        if event == nil {
            continue // timeout, no event
        }

        switch event.Kind {
        case thetadatadx.FpssQuoteEvent:
            q := event.Quote
            fmt.Printf("Quote: bid=%.4f ask=%.4f date=%d\n", q.Bid, q.Ask, q.Date)
        case thetadatadx.FpssTradeEvent:
            t := event.Trade
            fmt.Printf("Trade: price=%.4f size=%d\n", t.Price, t.Size)
        case thetadatadx.FpssControlEvent:
            c := event.Control
            fmt.Printf("Control: kind=%d detail=%s\n", c.Kind, c.Detail)
        }
    }

    fpss.Shutdown()
}
```

All prices in streaming events are `float64` -- decoded during parsing. No `PriceType` in the public API.

### FpssClient API

| Method | Signature | Description |
|--------|-----------|-------------|
| `NewFpssClient(creds, config)` | `(*FpssClient, error)` | Connect to FPSS streaming servers |
| `SubscribeQuotes(symbol)` | `(int, error)` | Subscribe to quotes |
| `SubscribeTrades(symbol)` | `(int, error)` | Subscribe to trades |
| `SubscribeOpenInterest(symbol)` | `(int, error)` | Subscribe to open interest |
| `SubscribeOptionQuotes(symbol, expiration, strike, right)` | `(int, error)` | Subscribe to option quotes |
| `SubscribeOptionTrades(symbol, expiration, strike, right)` | `(int, error)` | Subscribe to option trades |
| `SubscribeOptionOpenInterest(symbol, expiration, strike, right)` | `(int, error)` | Subscribe to option open interest |
| `SubscribeFullTrades(secType)` | `(int, error)` | Subscribe to all trades for a security type |
| `SubscribeFullOpenInterest(secType)` | `(int, error)` | Subscribe to all OI for a security type |
| `UnsubscribeQuotes(symbol)` | `(int, error)` | Unsubscribe from quotes |
| `UnsubscribeTrades(symbol)` | `(int, error)` | Unsubscribe from trades |
| `UnsubscribeOpenInterest(symbol)` | `(int, error)` | Unsubscribe from open interest |
| `UnsubscribeOptionQuotes(symbol, expiration, strike, right)` | `(int, error)` | Unsubscribe from option quotes |
| `UnsubscribeOptionTrades(symbol, expiration, strike, right)` | `(int, error)` | Unsubscribe from option trades |
| `UnsubscribeOptionOpenInterest(symbol, expiration, strike, right)` | `(int, error)` | Unsubscribe from option open interest |
| `UnsubscribeFullTrades(secType)` | `(int, error)` | Unsubscribe from all trades for a security type |
| `UnsubscribeFullOpenInterest(secType)` | `(int, error)` | Unsubscribe from all OI for a security type |
| `IsAuthenticated()` | `bool` | Check if FPSS client is authenticated |
| `ContractLookup(id)` | `(string, error)` | Look up contract by server-assigned ID |
| `ContractMap()` | `(map[int32]string, error)` | Get the full contract ID mapping |
| `ActiveSubscriptions()` | `([]Subscription, error)` | List currently active subscriptions |
| `NextEvent(timeoutMs)` | `(*FpssEvent, error)` | Poll next event as typed struct (nil on timeout) |
| `Reconnect()` | `error` | Reconnect streaming and restore subscriptions |
| `Shutdown()` | | Graceful shutdown of streaming |
| `Close()` | | Free the FPSS handle (call after Shutdown) |

### FPSS Event Types

| Type | Fields | Used when |
|------|--------|-----------|
| `FpssQuote` | ContractID, MsOfDay, BidSize, BidExchange, Bid (float64), BidCondition, AskSize, AskExchange, Ask (float64), AskCondition, Date, ReceivedAtNs | `Kind == FpssQuoteEvent` |
| `FpssTrade` | ContractID, MsOfDay, Sequence, ExtCondition1-4, Condition, Size, Exchange, Price (float64), ConditionFlags, PriceFlags, VolumeType, RecordsBack, Date, ReceivedAtNs | `Kind == FpssTradeEvent` |
| `FpssOpenInterestData` | ContractID, MsOfDay, OpenInterest, Date, ReceivedAtNs | `Kind == FpssOpenInterestEvent` |
| `FpssOhlcvc` | ContractID, MsOfDay, Open/High/Low/Close (float64), Volume (int64), Count (int64), Date, ReceivedAtNs | `Kind == FpssOhlcvcEvent` |
| `FpssControlData` | Kind (`FpssCtrl*` constants: 0..=6, 8..=12; 7 reserved), ID, Detail (string) | `Kind == FpssControlEvent` |
| Raw data | RawCode (uint8), RawPayload ([]byte) | `Kind == FpssRawDataEvent` |

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
