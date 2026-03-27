# API Reference (Go)

Complete type and method listing for the `thetadatadx` Go package. Every call runs through compiled Rust via CGo FFI.

## Credentials

```go
// From file
creds, err := thetadatadx.CredentialsFromFile("creds.txt")
defer creds.Close()

// Direct construction
creds := thetadatadx.NewCredentials("email@example.com", "password")
defer creds.Close()
```

## Config

```go
config := thetadatadx.ProductionConfig()
defer config.Close()

config := thetadatadx.DevConfig()
defer config.Close()
```

## Client

All data methods return typed Go structs deserialized from JSON over FFI.

```go
client, err := thetadatadx.Connect(creds, config)
defer client.Close()
```

### Stock Methods (14)

| Method | Returns | Description |
|--------|---------|-------------|
| `StockListSymbols()` | `([]string, error)` | All stock symbols |
| `StockListDates(requestType, symbol)` | `([]string, error)` | Available dates |
| `StockSnapshotOHLC(symbols)` | `([]OhlcTick, error)` | Latest OHLC |
| `StockSnapshotTrade(symbols)` | `([]TradeTick, error)` | Latest trade |
| `StockSnapshotQuote(symbols)` | `([]QuoteTick, error)` | Latest quote |
| `StockSnapshotMarketValue(symbols)` | `(interface{}, error)` | Latest market value |
| `StockHistoryEOD(symbol, start, end)` | `([]EodTick, error)` | EOD data |
| `StockHistoryOHLC(symbol, date, interval)` | `([]OhlcTick, error)` | Intraday OHLC |
| `StockHistoryOHLCRange(symbol, start, end, interval)` | `([]OhlcTick, error)` | OHLC date range |
| `StockHistoryTrade(symbol, date)` | `([]TradeTick, error)` | All trades |
| `StockHistoryQuote(symbol, date, interval)` | `([]QuoteTick, error)` | NBBO quotes |
| `StockHistoryTradeQuote(symbol, date)` | `(interface{}, error)` | Trade+quote |
| `StockAtTimeTrade(symbol, start, end, time)` | `([]TradeTick, error)` | Trade at time |
| `StockAtTimeQuote(symbol, start, end, time)` | `([]QuoteTick, error)` | Quote at time |

### Option Methods (34)

| Method | Returns | Description |
|--------|---------|-------------|
| `OptionListSymbols()` | `([]string, error)` | Option underlyings |
| `OptionListDates(reqType, sym, exp, strike, right)` | `([]string, error)` | Available dates |
| `OptionListExpirations(symbol)` | `([]string, error)` | Expirations |
| `OptionListStrikes(symbol, exp)` | `([]string, error)` | Strikes |
| `OptionListContracts(reqType, sym, date)` | `(interface{}, error)` | All contracts |
| `OptionSnapshotOHLC(sym, exp, strike, right)` | `([]OhlcTick, error)` | Latest OHLC |
| `OptionSnapshotTrade(sym, exp, strike, right)` | `([]TradeTick, error)` | Latest trade |
| `OptionSnapshotQuote(sym, exp, strike, right)` | `([]QuoteTick, error)` | Latest quote |
| `OptionSnapshotOpenInterest(sym, exp, strike, right)` | `(interface{}, error)` | Latest OI |
| `OptionSnapshotMarketValue(sym, exp, strike, right)` | `(interface{}, error)` | Latest MV |
| `OptionSnapshotGreeksIV(sym, exp, strike, right)` | `(interface{}, error)` | IV snapshot |
| `OptionSnapshotGreeksAll(sym, exp, strike, right)` | `(interface{}, error)` | All Greeks |
| `OptionSnapshotGreeksFirstOrder(...)` | `(interface{}, error)` | First-order |
| `OptionSnapshotGreeksSecondOrder(...)` | `(interface{}, error)` | Second-order |
| `OptionSnapshotGreeksThirdOrder(...)` | `(interface{}, error)` | Third-order |
| `OptionHistoryEOD(sym, exp, strike, right, start, end)` | `([]EodTick, error)` | EOD data |
| `OptionHistoryOHLC(sym, exp, strike, right, date, interval)` | `([]OhlcTick, error)` | OHLC bars |
| `OptionHistoryTrade(sym, exp, strike, right, date)` | `([]TradeTick, error)` | Trades |
| `OptionHistoryQuote(sym, exp, strike, right, date, interval)` | `([]QuoteTick, error)` | Quotes |
| `OptionHistoryTradeQuote(sym, exp, strike, right, date)` | `(interface{}, error)` | Trade+quote |
| `OptionHistoryOpenInterest(sym, exp, strike, right, date)` | `(interface{}, error)` | OI history |
| `OptionHistoryGreeksEOD(...)` | `(interface{}, error)` | EOD Greeks |
| `OptionHistoryGreeksAll(...)` | `(interface{}, error)` | All Greeks history |
| `OptionHistoryTradeGreeksAll(...)` | `(interface{}, error)` | Greeks on each trade |
| Plus 10 more Greeks variants | | First/second/third/IV history and trade variants |
| `OptionAtTimeTrade(sym, exp, strike, right, start, end, time)` | `([]TradeTick, error)` | Trade at time |
| `OptionAtTimeQuote(sym, exp, strike, right, start, end, time)` | `([]QuoteTick, error)` | Quote at time |

