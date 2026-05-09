# thetadatadx (C++)

C++ SDK for ThetaData market data. Header-only RAII wrappers over the `thetadatadx` Rust crate via the shared C FFI layer.

Every call crosses the C ABI boundary into compiled Rust: gRPC communication, protobuf parsing, zstd decompression, and TCP streaming run inside the `thetadatadx` crate.

> **Surface coverage:** the C++ binding exposes all three ThetaData surfaces — MDDS (historical), FPSS (streaming), and FLATFILES (whole-universe daily blobs). Flat files land via `unified.flat_files().*()` with `.to_arrow_ipc()` terminals plus a `flat_files().to_path(...)` raw-bytes helper — see the [Flat Files](#flat-files) section for the full method list.

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
// All 23 Greeks + IV. `right` accepts "C"/"P" or "call"/"put" (case-insensitive).
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
| `OptionContract` | symbol, expiration, strike, right | option_list_contracts |
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

    // Register a queued callback. The LMAX Disruptor consumer thread
    // invokes `fn` for every event under `catch_unwind`; the FPSS
    // reader thread never blocks on user code.
    fpss.set_callback([](const tdx::FpssEvent& event) {
        if (event.kind == TDX_FPSS_QUOTE) {
            std::cout << "quote bid=" << event.quote.bid
                      << " ask=" << event.quote.ask << std::endl;
        }
    });

    // Fluent contract-first subscriptions (primary surface).
    auto stock  = tdx::Contract::stock("AAPL");
    auto option = tdx::Contract::option("SPY", "20260620", "550", "C");

    unified.subscribe(stock.quote());
    unified.subscribe(option.trade());
    unified.subscribe(tdx::SecType::option().full_trades());

    // Bulk install:
    unified.subscribe_many({stock.quote(), option.quote()});

    // ... let the callback run ...

    fpss.shutdown();
}
```

All prices in streaming events are `double` (f64) -- decoded during parsing. Access them directly: `event.quote.bid`, `event.trade.price`, etc. No `price_type` decoding needed.

### Pull-iter delivery — `EventIterator` (high-throughput drain)

Push-callback (`fpss.set_callback(fn)` / `unified.set_callback(fn)`
above) is the recommended default for low-latency single-event
reaction. Pull-iter is the sibling delivery mode for high-throughput
batch processing: the user thread drains a per-client bounded queue
populated by the Disruptor consumer.

```cpp
#include "thetadx.hpp"
#include <chrono>
#include <iostream>

