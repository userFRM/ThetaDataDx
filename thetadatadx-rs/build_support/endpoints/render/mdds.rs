//! Rust emitters for the MDDS gRPC `HistoricalClient` surface.
//!
//! Emits per-endpoint `list_endpoint!`, `parsed_endpoint!`, and streaming
//! builder macro invocations into the `OUT_DIR`. Shared naming/type helpers
//! live in [`super::super::helpers`]; this file only owns the code shape.
//!
//! The emitted artifacts are consumed at `include!` time by
//! `thetadatadx-rs/src/mdds/endpoints.rs`.

use std::fmt::Write as _;

use super::super::build_helpers::{
    direct_date_arg_name, direct_method_arg_name, direct_optional_kind_and_default,
    direct_optional_rust_type, direct_optional_setter_arg_type, direct_optional_setter_assign_expr,
    direct_required_field_type, direct_required_kind, direct_required_param_type,
    direct_required_store_expr, direct_return_type, direct_stream_tick_type,
};
use super::super::helpers::{
    compose_endpoint_doc, direct_parser_name, is_method_call_param, to_pascal_case,
};
use super::super::model::{GeneratedEndpoint, ProtoField};

/// Emits a `list_endpoint!` macro invocation for a flat-list endpoint,
/// rendering its doc, signature, gRPC stub, request, and query fields.
pub(super) fn generate_mdds_list_endpoint(out: &mut String, endpoint: &GeneratedEndpoint) {
    writeln!(out, "list_endpoint! {{").unwrap();
    writeln!(out, "    #[doc = {:?}]", compose_endpoint_doc(endpoint)).unwrap();
    writeln!(
        out,
        "    #[doc = {:?}]",
        format!("gRPC stub: `{}`", endpoint.grpc_name)
    )
    .unwrap();

    let method_params = endpoint
        .params
        .iter()
        .filter(|param| is_method_call_param(param))
        .collect::<Vec<_>>();
    let signature = method_params
        .iter()
        .map(|param| format!("{}: &str", direct_method_arg_name(param)))
        .collect::<Vec<_>>()
        .join(", ");
    let list_column = endpoint
        .list_column
        .as_deref()
        .expect("list endpoint must declare list_column");
    // Emit the base method name plus the `<name>_with_deadline` overload
    // name. `macro_rules!` cannot concatenate identifiers, so the explicit
    // overload identifier is passed as a token here, keeping the macro a
    // pure template. This mirrors the builder endpoints, which expose the
    // same per-call deadline contract.
    let with_deadline_fn = format!("{}_with_deadline", endpoint.name);
    if signature.is_empty() {
        writeln!(out, "    fn {}() -> {list_column:?};", endpoint.name).unwrap();
    } else {
        writeln!(
            out,
            "    fn {}({signature}) -> {list_column:?};",
            endpoint.name
        )
        .unwrap();
    }
    writeln!(out, "    with_deadline_fn: {with_deadline_fn};").unwrap();

    writeln!(out, "    grpc: {};", endpoint.grpc_name).unwrap();
    writeln!(out, "    request: {};", endpoint.request_type).unwrap();
    if endpoint.fields.is_empty() {
        writeln!(out, "    query: {} {{}};", endpoint.query_type).unwrap();
    } else {
        writeln!(out, "    query: {} {{", endpoint.query_type).unwrap();
        for field in &endpoint.fields {
            let expr = mdds_query_field_expr(endpoint, field, true);
            writeln!(out, "        {}: {expr},", field.name).unwrap();
        }
        out.push_str("    };\n");
    }
    out.push_str("}\n\n");
}

