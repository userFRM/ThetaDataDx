# Historical Data (Rust)

All historical data is accessed through `DirectClient`, which communicates over gRPC with ThetaData's MDDS servers.

## Connecting

```rust
use thetadatadx::{DirectClient, Credentials, DirectConfig};

let creds = Credentials::from_file("creds.txt")?;
let client = DirectClient::connect(&creds, DirectConfig::production()).await?;
```

## Date Format

All dates are `YYYYMMDD` strings: `"20240315"` for March 15, 2024.

## Interval Format

Intervals are millisecond strings: `"60000"` for 1 minute, `"300000"` for 5 minutes, `"3600000"` for 1 hour.

---

## Stock Endpoints (14)

### List

```rust
// All available stock symbols
let symbols: Vec<String> = client.stock_list_symbols().await?;

// Available dates for a stock by request type
let dates: Vec<String> = client.stock_list_dates("EOD", "AAPL").await?;
```

### Snapshots

```rust
// Latest OHLC snapshot (one or more symbols)
let ticks: Vec<OhlcTick> = client.stock_snapshot_ohlc(&["AAPL", "MSFT"]).await?;

// Latest trade snapshot
let ticks: Vec<TradeTick> = client.stock_snapshot_trade(&["AAPL"]).await?;

// Latest NBBO quote snapshot
let ticks: Vec<QuoteTick> = client.stock_snapshot_quote(&["AAPL", "MSFT", "GOOGL"]).await?;
for q in &ticks {
    println!("bid={} ask={}", q.bid_price(), q.ask_price());
}

// Latest market value snapshot
let table: proto::DataTable = client.stock_snapshot_market_value(&["AAPL"]).await?;
```

### History

```rust
// End-of-day data for a date range
let eod: Vec<EodTick> = client.stock_history_eod("AAPL", "20240101", "20240301").await?;
for t in &eod {
    println!("{}: O={} H={} L={} C={} V={}",
        t.date, t.open_price(), t.high_price(),
        t.low_price(), t.close_price(), t.volume);
}

// Intraday OHLC bars (single date)
let bars: Vec<OhlcTick> = client.stock_history_ohlc("AAPL", "20240315", "60000").await?;

// Intraday OHLC bars (date range)
let bars: Vec<OhlcTick> = client.stock_history_ohlc_range(
    "AAPL", "20240101", "20240301", "300000"  // 5-min bars
).await?;

// All trades for a date
let trades: Vec<TradeTick> = client.stock_history_trade("AAPL", "20240315").await?;

// NBBO quotes at a given interval (use "0" for every quote change)
let quotes: Vec<QuoteTick> = client.stock_history_quote("AAPL", "20240315", "60000").await?;

// Combined trade + quote ticks (returns raw DataTable)
let table: proto::DataTable = client.stock_history_trade_quote("AAPL", "20240315").await?;
```

### At-Time

```rust
// Trade at a specific time of day across a date range
// time_of_day is milliseconds from midnight ET (34200000 = 9:30 AM)
let trades: Vec<TradeTick> = client.stock_at_time_trade(
    "AAPL", "20240101", "20240301", "34200000"
).await?;

// Quote at a specific time of day across a date range
let quotes: Vec<QuoteTick> = client.stock_at_time_quote(
    "AAPL", "20240101", "20240301", "34200000"
).await?;
```

### Streaming Large Responses

For endpoints returning millions of rows, use `_stream` variants to process chunk by chunk:

```rust
client.stock_history_trade_stream("AAPL", "20240315", |chunk| {
    println!("Got {} trades in this chunk", chunk.len());
    Ok(())
}).await?;

client.stock_history_quote_stream("AAPL", "20240315", "0", |chunk| {
    println!("Got {} quotes in this chunk", chunk.len());
    Ok(())
}).await?;
```

---

## Option Endpoints (34)

### List

```rust
// All option underlying symbols
let symbols: Vec<String> = client.option_list_symbols().await?;

// Available dates for a specific contract
let dates: Vec<String> = client.option_list_dates(
    "EOD", "SPY", "20240419", "500000", "C"
).await?;

// Expiration dates for an underlying
let exps: Vec<String> = client.option_list_expirations("SPY").await?;

// Strike prices for a given expiration
let strikes: Vec<String> = client.option_list_strikes("SPY", "20240419").await?;

// All contracts for a symbol on a date
let table: proto::DataTable = client.option_list_contracts("EOD", "SPY", "20240315").await?;
```

### Snapshots

```rust
let ohlc = client.option_snapshot_ohlc("SPY", "20240419", "500000", "C").await?;
let trades = client.option_snapshot_trade("SPY", "20240419", "500000", "C").await?;
let quotes = client.option_snapshot_quote("SPY", "20240419", "500000", "C").await?;
let oi = client.option_snapshot_open_interest("SPY", "20240419", "500000", "C").await?;
let mv = client.option_snapshot_market_value("SPY", "20240419", "500000", "C").await?;
```

### Snapshot Greeks

```rust
// All Greeks at once
let all = client.option_snapshot_greeks_all("SPY", "20240419", "500000", "C").await?;

// By order
let first = client.option_snapshot_greeks_first_order("SPY", "20240419", "500000", "C").await?;
let second = client.option_snapshot_greeks_second_order("SPY", "20240419", "500000", "C").await?;
let third = client.option_snapshot_greeks_third_order("SPY", "20240419", "500000", "C").await?;

// Just IV
let iv = client.option_snapshot_greeks_implied_volatility("SPY", "20240419", "500000", "C").await?;
```

