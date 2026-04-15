//! Live parameter-mode matrix validator generator for the C++ SDK.
//!
//! Emits `sdks/cpp/examples/validate.cpp`: one `cell(...)` lambda per
//! (endpoint, mode), behind a `std::packaged_task` + detached `std::thread`
//! timeout shim and `_Exit` teardown when any cell timed out. The lambda
//! helper, main wrapper, and artifact writer all live in
//! `templates/validate_cpp/`.

use std::fmt::Write as _;

use super::super::helpers::{
    cpp_arg_literal, cpp_builder_setter, is_streaming_endpoint, method_params,
};
use super::super::model::GeneratedEndpoint;
use super::super::modes::test_modes_for;

/// Generate the C++ SDK validator (one row per (endpoint, mode) pair).
pub(super) fn render_cpp_validate(endpoints: &[GeneratedEndpoint]) -> String {
    let mut out = String::from(include_str!("templates/validate_cpp/preamble.cpp.tmpl"));
    for endpoint in endpoints
        .iter()
        .filter(|endpoint| !is_streaming_endpoint(endpoint))
    {
        let mp = method_params(endpoint);
        for mode in test_modes_for(endpoint) {
            let mut args_parts: Vec<String> = mp
                .iter()
                .zip(mode.args.iter())
                .map(|(param, value)| cpp_arg_literal(param, value))
                .collect();
            if !mode.builder_overrides.is_empty() {
                let setters: String = mode
                    .builder_overrides
                    .iter()
                    .filter_map(|(name, value)| cpp_builder_setter(endpoint, name, value))
                    .collect();
                if !setters.is_empty() {
                    args_parts.push(format!("tdx::EndpointRequestOptions{{}}{setters}"));
                }
            }
            let args = args_parts.join(", ");
            write!(
                out,
                include_str!("templates/validate_cpp/cell.cpp.tmpl"),
                endpoint = endpoint.name,
                mode = mode.name,
                min_tier = mode.min_tier,
                endpoint_name = endpoint.name,
                args = args,
            )
            .unwrap();
        }
    }
    out.push_str(include_str!("templates/validate_cpp/postamble.cpp.tmpl"));
    out
}
