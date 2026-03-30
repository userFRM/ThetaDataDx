# API Reference

## ThetaDataDx

The unified client for all ThetaData access - historical data via MDDS/gRPC and real-time streaming via FPSS/TCP. Authenticates via Nexus, opens a gRPC channel, and exposes typed methods for every data endpoint. Streaming is started lazily via `start_streaming()`.

### Construction

```rust
pub async fn connect(creds: &Credentials, config: DirectConfig) -> Result<Self, Error>
```

1. Authenticates against the Nexus HTTP API to obtain a session UUID
2. Opens a gRPC channel (TLS) to the MDDS server

```rust
use thetadatadx::{ThetaDataDx, Credentials, DirectConfig};

let creds = Credentials::from_file("creds.txt")?;
let tdx = ThetaDataDx::connect(&creds, DirectConfig::production()).await?;
```

### Accessor Methods

| Method | Signature | Description |
|--------|-----------|-------------|
| `config()` | `&self -> &DirectConfig` | Return config snapshot |
| `session_uuid()` | `&self -> &str` | Return the Nexus session UUID |
| `channel()` | `&self -> &tonic::transport::Channel` | Access the underlying gRPC channel |
| `raw_query_info()` | `&self -> proto_v3::QueryInfo` | Get a QueryInfo for use with raw_query |

### Streaming Response Processing

```rust
pub async fn for_each_chunk<F>(
    &self,
    stream: tonic::Streaming<ResponseData>,
    mut callback: F,
) -> Result<(), Error>
where
    F: FnMut(proto::DataTable) -> Result<(), Error>,
```

Process gRPC response chunks one at a time via a callback, without materializing the entire response in memory. Each chunk is decompressed and decoded into a `DataTable` before being passed to the callback. Useful for large responses where holding all data in memory is undesirable.

```rust
tdx.for_each_chunk(stream, |table| {
    for tick in parse_trade_ticks(&table) {
        println!("{}: {}", tick.date, tick.get_price());
    }
    Ok(())
}).await?;
```

The standard `collect_stream` method now uses `original_size` from the `ResponseData` compression description as a pre-allocation hint for the decompression buffer, reducing intermediate reallocations.

**Empty streams**: When the gRPC stream contains no data chunks, `collect_stream` returns an empty `DataTable` (with headers, zero rows) rather than `Error::NoData`. Callers should check `.data_table.is_empty()` to detect the empty case. `Error::NoData` is reserved for cases where the endpoint genuinely has no usable data (e.g., a symbol that does not exist).

**Null values**: The `DataValue` protobuf oneof includes a `null_value` variant (bool). Null cells in the server response are preserved as `DataValue::NullValue(true)` rather than being silently dropped. The `extract_*_column` helper functions map null values to `None`.

### Stock - List (2)

```rust
pub async fn stock_list_symbols(&self) -> Result<Vec<String>, Error>
```

All available stock symbols. gRPC: `GetStockListSymbols`

```rust
pub async fn stock_list_dates(&self, request_type: &str, symbol: &str) -> Result<Vec<String>, Error>
```

Available dates for a stock by request type (e.g. `"EOD"`, `"TRADE"`, `"QUOTE"`). gRPC: `GetStockListDates`

### Stock - Snapshot (4)

```rust
pub async fn stock_snapshot_ohlc(&self, symbols: &[&str]) -> Result<Vec<OhlcTick>, Error>
```

Latest OHLC snapshot for one or more stocks. gRPC: `GetStockSnapshotOhlc`

```rust
pub async fn stock_snapshot_trade(&self, symbols: &[&str]) -> Result<Vec<TradeTick>, Error>
```

Latest trade snapshot for one or more stocks. gRPC: `GetStockSnapshotTrade`

```rust
pub async fn stock_snapshot_quote(&self, symbols: &[&str]) -> Result<Vec<QuoteTick>, Error>
```

Latest NBBO quote snapshot for one or more stocks. gRPC: `GetStockSnapshotQuote`

```rust
pub async fn stock_snapshot_market_value(&self, symbols: &[&str]) -> Result<Vec<MarketValueTick>, Error>
```

Latest market value snapshot for one or more stocks. gRPC: `GetStockSnapshotMarketValue`

### Stock - History (6)

```rust
pub async fn stock_history_eod(
    &self, symbol: &str, start: &str, end: &str
) -> Result<Vec<EodTick>, Error>
```

End-of-day stock data for a date range. Dates are `YYYYMMDD` strings. gRPC: `GetStockHistoryEod`

```rust
pub async fn stock_history_ohlc(
    &self, symbol: &str, date: &str, interval: &str
) -> Result<Vec<OhlcTick>, Error>
```

Intraday OHLC bars for a single date. `interval` is milliseconds (e.g., `"60000"` for 1-minute bars). gRPC: `GetStockHistoryOhlc`

```rust
pub async fn stock_history_ohlc_range(
    &self, symbol: &str, start_date: &str, end_date: &str, interval: &str
) -> Result<Vec<OhlcTick>, Error>
```

Intraday OHLC bars across a date range. Uses `start_date`/`end_date` instead of single `date`. gRPC: `GetStockHistoryOhlc`

```rust
pub async fn stock_history_trade(
    &self, symbol: &str, date: &str
) -> Result<Vec<TradeTick>, Error>
```

All trades for a stock on a given date. gRPC: `GetStockHistoryTrade`

```rust
pub async fn stock_history_quote(
    &self, symbol: &str, date: &str, interval: &str
) -> Result<Vec<QuoteTick>, Error>
```

NBBO quotes at a given interval. Use `"0"` for every quote change. gRPC: `GetStockHistoryQuote`

```rust
pub async fn stock_history_trade_quote(
    &self, symbol: &str, date: &str
) -> Result<Vec<TradeQuoteTick>, Error>
```

Combined trade + quote ticks. gRPC: `GetStockHistoryTradeQuote`

### Stock - AtTime (2)

```rust
pub async fn stock_at_time_trade(
    &self, symbol: &str, start_date: &str, end_date: &str, time_of_day: &str
) -> Result<Vec<TradeTick>, Error>
```

Trade at a specific time of day across a date range. `time_of_day` is milliseconds from midnight (e.g. `"34200000"` for 9:30 AM ET). gRPC: `GetStockAtTimeTrade`

```rust
pub async fn stock_at_time_quote(
    &self, symbol: &str, start_date: &str, end_date: &str, time_of_day: &str
) -> Result<Vec<QuoteTick>, Error>
```

Quote at a specific time of day across a date range. gRPC: `GetStockAtTimeQuote`

### Option - List (5)

```rust
pub async fn option_list_symbols(&self) -> Result<Vec<String>, Error>
```

All available option underlying symbols. gRPC: `GetOptionListSymbols`

```rust
pub async fn option_list_dates(
    &self, request_type: &str, symbol: &str, expiration: &str, strike: &str, right: &str
) -> Result<Vec<String>, Error>
```

Available dates for an option contract by request type. gRPC: `GetOptionListDates`

```rust
pub async fn option_list_expirations(&self, symbol: &str) -> Result<Vec<String>, Error>
```

