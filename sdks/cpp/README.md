# thetadatadx (C++)

C++ SDK for ThetaData market data, powered by the `thetadatadx` Rust crate via C FFI.

**This is NOT a C++ reimplementation.** Every call goes through compiled Rust via a C FFI layer. gRPC communication, protobuf parsing, zstd decompression, and TCP streaming all happen at native Rust speed. C++ is just the interface.

## Prerequisites

- C++17 compiler
- CMake 3.16+
- Rust toolchain (for building the FFI library)

## Building

First, build the Rust FFI library:

```bash
# From the repository root
cargo build --release -p thetadatadx-ffi
```

Then build the C++ SDK:

```bash
cd sdks/cpp
mkdir build && cd build
cmake ..
make
```

Run the example:

```bash
./thetadatadx_example
```

## Quick Start

```cpp
#include "thetadx.hpp"
#include <iostream>

int main() {
    auto creds = tdx::Credentials::from_file("creds.txt");
    // Or inline: auto creds = tdx::Credentials("user@example.com", "your-password");
    auto client = tdx::Client::connect(creds, tdx::Config::production());

    // Fetch EOD stock data
    auto eod = client.stock_history_eod("AAPL", "20240101", "20240301");
    for (auto& tick : eod) {
        std::cout << tick.date << ": O=" << tick.open << std::endl;
    }

    // Snapshot: latest quote for multiple symbols
    auto quotes = client.stock_snapshot_quote({"AAPL", "MSFT", "GOOG"});
    for (auto& q : quotes) {
        std::cout << "bid=" << q.bid << " ask=" << q.ask << std::endl;
    }

    // Greeks (no server connection needed)
    auto g = tdx::all_greeks(450.0, 455.0, 0.05, 0.015, 30.0/365.0, 8.50, true);
    std::cout << "IV=" << g.iv << " Delta=" << g.delta << std::endl;
}
```

## API

### Credentials

- `Credentials::from_file(path)` - load from file (line 1 = email, line 2 = password)
- `Credentials::from_email(email, password)` - direct construction

### Config

- `Config::production()` - ThetaData NJ production servers
- `Config::dev()` - dev servers with shorter timeouts

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
| `stock_history_eod(sym, start, end)` | `vector<EodTick>` | EOD data |
| `stock_history_ohlc(sym, date, interval)` | `vector<OhlcTick>` | Intraday OHLC bars |
| `stock_history_ohlc_range(sym, start, end, interval)` | `vector<OhlcTick>` | OHLC bars across date range |
| `stock_history_trade(sym, date)` | `vector<TradeTick>` | All trades on a date |
| `stock_history_quote(sym, date, interval)` | `vector<QuoteTick>` | NBBO quotes |
| `stock_history_trade_quote(sym, date)` | `vector<TradeQuoteTick>` | Combined trade + quote ticks |

#### Stock - At-Time (2)

| Method | Returns | Description |
|--------|---------|-------------|
| `stock_at_time_trade(sym, start, end, time)` | `vector<TradeTick>` | Trade at a specific time across date range |
| `stock_at_time_quote(sym, start, end, time)` | `vector<QuoteTick>` | Quote at a specific time across date range |

#### Option - List (5)

| Method | Returns | Description |
|--------|---------|-------------|
| `option_list_symbols()` | `vector<string>` | All option underlyings |
| `option_list_dates(req, sym, exp, strike, right)` | `vector<string>` | Available dates for an option contract |
| `option_list_expirations(sym)` | `vector<string>` | Expiration dates |
| `option_list_strikes(sym, exp)` | `vector<string>` | Strike prices |
| `option_list_contracts(req, sym, date)` | `vector<OptionContract>` | All option contracts on a date |

#### Option - Snapshot (10)

| Method | Returns | Description |
|--------|---------|-------------|
| `option_snapshot_ohlc(sym, exp, strike, right)` | `vector<OhlcTick>` | Latest OHLC snapshot |
| `option_snapshot_trade(sym, exp, strike, right)` | `vector<TradeTick>` | Latest trade snapshot |
| `option_snapshot_quote(sym, exp, strike, right)` | `vector<QuoteTick>` | Latest quote snapshot |
| `option_snapshot_open_interest(sym, exp, strike, right)` | `vector<OpenInterestTick>` | Latest open interest snapshot |
| `option_snapshot_market_value(sym, exp, strike, right)` | `vector<MarketValueTick>` | Latest market value snapshot |
| `option_snapshot_greeks_implied_volatility(sym, exp, strike, right)` | `vector<IvTick>` | IV snapshot |
| `option_snapshot_greeks_all(sym, exp, strike, right)` | `vector<GreeksTick>` | All Greeks snapshot |
| `option_snapshot_greeks_first_order(sym, exp, strike, right)` | `vector<GreeksTick>` | First-order Greeks snapshot |
| `option_snapshot_greeks_second_order(sym, exp, strike, right)` | `vector<GreeksTick>` | Second-order Greeks snapshot |
| `option_snapshot_greeks_third_order(sym, exp, strike, right)` | `vector<GreeksTick>` | Third-order Greeks snapshot |