### Index Methods (9)

| Method | Returns |
|--------|---------|
| `IndexListSymbols()` | `([]string, error)` |
| `IndexListDates(symbol)` | `([]string, error)` |
| `IndexSnapshotOHLC(symbols)` | `([]OhlcTick, error)` |
| `IndexSnapshotPrice(symbols)` | `(interface{}, error)` |
| `IndexSnapshotMarketValue(symbols)` | `(interface{}, error)` |
| `IndexHistoryEOD(symbol, start, end)` | `([]EodTick, error)` |
| `IndexHistoryOHLC(symbol, start, end, interval)` | `([]OhlcTick, error)` |
| `IndexHistoryPrice(symbol, date, interval)` | `(interface{}, error)` |
| `IndexAtTimePrice(symbol, start, end, time)` | `(interface{}, error)` |

### Calendar Methods (3)

| Method | Returns |
|--------|---------|
| `CalendarOpenToday()` | `(interface{}, error)` |
| `CalendarOnDate(date)` | `(interface{}, error)` |
| `CalendarYear(year)` | `(interface{}, error)` |

### Rate Methods (1)

| Method | Returns |
|--------|---------|
| `InterestRateHistoryEOD(symbol, start, end)` | `(interface{}, error)` |

## Greeks (Standalone Functions)

```go
// All 22 Greeks
g, err := thetadatadx.AllGreeks(spot, strike, rate, divYield, tte, price, isCall)
// g.IV, g.Delta, g.Gamma, g.Theta, g.Vega, g.Rho, etc.

// Just IV
iv, ivErr, err := thetadatadx.ImpliedVolatility(spot, strike, rate, divYield, tte, price, isCall)
```

## FpssClient (Streaming)

Streaming uses a separate `FpssClient`, created via `NewFpssClient(creds, config)`.

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
| `ContractLookup` | `(id int) (string, error)` | Look up contract by ID |
| `ActiveSubscriptions` | `() (json.RawMessage, error)` | Get active subscriptions |
| `Shutdown` | `()` | Graceful shutdown |
| `Close` | `()` | Free resources (calls Shutdown) |

## Tick Types

### EodTick

```go
type EodTick struct {
    Date   int32   `json:"date"`
    Open   float64 `json:"open"`
    High   float64 `json:"high"`
    Low    float64 `json:"low"`
    Close  float64 `json:"close"`
    Volume int32   `json:"volume"`
}
```

### OhlcTick

```go
type OhlcTick struct {
    MsOfDay int32   `json:"ms_of_day"`
    Open    float64 `json:"open"`
    High    float64 `json:"high"`
    Low     float64 `json:"low"`
    Close   float64 `json:"close"`
    Volume  int32   `json:"volume"`
    Count   int32   `json:"count"`
    Date    int32   `json:"date"`
}
```

### TradeTick

```go
type TradeTick struct {
    MsOfDay  int32   `json:"ms_of_day"`
    Price    float64 `json:"price"`
    Size     int32   `json:"size"`
    Exchange int32   `json:"exchange"`
    Date     int32   `json:"date"`
}
```

### QuoteTick

```go
type QuoteTick struct {
    MsOfDay int32   `json:"ms_of_day"`
    Bid     float64 `json:"bid"`
    Ask     float64 `json:"ask"`
    BidSize int32   `json:"bid_size"`
    AskSize int32   `json:"ask_size"`
    Date    int32   `json:"date"`
}
```

### GreeksResult

```go
type GreeksResult struct {
    IV        float64 `json:"iv"`
    IVError   float64 `json:"iv_error"`
    Value     float64 `json:"value"`
    Delta     float64 `json:"delta"`
    Gamma     float64 `json:"gamma"`
    Theta     float64 `json:"theta"`
    Vega      float64 `json:"vega"`
    Rho       float64 `json:"rho"`
    // ... 14 more fields
}
```

## Security Type Constants

```go
const (
    SecTypeStock  = 0
    SecTypeOption = 1
    SecTypeIndex  = 2
    SecTypeRate   = 3
)
```
