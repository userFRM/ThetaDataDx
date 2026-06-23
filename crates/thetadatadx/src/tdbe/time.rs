//! Eastern Time + DST primitives.
//!
//! Canonical Eastern-time conversion module reused by `thetadatadx` (historical
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

/// Whether `(year, month, day)` is a real Gregorian calendar date.
///
/// Validates:
/// - `year` ∈ `1900..=2100` (the practical range every market-data
///   surface in this workspace exercises — older / newer values
///   indicate a corrupt input, not a real date);
/// - `month` ∈ `1..=12`;
/// - `day` is in range for the month, including the 4 / 100 / 400
///   leap-year rule for February.
///
/// Used by [`crate::tdbe::time::is_valid_yyyymmdd`] and by the
/// `thetadatadx` historical + streaming validators to reject impossible
/// expirations (`00000000`, `20260230`, `19990431`, …) on every
/// public user input. Internal sentinel uses (e.g. an
/// implementation-detail "unset" date written to the wire) live
/// behind their own paths and don't run through this function.
#[must_use]
pub fn is_valid_gregorian_date(year: i32, month: u32, day: u32) -> bool {
    if !(1900..=2100).contains(&year) {
        return false;
    }
    if !(1..=12).contains(&month) {
        return false;
    }
    if day == 0 {
        return false;
    }
    let dim = match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 => {
            let leap = (year % 4 == 0 && year % 100 != 0) || year % 400 == 0;
            if leap {
                29
            } else {
                28
            }
        }
        _ => unreachable!("month bounded by the 1..=12 check above"),
    };
    day <= dim
}

/// Whether `yyyymmdd` decomposes into a real Gregorian calendar date
/// per [`is_valid_gregorian_date`]. Convenience wrapper for the common
/// "single-i32 packed date" shape this codebase carries on the wire.
#[must_use]
pub fn is_valid_yyyymmdd(yyyymmdd: i32) -> bool {
    if yyyymmdd <= 0 {
        return false;
    }
    let year = yyyymmdd / 10_000;
    let month = (yyyymmdd / 100) % 100;
    let day = yyyymmdd % 100;
    // month / day are non-negative because yyyymmdd > 0 implies the
    // numerator is non-negative; cast through u32 is sound.
    #[allow(clippy::cast_sign_loss)]
    is_valid_gregorian_date(year, month as u32, day as u32)
}

/// Inclusive lower bound of the supported `epoch_ms` conversion window:
/// `1900-01-01 00:00:00 UTC` as Unix epoch milliseconds.
///
/// The Eastern-Time conversion functions are documented to operate over
/// the `1900..=2100` calendar range (see [`is_valid_gregorian_date`]).
/// `epoch_ms` is `0` at the Unix epoch, so the lower bound is negative on
/// the i64 timeline; clamped to `0` here because the wire carries an
/// unsigned `u64`, and any pre-1970 instant is corrupt input on a
/// market-data surface.
pub const MIN_SUPPORTED_EPOCH_MS: u64 = 0;

/// Inclusive upper bound of the supported `epoch_ms` conversion window:
/// `2100-12-31 23:59:59.999 UTC` as Unix epoch milliseconds.
///
/// Day-count arithmetic in the DST-boundary helpers multiplies the civil
/// day index by `86_400_000`; bounding `epoch_ms` to this window keeps
/// every intermediate product inside `i64`, so the conversion never wraps
/// for an in-range value and rejects anything beyond it as corrupt.
pub const MAX_SUPPORTED_EPOCH_MS: u64 = 4_133_980_799_999;

/// Whether `epoch_ms` lies inside the supported Eastern-Time conversion
/// window (`MIN_SUPPORTED_EPOCH_MS..=MAX_SUPPORTED_EPOCH_MS`).
///
/// The decode boundary uses this to reject a corrupt wire `Timestamp`
/// (an unbounded `u64` from the proto) before it reaches the date
/// arithmetic, mirroring how the packed `YYYYMMDD` and `price_type`
/// cells are range-checked at the same boundary.
#[must_use]
pub fn epoch_ms_in_range(epoch_ms: u64) -> bool {
    epoch_ms <= MAX_SUPPORTED_EPOCH_MS
}

