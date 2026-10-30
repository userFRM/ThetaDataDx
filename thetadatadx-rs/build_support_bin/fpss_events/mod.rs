//! The streaming event table, expanded into plain records, and the C header
//! and C++ layout asserts rendered from them.
//!
//! The bindings expand `thetadatadx::__events!` themselves; the writer renders
//! only what is not Rust:
//!
//! - `thetadatadx-cpp/include/fpss_event_structs.h.inc`: the C mirror of the
//!   FFI event structs, `#include`'d from `thetadatadx.h`;
//! - `thetadatadx-cpp/include/fpss_layout_asserts.hpp.inc`: the C++
//!   `static_assert`s pinning every size and offset of that mirror. The FFI
//!   crate checks its Rust layout against the same lines.

use std::path::Path;

use heck::ToSnakeCase;

mod cpp_asserts;
mod ffi_c;
mod layout;

/// One event of the table, in kind-tag order.
pub(crate) struct Event {
    pub(crate) name: &'static str,
    /// A market-data event, as opposed to a control event.
    pub(crate) data: bool,
    pub(crate) doc: &'static str,
    pub(crate) columns: &'static [Column],
}

/// One column of an event: its name and table type (`i32`, `i64`, `u64`,
/// `u8`, `f64`, `String`, `Bytes` or `Contract`).
pub(crate) struct Column {
    pub(crate) name: &'static str,
    pub(crate) ty: &'static str,
}

impl Event {
    /// The event's field name on the tagged C event.
    fn field(&self) -> String {
        self.name.to_snake_case()
    }
}

macro_rules! event_records {
    ({} $( $(#[$attr:ident])* $kind:ident $name:ident {
        doc: $doc:literal,
        $(from: $v:ident $({ $($b:ident),* })?,)?
        columns: [$($c:ident : $t:ident $(= $e:tt)? $d:literal),* $(,)?],
    } )*) => {
        &[$(Event {
            name: stringify!($name),
            data: event_records!(@data $kind),
            doc: $doc,
            columns: &[$(Column { name: stringify!($c), ty: stringify!($t) }),*],
        }),*]
    };
    (@data data) => { true };
    (@data control) => { false };
}

/// Every streaming event, in table order.
pub(crate) const EVENTS: &[Event] = thetadatadx::__events! { event_records {{}} };

/// The data events, then the control events, each in table order: the order
/// the tagged C event embeds them.
fn data_then_control() -> impl Iterator<Item = &'static Event> {
    EVENTS
        .iter()
        .filter(|e| e.data)
        .chain(EVENTS.iter().filter(|e| !e.data))
}

fn render_files() -> [(&'static str, String); 2] {
    [
        (
            "thetadatadx-cpp/include/fpss_event_structs.h.inc",
            ffi_c::render_c_fpss_event_header(),
        ),
        (
            "thetadatadx-cpp/include/fpss_layout_asserts.hpp.inc",
            cpp_asserts::render_cpp_fpss_layout_asserts(),
        ),
    ]
}

/// Renders the streaming event headers and writes each to its path under the repository root.
pub fn write_sdk_generated_files(repo_root: &Path) -> Result<(), Box<dyn std::error::Error>> {
    for (path, contents) in render_files() {
        std::fs::write(repo_root.join(path), contents)?;
    }
    Ok(())
}

/// Verifies that the checked-in streaming event headers match freshly rendered output, returning an error on drift.
pub fn check_sdk_generated_files(repo_root: &Path) -> Result<(), Box<dyn std::error::Error>> {
    for (path, contents) in render_files() {
        let current = std::fs::read_to_string(repo_root.join(path)).unwrap_or_default();
        if current.replace("\r\n", "\n") != contents {
            return Err(format!(
                "generated file out of date: {path} (run: cargo run -p thetadatadx-rs --bin generate_sdk_surfaces --features config-file)"
            )
            .into());
        }
    }
    Ok(())
}
