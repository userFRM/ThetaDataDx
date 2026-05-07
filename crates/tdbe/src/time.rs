//! Eastern Time + DST primitives.
//!
//! Canonical Eastern-time conversion module reused by `thetadatadx` (mdds
//! decode + flatfiles) and the `tdbe` latency path. No external timezone
//! crate dependencies — pure civil-date arithmetic with the documented US
//! DST rules.
//!
//! ## DST rules
//!
//! **2007-onward** (Energy Policy Act of 2005):
//! - EDT (UTC-4): second Sunday of March at 2:00 AM local -> first Sunday
//!   of November at 2:00 AM local
//! - EST (UTC-5): rest of the year
//!
//! **Before 2007** (Uniform Time Act of 1966):
//! - EDT (UTC-4): first Sunday of April at 2:00 AM local -> last Sunday of
//!   October at 2:00 AM local
//! - EST (UTC-5): rest of the year
//!
//! Transition points are computed in UTC and compared, so callers do not
//! need to round-trip through a timezone library.

/// Eastern Time UTC offset in milliseconds for a given `epoch_ms`.
///
/// Returns `-4 * 3_600_000` (EDT) when DST is in effect for the civil
/// year of `epoch_ms`; otherwise `-5 * 3_600_000` (EST). DST window
/// selection follows the rules documented at the module level.
// Reason: the Euclidean date algorithm uses intentional signed/unsigned conversions for valid epoch timestamps.
#[allow(
    clippy::cast_possible_wrap,
    clippy::cast_sign_loss,
    clippy::cast_possible_truncation
)]
#[must_use]
pub fn eastern_offset_ms(epoch_ms: u64) -> i64 {
    // First, determine the UTC year/month/day to find DST boundaries.
    let epoch_secs = epoch_ms as i64 / 1_000;
    let days_since_epoch = epoch_secs / 86_400;

    // Civil date from days since 1970-01-01 (Euclidean algorithm).
    let z = days_since_epoch + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u32;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let year = yoe as i32 + (era * 400) as i32;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = if month <= 2 { year + 1 } else { year };

    let (dst_start_utc, dst_end_utc) = if year >= 2007 {
        // Post-2007: second Sunday of March -> first Sunday of November.
        (
            march_second_sunday_utc(year),
            november_first_sunday_utc(year),
        )
    } else {
        // Pre-2007: first Sunday of April -> last Sunday of October.
        (april_first_sunday_utc(year), october_last_sunday_utc(year))
    };

    let epoch_ms_i64 = epoch_ms as i64;
    if epoch_ms_i64 >= dst_start_utc && epoch_ms_i64 < dst_end_utc {
        -4 * 3_600 * 1_000 // EDT
    } else {
        -5 * 3_600 * 1_000 // EST
    }
}

/// Epoch ms of the second Sunday of March at 7:00 AM UTC (= 2:00 AM EST).
#[must_use]
pub fn march_second_sunday_utc(year: i32) -> i64 {
    // March 1 day-of-week, then find second Sunday.
    let mar1 = civil_to_epoch_days(year, 3, 1);
    // 1970-01-01 is Thursday. (days + 3) % 7 gives 0=Mon..6=Sun.
    let dow = ((mar1 + 3) % 7 + 7) % 7;
    let days_to_first_sunday = (6 - dow + 7) % 7; // days from Mar 1 to first Sunday
    let second_sunday = mar1 + days_to_first_sunday + 7; // second Sunday
    second_sunday * 86_400_000 + 7 * 3_600 * 1_000 // 7:00 AM UTC = 2:00 AM EST
}

/// Epoch ms of the first Sunday of November at 6:00 AM UTC (= 2:00 AM EDT).
#[must_use]
pub fn november_first_sunday_utc(year: i32) -> i64 {
    let nov1 = civil_to_epoch_days(year, 11, 1);
    let dow = ((nov1 + 3) % 7 + 7) % 7;
    let days_to_first_sunday = (6 - dow + 7) % 7;
    let first_sunday = nov1 + days_to_first_sunday;
    first_sunday * 86_400_000 + 6 * 3_600 * 1_000 // 6:00 AM UTC = 2:00 AM EDT
}