Expiration dates for an underlying. Returns `YYYYMMDD` strings. gRPC: `GetOptionListExpirations`

```rust
pub async fn option_list_strikes(
    &self, symbol: &str, expiration: &str
) -> Result<Vec<String>, Error>
```

Strike prices for a given expiration. gRPC: `GetOptionListStrikes`

```rust
pub async fn option_list_contracts(
    &self, request_type: &str, symbol: &str, date: &str
) -> Result<Vec<OptionContract>, Error>
```

All option contracts for a symbol on a given date. gRPC: `GetOptionListContracts`

### Option - Snapshot (5)

```rust
pub async fn option_snapshot_ohlc(
    &self, symbol: &str, expiration: &str, strike: &str, right: &str
) -> Result<Vec<OhlcTick>, Error>
```

Latest OHLC snapshot for option contracts. gRPC: `GetOptionSnapshotOhlc`

```rust
pub async fn option_snapshot_trade(
    &self, symbol: &str, expiration: &str, strike: &str, right: &str
) -> Result<Vec<TradeTick>, Error>
```

Latest trade snapshot for option contracts. gRPC: `GetOptionSnapshotTrade`

```rust
pub async fn option_snapshot_quote(
    &self, symbol: &str, expiration: &str, strike: &str, right: &str
) -> Result<Vec<QuoteTick>, Error>
```

Latest NBBO quote snapshot for option contracts. gRPC: `GetOptionSnapshotQuote`

```rust
pub async fn option_snapshot_open_interest(
    &self, symbol: &str, expiration: &str, strike: &str, right: &str
) -> Result<Vec<OpenInterestTick>, Error>
```

Latest open interest snapshot for option contracts. gRPC: `GetOptionSnapshotOpenInterest`

```rust
pub async fn option_snapshot_market_value(
    &self, symbol: &str, expiration: &str, strike: &str, right: &str
) -> Result<Vec<MarketValueTick>, Error>
```

Latest market value snapshot for option contracts. gRPC: `GetOptionSnapshotMarketValue`

### Option - Snapshot Greeks (5)

```rust
pub async fn option_snapshot_greeks_implied_volatility(
    &self, symbol: &str, expiration: &str, strike: &str, right: &str
) -> Result<Vec<IvTick>, Error>
```

Implied volatility snapshot. gRPC: `GetOptionSnapshotGreeksImpliedVolatility`

```rust
pub async fn option_snapshot_greeks_all(
    &self, symbol: &str, expiration: &str, strike: &str, right: &str
) -> Result<Vec<GreeksTick>, Error>
```

All Greeks snapshot. gRPC: `GetOptionSnapshotGreeksAll`

```rust
pub async fn option_snapshot_greeks_first_order(
    &self, symbol: &str, expiration: &str, strike: &str, right: &str
) -> Result<Vec<GreeksTick>, Error>
```

First-order Greeks snapshot (delta, theta, rho, etc.). gRPC: `GetOptionSnapshotGreeksFirstOrder`

```rust
pub async fn option_snapshot_greeks_second_order(
    &self, symbol: &str, expiration: &str, strike: &str, right: &str
) -> Result<Vec<GreeksTick>, Error>
```

Second-order Greeks snapshot (gamma, vanna, charm, etc.). gRPC: `GetOptionSnapshotGreeksSecondOrder`

```rust
pub async fn option_snapshot_greeks_third_order(
    &self, symbol: &str, expiration: &str, strike: &str, right: &str
) -> Result<Vec<GreeksTick>, Error>
```

Third-order Greeks snapshot (speed, color, ultima, etc.). gRPC: `GetOptionSnapshotGreeksThirdOrder`

### Option - History (6)

```rust
pub async fn option_history_eod(
    &self, symbol: &str, expiration: &str, strike: &str, right: &str,
    start: &str, end: &str
) -> Result<Vec<EodTick>, Error>
```

End-of-day option data. `right` is `"C"` or `"P"`. gRPC: `GetOptionHistoryEod`

```rust
pub async fn option_history_ohlc(
    &self, symbol: &str, expiration: &str, strike: &str, right: &str,
    date: &str, interval: &str
) -> Result<Vec<OhlcTick>, Error>
```

Intraday option OHLC bars. gRPC: `GetOptionHistoryOhlc`

```rust
pub async fn option_history_trade(
    &self, symbol: &str, expiration: &str, strike: &str, right: &str, date: &str
) -> Result<Vec<TradeTick>, Error>
```

Option trades on a given date. gRPC: `GetOptionHistoryTrade`

```rust
pub async fn option_history_quote(
    &self, symbol: &str, expiration: &str, strike: &str, right: &str,
    date: &str, interval: &str
) -> Result<Vec<QuoteTick>, Error>
```

Option NBBO quotes. gRPC: `GetOptionHistoryQuote`

```rust
pub async fn option_history_trade_quote(
    &self, symbol: &str, expiration: &str, strike: &str, right: &str, date: &str
) -> Result<Vec<TradeQuoteTick>, Error>
```

Combined trade + quote ticks for an option contract. gRPC: `GetOptionHistoryTradeQuote`

```rust
pub async fn option_history_open_interest(
    &self, symbol: &str, expiration: &str, strike: &str, right: &str, date: &str
) -> Result<Vec<OpenInterestTick>, Error>
```

Open interest history for an option contract. gRPC: `GetOptionHistoryOpenInterest`

### Option - History Greeks (6)

```rust
pub async fn option_history_greeks_eod(
    &self, symbol: &str, expiration: &str, strike: &str, right: &str,
    start_date: &str, end_date: &str
) -> Result<Vec<GreeksTick>, Error>
```

EOD Greeks history for an option contract. gRPC: `GetOptionHistoryGreeksEod`

```rust
pub async fn option_history_greeks_all(
    &self, symbol: &str, expiration: &str, strike: &str, right: &str,
    date: &str, interval: &str
) -> Result<Vec<GreeksTick>, Error>
```

All Greeks history (intraday, sampled by interval). gRPC: `GetOptionHistoryGreeksAll`

```rust
pub async fn option_history_greeks_first_order(
    &self, symbol: &str, expiration: &str, strike: &str, right: &str,
    date: &str, interval: &str
) -> Result<Vec<GreeksTick>, Error>
```

First-order Greeks history (intraday, sampled by interval). gRPC: `GetOptionHistoryGreeksFirstOrder`

```rust
pub async fn option_history_greeks_second_order(
    &self, symbol: &str, expiration: &str, strike: &str, right: &str,
    date: &str, interval: &str
) -> Result<Vec<GreeksTick>, Error>
```

Second-order Greeks history (intraday, sampled by interval). gRPC: `GetOptionHistoryGreeksSecondOrder`

```rust
pub async fn option_history_greeks_third_order(
    &self, symbol: &str, expiration: &str, strike: &str, right: &str,
    date: &str, interval: &str
) -> Result<Vec<GreeksTick>, Error>
```

Third-order Greeks history (intraday, sampled by interval). gRPC: `GetOptionHistoryGreeksThirdOrder`