#### Option - History (6)

| Method | Returns | Description |
|--------|---------|-------------|
| `option_history_eod(sym, exp, strike, right, start, end)` | `vector<EodTick>` | EOD option data |
| `option_history_ohlc(sym, exp, strike, right, date, interval)` | `vector<OhlcTick>` | Intraday OHLC for options |
| `option_history_trade(sym, exp, strike, right, date)` | `vector<TradeTick>` | All trades for an option |
| `option_history_quote(sym, exp, strike, right, date, interval)` | `vector<QuoteTick>` | Quotes for an option |
| `option_history_trade_quote(sym, exp, strike, right, date)` | `vector<TradeQuoteTick>` | Combined trade + quote for an option |
| `option_history_open_interest(sym, exp, strike, right, date)` | `vector<OpenInterestTick>` | Open interest history |

#### Option - History Greeks (11)

| Method | Returns | Description |
|--------|---------|-------------|
| `option_history_greeks_eod(sym, exp, strike, right, start, end)` | `vector<GreeksTick>` | EOD Greeks history |
| `option_history_greeks_all(sym, exp, strike, right, date, interval)` | `vector<GreeksTick>` | All Greeks history (intraday) |
| `option_history_trade_greeks_all(sym, exp, strike, right, date)` | `vector<GreeksTick>` | All Greeks on each trade |
| `option_history_greeks_first_order(sym, exp, strike, right, date, interval)` | `vector<GreeksTick>` | First-order Greeks history |
| `option_history_trade_greeks_first_order(sym, exp, strike, right, date)` | `vector<GreeksTick>` | First-order Greeks on each trade |
| `option_history_greeks_second_order(sym, exp, strike, right, date, interval)` | `vector<GreeksTick>` | Second-order Greeks history |
| `option_history_trade_greeks_second_order(sym, exp, strike, right, date)` | `vector<GreeksTick>` | Second-order Greeks on each trade |
| `option_history_greeks_third_order(sym, exp, strike, right, date, interval)` | `vector<GreeksTick>` | Third-order Greeks history |
| `option_history_trade_greeks_third_order(sym, exp, strike, right, date)` | `vector<GreeksTick>` | Third-order Greeks on each trade |
| `option_history_greeks_implied_volatility(sym, exp, strike, right, date, interval)` | `vector<IvTick>` | IV history (intraday) |
| `option_history_trade_greeks_implied_volatility(sym, exp, strike, right, date)` | `vector<IvTick>` | IV on each trade |

#### Option - At-Time (2)

| Method | Returns | Description |
|--------|---------|-------------|
| `option_at_time_trade(sym, exp, strike, right, start, end, time)` | `vector<TradeTick>` | Trade at a specific time for an option |
| `option_at_time_quote(sym, exp, strike, right, start, end, time)` | `vector<QuoteTick>` | Quote at a specific time for an option |

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
| `index_history_eod(sym, start, end)` | `vector<EodTick>` | EOD index data |
| `index_history_ohlc(sym, start, end, interval)` | `vector<OhlcTick>` | Intraday OHLC for an index |
| `index_history_price(sym, date, interval)` | `vector<PriceTick>` | Intraday price history |

#### Index - At-Time (1)

| Method | Returns | Description |
|--------|---------|-------------|
| `index_at_time_price(sym, start, end, time)` | `vector<PriceTick>` | Index price at a specific time |

#### Calendar (3)

| Method | Returns | Description |
|--------|---------|-------------|
| `calendar_open_today()` | `vector<CalendarDay>` | Whether the market is open today |
| `calendar_on_date(date)` | `vector<CalendarDay>` | Calendar for a specific date |
| `calendar_year(year)` | `vector<CalendarDay>` | Calendar for an entire year |

#### Interest Rate (1)

| Method | Returns | Description |
|--------|---------|-------------|
| `interest_rate_history_eod(sym, start, end)` | `vector<InterestRateTick>` | EOD interest rate history |

### Standalone Functions

```cpp
// All 22 Greeks + IV
auto g = tdx::all_greeks(spot, strike, rate, div_yield, tte, price, is_call);
// g.iv, g.delta, g.gamma, g.theta, g.vega, g.rho, g.vanna, g.charm, etc.

// Just IV
auto [iv, err] = tdx::implied_volatility(spot, strike, rate, div_yield, tte, price, is_call);
```

