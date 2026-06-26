//! Variable-precision fixed-point price.
//!
//! `ThetaData` encodes a price as a `(value, price_type)` pair where the real
//! price is `value * 10^(price_type - 10)`. [`Price`] stores that pair and
//! compares values by scaling to a common base via integer powers of ten,
//! falling back to f64 only when the exponent gap would overflow `i64`.
//!
//! The decimal exponent is held as a [`PriceType`] newtype whose only
//! constructors reject anything outside `0..=MAX_PRICE_TYPE`. Because an
//! out-of-range exponent cannot be represented, every `POW10_*` table
//! index derived from a `PriceType` is in bounds by construction — the
//! read paths convert and compare with no runtime range check and no
//! possibility of a fabricated out-of-range result.

use std::cmp::Ordering;
use std::fmt;

/// Highest valid `price_type` discriminant. The encoded power-of-ten
/// `exp = price_type - 10` ranges from -10 to +9 across the valid set
/// (`price_type` 0 means "unset" — at the boundary).
/// The constant bounds [`PriceType`]'s checked constructors and matches the
/// size of the `POW10_*` tables (20 entries / indices 0..=19), so any index
/// derived from a `PriceType` is in range by construction.
pub const MAX_PRICE_TYPE: i32 = 19;

/// Construction error for [`Price::with_value_and_type`] and
/// [`PriceType::new`].
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

/// Validated decimal exponent for a [`Price`], constrained to
/// `0..=MAX_PRICE_TYPE`.
///
/// The inner byte is private and every constructor checks the range, so an
/// out-of-range exponent is unrepresentable. The decimal exponent
/// `exp = price_type - 10` is therefore confined to `-10..=9`, so
/// `exp.unsigned_abs()` is a `POW10_*` table index that is provably in
/// bounds — the conversion and comparison paths drop every per-read range
/// check.
///
/// `Default` is the in-range "unset" exponent (`0`), matching
/// [`PriceType::UNSET`] and [`Price::ZERO`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PriceType(u8);

impl PriceType {
    /// The "unset" exponent (`price_type == 0`).
    pub const UNSET: Self = Self(0);

    /// Construct a `PriceType`, rejecting anything outside
    /// `0..=MAX_PRICE_TYPE`.
    ///
    /// # Errors
    ///
    /// Returns [`PriceError::PriceTypeOutOfRange`] when `price_type` is
    /// negative or larger than [`MAX_PRICE_TYPE`].
    #[inline]
    pub const fn new(price_type: i32) -> Result<Self, PriceError> {
        if price_type < 0 || price_type > MAX_PRICE_TYPE {
            return Err(PriceError::PriceTypeOutOfRange(price_type));
        }
        // In range, so the cast to u8 is exact.
        Ok(Self(price_type as u8))
    }

    /// Construct a `PriceType` by saturating into `0..=MAX_PRICE_TYPE`.
    /// Values below 0 snap to 0; values above the cap snap to
    /// [`MAX_PRICE_TYPE`]. Used by the lossy [`Price::new`] constructor.
    #[inline]
    #[must_use]
    pub const fn saturating(price_type: i32) -> Self {
        let clamped = if price_type < 0 {
            0
        } else if price_type > MAX_PRICE_TYPE {
            MAX_PRICE_TYPE
        } else {
            price_type
        };
        Self(clamped as u8)
    }

    /// The exponent as `i32`, in `0..=MAX_PRICE_TYPE`.
    #[inline]
    #[must_use]
    pub const fn get(self) -> i32 {
        self.0 as i32
    }
}

impl TryFrom<i32> for PriceType {
    type Error = PriceError;

    #[inline]
    fn try_from(price_type: i32) -> Result<Self, Self::Error> {
        Self::new(price_type)
    }
}

impl From<PriceType> for i32 {
    #[inline]
    fn from(price_type: PriceType) -> Self {
        price_type.get()
    }
}

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
    /// Mantissa. Read via the [`Price::value`] accessor.
    pub(crate) value: i32,
    /// Decimal exponent: 0 means zero/unset, otherwise the real price is
    /// `value * 10^(price_type - 10)`. Read via the [`Price::price_type`]
    /// accessor.
    ///
    /// Typed as [`PriceType`], which can only hold `0..=MAX_PRICE_TYPE`.
    /// The `POW10_I64` / `POW10_F64` tables have 20 entries
    /// (indices `0..=19`); since the exponent is bounded by the type,
    /// every read-path table access is in range without a runtime
    /// check. Index 19 is a reserved upper boundary — the arithmetic only
    /// ever uses indices `0..=18` (since `10^19` overflows `i64`, the
    /// index-19 slot stores `i64::MAX` as an unreachable placeholder), but
    /// admitting `price_type = 19` lets the wire decode boundary widen in
    /// one constant if a server-side schema extends the range.
    pub(crate) price_type: PriceType,
}

