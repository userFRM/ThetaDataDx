use std::cmp::Ordering;
use std::fmt;

/// Highest valid `price_type` discriminant. The encoded power-of-ten
/// `exp = price_type - 10` ranges from -10 to +9 across the valid set
/// (`price_type` 0 means "unset" — at the boundary, see [`Price::is_unset`]).
/// The constant defines the upper bound for `with_value_and_type`'s
/// range check and the matching `debug_assert!` guards on every path
/// that indexes the `POW10_*` tables (sized 20 entries / indices 0..=19).
pub const MAX_PRICE_TYPE: i32 = 19;

/// Construction error for [`Price::with_value_and_type`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PriceError {
    /// `price_type` outside the supported `0..=MAX_PRICE_TYPE` range.
    PriceTypeOutOfRange(i32),
}

impl fmt::Display for PriceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::PriceTypeOutOfRange(t) => write!(
                f,
                "price_type {t} outside valid range [0, {MAX_PRICE_TYPE}]"
            ),
        }
    }
}

impl std::error::Error for PriceError {}

/// Precomputed powers of 10 as i64 for fast integer scaling in `Price::compare`.
static POW10_I64: [i64; 20] = [
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
    10_000_000_000,
    100_000_000_000,
    1_000_000_000_000,
    10_000_000_000_000,
    100_000_000_000_000,
    1_000_000_000_000_000,
    10_000_000_000_000_000,
    100_000_000_000_000_000,
    1_000_000_000_000_000_000,
    // 10^19 overflows i64, but index 19 is unreachable (exp capped at 18).
    i64::MAX,
];

/// Precomputed powers of 10 as f64 for fast float conversion in `Price::to_f64`.
static POW10_F64: [f64; 20] = [
    1e0, 1e1, 1e2, 1e3, 1e4, 1e5, 1e6, 1e7, 1e8, 1e9, 1e10, 1e11, 1e12, 1e13, 1e14, 1e15, 1e16,
    1e17, 1e18, 1e19,
];

/// Fixed-point price with variable decimal precision.
///
/// `ThetaData` encodes prices as `(value, type)` where `type` indicates the
/// decimal power. The real price is `value * 10^(type - 10)`:
/// - type=0: zero price
/// - type=8: value * 0.01 (2 decimal places — cents)
/// - type=10: value * 1.0 (integer)
/// - type>10: value * 10^(type-10)
#[derive(Clone, Copy, Default)]
pub struct Price {
    /// Mantissa. Public for backwards compatibility; prefer the typed
    /// [`Price::value`] accessor in new code so the field can become
    /// `pub(crate)` in a future major release.
    pub value: i32,
    /// Decimal type: 0 means zero, otherwise `10 - type` = fractional digits.
    ///
    /// MUST stay within `0..=19` — the `POW10_I64` / `POW10_F64` tables
    /// have 20 entries (indices `0..=19`) and every indexing path
    /// debug-asserts the bound. Index 19 is a reserved upper boundary:
    /// the arithmetic only ever uses indices `0..=18` (since
    /// `10^19` overflows `i64`, the index-19 slot stores `i64::MAX` as
    /// an unreachable placeholder), but accepting `price_type = 19`
    /// without rejecting it lets us widen the wire decode boundary
    /// in one constant if a future server-side schema extends the
    /// range.
    /// Public for backwards compatibility; new code should construct via
    /// [`Price::new`] (clamps) or [`Price::with_value_and_type`] (errors
    /// out of range) so the invariant is enforced at the boundary.
    pub price_type: i32,
}

impl Price {
    pub const ZERO: Self = Self {
        value: 0,
        price_type: 0,
    };

    /// Construct a `Price` clamping `price_type` to the valid
    /// `0..=MAX_PRICE_TYPE` range. Out-of-range values silently snap to
    /// the boundary so existing call sites stay panic-free; new code
    /// that needs to reject bad inputs should call
    /// [`Self::with_value_and_type`] instead.
    #[inline]
    #[must_use]
    pub fn new(value: i32, price_type: i32) -> Self {
        Self {
            value,
            price_type: price_type.clamp(0, MAX_PRICE_TYPE),
        }
    }