```rust
pub async fn option_history_greeks_implied_volatility(
    &self, symbol: &str, expiration: &str, strike: &str, right: &str,
    date: &str, interval: &str
) -> Result<Vec<IvTick>, Error>
```

Implied volatility history (intraday, sampled by interval). gRPC: `GetOptionHistoryGreeksImpliedVolatility`

### Option - History Trade Greeks (5)

```rust
pub async fn option_history_trade_greeks_all(
    &self, symbol: &str, expiration: &str, strike: &str, right: &str, date: &str
) -> Result<Vec<GreeksTick>, Error>
```

All Greeks computed on each trade. gRPC: `GetOptionHistoryTradeGreeksAll`

```rust
pub async fn option_history_trade_greeks_first_order(
    &self, symbol: &str, expiration: &str, strike: &str, right: &str, date: &str
) -> Result<Vec<GreeksTick>, Error>
```

First-order Greeks on each trade. gRPC: `GetOptionHistoryTradeGreeksFirstOrder`

```rust
pub async fn option_history_trade_greeks_second_order(
    &self, symbol: &str, expiration: &str, strike: &str, right: &str, date: &str
) -> Result<Vec<GreeksTick>, Error>
```

Second-order Greeks on each trade. gRPC: `GetOptionHistoryTradeGreeksSecondOrder`

```rust
pub async fn option_history_trade_greeks_third_order(
    &self, symbol: &str, expiration: &str, strike: &str, right: &str, date: &str
) -> Result<Vec<GreeksTick>, Error>
```

Third-order Greeks on each trade. gRPC: `GetOptionHistoryTradeGreeksThirdOrder`

```rust
pub async fn option_history_trade_greeks_implied_volatility(
    &self, symbol: &str, expiration: &str, strike: &str, right: &str, date: &str
) -> Result<Vec<IvTick>, Error>
```

Implied volatility on each trade. gRPC: `GetOptionHistoryTradeGreeksImpliedVolatility`

### Option - AtTime (2)

```rust
pub async fn option_at_time_trade(
    &self, symbol: &str, expiration: &str, strike: &str, right: &str,
    start_date: &str, end_date: &str, time_of_day: &str
) -> Result<Vec<TradeTick>, Error>
```

Trade at a specific time of day across a date range for an option. gRPC: `GetOptionAtTimeTrade`

```rust
pub async fn option_at_time_quote(
    &self, symbol: &str, expiration: &str, strike: &str, right: &str,
    start_date: &str, end_date: &str, time_of_day: &str
) -> Result<Vec<QuoteTick>, Error>
```

Quote at a specific time of day across a date range for an option. gRPC: `GetOptionAtTimeQuote`

### Index - List (2)

```rust
pub async fn index_list_symbols(&self) -> Result<Vec<String>, Error>
```

All available index symbols. gRPC: `GetIndexListSymbols`

```rust
pub async fn index_list_dates(&self, symbol: &str) -> Result<Vec<String>, Error>
```

Available dates for an index symbol. gRPC: `GetIndexListDates`

### Index - Snapshot (3)

```rust
pub async fn index_snapshot_ohlc(&self, symbols: &[&str]) -> Result<Vec<OhlcTick>, Error>
```

Latest OHLC snapshot for one or more indices. gRPC: `GetIndexSnapshotOhlc`

```rust
pub async fn index_snapshot_price(&self, symbols: &[&str]) -> Result<Vec<PriceTick>, Error>
```

Latest price snapshot for one or more indices. gRPC: `GetIndexSnapshotPrice`

```rust
pub async fn index_snapshot_market_value(&self, symbols: &[&str]) -> Result<Vec<MarketValueTick>, Error>
```

Latest market value snapshot for one or more indices. gRPC: `GetIndexSnapshotMarketValue`

### Index - History (3)

```rust
pub async fn index_history_eod(
    &self, symbol: &str, start: &str, end: &str
) -> Result<Vec<EodTick>, Error>
```

End-of-day index data for a date range. gRPC: `GetIndexHistoryEod`

```rust
pub async fn index_history_ohlc(
    &self, symbol: &str, start_date: &str, end_date: &str, interval: &str
) -> Result<Vec<OhlcTick>, Error>
```

Intraday OHLC bars for an index. gRPC: `GetIndexHistoryOhlc`

```rust
pub async fn index_history_price(
    &self, symbol: &str, date: &str, interval: &str
) -> Result<Vec<PriceTick>, Error>
```

Intraday price history for an index. gRPC: `GetIndexHistoryPrice`

### Index - AtTime (1)

```rust
pub async fn index_at_time_price(
    &self, symbol: &str, start_date: &str, end_date: &str, time_of_day: &str
) -> Result<Vec<PriceTick>, Error>
```

Index price at a specific time of day across a date range. gRPC: `GetIndexAtTimePrice`

### Interest Rate (1)

```rust
pub async fn interest_rate_history_eod(
    &self, symbol: &str, start_date: &str, end_date: &str
) -> Result<Vec<InterestRateTick>, Error>
```

End-of-day interest rate history. gRPC: `GetInterestRateHistoryEod`

### Calendar (3)

```rust
pub async fn calendar_open_today(&self) -> Result<Vec<CalendarDay>, Error>
```

Whether the market is open today. gRPC: `GetCalendarOpenToday`

```rust
pub async fn calendar_on_date(&self, date: &str) -> Result<Vec<CalendarDay>, Error>
```

Calendar information for a specific date. gRPC: `GetCalendarOnDate`

```rust
pub async fn calendar_year(&self, year: &str) -> Result<Vec<CalendarDay>, Error>
```

Calendar information for an entire year. `year` is a 4-digit string (e.g. `"2024"`). gRPC: `GetCalendarYear`

### Raw Query

Escape hatch for endpoints not yet wrapped by typed methods:

```rust
pub async fn raw_query<F, Fut>(&self, call: F) -> Result<proto::DataTable, Error>
where
    F: FnOnce(BetaThetaTerminalClient<Channel>) -> Fut,
    Fut: Future<Output = Result<Streaming<ResponseData>, Error>>,
```

Example:

```rust
use thetadatadx::proto_v3;

let request = proto_v3::CalendarYearRequest {
    query_info: Some(tdx.raw_query_info()),
    params: Some(proto_v3::CalendarYearRequestQuery {
        year: "2024".to_string(),
    }),
};

let table = tdx.raw_query(|mut stub| {
    Box::pin(async move {
        Ok(stub.get_calendar_year(request).await?.into_inner())
    })
}).await?;
```

### Streaming `_stream` Endpoint Variants

These variants process gRPC response chunks via callback without materializing the full response in memory. Ideal for endpoints returning millions of rows.

```rust
pub async fn stock_history_trade_stream<F>(
    &self, symbol: &str, date: &str, handler: F,
) -> Result<(), Error>
where F: FnMut(Vec<TradeTick>) -> Result<(), Error>
```

Process all trades for a stock on a given date, one chunk at a time. gRPC: `GetStockHistoryTrade`

