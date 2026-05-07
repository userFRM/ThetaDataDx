//! Wire-to-application latency computation for FPSS events.
//!
//! Converts the exchange-side `ms_of_day` (Eastern Time) and `event_date`
//! (YYYYMMDD) into epoch nanoseconds, then subtracts from the local
//! `received_at_ns` wall-clock timestamp captured at frame decode time.
//!
//! Civil-date / DST primitives live in [`crate::time`]; this module is a
//! thin wrapper that adds the YYYYMMDD-and-`ms_of_day` decomposition.

use crate::time::{civil_to_epoch_days, eastern_offset_ms};

/// Compute wire-to-application latency in nanoseconds.
///
/// # Arguments
///
/// * `exchange_ms_of_day` -- milliseconds since midnight ET from the tick
/// * `event_date` -- YYYYMMDD integer from the tick
/// * `received_at_ns` -- nanoseconds since UNIX epoch from `FpssData.received_at_ns`
///
/// # Returns
///
/// Latency in nanoseconds. May be negative if clocks are skewed (exchange
/// timestamp ahead of local wall clock).
// Reason: epoch timestamps may exceed i64::MAX in the far future; wrapping is acceptable
// for the valid date range of market data (1970--2100).
#[allow(clippy::cast_possible_wrap)]
#[must_use]
pub fn latency_ns(exchange_ms_of_day: i32, event_date: i32, received_at_ns: u64) -> i64 {
    let exchange_epoch_ns = exchange_epoch_ns(exchange_ms_of_day, event_date);
    received_at_ns as i64 - exchange_epoch_ns
}

/// Convert `event_date` (YYYYMMDD) + `ms_of_day` (Eastern Time) to epoch nanoseconds.
// Reason: date components are from valid YYYYMMDD integers; casts are safe for the valid range.
#[allow(clippy::cast_sign_loss)]
fn exchange_epoch_ns(ms_of_day: i32, date_yyyymmdd: i32) -> i64 {
    let year = date_yyyymmdd / 10_000;
    let month = ((date_yyyymmdd % 10_000) / 100) as u32;
    let day = (date_yyyymmdd % 100) as u32;

    let days = civil_to_epoch_days(year, month, day);
    // Midnight UTC for that civil date
    let midnight_utc_ms = days * 86_400_000;

    // The ms_of_day is in Eastern Time. We need the UTC offset for this date.
    // Use the midday point (ms_of_day ~ 12:00) to determine DST status,
    // since market hours (09:30-16:00 ET) are unambiguous for DST.
    let approx_utc_ms = midnight_utc_ms + i64::from(ms_of_day) + 5 * 3_600 * 1_000; // rough EST guess
    let offset_ms = eastern_offset_ms(approx_utc_ms as u64);

    // exchange_epoch_ms = midnight_utc_ms + ms_of_day - offset_ms
    // (offset_ms is negative, e.g. -5h for EST, so subtracting it adds hours)
    let exchange_epoch_ms = midnight_utc_ms + i64::from(ms_of_day) - offset_ms;
    exchange_epoch_ms * 1_000_000
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn latency_basic_est() {
        // 2024-01-15 is a Monday in EST (UTC-5).
        // 09:30 AM ET = 34_200_000 ms_of_day
        // 09:30 AM ET = 14:30 UTC = 14*3600 + 30*60 = 52200 seconds since midnight UTC
        // Epoch seconds for 2024-01-15 00:00:00 UTC:
        let days = civil_to_epoch_days(2024, 1, 15);
        let midnight_utc_ns = days * 86_400_000 * 1_000_000;
        // 14:30 UTC in nanoseconds offset
        let utc_offset_ns = (14 * 3600 + 30 * 60) as i64 * 1_000_000_000;
        let received_at_ns = (midnight_utc_ns + utc_offset_ns) as u64;

        // If received_at_ns == exchange epoch time, latency should be ~0.
        let lat = latency_ns(34_200_000, 20240115, received_at_ns);
        assert!(
            lat.abs() < 1_000_000, // less than 1ms rounding
            "expected ~0 latency, got {lat} ns"
        );
    }

    #[test]
    fn latency_basic_edt() {
        // 2024-06-15 is in EDT (UTC-4).
        // 09:30 AM ET = 34_200_000 ms_of_day
        // 09:30 AM ET = 13:30 UTC
        let days = civil_to_epoch_days(2024, 6, 15);
        let midnight_utc_ns = days * 86_400_000 * 1_000_000;
        let utc_offset_ns = (13 * 3600 + 30 * 60) as i64 * 1_000_000_000;
        let received_at_ns = (midnight_utc_ns + utc_offset_ns) as u64;

        let lat = latency_ns(34_200_000, 20240615, received_at_ns);
        assert!(lat.abs() < 1_000_000, "expected ~0 latency, got {lat} ns");
    }

    #[test]
    fn latency_positive_when_received_later() {
        // Receive 100ms after the exchange timestamp.
        let days = civil_to_epoch_days(2024, 1, 15);
        let midnight_utc_ns = days * 86_400_000 * 1_000_000;
        let utc_offset_ns = (14 * 3600 + 30 * 60) as i64 * 1_000_000_000;
        let received_at_ns = (midnight_utc_ns + utc_offset_ns) as u64 + 100_000_000; // +100ms

        let lat = latency_ns(34_200_000, 20240115, received_at_ns);
        // Should be ~100ms = 100_000_000 ns
        assert!(
            (lat - 100_000_000).abs() < 1_000_000,
            "expected ~100ms latency, got {lat} ns"
        );
    }

    #[test]
    fn latency_negative_when_clock_skewed() {
        // Receive 50ms BEFORE the exchange timestamp (clock skew).
        let days = civil_to_epoch_days(2024, 1, 15);
        let midnight_utc_ns = days * 86_400_000 * 1_000_000;
        let utc_offset_ns = (14 * 3600 + 30 * 60) as i64 * 1_000_000_000;
        let received_at_ns = (midnight_utc_ns + utc_offset_ns) as u64 - 50_000_000; // -50ms

        let lat = latency_ns(34_200_000, 20240115, received_at_ns);
        assert!(
            (lat + 50_000_000).abs() < 1_000_000,
            "expected ~-50ms latency, got {lat} ns"
        );
    }
}
