# Historical Data (C++)

All historical data is accessed through `tdx::Client`. Every call runs through compiled Rust via the C FFI layer. RAII wrappers handle resource cleanup automatically.

## Connecting

```cpp
auto creds = tdx::Credentials::from_file("creds.txt");
auto client = tdx::Client::connect(creds, tdx::Config::production());
```

## Date Format

All dates are `YYYYMMDD` strings: `"20240315"` for March 15, 2024.

## Interval Format

Intervals are millisecond strings: `"60000"` for 1 minute, `"300000"` for 5 minutes.

---

## Stock Endpoints (14)

### List

```cpp
// All stock symbols
auto symbols = client.stock_list_symbols();

// Available dates by request type
auto dates = client.stock_list_dates("EOD", "AAPL");
```

### Snapshots

```cpp
// Latest quote snapshot (multiple symbols)
auto quotes = client.stock_snapshot_quote({"AAPL", "MSFT", "GOOGL"});
for (auto& q : quotes) {
    std::cout << "bid=" << q.bid << " ask=" << q.ask << std::endl;
}

auto ohlc = client.stock_snapshot_ohlc({"AAPL", "MSFT"});
auto trades = client.stock_snapshot_trade({"AAPL"});
auto mv = client.stock_snapshot_market_value({"AAPL"});
```

### History

```cpp
// End-of-day data
auto eod = client.stock_history_eod("AAPL", "20240101", "20240301");
for (auto& tick : eod) {
    std::cout << tick.date << ": O=" << tick.open
              << " H=" << tick.high << " L=" << tick.low
              << " C=" << tick.close << " V=" << tick.volume << std::endl;
}

// Intraday OHLC bars
auto bars = client.stock_history_ohlc("AAPL", "20240315", "60000");

// OHLC bars across date range
auto range_bars = client.stock_history_ohlc_range("AAPL", "20240101", "20240301", "300000");

// All trades
auto trades = client.stock_history_trade("AAPL", "20240315");

// NBBO quotes
auto quotes = client.stock_history_quote("AAPL", "20240315", "60000");

// Combined trade + quote
auto tq = client.stock_history_trade_quote("AAPL", "20240315");
```

### At-Time

```cpp
// Trade at 9:30 AM across a date range
auto trades = client.stock_at_time_trade("AAPL", "20240101", "20240301", "34200000");

// Quote at 9:30 AM
auto quotes = client.stock_at_time_quote("AAPL", "20240101", "20240301", "34200000");
```

---

## Option Endpoints (34)

### List

```cpp
auto symbols = client.option_list_symbols();
auto exps = client.option_list_expirations("SPY");
auto strikes = client.option_list_strikes("SPY", "20240419");
auto dates = client.option_list_dates("EOD", "SPY", "20240419", "500000", "C");
auto contracts = client.option_list_contracts("EOD", "SPY", "20240315");
```

### Snapshots

```cpp
auto ohlc = client.option_snapshot_ohlc("SPY", "20240419", "500000", "C");
auto trades = client.option_snapshot_trade("SPY", "20240419", "500000", "C");
auto quotes = client.option_snapshot_quote("SPY", "20240419", "500000", "C");
auto oi = client.option_snapshot_open_interest("SPY", "20240419", "500000", "C");
auto mv = client.option_snapshot_market_value("SPY", "20240419", "500000", "C");
```

### Snapshot Greeks

```cpp
auto all = client.option_snapshot_greeks_all("SPY", "20240419", "500000", "C");
auto first = client.option_snapshot_greeks_first_order("SPY", "20240419", "500000", "C");
auto second = client.option_snapshot_greeks_second_order("SPY", "20240419", "500000", "C");
auto third = client.option_snapshot_greeks_third_order("SPY", "20240419", "500000", "C");
auto iv = client.option_snapshot_greeks_implied_volatility("SPY", "20240419", "500000", "C");
```

### History

```cpp
auto eod = client.option_history_eod("SPY", "20240419", "500000", "C", "20240101", "20240301");
auto bars = client.option_history_ohlc("SPY", "20240419", "500000", "C", "20240315", "60000");
auto trades = client.option_history_trade("SPY", "20240419", "500000", "C", "20240315");
auto quotes = client.option_history_quote("SPY", "20240419", "500000", "C", "20240315", "60000");
auto tq = client.option_history_trade_quote("SPY", "20240419", "500000", "C", "20240315");
auto oi = client.option_history_open_interest("SPY", "20240419", "500000", "C", "20240315");
```

### History Greeks

```cpp
auto greeks_eod = client.option_history_greeks_eod("SPY", "20240419", "500000", "C",
                                                    "20240101", "20240301");
auto greeks_all = client.option_history_greeks_all("SPY", "20240419", "500000", "C",
                                                    "20240315", "60000");
auto greeks_iv = client.option_history_greeks_implied_volatility("SPY", "20240419", "500000", "C",
                                                                  "20240315", "60000");
```

### Trade Greeks

```cpp
auto tg_all = client.option_history_trade_greeks_all("SPY", "20240419", "500000", "C", "20240315");
auto tg_first = client.option_history_trade_greeks_first_order("SPY", "20240419", "500000", "C",
                                                                "20240315");
auto tg_iv = client.option_history_trade_greeks_implied_volatility("SPY", "20240419", "500000", "C",
                                                                    "20240315");
```

### At-Time

```cpp
auto trades = client.option_at_time_trade("SPY", "20240419", "500000", "C",
                                           "20240101", "20240301", "34200000");
auto quotes = client.option_at_time_quote("SPY", "20240419", "500000", "C",
                                           "20240101", "20240301", "34200000");
```

---

## Index Endpoints (9)

```cpp
auto symbols = client.index_list_symbols();
auto dates = client.index_list_dates("SPX");
auto ohlc = client.index_snapshot_ohlc({"SPX", "NDX"});
auto price = client.index_snapshot_price({"SPX"});
auto mv = client.index_snapshot_market_value({"SPX"});
auto eod = client.index_history_eod("SPX", "20240101", "20240301");
auto bars = client.index_history_ohlc("SPX", "20240101", "20240301", "60000");
auto price_hist = client.index_history_price("SPX", "20240315", "60000");
auto at_time = client.index_at_time_price("SPX", "20240101", "20240301", "34200000");
```

---

## Rate Endpoints (1)

```cpp
auto result = client.interest_rate_history_eod("SOFR", "20240101", "20240301");
```

---

## Calendar Endpoints (3)

```cpp
auto today = client.calendar_open_today();
auto date_info = client.calendar_on_date("20240315");
auto year_info = client.calendar_year("2024");
```

---

## Time Reference

| Time (ET) | Milliseconds |
|-----------|-------------|
| 9:30 AM | `34200000` |
| 12:00 PM | `43200000` |
| 4:00 PM | `57600000` |