/// Eastern Time UTC offset in milliseconds for a given `epoch_ms`.
///
/// Returns `-4 * 3_600_000` (EDT) when DST is in effect for the civil
/// year of `epoch_ms`; otherwise `-5 * 3_600_000` (EST). DST window
/// selection follows the rules documented at the module level.
///
/// Total over all `u64`: an `epoch_ms` beyond [`MAX_SUPPORTED_EPOCH_MS`]
/// would overflow the day-count multiply, so it short-circuits to the
/// EST default instead of wrapping. Callers that must distinguish a real
/// out-of-range timestamp use [`epoch_ms_in_range`] at their boundary;
/// this function never panics regardless of input.
// Reason: the Euclidean date algorithm uses intentional signed/unsigned conversions for valid epoch timestamps.
#[allow(
    clippy::cast_possible_wrap,
    clippy::cast_sign_loss,
    clippy::cast_possible_truncation
)]
#[must_use]
pub fn eastern_offset_ms(epoch_ms: u64) -> i64 {
    // Out-of-range timestamps would overflow the day-count multiply in
    // the DST-boundary helpers; keep the function total by returning the
    // EST default rather than wrapping. The decode boundary rejects such
    // values up front via `epoch_ms_in_range`, so this guard only fires
    // for inputs that never reach the typed surface.
    if !epoch_ms_in_range(epoch_ms) {
        return -5 * 3_600 * 1_000;
    }
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

/// Convert `epoch_ms` to a YYYYMMDD Eastern-Time date, rejecting an
/// out-of-range timestamp as `None`.
///
/// The decode boundary entry point: a wire `Timestamp` arrives as an
/// unbounded `u64`, and an `epoch_ms` past [`MAX_SUPPORTED_EPOCH_MS`]
/// would push the day-count arithmetic outside `i64`. Returning `None`
/// lets the caller surface a typed decode error for corrupt input rather
/// than a silently-wrapped date, while an in-range value yields exactly
/// the same result as [`timestamp_to_date`]. Total over all `u64`.
#[must_use]
pub fn try_timestamp_to_date(epoch_ms: u64) -> Option<i32> {
    if !epoch_ms_in_range(epoch_ms) {
        return None;
    }
    Some(timestamp_to_date(epoch_ms))
}

/// Convert `epoch_ms` to Eastern-Time milliseconds-of-day, rejecting an
/// out-of-range timestamp as `None`.
///
/// Companion to [`try_timestamp_to_date`] for the time-of-day columns;
/// an in-range value yields exactly the same result as
/// [`timestamp_to_ms_of_day`]. Total over all `u64`.
#[must_use]
pub fn try_timestamp_to_ms_of_day(epoch_ms: u64) -> Option<i32> {
    if !epoch_ms_in_range(epoch_ms) {
        return None;
    }
    Some(timestamp_to_ms_of_day(epoch_ms))
}

/// Combine an Eastern-Time `YYYYMMDD` date and milliseconds-of-day into
/// Unix epoch milliseconds (UTC, DST-aware).
///
/// Inverse of the [`timestamp_to_date`] / [`timestamp_to_ms_of_day`]
/// pair: for any market-data instant outside the overnight DST windows,
/// `date_ms_to_epoch_ms(timestamp_to_date(e), timestamp_to_ms_of_day(e)) == Some(e)`.
/// The Eastern offset is resolved with a two-pass fixed point (guess
/// EST, re-resolve at the implied instant). Two overnight windows are
/// inherently irregular and resolved deterministically: the fall-back
/// 01:00-02:00 local hour occurs twice and resolves to the
/// post-transition (EST) instant; the spring-forward 02:00-03:00 local
/// hour does not exist and resolves as if EST. No US market session
/// timestamps data in either window.
///
/// Returns `None` when `date` is not a valid Gregorian `YYYYMMDD` per
/// [`is_valid_yyyymmdd`] (including the `0` absent-date fill on rows
/// the server returned without a date column) or when `ms_of_day` is
/// outside `0..86_400_000`. Storage stays integer everywhere — this is
/// a read-side convenience for the epoch boundary only.
// Reason: date components decompose from a validated YYYYMMDD; casts cannot truncate.
#[allow(clippy::cast_sign_loss)]
#[must_use]
pub fn date_ms_to_epoch_ms(date: i32, ms_of_day: i32) -> Option<i64> {
    if !is_valid_yyyymmdd(date) || !(0..86_400_000).contains(&ms_of_day) {
        return None;
    }
    let year = date / 10_000;
    let month = ((date / 100) % 100) as u32;
    let day = (date % 100) as u32;
    let local_ms = civil_to_epoch_days(year, month, day) * 86_400_000 + i64::from(ms_of_day);

    // Two-pass offset resolution: assume EST, then re-resolve the
    // offset at the implied UTC instant. Converges for every instant
    // outside the 2 AM local transition window.
    let est_guess = local_ms + 5 * 3_600 * 1_000;
    // `eastern_offset_ms` takes epoch ms as u64; market-data dates are
    // bounded to 1900..=2100 by the validator, but pre-1970 dates would
    // go negative — clamp through max(0) for the offset probe only.
    let offset = eastern_offset_ms(est_guess.max(0) as u64);
    let epoch = local_ms - offset;
    let offset = eastern_offset_ms(epoch.max(0) as u64);
    Some(local_ms - offset)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gregorian_validator_accepts_real_dates() {
        // Common reference points from this codebase's tests.
        assert!(is_valid_gregorian_date(2024, 1, 1));
        assert!(is_valid_gregorian_date(2024, 2, 29)); // leap year (4 | not 100)
        assert!(is_valid_gregorian_date(2000, 2, 29)); // leap year (400 rule)
        assert!(is_valid_gregorian_date(2026, 4, 17));
        assert!(is_valid_gregorian_date(1900, 1, 1));
        assert!(is_valid_gregorian_date(2100, 12, 31));
    }

    #[test]
    fn gregorian_validator_rejects_impossible_dates() {
        // The exact garbage shapes the audit pass flagged as silently accepted.
        assert!(!is_valid_gregorian_date(0, 0, 0)); // 00000000 sentinel
        assert!(!is_valid_gregorian_date(2026, 2, 30)); // Feb 30 never exists
        assert!(!is_valid_gregorian_date(1999, 4, 31)); // April only has 30
                                                        // Leap-year edge cases.
        assert!(!is_valid_gregorian_date(1900, 2, 29)); // /100 not /400 = not leap
        assert!(!is_valid_gregorian_date(2023, 2, 29)); // not a multiple of 4
                                                        // Out-of-range fields.
        assert!(!is_valid_gregorian_date(2026, 13, 1));
        assert!(!is_valid_gregorian_date(2026, 0, 1));
        assert!(!is_valid_gregorian_date(2026, 1, 0));
        assert!(!is_valid_gregorian_date(2026, 1, 32));
        // Year out of practical range.
        assert!(!is_valid_gregorian_date(1899, 6, 15));
        assert!(!is_valid_gregorian_date(2101, 6, 15));
    }

    #[test]
    fn yyyymmdd_validator_rejects_zeros_and_negatives() {
        assert!(!is_valid_yyyymmdd(0));
        assert!(!is_valid_yyyymmdd(-1));
        assert!(!is_valid_yyyymmdd(20260230));
        assert!(is_valid_yyyymmdd(20260417));
        assert!(is_valid_yyyymmdd(20240229));
    }

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
    fn hostile_epoch_ms_is_rejected_not_wrapped() {
        // An unbounded wire `u64` must never panic or silently wrap into a
        // garbage date. The day-count multiply (`days * 86_400_000`)
        // overflows i64 well before these magnitudes; the fallible entry
        // points reject them as `None` so the decode boundary can raise a
        // typed error instead.
        for hostile in [
            MAX_SUPPORTED_EPOCH_MS + 1,
            #[allow(clippy::cast_sign_loss)]
            {
                i64::MAX as u64
            },
            u64::MAX,
        ] {
            assert_eq!(
                try_timestamp_to_date(hostile),
                None,
                "out-of-range epoch_ms {hostile} must be rejected, not wrapped",
            );
            assert_eq!(
                try_timestamp_to_ms_of_day(hostile),
                None,
                "out-of-range epoch_ms {hostile} must be rejected, not wrapped",
            );
            assert!(!epoch_ms_in_range(hostile));
            // `eastern_offset_ms` stays total: no panic for any u64, and it
            // returns a valid offset rather than a wrapped value.
            let off = eastern_offset_ms(hostile);
            assert!(off == -5 * 3_600 * 1_000 || off == -4 * 3_600 * 1_000);
        }
    }

    #[test]
    fn in_range_epoch_ms_matches_infallible_conversion() {
        // The fallible path must not alter the result for any valid
        // in-range timestamp.
        for epoch_ms in [1_775_050_200_000_u64, 1_768_487_400_000_u64] {
            assert!(epoch_ms_in_range(epoch_ms));
            assert_eq!(
                try_timestamp_to_date(epoch_ms),
                Some(timestamp_to_date(epoch_ms))
            );
            assert_eq!(
                try_timestamp_to_ms_of_day(epoch_ms),
                Some(timestamp_to_ms_of_day(epoch_ms)),
            );
        }
        // The window boundary itself is accepted.
        assert!(epoch_ms_in_range(MAX_SUPPORTED_EPOCH_MS));
        assert!(try_timestamp_to_date(MAX_SUPPORTED_EPOCH_MS).is_some());
    }

    #[test]
    fn date_ms_to_epoch_ms_round_trips_edt_and_est() {
        // 2026-04-01 09:30:00 ET (EDT) and 2026-01-15 09:30:00 ET (EST).
        for epoch_ms in [1_775_050_200_000_u64, 1_768_487_400_000_u64] {
            let date = timestamp_to_date(epoch_ms);
            let ms = timestamp_to_ms_of_day(epoch_ms);
            // Reason: market-data epochs fit i64.
            #[allow(clippy::cast_possible_wrap)]
            let expected = epoch_ms as i64;
            assert_eq!(date_ms_to_epoch_ms(date, ms), Some(expected));
        }
    }

    #[test]
    fn date_ms_to_epoch_ms_rejects_absent_and_invalid_inputs() {
        assert_eq!(date_ms_to_epoch_ms(0, 34_200_000), None);
        assert_eq!(date_ms_to_epoch_ms(20260230, 0), None);
        assert_eq!(date_ms_to_epoch_ms(20260401, -1), None);
        assert_eq!(date_ms_to_epoch_ms(20260401, 86_400_000), None);
    }

    proptest! {
        /// Round-trip invariant outside the overnight DST windows:
        /// `date_ms_to_epoch_ms(timestamp_to_date(e), timestamp_to_ms_of_day(e)) == e`.
        #[test]
        fn date_ms_round_trip(epoch_ms in arbitrary_epoch_ms_2000_2099()) {
            let ms = timestamp_to_ms_of_day(epoch_ms);
            // Skip the overnight DST windows (fall-back 1-2 AM occurs
            // twice, spring-forward 2-3 AM does not exist); resolution
            // there is documented as deterministic-but-lossy and market
            // data never carries those instants.
            prop_assume!(ms >= 3 * 3_600_000);
            let date = timestamp_to_date(epoch_ms);
            // Reason: bounded strategy fits i64.
            #[allow(clippy::cast_possible_wrap)]
            let expected = epoch_ms as i64;
            prop_assert_eq!(date_ms_to_epoch_ms(date, ms), Some(expected));
        }
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
