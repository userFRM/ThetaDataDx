# thetadatadx (C++)

C++ SDK for ThetaData market data. Header-only RAII wrappers over the `thetadatadx` Rust crate via the shared C FFI layer.

Every call crosses the C ABI boundary into compiled Rust: gRPC communication, protobuf parsing, zstd decompression, and TCP streaming run inside the `thetadatadx` crate.

## Prerequisites

- C++17 compiler
- CMake 3.16+
- Rust toolchain (for building the FFI library)

## Platform Support

- Linux: CI-validated
- macOS: CI-validated
- Windows: CI-validated

## Building

First, build the Rust FFI library:

```bash
# From the repository root
cargo build --release -p thetadatadx-ffi
```

Then build the C++ SDK:

```bash
cmake -S sdks/cpp -B build/cpp
cmake --build build/cpp --config Release --target thetadatadx_cpp
```

Run the example:

```bash
./build/cpp/thetadatadx_example
```

## Quick Start

```cpp
#include "thetadx.hpp"
#include <iostream>

int main() {
    auto creds = tdx::Credentials::from_file("creds.txt");
    // Or inline: auto creds = tdx::Credentials("user@example.com", "your-password");
    auto client = tdx::Client::connect(creds, tdx::Config::production());

    // Fetch EOD stock data -- all prices are f64, no decoding needed
    auto eod = client.stock_history_eod("AAPL", "20240101", "20240301");
    for (auto& tick : eod) {
        std::cout << tick.date << ": O=" << tick.open
                  << " H=" << tick.high << " L=" << tick.low
                  << " C=" << tick.close << std::endl;
    }

    // Snapshot: latest quote for multiple symbols
    auto quotes = client.stock_snapshot_quote({"AAPL", "MSFT", "GOOG"});
    for (auto& q : quotes) {
        std::cout << "bid=" << q.bid
                  << " ask=" << q.ask << std::endl;
    }

    // Greeks (no server connection needed)
    auto g = tdx::all_greeks(450.0, 455.0, 0.05, 0.015, 30.0/365.0, 8.50, "C");
    std::cout << "IV=" << g.iv << " Delta=" << g.delta << std::endl;
}
```

## API

### Credentials

- `Credentials::from_file(path)` - load from file (line 1 = email, line 2 = password)
- `Credentials::from_email(email, password)` - direct construction

### Config

- `Config::production()` - ThetaData NJ production servers
- `Config::dev()` - Dev FPSS servers (port 20200, infinite historical replay)
- `Config::stage()` - Stage FPSS servers (port 20100, testing, unstable)

### Client

RAII class. All methods throw `std::runtime_error` on failure.

```cpp
auto client = tdx::Client::connect(creds, tdx::Config::production());
```

#### Stock - List (2)

| Method | Returns | Description |
|--------|---------|-------------|
| `stock_list_symbols()` | `vector<string>` | All stock symbols |
| `stock_list_dates(req_type, symbol)` | `vector<string>` | Available dates for a stock |

#### Stock - Snapshot (4)

| Method | Returns | Description |
|--------|---------|-------------|
| `stock_snapshot_ohlc(symbols)` | `vector<OhlcTick>` | Latest OHLC snapshot |
| `stock_snapshot_trade(symbols)` | `vector<TradeTick>` | Latest trade snapshot |
| `stock_snapshot_quote(symbols)` | `vector<QuoteTick>` | Latest NBBO quote snapshot |
| `stock_snapshot_market_value(symbols)` | `vector<MarketValueTick>` | Latest market value snapshot |

#### Stock - History (6)

| Method | Returns | Description |
|--------|---------|-------------|
| `stock_history_eod(sym, start_date, end_date)` | `vector<EodTick>` | EOD data |
| `stock_history_ohlc(sym, date, interval)` | `vector<OhlcTick>` | Intraday OHLC bars. `interval` accepts ms (`"60000"`) or shorthand (`"1m"`). |
| `stock_history_ohlc_range(sym, start_date, end_date, interval)` | `vector<OhlcTick>` | OHLC bars across date range. `interval` accepts ms or shorthand. |
| `stock_history_trade(sym, date)` | `vector<TradeTick>` | All trades on a date |
| `stock_history_quote(sym, date, interval)` | `vector<QuoteTick>` | NBBO quotes. `interval` accepts ms or shorthand. |
| `stock_history_trade_quote(sym, date)` | `vector<TradeQuoteTick>` | Combined trade + quote ticks |