### History

```rust
// End-of-day option data
let eod: Vec<EodTick> = client.option_history_eod(
    "SPY", "20240419", "500000", "C", "20240101", "20240301"
).await?;

// Intraday OHLC bars
let bars: Vec<OhlcTick> = client.option_history_ohlc(
    "SPY", "20240419", "500000", "C", "20240315", "60000"
).await?;

// All trades for a date
let trades: Vec<TradeTick> = client.option_history_trade(
    "SPY", "20240419", "500000", "C", "20240315"
).await?;

// NBBO quotes at a given interval
let quotes: Vec<QuoteTick> = client.option_history_quote(
    "SPY", "20240419", "500000", "C", "20240315", "60000"
).await?;

// Combined trade + quote ticks
let table = client.option_history_trade_quote(
    "SPY", "20240419", "500000", "C", "20240315"
).await?;

// Open interest history
let table = client.option_history_open_interest(
    "SPY", "20240419", "500000", "C", "20240315"
).await?;
```

### History Greeks

```rust
// EOD Greeks over a date range
let table = client.option_history_greeks_eod(
    "SPY", "20240419", "500000", "C", "20240101", "20240301"
).await?;

// Intraday Greeks sampled by interval
let all = client.option_history_greeks_all(
    "SPY", "20240419", "500000", "C", "20240315", "60000"
).await?;
let first = client.option_history_greeks_first_order(
    "SPY", "20240419", "500000", "C", "20240315", "60000"
).await?;
let second = client.option_history_greeks_second_order(
    "SPY", "20240419", "500000", "C", "20240315", "60000"
).await?;
let third = client.option_history_greeks_third_order(
    "SPY", "20240419", "500000", "C", "20240315", "60000"
).await?;
let iv = client.option_history_greeks_implied_volatility(
    "SPY", "20240419", "500000", "C", "20240315", "60000"
).await?;
```

### Trade Greeks

Greeks computed on each individual trade:

```rust
let all = client.option_history_trade_greeks_all(
    "SPY", "20240419", "500000", "C", "20240315"
).await?;
let first = client.option_history_trade_greeks_first_order(
    "SPY", "20240419", "500000", "C", "20240315"
).await?;
let second = client.option_history_trade_greeks_second_order(
    "SPY", "20240419", "500000", "C", "20240315"
).await?;
let third = client.option_history_trade_greeks_third_order(
    "SPY", "20240419", "500000", "C", "20240315"
).await?;
let iv = client.option_history_trade_greeks_implied_volatility(
    "SPY", "20240419", "500000", "C", "20240315"
).await?;
```

### At-Time

```rust
let trades: Vec<TradeTick> = client.option_at_time_trade(
    "SPY", "20240419", "500000", "C",
    "20240101", "20240301", "34200000"  // 9:30 AM ET
).await?;

let quotes: Vec<QuoteTick> = client.option_at_time_quote(
    "SPY", "20240419", "500000", "C",
    "20240101", "20240301", "34200000"
).await?;
```

### Streaming Large Option Responses

```rust
client.option_history_trade_stream(
    "SPY", "20240419", "500000", "C", "20240315",
    |chunk| { Ok(()) }
).await?;

client.option_history_quote_stream(
    "SPY", "20240419", "500000", "C", "20240315", "0",
    |chunk| { Ok(()) }
).await?;
```

---

## Index Endpoints (9)

### List

```rust
let symbols: Vec<String> = client.index_list_symbols().await?;
let dates: Vec<String> = client.index_list_dates("SPX").await?;
```

### Snapshots

```rust
let ohlc: Vec<OhlcTick> = client.index_snapshot_ohlc(&["SPX", "NDX"]).await?;
let table: proto::DataTable = client.index_snapshot_price(&["SPX", "NDX"]).await?;
let table: proto::DataTable = client.index_snapshot_market_value(&["SPX"]).await?;
```

### History

```rust
let eod: Vec<EodTick> = client.index_history_eod("SPX", "20240101", "20240301").await?;

let bars: Vec<OhlcTick> = client.index_history_ohlc(
    "SPX", "20240101", "20240301", "60000"
).await?;

let table: proto::DataTable = client.index_history_price("SPX", "20240315", "60000").await?;
```

### At-Time

```rust
let table: proto::DataTable = client.index_at_time_price(
    "SPX", "20240101", "20240301", "34200000"
).await?;
```

---

## Rate Endpoints (1)

```rust
let table: proto::DataTable = client.interest_rate_history_eod(
    "SOFR", "20240101", "20240301"
).await?;
```

Available rate symbols: `SOFR`, `TREASURY_M1`, `TREASURY_M3`, `TREASURY_M6`, `TREASURY_Y1`, `TREASURY_Y2`, `TREASURY_Y3`, `TREASURY_Y5`, `TREASURY_Y7`, `TREASURY_Y10`, `TREASURY_Y20`, `TREASURY_Y30`.

---

## Calendar Endpoints (3)

```rust
let table: proto::DataTable = client.calendar_open_today().await?;
let table: proto::DataTable = client.calendar_on_date("20240315").await?;
let table: proto::DataTable = client.calendar_year("2024").await?;
```

---

## Time Reference

| Time (ET) | Milliseconds |
|-----------|-------------|
| 9:30 AM | `34200000` |
| 12:00 PM | `43200000` |
| 4:00 PM | `57600000` |

## Empty Responses

When a query returns no data (e.g., a non-trading date), the SDK returns an empty collection rather than an error. Check `.is_empty()` or `len() == 0`.
