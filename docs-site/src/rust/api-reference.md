# API Reference (Rust)

Complete type and method listing for the `thetadatadx` crate.

## ThetaDataDx

The unified client for all ThetaData access -- historical data via MDDS/gRPC and real-time streaming via FPSS/TCP.

### Construction

```rust
pub async fn connect(creds: &Credentials, config: DirectConfig) -> Result<Self, Error>
```

Authenticates against the Nexus HTTP API to obtain a session UUID, then opens a gRPC channel (TLS) to the MDDS server. Streaming is started lazily via `start_streaming()`.

### Accessor Methods

| Method | Return | Description |
|--------|--------|-------------|
| `config()` | `&DirectConfig` | Return config snapshot |
| `session_uuid()` | `&str` | Return the Nexus session UUID |
| `channel()` | `&tonic::transport::Channel` | Access the underlying gRPC channel |
| `raw_query_info()` | `proto_v3::QueryInfo` | Get a QueryInfo for use with raw_query |

### Endpoint Methods (61)

All methods are async and return `Result<T, Error>`.

**Stock (14):** `stock_list_symbols`, `stock_list_dates`, `stock_snapshot_ohlc`, `stock_snapshot_trade`, `stock_snapshot_quote`, `stock_snapshot_market_value`, `stock_history_eod`, `stock_history_ohlc`, `stock_history_ohlc_range`, `stock_history_trade`, `stock_history_quote`, `stock_history_trade_quote`, `stock_at_time_trade`, `stock_at_time_quote`

**Option (34):** `option_list_symbols`, `option_list_dates`, `option_list_expirations`, `option_list_strikes`, `option_list_contracts`, `option_snapshot_ohlc`, `option_snapshot_trade`, `option_snapshot_quote`, `option_snapshot_open_interest`, `option_snapshot_market_value`, `option_snapshot_greeks_implied_volatility`, `option_snapshot_greeks_all`, `option_snapshot_greeks_first_order`, `option_snapshot_greeks_second_order`, `option_snapshot_greeks_third_order`, `option_history_eod`, `option_history_ohlc`, `option_history_trade`, `option_history_quote`, `option_history_trade_quote`, `option_history_open_interest`, `option_history_greeks_eod`, `option_history_greeks_all`, `option_history_trade_greeks_all`, `option_history_greeks_first_order`, `option_history_trade_greeks_first_order`, `option_history_greeks_second_order`, `option_history_trade_greeks_second_order`, `option_history_greeks_third_order`, `option_history_trade_greeks_third_order`, `option_history_greeks_implied_volatility`, `option_history_trade_greeks_implied_volatility`, `option_at_time_trade`, `option_at_time_quote`

**Index (9):** `index_list_symbols`, `index_list_dates`, `index_snapshot_ohlc`, `index_snapshot_price`, `index_snapshot_market_value`, `index_history_eod`, `index_history_ohlc`, `index_history_price`, `index_at_time_price`

**Calendar (3):** `calendar_open_today`, `calendar_on_date`, `calendar_year`

**Rate (1):** `interest_rate_history_eod`

### Streaming Variants (4)

Process large responses chunk by chunk without loading everything into memory:

| Method | Description |
|--------|-------------|
| `stock_history_trade_stream` | Stream stock trades |
| `stock_history_quote_stream` | Stream stock quotes |
| `option_history_trade_stream` | Stream option trades |
| `option_history_quote_stream` | Stream option quotes |

### Raw Query

Escape hatch for endpoints not yet wrapped by typed methods:

```rust
pub async fn raw_query<F, Fut>(&self, call: F) -> Result<proto::DataTable, Error>
```

---

## Streaming (FPSS)

Real-time streaming via FPSS TLS/TCP, accessed through `ThetaDataDx`.

### Starting the Stream

```rust
pub fn start_streaming(
    &self,
    callback: impl Fn(&FpssEvent) + Send + 'static,
) -> Result<(), Error>
```

### Subscription Methods

| Method | Signature | Description |
|--------|-----------|-------------|
| `subscribe_quotes` | `(&self, &Contract) -> Result<i32, Error>` | Subscribe to quotes |
| `subscribe_trades` | `(&self, &Contract) -> Result<i32, Error>` | Subscribe to trades |
| `subscribe_open_interest` | `(&self, &Contract) -> Result<i32, Error>` | Subscribe to OI |
| `subscribe_full_trades` | `(&self, SecType) -> Result<i32, Error>` | Subscribe all trades for a security type |
| `unsubscribe_quotes` | `(&self, &Contract) -> Result<i32, Error>` | Unsubscribe quotes |
| `unsubscribe_trades` | `(&self, &Contract) -> Result<i32, Error>` | Unsubscribe trades |
| `unsubscribe_open_interest` | `(&self, &Contract) -> Result<i32, Error>` | Unsubscribe OI |

### State Methods

| Method | Return | Description |
|--------|--------|-------------|
| `is_authenticated()` | `bool` | Check if connection is live |
| `server_addr()` | `&str` | Get connected server address |
| `contract_map()` | `HashMap<i32, Contract>` | Server-assigned contract IDs |
| `shutdown()` | `Result<(), Error>` | Send STOP and shut down |

### Reconnection

```rust
pub fn reconnect_delay(reason: RemoveReason) -> Option<u64>
```

---

## Credentials

```rust
Credentials::from_file("creds.txt")?;
Credentials::new("user@example.com", "password");
Credentials::parse("user@example.com\npassword")?;
```

---

## Tick Types

All tick types are `Copy + Clone + Debug` structs with `i32` fields.

### TradeTick (16 fields)