#### Stock - At-Time (2)

| Method | Returns | Description |
|--------|---------|-------------|
| `stock_at_time_trade(sym, start_date, end_date, time)` | `vector<TradeTick>` | Trade at a specific time across date range |
| `stock_at_time_quote(sym, start_date, end_date, time)` | `vector<QuoteTick>` | Quote at a specific time across date range |

#### Option - List (5)

| Method | Returns | Description |
|--------|---------|-------------|
| `option_list_symbols()` | `vector<string>` | All option underlyings |
| `option_list_dates(req, sym, expiration, strike, right)` | `vector<string>` | Available dates for an option contract |
| `option_list_expirations(sym)` | `vector<string>` | Expiration dates |
| `option_list_strikes(sym, expiration)` | `vector<string>` | Strike prices |
| `option_list_contracts(req, sym, date)` | `vector<OptionContract>` | All option contracts on a date |

#### Option - Snapshot (10)

| Method | Returns | Description |
|--------|---------|-------------|
| `option_snapshot_ohlc(sym, expiration, strike, right)` | `vector<OhlcTick>` | Latest OHLC snapshot |
| `option_snapshot_trade(sym, expiration, strike, right)` | `vector<TradeTick>` | Latest trade snapshot |
| `option_snapshot_quote(sym, expiration, strike, right)` | `vector<QuoteTick>` | Latest quote snapshot |
| `option_snapshot_open_interest(sym, expiration, strike, right)` | `vector<OpenInterestTick>` | Latest open interest snapshot |
| `option_snapshot_market_value(sym, expiration, strike, right)` | `vector<MarketValueTick>` | Latest market value snapshot |
| `option_snapshot_greeks_implied_volatility(sym, expiration, strike, right)` | `vector<IvTick>` | IV snapshot |
| `option_snapshot_greeks_all(sym, expiration, strike, right)` | `vector<GreeksTick>` | All Greeks snapshot |
| `option_snapshot_greeks_first_order(sym, expiration, strike, right)` | `vector<GreeksTick>` | First-order Greeks snapshot |
| `option_snapshot_greeks_second_order(sym, expiration, strike, right)` | `vector<GreeksTick>` | Second-order Greeks snapshot |
| `option_snapshot_greeks_third_order(sym, expiration, strike, right)` | `vector<GreeksTick>` | Third-order Greeks snapshot |

#### Option - History (6)

| Method | Returns | Description |
|--------|---------|-------------|
| `option_history_eod(sym, expiration, strike, right, start_date, end_date)` | `vector<EodTick>` | EOD option data |
| `option_history_ohlc(sym, expiration, strike, right, date, interval)` | `vector<OhlcTick>` | Intraday OHLC for options |
| `option_history_trade(sym, expiration, strike, right, date)` | `vector<TradeTick>` | All trades for an option |
| `option_history_quote(sym, expiration, strike, right, date, interval)` | `vector<QuoteTick>` | Quotes for an option |
| `option_history_trade_quote(sym, expiration, strike, right, date)` | `vector<TradeQuoteTick>` | Combined trade + quote for an option |
| `option_history_open_interest(sym, expiration, strike, right, date)` | `vector<OpenInterestTick>` | Open interest history |

#### Option - History Greeks (11)

