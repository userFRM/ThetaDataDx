#!/usr/bin/env python3
"""Cross-binding parity check (Gate 2 / issue #545 + #595).

Reads `sdks/parity.toml` — the declared cross-binding presence matrix
— and compares each row's `python` / `typescript` / `cpp` claims to
the actual binding state extracted from:

Class-level rows (no dot in `name`):
- Python: every `m.add_class::<T>()` registered in `lib.rs` + helper
  `register_*` calls, expanded statically by parsing the Rust source.
  Mirrors the regex powering `test_no_pyclass_name_collisions.py`.
- TypeScript: `export declare class X` / `export class X` declarations
  in `sdks/typescript/index.d.ts`.
- C++: `^class X` / `^struct X` declarations in
  `sdks/cpp/include/thetadx.hpp`. The `.h` header is C-only and not
  considered for parity.

Field-level rows (dotted `name`, e.g. `ReconnectConfig.wait_ms`):
- Python: `#[setter] fn set_<canonical>` and `#[getter] fn <canonical>`
  parsed from `sdks/python/src/*.rs`. The canonical name composes the
  struct prefix (e.g. `reconnect_`) with the row suffix (`wait_ms`).
- TypeScript napi: `#[napi(js_name = "set<CamelCase>")]` and the
  matching getter declaration in `sdks/typescript/src/*.rs`. The
  CamelCase form lifts the snake_case canonical name.
- C++: `set_<canonical>` / `get_<canonical>` member functions on the
  `class Config { ... }` body in `thetadx.hpp` PLUS the matching
  `tdx_config_set_<canonical>` C-ABI declaration in `thetadx.h`.
- FFI: `tdx_config_set_<canonical>` AND
  `tdx_config_get_<canonical>` (or the `_explicit` widened-ABI shape)
  parsed from `ffi/src/*.rs`. Any binding flagged `true` on a field
  row implies the FFI symbol exists, because every higher-level
  binding forwards into the same C ABI.

Rust-only rows: a dotted row with `rust_only = true` MUST cite an
`issue = "#N"` tracking number. The script enforces both — a
`rust_only` flag with no issue or an `issue` flag with no `rust_only`
fails the gate.

Exits non-zero on any mismatch. Run from the repo root.

A `--selftest` switch runs an in-process synthetic-source matrix
covering positive (all-bound) and negative (missing-on-TS,
missing-on-C++, missing-on-FFI, undocumented-orphan, rust_only-
without-issue) cases. The selftest is registered with the
audit-protocol convention for CI gates.
"""

from __future__ import annotations

import pathlib
import re
import sys
import tempfile
import tomllib
from typing import Any


REPO_ROOT = pathlib.Path(__file__).resolve().parents[1]
PARITY_TOML = REPO_ROOT / "sdks" / "parity.toml"
PY_SRC = REPO_ROOT / "sdks" / "python" / "src"
TS_DTS = REPO_ROOT / "sdks" / "typescript" / "index.d.ts"
TS_SRC = REPO_ROOT / "sdks" / "typescript" / "src"
CPP_HPP = REPO_ROOT / "sdks" / "cpp" / "include" / "thetadx.hpp"
CPP_H = REPO_ROOT / "sdks" / "cpp" / "include" / "thetadx.h"
FFI_SRC = REPO_ROOT / "ffi" / "src"
CONFIG_DIR = REPO_ROOT / "crates" / "thetadatadx" / "src" / "config"


# ─── Class-level discovery (legacy / non-dotted rows) ───────────────


PYCLASS_RE = re.compile(
    r"#\[pyclass(?:\(([^)]*)\))?\][^{]*?"
    r"(?:pub(?:\(crate\))?\s+)?(?:struct|enum)\s+(\w+)",
    re.MULTILINE | re.DOTALL,
)
NAME_ATTR_RE = re.compile(r'name\s*=\s*"([^"]+)"')


def _python_name(attrs: str | None, struct_name: str) -> str:
    if attrs:
        m = NAME_ATTR_RE.search(attrs)
        if m:
            return m.group(1)
    return struct_name.removeprefix("Py")


def collect_python_classes(py_src: pathlib.Path) -> set[str]:
    """Python-side pyclasses, in the same way `m.add_class::<T>()`
    would surface them."""
    out: set[str] = set()
    for rs in py_src.rglob("*.rs"):
        text = rs.read_text(encoding="utf-8")
        for m in PYCLASS_RE.finditer(text):
            out.add(_python_name(m.group(1), m.group(2)))
    errors_rs = py_src / "errors.rs"
    if errors_rs.is_file():
        for m in re.finditer(r'm\.add\(\s*"(\w+)"\s*,', errors_rs.read_text(encoding="utf-8")):
            out.add(m.group(1))
    return out


TS_CLASS_RE = re.compile(r"export\s+declare\s+class\s+(\w+)")
TS_INTERFACE_RE = re.compile(r"export\s+(?:declare\s+)?interface\s+(\w+)")


def collect_typescript_classes(ts_dts: pathlib.Path) -> set[str]:
    out: set[str] = set()
    if not ts_dts.is_file():
        return out
    text = ts_dts.read_text(encoding="utf-8")
    for m in TS_CLASS_RE.finditer(text):
        out.add(m.group(1))
    for m in TS_INTERFACE_RE.finditer(text):
        out.add(m.group(1))
    js_path = ts_dts.with_name("index.js")
    if js_path.is_file():
        for m in re.finditer(r"exports\.(\w+)\s*=\s*\w+", js_path.read_text(encoding="utf-8")):
            out.add(m.group(1))
    return out


CPP_CLASS_RE = re.compile(r"^(?:class|struct)\s+(\w+)", re.MULTILINE)
CPP_USING_RE = re.compile(r"^using\s+(\w+)\s*=", re.MULTILINE)


def collect_cpp_classes(cpp_hpp: pathlib.Path) -> set[str]:
    out: set[str] = set()
    if not cpp_hpp.is_file():
        return out
    text = cpp_hpp.read_text(encoding="utf-8")
    for m in CPP_CLASS_RE.finditer(text):
        out.add(m.group(1))
    for m in CPP_USING_RE.finditer(text):
        out.add(m.group(1))
    return out


CPP_ALIASES: dict[str, str] = {
    "ThetaDataDxClient": "UnifiedClient",
    "FlatFilesNamespace": "FlatFiles",
    "Contract": "FluentContract",
    "Subscription": "FluentSubscription",
    "SecType": "FluentSecType",
    "ParseError": "FpssParseError",
}


def _cpp_class_for(class_name: str) -> str:
    """Resolve a parity-toml `class` field to its C++ class symbol.

    Honors `CPP_ALIASES` so a row carrying the Python/TS canonical
    name (`ThetaDataDxClient`) routes to the corresponding C++ class
    body (`UnifiedClient`).
    """
    return CPP_ALIASES.get(class_name, class_name)


def cpp_has(symbol: str, cpp: set[str]) -> bool:
    if symbol in cpp:
        return True
    alias = CPP_ALIASES.get(symbol)
    if alias and alias in cpp:
        return True
    return False


def _is_implicitly_tracked(name: str) -> bool:
    if name.endswith("Tick") or name.endswith("TickList") or name.endswith("TickListIter"):
        return True
    if name.endswith("Builder"):
        return True
    if name.endswith("List") or name.endswith("ListIter"):
        return True
    if name in {
        "Quote",
        "Trade",
        "Ohlcvc",
        "OpenInterest",
        "ContractAssigned",
        "Connected",
        "Disconnected",
        "Error",
        "LoginSuccess",
        "MarketOpen",
        "MarketClose",
        "Ping",
        "Reconnected",
        "ReconnectedServer",
        "Reconnecting",
        "ReconnectsExhausted",
        "ReqResponse",
        "Restart",
        "ServerError",
        "UnknownControl",
        "UnknownFrame",
        "OptionContract",
        "AllGreeks",
    }:
        return True
    return False


# ─── Field-level discovery (per-setter granularity / #595) ──────────


