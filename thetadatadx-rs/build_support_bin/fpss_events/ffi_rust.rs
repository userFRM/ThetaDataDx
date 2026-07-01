//! Rust FFI emitters for `thetadatadx-ffi/src/fpss_event_structs.rs` +
//! `thetadatadx-ffi/src/fpss_event_converter.rs`.
//!
//! Flattens every `kind = "control"` schema variant into its own
//! `#[repr(C)]` struct alongside the data-variant structs. The tagged
//! `ThetaDataDxStreamEvent` wrapper embeds all of them by value; only the field
//! matching `kind` carries valid data on each delivered event. Empty
//! control variants get a single `_padding: u8` so their C-side
//! `sizeof` is portable across GCC / Clang / MSVC (Rust `#[repr(C)]`
//! treats a zero-field struct as size 0; C does not).

use std::fmt::Write as _;

use super::common::{
    control_rust_variant, is_byte_buffer, is_contract, rust_ffi_scalar, rust_ffi_zero_literal,
    snake_case, zero_const_name,
};
use super::layout::{
    c_struct_size, contract_align, contract_size, fpss_event_align, fpss_event_size, struct_align,
};
use super::schema::{
    sorted_control_events, sorted_data_events, sorted_event_names, ColumnDef, EventDef, Schema,
};

/// Emit the kind-enum: one discriminant per data variant + one per
/// control variant. Schema-driven so adding a control variant to
/// `fpss_event_schema.toml` propagates automatically. Names emitted in
/// the same alphabetical order every other emitter consumes
/// (`sorted_event_names`). The schema no longer carries a `RawData`
/// fallback variant — decoder failures are filtered before the FFI
/// boundary and accounted on the `thetadatadx.fpss.decode_failures`
/// metric.
fn render_kind_enum_rust(schema: &Schema) -> String {
    let mut out = String::new();
    out.push_str("/// FPSS event kind tag. Check this to determine which field of\n");
    out.push_str("/// `ThetaDataDxStreamEvent` is valid. One discriminant per data variant\n");
    out.push_str("/// (Quote / Trade / OpenInterest / Ohlcvc) and one per control\n");
    out.push_str("/// variant (LoginSuccess / ContractAssigned / ...). Schema-driven\n");
    out.push_str("/// from `fpss_event_schema.toml`; values are stable for the\n");
    out.push_str("/// lifetime of the current C ABI but may renumber on a future major\n");
    out.push_str("/// bump.\n");
    out.push_str("#[repr(C)]\n");
    out.push_str("pub enum ThetaDataDxStreamEventKind {\n");
    for (idx, name) in sorted_event_names(schema).iter().enumerate() {
        writeln!(
            out,
            "    /// `{name}` event; the `{}` field carries its payload.",
            snake_case(name)
        )
        .unwrap();
        writeln!(out, "    {name} = {idx},").unwrap();
    }
    out.push_str("}\n\n");
    out
}

