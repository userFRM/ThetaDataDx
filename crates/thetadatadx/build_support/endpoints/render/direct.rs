//! Rust emitters for the `DirectClient` surface.
//!
//! Emits per-endpoint `list_endpoint!`, `parsed_endpoint!`, and streaming
//! builder macro invocations into the `OUT_DIR`. Shared naming/type helpers
//! live in [`super::super::helpers`]; this file only owns the code shape.

use std::fmt::Write as _;

use super::super::helpers::{
    direct_date_arg_name, direct_method_arg_name, direct_optional_kind_and_default,
    direct_optional_rust_type, direct_optional_setter_arg_type, direct_optional_setter_assign_expr,
    direct_parser_name, direct_required_field_type, direct_required_kind,
    direct_required_param_type, direct_required_store_expr, direct_return_type,
    direct_stream_tick_type, is_method_call_param, to_pascal_case,
};
use super::super::model::{GeneratedEndpoint, ProtoField};

pub(super) fn generate_direct_list_endpoint(out: &mut String, endpoint: &GeneratedEndpoint) {
    writeln!(out, "list_endpoint! {{").unwrap();
    writeln!(out, "    #[doc = {:?}]", endpoint.description).unwrap();
    writeln!(
        out,
        "    #[doc = {:?}]",
        format!("gRPC stub: `{}`", endpoint.grpc_name)
    )
    .unwrap();

    let signature = endpoint
        .params
        .iter()
        .map(|param| format!("{}: &str", direct_method_arg_name(endpoint, param)))
        .collect::<Vec<_>>()
        .join(", ");
    if signature.is_empty() {
        writeln!(
            out,
            "    fn {}() -> {:?};",
            endpoint.name,
            endpoint
                .list_column
                .as_deref()
                .expect("list endpoint must declare list_column")
        )
        .unwrap();
    } else {
        writeln!(
            out,
            "    fn {}({signature}) -> {:?};",
            endpoint.name,
            endpoint
                .list_column
                .as_deref()
                .expect("list endpoint must declare list_column")
        )
        .unwrap();
    }

    writeln!(out, "    grpc: {};", endpoint.grpc_name).unwrap();
    writeln!(out, "    request: {};", endpoint.request_type).unwrap();
    if endpoint.fields.is_empty() {
        writeln!(out, "    query: {} {{}};", endpoint.query_type).unwrap();
    } else {
        writeln!(out, "    query: {} {{", endpoint.query_type).unwrap();
        for field in &endpoint.fields {
            let expr = direct_query_field_expr(endpoint, field, true);
            writeln!(out, "        {}: {expr},", field.name).unwrap();
        }
        out.push_str("    };\n");
    }
    out.push_str("}\n\n");
}

