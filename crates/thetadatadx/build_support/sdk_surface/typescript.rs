//! TypeScript (napi-rs) streaming methods for `sdks/typescript/src/streaming_methods.rs`.

use std::fmt::Write as _;

use super::common::{generated_header, push_rust_doc_comment};
use super::spec::{MethodKind, MethodSpec};

pub(super) fn render_ts_streaming_methods(methods: &[&MethodSpec]) -> String {
    let mut out = String::new();
    out.push_str(generated_header());
    out.push_str("#[napi]\n");
    out.push_str("impl ThetaDataDx {\n");
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
    // require a callback-specific docstring after PR D (#482); render
    // those inline below instead of re-using the shared one.
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
                 The LMAX Disruptor consumer thread routes every typed\n\
                 FPSS event through napi-rs `ThreadsafeFunction` to the\n\
                 Node main thread, where the user's `callback(event)`\n\
                 runs. The FPSS TLS reader thread itself never touches\n\
                 V8: events cross the Disruptor ring first, with the\n\
                 consumer thread invoking the callback under\n\
                 `catch_unwind`.\n\
                 \n\
                 Node's libuv requires JS callbacks on the main thread,\n\
                 so `ThreadsafeFunction` (with its internal `uv_async_t`\n\
                 queue) is the only safe path. This binding deliberately\n\
                 does NOT expose a `start_streaming_inline` opt-in:\n\
                 calling into V8 from any thread other than the main\n\
                 loop is undefined behavior.\n\
                 \n\
                 Backpressure: a slow callback fills the Disruptor ring\n\
                 and overflow events are dropped, observable via\n\
                 `droppedEventCount()`. The FPSS TLS reader is never\n\
                 blocked — vendor disconnects on slow consumers cannot\n\
                 happen on this path.",
            );
            writeln!(out, "    #[napi(js_name = \"startStreaming\")]").unwrap();
            // `ThreadsafeFunction<FpssEvent, ErrorStrategy::CalleeHandled>`
            // is the napi-rs handle that owns a JS function reference
            // and routes calls onto the V8 main thread via
            // `uv_async_t`. We register a Rust closure with the SSOT
            // dispatcher; the closure clones the `ThreadsafeFunction`
            // (cheap `Arc` bump) and calls it with the typed event.
            writeln!(
                out,
                "    pub fn {}(&self, callback: napi::threadsafe_function::ThreadsafeFunction<FpssEvent, (), FpssEvent, napi::Status, false>) -> napi::Result<()> {{",
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
            out.push_str("        self.tdx.is_streaming()\n");
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
                "        self.tdx.{}(&contract).map_err(to_napi_err)",
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
                "        let contract = fpss::protocol::Contract::option(&{}, (&{}, &{}, &{})).map_err(to_napi_err)?;",
                method.params[0].name,
                method.params[1].name,
                method.params[2].name,
                method.params[3].name
            )
            .unwrap();
            writeln!(
                out,
                "        self.tdx.{}(&contract).map_err(to_napi_err)",
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
                "        self.tdx.{}(st).map_err(to_napi_err)",
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
            out.push_str("        self.tdx\n");
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
            // TypeScript removed `next_event` in PR D (#482) — the
            // napi-rs binding now uses callback registration via
            // `startStreaming(callback)`. The TS target is no longer
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
                 restored on the new connection — see\n\
                 `thetadatadx::ThetaDataDx::reconnect_streaming` for\n\
                 partial-failure semantics.",
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
        MethodKind::StopStreaming | MethodKind::Shutdown => {
            let js_name = to_ts_camel_case(&method.name);
            writeln!(out, "    #[napi(js_name = \"{js_name}\")]").unwrap();
            writeln!(out, "    pub fn {}(&self) {{", method.name).unwrap();
            // PR D (#482) replaced the receiver `rx` field with a
            // stored `ThreadsafeFunction` callback. Drop the stored
            // handle so the napi reference is released before the
            // streaming side tears down — re-installing via
            // `startStreaming` after stop / shutdown then sees a clean
            // slot.
            out.push_str("        self.tdx.stop_streaming();\n");
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