```rust
pub async fn stock_history_quote_stream<F>(
    &self, symbol: &str, date: &str, interval: &str, handler: F,
) -> Result<(), Error>
where F: FnMut(Vec<QuoteTick>) -> Result<(), Error>
```

Process quotes for a stock, one chunk at a time. gRPC: `GetStockHistoryQuote`

```rust
pub async fn option_history_trade_stream<F>(
    &self, symbol: &str, expiration: &str, strike: &str, right: &str, date: &str, handler: F,
) -> Result<(), Error>
where F: FnMut(Vec<TradeTick>) -> Result<(), Error>
```

Process all trades for an option contract, one chunk at a time. gRPC: `GetOptionHistoryTrade`

```rust
pub async fn option_history_quote_stream<F>(
    &self, symbol: &str, expiration: &str, strike: &str, right: &str,
    date: &str, interval: &str, handler: F,
) -> Result<(), Error>
where F: FnMut(Vec<QuoteTick>) -> Result<(), Error>
```

Process quotes for an option contract, one chunk at a time. gRPC: `GetOptionHistoryQuote`

### Auth Error Behavior

Nexus HTTP responses with status 401 (Unauthorized) or 404 (Not Found) are treated as `Error::Auth("invalid credentials (server returned 401/404)")`, matching the Java terminal's special-casing of these status codes. Other HTTP errors surface as `Error::Http`.

### Endpoint Count

ThetaDataDx exposes **61 typed methods** (plus 4 `_stream` variants) covering all 60 gRPC RPCs in `BetaThetaTerminal` plus 1 convenience range-query variant (`stock_history_ohlc_range`). Historical methods are provided via `Deref<Target = DirectClient>` (an internal implementation detail) and generated by the `define_endpoint!` macro in `direct.rs`.

### FFI Coverage

All 61 endpoints are exposed through the `thetadatadx-ffi` C ABI crate. Each method has a corresponding `extern "C"` function (e.g., `thetadatadx_stock_history_eod`). The Go and C++ SDKs wrap these FFI functions 1:1.

### Python SDK Coverage

All 61 endpoints are available in the Python SDK via PyO3 bindings (e.g., `tdx.stock_history_eod(...)`). Streaming is available via `tdx.start_streaming()` / `tdx.next_event()`. DataFrame conversion is available via `to_dataframe()` and `_df` method variants (requires `pip install thetadatadx[pandas]`).

### Python SDK: Streaming

```python
from thetadatadx import Credentials, Config, ThetaDataDx

creds = Credentials.from_file("creds.txt")
tdx = ThetaDataDx(creds, Config.production())

tdx.start_streaming()
tdx.subscribe_quotes("AAPL")
while True:
    event = tdx.next_event(timeout_ms=5000)
    if event is None:
        break
    print(event)

tdx.stop_streaming()
```

### Python SDK: pandas DataFrame Conversion

```python
from thetadatadx import Credentials, Config, ThetaDataDx, to_dataframe

creds = Credentials.from_file("creds.txt")
tdx = ThetaDataDx(creds, Config.production())

# Convert any result to a DataFrame
eod = tdx.stock_history_eod("AAPL", "20240101", "20240301")
df = to_dataframe(eod)

# Or use the _df convenience methods directly
df = tdx.stock_history_eod_df("AAPL", "20240101", "20240301")
```

Install with pandas support: `pip install thetadatadx[pandas]`

### FFI FPSS Functions

7 `extern "C"` functions for FPSS lifecycle management:

| Function | Signature | Description |
|----------|-----------|-------------|
| `thetadatadx_fpss_connect` | `(creds, buffer_size) -> *mut FpssHandle` | Connect and authenticate |
| `thetadatadx_fpss_subscribe_quotes` | `(handle, root, sec_type) -> i32` | Subscribe to quotes |
| `thetadatadx_fpss_subscribe_trades` | `(handle, root, sec_type) -> i32` | Subscribe to trades |
| `thetadatadx_fpss_subscribe_open_interest` | `(handle, root, sec_type) -> i32` | Subscribe to OI |
| `thetadatadx_fpss_next_event` | `(handle, timeout_ms) -> *mut FpssEventC` | Poll next event |
| `thetadatadx_fpss_shutdown` | `(handle) -> i32` | Graceful shutdown |
| `thetadatadx_fpss_free_event` | `(event) -> void` | Free an event |

### Go SDK: Streaming

```go
tdx, _ := thetadatadx.Connect(creds, config)
defer tdx.Close()

tdx.StartStreaming(1024)
tdx.SubscribeQuotes("AAPL", thetadatadx.SecTypeStock)
for {
    event, _ := tdx.NextEvent(5000)
    if event == nil {
        break
    }
    fmt.Println(event)
}
tdx.StopStreaming()
```

### C++ SDK: Streaming

```cpp
auto tdx = tdx::Client::connect(creds, tdx::Config::production());

tdx.start_streaming(1024);
tdx.subscribe_quotes("AAPL", tdx::SecType::Stock);
while (auto event = tdx.next_event(5000)) {
    std::cout << event->type() << std::endl;
}

tdx.stop_streaming();
```

---

## Streaming (FPSS)

Real-time streaming is accessed through `ThetaDataDx`. The streaming connection is established lazily via `start_streaming()`.

### Starting the Stream

```rust
pub fn start_streaming(&self, callback: impl Fn(&FpssEvent) + Send + 'static) -> Result<(), Error>
```

Establishes TLS connection, authenticates, starts background reader and heartbeat tasks.

```rust
tdx.start_streaming(|event: &FpssEvent| {
    // handle events
})?;
```

### Subscription Methods

| Method | Signature | Description |
|--------|-----------|-------------|
| `subscribe_quotes` | `(&self, &Contract) -> Result<i32, Error>` | Subscribe to quote data |
| `subscribe_trades` | `(&self, &Contract) -> Result<i32, Error>` | Subscribe to trade data |
| `subscribe_open_interest` | `(&self, &Contract) -> Result<i32, Error>` | Subscribe to open interest |
| `subscribe_full_trades` | `(&self, SecType) -> Result<i32, Error>` | Subscribe to all trades for a security type |
| `unsubscribe_quotes` | `(&self, &Contract) -> Result<i32, Error>` | Unsubscribe quotes |
| `unsubscribe_trades` | `(&self, &Contract) -> Result<i32, Error>` | Unsubscribe trades |
| `unsubscribe_open_interest` | `(&self, &Contract) -> Result<i32, Error>` | Unsubscribe open interest |

All subscription methods return the request ID. The server confirms via a `ReqResponse` event.

### State Methods

| Method | Signature | Description |
|--------|-----------|-------------|
| `is_streaming` | `(&self) -> bool` | Check if streaming connection is live |
| `server_addr` | `(&self) -> &str` | Get connected server address |
| `contract_map` | `(&self) -> HashMap<i32, Contract>` | Server-assigned contract IDs |
| `stop_streaming` | `(&self)` | Send STOP and shut down streaming |

### FpssEvent

