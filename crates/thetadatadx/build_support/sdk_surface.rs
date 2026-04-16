//! Checked-in generation for non-endpoint SDK/tool surfaces.
//!
//! `endpoint_surface.toml` remains the SSOT for validated request/response
//! endpoints. This module covers the remaining non-endpoint surfaces that still
//! need declarative checked-in generation: offline utilities, FPSS/unified
//! wrapper methods, and small public wrapper implementations.
//!
//! Implementation detail: method signatures are hardcoded in this file's match
//! tables. The TOML (`sdk_surface.toml`) controls WHICH methods exist for WHICH
//! languages, but HOW they're implemented (parameter types, return types, body)
//! is in this renderer. See the TOML header for more context.

use std::fmt::Write as _;
use std::path::Path;

use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct SdkSurfaceSpec {
    version: u32,
    #[serde(default)]
    utilities: Vec<UtilitySpec>,
    #[serde(default)]
    python_unified: Vec<String>,
    #[serde(default)]
    go_fpss: Vec<String>,
    #[serde(default)]
    cpp_fpss: Vec<String>,
    #[serde(default)]
    cpp_lifecycle: Vec<String>,
    /// Go-side FFI configuration. Holds the TLS-reader marker SSOT that
    /// drives both `inject_os_thread_pin` (build-time body rewriter) and
    /// the generated `tlsReaderMarkers` list consumed by the static-audit
    /// test in `sdks/go/timeout_pin_test.go`.
    #[serde(default)]
    go_ffi: GoFfiSpec,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct UtilitySpec {
    name: String,
    #[serde(default)]
    cli_name: Option<String>,
    targets: Vec<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct GoFfiSpec {
    #[serde(default)]
    tls_reader_markers: Vec<TlsReaderMarker>,
}

/// A substring that, when present on a Go source line, identifies an FFI
/// thread-local error read. The enclosing function must have executed
/// `runtime.LockOSThread()` + `defer runtime.UnlockOSThread()` before
/// reaching such a line. See `sdk_surface.toml` for the authoritative
/// description of each marker.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct TlsReaderMarker {
    substring: String,
    description: String,
}

struct GeneratedSourceFile {
    relative_path: &'static str,
    contents: String,
}

pub fn write_sdk_generated_files(repo_root: &Path) -> Result<(), Box<dyn std::error::Error>> {
    for file in render_sdk_generated_files(repo_root)? {
        let path = repo_root.join(file.relative_path);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(path, file.contents)?;
    }
    Ok(())
}

pub fn check_sdk_generated_files(repo_root: &Path) -> Result<(), Box<dyn std::error::Error>> {
    for file in render_sdk_generated_files(repo_root)? {
        let path = repo_root.join(file.relative_path);
        let actual = std::fs::read_to_string(&path)?;
        if actual.replace("\r\n", "\n") != file.contents {
            return Err(format!(
                "generated SDK surface '{}' is stale; run `cargo run -p thetadatadx --bin generate_sdk_surfaces` to refresh",
                file.relative_path
            )
            .into());
        }
    }
    Ok(())
}

fn render_sdk_generated_files(
    repo_root: &Path,
) -> Result<Vec<GeneratedSourceFile>, Box<dyn std::error::Error>> {
    let spec = load_sdk_surface_spec()?;
    validate_spec(&spec)?;
    let go_fpss_methods = render_go_fpss_methods(&spec.go_fpss, &spec.go_ffi.tls_reader_markers);
    let go_utility_functions =
        render_go_utility_functions(&spec.utilities, &spec.go_ffi.tls_reader_markers);

    Ok(vec![
        GeneratedSourceFile {
            relative_path: "sdks/python/src/streaming_methods.rs",
            contents: render_python_streaming_methods(&spec.python_unified),
        },
        GeneratedSourceFile {
            relative_path: "sdks/python/src/utility_functions.rs",
            contents: render_python_utility_functions(&spec.utilities),
        },
        GeneratedSourceFile {
            relative_path: "sdks/go/fpss_methods.go",
            contents: go_fpss_methods.clone(),
        },
        GeneratedSourceFile {
            relative_path: "sdks/go/utilities.go",
            contents: go_utility_functions.clone(),
        },
        GeneratedSourceFile {
            relative_path: "sdks/go/timeout_pin_generated_test.go",
            contents: render_go_timeout_pin_generated_test(
                repo_root,
                &spec.go_ffi.tls_reader_markers,
                &[
                    ("fpss_methods.go", go_fpss_methods.as_str()),
                    ("utilities.go", go_utility_functions.as_str()),
                ],
            )?,
        },
        GeneratedSourceFile {
            relative_path: "sdks/cpp/include/fpss.hpp.inc",
            contents: render_cpp_fpss_decls(&spec.cpp_fpss),
        },
        GeneratedSourceFile {
            relative_path: "sdks/cpp/src/fpss.cpp.inc",
            contents: render_cpp_fpss_defs(&spec.cpp_fpss),
        },
        GeneratedSourceFile {
            relative_path: "sdks/cpp/include/utilities.hpp.inc",
            contents: render_cpp_utility_decls(&spec.utilities),
        },
        GeneratedSourceFile {
            relative_path: "sdks/cpp/src/utilities.cpp.inc",
            contents: render_cpp_utility_defs(&spec.utilities),
        },
        GeneratedSourceFile {
            relative_path: "sdks/cpp/src/lifecycle.cpp.inc",
            contents: render_cpp_lifecycle_defs(&spec.cpp_lifecycle),
        },
        GeneratedSourceFile {
            relative_path: "tools/mcp/src/utilities.rs",
            contents: render_mcp_utilities(&spec.utilities),
        },
        GeneratedSourceFile {
            relative_path: "tools/cli/src/utilities.rs",
            contents: render_cli_utilities(&spec.utilities),
        },
    ])
}

fn load_sdk_surface_spec() -> Result<SdkSurfaceSpec, Box<dyn std::error::Error>> {
    let spec_path = "sdk_surface.toml";
    let spec_str = std::fs::read_to_string(spec_path)?;
    let spec: SdkSurfaceSpec = toml::from_str(&spec_str)?;
    println!("cargo:rerun-if-changed={spec_path}");
    Ok(spec)
}

fn validate_spec(spec: &SdkSurfaceSpec) -> Result<(), Box<dyn std::error::Error>> {
    if spec.version != 1 {
        return Err(format!("unsupported sdk_surface.toml version: {}", spec.version).into());
    }

    validate_known_names(
        "utility",
        spec.utilities.iter().map(|item| item.name.as_str()),
        &["auth", "ping", "all_greeks", "implied_volatility"],
    )?;
    validate_known_names(
        "python_unified",
        spec.python_unified.iter().map(String::as_str),
        &[
            "start_streaming",
            "is_streaming",
            "subscribe_quotes",
            "subscribe_trades",
            "subscribe_open_interest",
            "subscribe_option_quotes",
            "subscribe_option_trades",
            "subscribe_option_open_interest",
            "subscribe_full_trades",
            "subscribe_full_open_interest",
            "unsubscribe_full_trades",
            "unsubscribe_full_open_interest",
            "unsubscribe_quotes",
            "unsubscribe_trades",
            "unsubscribe_open_interest",
            "unsubscribe_option_quotes",
            "unsubscribe_option_trades",
            "unsubscribe_option_open_interest",
            "contract_map",
            "contract_lookup",
            "active_subscriptions",
            "next_event",
            "reconnect",
            "stop_streaming",
            "shutdown",
        ],
    )?;
    validate_known_names(
        "go_fpss",
        spec.go_fpss.iter().map(String::as_str),
        &[
            "subscribe_quotes",
            "subscribe_trades",
            "subscribe_open_interest",
            "subscribe_option_quotes",
            "subscribe_option_trades",
            "subscribe_option_open_interest",
            "subscribe_full_trades",
            "subscribe_full_open_interest",
            "unsubscribe_quotes",
            "unsubscribe_trades",
            "unsubscribe_open_interest",
            "unsubscribe_option_quotes",
            "unsubscribe_option_trades",
            "unsubscribe_option_open_interest",
            "unsubscribe_full_trades",
            "unsubscribe_full_open_interest",
            "is_authenticated",
            "contract_lookup",
            "contract_map",
            "active_subscriptions",
            "next_event",
            "reconnect",
            "shutdown",
        ],
    )?;
    validate_known_names(
        "cpp_fpss",
        spec.cpp_fpss.iter().map(String::as_str),
        &[
            "connect",
            "subscribe_quotes",
            "subscribe_trades",
            "subscribe_open_interest",
            "subscribe_option_quotes",
            "subscribe_option_trades",
            "subscribe_option_open_interest",
            "subscribe_full_trades",
            "subscribe_full_open_interest",
            "unsubscribe_quotes",
            "unsubscribe_trades",
            "unsubscribe_open_interest",
            "unsubscribe_option_quotes",
            "unsubscribe_option_trades",
            "unsubscribe_option_open_interest",
            "unsubscribe_full_trades",
            "unsubscribe_full_open_interest",
            "is_authenticated",
            "contract_lookup",
            "contract_map",
            "active_subscriptions",
            "next_event",
            "reconnect",
            "shutdown",
        ],
    )?;
    validate_known_names(
        "cpp_lifecycle",
        spec.cpp_lifecycle.iter().map(String::as_str),
        &[
            "credentials_from_file",
            "credentials_from_email",
            "config_production",
            "config_dev",
            "config_stage",
            "client_connect",
        ],
    )?;

    for utility in &spec.utilities {
        for target in &utility.targets {
            match target.as_str() {
                "python" | "go" | "cpp" | "mcp" | "cli" => {}
                other => {
                    return Err(format!(
                        "utility '{}' declares unknown target '{}'",
                        utility.name, other
                    )
                    .into())
                }
            }
        }
    }

    let mut seen_tls_markers = std::collections::HashSet::new();
    for marker in &spec.go_ffi.tls_reader_markers {
        if marker.substring.trim().is_empty() {
            return Err(
                "sdk_surface.toml go_ffi.tls_reader_markers contains an empty substring".into(),
            );
        }
        if !seen_tls_markers.insert(marker.substring.as_str()) {
            return Err(format!(
                "sdk_surface.toml go_ffi.tls_reader_markers contains duplicate substring '{}'",
                marker.substring
            )
            .into());
        }
    }

    Ok(())
}

fn validate_known_names<'a, I>(
    label: &str,
    values: I,
    known: &[&str],
) -> Result<(), Box<dyn std::error::Error>>
where
    I: IntoIterator<Item = &'a str>,
{
    let mut seen = std::collections::HashSet::new();
    let allowed = known
        .iter()
        .copied()
        .collect::<std::collections::HashSet<_>>();
    for value in values {
        if !allowed.contains(value) {
            return Err(format!("unknown {label} entry '{}'", value).into());
        }
        if !seen.insert(value.to_string()) {
            return Err(format!("duplicate {label} entry '{}'", value).into());
        }
    }
    Ok(())
}