    /// Construct a `Price` that errors when `price_type` is outside
    /// `0..=MAX_PRICE_TYPE`. The strict counterpart to [`Self::new`] —
    /// callers that own the wire decode boundary use this so a hostile
    /// upstream value surfaces as a typed error instead of silently
    /// clamping.
    ///
    /// # Errors
    ///
    /// Returns [`PriceError::PriceTypeOutOfRange`] when `price_type` is
    /// negative or larger than [`MAX_PRICE_TYPE`].
    #[inline]
    pub fn with_value_and_type(value: i32, price_type: i32) -> Result<Self, PriceError> {
        if !(0..=MAX_PRICE_TYPE).contains(&price_type) {
            return Err(PriceError::PriceTypeOutOfRange(price_type));
        }
        Ok(Self { value, price_type })
    }

    /// Mantissa accessor — prefer over direct field access in new code
    /// so the field can become `pub(crate)` in a future major release
    /// without breaking call sites.
    #[inline]
    #[must_use]
    pub const fn value(&self) -> i32 {
        self.value
    }

    /// Decimal-exponent accessor — prefer over direct field access in
    /// new code so the field can become `pub(crate)` in a future major
    /// release without breaking call sites.
    #[inline]
    #[must_use]
    pub const fn price_type(&self) -> i32 {
        self.price_type
    }

    /// Whether this `Price` carries the "unset" sentinel
    /// (`price_type == 0`). Distinct from a legitimate
    /// `0.0` price with a non-zero `price_type`; use [`Self::is_zero_value`]
    /// for that.
    #[inline]
    #[must_use]
    pub const fn is_unset(&self) -> bool {
        self.price_type == 0
    }

    /// Whether this `Price` represents a real zero (mantissa == 0 with
    /// a non-zero `price_type`). Distinct from the "unset" sentinel
    /// surfaced by [`Self::is_unset`].
    #[inline]
    #[must_use]
    pub const fn is_zero_value(&self) -> bool {
        self.value == 0 && self.price_type != 0
    }

    /// Whether this `Price` represents zero by either signal — sentinel
    /// (`price_type == 0`) or real zero mantissa. Kept for backwards
    /// compatibility with pre-fix callers; new code should call
    /// [`Self::is_unset`] / [`Self::is_zero_value`] explicitly so the
    /// "no quote yet" vs "zero price" branches stay distinguishable.
    #[must_use]
    pub fn is_zero(&self) -> bool {
        self.value == 0 || self.price_type == 0
    }

    /// Convert to f64. This is lossy but useful for display/calculations.
    // Reason: price_type is bounded to 0..=19 by new/with_value_and_type
    // (index 19 is a reserved unreachable placeholder; see the constant
    // doc on MAX_PRICE_TYPE), and a debug_assert below pins the
    // invariant for hand-constructed values that bypass those
    // constructors.
    #[allow(clippy::cast_sign_loss)]
    #[inline]
    #[must_use]
    pub fn to_f64(&self) -> f64 {
        if self.price_type == 0 {
            return 0.0;
        }
        debug_assert!(
            (self.price_type as usize) < POW10_F64.len(),
            "price_type {} outside POW10_F64 table",
            self.price_type
        );
        let exp = self.price_type - 10;
        if exp >= 0 {
            debug_assert!(
                (exp as usize) < POW10_F64.len(),
                "exp {exp} outside POW10_F64 table"
            );
            f64::from(self.value) * POW10_F64[exp as usize]
        } else {
            debug_assert!(
                ((-exp) as usize) < POW10_F64.len(),
                "(-exp) {} outside POW10_F64 table",
                -exp
            );
            f64::from(self.value) / POW10_F64[(-exp) as usize]
        }
    }

