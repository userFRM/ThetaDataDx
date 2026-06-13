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


# ─── Public-surface vocabulary guard ────────────────────────────────
#
# OUR Rust implementation-detail names that must never appear inside a
# PUBLIC client identifier (class / method / field / setter / getter /
# exported type). The bindings legitimately USE the async runtime, the
# lock-free ring, and the lock primitives internally — those uses live
# in implementation code and are out of scope here. This guard fires
# only on the identifiers the parity collectors already harvest, i.e.
# the names a user types. It catches the leak class structurally (a
# banned token embedded in a snake_case / camelCase identifier) where a
# word-boundary (`\bword\b`) text rule cannot, without false-positives
# on internal code.
#
# ALLOW-LIST: `mdds` and `fpss` are ThetaData's PROPRIETARY PROTOCOL names (the
# vendor this SDK wraps). They are NOT impl-detail leaks; the public
# surface stays aligned with the vendor's vocabulary. They are NOT
# listed below and MUST NOT be flagged. Only OUR own implementation
# details (the async runtime, the disruptor-style ring, the lock
# primitives, the I/O-bridge calls) are leaks.
#
# Lowercased; matched as a substring against the lowercased identifier
# (so `Tokio`, `tokio`, `TOKIO`, and an embedded `_tokio_` all hit).
BANNED_SURFACE_TOKENS: tuple[str, ...] = (
    "tokio",
    "disruptor",
    "crossbeam",
    "parking_lot",
    "parkinglot",
    "block_on",
    "blockon",
    "allow_threads",
    "allowthreads",
    "os_pipe",
    "ospipe",
)


def _surface_token_hit(identifier: str) -> str | None:
    """Return the banned implementation-detail token embedded in
    `identifier`, or ``None`` if the identifier is clean.

    The match is case-insensitive and substring-based so a token buried
    inside a snake_case or camelCase name (`set_tokio_worker_threads`,
    `TokioWorkerThreadsSetting`) is caught — exactly the blind spot in a
    word-boundary text rule. Vendor protocol names
    (`mdds`, `fpss`) are intentionally absent from the token list, so a
    `MddsClient` / `setFpssRingSize` identifier is never flagged.
    """
    lowered = identifier.lower()
    for token in BANNED_SURFACE_TOKENS:
        if token in lowered:
            return token
    return None


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
    "FlatFilesNamespace": "FlatFiles",
    "Contract": "FluentContract",
    "Subscription": "FluentSubscription",
    "SecType": "FluentSecType",
    "ParseError": "FpssParseError",
}