Events received through the ring buffer. `FpssEvent` is a 3-variant wrapper around `FpssData` (market data), `FpssControl` (lifecycle), and `RawData` (unparsed frames):

```rust
pub enum FpssEvent {
    /// Market data events — quote, trade, open interest, OHLCVC.
    Data(FpssData),
    /// Lifecycle events — login, disconnect, market open/close, errors.
    Control(FpssControl),
    /// Unparsed frames (unknown message codes).
    RawData { code: u8, payload: Vec<u8> },
}

pub enum FpssData {
    Quote { contract_id: i32, ms_of_day: i32, bid_size: i32, bid_exchange: i32,
            bid: i32, bid_condition: i32, ask_size: i32, ask_exchange: i32,
            ask: i32, ask_condition: i32, price_type: i32, date: i32 },
    Trade { contract_id: i32, ms_of_day: i32, sequence: i32,
            ext_condition1: i32, ext_condition2: i32, ext_condition3: i32,
            ext_condition4: i32, condition: i32, size: i32, exchange: i32,
            price: i32, condition_flags: i32, price_flags: i32,
            volume_type: i32, records_back: i32, price_type: i32, date: i32 },
    OpenInterest { contract_id: i32, ms_of_day: i32, open_interest: i32, date: i32 },
    Ohlcvc { contract_id: i32, ms_of_day: i32, open: i32, high: i32, low: i32,
             close: i32, volume: i32, count: i32, price_type: i32, date: i32 },
}

pub enum FpssControl {
    LoginSuccess { permissions: String },
    ContractAssigned { id: i32, contract: Contract },
    ReqResponse { req_id: i32, result: StreamResponseType },
    MarketOpen,
    MarketClose,
    ServerError { message: String },
    Disconnected { reason: RemoveReason },
    Error { message: String },
}
```

**Migration from v2.x**: Replace `FpssClient::connect()` with `tdx.start_streaming(handler)`. Replace `fpss.subscribe_quotes(...)` with `tdx.subscribe_quotes(...)`. Replace `fpss.shutdown()` with `tdx.stop_streaming()`.

### OhlcvcAccumulator

OHLCVC bars are derived from trade ticks via the internal `OhlcvcAccumulator`. The accumulator is per-contract and only begins emitting `FpssData::Ohlcvc` events after receiving a server-seeded initial OHLCVC bar. Subsequent trades update the bar's open/high/low/close/volume/count fields incrementally. This matches the Java terminal's behavior.

### Reconnection

```rust
pub fn reconnect_delay(reason: RemoveReason) -> Option<u64>
```

Returns `None` for permanent credential/account errors (`InvalidCredentials`, `InvalidLoginValues`, `InvalidLoginSize`, `AccountAlreadyConnected`, `FreeAccount`, `ServerUserDoesNotExist`, `InvalidCredentialsNullUser`), `Some(130_000)` for `TooManyRequests`, `Some(2_000)` for everything else.

### Contract

```rust
pub struct Contract {
    pub root: String,
    pub sec_type: SecType,
    pub exp_date: Option<i32>,
    pub is_call: Option<bool>,
    pub strike: Option<i32>,
}
```

Constructors:

```rust
Contract::stock("AAPL")
Contract::index("SPX")
Contract::rate("SOFR")
Contract::option("SPY", 20261218, true, 60000)  // call, strike 60000
```

Serialization:

```rust
let bytes = contract.to_bytes();                    // serialize for wire
let (contract, consumed) = Contract::from_bytes(&bytes)?;  // deserialize
```

---

## Tick Types

All 14 tick types are `Clone + Debug` structs generated from `endpoint_schema.toml`. Most are also `Copy` (except `OptionContract`, which contains a `String` field). Fields are typically `i32`, with `i64` for large values (e.g., `MarketValueTick.market_cap`), `f64` for Greeks/IV, and `String` for identifiers. Prices are stored in fixed-point encoding. Use the `*_price()` methods to get `Price` values with proper decimal handling.

### TradeTick

16 fields representing a single trade.

```rust
pub struct TradeTick {
    pub ms_of_day: i32,        // Milliseconds since midnight ET
    pub sequence: i32,          // Sequence number
    pub ext_condition1: i32,    // Extended condition code 1
    pub ext_condition2: i32,    // Extended condition code 2
    pub ext_condition3: i32,    // Extended condition code 3
    pub ext_condition4: i32,    // Extended condition code 4
    pub condition: i32,         // Trade condition code
    pub size: i32,              // Trade size (shares)
    pub exchange: i32,          // Exchange code
    pub price: i32,             // Price (fixed-point, use get_price())
    pub condition_flags: i32,   // Condition flags bitmap
    pub price_flags: i32,       // Price flags bitmap
    pub volume_type: i32,       // 0 = incremental, 1 = cumulative
    pub records_back: i32,      // Records back count
    pub price_type: i32,        // Decimal type for price decoding
    pub date: i32,              // Date as YYYYMMDD integer
}
```

Methods:

| Method | Return | Description |
|--------|--------|-------------|
| `get_price()` | `Price` | Trade price with decimal handling |
| `is_cancelled()` | `bool` | Condition code 40-44 |
| `trade_condition_no_last()` | `bool` | Condition flags bit 0 |
| `price_condition_set_last()` | `bool` | Price flags bit 0 |
| `is_incremental_volume()` | `bool` | volume_type == 0 |
| `regular_trading_hours()` | `bool` | 9:30 AM - 4:00 PM ET |
| `is_seller()` | `bool` | ext_condition1 == 12 |

### QuoteTick

11 fields representing an NBBO quote.

```rust
pub struct QuoteTick {
    pub ms_of_day: i32,
    pub bid_size: i32,
    pub bid_exchange: i32,
    pub bid: i32,
    pub bid_condition: i32,
    pub ask_size: i32,
    pub ask_exchange: i32,
    pub ask: i32,
    pub ask_condition: i32,
    pub price_type: i32,
    pub date: i32,
}
```

Methods:

| Method | Return | Description |
|--------|--------|-------------|
| `bid_price()` | `Price` | Bid price with decimal handling |
| `ask_price()` | `Price` | Ask price with decimal handling |
| `midpoint_value()` | `i32` | Integer midpoint (bid + ask) / 2 |
| `midpoint_price()` | `Price` | Midpoint as Price |

### OhlcTick

9 fields representing an aggregated bar.

```rust
pub struct OhlcTick {
    pub ms_of_day: i32,
    pub open: i32,
    pub high: i32,
    pub low: i32,
    pub close: i32,
    pub volume: i32,
    pub count: i32,
    pub price_type: i32,
    pub date: i32,
}
```

Methods: `open_price()`, `high_price()`, `low_price()`, `close_price()` - all return `Price`.

### EodTick

18 fields - full end-of-day snapshot with OHLC + quote data.

```rust
pub struct EodTick {
    pub ms_of_day: i32,
    pub ms_of_day2: i32,
    pub open: i32,
    pub high: i32,
    pub low: i32,
    pub close: i32,
    pub volume: i32,
    pub count: i32,
    pub bid_size: i32,
    pub bid_exchange: i32,
    pub bid: i32,
    pub bid_condition: i32,
    pub ask_size: i32,
    pub ask_exchange: i32,
    pub ask: i32,
    pub ask_condition: i32,
    pub price_type: i32,
    pub date: i32,
}
```