    /// Normalize both prices to the same type for comparison.
    // Reason: price_type is bounded to 0..=19, so differences are in
    // 0..=19 (safe cast). `&self` required by PartialOrd/Ord trait
    // implementations. Every index into POW10_I64 is guarded by a
    // debug_assert in addition to the explicit `exp > 18` early-out
    // (index 19 stores `i64::MAX` as an unreachable placeholder, so the
    // early-out is what keeps the arithmetic correct).
    #[allow(clippy::cast_sign_loss, clippy::trivially_copy_pass_by_ref)]
    #[inline]
    fn compare(&self, other: &Self) -> Ordering {
        if self.price_type == other.price_type {
            return self.value.cmp(&other.value);
        }
        // Scale to common base using i64 to avoid overflow.
        // For exponents > 18, i64 multiplication can overflow; fall back to f64.
        if self.price_type > other.price_type {
            let exp = (self.price_type - other.price_type) as usize;
            if exp > 18 {
                // Fall back to f64 comparison for very large exponent differences.
                return self.to_f64().total_cmp(&other.to_f64());
            }
            debug_assert!(exp < POW10_I64.len(), "exp {exp} outside POW10_I64 table");
            let scaled = i64::from(self.value).checked_mul(POW10_I64[exp]);
            match scaled {
                Some(s) => s.cmp(&i64::from(other.value)),
                // Overflow: fall back to f64 for correct sign handling.
                None => self.to_f64().total_cmp(&other.to_f64()),
            }
        } else {
            let exp = (other.price_type - self.price_type) as usize;
            if exp > 18 {
                return self.to_f64().total_cmp(&other.to_f64());
            }
            debug_assert!(exp < POW10_I64.len(), "exp {exp} outside POW10_I64 table");
            let scaled = i64::from(other.value).checked_mul(POW10_I64[exp]);
            match scaled {
                Some(s) => i64::from(self.value).cmp(&s),
                None => self.to_f64().total_cmp(&other.to_f64()),
            }
        }
    }
}

impl PartialEq for Price {
    fn eq(&self, other: &Self) -> bool {
        self.compare(other) == Ordering::Equal
    }
}

impl Eq for Price {}

impl PartialOrd for Price {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Price {
    fn cmp(&self, other: &Self) -> Ordering {
        self.compare(other)
    }
}

impl fmt::Debug for Price {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Price({self})")
    }
}

impl fmt::Display for Price {
    // Reason: price_type is bounded to 0..=19; debug_asserts pin the
    // invariant for the formatter paths that arithmetic-cast it to
    // unsigned. The formatter never actually reaches an `exp` of 9
    // (price_type 19) because `10^19` overflows i64 — the index-19 slot
    // is an unreachable placeholder, see MAX_PRICE_TYPE.
    #[allow(clippy::cast_sign_loss)]
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.price_type == 0 {
            return write!(f, "0.0");
        }
        debug_assert!(
            (0..=MAX_PRICE_TYPE).contains(&self.price_type),
            "price_type {} outside valid range",
            self.price_type
        );
        if self.price_type == 10 {
            return write!(f, "{}.0", self.value);
        }
        if self.price_type > 10 {
            let zeros = "0".repeat((self.price_type - 10) as usize);
            return write!(f, "{}{}.0", self.value, zeros);
        }

        let is_neg = self.value < 0;
        let abs_str = if is_neg {
            // Widen to i64 before negating so `i32::MIN` (no positive
            // i32 counterpart) does not overflow.
            format!("{}", -i64::from(self.value))
        } else {
            format!("{}", self.value)
        };

        let frac_digits = (10 - self.price_type) as usize;
        let padded = if abs_str.len() <= frac_digits {
            let pad = "0".repeat(frac_digits - abs_str.len() + 1);
            format!("{pad}{abs_str}")
        } else {
            abs_str
        };