pub(super) fn generate_direct_parsed_endpoint(out: &mut String, endpoint: &GeneratedEndpoint) {
    writeln!(out, "parsed_endpoint! {{").unwrap();
    writeln!(out, "    #[doc = {:?}]", endpoint.description).unwrap();
    writeln!(
        out,
        "    #[doc = {:?}]",
        format!("gRPC stub: `{}`", endpoint.grpc_name)
    )
    .unwrap();
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
                direct_method_arg_name(endpoint, param),
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
            let expr = direct_query_field_expr(endpoint, field, false);
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

    let date_args = method_params
        .iter()
        .filter_map(|param| direct_date_arg_name(endpoint, param))
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

pub(super) fn generate_direct_streaming_endpoint(out: &mut String, endpoint: &GeneratedEndpoint) {
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
        "/// Builder for the [`DirectClient::{}`] streaming endpoint.",
        endpoint.name
    )
    .unwrap();
    writeln!(out, "pub struct {builder_name}<'a> {{").unwrap();
    out.push_str("    client: &'a DirectClient,\n");
    for param in &method_params {
        writeln!(
            out,
            "    pub(crate) {}: {},",
            direct_method_arg_name(endpoint, param),
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
    out.push_str("\n    /// Execute the streaming request, calling `handler` for each chunk.\n");
    out.push_str("    ///\n");
    out.push_str("    /// # Errors\n");
    out.push_str("    ///\n");
    out.push_str("    /// Returns [`Error`] if the gRPC call fails or response parsing fails.\n");
    out.push_str("    pub async fn stream<F>(self, mut handler: F) -> Result<(), Error>\n");
    out.push_str("    where\n");
    writeln!(out, "        F: FnMut(&[{tick_type}]),").unwrap();
    out.push_str("    {\n");
    writeln!(out, "        let {builder_name} {{").unwrap();
    out.push_str("            client,\n");
    for param in &method_params {
        writeln!(
            out,
            "            {},",
            direct_method_arg_name(endpoint, param)
        )
        .unwrap();
    }
    for param in &optional_params {
        writeln!(out, "            {},", param.name).unwrap();
    }
    out.push_str("        } = self;\n");
    out.push_str("        let _ = &client;\n");
    for arg in method_params
        .iter()
        .filter_map(|param| direct_date_arg_name(endpoint, param))
    {
        writeln!(out, "        validate_date(&{arg})?;").unwrap();
    }
    writeln!(
        out,
        "        tracing::debug!(endpoint = {:?}, \"gRPC streaming request\");",
        endpoint.name
    )
    .unwrap();
    writeln!(
        out,
        "        metrics::counter!(\"thetadatadx.grpc.requests\", \"endpoint\" => {:?}).increment(1);",
        endpoint.name
    )
    .unwrap();
    out.push_str("        let _metrics_start = std::time::Instant::now();\n");
    out.push_str("        let _permit = client.request_semaphore.acquire().await\n");
    out.push_str(
        "            .map_err(|_| Error::Config(\"request semaphore closed\".into()))?;\n",
    );
    writeln!(
        out,
        "        let request = proto::{} {{",
        endpoint.request_type
    )
    .unwrap();
    out.push_str("            query_info: Some(client.query_info()),\n");
    if endpoint.fields.is_empty() {
        writeln!(
            out,
            "            params: Some(proto::{} {{}}),",
            endpoint.query_type
        )
        .unwrap();
    } else {
        writeln!(
            out,
            "            params: Some(proto::{} {{",
            endpoint.query_type
        )
        .unwrap();
        for field in &endpoint.fields {
            let expr = direct_query_field_expr(endpoint, field, false);
            if expr == field.name {
                writeln!(out, "                {expr},").unwrap();
            } else {
                writeln!(out, "                {}: {expr},", field.name).unwrap();
            }
        }
        out.push_str("            }),\n");
    }
    out.push_str("        };\n");
    writeln!(
        out,
        "        let stream = match client.stub().{}(request).await {{",
        endpoint.grpc_name
    )
    .unwrap();
    out.push_str("            Ok(resp) => resp.into_inner(),\n");
    out.push_str("            Err(e) => {\n");
    writeln!(
        out,
        "                metrics::counter!(\"thetadatadx.grpc.errors\", \"endpoint\" => {:?}).increment(1);",
        endpoint.name
    )
    .unwrap();
    out.push_str("                return Err(e.into());\n");
    out.push_str("            }\n");
    out.push_str("        };\n");
    out.push_str("        let result = client.for_each_chunk(stream, |_headers, rows| {\n");
    out.push_str("            let table = proto::DataTable {\n");
    out.push_str("                headers: _headers.to_vec(),\n");
    out.push_str("                data_table: rows.to_vec(),\n");
    out.push_str("            };\n");
    writeln!(out, "            let ticks = {parser_name}(&table);").unwrap();
    out.push_str("            handler(&ticks);\n");
    out.push_str("        }).await;\n");
    out.push_str("        match &result {\n");
    out.push_str("            Ok(()) => {\n");
    writeln!(
        out,
        "                metrics::histogram!(\"thetadatadx.grpc.latency_ms\", \"endpoint\" => {:?})",
        endpoint.name
    )
    .unwrap();
    out.push_str(
        "                    .record(_metrics_start.elapsed().as_secs_f64() * 1_000.0);\n",
    );
    out.push_str("            }\n");
    out.push_str("            Err(_) => {\n");
    writeln!(
        out,
        "                metrics::counter!(\"thetadatadx.grpc.errors\", \"endpoint\" => {:?}).increment(1);",
        endpoint.name
    )
    .unwrap();
    out.push_str("            }\n");
    out.push_str("        }\n");
    out.push_str("        result\n");
    out.push_str("    }\n");
    out.push_str("}\n\n");

    writeln!(out, "impl DirectClient {{").unwrap();
    write!(out, "    pub fn {}(&self", endpoint.name).unwrap();
    for param in &method_params {
        write!(
            out,
            ", {}: {}",
            direct_method_arg_name(endpoint, param),
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
            direct_method_arg_name(endpoint, param),
            direct_required_store_expr(endpoint, param)
        )
        .unwrap();
    }
    for param in &optional_params {
        let (_, default) = direct_optional_kind_and_default(param);
        writeln!(out, "            {}: {},", param.name, default).unwrap();
    }
    out.push_str("        }\n");
    out.push_str("    }\n");
    out.push_str("}\n\n");
}

pub(super) fn direct_query_field_expr(
    endpoint: &GeneratedEndpoint,
    field: &ProtoField,
    list_context: bool,
) -> String {
    if field.proto_type == "ContractSpec" {
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
    let arg_name = direct_method_arg_name(endpoint, param);
    let is_method_param = is_method_call_param(param);

    match field.name.as_str() {
        "symbol" if field.is_repeated => {
            if param.param_type == "Symbols" {
                "symbols.clone()".into()
            } else if list_context {
                format!("vec![{arg_name}.to_string()]")
            } else {
                format!("vec![{arg_name}.clone()]")
            }
        }
        "interval" => format!("normalize_interval(&{arg_name})"),
        "time_of_day" => format!("normalize_time_of_day(&{arg_name})"),
        // Top-level `expiration` fields on query messages get the same
        // wire canonicalization as the ContractSpec copy: `0` -> `*`, ISO
        // dashes stripped. Keeps the two expiration values on the request
        // in agreement and prevents the server from seeing a raw `0` on
        // either path. In list-endpoint context the arg is already `&str`
        // (no extra borrow); in the parsed-endpoint context it's owned.
        "expiration" => {
            if list_context {
                format!("normalize_expiration({arg_name})")
            } else {
                format!("normalize_expiration(&{arg_name})")
            }
        }
        "start_time" | "end_time" => format!("Some({arg_name}.clone())"),
        "venue" if endpoint.category == "stock" => {
            "venue.clone().or_else(|| Some(crate::wire_semantics::DEFAULT_STOCK_VENUE.to_string()))"
                .into()
        }
        _ if field.proto_type == "string" => {
            if field.is_optional {
                if is_method_param {
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