fn generated_header() -> &'static str {
    "// @generated DO NOT EDIT — regenerated by build.rs from sdk_surface.toml\n\n"
}

fn render_python_streaming_methods(methods: &[String]) -> String {
    let mut out = String::new();
    out.push_str(generated_header());
    out.push_str("#[pymethods]\n");
    out.push_str("impl ThetaDataDx {\n");
    for method in methods {
        out.push_str(python_streaming_method(method));
        out.push('\n');
    }
    out.push_str("}\n");
    out
}

fn render_python_utility_functions(utilities: &[UtilitySpec]) -> String {
    let mut out = String::new();
    out.push_str(generated_header());
    for utility in utilities
        .iter()
        .filter(|utility| utility.targets.iter().any(|target| target == "python"))
    {
        out.push_str(python_utility_function(&utility.name));
        out.push('\n');
    }
    out.push_str(
        "fn register_generated_utility_functions(m: &Bound<'_, PyModule>) -> PyResult<()> {\n",
    );
    for utility in utilities
        .iter()
        .filter(|utility| utility.targets.iter().any(|target| target == "python"))
    {
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

fn render_go_fpss_methods(methods: &[String], tls_reader_markers: &[TlsReaderMarker]) -> String {
    let mut out = String::new();
    out.push_str("// Code generated by build.rs from sdk_surface.toml; DO NOT EDIT.\n\n");
    out.push_str("package thetadatadx\n\n");
    out.push_str("/*\n#include \"ffi_bridge.h\"\n*/\nimport \"C\"\n\n");
    out.push_str("import (\n\t\"fmt\"\n\t\"runtime\"\n\t\"unsafe\"\n)\n\n");
    // Every FPSS wrapper that reads the FFI's thread-local error slot
    // MUST pin its goroutine to one OS thread for the full clear/call/
    // check sequence (see `docs/dev/w3-async-cancellation-design.md`
    // "cgo thread-local correctness" and `ffi/src/lib.rs::LAST_ERROR`).
    // `inject_os_thread_pin` rewrites the bodies of methods that touch
    // the error slot so they start with `runtime.LockOSThread()` +
    // deferred unlock. Pure-read methods like `IsAuthenticated` and
    // `NextEvent` (which don't read the TLS) pass through unchanged.
    for method in methods {
        out.push_str(&inject_os_thread_pin(
            go_fpss_method(method),
            tls_reader_markers,
        ));
        out.push('\n');
    }
    out
}

/// Insert `runtime.LockOSThread()` + deferred unlock at the top of every
/// Go method body in `src` whose body reads the FFI's thread-local error
/// slot (any substring in `sdk_surface.toml`'s
/// `go_ffi.tls_reader_markers`). Methods without TLS reads pass through
/// unchanged.
fn inject_os_thread_pin(src: &str, tls_reader_markers: &[TlsReaderMarker]) -> String {
    // Work line-by-line. A new method opens with `func (<recv>) <Name>(` and
    // the opening brace is always on the same line per our templates. The
    // first blank line or non-indented line after the brace closes the body
    // — we scan until we see a cgo/TLS read marker anywhere in the body;
    // if found, the pin is injected right after the opening brace.
    let mut out = String::new();
    let lines: Vec<&str> = src.split_inclusive('\n').collect();
    let mut i = 0;
    while i < lines.len() {
        let line = lines[i];
        if line.starts_with("func ") && line.trim_end().ends_with('{') {
            // Scan ahead to the matching `}` at column 0 for this method.
            let mut j = i + 1;
            let mut touches_tls = false;
            while j < lines.len() {
                let l = lines[j];
                if l.starts_with('}') {
                    break;
                }
                if tls_reader_markers
                    .iter()
                    .any(|marker| l.contains(&marker.substring))
                {
                    touches_tls = true;
                }
                j += 1;
            }
            out.push_str(line);
            if touches_tls {
                out.push_str("    runtime.LockOSThread()\n");
                out.push_str("    defer runtime.UnlockOSThread()\n");
            }
            // Emit the rest of the body verbatim.
            for k in (i + 1)..=j.min(lines.len().saturating_sub(1)) {
                out.push_str(lines[k]);
            }
            i = j + 1;
            continue;
        }
        out.push_str(line);
        i += 1;
    }
    out
}

fn render_go_utility_functions(
    utilities: &[UtilitySpec],
    tls_reader_markers: &[TlsReaderMarker],
) -> String {
    let mut out = String::new();
    out.push_str("// Code generated by build.rs from sdk_surface.toml; DO NOT EDIT.\n\n");
    out.push_str("package thetadatadx\n\n");
    out.push_str("/*\n#include <stdlib.h>\n#include \"ffi_bridge.h\"\n*/\nimport \"C\"\n\n");
    // `unsafe` is required for `unsafe.Pointer(cRight)` when freeing the
    // C string allocated by `C.CString`. Keep the imports grouped so gofmt
    // is happy. `runtime` is emitted by `inject_os_thread_pin` for
    // utilities that read the FFI's thread-local error slot (see
    // `docs/dev/w3-async-cancellation-design.md`).
    out.push_str("import (\n\t\"fmt\"\n\t\"runtime\"\n\t\"unsafe\"\n)\n\n");
    for utility in utilities
        .iter()
        .filter(|utility| utility.targets.iter().any(|target| target == "go"))
    {
        out.push_str(&inject_os_thread_pin(
            go_utility_function(&utility.name),
            tls_reader_markers,
        ));
        out.push('\n');
    }
    out
}

fn render_go_timeout_pin_generated_test(
    repo_root: &Path,
    tls_reader_markers: &[TlsReaderMarker],
    generated_overrides: &[(&str, &str)],
) -> Result<String, Box<dyn std::error::Error>> {
    let expected_pinned_methods =
        count_go_tls_reader_methods(repo_root, tls_reader_markers, generated_overrides)?;
    let mut out = String::new();
    out.push_str("// Code generated by build.rs from sdk_surface.toml; DO NOT EDIT.\n\n");
    out.push_str("package thetadatadx\n\n");
    out.push_str("// tlsReaderMarkers is the single source of truth for the static\n");
    out.push_str("// Go TLS-reader audit in timeout_pin_test.go.\n");
    out.push_str("var tlsReaderMarkers = []string{\n");
    for marker in tls_reader_markers {
        writeln!(out, "\t{:?}, // {}", marker.substring, marker.description)?;
    }
    out.push_str("}\n\n");
    out.push_str("// expectedPinnedMethods is derived from the current non-test Go\n");
    out.push_str("// source tree: every function body that reads the FFI thread-local\n");
    out.push_str("// error slot must pin its goroutine to one OS thread.\n");
    writeln!(
        out,
        "const expectedPinnedMethods = {expected_pinned_methods}"
    )?;
    Ok(out)
}

fn count_go_tls_reader_methods(
    repo_root: &Path,
    tls_reader_markers: &[TlsReaderMarker],
    generated_overrides: &[(&str, &str)],
) -> Result<usize, Box<dyn std::error::Error>> {
    let go_dir = repo_root.join("sdks/go");
    let mut files: Vec<_> = std::fs::read_dir(&go_dir)?
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let path = entry.path();
            let file_name = path.file_name()?.to_str()?;
            if path.extension().and_then(|ext| ext.to_str()) == Some("go")
                && !file_name.ends_with("_test.go")
            {
                Some((file_name.to_string(), path))
            } else {
                None
            }
        })
        .collect();
    files.sort_by(|a, b| a.0.cmp(&b.0));

    let generated_overrides: std::collections::HashMap<&str, &str> =
        generated_overrides.iter().copied().collect();

    let mut count = 0usize;
    for (file_name, path) in files {
        let contents = if let Some(generated) = generated_overrides.get(file_name.as_str()) {
            (*generated).to_string()
        } else {
            std::fs::read_to_string(path)?
        };
        let lines: Vec<&str> = contents
            .split('\n')
            .map(|line| line.trim_end_matches('\r'))
            .collect();
        for (idx, line) in lines.iter().enumerate() {
            if !line.starts_with("func ") || !line.ends_with('{') {
                continue;
            }
            let Some(method_name) = extract_go_method_name(line) else {
                continue;
            };
            if is_go_tls_helper(method_name) {
                continue;
            }
            let body_end = find_go_method_body_end(&lines, idx + 1);
            let body = &lines[idx + 1..body_end];
            if body.iter().any(|body_line| {
                tls_reader_markers
                    .iter()
                    .any(|marker| body_line.contains(&marker.substring))
            }) {
                count += 1;
            }
        }
    }
    Ok(count)
}

fn extract_go_method_name(line: &str) -> Option<&str> {
    let rest = line.strip_prefix("func ")?;
    let rest = if rest.starts_with('(') {
        let end = rest.find(") ")?;
        &rest[end + 2..]
    } else {
        rest
    };
    let end = rest.find('(')?;
    Some(rest[..end].trim())
}

fn find_go_method_body_end(lines: &[&str], from: usize) -> usize {
    for (idx, line) in lines.iter().enumerate().skip(from) {
        if line.starts_with('}') {
            return idx;
        }
    }
    lines.len()
}

fn is_go_tls_helper(method_name: &str) -> bool {
    matches!(method_name, "lastError" | "lastErrorRaw" | "fpssCall")
}

fn render_cpp_fpss_decls(methods: &[String]) -> String {
    let mut out = String::new();
    out.push_str(
        "    // @generated DO NOT EDIT — regenerated by build.rs from sdk_surface.toml\n\n",
    );
    for method in methods {
        out.push_str(cpp_fpss_decl(method));
        if !out.ends_with('\n') {
            out.push('\n');
        }
    }
    out
}

fn render_cpp_fpss_defs(methods: &[String]) -> String {
    let mut out = String::new();
    out.push_str(generated_header());
    for method in methods {
        out.push_str(cpp_fpss_def(method));
        out.push('\n');
    }
    out
}

fn render_cpp_utility_decls(utilities: &[UtilitySpec]) -> String {
    let mut out = String::new();
    out.push_str("// @generated DO NOT EDIT — regenerated by build.rs from sdk_surface.toml\n\n");
    for utility in utilities
        .iter()
        .filter(|utility| utility.targets.iter().any(|target| target == "cpp"))
    {
        out.push_str(cpp_utility_decl(&utility.name));
        out.push('\n');
    }
    out
}