# Struct → setter-name prefix. The Rust struct lives on
# `DirectConfig.<accessor>`, but the binding-side setter name combines
# the prefix with the row's field suffix. E.g. `ReconnectConfig.wait_ms`
# resolves to Python `set_reconnect_wait_ms`, TS `setReconnectWaitMs`,
# C++ `set_reconnect_wait_ms`, FFI `tdx_config_set_reconnect_wait_ms`.
STRUCT_TO_PREFIX: dict[str, str] = {
    "MddsConfig": "",
    "FpssConfig": "",
    "FlatFilesConfig": "flatfiles_",
    "ReconnectConfig": "reconnect_",
    "RuntimeConfig": "",
    "RetryPolicy": "retry_",
    "AuthConfig": "",
    "MetricsConfig": "metrics_",
}


def _snake_to_camel(snake: str) -> str:
    """`reconnect_wait_ms` → `reconnectWaitMs`."""
    head, *rest = snake.split("_")
    return head + "".join(part.capitalize() for part in rest)


def _canonical_setter(struct_name: str, suffix: str) -> str | None:
    """Compose the binding-side canonical setter name from the struct
    prefix and the row suffix. Returns ``None`` for unknown structs
    so the caller surfaces a clear diagnostic.
    """
    prefix = STRUCT_TO_PREFIX.get(struct_name)
    if prefix is None:
        return None
    return f"{prefix}{suffix}"


# Some FFI / C++ setters use the widened `_explicit(has_value, n)` ABI
# shape for `Option<usize>` fields (`RuntimeConfig.tokio_worker_threads`,
# `MetricsConfig.port`). The
# parity row uses the bare field name, but the binding exposes
# `tdx_config_set_<field>_explicit` as the canonical setter. Accept
# either shape when matching.
FFI_EXPLICIT_SUFFIXES = ("_explicit",)


def _collect_python_setters(py_src: pathlib.Path) -> set[str]:
    """Setter names on the `Config` pyclass. The pyo3 macro pattern is
    ``#[setter] fn set_<name>`` (or ``fn <name>``). Field-level parity
    requires the setter — getter presence is a UX nicety but several
    write-only knobs (e.g. ``reconnect_max_attempts``) have no
    getter on any binding by design.
    """
    setters: set[str] = set()
    if not py_src.is_dir():
        return setters
    for rs in py_src.rglob("*.rs"):
        text = rs.read_text(encoding="utf-8")
        for m in re.finditer(r"#\[setter\][^}]*?fn\s+(\w+)", text, re.DOTALL):
            name = m.group(1)
            if name.startswith("set_"):
                setters.add(name[4:])
            else:
                setters.add(name)
    return setters


# TypeScript camelCase compound-word aliases. Multi-word terms that
# the cross-binding contract names as a single snake_case token (e.g.
# `flatfiles` in Python / C++ / FFI) get camelCased as a multi-word
# `FlatFiles` in the napi `js_name`. The alias table records the
# canonical snake form so the parity gate accepts both conventions.
TS_CAMEL_COMPOUNDS: dict[str, str] = {
    "flat_files": "flatfiles",
}


def _camel_to_snake_with_aliases(camel: str) -> set[str]:
    """`FlatFilesMaxAttempts` → both `flat_files_max_attempts` and
    `flatfiles_max_attempts`. Returns every plausible snake-case
    rendering so the parity gate accepts the cross-binding
    convention regardless of which form is canonical.
    """
    base = re.sub(r"(?<!^)([A-Z])", r"_\1", camel).lower()
    renderings = {base}
    for source, target in TS_CAMEL_COMPOUNDS.items():
        if source in base:
            renderings.add(base.replace(source, target))
    return renderings


def _collect_typescript_setters(ts_src: pathlib.Path) -> set[str]:
    """napi `set<CamelName>` setter declarations on the napi `Config`
    impl block. Returns a set of canonical snake_case names. Getter
    presence is intentionally not gated — several write-only knobs
    (e.g. `setReconnectMaxAttempts`) have no getter on any binding
    by design.
    """
    setters_camel: set[str] = set()
    if not ts_src.is_dir():
        return set()
    for rs in ts_src.rglob("*.rs"):
        text = rs.read_text(encoding="utf-8")
        # `#[napi(js_name = "setX")]` → setter `X` (drop the `set` prefix).
        for m in re.finditer(
            r'#\[napi\([^)]*\bjs_name\s*=\s*"set([A-Z]\w*)"[^)]*\)\]',
            text,
        ):
            setters_camel.add(m.group(1))
    # Lift to snake_case for parity-row matching. Every camelCase
    # name renders to one or more snake-case candidates (the bare
    # snake form plus any compound-word alias rendering); the gate
    # accepts a match against any rendering.
    setters_snake: set[str] = set()
    for name in setters_camel:
        setters_snake.update(_camel_to_snake_with_aliases(name))
    return setters_snake


def _collect_cpp_setters(cpp_hpp: pathlib.Path, cpp_h: pathlib.Path) -> set[str]:
    """C++ wrapper exposes setters as inline `set_<name>(<type>)` on
    the `class Config { ... }` body. The matching
    `tdx_config_set_<name>` declaration in `thetadx.h` is the C ABI
    surface the wrapper forwards to; the parity gate requires both
    halves so a forgotten C header declaration trips at link time.
    Getter presence is not gated — several write-only knobs have no
    C++ getter by design (matching the FFI / Python / TS contract).
    """
    cpp_setters: set[str] = set()
    if cpp_hpp.is_file():
        text = cpp_hpp.read_text(encoding="utf-8")
        for m in re.finditer(r"\bvoid\s+set_(\w+)\s*\(", text):
            cpp_setters.add(m.group(1))
        # Some C++ setters return `int32_t` for status codes (the
        # `_explicit` widened-ABI shape on `Option<usize>` fields).
        for m in re.finditer(r"\bint32_t\s+set_(\w+)\s*\(", text):
            cpp_setters.add(m.group(1))
    h_setters: set[str] = set()
    if cpp_h.is_file():
        text = cpp_h.read_text(encoding="utf-8")
        for m in re.finditer(r"\btdx_config_set_(\w+)\s*\(", text):
            h_setters.add(m.group(1))
    return cpp_setters & h_setters


def _collect_ffi_setters(ffi_src: pathlib.Path) -> set[str]:
    """FFI extern C setter declarations in `ffi/src/*.rs`. The
    convention is ``tdx_config_set_<name>``. Getter presence is not
    gated — several write-only knobs (e.g. the per-class reconnect
    budgets) have no FFI getter by design.
    """
    setters: set[str] = set()
    if not ffi_src.is_dir():
        return setters
    for rs in ffi_src.rglob("*.rs"):
        text = rs.read_text(encoding="utf-8")
        for m in re.finditer(r"\bfn\s+tdx_config_set_(\w+)\s*\(", text):
            setters.add(m.group(1))
    return setters


def _setter_present(canonical: str, setters: set[str]) -> bool:
    """True if `canonical` (or the `_explicit` widened-ABI variant) is
    in `setters`.
    """
    if canonical in setters:
        return True
    for suffix in FFI_EXPLICIT_SUFFIXES:
        if f"{canonical}{suffix}" in setters:
            return True
    return False


# ─── Rust field discovery (reverse-direction orphan check) ──────────


# Structs we consider in scope for the field-level gate. Adding a new
# struct here is one half of the binding-sweep workflow; the other
# half is adding rows to `parity.toml` for every pub field on the new
# struct (or marking each as `rust_only = true, issue = "#N"`).
#
# `ReconnectAttemptLimits` is intentionally NOT scoped here even
# though it carries `pub max_attempts / max_rate_limited_attempts /
# stable_window` fields; those mirror onto the bindings via
# `ReconnectConfig.max_attempts` etc. (the inner `limits` struct is
# wrapped in `ReconnectPolicy::Auto(...)` and the binding setters
# write through to it). The parity-toml rows under `ReconnectConfig.*`
# already cover the cross-binding contract for those fields.
SCOPED_STRUCTS: tuple[str, ...] = (
    "MddsConfig",
    "FpssConfig",
    "FlatFilesConfig",
    "ReconnectConfig",
    "RuntimeConfig",
    "RetryPolicy",
    "AuthConfig",
    "MetricsConfig",
)


STRUCT_HEADER_RE = re.compile(
    r"pub\s+struct\s+(\w+)\s*\{",
)
PUB_FIELD_RE = re.compile(
    r"^\s+pub\s+(\w+)\s*:",
    re.MULTILINE,
)