/// Emit the `ThetaDataDxContract` struct + `ZERO_CONTRACT_STRUCT` const that every
/// event's `contract` field points at. Uses `#[repr(C)]` so the C header
/// mirror gets byte-identical layout.
///
/// Strings (the `symbol` field) cross as C strings — the field is
/// `*const c_char` backed by a `CString` inside `FfiBufferedEvent` so the
/// pointer stays valid for the lifetime of the buffered event. Optional
/// fields use a tagged-optional pattern (`has_*: bool` + value) because
/// `#[repr(C)]` rejects `Option<T>` for Rust->C interop.
fn render_contract_struct_rust() -> &'static str {
    "/// FPSS `Contract` shared across every data event.\n\
/// \n\
/// `symbol` is a NUL-terminated C string; may be null when the SDK has not\n\
/// yet resolved the server-assigned contract_id to a `ContractAssigned`\n\
/// frame. Optional option fields (`expiration`, `right`, `strike`) use a\n\
/// tagged-present bool because `#[repr(C)]` cannot express `Option<T>`\n\
/// directly. `right` is the ASCII byte `b'C'` / `b'P'` (`0` when\n\
/// `has_right` is false) and `strike` is the option strike in dollars —\n\
/// the same notation the public option builder takes.\n\
#[repr(C)]\n\
pub struct ThetaDataDxContract {\n\
    /// Ticker symbol (e.g. \"AAPL\"). Null until ContractAssigned arrives.\n\
    pub symbol: *const c_char,\n\
    /// Security type code — matches `thetadatadx::SecType`.\n\
    pub sec_type: i32,\n\
    /// Whether `expiration` is meaningful (options only).\n\
    pub has_expiration: bool,\n\
    /// Option expiration date as `YYYYMMDD` (0 when `has_expiration` is false).\n\
    pub expiration: i32,\n\
    /// Whether `right` is meaningful (options only).\n\
    pub has_right: bool,\n\
    /// Option side as ASCII: `b'C'` / `b'P'` (`0` when `has_right` is false).\n\
    pub right: c_char,\n\
    /// Whether `strike` is meaningful (options only).\n\
    pub has_strike: bool,\n\
    /// Option strike price in dollars (0.0 when `has_strike` is false).\n\
    pub strike: f64,\n\
    /// Option strike in thousandths of a dollar (a `$550.00` strike is\n\
    /// `550000`; `0` when `has_strike` is false). The exact integer the\n\
    /// wire carries — read this for an exact key, `strike` for dollars.\n\
    pub strike_thousandths: i32,\n\
}\n\
\n\
/// Zeroed `ThetaDataDxContract` literal: null symbol, all-false presence flags, zero scalars.\n\
pub(crate) const ZERO_CONTRACT_STRUCT: ThetaDataDxContract = ThetaDataDxContract {\n\
    symbol: ptr::null(),\n\
    sec_type: 0,\n\
    has_expiration: false,\n\
    expiration: 0,\n\
    has_right: false,\n\
    right: 0,\n\
    has_strike: false,\n\
    strike: 0.0,\n\
    strike_thousandths: 0,\n\
};\n\n"
}

/// Factual one-line doc for a generated event-struct field, keyed on the
/// schema column name. Mirrors the field docs on the matching Rust core
/// `StreamData::*` / `StreamControl::*` variant so the C-ABI surface reads the
/// same as the native one. Unknown names fall back to a humanized form of
/// the column name so every emitted field still carries a doc.
fn fpss_column_doc(name: &str) -> String {
    let doc = match name {
        "ask" => "Ask price.",
        "ask_condition" => "Quote condition code for the ask.",
        "ask_exchange" => "Exchange code posting the ask.",
        "ask_size" => "Number of contracts/shares resting at the ask.",
        "attempt" => "1-based index of this reconnect attempt.",
        "attempts" => "Number of consecutive reconnect attempts consumed before giving up.",
        "bid" => "Bid price.",
        "bid_condition" => "Quote condition code for the bid.",
        "bid_exchange" => "Exchange code posting the bid.",
        "bid_size" => "Number of contracts/shares resting at the bid.",
        "close" => "Closing price of the bar.",
        "code" => "Unrecognized frame code reported by the server.",
        "condition" => "Primary trade condition code.",
        "condition_flags" => "Bit flags qualifying the trade conditions.",
        "contract" => "Contract this event refers to.",
        "count" => "Number of trades aggregated into the bar.",
        "date" => "Trading date as `YYYYMMDD`.",
        "delay_ms" => "Delay, in milliseconds, before the attempt fires.",
        "exchange" => "Exchange code where the trade printed.",
        "ext_condition1" => "Extended trade condition code 1.",
        "ext_condition2" => "Extended trade condition code 2.",
        "ext_condition3" => "Extended trade condition code 3.",
        "ext_condition4" => "Extended trade condition code 4.",
        "high" => "Highest traded price within the bar.",
        "id" => "Wire-internal contract id the FPSS server assigns to this contract.",
        "low" => "Lowest traded price within the bar.",
        "market_ask" => "Calculated market ask (dollars), nudged from the quote ask.",
        "market_bid" => "Calculated market bid (dollars), nudged from the quote bid.",
        "market_price" => "Integer midpoint of `market_bid` / `market_ask` (dollars).",
        "message" => "Human-readable error text from the server.",
        "ms_of_day" => "Milliseconds since midnight Eastern Time when the event was recorded.",
        "open" => "Opening price of the bar.",
        "open_interest" => "Number of outstanding open contracts.",
        "payload" => "Raw frame payload bytes, preserved for diagnostics.",
        "permissions" => {
            "Server \"Bundle\" string copied verbatim from the METADATA frame; opaque diagnostic metadata."
        }
        "price" => "Trade price.",
        "price_flags" => "Bit flags qualifying the trade price.",
        "reason" => "Reason the server gave for dropping the connection.",
        "received_at_ns" => {
            "Wall-clock nanoseconds since UNIX epoch, captured at frame decode time."
        }
        "records_back" => {
            "Number of records back this trade was reported (out-of-order correction offset)."
        }
        "req_id" => "Identifier of the subscription request this response answers.",
        "result" => "Outcome of the subscription request.",
        "sequence" => "Exchange sequence number for ordering trades within the day.",
        "size" => "Trade size in contracts/shares.",
        "volume" => "Total traded volume within the bar, in contracts/shares.",
        "volume_type" => "Volume classification code for the trade.",
        other => return format!("`{other}` field."),
    };
    doc.to_string()
}

