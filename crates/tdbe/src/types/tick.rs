use crate::flags;
use crate::types::price::Price;

/// Calendar day. Market open/close schedule.
#[derive(Debug, Clone, Copy)]
#[repr(C, align(64))]
pub struct CalendarDay {
    pub date: i32,
    pub is_open: i32,
    pub open_time: i32,
    pub close_time: i32,
    pub status: i32,
}

/// End-of-day tick. Full EOD snapshot with OHLC + quote.
#[derive(Debug, Clone, Copy)]
#[repr(C, align(64))]
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

/// Greeks tick. Full set of option greeks.
#[derive(Debug, Clone, Copy)]
#[repr(C, align(64))]
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

/// Interest rate tick.
#[derive(Debug, Clone, Copy)]
#[repr(C, align(64))]
pub struct InterestRateTick {
    pub ms_of_day: i32,
    pub rate: f64,
    pub date: i32,
}

/// Implied volatility tick.
#[derive(Debug, Clone, Copy)]
#[repr(C, align(64))]
pub struct IvTick {
    pub ms_of_day: i32,
    pub implied_volatility: f64,
    pub iv_error: f64,
    pub date: i32,
}

/// Market value tick.
#[derive(Debug, Clone, Copy)]
#[repr(C, align(64))]
pub struct MarketValueTick {
    pub ms_of_day: i32,
    pub market_cap: i64,
    pub shares_outstanding: i64,
    pub enterprise_value: i64,
    pub book_value: i64,
    pub free_float: i64,
    pub date: i32,
}

/// OHLC tick. Aggregated bar data.
#[derive(Debug, Clone, Copy)]
#[repr(C, align(64))]
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

/// Open interest tick.
#[derive(Debug, Clone, Copy)]
pub struct OpenInterestTick {
    pub ms_of_day: i32,
    pub open_interest: i32,
    pub date: i32,
}

/// Option contract specification.
#[derive(Debug, Clone)]
pub struct OptionContract {
    pub root: String,
    pub expiration: i32,
    pub strike: i32,
    pub right: i32,
    pub strike_price_type: i32,
}

/// Price tick. Generic price data point.
#[derive(Debug, Clone, Copy)]
#[repr(C, align(64))]
pub struct PriceTick {
    pub ms_of_day: i32,
    pub price: i32,
    pub price_type: i32,
    pub date: i32,
}

/// Quote tick. NBBO quote data.
#[derive(Debug, Clone, Copy)]
#[repr(C, align(64))]
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

/// Snapshot trade tick. Abbreviated trade for snapshots.
#[derive(Debug, Clone, Copy)]
pub struct SnapshotTradeTick {
    pub ms_of_day: i32,
    pub sequence: i32,
    pub size: i32,
    pub condition: i32,
    pub price: i32,
    pub price_type: i32,
    pub date: i32,
}

/// Combined trade + quote tick.
#[derive(Debug, Clone, Copy)]
pub struct TradeQuoteTick {
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
    pub price_type: i32,
    pub date: i32,
}

/// Trade tick. Core unit of trade data.
#[derive(Debug, Clone, Copy)]
#[repr(C, align(64))]
pub struct TradeTick {
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
    pub price_type: i32,
    pub date: i32,
}

// ─────────────────────────────────────────────────────────────────────────────
//  Hand-written impl blocks
// ─────────────────────────────────────────────────────────────────────────────

impl TradeTick {
    #[inline]
    pub fn get_price(&self) -> Price {
        Price::new(self.price, self.price_type)
    }

    pub fn is_cancelled(&self) -> bool {
        flags::trade::CANCELLED_RANGE.contains(&self.condition)
    }

    pub fn trade_condition_no_last(&self) -> bool {
        self.condition_flags & flags::condition_flags::NO_LAST == flags::condition_flags::NO_LAST
    }

    pub fn price_condition_set_last(&self) -> bool {
        self.price_flags & flags::price_flags::SET_LAST == flags::price_flags::SET_LAST
    }

    pub fn is_incremental_volume(&self) -> bool {
        self.volume_type == flags::volume::INCREMENTAL
    }

    /// Regular trading hours: 9:30 AM - 4:00 PM ET.
    pub fn regular_trading_hours(&self) -> bool {
        (flags::trade::RTH_START_MS..=flags::trade::RTH_END_MS).contains(&self.ms_of_day)
    }

    pub fn is_seller(&self) -> bool {
        self.ext_condition1 == flags::trade::SELLER_CONDITION
    }
}

impl QuoteTick {
    #[inline]
    pub fn bid_price(&self) -> Price {
        Price::new(self.bid, self.price_type)
    }

    #[inline]
    pub fn ask_price(&self) -> Price {
        Price::new(self.ask, self.price_type)
    }

    pub fn midpoint_value(&self) -> i32 {
        self.bid / 2 + self.ask / 2 + (self.bid % 2 + self.ask % 2) / 2
    }

    #[inline]
    pub fn midpoint_price(&self) -> Price {
        Price::new(self.midpoint_value(), self.price_type)
    }
}

impl OhlcTick {
    #[inline]
    pub fn open_price(&self) -> Price {
        Price::new(self.open, self.price_type)
    }
    #[inline]
    pub fn high_price(&self) -> Price {
        Price::new(self.high, self.price_type)
    }
    #[inline]
    pub fn low_price(&self) -> Price {
        Price::new(self.low, self.price_type)
    }
    #[inline]
    pub fn close_price(&self) -> Price {
        Price::new(self.close, self.price_type)
    }
}

impl EodTick {
    #[inline]
    pub fn open_price(&self) -> Price {
        Price::new(self.open, self.price_type)
    }
    #[inline]
    pub fn high_price(&self) -> Price {
        Price::new(self.high, self.price_type)
    }
    #[inline]
    pub fn low_price(&self) -> Price {
        Price::new(self.low, self.price_type)
    }
    #[inline]
    pub fn close_price(&self) -> Price {
        Price::new(self.close, self.price_type)
    }
    #[inline]
    pub fn bid_price(&self) -> Price {
        Price::new(self.bid, self.price_type)
    }
    #[inline]
    pub fn ask_price(&self) -> Price {
        Price::new(self.ask, self.price_type)
    }
    #[inline]
    pub fn midpoint_value(&self) -> i32 {
        self.bid / 2 + self.ask / 2 + (self.bid % 2 + self.ask % 2) / 2
    }
}

impl SnapshotTradeTick {
    #[inline]
    pub fn get_price(&self) -> Price {
        Price::new(self.price, self.price_type)
    }
}

impl TradeQuoteTick {
    #[inline]
    pub fn trade_price(&self) -> Price {
        Price::new(self.price, self.price_type)
    }
    #[inline]
    pub fn bid_price(&self) -> Price {
        Price::new(self.bid, self.price_type)
    }
    #[inline]
    pub fn ask_price(&self) -> Price {
        Price::new(self.ask, self.price_type)
    }
}

impl PriceTick {
    #[inline]
    pub fn get_price(&self) -> Price {
        Price::new(self.price, self.price_type)
    }
}