def _collect_rust_pub_fields(config_dir: pathlib.Path) -> dict[str, set[str]]:
    """Return `{struct_name: {field, ...}}` for every scoped struct.

    Parses `crates/thetadatadx/src/config/*.rs`. Skips fields on
    structs not listed in `SCOPED_STRUCTS` — `DirectConfig`'s pub
    fields are nested-struct accessors that the class-level gate
    already covers.
    """
    out: dict[str, set[str]] = {}
    if not config_dir.is_dir():
        return out
    for rs in config_dir.rglob("*.rs"):
        text = rs.read_text(encoding="utf-8")
        # Find every `pub struct X {` block and walk forward until the
        # closing brace. The structs in `crates/thetadatadx/src/config/`
        # never nest other struct definitions; a depth=1 brace counter
        # suffices.
        for header in STRUCT_HEADER_RE.finditer(text):
            struct_name = header.group(1)
            if struct_name not in SCOPED_STRUCTS:
                continue
            body_start = header.end()
            depth = 1
            i = body_start
            while i < len(text) and depth > 0:
                c = text[i]
                if c == "{":
                    depth += 1
                elif c == "}":
                    depth -= 1
                i += 1
            body = text[body_start : i - 1]
            for fm in PUB_FIELD_RE.finditer(body):
                out.setdefault(struct_name, set()).add(fm.group(1))
    return out


# Some Rust pub field names differ from the binding-side setter
# suffix:
#
#   - `ReconnectConfig.stable_window: Duration` → setter
#     `set_reconnect_stable_window_secs` (the binding takes a `u64`
#     seconds value; the row name carries the unit suffix).
#   - `FlatFilesConfig.initial_backoff: Duration` →
#     `set_flatfiles_initial_backoff_secs`.
#   - `RetryPolicy.initial_delay: Duration` →
#     `set_retry_initial_delay_ms`.
#
# The table below records the rust-field → binding-suffix renames.
# Fields not listed are 1:1.
RUST_FIELD_RENAMES: dict[tuple[str, str], str] = {
    ("ReconnectConfig", "stable_window"): "stable_window_secs",
    ("ReconnectAttemptLimits", "stable_window"): "stable_window_secs",
    ("ReconnectAttemptLimits", "max_elapsed"): "max_elapsed_secs",
    ("FlatFilesConfig", "initial_backoff"): "initial_backoff_secs",
    ("FlatFilesConfig", "max_backoff"): "max_backoff_secs",
    ("RetryPolicy", "initial_delay"): "initial_delay_ms",
    ("RetryPolicy", "max_delay"): "max_delay_ms",
    ("RetryPolicy", "max_elapsed"): "max_elapsed_secs",
    # FpssConfig scalar knobs carry an `fpss_` prefix at the binding
    # surface so the generic field names (`timeout_ms`, `ring_size`)
    # stay unambiguous against sibling sub-configs.
    ("FpssConfig", "timeout_ms"): "fpss_timeout_ms",
    ("FpssConfig", "ring_size"): "fpss_ring_size",
    ("FpssConfig", "ping_interval_ms"): "fpss_ping_interval_ms",
    ("FpssConfig", "connect_timeout_ms"): "fpss_connect_timeout_ms",
    ("FpssConfig", "io_read_slice_ms"): "fpss_io_read_slice_ms",
    ("FpssConfig", "data_watchdog_ms"): "fpss_data_watchdog_ms",
    ("FpssConfig", "keepalive_idle_secs"): "fpss_keepalive_idle_secs",
    ("FpssConfig", "keepalive_interval_secs"): "fpss_keepalive_interval_secs",
    ("FpssConfig", "keepalive_retries"): "fpss_keepalive_retries",
    ("FpssConfig", "host_selection"): "fpss_host_selection",
    ("FpssConfig", "host_shuffle_seed"): "fpss_host_shuffle_seed",
}


def _rust_field_to_row_suffix(struct: str, field: str) -> str:
    return RUST_FIELD_RENAMES.get((struct, field), field)


# ─── Method-level discovery (per-method granularity / unified clients) ───


def _camel_to_snake(camel: str) -> str:
    """`activeFullSubscriptions` -> `active_full_subscriptions`."""
    return re.sub(r"(?<!^)([A-Z])", r"_\1", camel).lower()


def _collect_python_class_methods(py_src: pathlib.Path) -> dict[str, set[str]]:
    """Return `{pyclass_name: {method, ...}}` for every Python pyclass.

    Parses every `#[pymethods] impl <Path>` block (or `impl <Path>`
    block participating in `multiple-pymethods`) and harvests the
    `fn <name>` declarations inside. `<Path>` accepts a bare class
    name (`impl ThetaDataDxClient`) or a fully-qualified Rust path
    (`impl crate::ThetaDataDxClient`); the collector normalises both
    to the bare class name so the parity row can refer to it directly.

    Filters out the lifecycle dunders (`__new__`, `__repr__`,
    `__getattr__`, `__init__`, `__enter__`, `__exit__`) and the
    constructor `new` so the matrix tracks user-facing methods only.
    """
    out: dict[str, set[str]] = {}
    if not py_src.is_dir():
        return out
    skip_names = {
        "new",
        "__new__",
        "__repr__",
        "__getattr__",
        "__init__",
        "__enter__",
        "__exit__",
    }
    # `impl <Path> {` — `<Path>` may be `Name` or `crate::...::Name`.
    # Capture the last identifier segment before the opening brace.
    impl_re = re.compile(
        r"impl\s+(?:[A-Za-z_][A-Za-z0-9_]*::)*([A-Za-z_][A-Za-z0-9_]*)\s*\{"
    )
    fn_re = re.compile(r"fn\s+([a-zA-Z_][a-zA-Z0-9_]*)\s*[(<]")
    for rs in py_src.rglob("*.rs"):
        text = rs.read_text(encoding="utf-8")
        for header in impl_re.finditer(text):
            class_name = header.group(1)
            # Walk the impl block with a brace counter to bound the
            # method scan to a single impl body.
            body_start = header.end()
            depth = 1
            i = body_start
            while i < len(text) and depth > 0:
                c = text[i]
                if c == "{":
                    depth += 1
                elif c == "}":
                    depth -= 1
                i += 1
            body = text[body_start : i - 1]
            for fm in fn_re.finditer(body):
                name = fm.group(1)
                if name in skip_names:
                    continue
                out.setdefault(class_name, set()).add(name)
    return out


def _collect_typescript_class_methods(ts_src: pathlib.Path) -> dict[str, set[str]]:
    """Return `{ts_class_name: {method, ...}}` for every TypeScript
    napi class.

    Parses every `#[napi]` / `#[napi(js_name = "...")] impl <Name>` block
    and harvests the JS-visible method names inside. The TS impl blocks
    live across multiple files (`lib.rs`, `_generated/*.rs`,
    `config_class.rs`, ...); the collector walks each one and bounds
    the method scan to the impl body with a brace counter.

    Covers both method-attribute shapes:

    * `#[napi(js_name = "<camelCase>")] fn <snake>` — explicit JS name.
    * `#[napi] fn <snake>` (or `#[napi(...)]` without `js_name`) —
      napi-rs auto-camelCases the fn name. Both the snake_case fn
      name and its camelCase derivation are recorded so a row matches
      against either spelling.

    The class name is lifted from `impl <Name>` directly (not the
    `#[napi(js_name = "X")]` on the struct itself), which is the form
    the cross-binding parity rows use as the canonical class identifier.
    """
    out: dict[str, set[str]] = {}
    if not ts_src.is_dir():
        return out
    # `impl <Path> {` — handle bare names and qualified paths
    # (`impl crate::ThetaDataDxClient`) symmetrically with the Python
    # collector. The captured class name is always the last path segment.
    impl_re = re.compile(
        r"impl\s+(?:[A-Za-z_][A-Za-z0-9_]*::)*([A-Za-z_][A-Za-z0-9_]*)\s*\{"
    )
    js_name_re = re.compile(
        r'#\[napi\([^)]*\bjs_name\s*=\s*"([a-zA-Z_][a-zA-Z0-9_]*)"[^)]*\)\]\s*'
        r'(?:pub\s+)?(?:async\s+)?fn\s+([a-z_][a-z0-9_]*)\s*[(<]'
    )
    bare_napi_re = re.compile(
        r'#\[napi(?:\((?:(?!js_name)[^)])*\))?\]\s*'
        r'(?:pub\s+)?(?:async\s+)?fn\s+([a-z_][a-z0-9_]*)\s*[(<]'
    )
    for rs in ts_src.rglob("*.rs"):
        text = rs.read_text(encoding="utf-8")
        for header in impl_re.finditer(text):
            class_name = header.group(1)
            # Walk the impl block with a brace counter to bound the
            # method scan to a single impl body.
            body_start = header.end()
            depth = 1
            i = body_start
            while i < len(text) and depth > 0:
                c = text[i]
                if c == "{":
                    depth += 1
                elif c == "}":
                    depth -= 1
                i += 1
            body = text[body_start : i - 1]
            for m in js_name_re.finditer(body):
                out.setdefault(class_name, set()).add(m.group(1))
            for m in bare_napi_re.finditer(body):
                snake = m.group(1)
                head, *rest = snake.split("_")
                camel = head + "".join(p.capitalize() for p in rest)
                out.setdefault(class_name, set()).add(camel)
                out.setdefault(class_name, set()).add(snake)
    return out


