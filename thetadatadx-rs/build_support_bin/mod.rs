//! Code-generation modules reachable only from `bin/generate_sdk_surfaces`.

mod endpoints;
mod fpss_events;
mod ticks;
mod upstream_openapi;
#[path = "../src/mdds/wire_semantics.rs"]
mod wire_semantics;

pub use endpoints::{
    check_docs_site_files, check_sdk_generated_files as check_endpoint_sdk_generated_files,
    write_docs_site_files, write_sdk_generated_files as write_endpoint_sdk_generated_files,
};
pub use fpss_events::{
    check_sdk_generated_files as check_fpss_event_sdk_generated_files,
    write_sdk_generated_files as write_fpss_event_sdk_generated_files,
};
pub use ticks::{
    check_sdk_generated_files as check_tick_sdk_generated_files,
    write_sdk_generated_files as write_tick_sdk_generated_files,
};
