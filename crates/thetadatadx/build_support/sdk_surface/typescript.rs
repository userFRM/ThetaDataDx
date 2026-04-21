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
    push_rust_doc_comment(&mut out, "    ", &method.doc);
    match method.kind {
        MethodKind::StartStreaming => {
            writeln!(out, "    #[napi(js_name = \"startStreaming\")]").unwrap();
            writeln!(
                out,
                "    pub fn {}(&self) -> napi::Result<()> {{",
                method.name
            )
            .unwrap();
            // Clone the instance-level `Arc<AtomicU64>` into the closure.
            // Counter lives on `ThetaDataDx`, so it survives reconnect
            // and is observable from JS via `tdx.droppedEvents()` — a
            // closure-local `AtomicU64::new(0)` would reset on every
            // reconnect and be unreachable from consumers.
            //
            // `debug!` is the level ops-teams enable in production when
            // diagnosing drops (see Python SDK comment for full detail).
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
                "        let contract = fpss::protocol::Contract::option(&{}, &{}, &{}, &{}).map_err(to_napi_err)?;",
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
        MethodKind::ContractMap => {
            writeln!(out, "    #[napi(js_name = \"contractMap\")]").unwrap();
            writeln!(
                out,
                "    pub fn {}(&self) -> napi::Result<std::collections::HashMap<String, String>> {{",
                method.name
            )
            .unwrap();
            out.push_str("        self.tdx\n");
            out.push_str("            .contract_map()\n");
            out.push_str("            .map(|m| m.into_iter().map(|(id, c)| (id.to_string(), format!(\"{c}\"))).collect())\n");
            out.push_str("            .map_err(to_napi_err)\n");
            out.push_str("    }\n");
        }
        MethodKind::ContractLookup => {
            let param = &method.params[0];
            writeln!(out, "    #[napi(js_name = \"contractLookup\")]").unwrap();
            writeln!(
                out,
                "    pub fn {}(&self, {}: i32) -> napi::Result<Option<String>> {{",
                method.name, param.name,
            )
            .unwrap();
            writeln!(out, "        self.tdx.contract_lookup({})", param.name).unwrap();
            out.push_str("            .map(|opt| opt.map(|c| format!(\"{c}\")))\n");
            out.push_str("            .map_err(to_napi_err)\n");
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
            let param = &method.params[0];
            // Override the TS return type with a proper discriminated union so
            // `switch (ev.kind) case 'quote': ...` narrows `ev.quote` to
            // `Quote` (not `Quote | undefined`). The flat `FpssEvent` interface
            // that napi-rs emits from the Rust struct does not narrow in TS.
            // The union literal is generator-derived from
            // `fpss_event_schema.toml` via `fpss_events::ts_next_event_union_type`
            // so adding a new data variant tomorrow updates both sides.
            let union_ts = super::super::fpss_events::ts_next_event_union_type();
            // Wrap in `Promise<>` because the fn is `async`. napi-rs
            // would default-emit `Promise<FpssEvent | null>`, losing
            // the discriminated-union narrowing we want for
            // `switch (ev.kind)`; override preserves it.
            writeln!(
                out,
                "    #[napi(js_name = \"nextEvent\", ts_return_type = \"Promise<{union_ts}>\")]"
            )
            .unwrap();
            // ASYNC napi fn so `recv_timeout` runs on a tokio blocking
            // worker instead of the V8 main thread. An earlier sync
            // implementation called `rx.recv_timeout(timeout)` directly,
            // which froze the Node event loop for up to `timeout_ms`
            // milliseconds per call — any setTimeout / I/O callback on
            // the JS side stalled with it. Surfacing as `async` lets
            // Node keep servicing other work while the Rust side blocks.
            // JS callers now `await tdx.nextEvent(...)` instead of
            // calling it synchronously (breaking change in v7.4.0;
            // documented in CHANGELOG).
            writeln!(
                out,
                "    pub async fn {}(&self, {}: f64) -> napi::Result<Option<FpssEvent>> {{",
                method.name, param.name,
            )
            .unwrap();
            // Resolve the receiver handle in a scoped block so the outer
            // `MutexGuard` drops BEFORE we `.await` the spawned blocking
            // task — otherwise the guard would be held across `.await`
            // and the compiler (correctly) refuses to make the future
            // `Send`.
            out.push_str(include_str!(
                "templates/typescript/next_event_prelude.rs.tmpl"
            ));
            writeln!(
                out,
                "        let timeout = std::time::Duration::from_millis({} as u64);",
                param.name
            )
            .unwrap();
            // `spawn_blocking` offloads the OS-level blocking wait to a
            // dedicated tokio worker so the V8 main thread is free to
            // service other JS work while we wait for the next frame.
            //
            // Disconnected = streaming loop dropped the sender half.
            // Surface as an error, not `null`, so dead-socket consumers
            // can reconnect explicitly.
            out.push_str(include_str!("templates/typescript/next_event_body.rs.tmpl"));
        }
        MethodKind::Reconnect => {
            writeln!(out, "    #[napi(js_name = \"reconnect\")]").unwrap();
            writeln!(
                out,
                "    pub fn {}(&self) -> napi::Result<()> {{",
                method.name
            )
            .unwrap();
            // Clone the instance-level counter so the drop count survives
            // reconnect (see `StartStreaming` for the full rationale).
            out.push_str(include_str!("templates/typescript/reconnect_body.rs.tmpl"));
        }
        MethodKind::StopStreaming | MethodKind::Shutdown => {
            let js_name = to_ts_camel_case(&method.name);
            writeln!(out, "    #[napi(js_name = \"{js_name}\")]").unwrap();
            writeln!(out, "    pub fn {}(&self) {{", method.name).unwrap();
            out.push_str("        self.tdx.stop_streaming();\n");
            out.push_str(
                "        let mut guard = self.rx.lock().unwrap_or_else(|e| e.into_inner());\n",
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
