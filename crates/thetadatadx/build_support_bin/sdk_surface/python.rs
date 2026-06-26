//! Python (`pyo3`) streaming methods + utility functions.

use std::fmt::Write as _;

use super::common::{
    generated_header, greek_result_fields, push_rust_doc_comment, python_field_ident, python_type,
};
use super::spec::{ForwardReturn, MethodKind, MethodSpec, UtilityKind, UtilitySpec};

/// Renders the Python streaming methods source: the FPSS method-name inventory const and the `#[pymethods]` block on `Client`.
pub(super) fn render_python_streaming_methods(methods: &[&MethodSpec]) -> String {
    let mut out = String::new();
    out.push_str(generated_header());

    // Emit the inventory of FPSS-touching method names before the
    // `#[pymethods]` block. `mdds_client.rs` references this const in
    // a compile-time guard so its `BLOCKED_FPSS_METHODS` list can
    // never drift below the actual generated surface.
    out.push_str(
        "/// Names of every FPSS-touching method emitted on \
         `StreamView`.\n",
    );
    out.push_str(
        "/// SSOT for cross-class block-list drift checks (see \
         `mdds_client.rs`).\n",
    );
    out.push_str("pub(crate) const PYTHON_UNIFIED_FPSS_METHODS: &[&str] = &[\n");
    for method in methods {
        writeln!(out, "    \"{}\",", method.name).unwrap();
    }
    out.push_str("];\n\n");

    out.push_str("#[pymethods]\n");
    out.push_str("impl StreamView {\n");
    for method in methods {
        out.push_str(&python_streaming_method(method));
        out.push('\n');
    }
    out.push_str("}\n");
    out
}

/// Whether a utility kind lives in the `thetadatadx.util` submodule (the
/// lookup-table helpers) rather than as a top-level `#[pyfunction]` on the
/// root module (the offline Greeks calculators).
fn is_util_submodule_kind(kind: UtilityKind) -> bool {
    matches!(
        kind,
        UtilityKind::Forwarder
            | UtilityKind::CalendarStatusName
            | UtilityKind::TimestampMs
            | UtilityKind::SequenceSignedToUnsigned
            | UtilityKind::SequenceUnsignedToSigned
    )
}

/// Renders the Python utility functions source: the `AllGreeks` pyclass, the `#[pyfunction]` wrappers, and their module registration helpers.
pub(super) fn render_python_utility_functions(utilities: &[&UtilitySpec]) -> String {
    let mut out = String::new();
    out.push_str(generated_header());

    // Two registration scopes: the offline Greeks calculators bind on the
    // root module; the lookup-table helpers bind on the `thetadatadx.util`
    // submodule. Partition once so each renders into its own helper.
    let root: Vec<&&UtilitySpec> = utilities
        .iter()
        .filter(|u| !is_util_submodule_kind(u.kind))
        .collect();
    let util_mod: Vec<&&UtilitySpec> = utilities
        .iter()
        .filter(|u| is_util_submodule_kind(u.kind))
        .collect();

    // Emit the AllGreeks pyclass wrapper BEFORE any `#[pyfunction]` that
    // returns it. Mirrors the typed-pyclass policy applied to every other
    // Python return path — no PyDict leaks into the public Python surface.
    let has_all_greeks = root
        .iter()
        .any(|u| matches!(u.kind, UtilityKind::AllGreeks));
    if has_all_greeks {
        out.push_str(&render_all_greeks_pyclass());
        out.push('\n');
    }

    for utility in &root {
        out.push_str(&python_utility_function(utility));
        out.push('\n');
    }
    out.push_str(
        "fn register_generated_utility_functions(m: &Bound<'_, PyModule>) -> PyResult<()> {\n",
    );
    if has_all_greeks {
        out.push_str("    m.add_class::<AllGreeks>()?;\n");
    }
    for utility in &root {
        writeln!(
            out,
            "    m.add_function(wrap_pyfunction!({}, m)?)?;",
            utility.name
        )
        .unwrap();
    }
    out.push_str("    Ok(())\n");
    out.push_str("}\n\n");

    for utility in &util_mod {
        out.push_str(&python_utility_function(utility));
        out.push('\n');
    }
    out.push_str(&render_util_submodule_register(&util_mod));
    out
}

