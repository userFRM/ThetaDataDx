//! Python type-stub emitter for the market-data endpoint surface.
//!
//! Renders the region of `thetadatadx-py/python/thetadatadx/__init__.pyi`
//! that `thetadatadx-py/src/market_data.rs` expands from the same endpoint
//! table: `StringList`, one builder class per endpoint, and `MarketDataView`
//! with every endpoint's sync, `*_async` and `*_builder` method, so
//! `client.market_data.<endpoint>(...)`, its companions and the builder chain
//! type-check under mypy / pyright and `mypy.stubtest` verifies them against
//! the runtime.
//!
//! The signatures mirror the `#[pyo3(signature = ...)]` on each generated
//! pymethod exactly: required parameters stay positional, every optional
//! kwarg and `timeout_ms` is keyword-only (after `*`). Return types follow
//! the three shapes the runtime produces: the chainable `<Tick>List` wrapper
//! for parsed lists, a `StringList` for list-of-string endpoints, and a plain
//! `List[<row>]` for the snapshot fast path.

use std::fmt::Write as _;

use super::super::helpers::{
    compose_endpoint_doc, endpoint_streams, is_simple_list_endpoint, is_streaming_endpoint,
    to_pascal_case,
};
use super::super::sdk_helpers::{
    builder_params, method_params, python_pyclass_list_class, python_pyclass_row_class,
    sdk_method_arg_name, snapshot_returns_plain_pylist,
};
use super::super::table::{ENUMS, GeneratedEndpoint, GeneratedParam};

/// Marker comments bounding the generated module-scope region. Everything
/// between the two markers is owned by this emitter; the rest of
/// `__init__.pyi` stays hand-maintained.
pub(super) const STUB_BEGIN_MARKER: &str =
    "# --- BEGIN GENERATED MARKET DATA VIEW (endpoint table) ---";
pub(super) const STUB_END_MARKER: &str = "# --- END GENERATED MARKET DATA VIEW ---";

/// `StringList` and the terminals every builder has, declared once: the
/// builders derive from `_Builder` or `_ChunkedBuilder`, which do not exist
/// at runtime.
const SHARED_CLASSES: &str = "
@final
class StringList:
    \"\"\"The strings a list endpoint returns. Supports `len()`, indexing, slicing and
    iteration, and converts with `.to_list()`, `.to_arrow()`, `.to_pandas()` and
    `.to_polars()` into one column named for what the list holds.
    \"\"\"

    def __len__(self) -> int: ...
    def __bool__(self) -> bool: ...
    def __repr__(self) -> str: ...
    @overload
    def __getitem__(self, key: int) -> str: ...
    @overload
    def __getitem__(self, key: slice) -> StringList: ...
    def __iter__(self) -> Iterator[str]: ...
    def to_list(self) -> List[str]:
        \"\"\"Return a plain Python `list` of the strings.\"\"\"
        ...

    def to_arrow(self) -> Any:
        \"\"\"Return a one-column `pyarrow.Table` of the strings.\"\"\"
        ...

    def to_pandas(self) -> Any:
        \"\"\"Return a one-column `pandas.DataFrame` of the strings. Requires pandas and pyarrow.\"\"\"
        ...

    def to_polars(self) -> Any:
        \"\"\"Return a one-column `polars.DataFrame` of the strings. Requires polars and pyarrow.\"\"\"
        ...