fn render_cpp_utility_defs(utilities: &[UtilitySpec]) -> String {
    let mut out = String::new();
    out.push_str(generated_header());
    for utility in utilities
        .iter()
        .filter(|utility| utility.targets.iter().any(|target| target == "cpp"))
    {
        out.push_str(cpp_utility_def(&utility.name));
        out.push('\n');
    }
    out
}

fn render_cpp_lifecycle_defs(methods: &[String]) -> String {
    let mut out = String::new();
    out.push_str(generated_header());
    for method in methods {
        out.push_str(cpp_lifecycle_def(method));
        out.push('\n');
    }
    out
}

fn render_mcp_utilities(utilities: &[UtilitySpec]) -> String {
    let mut out = String::new();
    out.push_str(generated_header());
    out.push_str("fn push_generated_utility_tool_definitions(tools: &mut Vec<Value>) {\n");
    for utility in utilities
        .iter()
        .filter(|utility| utility.targets.iter().any(|target| target == "mcp"))
    {
        out.push_str(mcp_tool_definition(&utility.name));
    }
    out.push_str("}\n\n");
    out.push_str(
        "async fn try_execute_generated_utility(\n    client: &Option<ThetaDataDx>,\n    name: &str,\n    args: &Value,\n    start_time: std::time::Instant,\n) -> Option<Result<Value, ToolError>> {\n    let _ = client;\n    macro_rules! param_or_return {\n        ($expr:expr) => {\n            match $expr {\n                Ok(value) => value,\n                Err(error) => return Some(Err(ToolError::InvalidParams(error))),\n            }\n        };\n    }\n    match name {\n",
    );
    for utility in utilities
        .iter()
        .filter(|utility| utility.targets.iter().any(|target| target == "mcp"))
    {
        out.push_str(mcp_execute_arm(&utility.name));
    }
    out.push_str("        _ => None,\n    }\n}\n");
    out
}

fn render_cli_utilities(utilities: &[UtilitySpec]) -> String {
    let mut out = String::new();
    out.push_str(generated_header());
    out.push_str("fn add_generated_utility_commands(mut app: Command) -> Command {\n");
    for utility in utilities
        .iter()
        .filter(|utility| utility.targets.iter().any(|target| target == "cli"))
    {
        out.push_str(&cli_command_builder(utility));
    }
    out.push_str("    app\n}\n\n");
    out.push_str(
        "async fn try_run_generated_utility(\n    subcommand: Option<(&str, &ArgMatches)>,\n    fmt: &OutputFormat,\n    creds_path: &str,\n) -> Result<bool, thetadatadx::Error> {\n    match subcommand {\n",
    );
    for utility in utilities
        .iter()
        .filter(|utility| utility.targets.iter().any(|target| target == "cli"))
    {
        out.push_str(&cli_dispatch_arm(utility));
    }
    out.push_str("        _ => Ok(false),\n    }\n}\n");
    out
}

fn python_streaming_method(name: &str) -> &'static str {
    match name {
        "start_streaming" => {
            r#"    /// Start FPSS streaming. Events are buffered; poll with ``next_event()``.
    fn start_streaming(&self) -> PyResult<()> {
        let (tx, rx) = std::sync::mpsc::channel::<BufferedEvent>();

        self.tdx
            .start_streaming(move |event: &fpss::FpssEvent| {
                let buffered = fpss_event_to_buffered(event);
                let _ = tx.send(buffered);
            })
            .map_err(to_py_err)?;

        if let Ok(mut guard) = self.rx.lock() {
            *guard = Some(Arc::new(Mutex::new(rx)));
        }
        Ok(())
    }
"#
        }
        "is_streaming" => {
            r#"    /// Whether the streaming connection is active.
    fn is_streaming(&self) -> bool {
        self.tdx.is_streaming()
    }
"#
        }
        "subscribe_quotes" => {
            r#"    /// Subscribe to quote data for a stock symbol.
    fn subscribe_quotes(&self, symbol: &str) -> PyResult<()> {
        let contract = fpss::protocol::Contract::stock(symbol);
        self.tdx.subscribe_quotes(&contract).map_err(to_py_err)
    }
"#
        }
        "subscribe_trades" => {
            r#"    /// Subscribe to trade data for a stock symbol.
    fn subscribe_trades(&self, symbol: &str) -> PyResult<()> {
        let contract = fpss::protocol::Contract::stock(symbol);
        self.tdx.subscribe_trades(&contract).map_err(to_py_err)
    }
"#
        }
        "subscribe_open_interest" => {
            r#"    /// Subscribe to open interest data for a stock symbol.
    fn subscribe_open_interest(&self, symbol: &str) -> PyResult<()> {
        let contract = fpss::protocol::Contract::stock(symbol);
        self.tdx
            .subscribe_open_interest(&contract)
            .map_err(to_py_err)
    }
"#
        }
        "subscribe_option_quotes" => {
            r#"    /// Subscribe to quote data for an option contract.
    fn subscribe_option_quotes(
        &self,
        symbol: &str,
        expiration: &str,
        strike: &str,
        right: &str,
    ) -> PyResult<()> {
        let contract = fpss::protocol::Contract::option(symbol, expiration, strike, right);
        self.tdx.subscribe_quotes(&contract).map_err(to_py_err)
    }
"#
        }
        "subscribe_option_trades" => {
            r#"    /// Subscribe to trade data for an option contract.
    fn subscribe_option_trades(
        &self,
        symbol: &str,
        expiration: &str,
        strike: &str,
        right: &str,
    ) -> PyResult<()> {
        let contract = fpss::protocol::Contract::option(symbol, expiration, strike, right);
        self.tdx.subscribe_trades(&contract).map_err(to_py_err)
    }
"#
        }
        "subscribe_option_open_interest" => {
            r#"    /// Subscribe to open interest data for an option contract.
    fn subscribe_option_open_interest(
        &self,
        symbol: &str,
        expiration: &str,
        strike: &str,
        right: &str,
    ) -> PyResult<()> {
        let contract = fpss::protocol::Contract::option(symbol, expiration, strike, right);
        self.tdx
            .subscribe_open_interest(&contract)
            .map_err(to_py_err)
    }
"#
        }
        "subscribe_full_trades" => {
            r#"    /// Subscribe to all trades for a security type (full trade stream).
    fn subscribe_full_trades(&self, sec_type: &str) -> PyResult<()> {
        let st = parse_sec_type(sec_type)?;
        self.tdx.subscribe_full_trades(st).map_err(to_py_err)
    }
"#
        }
        "subscribe_full_open_interest" => {
            r#"    /// Subscribe to all open interest for a security type (full OI stream).
    fn subscribe_full_open_interest(&self, sec_type: &str) -> PyResult<()> {
        let st = parse_sec_type(sec_type)?;
        self.tdx.subscribe_full_open_interest(st).map_err(to_py_err)
    }
"#
        }
        "unsubscribe_full_trades" => {
            r#"    /// Unsubscribe from all trades for a security type (full trade stream).
    fn unsubscribe_full_trades(&self, sec_type: &str) -> PyResult<()> {
        let st = parse_sec_type(sec_type)?;
        self.tdx.unsubscribe_full_trades(st).map_err(to_py_err)
    }
"#
        }
        "unsubscribe_full_open_interest" => {
            r#"    /// Unsubscribe from all open interest for a security type (full OI stream).
    fn unsubscribe_full_open_interest(&self, sec_type: &str) -> PyResult<()> {
        let st = parse_sec_type(sec_type)?;
        self.tdx
            .unsubscribe_full_open_interest(st)
            .map_err(to_py_err)
    }
"#
        }
        "unsubscribe_quotes" => {
            r#"    /// Unsubscribe from quote data for a stock symbol.
    fn unsubscribe_quotes(&self, symbol: &str) -> PyResult<()> {
        let contract = fpss::protocol::Contract::stock(symbol);
        self.tdx.unsubscribe_quotes(&contract).map_err(to_py_err)
    }
"#
        }
        "unsubscribe_trades" => {
            r#"    /// Unsubscribe from trade data for a stock symbol.
    fn unsubscribe_trades(&self, symbol: &str) -> PyResult<()> {
        let contract = fpss::protocol::Contract::stock(symbol);
        self.tdx.unsubscribe_trades(&contract).map_err(to_py_err)
    }
"#
        }
        "unsubscribe_open_interest" => {
            r#"    /// Unsubscribe from open interest data for a stock symbol.
    fn unsubscribe_open_interest(&self, symbol: &str) -> PyResult<()> {
        let contract = fpss::protocol::Contract::stock(symbol);
        self.tdx
            .unsubscribe_open_interest(&contract)
            .map_err(to_py_err)
    }
"#
        }
        "unsubscribe_option_quotes" => {
            r#"    /// Unsubscribe from quote data for an option contract.
    fn unsubscribe_option_quotes(
        &self,
        symbol: &str,
        expiration: &str,
        strike: &str,
        right: &str,
    ) -> PyResult<()> {
        let contract = fpss::protocol::Contract::option(symbol, expiration, strike, right);
        self.tdx.unsubscribe_quotes(&contract).map_err(to_py_err)
    }
"#
        }
        "unsubscribe_option_trades" => {
            r#"    /// Unsubscribe from trade data for an option contract.
    fn unsubscribe_option_trades(
        &self,
        symbol: &str,
        expiration: &str,
        strike: &str,
        right: &str,
    ) -> PyResult<()> {
        let contract = fpss::protocol::Contract::option(symbol, expiration, strike, right);
        self.tdx.unsubscribe_trades(&contract).map_err(to_py_err)
    }
"#
        }
        "unsubscribe_option_open_interest" => {
            r#"    /// Unsubscribe from open interest data for an option contract.
    fn unsubscribe_option_open_interest(
        &self,
        symbol: &str,
        expiration: &str,
        strike: &str,
        right: &str,
    ) -> PyResult<()> {
        let contract = fpss::protocol::Contract::option(symbol, expiration, strike, right);
        self.tdx
            .unsubscribe_open_interest(&contract)
            .map_err(to_py_err)
    }
"#
        }
        "contract_map" => {
            r#"    /// Get the current contract map (server-assigned IDs -> contract strings).
    fn contract_map(&self) -> PyResult<std::collections::HashMap<i32, String>> {
        self.tdx
            .contract_map()
            .map(|m| m.into_iter().map(|(id, c)| (id, format!("{c}"))).collect())
            .map_err(to_py_err)
    }
"#
        }
        "contract_lookup" => {
            r#"    /// Look up a single contract by its server-assigned ID.
    fn contract_lookup(&self, id: i32) -> PyResult<Option<String>> {
        self.tdx
            .contract_lookup(id)
            .map(|opt| opt.map(|c| format!("{c}")))
            .map_err(to_py_err)
    }
"#
        }
        "active_subscriptions" => {
            r#"    /// Get a snapshot of currently active subscriptions.
    fn active_subscriptions(&self) -> PyResult<Vec<std::collections::HashMap<String, String>>> {
        self.tdx
            .active_subscriptions()
            .map(|subs| {
                subs.into_iter()
                    .map(|(kind, contract)| {
                        let mut m = std::collections::HashMap::new();
                        m.insert("kind".to_string(), format!("{kind:?}"));
                        m.insert("contract".to_string(), format!("{contract}"));
                        m
                    })
                    .collect()
            })
            .map_err(to_py_err)
    }
"#
        }
        "next_event" => {
            r#"    /// Poll for the next FPSS event.
    ///
    /// Args:
    ///     timeout_ms: Maximum time to wait in milliseconds.
    ///
    /// Returns:
    ///     A dict with ``kind`` key indicating event type, or ``None`` if timeout.
    ///     Raises ``RuntimeError`` if streaming has not been started.
    fn next_event(&self, py: Python<'_>, timeout_ms: u64) -> PyResult<Option<Py<PyAny>>> {
        let rx_outer = self.rx.lock().unwrap_or_else(|e| e.into_inner());
        let rx_arc = match rx_outer.as_ref() {
            Some(arc) => Arc::clone(arc),
            None => {
                return Err(PyRuntimeError::new_err(
                    "streaming not started -- call start_streaming() first",
                ))
            }
        };
        drop(rx_outer);
        let timeout = std::time::Duration::from_millis(timeout_ms);
        let result = py.detach(move || {
            let rx = rx_arc.lock().unwrap_or_else(|e| e.into_inner());
            rx.recv_timeout(timeout).ok()
        });
        match result {
            Some(event) => Ok(Some(buffered_event_to_py(py, &event))),
            None => Ok(None),
        }
        }
"#
        }
        "reconnect" => {
            r#"    /// Reconnect streaming and re-subscribe all previous subscriptions.
    fn reconnect(&self) -> PyResult<()> {
        let (tx, rx) = std::sync::mpsc::channel::<BufferedEvent>();
        self.tdx
            .reconnect_streaming(move |event: &fpss::FpssEvent| {
                let _ = tx.send(fpss_event_to_buffered(event));
            })
            .map_err(to_py_err)?;
        if let Ok(mut guard) = self.rx.lock() {
            *guard = Some(Arc::new(Mutex::new(rx)));
        }
        Ok(())
    }
"#
        }
        "stop_streaming" => {
            r#"    /// Stop streaming (historical remains active).
    fn stop_streaming(&self) {
        self.tdx.stop_streaming();
        if let Ok(mut guard) = self.rx.lock() {
            *guard = None;
        }
    }
"#
        }
        "shutdown" => {
            r#"    /// Stop streaming (alias for ``stop_streaming()``).
    ///
    /// Historical client remains usable until the ``ThetaDataDx`` object is dropped.
    fn shutdown(&self) {
        self.tdx.stop_streaming();
        if let Ok(mut guard) = self.rx.lock() {
            *guard = None;
        }
    }
"#
        }
        other => panic!("unknown python streaming method: {other}"),
    }
}

