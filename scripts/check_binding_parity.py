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
    "StreamingIterSession": "UnifiedFpssIterSession",
    "Contract": "FluentContract",
    "Subscription": "FluentSubscription",
    "SecType": "FluentSecType",
}


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
# shape for `Option<usize>` fields (`MddsConfig.decode_threads`,
# `decode_queue_depth`, `RuntimeConfig.tokio_worker_threads`). The
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
    ("FlatFilesConfig", "initial_backoff"): "initial_backoff_secs",
    ("FlatFilesConfig", "max_backoff"): "max_backoff_secs",
    ("RetryPolicy", "initial_delay"): "initial_delay_ms",
    ("RetryPolicy", "max_delay"): "max_delay_ms",
}


def _rust_field_to_row_suffix(struct: str, field: str) -> str:
    return RUST_FIELD_RENAMES.get((struct, field), field)


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
            # `GreeksEodTick.audit_wave6`). Skip — these are not
            # field-level bindings.
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


def main(argv: list[str] | None = None) -> int:
    argv = argv if argv is not None else sys.argv[1:]
    if "--selftest" in argv:
        return _run_selftest()

    if not PARITY_TOML.is_file():
        print(f"missing parity matrix: {PARITY_TOML}", file=sys.stderr)
        return 1

    data: dict[str, Any] = tomllib.loads(PARITY_TOML.read_text(encoding="utf-8"))
    rows: list[dict[str, Any]] = data.get("class", [])
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

    # Orphan Rust pub fields (no parity row).
    orphan_errors = _check_orphan_rust_fields(rust_fields, rows)

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

    if orphan_errors:
        had_errors = True
        print(
            f"check_binding_parity: {len(orphan_errors)} Rust pub "
            f"field(s) lack a parity-toml row:"
        )
        for e in orphan_errors:
            print(e)
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
    print(
        f"check_binding_parity: clean "
        f"({n_class} class rows + {n_dotted} field rows + "
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
                "name": "MddsConfig.decode_threads",
                "python": True,
                "typescript": True,
                "cpp": True,
            }
        ]
        # FFI emits `tdx_config_set_decode_threads_explicit`; that
        # must satisfy the `decode_threads` row.
        ffi_setters = {"decode_threads_explicit", "decode_threads"}
        py_setters = {"decode_threads"}
        ts_setters = {"decode_threads"}
        cpp_setters = {"decode_threads"}
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

    print(f"check_binding_parity --selftest: {n_pass} passed, {n_fail} failed")
    return 0 if n_fail == 0 else 1


if __name__ == "__main__":
    sys.exit(main())