### Tick Types

All endpoints return fully typed C++ structs. No raw JSON.

| Struct | Fields | Used by |
|--------|--------|---------|
| `EodTick` | ms_of_day, open, high, low, close, volume, count, bid, ask, date | EOD endpoints |
| `OhlcTick` | ms_of_day, open, high, low, close, volume, count, date | OHLC endpoints |
| `TradeTick` | ms_of_day, sequence, condition, size, exchange, price, price_raw, price_type, condition_flags, price_flags, volume_type, records_back, date | Trade endpoints |
| `QuoteTick` | ms_of_day, bid_size, bid_exchange, bid, bid_condition, ask_size, ask_exchange, ask, ask_condition, date | Quote endpoints |
| `TradeQuoteTick` | ms_of_day, sequence, ext_condition1-4, condition, size, exchange, price, condition_flags, price_flags, volume_type, records_back, quote_ms_of_day, bid_size, bid_exchange, bid, bid_condition, ask_size, ask_exchange, ask, ask_condition, date | Trade+quote endpoints |
| `OpenInterestTick` | ms_of_day, open_interest, date | Open interest endpoints |
| `GreeksTick` | ms_of_day, value, delta, gamma, theta, vega, rho, iv, iv_error, vanna, charm, vomma, veta, speed, zomma, color, ultima, d1, d2, dual_delta, dual_gamma, epsilon, lambda, date | Greeks snapshot/history |
| `IvTick` | ms_of_day, iv, iv_error, date | IV-only endpoints |
| `PriceTick` | ms_of_day, price, date | Index price endpoints |
| `MarketValueTick` | ms_of_day, market_cap, shares_outstanding, enterprise_value, book_value, free_float, date | Market value endpoints |
| `OptionContract` | root, expiration, strike, right | option_list_contracts |
| `CalendarDay` | date, is_open, open_time, close_time, status | Calendar endpoints |
| `InterestRateTick` | ms_of_day, rate, date | Interest rate endpoints |
| `Greeks` | value, delta, gamma, theta, vega, rho, iv, iv_error, vanna, charm, vomma, veta, speed, zomma, color, ultima, d1, d2, dual_delta, dual_gamma, epsilon, lambda | Standalone all_greeks() |

## FPSS Streaming

Real-time market data via ThetaData's FPSS servers. Streaming uses a **separate `FpssClient` class**, not methods on `Client`.

```cpp
#include "thetadx.hpp"
#include <iostream>

int main() {
    auto creds = tdx::Credentials::from_file("creds.txt");
    // Or inline: auto creds = tdx::Credentials("user@example.com", "your-password");
    auto config = tdx::Config::production();

    // Create a streaming client (separate from the historical Client)
    tdx::FpssClient fpss(creds, config);

    // Subscribe to real-time quotes
    int req_id = fpss.subscribe_quotes("AAPL");
    std::cout << "Subscribed (req_id=" << req_id << ")" << std::endl;

    // Poll for events (returns JSON string, empty on timeout)
    while (true) {
        std::string event = fpss.next_event(5000);  // 5s timeout
        if (event.empty()) continue;
        std::cout << "Event: " << event << std::endl;
    }

    fpss.shutdown();
}
```

### FpssClient API

| Method | Returns | Description |
|--------|---------|-------------|
| `FpssClient(creds, config)` | - | Connect to FPSS streaming servers |
| `subscribe_quotes(symbol)` | `int` | Subscribe to quote data for a stock symbol |
| `subscribe_trades(symbol)` | `int` | Subscribe to trade data for a stock symbol |
| `subscribe_open_interest(symbol)` | `int` | Subscribe to open interest data for a stock symbol |
| `subscribe_full_trades(sec_type)` | `int` | Subscribe to all trades for a security type (`"STOCK"`, `"OPTION"`, `"INDEX"`) |
| `unsubscribe_quotes(symbol)` | `int` | Unsubscribe from quote data |
| `unsubscribe_trades(symbol)` | `int` | Unsubscribe from trade data |
| `unsubscribe_open_interest(symbol)` | `int` | Unsubscribe from open interest data |
| `is_authenticated()` | `bool` | Check if the client is currently authenticated |
| `contract_lookup(id)` | `optional<string>` | Look up a contract by server-assigned ID |
| `active_subscriptions()` | `string` (JSON) | Get active subscriptions as a JSON array |
| `next_event(timeout_ms)` | `string` | Poll for the next event (empty string on timeout) |
| `shutdown()` | `void` | Shut down the FPSS client |

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