fn python_utility_function(name: &str) -> &'static str {
    match name {
        "all_greeks" => {
            r#"/// Compute all 22 Black-Scholes Greeks + IV in one call.
///
/// `right` accepts `"C"`/`"P"` or `"call"`/`"put"` case-insensitively.
/// Raises `ValueError` on unrecognised input via the tdbe-level panic handler.
#[pyfunction]
#[allow(clippy::too_many_arguments)] // Reason: mirrors Black-Scholes parameter set expected by Python callers
fn all_greeks(
    py: Python<'_>,
    spot: f64,
    strike: f64,
    rate: f64,
    div_yield: f64,
    tte: f64,
    option_price: f64,
    right: &str,
) -> Py<PyAny> {
    let g = tdbe::greeks::all_greeks(spot, strike, rate, div_yield, tte, option_price, right);
    let dict = PyDict::new(py);
    // PyO3: set_item is infallible for primitive types
    dict.set_item("value", g.value).unwrap();
    dict.set_item("iv", g.iv).unwrap();
    dict.set_item("iv_error", g.iv_error).unwrap();
    dict.set_item("delta", g.delta).unwrap();
    dict.set_item("gamma", g.gamma).unwrap();
    dict.set_item("theta", g.theta).unwrap();
    dict.set_item("vega", g.vega).unwrap();
    dict.set_item("rho", g.rho).unwrap();
    dict.set_item("vanna", g.vanna).unwrap();
    dict.set_item("charm", g.charm).unwrap();
    dict.set_item("vomma", g.vomma).unwrap();
    dict.set_item("veta", g.veta).unwrap();
    dict.set_item("speed", g.speed).unwrap();
    dict.set_item("zomma", g.zomma).unwrap();
    dict.set_item("color", g.color).unwrap();
    dict.set_item("ultima", g.ultima).unwrap();
    dict.set_item("d1", g.d1).unwrap();
    dict.set_item("d2", g.d2).unwrap();
    dict.set_item("dual_delta", g.dual_delta).unwrap();
    dict.set_item("dual_gamma", g.dual_gamma).unwrap();
    dict.set_item("epsilon", g.epsilon).unwrap();
    dict.set_item("lambda", g.lambda).unwrap();
    dict.into_any().unbind()
}
"#
        }
        "implied_volatility" => {
            r#"/// Compute implied volatility via bisection.
///
/// `right` accepts `"C"`/`"P"` or `"call"`/`"put"` case-insensitively.
#[pyfunction]
#[allow(clippy::too_many_arguments)] // Reason: mirrors Black-Scholes parameter set expected by Python callers
fn implied_volatility(
    spot: f64,
    strike: f64,
    rate: f64,
    div_yield: f64,
    tte: f64,
    option_price: f64,
    right: &str,
) -> (f64, f64) {
    tdbe::greeks::implied_volatility(spot, strike, rate, div_yield, tte, option_price, right)
}
"#
        }
        other => panic!("unknown python utility: {other}"),
    }
}