Methods: `open_price()`, `high_price()`, `low_price()`, `close_price()`, `bid_price()`, `ask_price()`, `midpoint_value()` - all operate on the shared `price_type`.

### OpenInterestTick

```rust
pub struct OpenInterestTick {
    pub ms_of_day: i32,
    pub open_interest: i32,
    pub date: i32,
}
```

### SnapshotTradeTick

7-field abbreviated trade for snapshots.

```rust
pub struct SnapshotTradeTick {
    pub ms_of_day: i32,
    pub sequence: i32,
    pub size: i32,
    pub condition: i32,
    pub price: i32,
    pub price_type: i32,
    pub date: i32,
}
```

Methods: `get_price() -> Price`.

### TradeQuoteTick

26-field combined trade + quote tick.

```rust
pub struct TradeQuoteTick {
    // Trade portion (14 fields)
    pub ms_of_day: i32,
    pub sequence: i32,
    pub ext_condition1: i32,
    pub ext_condition2: i32,
    pub ext_condition3: i32,
    pub ext_condition4: i32,
    pub condition: i32,
    pub size: i32,
    pub exchange: i32,
    pub price: i32,
    pub condition_flags: i32,
    pub price_flags: i32,
    pub volume_type: i32,
    pub records_back: i32,
    // Quote portion (10 fields)
    pub quote_ms_of_day: i32,
    pub bid_size: i32,
    pub bid_exchange: i32,
    pub bid: i32,
    pub bid_condition: i32,
    pub ask_size: i32,
    pub ask_exchange: i32,
    pub ask: i32,
    pub ask_condition: i32,
    pub quote_price_type: i32,
    // Shared
    pub price_type: i32,
    pub date: i32,
}
```

Methods: `trade_price()`, `bid_price()`, `ask_price()` - all return `Price`.

### MarketValueTick

7 fields - market capitalization and related data.

```rust
pub struct MarketValueTick {
    pub ms_of_day: i32,
    pub market_cap: i64,
    pub shares_outstanding: i64,
    pub enterprise_value: i64,
    pub book_value: i64,
    pub free_float: i64,
    pub date: i32,
}
```

### GreeksTick

24 fields - full set of option Greeks.

```rust
pub struct GreeksTick {
    pub ms_of_day: i32,
    pub implied_volatility: f64,
    pub delta: f64,
    pub gamma: f64,
    pub theta: f64,
    pub vega: f64,
    pub rho: f64,
    pub iv_error: f64,
    pub vanna: f64,
    pub charm: f64,
    pub vomma: f64,
    pub veta: f64,
    pub speed: f64,
    pub zomma: f64,
    pub color: f64,
    pub ultima: f64,
    pub d1: f64,
    pub d2: f64,
    pub dual_delta: f64,
    pub dual_gamma: f64,
    pub epsilon: f64,
    pub lambda: f64,
    pub vera: f64,
    pub date: i32,
}
```

### IvTick

4 fields - implied volatility.

```rust
pub struct IvTick {
    pub ms_of_day: i32,
    pub implied_volatility: f64,
    pub iv_error: f64,
    pub date: i32,
}
```

### PriceTick

4 fields - generic price data point.

```rust
pub struct PriceTick {
    pub ms_of_day: i32,
    pub price: i32,
    pub price_type: i32,
    pub date: i32,
}
```

Methods: `get_price() -> Price`.

### CalendarDay

5 fields - market open/close schedule.

```rust
pub struct CalendarDay {
    pub date: i32,
    pub is_open: i32,
    pub open_time: i32,
    pub close_time: i32,
    pub status: i32,
}
```

### InterestRateTick

3 fields - end-of-day interest rate.

```rust
pub struct InterestRateTick {
    pub ms_of_day: i32,
    pub rate: f64,
    pub date: i32,
}
```

### OptionContract

5 fields - option contract specification. Not `Copy` due to `String` root field.

```rust
pub struct OptionContract {
    pub root: String,
    pub expiration: i32,
    pub strike: i32,
    pub right: i32,
    pub strike_price_type: i32,
}
```

---

## Price

Fixed-point price with variable decimal precision.

```rust
pub struct Price {
    pub value: i32,
    pub price_type: i32,
}
```

The real price is `value * 10^(price_type - 10)`.

### Construction

```rust
Price::new(15025, 8)    // 150.25
Price::new(100, 10)     // 100.0
Price::ZERO             // 0.0
Price::from_proto(&proto_price)
```

### Methods

| Method | Return | Description |
|--------|--------|-------------|
| `to_f64()` | `f64` | Lossy float conversion |
| `is_zero()` | `bool` | True if value == 0 or price_type == 0 |
| `to_proto()` | `proto::Price` | Convert to protobuf |

### Traits

- `Display`: Formats with correct decimal places (`"150.25"`, `"0.005"`, `"500.0"`)
- `Debug`: `Price(150.25)`
- `Eq, Ord, PartialEq, PartialOrd`: Compares across different price_type values by normalizing to a common base
- `Copy, Clone, Default`

### Price Type Table

| price_type | Formula | Example |
|------------|---------|---------|
| 0 | Zero | `(0, 0)` = `0.0` |
| 6 | value * 0.0001 | `(1502500, 6)` = `150.2500` |
| 7 | value * 0.001 | `(5, 7)` = `0.005` |
| 8 | value * 0.01 | `(15025, 8)` = `150.25` |
| 10 | value * 1.0 | `(100, 10)` = `100.0` |
| 12 | value * 100.0 | `(5, 12)` = `500.0` |

---

## Enums

### SecType

Security type identifier.

| Variant | Code | String |
|---------|------|--------|
| `Stock` | 0 | `"STOCK"` |
| `Option` | 1 | `"OPTION"` |
| `Index` | 2 | `"INDEX"` |
| `Rate` | 3 | `"RATE"` |

Methods: `from_code(i32) -> Option<Self>`, `as_str() -> &str`

### DataType

80+ data field type codes. Grouped by category:

**Core:** Date(0), MsOfDay(1), Correction(2), PriceType(4), MsOfDay2(5), Undefined(6)

**Quote:** BidSize(101), BidExchange(102), Bid(103), BidCondition(104), AskSize(105), AskExchange(106), Ask(107), AskCondition(108), Midpoint(111), Vwap(112), Qwap(113), Wap(114)

**Open Interest:** OpenInterest(121)

**Trade:** Sequence(131), Size(132), Condition(133), Price(134), Exchange(135), ConditionFlags(136), PriceFlags(137), VolumeType(138), RecordsBack(139), Volume(141), Count(142)

**First-Order Greeks:** Theta(151), Vega(152), Delta(153), Rho(154), Epsilon(155), Lambda(156)

**Second-Order Greeks:** Gamma(161), Vanna(162), Charm(163), Vomma(164), Veta(165), **Vera(166)** *(added in v1.2.0)*, Sopdk(167)

