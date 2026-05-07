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
    // The doc string in `sdk_surface.toml` is the cross-language
    // semantic summary. Two Python kinds (StartStreaming, Reconnect)
    // require a callback-specific docstring after PR C (#482); render
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
                "Start FPSS streaming and register a Python callback for incoming events.\n\
                 \n\
                 The dispatcher's drain thread acquires the GIL via\n\
                 `Python::attach` to call `callback(event)` for every\n\
                 typed FPSS event. `callback` must accept exactly one\n\
                 positional argument — a `Quote`, `Trade`, `Ohlcvc`,\n\
                 `OpenInterest`, `Simple`, or `RawData` instance.\n\
                 \n\
                 Events flow from the FPSS reader thread through the\n\
                 SSOT `StreamingDispatcher` (bounded crossbeam queue,\n\
                 8192 slots) onto a dedicated drain thread that runs\n\
                 `callback`. The reader never blocks on user code; if\n\
                 the callback falls behind, overflow events are\n\
                 dropped and counted via `dropped_event_count()`.\n\
                 \n\
                 GIL acquisition can block, so this Python binding\n\
                 deliberately does NOT expose `start_streaming_inline`.\n\
                 A slow Python callable on the FPSS reader thread\n\
                 would fill the kernel TCP receive buffer and trigger\n\
                 a vendor-side disconnect — there is no safe way to\n\
                 acquire the GIL inside the FPSS read loop.\n\
                 \n\
                 Exceptions raised inside `callback` are routed through\n\
                 `PyErr::write_unraisable` (visible in `sys.stderr` and\n\
                 the unraisable hook) so a buggy callback cannot kill\n\
                 the streaming thread.",
            );
            writeln!(
                out,
                "    fn {}(&self, callback: Py<PyAny>) -> PyResult<()> {{",
                method.name
            )
            .unwrap();
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
                "        let contract = fpss::protocol::Contract::option({}, ({}, {}, {})).map_err(to_py_err)?;",
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
            // Python removed `next_event` in PR C (#482) — the PyO3
            // binding now uses callback registration via
            // `start_streaming(callback)`. The Python target is no
            // longer in `MethodKind::NextEvent`'s allowed list (see
            // `spec.rs`), so this arm is unreachable on the Python
            // surface. Panicking here is the loud failure we want if
            // someone re-adds `python_unified` to `next_event` without
            // also implementing a poll-style PyO3 method.
            panic!("MethodKind::NextEvent is not emitted on the Python target after PR C");
        }
        MethodKind::Reconnect => {
            push_rust_doc_comment(
                &mut out,
                "    ",
                "Reconnect FPSS streaming and re-register the previously installed callback.\n\
                 \n\
                 Requires a prior `start_streaming(callback)`; raises\n\
                 `RuntimeError` if no callback is registered. All\n\
                 active subscriptions are restored on the new\n\
                 connection — see `thetadatadx::ThetaDataDx::reconnect_streaming`\n\
                 for partial-failure semantics.",
            );
            writeln!(out, "    fn {}(&self) -> PyResult<()> {{", method.name).unwrap();
            out.push_str(include_str!("templates/python/reconnect_body.rs.tmpl"));
        }
        MethodKind::StopStreaming | MethodKind::Shutdown => {
            writeln!(out, "    fn {}(&self) {{", method.name).unwrap();
            // PR C (#482) replaced the receiver `rx` field with a
            // stored `Py<PyAny>` callback. Drop the callable so the
            // Python reference is released before the streaming side
            // tears down — re-installing via `start_streaming` after
            // stop / shutdown then sees a clean slot.
            out.push_str("        self.tdx.stop_streaming();\n");
            out.push_str(
                "        let mut guard = self.callback.lock().unwrap_or_else(|e| e.into_inner());\n",
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
