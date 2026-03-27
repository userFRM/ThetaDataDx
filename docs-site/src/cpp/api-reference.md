# API Reference (C++)

Complete type and method listing for the ThetaDataDx C++ SDK. Every call runs through compiled Rust via the C FFI layer. All objects use RAII for automatic resource cleanup.

## Credentials

```cpp
// From file (line 1 = email, line 2 = password)
auto creds = tdx::Credentials::from_file("creds.txt");

// Direct construction
auto creds = tdx::Credentials::from_email("email@example.com", "password");
```

## Config

```cpp
auto config = tdx::Config::production();  // production servers
auto config = tdx::Config::dev();         // dev servers
```

## Client

RAII class. All methods throw `std::runtime_error` on failure.

```cpp
auto client = tdx::Client::connect(creds, tdx::Config::production());
```

### Stock Methods (14)

| Method | Returns | Description |
|--------|---------|-------------|
| `stock_list_symbols()` | `vector<string>` | All stock symbols |
| `stock_list_dates(req_type, symbol)` | `vector<string>` | Available dates |
| `stock_snapshot_ohlc(symbols)` | `vector<OhlcTick>` | Latest OHLC |
| `stock_snapshot_trade(symbols)` | `vector<TradeTick>` | Latest trade |
| `stock_snapshot_quote(symbols)` | `vector<QuoteTick>` | Latest quote |
| `stock_snapshot_market_value(symbols)` | JSON result | Latest market value |
| `stock_history_eod(sym, start, end)` | `vector<EodTick>` | EOD data |
| `stock_history_ohlc(sym, date, interval)` | `vector<OhlcTick>` | Intraday OHLC |
| `stock_history_ohlc_range(sym, start, end, interval)` | `vector<OhlcTick>` | OHLC range |
| `stock_history_trade(sym, date)` | `vector<TradeTick>` | All trades |
| `stock_history_quote(sym, date, interval)` | `vector<QuoteTick>` | NBBO quotes |
| `stock_history_trade_quote(sym, date)` | JSON result | Trade+quote |
| `stock_at_time_trade(sym, start, end, time)` | `vector<TradeTick>` | Trade at time |
| `stock_at_time_quote(sym, start, end, time)` | `vector<QuoteTick>` | Quote at time |

### Option Methods (34)

All option methods follow the pattern `(symbol, expiration, strike, right, ...)`.

| Method | Returns |
|--------|---------|
| `option_list_symbols()` | `vector<string>` |
| `option_list_expirations(sym)` | `vector<string>` |
| `option_list_strikes(sym, exp)` | `vector<string>` |
| `option_list_dates(req, sym, exp, strike, right)` | `vector<string>` |
| `option_list_contracts(req, sym, date)` | JSON result |
| `option_snapshot_ohlc(sym, exp, strike, right)` | `vector<OhlcTick>` |
| `option_snapshot_trade(sym, exp, strike, right)` | `vector<TradeTick>` |
| `option_snapshot_quote(sym, exp, strike, right)` | `vector<QuoteTick>` |
| `option_snapshot_open_interest(...)` | JSON result |
| `option_snapshot_market_value(...)` | JSON result |
| `option_snapshot_greeks_all(...)` | JSON result |
| `option_snapshot_greeks_first_order(...)` | JSON result |
| `option_snapshot_greeks_second_order(...)` | JSON result |
| `option_snapshot_greeks_third_order(...)` | JSON result |
| `option_snapshot_greeks_implied_volatility(...)` | JSON result |
| `option_history_eod(sym, exp, strike, right, start, end)` | `vector<EodTick>` |
| `option_history_ohlc(sym, exp, strike, right, date, interval)` | `vector<OhlcTick>` |
| `option_history_trade(sym, exp, strike, right, date)` | `vector<TradeTick>` |
| `option_history_quote(sym, exp, strike, right, date, interval)` | `vector<QuoteTick>` |
| Plus 15 more history/trade-greeks/at-time variants | |

### Index Methods (9)

| Method | Returns |
|--------|---------|
| `index_list_symbols()` | `vector<string>` |
| `index_list_dates(sym)` | `vector<string>` |
| `index_snapshot_ohlc(symbols)` | `vector<OhlcTick>` |
| `index_snapshot_price(symbols)` | JSON result |
| `index_snapshot_market_value(symbols)` | JSON result |
| `index_history_eod(sym, start, end)` | `vector<EodTick>` |
| `index_history_ohlc(sym, start, end, interval)` | `vector<OhlcTick>` |
| `index_history_price(sym, date, interval)` | JSON result |
| `index_at_time_price(sym, start, end, time)` | JSON result |

### Calendar & Rate Methods

| Method | Returns |
|--------|---------|
| `calendar_open_today()` | JSON result |
| `calendar_on_date(date)` | JSON result |
| `calendar_year(year)` | JSON result |
| `interest_rate_history_eod(sym, start, end)` | JSON result |

## Standalone Functions

```cpp
// All 22 Greeks
auto g = tdx::all_greeks(spot, strike, rate, div_yield, tte, price, is_call);
// g.iv, g.delta, g.gamma, g.theta, g.vega, g.rho, etc.

// Just IV
auto [iv, err] = tdx::implied_volatility(spot, strike, rate, div_yield, tte, price, is_call);
```

## Streaming (via Client)

| Method | Signature | Description |
|--------|-----------|-------------|
| `start_streaming` | `(buf_size) -> void` | Connect to FPSS streaming servers |
| `subscribe_quotes` | `(root, sec_type) -> int32_t` | Subscribe to quotes |
| `subscribe_trades` | `(root, sec_type) -> int32_t` | Subscribe to trades |
| `subscribe_open_interest` | `(root, sec_type) -> int32_t` | Subscribe to OI |
| `next_event` | `(timeout_ms) -> unique_ptr<FpssEvent>` | Poll next event |
| `stop_streaming` | `() -> void` | Graceful shutdown of streaming |

## Tick Types

### EodTick

```cpp
struct EodTick {
    int32_t date;
    double open, high, low, close;
    int32_t volume;
    double bid, ask;
};
```

### OhlcTick

```cpp
struct OhlcTick {
    int32_t ms_of_day, date;
    double open, high, low, close;
    int32_t volume, count;
};
```

### TradeTick

```cpp
struct TradeTick {
    int32_t ms_of_day, date;
    double price;
    int32_t size, exchange, condition;
};
```

### QuoteTick

```cpp
struct QuoteTick {
    int32_t ms_of_day, date;
    double bid, ask;
    int32_t bid_size, ask_size;
};
```

## Security Type Enum

```cpp
enum class SecType : int32_t {
    Stock  = 0,
    Option = 1,
    Index  = 2,
    Rate   = 3,
};
```

## C FFI Layer

The raw C interface can be used directly from any language with C interop:

| Category | Functions |
|----------|-----------|
| Lifecycle | `tdx_credentials_new`, `tdx_credentials_from_file`, `tdx_credentials_free` |
| Config | `tdx_config_production`, `tdx_config_dev`, `tdx_config_free` |
| Client | `tdx_client_connect`, `tdx_client_free` |
| Greeks | `tdx_all_greeks`, `tdx_implied_volatility` |
| FPSS | `tdx_fpss_connect`, `tdx_fpss_subscribe_*`, `tdx_fpss_next_event`, `tdx_fpss_shutdown` |
| Memory | `tdx_string_free`, `tdx_last_error` |

Results are returned as JSON strings (`char*`) that must be freed with `tdx_string_free`.
