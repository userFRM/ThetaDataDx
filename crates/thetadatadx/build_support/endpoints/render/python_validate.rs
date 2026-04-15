//! Live parameter-mode matrix validator generator for the Python SDK.
//!
//! Emits `scripts/validate_python.py`: one callable-driven row per
//! (endpoint, mode) cell, including builder-override overlays. The script
//! body lives in `templates/validate_python/` so syntax highlighting works
//! and the per-cell line is a format template rather than a push_str soup.

use std::fmt::Write as _;

use super::super::helpers::{
    is_streaming_endpoint, method_params, python_arg_literal, python_builder_kwarg,
};
use super::super::model::GeneratedEndpoint;
use super::super::modes::test_modes_for;

/// Generate the Python SDK validator (one row per (endpoint, mode) pair).
pub(super) fn render_python_validate(endpoints: &[GeneratedEndpoint]) -> String {
    let mut out = String::from(include_str!("templates/validate_python/preamble.py.tmpl"));
    for endpoint in endpoints
        .iter()
        .filter(|endpoint| !is_streaming_endpoint(endpoint))
    {
        let mp = method_params(endpoint);
        for mode in test_modes_for(endpoint) {
            let mut args_parts: Vec<String> = mp
                .iter()
                .zip(mode.args.iter())
                .map(|(param, value)| python_arg_literal(param, value))
                .collect();
            for (name, value) in &mode.builder_overrides {
                if let Some(kwarg) = python_builder_kwarg(endpoint, name, value) {
                    args_parts.push(kwarg);
                }
            }
            let args = args_parts.join(", ");
            write!(
                out,
                include_str!("templates/validate_python/cell.py.tmpl"),
                endpoint = endpoint.name,
                mode = mode.name,
                min_tier = mode.min_tier,
                endpoint_name = endpoint.name,
                args = args,
            )
            .unwrap();
        }
    }
    out.push_str(include_str!("templates/validate_python/postamble.py.tmpl"));
    out
}