/// Epoch ms of the first Sunday of April at 7:00 AM UTC (= 2:00 AM EST).
///
/// Used for pre-2007 DST start (Uniform Time Act of 1966).
#[must_use]
pub fn april_first_sunday_utc(year: i32) -> i64 {
    let apr1 = civil_to_epoch_days(year, 4, 1);
    let dow = ((apr1 + 3) % 7 + 7) % 7;
    let days_to_first_sunday = (6 - dow + 7) % 7;
    let first_sunday = apr1 + days_to_first_sunday;
    first_sunday * 86_400_000 + 7 * 3_600 * 1_000 // 7:00 AM UTC = 2:00 AM EST
}

/// Epoch ms of the last Sunday of October at 6:00 AM UTC (= 2:00 AM EDT).
///
/// Used for pre-2007 DST end (Uniform Time Act of 1966).
#[must_use]
pub fn october_last_sunday_utc(year: i32) -> i64 {
    // Start from October 31 and walk back to find the last Sunday.
    let oct31 = civil_to_epoch_days(year, 10, 31);
    let dow = ((oct31 + 3) % 7 + 7) % 7; // 0=Mon..6=Sun
    let days_back = (dow + 1) % 7; // days back from Oct 31 to last Sunday
    let last_sunday = oct31 - days_back;
    last_sunday * 86_400_000 + 6 * 3_600 * 1_000 // 6:00 AM UTC = 2:00 AM EDT
}

