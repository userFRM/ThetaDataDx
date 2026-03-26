# API Reference

## DirectClient

The primary client for historical data via MDDS/gRPC. Authenticates via Nexus, opens a gRPC channel, and exposes typed methods for every data endpoint.

### Construction

```rust
pub async fn connect(creds: &Credentials, config: DirectConfig) -> Result<Self, Error>
```

1. Authenticates against the Nexus HTTP API to obtain a session UUID
2. Opens a gRPC channel (TLS) to the MDDS server

```rust
use thetadatadx::{DirectClient, Credentials, DirectConfig};

let creds = Credentials::from_file("creds.txt")?;
let client = DirectClient::connect(&creds, DirectConfig::production()).await?;
```

### Accessor Methods

| Method | Signature | Description |
|--------|-----------|-------------|
| `config()` | `&self -> &DirectConfig` | Return config snapshot |
| `session_uuid()` | `&self -> &str` | Return the Nexus session UUID |
| `channel()` | `&self -> &tonic::transport::Channel` | Access the underlying gRPC channel |
| `raw_query_info()` | `&self -> proto_v3::QueryInfo` | Get a QueryInfo for use with raw_query |

### Stock -- List (2)

```rust
pub async fn stock_list_symbols(&self) -> Result<Vec<String>, Error>
```

All available stock symbols. gRPC: `GetStockListSymbols`

```rust
pub async fn stock_list_dates(&self, request_type: &str, symbol: &str) -> Result<Vec<String>, Error>
```

Available dates for a stock by request type (e.g. `"EOD"`, `"TRADE"`, `"QUOTE"`). gRPC: `GetStockListDates`

### Stock -- Snapshot (4)

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
pub async fn stock_snapshot_market_value(&self, symbols: &[&str]) -> Result<proto::DataTable, Error>
```

Latest market value snapshot for one or more stocks. gRPC: `GetStockSnapshotMarketValue`

### Stock -- History (6)

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
) -> Result<proto::DataTable, Error>
```

Combined trade + quote ticks. Returns raw `DataTable`. gRPC: `GetStockHistoryTradeQuote`

### Stock -- AtTime (2)

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

### Option -- List (5)

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
) -> Result<proto::DataTable, Error>
```

All option contracts for a symbol on a given date. Returns `DataTable` with contract details. gRPC: `GetOptionListContracts`

### Option -- Snapshot (5)

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
) -> Result<proto::DataTable, Error>
```

Latest open interest snapshot for option contracts. gRPC: `GetOptionSnapshotOpenInterest`

```rust
pub async fn option_snapshot_market_value(
    &self, symbol: &str, expiration: &str, strike: &str, right: &str
) -> Result<proto::DataTable, Error>
```

Latest market value snapshot for option contracts. gRPC: `GetOptionSnapshotMarketValue`

### Option -- Snapshot Greeks (5)

```rust
pub async fn option_snapshot_greeks_implied_volatility(
    &self, symbol: &str, expiration: &str, strike: &str, right: &str
) -> Result<proto::DataTable, Error>
```

Implied volatility snapshot. gRPC: `GetOptionSnapshotGreeksImpliedVolatility`

```rust
pub async fn option_snapshot_greeks_all(
    &self, symbol: &str, expiration: &str, strike: &str, right: &str
) -> Result<proto::DataTable, Error>
```

All Greeks snapshot. gRPC: `GetOptionSnapshotGreeksAll`

```rust
pub async fn option_snapshot_greeks_first_order(
    &self, symbol: &str, expiration: &str, strike: &str, right: &str
) -> Result<proto::DataTable, Error>
```

First-order Greeks snapshot (delta, theta, rho, etc.). gRPC: `GetOptionSnapshotGreeksFirstOrder`

```rust
pub async fn option_snapshot_greeks_second_order(
    &self, symbol: &str, expiration: &str, strike: &str, right: &str
) -> Result<proto::DataTable, Error>
```

