//! TypeScript (napi-rs) streaming methods for `sdks/typescript/src/_generated/streaming_methods.rs`
//! and offline utility free functions for `sdks/typescript/src/_generated/utility_functions.rs`.

use std::fmt::Write as _;

use super::common::{generated_header, greek_result_fields, push_rust_doc_comment, ts_field_ident};
use super::spec::{MethodKind, MethodSpec, UtilityKind, UtilitySpec};

/// Renders the TypeScript streaming methods source: the `#[napi]` block on `StreamView`.
pub(super) fn render_ts_streaming_methods(methods: &[&MethodSpec]) -> String {
    let mut out = String::new();
    out.push_str(generated_header());
    out.push_str("#[napi]\n");
    out.push_str("impl StreamView {\n");
    for method in methods {
        out.push_str(&ts_streaming_method(method));
        out.push('\n');
    }
    out.push_str("}\n");
    out
}

fn ts_streaming_method(method: &MethodSpec) -> String {
    let mut out = String::new();
    // The doc string in `sdk_surface.toml` is the cross-language
    // semantic summary. Two TS kinds (StartStreaming, Reconnect)
    // require a callback-specific docstring; render those inline below
    // instead of re-using the shared one.
    let render_shared_doc = !matches!(
        method.kind,
        MethodKind::StartStreaming | MethodKind::Reconnect
    );
    if render_shared_doc {
        push_rust_doc_comment(&mut out, "    ", &method.doc);
    }
    match method.kind {
        MethodKind::StartStreaming => {
            push_rust_doc_comment(
                &mut out,
                "    ",
                "Start FPSS streaming and register a JS callback for incoming events.\n\
                 \n\
                 Each typed FPSS event is delivered to your\n\
                 `callback(event)` on the Node main thread, so the\n\
                 callback may use any JS API safely. A callback that\n\
                 panics or throws is isolated and does not interrupt\n\
                 the stream.\n\
                 \n\
                 Backpressure: a slow callback causes incoming events\n\
                 to queue and, once the buffer is full, newly arriving\n\
                 events are dropped. The dropped count is observable\n\
                 via `droppedEventCount()`. The receive path is never\n\
                 blocked by a slow callback, so the upstream connection\n\
                 stays healthy regardless of callback speed.",
            );
            writeln!(out, "    #[napi(js_name = \"startStreaming\")]").unwrap();
            // The callback handle owns a JS function reference and
            // marshals each call onto the Node main thread. We register
            // a Rust closure with the dispatcher; the closure clones the
            // handle (a cheap reference bump) and invokes it with the
            // typed event.
            writeln!(
                out,
                "    pub fn {}(&self, callback: napi::threadsafe_function::ThreadsafeFunction<StreamEvent, (), StreamEvent, napi::Status, false>) -> napi::Result<()> {{",
                method.name
            )
            .unwrap();
            out.push_str(include_str!(
                "templates/typescript/start_streaming_body.rs.tmpl"
            ));
        }
        MethodKind::IsStreaming => {
            writeln!(out, "    #[napi(js_name = \"isStreaming\")]").unwrap();
            writeln!(out, "    pub fn {}(&self) -> bool {{", method.name).unwrap();
            out.push_str("        self.client.stream().is_streaming()\n");
            out.push_str("    }\n");
        }
        MethodKind::StockContractCall => {
            let param = &method.params[0];
            let js_name = to_ts_camel_case(&method.name);
            writeln!(out, "    #[napi(js_name = \"{js_name}\")]").unwrap();
            writeln!(
                out,
                "    pub fn {}(&self, {}: String) -> napi::Result<()> {{",
                method.name, param.name,
            )
            .unwrap();
            writeln!(
                out,
                "        let contract = fpss::protocol::Contract::stock(&{});",
                param.name
            )
            .unwrap();
            writeln!(
                out,
                "        self.client.stream().{}(&contract).map_err(to_napi_err)",
                method.runtime_call.as_deref().unwrap()
            )
            .unwrap();
            out.push_str("    }\n");
        }
        MethodKind::OptionContractCall => {
            let js_name = to_ts_camel_case(&method.name);
            writeln!(out, "    #[napi(js_name = \"{js_name}\")]").unwrap();
            writeln!(out, "    pub fn {}(", method.name).unwrap();
            out.push_str("        &self,\n");
            for param in &method.params {
                writeln!(out, "        {}: String,", param.name).unwrap();
            }
            out.push_str("    ) -> napi::Result<()> {\n");
            writeln!(
                out,
                "        let contract = fpss::protocol::Contract::option(&{}, fpss::protocol::OptionLeg {{ expiration: &{}, strike: &{}, right: &{} }}).map_err(to_napi_err)?;",
                method.params[0].name,
                method.params[1].name,
                method.params[2].name,
                method.params[3].name
            )
            .unwrap();
            writeln!(
                out,
                "        self.client.stream().{}(&contract).map_err(to_napi_err)",
                method.runtime_call.as_deref().unwrap()
            )
            .unwrap();
            out.push_str("    }\n");
        }
        MethodKind::FullCall => {
            let param = &method.params[0];
            let js_name = to_ts_camel_case(&method.name);
            writeln!(out, "    #[napi(js_name = \"{js_name}\")]").unwrap();
            writeln!(
                out,
                "    pub fn {}(&self, {}: String) -> napi::Result<()> {{",
                method.name, param.name,
            )
            .unwrap();
            writeln!(out, "        let st = parse_sec_type(&{})?;", param.name).unwrap();
            writeln!(
                out,
                "        self.client.stream().{}(st).map_err(to_napi_err)",
                method.runtime_call.as_deref().unwrap()
            )
            .unwrap();
            out.push_str("    }\n");
        }
        MethodKind::ActiveSubscriptions => {
            writeln!(out, "    #[napi(js_name = \"activeSubscriptions\")]").unwrap();
            writeln!(
                out,
                "    pub fn {}(&self) -> napi::Result<serde_json::Value> {{",
                method.name
            )
            .unwrap();
            out.push_str("        self.client\n");
            out.push_str("            .stream()\n");
            out.push_str("            .active_subscriptions()\n");
            out.push_str("            .map(|subs| {\n");
            out.push_str("                serde_json::json!(subs.into_iter()\n");
            out.push_str("                    .map(|(kind, contract)| {\n");
            out.push_str("                        serde_json::json!({ \"kind\": format!(\"{kind:?}\"), \"contract\": format!(\"{contract}\") })\n");
            out.push_str("                    })\n");
            out.push_str("                    .collect::<Vec<_>>())\n");
            out.push_str("            })\n");
            out.push_str("            .map_err(to_napi_err)\n");
            out.push_str("    }\n");
        }
        MethodKind::NextEvent => {
            // The TypeScript binding uses callback registration via
            // `startStreaming(callback)` rather than a poll-style
            // `next_event`. The TS target is no longer
            // in `MethodKind::NextEvent`'s allowed list (see
            // `spec.rs`), so this arm is unreachable on the TS
            // surface. Panicking here is the loud failure we want if
            // someone re-adds `typescript_napi` to `next_event` without
            // also implementing a poll-style napi method.
            panic!("MethodKind::NextEvent is not emitted on the TypeScript target after PR D");
        }
        MethodKind::Reconnect => {
            push_rust_doc_comment(
                &mut out,
                "    ",
                "Reconnect FPSS streaming and re-register the previously installed callback.\n\
                 \n\
                 Requires a prior `startStreaming(callback)`; throws if\n\
                 no callback is registered. All active subscriptions are\n\
                 restored on the new connection. If some subscriptions\n\
                 cannot be restored, the reconnect still completes for\n\
                 the rest and the failures are reported through the\n\
                 callback.\n\
                 \n\
                 # Callback lifetime across `stopStreaming`\n\
                 \n\
                 `stopStreaming()` and `shutdown()` clear the registered\n\
                 callback. To resume streaming on this client after\n\
                 `stopStreaming()`, you MUST call `startStreaming(callback)`\n\
                 again with a freshly bound function; `reconnect()` throws\n\
                 because no callback is held.\n\
                 \n\
                 This explicit-handoff model matches the C++ wrapper's RAII\n\
                 destructor and the Python `with` block's `__exit__`: the\n\
                 resource (the JS callback handle) is cleared at the same\n\
                 scope boundary the application observes. The unified C API\n\
                 preserves the callback across stop/reconnect, but the\n\
                 TypeScript and Python bindings deliberately diverge to enforce\n\
                 the explicit handoff and avoid retaining captured references\n\
                 past a teardown the caller has already observed.",
            );
            writeln!(out, "    #[napi(js_name = \"reconnect\")]").unwrap();
            writeln!(
                out,
                "    pub fn {}(&self) -> napi::Result<()> {{",
                method.name
            )
            .unwrap();
            out.push_str(include_str!("templates/typescript/reconnect_body.rs.tmpl"));
        }
        MethodKind::AwaitDrain => {
            let js_name = to_ts_camel_case(&method.name);
            writeln!(out, "    #[napi(js_name = \"{js_name}\")]").unwrap();
            // Wrap the polling barrier in `tokio::task::spawn_blocking` and
            // expose it as an async napi method so JS can `await` it
            // without blocking the Node main thread. napi-rs maps the
            // returned `napi::Result<bool>` on an `async fn` to a
            // `Promise<boolean>` on the JS side.
            writeln!(
                out,
                "    pub async fn {}(&self, timeout_ms: u32) -> napi::Result<bool> {{",
                method.name,
            )
            .unwrap();
            // Clone the Arc<thetadatadx::Client> so the blocking
            // closure can outlive `&self` — `spawn_blocking` requires
            // `'static`. The polling itself is cheap (1 ms sleep loop)
            // and the Arc clone is one atomic bump.
            out.push_str("        let client = self.client.clone();\n");
            out.push_str(
                "        let timeout = std::time::Duration::from_millis(u64::from(timeout_ms));\n",
            );
            out.push_str(
                "        tokio::task::spawn_blocking(move || client.stream().await_drain(timeout))\n",
            );
            out.push_str("            .await\n");
            out.push_str("            .map_err(|e| napi::Error::from_reason(format!(\"await_drain task panicked: {e}\")))\n");
            out.push_str("    }\n");
        }
        MethodKind::StopStreaming | MethodKind::Shutdown => {
            let js_name = to_ts_camel_case(&method.name);
            writeln!(out, "    #[napi(js_name = \"{js_name}\")]").unwrap();
            writeln!(out, "    pub fn {}(&self) {{", method.name).unwrap();
            // Drop the stored callback handle so its JS reference is
            // released before the streaming side tears down —
            // re-installing via `startStreaming` after stop / shutdown
            // then sees a clean slot.
            out.push_str("        self.client.stream().stop_streaming();\n");
            out.push_str(
                "        let mut guard = self.callback.lock().unwrap_or_else(|e| e.into_inner());\n",
            );
            out.push_str("        *guard = None;\n");
            out.push_str("    }\n");
        }
        other => panic!("unsupported TypeScript method kind: {other:?}"),
    }
    out
}