impl Price {
    /// A zero price with the unset price type, useful as a neutral initializer.
    pub const ZERO: Self = Self {
        value: 0,
        price_type: PriceType::UNSET,
    };

    /// Construct a `Price`, saturating `price_type` into the valid
    /// `0..=MAX_PRICE_TYPE` range. Out-of-range inputs snap to the nearest
    /// boundary; callers that must reject bad inputs use
    /// [`Self::with_value_and_type`] instead.
    #[inline]
    #[must_use]
    pub fn new(value: i32, price_type: i32) -> Self {
        Self {
            value,
            price_type: PriceType::saturating(price_type),
        }
    }

    /// Construct a `Price` that errors when `price_type` is outside
    /// `0..=MAX_PRICE_TYPE`. The strict counterpart to [`Self::new`] —
    /// callers that own the wire decode boundary use this so a hostile
    /// upstream value surfaces as a typed error instead of saturating.
    ///
    /// # Errors
    ///
    /// Returns [`PriceError::PriceTypeOutOfRange`] when `price_type` is
    /// negative or larger than [`MAX_PRICE_TYPE`].
    #[inline]
    pub fn with_value_and_type(value: i32, price_type: i32) -> Result<Self, PriceError> {
        Ok(Self {
            value,
            price_type: PriceType::new(price_type)?,
        })
    }

    /// Mantissa accessor. The raw field is crate-private so the
    /// `0..=MAX_PRICE_TYPE` invariant cannot be bypassed by construction.
    #[inline]
    #[must_use]
    pub const fn value(&self) -> i32 {
        self.value
    }

    /// Decimal-exponent accessor, in `0..=MAX_PRICE_TYPE`. The exponent is
    /// stored as a validated [`PriceType`] so the invariant cannot be
    /// bypassed by construction.
    #[inline]
    #[must_use]
    pub const fn price_type(&self) -> i32 {
        self.price_type.get()
    }

    /// Convert to f64. This is lossy but useful for display/calculations.
    #[inline]
    #[must_use]
    pub fn to_f64(self) -> f64 {
        let price_type = self.price_type.get();
        if price_type == 0 {
            return 0.0;
        }
        let exp = price_type - 10;
        // `price_type` is `1..=MAX_PRICE_TYPE` here, so `exp` is `-9..=9`
        // and `exp.unsigned_abs()` is `0..=9` — a valid `POW10_F64` index
        // by construction.
        let scale = POW10_F64[exp.unsigned_abs() as usize];
        if exp >= 0 {
            f64::from(self.value) * scale
        } else {
            f64::from(self.value) / scale
        }
    }