Second-order Greeks snapshot (gamma, vanna, charm, etc.). gRPC: `GetOptionSnapshotGreeksSecondOrder`

```rust
pub async fn option_snapshot_greeks_third_order(
    &self, symbol: &str, expiration: &str, strike: &str, right: &str
) -> Result<proto::DataTable, Error>
```

Third-order Greeks snapshot (speed, color, ultima, etc.). gRPC: `GetOptionSnapshotGreeksThirdOrder`

### Option -- History (6)

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
) -> Result<proto::DataTable, Error>
```

Combined trade + quote ticks for an option contract. gRPC: `GetOptionHistoryTradeQuote`

```rust
pub async fn option_history_open_interest(
    &self, symbol: &str, expiration: &str, strike: &str, right: &str, date: &str
) -> Result<proto::DataTable, Error>
```

Open interest history for an option contract. gRPC: `GetOptionHistoryOpenInterest`

### Option -- History Greeks (6)

```rust
pub async fn option_history_greeks_eod(
    &self, symbol: &str, expiration: &str, strike: &str, right: &str,
    start_date: &str, end_date: &str
) -> Result<proto::DataTable, Error>
```

EOD Greeks history for an option contract. gRPC: `GetOptionHistoryGreeksEod`

```rust
pub async fn option_history_greeks_all(
    &self, symbol: &str, expiration: &str, strike: &str, right: &str,
    date: &str, interval: &str
) -> Result<proto::DataTable, Error>
```

All Greeks history (intraday, sampled by interval). gRPC: `GetOptionHistoryGreeksAll`

```rust
pub async fn option_history_greeks_first_order(
    &self, symbol: &str, expiration: &str, strike: &str, right: &str,
    date: &str, interval: &str
) -> Result<proto::DataTable, Error>
```

First-order Greeks history (intraday, sampled by interval). gRPC: `GetOptionHistoryGreeksFirstOrder`

```rust
pub async fn option_history_greeks_second_order(
    &self, symbol: &str, expiration: &str, strike: &str, right: &str,
    date: &str, interval: &str
) -> Result<proto::DataTable, Error>
```

Second-order Greeks history (intraday, sampled by interval). gRPC: `GetOptionHistoryGreeksSecondOrder`

```rust
pub async fn option_history_greeks_third_order(
    &self, symbol: &str, expiration: &str, strike: &str, right: &str,
    date: &str, interval: &str
) -> Result<proto::DataTable, Error>
```

Third-order Greeks history (intraday, sampled by interval). gRPC: `GetOptionHistoryGreeksThirdOrder`

```rust
pub async fn option_history_greeks_implied_volatility(
    &self, symbol: &str, expiration: &str, strike: &str, right: &str,
    date: &str, interval: &str
) -> Result<proto::DataTable, Error>
```

Implied volatility history (intraday, sampled by interval). gRPC: `GetOptionHistoryGreeksImpliedVolatility`

### Option -- History Trade Greeks (5)

```rust
pub async fn option_history_trade_greeks_all(
    &self, symbol: &str, expiration: &str, strike: &str, right: &str, date: &str
) -> Result<proto::DataTable, Error>
```

All Greeks computed on each trade. gRPC: `GetOptionHistoryTradeGreeksAll`

```rust
pub async fn option_history_trade_greeks_first_order(
    &self, symbol: &str, expiration: &str, strike: &str, right: &str, date: &str
) -> Result<proto::DataTable, Error>
```

First-order Greeks on each trade. gRPC: `GetOptionHistoryTradeGreeksFirstOrder`

```rust
pub async fn option_history_trade_greeks_second_order(
    &self, symbol: &str, expiration: &str, strike: &str, right: &str, date: &str
) -> Result<proto::DataTable, Error>
```

Second-order Greeks on each trade. gRPC: `GetOptionHistoryTradeGreeksSecondOrder`

```rust
pub async fn option_history_trade_greeks_third_order(
    &self, symbol: &str, expiration: &str, strike: &str, right: &str, date: &str
) -> Result<proto::DataTable, Error>
```

Third-order Greeks on each trade. gRPC: `GetOptionHistoryTradeGreeksThirdOrder`

```rust
pub async fn option_history_trade_greeks_implied_volatility(
    &self, symbol: &str, expiration: &str, strike: &str, right: &str, date: &str
) -> Result<proto::DataTable, Error>
```

Implied volatility on each trade. gRPC: `GetOptionHistoryTradeGreeksImpliedVolatility`

### Option -- AtTime (2)

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

### Index -- List (2)

```rust
pub async fn index_list_symbols(&self) -> Result<Vec<String>, Error>
```

All available index symbols. gRPC: `GetIndexListSymbols`

```rust
pub async fn index_list_dates(&self, symbol: &str) -> Result<Vec<String>, Error>
```

Available dates for an index symbol. gRPC: `GetIndexListDates`

### Index -- Snapshot (3)

```rust
pub async fn index_snapshot_ohlc(&self, symbols: &[&str]) -> Result<Vec<OhlcTick>, Error>
```

Latest OHLC snapshot for one or more indices. gRPC: `GetIndexSnapshotOhlc`

```rust
pub async fn index_snapshot_price(&self, symbols: &[&str]) -> Result<proto::DataTable, Error>
```

Latest price snapshot for one or more indices. gRPC: `GetIndexSnapshotPrice`

```rust
pub async fn index_snapshot_market_value(&self, symbols: &[&str]) -> Result<proto::DataTable, Error>
```

Latest market value snapshot for one or more indices. gRPC: `GetIndexSnapshotMarketValue`

### Index -- History (3)

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
) -> Result<proto::DataTable, Error>
```