/// Single source of truth for the `.await` vs `.stream(handler)`
/// guidance attached to every parsed historical builder. Composed here
/// so every `option_history_*` / `stock_history_*` / `index_history_*`
/// / `interest_rate_history_*` builder advertises the same matrix in
/// rustdoc.
const AWAIT_VS_STREAM_DOC: &str = "\n\
# When to use `.await` vs `.stream(handler)`\n\
\n\
| Workload | Use |\n\
|---|---|\n\
| Single day / one-shot ad-hoc query | `.await` |\n\
| Single day, deterministic small response | `.await` |\n\
| Bulk / multi-day backfill | `.stream(handler)` |\n\
| Tick-interval responses | `.stream(handler)` |\n\
| Greeks responses across a long horizon | `.stream(handler)` |\n\
\n\
Buffered (`.await`) collects the full response into `Vec<Tick>`. On a\n\
2.4 M-tick day this consumes ~5 GiB before any caller code runs.\n\
Streaming yields chunks via `handler(&[Tick])`, capping per-request\n\
RSS at ~150 MiB regardless of response size.\n\
\n\
When the buffered path returns a response whose estimated size\n\
exceeds [`crate::config::HistoricalConfig::warn_on_buffered_threshold_bytes`]\n\
(default 100 MiB), a single `tracing::warn!` event fires with\n\
`endpoint`, `row_count`, and `bytes_est` fields.\n\
\n\
";

/// Emits a `parsed_endpoint!` macro invocation for a typed historical
/// endpoint, rendering its builder, signature, query fields, parser, per-tick
/// stream item type, and optional setters.
pub(super) fn generate_mdds_parsed_endpoint(out: &mut String, endpoint: &GeneratedEndpoint) {
    writeln!(out, "parsed_endpoint! {{").unwrap();
    writeln!(out, "    #[doc = {:?}]", compose_endpoint_doc(endpoint)).unwrap();
    writeln!(
        out,
        "    #[doc = {:?}]",
        format!("gRPC stub: `{}`", endpoint.grpc_name)
    )
    .unwrap();
    // Surface the `.await` vs `.stream(handler)` decision matrix in
    // rustdoc on every historical builder. Same copy on every
    // endpoint so `cargo doc` readers do not have to hunt for it —
    // placed AFTER the per-endpoint description so the endpoint's
    // own prose stays at the top of the rustdoc panel.
    writeln!(out, "    #[doc = {:?}]", AWAIT_VS_STREAM_DOC).unwrap();
    writeln!(
        out,
        "    builder {}Builder;",
        to_pascal_case(&endpoint.name)
    )
    .unwrap();

    let method_params = endpoint
        .params
        .iter()
        .filter(|param| is_method_call_param(param))
        .collect::<Vec<_>>();
    let signature = method_params
        .iter()
        .map(|param| {
            format!(
                "{}: {}",
                direct_method_arg_name(param),
                direct_required_kind(param)
            )
        })
        .collect::<Vec<_>>()
        .join(", ");
    writeln!(
        out,
        "    fn {}({signature}) -> {};",
        endpoint.name,
        direct_return_type(&endpoint.return_type)
    )
    .unwrap();

    writeln!(out, "    grpc: {};", endpoint.grpc_name).unwrap();
    writeln!(out, "    request: {};", endpoint.request_type).unwrap();
    if endpoint.fields.is_empty() {
        writeln!(out, "    query: {} {{}};", endpoint.query_type).unwrap();
    } else {
        writeln!(out, "    query: {} {{", endpoint.query_type).unwrap();
        for field in &endpoint.fields {
            let expr = mdds_query_field_expr(endpoint, field, false);
            writeln!(out, "        {}: {expr},", field.name).unwrap();
        }
        out.push_str("    };\n");
    }
    writeln!(
        out,
        "    parse: {};",
        direct_parser_name(&endpoint.return_type)
    )
    .unwrap();
    // Per-tick item type for the `.stream(handler)` method emitted
    // by `parsed_endpoint!`. The streaming variant lets callers drain
    // row-by-row instead of materializing the full `Vec<T>`,
    // eliminating the 6× memory amplification on tick-interval
    // responses (h2 frames + concatenated proto + decoded Vec +
    // Vec::push doubling).
    writeln!(
        out,
        "    item: {};",
        direct_stream_tick_type(&endpoint.return_type)
    )
    .unwrap();

    let date_args = method_params
        .iter()
        .filter_map(|param| direct_date_arg_name(param))
        .collect::<Vec<_>>();
    if !date_args.is_empty() {
        writeln!(out, "    dates: {};", date_args.join(", ")).unwrap();
    }

    let optional_params = endpoint
        .params
        .iter()
        .filter(|param| !is_method_call_param(param))
        .collect::<Vec<_>>();
    if optional_params.is_empty() {
        out.push_str("    optional {}\n");
    } else {
        out.push_str("    optional {\n");
        for param in optional_params {
            let (kind, default) = direct_optional_kind_and_default(param);
            writeln!(out, "        {}: {} = {},", param.name, kind, default).unwrap();
        }
        out.push_str("    }\n");
    }
    out.push_str("}\n\n");
}