**Third-Order Greeks:** Speed(171), Zomma(172), Color(173), Ultima(174)

**Black-Scholes Internals:** D1(181), D2(182), DualDelta(183), DualGamma(184)

**OHLC:** Open(191), High(192), Low(193), Close(194), NetChange(195)

**Implied Volatility:** ImpliedVol(201), BidImpliedVol(202), AskImpliedVol(203), UnderlyingPrice(204), IvError(205)

**Ratios:** Ratio(211), Rating(212)

**Dividends:** ExDate(221), RecordDate(222), PaymentDate(223), AnnDate(224), DividendAmount(225), LessAmount(226), Rate(230)

**Extended Conditions:** ExtCondition1(241), ExtCondition2(242), ExtCondition3(243), ExtCondition4(244)

**Splits:** SplitDate(251), BeforeShares(252), AfterShares(253)

**Fundamentals:** OutstandingShares(261), ShortShares(262), InstitutionalInterest(263), LastFiscalQuarter(264), LastFiscalYear(265), Assets(266), Liabilities(267), LongTermDebt(268), EpsMrq(269), EpsMry(270), EpsDiluted(271), SymbolChangeDate(272), SymbolChangeType(273), Symbol(274)

Methods: `from_code(i32) -> Option<Self>`, `is_price() -> bool`

### ReqType

Request type codes for historical data queries.

| Category | Variants |
|----------|----------|
| EOD | Eod(1), EodCta(3), EodUtp(4), EodOpra(5), EodOtc(6), EodOtcbb(7), EodTd(8) |
| Market Data | Quote(101), Volume(102), OpenInterest(103), Ohlc(104), OhlcQuote(105), Price(106) |
| Fundamentals | Fundamental(107), Dividend(108), Split(210), SymbolHistory(212) |
| Trade | Trade(201), TradeQuote(207) |
| Greeks | Greeks(203), TradeGreeks(301), AllGreeks(307), AllTradeGreeks(308) |
| Greeks Detail | GreeksSecondOrder(302), GreeksThirdOrder(303), TradeGreeksSecondOrder(305), TradeGreeksThirdOrder(306) |
| IV | ImpliedVolatility(202), ImpliedVolatilityVerbose(206) |
| Misc | TrailingDiv(0), Rate(2), Default(100), Quote1Min(109), Liquidity(204), LiquidityPlus(205), AltCalcs(304), EodQuoteGreeks(208), EodTradeGreeks(209), EodGreeks(211) |

### StreamMsgType

FPSS wire message codes (u8).

| Code | Name | Direction |
|------|------|-----------|
| 0 | Credentials | C->S |
| 1 | SessionToken | C->S |
| 2 | Info | S->C |
| 3 | Metadata | S->C |
| 4 | Connected | S->C |
| 10 | Ping | C->S |
| 11 | Error | S->C |
| 12 | Disconnected | S->C |
| 13 | Reconnected | S->C |
| 20 | Contract | S->C |
| 21 | Quote | Both |
| 22 | Trade | Both |
| 23 | OpenInterest | Both |
| 24 | Ohlcvc | S->C |
| 30 | Start | S->C |
| 31 | Restart | S->C |
| 32 | Stop | Both |
| 40 | ReqResponse | S->C |
| 51 | RemoveQuote | C->S |
| 52 | RemoveTrade | C->S |
| 53 | RemoveOpenInterest | C->S |

Methods: `from_code(u8) -> Option<Self>`

### StreamResponseType

Subscription response codes.

| Variant | Code | Meaning |
|---------|------|---------|
| `Subscribed` | 0 | Success |
| `Error` | 1 | General error |
| `MaxStreamsReached` | 2 | Subscription limit hit |
| `InvalidPerms` | 3 | Insufficient permissions |

### RemoveReason