| Field | Type | Description |
|-------|------|-------------|
| `ms_of_day` | `i32` | Milliseconds since midnight ET |
| `sequence` | `i32` | Sequence number |
| `size` | `i32` | Trade size (shares) |
| `price` | `i32` | Fixed-point price (use `get_price()`) |
| `exchange` | `i32` | Exchange code |
| `condition` | `i32` | Trade condition code |
| `price_type` | `i32` | Decimal type for price decoding |
| `date` | `i32` | Date as YYYYMMDD integer |

Methods: `get_price()`, `is_cancelled()`, `regular_trading_hours()`, `is_seller()`, `is_incremental_volume()`

### QuoteTick (11 fields)

| Field | Type | Description |
|-------|------|-------------|
| `ms_of_day` | `i32` | Milliseconds since midnight ET |
| `bid` / `ask` | `i32` | Fixed-point prices |
| `bid_size` / `ask_size` | `i32` | Quote sizes |
| `price_type` | `i32` | Decimal type |
| `date` | `i32` | Date as YYYYMMDD |

Methods: `bid_price()`, `ask_price()`, `midpoint_price()`, `midpoint_value()`

### OhlcTick (9 fields)

Fields: `ms_of_day`, `open`, `high`, `low`, `close`, `volume`, `count`, `price_type`, `date`

Methods: `open_price()`, `high_price()`, `low_price()`, `close_price()`

### EodTick (18 fields)

Full end-of-day snapshot with OHLC + closing quote data.

Methods: `open_price()`, `high_price()`, `low_price()`, `close_price()`, `bid_price()`, `ask_price()`

### OpenInterestTick (3 fields)

Fields: `ms_of_day`, `open_interest`, `date`

### SnapshotTradeTick (7 fields)

Abbreviated trade for snapshots. Method: `get_price()`

### TradeQuoteTick (25 fields)

Combined trade + quote tick. Methods: `trade_price()`, `bid_price()`, `ask_price()`

---

## Price

Fixed-point price with variable decimal precision.

```rust
pub struct Price {
    pub value: i32,
    pub price_type: i32,
}
```

Formula: `real_price = value * 10^(price_type - 10)`

| Method | Return | Description |
|--------|--------|-------------|
| `to_f64()` | `f64` | Lossy float conversion |
| `is_zero()` | `bool` | True if value == 0 or price_type == 0 |

Implements: `Display`, `Eq`, `Ord`, `Copy`, `Clone`.

Prices with different `price_type` values can be compared directly:

```rust
let a = Price::new(15025, 8);    // 150.25
let b = Price::new(1502500, 6);  // 150.2500
assert_eq!(a, b);                // true
```

---

## FpssEvent

```rust
pub enum FpssEvent {
    Data(FpssData),
    Control(FpssControl),
    RawData { code: u8, payload: Vec<u8> },
}
```

### FpssData

```rust
pub enum FpssData {
    Quote { contract_id: i32, ms_of_day: i32, bid: i32, ask: i32, bid_size: i32,
            ask_size: i32, bid_exchange: i32, ask_exchange: i32,
            bid_condition: i32, ask_condition: i32, price_type: i32, date: i32 },
    Trade { contract_id: i32, ms_of_day: i32, sequence: i32, price: i32, size: i32,
            exchange: i32, condition: i32, ext_condition1-4: i32,
            condition_flags: i32, price_flags: i32, volume_type: i32,
            records_back: i32, price_type: i32, date: i32 },
    OpenInterest { contract_id: i32, ms_of_day: i32, open_interest: i32, date: i32 },
    Ohlcvc { contract_id: i32, ms_of_day: i32, open: i32, high: i32, low: i32,
             close: i32, volume: i32, count: i32, price_type: i32, date: i32 },
}
```

### FpssControl

```rust
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

---

## Contract

```rust
pub struct Contract {
    pub root: String,
    pub sec_type: SecType,
    pub exp_date: Option<i32>,    // YYYYMMDD for options
    pub is_call: Option<bool>,    // true=call, false=put
    pub strike: Option<i32>,      // scaled integer
}
```

Constructors: `Contract::stock("AAPL")`, `Contract::index("SPX")`, `Contract::rate("SOFR")`, `Contract::option("SPY", 20261218, true, 60000)`

---

## Enums

### SecType

| Variant | Code |
|---------|------|
| `Stock` | 0 |
| `Option` | 1 |
| `Index` | 2 |
| `Rate` | 3 |

### StreamResponseType

| Variant | Meaning |
|---------|---------|
| `Subscribed` | Success |
| `Error` | General error |
| `MaxStreamsReached` | Subscription limit hit |
| `InvalidPerms` | Insufficient permissions |

### Right

Option right: `Call`, `Put`. Methods: `from_char(char)`, `as_char()`

---

## GreeksResult

```rust
pub struct GreeksResult {
    pub value: f64,      pub delta: f64,      pub gamma: f64,
    pub theta: f64,      pub vega: f64,       pub rho: f64,
    pub iv: f64,         pub iv_error: f64,
    pub vanna: f64,      pub charm: f64,      pub vomma: f64,
    pub veta: f64,       pub speed: f64,      pub zomma: f64,
    pub color: f64,      pub ultima: f64,
    pub d1: f64,         pub d2: f64,
    pub dual_delta: f64, pub dual_gamma: f64,
    pub epsilon: f64,    pub lambda: f64,
}
```

---

## Error

```rust
pub enum Error {
    Auth(String),          // Invalid credentials
    Http(String),          // Network or server issue
    Grpc(tonic::Status),   // gRPC error
    NoData,                // Symbol does not exist
    // ... other variants
}
```
