use std::cmp::Ordering;
use std::fmt;

/// Precomputed powers of 10 as i64 for fast integer scaling in Price::compare.
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

/// Precomputed powers of 10 as f64 for fast float conversion in Price::to_f64.
static POW10_F64: [f64; 20] = [
    1e0, 1e1, 1e2, 1e3, 1e4, 1e5, 1e6, 1e7, 1e8, 1e9, 1e10, 1e11, 1e12, 1e13, 1e14, 1e15, 1e16,
    1e17, 1e18, 1e19,
];

/// Fixed-point price with variable decimal precision.
///
/// ThetaData encodes prices as `(value, type)` where `type` indicates the
/// decimal power. The real price is `value * 10^(type - 10)`:
/// - type=0: zero price
/// - type=8: value * 0.01 (2 decimal places — cents)
/// - type=10: value * 1.0 (integer)
/// - type>10: value * 10^(type-10)
#[derive(Clone, Copy, Default)]
pub struct Price {
    pub value: i32,
    /// Decimal type: 0 means zero, otherwise `10 - type` = fractional digits.
    pub price_type: i32,
}

impl Price {
    pub const ZERO: Self = Self {
        value: 0,
        price_type: 0,
    };

    #[inline]
    pub fn new(value: i32, price_type: i32) -> Self {
        debug_assert!(
            (0..20).contains(&price_type),
            "price_type must be 0..20, got {price_type}"
        );
        let clamped = price_type.clamp(0, 19);
        if clamped != price_type {
            tracing::warn!(
                price_type,
                clamped,
                "price_type out of range 0..20, clamping"
            );
        }
        Self {
            value,
            price_type: clamped,
        }
    }

    pub fn is_zero(&self) -> bool {
        self.value == 0 || self.price_type == 0
    }

    /// Convert to f64. This is lossy but useful for display/calculations.
    #[inline]
    pub fn to_f64(&self) -> f64 {
        if self.price_type == 0 {
            return 0.0;
        }
        let exp = self.price_type - 10;
        if exp >= 0 {
            self.value as f64 * POW10_F64[exp as usize]
        } else {
            self.value as f64 / POW10_F64[(-exp) as usize]
        }
    }

    /// Create from a protobuf Price message.
    pub fn from_proto(proto: &crate::proto::Price) -> Self {
        Self::new(proto.value, proto.r#type)
    }

    /// Convert to the protobuf Price message.
    pub fn to_proto(&self) -> crate::proto::Price {
        crate::proto::Price {
            value: self.value,
            r#type: self.price_type,
        }
    }

    /// Normalize both prices to the same type for comparison.
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
            let scaled = (self.value as i64).checked_mul(POW10_I64[exp]);
            match scaled {
                Some(s) => s.cmp(&(other.value as i64)),
                // Overflow: fall back to f64 for correct sign handling.
                None => self.to_f64().total_cmp(&other.to_f64()),
            }
        } else {
            let exp = (other.price_type - self.price_type) as usize;
            if exp > 18 {
                return self.to_f64().total_cmp(&other.to_f64());
            }
            let scaled = (other.value as i64).checked_mul(POW10_I64[exp]);
            match scaled {
                Some(s) => (self.value as i64).cmp(&s),
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
        write!(f, "Price({})", self)
    }
}

impl fmt::Display for Price {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.price_type == 0 {
            return write!(f, "0.0");
        }
        if self.price_type == 10 {
            return write!(f, "{}.0", self.value);
        }
        if self.price_type > 10 {
            let zeros = "0".repeat((self.price_type - 10) as usize);
            return write!(f, "{}{}.0", self.value, zeros);
        }

        let is_neg = self.value < 0;
        let abs_str = if is_neg {
            format!("{}", -self.value as i64)
        } else {
            format!("{}", self.value)
        };

        let frac_digits = (10 - self.price_type) as usize;
        let padded = if abs_str.len() <= frac_digits {
            let pad = "0".repeat(frac_digits - abs_str.len() + 1);
            format!("{}{}", pad, abs_str)
        } else {
            abs_str
        };

        let split = padded.len() - frac_digits;
        let result = format!("{}.{}", &padded[..split], &padded[split..]);
        if is_neg {
            write!(f, "-{}", result)
        } else {
            write!(f, "{}", result)
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
}
