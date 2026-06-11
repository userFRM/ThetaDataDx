//! Code-generation modules reachable only from
//! `bin/generate_sdk_surfaces` and `bin/generate_docs_site`.
//!
//! Items here are physically separate from `build_support/` so the build
//! script never compiles them. The build script's compile unit reads
//! `crates/thetadatadx/build_support/mod.rs`; the binary's compile unit
//! reads this file. Each declares only the modules its compile unit
//! actually needs, which is why neither carries any `#[allow(dead_code)]`
//! umbrella attribute.
//!
//! Shared core modules (`endpoints::model`, `endpoints::parser`,
//! `endpoints::helpers`, `endpoints::proto_parser`, `ticks::schema`) live
//! in `build_support/` and are pulled in here via `#[path]` aliases.

mod endpoints;
mod fpss_events;
mod sdk_surface;
mod ticks;
mod upstream_openapi;
#[path = "../src/mdds/wire_semantics.rs"]
mod wire_semantics;

// Consumed by `generate_docs_site` only; when `generate_sdk_surfaces`
// is compiled with the `__internal` feature in the same invocation, the
// re-export is unused in that compile unit by design.
#[cfg(feature = "__internal")]
#[allow(unused_imports)]
pub use endpoints::{check_docs_site_files, write_docs_site_files};
pub use endpoints::{
    check_sdk_generated_files as check_endpoint_sdk_generated_files,
    write_sdk_generated_files as write_endpoint_sdk_generated_files,
};
pub use fpss_events::{
    check_sdk_generated_files as check_fpss_event_sdk_generated_files,
    write_sdk_generated_files as write_fpss_event_sdk_generated_files,
};
pub use sdk_surface::{
    check_sdk_generated_files as check_sdk_surface_generated_files,
    write_sdk_generated_files as write_sdk_surface_generated_files,
};
pub use ticks::{
    check_sdk_generated_files as check_tick_sdk_generated_files,
    write_sdk_generated_files as write_tick_sdk_generated_files,
};
