//! TypeScript (napi-rs) streaming methods for `sdks/typescript/src/_generated/streaming_methods.rs`
//! and offline utility free functions for `sdks/typescript/src/_generated/utility_functions.rs`.

use std::fmt::Write as _;

use heck::ToLowerCamelCase;

use super::common::{generated_header, greek_result_fields, push_rust_doc_comment};
use super::spec::{
    assert_forwarder_code_params, ForwardReturn, MethodKind, MethodSpec, UtilityKind, UtilitySpec,
};

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
                "Start streaming and register a JS callback for incoming events.\n\
                 \n\
                 Each typed streaming event is delivered to your\n\
                 `callback(event)` on the Node main thread, so the\n\
                 callback may use any JS API safely. A callback that\n\
                 panics or throws is isolated and does not interrupt\n\
                 the stream.\n\
                 \n\
                 Backpressure: a slow callback first fills a bounded\n\
                 delivery queue and then the event ring behind it, at\n\
                 which point the oldest events are dropped and counted by\n\
                 `droppedEventCount()` while `ringOccupancy()` reports the\n\
                 in-flight depth. Watch those two signals to detect a\n\
                 callback that cannot keep up. The receive path is never\n\
                 blocked by a slow callback, so the upstream connection\n\
                 stays healthy regardless of callback speed.",
            );
            writeln!(out, "    #[napi(js_name = \"startStreaming\")]").unwrap();
            // The callback handle owns a JS function reference and
            // marshals each call onto the Node main thread. We register
            // a Rust closure with the dispatcher; the closure clones the
            // handle (a cheap reference bump) and invokes it with the
            // typed event. The method is `async`: the FPSS connect and
            // authentication handshake run on a blocking worker via
            // `spawn_blocking` so the Node event loop is never frozen for
            // the handshake. napi-rs maps the `napi::Result<()>` on an
            // `async fn` to a `Promise<void>`.
            // The callback parameter is spelled with the inline
            // `ThreadsafeFunction<StreamEvent, …>` rather than the
            // `TsfnCallback` alias so napi-rs emits a typed
            // `(event: StreamEvent) => void` signature into `index.d.ts`. The
            // const generics match `TsfnCallback` exactly so the value
            // coerces into `Arc<TsfnCallback>` in the body; the seventh,
            // `STREAMING_CALLBACK_QUEUE_DEPTH`, bounds the call queue so the
            // `Blocking` mode on the dispatcher applies real back-pressure
            // instead of letting a slow callback grow the queue without limit.
            writeln!(
                out,
                "    pub async fn {}(&self, callback: napi::threadsafe_function::ThreadsafeFunction<StreamEvent, (), StreamEvent, napi::Status, false, false, {{ crate::STREAMING_CALLBACK_QUEUE_DEPTH }}>) -> napi::Result<()> {{",
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
        MethodKind::Batches => {
            // Thin entry: forward the optional tuning knobs to the
            // hand-written `RecordBatchStreamHandle` constructor. The
            // package's JS wrapper decodes the handle's Arrow IPC buffers
            // with apache-arrow and presents the `AsyncIterable<RecordBatch>`
            // + `Symbol.asyncDispose` surface; only this entry is generated
            // so the cross-binding surface stays in lockstep. `async`
            // because the FPSS connect runs on a blocking worker.
            push_rust_doc_comment(
                &mut out,
                "    ",
                "Open a pull-based columnar reader over the live stream.\n\
                 \n\
                 Returns a reader handle — a sibling to the per-event\n\
                 `startStreaming(callback)`. The same subscriptions feed it,\n\
                 but market-data events arrive as apache-arrow `RecordBatch`\n\
                 values under a fixed schema, consumed with `for await`. The\n\
                 reader closes (unsubscribes + tears down) on `close()` or\n\
                 `Symbol.asyncDispose`. Subscribe on this same surface first,\n\
                 then open the reader.\n\
                 \n\
                 `batchSize` rows per batch (default 65536); `lingerMs`\n\
                 flushes a partial batch on a quiet stream (default 50);\n\
                 `backpressure` is `\"block\"` (default, lossless) or\n\
                 `\"dropOldest\"`; `capacity` bounds the drop-oldest buffer.",
            );
            writeln!(out, "    #[napi(js_name = \"batches\")]").unwrap();
            writeln!(
                out,
                "    pub async fn {}(&self, options: Option<crate::streaming_batches::BatchesOptions>) -> napi::Result<crate::streaming_batches::RecordBatchStreamHandle> {{",
                method.name
            )
            .unwrap();
            out.push_str("        let options = options.unwrap_or_default();\n");
            out.push_str(
                "        crate::streaming_batches::open_handle(std::sync::Arc::clone(&self.client), options.batch_size, options.linger_ms, options.backpressure, options.capacity).await\n",
            );
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
        MethodKind::Reconnect => {
            push_rust_doc_comment(
                &mut out,
                "    ",
                "Reconnect streaming and re-register the previously installed callback.\n\
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
            // `async`: the reconnect re-runs the FPSS connect and
            // authentication handshake plus the paced subscription
            // restore, all of which are network-bound. Running them on a
            // blocking worker via `spawn_blocking` keeps the Node event
            // loop free; napi-rs maps the `napi::Result<()>` to a
            // `Promise<void>`.
            writeln!(
                out,
                "    pub async fn {}(&self) -> napi::Result<()> {{",
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
    name.to_lower_camel_case()
}

/// Emit one `Util` static method (4-space indent inside the impl block).
/// Forwarders share a single shape; the four special helpers carry
/// TypeScript-specific docs and bodies (BigInt at the JS boundary).
fn ts_util_method(utility: &UtilitySpec) -> String {
    let mut out = String::new();
    let js_name = to_ts_camel_case(&utility.name);
    match utility.kind {
        UtilityKind::Forwarder => {
            assert_forwarder_code_params(utility);
            push_rust_doc_comment(&mut out, "    ", &utility.doc);
            writeln!(out, "    #[napi(js_name = \"{js_name}\")]").unwrap();
            let call = utility
                .forward_call
                .as_deref()
                .expect("forwarder call validated");
            match utility.forward_return.expect("forwarder return validated") {
                ForwardReturn::Str => {
                    writeln!(out, "    pub fn {}(code: i32) -> String {{", utility.name).unwrap();
                    writeln!(out, "        {call}(code).to_string()").unwrap();
                }
                ForwardReturn::Bool => {
                    writeln!(out, "    pub fn {}(code: i32) -> bool {{", utility.name).unwrap();
                    writeln!(out, "        {call}(code)").unwrap();
                }
            }
            out.push_str("    }\n");
        }
        UtilityKind::CalendarStatusName => {
            push_rust_doc_comment(
                &mut out,
                "    ",
                "Vendor vocabulary text for a calendar-day `status` code (`0` ->\n\
                 `\"open\"`, `1` -> `\"early_close\"`, `2` -> `\"full_close\"`, `3` ->\n\
                 `\"weekend\"`). Returns the literal `\"UNKNOWN\"` for codes outside\n\
                 the table.",
            );
            writeln!(out, "    #[napi(js_name = \"{js_name}\")]").unwrap();
            writeln!(out, "    pub fn {}(code: i32) -> String {{", utility.name).unwrap();
            out.push_str("        thetadatadx::CalendarStatus::from_code(code)\n");
            out.push_str("            .map_or(\"UNKNOWN\", thetadatadx::CalendarStatus::as_str)\n");
            out.push_str("            .to_string()\n");
            out.push_str("    }\n");
        }
        UtilityKind::TimestampMs => {
            push_rust_doc_comment(
                &mut out,
                "    ",
                "Combine an Eastern-Time `YYYYMMDD` date and milliseconds-of-day\n\
                 into Unix epoch milliseconds (UTC, DST-aware) as a JS BigInt.\n\
                 Usable with any `(date, *_ms_of_day)` pair on the tick structs.\n\
                 Returns `null` when `date` is absent (`0`) or either input is out\n\
                 of domain. BigInt matches the `*TimestampMs` tick accessors so the\n\
                 epoch domain is uniform.",
            );
            writeln!(out, "    #[napi(js_name = \"{js_name}\")]").unwrap();
            writeln!(
                out,
                "    pub fn {}(date: i32, ms_of_day: i32) -> Option<napi::bindgen_prelude::BigInt> {{",
                utility.name
            )
            .unwrap();
            out.push_str(
                "        thetadatadx::time::date_ms_to_epoch_ms(date, ms_of_day).map(napi::bindgen_prelude::BigInt::from)\n",
            );
            out.push_str("    }\n");
        }
        UtilityKind::SequenceSignedToUnsigned => {
            push_rust_doc_comment(
                &mut out,
                "    ",
                "Convert a signed wire-encoded trade-sequence value to its unsigned\n\
                 monotonic form. Accepts a JS BigInt in the **32-bit signed wire\n\
                 range** (`-2_147_483_648 ..= 2_147_483_647`) — the upstream feed\n\
                 encodes trade sequences as a 32-bit signed integer. Returns a JS\n\
                 BigInt because the unsigned monotonic sequence id can exceed\n\
                 `Number.MAX_SAFE_INTEGER`. Inputs outside the wire range throw so\n\
                 silent coercion cannot produce a look-correct-but-wrong sequence id\n\
                 downstream.",
            );
            writeln!(out, "    #[napi(js_name = \"{js_name}\")]").unwrap();
            writeln!(
                out,
                "    pub fn {}(signed_value: napi::bindgen_prelude::BigInt) -> napi::Result<napi::bindgen_prelude::BigInt> {{",
                utility.name
            )
            .unwrap();
            out.push_str(
                "        let signed: i64 = bigint_to_i32(&signed_value).map(i64::from).ok_or_else(|| {\n",
            );
            out.push_str("            crate::invalid_parameter_err(\n");
            out.push_str(
                "                \"sequenceSignedToUnsigned: BigInt outside the i32 wire range \\\n",
            );
            out.push_str("                 (-2_147_483_648 ..= 2_147_483_647)\",\n");
            out.push_str("            )\n");
            out.push_str("        })?;\n");
            out.push_str("        Ok(napi::bindgen_prelude::BigInt::from(\n");
            out.push_str(
                "            thetadatadx::utils::sequences::signed_to_unsigned(signed),\n",
            );
            out.push_str("        ))\n");
            out.push_str("    }\n");
        }
        UtilityKind::SequenceUnsignedToSigned => {
            push_rust_doc_comment(
                &mut out,
                "    ",
                "Convert an unsigned monotonic trade-sequence value back to its\n\
                 signed wire encoding. Accepts a JS BigInt in the unsigned wire\n\
                 range (`0 ..= 2^32 - 1`); returns a JS BigInt for symmetry with\n\
                 `sequenceSignedToUnsigned`. Negative inputs and inputs above the\n\
                 wire range throw — the unsigned monotonic sequence id is always\n\
                 non-negative and never wider than the 32-bit wire range.",
            );
            writeln!(out, "    #[napi(js_name = \"{js_name}\")]").unwrap();
            writeln!(
                out,
                "    pub fn {}(unsigned_value: napi::bindgen_prelude::BigInt) -> napi::Result<napi::bindgen_prelude::BigInt> {{",
                utility.name
            )
            .unwrap();
            out.push_str(
                "        if unsigned_value.sign_bit && !unsigned_value.words.iter().all(|w| *w == 0) {\n",
            );
            out.push_str("            return Err(crate::invalid_parameter_err(\n");
            out.push_str(
                "                \"sequenceUnsignedToSigned: negative BigInt rejected; the unsigned \\\n",
            );
            out.push_str("                 monotonic sequence id is always non-negative\",\n");
            out.push_str("            ));\n");
            out.push_str("        }\n");
            out.push_str("        if unsigned_value.words.len() > 1 {\n");
            out.push_str("            return Err(crate::invalid_parameter_err(\n");
            out.push_str(
                "                \"sequenceUnsignedToSigned: BigInt above the wire range \\\n",
            );
            out.push_str("                 (0 ..= 2^32 - 1)\",\n");
            out.push_str("            ));\n");
            out.push_str("        }\n");
            out.push_str(
                "        let value = unsigned_value.words.first().copied().unwrap_or(0);\n",
            );
            out.push_str("        if value > u32::MAX as u64 {\n");
            out.push_str("            return Err(crate::invalid_parameter_err(\n");
            out.push_str(
                "                \"sequenceUnsignedToSigned: BigInt above the wire range \\\n",
            );
            out.push_str("                 (0 ..= 2^32 - 1)\",\n");
            out.push_str("            ));\n");
            out.push_str("        }\n");
            out.push_str("        Ok(napi::bindgen_prelude::BigInt::from(\n");
            out.push_str("            thetadatadx::utils::sequences::unsigned_to_signed(value),\n");
            out.push_str("        ))\n");
            out.push_str("    }\n");
        }
        other => panic!("unsupported TypeScript Util method kind: {other:?}"),
    }
    out
}

/// Whether a utility kind is exposed as a `Util` static method (the
/// lookup-table helpers) rather than a top-level napi free function (the
/// offline Greeks calculators).
fn is_util_class_kind(kind: UtilityKind) -> bool {
    matches!(
        kind,
        UtilityKind::Forwarder
            | UtilityKind::CalendarStatusName
            | UtilityKind::TimestampMs
            | UtilityKind::SequenceSignedToUnsigned
            | UtilityKind::SequenceUnsignedToSigned
    )
}

/// Renders the TypeScript utility functions source: the `AllGreeks` napi object, the offline utility free functions, and the `Util` lookup-table class.
pub(super) fn render_ts_utility_functions(utilities: &[&UtilitySpec]) -> String {
    let mut out = String::new();
    out.push_str(generated_header());

    let free: Vec<&&UtilitySpec> = utilities
        .iter()
        .filter(|u| !is_util_class_kind(u.kind))
        .collect();
    let class: Vec<&&UtilitySpec> = utilities
        .iter()
        .filter(|u| is_util_class_kind(u.kind))
        .collect();

    // Emit the typed `AllGreeks` napi object before any function that
    // returns it, mirroring the Python typed-pyclass policy. napi-rs
    // lowers the `#[napi(object)]` struct to a TypeScript interface in
    // `index.d.ts`, so `allGreeks(...)` returns a concrete object type —
    // never `any` or a loose record.
    let has_all_greeks = free
        .iter()
        .any(|u| matches!(u.kind, UtilityKind::AllGreeks));
    if has_all_greeks {
        out.push_str(&render_all_greeks_napi_object());
        out.push('\n');
    }

    for utility in &free {
        out.push_str(&ts_utility_function(utility));
        out.push('\n');
    }

    if !class.is_empty() {
        out.push_str(&render_util_napi_class(&class));
    }
    out
}

/// Emit the `Util` napi class: a unit struct plus a `#[napi] impl` block
/// of static lookup-table methods, then the `bigint_to_i32` BigInt
/// decoder the sequence converters use. The class mirrors the Python
/// `thetadatadx.util` submodule one-for-one under camelCase JS names.
fn render_util_napi_class(utilities: &[&&UtilitySpec]) -> String {
    let mut out = String::new();
    out.push_str(
        "/// Cross-language lookup-table namespace. Exposes the static condition,\n\
         /// exchange, calendar, timestamp, and sequence helpers as `Util.*` static\n\
         /// methods so the JS surface mirrors the Python / C++ / C ABI utility sets.\n",
    );
    out.push_str("#[napi(js_name = \"Util\")]\n");
    out.push_str("pub struct Util;\n\n");
    out.push_str("#[napi]\n");
    out.push_str("impl Util {\n");
    for utility in utilities {
        out.push_str(&ts_util_method(utility));
        out.push('\n');
    }
    out.push_str("}\n\n");
    out.push_str(include_str!("templates/typescript/bigint_to_i32.rs.tmpl"));
    out
}

/// Emit the `AllGreeks` `#[napi(object)]` struct so `allGreeks(...)`
/// returns a typed object whose fields mirror
/// `thetadatadx::greeks::GreeksResult` 1:1. napi-rs camelCases each
/// snake_case Rust field into the JS property name (`dual_delta` ->
/// `dualDelta`); the field name is emitted unchanged because object keys
/// admit reserved words (`lambda` stays `lambda`, matching the
/// `GreeksAllTick` tick object).
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
        writeln!(out, "    pub {field}: f64,").unwrap();
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
                writeln!(out, "        {field}: g.{rust_field},").unwrap();
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
        ParamType::I64 => "i64",
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