/// Emit `register_generated_util_submodule(parent)` — builds the
/// `thetadatadx.util` child module, adds every lookup-table helper, and
/// registers it both as a submodule and under the dotted `sys.modules`
/// name so `import thetadatadx.util` works like a pure-Python submodule.
fn render_util_submodule_register(utilities: &[&&UtilitySpec]) -> String {
    let mut out = String::new();
    out.push_str(
        "/// Register the `thetadatadx.util` submodule on the parent module.\n\
         ///\n\
         /// All functions are added to a child PyModule named `util`, then that\n\
         /// child is registered both as a submodule of the parent and (so\n\
         /// `import thetadatadx.util` works) inserted into `sys.modules` under\n\
         /// the dotted name. This is the standard pyo3 idiom for native Python\n\
         /// submodules.\n",
    );
    out.push_str(
        "pub(crate) fn register_generated_util_submodule(parent: &Bound<'_, PyModule>) -> PyResult<()> {\n",
    );
    out.push_str("    let py = parent.py();\n");
    out.push_str("    let util = PyModule::new(py, \"util\")?;\n");
    for utility in utilities {
        writeln!(
            out,
            "    util.add_function(wrap_pyfunction!({}, &util)?)?;",
            utility.name
        )
        .unwrap();
    }
    out.push('\n');
    out.push_str(
        "    // Insert under the dotted name so `import thetadatadx.util` works\n\
         \x20   // identically to a pure-Python submodule.\n",
    );
    out.push_str("    let sys_modules = py.import(\"sys\")?.getattr(\"modules\")?;\n");
    out.push_str("    sys_modules.set_item(\"thetadatadx.util\", &util)?;\n");
    out.push('\n');
    out.push_str("    parent.add_submodule(&util)?;\n");
    out.push_str("    Ok(())\n");
    out.push_str("}\n");
    out
}