| Method | Returns | Description |
|--------|---------|-------------|
| `option_history_greeks_eod(sym, expiration, strike, right, start_date, end_date[, options])` | `vector<GreeksTick>` | EOD Greeks history. Optional `EndpointRequestOptions` exposes filters such as `strike_range`. |
| `option_history_greeks_all(sym, expiration, strike, right, date, interval)` | `vector<GreeksTick>` | All Greeks history (intraday) |
| `option_history_trade_greeks_all(sym, expiration, strike, right, date)` | `vector<GreeksTick>` | All Greeks on each trade |
| `option_history_greeks_first_order(sym, expiration, strike, right, date, interval)` | `vector<GreeksTick>` | First-order Greeks history |
| `option_history_trade_greeks_first_order(sym, expiration, strike, right, date)` | `vector<GreeksTick>` | First-order Greeks on each trade |
| `option_history_greeks_second_order(sym, expiration, strike, right, date, interval)` | `vector<GreeksTick>` | Second-order Greeks history |
| `option_history_trade_greeks_second_order(sym, expiration, strike, right, date)` | `vector<GreeksTick>` | Second-order Greeks on each trade |
| `option_history_greeks_third_order(sym, expiration, strike, right, date, interval)` | `vector<GreeksTick>` | Third-order Greeks history |
| `option_history_trade_greeks_third_order(sym, expiration, strike, right, date)` | `vector<GreeksTick>` | Third-order Greeks on each trade |
| `option_history_greeks_implied_volatility(sym, expiration, strike, right, date, interval)` | `vector<IvTick>` | IV history (intraday) |
| `option_history_trade_greeks_implied_volatility(sym, expiration, strike, right, date)` | `vector<IvTick>` | IV on each trade |

#### Option - At-Time (2)

| Method | Returns | Description |
|--------|---------|-------------|
| `option_at_time_trade(sym, expiration, strike, right, start_date, end_date, time)` | `vector<TradeTick>` | Trade at a specific time for an option |
| `option_at_time_quote(sym, expiration, strike, right, start_date, end_date, time)` | `vector<QuoteTick>` | Quote at a specific time for an option |

#### Index - List (2)

| Method | Returns | Description |
|--------|---------|-------------|
| `index_list_symbols()` | `vector<string>` | All index symbols |
| `index_list_dates(sym)` | `vector<string>` | Available dates for an index |

#### Index - Snapshot (3)

| Method | Returns | Description |
|--------|---------|-------------|
| `index_snapshot_ohlc(symbols)` | `vector<OhlcTick>` | Latest OHLC snapshot for indices |
| `index_snapshot_price(symbols)` | `vector<PriceTick>` | Latest price snapshot for indices |
| `index_snapshot_market_value(symbols)` | `vector<MarketValueTick>` | Latest market value for indices |

#### Index - History (3)

| Method | Returns | Description |
|--------|---------|-------------|
| `index_history_eod(sym, start_date, end_date)` | `vector<EodTick>` | EOD index data |
| `index_history_ohlc(sym, start_date, end_date, interval)` | `vector<OhlcTick>` | Intraday OHLC for an index |
| `index_history_price(sym, date, interval)` | `vector<PriceTick>` | Intraday price history |

#### Index - At-Time (1)

| Method | Returns | Description |
|--------|---------|-------------|
| `index_at_time_price(sym, start_date, end_date, time)` | `vector<PriceTick>` | Index price at a specific time |

#### Calendar (3)

| Method | Returns | Description |
|--------|---------|-------------|
| `calendar_open_today()` | `vector<CalendarDay>` | Whether the market is open today |
| `calendar_on_date(date)` | `vector<CalendarDay>` | Calendar for a specific date |
| `calendar_year(year)` | `vector<CalendarDay>` | Calendar for an entire year |

#### Interest Rate (1)

| Method | Returns | Description |
|--------|---------|-------------|
| `interest_rate_history_eod(sym, start_date, end_date)` | `vector<InterestRateTick>` | EOD interest rate history |

### Standalone Functions

```cpp
// All 22 Greeks + IV. `right` accepts "C"/"P" or "call"/"put" (case-insensitive).
auto g = tdx::all_greeks(spot, strike, rate, div_yield, tte, price, "C");
// g.iv, g.delta, g.gamma, g.theta, g.vega, g.rho, g.vanna, g.charm, etc.

// Just IV
auto [iv, err] = tdx::implied_volatility(spot, strike, rate, div_yield, tte, price, "C");
```

### Tick Types

All endpoints return fully typed C++ structs. No raw JSON.