fn go_fpss_method(name: &str) -> &'static str {
    match name {
        "subscribe_quotes" => {
            r#"// SubscribeQuotes subscribes to real-time quote data for a stock symbol.
func (f *FpssClient) SubscribeQuotes(symbol string) (int, error) {
    cs := C.CString(symbol)
    defer C.free(unsafe.Pointer(cs))
    return f.fpssCall(C.tdx_fpss_subscribe_quotes(f.handle, cs))
}
"#
        }
        "subscribe_trades" => {
            r#"// SubscribeTrades subscribes to real-time trade data for a stock symbol.
func (f *FpssClient) SubscribeTrades(symbol string) (int, error) {
    cs := C.CString(symbol)
    defer C.free(unsafe.Pointer(cs))
    return f.fpssCall(C.tdx_fpss_subscribe_trades(f.handle, cs))
}
"#
        }
        "subscribe_open_interest" => {
            r#"// SubscribeOpenInterest subscribes to open interest data for a stock symbol.
func (f *FpssClient) SubscribeOpenInterest(symbol string) (int, error) {
    cs := C.CString(symbol)
    defer C.free(unsafe.Pointer(cs))
    return f.fpssCall(C.tdx_fpss_subscribe_open_interest(f.handle, cs))
}
"#
        }
        "subscribe_full_trades" => {
            r#"// SubscribeFullTrades subscribes to all trades for a security type ("STOCK", "OPTION", "INDEX").
func (f *FpssClient) SubscribeFullTrades(secType string) (int, error) {
    cs := C.CString(secType)
    defer C.free(unsafe.Pointer(cs))
    return f.fpssCall(C.tdx_fpss_subscribe_full_trades(f.handle, cs))
}
"#
        }
        "subscribe_full_open_interest" => {
            r#"// SubscribeFullOpenInterest subscribes to all open interest for a security type ("STOCK", "OPTION", "INDEX").
func (f *FpssClient) SubscribeFullOpenInterest(secType string) (int, error) {
    cs := C.CString(secType)
    defer C.free(unsafe.Pointer(cs))
    return f.fpssCall(C.tdx_fpss_subscribe_full_open_interest(f.handle, cs))
}
"#
        }
        "unsubscribe_quotes" => {
            r#"// UnsubscribeQuotes unsubscribes from quote data for a stock symbol.
func (f *FpssClient) UnsubscribeQuotes(symbol string) (int, error) {
    cs := C.CString(symbol)
    defer C.free(unsafe.Pointer(cs))
    return f.fpssCall(C.tdx_fpss_unsubscribe_quotes(f.handle, cs))
}
"#
        }
        "unsubscribe_trades" => {
            r#"// UnsubscribeTrades unsubscribes from trade data for a stock symbol.
func (f *FpssClient) UnsubscribeTrades(symbol string) (int, error) {
    cs := C.CString(symbol)
    defer C.free(unsafe.Pointer(cs))
    return f.fpssCall(C.tdx_fpss_unsubscribe_trades(f.handle, cs))
}
"#
        }
        "unsubscribe_open_interest" => {
            r#"// UnsubscribeOpenInterest unsubscribes from open interest data for a stock symbol.
func (f *FpssClient) UnsubscribeOpenInterest(symbol string) (int, error) {
    cs := C.CString(symbol)
    defer C.free(unsafe.Pointer(cs))
    return f.fpssCall(C.tdx_fpss_unsubscribe_open_interest(f.handle, cs))
}
"#
        }
        "unsubscribe_full_trades" => {
            r#"// UnsubscribeFullTrades unsubscribes from all trades for a security type ("STOCK", "OPTION", "INDEX").
func (f *FpssClient) UnsubscribeFullTrades(secType string) (int, error) {
    cs := C.CString(secType)
    defer C.free(unsafe.Pointer(cs))
    return f.fpssCall(C.tdx_fpss_unsubscribe_full_trades(f.handle, cs))
}
"#
        }
        "unsubscribe_full_open_interest" => {
            r#"// UnsubscribeFullOpenInterest unsubscribes from all open interest for a security type ("STOCK", "OPTION", "INDEX").
func (f *FpssClient) UnsubscribeFullOpenInterest(secType string) (int, error) {
    cs := C.CString(secType)
    defer C.free(unsafe.Pointer(cs))
    return f.fpssCall(C.tdx_fpss_unsubscribe_full_open_interest(f.handle, cs))
}
"#
        }
        "is_authenticated" => {
            r#"// IsAuthenticated returns true if the FPSS client is currently authenticated.
func (f *FpssClient) IsAuthenticated() bool {
    return C.tdx_fpss_is_authenticated(f.handle) != 0
}
"#
        }
        "contract_lookup" => {
            r#"// ContractLookup looks up a contract by its server-assigned ID.
// Returns ("", nil) when the ID is not found, ("", error) on real errors.
func (f *FpssClient) ContractLookup(id int) (string, error) {
    cstr := C.tdx_fpss_contract_lookup(f.handle, C.int(id))
    if cstr == nil {
        // NULL + empty last-error means "not found"; non-empty means real error.
        if msg := lastError(); msg != "" {
            return "", fmt.Errorf("thetadatadx: %s", msg)
        }
        return "", nil
    }
    goStr := C.GoString(cstr)
    C.tdx_string_free(cstr)
    return goStr, nil
}
"#
        }
        "active_subscriptions" => {
            r#"// ActiveSubscriptions returns the currently active subscriptions as typed structs.
func (f *FpssClient) ActiveSubscriptions() ([]Subscription, error) {
    arr := C.tdx_fpss_active_subscriptions(f.handle)
    if arr == nil {
        return nil, fmt.Errorf("thetadatadx: %s", lastError())
    }
    defer C.tdx_subscription_array_free(arr)
    n := int(arr.len)
    if n == 0 || arr.data == nil {
        return nil, nil
    }
    subs := unsafe.Slice(arr.data, n)
    result := make([]Subscription, n)
    for i := 0; i < n; i++ {
        if subs[i].kind != nil {
            result[i].Kind = C.GoString(subs[i].kind)
        }
        if subs[i].contract != nil {
            result[i].Contract = C.GoString(subs[i].contract)
        }
    }
    return result, nil
}
"#
        }
        "next_event" => {
            r#"// NextEvent polls for the next streaming event with the given timeout in milliseconds.
// Returns nil if the timeout expires with no event.
func (f *FpssClient) NextEvent(timeoutMs uint64) (*FpssEvent, error) {
    raw := C.tdx_fpss_next_event(f.handle, C.uint64_t(timeoutMs))
    if raw == nil {
        return nil, nil
    }
    defer C.tdx_fpss_event_free(raw)

    event := &FpssEvent{
        Kind: FpssEventKind(raw.kind),
    }

    switch event.Kind {
    case FpssQuoteEvent:
        q := raw.quote
        event.Quote = &FpssQuote{
            ContractID:   int32(q.contract_id),
            MsOfDay:      int32(q.ms_of_day),
            BidSize:      int32(q.bid_size),
            BidExchange:  int32(q.bid_exchange),
            Bid:          float64(q.bid),
            BidCondition: int32(q.bid_condition),
            AskSize:      int32(q.ask_size),
            AskExchange:  int32(q.ask_exchange),
            Ask:          float64(q.ask),
            AskCondition: int32(q.ask_condition),
            Date:         int32(q.date),
            ReceivedAtNs: uint64(q.received_at_ns),
        }
    case FpssTradeEvent:
        t := raw.trade
        event.Trade = &FpssTrade{
            ContractID:     int32(t.contract_id),
            MsOfDay:        int32(t.ms_of_day),
            Sequence:       int32(t.sequence),
            ExtCondition1:  int32(t.ext_condition1),
            ExtCondition2:  int32(t.ext_condition2),
            ExtCondition3:  int32(t.ext_condition3),
            ExtCondition4:  int32(t.ext_condition4),
            Condition:      int32(t.condition),
            Size:           int32(t.size),
            Exchange:       int32(t.exchange),
            Price:          float64(t.price),
            ConditionFlags: int32(t.condition_flags),
            PriceFlags:     int32(t.price_flags),
            VolumeType:     int32(t.volume_type),
            RecordsBack:    int32(t.records_back),
            Date:           int32(t.date),
            ReceivedAtNs:   uint64(t.received_at_ns),
        }
    case FpssOpenInterestEvent:
        oi := raw.open_interest
        event.OpenInterest = &FpssOpenInterestData{
            ContractID:   int32(oi.contract_id),
            MsOfDay:      int32(oi.ms_of_day),
            OpenInterest: int32(oi.open_interest),
            Date:         int32(oi.date),
            ReceivedAtNs: uint64(oi.received_at_ns),
        }
    case FpssOhlcvcEvent:
        o := raw.ohlcvc
        event.Ohlcvc = &FpssOhlcvc{
            ContractID:   int32(o.contract_id),
            MsOfDay:      int32(o.ms_of_day),
            Open:         float64(o.open),
            High:         float64(o.high),
            Low:          float64(o.low),
            Close:        float64(o.close),
            Volume:       int64(o.volume),
            Count:        int64(o.count),
            Date:         int32(o.date),
            ReceivedAtNs: uint64(o.received_at_ns),
        }
    case FpssControlEvent:
        ctrl := raw.control
        detail := ""
        if ctrl.detail != nil {
            detail = C.GoString(ctrl.detail)
        }
        event.Control = &FpssControlData{
            Kind:   int32(ctrl.kind),
            ID:     int32(ctrl.id),
            Detail: detail,
        }
    case FpssRawDataEvent:
        rd := raw.raw_data
        event.RawCode = uint8(rd.code)
        if rd.payload != nil && rd.payload_len > 0 {
            event.RawPayload = C.GoBytes(unsafe.Pointer(rd.payload), C.int(rd.payload_len))
        }
    }

    return event, nil
}
"#
        }
        "subscribe_option_quotes" => {
            r#"// SubscribeOptionQuotes subscribes to quote data for an option contract.
func (f *FpssClient) SubscribeOptionQuotes(symbol, expiration, strike, right string) (int, error) {
    cs := C.CString(symbol)
    ce := C.CString(expiration)
    ck := C.CString(strike)
    cr := C.CString(right)
    defer C.free(unsafe.Pointer(cs))
    defer C.free(unsafe.Pointer(ce))
    defer C.free(unsafe.Pointer(ck))
    defer C.free(unsafe.Pointer(cr))
    return f.fpssCall(C.tdx_fpss_subscribe_option_quotes(f.handle, cs, ce, ck, cr))
}
"#
        }
        "subscribe_option_trades" => {
            r#"// SubscribeOptionTrades subscribes to trade data for an option contract.
func (f *FpssClient) SubscribeOptionTrades(symbol, expiration, strike, right string) (int, error) {
    cs := C.CString(symbol)
    ce := C.CString(expiration)
    ck := C.CString(strike)
    cr := C.CString(right)
    defer C.free(unsafe.Pointer(cs))
    defer C.free(unsafe.Pointer(ce))
    defer C.free(unsafe.Pointer(ck))
    defer C.free(unsafe.Pointer(cr))
    return f.fpssCall(C.tdx_fpss_subscribe_option_trades(f.handle, cs, ce, ck, cr))
}
"#
        }
        "subscribe_option_open_interest" => {
            r#"// SubscribeOptionOpenInterest subscribes to open interest data for an option contract.
func (f *FpssClient) SubscribeOptionOpenInterest(symbol, expiration, strike, right string) (int, error) {
    cs := C.CString(symbol)
    ce := C.CString(expiration)
    ck := C.CString(strike)
    cr := C.CString(right)
    defer C.free(unsafe.Pointer(cs))
    defer C.free(unsafe.Pointer(ce))
    defer C.free(unsafe.Pointer(ck))
    defer C.free(unsafe.Pointer(cr))
    return f.fpssCall(C.tdx_fpss_subscribe_option_open_interest(f.handle, cs, ce, ck, cr))
}
"#
        }
        "unsubscribe_option_quotes" => {
            r#"// UnsubscribeOptionQuotes unsubscribes from quote data for an option contract.
func (f *FpssClient) UnsubscribeOptionQuotes(symbol, expiration, strike, right string) (int, error) {
    cs := C.CString(symbol)
    ce := C.CString(expiration)
    ck := C.CString(strike)
    cr := C.CString(right)
    defer C.free(unsafe.Pointer(cs))
    defer C.free(unsafe.Pointer(ce))
    defer C.free(unsafe.Pointer(ck))
    defer C.free(unsafe.Pointer(cr))
    return f.fpssCall(C.tdx_fpss_unsubscribe_option_quotes(f.handle, cs, ce, ck, cr))
}
"#
        }
        "unsubscribe_option_trades" => {
            r#"// UnsubscribeOptionTrades unsubscribes from trade data for an option contract.
func (f *FpssClient) UnsubscribeOptionTrades(symbol, expiration, strike, right string) (int, error) {
    cs := C.CString(symbol)
    ce := C.CString(expiration)
    ck := C.CString(strike)
    cr := C.CString(right)
    defer C.free(unsafe.Pointer(cs))
    defer C.free(unsafe.Pointer(ce))
    defer C.free(unsafe.Pointer(ck))
    defer C.free(unsafe.Pointer(cr))
    return f.fpssCall(C.tdx_fpss_unsubscribe_option_trades(f.handle, cs, ce, ck, cr))
}
"#
        }
        "unsubscribe_option_open_interest" => {
            r#"// UnsubscribeOptionOpenInterest unsubscribes from open interest data for an option contract.
func (f *FpssClient) UnsubscribeOptionOpenInterest(symbol, expiration, strike, right string) (int, error) {
    cs := C.CString(symbol)
    ce := C.CString(expiration)
    ck := C.CString(strike)
    cr := C.CString(right)
    defer C.free(unsafe.Pointer(cs))
    defer C.free(unsafe.Pointer(ce))
    defer C.free(unsafe.Pointer(ck))
    defer C.free(unsafe.Pointer(cr))
    return f.fpssCall(C.tdx_fpss_unsubscribe_option_open_interest(f.handle, cs, ce, ck, cr))
}
"#
        }
        "contract_map" => {
            r#"// ContractMap returns the current contract ID mapping.
func (f *FpssClient) ContractMap() (map[int32]string, error) {
    arr := C.tdx_fpss_contract_map(f.handle)
    if arr == nil {
        return nil, fmt.Errorf("thetadatadx: %s", lastError())
    }
    defer C.tdx_contract_map_array_free(arr)
    result := make(map[int32]string, int(arr.len))
    if arr.data == nil || arr.len == 0 {
        return result, nil
    }
    entries := unsafe.Slice(arr.data, int(arr.len))
    for _, entry := range entries {
        value := ""
        if entry.contract != nil {
            value = C.GoString(entry.contract)
        }
        result[int32(entry.id)] = value
    }
    return result, nil
}
"#
        }
        "reconnect" => {
            r#"// Reconnect reconnects the FPSS streaming connection, re-subscribing all previous subscriptions.
func (f *FpssClient) Reconnect() error {
    rc := C.tdx_fpss_reconnect(f.handle)
    if rc < 0 {
        return fmt.Errorf("thetadatadx: %s", lastError())
    }
    return nil
}
"#
        }
        "shutdown" => {
            r#"// Shutdown gracefully shuts down the FPSS streaming connection.
func (f *FpssClient) Shutdown() {
    if f.handle != nil {
        C.tdx_fpss_shutdown(f.handle)
    }
}
"#
        }
        other => panic!("unknown go fpss method: {other}"),
    }
}

