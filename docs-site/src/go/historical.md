# Historical Data (Go)

All historical data is accessed through the `Client` type. Every call runs through compiled Rust via CGo FFI.

## Connecting

```go
creds, _ := thetadatadx.CredentialsFromFile("creds.txt")
defer creds.Close()

config := thetadatadx.ProductionConfig()
defer config.Close()

client, err := thetadatadx.Connect(creds, config)
if err != nil {
    log.Fatal(err)
}
defer client.Close()
```

## Date Format

All dates are `YYYYMMDD` strings: `"20240315"` for March 15, 2024.

## Interval Format

Intervals are millisecond strings: `"60000"` for 1 minute, `"300000"` for 5 minutes.

---

## Stock Endpoints (14)

### List

```go
// All stock symbols
symbols, _ := client.StockListSymbols()

// Available dates by request type
dates, _ := client.StockListDates("EOD", "AAPL")
```

### Snapshots

```go
// Latest quote snapshot (multiple symbols)
quotes, _ := client.StockSnapshotQuote([]string{"AAPL", "MSFT", "GOOGL"})
for _, q := range quotes {
    fmt.Printf("bid=%.2f ask=%.2f\n", q.Bid, q.Ask)
}

ohlc, _ := client.StockSnapshotOHLC([]string{"AAPL", "MSFT"})
trades, _ := client.StockSnapshotTrade([]string{"AAPL"})
mv, _ := client.StockSnapshotMarketValue([]string{"AAPL"})
```

### History

```go
// End-of-day data
eod, _ := client.StockHistoryEOD("AAPL", "20240101", "20240301")
for _, tick := range eod {
    fmt.Printf("%d: O=%.2f H=%.2f L=%.2f C=%.2f V=%d\n",
        tick.Date, tick.Open, tick.High, tick.Low, tick.Close, tick.Volume)
}

// Intraday OHLC bars
bars, _ := client.StockHistoryOHLC("AAPL", "20240315", "60000")

// OHLC bars across date range
bars, _ = client.StockHistoryOHLCRange("AAPL", "20240101", "20240301", "300000")

// All trades
trades, _ := client.StockHistoryTrade("AAPL", "20240315")

// NBBO quotes
quotes, _ := client.StockHistoryQuote("AAPL", "20240315", "60000")

// Combined trade + quote
result, _ := client.StockHistoryTradeQuote("AAPL", "20240315")
```

### At-Time

```go
// Trade at 9:30 AM across a date range
trades, _ := client.StockAtTimeTrade("AAPL", "20240101", "20240301", "34200000")

// Quote at 9:30 AM
quotes, _ := client.StockAtTimeQuote("AAPL", "20240101", "20240301", "34200000")
```

---

## Option Endpoints (34)

### List

```go
symbols, _ := client.OptionListSymbols()
exps, _ := client.OptionListExpirations("SPY")
strikes, _ := client.OptionListStrikes("SPY", "20240419")
dates, _ := client.OptionListDates("EOD", "SPY", "20240419", "500000", "C")
contracts, _ := client.OptionListContracts("EOD", "SPY", "20240315")
```

### Snapshots

```go
ohlc, _ := client.OptionSnapshotOHLC("SPY", "20240419", "500000", "C")
trades, _ := client.OptionSnapshotTrade("SPY", "20240419", "500000", "C")
quotes, _ := client.OptionSnapshotQuote("SPY", "20240419", "500000", "C")
oi, _ := client.OptionSnapshotOpenInterest("SPY", "20240419", "500000", "C")
mv, _ := client.OptionSnapshotMarketValue("SPY", "20240419", "500000", "C")
```

### Snapshot Greeks

```go
all, _ := client.OptionSnapshotGreeksAll("SPY", "20240419", "500000", "C")
first, _ := client.OptionSnapshotGreeksFirstOrder("SPY", "20240419", "500000", "C")
second, _ := client.OptionSnapshotGreeksSecondOrder("SPY", "20240419", "500000", "C")
third, _ := client.OptionSnapshotGreeksThirdOrder("SPY", "20240419", "500000", "C")
iv, _ := client.OptionSnapshotGreeksIV("SPY", "20240419", "500000", "C")
```

### History

```go
eod, _ := client.OptionHistoryEOD("SPY", "20240419", "500000", "C", "20240101", "20240301")
bars, _ := client.OptionHistoryOHLC("SPY", "20240419", "500000", "C", "20240315", "60000")
trades, _ := client.OptionHistoryTrade("SPY", "20240419", "500000", "C", "20240315")
quotes, _ := client.OptionHistoryQuote("SPY", "20240419", "500000", "C", "20240315", "60000")
tq, _ := client.OptionHistoryTradeQuote("SPY", "20240419", "500000", "C", "20240315")
oi, _ := client.OptionHistoryOpenInterest("SPY", "20240419", "500000", "C", "20240315")
```

### History Greeks

```go
greeksEOD, _ := client.OptionHistoryGreeksEOD("SPY", "20240419", "500000", "C", "20240101", "20240301")
greeksAll, _ := client.OptionHistoryGreeksAll("SPY", "20240419", "500000", "C", "20240315", "60000")
greeksFirst, _ := client.OptionHistoryGreeksFirstOrder("SPY", "20240419", "500000", "C", "20240315", "60000")
greeksIV, _ := client.OptionHistoryGreeksIV("SPY", "20240419", "500000", "C", "20240315", "60000")
```

### Trade Greeks

```go
tgAll, _ := client.OptionHistoryTradeGreeksAll("SPY", "20240419", "500000", "C", "20240315")
tgFirst, _ := client.OptionHistoryTradeGreeksFirstOrder("SPY", "20240419", "500000", "C", "20240315")
tgIV, _ := client.OptionHistoryTradeGreeksIV("SPY", "20240419", "500000", "C", "20240315")
```

### At-Time

```go
trades, _ := client.OptionAtTimeTrade("SPY", "20240419", "500000", "C",
    "20240101", "20240301", "34200000")
quotes, _ := client.OptionAtTimeQuote("SPY", "20240419", "500000", "C",
    "20240101", "20240301", "34200000")
```

---

## Index Endpoints (9)

```go
symbols, _ := client.IndexListSymbols()
dates, _ := client.IndexListDates("SPX")
ohlc, _ := client.IndexSnapshotOHLC([]string{"SPX", "NDX"})
price, _ := client.IndexSnapshotPrice([]string{"SPX"})
mv, _ := client.IndexSnapshotMarketValue([]string{"SPX"})
eod, _ := client.IndexHistoryEOD("SPX", "20240101", "20240301")
bars, _ := client.IndexHistoryOHLC("SPX", "20240101", "20240301", "60000")
priceHist, _ := client.IndexHistoryPrice("SPX", "20240315", "60000")
atTime, _ := client.IndexAtTimePrice("SPX", "20240101", "20240301", "34200000")
```

---

## Rate Endpoints (1)

```go
result, _ := client.InterestRateHistoryEOD("SOFR", "20240101", "20240301")
```

---

## Calendar Endpoints (3)

```go
result, _ := client.CalendarOpenToday()
result, _ = client.CalendarOnDate("20240315")
result, _ = client.CalendarYear("2024")
```

---

## Time Reference

| Time (ET) | Milliseconds |
|-----------|-------------|
| 9:30 AM | `34200000` |
| 12:00 PM | `43200000` |
| 4:00 PM | `57600000` |