| Struct | Fields | Used by |
|--------|--------|---------|
| `EodTick` | ms_of_day, open, high, low, close, volume, count, bid, ask, date, **expiration, strike, right** | EOD endpoints |
| `OhlcTick` | ms_of_day, open, high, low, close, volume, count, date, **expiration, strike, right** | OHLC endpoints |
| `TradeTick` | ms_of_day, sequence, condition, size, exchange, price, condition_flags, price_flags, volume_type, records_back, date, **expiration, strike, right** | Trade endpoints |
| `QuoteTick` | ms_of_day, bid_size, bid_exchange, bid, bid_condition, ask_size, ask_exchange, ask, ask_condition, midpoint, date, **expiration, strike, right** | Quote endpoints |
| `TradeQuoteTick` | ms_of_day, sequence, ext_condition1-4, condition, size, exchange, price, condition_flags, price_flags, volume_type, records_back, quote_ms_of_day, bid_size, bid_exchange, bid, bid_condition, ask_size, ask_exchange, ask, ask_condition, date, **expiration, strike, right** | Trade+quote endpoints |
| `OpenInterestTick` | ms_of_day, open_interest, date, **expiration, strike, right** | Open interest endpoints |
| `GreeksTick` | ms_of_day, implied_volatility, delta, gamma, theta, vega, rho, iv_error, vanna, charm, vomma, veta, speed, zomma, color, ultima, d1, d2, dual_delta, dual_gamma, epsilon, lambda, vera, date, **expiration, strike, right** | Greeks snapshot/history |
| `IvTick` | ms_of_day, implied_volatility, iv_error, date, **expiration, strike, right** | IV-only endpoints |
| `PriceTick` | ms_of_day, price, date | Index price endpoints |
| `MarketValueTick` | ms_of_day, market_bid, market_ask, market_price, date, **expiration, strike, right** | Market value endpoints |
| `OptionContract` | root, expiration, strike, right | option_list_contracts |
| `CalendarDay` | date, is_open, open_time, close_time, status | Calendar endpoints |
| `InterestRateTick` | ms_of_day, rate, date | Interest rate endpoints |
| `Greeks` | implied_volatility, delta, gamma, theta, vega, rho, iv_error, vanna, charm, vomma, veta, speed, zomma, color, ultima, d1, d2, dual_delta, dual_gamma, epsilon, lambda, vera | Standalone all_greeks() |

All price fields (`open`, `high`, `low`, `close`, `bid`, `ask`, `price`, `strike`) are `double` (f64) -- decoded during parsing. No `price_type` in the public API.

**Contract identification fields** (bold above): `expiration`, `strike`, `right` are populated by the server on wildcard queries (pass `"0"` for expiration/strike). On single-contract queries these fields are `0`. The `right` input parameter accepts `"call"`, `"put"`, `"C"`, `"P"` (case-insensitive), plus `"both"`/`"*"` on endpoints that support a wildcard -- not `"0"`. Output values stay `"C"`/`"P"`.

## FPSS Streaming

Real-time market data via ThetaData's FPSS servers. Streaming uses a **separate `FpssClient` class**, not methods on `Client`. Events are returned as typed `#[repr(C)]` structs -- no JSON parsing on the hot path.

```cpp
#include "thetadx.hpp"
#include <iostream>

int main() {
    auto creds = tdx::Credentials::from_file("creds.txt");
    auto config = tdx::Config::production();

    // Create a streaming client (separate from the historical Client)
    tdx::FpssClient fpss(creds, config);

    // Subscribe to real-time quotes
    int req_id = fpss.subscribe_quotes("AAPL");
    std::cout << "Subscribed (req_id=" << req_id << ")" << std::endl;

    // Poll for events (returns FpssEventPtr, nullptr on timeout)
    while (true) {
        auto event = fpss.next_event(5000);  // 5s timeout
        if (!event) continue;

        switch (event->kind) {
        case TDX_FPSS_QUOTE:
            std::cout << "Quote: bid=" << event->quote.bid
                      << " ask=" << event->quote.ask << std::endl;
            break;
        case TDX_FPSS_TRADE:
            std::cout << "Trade: price=" << event->trade.price
                      << " size=" << event->trade.size << std::endl;
            break;
        case TDX_FPSS_CONTROL:
            if (event->control.detail)
                std::cout << "Control: " << event->control.detail << std::endl;
            break;
        default:
            break;
        }
    }

    fpss.shutdown();
}
```