fn go_utility_function(name: &str) -> &'static str {
    match name {
        "all_greeks" => {
            r#"// AllGreeks computes all Black-Scholes Greeks + IV locally.
//
// right accepts "C"/"P" or "call"/"put" case-insensitively. Returns an error
// if the underlying FFI call fails; invalid right strings cause the native
// tdbe layer to panic, which the FFI wrapper surfaces as a null result.
func AllGreeks(spot, strike, rate, divYield, tte, optionPrice float64, right string) (*Greeks, error) {
    cRight := C.CString(right)
    defer C.free(unsafe.Pointer(cRight))
    ptr := C.tdx_all_greeks(C.double(spot), C.double(strike), C.double(rate), C.double(divYield), C.double(tte), C.double(optionPrice), cRight)
    if ptr == nil {
        return nil, fmt.Errorf("thetadatadx: %s", lastError())
    }
    defer C.tdx_greeks_result_free(ptr)
    return &Greeks{
        Value:     float64(ptr.value),
        Delta:     float64(ptr.delta),
        Gamma:     float64(ptr.gamma),
        Theta:     float64(ptr.theta),
        Vega:      float64(ptr.vega),
        Rho:       float64(ptr.rho),
        IV:        float64(ptr.iv),
        IVError:   float64(ptr.iv_error),
        Vanna:     float64(ptr.vanna),
        Charm:     float64(ptr.charm),
        Vomma:     float64(ptr.vomma),
        Veta:      float64(ptr.veta),
        Speed:     float64(ptr.speed),
        Zomma:     float64(ptr.zomma),
        Color:     float64(ptr.color),
        Ultima:    float64(ptr.ultima),
        D1:        float64(ptr.d1),
        D2:        float64(ptr.d2),
        DualDelta: float64(ptr.dual_delta),
        DualGamma: float64(ptr.dual_gamma),
        Epsilon:   float64(ptr.epsilon),
        Lambda:    float64(ptr.lambda),
    }, nil
}
"#
        }
        "implied_volatility" => {
            r#"// ImpliedVolatility computes implied volatility locally.
//
// right accepts "C"/"P" or "call"/"put" case-insensitively.
func ImpliedVolatility(spot, strike, rate, divYield, tte, optionPrice float64, right string) (float64, float64, error) {
    cRight := C.CString(right)
    defer C.free(unsafe.Pointer(cRight))
    var iv, ivErr C.double
    rc := C.tdx_implied_volatility(C.double(spot), C.double(strike), C.double(rate), C.double(divYield), C.double(tte), C.double(optionPrice), cRight, &iv, &ivErr)
    if rc != 0 {
        return 0, 0, fmt.Errorf("thetadatadx: %s", lastError())
    }
    return float64(iv), float64(ivErr), nil
}
"#
        }
        other => panic!("unknown go utility: {other}"),
    }
}

fn cpp_fpss_decl(name: &str) -> &'static str {
    match name {
        "connect" => "    /** Connect to FPSS streaming servers. Throws on failure. */\n    FpssClient(const Credentials& creds, const Config& config);\n",
        "subscribe_quotes" => "    int subscribe_quotes(const std::string& symbol);\n",
        "subscribe_trades" => "    int subscribe_trades(const std::string& symbol);\n",
        "subscribe_open_interest" => "    int subscribe_open_interest(const std::string& symbol);\n",
        "subscribe_full_trades" => "    int subscribe_full_trades(const std::string& sec_type);\n",
        "subscribe_full_open_interest" => "    int subscribe_full_open_interest(const std::string& sec_type);\n",
        "unsubscribe_quotes" => "    int unsubscribe_quotes(const std::string& symbol);\n",
        "unsubscribe_trades" => "    int unsubscribe_trades(const std::string& symbol);\n",
        "unsubscribe_open_interest" => "    int unsubscribe_open_interest(const std::string& symbol);\n",
        "unsubscribe_full_trades" => "    int unsubscribe_full_trades(const std::string& sec_type);\n",
        "unsubscribe_full_open_interest" => "    int unsubscribe_full_open_interest(const std::string& sec_type);\n",
        "subscribe_option_quotes" => "    int subscribe_option_quotes(const std::string& symbol, const std::string& expiration, const std::string& strike, const std::string& right);\n",
        "subscribe_option_trades" => "    int subscribe_option_trades(const std::string& symbol, const std::string& expiration, const std::string& strike, const std::string& right);\n",
        "subscribe_option_open_interest" => "    int subscribe_option_open_interest(const std::string& symbol, const std::string& expiration, const std::string& strike, const std::string& right);\n",
        "unsubscribe_option_quotes" => "    int unsubscribe_option_quotes(const std::string& symbol, const std::string& expiration, const std::string& strike, const std::string& right);\n",
        "unsubscribe_option_trades" => "    int unsubscribe_option_trades(const std::string& symbol, const std::string& expiration, const std::string& strike, const std::string& right);\n",
        "unsubscribe_option_open_interest" => "    int unsubscribe_option_open_interest(const std::string& symbol, const std::string& expiration, const std::string& strike, const std::string& right);\n",
        "is_authenticated" => "    bool is_authenticated() const;\n",
        "contract_lookup" => "    std::optional<std::string> contract_lookup(int id) const;\n",
        "contract_map" => "    /** Get the full contract map keyed by server-assigned contract ID. */\n    std::map<int32_t, std::string> contract_map() const;\n",
        "active_subscriptions" => "    std::vector<Subscription> active_subscriptions() const;\n",
        "next_event" => "    /** Poll for the next event as a typed struct. Returns nullptr on timeout. */\n    FpssEventPtr next_event(uint64_t timeout_ms);\n",
        "reconnect" => "    /** Reconnect, re-subscribing all previous subscriptions. Throws on failure. */\n    void reconnect();\n",
        "shutdown" => "    void shutdown();\n",
        other => panic!("unknown cpp fpss method: {other}"),
    }
}

fn cpp_fpss_def(name: &str) -> &'static str {
    match name {
        "connect" => {
            r#"FpssClient::FpssClient(const Credentials& creds, const Config& config) {
    auto h = tdx_fpss_connect(creds.get(), config.get());
    if (!h) throw std::runtime_error("thetadatadx: " + detail::last_ffi_error());
    handle_.reset(h);
}
"#
        }
        "subscribe_quotes" => {
            r#"int FpssClient::subscribe_quotes(const std::string& symbol) { return tdx_fpss_subscribe_quotes(handle_.get(), symbol.c_str()); }
"#
        }
        "subscribe_trades" => {
            r#"int FpssClient::subscribe_trades(const std::string& symbol) { return tdx_fpss_subscribe_trades(handle_.get(), symbol.c_str()); }
"#
        }
        "subscribe_open_interest" => {
            r#"int FpssClient::subscribe_open_interest(const std::string& symbol) { return tdx_fpss_subscribe_open_interest(handle_.get(), symbol.c_str()); }
"#
        }
        "subscribe_full_trades" => {
            r#"int FpssClient::subscribe_full_trades(const std::string& sec_type) { return tdx_fpss_subscribe_full_trades(handle_.get(), sec_type.c_str()); }
"#
        }
        "subscribe_full_open_interest" => {
            r#"int FpssClient::subscribe_full_open_interest(const std::string& sec_type) { return tdx_fpss_subscribe_full_open_interest(handle_.get(), sec_type.c_str()); }
"#
        }
        "unsubscribe_quotes" => {
            r#"int FpssClient::unsubscribe_quotes(const std::string& symbol) { return tdx_fpss_unsubscribe_quotes(handle_.get(), symbol.c_str()); }
"#
        }
        "unsubscribe_trades" => {
            r#"int FpssClient::unsubscribe_trades(const std::string& symbol) { return tdx_fpss_unsubscribe_trades(handle_.get(), symbol.c_str()); }
"#
        }
        "unsubscribe_open_interest" => {
            r#"int FpssClient::unsubscribe_open_interest(const std::string& symbol) { return tdx_fpss_unsubscribe_open_interest(handle_.get(), symbol.c_str()); }
"#
        }
        "unsubscribe_full_trades" => {
            r#"int FpssClient::unsubscribe_full_trades(const std::string& sec_type) { return tdx_fpss_unsubscribe_full_trades(handle_.get(), sec_type.c_str()); }
"#
        }
        "unsubscribe_full_open_interest" => {
            r#"int FpssClient::unsubscribe_full_open_interest(const std::string& sec_type) { return tdx_fpss_unsubscribe_full_open_interest(handle_.get(), sec_type.c_str()); }
"#
        }
        "subscribe_option_quotes" => {
            r#"int FpssClient::subscribe_option_quotes(const std::string& symbol, const std::string& expiration, const std::string& strike, const std::string& right) { return tdx_fpss_subscribe_option_quotes(handle_.get(), symbol.c_str(), expiration.c_str(), strike.c_str(), right.c_str()); }
"#
        }
        "subscribe_option_trades" => {
            r#"int FpssClient::subscribe_option_trades(const std::string& symbol, const std::string& expiration, const std::string& strike, const std::string& right) { return tdx_fpss_subscribe_option_trades(handle_.get(), symbol.c_str(), expiration.c_str(), strike.c_str(), right.c_str()); }
"#
        }
        "subscribe_option_open_interest" => {
            r#"int FpssClient::subscribe_option_open_interest(const std::string& symbol, const std::string& expiration, const std::string& strike, const std::string& right) { return tdx_fpss_subscribe_option_open_interest(handle_.get(), symbol.c_str(), expiration.c_str(), strike.c_str(), right.c_str()); }
"#
        }
        "unsubscribe_option_quotes" => {
            r#"int FpssClient::unsubscribe_option_quotes(const std::string& symbol, const std::string& expiration, const std::string& strike, const std::string& right) { return tdx_fpss_unsubscribe_option_quotes(handle_.get(), symbol.c_str(), expiration.c_str(), strike.c_str(), right.c_str()); }
"#
        }
        "unsubscribe_option_trades" => {
            r#"int FpssClient::unsubscribe_option_trades(const std::string& symbol, const std::string& expiration, const std::string& strike, const std::string& right) { return tdx_fpss_unsubscribe_option_trades(handle_.get(), symbol.c_str(), expiration.c_str(), strike.c_str(), right.c_str()); }
"#
        }
        "unsubscribe_option_open_interest" => {
            r#"int FpssClient::unsubscribe_option_open_interest(const std::string& symbol, const std::string& expiration, const std::string& strike, const std::string& right) { return tdx_fpss_unsubscribe_option_open_interest(handle_.get(), symbol.c_str(), expiration.c_str(), strike.c_str(), right.c_str()); }
"#
        }
        "is_authenticated" => {
            r#"bool FpssClient::is_authenticated() const { return tdx_fpss_is_authenticated(handle_.get()) != 0; }
"#
        }
        "contract_lookup" => {
            r#"std::optional<std::string> FpssClient::contract_lookup(int id) const {
    detail::FfiString result(tdx_fpss_contract_lookup(handle_.get(), id));
    if (!result.ok()) {
        // NULL + non-empty last-error means a real error; throw.
        // NULL + empty last-error means "not found"; return nullopt.
        std::string err = detail::last_ffi_error();
        if (!err.empty()) throw std::runtime_error("thetadatadx: " + err);
        return std::nullopt;
    }
    return result.str();
}
"#
        }
        "contract_map" => {
            r#"std::map<int32_t, std::string> FpssClient::contract_map() const {
    auto* arr = tdx_fpss_contract_map(handle_.get());
    if (!arr) throw std::runtime_error("thetadatadx: " + detail::last_ffi_error());
    std::map<int32_t, std::string> result;
    if (arr->data != nullptr && arr->len > 0) {
        for (size_t i = 0; i < arr->len; ++i) {
            result.emplace(arr->data[i].id, arr->data[i].contract ? std::string(arr->data[i].contract) : std::string());
        }
    }
    tdx_contract_map_array_free(arr);
    return result;
}
"#
        }
        "active_subscriptions" => {
            r#"std::vector<Subscription> FpssClient::active_subscriptions() const {
    return detail::subscription_array_to_vector(tdx_fpss_active_subscriptions(handle_.get()));
}
"#
        }
        "next_event" => {
            r#"FpssEventPtr FpssClient::next_event(uint64_t timeout_ms) {
    auto* raw = tdx_fpss_next_event(handle_.get(), timeout_ms);
    return FpssEventPtr(raw);
}
"#
        }
        "reconnect" => {
            r#"void FpssClient::reconnect() {
    int rc = tdx_fpss_reconnect(handle_.get());
    if (rc < 0) throw std::runtime_error("thetadatadx: " + detail::last_ffi_error());
}
"#
        }
        "shutdown" => {
            r#"void FpssClient::shutdown() { tdx_fpss_shutdown(handle_.get()); }
"#
        }
        other => panic!("unknown cpp fpss method: {other}"),
    }
}

