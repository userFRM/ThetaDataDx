//! Python (`pyo3`) streaming methods + utility functions.

use std::fmt::Write as _;

use super::common::{generated_header, greek_result_fields, push_rust_doc_comment, python_type};
use super::spec::{MethodKind, MethodSpec, UtilityKind, UtilitySpec};

pub(super) fn render_python_streaming_methods(methods: &[&MethodSpec]) -> String {
    let mut out = String::new();
    out.push_str(generated_header());
    out.push_str("#[pymethods]\n");
    out.push_str("impl ThetaDataDx {\n");
    for method in methods {
        out.push_str(&python_streaming_method(method));
        out.push('\n');
    }
    out.push_str("}\n");
    out
}

pub(super) fn render_python_utility_functions(utilities: &[&UtilitySpec]) -> String {
    let mut out = String::new();
    out.push_str(generated_header());

    // Emit the AllGreeks pyclass wrapper BEFORE any `#[pyfunction]` that
    // returns it. Mirrors the typed-pyclass policy applied to every other
    // Python return path — no PyDict leaks into the public Python surface.
    let has_all_greeks = utilities
        .iter()
        .any(|u| matches!(u.kind, UtilityKind::AllGreeks));
    if has_all_greeks {
        out.push_str(&render_all_greeks_pyclass());
        out.push('\n');
    }

    for utility in utilities {
        out.push_str(&python_utility_function(utility));
        out.push('\n');
    }
    out.push_str(
        "fn register_generated_utility_functions(m: &Bound<'_, PyModule>) -> PyResult<()> {\n",
    );
    if has_all_greeks {
        out.push_str("    m.add_class::<AllGreeks>()?;\n");
    }
    for utility in utilities {
        writeln!(
            out,
            "    m.add_function(wrap_pyfunction!({}, m)?)?;",
            utility.name
        )
        .unwrap();
    }
    out.push_str("    Ok(())\n");
    out.push_str("}\n");
    out
}

/// Emit a typed `AllGreeks` pyclass so `all_greeks(...)` returns a
/// frozen, attribute-accessible value instead of a `PyDict`. Fields
/// mirror `tdbe::greeks::GreeksResult` 1:1 via `greek_result_fields()`.
fn render_all_greeks_pyclass() -> String {
    let mut out = String::new();
    out.push_str(include_str!(
        "templates/python/all_greeks_pyclass_header.rs.tmpl"
    ));
    for (field, _rust_field) in greek_result_fields() {
        writeln!(out, "    #[pyo3(get)]").unwrap();
        writeln!(out, "    pub {field}: f64,").unwrap();
    }
    out.push_str("}\n\n");
    out.push_str(include_str!(
        "templates/python/all_greeks_pymethods.rs.tmpl"
    ));
    out
}