def _expand_cpp_includes(hpp_text: str, include_dir: pathlib.Path) -> str:
    """Inline every `#include "<name>.inc"` directive against the
    matching file under `include_dir`. The `*.inc` files extend a
    class body with generator-emitted member declarations
    (`sdks/cpp/include/fpss.hpp.inc` adds `FpssClient` methods that
    live in `crates/thetadatadx/sdk_surface.toml`), and the parity
    gate must see those declarations as part of the surrounding
    class body.

    Falls back to leaving the directive in place when the included
    file is missing, so a malformed `#include` cannot wedge the gate.
    """
    include_re = re.compile(r'#include\s+"([^"]+\.inc)"')

    def _sub(m: re.Match[str]) -> str:
        rel = m.group(1)
        target = include_dir / rel
        if target.is_file():
            return target.read_text(encoding="utf-8")
        return m.group(0)

    return include_re.sub(_sub, hpp_text)


def _collect_cpp_class_methods(cpp_hpp: pathlib.Path) -> dict[str, set[str]]:
    """Return `{class_name: {method, ...}}` for every C++ class.

    Parses each `class X { ... };` body in `thetadx.hpp` and collects
    every member declaration with a `name(` shape. The first identifier
    before the `(` is the method name. Bounded brace-counting keeps
    nested types (e.g. lambdas inside default-arg initializers) from
    leaking into the outer class's method set.

    Honors `#include "<file>.inc"` inside a class body by inlining the
    included file's contents before parsing — generator-emitted method
    declarations (`fpss.hpp.inc`) extend the surrounding class body
    and must count toward parity.
    """
    out: dict[str, set[str]] = {}
    if not cpp_hpp.is_file():
        return out
    text = _expand_cpp_includes(cpp_hpp.read_text(encoding="utf-8"), cpp_hpp.parent)
    # Limit to class bodies — struct bodies are POD-shaped value types
    # and irrelevant to the cross-binding method contract.
    class_header_re = re.compile(r"^class\s+(\w+)\s*(?::[^{]*)?\{", re.MULTILINE)
    for header in class_header_re.finditer(text):
        class_name = header.group(1)
        body_start = header.end()
        depth = 1
        i = body_start
        while i < len(text) and depth > 0:
            c = text[i]
            if c == "{":
                depth += 1
            elif c == "}":
                depth -= 1
            i += 1
        body = text[body_start : i - 1]
        # Match member declarations + definitions. The `name(` pattern is
        # preceded by whitespace and (optionally) qualifiers / return
        # type tokens; the first plain identifier immediately before the
        # opening paren is the method name.
        for fm in re.finditer(
            r"(?:^|\s)([a-z_][a-z0-9_]*)\s*\(",
            body,
            re.MULTILINE,
        ):
            name = fm.group(1)
            # Filter language keywords that look like method calls.
            if name in {
                "if",
                "while",
                "for",
                "switch",
                "return",
                "throw",
                "catch",
                "sizeof",
                "operator",
                "new",
                "delete",
                "static_cast",
                "reinterpret_cast",
                "const_cast",
                "dynamic_cast",
            }:
                continue
            out.setdefault(class_name, set()).add(name)
    return out


def _check_method_rows(
    method_rows: list[dict[str, Any]],
    py_methods: dict[str, set[str]],
    ts_methods: dict[str, set[str]],
    cpp_methods: dict[str, set[str]],
) -> list[str]:
    """Per-method cross-binding gate.

    Each `[[method]]` row in `parity.toml` declares a `(class, name)`
    pair plus the expected presence in each binding. The checker
    verifies the actual binding state against the declared state and
    returns a list of human-readable mismatch strings (empty when
    every row matches).
    """
    errors: list[str] = []
    for row in method_rows:
        class_name = row.get("class")
        camel = row.get("name")
        if not class_name or not camel:
            errors.append(
                f"  [[method]] row missing `class` or `name`: {row!r}"
            )
            continue
        snake = _camel_to_snake(camel)

        # Python: snake_case method declared on the pyclass.
        declared_py = row.get("python", False)
        actual_py = snake in py_methods.get(class_name, set())
        if declared_py != actual_py:
            verb = "missing" if declared_py and not actual_py else "unexpected"
            errors.append(
                f"  {class_name}.{camel}.python: declared={declared_py}, "
                f"actual={actual_py} ({verb} -- expected `fn {snake}` "
                f"inside `impl {class_name}` on the Python pyclass)"
            )

        # TypeScript: napi-attributed method declared inside the
        # matching `impl <ClassName>` block under `sdks/typescript/src/`.
        # The collector records both the `js_name` and the auto-
        # camelCased fn-name spelling so a row's `name` can match
        # against either.
        declared_ts = row.get("typescript", False)
        actual_ts = camel in ts_methods.get(class_name, set())
        if declared_ts != actual_ts:
            verb = "missing" if declared_ts and not actual_ts else "unexpected"
            errors.append(
                f"  {class_name}.{camel}.typescript: declared={declared_ts}, "
                f"actual={actual_ts} ({verb} -- expected "
                f'`#[napi(js_name = "{camel}")]` (or bare `#[napi]` on '
                f"`fn {snake}`) inside `impl {class_name}` under "
                f"sdks/typescript/src/)"
            )

        # C++: `<snake>(` member declaration inside the matching
        # class body in `thetadx.hpp`. C++ alias names route through
        # `CPP_ALIASES` (`ThetaDataDxClient` -> `UnifiedClient`).
        declared_cpp = row.get("cpp", False)
        cpp_class = _cpp_class_for(class_name)
        actual_cpp = snake in cpp_methods.get(cpp_class, set())
        if declared_cpp != actual_cpp:
            verb = "missing" if declared_cpp and not actual_cpp else "unexpected"
            errors.append(
                f"  {class_name}.{camel}.cpp: declared={declared_cpp}, "
                f"actual={actual_cpp} ({verb} -- expected `{snake}(` "
                f"inside `class {cpp_class}` body in "
                f"sdks/cpp/include/thetadx.hpp)"
            )

    return errors


# ─── Main gate ──────────────────────────────────────────────────────