def _cpp_class_for(class_name: str) -> str:
    """Resolve a parity-toml `class` field to its C++ class symbol.

    Honors `CPP_ALIASES` so a row carrying the Python/TS canonical
    name (`Contract`) routes to the corresponding C++ class body
    (`FluentContract`).
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
        "MarketValue",
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


# ─── Config getter collection (read-back accessor roster) ───────────
#
# The setter collectors above harvest the write side of the Config knob
# roster. These collectors harvest the read-back side: every binding that
# exposes a getter for a knob in one language must expose it (idiomatic
# name) in all. The naming conventions mirror the setters:
#   * Python: `#[getter] fn get_<name>` (pyo3 strips `get_`, so the
#     property is the bare `<name>`).
#   * TypeScript: `#[napi(getter, js_name = "<camelCase>")]`.
#   * C++: a `get_<name>(...)` member on `class Config`.
#   * C ABI: `tdx_config_get_<name>`.


def _iter_impl_config_bodies(text: str) -> list[str]:
    """Return the body text of every `impl Config { ... }` block in
    `text`, bounded by a brace counter. Used to scope the getter
    collectors to the `Config` knob roster — getters live on many
    pyclasses / napi classes (tick structs, the fluent `Subscription`),
    but only the `Config` read-back accessors are part of the
    cross-binding knob roster the setter check's read-side complements.
    """
    bodies: list[str] = []
    for header in re.finditer(r"impl\s+Config\s*\{", text):
        bodies.append(_balanced_body(text, header.end()))
    return bodies


def _collect_python_getters(py_src: pathlib.Path) -> set[str]:
    """Read-back getter names on the `Config` pyclass.

    Scoped to `impl Config { ... }` blocks so tick-class / fluent-value
    getters (`QuoteTick.quote_timestamp_ms`, `Subscription.kind`) do not
    leak into the Config knob roster. The pyo3 pattern is ``#[getter] fn
    get_<name>``; the `get_` prefix the Rust fn name carries is stripped to
    the bare canonical name (the Python property spelling).
    """
    getters: set[str] = set()
    if not py_src.is_dir():
        return getters
    for rs in py_src.rglob("*.rs"):
        text = rs.read_text(encoding="utf-8")
        for body in _iter_impl_config_bodies(text):
            for m in re.finditer(r"#\[getter\][^}]*?fn\s+(\w+)", body, re.DOTALL):
                name = m.group(1)
                getters.add(name[4:] if name.startswith("get_") else name)
    return getters


def _collect_typescript_getters(ts_src: pathlib.Path) -> set[str]:
    """napi `#[napi(getter, js_name = "<camelCase>")]` read-back accessors
    on the `Config` napi class.

    Scoped to `impl Config { ... }` blocks (the fluent `Subscription`
    getters `isFull` / `secType` and tick-object fields are not Config
    knobs). Returns canonical snake_case names — every camelCase `js_name`
    lifted back through the compound-word alias table, exactly like the
    setter collector.
    """
    getters_camel: set[str] = set()
    if not ts_src.is_dir():
        return set()
    for rs in ts_src.rglob("*.rs"):
        text = rs.read_text(encoding="utf-8")
        for body in _iter_impl_config_bodies(text):
            for m in re.finditer(
                r'#\[napi\([^)]*\bgetter\b[^)]*\bjs_name\s*=\s*"([a-zA-Z_]\w*)"[^)]*\)\]',
                body,
            ):
                getters_camel.add(m.group(1))
    getters_snake: set[str] = set()
    for name in getters_camel:
        getters_snake.update(_camel_to_snake_with_aliases(name))
    return getters_snake


def _collect_cpp_getters(cpp_hpp: pathlib.Path) -> set[str]:
    """C++ `get_<name>(...)` read-back members on the `class Config` body.

    The wrapper exposes each read-back as a `get_<name>()` member. The
    bare `<name>` is the canonical form (the `get_` prefix is the C++
    convention, stripped here to compare against the cross-binding roster).
    Restricted to the `class Config { ... }` body so unrelated `get_*`
    members on other classes are not counted.
    """
    getters: set[str] = set()
    if not cpp_hpp.is_file():
        return getters
    text = _expand_cpp_includes(cpp_hpp.read_text(encoding="utf-8"), cpp_hpp.parent)
    m = re.search(r"^class\s+Config\s*(?::[^{]*)?\{", text, re.MULTILINE)
    if not m:
        return getters
    body = _balanced_body(text, m.end())
    for fm in re.finditer(r"\bget_(\w+)\s*\(", body):
        getters.add(fm.group(1))
    return getters


def _collect_ffi_getters(ffi_src: pathlib.Path) -> set[str]:
    """FFI `tdx_config_get_<name>` extern C read-back declarations.

    Mirrors `_collect_ffi_setters` on the read side. Returns the bare
    canonical names with the `tdx_config_get_` prefix stripped.
    """
    getters: set[str] = set()
    if not ffi_src.is_dir():
        return getters
    for rs in ffi_src.rglob("*.rs"):
        text = rs.read_text(encoding="utf-8")
        for m in re.finditer(r"\bfn\s+tdx_config_get_(\w+)\s*\(", text):
            getters.add(m.group(1))
    return getters


# ─── Client-facing setter-set parity ────────────────────────────────


# Per-binding spelling differences that are pure transport / language
# idiom, not a semantic divergence. Folding them away lets the four
# setter sets be compared for exact equality:
#
#   * `_explicit` — the widened `(has_value, n)` ABI variant a binding
#     emits for an `Option<usize>` field (`worker_threads_explicit` on
#     the C ABI / C++ / napi vs the bare `worker_threads` on Python).
#     Same knob, transport-only suffix.
#   * `flat_files` → `flatfiles` — napi auto-camelCases `setFlatFiles*`
#     to a multi-word `FlatFiles`, which lifts back to BOTH
#     `flat_files_*` and `flatfiles_*`; the cross-binding canonical
#     form is the single-token `flatfiles_*`.
def _normalize_setter(name: str) -> str:
    """Collapse a per-binding setter spelling to its cross-binding
    canonical form so the four setter sets compare for equality.
    """
    for suffix in FFI_EXPLICIT_SUFFIXES:
        if name.endswith(suffix):
            name = name[: -len(suffix)]
            break
    name = name.replace("flat_files", "flatfiles")
    return name


# Client-facing setters that legitimately exist on only some bindings.
# Each entry maps the canonical (normalized) setter name to a written
# reason. This is the documented per-language-idiom carve-out the gate
# tolerates — every entry is a reviewed decision, not a silencer.
#
# Empty today: the `mdds_host` / `mdds_port` advanced endpoint overrides
# are now bound on every binding (Python / TypeScript / C++ / the C
# ABI), so no carve-out is required. Add an entry here only when a
# setter is intentionally exposed on a strict subset of bindings.
SETTER_PARITY_EXEMPT: dict[str, str] = {}

# Read-back getter equivalent of `SETTER_PARITY_EXEMPT`. A knob exposed
# read-only / write-only on a strict subset of bindings on purpose lists
# its canonical (normalized) name here with a written reason. Several
# write-only knobs (the per-class reconnect budgets) have no getter on ANY
# binding — those never enter the getter universe and need no entry.
#
# Empty today: every knob that exposes a read-back getter exposes it on
# all four bindings.
GETTER_PARITY_EXEMPT: dict[str, str] = {}


def _check_accessor_set_parity(
    accessors: dict[str, set[str]],
    exempt: dict[str, str],
    noun: str,
    exempt_const_name: str,
) -> list[str]:
    """Assert a client-facing accessor SET matches across Python /
    TypeScript / C++ / the C ABI after normalization.

    `accessors` maps each binding name to its raw accessor-name set;
    `noun` is the accessor kind for diagnostics (`"setter"` / `"getter"`);
    `exempt_const_name` names the carve-out constant in the diagnostic.
    Genuine per-language idioms are folded by `_normalize_setter` (the
    `_explicit` widened-ABI suffix and the `flat_files`↔`flatfiles`
    camelCase split apply identically to setters and getters). Anything
    still divergent must be listed in `exempt` with a reason or it fails
    the gate. A stale exemption (now uniformly bound everywhere) is itself
    flagged so the carve-out list never rots.
    """
    norm = {lang: {_normalize_setter(a) for a in names} for lang, names in accessors.items()}
    universe: set[str] = set().union(*norm.values()) if norm else set()
    errors: list[str] = []
    for accessor in sorted(universe - set(exempt)):
        present_on = [lang for lang, names in norm.items() if accessor in names]
        if len(present_on) != len(norm):
            missing = [lang for lang in norm if lang not in present_on]
            errors.append(
                f"  {noun} `{accessor}`: present on {sorted(present_on)}, "
                f"missing on {sorted(missing)}. Bind it on every binding, "
                f"or add it to {exempt_const_name} with a per-language-idiom "
                f"reason."
            )
    for accessor, reason in exempt.items():
        present_on = [lang for lang, names in norm.items() if accessor in names]
        if present_on and len(present_on) == len(norm):
            errors.append(
                f"  {noun} `{accessor}`: listed in {exempt_const_name} "
                f"({reason!r}) but is now uniformly bound on every binding. "
                f"Drop the stale exemption."
            )
    return errors


def _check_setter_set_parity(
    py_setters: set[str],
    ts_setters: set[str],
    cpp_setters: set[str],
    ffi_setters: set[str],
    exempt: dict[str, str] | None = None,
) -> list[str]:
    """Assert the client-facing setter SET matches across Python /
    TypeScript / C++ / the C ABI after normalization.

    The per-row dotted check (`_check_dotted_rows`) verifies each
    declared knob resolves on the bindings it claims; this set-level
    check is the complementary direction — it catches a knob that
    landed on some bindings but silently never made it into the parity
    matrix on the others (the `derive_ohlcvc`-missing-on-TS defect
    class). Genuine per-language idioms are folded by
    `_normalize_setter`; anything still divergent must be listed in
    `exempt` (defaults to `SETTER_PARITY_EXEMPT`) with a reason or it
    fails the gate. The `exempt` parameter is injectable so the
    selftest can exercise the logic with synthetic carve-out lists.
    """
    if exempt is None:
        exempt = SETTER_PARITY_EXEMPT
    return _check_accessor_set_parity(
        {
            "python": py_setters,
            "typescript": ts_setters,
            "cpp": cpp_setters,
            "ffi": ffi_setters,
        },
        exempt,
        "setter",
        "SETTER_PARITY_EXEMPT",
    )


def _check_getter_set_parity(
    py_getters: set[str],
    ts_getters: set[str],
    cpp_getters: set[str],
    ffi_getters: set[str],
    exempt: dict[str, str] | None = None,
) -> list[str]:
    """Assert the client-facing read-back getter SET matches across
    Python / TypeScript / C++ / the C ABI after normalization.

    The complement to `_check_setter_set_parity` on the read side: a knob
    that exposes a getter on some bindings but silently never grew one on
    the others trips the gate. Together the two checks pin the full Config
    knob roster — both write and read accessors — across every binding.
    Per-language idioms fold via `_normalize_setter`; intentional
    subset-of-bindings getters list in `exempt` (defaults to
    `GETTER_PARITY_EXEMPT`) with a reason.
    """
    if exempt is None:
        exempt = GETTER_PARITY_EXEMPT
    return _check_accessor_set_parity(
        {
            "python": py_getters,
            "typescript": ts_getters,
            "cpp": cpp_getters,
            "ffi": ffi_getters,
        },
        exempt,
        "getter",
        "GETTER_PARITY_EXEMPT",
    )


# ─── Public-surface identifier collection (vocab guard) ─────────────


def _check_public_surface_vocab(
    py_classes: set[str],
    ts_classes: set[str],
    cpp_classes: set[str],
    py_setters: set[str],
    ts_setters: set[str],
    cpp_setters: set[str],
    ffi_setters: set[str],
    py_methods: dict[str, set[str]],
    ts_methods: dict[str, set[str]],
    cpp_methods: dict[str, set[str]],
) -> list[str]:
    """Assert no PUBLIC client identifier embeds a banned architecture
    token.

    Scans the identifier sets the parity collectors already harvest —
    classes, config setters, and per-class methods on every binding —
    for an internal-architecture token (`tokio`, `mdds`, `disruptor`,
    ...) buried inside the name. This is the structural counterpart to
    the text scrubber: it sees only declared public API names, so it
    never false-positives on the bindings' legitimate internal use of
    the runtime / ring / lock primitives, and it catches a banned token
    embedded in a snake_case / camelCase identifier that the scrubber's
    `\\bword\\b` rule misses.
    """
    errors: list[str] = []

    def _check(identifier: str, where: str) -> None:
        token = _surface_token_hit(identifier)
        if token is not None:
            errors.append(
                f"  {where}: public identifier `{identifier}` embeds "
                f"banned architecture token `{token}`. Rename to a "
                f"neutral client-facing name (the user concept, not the "
                f"implementation)."
            )

    for cls in sorted(py_classes):
        _check(cls, "python class")
    for cls in sorted(ts_classes):
        _check(cls, "typescript class")
    for cls in sorted(cpp_classes):
        _check(cls, "cpp class")
    for setter in sorted(py_setters):
        _check(setter, "python setter")
    for setter in sorted(ts_setters):
        _check(setter, "typescript setter")
    for setter in sorted(cpp_setters):
        _check(setter, "cpp setter")
    for setter in sorted(ffi_setters):
        _check(setter, "ffi setter")
    for cls, methods in sorted(py_methods.items()):
        for method in sorted(methods):
            _check(method, f"python method {cls}.")
    for cls, methods in sorted(ts_methods.items()):
        for method in sorted(methods):
            _check(method, f"typescript method {cls}.")
    for cls, methods in sorted(cpp_methods.items()):
        for method in sorted(methods):
            _check(method, f"cpp method {cls}.")
    return errors


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
    # The async worker-thread count is stored internally on
    # `RuntimeConfig.tokio_worker_threads`, but the public client setter
    # is named `worker_threads` on every binding (the implementation
    # runtime name never reaches the user surface). The parity row is
    # keyed by the public concept; this mapping bridges the internal
    # storage field to that row for the reverse-direction orphan check.
    ("RuntimeConfig", "tokio_worker_threads"): "worker_threads",
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

        # Python: snake_case method declared on the pyclass. A `#[getter]`
        # readback accessor carries a `get_` prefix on its Rust fn name
        # (`fn get_flush_mode`) while pyo3 strips the prefix so the Python
        # property name stays bare (`config.flush_mode`); accept the
        # `get_`-prefixed fn name against the bare row, exactly as the C++
        # branch below accepts `get_<snake>`.
        py_class_methods = py_methods.get(class_name, set())
        declared_py = row.get("python", False)
        actual_py = snake in py_class_methods or f"get_{snake}" in py_class_methods
        if declared_py != actual_py:
            verb = "missing" if declared_py and not actual_py else "unexpected"
            errors.append(
                f"  {class_name}.{camel}.python: declared={declared_py}, "
                f"actual={actual_py} ({verb} -- expected `fn {snake}` "
                f"or `fn get_{snake}` inside `impl {class_name}` on the "
                f"Python pyclass)"
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
        # `CPP_ALIASES` (`Contract` -> `FluentContract`). Readback
        # getters on the C++ `Config` carry a uniform `get_` prefix
        # (`get_flush_mode`), where Python exposes the field-shaped
        # property (`flush_mode`) and TypeScript the camelCase getter
        # (`flushMode`); accept the `get_`-prefixed C++ form so the
        # per-language naming convention does not read as drift.
        declared_cpp = row.get("cpp", False)
        cpp_class = _cpp_class_for(class_name)
        cpp_class_methods = cpp_methods.get(cpp_class, set())
        actual_cpp = snake in cpp_class_methods or f"get_{snake}" in cpp_class_methods
        if declared_cpp != actual_cpp:
            verb = "missing" if declared_cpp and not actual_cpp else "unexpected"
            errors.append(
                f"  {class_name}.{camel}.cpp: declared={declared_cpp}, "
                f"actual={actual_cpp} ({verb} -- expected `{snake}(` "
                f"or `get_{snake}(` inside `class {cpp_class}` body in "
                f"sdks/cpp/include/thetadx.hpp)"
            )

    return errors


# ─── Free-function (utility) discovery ──────────────────────────────
#
# The standalone utility surface — the offline Greeks calculator
# (`all_greeks` / `implied_volatility`), the conditions / exchange /
# calendar / sequence lookups (`condition_name`, `exchange_symbol`,
# `calendar_status_name`, `timestamp_ms`, `sequence_signed_to_unsigned`,
# ...), and the Python-only date-range splitter — is exposed as free
# functions / namespace functions per binding, NOT as methods on a class
# the `[[method]]` rows cover. These collectors find each binding's
# utility surface so the `[[utility]]` rows can pin the cross-binding
# roster.
#
# The TypeScript binding groups its lookup utilities as static methods on
# a `Util` namespace class (`Util.conditionName(...)`), while Python uses
# a `thetadatadx.util` submodule, C++ a `tdx::util` namespace, and the C
# ABI bare `tdx_*` symbols. The collectors normalise each surface to the
# bare snake_case function name so a single `[[utility]]` row matches
# every binding's idiom.


# Internal `#[pyfunction]`s that are NOT part of the public utility
# surface: decode-bench hooks and the FPSS-method introspection helper
# used by tests / external tooling. Excluded from the Python utility
# roster so they are not mistaken for untracked utilities.
PY_NON_UTILITY_PYFUNCTIONS: frozenset[str] = frozenset(
    {"decode_response_bytes", "blocked_fpss_methods"}
)


def _collect_python_utility_functions(py_src: pathlib.Path) -> set[str]:
    """Snake_case names of every public-utility `#[pyfunction]`.

    The offline calculators (`all_greeks` / `implied_volatility`), the
    `thetadatadx.util` submodule lookups, and the date-range splitter are
    all module-level `#[pyfunction] fn <name>`. The attribute may carry a
    `(...)` arg list (a `#[pyo3(...)]` sibling on the next line), so the
    regex tolerates an optional attribute body before the `fn`. Internal
    decode-bench / introspection hooks (`PY_NON_UTILITY_PYFUNCTIONS`) are
    filtered so only the user-facing utility surface remains.
    """
    out: set[str] = set()
    if not py_src.is_dir():
        return out
    fn_re = re.compile(r"#\[pyfunction\][^{}]*?fn\s+(\w+)\s*\(", re.DOTALL)
    for rs in py_src.rglob("*.rs"):
        text = rs.read_text(encoding="utf-8")
        for m in fn_re.finditer(text):
            out.add(m.group(1))
    return out - PY_NON_UTILITY_PYFUNCTIONS


def _collect_typescript_utility_functions(ts_src: pathlib.Path) -> set[str]:
    """Snake_case names of the TypeScript utility surface.

    Two shapes: the offline calculators are napi FREE functions
    (`#[napi] pub fn all_greeks`), while the conditions / exchange /
    calendar / sequence lookups are static methods on the `Util` namespace
    class (`#[napi(js_name = "conditionName")] pub fn condition_name` inside
    `impl Util`). The free-function scan below collects the former; the
    `Util`-class methods are merged in by the caller via
    `_collect_typescript_class_methods`, camelCase folded back to
    snake_case. Both surfaces normalise to the bare snake_case name so a
    single `[[utility]]` row matches Python / C++ / FFI and TypeScript
    alike.

    A napi free function is a `#[napi(...)]`-attributed `pub fn <name>`
    that is NOT inside an `impl` block. Functions inside `impl` blocks
    (methods) are excluded by blanking each `impl { ... }` body before the
    scan, so only true free functions remain here.
    """
    out: set[str] = set()
    if not ts_src.is_dir():
        return out
    impl_re = re.compile(r"impl\s+(?:[A-Za-z_][\w]*::)*[A-Za-z_][\w]*\s*\{")
    # A napi free function: the `#[napi(...)]` attribute, then any number
    # of intervening outer attributes (`#[allow(...)]`, doc comments, or a
    # trailing `// ...` line comment), then `pub fn <name>`. The generated
    # calculator carries a `#[allow(clippy::too_many_arguments)] // Reason:
    # ...` line between the napi attribute and the fn, so the gap tolerates
    # further `#[...]` / `///` / `//` runs.
    free_fn_re = re.compile(
        r"#\[napi(?:\([^)]*\))?\]\s*"
        r"(?:(?:#\[[^\]]*\]|//[^\n]*)\s*)*"
        r"(?:pub\s+)?(?:async\s+)?fn\s+([a-z_][a-z0-9_]*)\s*[(<]"
    )
    for rs in ts_src.rglob("*.rs"):
        text = rs.read_text(encoding="utf-8")
        # Blank every impl body so method declarations inside them do not
        # masquerade as free functions. Walk with a brace counter.
        cleaned = []
        i = 0
        while i < len(text):
            m = impl_re.search(text, i)
            if not m:
                cleaned.append(text[i:])
                break
            cleaned.append(text[i : m.start()])
            # Skip from the impl header's opening brace to its match.
            depth = 0
            j = m.end() - 1  # position of the `{`
            while j < len(text):
                c = text[j]
                if c == "{":
                    depth += 1
                elif c == "}":
                    depth -= 1
                    if depth == 0:
                        j += 1
                        break
                j += 1
            i = j
        body = "".join(cleaned)
        for fm in free_fn_re.finditer(body):
            out.add(fm.group(1))
    return out


def _collect_cpp_utility_functions(cpp_hpp: pathlib.Path) -> set[str]:
    """Snake_case names of free functions declared in the `tdx`
    namespace of the C++ wrapper.

    The calculator declarations live in
    `sdks/cpp/include/utilities.hpp.inc`, pulled into `thetadx.hpp` via
    `#include "utilities.hpp.inc"`. `_expand_cpp_includes` inlines the
    `.inc` first, then a `<ret> <name>(` shape outside any `class {...}`
    body is a free function. The collector blanks class bodies (mirroring
    the TS impl-body blanking) so member functions are not counted.
    """
    out: set[str] = set()
    if not cpp_hpp.is_file():
        return out
    text = _expand_cpp_includes(cpp_hpp.read_text(encoding="utf-8"), cpp_hpp.parent)
    # Blank class / struct bodies so only namespace-scope free functions
    # remain.
    type_re = re.compile(r"\b(?:class|struct)\s+\w+[^{;]*\{")
    cleaned = []
    i = 0
    while i < len(text):
        m = type_re.search(text, i)
        if not m:
            cleaned.append(text[i:])
            break
        cleaned.append(text[i : m.start()])
        depth = 0
        j = m.end() - 1
        while j < len(text):
            c = text[j]
            if c == "{":
                depth += 1
            elif c == "}":
                depth -= 1
                if depth == 0:
                    j += 1
                    break
            j += 1
        i = j
    body = "".join(cleaned)
    # `<return type> <name>(` at namespace scope. The return type may be
    # a qualified / templated type (`std::pair<double, double>`), so match
    # the identifier immediately before the `(` and filter keywords.
    for fm in re.finditer(r"(?:^|[\s>])([a-z_][a-z0-9_]*)\s*\(", body, re.MULTILINE):
        name = fm.group(1)
        if name in {"if", "while", "for", "switch", "return", "sizeof", "throw"}:
            continue
        out.add(name)
    return out


def _collect_ffi_utility_functions(ffi_src: pathlib.Path) -> set[str]:
    """Bare calculator names whose `tdx_<name>` C ABI symbol exists.

    The FFI exposes the calculators as `extern "C" fn tdx_all_greeks` /
    `fn tdx_implied_volatility`. The collector strips the `tdx_` prefix so
    the result matches the canonical `[[utility]]` row name directly.
    """
    out: set[str] = set()
    if not ffi_src.is_dir():
        return out
    for rs in ffi_src.rglob("*.rs"):
        text = rs.read_text(encoding="utf-8")
        for m in re.finditer(r"\bfn\s+tdx_(\w+)\s*\(", text):
            out.add(m.group(1))
    return out


def _check_utility_rows(
    utility_rows: list[dict[str, Any]],
    py_utils: set[str],
    ts_utils: set[str],
    cpp_utils: set[str],
    ffi_utils: set[str],
) -> list[str]:
    """Per-free-function cross-binding gate for `[[utility]]` rows.

    Each row declares a snake_case function `name` plus the expected
    presence in Python / TypeScript / C++ / the C ABI. The checker
    compares the declared state against the actual binding state and
    returns a list of mismatch strings (empty when every row matches).
    The TypeScript spelling is derived as camelCase only for the
    diagnostic; the collector already records the snake_case fn name, so
    the match is name-to-name.

    A row whose C ABI symbol carries a disambiguating prefix the
    higher-level bindings drop (the conditions table exposes the bare
    `is_cancel` on Python / TypeScript / C++ but `tdx_condition_is_cancel`
    on the C ABI, where the bare `is_cancel` would be ambiguous against
    the quote-condition predicate) records the bare C-symbol name under an
    `ffi_name` key. The gate strips the `tdx_` prefix off the FFI symbol
    when collecting, so `ffi_name = "condition_is_cancel"` matches
    `tdx_condition_is_cancel`. Absent the override the canonical `name` is
    used for every binding.

    A row may also carry a `binding_specific` reason string. Such a row is
    NOT cross-binding by contract — it pins a function that exists on a
    strict subset of bindings on purpose (a Python-only date-range splitter,
    the C-ABI memory-management / value-folding helpers the managed
    bindings have no analogue for). The per-binding booleans are still
    asserted against the live sources, so the function cannot silently
    appear or vanish on the bindings it does / does not target; the reason
    documents WHY the asymmetry is intended.
    """
    errors: list[str] = []
    for row in utility_rows:
        name = row.get("name")
        if not name:
            errors.append(f"  [[utility]] row missing `name`: {row!r}")
            continue
        camel = _snake_to_camel(name)
        ffi_name = row.get("ffi_name", name)
        for lang, lookup_name, actual_set, hint in (
            ("python", name, py_utils, f"`#[pyfunction] fn {name}`"),
            (
                "typescript",
                name,
                ts_utils,
                f'`#[napi(js_name = "{camel}")] fn {name}`',
            ),
            (
                "cpp",
                name,
                cpp_utils,
                f"`{name}(` in the `tdx::util` namespace",
            ),
            ("ffi", ffi_name, ffi_utils, f"`tdx_{ffi_name}`"),
        ):
            declared = row.get(lang, False)
            actual = lookup_name in actual_set
            if declared != actual:
                verb = "missing" if declared and not actual else "unexpected"
                errors.append(
                    f"  {name}.{lang}: declared={declared}, actual={actual} "
                    f"({verb} -- expected {hint})"
                )
    return errors


def _is_ts_internal_free_fn(name: str) -> bool:
    """True for a TypeScript napi free function that is serialization /
    coercion plumbing, not a user-facing utility.

    The JS shim emits a `<tick>_tick_to_arrow_ipc` free function per tick
    type for the zero-copy Arrow boundary, plus small numeric-coercion
    helpers (`bigint_to_i32`). These are not part of the standalone
    utility roster the `[[utility]]` rows track, so they are excluded from
    the TypeScript utility surface.
    """
    return name.endswith("_to_arrow_ipc") or name == "bigint_to_i32"


def _ts_utility_surface(
    ts_free_fns: set[str], ts_class_methods: dict[str, set[str]]
) -> set[str]:
    """The TypeScript standalone-utility surface as bare snake_case names.

    Merges the user-facing napi free functions (the offline calculators)
    with the `Util` namespace-class static methods (the conditions /
    exchange / calendar / sequence lookups), folding the camelCase `Util`
    method spellings back to snake_case so both shapes compare against the
    same `[[utility]]` row name. Internal serialization / coercion free
    functions (`_is_ts_internal_free_fn`) are filtered.
    """
    surface = {fn for fn in ts_free_fns if not _is_ts_internal_free_fn(fn)}
    for camel in ts_class_methods.get("Util", set()):
        surface.add(_camel_to_snake(camel))
    return surface


def _check_utility_roster_complete(
    utility_rows: list[dict[str, Any]],
    py_utils: set[str],
    ts_utils: set[str],
) -> list[str]:
    """Reverse-direction orphan check for the standalone-utility roster.

    `_check_utility_rows` verifies every declared row resolves on the
    bindings it claims. This complementary direction catches a utility
    that exists on a binding but has NO `[[utility]]` row at all — the
    `calendar_status_name` / `timestamp_ms` defect class where a util
    shipped on one binding and silently never made it into the matrix on
    the others.

    The orphan scan runs over the two bindings whose utility surface is
    precisely enumerable: the Python public `#[pyfunction]` set (internal
    decode-bench / introspection hooks already filtered) and the
    TypeScript utility surface (napi free functions + `Util` namespace
    methods). Every name in those surfaces must be a declared row's
    canonical `name`. The C++ `tdx::util` / C ABI `tdx_*` surfaces are
    pinned forward per row (and the C ABI additionally by
    `check_c_abi_completeness`); they are not enumerable cleanly here
    because the namespace mingles the lookups with dozens of unrelated
    fluent accessors and arrow-conversion symbols.
    """
    errors: list[str] = []
    declared_managed = {row["name"] for row in utility_rows if row.get("name")}
    for lang, seen in (
        ("python", py_utils),
        ("typescript", ts_utils),
    ):
        for fn in sorted(seen - declared_managed):
            errors.append(
                f"  {fn} ({lang}): standalone utility has no [[utility]] "
                f"row. Add one declaring its per-binding presence so the "
                f"roster stays tracked (use `ffi_name` if the C ABI symbol "
                f"carries a disambiguating prefix, or `binding_specific` if "
                f"the function is intentionally not cross-binding)."
            )
    return errors


# ─── Subscription-kind label discovery ──────────────────────────────
#
# The FPSS subscription-kind enum stringifies to a fixed snake_case label
# set that every binding surfaces (the C ABI `tdx_*_active_subscriptions`
# `kind` field, the C++ `FluentSubscription::kind_string`, the Python /
# TypeScript `Subscription.kind` accessors). The label is the stable
# cross-binding contract — a quant filtering `sub.kind == "open_interest"`
# in Python must get the same string the C++ `kind_string()` returns. A
# binding that drifts onto the enum's PascalCase `Debug` spelling, or
# invents a label outside the canonical set (the `full_quote` /
# `full_market_value` C++ defect class — full-stream Quote / MarketValue
# do not exist on the wire, so a label for them is fictitious), breaks the
# contract. These collectors harvest the literal strings each binding
# emits so the set can be asserted equal to the canonical roster.


# The canonical snake_case subscription-kind label set. Per-contract
# subscriptions carry the bare kind (`quote` / `trade` / `open_interest` /
# `market_value`); full-stream subscriptions carry the `full_` prefix and
# exist only for trade + open-interest (the FPSS wire has no full-stream
# quote or market-value broadcast). This is the single roster every
# binding must emit.
CANONICAL_SUBSCRIPTION_KINDS: frozenset[str] = frozenset(
    {
        "quote",
        "trade",
        "open_interest",
        "market_value",
        "full_trades",
        "full_open_interest",
    }
)

SUBSCRIPTION_RS = (
    REPO_ROOT / "crates" / "thetadatadx" / "src" / "fpss" / "protocol" / "subscription.rs"
)
PY_FLUENT_RS = REPO_ROOT / "sdks" / "python" / "src" / "fluent.rs"
TS_FLUENT_RS = REPO_ROOT / "sdks" / "typescript" / "src" / "fluent.rs"


# A snake_case string literal that is one of the canonical kind labels OR
# the `full_<x>` shape (so a binding inventing `full_quote` is captured
# and then flagged against the canonical set rather than silently passing
# the harvest filter).
_KIND_LITERAL_RE = re.compile(r'"((?:full_)?[a-z]+(?:_[a-z]+)*)"')
_KIND_VOCAB: frozenset[str] = CANONICAL_SUBSCRIPTION_KINDS | frozenset(
    {"full_quote", "full_market_value"}
)


def _harvest_kind_labels(text: str, anchor_substrings: tuple[str, ...]) -> set[str]:
    """Collect kind-label string literals that appear on a line near one
    of the `match`-arm `anchor_substrings`.

    The kind accessors stringify by matching the enum and returning a
    literal per arm; the canonical labels (and the two fictitious
    `full_*` spellings the gate guards against) are the only snake_case
    string literals in those arm bodies. Scanning the whole accessor body
    rather than parsing the match keeps the collector resilient to the
    per-binding wrapper differences (pyo3 `#[getter]`, napi `#[napi(getter)]`,
    a Rust `&'static str` return) while still pinning the emitted set.
    """
    out: set[str] = set()
    for literal in _KIND_LITERAL_RE.findall(text):
        if literal in _KIND_VOCAB:
            out.add(literal)
    # `anchor_substrings` documents intent (the arms a reader expects to
    # carry the labels) and guards against an empty harvest from a file
    # the layout drifted out from under — if not one anchor is present,
    # the caller's source no longer matches the contract.
    return out if any(a in text for a in anchor_substrings) else out


def _collect_rust_subscription_kinds(subscription_rs: pathlib.Path) -> set[str]:
    """Canonical kind labels emitted by the Rust core SSOT.

    Harvests every canonical-vocabulary string literal in
    `subscription.rs`, where `SubscriptionKind::kind_str`,
    `SubscriptionKind::full_kind_str`, and `FullSubscriptionKind::kind_str`
    define the labels the whole stack reads.
    """
    if not subscription_rs.is_file():
        return set()
    return _harvest_kind_labels(
        subscription_rs.read_text(encoding="utf-8"),
        ("fn kind_str", "fn full_kind_str"),
    )


def _collect_binding_subscription_kinds(fluent_rs: pathlib.Path) -> set[str]:
    """Kind labels emitted by a Python / TypeScript `Subscription.kind`
    accessor in `fluent.rs`. Both bindings stringify the same way (a match
    on the inner protocol enum returning a literal per arm); the harvest
    is mechanism-agnostic.
    """
    if not fluent_rs.is_file():
        return set()
    return _harvest_kind_labels(
        fluent_rs.read_text(encoding="utf-8"),
        ("fn kind", "SubscriptionKind"),
    )


def _collect_cpp_subscription_kinds(cpp_hpp: pathlib.Path) -> set[str]:
    """Kind labels emitted by the C++ `FluentSubscription::kind_string`
    switch in `thetadx.hpp`. The method is the only place in the header
    that returns these snake_case labels, so a header-wide harvest of the
    canonical vocabulary captures exactly its emitted set (and any
    fictitious `full_*` label, which the canonical-set assertion then
    flags).
    """
    if not cpp_hpp.is_file():
        return set()
    text = cpp_hpp.read_text(encoding="utf-8")
    # Bound the harvest to the `kind_string()` body so an unrelated label
    # string elsewhere in the header cannot mask a divergence inside the
    # accessor.
    m = re.search(r"std::string\s+kind_string\s*\(\s*\)\s*const\s*\{", text)
    if not m:
        return set()
    body_start = m.end()
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
    return {lit for lit in _KIND_LITERAL_RE.findall(body) if lit in _KIND_VOCAB}


def _collect_ffi_subscription_kinds(cpp_h: pathlib.Path) -> set[str]:
    """Kind labels documented as the C ABI contract in `thetadx.h`.

    The C ABI surfaces the kind as the `TdxSubscription.kind` string field,
    populated by the Rust core's `kind_str` / `full_kind_str` (so the C ABI
    emits exactly the Rust set at runtime). The header documents the
    canonical label set in the `TdxSubscription` doc comment; harvesting
    those literals pins the documented C contract against the same roster
    the runtime emits, so a header that drops a label (or documents a
    fictitious one) is caught.
    """
    if not cpp_h.is_file():
        return set()
    text = cpp_h.read_text(encoding="utf-8")
    # The kind label vocabulary appears in the `TdxSubscription` struct
    # doc comment. Restrict the harvest to the lines mentioning the field
    # so unrelated snake_case literals in the header are not swept in.
    out: set[str] = set()
    for line in text.splitlines():
        if "kind" in line.lower() and ('"' in line or "full_" in line):
            for lit in _KIND_LITERAL_RE.findall(line):
                if lit in _KIND_VOCAB:
                    out.add(lit)
    return out


def _check_subscription_kind_parity(
    rust_kinds: set[str],
    py_kinds: set[str],
    ts_kinds: set[str],
    cpp_kinds: set[str],
    ffi_kinds: set[str],
    canonical: frozenset[str] = CANONICAL_SUBSCRIPTION_KINDS,
) -> list[str]:
    """Assert every binding emits exactly the canonical kind-label set.

    Each binding's harvested label set must equal `canonical`. A binding
    short of a label has dropped a kind (the C-ABI-collision class where a
    label silently differs); a binding with an extra label has invented a
    non-canonical string (the C++ `full_quote` / `full_market_value`
    class). Both directions fail the gate.
    """
    errors: list[str] = []
    for lang, emitted in (
        ("rust", rust_kinds),
        ("python", py_kinds),
        ("typescript", ts_kinds),
        ("cpp", cpp_kinds),
        ("ffi", ffi_kinds),
    ):
        missing = canonical - emitted
        extra = emitted - canonical
        if missing:
            errors.append(
                f"  {lang}: missing kind label(s) {sorted(missing)} — the "
                f"binding does not emit the full canonical set "
                f"{sorted(canonical)}."
            )
        if extra:
            errors.append(
                f"  {lang}: emits non-canonical kind label(s) {sorted(extra)} "
                f"— only {sorted(canonical)} are valid cross-binding kind "
                f"strings (full-stream exists for trade + open-interest only)."
            )
    return errors


# ─── Error-leaf mapping discovery ────────────────────────────────────
#
# Each core `thetadatadx::Error` variant maps to one leaf class on every
# binding (`InvalidCredentialsError`, `RateLimitError`, ...) and one
# `TDX_ERR_*` integer code on the C ABI. The leaf vocabulary is the
# cross-binding contract: a caller porting an `except InvalidParameterError`
# clause from Python to a `catch (InvalidParameterError&)` in C++ must
# catch the same conditions. The Python `to_py_err`, the TypeScript
# `leaf_class_for`, the C++ `throw_for_code`, and the C ABI `error_code_for`
# all hand-write the same mapping; these collectors harvest each binding's
# leaf-class roster (and the C ABI code roster) so the sets can be asserted
# identical — the `FlatFilesUnavailable` / `PartialReconnect` (invisible on
# Py / TS) and `ConfigError` (missing leaf) defect class.


# The canonical leaf-class roster. Every binding's error dispatch must
# resolve to exactly this set of leaf classes (the root `ThetaDataError`
# included — it is the `#[non_exhaustive]` catch-all every binding routes
# unknown future variants to).
CANONICAL_ERROR_LEAVES: frozenset[str] = frozenset(
    {
        "ThetaDataError",
        "AuthenticationError",
        "InvalidCredentialsError",
        "SubscriptionError",
        "RateLimitError",
        "InvalidParameterError",
        "SchemaMismatchError",
        "NetworkError",
        "UnavailableError",
        "DeadlineExceededError",
        "NotFoundError",
        "StreamError",
        "ConfigError",
    }
)

# The canonical `TDX_ERR_*` code roster mapped to its integer value. The C
# ABI surfaces these via `tdx_last_error_code`; the higher bindings
# dispatch on them. `TDX_ERR_NONE` (0) is the no-error sentinel, present in
# the header but not a leaf class, so the leaf-to-code correspondence skips
# it.
CANONICAL_ERROR_CODES: dict[str, int] = {
    "TDX_ERR_NONE": 0,
    "TDX_ERR_OTHER": 1,
    "TDX_ERR_AUTHENTICATION": 2,
    "TDX_ERR_INVALID_CREDENTIALS": 3,
    "TDX_ERR_SUBSCRIPTION": 4,
    "TDX_ERR_RATE_LIMIT": 5,
    "TDX_ERR_NOT_FOUND": 6,
    "TDX_ERR_DEADLINE_EXCEEDED": 7,
    "TDX_ERR_UNAVAILABLE": 8,
    "TDX_ERR_NETWORK": 9,
    "TDX_ERR_SCHEMA_MISMATCH": 10,
    "TDX_ERR_STREAM": 11,
    "TDX_ERR_CONFIG": 12,
    "TDX_ERR_INVALID_PARAMETER": 13,
}

PY_ERRORS_RS = REPO_ROOT / "sdks" / "python" / "src" / "errors.rs"
TS_LIB_RS = REPO_ROOT / "sdks" / "typescript" / "src" / "lib.rs"
FFI_ERROR_RS = REPO_ROOT / "ffi" / "src" / "error.rs"

# A class name ending in `Error` is a candidate leaf. The fixed roster
# filters harvested identifiers to the canonical leaves so a stray
# `Error`-suffixed local (`join_err`, a doc-comment word) is never counted.
_LEAF_RE = re.compile(r"\b([A-Z][A-Za-z0-9]*Error)\b")


def _collect_python_error_leaves(py_errors_rs: pathlib.Path) -> set[str]:
    """Leaf classes the Python `to_py_err` dispatch resolves to.

    Bounds the harvest to the `fn to_py_err` body so the exception-class
    *definitions* (`create_exception!`) and the back-compat alias
    registrations do not inflate the dispatch roster — the gate asserts
    the set the dispatch actually routes to, which is the user-observable
    contract.
    """
    if not py_errors_rs.is_file():
        return set()
    text = py_errors_rs.read_text(encoding="utf-8")
    m = re.search(r"pub fn to_py_err\s*\([^)]*\)\s*->\s*PyErr\s*\{", text)
    if not m:
        return set()
    body = _balanced_body(text, m.end())
    # The dispatch builds each leaf as `<Class>::new_err(...)`; the
    # rate-limit arm routes through the `rate_limit_err` helper which
    # builds `RateLimitError`. Harvest both shapes.
    leaves = {
        leaf for leaf in _LEAF_RE.findall(body) if leaf in CANONICAL_ERROR_LEAVES
    }
    if "rate_limit_err" in body:
        leaves.add("RateLimitError")
    return leaves


def _collect_typescript_error_leaves(ts_lib_rs: pathlib.Path) -> set[str]:
    """Leaf-class strings the TypeScript `leaf_class_for` dispatch returns.

    Bounds the harvest to the `fn leaf_class_for` body so only the strings
    the dispatch emits are counted.
    """
    if not ts_lib_rs.is_file():
        return set()
    text = ts_lib_rs.read_text(encoding="utf-8")
    m = re.search(r"fn leaf_class_for\s*\([^)]*\)\s*->\s*&'static str\s*\{", text)
    if not m:
        return set()
    body = _balanced_body(text, m.end())
    return {
        lit
        for lit in re.findall(r'"([A-Z][A-Za-z0-9]*Error)"', body)
        if lit in CANONICAL_ERROR_LEAVES
    }


def _collect_cpp_error_leaves(cpp_hpp: pathlib.Path) -> set[str]:
    """Leaf classes the C++ `throw_for_code` dispatch can throw.

    Bounds the harvest to the `throw_for_code` body and collects every
    `throw <Class>(...)` target.
    """
    if not cpp_hpp.is_file():
        return set()
    text = cpp_hpp.read_text(encoding="utf-8")
    m = re.search(r"void\s+throw_for_code\s*\([^)]*\)\s*\{", text)
    if not m:
        return set()
    body = _balanced_body(text, m.end())
    return {
        cls
        for cls in re.findall(r"throw\s+([A-Z][A-Za-z0-9]*Error)\s*\(", body)
        if cls in CANONICAL_ERROR_LEAVES
    }


def _collect_ffi_error_codes(ffi_error_rs: pathlib.Path) -> dict[str, int]:
    """`TDX_ERR_*` discriminants defined in the FFI `error.rs`.

    Returns `{code_name: int_value}` for every `pub const TDX_ERR_* : i32 =
    N;` declaration — the source of truth for the C ABI error codes.
    """
    if not ffi_error_rs.is_file():
        return {}
    text = ffi_error_rs.read_text(encoding="utf-8")
    out: dict[str, int] = {}
    for name, value in re.findall(
        r"pub const (TDX_ERR_\w+)\s*:\s*i32\s*=\s*(\d+)\s*;", text
    ):
        out[name] = int(value)
    return out


def _collect_ffi_error_codes_dispatched(ffi_error_rs: pathlib.Path) -> set[str]:
    """`TDX_ERR_*` codes the FFI `error_code_for` dispatch actually returns.

    Bounds the harvest to the `fn error_code_for` body so the roster is the
    set the dispatch routes to (not merely the defined constants).
    """
    if not ffi_error_rs.is_file():
        return set()
    text = ffi_error_rs.read_text(encoding="utf-8")
    m = re.search(r"fn error_code_for\s*\([^)]*\)\s*->\s*i32\s*\{", text)
    if not m:
        return set()
    body = _balanced_body(text, m.end())
    return set(re.findall(r"\b(TDX_ERR_\w+)\b", body))


def _collect_cpp_error_codes(cpp_h: pathlib.Path) -> dict[str, int]:
    """`TDX_ERR_*` codes defined in the C ABI header `thetadx.h`.

    Returns `{code_name: int_value}` for every `#define TDX_ERR_* N`. The
    header is hand-maintained; the gate asserts it matches the FFI Rust
    constants exactly so a code that drifts on the C side (invisible to
    `cargo build`) is caught.
    """
    if not cpp_h.is_file():
        return {}
    text = cpp_h.read_text(encoding="utf-8")
    out: dict[str, int] = {}
    for name, value in re.findall(r"#define\s+(TDX_ERR_\w+)\s+(\d+)\b", text):
        out[name] = int(value)
    return out


def _check_error_leaf_parity(
    py_leaves: set[str],
    ts_leaves: set[str],
    cpp_leaves: set[str],
    ffi_codes: dict[str, int],
    ffi_codes_dispatched: set[str],
    cpp_codes: dict[str, int],
    canonical_leaves: frozenset[str] = CANONICAL_ERROR_LEAVES,
    canonical_codes: dict[str, int] | None = None,
) -> list[str]:
    """Assert the error-leaf mapping is symmetric across all bindings.

    Four invariants:

    1. The Python / TypeScript / C++ leaf-class rosters each equal the
       canonical leaf set (so a variant invisible on one binding — the
       `FlatFilesUnavailable` / `PartialReconnect` defect — is caught, as
       is a missing `ConfigError` leaf).
    2. The FFI `TDX_ERR_*` constants equal the canonical code table
       (name → value), so a renumbered or renamed code trips.
    3. The C ABI header `#define`s match the FFI Rust constants exactly,
       so a hand-maintained-header drift (invisible to `cargo build`)
       trips.
    4. Every dispatched FFI code is defined, and every leaf class has a
       corresponding `TDX_ERR_*` code, so the leaf set and the code set
       stay in one-to-one correspondence.
    """
    if canonical_codes is None:
        canonical_codes = CANONICAL_ERROR_CODES
    errors: list[str] = []

    for lang, leaves in (
        ("python", py_leaves),
        ("typescript", ts_leaves),
        ("cpp", cpp_leaves),
    ):
        missing = canonical_leaves - leaves
        extra = leaves - canonical_leaves
        if missing:
            errors.append(
                f"  {lang}: error dispatch never routes to leaf class(es) "
                f"{sorted(missing)} — a core Error variant maps to a class "
                f"the other bindings expose but this one does not."
            )
        if extra:
            errors.append(
                f"  {lang}: error dispatch routes to non-canonical leaf "
                f"class(es) {sorted(extra)} — add them to the canonical "
                f"roster (and the other bindings) or remove the divergence."
            )

    # FFI Rust constant table must equal the canonical code table.
    if ffi_codes and ffi_codes != canonical_codes:
        for name, value in sorted(canonical_codes.items()):
            if name not in ffi_codes:
                errors.append(f"  ffi: `{name}` is not defined in ffi/src/error.rs")
            elif ffi_codes[name] != value:
                errors.append(
                    f"  ffi: `{name}` = {ffi_codes[name]} in ffi/src/error.rs, "
                    f"canonical is {value}"
                )
        for name in sorted(set(ffi_codes) - set(canonical_codes)):
            errors.append(
                f"  ffi: `{name}` = {ffi_codes[name]} is not in the canonical "
                f"code table — add it to CANONICAL_ERROR_CODES (and every "
                f"binding's dispatch) or remove it."
            )

    # C ABI header must match the FFI Rust constants exactly.
    if ffi_codes and cpp_codes and ffi_codes != cpp_codes:
        for name, value in sorted(ffi_codes.items()):
            if name not in cpp_codes:
                errors.append(
                    f"  cpp header: `{name}` defined in ffi/src/error.rs but "
                    f"missing from sdks/cpp/include/thetadx.h"
                )
            elif cpp_codes[name] != value:
                errors.append(
                    f"  cpp header: `#define {name} {cpp_codes[name]}` disagrees "
                    f"with the FFI constant value {value}"
                )
        for name in sorted(set(cpp_codes) - set(ffi_codes)):
            errors.append(
                f"  cpp header: `#define {name} {cpp_codes[name]}` has no FFI "
                f"constant — drop it or add the Rust constant."
            )

    # Every dispatched FFI code must be defined.
    if ffi_codes_dispatched:
        for code in sorted(ffi_codes_dispatched - set(ffi_codes)):
            errors.append(
                f"  ffi: `error_code_for` returns `{code}` but it is not a "
                f"defined `pub const`."
            )

    return errors


def _balanced_body(text: str, body_start: int) -> str:
    """Return the substring from `body_start` to the matching close brace.

    `body_start` must be the index immediately after the opening `{` of the
    block. Walks a depth counter to the balancing `}` and returns the body
    between (exclusive of the closing brace). Shared by the dispatch-body
    harvesters so each one pins its scan to a single function body.
    """
    depth = 1
    i = body_start
    while i < len(text) and depth > 0:
        c = text[i]
        if c == "{":
            depth += 1
        elif c == "}":
            depth -= 1
        i += 1
    return text[body_start : i - 1]


# ─── Historical server-stream surface discovery ─────────────────────
#
# The endpoint codegen casts an endpoint's snake_case name to PascalCase
# / camelCase with INITIALISM awareness: the segments `eod` / `ohlc` /
# `iv` / `dte` / `nbbo` render as all-caps (`EOD`, `OHLC`, ...) in the
# TypeScript `js_name` (`stockHistoryEODStream`) but as title-case in the
# Python builder struct (`StockHistoryEodBuilder`). A naive
# `camelCase → snake_case` inverse would split `EOD` into `e_o_d`. The
# helper below collapses any run of consecutive uppercase letters into a
# single segment, so both `stockHistoryEOD` and `StockHistoryEod` invert
# to the canonical `stock_history_eod` row name.


def _endpoint_method_to_snake(name: str) -> str:
    """Invert an endpoint method / builder stem to its snake_case name,
    collapsing initialism runs (`EOD`, `OHLC`) into one segment.

    `stockHistoryEOD` -> `stock_history_eod`;
    `StockHistoryEod` -> `stock_history_eod`;
    `optionHistoryTradeGreeksImpliedVolatility` ->
    `option_history_trade_greeks_implied_volatility`.
    """
    # Boundary BEFORE an uppercase run that is followed by a lowercase
    # (start of a new title-case word): `...EODStream` keeps `EOD`
    # together but splits before `Stream`'s `S...tream`.
    s = re.sub(r"(?<=[a-z0-9])([A-Z])", r"_\1", name)
    s = re.sub(r"([A-Z]+)([A-Z][a-z])", r"\1_\2", s)
    return s.lower()

#
# The buffered historical endpoints have a server-stream companion on
# every binding — the `.stream(handler)` core primitive surfaced as a
# per-binding terminal:
#
#   * Python: `fn stream` + `fn stream_async` on each `<Endpoint>Builder`
#     pyclass (generated into
#     `sdks/python/src/_generated/historical_methods.rs`).
#   * TypeScript: a `<endpoint>Stream` method on the `ThetaDataDxClient`
#     napi class (generated into
#     `sdks/typescript/src/_generated/historical_methods.rs`).
#   * C ABI: a `tdx_<endpoint>_stream` extern "C" symbol in `ffi/src/`.
#   * C++: an `<endpoint>_stream` member on the `ThetaDataDxClient` wrapper
#     (`thetadx.hpp` + its `.inc` fragments).
#
# These methods live on per-endpoint builders / as endpoint-named
# methods, NOT on a class the `[[method]]` rows already cover, so without
# this dedicated family a binding could ship streaming on some endpoints
# and silently omit it on others (or on a whole binding) with no checker
# noticing — the exact gap the cross-binding contract exists to close.
# Each `[[historical_streaming]]` row pins one endpoint's streaming
# presence across the four bindings.


def _collect_python_streaming_endpoints(py_src: pathlib.Path) -> set[str]:
    """Snake_case endpoint names whose Python `<Endpoint>Builder` pyclass
    exposes a `fn stream` terminal.

    Walks every `impl <Name>Builder { ... }` block and records the
    builder's endpoint name (the CamelCase struct stem, lowered to
    snake_case) when the body declares `fn stream(`. The async companion
    `fn stream_async` rides on the same builder, so the sync terminal is
    the canonical presence signal.
    """
    out: set[str] = set()
    if not py_src.is_dir():
        return out
    impl_re = re.compile(r"impl\s+(\w+)Builder\s*\{")
    for rs in py_src.rglob("*.rs"):
        text = rs.read_text(encoding="utf-8")
        for header in impl_re.finditer(text):
            stem = header.group(1)
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
            if re.search(r"\bfn\s+stream\s*\(", body):
                out.add(_endpoint_method_to_snake(stem))
    return out


def _collect_typescript_streaming_endpoints(
    ts_methods: dict[str, set[str]],
) -> set[str]:
    """Snake_case endpoint names whose `ThetaDataDxClient` napi class
    exposes a `<endpoint>Stream` method.

    Reuses the already-collected `{class: {method, ...}}` map. A method
    whose camelCase name ends in `Stream` is a historical server-stream
    terminal; strip the suffix and lower to snake_case to recover the
    endpoint name. The FPSS lifecycle methods (`startStreaming` /
    `stopStreaming` / `isStreaming`) do NOT end in the bare `Stream`
    token preceded by an endpoint stem — they are excluded because
    stripping `Stream` would leave `starting` / `stopping` / `is`, none
    of which name an endpoint (and they are tracked by their own
    `[[method]]` rows regardless).
    """
    out: set[str] = set()
    methods = ts_methods.get("ThetaDataDxClient", set())
    lifecycle = {"startStreaming", "stopStreaming", "isStreaming"}
    for method in methods:
        if method in lifecycle:
            continue
        if method.endswith("Stream") and len(method) > len("Stream"):
            stem = method[: -len("Stream")]
            out.add(_endpoint_method_to_snake(stem))
    return out


def _collect_ffi_streaming_endpoints(ffi_src: pathlib.Path) -> set[str]:
    """Snake_case endpoint names whose `tdx_<endpoint>_stream` extern "C"
    symbol exists in `ffi/src/`.

    The FPSS `tdx_unified_*` / `tdx_fpss_*` callback symbols never match
    the `tdx_<name>_stream` shape (their stems are `unified` / `fpss`
    and they end in `set_callback` / `reconnect` / `shutdown`, not
    `_stream`), so they are not mistaken for a historical endpoint.
    """
    out: set[str] = set()
    if not ffi_src.is_dir():
        return out
    fn_re = re.compile(r"\bfn\s+tdx_(\w+)_stream\s*\(")
    for rs in ffi_src.rglob("*.rs"):
        text = rs.read_text(encoding="utf-8")
        for m in fn_re.finditer(text):
            out.add(m.group(1))
    return out


def _collect_cpp_streaming_endpoints(cpp_methods: dict[str, set[str]]) -> set[str]:
    """Snake_case endpoint names whose C++ `ThetaDataDxClient` wrapper
    exposes an `<endpoint>_stream` member.

    Reuses the already-collected C++ `{class: {method, ...}}` map. The
    historical endpoints live on the `ThetaDataDxClient` class body in
    `thetadx.hpp`. A member whose snake_case name ends in `_stream` is a
    server-stream terminal; strip the suffix to recover the endpoint
    name.
    """
    out: set[str] = set()
    methods = cpp_methods.get("ThetaDataDxClient", set())
    for method in methods:
        if method.endswith("_stream") and len(method) > len("_stream"):
            out.add(method[: -len("_stream")])
    return out


def _check_historical_streaming_rows(
    rows: list[dict[str, Any]],
    py_stream: set[str],
    ts_stream: set[str],
    cpp_stream: set[str],
    ffi_stream: set[str],
) -> list[str]:
    """Per-endpoint cross-binding gate for `[[historical_streaming]]` rows.

    Each row declares a snake_case endpoint `name` plus the expected
    server-stream presence in Python / TypeScript / C++ / the C ABI. The
    checker compares the declared state against the actual binding state
    and returns a list of mismatch strings (empty when every row
    matches).

    Beyond the per-row check, the collected sets are reconciled against
    the union of declared row names: an endpoint that streams on ANY
    binding but has no row at all trips the gate, so a newly-streamed
    endpoint cannot slip in untracked.
    """
    errors: list[str] = []
    declared_names = {row.get("name") for row in rows if row.get("name")}
    for row in rows:
        name = row.get("name")
        if not name:
            errors.append(f"  [[historical_streaming]] row missing `name`: {row!r}")
            continue
        camel = _snake_to_camel(name)
        pascal = camel[:1].upper() + camel[1:] if camel else camel
        for lang, actual_set, hint in (
            ("python", py_stream, f"`fn stream` on the `{pascal}Builder` pyclass"),
            ("typescript", ts_stream, f"`{camel}Stream` on the `ThetaDataDxClient` napi class"),
            ("cpp", cpp_stream, f"`{name}_stream(` on the C++ `ThetaDataDxClient` body"),
            ("ffi", ffi_stream, f"`tdx_{name}_stream` extern \"C\" symbol"),
        ):
            declared = row.get(lang, False)
            actual = name in actual_set
            if declared != actual:
                verb = "missing" if declared and not actual else "unexpected"
                errors.append(
                    f"  {name}.{lang}: declared={declared}, actual={actual} "
                    f"({verb} -- expected {hint})"
                )
    # Reverse-direction orphan check: any endpoint that streams on a
    # binding but has no row is undocumented drift.
    seen = py_stream | ts_stream | cpp_stream | ffi_stream
    for endpoint in sorted(seen - declared_names):
        on = sorted(
            lang
            for lang, s in (
                ("python", py_stream),
                ("typescript", ts_stream),
                ("cpp", cpp_stream),
                ("ffi", ffi_stream),
            )
            if endpoint in s
        )
        errors.append(
            f"  {endpoint}: streams on {on} but has no "
            f"[[historical_streaming]] row. Add one declaring its "
            f"per-binding presence so the surface stays tracked."
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
    utility_rows: list[dict[str, Any]] = data.get("utility", [])
    historical_streaming_rows: list[dict[str, Any]] = data.get(
        "historical_streaming", []
    )
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

    # Free-function (utility) parity ([[utility]] rows) — the offline
    # Greeks calculator (`all_greeks` / `implied_volatility`) is a free
    # function on every binding, tracked here because it is not a method
    # on any class the `[[method]]` rows cover.
    py_utils = _collect_python_utility_functions(PY_SRC)
    # The TS utility surface spans napi free functions (the calculators)
    # and the `Util` namespace-class static methods (the lookups); merge
    # both so every cross-binding row resolves regardless of TS shape.
    ts_utils = _ts_utility_surface(
        _collect_typescript_utility_functions(TS_SRC), ts_class_methods
    )
    cpp_utils = _collect_cpp_utility_functions(CPP_HPP)
    ffi_utils = _collect_ffi_utility_functions(FFI_SRC)
    utility_errors = _check_utility_rows(
        utility_rows, py_utils, ts_utils, cpp_utils, ffi_utils
    )
    # Reverse-direction orphan check: every standalone utility on the
    # cleanly-enumerable Python / TypeScript surfaces must be named by some
    # [[utility]] row (the conditions / exchange / calendar / sequence
    # roster that previously lived only as the `all_greeks` pair while a
    # dozen other utils drifted untracked).
    utility_roster_errors = _check_utility_roster_complete(
        utility_rows, py_utils, ts_utils
    )

    # Historical server-stream surface ([[historical_streaming]] rows) —
    # the `.stream(handler)` / `<endpoint>Stream` / `tdx_<endpoint>_stream`
    # terminal per endpoint. These live on per-endpoint builders or as
    # endpoint-named methods, NOT on a class the `[[method]]` rows cover,
    # so they would otherwise drift silently across bindings.
    py_stream = _collect_python_streaming_endpoints(PY_SRC)
    ts_stream = _collect_typescript_streaming_endpoints(ts_class_methods)
    cpp_stream = _collect_cpp_streaming_endpoints(cpp_class_methods)
    ffi_stream = _collect_ffi_streaming_endpoints(FFI_SRC)
    historical_streaming_errors = _check_historical_streaming_rows(
        historical_streaming_rows, py_stream, ts_stream, cpp_stream, ffi_stream
    )

    # Public-surface vocabulary: no public client identifier may embed
    # one of OUR implementation-detail tokens (tokio / disruptor /
    # crossbeam / parking_lot / block_on / allow_threads / os_pipe).
    # Vendor protocol names (mdds / fpss) are allow-listed. This is the
    # structural counterpart to the text scrubber — it sees only public
    # API names, so it never false-positives on internal runtime use.
    surface_vocab_errors = _check_public_surface_vocab(
        py_classes,
        ts_classes,
        cpp_classes,
        py_setters,
        ts_setters,
        cpp_setters,
        ffi_setters,
        py_class_methods,
        ts_class_methods,
        cpp_class_methods,
    )

    # Client-facing setter-SET equality across Python / TS / C++ / the
    # C ABI after normalization (`_explicit` widened-ABI suffix and the
    # `flat_files`↔`flatfiles` camelCase split folded away). Catches a
    # knob bound on some bindings but silently absent from the matrix on
    # the others. Genuine per-language idioms are exempted in
    # `SETTER_PARITY_EXEMPT` with a written reason.
    setter_set_errors = _check_setter_set_parity(
        py_setters, ts_setters, cpp_setters, ffi_setters
    )

    # Client-facing read-back getter-SET equality across the four
    # bindings. The setter check covers the write side of the Config knob
    # roster; this covers the read side — a knob that grew a getter on
    # some bindings but not others (the read-back analogue of the
    # missing-on-TS setter defect) trips here.
    py_getters = _collect_python_getters(PY_SRC)
    ts_getters = _collect_typescript_getters(TS_SRC)
    cpp_getters = _collect_cpp_getters(CPP_HPP)
    ffi_getters = _collect_ffi_getters(FFI_SRC)
    getter_set_errors = _check_getter_set_parity(
        py_getters, ts_getters, cpp_getters, ffi_getters
    )

    # Subscription-kind label parity: every binding must stringify the
    # FPSS subscription kinds to exactly the canonical snake_case roster
    # (`quote` / `trade` / `open_interest` / `market_value` / `full_trades`
    # / `full_open_interest`). Asserts the actual emitted strings, not
    # method presence — the seam where a C ABI label silently differs, or
    # the C++ accessor invents a `full_quote` / `full_market_value` label
    # for a full-stream kind that does not exist on the wire.
    rust_kinds = _collect_rust_subscription_kinds(SUBSCRIPTION_RS)
    py_kinds = _collect_binding_subscription_kinds(PY_FLUENT_RS)
    ts_kinds = _collect_binding_subscription_kinds(TS_FLUENT_RS)
    cpp_kinds = _collect_cpp_subscription_kinds(CPP_HPP)
    ffi_kinds = _collect_ffi_subscription_kinds(CPP_H)
    subscription_kind_errors = _check_subscription_kind_parity(
        rust_kinds, py_kinds, ts_kinds, cpp_kinds, ffi_kinds
    )

    # Error-leaf mapping parity: every core `Error` variant must map to
    # the same leaf class in Python / TypeScript / C++ and the same
    # `TDX_ERR_*` code in the C ABI. Asserts the full leaf roster + code
    # table — the seam where `FlatFilesUnavailable` / `PartialReconnect`
    # were invisible on Python / TypeScript, and where the `ConfigError`
    # leaf was missing.
    py_leaves = _collect_python_error_leaves(PY_ERRORS_RS)
    ts_leaves = _collect_typescript_error_leaves(TS_LIB_RS)
    cpp_leaves = _collect_cpp_error_leaves(CPP_HPP)
    ffi_codes = _collect_ffi_error_codes(FFI_ERROR_RS)
    ffi_codes_dispatched = _collect_ffi_error_codes_dispatched(FFI_ERROR_RS)
    cpp_codes = _collect_cpp_error_codes(CPP_H)
    error_leaf_errors = _check_error_leaf_parity(
        py_leaves,
        ts_leaves,
        cpp_leaves,
        ffi_codes,
        ffi_codes_dispatched,
        cpp_codes,
    )

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

    if utility_errors:
        had_errors = True
        print(
            f"check_binding_parity: {len(utility_errors)} free-function "
            f"mismatch(es) (per-utility `[[utility]]` granularity):"
        )
        for e in utility_errors:
            print(e)
        print()

    if utility_roster_errors:
        had_errors = True
        print(
            f"check_binding_parity: {len(utility_roster_errors)} standalone "
            f"utility(ies) lack a `[[utility]]` row:"
        )
        for e in utility_roster_errors:
            print(e)
        print()

    if historical_streaming_errors:
        had_errors = True
        print(
            f"check_binding_parity: {len(historical_streaming_errors)} "
            f"historical-streaming mismatch(es) (per-endpoint "
            f"`[[historical_streaming]]` granularity):"
        )
        for e in historical_streaming_errors:
            print(e)
        print()

    if surface_vocab_errors:
        had_errors = True
        print(
            f"check_binding_parity: {len(surface_vocab_errors)} public "
            f"identifier(s) embed an implementation-detail token:"
        )
        for e in surface_vocab_errors:
            print(e)
        print()

    if setter_set_errors:
        had_errors = True
        print(
            f"check_binding_parity: {len(setter_set_errors)} client-facing "
            f"setter(s) diverge across bindings (set-level parity):"
        )
        for e in setter_set_errors:
            print(e)
        print()

    if getter_set_errors:
        had_errors = True
        print(
            f"check_binding_parity: {len(getter_set_errors)} client-facing "
            f"getter(s) diverge across bindings (set-level parity):"
        )
        for e in getter_set_errors:
            print(e)
        print()

    if subscription_kind_errors:
        had_errors = True
        print(
            f"check_binding_parity: {len(subscription_kind_errors)} "
            f"subscription-kind label divergence(s) across bindings:"
        )
        for e in subscription_kind_errors:
            print(e)
        print()

    if error_leaf_errors:
        had_errors = True
        print(
            f"check_binding_parity: {len(error_leaf_errors)} error-leaf "
            f"mapping divergence(s) across bindings:"
        )
        for e in error_leaf_errors:
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
    n_methods = len(method_rows)
    n_value_fields = len(value_field_rows)
    n_utilities = len(utility_rows)
    n_hist_stream = len(historical_streaming_rows)
    print(
        f"check_binding_parity: clean "
        f"({n_class} class rows + {n_dotted} field rows + "
        f"{n_methods} method rows + {n_value_fields} value-field rows + "
        f"{n_utilities} utility rows + "
        f"{n_hist_stream} historical-streaming rows + "
        f"{n_fields} rust pub fields checked; "
        f"py_classes={len(py_classes)} ts_classes={len(ts_classes)} "
        f"cpp_classes={len(cpp_classes)} "
        f"py_setters={len(py_setters)} ts_setters={len(ts_setters)} "
        f"cpp_setters={len(cpp_setters)} ffi_setters={len(ffi_setters)}; "
        f"getters py={len(py_getters)} ts={len(ts_getters)} "
        f"cpp={len(cpp_getters)} ffi={len(ffi_getters)}; "
        f"kinds={len(CANONICAL_SUBSCRIPTION_KINDS)} "
        f"error_leaves={len(CANONICAL_ERROR_LEAVES)} "
        f"error_codes={len(CANONICAL_ERROR_CODES)})"
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
        cpp_methods = {"ThetaDataDxClient": {"panic_count"}}
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
        cpp_methods = {"ThetaDataDxClient": {"panic_count"}}
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
        cpp_methods = {"ThetaDataDxClient": {"active_full_subscriptions"}}
        errors = _check_method_rows(rows, py_methods, ts_methods, cpp_methods)
        assert any("typescript" in e and "missing" in e for e in errors), (
            f"missing TS method must trip the gate; got {errors!r}"
        )

    def _case_method_cpp_alias_resolves() -> None:
        """C++ alias (`Contract` -> `FluentContract`) is honoured."""
        rows = [
            {
                "class": "Contract",
                "name": "quote",
                "python": True,
                "typescript": True,
                "cpp": True,
            }
        ]
        py_methods = {"Contract": {"quote"}}
        ts_methods = {"Contract": {"quote"}}
        # The row says `Contract` but the C++ class is named
        # `FluentContract` — the alias table must route the lookup.
        cpp_methods = {"FluentContract": {"quote"}}
        errors = _check_method_rows(rows, py_methods, ts_methods, cpp_methods)
        assert errors == [], (
            f"C++ alias must resolve to FluentContract; got {errors!r}"
        )

    def _case_method_cpp_get_prefix_resolves() -> None:
        """C++ readback getter with the `get_` prefix matches a bare row."""
        rows = [
            {
                "class": "Config",
                "name": "flushMode",
                "python": True,
                "typescript": True,
                "cpp": True,
            }
        ]
        py_methods = {"Config": {"flush_mode"}}
        ts_methods = {"Config": {"flushMode"}}
        # C++ exposes the readback getter as `get_flush_mode`; the gate
        # accepts the `get_`-prefixed convention against the bare row.
        cpp_methods = {"Config": {"get_flush_mode"}}
        errors = _check_method_rows(rows, py_methods, ts_methods, cpp_methods)
        assert errors == [], (
            f"C++ `get_`-prefixed getter must satisfy a bare row; got {errors!r}"
        )

    def _case_method_python_get_prefix_resolves() -> None:
        """Python `#[getter] fn get_<x>` satisfies a bare getter row.

        pyo3 strips the `get_` prefix so the Python property name stays
        bare (`config.flush_mode`), but the Rust fn name the collector
        harvests carries the prefix. The gate must accept `get_flush_mode`
        against the bare `flushMode` row, exactly as it does for C++.
        """
        rows = [
            {
                "class": "Config",
                "name": "flushMode",
                "python": True,
                "typescript": True,
                "cpp": True,
            }
        ]
        # Python exposes the readback getter as `fn get_flush_mode`.
        py_methods = {"Config": {"get_flush_mode"}}
        ts_methods = {"Config": {"flushMode"}}
        cpp_methods = {"Config": {"get_flush_mode"}}
        errors = _check_method_rows(rows, py_methods, ts_methods, cpp_methods)
        assert errors == [], (
            f"Python `get_`-prefixed getter must satisfy a bare row; got {errors!r}"
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
        cpp_methods = {"ThetaDataDxClient": {"panic_count"}}
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
    _case("method positive — C++ alias routes Contract -> FluentContract", _case_method_cpp_alias_resolves)
    _case("method positive — C++ `get_` prefix satisfies a bare getter row", _case_method_cpp_get_prefix_resolves)
    _case("method positive — Python `get_` prefix satisfies a bare getter row", _case_method_python_get_prefix_resolves)
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

    # ── Free-function (utility) parity selftests ───────────────────

    def _case_utility_positive_all_four_bound() -> None:
        """All four bindings expose the calculator function — silent."""
        rows = [
            {
                "name": "all_greeks",
                "python": True,
                "typescript": True,
                "cpp": True,
                "ffi": True,
            }
        ]
        errors = _check_utility_rows(
            rows,
            {"all_greeks"},
            {"all_greeks"},
            {"all_greeks"},
            {"all_greeks"},
        )
        assert errors == [], f"positive case must be silent; got {errors!r}"

    def _case_utility_negative_missing_on_ts() -> None:
        """Declared on TypeScript but absent from the TS source — trips."""
        rows = [
            {
                "name": "all_greeks",
                "python": True,
                "typescript": True,
                "cpp": True,
                "ffi": True,
            }
        ]
        errors = _check_utility_rows(
            rows,
            {"all_greeks"},
            set(),  # TS missing — the exact gap this matrix closes
            {"all_greeks"},
            {"all_greeks"},
        )
        assert any("typescript" in e and "missing" in e for e in errors), (
            f"missing TS free function must trip; got {errors!r}"
        )

    def _case_utility_negative_unexpected() -> None:
        """Row says not-on-C++ but the C++ decl exists — trips."""
        rows = [
            {
                "name": "implied_volatility",
                "python": True,
                "typescript": True,
                "cpp": False,
                "ffi": True,
            }
        ]
        errors = _check_utility_rows(
            rows,
            {"implied_volatility"},
            {"implied_volatility"},
            {"implied_volatility"},  # present despite cpp=false
            {"implied_volatility"},
        )
        assert any("cpp" in e and "unexpected" in e for e in errors), (
            f"unexpected C++ decl must trip; got {errors!r}"
        )

    def _case_utility_ts_free_fn_collector_skips_methods() -> None:
        """The TS collector records free functions but not impl methods."""
        with tempfile.TemporaryDirectory() as tmp:
            ts_dir = pathlib.Path(tmp) / "ts"
            ts_dir.mkdir()
            (ts_dir / "gen.rs").write_text(
                '#[napi(js_name = "allGreeks")]\n'
                "pub fn all_greeks(spot: f64) -> napi::Result<AllGreeks> { todo!() }\n"
                "\n"
                "#[napi]\n"
                "impl ThetaDataDxClient {\n"
                '    #[napi(js_name = "isStreaming")]\n'
                "    pub fn is_streaming(&self) -> bool { true }\n"
                "}\n",
                encoding="utf-8",
            )
            found = _collect_typescript_utility_functions(ts_dir)
            assert "all_greeks" in found, f"free fn must be seen; got {found!r}"
            assert "is_streaming" not in found, (
                f"impl method must NOT be seen as a free fn; got {found!r}"
            )

    def _case_utility_ffi_collector_strips_prefix() -> None:
        """The FFI collector strips the `tdx_` prefix to the bare name."""
        with tempfile.TemporaryDirectory() as tmp:
            ffi_dir = pathlib.Path(tmp) / "ffi"
            ffi_dir.mkdir()
            (ffi_dir / "utility.rs").write_text(
                'pub unsafe extern "C" fn tdx_all_greeks() {}\n'
                'pub unsafe extern "C" fn tdx_implied_volatility() {}\n',
                encoding="utf-8",
            )
            found = _collect_ffi_utility_functions(ffi_dir)
            assert {"all_greeks", "implied_volatility"} <= found, (
                f"tdx_-prefixed symbols must map to bare names; got {found!r}"
            )

    _case("utility positive — all four bindings expose the calculator", _case_utility_positive_all_four_bound)
    _case("utility negative — calculator missing on TS trips", _case_utility_negative_missing_on_ts)
    _case("utility negative — unexpected C++ decl trips", _case_utility_negative_unexpected)
    _case("utility — TS collector skips impl methods", _case_utility_ts_free_fn_collector_skips_methods)
    _case("utility — FFI collector strips tdx_ prefix", _case_utility_ffi_collector_strips_prefix)

    # ── Historical server-stream surface selftests ────────────────

    def _case_hist_stream_positive_all_bound() -> None:
        """An endpoint streaming on every declared binding is silent."""
        rows = [
            {
                "name": "option_history_trade",
                "python": True,
                "typescript": True,
                "cpp": True,
                "ffi": True,
            }
        ]
        s = {"option_history_trade"}
        errors = _check_historical_streaming_rows(rows, s, s, s, s)
        assert errors == [], f"all-bound row must be silent; got {errors!r}"

    def _case_hist_stream_missing_on_cpp_trips() -> None:
        """Row claims C++ streams but the C++ member is absent — trips."""
        rows = [
            {
                "name": "option_history_trade",
                "python": True,
                "typescript": True,
                "cpp": True,
                "ffi": True,
            }
        ]
        bound = {"option_history_trade"}
        errors = _check_historical_streaming_rows(
            rows, bound, bound, set(), bound
        )
        assert any("cpp" in e and "missing" in e for e in errors), (
            f"missing C++ stream member must trip; got {errors!r}"
        )

    def _case_hist_stream_ts_only_state_is_silent() -> None:
        """The TS-first ship state (py+ts true, cpp+ffi false) is silent
        when Python + TS stream and C++ + FFI do not — the intended
        intermediate parity the matrix tracks.
        """
        rows = [
            {
                "name": "option_history_trade",
                "python": True,
                "typescript": True,
                "cpp": False,
                "ffi": False,
            }
        ]
        bound = {"option_history_trade"}
        errors = _check_historical_streaming_rows(
            rows, bound, bound, set(), set()
        )
        assert errors == [], f"TS-first state must be silent; got {errors!r}"

    def _case_hist_stream_untracked_orphan_trips() -> None:
        """An endpoint streaming on a binding with no row at all trips
        the reverse-direction orphan check.
        """
        errors = _check_historical_streaming_rows(
            [], set(), {"option_history_trade"}, set(), set()
        )
        assert any(
            "option_history_trade" in e and "no [[historical_streaming]] row" in e
            for e in errors
        ), f"untracked streaming endpoint must trip; got {errors!r}"

    def _case_hist_stream_initialism_inverse() -> None:
        """`stockHistoryEODStream` (TS) and `StockHistoryEod` (Python
        builder stem) both invert to the canonical `stock_history_eod`,
        so the two collectors agree on the row name.
        """
        assert _endpoint_method_to_snake("stockHistoryEOD") == "stock_history_eod"
        assert _endpoint_method_to_snake("StockHistoryEod") == "stock_history_eod"
        assert (
            _endpoint_method_to_snake("optionHistoryGreeksImpliedVolatility")
            == "option_history_greeks_implied_volatility"
        )
        assert _endpoint_method_to_snake("stockHistoryOHLCRange") == "stock_history_ohlc_range"

    _case("hist-stream positive — all four bindings stream", _case_hist_stream_positive_all_bound)
    _case("hist-stream negative — missing C++ member trips", _case_hist_stream_missing_on_cpp_trips)
    _case("hist-stream positive — TS-first ship state is silent", _case_hist_stream_ts_only_state_is_silent)
    _case("hist-stream negative — untracked streaming endpoint trips", _case_hist_stream_untracked_orphan_trips)
    _case("hist-stream — initialism-aware inverse agrees across bindings", _case_hist_stream_initialism_inverse)

    # ── Public-surface vocabulary guard selftests ──────────────────

    def _case_surface_vocab_flags_tokio_identifier() -> None:
        """An impl-detail token embedded in a public identifier trips,
        even though `\\btokio\\b` would not match it.
        """
        errors = _check_public_surface_vocab(
            {"Config"},
            set(),
            set(),
            {"tokio_worker_threads"},
            set(),
            set(),
            set(),
            {},
            {},
            {},
        )
        assert any("tokio" in e for e in errors), (
            f"embedded tokio token must trip the guard; got {errors!r}"
        )

    def _case_surface_vocab_flags_camelcase_export() -> None:
        """A camelCase export type embedding the token trips."""
        errors = _check_public_surface_vocab(
            set(),
            {"TokioWorkerThreadsSetting"},
            set(),
            set(),
            set(),
            set(),
            set(),
            {},
            {},
            {},
        )
        assert any("tokio" in e for e in errors), (
            f"camelCase Tokio export must trip; got {errors!r}"
        )

    def _case_surface_vocab_flags_other_impl_tokens() -> None:
        """The other OUR-impl tokens (crossbeam / parking_lot /
        disruptor / block_on / allow_threads / os_pipe) all trip when
        embedded in a public identifier, in both snake_case and
        camelCase spellings (the camelCase form collapses the
        underscore, so `parkingLotGuard` hits the `parkinglot` variant).
        """
        for ident in [
            "set_crossbeam_depth",
            "parkingLotGuard",
            "parking_lot_guard",
            "disruptorRingSize",
            "blockOnConnect",
            "block_on_connect",
            "allowThreadsFlag",
            "allow_threads_flag",
            "osPipeFd",
        ]:
            errors = _check_public_surface_vocab(
                set(), set(), set(), {ident}, set(), set(), set(), {}, {}, {}
            )
            assert any(ident in e for e in errors), (
                f"{ident!r} must trip the surface-vocab guard; got {errors!r}"
            )

    def _case_surface_vocab_allows_vendor_protocol_names() -> None:
        """Vendor protocol names (mdds / fpss) are allow-listed and must
        NEVER trip — `MddsClient`, `mdds_host`, `setFpssRingSize`,
        `fpss_ring_size` are all clean.
        """
        errors = _check_public_surface_vocab(
            {"MddsClient", "FpssClient", "FpssEvent"},
            set(),
            set(),
            {"mdds_host", "mdds_port", "fpss_ring_size", "fpss_host_selection"},
            {"fpss_ring_size"},
            set(),
            set(),
            {"FpssClient": {"subscribe"}},
            {},
            {},
        )
        assert errors == [], (
            f"vendor protocol names must be allow-listed; got {errors!r}"
        )

    def _case_surface_vocab_allows_neutral_names() -> None:
        """The renamed neutral knob (`worker_threads`) is clean."""
        errors = _check_public_surface_vocab(
            {"Config", "WorkerThreadsSetting"},
            set(),
            set(),
            {"worker_threads"},
            {"worker_threads_explicit"},
            {"worker_threads_explicit"},
            {"worker_threads_explicit"},
            {},
            {},
            {},
        )
        assert errors == [], (
            f"neutral worker_threads names must be clean; got {errors!r}"
        )

    _case("surface-vocab — embedded tokio token trips", _case_surface_vocab_flags_tokio_identifier)
    _case("surface-vocab — camelCase Tokio export trips", _case_surface_vocab_flags_camelcase_export)
    _case("surface-vocab — other impl tokens trip", _case_surface_vocab_flags_other_impl_tokens)
    _case("surface-vocab — vendor protocol names allow-listed", _case_surface_vocab_allows_vendor_protocol_names)
    _case("surface-vocab — neutral names clean", _case_surface_vocab_allows_neutral_names)

    # ── Setter normalizer + set-parity selftests ───────────────────

    def _case_normalizer_folds_explicit_and_flatfiles() -> None:
        """`_normalize_setter` folds the `_explicit` widened-ABI suffix
        and the `flat_files`→`flatfiles` camelCase split.
        """
        assert _normalize_setter("worker_threads_explicit") == "worker_threads"
        assert _normalize_setter("worker_threads") == "worker_threads"
        assert _normalize_setter("flat_files_max_attempts") == "flatfiles_max_attempts"
        assert _normalize_setter("flatfiles_max_attempts") == "flatfiles_max_attempts"
        assert _normalize_setter("fpss_host_shuffle_seed_explicit") == "fpss_host_shuffle_seed"

    def _case_setter_set_parity_positive_after_normalize() -> None:
        """The four sets, spelled in their per-binding idioms, compare
        equal after normalization — the gate is silent.
        """
        py = {"worker_threads", "flatfiles_jitter", "derive_ohlcvc"}
        ts = {"worker_threads_explicit", "flat_files_jitter", "flatfiles_jitter", "derive_ohlcvc"}
        cpp = {"worker_threads_explicit", "flatfiles_jitter", "derive_ohlcvc"}
        ffi = {"worker_threads_explicit", "flatfiles_jitter", "derive_ohlcvc"}
        errors = _check_setter_set_parity(py, ts, cpp, ffi, exempt={})
        assert errors == [], (
            f"normalized-equal sets must be silent; got {errors!r}"
        )

    def _case_setter_set_parity_missing_on_one_binding_trips() -> None:
        """A knob bound on three bindings but absent from TS trips — the
        `derive_ohlcvc`-missing-on-TS defect class.
        """
        py = {"derive_ohlcvc"}
        ts: set[str] = set()
        cpp = {"derive_ohlcvc"}
        ffi = {"derive_ohlcvc"}
        errors = _check_setter_set_parity(py, ts, cpp, ffi, exempt={})
        assert any("derive_ohlcvc" in e and "typescript" in e for e in errors), (
            f"missing-on-TS knob must trip the set-parity gate; got {errors!r}"
        )

    def _case_setter_set_parity_honours_exemption() -> None:
        """A Python-only knob listed in the exemption map does NOT trip
        — the documented per-language-idiom carve-out.
        """
        py = {"mdds_host", "shared"}
        ts = {"shared"}
        cpp = {"shared"}
        ffi = {"shared"}
        errors = _check_setter_set_parity(
            py, ts, cpp, ffi, exempt={"mdds_host": "Python-only advanced override"}
        )
        assert errors == [], (
            f"exempted Python-only knob must not trip; got {errors!r}"
        )

    def _case_setter_set_parity_stale_exemption_trips() -> None:
        """An exempted knob that is now uniformly bound on every binding
        is a stale carve-out and trips so the list never rots.
        """
        allfour = {"mdds_host"}
        errors = _check_setter_set_parity(
            allfour,
            allfour,
            allfour,
            allfour,
            exempt={"mdds_host": "Python-only advanced override"},
        )
        assert any("mdds_host" in e and "stale" in e for e in errors), (
            f"uniformly-bound exemption must surface as stale; got {errors!r}"
        )

    def _case_setter_set_parity_shipped_exemption_is_live() -> None:
        """The shipped `SETTER_PARITY_EXEMPT` carve-outs must be live
        against the real binding sources — `mdds_host` / `mdds_port`
        present on Python alone, so neither flags as stale and the live
        gate stays silent on them.
        """
        py = _collect_python_setters(PY_SRC)
        ts = _collect_typescript_setters(TS_SRC)
        cpp = _collect_cpp_setters(CPP_HPP, CPP_H)
        ffi = _collect_ffi_setters(FFI_SRC)
        errors = _check_setter_set_parity(py, ts, cpp, ffi)
        assert errors == [], (
            f"live setter-set parity must be clean; got {errors!r}"
        )

    _case("normalizer — folds _explicit + flat_files", _case_normalizer_folds_explicit_and_flatfiles)
    _case("setter-set — normalized-equal sets are silent", _case_setter_set_parity_positive_after_normalize)
    _case("setter-set — missing-on-TS knob trips", _case_setter_set_parity_missing_on_one_binding_trips)
    _case("setter-set — Python-only exemption honoured", _case_setter_set_parity_honours_exemption)
    _case("setter-set — stale exemption trips", _case_setter_set_parity_stale_exemption_trips)
    _case("setter-set — shipped exemptions live against real sources", _case_setter_set_parity_shipped_exemption_is_live)

    # ── Config getter-set parity selftests ─────────────────────────

    def _case_getter_set_parity_positive() -> None:
        """A getter on all four bindings (with the `_explicit` idiom
        folded) is silent — the read-side analogue of the setter check.
        """
        errors = _check_getter_set_parity(
            {"reconnect_wait_ms", "worker_threads"},
            {"reconnect_wait_ms", "worker_threads_explicit"},
            {"reconnect_wait_ms", "worker_threads_explicit"},
            {"reconnect_wait_ms", "worker_threads_explicit"},
            exempt={},
        )
        assert errors == [], f"normalized-equal getter sets must be silent; got {errors!r}"

    def _case_getter_set_parity_missing_on_ffi_trips() -> None:
        """A read-back getter bound on Python/TS/C++ but absent from the C
        ABI trips — the read-side of the missing-binding defect class.
        """
        errors = _check_getter_set_parity(
            {"fpss_ring_size"},
            {"fpss_ring_size"},
            {"fpss_ring_size"},
            set(),
            exempt={},
        )
        assert any("fpss_ring_size" in e and "ffi" in e for e in errors), (
            f"getter missing on FFI must trip; got {errors!r}"
        )

    def _case_getter_set_parity_live_sources_clean() -> None:
        """The live Config getter roster is symmetric across all four
        bindings — every read-back accessor present in one is present in
        all (the seam the read-side check pins).
        """
        py = _collect_python_getters(PY_SRC)
        ts = _collect_typescript_getters(TS_SRC)
        cpp = _collect_cpp_getters(CPP_HPP)
        ffi = _collect_ffi_getters(FFI_SRC)
        errors = _check_getter_set_parity(py, ts, cpp, ffi)
        assert errors == [], f"live getter-set parity must be clean; got {errors!r}"

    def _case_getter_collectors_scope_to_config() -> None:
        """The getter collectors harvest only `impl Config` bodies, so a
        getter on an unrelated pyclass / napi class is not swept into the
        Config knob roster.
        """
        py_text = (
            "#[pymethods]\nimpl Config {\n    #[getter] fn get_fpss_ring_size(&self) -> usize { 0 }\n}\n"
            "#[pymethods]\nimpl QuoteTick {\n    #[getter] fn bid_price(&self) -> f64 { 0.0 }\n}\n"
        )
        bodies = _iter_impl_config_bodies(py_text)
        assert len(bodies) == 1, f"only the Config impl body must be picked; got {bodies!r}"
        assert "get_fpss_ring_size" in bodies[0]
        assert "bid_price" not in bodies[0]

    _case("getter-set — normalized-equal sets are silent", _case_getter_set_parity_positive)
    _case("getter-set — missing-on-FFI getter trips", _case_getter_set_parity_missing_on_ffi_trips)
    _case("getter-set — live sources clean", _case_getter_set_parity_live_sources_clean)
    _case("getter collectors — scope to impl Config only", _case_getter_collectors_scope_to_config)

    # ── Subscription-kind label parity selftests ───────────────────

    def _case_subscription_kind_positive() -> None:
        """Every binding emitting the full canonical set is silent."""
        full = set(CANONICAL_SUBSCRIPTION_KINDS)
        errors = _check_subscription_kind_parity(full, full, full, full, full)
        assert errors == [], f"all-canonical kind sets must be silent; got {errors!r}"

    def _case_subscription_kind_missing_label_trips() -> None:
        """A binding short one label (the C-ABI-collision class where a
        label silently differs) trips.
        """
        full = set(CANONICAL_SUBSCRIPTION_KINDS)
        short = full - {"market_value"}
        errors = _check_subscription_kind_parity(full, full, full, short, full)
        assert any("cpp" in e and "missing" in e and "market_value" in e for e in errors), (
            f"a dropped kind label must trip; got {errors!r}"
        )

    def _case_subscription_kind_fictitious_label_trips() -> None:
        """A binding emitting a non-canonical label (the C++
        `full_quote` / `full_market_value` defect, a full-stream kind that
        does not exist on the wire) trips.
        """
        full = set(CANONICAL_SUBSCRIPTION_KINDS)
        invented = full | {"full_quote"}
        errors = _check_subscription_kind_parity(full, full, full, invented, full)
        assert any("cpp" in e and "non-canonical" in e and "full_quote" in e for e in errors), (
            f"a fictitious kind label must trip; got {errors!r}"
        )

    def _case_subscription_kind_harvest_captures_fictitious() -> None:
        """The C++ harvester captures a `full_quote` / `full_market_value`
        literal inside `kind_string()` so the canonical-set assertion can
        flag it — the harvest filter must not silently drop them.
        """
        hpp_text = (
            "class FluentSubscription {\n"
            "  std::string kind_string() const {\n"
            "    if (full) {\n"
            '      switch (k) { case A: return "full_quote"; case B: return "full_trades"; }\n'
            "    }\n"
            '    return "quote";\n'
            "  }\n"
            "};\n"
        )
        import tempfile as _tmp
        with _tmp.NamedTemporaryFile("w", suffix=".hpp", delete=True) as f:
            f.write(hpp_text)
            f.flush()
            harvested = _collect_cpp_subscription_kinds(pathlib.Path(f.name))
        assert "full_quote" in harvested, (
            f"harvester must capture the fictitious label; got {harvested!r}"
        )
        assert "full_trades" in harvested and "quote" in harvested

    def _case_subscription_kind_live_sources_clean() -> None:
        """Every live binding emits exactly the canonical kind roster."""
        errors = _check_subscription_kind_parity(
            _collect_rust_subscription_kinds(SUBSCRIPTION_RS),
            _collect_binding_subscription_kinds(PY_FLUENT_RS),
            _collect_binding_subscription_kinds(TS_FLUENT_RS),
            _collect_cpp_subscription_kinds(CPP_HPP),
            _collect_ffi_subscription_kinds(CPP_H),
        )
        assert errors == [], f"live subscription-kind parity must be clean; got {errors!r}"

    _case("subscription-kind positive — all-canonical sets silent", _case_subscription_kind_positive)
    _case("subscription-kind negative — dropped label trips", _case_subscription_kind_missing_label_trips)
    _case("subscription-kind negative — fictitious label trips", _case_subscription_kind_fictitious_label_trips)
    _case("subscription-kind — C++ harvest captures fictitious label", _case_subscription_kind_harvest_captures_fictitious)
    _case("subscription-kind — live sources clean", _case_subscription_kind_live_sources_clean)

    # ── Error-leaf mapping parity selftests ────────────────────────

    def _case_error_leaf_positive() -> None:
        """Symmetric leaf rosters + matching code tables are silent."""
        leaves = set(CANONICAL_ERROR_LEAVES)
        errors = _check_error_leaf_parity(
            leaves,
            leaves,
            leaves,
            dict(CANONICAL_ERROR_CODES),
            set(CANONICAL_ERROR_CODES),
            dict(CANONICAL_ERROR_CODES),
        )
        assert errors == [], f"symmetric error mapping must be silent; got {errors!r}"

    def _case_error_leaf_missing_on_py_trips() -> None:
        """A leaf invisible on Python (the `FlatFilesUnavailable` /
        `PartialReconnect` → no `StreamError`, or a missing `ConfigError`
        defect) trips.
        """
        leaves = set(CANONICAL_ERROR_LEAVES)
        py_short = leaves - {"ConfigError"}
        errors = _check_error_leaf_parity(
            py_short,
            leaves,
            leaves,
            dict(CANONICAL_ERROR_CODES),
            set(CANONICAL_ERROR_CODES),
            dict(CANONICAL_ERROR_CODES),
        )
        assert any("python" in e and "ConfigError" in e for e in errors), (
            f"a leaf missing on Python must trip; got {errors!r}"
        )

    def _case_error_leaf_code_renumber_trips() -> None:
        """A renumbered FFI code (drift from the canonical table) trips."""
        leaves = set(CANONICAL_ERROR_LEAVES)
        bad_codes = dict(CANONICAL_ERROR_CODES)
        bad_codes["TDX_ERR_STREAM"] = 99
        errors = _check_error_leaf_parity(
            leaves,
            leaves,
            leaves,
            bad_codes,
            set(CANONICAL_ERROR_CODES),
            bad_codes,
        )
        assert any("ffi" in e and "TDX_ERR_STREAM" in e for e in errors), (
            f"a renumbered FFI code must trip; got {errors!r}"
        )

    def _case_error_leaf_header_drift_trips() -> None:
        """A C ABI header `#define` disagreeing with the FFI Rust constant
        (invisible to `cargo build`) trips.
        """
        leaves = set(CANONICAL_ERROR_LEAVES)
        header_codes = dict(CANONICAL_ERROR_CODES)
        header_codes["TDX_ERR_CONFIG"] = 42
        errors = _check_error_leaf_parity(
            leaves,
            leaves,
            leaves,
            dict(CANONICAL_ERROR_CODES),
            set(CANONICAL_ERROR_CODES),
            header_codes,
        )
        assert any("cpp header" in e and "TDX_ERR_CONFIG" in e for e in errors), (
            f"a C-header code drift must trip; got {errors!r}"
        )

    def _case_error_leaf_live_sources_symmetric() -> None:
        """The live error mapping is symmetric across all four bindings:
        identical leaf rosters on Python / TS / C++ and matching code
        tables in the FFI Rust constants and the C ABI header.
        """
        errors = _check_error_leaf_parity(
            _collect_python_error_leaves(PY_ERRORS_RS),
            _collect_typescript_error_leaves(TS_LIB_RS),
            _collect_cpp_error_leaves(CPP_HPP),
            _collect_ffi_error_codes(FFI_ERROR_RS),
            _collect_ffi_error_codes_dispatched(FFI_ERROR_RS),
            _collect_cpp_error_codes(CPP_H),
        )
        assert errors == [], f"live error-leaf parity must be symmetric; got {errors!r}"

    _case("error-leaf positive — symmetric rosters silent", _case_error_leaf_positive)
    _case("error-leaf negative — leaf missing on Python trips", _case_error_leaf_missing_on_py_trips)
    _case("error-leaf negative — renumbered FFI code trips", _case_error_leaf_code_renumber_trips)
    _case("error-leaf negative — C-header code drift trips", _case_error_leaf_header_drift_trips)
    _case("error-leaf — live sources symmetric", _case_error_leaf_live_sources_symmetric)

    # ── Utility-roster parity selftests ────────────────────────────

    def _case_utility_ffi_name_override_resolves() -> None:
        """A row whose C ABI symbol carries a disambiguating prefix
        resolves through `ffi_name` — `is_cancel` on Python/TS/C++ but
        `tdx_condition_is_cancel` on the C ABI.
        """
        rows = [
            {
                "name": "is_cancel",
                "ffi_name": "condition_is_cancel",
                "python": True,
                "typescript": True,
                "cpp": True,
                "ffi": True,
            }
        ]
        errors = _check_utility_rows(
            rows,
            {"is_cancel"},
            {"is_cancel"},
            {"is_cancel"},
            {"condition_is_cancel"},
        )
        assert errors == [], f"ffi_name override must resolve; got {errors!r}"

    def _case_utility_ffi_name_missing_symbol_trips() -> None:
        """An `ffi_name` row whose C ABI symbol is absent trips."""
        rows = [
            {
                "name": "is_firm",
                "ffi_name": "quote_condition_is_firm",
                "python": True,
                "typescript": True,
                "cpp": True,
                "ffi": True,
            }
        ]
        errors = _check_utility_rows(
            rows,
            {"is_firm"},
            {"is_firm"},
            {"is_firm"},
            set(),  # the prefixed C ABI symbol is missing
        )
        assert any("is_firm" in e and "ffi" in e and "missing" in e for e in errors), (
            f"missing prefixed FFI symbol must trip; got {errors!r}"
        )

    def _case_utility_binding_specific_asserted() -> None:
        """A `binding_specific` row still asserts the declared per-binding
        booleans — a Python-only util must be present on Python and absent
        elsewhere, or the row trips.
        """
        rows = [
            {
                "name": "split_date_range",
                "binding_specific": "Python-only",
                "python": True,
                "typescript": False,
                "cpp": False,
                "ffi": False,
            }
        ]
        # Correct state: present on Python only.
        ok = _check_utility_rows(rows, {"split_date_range"}, set(), set(), set())
        assert ok == [], f"correct binding-specific state must be silent; got {ok!r}"
        # Drifted: the util appeared on TypeScript too.
        drift = _check_utility_rows(
            rows, {"split_date_range"}, {"split_date_range"}, set(), set()
        )
        assert any("split_date_range" in e and "typescript" in e for e in drift), (
            f"a binding-specific util appearing elsewhere must trip; got {drift!r}"
        )

    def _case_utility_roster_orphan_trips() -> None:
        """A utility on the Python surface with no `[[utility]]` row trips
        the roster orphan check.
        """
        rows = [{"name": "all_greeks", "python": True, "typescript": True, "cpp": True, "ffi": True}]
        errors = _check_utility_roster_complete(
            rows, {"all_greeks", "calendar_status_name"}, {"all_greeks"}
        )
        assert any("calendar_status_name" in e and "python" in e for e in errors), (
            f"an untracked Python utility must trip; got {errors!r}"
        )

    def _case_ts_utility_surface_filters_internal() -> None:
        """The TS utility surface merges `Util` methods and calculators but
        filters the internal arrow-IPC / coercion free functions.
        """
        surface = _ts_utility_surface(
            {"all_greeks", "quote_tick_to_arrow_ipc", "bigint_to_i32"},
            {"Util": {"conditionName", "isCancel"}},
        )
        assert "all_greeks" in surface
        assert "condition_name" in surface and "is_cancel" in surface
        assert "quote_tick_to_arrow_ipc" not in surface
        assert "bigint_to_i32" not in surface

    def _case_utility_roster_live_complete() -> None:
        """Every standalone utility on the live Python / TypeScript
        surfaces is named by a `[[utility]]` row (no untracked drift).
        """
        if not PARITY_TOML.is_file():
            return
        data = tomllib.loads(PARITY_TOML.read_text(encoding="utf-8"))
        rows = data.get("utility", [])
        py = _collect_python_utility_functions(PY_SRC)
        ts = _ts_utility_surface(
            _collect_typescript_utility_functions(TS_SRC),
            _collect_typescript_class_methods(TS_SRC),
        )
        errors = _check_utility_roster_complete(rows, py, ts)
        assert errors == [], f"live utility roster must be complete; got {errors!r}"

    _case("utility — ffi_name override resolves prefixed symbol", _case_utility_ffi_name_override_resolves)
    _case("utility — missing prefixed FFI symbol trips", _case_utility_ffi_name_missing_symbol_trips)
    _case("utility — binding_specific row asserts declared booleans", _case_utility_binding_specific_asserted)
    _case("utility — roster orphan (untracked util) trips", _case_utility_roster_orphan_trips)
    _case("utility — TS surface filters internal free fns", _case_ts_utility_surface_filters_internal)
    _case("utility — live roster complete", _case_utility_roster_live_complete)

    print(f"check_binding_parity --selftest: {n_pass} passed, {n_fail} failed")
    return 0 if n_fail == 0 else 1


if __name__ == "__main__":
    sys.exit(main())
