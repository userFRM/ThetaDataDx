//! Wire-to-application latency computation for FPSS events.
//!
//! Converts the exchange-side `ms_of_day` (Eastern Time) and `event_date`
//! (YYYYMMDD) into epoch nanoseconds, then subtracts from the local
//! `received_at_ns` wall-clock timestamp captured at frame decode time.
//!
//! No external timezone crate -- uses the same civil-date math and US DST
//! rules (Energy Policy Act 2005) as `thetadatadx::decode`.

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
// reason: Date/time math on exchange wire values requires i32->i64 and u64->i64
// casts throughout. These values are bounded by calendar dates and wall-clock
// nanoseconds, making overflow impossible in practice.
#[allow(
    clippy::cast_possible_wrap,
    clippy::cast_sign_loss,
    clippy::cast_possible_truncation
)]
#[must_use]
pub fn latency_ns(exchange_ms_of_day: i32, event_date: i32, received_at_ns: u64) -> i64 {
    let exchange_epoch_ns = exchange_epoch_ns(exchange_ms_of_day, event_date);
    received_at_ns as i64 - exchange_epoch_ns
}

/// Convert `event_date` (YYYYMMDD) + `ms_of_day` (Eastern Time) to epoch nanoseconds.
// reason: Calendar arithmetic on YYYYMMDD wire values requires i32->i64 and u32
// casts. Values are bounded by valid dates (year ~2000-2100).
#[allow(
    clippy::cast_possible_wrap,
    clippy::cast_sign_loss,
    clippy::cast_possible_truncation
)]
fn exchange_epoch_ns(ms_of_day: i32, date_yyyymmdd: i32) -> i64 {
    let year = date_yyyymmdd / 10000;
    let month = ((date_yyyymmdd % 10000) / 100) as u32;
    let day = (date_yyyymmdd % 100) as u32;

    let days = civil_to_epoch_days(year, month, day);
    // Midnight UTC for that civil date
    let midnight_utc_ms = days * 86_400_000;

    // The ms_of_day is in Eastern Time. We need the UTC offset for this date.
    // Use the midday point (ms_of_day ~ 12:00) to determine DST status,
    // since market hours (09:30-16:00 ET) are unambiguous for DST.
    let approx_utc_ms = midnight_utc_ms + i64::from(ms_of_day) + 5 * 3600 * 1000; // rough EST guess
    let offset_ms = eastern_offset_ms(approx_utc_ms as u64);

    // exchange_epoch_ms = midnight_utc_ms + ms_of_day - offset_ms
    // (offset_ms is negative, e.g. -5h for EST, so subtracting it adds hours)
    let exchange_epoch_ms = midnight_utc_ms + i64::from(ms_of_day) - offset_ms;
    exchange_epoch_ms * 1_000_000
}

// ---------------------------------------------------------------------------
//  Civil-date / DST helpers (same algorithm as thetadatadx::decode)
// ---------------------------------------------------------------------------

/// Convert civil date to days since 1970-01-01 (Euclidean algorithm).
// reason: Euclidean calendar algorithm requires mixed i64/u64 arithmetic
// on year/month/day values bounded by valid civil dates.
#[allow(clippy::cast_possible_wrap, clippy::cast_sign_loss)]
fn civil_to_epoch_days(year: i32, month: u32, day: u32) -> i64 {
    let y = if month <= 2 {
        i64::from(year) - 1
    } else {
        i64::from(year)
    };
    let m = if month <= 2 {
        i64::from(month) + 9
    } else {
        i64::from(month) - 3
    };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = (y - era * 400) as u64;
    let doy = (153 * m as u64 + 2) / 5 + u64::from(day) - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe as i64 - 719_468
}

/// Eastern Time UTC offset in milliseconds for a given `epoch_ms`.
///
/// US DST rule (Energy Policy Act of 2005):
/// - EDT (UTC-4): second Sunday of March 2:00 AM local -> first Sunday of November 2:00 AM local
/// - EST (UTC-5): rest of the year
// reason: DST detection requires converting between epoch milliseconds (u64/i64)
// and civil date components (u32/i32). Values are bounded by valid calendar dates.
#[allow(
    clippy::cast_possible_wrap,
    clippy::cast_sign_loss,
    clippy::cast_possible_truncation
)]
fn eastern_offset_ms(epoch_ms: u64) -> i64 {
    let epoch_secs = epoch_ms as i64 / 1000;
    let days_since_epoch = epoch_secs / 86400;

    // Civil date from days since 1970-01-01.
    let z = days_since_epoch + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u32;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let year = yoe as i32 + (era * 400) as i32;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = if month <= 2 { year + 1 } else { year };

    let dst_start_utc = march_second_sunday_utc(year);
    let dst_end_utc = november_first_sunday_utc(year);

    let epoch_ms_i64 = epoch_ms as i64;
    if epoch_ms_i64 >= dst_start_utc && epoch_ms_i64 < dst_end_utc {
        -4 * 3600 * 1000 // EDT
    } else {
        -5 * 3600 * 1000 // EST
    }
}

/// Epoch ms of the second Sunday of March at 7:00 AM UTC (= 2:00 AM EST).
fn march_second_sunday_utc(year: i32) -> i64 {
    let mar1 = civil_to_epoch_days(year, 3, 1);
    let dow = ((mar1 + 3) % 7 + 7) % 7; // 0=Mon..6=Sun
    let days_to_first_sunday = (6 - dow + 7) % 7;
    let second_sunday = mar1 + days_to_first_sunday + 7;
    second_sunday * 86_400_000 + 7 * 3600 * 1000
}

/// Epoch ms of the first Sunday of November at 6:00 AM UTC (= 2:00 AM EDT).
fn november_first_sunday_utc(year: i32) -> i64 {
    let nov1 = civil_to_epoch_days(year, 11, 1);
    let dow = ((nov1 + 3) % 7 + 7) % 7;
    let days_to_first_sunday = (6 - dow + 7) % 7;
    let first_sunday = nov1 + days_to_first_sunday;
    first_sunday * 86_400_000 + 6 * 3600 * 1000
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
