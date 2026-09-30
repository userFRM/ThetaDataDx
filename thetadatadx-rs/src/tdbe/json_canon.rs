//! JSON canonicalisation helpers shared by the REST server and MCP tools.
//!
//! JSON has no representation for `NaN`, `+Infinity`, or `-Infinity`, and the
//! cross-language SDK contract (see `scripts/ci/check_agreement.py`) requires
//! every frontend (REST and MCP) to emit JSON `null` for a non-finite f64.
//!
//! [`finite_or_null`] is the single conversion point. No pass over a finished
//! value tree is needed: a `sonic_rs::Value` cannot hold a non-finite number,
//! because its constructors and serde serializer turn one into `null` or
//! refuse it, and its parser rejects a literal that overflows to infinity.

#![forbid(unsafe_code)]

use sonic_rs::{Number, Value};

/// Convert a single f64 to a JSON-safe value: finite passthrough, non-finite
/// becomes JSON null. The single canonicalisation point so both frontends
/// produce byte-identical output for the same tick payload.
#[must_use]
pub fn finite_or_null(value: f64) -> Value {
    Number::from_f64(value).map_or_else(Value::new_null, Value::from)
}

#[cfg(test)]
mod tests {
    use super::*;
    use sonic_rs::JsonValueTrait;

    #[test]
    fn nan_becomes_null() {
        let v = finite_or_null(f64::NAN);
        assert!(v.is_null(), "NaN must canonicalise to null, got {v:?}");
    }

    #[test]
    fn pos_inf_becomes_null() {
        let v = finite_or_null(f64::INFINITY);
        assert!(v.is_null(), "+Inf must canonicalise to null, got {v:?}");
    }

    #[test]
    fn neg_inf_becomes_null() {
        let v = finite_or_null(f64::NEG_INFINITY);
        assert!(v.is_null(), "-Inf must canonicalise to null, got {v:?}");
    }

    #[test]
    fn finite_passthrough_42_5() {
        let v = finite_or_null(42.5);
        assert_eq!(v.as_f64(), Some(42.5));
    }

    #[test]
    fn finite_zero_passthrough() {
        let v = finite_or_null(0.0);
        assert_eq!(v.as_f64(), Some(0.0));
    }

    #[test]
    fn finite_negative_passthrough() {
        let v = finite_or_null(-1234.5678);
        assert_eq!(v.as_f64(), Some(-1234.5678));
    }

    /// Pins the `sonic_rs` property the non-finite contract rests on:
    /// a `Value` cannot carry a non-finite number, so serialising any value
    /// tree built through serde emits `null` and the parser refuses an
    /// overflowing literal. An upgrade that starts storing non-finite floats
    /// would leak `NaN` into REST and MCP bodies; this fails first.
    #[test]
    fn sonic_value_cannot_hold_a_non_finite_number() {
        let value = sonic_rs::to_value(&[f64::NAN, f64::INFINITY, f64::NEG_INFINITY])
            .expect("serde serialisation of floats");
        assert_eq!(
            sonic_rs::to_string(&value).expect("serialises"),
            "[null,null,null]"
        );
        assert!(sonic_rs::from_str::<Value>("1e999").is_err());
    }
}
