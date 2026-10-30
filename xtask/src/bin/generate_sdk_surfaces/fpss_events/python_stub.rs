//! The streaming event classes of the Python type stub.

use std::fmt::Write as _;

use heck::ToSnakeCase;

use super::data_then_control;

pub(crate) const BEGIN_MARKER: &str = "# --- BEGIN GENERATED STREAMING EVENTS (event table) ---";
pub(crate) const END_MARKER: &str = "# --- END GENERATED STREAMING EVENTS ---";

/// The Python type of an event column.
fn py_type(ty: &str) -> &'static str {
    match ty {
        "i32" | "i64" | "u64" | "u8" => "int",
        "f64" => "float",
        "String" => "str",
        "Bytes" => "bytes",
        "Contract" => "ContractRef",
        other => panic!("no Python type for event column type '{other}'"),
    }
}

/// The region of `__init__.pyi` holding one class per streaming event: the
/// data events, then the control events. The class and column documentation
/// is the table's, which the runtime classes carry too.
pub(crate) fn render_python_event_stubs() -> String {
    let mut out = format!(
        "{BEGIN_MARKER}\n\
         # Generated from the streaming event table; do not edit by hand. Run\n\
         # `cargo run -p xtask --bin generate_sdk_surfaces` to refresh.\n"
    );
    for event in data_then_control() {
        write!(
            out,
            "\n\n@final\nclass {}:\n    \"\"\"{}\"\"\"\n\n",
            event.name, event.doc
        )
        .unwrap();
        for column in event.columns {
            writeln!(
                out,
                "    {}: {}\n    \"\"\"{}\"\"\"",
                column.name,
                py_type(column.ty),
                column.doc
            )
            .unwrap();
        }
        if !event.columns.is_empty() {
            out.push('\n');
        }
        writeln!(
            out,
            "    @property\n    def kind(self) -> str:\n        \
             \"\"\"Event kind discriminator (``\"{}\"``).\"\"\"\n        ...\n",
            event.name.to_snake_case()
        )
        .unwrap();
        if event.disconnect {
            out.push_str(
                "    @property\n    def reason_name(self) -> str:\n        \
                 \"\"\"Resolved disconnect-reason name (e.g. `\"TooManyRequests\"`, `\"InvalidCredentials\"`, \
                 `\"Unspecified\"` for unknown codes). Derived from the wire-level `reason` integer.\"\"\"\n        \
                 ...\n\n    \
                 @property\n    def cause_name(self) -> str:\n        \
                 \"\"\"Resolved disconnect-cause name: `\"ServerSent\"`, `\"ClosedByServer\"`, `\"ReadFailed\"`, \
                 `\"WriteFailed\"` or `\"ReadTimeout\"`. Derived from the `cause` integer.\"\"\"\n        \
                 ...\n\n",
            );
        }
        out.push_str(
            "    def __repr__(self) -> str:\n        \
             \"\"\"Return a representation of the event.\"\"\"\n        ...\n",
        );
    }
    out.push_str("\n\n");
    out.push_str(END_MARKER);
    out
}
