//! **Deprecated** -- retained for reference only.
//!
//! Optional parameters are now set via the builder pattern on each endpoint.
//! Instead of passing an options struct, chain setter methods before `.await`:
//!
//! ```rust,ignore
//! // Before:
//! let ticks = client.stock_snapshot_ohlc(&["AAPL"], &Default::default()).await?;
//!
//! // After:
//! let ticks = client.stock_snapshot_ohlc(&["AAPL"]).await?;
//! let ticks = client.stock_snapshot_ohlc(&["AAPL"]).venue("arca").await?;
//! ```
//!
//! These structs were previously used to pass optional filters (venue, date
//! ranges, Greeks parameters, etc.) to endpoints. All fields default to `None`.
//!
//! # Usage
//!
//! ```rust,ignore
//! use thetadatadx::options::StockSnapshotOptions;
//!
//! // Default (no overrides):
//! let ticks = client.stock_snapshot_ohlc(&["AAPL"], &Default::default()).await?;
//!
//! // Override venue:
//! let opts = StockSnapshotOptions {
//!     venue: Some("arca".into()),
//!     ..Default::default()
//! };
//! let ticks = client.stock_snapshot_ohlc(&["AAPL"], &opts).await?;
//! ```

/// Optional parameters for stock snapshot endpoints.
#[derive(Default, Clone, Debug)]
pub struct StockSnapshotOptions {
    /// Venue override. Default: "nqb" (NASDAQ Best).
    pub venue: Option<String>,
    /// Only return data after this time (e.g., "09:45:00").
    pub min_time: Option<String>,
}

/// Optional parameters for stock history endpoints.
#[derive(Default, Clone, Debug)]
pub struct StockHistoryOptions {
    /// Venue override. Default: "nqb" (NASDAQ Best).
    pub venue: Option<String>,
    /// Start date for date-range queries (YYYYMMDD).
    pub start_date: Option<String>,
    /// End date for date-range queries (YYYYMMDD).
    pub end_date: Option<String>,
    /// Exclude pre/post market trades (trade_quote only).
    pub exclusive: Option<bool>,
}

/// Optional parameters for option snapshot endpoints (non-greeks).
#[derive(Default, Clone, Debug)]
pub struct OptionSnapshotOptions {
    /// Maximum days to expiration filter.
    pub max_dte: Option<i32>,
    /// Number of strikes from ATM to include.
    pub strike_range: Option<i32>,
    /// Only return data after this time.
    pub min_time: Option<String>,
}

/// Optional parameters for option snapshot greeks endpoints.
#[derive(Default, Clone, Debug)]
pub struct OptionGreeksSnapshotOptions {
    /// Maximum days to expiration filter.
    pub max_dte: Option<i32>,
    /// Number of strikes from ATM to include.
    pub strike_range: Option<i32>,
    /// Only return data after this time.
    pub min_time: Option<String>,
    /// Annual dividend yield for Greeks calculation.
    pub annual_dividend: Option<f64>,
    /// Rate type for Greeks calculation.
    pub rate_type: Option<String>,
    /// Rate value for Greeks calculation.
    pub rate_value: Option<f64>,
    /// Override stock price for Greeks calculation.
    pub stock_price: Option<f64>,
    /// Greeks calculation version.
    pub version: Option<String>,
    /// Use market value instead of mid for Greeks.
    pub use_market_value: Option<bool>,
}

/// Optional parameters for option history endpoints (non-greeks).
#[derive(Default, Clone, Debug)]
pub struct OptionHistoryOptions {
    /// Maximum days to expiration filter.
    pub max_dte: Option<i32>,
    /// Number of strikes from ATM to include.
    pub strike_range: Option<i32>,
    /// Start date for date-range queries (YYYYMMDD).
    pub start_date: Option<String>,
    /// End date for date-range queries (YYYYMMDD).
    pub end_date: Option<String>,
    /// Exclude pre/post market trades.
    pub exclusive: Option<bool>,
}

/// Optional parameters for option history greeks endpoints.
#[derive(Default, Clone, Debug)]
pub struct OptionGreeksHistoryOptions {
    /// Maximum days to expiration filter.
    pub max_dte: Option<i32>,
    /// Number of strikes from ATM to include.
    pub strike_range: Option<i32>,
    /// Start date for date-range queries (YYYYMMDD).
    pub start_date: Option<String>,
    /// End date for date-range queries (YYYYMMDD).
    pub end_date: Option<String>,
    /// Annual dividend yield for Greeks calculation.
    pub annual_dividend: Option<f64>,
    /// Rate type for Greeks calculation.
    pub rate_type: Option<String>,
    /// Rate value for Greeks calculation.
    pub rate_value: Option<f64>,
    /// Greeks calculation version.
    pub version: Option<String>,
    /// Use NBBO for underlyer price.
    pub underlyer_use_nbbo: Option<bool>,
}

/// Optional parameters for index snapshot endpoints.
#[derive(Default, Clone, Debug)]
pub struct IndexSnapshotOptions {
    /// Only return data after this time.
    pub min_time: Option<String>,
}

/// Optional parameters for index history price endpoint.
#[derive(Default, Clone, Debug)]
pub struct IndexHistoryOptions {
    /// Start date for date-range queries (YYYYMMDD).
    pub start_date: Option<String>,
    /// End date for date-range queries (YYYYMMDD).
    pub end_date: Option<String>,
}

/// Optional parameters for option list contracts.
#[derive(Default, Clone, Debug)]
pub struct OptionListOptions {
    /// Maximum days to expiration filter.
    pub max_dte: Option<i32>,
}
