//! Per-contract OHLCVC accumulator and price-type scaling helper.
//!
//! Mirrors the Java terminal's `OHLCVC` tick aggregation: an accumulator is
//! seeded either from a server-sent OHLCVC bar (code 24) or from the first
//! trade, then each subsequent trade advances the open/high/low/close and
//! bumps `volume` + `count`. Prices sent with a different `price_type` than
//! the accumulator's are rescaled via [`change_price_type`].

/// Per-contract OHLCVC accumulator, updated on every Trade event.
///
/// `volume` and `count` use `i64` because they accumulate across many trades
/// and can exceed `i32::MAX` for high-volume symbols (e.g. SPY). The Java
/// terminal uses `int` (32-bit) but silently wraps on overflow; we use `i64`
/// to avoid overflow entirely.
pub(super) struct OhlcvcAccumulator {
    pub(super) open: i32,
    pub(super) high: i32,
    pub(super) low: i32,
    pub(super) close: i32,
    pub(super) volume: i64,
    pub(super) count: i64,
    pub(super) price_type: i32,
    pub(super) date: i32,
    pub(super) ms_of_day: i32,
    pub(super) initialized: bool,
}

impl OhlcvcAccumulator {
    pub(super) fn new() -> Self {
        Self {
            open: 0,
            high: 0,
            low: 0,
            close: 0,
            volume: 0,
            count: 0,
            price_type: 0,
            date: 0,
            ms_of_day: 0,
            initialized: false,
        }
    }

    #[allow(clippy::too_many_arguments)] // Reason: OHLCVC bar has many fields from server init message
    pub(super) fn init_from_server(
        &mut self,
        ms_of_day: i32,
        open: i32,
        high: i32,
        low: i32,
        close: i32,
        volume: i32,
        count: i32,
        price_type: i32,
        date: i32,
    ) {
        self.ms_of_day = ms_of_day;
        self.open = open;
        self.high = high;
        self.low = low;
        self.close = close;
        self.volume = i64::from(volume);
        self.count = i64::from(count);
        self.price_type = price_type;
        self.date = date;
        self.initialized = true;
    }

    pub(super) fn process_trade(
        &mut self,
        ms_of_day: i32,
        price: i32,
        size: i32,
        price_type: i32,
        date: i32,
    ) {
        if self.initialized {
            self.ms_of_day = ms_of_day;
            let adjusted_price = change_price_type(price, price_type, self.price_type);
            self.volume += i64::from(size);
            self.count += 1;
            if adjusted_price > self.high {
                self.high = adjusted_price;
            }
            if adjusted_price < self.low {
                self.low = adjusted_price;
            }
            self.close = adjusted_price;
        } else {
            self.open = price;
            self.high = price;
            self.low = price;
            self.close = price;
            self.volume = i64::from(size);
            self.count = 1;
            self.price_type = price_type;
            self.date = date;
            self.ms_of_day = ms_of_day;
            self.initialized = true;
        }
    }
}

/// Convert a price from one `price_type` to another (mirrors Java PriceCalcUtils.changePriceType).
// Reason: protocol-defined integer widths from Java FPSS specification.
#[allow(clippy::cast_possible_truncation)]
pub(super) fn change_price_type(price: i32, price_type: i32, new_price_type: i32) -> i32 {
    const POW10: [i32; 10] = [
        1,
        10,
        100,
        1_000,
        10_000,
        100_000,
        1_000_000,
        10_000_000,
        100_000_000,
        1_000_000_000,
    ];
    if price == 0 || price_type == new_price_type {
        return price;
    }
    let exp = new_price_type - price_type;
    if exp <= 0 {
        let idx = usize::try_from(-exp).unwrap_or(0);
        if idx < POW10.len() {
            price * POW10[idx]
        } else {
            price
        }
    } else {
        let idx = usize::try_from(exp).unwrap_or(0);
        if idx < POW10.len() {
            price / POW10[idx]
        } else {
            0
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ohlcvc_accumulator_first_trade_initializes() {
        let mut acc = OhlcvcAccumulator::new();
        assert!(!acc.initialized);
        acc.process_trade(34200000, 15025, 100, 8, 20240315);
        assert!(acc.initialized);
        assert_eq!(acc.open, 15025);
        assert_eq!(acc.high, 15025);
        assert_eq!(acc.low, 15025);
        assert_eq!(acc.close, 15025);
        assert_eq!(acc.volume, 100);
        assert_eq!(acc.count, 1);
    }

    #[test]
    fn ohlcvc_accumulator_updates() {
        let mut acc = OhlcvcAccumulator::new();
        acc.process_trade(34200000, 15025, 100, 8, 20240315);
        acc.process_trade(34200100, 15100, 200, 8, 20240315);
        acc.process_trade(34200200, 14950, 50, 8, 20240315);
        assert_eq!(acc.open, 15025);
        assert_eq!(acc.high, 15100);
        assert_eq!(acc.low, 14950);
        assert_eq!(acc.close, 14950);
        assert_eq!(acc.volume, 350);
        assert_eq!(acc.count, 3);
    }

    #[test]
    fn ohlcvc_accumulator_server_init_then_trade() {
        let mut acc = OhlcvcAccumulator::new();
        acc.init_from_server(34200000, 15000, 15100, 14900, 15050, 1000, 10, 8, 20240315);
        acc.process_trade(34200300, 15200, 50, 8, 20240315);
        assert_eq!(acc.high, 15200);
        assert_eq!(acc.low, 14900);
        assert_eq!(acc.volume, 1050);
        assert_eq!(acc.count, 11);
    }

    #[test]
    fn ohlcvc_accumulator_no_overflow_on_high_volume() {
        let mut acc = OhlcvcAccumulator::new();
        acc.process_trade(34200000, 15025, i32::MAX, 8, 20240315);
        acc.process_trade(34200100, 15100, i32::MAX, 8, 20240315);
        // Would overflow i32 (2 * 2_147_483_647 = 4_294_967_294), fine in i64
        assert_eq!(acc.volume, 2 * i64::from(i32::MAX));
        assert_eq!(acc.count, 2);
    }

    #[test]
    fn change_price_type_tests() {
        assert_eq!(change_price_type(15025, 8, 8), 15025);
        assert_eq!(change_price_type(15025, 8, 7), 150250);
        assert_eq!(change_price_type(150250, 7, 8), 15025);
        assert_eq!(change_price_type(0, 8, 7), 0);
    }
}
