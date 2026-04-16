//! Live parameter-mode matrix validator generator for the C++ SDK.
//!
//! Emits `sdks/cpp/examples/validate.cpp`: one `cell(...)` lambda per
//! (endpoint, mode). Each cell sets `EndpointRequestOptions::timeout_ms`
//! to 60_000 so the Rust SDK enforces the deadline and throws a
//! `tdx::Error` carrying "Request deadline exceeded" on expiry (W3).
//! The gRPC stream is cancelled by the SDK; the `Client` handle stays
//! usable. Preamble/cell/postamble templates live in
//! `templates/validate_cpp/`.

use std::fmt::Write as _;

use super::super::helpers::{
    cpp_arg_literal, cpp_builder_setter, is_streaming_endpoint, method_params,
};
use super::super::model::{GeneratedEndpoint, TestFixtures};
use super::super::modes::test_modes_for;

/// Generate the C++ SDK validator (one row per (endpoint, mode) pair).
pub(super) fn render_cpp_validate(
    endpoints: &[GeneratedEndpoint],
    fixtures: &TestFixtures,
) -> String {
    let mut out = String::from(include_str!("templates/validate_cpp/preamble.cpp.tmpl"));
    for endpoint in endpoints
        .iter()
        .filter(|endpoint| !is_streaming_endpoint(endpoint))
    {
        let mp = method_params(endpoint);
        for mode in test_modes_for(endpoint, fixtures) {
            let mut args_parts: Vec<String> = mp
                .iter()
                .zip(mode.args.iter())
                .map(|(param, value)| cpp_arg_literal(param, value))
                .collect();
            // Every cell carries the cross-cutting per-call deadline (W3).
            // Other builder overrides (if any) are chained into the same
            // EndpointRequestOptions via the fluent `with_<name>` setters.
            let setters: String = mode
                .builder_overrides
                .iter()
                .filter_map(|(name, value)| cpp_builder_setter(endpoint, name, value))
                .collect();
            // Bulk-chain / all-strike cells use `kSlowModeTimeoutMs` since
            // a full option chain payload legitimately takes longer than
            // 60s; all other cells use `kPerCellTimeoutMs`.
            let timeout_sym = if matches!(mode.name.as_str(), "all_strikes_one_exp" | "bulk_chain")
            {
                "kSlowModeTimeoutMs"
            } else {
                "kPerCellTimeoutMs"
            };
            args_parts.push(format!(
                "tdx::EndpointRequestOptions{{}}{setters}.with_timeout_ms({timeout_sym})"
            ));
            let args = args_parts.join(", ");
            write!(
                out,
                include_str!("templates/validate_cpp/cell.cpp.tmpl"),
                endpoint = endpoint.name,
                mode = mode.name,
                min_tier = mode.min_tier,
                rationale = mode.rationale,
                endpoint_name = endpoint.name,
                args = args,
            )
            .unwrap();
        }
    }
    out.push_str(include_str!("templates/validate_cpp/postamble.cpp.tmpl"));
    out
}