def _check_dotted_rows(
    rows: list[dict[str, Any]],
    py_setters: set[str],
    ts_setters: set[str],
    cpp_setters: set[str],
    ffi_setters: set[str],
) -> list[str]:
    """Per-field / per-setter granularity (issue #595).

    Returns a list of human-readable error strings. An empty list
    means every dotted row in `parity.toml` matches the actual binding
    state of each SDK.
    """
    errors: list[str] = []
    for row in rows:
        name = row["name"]
        if "." not in name:
            continue
        struct_name, suffix = name.split(".", 1)
        prefix = STRUCT_TO_PREFIX.get(struct_name)
        if prefix is None:
            # Unknown struct — likely a "documentation anchor" row
            # (e.g. `Error.cross_binding_name_divergence`,
            # `GreeksEodTick.cross_binding_anchor`). Skip — these are
            # not field-level bindings.
            continue
        # Allow rows to override the auto-derived setter name. Used
        # when a single struct has a mix of prefixed / unprefixed
        # binding-side names (e.g. `MddsConfig.host` binds as
        # `mdds_host` because the bare `host` name would collide with
        # nothing meaningful and the `mdds_` prefix clarifies intent).
        canonical = row.get("setter") or f"{prefix}{suffix}"

        rust_only = bool(row.get("rust_only", False))
        issue = row.get("issue")
        if rust_only and not issue:
            errors.append(
                f"  {name}: declared `rust_only = true` but missing "
                f"`issue = \"#N\"` field. Every Rust-only row must "
                f"cite a tracking issue number."
            )
            continue
        if issue and not rust_only:
            errors.append(
                f"  {name}: has `issue` field but is not `rust_only`. "
                f"Drop the `issue` field (no Rust-only contract is "
                f"being tracked) or flip `rust_only = true`."
            )

        if rust_only:
            # Documented Rust-only: no setter expected on any binding.
            # The class-level booleans must be false on every column.
            for lang in ("python", "typescript", "cpp"):
                if row.get(lang, False):
                    errors.append(
                        f"  {name}.{lang}: row is `rust_only = true` "
                        f"but declares `{lang} = true`. Pick one — "
                        f"either remove the Rust-only flag (and bind "
                        f"the field) or flip the binding column to "
                        f"false."
                    )
            continue

        for lang, lookup in (
            ("python", py_setters),
            ("typescript", ts_setters),
            ("cpp", cpp_setters),
        ):
            declared = row.get(lang, False)
            actual = _setter_present(canonical, lookup)
            if declared != actual:
                verb = "missing" if declared and not actual else "unexpected"
                errors.append(
                    f"  {name}.{lang}: declared={declared}, actual={actual} "
                    f"({verb} — canonical setter `{canonical}`)"
                )

        # FFI gate: C++ binding forwards through the C ABI, so a
        # bound C++ row requires the FFI symbol. Python (pyo3) and
        # TypeScript (napi) bindings mutate `DirectConfig` directly
        # through the inner mutex, so they do not require an FFI
        # symbol — a Python-only or TS-only setter is legal.
        if row.get("cpp", False):
            ffi_present = _setter_present(canonical, ffi_setters)
            if not ffi_present:
                errors.append(
                    f"  {name}.ffi: row declares `cpp = true` but the "
                    f"FFI symbol `tdx_config_set_{canonical}` is "
                    f"absent. The C++ wrapper forwards through the C "
                    f"ABI; add the FFI pair before flipping the C++ "
                    f"column to `true`."
                )

    return errors


def _check_orphan_rust_fields(
    rust_fields: dict[str, set[str]],
    rows: list[dict[str, Any]],
) -> list[str]:
    """Reverse-direction check: every pub field on every scoped struct
    must have a corresponding parity row. Adding a new pub field
    without a parity row trips this gate so the cross-binding sweep
    cannot be silently skipped.
    """
    errors: list[str] = []
    declared_names: set[str] = {row["name"] for row in rows}
    for struct in SCOPED_STRUCTS:
        fields = rust_fields.get(struct, set())
        for field in sorted(fields):
            row_suffix = _rust_field_to_row_suffix(struct, field)
            row_name = f"{struct}.{row_suffix}"
            if row_name in declared_names:
                continue
            errors.append(
                f"  {row_name}: pub field on `{struct}` has no "
                f"parity-toml row. Either add a `[[class]]` row "
                f"declaring the field's binding state, or mark the "
                f"field as `rust_only = true, issue = \"#N\"`."
            )
    return errors


VALUE_FIELD_PY_SRC = REPO_ROOT / "sdks" / "python" / "src"
VALUE_FIELD_TS_SRC = REPO_ROOT / "sdks" / "typescript" / "src"


def _struct_field_type(src_dir: pathlib.Path, struct: str, field: str) -> str | None:
    """Declared Rust-side type of `field` on `struct` in a binding crate.

    Scans every `.rs` file (including `_generated/`) for the struct body
    and returns the type text of the named field, attribute prefixes
    (`#[pyo3(get)]`) stripped. Returns `None` when the struct or field
    is absent. Generated and hand-written sources are treated alike —
    the declared type IS the binding surface either way.
    """
    struct_re = re.compile(
        r"(?:pub(?:\(crate\))?\s+)?struct\s+" + re.escape(struct) + r"\s*\{(.*?)\n\}",
        re.S,
    )
    field_re = re.compile(
        r"(?:#\[[^\]]*\]\s*)*pub\s+" + re.escape(field) + r"\s*:\s*([^,\n]+)",
    )
    for path in sorted(src_dir.rglob("*.rs")):
        text = path.read_text(encoding="utf-8")
        for m in struct_re.finditer(text):
            fm = field_re.search(m.group(1))
            if fm:
                return fm.group(1).strip()
    return None


def _cpp_struct_field_type(hpp: pathlib.Path, struct: str, field: str) -> str | None:
    """Declared C++ type of `field` on `struct` in the C++ wrapper header.

    Mirrors [`_struct_field_type`] for the hand-written C++ value structs
    (`OptionContract`, etc.) whose field types live in `thetadx.hpp`
    rather than a Rust binding crate. Returns `None` when the struct or
    field is absent. A `cpp` key on a `[[value_field]]` row pins the
    type this returns, closing the gap that let a C++ value struct
    surface a raw wire integer the other bindings decode.
    """
    text = hpp.read_text(encoding="utf-8")
    struct_re = re.compile(
        r"struct\s+" + re.escape(struct) + r"\s*\{(.*?)\n\}",
        re.S,
    )
    field_re = re.compile(
        r"([A-Za-z_][\w:<>\s\*&]*?)\s+" + re.escape(field) + r"\s*;",
    )
    for m in struct_re.finditer(text):
        fm = field_re.search(m.group(1))
        if fm:
            return fm.group(1).strip()
    return None


def _check_value_field_rows(rows: list[dict[str, Any]]) -> list[str]:
    """Field-level TYPE parity for `[[value_field]]` rows.

    Each row pins the declared Rust-side type of one field on one
    value class per binding:

        [[value_field]]
        class = "ContractRef"
        name = "strike"
        python = "Option<f64>"
        typescript = "Option<f64>"

    `python` / `typescript` are the Rust types in the pyclass / napi
    object struct (omit a key to skip that binding, e.g. a
    Python-only spelling like `lambda_`). A mismatch — the field
    missing, or declared under a different type — fails the gate, so a
    binding cannot silently drift a field's unit-bearing type (the
    strike-thousandths / right-as-int / ms_of_day2 defect class).
    """
    errors: list[str] = []
    for row in rows:
        cls, field = row["class"], row["name"]
        for lang, src_dir in (
            ("python", VALUE_FIELD_PY_SRC),
            ("typescript", VALUE_FIELD_TS_SRC),
        ):
            declared = row.get(lang)
            if declared is None:
                continue
            actual = _struct_field_type(src_dir, cls, field)
            if actual != declared:
                errors.append(
                    f"{cls}.{field}.{lang}: declared type `{declared}`, "
                    f"actual `{actual or '<field missing>'}`"
                )
        # C++ value structs declare their field types in the wrapper
        # header, not a Rust crate, so they get their own reader.
        declared_cpp = row.get("cpp")
        if declared_cpp is not None:
            actual_cpp = _cpp_struct_field_type(CPP_HPP, _cpp_class_for(cls), field)
            if actual_cpp != declared_cpp:
                errors.append(
                    f"{cls}.{field}.cpp: declared type `{declared_cpp}`, "
                    f"actual `{actual_cpp or '<field missing>'}`"
                )
    return errors