Intraday price history for an index. gRPC: `GetIndexHistoryPrice`

### Index -- AtTime (1)

```rust
pub async fn index_at_time_price(
    &self, symbol: &str, start_date: &str, end_date: &str, time_of_day: &str
) -> Result<proto::DataTable, Error>
```

Index price at a specific time of day across a date range. gRPC: `GetIndexAtTimePrice`

### Interest Rate (1)

```rust
pub async fn interest_rate_history_eod(
    &self, symbol: &str, start_date: &str, end_date: &str
) -> Result<proto::DataTable, Error>
```

End-of-day interest rate history. gRPC: `GetInterestRateHistoryEod`

### Calendar (3)

```rust
pub async fn calendar_open_today(&self) -> Result<proto::DataTable, Error>
```

Whether the market is open today. gRPC: `GetCalendarOpenToday`

```rust
pub async fn calendar_on_date(&self, date: &str) -> Result<proto::DataTable, Error>
```

Calendar information for a specific date. gRPC: `GetCalendarOnDate`

```rust
pub async fn calendar_year(&self, year: &str) -> Result<proto::DataTable, Error>
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
    query_info: Some(client.raw_query_info()),
    params: Some(proto_v3::CalendarYearRequestQuery {
        year: "2024".to_string(),
    }),
};

let table = client.raw_query(|mut stub| {
    Box::pin(async move {
        Ok(stub.get_calendar_year(request).await?.into_inner())
    })
}).await?;
```

### Endpoint Count

DirectClient exposes **61 typed methods** covering all 60 gRPC RPCs in `BetaThetaTerminal` plus 1 convenience range-query variant (`stock_history_ohlc_range`). All 61 methods are generated by the `define_endpoint!` macro in `direct.rs`.

### FFI Coverage

All 61 DirectClient endpoints are exposed through the `thetadatadx-ffi` C ABI crate. Each Rust method has a corresponding `extern "C"` function (e.g., `thetadatadx_stock_history_eod`). The Go and C++ SDKs wrap these FFI functions 1:1.