All prices in streaming events are `double` (f64) -- decoded during parsing. Access them directly: `event->quote.bid`, `event->trade.price`, etc. No `price_type` decoding needed.

### FpssClient API

| Method | Returns | Description |
|--------|---------|-------------|
| `FpssClient(creds, config)` | - | Connect to FPSS streaming servers |
| `subscribe_quotes(symbol)` | `int` | Subscribe to quote data for a stock symbol |
| `subscribe_trades(symbol)` | `int` | Subscribe to trade data for a stock symbol |
| `subscribe_open_interest(symbol)` | `int` | Subscribe to open interest data for a stock symbol |
| `subscribe_option_quotes(symbol, expiration, strike, right)` | `int` | Subscribe to option quote data |
| `subscribe_option_trades(symbol, expiration, strike, right)` | `int` | Subscribe to option trade data |
| `subscribe_option_open_interest(symbol, expiration, strike, right)` | `int` | Subscribe to option open interest data |
| `subscribe_full_trades(sec_type)` | `int` | Subscribe to all trades for a security type (`"STOCK"`, `"OPTION"`, `"INDEX"`) |
| `subscribe_full_open_interest(sec_type)` | `int` | Subscribe to all OI for a security type |
| `unsubscribe_full_trades(sec_type)` | `int` | Unsubscribe from all trades for a security type |
| `unsubscribe_full_open_interest(sec_type)` | `int` | Unsubscribe from all OI for a security type |
| `unsubscribe_quotes(symbol)` | `int` | Unsubscribe from quote data |
| `unsubscribe_trades(symbol)` | `int` | Unsubscribe from trade data |
| `unsubscribe_open_interest(symbol)` | `int` | Unsubscribe from open interest data |
| `unsubscribe_option_quotes(symbol, expiration, strike, right)` | `int` | Unsubscribe from option quote data |
| `unsubscribe_option_trades(symbol, expiration, strike, right)` | `int` | Unsubscribe from option trade data |
| `unsubscribe_option_open_interest(symbol, expiration, strike, right)` | `int` | Unsubscribe from option open interest data |
| `is_authenticated()` | `bool` | Check if the client is currently authenticated |
| `contract_lookup(id)` | `optional<string>` | Look up a contract by server-assigned ID |
| `contract_map()` | `map<int32_t, string>` | Get the full contract ID mapping |
| `active_subscriptions()` | `vector<Subscription>` | Get active subscriptions as typed structs |
| `next_event(timeout_ms)` | `FpssEventPtr` | Poll for the next event (nullptr on timeout) |
| `reconnect()` | `void` | Reconnect streaming and restore subscriptions |
| `shutdown()` | `void` | Shut down the FPSS client |

### FPSS Event Types

| Type | Fields | Used when |
|------|--------|-----------|
| `TdxFpssQuote` | contract_id, ms_of_day, bid_size, bid_exchange, bid, bid_condition, ask_size, ask_exchange, ask, ask_condition, date, received_at_ns | `kind == TDX_FPSS_QUOTE` |
| `TdxFpssTrade` | contract_id, ms_of_day, sequence, ext_condition1-4, condition, size, exchange, price, condition_flags, price_flags, volume_type, records_back, date, received_at_ns | `kind == TDX_FPSS_TRADE` |
| `TdxFpssOpenInterest` | contract_id, ms_of_day, open_interest, date, received_at_ns | `kind == TDX_FPSS_OPEN_INTEREST` |
| `TdxFpssOhlcvc` | contract_id, ms_of_day, open, high, low, close, volume (i64), count (i64), date, received_at_ns | `kind == TDX_FPSS_OHLCVC` |
| `TdxFpssControl` | kind (0-7), id, detail (nullable string) | `kind == TDX_FPSS_CONTROL` |

`FpssClient` is non-copyable but movable. The destructor calls `shutdown()` automatically.

## Architecture

```
C++ code
    |  (RAII wrappers)
    v
thetadatadx.h (C FFI)
    |
    v
libthetadatadx_ffi.so / .a
    |  (Rust FFI crate)
    v
thetadatadx Rust crate
    |  (tonic gRPC / tokio TCP)
    v
ThetaData servers
```