fn python_streaming_method(method: &MethodSpec) -> String {
    let mut out = String::new();
    push_rust_doc_comment(&mut out, "    ", &method.doc);
    match method.kind {
        MethodKind::StartStreaming => {
            writeln!(out, "    fn {}(&self) -> PyResult<()> {{", method.name).unwrap();
            // Clone the instance-level `Arc<AtomicU64>` into the closure.
            // Counter lives on `ThetaDataDx`, so it survives reconnect and
            // is observable from Python via `tdx.dropped_events()` — a
            // closure-local `AtomicU64::new(0)` would reset on every
            // reconnect and would never be reachable from consumers.
            //
            // `debug!` is the level ops-teams enable in production when
            // diagnosing drops. `trace` is too quiet (default filters
            // strip it); `warn` is too loud for normal shutdown-time
            // drops. Consumers polling via `dropped_events()` remain the
            // primary observability path.
            //
            // Recover poisoned lock rather than silently dropping the
            // swap. A stale receiver behind a closed channel is worse
            // than a partial state from a prior panic.
            out.push_str(include_str!(
                "templates/python/start_streaming_body.rs.tmpl"
            ));
        }
        MethodKind::IsStreaming => {
            writeln!(out, "    fn {}(&self) -> bool {{", method.name).unwrap();
            out.push_str("        self.tdx.is_streaming()\n");
            out.push_str("    }\n");
        }
        MethodKind::StockContractCall => {
            let param = &method.params[0];
            writeln!(
                out,
                "    fn {}(&self, {}: {}) -> PyResult<()> {{",
                method.name,
                param.name,
                python_type(param.param_type)
            )
            .unwrap();
            writeln!(
                out,
                "        let contract = fpss::protocol::Contract::stock({});",
                param.name
            )
            .unwrap();
            writeln!(
                out,
                "        self.tdx.{}(&contract).map_err(to_py_err)",
                method.runtime_call.as_deref().unwrap()
            )
            .unwrap();
            out.push_str("    }\n");
        }
        MethodKind::OptionContractCall => {
            writeln!(out, "    fn {}(", method.name).unwrap();
            out.push_str("        &self,\n");
            for param in &method.params {
                writeln!(
                    out,
                    "        {}: {},",
                    param.name,
                    python_type(param.param_type)
                )
                .unwrap();
            }
            out.push_str("    ) -> PyResult<()> {\n");
            writeln!(
                out,
                "        let contract = fpss::protocol::Contract::option({}, {}, {}, {}).map_err(to_py_err)?;",
                method.params[0].name,
                method.params[1].name,
                method.params[2].name,
                method.params[3].name
            )
            .unwrap();
            writeln!(
                out,
                "        self.tdx.{}(&contract).map_err(to_py_err)",
                method.runtime_call.as_deref().unwrap()
            )
            .unwrap();
            out.push_str("    }\n");
        }
        MethodKind::FullCall => {
            let param = &method.params[0];
            writeln!(
                out,
                "    fn {}(&self, {}: {}) -> PyResult<()> {{",
                method.name,
                param.name,
                python_type(param.param_type)
            )
            .unwrap();
            writeln!(out, "        let st = parse_sec_type({})?;", param.name).unwrap();
            writeln!(
                out,
                "        self.tdx.{}(st).map_err(to_py_err)",
                method.runtime_call.as_deref().unwrap()
            )
            .unwrap();
            out.push_str("    }\n");
        }
        MethodKind::ContractMap => {
            writeln!(
                out,
                "    fn {}(&self) -> PyResult<std::collections::HashMap<i32, String>> {{",
                method.name
            )
            .unwrap();
            out.push_str("        self.tdx\n");
            out.push_str("            .contract_map()\n");
            out.push_str("            .map(|m| m.into_iter().map(|(id, c)| (id, format!(\"{c}\"))).collect())\n");
            out.push_str("            .map_err(to_py_err)\n");
            out.push_str("    }\n");
        }
        MethodKind::ContractLookup => {
            let param = &method.params[0];
            writeln!(
                out,
                "    fn {}(&self, {}: {}) -> PyResult<Option<String>> {{",
                method.name,
                param.name,
                python_type(param.param_type)
            )
            .unwrap();
            writeln!(out, "        self.tdx.contract_lookup({})", param.name).unwrap();
            out.push_str("            .map(|opt| opt.map(|c| format!(\"{c}\")))\n");
            out.push_str("            .map_err(to_py_err)\n");
            out.push_str("    }\n");
        }
        MethodKind::ActiveSubscriptions => {
            writeln!(
                out,
                "    fn {}(&self) -> PyResult<Vec<std::collections::HashMap<String, String>>> {{",
                method.name
            )
            .unwrap();
            out.push_str("        self.tdx\n");
            out.push_str("            .active_subscriptions()\n");
            out.push_str("            .map(|subs| {\n");
            out.push_str("                subs.into_iter()\n");
            out.push_str("                    .map(|(kind, contract)| {\n");
            out.push_str("                        let mut m = std::collections::HashMap::new();\n");
            out.push_str(
                "                        m.insert(\"kind\".to_string(), format!(\"{kind:?}\"));\n",
            );
            out.push_str("                        m.insert(\"contract\".to_string(), format!(\"{contract}\"));\n");
            out.push_str("                        m\n");
            out.push_str("                    })\n");
            out.push_str("                    .collect()\n");
            out.push_str("            })\n");
            out.push_str("            .map_err(to_py_err)\n");
            out.push_str("    }\n");
        }
        MethodKind::NextEvent => {
            let param = &method.params[0];
            writeln!(
                out,
                "    fn {}(&self, py: Python<'_>, {}: {}) -> PyResult<Option<Py<PyAny>>> {{",
                method.name,
                param.name,
                python_type(param.param_type)
            )
            .unwrap();
            out.push_str(include_str!("templates/python/next_event_prelude.rs.tmpl"));
            writeln!(
                out,
                "        let total_timeout = std::time::Duration::from_millis({});",
                param.name
            )
            .unwrap();
            // Poll in ≤100 ms chunks so Python's `KeyboardInterrupt`
            // (Ctrl+C) is honoured even on multi-minute `timeout_ms`
            // values. A pure blocking `recv_timeout(total_timeout)` on a
            // cold subscription makes the process un-killable from the
            // REPL — signals are delivered to the main thread, but the
            // GIL is released inside `py.detach` and `check_signals`
            // never runs until the mpsc wakes us. Asymmetric with
            // `run_blocking` (which polls signals on the async side
            // every 100 ms); unify the two cancellation stories here.
            // Disconnect is still distinguished from timeout so consumer
            // loops don't spin 100% CPU on a dead socket.
            out.push_str(include_str!("templates/python/next_event_body.rs.tmpl"));
        }
        MethodKind::Reconnect => {
            writeln!(out, "    fn {}(&self) -> PyResult<()> {{", method.name).unwrap();
            // Clone the instance-level counter so the drop count survives
            // reconnect (see `StartStreaming` for the full rationale).
            out.push_str(include_str!("templates/python/reconnect_body.rs.tmpl"));
        }
        MethodKind::StopStreaming | MethodKind::Shutdown => {
            writeln!(out, "    fn {}(&self) {{", method.name).unwrap();
            out.push_str("        self.tdx.stop_streaming();\n");
            out.push_str(
                "        let mut guard = self.rx.lock().unwrap_or_else(|e| e.into_inner());\n",
            );
            out.push_str("        *guard = None;\n");
            out.push_str("    }\n");
        }
        other => panic!("unsupported Python method kind: {other:?}"),
    }
    out
}