/// Convert civil date to days since 1970-01-01 (inverse of the Euclidean algorithm).
// Reason: the Euclidean date algorithm uses intentional signed/unsigned conversions for valid calendar dates.
#[allow(clippy::cast_sign_loss, clippy::cast_possible_wrap)]
#[must_use]
pub fn civil_to_epoch_days(year: i32, month: u32, day: u32) -> i64 {
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

/// Convert `epoch_ms` to milliseconds-of-day in Eastern Time (DST-aware).
// Reason: ms_of_day fits in i32; epoch_ms is in valid market data range.
#[allow(clippy::cast_possible_wrap, clippy::cast_possible_truncation)]
#[must_use]
pub fn timestamp_to_ms_of_day(epoch_ms: u64) -> i32 {
    let offset = eastern_offset_ms(epoch_ms);
    let local_ms = epoch_ms as i64 + offset;
    (local_ms.rem_euclid(86_400_000)) as i32
}

/// Convert `epoch_ms` to YYYYMMDD date integer in Eastern Time (DST-aware).
// Reason: date components fit in i32; epoch_ms is in valid market data range.
#[allow(
    clippy::cast_possible_wrap,
    clippy::cast_sign_loss,
    clippy::cast_possible_truncation
)]
#[must_use]
pub fn timestamp_to_date(epoch_ms: u64) -> i32 {
    let offset = eastern_offset_ms(epoch_ms);
    let local_secs = (epoch_ms as i64 + offset) / 1_000;
    let days = local_secs / 86400 + 719_468;
    let era = if days >= 0 { days } else { days - 146_096 } / 146_097;
    let doe = (days - era * 146_097) as u32;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let y = i64::from(yoe) + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    (y as i32) * 10_000 + (m as i32) * 100 + (d as i32)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    // Reason: ms_of_day fits in i32; epoch_ms is in valid market data range.
    #[allow(clippy::cast_possible_wrap, clippy::cast_possible_truncation)]
    fn timestamp_to_ms_of_day_edt() {
        // 2026-04-01 09:30:00 ET (EDT, UTC-4) = 2026-04-01 13:30:00 UTC
        let epoch_ms: u64 = 1_775_050_200_000; // Apr 1 2026, 13:30 UTC
        let ms = timestamp_to_ms_of_day(epoch_ms);
        assert_eq!(ms, 34_200_000, "9:30 AM ET in milliseconds");
    }

    #[test]
    // Reason: ms_of_day fits in i32; epoch_ms is in valid market data range.
    #[allow(clippy::cast_possible_wrap, clippy::cast_possible_truncation)]
    fn timestamp_to_ms_of_day_est() {
        // 2026-01-15 09:30:00 ET (EST, UTC-5) = 2026-01-15 14:30:00 UTC
        let epoch_ms: u64 = 1_768_487_400_000;
        let ms = timestamp_to_ms_of_day(epoch_ms);
        assert_eq!(ms, 34_200_000, "9:30 AM ET in milliseconds (winter)");
    }

    #[test]
    fn timestamp_to_date_edt() {
        let epoch_ms: u64 = 1_775_050_200_000; // Apr 1 2026, 13:30 UTC
        let date = timestamp_to_date(epoch_ms);
        assert_eq!(date, 20260401);
    }

    #[test]
    fn timestamp_to_date_est() {
        let epoch_ms: u64 = 1_768_487_400_000; // Jan 15 2026, 14:30 UTC
        let date = timestamp_to_date(epoch_ms);
        assert_eq!(date, 20260115);
    }

    #[test]
    fn dst_transition_march_2026() {
        // 2026 DST starts March 8 (second Sunday of March)
        // Before: EST (UTC-5) at 06:59 UTC. After: EDT (UTC-4) at 07:01 UTC.
        let before: u64 = 1_772_953_140_000; // Mar 8 2026, 06:59 UTC
        assert_eq!(eastern_offset_ms(before), -5 * 3_600 * 1_000);
        let after: u64 = 1_772_953_260_000; // Mar 8 2026, 07:01 UTC
        assert_eq!(eastern_offset_ms(after), -4 * 3_600 * 1_000);
    }

    #[test]
    fn pre2007_dst_summer_uses_old_rules() {
        // 2006: old rules apply (first Sunday April -> last Sunday October).
        // 2006-07-15 18:00:00 UTC = 2006-07-15 14:00:00 EDT (summer, mid-July).
        // This is well within DST under both old and new rules, so EDT (UTC-4).
        let epoch_ms: u64 = 1_153_065_600_000; // Jul 15 2006, 18:00 UTC
        assert_eq!(
            eastern_offset_ms(epoch_ms),
            -4 * 3_600 * 1_000,
            "mid-July 2006 should be EDT under old DST rules"
        );
    }

    #[test]
    fn pre2007_est_before_april_dst_start() {
        // 2006: old rules — DST starts first Sunday of April (April 2, 2006).
        // 2006-02-15 15:00:00 UTC = 2006-02-15 10:00:00 EST (winter, mid-Feb).
        let epoch_ms: u64 = 1_140_015_600_000; // Feb 15 2006, 15:00 UTC
        assert_eq!(
            eastern_offset_ms(epoch_ms),
            -5 * 3_600 * 1_000,
            "mid-February 2006 should be EST under old DST rules"
        );
    }

    // ---------------------------------------------------------------------------
    // Property-based tests
    // ---------------------------------------------------------------------------
    //
    // Three invariants:
    //
    //   1. `civil_to_epoch_days` is monotone over the full
    //      `[1970-01-01, 2099-12-31]` calendar range — a strictly later
    //      civil date never returns a smaller day count.
    //   2. `eastern_offset_ms` returns exactly one of two values
    //      (`-5*3_600_000` or `-4*3_600_000`) for any timestamp in
    //      `[2000-01-01 UTC, 2099-12-31 UTC]`.
    //   3. DST cutover sanity: at the spring-forward boundary, `+1 ms`
    //      already returns EDT; at the fall-back boundary, `-1 ms` is
    //      still EDT. Asserted across a sweep of years that covers both
    //      the pre-2007 and post-2007 rule windows.

    use proptest::prelude::*;

    /// Strategy for `(year, month, day)` triples in the
    /// `[1970-01-01, 2099-12-31]` range. Days are clamped per-month so
    /// every emitted triple is a valid civil date.
    fn arbitrary_civil_date() -> impl Strategy<Value = (i32, u32, u32)> {
        (1970i32..=2099, 1u32..=12).prop_flat_map(|(y, m)| {
            // Days in month, accounting for leap years.
            let dim = match m {
                1 | 3 | 5 | 7 | 8 | 10 | 12 => 31u32,
                4 | 6 | 9 | 11 => 30,
                2 => {
                    let leap = (y % 4 == 0 && y % 100 != 0) || y % 400 == 0;
                    if leap {
                        29
                    } else {
                        28
                    }
                }
                _ => unreachable!(),
            };
            (Just(y), Just(m), 1u32..=dim)
        })
    }

    /// Strategy for an epoch_ms timestamp in [2000-01-01 UTC, 2099-12-31 UTC].
    // Reason: the upper bound lies well within u64 range.
    #[allow(clippy::cast_sign_loss)]
    fn arbitrary_epoch_ms_2000_2099() -> impl Strategy<Value = u64> {
        // 2000-01-01 00:00:00 UTC = 946_684_800_000 ms
        // 2099-12-31 23:59:59 UTC = 4_102_444_799_000 ms
        946_684_800_000u64..=4_102_444_799_000u64
    }

    proptest! {
        /// `civil_to_epoch_days` monotonicity over 1970..=2099.
        ///
        /// For any two valid civil dates `(y1, m1, d1) <= (y2, m2, d2)`
        /// (lexicographic order), the day-count of the second is
        /// `>=` the day-count of the first. Asserted by drawing two
        /// independent dates and ordering them lexicographically.
        #[test]
        fn civil_to_epoch_days_monotone(
            a in arbitrary_civil_date(),
            b in arbitrary_civil_date(),
        ) {
            let (lo, hi) = if a <= b { (a, b) } else { (b, a) };
            let lo_days = civil_to_epoch_days(lo.0, lo.1, lo.2);
            let hi_days = civil_to_epoch_days(hi.0, hi.1, hi.2);
            prop_assert!(
                hi_days >= lo_days,
                "monotonicity violated: {:?} -> {} but {:?} -> {}",
                lo, lo_days, hi, hi_days,
            );
        }

        /// `eastern_offset_ms` returns either `-5*3_600_000` (EST) or
        /// `-4*3_600_000` (EDT) for any timestamp in 2000..=2099.
        #[test]
        fn eastern_offset_only_returns_est_or_edt(epoch_ms in arbitrary_epoch_ms_2000_2099()) {
            let off = eastern_offset_ms(epoch_ms);
            prop_assert!(
                off == -5 * 3_600 * 1_000 || off == -4 * 3_600 * 1_000,
                "eastern_offset_ms returned unexpected value {off} for epoch_ms {epoch_ms}",
            );
        }

        /// DST cutover sanity:
        ///   * at the spring-forward instant, `+1 ms` already returns EDT;
        ///   * at the fall-back instant, `-1 ms` still returns EDT.
        /// Asserted across both the pre-2007 (Uniform Time Act) and
        /// post-2007 (Energy Policy Act) rule windows by sweeping years
        /// 1990..=2099.
        #[test]
        fn dst_cutover_boundaries(year in 1990i32..=2099) {
            let (start_utc, end_utc) = if year >= 2007 {
                (march_second_sunday_utc(year), november_first_sunday_utc(year))
            } else {
                (april_first_sunday_utc(year), october_last_sunday_utc(year))
            };

            // Spring forward: just after the start instant must be EDT.
            // Reason: start_utc/end_utc are positive year-specific epoch ms.
            #[allow(clippy::cast_sign_loss)]
            let after_spring = (start_utc as u64) + 1;
            prop_assert_eq!(
                eastern_offset_ms(after_spring),
                -4 * 3_600 * 1_000,
                "expected EDT immediately after spring-forward boundary in {}",
                year,
            );

            // Fall back: just before the end instant is still EDT.
            #[allow(clippy::cast_sign_loss)]
            let before_fall = (end_utc as u64) - 1;
            prop_assert_eq!(
                eastern_offset_ms(before_fall),
                -4 * 3_600 * 1_000,
                "expected EDT immediately before fall-back boundary in {}",
                year,
            );
        }
    }
}