fn cpp_utility_decl(name: &str) -> &'static str {
    match name {
        "all_greeks" => {
            "/** Compute all 22 Greeks + IV. `right` accepts \"C\"/\"P\" or \"call\"/\"put\" (case-insensitive). Throws on failure. */\nGreeks all_greeks(double spot, double strike, double rate, double div_yield,\n                  double tte, double option_price, const std::string& right);\n"
        }
        "implied_volatility" => {
            "/** Compute implied volatility. `right` accepts \"C\"/\"P\" or \"call\"/\"put\" (case-insensitive). Returns (iv, error). Throws on failure. */\nstd::pair<double, double> implied_volatility(double spot, double strike,\n                                             double rate, double div_yield,\n                                             double tte, double option_price,\n                                             const std::string& right);\n"
        }
        other => panic!("unknown cpp utility: {other}"),
    }
}

fn cpp_utility_def(name: &str) -> &'static str {
    match name {
        "all_greeks" => {
            r#"Greeks all_greeks(double spot, double strike, double rate, double div_yield,
                  double tte, double option_price, const std::string& right) {
    TdxGreeksResult* raw = tdx_all_greeks(
        spot,
        strike,
        rate,
        div_yield,
        tte,
        option_price,
        right.c_str()
    );
    if (raw == nullptr) {
        throw std::runtime_error("thetadatadx: " + detail::last_ffi_error());
    }

    Greeks result{
        raw->value,
        raw->delta,
        raw->gamma,
        raw->theta,
        raw->vega,
        raw->rho,
        raw->iv,
        raw->iv_error,
        raw->vanna,
        raw->charm,
        raw->vomma,
        raw->veta,
        raw->speed,
        raw->zomma,
        raw->color,
        raw->ultima,
        raw->d1,
        raw->d2,
        raw->dual_delta,
        raw->dual_gamma,
        raw->epsilon,
        raw->lambda,
    };
    tdx_greeks_result_free(raw);
    return result;
}
"#
        }
        "implied_volatility" => {
            r#"std::pair<double, double> implied_volatility(double spot, double strike,
                                              double rate, double div_yield,
                                              double tte, double option_price,
                                              const std::string& right) {
    double iv = 0.0, err = 0.0;
    int rc = tdx_implied_volatility(spot, strike, rate, div_yield, tte, option_price, right.c_str(), &iv, &err);
    if (rc != 0) throw std::runtime_error("thetadatadx: " + detail::last_ffi_error());
    return {iv, err};
}
"#
        }
        other => panic!("unknown cpp utility: {other}"),
    }
}

fn cpp_lifecycle_def(name: &str) -> &'static str {
    match name {
        "credentials_from_file" => {
            r#"Credentials Credentials::from_file(const std::string& path) {
    auto h = tdx_credentials_from_file(path.c_str());
    if (!h) throw std::runtime_error("thetadatadx: " + detail::last_ffi_error());
    return Credentials(h);
}
"#
        }
        "credentials_from_email" => {
            r#"Credentials Credentials::from_email(const std::string& email, const std::string& password) {
    auto h = tdx_credentials_new(email.c_str(), password.c_str());
    if (!h) throw std::runtime_error("thetadatadx: " + detail::last_ffi_error());
    return Credentials(h);
}
"#
        }
        "config_production" => {
            r#"Config Config::production() { return Config(tdx_config_production()); }
"#
        }
        "config_dev" => {
            r#"Config Config::dev() { return Config(tdx_config_dev()); }
"#
        }
        "config_stage" => {
            r#"Config Config::stage() { return Config(tdx_config_stage()); }
"#
        }
        "client_connect" => {
            r#"Client Client::connect(const Credentials& creds, const Config& config) {
    auto h = tdx_client_connect(creds.get(), config.get());
    if (!h) throw std::runtime_error("thetadatadx: " + detail::last_ffi_error());
    return Client(h);
}
"#
        }
        other => panic!("unknown cpp lifecycle method: {other}"),
    }
}

fn mcp_tool_definition(name: &str) -> &'static str {
    match name {
        "ping" => {
            r#"    tools.push(json!({
        "name": "ping",
        "description": "Check MCP server status. Returns uptime and connection info without hitting ThetaData servers.",
        "inputSchema": {
            "type": "object",
            "properties": {},
            "required": []
        }
    }));
"#
        }
        "all_greeks" => {
            r#"    tools.push(json!({
        "name": "all_greeks",
        "description": "Compute all 22 Black-Scholes Greeks OFFLINE (no ThetaData server needed). Returns value, delta, gamma, theta, vega, rho, IV, vanna, charm, vomma, veta, speed, zomma, color, ultima, d1, d2, dual_delta, dual_gamma, epsilon, lambda.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "spot": { "type": "number", "description": "Spot price (underlying)" },
                "strike": { "type": "number", "description": "Strike price" },
                "rate": { "type": "number", "description": "Risk-free rate (e.g. 0.05 for 5%)" },
                "dividend_yield": { "type": "number", "description": "Dividend yield (e.g. 0.02 for 2%)" },
                "time_to_expiry": { "type": "number", "description": "Time to expiration in years (e.g. 0.25 for 3 months)" },
                "option_price": { "type": "number", "description": "Market price of the option" },
                "right": { "type": "string", "description": "Option side: \"C\"/\"P\" or \"call\"/\"put\" (case-insensitive)", "enum": ["C", "P", "c", "p", "call", "put", "CALL", "PUT", "Call", "Put"] }
            },
            "required": ["spot", "strike", "rate", "dividend_yield", "time_to_expiry", "option_price", "right"]
        }
    }));
"#
        }
        "implied_volatility" => {
            r#"    tools.push(json!({
        "name": "implied_volatility",
        "description": "Compute implied volatility OFFLINE using bisection (no ThetaData server needed). Returns IV and error.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "spot": { "type": "number", "description": "Spot price (underlying)" },
                "strike": { "type": "number", "description": "Strike price" },
                "rate": { "type": "number", "description": "Risk-free rate (e.g. 0.05)" },
                "dividend_yield": { "type": "number", "description": "Dividend yield (e.g. 0.02)" },
                "time_to_expiry": { "type": "number", "description": "Time to expiration in years" },
                "option_price": { "type": "number", "description": "Market price of the option" },
                "right": { "type": "string", "description": "Option side: \"C\"/\"P\" or \"call\"/\"put\" (case-insensitive)", "enum": ["C", "P", "c", "p", "call", "put", "CALL", "PUT", "Call", "Put"] }
            },
            "required": ["spot", "strike", "rate", "dividend_yield", "time_to_expiry", "option_price", "right"]
        }
    }));
"#
        }
        other => panic!("unknown mcp utility: {other}"),
    }
}

fn mcp_execute_arm(name: &str) -> &'static str {
    match name {
        "ping" => {
            r#"        "ping" => {
            let uptime = start_time.elapsed();
            Some(Ok(json!({
                "status": "ok",
                "server": "thetadatadx-mcp",
                "version": VERSION,
                "uptime_secs": uptime.as_secs(),
                "connected": client.is_some(),
            })))
        }
"#
        }
        "all_greeks" => {
            r#"        "all_greeks" => {
            let s = param_or_return!(arg_f64(args, "spot"));
            let x = param_or_return!(arg_f64(args, "strike"));
            let r = param_or_return!(arg_f64(args, "rate"));
            let q = param_or_return!(arg_f64(args, "dividend_yield"));
            let t = param_or_return!(arg_f64(args, "time_to_expiry"));
            let price = param_or_return!(arg_f64(args, "option_price"));
            let right = param_or_return!(arg_str(args, "right"));
            param_or_return!(thetadatadx::parse_right_strict(&right).map_err(|e| e.to_string()));

            let g = tdbe::greeks::all_greeks(s, x, r, q, t, price, &right);
            Some(Ok(json!({
                "value": g.value,
                "iv": g.iv,
                "iv_error": g.iv_error,
                "delta": g.delta,
                "gamma": g.gamma,
                "theta": g.theta,
                "vega": g.vega,
                "rho": g.rho,
                "vanna": g.vanna,
                "charm": g.charm,
                "vomma": g.vomma,
                "veta": g.veta,
                "speed": g.speed,
                "zomma": g.zomma,
                "color": g.color,
                "ultima": g.ultima,
                "d1": g.d1,
                "d2": g.d2,
                "dual_delta": g.dual_delta,
                "dual_gamma": g.dual_gamma,
                "epsilon": g.epsilon,
                "lambda": g.lambda,
            })))
        }