fn to_ts_camel_case(name: &str) -> String {
    let mut parts = name.split('_');
    let first = parts.next().unwrap_or_default();
    let mut result = first.to_string();
    for part in parts {
        if !part.is_empty() {
            let mut chars = part.chars();
            result.push(chars.next().unwrap().to_uppercase().next().unwrap());
            result.extend(chars);
        }
    }
    result
}

/// Renders the TypeScript utility functions source: the `AllGreeks` napi object and the offline utility free functions.
pub(super) fn render_ts_utility_functions(utilities: &[&UtilitySpec]) -> String {
    let mut out = String::new();
    out.push_str(generated_header());

    // Emit the typed `AllGreeks` napi object before any function that
    // returns it, mirroring the Python typed-pyclass policy. napi-rs
    // lowers the `#[napi(object)]` struct to a TypeScript interface in
    // `index.d.ts`, so `allGreeks(...)` returns a concrete object type —
    // never `any` or a loose record.
    let has_all_greeks = utilities
        .iter()
        .any(|u| matches!(u.kind, UtilityKind::AllGreeks));
    if has_all_greeks {
        out.push_str(&render_all_greeks_napi_object());
        out.push('\n');
    }

    for utility in utilities {
        out.push_str(&ts_utility_function(utility));
        out.push('\n');
    }
    out
}