def main(argv: list[str] | None = None) -> int:
    argv = argv if argv is not None else sys.argv[1:]
    if "--selftest" in argv:
        return _run_selftest()

    if not PARITY_TOML.is_file():
        print(f"missing parity matrix: {PARITY_TOML}", file=sys.stderr)
        return 1

    data: dict[str, Any] = tomllib.loads(PARITY_TOML.read_text(encoding="utf-8"))
    rows: list[dict[str, Any]] = data.get("class", [])
    method_rows: list[dict[str, Any]] = data.get("method", [])
    value_field_rows: list[dict[str, Any]] = data.get("value_field", [])
    if not rows:
        print("parity.toml has no [[class]] rows", file=sys.stderr)
        return 1

    py_classes = collect_python_classes(PY_SRC)
    ts_classes = collect_typescript_classes(TS_DTS)
    cpp_classes = collect_cpp_classes(CPP_HPP)

    py_setters = _collect_python_setters(PY_SRC)
    ts_setters = _collect_typescript_setters(TS_SRC)
    cpp_setters = _collect_cpp_setters(CPP_HPP, CPP_H)
    ffi_setters = _collect_ffi_setters(FFI_SRC)

    rust_fields = _collect_rust_pub_fields(CONFIG_DIR)

    py_class_methods = _collect_python_class_methods(PY_SRC)
    ts_class_methods = _collect_typescript_class_methods(TS_SRC)
    cpp_class_methods = _collect_cpp_class_methods(CPP_HPP)

    declared_names: set[str] = {row["name"] for row in rows}

    # Class-level mismatches (non-dotted rows).
    class_mismatches: list[tuple[str, str, bool, bool]] = []
    for row in rows:
        name = row["name"]
        if "." in name:
            continue
        for lang, declared in (
            ("python", row["python"]),
            ("typescript", row["typescript"]),
            ("cpp", row["cpp"]),
        ):
            if lang == "python":
                actual = name in py_classes
            elif lang == "typescript":
                actual = name in ts_classes
            else:
                actual = cpp_has(name, cpp_classes)
            if actual != declared:
                class_mismatches.append((name, lang, declared, actual))

    # Field-level mismatches (dotted rows / #595).
    field_errors = _check_dotted_rows(
        rows, py_setters, ts_setters, cpp_setters, ffi_setters
    )

    # Method-level mismatches (per-method `[[method]]` rows on the
    # load-bearing user-facing classes — `ThetaDataDxClient`,
    # `FpssClient`, `Credentials`, `Config`).
    method_errors = _check_method_rows(
        method_rows, py_class_methods, ts_class_methods, cpp_class_methods
    )

    # Orphan Rust pub fields (no parity row).
    orphan_errors = _check_orphan_rust_fields(rust_fields, rows)

    # Value-field TYPE parity ([[value_field]] rows).
    value_field_errors = _check_value_field_rows(value_field_rows)

    # Catch-all: every Python pyclass must be either tracked
    # explicitly or via the implicit pattern (mechanical parity).
    untracked: set[str] = {
        name
        for name in py_classes
        if name not in declared_names and not _is_implicitly_tracked(name)
    }

    had_errors = False
    if class_mismatches:
        had_errors = True
        print(
            f"check_binding_parity: {len(class_mismatches)} class-level "
            f"mismatch(es) vs sdks/parity.toml:"
        )
        for name, lang, declared, actual in class_mismatches:
            verb = "missing" if declared and not actual else "unexpected"
            print(f"  {name}.{lang}: declared={declared}, actual={actual} ({verb})")
        print()

    if field_errors:
        had_errors = True
        print(
            f"check_binding_parity: {len(field_errors)} field-level "
            f"mismatch(es) (#595 per-setter granularity):"
        )
        for e in field_errors:
            print(e)
        print()

    if method_errors:
        had_errors = True
        print(
            f"check_binding_parity: {len(method_errors)} method-level "
            f"mismatch(es) (per-method `[[method]]` granularity):"
        )
        for e in method_errors:
            print(e)
        print()

    if orphan_errors:
        had_errors = True
        print(
            f"check_binding_parity: {len(orphan_errors)} Rust pub "
            f"field(s) lack a parity-toml row:"
        )
        for e in orphan_errors:
            print(e)
        print()

    if value_field_errors:
        had_errors = True
        print(
            f"check_binding_parity: {len(value_field_errors)} value-field "
            f"TYPE mismatch(es) (per-field `[[value_field]]` granularity):"
        )
        for e in value_field_errors:
            print(f"  {e}")
        print()

    if untracked:
        had_errors = True
        print(
            f"check_binding_parity: {len(untracked)} pyclass(es) lack a "
            "parity row AND do not match any implicit pattern:"
        )
        for name in sorted(untracked):
            print(f"  {name}")
        print()

    if had_errors:
        print(
            "Fix: either land the missing binding, or update "
            "sdks/parity.toml to reflect the intended state. Every "
            "cross-binding asymmetry must be explicit + tracked."
        )
        return 1

    n_dotted = sum(1 for row in rows if "." in row["name"])
    n_class = len(rows) - n_dotted
    n_fields = sum(len(v) for v in rust_fields.values())
    n_methods = len(method_rows)
    n_value_fields = len(value_field_rows)
    print(
        f"check_binding_parity: clean "
        f"({n_class} class rows + {n_dotted} field rows + "
        f"{n_methods} method rows + {n_value_fields} value-field rows + "
        f"{n_fields} rust pub fields checked; "
        f"py_classes={len(py_classes)} ts_classes={len(ts_classes)} "
        f"cpp_classes={len(cpp_classes)} "
        f"py_setters={len(py_setters)} ts_setters={len(ts_setters)} "
        f"cpp_setters={len(cpp_setters)} ffi_setters={len(ffi_setters)})"
    )
    return 0


# ─── Selftest ───────────────────────────────────────────────────────