"#
        }
        "implied_volatility" => {
            r#"        "implied_volatility" => {
            let s = param_or_return!(arg_f64(args, "spot"));
            let x = param_or_return!(arg_f64(args, "strike"));
            let r = param_or_return!(arg_f64(args, "rate"));
            let q = param_or_return!(arg_f64(args, "dividend_yield"));
            let t = param_or_return!(arg_f64(args, "time_to_expiry"));
            let price = param_or_return!(arg_f64(args, "option_price"));
            let right = param_or_return!(arg_str(args, "right"));
            param_or_return!(thetadatadx::parse_right_strict(&right).map_err(|e| e.to_string()));

            let (iv, err) = tdbe::greeks::implied_volatility(s, x, r, q, t, price, &right);
            Some(Ok(json!({
                "implied_volatility": iv,
                "error": err,
            })))
        }
"#
        }
        other => panic!("unknown mcp utility: {other}"),
    }
}

fn cli_command_builder(utility: &UtilitySpec) -> String {
    match utility.name.as_str() {
        "auth" => "    app = app.subcommand(Command::new(\"auth\").about(\"Test authentication and print session info\"));\n".into(),
        "all_greeks" => format!(
            "    app = app.subcommand(\n        Command::new({:?})\n            .about(\"Compute Black-Scholes Greeks (offline, no server needed)\")\n            .arg(Arg::new(\"spot\").required(true).help(\"Spot price\"))\n            .arg(Arg::new(\"strike\").required(true).help(\"Strike price\"))\n            .arg(\n                Arg::new(\"rate\")\n                    .required(true)\n                    .help(\"Risk-free rate (e.g. 0.05)\"),\n            )\n            .arg(\n                Arg::new(\"dividend\")\n                    .required(true)\n                    .help(\"Dividend yield (e.g. 0.015)\"),\n            )\n            .arg(\n                Arg::new(\"time\")\n                    .required(true)\n                    .help(\"Time to expiration in years (e.g. 0.082 for ~30 days)\"),\n            )\n            .arg(Arg::new(\"option_price\").required(true).help(\"Option price\"))\n            .arg(\n                Arg::new(\"right\")\n                    .required(true)\n                    .help(\"Option side: \\\"C\\\"/\\\"P\\\" or \\\"call\\\"/\\\"put\\\" (case-insensitive)\"),\n            ),\n    );\n",
            utility.cli_name.as_deref().unwrap_or("greeks")
        ),
        "implied_volatility" => format!(
            "    app = app.subcommand(\n        Command::new({:?})\n            .about(\"Compute implied volatility only (offline, no server needed)\")\n            .arg(Arg::new(\"spot\").required(true).help(\"Spot price\"))\n            .arg(Arg::new(\"strike\").required(true).help(\"Strike price\"))\n            .arg(Arg::new(\"rate\").required(true).help(\"Risk-free rate\"))\n            .arg(Arg::new(\"dividend\").required(true).help(\"Dividend yield\"))\n            .arg(\n                Arg::new(\"time\")\n                    .required(true)\n                    .help(\"Time to expiration in years\"),\n            )\n            .arg(Arg::new(\"option_price\").required(true).help(\"Option price\"))\n            .arg(\n                Arg::new(\"right\")\n                    .required(true)\n                    .help(\"Option side: \\\"C\\\"/\\\"P\\\" or \\\"call\\\"/\\\"put\\\" (case-insensitive)\"),\n            ),\n    );\n",
            utility.cli_name.as_deref().unwrap_or("iv")
        ),
        other => panic!("unknown cli utility: {other}"),
    }
}

fn cli_dispatch_arm(utility: &UtilitySpec) -> String {
    match utility.name.as_str() {
        "auth" => r#"        Some(("auth", _)) => {
            let creds = thetadatadx::Credentials::from_file(creds_path)?;
            let resp = thetadatadx::auth::authenticate(&creds).await?;
            let mut td = TabularData::new(vec![
                "session_id",
                "email",
                "stock_tier",
                "options_tier",
                "indices_tier",
                "rate_tier",
                "created",
            ]);
            let user = resp.user.as_ref();
            let redacted_session = if resp.session_id.len() >= 8 {
                format!("{}...", &resp.session_id[..8])
            } else {
                resp.session_id.clone()
            };
            td.push(vec![
                redacted_session,
                user.and_then(|u| u.email.clone()).unwrap_or_default(),
                user.and_then(|u| u.stock_subscription)
                    .map(|t| format!("{t}"))
                    .unwrap_or_default(),
                user.and_then(|u| u.options_subscription)
                    .map(|t| format!("{t}"))
                    .unwrap_or_default(),
                user.and_then(|u| u.indices_subscription)
                    .map(|t| format!("{t}"))
                    .unwrap_or_default(),
                user.and_then(|u| u.interest_rate_subscription)
                    .map(|t| format!("{t}"))
                    .unwrap_or_default(),
                resp.session_created.unwrap_or_default(),
            ]);
            td.render(fmt);
            Ok(true)
        }
"#
        .into(),
        "all_greeks" => {
            let cli_name = utility.cli_name.as_deref().unwrap_or("greeks");
            format!(
                "        Some(({cli_name:?}, sub_m)) => {{\n            let spot: f64 = get_arg(sub_m, \"spot\")\n                .parse()\n                .map_err(|e| thetadatadx::Error::Config(format!(\"invalid spot price: {{e}}\")))?;\n            let strike: f64 = get_arg(sub_m, \"strike\")\n                .parse()\n                .map_err(|e| thetadatadx::Error::Config(format!(\"invalid strike price: {{e}}\")))?;\n            let rate: f64 = get_arg(sub_m, \"rate\")\n                .parse()\n                .map_err(|e| thetadatadx::Error::Config(format!(\"invalid rate: {{e}}\")))?;\n            let dividend: f64 = get_arg(sub_m, \"dividend\")\n                .parse()\n                .map_err(|e| thetadatadx::Error::Config(format!(\"invalid dividend: {{e}}\")))?;\n            let time: f64 = get_arg(sub_m, \"time\")\n                .parse()\n                .map_err(|e| thetadatadx::Error::Config(format!(\"invalid time: {{e}}\")))?;\n            let option_price: f64 = get_arg(sub_m, \"option_price\")\n                .parse()\n                .map_err(|e| thetadatadx::Error::Config(format!(\"invalid option_price: {{e}}\")))?;\n            let right = get_arg(sub_m, \"right\");\n            thetadatadx::parse_right_strict(right)?;\n\n            let g = tdbe::greeks::all_greeks(spot, strike, rate, dividend, time, option_price, right);\n            let mut td = TabularData::new(vec![\"greek\", \"value\"]);\n            let rows = [\n                (\"value\", g.value),\n                (\"iv\", g.iv),\n                (\"iv_error\", g.iv_error),\n                (\"delta\", g.delta),\n                (\"gamma\", g.gamma),\n                (\"theta\", g.theta),\n                (\"vega\", g.vega),\n                (\"rho\", g.rho),\n                (\"d1\", g.d1),\n                (\"d2\", g.d2),\n                (\"vanna\", g.vanna),\n                (\"charm\", g.charm),\n                (\"vomma\", g.vomma),\n                (\"veta\", g.veta),\n                (\"speed\", g.speed),\n                (\"zomma\", g.zomma),\n                (\"color\", g.color),\n                (\"ultima\", g.ultima),\n                (\"dual_delta\", g.dual_delta),\n                (\"dual_gamma\", g.dual_gamma),\n                (\"epsilon\", g.epsilon),\n                (\"lambda\", g.lambda),\n            ];\n            for (name, val) in rows {{\n                td.push(vec![name.to_string(), format!(\"{{val:.8}}\")]);\n            }}\n            td.render(fmt);\n            Ok(true)\n        }}\n"
            )
        }
        "implied_volatility" => {
            let cli_name = utility.cli_name.as_deref().unwrap_or("iv");
            format!(
                "        Some(({cli_name:?}, sub_m)) => {{\n            let spot: f64 = get_arg(sub_m, \"spot\")\n                .parse()\n                .map_err(|e| thetadatadx::Error::Config(format!(\"invalid spot price: {{e}}\")))?;\n            let strike: f64 = get_arg(sub_m, \"strike\")\n                .parse()\n                .map_err(|e| thetadatadx::Error::Config(format!(\"invalid strike price: {{e}}\")))?;\n            let rate: f64 = get_arg(sub_m, \"rate\")\n                .parse()\n                .map_err(|e| thetadatadx::Error::Config(format!(\"invalid rate: {{e}}\")))?;\n            let dividend: f64 = get_arg(sub_m, \"dividend\")\n                .parse()\n                .map_err(|e| thetadatadx::Error::Config(format!(\"invalid dividend: {{e}}\")))?;\n            let time: f64 = get_arg(sub_m, \"time\")\n                .parse()\n                .map_err(|e| thetadatadx::Error::Config(format!(\"invalid time: {{e}}\")))?;\n            let option_price: f64 = get_arg(sub_m, \"option_price\")\n                .parse()\n                .map_err(|e| thetadatadx::Error::Config(format!(\"invalid option_price: {{e}}\")))?;\n            let right = get_arg(sub_m, \"right\");\n            thetadatadx::parse_right_strict(right)?;\n\n            let (iv, iv_error) = tdbe::greeks::implied_volatility(\n                spot,\n                strike,\n                rate,\n                dividend,\n                time,\n                option_price,\n                right,\n            );\n            let mut td = TabularData::new(vec![\"iv\", \"iv_error\"]);\n            td.push(vec![format!(\"{{iv:.8}}\"), format!(\"{{iv_error:.8}}\")]);\n            td.render(fmt);\n            Ok(true)\n        }}\n"
            )
        }
        other => panic!("unknown cli utility: {other}"),
    }
}