/// Emit a typed `AllGreeks` pyclass so `all_greeks(...)` returns a
/// frozen, attribute-accessible value instead of a `PyDict`. Fields
/// mirror `thetadatadx::greeks::GreeksResult` 1:1 via `greek_result_fields()`.
fn render_all_greeks_pyclass() -> String {
    let mut out = String::new();
    out.push_str(include_str!(
        "templates/python/all_greeks_pyclass_header.rs.tmpl"
    ));
    for (field, _rust_field) in greek_result_fields() {
        // PEP 8 keyword escape (`lambda` -> `lambda_`) keeps every
        // attribute reachable with normal Python syntax, matching the
        // tick pyclasses (`GreeksAllTick.lambda_`). Without it the
        // `lambda` Greek would be reachable only via `getattr`.
        writeln!(out, "    #[pyo3(get)]").unwrap();
        writeln!(out, "    pub {}: f64,", python_field_ident(field)).unwrap();
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
                "Start streaming and register a Python callback for incoming events.\n\
                 \n\
                 The dispatcher thread acquires the GIL\n\
                 via `Python::attach` to call `callback(event)` for\n\
                 every typed streaming event, with each invocation wrapped\n\
                 in `catch_unwind`. `callback` must accept exactly one\n\
                 positional argument — a typed streaming event class\n\
                 (`Quote`, `Trade`, `Ohlcvc`, `OpenInterest`,\n\
                 `LoginSuccess`, `ContractAssigned`, `ReqResponse`,\n\
                 `MarketOpen`, `MarketClose`, `ServerError`,\n\
                 `Disconnected`, `Reconnecting`, `Reconnected`, `Error`,\n\
                 `UnknownFrame`, `Connected`, `Ping`, `ReconnectedServer`,\n\
                 `Restart`, `UnknownControl`). Dispatch via\n\
                 `match event: case Quote(): ...`. Truncated / unrecognised\n\
                 wire frames are filtered before the callback fires and\n\
                 accounted on the `thetadatadx.fpss.decode_failures` metric.\n\
                 \n\
                 Events flow from the streaming reader thread into the\n\
                 streaming ring (`Producer::try_publish`) and out\n\
                 to the consumer thread that runs `callback`. The\n\
                 reader never blocks on user code; if the callback\n\
                 falls behind, ring-overflow events are dropped and\n\
                 counted via `dropped_event_count()`.\n\
                 \n\
                 Exceptions raised inside `callback` are caught by the\n\
                 `catch_unwind` boundary and routed through\n\
                 `PyErr::write_unraisable` (visible in `sys.stderr` and\n\
                 the unraisable hook); each one bumps `panic_count()`.",
            );
            // `pub(crate)` so the `StreamableHandle` enum in
            // `streaming_session.rs` can dispatch through the typed
            // pyclass borrow without going back through Python
            // attribute lookup.
            writeln!(
                out,
                "    pub(crate) fn {}(&self, py: Python<'_>, callback: Py<PyAny>) -> PyResult<()> {{",
                method.name
            )
            .unwrap();
            out.push_str(include_str!(
                "templates/python/start_streaming_body.rs.tmpl"
            ));
        }
        MethodKind::IsStreaming => {
            writeln!(out, "    fn {}(&self) -> bool {{", method.name).unwrap();
            out.push_str("        self.client.stream().is_streaming()\n");
            out.push_str("    }\n");
        }
        MethodKind::Batches => {
            // Thin entry: forward the optional tuning knobs to the
            // hand-written `RecordBatchStream` constructor (the protocol
            // surface — sync + async iteration, context managers — is
            // intrinsic Python shape, not a per-endpoint projection, so the
            // reader object itself is hand-written; only this entry is
            // generated so the cross-binding surface stays in lockstep).
            push_rust_doc_comment(
                &mut out,
                "    ",
                "Open a pull-based columnar reader over the live stream.\n\
                 \n\
                 Returns a `RecordBatchStream` — a sibling to the per-event\n\
                 `start_streaming(callback)`. The same subscriptions feed it,\n\
                 but market-data events arrive as `pyarrow.RecordBatch`\n\
                 values under a fixed schema. The reader is both a\n\
                 synchronous `Iterable` (the blocking pull releases the GIL)\n\
                 and an `AsyncIterable` (`async for`), and a sync / async\n\
                 context manager that closes the stream on exit. Subscribe on\n\
                 this same surface first, then open the reader.\n\
                 \n\
                 `batch_size` rows per batch (default 65536); `linger_ms`\n\
                 flushes a partial batch on a quiet stream (default 50);\n\
                 `backpressure` is `\"block\"` (default, lossless) or\n\
                 `\"drop_oldest\"`; `capacity` bounds the drop-oldest buffer.",
            );
            out.push_str(
                "    #[pyo3(signature = (*, batch_size=None, linger_ms=None, backpressure=None, capacity=None))]\n",
            );
            writeln!(out, "    fn {}(", method.name).unwrap();
            out.push_str("        &self,\n");
            out.push_str("        py: Python<'_>,\n");
            out.push_str("        batch_size: Option<usize>,\n");
            out.push_str("        linger_ms: Option<u64>,\n");
            out.push_str("        backpressure: Option<&str>,\n");
            out.push_str("        capacity: Option<usize>,\n");
            out.push_str("    ) -> PyResult<crate::streaming_batches::RecordBatchStream> {\n");
            out.push_str(
                "        crate::streaming_batches::open_reader(py, &self.client, batch_size, linger_ms, backpressure, capacity)\n",
            );
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
                "        self.client.stream().{}(&contract).map_err(to_py_err)",
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
                "        let contract = fpss::protocol::Contract::option({}, fpss::protocol::OptionLeg {{ expiration: {}, strike: {}, right: {} }}).map_err(to_py_err)?;",
                method.params[0].name,
                method.params[1].name,
                method.params[2].name,
                method.params[3].name
            )
            .unwrap();
            writeln!(
                out,
                "        self.client.stream().{}(&contract).map_err(to_py_err)",
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
                "        self.client.stream().{}(st).map_err(to_py_err)",
                method.runtime_call.as_deref().unwrap()
            )
            .unwrap();
            out.push_str("    }\n");
        }
        MethodKind::ActiveSubscriptions => {
            // Project per-contract subscriptions to typed `PySubscription`
            // values that round-trip with the `subscribe()` input shape.
            // The previous generator emitted `Vec<HashMap<String, String>>`
            // with debug-format strings, which contradicted the
            // `List[Subscription]` claim in the .pyi stub and broke the
            // `for sub in client.active_subscriptions(): client.unsubscribe(sub)`
            // user pattern. The hand-written `StreamingClient` projection
            // already followed this shape; this brings the unified
            // pyclass into lockstep.
            writeln!(
                out,
                "    fn {}(&self) -> PyResult<Vec<crate::fluent::PySubscription>> {{",
                method.name
            )
            .unwrap();
            out.push_str("        self.client\n");
            out.push_str("            .stream()\n");
            out.push_str("            .active_subscriptions()\n");
            out.push_str("            .map(|subs| {\n");
            out.push_str("                subs.into_iter()\n");
            out.push_str(
                "                    .map(|(kind, contract)| crate::fluent::PySubscription {\n",
            );
            out.push_str("                        inner: fpss::protocol::Subscription::Contract { contract, kind },\n");
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
                "Reconnect streaming and re-register the previously installed callback.\n\
                 \n\
                 Requires a prior `start_streaming(callback)`; raises\n\
                 `RuntimeError` if no callback is registered. All\n\
                 active subscriptions are restored on the new\n\
                 connection — see `thetadatadx::Client::reconnect_streaming`\n\
                 for partial-failure semantics.\n\
                 \n\
                 # Callback lifetime across `stop_streaming`\n\
                 \n\
                 `stop_streaming()` and `shutdown()` clear the registered\n\
                 callback. To resume streaming on this client after\n\
                 `stop_streaming()`, you MUST call `start_streaming(callback)`\n\
                 again with a freshly bound callable; `reconnect()` raises\n\
                 `RuntimeError` because no callback is held.\n\
                 \n\
                 This explicit-handoff model matches the C++ wrapper's RAII\n\
                 destructor and the Python `with` block's `__exit__`: the\n\
                 resource (the callback closure plus its captured environment)\n\
                 is cleared at the same scope boundary the user observes. The\n\
                 unified C API preserves the callback across stop/reconnect,\n\
                 but the Python and TypeScript bindings deliberately diverge\n\
                 to enforce the explicit handoff and avoid retaining captured\n\
                 references past a teardown the application has already\n\
                 observed.",
            );
            writeln!(
                out,
                "    fn {}(&self, py: Python<'_>) -> PyResult<()> {{",
                method.name
            )
            .unwrap();
            out.push_str(include_str!("templates/python/reconnect_body.rs.tmpl"));
        }
        MethodKind::AwaitDrain => {
            // The body wraps the core SDK's `await_drain(Duration)`. The
            // PyO3 surface takes a `u64` millisecond timeout to keep the
            // ABI symmetric with the TypeScript / C ABI / C++ surfaces;
            // every binding speaks milliseconds at the language boundary
            // and converts to `Duration` at the Rust callsite.
            // `pub(crate)` so the `StreamableHandle` enum in
            // `streaming_session.rs` can dispatch through the typed
            // pyclass borrow without going back through Python
            // attribute lookup.
            writeln!(
                out,
                "    pub(crate) fn {}(&self, py: Python<'_>, timeout_ms: u64) -> bool {{",
                method.name
            )
            .unwrap();
            // Release the GIL while polling the drain barrier; otherwise a
            // multi-second wait would block every other Python thread,
            // including the consumer thread that needs the GIL to fire
            // the user callback (which is exactly what await_drain is
            // waiting on).
            out.push_str("        py.detach(|| {\n");
            out.push_str(
                "            self.client.stream().await_drain(std::time::Duration::from_millis(timeout_ms))\n",
            );
            out.push_str("        })\n");
            out.push_str("    }\n");
        }
        MethodKind::StopStreaming | MethodKind::Shutdown => {
            // `pub(crate)` so the `StreamableHandle` enum in
            // `streaming_session.rs` can dispatch through the typed
            // pyclass borrow without going back through Python
            // attribute lookup.
            writeln!(
                out,
                "    pub(crate) fn {}(&self, py: Python<'_>) {{",
                method.name
            )
            .unwrap();
            // PR C (#482) replaced the receiver `rx` field with a
            // stored `Py<PyAny>` callback. Drop the callable so the
            // Python reference is released before the streaming side
            // tears down — re-installing via `start_streaming` after
            // stop / shutdown then sees a clean slot.
            //
            // Detach the GIL while the Rust teardown runs.
            // `Client::stop_streaming` drops the slot `Arc`;
            // if its refcount reaches zero the `StreamingClient` drop joins
            // the dispatcher thread, which re-acquires the GIL on every
            // event via `Python::attach`. Holding the GIL across the
            // join would deadlock.
            out.push_str("        py.detach(|| self.client.stream().stop_streaming());\n");
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

/// The doc comment text for a Python utility, or `None` when the surface
/// is intentionally undocumented (the lookup-table forwarders, whose
/// user-facing text lives in the `util.pyi` stub). The four special
/// helpers carry Python-specific doc that differs from the TypeScript
/// phrasing.
fn python_utility_doc(utility: &UtilitySpec) -> Option<&str> {
    match utility.kind {
        UtilityKind::AllGreeks | UtilityKind::ImpliedVolatility => Some(&utility.doc),
        UtilityKind::CalendarStatusName => Some(
            "Vendor vocabulary text for a calendar-day `status` code (`0` ->\n\
             `\"open\"`, `1` -> `\"early_close\"`, `2` -> `\"full_close\"`, `3` ->\n\
             `\"weekend\"`). Returns the literal `\"UNKNOWN\"` for codes outside the\n\
             table. Mirrors the C++ `thetadatadx::calendar_status_name` and the C ABI\n\
             `thetadatadx_calendar_status_name`.",
        ),
        UtilityKind::TimestampMs => Some(
            "Combine an Eastern-Time `YYYYMMDD` date and milliseconds-of-day into\n\
             Unix epoch milliseconds (UTC, DST-aware). Usable with any\n\
             `(date, *_ms_of_day)` pair on the tick structs. Returns `None` when\n\
             `date` is absent (`0`) or either input is out of domain — the same\n\
             `std::nullopt` contract the C++ `thetadatadx::timestamp_ms` returns (the C\n\
             ABI `thetadatadx_timestamp_ms` encodes that absence as the `-1` sentinel).",
        ),
        UtilityKind::SequenceSignedToUnsigned => Some(
            "Convert a signed wire-encoded trade-sequence value to its unsigned\n\
             monotonic form. `signed_value` must lie in the i32 wire range\n\
             (`-2_147_483_648 ..= 2_147_483_647`): the upstream terminal encodes\n\
             trade sequences as i32, so a value outside that domain is not a wire\n\
             sequence and is rejected with `ValueError` rather than silently\n\
             reinterpreted into a look-correct-but-wrong id. A value that does not\n\
             fit the `i64` parameter type still surfaces as the built-in\n\
             `OverflowError` from argument coercion, unchanged.",
        ),
        UtilityKind::SequenceUnsignedToSigned => Some(
            "Convert an unsigned monotonic trade-sequence value back to its signed\n\
             wire encoding. `unsigned_value` must lie in the unsigned wire range\n\
             (`0 ..= 2^32 - 1`): the monotonic sequence id is never wider than one\n\
             i32 cycle, so a value above that domain is rejected with `ValueError`\n\
             rather than silently reinterpreted. A negative argument still\n\
             surfaces as the built-in `OverflowError` from `u64` coercion,\n\
             unchanged.",
        ),
        UtilityKind::Forwarder => None,
        other => panic!("python_utility_doc: unsupported kind {other:?}"),
    }
}

fn python_utility_function(utility: &UtilitySpec) -> String {
    let mut out = String::new();
    // Doc policy differs by kind: the Greeks calculators carry the shared
    // cross-language `doc`; the four special helpers carry Python-specific
    // doc; the lookup-table forwarders are self-evident and undocumented
    // on the Python surface (the `.pyi` stub holds the user-facing text).
    if let Some(doc) = python_utility_doc(utility) {
        push_rust_doc_comment(&mut out, "", doc);
    }
    out.push_str("#[pyfunction]\n");
    if utility.params.len() > 6 {
        out.push_str(
            "#[allow(clippy::too_many_arguments)] // Reason: mirrors Black-Scholes parameter set expected by SDK callers\n",
        );
    }
    match utility.kind {
        UtilityKind::Forwarder => {
            let ret = match utility.forward_return.expect("forwarder return validated") {
                ForwardReturn::Str => "&'static str",
                ForwardReturn::Bool => "bool",
            };
            let call = utility
                .forward_call
                .as_deref()
                .expect("forwarder call validated");
            writeln!(out, "fn {}(code: i32) -> {ret} {{", utility.name).unwrap();
            writeln!(out, "    {call}(code)").unwrap();
            out.push_str("}\n");
        }
        UtilityKind::CalendarStatusName => {
            writeln!(out, "fn {}(code: i32) -> &'static str {{", utility.name).unwrap();
            out.push_str("    thetadatadx::CalendarStatus::from_code(code)\n");
            out.push_str("        .map_or(\"UNKNOWN\", thetadatadx::CalendarStatus::as_str)\n");
            out.push_str("}\n");
        }
        UtilityKind::TimestampMs => {
            writeln!(
                out,
                "fn {}(date: i32, ms_of_day: i32) -> Option<i64> {{",
                utility.name
            )
            .unwrap();
            out.push_str("    thetadatadx::time::date_ms_to_epoch_ms(date, ms_of_day)\n");
            out.push_str("}\n");
        }
        UtilityKind::SequenceSignedToUnsigned => {
            writeln!(
                out,
                "fn {}(signed_value: i64) -> PyResult<u64> {{",
                utility.name
            )
            .unwrap();
            out.push_str(
                "    if !(thetadatadx::utils::sequences::SEQUENCE_MIN..=thetadatadx::utils::sequences::SEQUENCE_MAX)\n",
            );
            out.push_str("        .contains(&signed_value)\n");
            out.push_str("    {\n");
            out.push_str("        return Err(PyValueError::new_err(format!(\n");
            out.push_str(
                "            \"sequence_signed_to_unsigned: {signed_value} is outside the i32 wire range \\\n",
            );
            out.push_str("             (-2_147_483_648 ..= 2_147_483_647)\"\n");
            out.push_str("        )));\n");
            out.push_str("    }\n");
            out.push_str("    Ok(thetadatadx::utils::sequences::signed_to_unsigned(\n");
            out.push_str("        signed_value,\n");
            out.push_str("    ))\n");
            out.push_str("}\n");
        }
        UtilityKind::SequenceUnsignedToSigned => {
            writeln!(
                out,
                "fn {}(unsigned_value: u64) -> PyResult<i64> {{",
                utility.name
            )
            .unwrap();
            out.push_str("    if unsigned_value > u64::from(u32::MAX) {\n");
            out.push_str("        return Err(PyValueError::new_err(format!(\n");
            out.push_str(
                "            \"sequence_unsigned_to_signed: {unsigned_value} is above the unsigned wire range \\\n",
            );
            out.push_str("             (0 ..= 2^32 - 1)\"\n");
            out.push_str("        )));\n");
            out.push_str("    }\n");
            out.push_str("    Ok(thetadatadx::utils::sequences::unsigned_to_signed(\n");
            out.push_str("        unsigned_value,\n");
            out.push_str("    ))\n");
            out.push_str("}\n");
        }
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
                "    let g = thetadatadx::greeks::all_greeks({}).map_err(thetadatadx::Error::from).map_err(to_py_err)?;",
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
                // The pyclass field carries the Python keyword escape
                // (`lambda_`); the `GreeksResult` member it reads stays
                // the bare Rust name (`lambda`).
                writeln!(
                    out,
                    "        {}: g.{rust_field},",
                    python_field_ident(field)
                )
                .unwrap();
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
                "    thetadatadx::greeks::implied_volatility({}).map_err(thetadatadx::Error::from).map_err(to_py_err)",
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
