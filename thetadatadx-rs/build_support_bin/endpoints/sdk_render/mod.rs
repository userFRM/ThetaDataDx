//! Per-language emitters for the checked-in SDK projections.
//!
//! Each submodule owns one render target (Python, TypeScript, C++, FFI,
//! per-language live-validators, enums). The build script never compiles
//! this tree — only the `generate_sdk_surfaces` binary reaches here.

mod config_accessors;
mod cpp;
mod cpp_validate;
mod doc;
mod enums;
mod ffi;
mod python;
mod python_stub;
mod python_validate;
mod sdk_files;
mod typescript;

pub(super) use sdk_files::{check_sdk_generated_files, write_sdk_generated_files};

/// Whether a streamed endpoint's `.stream` output can fan out across
/// concurrent sub-requests under `bulk_fetch = "auto"`, so its stream
/// docs must carry the chunks-interleave-across-bands caveat; every
/// other endpoint runs single-stream and must not claim a fan-out.
///
/// Same predicate as the runtime shardable set
/// (`SHARDABLE_HISTORY_ENDPOINTS` in `crate::mdds::shard`, tied to the
/// registry by `shardable_set_matches_registry_bandable`): an intraday
/// `history*` endpoint carrying the `start_time` / `end_time` window
/// filters, or an `at_time` family banding its date range. Derived
/// from the endpoint model instead of a name list so the generated
/// docs can never drift from the runtime set.
fn endpoint_can_fan_out(endpoint: &super::model::GeneratedEndpoint) -> bool {
    (endpoint.subcategory.starts_with("history")
        && endpoint.params.iter().any(|p| p.name == "start_time")
        && endpoint.params.iter().any(|p| p.name == "end_time"))
        || endpoint.subcategory == "at_time"
}