int main() {
    auto unified = tdx::UnifiedClient::connect(
        tdx::Credentials::from_file("creds.txt"),
        tdx::Config::production());

    auto iter = unified.start_streaming_iter();
    unified.subscribe(tdx::SecType::option().full_trades());

    // Range-for adapter — 1-second per-pop timeout by default.
    for (const auto& event : iter) {
        if (event.kind == TDX_FPSS_TRADE) {
            std::cout << "trade " << event.trade.price
                      << " x " << event.trade.size << std::endl;
        }
    }

    // Or explicit poll with caller-chosen deadline:
    while (auto event = iter.next(std::chrono::milliseconds(500))) {
        // ... process *event ...
    }
    if (iter.ended()) {
        // terminal end-of-stream — the streaming session shut down
        // and the queue is drained.
    }
}
```

`tdx::EventIterator` is move-only; the destructor frees the
underlying C handle. Mutually exclusive with the push-callback
methods on the same client; switch by stopping streaming and
starting again.

### Fluent contract-first API

| Method | Returns | Description |
|--------|---------|-------------|
| `tdx::Contract::stock(symbol)` | `Contract` | Stock contract |
| `tdx::Contract::option(symbol, exp, strike, right)` | `Contract` | Option contract |
| `contract.quote()` / `.trade()` / `.open_interest()` | `SubscriptionRef` | Per-contract subscription |
| `tdx::SecType::option().full_trades()` / `.full_open_interest()` | `SubscriptionRef` | Full-stream subscription |
| `unified.subscribe(sub)` | `void` | Install a subscription |
| `unified.subscribe_many({sub, ...})` | `void` | Install many subscriptions |
| `unified.unsubscribe(sub)` / `unsubscribe_many({...})` | `void` | Drop subscriptions |
| `fpss.subscribe(sub)` / `subscribe_many({...})` | `void` | Same shape on the standalone FpssClient |
| `fpss.unsubscribe(sub)` / `unsubscribe_many({...})` | `void` | Standalone-FpssClient drop |

### FpssClient lifecycle

| Method | Returns | Description |
|--------|---------|-------------|
| `FpssClient(creds, config)` | - | Connect to FPSS streaming servers |
| `is_authenticated()` | `bool` | Check if the client is currently authenticated |
| `active_subscriptions()` | `vector<Subscription>` | Get active subscriptions as typed structs |
| `set_callback(std::function<void(const FpssEvent&)>)` | `void` | Disruptor consumer thread invokes `fn` under `catch_unwind`; reader never blocks |
| `dropped_events()` | `uint64_t` | Cumulative ring-buffer overflow count (`Producer::try_publish` failures) |
| `reconnect()` | `void` | Reconnect streaming and restore subscriptions |
| `shutdown()` | `void` | Shut down the FPSS client |

### FPSS Event Types

| Type | Fields | Used when |
|------|--------|-----------|
| `TdxFpssQuote` | contract, ms_of_day, bid_size, bid_exchange, bid, bid_condition, ask_size, ask_exchange, ask, ask_condition, date, received_at_ns | `kind == TDX_FPSS_QUOTE` |
| `TdxFpssTrade` | contract, ms_of_day, sequence, ext_condition1-4, condition, size, exchange, price, condition_flags, price_flags, volume_type, records_back, date, received_at_ns | `kind == TDX_FPSS_TRADE` |
| `TdxFpssOpenInterest` | contract, ms_of_day, open_interest, date, received_at_ns | `kind == TDX_FPSS_OPEN_INTEREST` |
| `TdxFpssOhlcvc` | contract, ms_of_day, open, high, low, close, volume (i64), count (i64), date, received_at_ns | `kind == TDX_FPSS_OHLCVC` |
| `TdxFpssLoginSuccess` | permissions (nullable C string) | `kind == TDX_FPSS_LOGIN_SUCCESS` |
| `TdxFpssContractAssigned` | id (i32), contract (TdxContract) | `kind == TDX_FPSS_CONTRACT_ASSIGNED` |
| `TdxFpssReqResponse` | req_id (i32), result (i32) | `kind == TDX_FPSS_REQ_RESPONSE` |
| `TdxFpssMarketOpen` | (none) | `kind == TDX_FPSS_MARKET_OPEN` |
| `TdxFpssMarketClose` | (none) | `kind == TDX_FPSS_MARKET_CLOSE` |
| `TdxFpssServerError` | message (nullable C string) | `kind == TDX_FPSS_SERVER_ERROR` |
| `TdxFpssDisconnected` | reason (i32 RemoveReason) | `kind == TDX_FPSS_DISCONNECTED` |
| `TdxFpssReconnecting` | reason (i32), attempt (i32), delay_ms (u64) | `kind == TDX_FPSS_RECONNECTING` |
| `TdxFpssReconnected` | (none) | `kind == TDX_FPSS_RECONNECTED` |
| `TdxFpssError` | message (nullable C string) | `kind == TDX_FPSS_ERROR` |
| `TdxFpssUnknownFrame` | code (u8), payload (`uint8_t*`), payload_len (size_t) | `kind == TDX_FPSS_UNKNOWN_FRAME` |
| `TdxFpssConnected` | (none) | `kind == TDX_FPSS_CONNECTED` |
| `TdxFpssPing` | payload (`uint8_t*`), payload_len (size_t) | `kind == TDX_FPSS_PING` |
| `TdxFpssReconnectedServer` | (none) | `kind == TDX_FPSS_RECONNECTED_SERVER` |
| `TdxFpssRestart` | (none) | `kind == TDX_FPSS_RESTART` |
| `TdxFpssUnknownControl` | (none) | `kind == TDX_FPSS_UNKNOWN_CONTROL` |

Truncated / unrecognised wire frames are filtered before the user
callback fires and accounted on the `thetadatadx.fpss.decode_failures`
metric counter on the Rust side; they never surface through the C ABI
event stream.

`FpssClient` is non-copyable but movable. The destructor calls `shutdown()` automatically.

## Flat Files

Whole-universe daily snapshots over the legacy MDDS port. Decoded
schema is determined at runtime by `(SecType, ReqType)`, so the C++
wrapper exposes Arrow IPC stream bytes — pair with arrow-cpp on the
consumer side to materialise an `arrow::Table`.

```cpp
#include "thetadx.hpp"

auto creds = tdx::Credentials::from_file("creds.txt");
auto config = tdx::Config::production();
auto unified = tdx::UnifiedClient::connect(creds, config);

auto rows = unified.flat_files().option_quote("20260428");
auto ipc = rows.to_arrow_ipc();              // std::vector<uint8_t>

// Generic dispatcher
auto oi = unified.flat_files().request("OPTION", "OPEN_INTEREST", "20260428");

// Raw vendor CSV / JSONL straight to disk
unified.flat_files().to_path("OPTION", "QUOTE", "20260428",
                             "/tmp/option-quote", "csv");
```

Available `flat_files().*` methods: `option_quote`, `option_trade`,
`option_trade_quote`, `option_ohlc`, `option_open_interest`,
`option_eod`, `stock_quote`, `stock_trade`, `stock_trade_quote`,
`stock_eod`, plus `request(sec_type, req_type, date)` and
`to_path(...)`. The `tdx::UnifiedClient` wraps `TdxUnified`; the
existing `tdx::Client` (wrapping `TdxClient`) remains the
historical-only entry point.

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
