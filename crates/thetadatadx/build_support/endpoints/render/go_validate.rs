//! Live parameter-mode matrix validator generator for the Go SDK.
//!
//! Emits `sdks/go/validate.go`: one goroutine-backed cell per
//! (endpoint, mode), with a select/timeout wrapper and a shared classifier.
//! The function body and helpers live in `templates/validate_go/`.

use std::fmt::Write as _;

use super::super::helpers::{
    go_arg_literal, go_builder_option, is_streaming_endpoint, method_params, to_go_exported_name,
};
use super::super::model::{GeneratedEndpoint, TestFixtures};
use super::super::modes::test_modes_for;

/// Generate the Go SDK validator (one row per (endpoint, mode) pair).
pub(super) fn render_go_validate(
    endpoints: &[GeneratedEndpoint],
    fixtures: &TestFixtures,
) -> String {
    let mut out = String::from(include_str!("templates/validate_go/preamble.go.tmpl"));
    for endpoint in endpoints
        .iter()
        .filter(|endpoint| !is_streaming_endpoint(endpoint))
    {
        let go_name = to_go_exported_name(&endpoint.name);
        let mp = method_params(endpoint);
        for mode in test_modes_for(endpoint, fixtures) {
            let mut args_parts: Vec<String> = mp
                .iter()
                .zip(mode.args.iter())
                .map(|(param, value)| go_arg_literal(param, value))
                .collect();
            for (name, value) in &mode.builder_overrides {
                if let Some(opt) = go_builder_option(endpoint, name, value) {
                    args_parts.push(opt);
                }
            }
            // Cross-cutting deadline (W3): SDK enforces and cancels on expiry.
            // Bulk-chain / all-strike cells use `slowModeTimeoutMs` since a
            // full option chain payload legitimately takes longer than 60s.
            let timeout_sym = if matches!(mode.name.as_str(), "all_strikes_one_exp" | "bulk_chain")
            {
                "slowModeTimeoutMs"
            } else {
                "perCellTimeoutMs"
            };
            args_parts.push(format!("WithTimeoutMs({timeout_sym})"));
            let args = args_parts.join(", ");
            write!(
                out,
                include_str!("templates/validate_go/cell.go.tmpl"),
                endpoint = endpoint.name,
                mode = mode.name,
                min_tier = mode.min_tier,
                rationale = mode.rationale,
                go_method_name = go_name,
                args = args,
            )
            .unwrap();
        }
        out.push('\n');
    }
    out.push_str(include_str!("templates/validate_go/postamble.go.tmpl"));
    out
}