    /// Normalize both prices to the same type for comparison.
    // `&self` is required by the `PartialOrd`/`Ord` trait signatures.
    #[allow(clippy::trivially_copy_pass_by_ref)]
    #[inline]
    fn compare(&self, other: &Self) -> Ordering {
        let self_type = self.price_type.get();
        let other_type = other.price_type.get();
        if self_type == other_type {
            return self.value.cmp(&other.value);
        }
        // Scale to common base using i64 to avoid overflow.
        // For exponents > 18, i64 multiplication can overflow; fall back to f64.
        if self_type > other_type {
            let exp = (self_type - other_type).unsigned_abs() as usize;
            if exp > 18 {
                // Fall back to f64 comparison for very large exponent differences.
                return self.to_f64().total_cmp(&other.to_f64());
            }
            let scaled = i64::from(self.value).checked_mul(POW10_I64[exp]);
            match scaled {
                Some(s) => s.cmp(&i64::from(other.value)),
                // Overflow: fall back to f64 for correct sign handling.
                None => self.to_f64().total_cmp(&other.to_f64()),
            }
        } else {
            let exp = (other_type - self_type).unsigned_abs() as usize;
            if exp > 18 {
                return self.to_f64().total_cmp(&other.to_f64());
            }
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
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // `price_type` is `0..=MAX_PRICE_TYPE` by construction, so the
        // digit-count arithmetic below stays non-negative and bounded.
        let price_type = self.price_type.get();
        if price_type == 0 {
            return write!(f, "0.0");
        }
        if price_type == 10 {
            return write!(f, "{}.0", self.value);
        }
        if price_type > 10 {
            let zeros = "0".repeat((price_type - 10).unsigned_abs() as usize);
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

        let frac_digits = (10 - price_type).unsigned_abs() as usize;
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
    /// The assertion is "no panic + finite numeric", not a
    /// `Display`->parse round-trip.
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

    /// `Price::new`'s saturating behaviour: out-of-range `price_type`
    /// snaps to the nearest valid boundary so existing callers stay
    /// panic-free even with a hostile wire byte.
    #[test]
    fn new_saturates_out_of_range_price_type() {
        // Above the cap saturates to MAX_PRICE_TYPE.
        assert_eq!(Price::new(1, 99).price_type(), MAX_PRICE_TYPE);
        // Below 0 saturates to 0.
        assert_eq!(Price::new(1, -3).price_type(), 0);
        // Extremes do not panic and land in range.
        assert_eq!(Price::new(1, i32::MIN).price_type(), 0);
        assert_eq!(Price::new(1, i32::MAX).price_type(), MAX_PRICE_TYPE);
    }

    /// Accessors return the validated exponent and mantissa. The exponent
    /// is stored as a [`PriceType`] so the `0..=MAX_PRICE_TYPE` invariant
    /// cannot be bypassed by external construction.
    #[test]
    fn accessors_match_fields() {
        let p = Price::new(15025, 8);
        assert_eq!(p.value(), 15025);
        assert_eq!(p.price_type(), 8);
        assert_eq!(p.price_type(), p.price_type.get());
    }

    /// `PriceType::new` / `TryFrom<i32>` accept the supported range and
    /// reject everything else, so an out-of-range exponent is
    /// unrepresentable — there is no read path that can observe one.
    #[test]
    fn price_type_rejects_out_of_range() {
        for pt in 0..=MAX_PRICE_TYPE {
            assert_eq!(PriceType::new(pt).map(PriceType::get), Ok(pt));
            assert_eq!(PriceType::try_from(pt).map(PriceType::get), Ok(pt));
        }
        for bad in [-1, MAX_PRICE_TYPE + 1, 20, 99, i32::MIN, i32::MAX] {
            assert_eq!(
                PriceType::new(bad),
                Err(PriceError::PriceTypeOutOfRange(bad)),
                "price_type {bad} must be rejected"
            );
            assert!(PriceType::try_from(bad).is_err());
        }
    }

    /// `PriceType::saturating` snaps into range at both boundaries while
    /// leaving in-range values untouched.
    #[test]
    fn price_type_saturating_clamps_to_boundaries() {
        assert_eq!(PriceType::saturating(-100).get(), 0);
        assert_eq!(PriceType::saturating(i32::MIN).get(), 0);
        assert_eq!(PriceType::saturating(8).get(), 8);
        assert_eq!(PriceType::saturating(MAX_PRICE_TYPE).get(), MAX_PRICE_TYPE);
        assert_eq!(PriceType::saturating(1000).get(), MAX_PRICE_TYPE);
        assert_eq!(PriceType::saturating(i32::MAX).get(), MAX_PRICE_TYPE);
    }

    /// Every representable exponent (`0..=MAX_PRICE_TYPE`) indexes the
    /// `POW10_*` tables in bounds, and conversion stays finite — the
    /// type-level guarantee the read paths now rely on instead of a
    /// per-read clamp.
    #[test]
    fn every_price_type_indexes_tables_in_bounds() {
        for pt in 0..=MAX_PRICE_TYPE {
            let p = Price::new(1, pt);
            let f = p.to_f64();
            assert!(f.is_finite(), "to_f64 must be finite for price_type {pt}");
            assert!(!p.to_string().is_empty(), "Display must be non-empty");
            let reference = Price::new(15025, 8);
            let _ = p.cmp(&reference);
            let _ = reference.cmp(&p);
            let _ = p.cmp(&p);
        }
    }

    /// In-range conversions are unchanged by the newtype: validated
    /// values must reproduce the exact prior results for known
    /// `(value, price_type)` pairs.
    #[test]
    fn in_range_conversions_are_unchanged() {
        // value=12345, price_type=8 -> 12345 * 10^(8-10) = 123.45
        assert!((Price::new(12345, 8).to_f64() - 123.45).abs() < 1e-10);
        assert_eq!(Price::new(12345, 8).to_string(), "123.45");
        // value=5, price_type=12 -> 5 * 10^2 = 500.0
        assert!((Price::new(5, 12).to_f64() - 500.0).abs() < 1e-10);
        assert_eq!(Price::new(5, 12).to_string(), "500.0");
        // value=100, price_type=10 -> 100 * 10^0 = 100.0
        assert!((Price::new(100, 10).to_f64() - 100.0).abs() < 1e-10);
        assert_eq!(Price::new(100, 10).to_string(), "100.0");
    }
}