_Result = TypeVar(\"_Result\")


class _Builder(Generic[_Result]):
    \"\"\"Chain the setters, then run the request with `.list()` or `.list_async()`.\"\"\"

    def timeout_ms(self, value: int) -> Self:
        \"\"\"Apply a per-request deadline in milliseconds.\"\"\"
        ...

    def list(self) -> _Result:
        \"\"\"Run the request and return its result.\"\"\"
        ...

    def list_async(self) -> Awaitable[_Result]:
        \"\"\"Awaitable companion of `list()`.\"\"\"
        ...


class _ChunkedBuilder(_Builder[_Result]):
    \"\"\"A builder whose response can also be streamed chunk by chunk.\"\"\"

    def stream(self, handler: Callable[[_Result], object]) -> None:
        \"\"\"Call `handler` once per response chunk, with the chunk's rows in the typed
        list a whole result returns, without holding the full response in memory.
        An exception raised by `handler` stops the stream and is raised from this call.
        \"\"\"
        ...

    def stream_async(self, handler: Callable[[_Result], object]) -> Awaitable[None]:
        \"\"\"Awaitable companion of `stream()`, resolving to `None` once the response
        finishes. `handler` runs on a worker thread, one chunk at a time; cancelling
        the awaitable fetches no further chunks.
        \"\"\"
        ...
";

/// Render the generated region (markers included): `StringList`, the builder
/// classes, the awaitable methods `MarketDataView` shares with `AsyncClient`,
/// and `MarketDataView` itself.
pub(super) fn render_python_market_data_view_stub(endpoints: &[GeneratedEndpoint]) -> String {
    let endpoints: Vec<&GeneratedEndpoint> = endpoints
        .iter()
        .filter(|endpoint| !is_streaming_endpoint(endpoint))
        .collect();
    let mut out = String::new();
    out.push_str(STUB_BEGIN_MARKER);
    out.push('\n');
    out.push_str(
        "# Generated from the endpoint table; do not edit by hand. Run\n\
         # `cargo run -p xtask --bin generate_sdk_surfaces` to refresh.\n",
    );
    out.push_str(SHARED_CLASSES);

    for endpoint in &endpoints {
        out.push_str("\n\n");
        out.push_str(&render_builder_class(endpoint));
    }

    out.push_str(
        "\n\n\
         class _AsyncMarketData:\n\
         \x20   \"\"\"The awaitable market-data methods of :class:`MarketDataView`, which\n\
         \x20   :class:`AsyncClient` forwards. It does not exist at runtime.\n\
         \x20   \"\"\"\n",
    );
    for endpoint in &endpoints {
        out.push('\n');
        out.push_str(&render_stub_async(endpoint));
        if endpoint.subcategory == "flat_file" {
            out.push('\n');
            out.push_str(&render_stub_to_path(endpoint, true));
        }
    }

    out.push_str("\n\n@final\n");
    out.push_str("class MarketDataView(_AsyncMarketData):\n");
    out.push_str(
        "    \"\"\"Market-data sub-namespace returned by :attr:`Client.market_data`.\n\
         \n\
         \x20   Exposes every historical / list / snapshot / at-time endpoint as a\n\
         \x20   method: the synchronous call, its ``*_async`` awaitable companion,\n\
         \x20   and a ``*_builder`` fluent constructor. The surface is projected\n\
         \x20   from the same endpoint definition that drives the runtime methods,\n\
         \x20   so the stubs cannot drift from the installed extension.\n\
         \x20   \"\"\"\n",
    );
    for endpoint in &endpoints {
        out.push('\n');
        out.push_str(&render_stub_sync(endpoint));
        out.push('\n');
        out.push_str(&render_stub_builder(endpoint));
        if endpoint.subcategory == "flat_file" {
            out.push('\n');
            out.push_str(&render_stub_to_path(endpoint, false));
        }
    }

    out.push('\n');
    out.push_str(STUB_END_MARKER);
    out
}

/// One endpoint's builder class: a setter per parameter it can change, over
/// the shared terminals. A list endpoint's builder sets only its required
/// parameters, as the runtime's does.
fn render_builder_class(endpoint: &GeneratedEndpoint) -> String {
    let base = if endpoint_streams(endpoint) {
        "_ChunkedBuilder"
    } else {
        "_Builder"
    };
    let result = if is_simple_list_endpoint(endpoint) {
        "StringList".to_string()
    } else {
        python_pyclass_list_class(endpoint.return_type)
    };
    let mut out = String::new();
    writeln!(
        out,
        "@final\nclass {}({base}[{result}]):",
        endpoint_builder_class(endpoint.name)
    )
    .unwrap();
    writeln!(
        out,
        "    \"\"\"Fluent builder for :meth:`MarketDataView.{}`.\"\"\"",
        endpoint.name
    )
    .unwrap();
    for param in endpoint
        .params
        .iter()
        .filter(|param| param.required || !is_simple_list_endpoint(endpoint))
    {
        // The stub imports `date` as `_date` too: a `date` setter shadows
        // `date` for the setters after it.
        let value_type = match param.param_type {
            "Date" | "Expiration" => "Union[str, _date, datetime]".to_string(),
            _ => stub_value_type(param),
        };
        writeln!(
            out,
            "\n    def {}(self, value: {value_type}) -> Self:",
            sdk_method_arg_name(param),
        )
        .unwrap();
        write_docstring(&mut out, param.description);
    }
    out
}

// ───────────────────────── Per-variant stub bodies ─────────────────────────

fn render_stub_sync(endpoint: &GeneratedEndpoint) -> String {
    let mut out = String::new();
    let return_type = stub_return_type(endpoint);
    write_signature(
        &mut out,
        endpoint.name,
        endpoint,
        &return_type,
        SignatureKind::SyncMethod,
    );
    write_docstring(&mut out, &compose_endpoint_doc(endpoint));
    out
}

fn render_stub_async(endpoint: &GeneratedEndpoint) -> String {
    let mut out = String::new();
    let return_type = format!("Awaitable[{}]", stub_return_type(endpoint));
    let mut doc = compose_endpoint_doc(endpoint);
    doc.push_str(
        "\n\nAwaitable companion of the sync variant. The returned object \
         resolves the request off the calling thread so a running event \
         loop keeps servicing other coroutines.",
    );
    write_signature(
        &mut out,
        &format!("{}_async", endpoint.name),
        endpoint,
        &return_type,
        SignatureKind::AsyncMethod,
    );
    write_docstring(&mut out, &doc);
    out
}

fn render_stub_builder(endpoint: &GeneratedEndpoint) -> String {
    let mut out = String::new();
    let return_type = endpoint_builder_class(endpoint.name);
    let doc = format!(
        "Fluent builder for `{name}`. Chain the optional setters, then \
         call `.list()` (or `.list_async()`) to execute; the returned \
         typed list wrapper exposes `.to_list()` / `.to_arrow()` / \
         `.to_pandas()` / `.to_polars()`.",
        name = endpoint.name
    );
    write_signature(
        &mut out,
        &format!("{}_builder", endpoint.name),
        endpoint,
        &return_type,
        SignatureKind::Builder,
    );
    write_docstring(&mut out, &doc);
    out
}

/// A flat file's `<endpoint>_to_path` method, or its `_async` companion.
fn render_stub_to_path(endpoint: &GeneratedEndpoint, awaitable: bool) -> String {
    let mut doc = format!(
        "Write the `{}` file for `date` to `path` as the terminal writes it, \
         and return the path written.\n\n``format`` is ``\"csv\"`` (the default), \
         ``\"json\"``, ``\"json_legacy\"``, ``\"ndjson\"`` (or ``\"jsonl\"``) or \
         ``\"html\"``. The file streams to disk chunk by chunk and takes its name \
         only once complete; a path without an extension gets the format's.",
        endpoint.name
    );
    let (suffix, return_type) = if awaitable {
        doc.push_str(
            "\n\nAwaitable companion of the sync variant. The returned object \
             resolves to the path written, so a running event loop keeps servicing \
             other coroutines while the file streams to disk.",
        );
        ("_async", "Awaitable[str]")
    } else {
        ("", "str")
    };
    let mut out = String::new();
    writeln!(out, "    def {}_to_path{suffix}(", endpoint.name).unwrap();
    out.push_str(
        "        self,\n        date: Union[str, date, datetime],\n        \
         path: Union[str, PathLike[str]],\n        format: Optional[str] = None,\n",
    );
    writeln!(out, "    ) -> {return_type}:").unwrap();
    write_docstring(&mut out, &doc);
    out
}

// ───────────────────────── Signature writer ────────────────────────────────

enum SignatureKind {
    /// Sync endpoint method: required positional args, then a keyword-only
    /// block of optional builder kwargs and `timeout_ms`. Scalar kwargs
    /// carry the upstream default so the stub matches the runtime
    /// `#[pyo3(signature = ...)]` exactly.
    SyncMethod,
    /// Async endpoint method: same shape as the sync variant, but every
    /// kwarg defaults to `None`, as the generated `*_async` signature does;
    /// an omitted kwarg leaves the upstream default to the server.
    AsyncMethod,
    /// `*_builder` constructor: required positional args only.
    Builder,
}

fn write_signature(
    out: &mut String,
    method_name: &str,
    endpoint: &GeneratedEndpoint,
    return_type: &str,
    kind: SignatureKind,
) {
    let method_params = method_params(endpoint);
    // A list endpoint's optionals are keyword arguments like any other
    // endpoint's, so the stub advertises them.
    let builder_params = builder_params(endpoint);

    let positional: Vec<String> = method_params
        .iter()
        .map(|param| format!("{}: {}", sdk_method_arg_name(param), stub_value_type(param)))
        .collect();

    let honor_defaults = matches!(kind, SignatureKind::SyncMethod);
    let keyword_only: Vec<String> = match kind {
        SignatureKind::Builder => Vec::new(),
        SignatureKind::SyncMethod | SignatureKind::AsyncMethod => {
            let mut kwargs: Vec<String> = builder_params
                .iter()
                .map(|param| {
                    format!(
                        "{}: Optional[{}] = {}",
                        param.name,
                        stub_value_type(param),
                        stub_kwarg_default(param, honor_defaults)
                    )
                })
                .collect();
            kwargs.push("timeout_ms: Optional[int] = None".to_string());
            kwargs
        }
    };

    // Single-line form when the whole signature fits comfortably; the
    // hand-written stub keeps short methods on one line, so match that.
    let inline = positional.is_empty() && keyword_only.len() <= 1;
    if inline && keyword_only.is_empty() {
        writeln!(out, "    def {method_name}(self) -> {return_type}:").unwrap();
        return;
    }

    writeln!(out, "    def {method_name}(").unwrap();
    out.push_str("        self,\n");
    for param in &positional {
        writeln!(out, "        {param},").unwrap();
    }
    if !keyword_only.is_empty() {
        out.push_str("        *,\n");
        for kwarg in &keyword_only {
            writeln!(out, "        {kwarg},").unwrap();
        }
    }
    writeln!(out, "    ) -> {return_type}:").unwrap();
}

fn write_docstring(out: &mut String, doc: &str) {
    let lines: Vec<&str> = doc.lines().collect();
    if lines.len() == 1 {
        writeln!(out, "        \"\"\"{}\"\"\"", lines[0]).unwrap();
        out.push_str("        ...\n");
        return;
    }
    out.push_str("        \"\"\"");
    for (idx, line) in lines.iter().enumerate() {
        if idx == 0 {
            writeln!(out, "{line}").unwrap();
        } else if line.is_empty() {
            out.push('\n');
        } else {
            writeln!(out, "        {line}").unwrap();
        }
    }
    out.push_str("        \"\"\"\n");
    out.push_str("        ...\n");
}

// ───────────────────────── Type tables ─────────────────────────────────────

/// Python type of an endpoint argument, wherever it is passed: as a method
/// argument, a keyword (wrapped in `Optional[...]` by the caller) or a builder
/// setter's value. Mirrors the runtime coercion in `coerce.rs`: a multi-symbol
/// argument takes a single symbol or a sequence, a date or time argument the
/// matching `datetime` objects as well as the wire string, and an enum
/// argument its enum class as well as the wire string.
fn stub_value_type(param: &GeneratedParam) -> String {
    match param.param_type {
        "Symbols" => "Union[str, Sequence[str]]".into(),
        "Int" => "int".into(),
        "Float" => "float".into(),
        "Bool" => "bool".into(),
        "Date" | "Expiration" => "Union[str, date, datetime]".into(),
        "Time" => "Union[str, time, datetime]".into(),
        name if ENUMS.iter().any(|spec| spec.name == name) => format!("Union[str, {name}]"),
        _ => "str".into(),
    }
}

/// Return-type annotation for the sync endpoint method. The async variant
/// wraps this in `Awaitable[...]`. Matches the runtime shapes: `StringList`,
/// the plain-list snapshot fast path, and the chainable `<Tick>List` wrapper.
fn stub_return_type(endpoint: &GeneratedEndpoint) -> String {
    if endpoint.return_type == "StringList" {
        return "StringList".to_string();
    }
    if snapshot_returns_plain_pylist(endpoint) {
        return format!("List[{}]", python_pyclass_row_class(endpoint.return_type));
    }
    python_pyclass_list_class(endpoint.return_type)
}

/// Name of the per-endpoint `*_builder` return class, matching the
/// `#[pyclass(name = ...)]` the runtime emitter assigns.
fn endpoint_builder_class(name: &str) -> String {
    format!("{}Builder", to_pascal_case(name))
}

/// Default expression for an optional keyword-only argument. Mirrors the
/// `@sig` arms of `py_endpoint!` in `market_data.rs`: when `honor_defaults`
/// is set (the sync variant) a scalar param surfaces its upstream default;
/// otherwise, and for every non-scalar param, the default is `None`. Bool
/// literals are rendered in Python casing.
fn stub_kwarg_default(param: &GeneratedParam, honor_defaults: bool) -> String {
    if !honor_defaults {
        return "None".to_string();
    }
    let Some(default) = param.default else {
        return "None".to_string();
    };
    match param.param_type {
        "Bool" => match default {
            "true" => "True".to_string(),
            "false" => "False".to_string(),
            other => other.to_string(),
        },
        "Int" | "Float" => default.to_string(),
        _ => "None".to_string(),
    }
}