/// Emits the hand-shaped streaming builder for a subscription endpoint: the
/// builder struct, its optional setters, the deadline-and-retry stream method,
/// and the `HistoricalClient` constructor that returns it.
pub(super) fn generate_mdds_streaming_endpoint(out: &mut String, endpoint: &GeneratedEndpoint) {
    let method_params = endpoint
        .params
        .iter()
        .filter(|param| is_method_call_param(param))
        .collect::<Vec<_>>();
    let optional_params = endpoint
        .params
        .iter()
        .filter(|param| !is_method_call_param(param))
        .collect::<Vec<_>>();
    let builder_name = format!("{}Builder", to_pascal_case(&endpoint.name));
    let tick_type = direct_stream_tick_type(&endpoint.return_type);
    let parser_name = direct_parser_name(&endpoint.return_type);

    writeln!(
        out,
        "/// Builder for the [`HistoricalClient::{}`] streaming endpoint.",
        endpoint.name
    )
    .unwrap();
    writeln!(out, "pub struct {builder_name}<'a> {{").unwrap();
    out.push_str("    client: &'a HistoricalClient,\n");
    for param in &method_params {
        writeln!(
            out,
            "    pub(crate) {}: {},",
            direct_method_arg_name(param),
            direct_required_field_type(param)
        )
        .unwrap();
    }
    for param in &optional_params {
        writeln!(
            out,
            "    pub(crate) {}: {},",
            param.name,
            direct_optional_rust_type(param)
        )
        .unwrap();
    }
    out.push_str("    pub(crate) deadline: Option<std::time::Duration>,\n");
    out.push_str("}\n\n");

    writeln!(out, "impl<'a> {builder_name}<'a> {{").unwrap();
    for param in &optional_params {
        writeln!(out, "    #[must_use]").unwrap();
        writeln!(
            out,
            "    pub fn {}(mut self, v: {}) -> Self {{",
            param.name,
            direct_optional_setter_arg_type(param)
        )
        .unwrap();
        writeln!(
            out,
            "        self.{0} = {1};",
            param.name,
            direct_optional_setter_assign_expr(param)
        )
        .unwrap();
        out.push_str("        self\n");
        out.push_str("    }\n");
    }
    out.push_str(
        &include_str!("templates/mdds/stream_method_header.rs.tmpl")
            .replace("__TICK_TYPE__", tick_type),
    );
    writeln!(out, "        let {builder_name} {{").unwrap();
    out.push_str("            client,\n");
    for param in &method_params {
        writeln!(out, "            {},", direct_method_arg_name(param)).unwrap();
    }
    for param in &optional_params {
        writeln!(out, "            {},", param.name).unwrap();
    }
    out.push_str("            deadline,\n");
    out.push_str("        } = self;\n");
    out.push_str("        let _ = &client;\n");
    for arg in method_params
        .iter()
        .filter_map(|param| direct_date_arg_name(param))
    {
        writeln!(out, "        validate_date_required(&{arg})?;").unwrap();
    }
    let endpoint_name_literal = format!("{:?}", endpoint.name);
    // Resolve the effective deadline once, exactly like the
    // `parsed_endpoint!` stream arm: an unset deadline falls back to the
    // configured `request_timeout_secs` so a silent-but-live server
    // cannot hang the request (and starve the request-semaphore) forever;
    // an explicit `Duration::ZERO` is the opt-out. The deadline then wraps
    // the whole retry loop so the caller's budget covers every attempt.
    out.push_str("        let deadline = crate::mdds::macros::effective_deadline(\n");
    out.push_str("            deadline,\n");
    out.push_str("            client.config().historical.request_timeout_secs,\n");
    out.push_str("        );\n");
    writeln!(
        out,
        "        crate::mdds::macros::run_with_optional_deadline(deadline, async move {{"
    )
    .unwrap();
    writeln!(
        out,
        "            tracing::debug!(endpoint = {endpoint_name_literal}, \"gRPC streaming request\");"
    )
    .unwrap();
    writeln!(
        out,
        "            metrics::counter!(\"thetadatadx.grpc.requests\", \"endpoint\" => {endpoint_name_literal}).increment(1);"
    )
    .unwrap();
    out.push_str("            let _metrics_start = std::time::Instant::now();\n");
    out.push_str("            let _permit = client.request_semaphore.acquire().await\n");
    out.push_str(
        "                .map_err(|_| Error::config_internal(\"request semaphore closed\"))?;\n",
    );
    out.push_str("            let policy = client.config().retry;\n");
    // Wrap the `FnMut + Send` handler in a `Mutex` so the per-attempt
    // closure passed to `run_streaming_retry_loop` gets a fresh mutable
    // borrow on each invocation (the closure may run twice on a
    // post-refresh restart) and the returned future stays Send.
    out.push_str("            let handler_mutex = std::sync::Mutex::new(handler);\n");
    out.push_str("            let handler_mutex = &handler_mutex;\n");
    // Set once a chunk reaches `handler`: the no-resume restart replays
    // from chunk zero, so a later transient after delivery began is made
    // terminal by `run_streaming_retry_loop` to avoid duplicating rows.
    out.push_str("            let delivered = std::sync::atomic::AtomicBool::new(false);\n");
    out.push_str("            let delivered = &delivered;\n");
    out.push_str("            crate::mdds::macros::run_streaming_retry_loop(\n");
    out.push_str("                client.session(),\n");
    out.push_str("                &policy,\n");
    writeln!(out, "                {endpoint_name_literal},").unwrap();
    out.push_str("                delivered,\n");
    out.push_str("                move |snap| {\n");
    // Clone per-attempt: the FnMut closure may fire twice (post-refresh
    // restart) and the `async move` block would otherwise move each
    // captured binding into the first attempt's future, so non-Copy
    // params clone fresh each iteration. Copy scalars (`Option<i32>` etc.)
    // are copied into the future automatically and need no rebind.
    for arg in method_params
        .iter()
        .map(|param| direct_method_arg_name(param))
    {
        writeln!(out, "                    let {arg} = {arg}.clone();").unwrap();
    }
    for param in &optional_params {
        if matches!(
            direct_optional_rust_type(param),
            "Option<i32>" | "Option<f64>" | "Option<bool>"
        ) {
            continue;
        }
        writeln!(
            out,
            "                    let {0} = {0}.clone();",
            param.name
        )
        .unwrap();
    }
    out.push_str("                    async move {\n");
    out.push_str("                        let qi = client.build_query_info(snap.uuid.clone());\n");
    writeln!(
        out,
        "                        let request = proto::{} {{",
        endpoint.request_type
    )
    .unwrap();
    out.push_str("                            query_info: Some(qi),\n");
    if endpoint.fields.is_empty() {
        writeln!(
            out,
            "                            params: Some(proto::{} {{}}),",
            endpoint.query_type
        )
        .unwrap();
    } else {
        writeln!(
            out,
            "                            params: Some(proto::{} {{",
            endpoint.query_type
        )
        .unwrap();
        for field in &endpoint.fields {
            let expr = mdds_query_field_expr(endpoint, field, false);
            if expr == field.name {
                writeln!(out, "                                {expr},").unwrap();
            } else {
                writeln!(
                    out,
                    "                                {}: {expr},",
                    field.name
                )
                .unwrap();
            }
        }
        out.push_str("                            }),\n");
    }
    out.push_str("                        };\n");
    out.push_str(
        &include_str!("templates/mdds/stub_call_error_arm.rs.tmpl")
            .replace("__GRPC_NAME__", &endpoint.grpc_name)
            .replace("__ENDPOINT_NAME_LITERAL__", &endpoint_name_literal),
    );
    out.push_str(
        &include_str!("templates/mdds/for_each_chunk_body.rs.tmpl")
            .replace("__PARSER_NAME__", &parser_name),
    );
    out.push_str("                    }\n");
    out.push_str("                },\n");
    out.push_str("            ).await?;\n");
    writeln!(
        out,
        "            metrics::histogram!(\"thetadatadx.grpc.latency_ms\", \"endpoint\" => {endpoint_name_literal})"
    )
    .unwrap();
    out.push_str("                .record(_metrics_start.elapsed().as_secs_f64() * 1_000.0);\n");
    out.push_str("            Ok::<(), Error>(())\n");
    out.push_str("        }).await\n");
    out.push_str(include_str!("templates/mdds/metrics_result_block.rs.tmpl"));

    writeln!(out, "impl HistoricalClient {{").unwrap();
    writeln!(
        out,
        "    /// Open a `{builder_name}` for the `{}` streaming endpoint.",
        endpoint.name
    )
    .unwrap();
    writeln!(out, "    ///").unwrap();
    writeln!(
        out,
        "    /// Set any optional filters on the returned builder, then drain the"
    )
    .unwrap();
    writeln!(
        out,
        "    /// response chunk-by-chunk with the builder's `stream` method."
    )
    .unwrap();
    write!(out, "    pub fn {}(&self", endpoint.name).unwrap();
    for param in &method_params {
        write!(
            out,
            ", {}: {}",
            direct_method_arg_name(param),
            direct_required_param_type(param)
        )
        .unwrap();
    }
    writeln!(out, ") -> {builder_name}<'_> {{").unwrap();
    writeln!(out, "        {builder_name} {{").unwrap();
    out.push_str("            client: self,\n");
    for param in &method_params {
        writeln!(
            out,
            "            {}: {},",
            direct_method_arg_name(param),
            direct_required_store_expr(param)
        )
        .unwrap();
    }
    for param in &optional_params {
        let (_, default) = direct_optional_kind_and_default(param);
        writeln!(out, "            {}: {},", param.name, default).unwrap();
    }
    out.push_str("            deadline: None,\n");
    out.push_str("        }\n");
    out.push_str("    }\n");
    out.push_str("}\n\n");
}