        let split = padded.len() - frac_digits;
        let result = format!("{}.{}", &padded[..split], &padded[split..]);
        if is_neg {
            write!(f, "-{result}")
        } else {
            write!(f, "{result}")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_price_display() {
        assert_eq!(Price::new(0, 0).to_string(), "0.0");
        assert_eq!(Price::new(15025, 8).to_string(), "150.25");
        assert_eq!(Price::new(100, 10).to_string(), "100.0");
        assert_eq!(Price::new(5, 12).to_string(), "500.0");
        assert_eq!(Price::new(-15025, 8).to_string(), "-150.25");
        assert_eq!(Price::new(5, 7).to_string(), "0.005");
    }

    #[test]
    fn test_price_display_i32_min_does_not_overflow() {
        // `i32::MIN` has no positive i32 counterpart; the formatter must
        // widen to i64 before negating rather than negating the i32.
        assert_eq!(Price::new(i32::MIN, 8).to_string(), "-21474836.48");
        assert_eq!(Price::new(i32::MIN, 10).to_string(), "-2147483648.0");
        assert_eq!(Price::new(i32::MIN, 7).to_string(), "-2147483.648");
    }

    #[test]
    fn test_price_to_f64() {
        let p = Price::new(15025, 8);
        assert!((p.to_f64() - 150.25).abs() < 1e-10);
    }

    #[test]
    fn test_price_comparison() {
        let a = Price::new(15025, 8); // 150.25
        let b = Price::new(15000, 8); // 150.00
        let c = Price::new(1502500, 6); // 150.25 (same value, different type)
        assert!(a > b);
        assert_eq!(a, c);
    }

    /// Every valid `price_type` (`0..=MAX_PRICE_TYPE`, i.e. `0..=19`)
    /// renders and converts to `f64` without panicking. Pins the
    /// invariant that the POW10 tables are correctly sized for the
    /// supported range, including the index-19 placeholder slot, and
    /// that both Display and to_f64 agree on a finite numeric value.
    /// (Renamed from `_round_trips` -- the assertion is "no panic +
    /// finite numeric", not a Display->parse round-trip, per S37.)
    #[test]
    fn every_valid_price_type_renders_and_converts_finitely() {
        for pt in 0..=MAX_PRICE_TYPE {
            let p = Price::new(12345, pt);
            // Display path must not panic for any valid price_type
            // AND must produce a non-empty string.
            let rendered = p.to_string();
            assert!(
                !rendered.is_empty(),
                "Display must produce a non-empty string for price_type {pt}"
            );
            // to_f64 path must not panic AND must return a finite value
            // for any valid price_type. NaN / infinity would imply the
            // POW10 divisor was zero or the cast wrapped past f64 range.
            let f = p.to_f64();
            assert!(
                f.is_finite(),
                "to_f64 must be finite for price_type {pt}; got {f}"
            );
        }
    }

    /// `with_value_and_type` accepts the supported range and rejects
    /// everything else with a typed error.
    #[test]
    fn with_value_and_type_enforces_range() {
        for pt in 0..=MAX_PRICE_TYPE {
            assert!(
                Price::with_value_and_type(1, pt).is_ok(),
                "price_type {pt} must be accepted"
            );
        }
        assert!(matches!(
            Price::with_value_and_type(1, -1),
            Err(PriceError::PriceTypeOutOfRange(-1))
        ));
        assert!(matches!(
            Price::with_value_and_type(1, 20),
            Err(PriceError::PriceTypeOutOfRange(20))
        ));
        assert!(matches!(
            Price::with_value_and_type(1, 99),
            Err(PriceError::PriceTypeOutOfRange(99))
        ));
        assert!(matches!(
            Price::with_value_and_type(1, i32::MAX),
            Err(PriceError::PriceTypeOutOfRange(_))
        ));
    }

    /// `is_unset` and `is_zero_value` are distinct signals: a fresh
    /// `price_type = 0` is unset; a real zero needs `price_type != 0`
    /// with mantissa 0.
    #[test]
    fn is_unset_vs_is_zero_value() {
        let unset = Price::new(0, 0);
        assert!(unset.is_unset());
        assert!(!unset.is_zero_value());

        let real_zero = Price::new(0, 8);
        assert!(!real_zero.is_unset());
        assert!(real_zero.is_zero_value());

        let nonzero = Price::new(15025, 8);
        assert!(!nonzero.is_unset());
        assert!(!nonzero.is_zero_value());
    }

    /// `Price::new`'s clamp behaviour: out-of-range `price_type`
    /// silently snaps to the valid boundary so existing callers stay
    /// panic-free even with a hostile wire byte.
    #[test]
    fn new_clamps_out_of_range_price_type() {
        // Above the cap clamps to MAX_PRICE_TYPE.
        let p = Price::new(1, 99);
        assert_eq!(p.price_type, MAX_PRICE_TYPE);
        // Below 0 clamps to 0.
        let p = Price::new(1, -3);
        assert_eq!(p.price_type, 0);
        // i32::MIN does not panic.
        let _ = Price::new(1, i32::MIN);
        // i32::MAX does not panic.
        let _ = Price::new(1, i32::MAX);
    }

    /// Accessors return the same value as direct field access — they
    /// exist so the fields can become `pub(crate)` in a future major
    /// release without breaking call sites.
    #[test]
    fn accessors_match_fields() {
        let p = Price::new(15025, 8);
        assert_eq!(p.value(), p.value);
        assert_eq!(p.price_type(), p.price_type);
    }
}
