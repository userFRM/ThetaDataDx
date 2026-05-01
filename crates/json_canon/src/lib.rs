//! JSON canonicalisation helpers shared by the CLI, REST server, and MCP tools.
//!
//! Standards-compliant JSON encoders refuse `NaN`, `+Infinity`, and
//! `-Infinity` because the JSON spec has no representation for them. The
//! `sonic_rs` encoder writes JSON `null` in their place, but the cross-language
//! SDK contract (see `scripts/validate_agreement.py` and the `raw_f64` helper
//! in `tools/cli/src/main.rs`) requires every frontend — REST, MCP, CLI — to
//! canonicalise non-finite f64 to `null` BEFORE the value tree leaves the
//! frontend, so callers cannot observe a backend-specific drift.
//!
//! This crate owns the canonicalisation pass so all three frontends share one
//! implementation.

#![forbid(unsafe_code)]

use sonic_rs::{JsonNumberTrait, JsonValueMutTrait, JsonValueTrait, Number, Value};

/// Convert a single f64 to a JSON-safe value: finite passthrough, non-finite
/// becomes JSON null. Mirrors the behaviour of `tools/cli/src/main.rs`
/// `raw_f64` exactly so all three frontends produce byte-identical output for
/// the same tick payload.
#[must_use]
pub fn finite_or_null(value: f64) -> Value {
    Number::from_f64(value).map_or_else(Value::new_null, Value::from)
}

/// Walk `value` in place and replace every non-finite f64 inside numbers,
/// arrays, and objects with JSON `null`.
///
/// After this call returns, every leaf number in the tree is either a JSON
/// integer or a finite f64. The walk is non-allocating in the steady state —
/// only the non-finite leaves are replaced.
pub fn canonicalize(value: &mut Value) {
    if let Some(arr) = value.as_array_mut() {
        for item in arr.iter_mut() {
            canonicalize(item);
        }
        return;
    }
    if let Some(obj) = value.as_object_mut() {
        for (_, v) in obj.iter_mut() {
            canonicalize(v);
        }
        return;
    }
    if let Some(num) = value.as_number() {
        if let Some(f) = num.as_f64() {
            if !f.is_finite() {
                *value = Value::new_null();
            }
        }
    }
}

/// Canonicalise the value tree in place and serialise it.
///
/// # Errors
///
/// Forwards any `sonic_rs::Error` from the serialiser. Callers MUST translate
/// this into a structured error response — never an empty body.
pub fn canonicalize_and_serialize(value: &mut Value) -> Result<String, sonic_rs::Error> {
    canonicalize(value);
    sonic_rs::to_string(&*value)
}

#[cfg(test)]
mod tests {
    use super::*;

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

    #[test]
    fn canonicalize_walks_arrays() {
        // `Value::from(f64::NAN)` is not callable directly (no `From<f64>`).
        // We seed the array with three slots — a finite f64, a pre-collapsed
        // NaN, and another finite f64 — then run the canonicaliser to prove
        // the walk does not corrupt the surrounding finite values.
        let mut arr = sonic_rs::array![
            sonic_rs::to_value(&1.0_f64).expect("finite ok"),
            finite_or_null(f64::NAN),
            sonic_rs::to_value(&3.0_f64).expect("finite ok"),
        ]
        .into_value();
        canonicalize(&mut arr);
        let s = sonic_rs::to_string(&arr).expect("serialises after canonicalise");
        assert_eq!(s, "[1.0,null,3.0]");
    }

    #[test]
    fn canonicalize_walks_objects() {
        let mut obj = sonic_rs::json!({
            "ok": 1.5_f64,
            "bad": Value::new_null(),
            "nested": {
                "deep": Value::new_null(),
            }
        });
        if let Some(o) = obj.as_object_mut() {
            o.insert(&"bad", finite_or_null(f64::NAN));
            if let Some(nested) = o.get_mut(&"nested").and_then(|v| v.as_object_mut()) {
                nested.insert(&"deep", finite_or_null(f64::INFINITY));
            }
        }
        canonicalize(&mut obj);
        let s = sonic_rs::to_string(&obj).expect("serialises");
        assert!(s.contains("\"bad\":null"), "got {s}");
        assert!(s.contains("\"deep\":null"), "got {s}");
        assert!(s.contains("\"ok\":1.5"), "got {s}");
    }

    #[test]
    fn canonicalize_and_serialize_succeeds_after_nan() {
        let mut v = sonic_rs::array![
            finite_or_null(f64::NAN),
            sonic_rs::to_value(&2.0_f64).expect("finite"),
        ]
        .into_value();
        let s = canonicalize_and_serialize(&mut v).expect("must serialise");
        assert_eq!(s, "[null,2.0]");
    }
}
