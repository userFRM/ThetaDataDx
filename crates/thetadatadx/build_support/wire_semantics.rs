//! Build-time-only companion to [`crate::wire_semantics`] (runtime file at
//! `src/wire_semantics.rs`).
//!
//! Items here are consumed by `build_support/endpoints/modes.rs` when
//! collapsing parameter-mode cells into canonical wire signatures. They
//! are not used at runtime, so they deliberately live outside the runtime
//! module — keeping `src/wire_semantics.rs` free of dead code without
//! needing a blanket `#[allow(dead_code)]`.

use super::wire_semantics_runtime::{normalize_expiration, wire_right_opt, wire_strike_opt};

/// Canonical token used by build-time wire-shape signatures for
/// proto-unset optional fields.
pub(crate) const UNSET_WIRE_ARG_SENTINEL: &str = "<unset>";

/// Canonicalize an argument the same way the runtime request builder does.
///
/// Build-time mode collapsing uses this to decide whether two cells produce
/// identical wire requests.
pub(crate) fn canonicalize_wire_arg(param_name: &str, value: &str) -> String {
    match param_name {
        "expiration" => normalize_expiration(value),
        "strike" => wire_strike_opt(value).unwrap_or_else(|| UNSET_WIRE_ARG_SENTINEL.to_string()),
        "right" => wire_right_opt(value).unwrap_or_else(|| UNSET_WIRE_ARG_SENTINEL.to_string()),
        _ => value.to_string(),
    }
}