/// Returns the Rust expression that populates one proto query field from the
/// endpoint's arguments, applying the per-field normalization and contract-spec
/// handling the wire contract requires.
pub(super) fn mdds_query_field_expr(
    endpoint: &GeneratedEndpoint,
    field: &ProtoField,
    list_context: bool,
) -> String {
    if field.proto_type == "ContractSpec" {
        if list_context {
            let has_strike_method = endpoint
                .params
                .iter()
                .any(|p| p.name == "strike" && is_method_call_param(p));
            let has_right_method = endpoint
                .params
                .iter()
                .any(|p| p.name == "right" && is_method_call_param(p));
            let strike = if has_strike_method { "strike" } else { "\"*\"" };
            let right = if has_right_method {
                "right"
            } else {
                "\"both\""
            };
            return format!("contract_spec!(symbol, expiration, {strike}, {right})");
        }
        return "contract_spec!(symbol, expiration, strike, right)".into();
    }
    if field.name == "date" && endpoint.name == "stock_history_ohlc_range" {
        return "None".into();
    }

    let param = endpoint
        .params
        .iter()
        .find(|param| param.name == field.name)
        .unwrap_or_else(|| {
            panic!(
                "missing param metadata for {}::{}",
                endpoint.name, field.name
            )
        });
    let arg_name = direct_method_arg_name(param);
    let is_method_param = is_method_call_param(param);

    match field.name.as_str() {
        "symbol" if field.is_repeated => {
            if param.param_type == "Symbols" {
                "symbols.clone()".into()
            } else if !is_method_param && param.default.is_none() {
                // Optional builder symbol stored as `Option<String>`: an
                // unset filter sends an empty repeated field (proto3 omits
                // it), which the server reads as "list the full universe".
                // A set filter sends the single supplied symbol.
                format!("{arg_name}.iter().cloned().collect()")
            } else if list_context {
                format!("vec![{arg_name}.to_string()]")
            } else {
                format!("vec![{arg_name}.clone()]")
            }
        }
        "interval" => format!("normalize_interval(&{arg_name})"),
        "time_of_day" => format!("normalize_time_of_day(&{arg_name})"),
        // Date-semantic wire fields. The validators accept both
        // `YYYYMMDD` and `YYYY-MM-DD`; the wire contract is the compact
        // form only, so every date flows through `normalize_date` at
        // request construction — the same choke point `interval` and
        // `expiration` already normalize through. Covers required
        // method args, optional builder setters, and SSOT-defaulted
        // builder fields uniformly.
        "date" | "start_date" | "end_date" => {
            let value_expr = if list_context {
                format!("normalize_date({arg_name})")
            } else {
                format!("normalize_date(&{arg_name})")
            };
            if !is_method_param && param.default.is_none() {
                // Optional builder field without an SSOT default:
                // stored as `Option<String>`; normalize through the
                // `Option` without forcing a populated wire value.
                format!("{arg_name}.as_deref().map(normalize_date)")
            } else if field.is_optional {
                format!("Some({value_expr})")
            } else {
                value_expr
            }
        }
        // Top-level `expiration` field on option query messages.
        //
        // Many option query protos carry BOTH a `ContractSpec` (whose
        // `expiration` is the contract identity) AND a top-level
        // `expiration` string (a vestigial wire field that predates
        // `ContractSpec`). The vendor's v3 client never populates the
        // top-level field when `contract_spec` is present — the server
        // uses the `ContractSpec` copy for identity and treats a
        // populated top-level `expiration` as a narrow filter that
        // forces per-contract enumeration. Mirror vendor's shape: on
        // messages that also carry `contract_spec`, emit an empty
        // string for the top-level `expiration` field. On messages
        // that only carry the top-level field (e.g. `option_list_strikes`,
        // which has no `ContractSpec`), canonicalize the user-supplied
        // value the way we always did.
        "expiration" => {
            let has_contract_spec = endpoint
                .fields
                .iter()
                .any(|f| f.proto_type == "ContractSpec");
            if has_contract_spec {
                "String::new()".into()
            } else if list_context {
                format!("normalize_expiration({arg_name})")
            } else {
                format!("normalize_expiration(&{arg_name})")
            }
        }
        "start_time" | "end_time" => format!("Some({arg_name}.clone())"),
        _ if field.proto_type == "string" => {
            if field.is_optional {
                if is_method_param {
                    format!("Some({arg_name}.clone())")
                } else if param.default.is_some() {
                    // Builder field carries an SSOT-supplied default, so it is
                    // stored as bare `String` (never `None`). The proto field
                    // stays `Option<String>`, so wrap on the way in.
                    format!("Some({arg_name}.clone())")
                } else {
                    format!("{arg_name}.clone()")
                }
            } else if list_context {
                format!("{arg_name}.to_string()")
            } else {
                format!("{arg_name}.clone()")
            }
        }
        _ => arg_name,
    }
}
