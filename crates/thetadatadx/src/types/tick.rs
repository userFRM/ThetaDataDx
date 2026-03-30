// Struct definitions are generated from endpoint_schema.toml by build.rs.
include!(concat!(env!("OUT_DIR"), "/tick_generated.rs"));

// ─────────────────────────────────────────────────────────────────────────────
//  Hand-written impl blocks on generated types
// ─────────────────────────────────────────────────────────────────────────────

impl TradeTick {
    #[inline]
    pub fn get_price(&self) -> Price {
        Price::new(self.price, self.price_type)
    }

    pub fn is_cancelled(&self) -> bool {
        (40..=44).contains(&self.condition)
    }

    pub fn trade_condition_no_last(&self) -> bool {
        self.condition_flags & 1 == 1
    }

    pub fn price_condition_set_last(&self) -> bool {
        self.price_flags & 1 == 1
    }

    pub fn is_incremental_volume(&self) -> bool {
        self.volume_type == 0
    }

    /// Regular trading hours: 9:30 AM - 4:00 PM ET.
    pub fn regular_trading_hours(&self) -> bool {
        (34_200_000..=57_600_000).contains(&self.ms_of_day)
    }

    pub fn is_seller(&self) -> bool {
        self.ext_condition1 == 12
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