### Python SDK Coverage

All 61 DirectClient endpoints are available in the Python SDK via PyO3 bindings (e.g., `client.stock_history_eod(...)`). FPSS streaming is not yet exposed in Python (see TODO.md).

---

## FpssClient

Real-time streaming client for ThetaData's FPSS servers.

### Construction

```rust
pub async fn connect(
    creds: &Credentials, event_buffer: usize
) -> Result<(Self, mpsc::Receiver<FpssEvent>), Error>
```

Establishes TLS connection, authenticates, starts background reader and heartbeat tasks. Returns the client and an event receiver channel.

```rust
let (client, mut events) = FpssClient::connect(&creds, 1024).await?;
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
| `is_authenticated` | `(&self) -> bool` | Check if connection is live |
| `server_addr` | `(&self) -> &str` | Get connected server address |
| `contract_map` | `(&self) -> HashMap<i32, Contract>` | Server-assigned contract IDs |
| `event_sender` | `(&self) -> mpsc::Sender<FpssEvent>` | Clone the event sender |
| `shutdown` | `(&mut self) -> Result<(), Error>` | Send STOP and shut down tasks |

### FpssEvent

Events received through the channel:

```rust
pub enum FpssEvent {
    LoginSuccess { permissions: String },
    ContractAssigned { id: i32, contract: Contract },
    QuoteData { payload: Vec<u8> },
    TradeData { payload: Vec<u8> },
    OpenInterestData { payload: Vec<u8> },
    OhlcvcData { payload: Vec<u8> },
    ReqResponse { req_id: i32, result: StreamResponseType },
    MarketOpen,
    MarketClose,
    ServerError { message: String },
    Disconnected { reason: RemoveReason },
}
```

### Reconnection

```rust
pub async fn reconnect(
    creds: &Credentials,
    previous_subs: Vec<(SubscriptionKind, Contract)>,
    delay_ms: u64,
    event_buffer: usize,
) -> Result<(FpssClient, mpsc::Receiver<FpssEvent>), Error>
```

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

All tick types are `Copy + Clone + Debug` structs with `i32` fields. Prices are stored in fixed-point encoding. Use the `*_price()` methods to get `Price` values with proper decimal handling.

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

Methods: `open_price()`, `high_price()`, `low_price()`, `close_price()` -- all return `Price`.

### EodTick

18 fields -- full end-of-day snapshot with OHLC + quote data.

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

Methods: `open_price()`, `high_price()`, `low_price()`, `close_price()`, `bid_price()`, `ask_price()`, `midpoint_value()` -- all operate on the shared `price_type`.

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

25-field combined trade + quote tick.

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
    // Quote portion (9 fields)
    pub quote_ms_of_day: i32,
    pub bid_size: i32,
    pub bid_exchange: i32,
    pub bid: i32,
    pub bid_condition: i32,
    pub ask_size: i32,
    pub ask_exchange: i32,
    pub ask: i32,
    pub ask_condition: i32,
    // Shared
    pub price_type: i32,
    pub date: i32,
}
```

Methods: `trade_price()`, `bid_price()`, `ask_price()` -- all return `Price`.

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

**Second-Order Greeks:** Gamma(161), Vanna(162), Charm(163), Vomma(164), Veta(165), Vera(166), Sopdk(167)

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
- `s: f64` -- Spot price (underlying)
- `x: f64` -- Strike price
- `v: f64` -- Volatility (sigma)
- `r: f64` -- Risk-free rate
- `q: f64` -- Dividend yield
- `t: f64` -- Time to expiration (years)
- `is_call: bool` -- true for call, false for put

### Individual Greeks

| Function | Signature | Order |
|----------|-----------|-------|
| `value` | `(s, x, v, r, q, t, is_call) -> f64` | -- |
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
    NoData,                               // empty response
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

## Decode Utilities

Low-level functions for working with raw `DataTable` responses:

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