/// Emit the `AllGreeks` `#[napi(object)]` struct so `allGreeks(...)`
/// returns a typed object whose fields mirror
/// `thetadatadx::greeks::GreeksResult` 1:1. napi-rs camelCases each
/// snake_case Rust field into the JS property name (`dual_delta` ->
/// `dualDelta`); the field name passes through [`ts_field_ident`]
/// unchanged because object keys admit reserved words (`lambda` stays
/// `lambda`, matching the `GreeksAllTick` tick object).
fn render_all_greeks_napi_object() -> String {
    let mut out = String::new();
    out.push_str(
        "/// All 23 Black-Scholes Greeks + IV in a single typed object.\n\
         /// Returned by `allGreeks(...)`.\n",
    );
    out.push_str("#[must_use]\n");
    out.push_str("#[napi(object)]\n");
    out.push_str("#[derive(Clone)]\n");
    out.push_str("pub struct AllGreeks {\n");
    for (field, _rust_field) in greek_result_fields() {
        writeln!(out, "    pub {}: f64,", ts_field_ident(field)).unwrap();
    }
    out.push_str("}\n");
    out
}

fn ts_utility_function(utility: &UtilitySpec) -> String {
    let mut out = String::new();
    push_rust_doc_comment(&mut out, "", &utility.doc);
    let js_name = to_ts_camel_case(&utility.name);
    // Greeks params (`option_price`, `div_yield`) carry an underscore, so
    // the napi-rs auto-camelCased JS arg names (`optionPrice`, `divYield`)
    // differ from the Rust idents. The Rust param idents stay snake_case so
    // the core-fn call site reads naturally; napi handles the JS-name lift.
    match utility.kind {
        UtilityKind::AllGreeks => {
            writeln!(out, "#[napi(js_name = \"{js_name}\")]").unwrap();
            if utility.params.len() > 6 {
                out.push_str(
                    "#[allow(clippy::too_many_arguments)] // Reason: mirrors Black-Scholes parameter set expected by SDK callers\n",
                );
            }
            writeln!(out, "pub fn {}(", utility.name).unwrap();
            for param in &utility.params {
                writeln!(out, "    {}: {},", param.name, ts_param_type(param)).unwrap();
            }
            out.push_str(") -> napi::Result<AllGreeks> {\n");
            writeln!(
                out,
                "    let g = thetadatadx::greeks::all_greeks({}).map_err(thetadatadx::Error::from).map_err(to_napi_err)?;",
                ts_call_args(utility)
            )
            .unwrap();
            out.push_str("    Ok(AllGreeks {\n");
            for (field, rust_field) in greek_result_fields() {
                // The napi object field carries the TS spelling
                // (`lambda`); the `GreeksResult` member it reads stays the
                // bare Rust name (`lambda`).
                writeln!(out, "        {}: g.{rust_field},", ts_field_ident(field)).unwrap();
            }
            out.push_str("    })\n");
            out.push_str("}\n");
        }
        UtilityKind::ImpliedVolatility => {
            writeln!(out, "#[napi(js_name = \"{js_name}\")]").unwrap();
            if utility.params.len() > 6 {
                out.push_str(
                    "#[allow(clippy::too_many_arguments)] // Reason: mirrors Black-Scholes parameter set expected by SDK callers\n",
                );
            }
            writeln!(out, "pub fn {}(", utility.name).unwrap();
            for param in &utility.params {
                writeln!(out, "    {}: {},", param.name, ts_param_type(param)).unwrap();
            }
            // napi-rs maps a Rust `(f64, f64)` return to a JS
            // `[number, number]` tuple, matching the Python
            // `tuple[float, float]` `(iv, iv_error)` shape exactly.
            out.push_str(") -> napi::Result<(f64, f64)> {\n");
            writeln!(
                out,
                "    thetadatadx::greeks::implied_volatility({}).map_err(thetadatadx::Error::from).map_err(to_napi_err)",
                ts_call_args(utility)
            )
            .unwrap();
            out.push_str("}\n");
        }
        other => panic!("unsupported TypeScript utility kind: {other:?}"),
    }
    out
}

/// napi-rs Rust parameter type for a utility param. The offline Greeks
/// calculators take `f64` scalars and a `String` right side (napi
/// accepts the JS string by value at the boundary).
fn ts_param_type(param: &super::spec::ParamSpec) -> &'static str {
    use super::spec::ParamType;
    match param.param_type {
        ParamType::String => "String",
        ParamType::F64 => "f64",
        ParamType::I32 => "i32",
        ParamType::U64 => "u32",
        ParamType::CredentialsRef | ParamType::ConfigRef => {
            panic!("credentials/config refs are not valid for TypeScript utility emitters")
        }
    }
}

/// Comma-joined argument list for the core `thetadatadx::greeks::*`
/// call. String params cross the napi boundary as an owned `String`, so
/// they are forwarded as `&right`; scalar params pass by value.
fn ts_call_args(utility: &UtilitySpec) -> String {
    use super::spec::ParamType;
    utility
        .params
        .iter()
        .map(|param| match param.param_type {
            ParamType::String => format!("&{}", param.name),
            _ => param.name.clone(),
        })
        .collect::<Vec<_>>()
        .join(", ")
}