Disconnect reason codes (i16). See [Architecture: Disconnect Reason Codes](architecture.md#disconnect-reason-codes) for the full table.

### Right

Option right: `Call`, `Put`.

Methods: `from_char(char) -> Option<Self>` (accepts `C/c/P/p`), `as_char() -> char`

### Venue

Data venue: `Nqb`, `UtpCta`.

Methods: `as_str() -> &str` (`"NQB"`, `"UTP_CTA"`)

### RateType

Interest rate types for Greeks calculations.

Variants: `Sofr`, `TreasuryM1`, `TreasuryM3`, `TreasuryM6`, `TreasuryY1`, `TreasuryY2`, `TreasuryY3`, `TreasuryY5`, `TreasuryY7`, `TreasuryY10`, `TreasuryY20`, `TreasuryY30`

---

## Greeks Calculator

Full Black-Scholes calculator ported from ThetaData's Java implementation.

All functions take the same base parameters:
- `s: f64` - Spot price (underlying)
- `x: f64` - Strike price
- `v: f64` - Volatility (sigma)
- `r: f64` - Risk-free rate
- `q: f64` - Dividend yield
- `t: f64` - Time to expiration (years)
- `is_call: bool` - true for call, false for put

### Individual Greeks

| Function | Signature | Order |
|----------|-----------|-------|
| `value` | `(s, x, v, r, q, t, is_call) -> f64` | - |
| `delta` | `(s, x, v, r, q, t, is_call) -> f64` | 1st |
| `theta` | `(s, x, v, r, q, t, is_call) -> f64` | 1st (daily, /365) |
| `vega` | `(s, x, v, r, q, t) -> f64` | 1st |
| `rho` | `(s, x, v, r, q, t, is_call) -> f64` | 1st |
| `epsilon` | `(s, x, v, r, q, t, is_call) -> f64` | 1st |
| `lambda` | `(s, x, v, r, q, t, is_call) -> f64` | 1st |
| `gamma` | `(s, x, v, r, q, t) -> f64` | 2nd |
| `vanna` | `(s, x, v, r, q, t) -> f64` | 2nd |
| `charm` | `(s, x, v, r, q, t, is_call) -> f64` | 2nd |
| `vomma` | `(s, x, v, r, q, t) -> f64` | 2nd |
| `veta` | `(s, x, v, r, q, t) -> f64` | 2nd |
| `speed` | `(s, x, v, r, q, t) -> f64` | 3rd |
| `zomma` | `(s, x, v, r, q, t) -> f64` | 3rd |
| `color` | `(s, x, v, r, q, t) -> f64` | 3rd |
| `ultima` | `(s, x, v, r, q, t) -> f64` | 3rd (clamped [-100, 100]) |
| `dual_delta` | `(s, x, v, r, q, t, is_call) -> f64` | Aux |
| `dual_gamma` | `(s, x, v, r, q, t) -> f64` | Aux |
| `d1` | `(s, x, v, r, q, t) -> f64` | Internal |
| `d2` | `(s, x, v, r, q, t) -> f64` | Internal |

### Implied Volatility

```rust
pub fn implied_volatility(
    s: f64, x: f64, r: f64, q: f64, t: f64,
    option_price: f64, is_call: bool,
) -> (f64, f64)  // (iv, error)
```

Bisection solver with up to 128 iterations. Returns `(iv, error)` where error is the relative difference `(theoretical - market) / market`.

### All Greeks at Once

```rust
pub fn all_greeks(
    s: f64, x: f64, r: f64, q: f64, t: f64,
    option_price: f64, is_call: bool,
) -> GreeksResult
```

Computes IV first, then all 22 Greeks using the solved IV.

```rust
pub struct GreeksResult {
    pub value: f64,
    pub delta: f64,
    pub gamma: f64,
    pub theta: f64,
    pub vega: f64,
    pub rho: f64,
    pub iv: f64,
    pub iv_error: f64,
    pub vanna: f64,
    pub charm: f64,
    pub vomma: f64,
    pub veta: f64,
    pub speed: f64,
    pub zomma: f64,
    pub color: f64,
    pub ultima: f64,
    pub d1: f64,
    pub d2: f64,
    pub dual_delta: f64,
    pub dual_gamma: f64,
    pub epsilon: f64,
    pub lambda: f64,
}
```

Example:

```rust
use thetadatadx::greeks;

// SPY $450 call, strike $455, 30 DTE
let result = greeks::all_greeks(
    450.0,            // spot
    455.0,            // strike
    0.05,             // risk-free rate
    0.015,            // dividend yield
    30.0 / 365.0,     // time to expiration (years)
    8.50,             // market price
    true,             // is_call
);
println!("IV: {:.4}, Delta: {:.4}, Gamma: {:.6}, Theta: {:.4}",
    result.iv, result.delta, result.gamma, result.theta);
```

---

## Credentials

```rust
pub struct Credentials {
    pub email: String,
    pub password: String,
}
```

### Construction

```rust
// From file (line 1 = email, line 2 = password)
let creds = Credentials::from_file("creds.txt")?;

// From string
let creds = Credentials::parse("user@example.com\nhunter2")?;

// Direct construction
let creds = Credentials::new("user@example.com", "hunter2");
```

Email is automatically lowercased and trimmed. Password is trimmed.

---

## DirectConfig

```rust
pub struct DirectConfig {
    // MDDS (gRPC)
    pub mdds_host: String,
    pub mdds_port: u16,
    pub mdds_tls: bool,
    pub mdds_max_message_size: usize,
    pub mdds_keepalive_secs: u64,
    pub mdds_keepalive_timeout_secs: u64,
    // FPSS (TCP)
    pub fpss_hosts: Vec<(String, u16)>,
    pub fpss_timeout_ms: u64,
    pub fpss_queue_depth: usize,
    pub fpss_ping_interval_ms: u64,
    pub fpss_connect_timeout_ms: u64,
    // Reconnection
    pub reconnect_wait_ms: u64,
    pub reconnect_wait_rate_limited_ms: u64,
    // Concurrency
    pub mdds_concurrent_requests: Option<usize>,  // max in-flight gRPC requests
                                                   // None = auto from tier (2^tier)
                                                   // Some(n) = manual override
    // Threading
    pub tokio_worker_threads: Option<usize>,
}
```

### Presets

```rust
DirectConfig::production()  // NJ datacenter, TLS, 4 FPSS hosts, 10s timeout
DirectConfig::dev()         // Same servers, 2 FPSS hosts, 5s timeout
```

### Methods

| Method | Signature | Description |
|--------|-----------|-------------|
| `mdds_uri()` | `&self -> String` | Build gRPC URI (`https://mdds-01...`) |
| `parse_fpss_hosts()` | `(hosts_str: &str) -> Result<Vec<(String, u16)>, Error>` | Parse `host:port,...` format |

---

## Error Types

```rust
pub enum Error {
    Transport(tonic::transport::Error),   // gRPC channel errors
    Status(Box<tonic::Status>),           // gRPC status codes
    Decompress(String),                   // zstd decompression failure
    Decode(String),                       // protobuf decode failure
    NoData,                               // endpoint returned no usable data
    Auth(String),                         // Nexus auth errors
    Fpss(String),                         // FPSS connection errors
    FpssProtocol(String),                 // FPSS wire protocol errors
    FpssDisconnected(String),             // FPSS server rejected connection
    Config(String),                       // configuration errors
    Http(reqwest::Error),                 // HTTP request errors
    Io(std::io::Error),                   // I/O errors
    Tls(rustls::Error),                  // TLS handshake errors
}
```

All variants implement `Display` and `std::error::Error`. Automatic conversions via `From` are provided for `tonic::transport::Error`, `tonic::Status`, `reqwest::Error`, `std::io::Error`, and `rustls::Error`.

---

## AuthUser

The Nexus authentication response includes per-asset subscription tier information:

```rust
pub struct AuthUser {
    pub session_id: String,
    pub stock_tier: i32,
    pub option_tier: i32,
    pub index_tier: i32,
    pub futures_tier: i32,
    // ... other fields
}
```

These tiers determine the dynamic gRPC concurrency limit (`2^tier`) and are available for per-asset-class permission checks. The `stock_tier` is used as the default for `mdds_concurrent_requests` unless manually overridden in `DirectConfig`.

---

## Decode Utilities

Low-level functions for working with raw `DataTable` responses.

**Column lookup warning**: The `extract_*_column` functions emit a `warn!` log when a requested column header is not found in the DataTable, instead of silently returning a vec of `None`s. This makes schema mismatches immediately visible in logs.

```rust
pub fn decode_data_table(response: &ResponseData) -> Result<DataTable, Error>
pub fn decompress_response(response: &ResponseData) -> Result<Vec<u8>, Error>
pub fn extract_number_column(table: &DataTable, header: &str) -> Vec<Option<i64>>
pub fn extract_text_column(table: &DataTable, header: &str) -> Vec<Option<String>>
pub fn extract_price_column(table: &DataTable, header: &str) -> Vec<Option<Price>>
pub fn parse_trade_ticks(table: &DataTable) -> Vec<TradeTick>
pub fn parse_quote_ticks(table: &DataTable) -> Vec<QuoteTick>
pub fn parse_ohlc_ticks(table: &DataTable) -> Vec<OhlcTick>
```

---

## FIT Codec

### FitReader

```rust
pub struct FitReader<'a> {
    pub is_date: bool,
}

impl<'a> FitReader<'a> {
    pub fn new(buf: &'a [u8]) -> Self;
    pub fn with_offset(buf: &'a [u8], offset: usize) -> Self;
    pub fn position(&self) -> usize;
    pub fn is_exhausted(&self) -> bool;
    pub fn read_changes(&mut self, alloc: &mut [i32]) -> usize;
}
```

```rust
pub fn apply_deltas(tick: &mut [i32], prev: &[i32], n_fields: usize);
```

### FIE Encoder

```rust
pub fn string_to_fie_line(input: &str) -> Vec<u8>;
pub fn try_string_to_fie_line(input: &str) -> Result<Vec<u8>, u8>;
pub fn fie_line_to_string(data: &[u8]) -> Option<String>;
pub const fn char_to_nibble(c: u8) -> Option<u8>;
pub const fn nibble_to_char(n: u8) -> Option<u8>;
```