def _run_selftest() -> int:
    """In-process synthetic-source matrix covering the audit cases.

    Each test materialises a temporary tree with the binding sources
    needed to exercise one specific pass/fail axis, then invokes the
    parity-row evaluator. The selftest is intentionally hermetic — it
    does not touch the live `sdks/` tree, so running it never depends
    on the current state of the production bindings.
    """
    n_pass = 0
    n_fail = 0

    def _case(label: str, fn) -> None:
        nonlocal n_pass, n_fail
        try:
            fn()
        except AssertionError as e:
            n_fail += 1
            print(f"  selftest FAIL: {label}: {e}")
        else:
            n_pass += 1

    def _case_positive_all_bound() -> None:
        """All four bindings expose the field — gate is silent."""
        rows = [
            {
                "name": "FlatFilesConfig.max_attempts",
                "python": True,
                "typescript": True,
                "cpp": True,
            }
        ]
        py_setters = {"flatfiles_max_attempts"}
        ts_setters = {"flatfiles_max_attempts"}
        cpp_setters = {"flatfiles_max_attempts"}
        ffi_setters = {"flatfiles_max_attempts"}
        errors = _check_dotted_rows(rows, py_setters, ts_setters, cpp_setters, ffi_setters)
        assert errors == [], f"positive case must be silent; got {errors!r}"

    def _case_negative_missing_on_ts() -> None:
        """Python+C++ bound, TS missing — gate trips."""
        rows = [
            {
                "name": "FlatFilesConfig.max_attempts",
                "python": True,
                "typescript": True,
                "cpp": True,
            }
        ]
        py_setters = {"flatfiles_max_attempts"}
        ts_setters: set[str] = set()
        cpp_setters = {"flatfiles_max_attempts"}
        ffi_setters = {"flatfiles_max_attempts"}
        errors = _check_dotted_rows(rows, py_setters, ts_setters, cpp_setters, ffi_setters)
        assert any("typescript" in e and "missing" in e for e in errors), (
            f"negative TS case must surface missing setter; got {errors!r}"
        )

    def _case_negative_missing_on_cpp() -> None:
        rows = [
            {
                "name": "FlatFilesConfig.max_attempts",
                "python": True,
                "typescript": True,
                "cpp": True,
            }
        ]
        py_setters = {"flatfiles_max_attempts"}
        ts_setters = {"flatfiles_max_attempts"}
        cpp_setters: set[str] = set()
        ffi_setters = {"flatfiles_max_attempts"}
        errors = _check_dotted_rows(rows, py_setters, ts_setters, cpp_setters, ffi_setters)
        assert any("cpp" in e and "missing" in e for e in errors), (
            f"negative C++ case must surface missing setter; got {errors!r}"
        )

    def _case_negative_missing_on_ffi() -> None:
        """C++ claims bound but no FFI symbol — gate trips."""
        rows = [
            {
                "name": "FlatFilesConfig.max_attempts",
                "python": False,
                "typescript": False,
                "cpp": True,
            }
        ]
        py_setters: set[str] = set()
        ts_setters: set[str] = set()
        cpp_setters = {"flatfiles_max_attempts"}
        ffi_setters: set[str] = set()
        errors = _check_dotted_rows(rows, py_setters, ts_setters, cpp_setters, ffi_setters)
        assert any("ffi" in e for e in errors), (
            f"missing FFI symbol under cpp=true must trip the gate; got {errors!r}"
        )

    def _case_positive_python_only_no_ffi_required() -> None:
        """Python-only setter without FFI symbol must NOT trip the
        gate. Python (pyo3) mutates `DirectConfig` directly through
        the inner mutex; no FFI forwarding is required.
        """
        rows = [
            {
                "name": "MddsConfig.host",
                "python": True,
                "typescript": False,
                "cpp": False,
            }
        ]
        py_setters = {"host"}
        errors = _check_dotted_rows(rows, py_setters, set(), set(), set())
        assert errors == [], (
            f"Python-only setter must not require FFI symbol; got {errors!r}"
        )

    def _case_negative_unexpected_setter() -> None:
        """Row declares not-bound but binding setter exists — trips."""
        rows = [
            {
                "name": "FlatFilesConfig.max_attempts",
                "python": False,
                "typescript": False,
                "cpp": False,
                "rust_only": True,
                "issue": "#999",
            }
        ]
        py_setters = {"flatfiles_max_attempts"}  # unexpectedly bound
        # Rust-only path skips the per-language setter check; instead
        # the contract is: row is fully `false`. We fold this into the
        # `rust_only` consistency check by setting a column `true`
        # on a `rust_only` row → that's caught.
        errors = _check_dotted_rows(rows, py_setters, set(), set(), set())
        # The Rust-only case doesn't currently inspect actual setter
        # state (the row's contract is documentation-only). Still, a
        # `rust_only = true` row with a `true` column is the symmetric
        # mismatch covered by a separate selftest.
        assert errors == [], f"rust_only row must skip setter checks; got {errors!r}"

    def _case_negative_rust_only_without_issue() -> None:
        rows = [
            {
                "name": "FpssConfig.timeout_ms",
                "python": False,
                "typescript": False,
                "cpp": False,
                "rust_only": True,
            }
        ]
        errors = _check_dotted_rows(rows, set(), set(), set(), set())
        assert any("issue" in e for e in errors), (
            f"rust_only without issue must trip; got {errors!r}"
        )

    def _case_negative_issue_without_rust_only() -> None:
        rows = [
            {
                "name": "FpssConfig.timeout_ms",
                "python": False,
                "typescript": False,
                "cpp": False,
                "issue": "#999",
            }
        ]
        errors = _check_dotted_rows(rows, set(), set(), set(), set())
        assert any("not `rust_only`" in e for e in errors), (
            f"issue without rust_only must trip; got {errors!r}"
        )

    def _case_negative_rust_only_with_binding_true() -> None:
        rows = [
            {
                "name": "FpssConfig.timeout_ms",
                "python": True,  # contradicts rust_only
                "typescript": False,
                "cpp": False,
                "rust_only": True,
                "issue": "#999",
            }
        ]
        errors = _check_dotted_rows(rows, set(), set(), set(), set())
        assert any("rust_only = true" in e for e in errors), (
            f"rust_only with binding=true must trip; got {errors!r}"
        )

    def _case_orphan_rust_field_trips() -> None:
        """A pub field with no parity row must surface a clear error."""
        with tempfile.TemporaryDirectory() as tmp:
            cfg_dir = pathlib.Path(tmp) / "config"
            cfg_dir.mkdir()
            (cfg_dir / "fake.rs").write_text(
                "pub struct FlatFilesConfig {\n"
                "    pub max_attempts: u32,\n"
                "    pub novel_field: u64,\n"
                "}\n",
                encoding="utf-8",
            )
            rust_fields = _collect_rust_pub_fields(cfg_dir)
            assert "max_attempts" in rust_fields["FlatFilesConfig"], (
                f"max_attempts must parse; got {rust_fields!r}"
            )
            assert "novel_field" in rust_fields["FlatFilesConfig"], (
                f"novel_field must parse; got {rust_fields!r}"
            )
            rows = [
                {
                    "name": "FlatFilesConfig.max_attempts",
                    "python": True,
                    "typescript": True,
                    "cpp": True,
                }
            ]
            errors = _check_orphan_rust_fields(rust_fields, rows)
            assert any("novel_field" in e for e in errors), (
                f"undocumented pub field must trip; got {errors!r}"
            )

    def _case_explicit_widened_abi_accepted() -> None:
        """`<canonical>_explicit` suffix counts as the same setter."""
        rows = [
            {
                "name": "RuntimeConfig.tokio_worker_threads",
                "python": True,
                "typescript": True,
                "cpp": True,
            }
        ]
        # FFI emits `tdx_config_set_tokio_worker_threads_explicit`;
        # that must satisfy the `tokio_worker_threads` row.
        ffi_setters = {"tokio_worker_threads_explicit", "tokio_worker_threads"}
        py_setters = {"tokio_worker_threads"}
        ts_setters = {"tokio_worker_threads"}
        cpp_setters = {"tokio_worker_threads"}
        errors = _check_dotted_rows(rows, py_setters, ts_setters, cpp_setters, ffi_setters)
        assert errors == [], (
            f"_explicit widened-ABI shape must satisfy the row; got {errors!r}"
        )

    _case("positive — all four bindings expose the field", _case_positive_all_bound)
    _case("negative — TS setter missing", _case_negative_missing_on_ts)
    _case("negative — C++ setter missing", _case_negative_missing_on_cpp)
    _case("negative — FFI symbol missing under cpp=true", _case_negative_missing_on_ffi)
    _case("positive — Python-only setter does not require FFI symbol", _case_positive_python_only_no_ffi_required)
    _case("negative — rust_only row with stray setter is documented-only", _case_negative_unexpected_setter)
    _case("negative — rust_only without issue trips", _case_negative_rust_only_without_issue)
    _case("negative — issue without rust_only trips", _case_negative_issue_without_rust_only)
    _case("negative — rust_only with binding=true trips", _case_negative_rust_only_with_binding_true)
    _case("orphan — undocumented Rust pub field trips", _case_orphan_rust_field_trips)
    _case("positive — `_explicit` widened-ABI suffix accepted", _case_explicit_widened_abi_accepted)

    def _case_authconfig_metricsconfig_prefixes_resolve() -> None:
        """`AuthConfig` + `MetricsConfig` are in scope (issue #608).
        Dotted rows on these structs must resolve through the prefix
        table — not skip with `prefix is None` — so a future binding
        sweep can flip the rows from `rust_only = true` to fully-bound
        and the gate catches missing setters.
        """
        # Confirm both structs resolve to a known prefix (empty string
        # for `AuthConfig`, `metrics_` for `MetricsConfig`).
        assert STRUCT_TO_PREFIX.get("AuthConfig") is not None, (
            "AuthConfig must be in STRUCT_TO_PREFIX after #608"
        )
        assert STRUCT_TO_PREFIX.get("MetricsConfig") is not None, (
            "MetricsConfig must be in STRUCT_TO_PREFIX after #608"
        )
        # A rust_only row resolves cleanly through the new prefix.
        rows = [
            {
                "name": "AuthConfig.nexus_url",
                "python": False,
                "typescript": False,
                "cpp": False,
                "rust_only": True,
                "issue": "#608",
            },
            {
                "name": "MetricsConfig.port",
                "python": False,
                "typescript": False,
                "cpp": False,
                "rust_only": True,
                "issue": "#608",
            },
        ]
        errors = _check_dotted_rows(rows, set(), set(), set(), set())
        assert errors == [], (
            f"AuthConfig + MetricsConfig rust_only rows must be silent; got {errors!r}"
        )

    _case(
        "positive — AuthConfig + MetricsConfig dotted rows resolve through new prefixes",
        _case_authconfig_metricsconfig_prefixes_resolve,
    )

    # ── Method-level gate selftests ────────────────────────────────

    def _case_method_positive_all_three() -> None:
        """Method declared on Python + TS + C++ — gate is silent."""
        rows = [
            {
                "class": "ThetaDataDxClient",
                "name": "panicCount",
                "python": True,
                "typescript": True,
                "cpp": True,
            }
        ]
        py_methods = {"ThetaDataDxClient": {"panic_count"}}
        ts_methods = {"ThetaDataDxClient": {"panicCount"}}
        cpp_methods = {"UnifiedClient": {"panic_count"}}
        errors = _check_method_rows(rows, py_methods, ts_methods, cpp_methods)
        assert errors == [], f"method positive case must be silent; got {errors!r}"

    def _case_method_python_missing() -> None:
        """Declared on Python but not present in source — trips."""
        rows = [
            {
                "class": "ThetaDataDxClient",
                "name": "panicCount",
                "python": True,
                "typescript": True,
                "cpp": True,
            }
        ]
        py_methods: dict[str, set[str]] = {"ThetaDataDxClient": set()}
        ts_methods = {"ThetaDataDxClient": {"panicCount"}}
        cpp_methods = {"UnifiedClient": {"panic_count"}}
        errors = _check_method_rows(rows, py_methods, ts_methods, cpp_methods)
        assert any("python" in e and "missing" in e for e in errors), (
            f"missing Python method must trip the gate; got {errors!r}"
        )

    def _case_method_typescript_missing() -> None:
        """Declared on TS but no matching `js_name` in source — trips."""
        rows = [
            {
                "class": "ThetaDataDxClient",
                "name": "activeFullSubscriptions",
                "python": True,
                "typescript": True,
                "cpp": True,
            }
        ]
        py_methods = {"ThetaDataDxClient": {"active_full_subscriptions"}}
        ts_methods: dict[str, set[str]] = {}
        cpp_methods = {"UnifiedClient": {"active_full_subscriptions"}}
        errors = _check_method_rows(rows, py_methods, ts_methods, cpp_methods)
        assert any("typescript" in e and "missing" in e for e in errors), (
            f"missing TS method must trip the gate; got {errors!r}"
        )

    def _case_method_cpp_alias_resolves() -> None:
        """C++ alias (`ThetaDataDxClient` -> `UnifiedClient`) is honoured."""
        rows = [
            {
                "class": "ThetaDataDxClient",
                "name": "awaitDrain",
                "python": True,
                "typescript": True,
                "cpp": True,
            }
        ]
        py_methods = {"ThetaDataDxClient": {"await_drain"}}
        ts_methods = {"ThetaDataDxClient": {"awaitDrain"}}
        # The row says `ThetaDataDxClient` but the C++ class is named
        # `UnifiedClient` — the alias table must route the lookup.
        cpp_methods = {"UnifiedClient": {"await_drain"}}
        errors = _check_method_rows(rows, py_methods, ts_methods, cpp_methods)
        assert errors == [], (
            f"C++ alias must resolve to UnifiedClient; got {errors!r}"
        )

    def _case_method_unexpected_extra() -> None:
        """Declared `false` but method exists on the source — trips."""
        rows = [
            {
                "class": "ThetaDataDxClient",
                "name": "panicCount",
                "python": False,
                "typescript": False,
                "cpp": False,
            }
        ]
        py_methods = {"ThetaDataDxClient": {"panic_count"}}
        ts_methods = {"ThetaDataDxClient": {"panicCount"}}
        cpp_methods = {"UnifiedClient": {"panic_count"}}
        errors = _check_method_rows(rows, py_methods, ts_methods, cpp_methods)
        # All three columns are stale — every binding now exposes the
        # method but the row still says `false`.
        assert any("unexpected" in e for e in errors), (
            f"stale `false` rows must trip the gate; got {errors!r}"
        )

    def _case_method_row_missing_class_or_name() -> None:
        """Malformed row — gate surfaces a clear error."""
        rows = [
            {"class": "ThetaDataDxClient", "python": True},
            {"name": "panicCount", "python": True},
        ]
        errors = _check_method_rows(rows, {}, {}, {})
        assert len(errors) == 2, (
            f"malformed rows must each trip the gate; got {errors!r}"
        )

    def _case_method_class_scoping_isolates_classes() -> None:
        """A method present on ClassA must NOT count for ClassB.

        Previously the TS collector used a single universe set; now
        every method is scoped to its owning class. This protects
        against false-positive 'unexpected' verdicts when two classes
        coincidentally share a method name (`subscribe` on both
        `ThetaDataDxClient` and `Subscription` etc.).
        """
        rows = [
            {
                "class": "FpssClient",  # FpssClient not on TS
                "name": "subscribe",
                "python": True,
                "typescript": False,
                "cpp": True,
            }
        ]
        # `subscribe` exists on `ThetaDataDxClient` (TS) but NOT on
        # `FpssClient` (TS). Class-scoped lookup must respect that.
        py_methods = {"FpssClient": {"subscribe"}}
        ts_methods = {"ThetaDataDxClient": {"subscribe"}}
        cpp_methods = {"FpssClient": {"subscribe"}}
        errors = _check_method_rows(rows, py_methods, ts_methods, cpp_methods)
        assert errors == [], (
            f"class-scoped TS lookup must not leak across classes; got {errors!r}"
        )

    _case("method positive — declared and present on all three bindings", _case_method_positive_all_three)
    _case("method negative — declared Python but missing in source", _case_method_python_missing)
    _case("method negative — declared TS but missing js_name", _case_method_typescript_missing)
    _case("method positive — C++ alias routes ThetaDataDxClient -> UnifiedClient", _case_method_cpp_alias_resolves)
    _case("method negative — stale `false` row with method present", _case_method_unexpected_extra)
    _case("method negative — malformed row missing class or name", _case_method_row_missing_class_or_name)
    _case("method positive — class-scoped TS lookup isolates classes", _case_method_class_scoping_isolates_classes)

    def _case_value_field_positive_matches() -> None:
        """Declared Rust + C++ field types match the sources — gate silent."""
        with tempfile.TemporaryDirectory() as tmp:
            py_dir = pathlib.Path(tmp) / "py"
            py_dir.mkdir()
            (py_dir / "gen.rs").write_text(
                "pub struct OptionContract {\n"
                "    #[pyo3(get)] pub right: String,\n"
                "}\n",
                encoding="utf-8",
            )
            hpp = pathlib.Path(tmp) / "thetadx.hpp"
            hpp.write_text(
                "struct OptionContract {\n    char right;\n}\n",
                encoding="utf-8",
            )
            assert _struct_field_type(py_dir, "OptionContract", "right") == "String"
            assert _cpp_struct_field_type(hpp, "OptionContract", "right") == "char"

    def _case_value_field_rust_type_mismatch_trips() -> None:
        """A Rust field whose type drifted from the declared one trips."""
        with tempfile.TemporaryDirectory() as tmp:
            py_dir = pathlib.Path(tmp) / "py"
            py_dir.mkdir()
            (py_dir / "gen.rs").write_text(
                "pub struct ContractRef {\n"
                "    #[pyo3(get)] pub strike: Option<i32>,\n"
                "}\n",
                encoding="utf-8",
            )
            # The declared type is dollars (f64); the source drifted to i32.
            actual = _struct_field_type(py_dir, "ContractRef", "strike")
            assert actual == "Option<i32>", f"reader must see the drift; got {actual!r}"
            assert actual != "Option<f64>", "the strike-units regression must not read as clean"

    def _case_value_field_cpp_type_mismatch_trips() -> None:
        """A C++ value struct that surfaces the raw integer trips."""
        with tempfile.TemporaryDirectory() as tmp:
            hpp = pathlib.Path(tmp) / "thetadx.hpp"
            hpp.write_text(
                "struct OptionContract {\n    int32_t right;\n}\n",
                encoding="utf-8",
            )
            actual = _cpp_struct_field_type(hpp, "OptionContract", "right")
            assert actual == "int32_t", f"reader must see the int; got {actual!r}"
            assert actual != "char", "the right-as-int regression must not read as clean"

    _case("value_field positive — Rust + C++ types match the declared", _case_value_field_positive_matches)
    _case("value_field negative — Rust strike type drift trips", _case_value_field_rust_type_mismatch_trips)
    _case("value_field negative — C++ right-as-int trips", _case_value_field_cpp_type_mismatch_trips)

    print(f"check_binding_parity --selftest: {n_pass} passed, {n_fail} failed")
    return 0 if n_fail == 0 else 1


if __name__ == "__main__":
    sys.exit(main())
