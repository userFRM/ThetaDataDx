//! Live parameter-mode matrix validator generator for the CLI surface.
//!
//! Emits `scripts/validate_cli.py`: one subprocess-driven row per
//! (endpoint, mode) cell. CLI-specific caveat: builder-override cells are
//! skipped because clap positional args don't support targeted optional
//! injection. The script body lives in `templates/validate_cli/`.

use std::fmt::Write as _;

use super::super::helpers::{cli_command_tokens_for_mode, is_streaming_endpoint};
use super::super::model::{GeneratedEndpoint, TestFixtures};
use super::super::modes::test_modes_for;

/// Generate the CLI validator (one row per (endpoint, mode) pair).
pub(super) fn render_cli_validate(
    endpoints: &[GeneratedEndpoint],
    fixtures: &TestFixtures,
) -> String {
    let mut out = String::from(include_str!("templates/validate_cli/preamble.py.tmpl"));
    for endpoint in endpoints
        .iter()
        .filter(|endpoint| !is_streaming_endpoint(endpoint))
    {
        // CLI uses positional args, so builder-override cells (single-optional
        // isolation) are unrepresentable; drop them. See issue #290.
        for mode in test_modes_for(endpoint, fixtures)
            .into_iter()
            .filter(|mode| mode.builder_overrides.is_empty())
        {
            let tokens = cli_command_tokens_for_mode(endpoint, &mode)
                .into_iter()
                .map(|token| format!("{token:?}"))
                .collect::<Vec<_>>()
                .join(", ");
            write!(
                out,
                include_str!("templates/validate_cli/cell.py.tmpl"),
                endpoint = endpoint.name,
                mode = mode.name,
                min_tier = mode.min_tier,
                rationale = mode.rationale,
                args = tokens,
            )
            .unwrap();
        }
    }
    out.push_str(include_str!("templates/validate_cli/postamble.py.tmpl"));
    out
}