fn python_utility_function(utility: &UtilitySpec) -> String {
    let mut out = String::new();
    push_rust_doc_comment(&mut out, "", &utility.doc);
    out.push_str("#[pyfunction]\n");
    if utility.params.len() > 6 {
        out.push_str(
            "#[allow(clippy::too_many_arguments)] // Reason: mirrors Black-Scholes parameter set expected by SDK callers\n",
        );
    }
    match utility.kind {
        UtilityKind::AllGreeks => {
            writeln!(out, "fn {}(", utility.name).unwrap();
            for param in &utility.params {
                writeln!(
                    out,
                    "    {}: {},",
                    param.name,
                    python_type(param.param_type)
                )
                .unwrap();
            }
            out.push_str(") -> PyResult<AllGreeks> {\n");
            writeln!(
                out,
                "    let g = tdbe::greeks::all_greeks({}).map_err(thetadatadx::Error::from).map_err(to_py_err)?;",
                utility
                    .params
                    .iter()
                    .map(|param| param.name.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            )
            .unwrap();
            out.push_str("    Ok(AllGreeks {\n");
            for (field, rust_field) in greek_result_fields() {
                writeln!(out, "        {field}: g.{rust_field},").unwrap();
            }
            out.push_str("    })\n");
            out.push_str("}\n");
        }
        UtilityKind::ImpliedVolatility => {
            writeln!(out, "fn {}(", utility.name).unwrap();
            for param in &utility.params {
                writeln!(
                    out,
                    "    {}: {},",
                    param.name,
                    python_type(param.param_type)
                )
                .unwrap();
            }
            out.push_str(") -> PyResult<(f64, f64)> {\n");
            writeln!(
                out,
                "    tdbe::greeks::implied_volatility({}).map_err(thetadatadx::Error::from).map_err(to_py_err)",
                utility
                    .params
                    .iter()
                    .map(|param| param.name.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            )
            .unwrap();
            out.push_str("}\n");
        }
        other => panic!("unsupported Python utility kind: {other:?}"),
    }
    out
}