/// Render one `#[repr(C)]` struct for a data or control variant. For
/// `Vec<u8>` columns the schema's single logical name expands into two
/// physical fields: `<name>: *const u8` and `<name>_len: usize`. Empty
/// (unit) control variants get a single `_padding: u8` so their size is
/// 1 on every C compiler — Rust's zero-field `#[repr(C)]` is size 0,
/// which would mismatch MSVC's mandatory size-1 minimum.
fn render_event_struct_rust(out: &mut String, event_name: &str, def: &EventDef) {
    let doc_text = if def.doc.is_empty() {
        format!("`#[repr(C)]` FPSS {event_name} event.")
    } else {
        def.doc.clone()
    };
    for line in doc_text.lines() {
        writeln!(out, "/// {line}").unwrap();
    }
    out.push_str("#[repr(C)]\n");
    writeln!(out, "pub struct ThetaDataDxStream{event_name} {{").unwrap();
    if def.columns.is_empty() {
        out.push_str("    /// Placeholder so the struct has size 1 on every C compiler.\n");
        out.push_str("    /// Empty `#[repr(C)]` structs are size 0 on Rust but size 1 on\n");
        out.push_str("    /// MSVC; the byte keeps the layout consistent across both.\n");
        out.push_str("    pub _padding: u8,\n");
    } else {
        for column in &def.columns {
            let col_doc = fpss_column_doc(&column.name);
            if is_byte_buffer(&column.r#type) {
                writeln!(
                    out,
                    "    /// {col_doc} Pointer to the first byte; null when empty."
                )
                .unwrap();
                writeln!(out, "    pub {}: *const u8,", column.name).unwrap();
                writeln!(
                    out,
                    "    /// Length in bytes of the `{}` buffer.",
                    column.name
                )
                .unwrap();
                writeln!(out, "    pub {}_len: usize,", column.name).unwrap();
            } else {
                let ty = rust_ffi_scalar(&column.r#type, event_name, &column.name);
                writeln!(out, "    /// {col_doc}").unwrap();
                writeln!(out, "    pub {}: {ty},", column.name).unwrap();
            }
        }
    }
    out.push_str("}\n\n");
}

/// Emit one `const _: () = { ... };` block pinning a struct's `size_of`
/// and `align_of`. The compiler evaluates both intrinsics against the
/// real `#[repr(C)]` layout, so an accidental schema / field-width change
/// that shifts the Rust layout fails the build at the definition site,
/// before the generated C header and its C++ `static_assert` mirror can
/// drift apart. Numbers come from the same layout walk that feeds the C++
/// asserts, never hand-typed.
fn render_layout_assert(out: &mut String, type_name: &str, size: usize, align: usize) {
    writeln!(out, "const _: () = {{").unwrap();
    writeln!(
        out,
        "    assert!(core::mem::size_of::<{type_name}>() == {size});"
    )
    .unwrap();
    writeln!(
        out,
        "    assert!(core::mem::align_of::<{type_name}>() == {align});"
    )
    .unwrap();
    out.push_str("};\n");
}

/// Render the size + align asserts for every emitted `#[repr(C)]` struct:
/// the shared `ThetaDataDxContract`, each data and control variant, and
/// the tagged `ThetaDataDxStreamEvent` wrapper. Mirrors the C++
/// `static_assert` coverage so the layout is pinned on the Rust side that
/// defines the ABI, not only on the C++ consumer.
fn render_rust_layout_asserts(out: &mut String, schema: &Schema) {
    out.push_str("// Layout drift-guard: pin the `#[repr(C)]` size + alignment of every\n");
    out.push_str("// emitted struct on the Rust side that defines the ABI. A schema or\n");
    out.push_str("// field-width change that shifts a layout fails the build here, before\n");
    out.push_str("// the generated C header and its C++ asserts can drift out of sync.\n");
    render_layout_assert(
        out,
        "ThetaDataDxContract",
        contract_size(),
        contract_align(),
    );
    for (event_name, def) in sorted_data_events(schema) {
        render_layout_assert(
            out,
            &format!("ThetaDataDxStream{event_name}"),
            c_struct_size(&def.columns),
            struct_align(&def.columns),
        );
    }
    for (event_name, def) in sorted_control_events(schema) {
        render_layout_assert(
            out,
            &format!("ThetaDataDxStream{event_name}"),
            c_struct_size(&def.columns),
            struct_align(&def.columns),
        );
    }
    render_layout_assert(
        out,
        "ThetaDataDxStreamEvent",
        fpss_event_size(schema),
        fpss_event_align(schema),
    );
    out.push('\n');
}

/// Render the matching `ZERO_*` const for one variant. All scalars zero,
/// pointers null, lengths zero, contracts `ZERO_CONTRACT_STRUCT`, byte
/// buffers `(null, 0)`, padding bytes 0.
fn render_event_zero_const(out: &mut String, event_name: &str, def: &EventDef) {
    let const_name = zero_const_name(event_name);
    writeln!(
        out,
        "pub(crate) const {const_name}: ThetaDataDxStream{event_name} = ThetaDataDxStream{event_name} {{"
    )
    .unwrap();
    if def.columns.is_empty() {
        out.push_str("    _padding: 0,\n");
    } else {
        for column in &def.columns {
            if is_byte_buffer(&column.r#type) {
                writeln!(out, "    {}: ptr::null(),", column.name).unwrap();
                writeln!(out, "    {}_len: 0,", column.name).unwrap();
            } else {
                let lit = rust_ffi_zero_literal(&column.r#type);
                writeln!(out, "    {}: {lit},", column.name).unwrap();
            }
        }
    }
    out.push_str("};\n");
}

/// Emit the `#[repr(C)]` FPSS event structs + `ZERO_*` consts + tagged
/// `ThetaDataDxStreamEvent` for the Rust FFI crate.
///
/// The file is `include!`'d from `thetadatadx-ffi/src/lib.rs` BEFORE `FfiBufferedEvent`
/// so the hand-written backing-memory wrapper can name the generated
/// tagged struct. Required imports (`std::ffi::CString`,
/// `std::os::raw::c_char`, `std::ptr`) must already be in scope at the
/// include site — this file does not re-declare them.
pub(super) fn render_ffi_fpss_event_structs(schema: &Schema) -> String {
    let mut out = String::new();
    out.push_str(
        "// @generated DO NOT EDIT — regenerated by build.rs from fpss_event_schema.toml\n",
    );
    out.push_str("// Rust FFI `#[repr(C)]` FPSS event structs + ZERO_* consts + tagged\n");
    out.push_str(
        "// `ThetaDataDxStreamEvent`. `include!`'d from `thetadatadx-ffi/src/lib.rs`; do not hand-edit.\n",
    );
    out.push_str("// Expects `std::os::raw::c_char`, `std::ptr` already in scope at the\n");
    out.push_str("// include site.\n\n");

    // Kind enum — schema-driven; one discriminant per variant.
    out.push_str(&render_kind_enum_rust(schema));

    // Contract struct — every data variant's `contract` field points here.
    out.push_str(render_contract_struct_rust());

    // One #[repr(C)] struct per data variant.
    for (event_name, def) in sorted_data_events(schema) {
        render_event_struct_rust(&mut out, event_name, def);
    }

    // One #[repr(C)] struct per control variant. Empty variants get a
    // single _padding byte.
    for (event_name, def) in sorted_control_events(schema) {
        render_event_struct_rust(&mut out, event_name, def);
    }

    // Tagged union-style wrapper. Flat struct (not a C union) so the
    // layout is trivially FFI-safe.
    out.push_str("/// Tagged FPSS event for FFI. Check `kind` then read the corresponding\n");
    out.push_str("/// field. Only the field matching `kind` contains valid data — this is\n");
    out.push_str("/// a flat struct (not a C union) for simplicity and safety. The\n");
    out.push_str("/// per-variant control payloads (LoginSuccess, ContractAssigned, ...)\n");
    out.push_str("/// mirror the Rust `StreamControl::*` enum one-for-one; consumers\n");
    out.push_str("/// dispatch via `kind` and read the matching `event.<variant>` field.\n");
    out.push_str("#[repr(C)]\n");
    out.push_str("pub struct ThetaDataDxStreamEvent {\n");
    out.push_str("    /// Discriminant selecting which payload field below is valid.\n");
    out.push_str("    pub kind: ThetaDataDxStreamEventKind,\n");
    for (event_name, _) in sorted_data_events(schema) {
        let field = snake_case(event_name);
        writeln!(
            out,
            "    /// `{event_name}` payload; valid when `kind` is `{event_name}`."
        )
        .unwrap();
        writeln!(out, "    pub {field}: ThetaDataDxStream{event_name},").unwrap();
    }
    for (event_name, _) in sorted_control_events(schema) {
        let field = snake_case(event_name);
        writeln!(
            out,
            "    /// `{event_name}` payload; valid when `kind` is `{event_name}`."
        )
        .unwrap();
        writeln!(out, "    pub {field}: ThetaDataDxStream{event_name},").unwrap();
    }
    out.push_str("}\n\n");

    // Size + align drift-guard for every emitted `#[repr(C)]` struct.
    render_rust_layout_asserts(&mut out, schema);

    // Zero-initialised defaults for inactive fields.
    out.push_str("// Zero-initialized defaults for inactive union-style fields.\n");
    for (event_name, def) in sorted_data_events(schema) {
        render_event_zero_const(&mut out, event_name, def);
    }
    for (event_name, def) in sorted_control_events(schema) {
        render_event_zero_const(&mut out, event_name, def);
    }

    // Fully-zeroed event used as the `..ZERO_STREAM_EVENT` struct-update base
    // in the converter: each arm names its `kind` plus the one active payload
    // field, and this fills every inactive sibling with its zero const.
    render_zero_stream_event(&mut out, schema);

    out
}

/// Emit the `ZERO_STREAM_EVENT` const: every payload field set to its
/// matching `ZERO_*` const. Converter arms spread it with
/// `..ZERO_STREAM_EVENT` after naming `kind` and the active payload, so
/// the inactive siblings are zeroed in one line instead of listing all of
/// them per arm. `kind` is `UnknownControl` (the payload-less sentinel) but
/// every arm overrides it, so the base discriminant is never observed.
fn render_zero_stream_event(out: &mut String, schema: &Schema) {
    out.push_str(
        "pub(crate) const ZERO_STREAM_EVENT: ThetaDataDxStreamEvent = ThetaDataDxStreamEvent {\n",
    );
    out.push_str("    kind: ThetaDataDxStreamEventKind::UnknownControl,\n");
    for (event_name, _) in sorted_data_events(schema) {
        let field = snake_case(event_name);
        let zero = zero_const_name(event_name);
        writeln!(out, "    {field}: {zero},").unwrap();
    }
    for (event_name, _) in sorted_control_events(schema) {
        let field = snake_case(event_name);
        let zero = zero_const_name(event_name);
        writeln!(out, "    {field}: {zero},").unwrap();
    }
    out.push_str("};\n");
}

/// True if a control variant carries a backing `String` field — i.e.
/// the `FfiBufferedEvent` for this variant must own a `CString` to keep
/// the borrowed `*const c_char` alive across the user callback.
fn control_has_string(columns: &[ColumnDef]) -> Option<&str> {
    columns
        .iter()
        .find(|c| c.r#type == "String")
        .map(|c| c.name.as_str())
}

/// True if a control variant carries a backing `Vec<u8>` field — i.e.
/// the `FfiBufferedEvent` for this variant must own a `Vec<u8>` to keep
/// the borrowed `*const u8` alive across the user callback.
fn control_has_byte_buffer(columns: &[ColumnDef]) -> Option<&str> {
    columns
        .iter()
        .find(|c| c.r#type == "Vec<u8>")
        .map(|c| c.name.as_str())
}

/// True if a control variant embeds a `Contract`. Only `ContractAssigned`
/// hits this today, but the codegen handles it by-schema so future
/// `Contract`-bearing control variants drop in without touching this
/// file.
fn control_has_contract(columns: &[ColumnDef]) -> Option<&str> {
    columns
        .iter()
        .find(|c| is_contract(&c.r#type))
        .map(|c| c.name.as_str())
}

/// Emit the data-event match arm. Each arm fills the matching
/// `ThetaDataDxStream{Variant}` field, captures the contract symbol into
/// `_contract_symbol`, and zero-fills every sibling field.
fn render_data_arm(out: &mut String, event_name: &str, def: &EventDef) {
    let has_contract = def.columns.iter().any(|c| is_contract(&c.r#type));
    writeln!(out, "        StreamEvent::Data(StreamData::{event_name} {{").unwrap();
    for column in &def.columns {
        writeln!(out, "            {},", column.name).unwrap();
    }
    out.push_str("            ..\n");
    out.push_str("        }) => {\n");

    if has_contract {
        out.push_str(
            "            let contract_symbol_cstring = if contract.symbol.is_empty() {\n                None\n            } else {\n                std::ffi::CString::new(&contract.symbol[..]).ok()\n            };\n            let contract_symbol_ptr = contract_symbol_cstring\n                .as_ref()\n                .map_or(ptr::null(), |cs| cs.as_ptr());\n            let thetadatadx_contract = ThetaDataDxContract {\n                symbol: contract_symbol_ptr,\n                sec_type: contract.sec_type as i32,\n                has_expiration: contract.expiration.is_some(),\n                expiration: contract.expiration.unwrap_or(0),\n                has_right: contract.is_call.is_some(),\n                right: contract.right().map_or(0, |r| r.as_char() as c_char),\n                has_strike: contract.strike_thousandths.is_some(),\n                strike: contract.strike_dollars().unwrap_or(0.0),\n                strike_thousandths: contract.strike_thousandths.unwrap_or(0),\n            };\n",
        );
    }

    out.push_str("            FfiBufferedEvent {\n");
    out.push_str("                event: ThetaDataDxStreamEvent {\n");
    writeln!(
        out,
        "                    kind: ThetaDataDxStreamEventKind::{event_name},"
    )
    .unwrap();
    let field = snake_case(event_name);
    writeln!(
        out,
        "                    {field}: ThetaDataDxStream{event_name} {{"
    )
    .unwrap();
    for column in &def.columns {
        if is_contract(&column.r#type) {
            writeln!(
                out,
                "                        {}: thetadatadx_contract,",
                column.name
            )
            .unwrap();
        } else {
            writeln!(
                out,
                "                        {}: *{},",
                column.name, column.name
            )
            .unwrap();
        }
    }
    out.push_str("                    },\n");
    // Zero-fill all sibling data + control + raw fields.
    render_zero_fill_siblings(out, "                    ");
    out.push_str("                },\n");
    render_zero_buffered_storage(
        out,
        "                ",
        if has_contract {
            "contract_symbol_cstring"
        } else {
            "None"
        },
        "None",
        "None",
        "None",
    );
    out.push_str("            }\n");
    out.push_str("        }\n\n");
}

/// Emit the control-event match arms. One arm per `kind = "control"`
/// schema variant. Each arm constructs the corresponding typed
/// `ThetaDataDxStream{Variant}` payload, captures any `String` into
/// `_*_string`, any `Vec<u8>` into `_*_bytes`, and zero-fills every
/// sibling field on the tagged event.
fn render_control_arms(out: &mut String, schema: &Schema) {
    out.push_str("        StreamEvent::Control(ctrl) => match ctrl {\n");
    for (event_name, def) in sorted_control_events(schema) {
        if event_name == "UnknownControl" {
            // Wildcard arm at the end; StreamControl is #[non_exhaustive].
            continue;
        }
        render_control_arm(out, event_name, def);
    }
    // UnknownControl wildcard for any future / non-recognised variant.
    out.push_str("            _ => unknown_control_event(),\n");
    out.push_str("        },\n\n");
}

/// Per-variant Rust pattern (the brace-body) and the field-construction
/// list for the typed payload. Mirrors `control_variant_mapping` in
/// `buffered.rs` but renders into the FFI tagged-struct world rather
/// than the buffered-event world.
fn control_variant_mapping(event_name: &str) -> (&'static str, Vec<&'static str>) {
    match event_name {
        "LoginSuccess" => ("permissions", vec!["permissions: permissions_ptr"]),
        "ContractAssigned" => (
            "id, contract",
            vec!["id: *id", "contract: thetadatadx_contract"],
        ),
        "ReqResponse" => (
            "req_id, result",
            vec!["req_id: *req_id", "result: i32::from(*result as u8)"],
        ),
        "ServerError" => ("message", vec!["message: message_ptr"]),
        "Disconnected" => ("reason", vec!["reason: i32::from(*reason as i16)"]),
        "Reconnecting" => (
            "reason, attempt, delay_ms",
            vec![
                "reason: i32::from(*reason as i16)",
                "attempt: i32::try_from(*attempt).unwrap_or(i32::MAX)",
                "delay_ms: *delay_ms",
            ],
        ),
        "ReconnectsExhausted" => (
            "reason, attempts",
            vec![
                "reason: i32::from(*reason as i16)",
                "attempts: i32::try_from(*attempts).unwrap_or(i32::MAX)",
            ],
        ),
        "ParseError" => ("message", vec!["message: message_ptr"]),
        "UnknownFrame" => (
            "code, payload",
            vec![
                "code: *code",
                "payload: payload_ptr",
                "payload_len: payload_len_val",
            ],
        ),
        "Ping" => (
            "payload",
            vec!["payload: payload_ptr", "payload_len: payload_len_val"],
        ),
        "MarketOpen" | "MarketClose" | "Reconnected" | "Connected" | "ReconnectedServer"
        | "Restart" => ("", vec!["_padding: 0"]),
        other => panic!(
            "control variant '{other}' has no Rust→FFI mapping; \
             add it to control_variant_mapping in build_support/fpss_events/ffi_rust.rs"
        ),
    }
}

fn render_control_arm(out: &mut String, event_name: &str, def: &EventDef) {
    let rust_variant = control_rust_variant(event_name);
    let (rust_pattern, field_assigns) = control_variant_mapping(event_name);
    let has_string = control_has_string(&def.columns);
    let has_bytes = control_has_byte_buffer(&def.columns);
    let has_contract = control_has_contract(&def.columns);

    if rust_pattern.is_empty() {
        writeln!(out, "            StreamControl::{rust_variant} => {{").unwrap();
    } else {
        writeln!(
            out,
            "            StreamControl::{rust_variant} {{ {rust_pattern} }} => {{"
        )
        .unwrap();
    }

    // Stage backing storage for any borrowed pointer fields.
    if let Some(field) = has_string {
        writeln!(
            out,
            "                let cstring_owned = std::ffi::CString::new({field}.as_str()).ok();"
        )
        .unwrap();
        writeln!(
            out,
            "                let {field}_ptr = cstring_owned.as_ref().map_or(ptr::null(), |cs| cs.as_ptr());"
        )
        .unwrap();
    }
    if let Some(field) = has_bytes {
        writeln!(out, "                let bytes_owned = {field}.clone();").unwrap();
        writeln!(
            out,
            "                let payload_ptr = bytes_owned.as_ptr();"
        )
        .unwrap();
        writeln!(
            out,
            "                let payload_len_val = bytes_owned.len();"
        )
        .unwrap();
    }
    if let Some(field) = has_contract {
        writeln!(
            out,
            "                let contract_symbol_cstring = if {field}.symbol.is_empty() {{\n                    None\n                }} else {{\n                    std::ffi::CString::new(&{field}.symbol[..]).ok()\n                }};"
        )
        .unwrap();
        writeln!(
            out,
            "                let contract_symbol_ptr = contract_symbol_cstring\n                    .as_ref()\n                    .map_or(ptr::null(), |cs| cs.as_ptr());"
        )
        .unwrap();
        writeln!(
            out,
            "                let thetadatadx_contract = ThetaDataDxContract {{\n                    symbol: contract_symbol_ptr,\n                    sec_type: {field}.sec_type as i32,\n                    has_expiration: {field}.expiration.is_some(),\n                    expiration: {field}.expiration.unwrap_or(0),\n                    has_right: {field}.is_call.is_some(),\n                    right: {field}.right().map_or(0, |r| r.as_char() as c_char),\n                    has_strike: {field}.strike_thousandths.is_some(),\n                    strike: {field}.strike_dollars().unwrap_or(0.0),\n                    strike_thousandths: {field}.strike_thousandths.unwrap_or(0),\n                }};"
        )
        .unwrap();
    }

    out.push_str("                FfiBufferedEvent {\n");
    out.push_str("                    event: ThetaDataDxStreamEvent {\n");
    writeln!(
        out,
        "                        kind: ThetaDataDxStreamEventKind::{event_name},"
    )
    .unwrap();
    let field = snake_case(event_name);
    writeln!(
        out,
        "                        {field}: ThetaDataDxStream{event_name} {{"
    )
    .unwrap();
    for assign in field_assigns {
        writeln!(out, "                            {assign},").unwrap();
    }
    out.push_str("                        },\n");
    render_zero_fill_siblings(out, "                        ");
    out.push_str("                    },\n");

    let contract_slot = if has_contract.is_some() {
        "contract_symbol_cstring"
    } else {
        "None"
    };
    let permissions_slot = if event_name == "LoginSuccess" {
        "cstring_owned"
    } else {
        "None"
    };
    let message_slot = if matches!(event_name, "ServerError" | "ParseError") {
        "cstring_owned"
    } else {
        "None"
    };
    let bytes_slot = if has_bytes.is_some() {
        "Some(bytes_owned)"
    } else {
        "None"
    };
    render_zero_buffered_storage(
        out,
        "                    ",
        contract_slot,
        permissions_slot,
        message_slot,
        bytes_slot,
    );
    out.push_str("                }\n");
    out.push_str("            }\n");
}

/// Helper: zero every inactive sibling field on the tagged
/// `ThetaDataDxStreamEvent` via one `..ZERO_STREAM_EVENT` struct-update
/// line. The arm names `kind` and the single active payload above this;
/// struct-update fills the rest. Indent governs the leading spaces.
fn render_zero_fill_siblings(out: &mut String, indent: &str) {
    writeln!(out, "{indent}..ZERO_STREAM_EVENT").unwrap();
}

/// Helper: render the four backing-storage slot assignments on the
/// `FfiBufferedEvent`. The caller passes the indent string so the same
/// helper works for the data-arm body (16 spaces) and the control-arm
/// body (16 spaces — control arms are nested one extra level inside
/// `match ctrl { ... }` but the buffered-event braces sit at the same
/// depth as the data arms because the `FfiBufferedEvent` struct lives
/// inside the variant arm regardless).
fn render_zero_buffered_storage(
    out: &mut String,
    indent: &str,
    contract_symbol: &str,
    login_permissions: &str,
    control_message: &str,
    payload_bytes: &str,
) {
    writeln!(out, "{indent}_contract_symbol: {contract_symbol},").unwrap();
    writeln!(out, "{indent}_login_permissions: {login_permissions},").unwrap();
    writeln!(out, "{indent}_control_message: {control_message},").unwrap();
    writeln!(out, "{indent}_payload_bytes: {payload_bytes},").unwrap();
}

/// Helper invoked from both the `_ => UnknownControl` wildcard inside the
/// Control arm and the outer `_ => ` fallback for non-exhaustive
/// `StreamEvent`. Surfaces a payload-less `UnknownControl` event so
/// downstream consumers see every variant the SDK does not yet
/// recognise without losing the kind discriminator.
fn render_unknown_control_helper(out: &mut String) {
    out.push_str("    fn unknown_control_event() -> FfiBufferedEvent {\n");
    out.push_str("        FfiBufferedEvent {\n");
    out.push_str("            event: ThetaDataDxStreamEvent {\n");
    out.push_str("                kind: ThetaDataDxStreamEventKind::UnknownControl,\n");
    out.push_str("                unknown_control: ZERO_UNKNOWN_CONTROL,\n");
    render_zero_fill_siblings(out, "                ");
    out.push_str("            },\n");
    render_zero_buffered_storage(out, "            ", "None", "None", "None", "None");
    out.push_str("        }\n");
    out.push_str("    }\n\n");
}

/// Emit `fpss_event_to_ffi`, which converts a `thetadatadx::fpss::StreamEvent`
/// into an `FfiBufferedEvent` ready to box + cast across the FFI boundary.
///
/// `include!`'d from `thetadatadx-ffi/src/lib.rs` AFTER `FfiBufferedEvent` is defined
/// so it can name the backing-memory wrapper.
pub(super) fn render_ffi_fpss_event_converter(schema: &Schema) -> String {
    let mut out = String::new();
    out.push_str(
        "// @generated DO NOT EDIT — regenerated by build.rs from fpss_event_schema.toml\n",
    );
    out.push_str("// FPSS event → FFI buffered-event converter. `include!`'d from\n");
    out.push_str("// `thetadatadx-ffi/src/lib.rs` after `FfiBufferedEvent` is in scope.\n\n");

    out.push_str("pub(crate) fn fpss_event_to_ffi(event: &thetadatadx::fpss::StreamEvent) -> FfiBufferedEvent {\n");
    out.push_str("    use thetadatadx::fpss::{StreamControl, StreamData, StreamEvent};\n\n");

    render_unknown_control_helper(&mut out);

    out.push_str("    match event {\n");

    for (event_name, def) in sorted_data_events(schema) {
        render_data_arm(&mut out, event_name, def);
    }

    render_control_arms(&mut out, schema);

    // StreamEvent itself is `#[non_exhaustive]`.
    out.push_str("        _ => unknown_control_event(),\n");

    out.push_str("    }\n");
    out.push_str("}\n");
    out
}
