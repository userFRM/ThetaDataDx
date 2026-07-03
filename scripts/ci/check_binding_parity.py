#!/usr/bin/env python3
"""Cross-binding parity check (Gate 2 / issue #545 + #595).

Reads `parity.toml` — the declared cross-binding presence matrix
— and compares each row's `python` / `typescript` / `cpp` claims to
the actual binding state extracted from:

Class-level rows (no dot in `name`):
- Python: every `m.add_class::<T>()` registered in `lib.rs` + helper
  `register_*` calls, expanded statically by parsing the Rust source.
  Mirrors the regex powering `test_no_pyclass_name_collisions.py`.
- TypeScript: the package entry resolved from `thetadatadx-ts/package.json`
  `types` (declarations) and `main` (runtime). The gate scans that entry,
  follows its `export * from './...'` re-exports, and harvests
  `export class X` / `export declare class X` / `export declare const X`
  declarations plus the runtime `module.exports` names. This sees the
  actually-shipped surface (the napi `index.*` re-export PLUS the wrapper-side
  classes such as the context-managed session and the typed-error leaves).
- C++: `^class X` / `^struct X` declarations in
  `thetadatadx-cpp/include/thetadatadx.hpp`. The `.h` header is C-only and not
  considered for parity.

Field-level rows (dotted `name`, e.g. `ReconnectConfig.wait_ms`):
- Python: `#[setter] fn set_<canonical>` and `#[getter] fn <canonical>`
  parsed from `thetadatadx-py/src/*.rs`. The canonical name composes the
  struct prefix (e.g. `reconnect_`) with the row suffix (`wait_ms`).
- TypeScript napi: `#[napi(js_name = "set<CamelCase>")]` and the
  matching getter declaration in `thetadatadx-ts/src/*.rs`. The
  CamelCase form lifts the snake_case canonical name.
- C++: `set_<canonical>` / `get_<canonical>` member functions on the
  `class Config { ... }` body in `thetadatadx.hpp` PLUS the matching
  `thetadatadx_config_set_<canonical>` C-ABI declaration in `thetadatadx.h`.
- FFI: `thetadatadx_config_set_<canonical>` AND
  `thetadatadx_config_get_<canonical>` (or the `_explicit` widened-ABI shape)
  parsed from `thetadatadx-ffi/src/*.rs`. Any binding flagged `true` on a field
  row implies the FFI symbol exists, because every higher-level
  binding forwards into the same C ABI.

Rust-only rows: a dotted row with `rust_only = true` MUST cite an
`issue = "#N"` tracking number. The script enforces both — a
`rust_only` flag with no issue or an `issue` flag with no `rust_only`
fails the gate.

Historical endpoint families (`[[historical_base]]` /
`[[historical_async]]` / `[[historical_streaming]]`): the buffered,
async-query, and server-stream surfaces per endpoint. Each carries a
`rust` column whose source of truth is the registry of record,
`thetadatadx-rs/endpoint_surface.toml` — the file the build pipeline
generates every binding's historical method from. The Rust buffered
surface is every `[[endpoints]]` entry except the four `*_stream` FPSS
subscription endpoints (61 endpoints); the Rust streaming subset mirrors
the build's `endpoint_streams` SSOT (list / snapshot / calendar endpoints
get no server-stream terminal). `[[historical_base]]` additionally pins
the C-ABI `thetadatadx_<endpoint>_with_options` base symbol read from the
SHIPPED header (`thetadatadx-cpp/include/endpoint_with_options.h.inc`) and
cross-checks the shipped header, the `thetadatadx-ffi/src` source, and the registry
for agreement so a stale regenerated header is caught. This is the core
"every endpoint exists on all five surfaces" guarantee.

Exits non-zero on any mismatch. Run from the repo root.

The `.rs` / `.hpp` / `.h` / `.inc` collectors read source with C-style
comments stripped first (`_read_source` / `_read_cpp_expanded`), so a
symbol left behind only in a comment (a deleted-but-not-removed
declaration) is not counted as present and cannot mask cross-binding
drift. The one collector that intentionally harvests the documented
label vocabulary FROM a doc comment
(`_collect_ffi_subscription_kinds`) reads raw, and says so at its call.

A `--selftest` switch runs an in-process synthetic-source matrix
covering positive (all-bound) and negative (missing-on-TS,
missing-on-C++, missing-on-FFI, undocumented-orphan, rust_only-
without-issue) cases, plus comment-stripping cases proving a
commented-out declaration is ignored. The selftest is registered with
the audit-protocol convention for CI gates.
"""

from __future__ import annotations

import json
import pathlib
import re
import sys
import tempfile
import tomllib
from typing import Any


REPO_ROOT = pathlib.Path(__file__).resolve().parents[2]
PARITY_TOML = REPO_ROOT / "parity.toml"
PY_SRC = REPO_ROOT / "thetadatadx-py" / "src"
# The shipped PEP 561 stub — the client-facing Python type surface
# (mypy / pyright) the `python_pyi` signature lane checks against the spec.
PY_PYI = REPO_ROOT / "thetadatadx-py" / "python" / "thetadatadx" / "__init__.pyi"
TS_PKG_DIR = REPO_ROOT / "thetadatadx-ts"


def _resolve_ts_entry(pkg_dir: pathlib.Path, key: str, fallback: str) -> pathlib.Path:
    """Resolve the package's declared entry file for a `package.json` key.

    The published TypeScript package declares its root through `package.json`
    `main` (the runtime `.js` entry) and `types` (the `.d.ts` entry). The
    parity gate must scan THOSE files, not a hardcoded `index.*`, so it sees
    the surface a consumer actually imports. The entry may re-export the napi
    `index.*` and add its own wrapper-side exports (the context-managed
    streaming session, the typed-error classes). Falls back to `fallback`
    (e.g. `index.d.ts`) only when the key is absent so a package without an
    explicit `main`/`types` still resolves.
    """
    pkg_json = pkg_dir / "package.json"
    declared: str | None = None
    if pkg_json.is_file():
        try:
            declared = json.loads(pkg_json.read_text(encoding="utf-8")).get(key)
        except (json.JSONDecodeError, OSError):
            declared = None
    return pkg_dir / (declared or fallback)


# The package entry the gate scans for the TypeScript surface. Resolved from
# `package.json` `types` (declarations) / `main` (runtime) so the gate reads
# the actually-shipped root, which re-exports the napi `index.*` and layers
# the wrapper-side exports on top.
TS_DTS = _resolve_ts_entry(TS_PKG_DIR, "types", "index.d.ts")
TS_MAIN_JS = _resolve_ts_entry(TS_PKG_DIR, "main", "index.js")
TS_SRC = REPO_ROOT / "thetadatadx-ts" / "src"
# The committed generated napi historical surface. The `<endpoint>WithColumns`
# reachability gate reads its `#[napi(js_name = ...)]` attributes directly (a
# deterministic no-build source the napi compile lowers into `index.d.ts`).
TS_HISTORICAL_METHODS_RS = TS_SRC / "_generated" / "historical_methods.rs"
CPP_HPP = REPO_ROOT / "thetadatadx-cpp" / "include" / "thetadatadx.hpp"
CPP_H = REPO_ROOT / "thetadatadx-cpp" / "include" / "thetadatadx.h"
FFI_SRC = REPO_ROOT / "thetadatadx-ffi" / "src"
CONFIG_DIR = REPO_ROOT / "thetadatadx-rs" / "src" / "config"
RUST_CLIENT_BUILDER_RS = (
    REPO_ROOT / "thetadatadx-rs" / "src" / "client_builder.rs"
)
# Core Rust streaming surfaces whose public observability accessors must
# each carry a parity row. The unified surface lives on the
# `StreamSurface` view returned by `Client::stream()`; the standalone
# surface is the `StreamingClient` FPSS client.
CORE_CLIENT_RS = REPO_ROOT / "thetadatadx-rs" / "src" / "client.rs"
CORE_FPSS_MOD_RS = REPO_ROOT / "thetadatadx-rs" / "src" / "fpss" / "mod.rs"
# The canonical endpoint registry of record. The build pipeline generates
# the Rust historical surface (`HistoricalClient::<endpoint>` methods + the
# per-endpoint streaming builders) from this file; the parity gate reads it
# directly so a dropped or renamed Rust historical endpoint trips without a
# full build. Every `[[endpoints]]` entry not named `*_stream` (the four FPSS
# real-time subscription endpoints) is one of the buffered historical
# endpoints (the 61-endpoint base surface).
ENDPOINT_SURFACE_TOML = (
    REPO_ROOT / "thetadatadx-rs" / "endpoint_surface.toml"
)
# The shipped C-ABI base header fragment. `thetadatadx.h` includes it; it
# declares one `thetadatadx_<endpoint>_with_options` extern "C" symbol per
# buffered endpoint. Reading the SHIPPED header (not just the `thetadatadx-ffi/src`
# source) catches a stale regenerated header that drifted from the Rust
# source of truth.
ENDPOINT_WITH_OPTIONS_INC = (
    REPO_ROOT / "thetadatadx-cpp" / "include" / "endpoint_with_options.h.inc"
)
# The two generated consumers of the request-options SSOT: the C++ fluent
# `with_*` setters and the FFI `#[repr(C)]` bridge struct. Both are emitted
# from `endpoint_surface.toml` and must carry the same option roster.
ENDPOINT_OPTIONS_HPP_INC = (
    REPO_ROOT / "thetadatadx-cpp" / "include" / "endpoint_options.hpp.inc"
)
ENDPOINT_REQUEST_OPTIONS_RS = (
    REPO_ROOT / "thetadatadx-ffi" / "src" / "endpoint_request_options.rs"
)


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
    `mdds_client` / `fpss` engine stem is never flagged.
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
        text = _read_source(rs)
        for m in PYCLASS_RE.finditer(text):
            out.add(_python_name(m.group(1), m.group(2)))
    errors_rs = py_src / "errors.rs"
    if errors_rs.is_file():
        for m in re.finditer(r'm\.add\(\s*"(\w+)"\s*,', _read_source(errors_rs)):
            out.add(m.group(1))
    return out


# `export class X` / `export declare class X` (the `declare` keyword is
# optional: the napi `index.d.ts` declares ambient classes, while the
# wrapper entry ships concrete `export class` leaves such as the typed-error
# hierarchy).
TS_CLASS_RE = re.compile(r"export\s+(?:declare\s+)?(?:abstract\s+)?class\s+(\w+)")
TS_INTERFACE_RE = re.compile(r"export\s+(?:declare\s+)?interface\s+(\w+)")
# Runtime-class-const shapes: a `const` export whose value is a real runtime
# constructor. Two forms ship on the wrapper entry, and both must be held to a
# matching runtime export exactly like a declared `class`:
#
#   export const Contract: typeof ContractRef;              (alias to a class)
#   export declare const StreamingSession: { new (...): X } (inline ctor object)
#
# The `declare` keyword is optional: the `Contract` alias is emitted without it
# (`export const Contract: typeof ContractRef`), while `StreamingSession` ships
# with it. The annotation MUST be a `typeof` alias or an object type carrying a
# `new` constructor signature; a plain value const (`export declare const
# VERSION: string`) compiles to no constructor, so it is NOT a runtime class and
# must not be forced to have a runtime export. `const enum` declarations are
# excluded because the captured token would be `enum`, which is not followed by
# the `:` annotation this pattern requires.
TS_CONST_CLASS_RE = re.compile(
    r"export\s+(?:declare\s+)?const\s+(\w+)\s*:\s*"
    r"(?:typeof\s+\w+|\{[^}]*\bnew\b[^}]*\})"
)
# `export * from './<module>'`: the wrapper entry re-exports the whole napi
# surface this way, so the gate must follow the re-export to see those
# declarations as part of the scanned entry.
TS_REEXPORT_RE = re.compile(r"export\s+\*\s+from\s+['\"]([^'\"]+)['\"]")


def _ts_resolve_module(from_file: pathlib.Path, spec: str) -> pathlib.Path | None:
    """Resolve a relative module specifier (`./index`) to a `.d.ts` file
    next to `from_file`, honoring an explicit or implied `.d.ts` extension."""
    if not spec.startswith("."):
        return None
    base = (from_file.parent / spec).resolve()
    for cand in (base, base.with_suffix(".d.ts"), base.parent / (base.name + ".d.ts")):
        if cand.is_file():
            return cand
    return None


def _collect_ts_dts_classes(
    dts: pathlib.Path, _seen: set[pathlib.Path] | None = None
) -> set[str]:
    """Harvest every exported class / interface / runtime-class-const name
    declared in `dts`, following `export * from './...'` re-exports so the
    declared package entry's full surface is seen."""
    classes, interfaces = _collect_ts_dts_class_kinds(dts, _seen)
    return classes | interfaces


def _collect_ts_dts_class_kinds(
    dts: pathlib.Path, _seen: set[pathlib.Path] | None = None
) -> tuple[set[str], set[str]]:
    """Harvest declaration-side names from `dts`, split by runtime kind.

    Returns `(runtime_classes, interfaces)`:

    - `runtime_classes` are `export class` / `export declare class` and the
      `export declare const X: { new (...): X }` runtime-class shape. These
      compile to a real constructor that MUST be reachable at runtime, so a
      row declaring one is held to having a matching runtime export.
    - `interfaces` are `export interface` declarations (the napi `#[napi(object)]`
      plain-object shape and hand-written interfaces). These are erased at
      compile time and carry no runtime constructor, so a row backed by one
      is satisfied by the declaration alone.

    Follows `export * from './...'` re-exports so the declared package entry's
    full surface is seen.
    """
    classes: set[str] = set()
    interfaces: set[str] = set()
    if not dts.is_file():
        return classes, interfaces
    seen = _seen if _seen is not None else set()
    resolved = dts.resolve()
    if resolved in seen:
        return classes, interfaces
    seen.add(resolved)
    text = dts.read_text(encoding="utf-8")
    for rx in (TS_CLASS_RE, TS_CONST_CLASS_RE):
        for m in rx.finditer(text):
            classes.add(m.group(1))
    for m in TS_INTERFACE_RE.finditer(text):
        interfaces.add(m.group(1))
    for m in TS_REEXPORT_RE.finditer(text):
        target = _ts_resolve_module(dts, m.group(1))
        if target is not None:
            sub_classes, sub_interfaces = _collect_ts_dts_class_kinds(target, seen)
            classes |= sub_classes
            interfaces |= sub_interfaces
    return classes, interfaces


# Runtime (`.js`) export shapes the wrapper entry uses:
# `exports.X = Y` / `module.exports.X = Y` (napi-generated) and
# `module.exports = Object.assign(..., { X, Y, ... })` (the wrapper's
# object-shorthand re-export of its hand-written classes). The object-literal
# scan is bounded to identifier shorthand / `Name: expr` entries so it cannot
# pick up arbitrary code. `module.exports.X` contains the `exports.X` substring,
# so the same pattern catches both forms.
JS_EXPORTS_ASSIGN_RE = re.compile(r"exports\.(\w+)\s*=")
JS_REQUIRE_RE = re.compile(r"require\(\s*['\"]([^'\"]+)['\"]\s*\)")
# A local binding initialised straight from a relative `require`
# (`const native = require('./index.js')`). Captured so a require'd module's
# surface is followed ONLY when its binding is later re-exported on
# `module.exports` (a bare `const helper = require('./helper')` used purely for
# its side effects must NOT leak its names into the runtime surface).
JS_REQUIRE_BINDING_RE = re.compile(
    r"\b(?:const|let|var)\s+([A-Za-z_$][\w$]*)\s*=\s*"
    r"require\(\s*['\"](\.[^'\"]+)['\"]\s*\)"
)
# Inline relative `require` sitting in a genuine re-export position:
#   module.exports = require('./x')
#   module.exports = Object.assign(<...> require('./x') <...>)   (assign arg)
#   module.exports = { ...require('./x') }                       (object spread)
#   Object.assign(module.exports, require('./x'))                (assign onto exports)
#   exports.NAME = require('./x') / module.exports.NAME = require('./x')
# Each form actually places the required surface on this module's exports, so
# its names are part of the shipped runtime surface and must be followed.
JS_REEXPORT_WHOLE_RE = re.compile(
    r"module\.exports\s*=\s*require\(\s*['\"](\.[^'\"]+)['\"]\s*\)"
)
JS_REEXPORT_EXPORTS_PROP_REQUIRE_RE = re.compile(
    r"(?:module\.)?exports\.\w+\s*=\s*require\(\s*['\"](\.[^'\"]+)['\"]\s*\)"
)
JS_REEXPORT_OBJECT_SPREAD_REQUIRE_RE = re.compile(
    r"\{[^{}]*\.\.\.\s*require\(\s*['\"](\.[^'\"]+)['\"]\s*\)[^{}]*\}"
)
JS_REEXPORT_ASSIGN_ONTO_EXPORTS_RE = re.compile(
    r"Object\.assign\(\s*module\.exports\b([^;]*)\)", re.DOTALL
)
# Identifier-shorthand (`Name,`) and `Name: expr` keys of an object literal.
# The trailing delimiter is a lookahead, not a consumed group, so a key never
# eats the comma that the next key needs to anchor on; otherwise alternating
# keys drop out of the scan and a runtime export reads as absent.
JS_OBJECT_KEY_RE = re.compile(r"(?:[{,])\s*([A-Za-z_$][\w$]*)\s*(?=[,:}])")
# Strip `/* ... */` block comments and `// ...` line comments. The runtime
# export blocks carry explanatory comments between the keys; without removing
# them the whitespace-then-comment run breaks the object-key scan's anchor.
JS_BLOCK_COMMENT_RE = re.compile(r"/\*.*?\*/", re.DOTALL)
JS_LINE_COMMENT_RE = re.compile(r"//[^\n]*")


def _strip_js_comments(text: str) -> str:
    """Remove block and line comments from JS source.

    Export blocks rely on identifier / delimiter adjacency to be scanned; an
    interleaved comment otherwise hides the keys that follow it. The export
    names this gate reads are bare identifiers, never string literals, so a
    coarse comment strip is sufficient and cannot drop a tracked name.
    """
    return JS_LINE_COMMENT_RE.sub("", JS_BLOCK_COMMENT_RE.sub("", text))


# Rust, C, and C++ share JavaScript's `//` line / `/* */` block comment
# syntax (Rust `///` and `//!` doc comments and `/** */` are subsumed),
# so the same coarse strip applies. Every symbol this gate reads from
# those sources is a bare identifier in declaration position, never a
# string-literal payload, so stripping comments before the symbol regexes
# cannot drop a tracked name — but it DOES stop a deleted symbol left
# behind in a comment (`// removed: historical()`) from reading as still
# present, which would otherwise let real cross-binding drift pass.
def _read_source(path: pathlib.Path) -> str:
    """Read a Rust / C / C++ source file with comments stripped."""
    return _strip_js_comments(path.read_text(encoding="utf-8"))


# napi attribute argument lists can contain a callback type with its own
# parentheses, e.g.
#   #[napi(ts_args_type = "cb: (e: Event) => void", js_name = "setCallback")]
# The `#[napi(` ... `)]` argument list therefore is NOT
# parenthesis-free, so the historical `#[napi(...[^)]*...)]` collectors
# stopped at the FIRST inner `)` (inside `(e: Event)`) and never reached
# the trailing `js_name`, silently reading a callback-typed method as
# ABSENT. The helper below scans the attribute argument list with a paren
# balance counter (respecting string literals) so the full inner content
# — across any nested `(...)` — is returned for sub-matching.
_NAPI_ATTR_START_RE = re.compile(r"#\[\s*napi\b")


def _iter_napi_attrs(text: str):
    """Yield `(inner, after)` for every `#[napi ...]` attribute in `text`.

    `inner` is the argument list between the `(` after `napi` and its
    balanced closing `)` (empty string for the bare `#[napi]` form with no
    args). `after` is the index just past the attribute's closing `]`, so a
    caller can resume scanning for the `fn <name>` that the attribute
    decorates. The paren/bracket walk respects `"..."` string literals
    (with `\\` escapes), so a `)` or `]` inside a `ts_args_type` /
    `ts_return_type` string does not terminate the span early.
    """
    n = len(text)
    for m in _NAPI_ATTR_START_RE.finditer(text):
        i = m.end()
        # Skip whitespace between `napi` and an optional `(`.
        j = i
        while j < n and text[j].isspace():
            j += 1
        inner = ""
        if j < n and text[j] == "(":
            # Walk to the balanced `)`, tracking string literals.
            depth = 0
            k = j
            in_str = False
            esc = False
            start_inner = j + 1
            while k < n:
                c = text[k]
                if in_str:
                    if esc:
                        esc = False
                    elif c == "\\":
                        esc = True
                    elif c == '"':
                        in_str = False
                elif c == '"':
                    in_str = True
                elif c == "(":
                    depth += 1
                elif c == ")":
                    depth -= 1
                    if depth == 0:
                        inner = text[start_inner:k]
                        k += 1
                        break
                k += 1
            else:
                # Unbalanced (malformed source) — skip this attribute.
                continue
            pos = k
        else:
            pos = j
        # Advance to the attribute's closing `]` (also string-aware so a
        # `]` inside a string literal in the args does not fool us; we
        # already consumed the args, so this only spans `)]` / `]`).
        while pos < n and text[pos] != "]":
            pos += 1
        after = pos + 1 if pos < n else n
        yield inner, after


def _read_cpp_expanded(cpp_hpp: pathlib.Path) -> str:
    """Read a C++ wrapper header, inline its `.inc` includes, and strip
    comments from the combined text.

    The `.inc` fragments extend class bodies with generator-emitted
    declarations; they must be inlined BEFORE the symbol scan and stripped
    alongside the host header so a comment in either the header or an
    included fragment cannot hide a member declaration.
    """
    expanded = _expand_cpp_includes(
        cpp_hpp.read_text(encoding="utf-8"), cpp_hpp.parent
    )
    return _strip_js_comments(expanded)


def _resolve_js_module(from_js: pathlib.Path, spec: str) -> pathlib.Path | None:
    """Resolve a relative module specifier (`./index`) to a sibling `.js`
    file, honoring an explicit or implied `.js` extension. Returns ``None``
    for a non-relative specifier or one that resolves to no file on disk."""
    if not spec.startswith("."):
        return None
    target = from_js.parent / spec
    for cand in (target, target.with_suffix(".js"), target.parent / (target.name + ".js")):
        if cand.is_file():
            return cand
    return None


def _js_reexported_require_specs(text: str) -> set[str]:
    """Relative `require(...)` specifiers this module actually RE-EXPORTS.

    A required module's surface is part of the shipped runtime surface only
    when it is genuinely placed on `module.exports`. This recognises the
    re-export forms and returns their specifiers; a bare
    `const helper = require('./helper')` used only for its side effects is
    deliberately excluded, so helper-only names never read as runtime exports.

    Followed forms:

    - ``module.exports = require('./x')`` — whole-module re-export.
    - ``exports.NAME = require('./x')`` / ``module.exports.NAME = require('./x')``
      — a single binding re-exported from a require.
    - ``module.exports = { ...require('./x') }`` — object-spread re-export.
    - ``Object.assign(module.exports, require('./x'), ...)`` — assign onto
      ``module.exports``; every inline `require(...)` argument is re-exported.
    - ``const native = require('./x'); module.exports = Object.assign({}, native, ...)``
      — a require-bound name spread into the export object / passed as an
      ``Object.assign`` argument that builds ``module.exports`` (the wrapper
      entry's shape). The binding counts only when it is referenced as a bare
      re-export argument, not merely defined.
    """
    specs: set[str] = set()

    # Inline `require(...)` directly in a re-export position.
    for m in JS_REEXPORT_WHOLE_RE.finditer(text):
        specs.add(m.group(1))
    for m in JS_REEXPORT_EXPORTS_PROP_REQUIRE_RE.finditer(text):
        specs.add(m.group(1))
    for m in JS_REEXPORT_OBJECT_SPREAD_REQUIRE_RE.finditer(text):
        specs.add(m.group(1))

    # `Object.assign` blocks that build / extend `module.exports`: the
    # `module.exports = Object.assign(<args>)` body and the
    # `Object.assign(module.exports, <args>)` body. A bare top-level identifier
    # argument here is a wholesale spread of that value's own enumerable props
    # (`Object.assign({}, native, ...)`), so a require-bound name in this
    # position re-exports the required surface; an inline `require(...)`
    # argument does the same.
    assign_arg_blocks = [
        m.group(1)
        for m in re.finditer(
            r"module\.exports\s*=\s*Object\.assign\(([^;]*)\)", text, re.DOTALL
        )
    ]
    assign_arg_blocks += [
        m.group(1) for m in JS_REEXPORT_ASSIGN_ONTO_EXPORTS_RE.finditer(text)
    ]
    for block in assign_arg_blocks:
        for inner in JS_REQUIRE_RE.finditer(block):
            if inner.group(1).startswith("."):
                specs.add(inner.group(1))

    # `require`-bound local names, mapped to their specifier. Such a binding
    # contributes its surface ONLY when its name is forwarded WHOLESALE — as a
    # bare `Object.assign` argument (`Object.assign({}, native, ...)`) or an
    # object spread (`{ ...native }`). A `const helper = require('./helper')`
    # referenced only internally, or used as a property VALUE
    # (`{ alias: helper }` re-exports the single key `alias`, not the surface),
    # never contributes its names.
    binding_specs: dict[str, str] = {
        m.group(1): m.group(2) for m in JS_REQUIRE_BINDING_RE.finditer(text)
    }
    if binding_specs:
        # Bare identifier argument of an `Object.assign` export block.
        for block in assign_arg_blocks:
            for arg in re.split(r",", block):
                stripped = arg.strip()
                if stripped in binding_specs:
                    specs.add(binding_specs[stripped])
        # Object-spread of a bound name in an object that becomes
        # `module.exports`: `module.exports = { ...native }` and any object
        # literal passed to an `Object.assign` export block
        # (`Object.assign({}, { ...native })`). Spreads in unrelated internal
        # objects are deliberately NOT followed.
        spread_bodies = [
            m.group(1)
            for m in re.finditer(
                r"module\.exports\s*=\s*\{([^{}]*)\}", text, re.DOTALL
            )
        ]
        for block in assign_arg_blocks:
            spread_bodies += [
                obj.group(1) for obj in re.finditer(r"\{([^{}]*)\}", block, re.DOTALL)
            ]
        for body in spread_bodies:
            for spread in re.finditer(r"\.\.\.\s*([A-Za-z_$][\w$]*)", body):
                name = spread.group(1)
                if name in binding_specs:
                    specs.add(binding_specs[name])

    return specs


def _collect_js_exports(js: pathlib.Path, _seen: set[pathlib.Path] | None = None) -> set[str]:
    """Harvest the runtime export names from a `.js` entry: `exports.X = ...`,
    the keys of a `module.exports = Object.assign(..., { ... })` object, and
    (transitively) the exports of any `require('./...')` it actually RE-EXPORTS.

    This is the TRUE shipped surface: the names a consumer can reach at
    runtime through the package `main` entry. A class declared only in the
    `.d.ts` but dropped from this set is not actually exported, so the gate
    must never substitute a declaration-side hit for a runtime export.

    Recursion follows a `require(...)` ONLY when its surface is genuinely
    placed on this module's `module.exports` (see
    `_js_reexported_require_specs`). A side-effect / helper
    `require('./helper')` whose names are not re-exported does NOT contribute,
    so helper-only names cannot masquerade as package runtime exports.
    """
    out: set[str] = set()
    if not js.is_file():
        return out
    seen = _seen if _seen is not None else set()
    resolved = js.resolve()
    if resolved in seen:
        return out
    seen.add(resolved)
    text = _strip_js_comments(js.read_text(encoding="utf-8"))
    for m in JS_EXPORTS_ASSIGN_RE.finditer(text):
        out.add(m.group(1))
    # A `require`-bound local name (`const native = require('./index.js')`) is a
    # spread SOURCE, not an export NAME; its surface is followed transitively,
    # so the binding identifier itself must not read as a runtime export.
    require_bindings = {m.group(1) for m in JS_REQUIRE_BINDING_RE.finditer(text)}
    # `module.exports = Object.assign(..., { StreamingSession, ThetaDataError, ... })`:
    # capture each object-literal block passed to the export assignment and
    # read its identifier-shorthand / `Name:` keys.
    for obj in re.finditer(
        r"module\.exports\s*=\s*Object\.assign\(([^;]*)\)", text, re.DOTALL
    ):
        for key in JS_OBJECT_KEY_RE.finditer(obj.group(1)):
            name = key.group(1)
            if name not in {"Object", "assign"} and name not in require_bindings:
                out.add(name)
    # Follow ONLY the requires this module genuinely re-exports — the napi
    # `index.js` surface ships through the wrapper entry via
    # `module.exports = Object.assign({}, native, ...)` (with
    # `const native = require('./index.js')`), so that chain still resolves.
    for spec in _js_reexported_require_specs(text):
        target = _resolve_js_module(js, spec)
        if target is not None:
            out |= _collect_js_exports(target, seen)
    return out


def _collect_ts_runtime_classes(ts_dts: pathlib.Path) -> set[str]:
    """Runtime export names reachable through the package `main` entry.

    Resolves the runtime entry from the package directory the `types` entry
    lives in (so a caller passing a synthetic `types` path picks up the sibling
    `.js`), then harvests its export names. This is the shipped surface a
    consumer can `require`; the class-presence check holds every declared
    runtime class to a hit here.
    """
    return _collect_js_exports(_resolve_ts_entry(ts_dts.parent, "main", "index.js"))


def collect_typescript_classes(ts_dts: pathlib.Path) -> set[str]:
    """Every TypeScript surface name: declarations unioned with runtime.

    Scans the declared package entry (`package.json` `types`), following its
    `export * from './...'` re-exports, and unions the runtime entry
    (`package.json` `main`) export names. This union is the name UNIVERSE used
    by the public-surface vocabulary scan (which only needs the full set of
    identifiers to check for banned tokens). It deliberately does NOT decide
    class presence: a name present in the `.d.ts` but absent from the runtime
    is in this union yet is NOT a shipped class. Presence is decided by
    `_collect_ts_dts_class_kinds` (declaration kind) plus
    `_collect_ts_runtime_classes` (runtime export) so a declaration-side hit
    can never stand in for a dropped runtime export.
    """
    classes, interfaces = _collect_ts_dts_class_kinds(ts_dts)
    return classes | interfaces | _collect_ts_runtime_classes(ts_dts)


def _ts_class_presence(
    name: str,
    ts_declared_classes: set[str],
    ts_declared_interfaces: set[str],
    ts_runtime_exports: set[str],
) -> tuple[bool, str | None]:
    """Decide whether `name` is shipped on the TypeScript surface.

    Returns `(present, detail)`. `present` is the actual-state boolean the
    class-row check compares against the row's `typescript` claim; `detail`
    is a non-empty string only when the name is in a half-shipped state the
    gate must call out even where it would otherwise read as present:

    - declared as a runtime `class` / runtime-class `const` but missing from
      the runtime export set: NOT present. A consumer importing it gets
      `undefined`, so this is a dropped export, not a typing detail.
    - exported at runtime but with no declaration of any kind: present, but
      flagged as an untyped runtime export (a `.d.ts` gap) so it cannot pass
      silently on a declaration the package never ships.
    - declared only as an `interface` (the napi plain-object shape, erased at
      compile time): present on the declaration alone, since the type carries
      no runtime constructor to export.
    """
    declared_class = name in ts_declared_classes
    declared_interface = name in ts_declared_interfaces
    runtime = name in ts_runtime_exports
    if declared_class:
        if runtime:
            return True, None
        return (
            False,
            "declared on the TypeScript .d.ts as a runtime class but missing "
            "from the package runtime export; a consumer importing it resolves "
            "to undefined (restore the export on the package `main` entry)",
        )
    if declared_interface:
        # Object-shape type: no runtime constructor, declaration is the surface.
        return True, None
    if runtime:
        return (
            True,
            "exported at runtime with no matching .d.ts declaration (add the "
            "declaration so the typed surface matches what ships)",
        )
    return False, None


def _check_class_rows(
    rows: list[dict[str, Any]],
    py_classes: set[str],
    cpp_classes: set[str],
    ts_declared_classes: set[str],
    ts_declared_interfaces: set[str],
    ts_runtime_exports: set[str],
) -> list[str]:
    """Class-level presence parity for the non-dotted `[[class]]` rows.

    Compares each row's `python` / `typescript` / `cpp` claim to the actual
    binding state. The TypeScript verdict goes through `_ts_class_presence`, so
    a row marked `typescript = true` requires the runtime export when the class
    is a real runtime class; a `.d.ts` declaration never stands in for a dropped
    runtime export, and an untyped runtime export is flagged as a typing gap.
    """
    errors: list[str] = []
    for row in rows:
        name = row["name"]
        if "." in name:
            continue
        for lang, declared in (
            ("python", row["python"]),
            ("typescript", row["typescript"]),
            ("cpp", row["cpp"]),
        ):
            detail: str | None = None
            if lang == "python":
                actual = name in py_classes
            elif lang == "typescript":
                actual, detail = _ts_class_presence(
                    name,
                    ts_declared_classes,
                    ts_declared_interfaces,
                    ts_runtime_exports,
                )
            else:
                actual = cpp_has(name, cpp_classes)
            if actual != declared:
                verb = "missing" if declared and not actual else "unexpected"
                suffix = f" -- {detail}" if detail else ""
                errors.append(
                    f"  {name}.{lang}: declared={declared}, actual={actual} "
                    f"({verb}){suffix}"
                )
            elif detail and declared:
                # actual == declared (both true) yet the surface is half-shipped
                # (e.g. a runtime export with no declaration). Flag it so a
                # typing/parity gap cannot pass silently behind a true claim.
                errors.append(f"  {name}.{lang}: {detail}")
    return errors


CPP_CLASS_RE = re.compile(r"^(?:class|struct)\s+(\w+)", re.MULTILINE)
CPP_USING_RE = re.compile(r"^using\s+(\w+)\s*=", re.MULTILINE)


def collect_cpp_classes(cpp_hpp: pathlib.Path) -> set[str]:
    out: set[str] = set()
    if not cpp_hpp.is_file():
        return out
    text = _read_source(cpp_hpp)
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
    "ParseError": "StreamParseError",
    # The unified client's sub-namespace views carry the Python / TypeScript
    # canonical names in `parity.toml`; the C++ header names them without the
    # `View` suffix (`client.historical()` -> `Historical`, `client.stream()`
    # -> `Stream`).
    "HistoricalView": "Historical",
    "StreamView": "Stream",
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


# Parity-toml `class` field → the Rust struct name the Python method
# collector keys on. The collector harvests methods under the bare Rust
# struct identifier (`impl PyContract`), while the cross-binding row uses
# the canonical pyclass `name` (`Contract`). The fluent builders are the
# load-bearing case: `Contract.quote()` lives on `impl PyContract`,
# `SecType.fullTrades()` on `impl PySecType`. Routing the row through this
# table lets a single canonical row resolve against the Python source.
PY_CLASS_ALIASES: dict[str, str] = {
    "Contract": "PyContract",
    "SecType": "PySecType",
    "Subscription": "PySubscription",
}

# Parity-toml `class` field → the Rust struct name the TypeScript method
# collector keys on (lifted from `impl <Name>`). The per-contract fluent
# builders live on the napi `ContractRef` impl, which the cross-binding
# matrix tracks under the canonical `Contract` name (the `ContractRef`
# class itself carries its own `[[class]]` row). The full-stream builders
# live on `impl SecType`, already the canonical name.
TS_CLASS_ALIASES: dict[str, str] = {
    "Contract": "ContractRef",
}


def _py_class_for(class_name: str) -> str:
    """Resolve a parity-toml `class` field to the Python collector's key.

    Honors `PY_CLASS_ALIASES` so a row carrying the canonical pyclass name
    (`Contract`) routes to the Rust struct the collector harvests
    (`PyContract`).
    """
    return PY_CLASS_ALIASES.get(class_name, class_name)


def _ts_class_for(class_name: str) -> str:
    """Resolve a parity-toml `class` field to the TypeScript collector's key.

    Honors `TS_CLASS_ALIASES` so a row carrying the canonical name
    (`Contract`) routes to the napi impl the collector harvests
    (`ContractRef`).
    """
    return TS_CLASS_ALIASES.get(class_name, class_name)


def _py_methods_for(class_name: str, py_methods: dict[str, set[str]]) -> set[str]:
    """Python method set for a parity-toml `class`, trying the aliased
    struct key first and falling back to the bare canonical name.

    The fallback keeps synthetic selftest matrices keyed by the canonical
    name (`{"Contract": {"quote"}}`) resolving while the production
    collector keys by the Rust struct (`PyContract`).
    """
    aliased = _py_class_for(class_name)
    if aliased in py_methods:
        return py_methods[aliased]
    return py_methods.get(class_name, set())


def _ts_methods_for(class_name: str, ts_methods: dict[str, set[str]]) -> set[str]:
    """TypeScript method set for a parity-toml `class`, trying the aliased
    impl key first and falling back to the bare canonical name."""
    aliased = _ts_class_for(class_name)
    if aliased in ts_methods:
        return ts_methods[aliased]
    return ts_methods.get(class_name, set())


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
    }:
        return True
    return False


# ─── Field-level discovery (per-setter granularity / #595) ──────────


# Struct → setter-name prefix. The Rust struct lives on
# `DirectConfig.<accessor>`, but the binding-side setter name combines
# the prefix with the row's field suffix. E.g. `ReconnectConfig.wait_ms`
# resolves to Python `set_reconnect_wait_ms`, TS `setReconnectWaitMs`,
# C++ `set_reconnect_wait_ms`, FFI `thetadatadx_config_set_reconnect_wait_ms`.
STRUCT_TO_PREFIX: dict[str, str] = {
    "HistoricalConfig": "",
    "StreamingConfig": "",
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
# `thetadatadx_config_set_<field>_explicit` as the canonical setter. Accept
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
        text = _read_source(rs)
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
        text = _read_source(rs)
        # Scope to `impl Config { ... }` blocks so a `set<X>` napi method
        # on a non-Config class (e.g. the live `StreamView` streaming
        # `setCallback`) is not mistaken for a
        # Config knob setter. This mirrors the Python collector, which
        # relies on `#[setter]` being a Config-property-only attribute,
        # and the C++/FFI collectors, which intersect with the
        # `thetadatadx_config_set_*` C ABI surface.
        for body in _iter_impl_config_bodies(text):
            # `#[napi(js_name = "setX")]` → setter `X` (drop the `set` prefix).
            # Scan the balanced attribute arg list so a callback-typed
            # method (`ts_args_type = "cb: (e) => void", js_name = "setX"`)
            # is still seen — the old `[^)]*` stopped at the inner `)`.
            for inner, _ in _iter_napi_attrs(body):
                m = re.search(r'\bjs_name\s*=\s*"set([A-Z]\w*)"', inner)
                if m:
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
    `thetadatadx_config_set_<name>` declaration in `thetadatadx.h` is the C ABI
    surface the wrapper forwards to; the parity gate requires both
    halves so a forgotten C header declaration trips at link time.
    Getter presence is not gated — several write-only knobs have no
    C++ getter by design (matching the FFI / Python / TS contract).
    """
    cpp_setters: set[str] = set()
    if cpp_hpp.is_file():
        # Inline the `.inc` fragments (`config_accessors.hpp.inc` extends
        # the `Config` body with generator-emitted setters) before the
        # scan, matching `_collect_cpp_getters` / `_collect_cpp_class_methods`.
        text = _read_cpp_expanded(cpp_hpp)
        for m in re.finditer(r"\bvoid\s+set_(\w+)\s*\(", text):
            cpp_setters.add(m.group(1))
        # Some C++ setters return `int32_t` for status codes (the
        # `_explicit` widened-ABI shape on `Option<usize>` fields).
        for m in re.finditer(r"\bint32_t\s+set_(\w+)\s*\(", text):
            cpp_setters.add(m.group(1))
    h_setters: set[str] = set()
    if cpp_h.is_file():
        text = _read_source(cpp_h)
        for m in re.finditer(r"\bthetadatadx_config_set_(\w+)\s*\(", text):
            h_setters.add(m.group(1))
    return cpp_setters & h_setters


def _collect_ffi_setters(ffi_src: pathlib.Path) -> set[str]:
    """FFI extern C setter declarations in `thetadatadx-ffi/src/*.rs`. The
    convention is ``thetadatadx_config_set_<name>``. Getter presence is not
    gated — several write-only knobs (e.g. the per-class reconnect
    budgets) have no FFI getter by design.
    """
    setters: set[str] = set()
    if not ffi_src.is_dir():
        return setters
    for rs in ffi_src.rglob("*.rs"):
        text = _read_source(rs)
        for m in re.finditer(r"\bfn\s+thetadatadx_config_set_(\w+)\s*\(", text):
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
#   * C ABI: `thetadatadx_config_get_<name>`.


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
        text = _read_source(rs)
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
        text = _read_source(rs)
        for body in _iter_impl_config_bodies(text):
            # A getter is `#[napi(getter, js_name = "<X>")]` in either
            # attribute order. Scan the balanced arg list and require both
            # the `getter` flag and a `js_name` to be present, so order is
            # irrelevant and a callback-typed arg cannot truncate the scan.
            for inner, _ in _iter_napi_attrs(body):
                if not re.search(r"\bgetter\b", inner):
                    continue
                m = re.search(r'\bjs_name\s*=\s*"([a-zA-Z_]\w*)"', inner)
                if m:
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
    text = _read_cpp_expanded(cpp_hpp)
    m = re.search(r"^class\s+Config\s*(?::[^{]*)?\{", text, re.MULTILINE)
    if not m:
        return getters
    body = _balanced_body(text, m.end())
    for fm in re.finditer(r"\bget_(\w+)\s*\(", body):
        getters.add(fm.group(1))
    return getters


def _collect_ffi_getters(ffi_src: pathlib.Path) -> set[str]:
    """FFI `thetadatadx_config_get_<name>` extern C read-back declarations.

    Mirrors `_collect_ffi_setters` on the read side. Returns the bare
    canonical names with the `thetadatadx_config_get_` prefix stripped.
    """
    getters: set[str] = set()
    if not ffi_src.is_dir():
        return getters
    for rs in ffi_src.rglob("*.rs"):
        text = _read_source(rs)
        for m in re.finditer(r"\bfn\s+thetadatadx_config_get_(\w+)\s*\(", text):
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
# Empty today: the `historical_host` / `historical_port` advanced endpoint
# overrides are now bound on every binding (Python / TypeScript / C++ /
# the C ABI), so no carve-out is required. Add an entry here only when a
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
    matrix on the others (the `flush_mode`-missing-on-TS defect
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


# ─── ClientBuilder fluent-setter parity (Rust vs C++) ──────────────


# Fluent `ClientBuilder` setters that intentionally exist on only one of
# the two builder surfaces. Empty today: every public Rust builder setter
# is expected to exist on the C++ builder, and vice versa.
CLIENT_BUILDER_SETTER_PARITY_EXEMPT: dict[str, str] = {}


def _collect_rust_client_builder_setters(client_builder_rs: pathlib.Path) -> set[str]:
    """Public Rust `ClientBuilder` fluent setters returning `Self`.

    Scoped to `impl ClientBuilder { ... }` and to the `-> Self` return
    shape so helper methods (`new`, `set_auth`) and the terminal
    `connect()` are excluded. The collected names are the canonical
    snake_case setter identifiers the Rust docs surface.
    """
    setters: set[str] = set()
    if not client_builder_rs.is_file():
        return setters
    text = _read_source(client_builder_rs)
    for header in re.finditer(r"impl\s+ClientBuilder\s*\{", text):
        body = _balanced_body(text, header.end())
        for m in re.finditer(
            r"\bpub\s+fn\s+([a-z_][a-z0-9_]*)\s*\([^)]*\)\s*->\s*Self\b",
            body,
            re.DOTALL,
        ):
            setters.add(m.group(1))
    return setters


def _collect_cpp_client_builder_setters(cpp_hpp: pathlib.Path) -> set[str]:
    """Public C++ `ClientBuilder` fluent setters returning
    `ClientBuilder&` / `ClientBuilder&&`.

    Scoped to the `class ClientBuilder { ... }` body and to the fluent
    return-type shape, so lifecycle members, helper methods, and the
    terminal `connect()` are excluded. The returned names are the public
    snake_case setter identifiers on the C++ builder surface.
    """
    setters: set[str] = set()
    if not cpp_hpp.is_file():
        return setters
    text = _read_cpp_expanded(cpp_hpp)
    m = re.search(r"^class\s+ClientBuilder\s*(?::[^{]*)?\{", text, re.MULTILINE)
    if not m:
        return setters
    body = _balanced_body(text, m.end())
    for fm in re.finditer(r"\bClientBuilder(?:&&|&)\s+([a-z_][a-z0-9_]*)\s*\(", body):
        setters.add(fm.group(1))
    return setters


def _check_client_builder_setter_parity(
    rust_setters: set[str],
    cpp_setters: set[str],
    exempt: dict[str, str] | None = None,
) -> list[str]:
    """Assert the Rust and C++ `ClientBuilder` fluent-setter NAME-SETS
    are equal.

    This is the inline client-construction parity guard: a setter that
    exists on only one builder cannot hide as an unenrolled
    binding-specific capability. Anything intentionally asymmetric must be
    listed in `CLIENT_BUILDER_SETTER_PARITY_EXEMPT` with a documented
    reason; stale exemptions are themselves flagged so the carve-out list
    does not rot.
    """
    if exempt is None:
        exempt = CLIENT_BUILDER_SETTER_PARITY_EXEMPT
    bindings = {"rust": rust_setters, "cpp": cpp_setters}
    universe: set[str] = set().union(*bindings.values()) if bindings else set()
    errors: list[str] = []
    for setter in sorted(universe - set(exempt)):
        present_on = [lang for lang, names in bindings.items() if setter in names]
        if len(present_on) != len(bindings):
            missing = [lang for lang in bindings if lang not in present_on]
            errors.append(
                f"  ClientBuilder setter `{setter}`: present on {sorted(present_on)}, "
                f"missing on {sorted(missing)}. Bind it on both the Rust and C++ "
                f"builder surfaces, or add it to "
                f"CLIENT_BUILDER_SETTER_PARITY_EXEMPT with a documented reason."
            )
    for setter, reason in exempt.items():
        present_on = [lang for lang, names in bindings.items() if setter in names]
        if present_on and len(present_on) == len(bindings):
            errors.append(
                f"  ClientBuilder setter `{setter}`: listed in "
                f"CLIENT_BUILDER_SETTER_PARITY_EXEMPT ({reason!r}) but is now "
                f"present on both Rust and C++. Drop the stale exemption."
            )
    return errors


# ─── TypeScript connectWith option-field roster ────────────────────


# The canonical JS-visible `Client.connectWith(...)` option-field roster.
# The Rust `ClientConnectOptions` struct in `thetadatadx-ts/src/lib.rs`
# must emit exactly this set after napi camel-casing. A dropped, renamed,
# or newly-added field trips here even if no `[[connect]]` / `[[method]]`
# row changes, so the inline connect surface stays pinned.
TYPESCRIPT_CONNECT_WITH_FIELD_ROSTER: frozenset[str] = frozenset(
    {
        "apiKey",
        "apiKeyFromEnv",
        "apiKeyFromDotenv",
        "email",
        "password",
        "credentialsFile",
        "historicalType",
        "streamingType",
    }
)


def _collect_typescript_connect_with_fields(ts_lib_rs: pathlib.Path) -> set[str]:
    """JS-visible field names on `ClientConnectOptions`.

    Parses the `#[napi(object)] pub struct ClientConnectOptions { ... }`
    body in `thetadatadx-ts/src/lib.rs`, honoring an explicit
    `#[napi(js_name = "...")]` on a field when present and otherwise
    applying napi-rs' snake_case → camelCase object-field mapping.
    """
    out: set[str] = set()
    if not ts_lib_rs.is_file():
        return out
    text = _read_source(ts_lib_rs)
    m = re.search(
        r"#\[napi\(object\)\]\s*pub\s+struct\s+ClientConnectOptions\s*\{",
        text,
    )
    if not m:
        return out
    body = _balanced_body(text, m.end())
    js_name_re = re.compile(r'js_name\s*=\s*"([a-zA-Z_][a-zA-Z0-9_]*)"')
    field_re = re.compile(
        r"((?:\s*#\[[^\]]*\]\s*)*)\s*pub\s+([a-z_][a-z0-9_]*)\s*:",
        re.MULTILINE,
    )
    for fm in field_re.finditer(body):
        attrs = fm.group(1)
        snake = fm.group(2)
        js_name = js_name_re.search(attrs)
        out.add(js_name.group(1) if js_name else _snake_to_camel(snake))
    return out


def _check_typescript_connect_with_field_roster(
    actual_fields: set[str],
    expected_fields: frozenset[str] | None = None,
) -> list[str]:
    """Assert `ClientConnectOptions` emits exactly the canonical
    connectWith option-field roster."""
    if expected_fields is None:
        expected_fields = TYPESCRIPT_CONNECT_WITH_FIELD_ROSTER
    errors: list[str] = []
    for field in sorted(expected_fields - actual_fields):
        errors.append(
            f"  Client.connectWith options: missing TypeScript field `{field}` on "
            f"`ClientConnectOptions`. Expected roster is {sorted(expected_fields)}. "
            f"Add the field back, or update TYPESCRIPT_CONNECT_WITH_FIELD_ROSTER "
            f"intentionally."
        )
    for field in sorted(actual_fields - expected_fields):
        errors.append(
            f"  Client.connectWith options: unexpected TypeScript field `{field}` on "
            f"`ClientConnectOptions`. Add it to TYPESCRIPT_CONNECT_WITH_FIELD_ROSTER "
            f"if intentional, or remove / rename it."
        )
    return errors


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
    "HistoricalConfig",
    "StreamingConfig",
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

    Parses `thetadatadx-rs/src/config/*.rs`. Skips fields on
    structs not listed in `SCOPED_STRUCTS` — `DirectConfig`'s pub
    fields are nested-struct accessors that the class-level gate
    already covers.
    """
    out: dict[str, set[str]] = {}
    if not config_dir.is_dir():
        return out
    for rs in config_dir.rglob("*.rs"):
        text = _read_source(rs)
        # Find every `pub struct X {` block and walk forward until the
        # closing brace. The structs in `thetadatadx-rs/src/config/`
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
    # StreamingConfig scalar knobs carry a `streaming_` prefix at the
    # binding surface so the generic field names (`timeout_ms`, `ring_size`)
    # stay unambiguous against sibling sub-configs.
    ("StreamingConfig", "timeout_ms"): "streaming_timeout_ms",
    ("StreamingConfig", "ring_size"): "streaming_ring_size",
    ("StreamingConfig", "ping_interval_ms"): "streaming_ping_interval_ms",
    ("StreamingConfig", "connect_timeout_ms"): "streaming_connect_timeout_ms",
    ("StreamingConfig", "io_read_slice_ms"): "streaming_io_read_slice_ms",
    ("StreamingConfig", "keepalive_idle_secs"): "streaming_keepalive_idle_secs",
    ("StreamingConfig", "keepalive_interval_secs"): "streaming_keepalive_interval_secs",
    ("StreamingConfig", "keepalive_retries"): "streaming_keepalive_retries",
    ("StreamingConfig", "host_selection"): "streaming_host_selection",
    ("StreamingConfig", "host_shuffle_seed"): "streaming_host_shuffle_seed",
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
    name (`impl Client`) or a fully-qualified Rust path
    (`impl crate::Client`); the collector normalises both
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
        "__aenter__",
        "__aexit__",
    }
    # `impl <Path> {` — `<Path>` may be `Name` or `crate::...::Name`.
    # Capture the last identifier segment before the opening brace.
    impl_re = re.compile(
        r"impl\s+(?:[A-Za-z_][A-Za-z0-9_]*::)*([A-Za-z_][A-Za-z0-9_]*)\s*\{"
    )
    fn_re = re.compile(r"fn\s+([a-zA-Z_][a-zA-Z0-9_]*)\s*[(<]")
    for rs in py_src.rglob("*.rs"):
        text = _read_source(rs)
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


def _collect_typescript_class_methods(
    ts_src: pathlib.Path, ts_pkg_dir: pathlib.Path | None = None
) -> dict[str, set[str]]:
    """Return `{ts_class_name: {method, ...}}` for every TypeScript
    napi class.

    Parses every `#[napi]` / `#[napi(js_name = "...")] impl <Name>` block
    and harvests the JS-visible method names inside. The TS impl blocks
    live across multiple files (`lib.rs`, `_generated/*.rs`,
    `config_class.rs`, ...); the collector walks each one and bounds
    the method scan to the impl body with a brace counter.

    A method may also be added on the WRAPPER side rather than the napi
    `impl`. The package entry augments a napi class through a
    `declare module './index' { interface Client { ... } }` block and a
    matching `Client.prototype.<name> = ...` runtime assignment (the
    context-managed `Client.streaming(...)` helper is the load-bearing
    case). When `ts_pkg_dir` is supplied, those augmentations are merged
    in so a wrapper-only method is seen exactly like a napi one.

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
    # (`impl crate::Client`) symmetrically with the Python
    # collector. The captured class name is always the last path segment.
    impl_re = re.compile(
        r"impl\s+(?:[A-Za-z_][A-Za-z0-9_]*::)*([A-Za-z_][A-Za-z0-9_]*)\s*\{"
    )
    # The `js_name` literal inside a napi attribute arg list, and the
    # `fn <snake>` that the attribute decorates (allowing intervening outer
    # attributes / doc comments between the `#[napi(...)]` and the `fn`).
    js_name_in_attr_re = re.compile(r'\bjs_name\s*=\s*"([a-zA-Z_][a-zA-Z0-9_]*)"')
    # `.match(body, after)` anchors at `after`, so no `\A`/`^` is needed
    # (and `\A` would WRONGLY anchor at string start, never matching when
    # `after > 0`). Tolerates intervening outer attributes / doc comments
    # between the `#[napi(...)]` and the decorated `fn`.
    following_fn_re = re.compile(
        r"(?:\s*(?:#\[[^\]]*\]|///[^\n]*|//[^\n]*))*\s*"
        r"(?:pub\s+)?(?:async\s+)?fn\s+([a-z_][a-z0-9_]*)\s*[(<]"
    )
    for rs in ts_src.rglob("*.rs"):
        text = _read_source(rs)
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
            # Balanced-paren attribute scan: a callback-typed arg list
            # (`ts_args_type = "cb: (e) => void", js_name = "setX"`) no
            # longer truncates the `js_name` read at the inner `)`.
            for inner, after in _iter_napi_attrs(body):
                fn_m = following_fn_re.match(body, after)
                if not fn_m:
                    # The `#[napi(...)]` does not decorate a fn (e.g. a
                    # struct attribute) — nothing to record from this site.
                    continue
                js_m = js_name_in_attr_re.search(inner)
                if js_m:
                    out.setdefault(class_name, set()).add(js_m.group(1))
                else:
                    snake = fn_m.group(1)
                    head, *rest = snake.split("_")
                    camel = head + "".join(p.capitalize() for p in rest)
                    out.setdefault(class_name, set()).add(camel)
                    out.setdefault(class_name, set()).add(snake)
    if ts_pkg_dir is not None:
        for cls, methods in _collect_ts_wrapper_class_methods(ts_pkg_dir).items():
            out.setdefault(cls, set()).update(methods)
    return out


# A wrapper-side method augmentation on the `.d.ts` entry:
# `declare module './index' { interface <Class> { <name>(...): ...; } }`.
# Captures each augmented interface name and its method declarations so a
# method the wrapper layers onto a napi class (e.g. `Client.streaming`) is
# harvested under that class.
TS_AUGMENT_MODULE_RE = re.compile(
    r"declare\s+module\s+['\"][^'\"]+['\"]\s*\{(.*?)\n\}", re.DOTALL
)
TS_AUGMENT_IFACE_RE = re.compile(r"interface\s+(\w+)\s*\{(.*?)\n\s*\}", re.DOTALL)
TS_AUGMENT_METHOD_RE = re.compile(r"^\s*(\w+)\s*[(<]", re.MULTILINE)
# A wrapper-side runtime method on the `.js` entry:
# `<Native>.Client.prototype.<name> = ...` or `Client.prototype.<name> = ...`.
JS_PROTOTYPE_METHOD_RE = re.compile(
    r"(?:\b\w+\.)?(\w+)\.prototype\.(\w+)\s*="
)

# Wrapper classes whose public surface lives in a STANDALONE `.d.ts`
# `interface`/`class` declaration (not a napi `impl` and not a `declare
# module` augmentation), keyed by the canonical parity-row class name. The
# pull-based `RecordBatchStream` reader is the case: the JS wrapper class in
# `streaming-session.{js,d.ts}` over the napi `RecordBatchStreamHandle`
# transport is the public TypeScript surface, so its declared members must be
# visible to the method matrix. Scoped to this allow-set so unrelated `.d.ts`
# interfaces never pollute another class's harvested method set.
TS_WRAPPER_STANDALONE_CLASSES: frozenset[str] = frozenset({"RecordBatchStream"})
# A top-level `(export )(declare )(class|interface) <Name> ... { <body> }`
# declaration. The body capture is balanced by the caller's brace walk.
TS_STANDALONE_DECL_RE = re.compile(
    r"(?:export\s+)?(?:declare\s+)?(?:class|interface)\s+(\w+)\b[^\{]*\{"
)
# A property or method member inside such a body: an optional `readonly`,
# then the member name, then `(` (method) or `:` (typed property). The
# `[Symbol.x]` computed members are bracketed and never match `\w+`.
TS_STANDALONE_MEMBER_RE = re.compile(
    r"^\s*(?:readonly\s+)?(\w+)\s*[(:?<]", re.MULTILINE
)


def _collect_ts_wrapper_class_methods(ts_pkg_dir: pathlib.Path) -> dict[str, set[str]]:
    """Harvest wrapper-side method augmentations from the package entry.

    Reads the declared `.d.ts` entry (`package.json` `types`) for
    `declare module './index' { interface <Class> { ... } }` augmentations,
    the declared `.js` entry (`main`) for `<Class>.prototype.<name> = ...`
    runtime additions, and the STANDALONE `.d.ts` `interface`/`class`
    declarations of the wrapper classes in `TS_WRAPPER_STANDALONE_CLASSES`,
    returning `{class: {method, ...}}`. This is how a method present on the
    public TS surface only because the wrapper adds it (not the napi `impl`)
    becomes visible to the parity gate.
    """
    out: dict[str, set[str]] = {}
    dts = _resolve_ts_entry(ts_pkg_dir, "types", "index.d.ts")
    if dts.is_file():
        text = dts.read_text(encoding="utf-8")
        for mod in TS_AUGMENT_MODULE_RE.finditer(text):
            for iface in TS_AUGMENT_IFACE_RE.finditer(mod.group(1)):
                cls = iface.group(1)
                for meth in TS_AUGMENT_METHOD_RE.finditer(iface.group(2)):
                    out.setdefault(cls, set()).add(meth.group(1))
        # Standalone wrapper-class declarations: walk every top-level
        # class/interface, bound its body with a brace counter, and harvest
        # the member names when the class is one of the allow-listed wrappers.
        for header in TS_STANDALONE_DECL_RE.finditer(text):
            cls = header.group(1)
            if cls not in TS_WRAPPER_STANDALONE_CLASSES:
                continue
            body_start = header.end()
            depth = 1
            i = body_start
            while i < len(text) and depth > 0:
                ch = text[i]
                if ch == "{":
                    depth += 1
                elif ch == "}":
                    depth -= 1
                i += 1
            body = text[body_start : i - 1]
            for meth in TS_STANDALONE_MEMBER_RE.finditer(body):
                out.setdefault(cls, set()).add(meth.group(1))
    js = _resolve_ts_entry(ts_pkg_dir, "main", "index.js")
    if js.is_file():
        for m in JS_PROTOTYPE_METHOD_RE.finditer(js.read_text(encoding="utf-8")):
            out.setdefault(m.group(1), set()).add(m.group(2))
    return out


def _expand_cpp_includes(hpp_text: str, include_dir: pathlib.Path) -> str:
    """Inline every `#include "<name>.inc"` directive against the
    matching file under `include_dir`. The `*.inc` files extend a
    class body with generator-emitted member declarations
    (`thetadatadx-cpp/include/fpss.hpp.inc` adds `StreamingClient` methods that
    live in `thetadatadx-rs/sdk_surface.toml`), and the parity
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


# Keywords that can front a `name(` as a CALL or statement, never as a C++
# return type. Shared by the method-presence collector and the signature
# extractor so both reject a `return foo(` / `.foo(` in-body call site rather
# than read it as a declaration (the G11 in-body-shadow defect class).
_SIG_CPP_CALL_PREV_KEYWORDS: frozenset[str] = frozenset(
    {
        "return",
        "co_return",
        "co_await",
        "if",
        "while",
        "for",
        "switch",
        "throw",
        "catch",
        "sizeof",
        "new",
        "delete",
        "and",
        "or",
        "not",
    }
)


def _collect_cpp_class_methods(cpp_hpp: pathlib.Path) -> dict[str, set[str]]:
    """Return `{class_name: {method, ...}}` for every C++ class.

    Parses each `class X { ... };` body in `thetadatadx.hpp` and collects
    every member *declaration* with a `<return-type> name(` shape. Bounded
    brace-counting keeps nested types (e.g. lambdas inside default-arg
    initializers) from leaking into the outer class's method set.

    A member declaration is in DECLARATION position: the method name is
    immediately preceded by a return-type token (an identifier ending the
    return type, or a `>` / `*` / `&` / `]` from a templated, pointer,
    reference, or array return). A bare `name(` CALL inside another inline
    method body — e.g. `return request(...)` in a convenience accessor, or
    a statement-leading `helper();` — is NOT in declaration position and is
    rejected. Counting such call sites let a method *declaration* be
    deleted while its in-class call sites kept the name alive, so the
    parity gate read the binding as still exposing a method it no longer
    declared (the G11 bypass). This mirrors `_collect_cpp_setters`, which
    already keys on the `void` / `int32_t` return type so a bare call
    cannot satisfy it.

    Honors `#include "<file>.inc"` inside a class body by inlining the
    included file's contents before parsing — generator-emitted method
    declarations (`fpss.hpp.inc`) extend the surrounding class body
    and must count toward parity.
    """
    out: dict[str, set[str]] = {}
    if not cpp_hpp.is_file():
        return out
    text = _read_cpp_expanded(cpp_hpp)
    # Limit to class bodies — struct bodies are POD-shaped value types
    # and irrelevant to the cross-binding method contract.
    class_header_re = re.compile(r"^class\s+(\w+)\s*(?::[^{]*)?\{", re.MULTILINE)
    # A member declaration: a return-type token (`prev`), then whitespace,
    # then the method `name(`. `prev` is captured so a control keyword that
    # can front a CALL (`return foo(`) is rejected — a real return type is
    # never one of those keywords. The leading return-type token is what
    # separates `FlatFileRowList request(` (declaration) from
    # `return request(` and the statement-leading `request(` (calls).
    decl_re = re.compile(
        r"(?P<prev>[A-Za-z_]\w*|[>*&\]])\s+(?P<name>[a-z_][a-z0-9_]*)\s*\(",
    )
    # Keywords that can precede a `name(` as a CALL or statement, never as
    # a return type. `return foo(` is the live G11 mask; the rest guard the
    # general class (`else if (`, `co_return bar(`, etc.).
    _CALL_PREV_KEYWORDS = _SIG_CPP_CALL_PREV_KEYWORDS
    # Method names that are themselves keywords (defensive; a declaration
    # never names a method these).
    _NAME_KEYWORDS = {
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
    }
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
        for fm in decl_re.finditer(body):
            prev = fm.group("prev")
            name = fm.group("name")
            # The preceding token must be a return type, not a call-context
            # keyword. `request` declared as `FlatFileRowList request(`
            # passes (`prev == "FlatFileRowList"`); the `return request(`
            # call is rejected (`prev == "return"`).
            if prev in _CALL_PREV_KEYWORDS:
                continue
            if name in _NAME_KEYWORDS:
                continue
            out.setdefault(class_name, set()).add(name)
    return out


# The unified `Client`'s data surfaces are reached through view accessors
# that return a cheap handle clone (`historical`, `stream`, `flatFiles`).
# They are hand-written per binding rather than generated from
# `endpoint_surface.toml`. Kept as the canonical-name reference for the view
# accessors; the reverse-orphan scan below is no longer limited to this set.
# It discovers every Client-level helper from the bindings so a NEW helper
# (any shape, not just a view accessor) cannot ship on a binding untracked.
CLIENT_VIEW_ACCESSORS: frozenset[str] = frozenset(
    {"historical", "stream", "flatFiles"}
)


# Client-level members the reverse-orphan scan must NOT treat as
# cross-binding helper methods needing a `[[method]]` row. The scan
# discovers candidate helpers from the precise Python (`#[pymethods]`) and
# TypeScript (napi `#[napi]` + wrapper augmentation) collectors; this roster
# names the members those collectors harvest that carry no Client
# `[[method]]` contract: the constructors / connection factories, gated by
# the `[[connect]]` and `[[from_file]]` families rather than Client
# `[[method]]` rows.
#
# Names are the canonical camelCase spelling the scan derives. The blob-to-
# disk helper (`flatFileToPath` / `flatfile_to_path`) is enrolled through
# `METHOD_BINDING_OVERRIDES` against a `FlatFilesNamespace` row, so it is
# resolved as enrolled by the scan (not listed here) and never double-counts.
# The Python-only auth-time readbacks (`sessionUuid` / `subscriptionInfo`)
# are now enrolled as explicit Python-only Client `[[method]]` rows, so they
# resolve as enrolled too and are no longer exempted here.
CLIENT_REVERSE_ORPHAN_EXEMPT_MEMBERS: frozenset[str] = frozenset(
    {
        "connect",
        "connectBlocking",
        "connectFromFile",
        "fromFile",
        "fromEnv",
        "fromDotenv",
        # Private shared teardown behind `close` / `__exit__` / `__aexit__`;
        # a plain (non-`#[pymethods]`) `impl Client` helper the collector still
        # harvests. Not exposed to Python, so it carries no cross-binding
        # contract and is enrolled by the public `close` row instead.
        "closeImpl",
        # Private closed-guard accessor resolving the live core handle (or a
        # "client is closed" error) behind every vended surface; a plain
        # (non-`#[pymethods]`) `impl Client` helper the collector harvests. Not
        # exposed to Python — the TypeScript twin (`clientHandle`) is likewise a
        # plain `impl` helper the napi collector never sees — so it carries no
        # cross-binding contract.
        "clientArc",
    }
)


# The flat-file fetch surface lives on the namespace returned by
# `Client.flatFiles`. The forward `[[method]]` rows gate each declared fetch
# method against the bindings; this roster anchors the reverse-direction scan
# that catches a NEW fetch method added on a binding's namespace class without
# an enrolling row. Names are the canonical camelCase row spelling (the scan
# derives the per-binding form exactly as the forward check does). `request` is
# the generic dispatcher; the five named methods are the served-dataset
# conveniences. The blob-to-disk helper (`flatFileToPath`) is NOT a namespace
# method — it lives on the unified client (or, on C++, on the namespace under a
# different name) and is enrolled by its own row routed through
# `METHOD_BINDING_OVERRIDES`, so it is intentionally absent from this roster.
FLATFILES_NAMESPACE_METHODS: frozenset[str] = frozenset(
    {
        "optionTradeQuote",
        "optionOpenInterest",
        "optionEod",
        "stockTradeQuote",
        "stockEod",
        "request",
    }
)


# Names the per-binding method collectors harvest from the flat-file namespace
# class body that are NOT cross-binding fetch methods and so carry no
# `[[method]]` row of their own: the C++ collector picks up the `handle_` data
# member and the FFI extern declarations called inside the inline method bodies,
# and the Python collector picks up the module-private `pull_decoded` free fn.
# `to_path` is the C++ spelling of the blob-to-disk helper — a real public
# method, but enrolled by the `flatFileToPath` row routed through
# `METHOD_BINDING_OVERRIDES`, not by a namespace row — so it is exempt from the
# namespace reverse-orphan scan to avoid a double-count.
FLATFILES_NAMESPACE_EXEMPT_MEMBERS: frozenset[str] = frozenset(
    {
        "handle_",
        "pull_decoded",
        "pull_decoded_async",
        "thetadatadx_flatfile_request_decoded",
        "thetadatadx_flatfile_request_to_path",
        "to_path",
    }
)


# A few cross-binding methods do not share a single home class or a single
# name-derivation rule across bindings; the forward `[[method]]` check resolves
# one `(class, name)` per binding from the row, so those methods need an explicit
# per-binding target. The blob-to-disk flat-file helper is the load-bearing case:
# it is `Client.flatfile_to_path` (one word) on the Python pyclass,
# `Client.flatFileToPath` on the TypeScript napi class, and `FlatFiles::to_path`
# on the C++ class. Keyed by the canonical `(row_class, row_name)`, each entry
# gives the binding-specific `(collector_class, member_name)` the forward check
# must look up instead of deriving from the row. `collector_class` is the key the
# per-binding collector uses (the C++ entry already names the resolved C++ class,
# so it bypasses the alias table); `member_name` is the exact harvested spelling.
METHOD_BINDING_OVERRIDES: dict[tuple[str, str], dict[str, tuple[str, str]]] = {
    ("FlatFilesNamespace", "flatFileToPath"): {
        "python": ("Client", "flatfile_to_path"),
        "typescript": ("Client", "flatFileToPath"),
        "cpp": ("FlatFiles", "to_path"),
        # Rust exposes the blob-to-disk helper as `FlatFiles::to_path` on
        # the view returned by `Client::flat_files()` — the same `to_path`
        # spelling and home class as C++.
        "rust": ("FlatFiles", "to_path"),
    },
    # The Subscription wire-string accessor is `kind` on Python / TypeScript
    # but `kind_string` on C++ (where the bare `kind()` returns the typed
    # enum); only the C++ member name diverges, so just that binding is
    # overridden — Python / TypeScript derive `kind` from the row as usual.
    ("Subscription", "kind"): {
        "cpp": ("FluentSubscription", "kind_string"),
    },
    # FlatFileRowList's row-count and Arrow-IPC methods are spelled per the
    # idiom of each binding: row count is Python `__len__` / TypeScript `len`
    # / C++ `size`; the Arrow-IPC serializer is Python `to_arrow` /
    # TypeScript `toArrowIpc` / C++ `to_arrow_ipc`.
    ("FlatFileRowList", "count"): {
        "python": ("FlatFileRowList", "__len__"),
        "typescript": ("FlatFileRowList", "len"),
        "cpp": ("FlatFileRowList", "size"),
    },
    ("FlatFileRowList", "toArrowIpc"): {
        "python": ("FlatFileRowList", "to_arrow"),
        "typescript": ("FlatFileRowList", "toArrowIpc"),
        "cpp": ("FlatFileRowList", "to_arrow_ipc"),
    },
}


# `[[method]]` rows that carry NO `[method.signature]` by design: their
# per-binding shapes are structurally divergent binding machinery with no
# comparable cross-binding signature to pin. Every other `[[method]]` row MUST
# carry a `[method.signature]` (the gate fails closed for any name-only row
# absent from this set), so a NEW row can never be silently name-only — it is
# either pinned or it is enrolled here with a stated reason.
NAME_ONLY_METHOD_ALLOWLIST: dict[tuple[str, str], str] = {
    ("Config", "setReconnectCallback"): (
        "Reconnect-callback registration is structurally divergent per binding "
        "with no comparable signature: Python takes an optional callable, the "
        "napi side a five-type-argument ThreadsafeFunction, C++ a C function "
        "pointer plus a void* userdata (two params, not one). There is no "
        "cross-binding param/return contract to pin."
    ),
}


# `[[method]]` setter rows whose Python `.pyi` surface is the assignable
# read-write PROPERTY, not a `def set_<name>(...)` method. pyo3 exposes a
# `#[setter] fn set_x(value)` as the attribute `config.x = value`, and the
# hand-written stub models it as the bare read-write annotation `x: T` — there
# is no `set_x` declaration to extract. The MATCHING getter row (`x`) pins that
# property's type through the `.pyi` lane (return = T), which is the same `T`
# this setter's param would check, so re-checking it off a synthetic `set_x` is
# redundant and would false-fail on the (correctly) absent `set_x`. So these
# rows DEGRADE in the `.pyi` lane to the pyo3-source `python` lane + stubtest
# (which DO see the runtime `set_x` setter and its parameter).
#
# The exemption is sound ONLY because every entry has a getter twin whose `.pyi`
# property TYPE the `python_pyi` lane checks directly — verified per setter:
#   setFlushMode        → `flushMode`           (Literal["batched", "immediate"])
#   setConsumerCpu      → `consumerCpu`         (Optional[int])
#   setReconnectPolicy  → `reconnectPolicy`     (str)
#   setStreamingRingSize→ `streamingRingSize`   (int)  ← getter row added so the
#       property type is checked; before it, `streaming_ring_size` had no pinned
#       getter twin and a stub drift to a non-integer would have passed.
#   setWorkerThreads    → `workerThreads`       (Optional[int])
# `_case_sig_python_pyi_setter_property_degrade` asserts each twin is extracted.
# A setter NOT listed here whose `set_<name>` is dropped from a fully-enumerated
# stub class still fails (the absence-promotion rule), so this is an explicit,
# bounded exemption — not a blanket "missing setter is fine".
PYI_SETTER_PROPERTY_ROWS: frozenset[tuple[str, str]] = frozenset(
    ("Config", name)
    for name in (
        "setFlushMode",
        "setConsumerCpu",
        "setReconnectPolicy",
        "setStreamingRingSize",
        "setWorkerThreads",
    )
)


# Parity-toml `class` field → the Rust struct the Rust method collector
# keys on. The flat-file namespace is the borrowed `FlatFiles` view
# returned by `Client::flat_files()`; the unified entry-point client is
# `Client`. Both live in `thetadatadx-rs/src/client.rs`. A row class
# absent from this table is not gated on the Rust column (Python /
# TypeScript / C++ only), so the Rust column is opt-in per row — exactly
# the surfaces where Rust must mirror the other bindings.
RUST_METHOD_CLASS: dict[str, str] = {
    "FlatFilesNamespace": "FlatFiles",
    "Client": "Client",
}


def _collect_rust_view_methods(client_rs: pathlib.Path) -> dict[str, set[str]]:
    """Return `{rust_class: {method, ...}}` for the public methods on the
    Rust view structs that mirror the cross-binding surface.

    Parses every `impl FlatFiles<...> { ... }` and `impl Client { ... }`
    block in `client.rs` and collects each `pub fn <name>` /
    `pub async fn <name>` (snake_case). `#[cfg(...)]` / `#[doc(hidden)]`
    gated methods are skipped — they are not part of the published surface
    a binding must mirror. The harvested set is keyed by the bare Rust
    struct name (`FlatFiles`, `Client`), which the forward check resolves
    a parity row's class to via `RUST_METHOD_CLASS`.
    """
    out: dict[str, set[str]] = {}
    if not client_rs.is_file():
        return out
    text = _read_source(client_rs)
    # `pub fn <name>(` or `pub async fn <name>(`, capturing any leading
    # attribute lines so cfg/doc-hidden gating can be honoured.
    pub_fn_re = re.compile(
        r"((?:#\[[^\]]*\]\s*)*)\bpub\s+(?:async\s+)?fn\s+([a-z_][a-z0-9_]*)\s*[(<]"
    )
    for struct in ("FlatFiles", "Client"):
        # `impl FlatFiles<'_> {` / `impl Client {` — tolerate an optional
        # generic / lifetime parameter list before the brace. Walk every
        # matching impl block (there are several `impl Client` blocks).
        impl_re = re.compile(rf"impl\s+{struct}\b[^{{]*\{{")
        for header in impl_re.finditer(text):
            body = _balanced_body(text, header.end())
            for m in pub_fn_re.finditer(body):
                attrs = m.group(1)
                name = m.group(2)
                if "cfg(" in attrs or "doc(hidden)" in attrs:
                    continue
                out.setdefault(struct, set()).add(name)
    return out


def _check_class_reverse_orphans(
    class_name: str,
    members_by_lang: dict[str, set[str]],
    enrolled: set[str],
    exempt: frozenset[str],
    *,
    discover_langs: frozenset[str],
    strip_async: bool,
    override_members: frozenset[str] = frozenset(),
    report_camel: bool,
) -> list[str]:
    """Reverse-direction orphan scan for one user-facing class.

    A method present on `class_name` in some binding but carrying no
    enrolling `[[method]]` row is undocumented drift: the row simply does
    not exist, so the forward check never fires. This flags any harvested
    member that is neither enrolled, enrolled-elsewhere through
    `METHOD_BINDING_OVERRIDES` (`override_members`, raw spellings), nor a
    documented non-contract idiom in `exempt`.

    `members_by_lang` maps each binding to the set its per-binding collector
    returned for the class (used to report which bindings a flagged member
    appears on). `discover_langs` names the subset of bindings whose members
    SEED candidates: the C++ heuristic scan is usually excluded from
    discovery because it picks up FFI-extern calls and data members that are
    not public methods (the C++ column is validated by the forward row
    check), while still contributing to the presence report.

    `strip_async` matches a Python `<base>_async` member against its sync
    base (the awaitable twin rides the same enrolled row). `report_camel`
    selects the spelling used in the error (camelCase candidate for the
    unified `Client`, the raw harvested spelling for the namespace / session
    surfaces). Returns human-readable error strings (empty when every
    harvested member is accounted for).
    """
    errors: list[str] = []
    # Per-binding harvested members the error can report presence on.
    py_members = members_by_lang.get("python", set())
    ts_members = members_by_lang.get("typescript", set())
    cpp_members = members_by_lang.get("cpp", set())

    # The candidate set: every harvested member across the DISCOVERY
    # bindings, with a Python `_async` twin folded onto its base when
    # requested, minus the override-home members enrolled elsewhere. Each
    # candidate also remembers the discovery binding(s) it was harvested
    # from, so an unenrolled member always reports presence on its source
    # binding even when the snake/camel re-derivation below does not round
    # back to the raw harvested spelling (a camelCase py/cpp member).
    candidates: dict[str, set[str]] = {}
    for lang, members in (
        ("python", py_members),
        ("typescript", ts_members),
        ("cpp", cpp_members),
    ):
        if lang not in discover_langs:
            continue
        for member in members:
            base = (
                member[: -len("_async")]
                if strip_async and member.endswith("_async")
                else member
            )
            if base in override_members or member in override_members:
                continue
            candidates.setdefault(base, set()).add(lang)

    for member in sorted(candidates):
        camel = _snake_to_camel(member) if "_" in member else member
        snake = _camel_to_snake(camel)
        if camel in enrolled or snake in enrolled or member in enrolled:
            continue
        if member in exempt or camel in exempt:
            continue
        present_on = sorted(
            set(
                lang
                for lang, seen in (
                    (
                        "python",
                        snake in py_members
                        or (strip_async and f"{snake}_async" in py_members),
                    ),
                    ("typescript", camel in ts_members or snake in ts_members),
                    ("cpp", snake in cpp_members or f"get_{snake}" in cpp_members),
                )
                if seen
            )
            # The discovery binding the member actually came from always
            # counts as present — a camelCase harvest whose snake re-derivation
            # misses the raw set must still trip, never silently drop.
            | candidates[member]
        )
        if not present_on:
            continue
        reported = camel if report_camel else member
        errors.append(
            f"  {class_name}.{reported}: method present on {present_on} but "
            f"has no `[[method]]` row (class {class_name}). Either enroll it "
            f"(with its per-binding presence) so the surface stays tracked, or "
            f"add it to the documented reverse-scan exempt roster with the "
            f"reason it carries no cross-binding contract."
        )
    return errors


def _check_method_rows(
    method_rows: list[dict[str, Any]],
    py_methods: dict[str, set[str]],
    ts_methods: dict[str, set[str]],
    cpp_methods: dict[str, set[str]],
    rust_methods: dict[str, set[str]] | None = None,
) -> list[str]:
    """Per-method cross-binding gate.

    Each `[[method]]` row in `parity.toml` declares a `(class, name)`
    pair plus the expected presence in each binding. The checker
    verifies the actual binding state against the declared state and
    returns a list of human-readable mismatch strings (empty when
    every row matches).

    Beyond the per-row forward check, Client-level helper methods get a
    reverse-direction orphan scan. It is NOT limited to the view accessors:
    candidate helpers are discovered from the precise Python and TypeScript
    collectors (the latter including the wrapper-side `Client` augmentation),
    so any helper a user calls as `client.<helper>(...)` that exists on a
    binding but carries no enrolling `Client` `[[method]]` row trips the gate
    unless it is named in `CLIENT_REVERSE_ORPHAN_EXEMPT_MEMBERS`. A future
    helper cannot ship on one binding untracked.
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

        # A handful of methods do not share a home class or a single
        # name-derivation rule across bindings (the flat-file blob-to-disk
        # helper is the load-bearing case). `METHOD_BINDING_OVERRIDES` supplies
        # the binding-specific `(collector_class, member_name)` for those rows;
        # absent an entry, every binding resolves from the row's `class` / `name`
        # exactly as before.
        override = METHOD_BINDING_OVERRIDES.get((class_name, camel))

        # Python: snake_case method declared on the pyclass. A `#[getter]`
        # readback accessor carries a `get_` prefix on its Rust fn name
        # (`fn get_flush_mode`) while pyo3 strips the prefix so the Python
        # property name stays bare (`config.flush_mode`); accept the
        # `get_`-prefixed fn name against the bare row, exactly as the C++
        # branch below accepts `get_<snake>`.
        if override and "python" in override:
            py_lookup_class, py_member = override["python"]
        else:
            py_lookup_class, py_member = class_name, snake
        py_class_methods = _py_methods_for(py_lookup_class, py_methods)
        declared_py = row.get("python", False)
        actual_py = py_member in py_class_methods or f"get_{py_member}" in py_class_methods
        if declared_py != actual_py:
            verb = "missing" if declared_py and not actual_py else "unexpected"
            errors.append(
                f"  {class_name}.{camel}.python: declared={declared_py}, "
                f"actual={actual_py} ({verb} -- expected `fn {py_member}` "
                f"or `fn get_{py_member}` inside `impl {py_lookup_class}` on the "
                f"Python pyclass)"
            )

        # TypeScript: napi-attributed method declared inside the
        # matching `impl <ClassName>` block under `thetadatadx-ts/src/`.
        # The collector records both the `js_name` and the auto-
        # camelCased fn-name spelling so a row's `name` can match
        # against either.
        if override and "typescript" in override:
            ts_lookup_class, ts_member = override["typescript"]
        else:
            ts_lookup_class, ts_member = class_name, camel
        declared_ts = row.get("typescript", False)
        actual_ts = ts_member in _ts_methods_for(ts_lookup_class, ts_methods)
        if declared_ts != actual_ts:
            verb = "missing" if declared_ts and not actual_ts else "unexpected"
            errors.append(
                f"  {class_name}.{camel}.typescript: declared={declared_ts}, "
                f"actual={actual_ts} ({verb} -- expected "
                f'`#[napi(js_name = "{ts_member}")]` (or bare `#[napi]`) '
                f"inside `impl {ts_lookup_class}` under "
                f"thetadatadx-ts/src/)"
            )

        # C++: `<snake>(` member declaration inside the matching
        # class body in `thetadatadx.hpp`. C++ alias names route through
        # `CPP_ALIASES` (`Contract` -> `FluentContract`). Readback
        # getters on the C++ `Config` carry a uniform `get_` prefix
        # (`get_flush_mode`), where Python exposes the field-shaped
        # property (`flush_mode`) and TypeScript the camelCase getter
        # (`flushMode`); accept the `get_`-prefixed C++ form so the
        # per-language naming convention does not read as drift. An override
        # entry already names the resolved C++ class, so it bypasses the alias
        # table.
        if override and "cpp" in override:
            cpp_class, cpp_member = override["cpp"]
        else:
            cpp_class, cpp_member = _cpp_class_for(class_name), snake
        declared_cpp = row.get("cpp", False)
        cpp_class_methods = cpp_methods.get(cpp_class, set())
        actual_cpp = (
            cpp_member in cpp_class_methods or f"get_{cpp_member}" in cpp_class_methods
        )
        if declared_cpp != actual_cpp:
            verb = "missing" if declared_cpp and not actual_cpp else "unexpected"
            errors.append(
                f"  {class_name}.{camel}.cpp: declared={declared_cpp}, "
                f"actual={actual_cpp} ({verb} -- expected `{cpp_member}(` "
                f"or `get_{cpp_member}(` inside `class {cpp_class}` body in "
                f"thetadatadx-cpp/include/thetadatadx.hpp)"
            )

        # Rust: `pub fn <snake>` / `pub async fn <snake>` inside the
        # matching `impl <Struct>` block in `client.rs`. Only rows whose
        # class maps through `RUST_METHOD_CLASS` are gated on the Rust
        # column — the surfaces where the Rust core must mirror the other
        # bindings (the unified `Client` view accessors and the `FlatFiles`
        # view fetch + path methods). An override entry already names the
        # resolved Rust `(struct, member)`; otherwise the row's class is
        # mapped to its Rust struct and the member is the snake_case name.
        # The column is opt-in: a row that omits `rust` and whose class is
        # not mapped is simply not Rust-gated, so this never weakens the
        # existing Python / TypeScript / C++ checks.
        rust_lookup_class: str | None
        if override and "rust" in override:
            rust_lookup_class, rust_member = override["rust"]
        elif class_name in RUST_METHOD_CLASS:
            rust_lookup_class, rust_member = RUST_METHOD_CLASS[class_name], snake
        else:
            rust_lookup_class, rust_member = None, snake
        if rust_lookup_class is not None:
            declared_rust = row.get("rust", False)
            rust_class_methods = (rust_methods or {}).get(rust_lookup_class, set())
            actual_rust = rust_member in rust_class_methods
            if declared_rust != actual_rust:
                verb = "missing" if declared_rust and not actual_rust else "unexpected"
                errors.append(
                    f"  {class_name}.{camel}.rust: declared={declared_rust}, "
                    f"actual={actual_rust} ({verb} -- expected "
                    f"`pub fn {rust_member}` or `pub async fn {rust_member}` "
                    f"inside `impl {rust_lookup_class}` in "
                    f"thetadatadx-rs/src/client.rs)"
                )

    # Reverse-direction orphan scan for Client-level helper methods. A helper
    # present on the `Client` class in a binding but carrying no enrolling
    # `Client` `[[method]]` row is undocumented drift. This is NOT limited to
    # the view accessors: candidate helpers are discovered from the PRECISE
    # Python (`#[pymethods]`) and TypeScript (napi `#[napi]` impls + the
    # wrapper-side `Client` augmentation) collectors, so any new Client helper
    # (a view accessor, a context-managed session opener, anything a user
    # calls as `client.<helper>(...)`) trips unless it is enrolled or named
    # in the exempt roster. The C++ collector is intentionally not a discovery
    # source here: its heuristic class-body scan harvests FFI-extern calls,
    # data members, and tokens inside method bodies that are not public
    # methods; the C++ column of each helper is validated by the FORWARD row
    # check instead. The error still reports every binding the flagged helper
    # appears on (Python / TypeScript / C++) for context.
    # Enrolled `[[method]]` row names per class the reverse scans gate.
    def _enrolled_for(cls: str) -> set[str]:
        return {
            row["name"]
            for row in method_rows
            if row.get("class") == cls and row.get("name")
        }

    # Binding-specific member spellings a row resolves to through
    # `METHOD_BINDING_OVERRIDES`, collected per the class that HOSTS the
    # member. Two cases both land here: a member enrolled under a row on a
    # DIFFERENT class (the blob-to-disk helper lives on `Client` per binding
    # but is enrolled under a `FlatFilesNamespace` row) and a member whose
    # per-binding spelling diverges from the row name on the SAME class
    # (FlatFileRowList `count` -> C++ `size`; Subscription `kind` -> C++
    # `kind_string`). In both cases the member is already enrolled via its
    # row, so each reverse scan must treat it as enrolled (skip it) rather
    # than flag the idiomatic spelling as an orphan.
    override_home_members: dict[str, set[str]] = {}
    for (row_class, _row_name), binding_targets in METHOD_BINDING_OVERRIDES.items():
        for _binding, (target_class, target_member) in binding_targets.items():
            # The member is enrolled via this row, so register it both under
            # the canonical row class (the key a same-class divergent-spelling
            # scan uses, e.g. Subscription/FlatFileRowList) and under the
            # per-binding collector class the member actually lives on (the key
            # a cross-class scan uses, e.g. the unified Client for the
            # blob-to-disk helper).
            override_home_members.setdefault(row_class, set()).add(target_member)
            override_home_members.setdefault(target_class, set()).add(target_member)

    # Each reverse scan is the same shape — harvest the class members per
    # binding, flag any not enrolled / enrolled-elsewhere / exempt — so they
    # share `_check_class_reverse_orphans`. Per class: the discovery bindings
    # that SEED candidates, whether the Python `_async` twin folds onto its
    # base, the exempt roster, and the report spelling. The unified `Client`
    # excludes C++ from discovery (its heuristic scan over-harvests FFI-extern
    # calls and data members that are not public methods, and the C++ column
    # is validated by the forward row check instead). Subscription /
    # RecordBatchStream / FlatFileRowList / FlatFilesNamespace DO discover from
    # C++: the collector genuinely harvests those classes' methods, and the
    # per-class exempt rosters absorb the handful of heuristic over-harvests
    # (the C++ private builders / decoders and inline-body locals named in
    # each `*_EXEMPT_*` set) so real unenrolled C++ methods still trip.
    errors += _check_class_reverse_orphans(
        "Client",
        {
            "python": py_methods.get("Client", set()),
            "typescript": ts_methods.get("Client", set()),
            "cpp": cpp_methods.get(_cpp_class_for("Client"), set()),
        },
        _enrolled_for("Client"),
        CLIENT_REVERSE_ORPHAN_EXEMPT_MEMBERS,
        discover_langs=frozenset({"python", "typescript"}),
        strip_async=True,
        override_members=frozenset(override_home_members.get("Client", set())),
        report_camel=True,
    )
    errors += _check_class_reverse_orphans(
        "FlatFilesNamespace",
        {
            "python": py_methods.get("FlatFilesNamespace", set()),
            "typescript": ts_methods.get("FlatFilesNamespace", set()),
            "cpp": cpp_methods.get(_cpp_class_for("FlatFilesNamespace"), set()),
        },
        _enrolled_for("FlatFilesNamespace"),
        FLATFILES_NAMESPACE_EXEMPT_MEMBERS,
        discover_langs=frozenset({"python", "typescript", "cpp"}),
        strip_async=True,
        report_camel=False,
    )
    errors += _check_class_reverse_orphans(
        "StreamingSession",
        {"python": py_methods.get("StreamingSession", set())},
        _enrolled_for("StreamingSession"),
        STREAMING_SESSION_EXEMPT_METHODS,
        discover_langs=frozenset({"python"}),
        strip_async=False,
        report_camel=False,
    )
    # The same scan now also covers the fluent subscription handle, the
    # columnar reader, and the flat-file row list — each enrolled above with a
    # per-binding-meaningful method set plus a documented exempt roster for the
    # per-language iterator / enum / materializer idioms that carry no
    # cross-binding contract.
    errors += _check_class_reverse_orphans(
        "Subscription",
        {
            "python": py_methods.get(_py_class_for("Subscription"), set()),
            "typescript": ts_methods.get(_ts_class_for("Subscription"), set()),
            "cpp": cpp_methods.get(_cpp_class_for("Subscription"), set()),
        },
        _enrolled_for("Subscription"),
        SUBSCRIPTION_REVERSE_EXEMPT_METHODS,
        discover_langs=frozenset({"python", "typescript", "cpp"}),
        strip_async=False,
        override_members=frozenset(override_home_members.get("Subscription", set())),
        report_camel=False,
    )
    errors += _check_class_reverse_orphans(
        "RecordBatchStream",
        {
            "python": py_methods.get("RecordBatchStream", set()),
            "typescript": ts_methods.get("RecordBatchStream", set()),
            "cpp": cpp_methods.get("RecordBatchStream", set()),
        },
        _enrolled_for("RecordBatchStream"),
        RECORD_BATCH_STREAM_EXEMPT_METHODS,
        discover_langs=frozenset({"python", "typescript", "cpp"}),
        strip_async=False,
        report_camel=False,
    )
    errors += _check_class_reverse_orphans(
        "FlatFileRowList",
        {
            "python": py_methods.get("FlatFileRowList", set()),
            "typescript": ts_methods.get("FlatFileRowList", set()),
            "cpp": cpp_methods.get("FlatFileRowList", set()),
        },
        _enrolled_for("FlatFileRowList"),
        FLATFILE_ROWLIST_EXEMPT_METHODS,
        discover_langs=frozenset({"python", "typescript", "cpp"}),
        strip_async=False,
        override_members=frozenset(
            override_home_members.get("FlatFileRowList", set())
        ),
        report_camel=False,
    )

    return errors


# `StreamingSession`'s own methods that carry no cross-binding contract:
# the async-iterator protocol (`__aiter__` / `__anext__`), the async and
# sync context-manager entries (`__aenter__` / `__aexit__` / `__enter__` /
# `__exit__`), and the attribute-proxy hook (`__getattr__`) that forwards
# every data call to the inner `Client`. None of these are a method a user
# calls by name expecting cross-binding symmetry, so they are exempt from
# the `StreamingSession` orphan scan. (The cross-binding method collector
# already filters the sync dunders; this roster names the full set
# explicitly so the exemption is self-documenting.)
STREAMING_SESSION_EXEMPT_METHODS: frozenset[str] = frozenset(
    {
        "__aiter__",
        "__anext__",
        "__aenter__",
        "__aexit__",
        "__enter__",
        "__exit__",
        "__getattr__",
        "__next__",
        "__iter__",
    }
)


# Subscription members harvested by the per-binding collectors that carry no
# cross-binding accessor contract: the C++ private static factories
# (`per_contract_stock` / `per_contract_option` / `full_stream`) the heuristic
# class scan picks up, and the TypeScript stringify idiom (`toString`). The
# C++ enum accessor `kind()` is resolved by the enrolled `kind` row name (its
# wire-string sibling is the row's C++ override `kind_string`), so it is not
# listed here.
SUBSCRIPTION_REVERSE_EXEMPT_METHODS: frozenset[str] = frozenset(
    {
        "per_contract_stock",
        "per_contract_option",
        "full_stream",
        "toString",
    }
)


# RecordBatchStream members that are per-language iterator / context-manager
# idioms or private construction helpers, not cross-binding methods: the
# Python sync/async iterator + context-manager dunders, and the C++ private
# static builders / decoders (`create` / `decode_one` / `decode_schema` /
# `open_ipc`) the heuristic class scan harvests. The TypeScript
# `[Symbol.asyncIterator]` / `[Symbol.asyncDispose]` are bracketed and never
# harvested as `\w+` members, so they need no entry.
RECORD_BATCH_STREAM_EXEMPT_METHODS: frozenset[str] = frozenset(
    {
        "__iter__",
        "__next__",
        "__aiter__",
        "__anext__",
        "__aenter__",
        "__aexit__",
        "create",
        "decode_one",
        "decode_schema",
        "open_ipc",
    }
)


# FlatFileRowList members that are the per-language emptiness idiom
# (`__bool__` on Python, `isEmpty` on TypeScript) — the inverse-of-count
# convenience with no cross-binding contract — plus the C++ local variable
# `out` the declaration-shaped scan picks up inside the inline
# `to_arrow_ipc()` body. The row-count / Arrow-IPC methods themselves are
# enrolled via `METHOD_BINDING_OVERRIDES`, not exempted here.
FLATFILE_ROWLIST_EXEMPT_METHODS: frozenset[str] = frozenset(
    {
        "__bool__",
        "isEmpty",
        "out",
    }
)


# ─── Core streaming observability-surface orphan check ──────────────
#
# The cross-binding `[[method]]` rows above gate each DECLARED method
# against the bindings. They do not gate the reverse direction on the
# Rust side: a public observability accessor wired onto the core
# streaming surface (`StreamSurface` view / `StreamingClient`) that never
# grew a parity row is invisible — the row simply does not exist, so no
# check fires, and the accessor silently reaches none of the bindings.
# That is exactly how a wired-but-unbound counter/threshold knob drifts.
#
# This check harvests the public observability accessors on the two core
# streaming surfaces and asserts each maps to a `[[method]]` row. It is
# scoped to the observability accessor SHAPE (cumulative counters, ring
# telemetry, and the slow-callback threshold setter) so it never trips on
# the lifecycle / subscription methods whose cross-binding spelling
# legitimately diverges and which the forward rows already cover.

# The core Rust class name → the parity-toml `class` field. The unified
# streaming surface is the `StreamSurface` view returned by
# `Client::stream()`, tracked in `parity.toml` under the binding-facing
# `StreamView` name.
CORE_STREAMING_CLASS_TO_ROW: dict[str, str] = {
    "StreamSurface": "StreamView",
    "StreamingClient": "StreamingClient",
}

# Core Rust accessor name → the canonical camelCase parity-row `name`.
# Most accessors camelCase directly; the standalone `StreamingClient`
# core counter is named `dropped_count` but the binding/row contract
# spells it `droppedEventCount` (the same counter as the unified
# surface), so it is bridged explicitly here.
CORE_STREAMING_METHOD_RENAMES: dict[str, str] = {
    "dropped_count": "droppedEventCount",
    "dropped_event_count": "droppedEventCount",
}

# An observability accessor is one whose name matches this shape. The
# closure is intentionally narrow: cumulative counters (`*_count`), ring
# telemetry (`ring_*`), and the slow-callback threshold setter. Lifecycle
# / subscription / connection methods do not match and stay governed by
# the forward `[[method]]` rows alone.
def _is_core_observability_accessor(name: str) -> bool:
    if name == "record_panic":
        # Internal `#[cfg(feature = "__internal")]` fault-injection hook,
        # not a public client accessor.
        return False
    if name.endswith("_count"):
        return True
    if name.startswith("ring_"):
        return True
    if name.startswith("set_") and name.endswith("_threshold"):
        return True
    return False


def _collect_core_streaming_observability_methods(
    client_rs: pathlib.Path,
    fpss_mod_rs: pathlib.Path,
) -> dict[str, set[str]]:
    """Return `{core_class: {accessor, ...}}` for the public observability
    accessors on the core streaming surfaces.

    Parses `impl StreamSurface<...> { ... }` in `client.rs` and
    `impl StreamingClient { ... }` in `fpss/mod.rs`, collecting every
    `pub fn <name>` whose name matches the observability shape
    (`_is_core_observability_accessor`). `#[cfg(...)]` / `#[doc(hidden)]`
    gated methods are skipped — they are not part of the published
    surface a binding must mirror.
    """
    out: dict[str, set[str]] = {}
    sources: list[tuple[str, pathlib.Path]] = [
        ("StreamSurface", client_rs),
        ("StreamingClient", fpss_mod_rs),
    ]
    # `impl StreamSurface<'_> {` / `impl StreamingClient {` — tolerate an
    # optional generic / lifetime parameter list before the brace.
    pub_fn_re = re.compile(
        r"((?:#\[[^\]]*\]\s*)*)\bpub\s+fn\s+([a-z_][a-z0-9_]*)\s*[(<]"
    )
    for class_name, rs in sources:
        if not rs.is_file():
            continue
        text = _read_source(rs)
        impl_re = re.compile(
            rf"impl\s+{class_name}\b[^{{]*\{{"
        )
        for header in impl_re.finditer(text):
            body = _balanced_body(text, header.end())
            for m in pub_fn_re.finditer(body):
                attrs = m.group(1)
                name = m.group(2)
                if "cfg(" in attrs or "doc(hidden)" in attrs:
                    continue
                if not _is_core_observability_accessor(name):
                    continue
                out.setdefault(class_name, set()).add(name)
    return out


def _check_core_streaming_method_rows(
    core_methods: dict[str, set[str]],
    method_rows: list[dict[str, Any]],
) -> list[str]:
    """Assert every public observability accessor on the core streaming
    surfaces carries a `[[method]]` parity row.

    Closes the blind spot where a counter / threshold knob wired onto the
    core `StreamSurface` / `StreamingClient` surface reaches none of the
    bindings because no row enrolls it. The row's per-binding columns are
    then gated by `_check_method_rows`; this check only asserts the row
    EXISTS for the right class.
    """
    errors: list[str] = []
    declared: set[tuple[str, str]] = {
        (row.get("class", ""), row.get("name", "")) for row in method_rows
    }
    for core_class, names in sorted(core_methods.items()):
        row_class = CORE_STREAMING_CLASS_TO_ROW.get(core_class, core_class)
        for name in sorted(names):
            row_name = CORE_STREAMING_METHOD_RENAMES.get(name, _snake_to_camel(name))
            if (row_class, row_name) not in declared:
                errors.append(
                    f"  {core_class}::{name}: public observability accessor on "
                    f"the core streaming surface has no `[[method]]` row "
                    f"(expected `class = \"{row_class}\"`, `name = "
                    f"\"{row_name}\"` in parity.toml). A wired-but-"
                    f"unenrolled accessor reaches none of the bindings — add "
                    f"the row and bind it on python / typescript / cpp."
                )
    return errors


# ─── Free-function (utility) discovery ──────────────────────────────
#
# The standalone utility surface — the conditions / exchange / calendar /
# sequence lookups (`condition_name`, `exchange_symbol`,
# `calendar_status_name`, `timestamp_ms`, `sequence_signed_to_unsigned`,
# ...) and the Python-only date-range splitter — is exposed as free
# functions / namespace functions per binding, NOT as methods on a class
# the `[[method]]` rows cover. These collectors find each binding's
# utility surface so the `[[utility]]` rows can pin the cross-binding
# roster.
#
# The TypeScript binding groups its lookup utilities as static methods on
# a `Util` namespace class (`Util.conditionName(...)`), while Python uses
# a `thetadatadx.util` submodule, C++ a `thetadatadx::util` namespace, and the C
# ABI bare `thetadatadx_*` symbols. The collectors normalise each surface to the
# bare snake_case function name so a single `[[utility]]` row matches
# every binding's idiom.


# Internal `#[pyfunction]`s that are NOT part of the public utility
# surface: decode-bench hooks, the FPSS-method introspection helper, and
# the offline streaming-saturation bench hooks (per-event baseline plus the
# batched-delivery and Arrow-columnar throughput levers), all used by tests /
# external benchmarking tooling. Excluded from the Python utility roster
# so they are not mistaken for untracked utilities.
PY_NON_UTILITY_PYFUNCTIONS: frozenset[str] = frozenset(
    {
        "decode_response_bytes",
        "blocked_fpss_methods",
        "__bench_flood_events",
        "__bench_flood_events_batched_calls",
        "__bench_flood_events_batched_list",
        "__bench_flood_events_arrow",
    }
)


def _collect_python_utility_functions(py_src: pathlib.Path) -> set[str]:
    """Snake_case names of every public-utility `#[pyfunction]`.

    The `thetadatadx.util` submodule lookups and the date-range splitter
    are module-level `#[pyfunction] fn <name>`. The attribute may carry a
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
        text = _read_source(rs)
        for m in fn_re.finditer(text):
            out.add(m.group(1))
    return out - PY_NON_UTILITY_PYFUNCTIONS


def _collect_typescript_utility_functions(ts_src: pathlib.Path) -> set[str]:
    """Snake_case names of the TypeScript utility surface.

    The conditions / exchange / calendar / sequence lookups are static
    methods on the `Util` namespace class (`#[napi(js_name =
    "conditionName")] pub fn condition_name` inside `impl Util`), merged in
    by the caller via `_collect_typescript_class_methods` with camelCase
    folded back to snake_case. This scan collects any napi FREE functions
    (none today, but kept so a future free-function utility is caught);
    both surfaces normalise to the bare snake_case name so a single
    `[[utility]]` row matches Python / C++ / FFI and TypeScript alike.

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
    # further `#[...]` / `///` / `//` runs. The attribute arg list is
    # consumed with the balanced-paren scanner so a callback-typed arg
    # (`ts_args_type = "cb: (e) => void"`) cannot truncate the match.
    # `.match(body, after)` anchors at `after`; no `\A`/`^` (which would
    # anchor at string start and never match when `after > 0`).
    following_fn_re = re.compile(
        r"(?:\s*(?:#\[[^\]]*\]|///[^\n]*|//[^\n]*))*\s*"
        r"(?:pub\s+)?(?:async\s+)?fn\s+([a-z_][a-z0-9_]*)\s*[(<]"
    )
    for rs in ts_src.rglob("*.rs"):
        text = _read_source(rs)
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
        for _inner, after in _iter_napi_attrs(body):
            fn_m = following_fn_re.match(body, after)
            if fn_m:
                out.add(fn_m.group(1))
    return out


def _collect_cpp_utility_functions(cpp_hpp: pathlib.Path) -> set[str]:
    """Snake_case names of free functions declared in the `thetadatadx`
    namespace of the C++ wrapper.

    The calculator declarations live in
    `thetadatadx-cpp/include/utilities.hpp.inc`, pulled into `thetadatadx.hpp` via
    `#include "utilities.hpp.inc"`. `_expand_cpp_includes` inlines the
    `.inc` first, then a `<ret> <name>(` shape outside any `class {...}`
    body is a free function. The collector blanks class bodies (mirroring
    the TS impl-body blanking) so member functions are not counted.
    """
    out: set[str] = set()
    if not cpp_hpp.is_file():
        return out
    text = _read_cpp_expanded(cpp_hpp)
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
    """Bare utility names whose `thetadatadx_<name>` C ABI symbol exists.

    The FFI exposes each utility as an `extern "C" fn thetadatadx_<name>`. The
    collector strips the `thetadatadx_` prefix so the result matches the
    canonical `[[utility]]` row name directly.
    """
    out: set[str] = set()
    if not ffi_src.is_dir():
        return out
    for rs in ffi_src.rglob("*.rs"):
        text = _read_source(rs)
        for m in re.finditer(r"\bfn\s+thetadatadx_(\w+)\s*\(", text):
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
    `is_cancel` on Python / TypeScript / C++ but `thetadatadx_condition_is_cancel`
    on the C ABI, where the bare `is_cancel` would be ambiguous against
    the quote-condition predicate) records the bare C-symbol name under an
    `ffi_name` key. The gate strips the `thetadatadx_` prefix off the FFI symbol
    when collecting, so `ffi_name = "condition_is_cancel"` matches
    `thetadatadx_condition_is_cancel`. Absent the override the canonical `name` is
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
                f"`{name}(` in the `thetadatadx::util` namespace",
            ),
            ("ffi", ffi_name, ffi_utils, f"`thetadatadx_{ffi_name}`"),
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
    coercion plumbing or an offline bench hook, not a user-facing utility.

    The JS shim emits a `<tick>_to_arrow_ipc` free function per tick type for
    the zero-copy Arrow boundary, plus the decode-fed projected pair
    (`<tick>_present_columns` + `<tick>_to_arrow_ipc_projected`) that resolves a
    response's wire columns and serialises only those. All three are the same
    per-tick Arrow serialisation family and are not cross-binding utility
    lookups: the managed spelling is singular (`<tick>`) while the C ABI / C++
    projected symbols are plural (`<collection>`), and Python exposes the
    projected shape as a `<Tick>List.to_arrow()` method rather than a free
    function, so a single `[[utility]]` row cannot align them. There are also
    small numeric-coercion helpers (`bigint_to_i32`) and the offline
    streaming-saturation bench hooks (`__bench_flood_events` /
    `__bench_flood_events_batched` / `__bench_flood_events_arrow_ipc`, exported
    as `__benchFloodEvents` / `__benchFloodEventsBatched` /
    `__benchFloodEventsArrowIpc`) that push synthetic events through the real
    tsfn path for benchmarking. None of these are part of the standalone utility
    roster the `[[utility]]` rows track, so they are excluded from the
    TypeScript utility surface — the same carve-out the Python bench hooks get
    via `PY_NON_UTILITY_PYFUNCTIONS`.

    Note `*_to_arrow_ipc` also matches the projected serialiser's stem, but the
    `*_to_arrow_ipc_projected` suffix is listed explicitly since it does not end
    in `_to_arrow_ipc`; `__bench_flood_events_arrow_ipc` is likewise explicit.
    """
    return (
        name.endswith("_to_arrow_ipc")
        or name.endswith("_to_arrow_ipc_projected")
        or name.endswith("_present_columns")
        or name == "bigint_to_i32"
        or name == "__bench_flood_events"
        or name == "__bench_flood_events_batched"
        or name == "__bench_flood_events_arrow_ipc"
    )


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
    canonical `name`. The C++ `thetadatadx::util` / C ABI `thetadatadx_*` surfaces are
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
# set that every binding surfaces (the C ABI `thetadatadx_*_active_subscriptions`
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
    REPO_ROOT / "thetadatadx-rs" / "src" / "fpss" / "protocol" / "subscription.rs"
)
PY_FLUENT_RS = REPO_ROOT / "thetadatadx-py" / "src" / "fluent.rs"
TS_FLUENT_RS = REPO_ROOT / "thetadatadx-ts" / "src" / "fluent.rs"


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
        _read_source(subscription_rs),
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
        _read_source(fluent_rs),
        ("fn kind", "SubscriptionKind"),
    )


def _collect_cpp_subscription_kinds(cpp_hpp: pathlib.Path) -> set[str]:
    """Kind labels emitted by the C++ `FluentSubscription::kind_string`
    switch in `thetadatadx.hpp`. The method is the only place in the header
    that returns these snake_case labels, so a header-wide harvest of the
    canonical vocabulary captures exactly its emitted set (and any
    fictitious `full_*` label, which the canonical-set assertion then
    flags).
    """
    if not cpp_hpp.is_file():
        return set()
    text = _read_source(cpp_hpp)
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
    """Kind labels documented as the C ABI contract in `thetadatadx.h`.

    The C ABI surfaces the kind as the `ThetaDataDxSubscription.kind` string field,
    populated by the Rust core's `kind_str` / `full_kind_str` (so the C ABI
    emits exactly the Rust set at runtime). The header documents the
    canonical label set in the `ThetaDataDxSubscription` doc comment; harvesting
    those literals pins the documented C contract against the same roster
    the runtime emits, so a header that drops a label (or documents a
    fictitious one) is caught.
    """
    if not cpp_h.is_file():
        return set()
    # NB: read RAW (comments intact). Unlike the symbol collectors, this
    # harvest reads the canonical label vocabulary FROM the
    # `ThetaDataDxSubscription` doc comment itself — the documented C
    # contract lives in prose, so stripping comments here would erase
    # exactly what must be checked.
    text = cpp_h.read_text(encoding="utf-8")
    # The kind label vocabulary appears in the `ThetaDataDxSubscription` struct
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
# `THETADATADX_ERR_*` integer code on the C ABI. The leaf vocabulary is the
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

# The canonical `THETADATADX_ERR_*` code roster mapped to its integer value. The C
# ABI surfaces these via `thetadatadx_last_error_code`; the higher bindings
# dispatch on them. `THETADATADX_ERR_NONE` (0) is the no-error sentinel, present in
# the header but not a leaf class, so the leaf-to-code correspondence skips
# it.
CANONICAL_ERROR_CODES: dict[str, int] = {
    "THETADATADX_ERR_NONE": 0,
    "THETADATADX_ERR_OTHER": 1,
    "THETADATADX_ERR_AUTHENTICATION": 2,
    "THETADATADX_ERR_INVALID_CREDENTIALS": 3,
    "THETADATADX_ERR_SUBSCRIPTION": 4,
    "THETADATADX_ERR_RATE_LIMIT": 5,
    "THETADATADX_ERR_NOT_FOUND": 6,
    "THETADATADX_ERR_DEADLINE_EXCEEDED": 7,
    "THETADATADX_ERR_UNAVAILABLE": 8,
    "THETADATADX_ERR_NETWORK": 9,
    "THETADATADX_ERR_SCHEMA_MISMATCH": 10,
    "THETADATADX_ERR_STREAM": 11,
    "THETADATADX_ERR_CONFIG": 12,
    "THETADATADX_ERR_INVALID_PARAMETER": 13,
}

PY_ERRORS_RS = REPO_ROOT / "thetadatadx-py" / "src" / "errors.rs"
TS_LIB_RS = REPO_ROOT / "thetadatadx-ts" / "src" / "lib.rs"
FFI_ERROR_RS = REPO_ROOT / "thetadatadx-ffi" / "src" / "error.rs"

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
    text = _read_source(py_errors_rs)
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
    text = _read_source(ts_lib_rs)
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
    text = _read_source(cpp_hpp)
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
    """`THETADATADX_ERR_*` discriminants defined in the FFI `error.rs`.

    Returns `{code_name: int_value}` for every `pub const THETADATADX_ERR_* : i32 =
    N;` declaration — the source of truth for the C ABI error codes.
    """
    if not ffi_error_rs.is_file():
        return {}
    text = _read_source(ffi_error_rs)
    out: dict[str, int] = {}
    for name, value in re.findall(
        r"pub const (THETADATADX_ERR_\w+)\s*:\s*i32\s*=\s*(\d+)\s*;", text
    ):
        out[name] = int(value)
    return out


def _collect_ffi_error_codes_dispatched(ffi_error_rs: pathlib.Path) -> set[str]:
    """`THETADATADX_ERR_*` codes the FFI `error_code_for` dispatch actually returns.

    Bounds the harvest to the `fn error_code_for` body so the roster is the
    set the dispatch routes to (not merely the defined constants).
    """
    if not ffi_error_rs.is_file():
        return set()
    text = _read_source(ffi_error_rs)
    m = re.search(r"fn error_code_for\s*\([^)]*\)\s*->\s*i32\s*\{", text)
    if not m:
        return set()
    body = _balanced_body(text, m.end())
    return set(re.findall(r"\b(THETADATADX_ERR_\w+)\b", body))


def _collect_cpp_error_codes(cpp_h: pathlib.Path) -> dict[str, int]:
    """`THETADATADX_ERR_*` codes defined in the C ABI header `thetadatadx.h`.

    Returns `{code_name: int_value}` for every `#define THETADATADX_ERR_* N`. The
    header is hand-maintained; the gate asserts it matches the FFI Rust
    constants exactly so a code that drifts on the C side (invisible to
    `cargo build`) is caught.
    """
    if not cpp_h.is_file():
        return {}
    text = _read_source(cpp_h)
    out: dict[str, int] = {}
    for name, value in re.findall(r"#define\s+(THETADATADX_ERR_\w+)\s+(\d+)\b", text):
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
    2. The FFI `THETADATADX_ERR_*` constants equal the canonical code table
       (name → value), so a renumbered or renamed code trips.
    3. The C ABI header `#define`s match the FFI Rust constants exactly,
       so a hand-maintained-header drift (invisible to `cargo build`)
       trips.
    4. Every dispatched FFI code is defined, and every leaf class has a
       corresponding `THETADATADX_ERR_*` code, so the leaf set and the code set
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
                errors.append(f"  ffi: `{name}` is not defined in thetadatadx-ffi/src/error.rs")
            elif ffi_codes[name] != value:
                errors.append(
                    f"  ffi: `{name}` = {ffi_codes[name]} in thetadatadx-ffi/src/error.rs, "
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
                    f"  cpp header: `{name}` defined in thetadatadx-ffi/src/error.rs but "
                    f"missing from thetadatadx-cpp/include/thetadatadx.h"
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
#     `thetadatadx-py/src/_generated/historical_methods.rs`).
#   * TypeScript: a `<endpoint>Stream` method on the `Client`
#     napi class (generated into
#     `thetadatadx-ts/src/_generated/historical_methods.rs`).
#   * C ABI: a `thetadatadx_<endpoint>_stream` extern "C" symbol in `thetadatadx-ffi/src/`.
#   * C++: an `<endpoint>_stream` member on the `Client` wrapper
#     (`thetadatadx.hpp` + its `.inc` fragments).
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
        text = _read_source(rs)
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
    """Snake_case endpoint names whose `HistoricalView` napi class
    exposes a `<endpoint>Stream` method.

    The server-stream companions live on the `client.historical`
    `HistoricalView` view alongside the buffered historical queries.
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
    methods = ts_methods.get("HistoricalView", set())
    lifecycle = {"startStreaming", "stopStreaming", "isStreaming"}
    for method in methods:
        if method in lifecycle:
            continue
        if method.endswith("Stream") and len(method) > len("Stream"):
            stem = method[: -len("Stream")]
            out.add(_endpoint_method_to_snake(stem))
    return out


def _collect_ffi_streaming_endpoints(ffi_src: pathlib.Path) -> set[str]:
    """Snake_case endpoint names whose `thetadatadx_<endpoint>_stream` extern "C"
    symbol exists in `thetadatadx-ffi/src/`.

    The `thetadatadx_client_*` / `thetadatadx_streaming_*` callback symbols never match
    the `thetadatadx_<name>_stream` shape (their stems are `client` / `streaming`
    and they end in `set_callback` / `reconnect` / `shutdown`, not
    `_stream`), so they are not mistaken for a historical endpoint.
    """
    out: set[str] = set()
    if not ffi_src.is_dir():
        return out
    fn_re = re.compile(r"\bfn\s+thetadatadx_(\w+)_stream\s*\(")
    for rs in ffi_src.rglob("*.rs"):
        text = _read_source(rs)
        for m in fn_re.finditer(text):
            out.add(m.group(1))
    return out


def _collect_cpp_streaming_endpoints(cpp_methods: dict[str, set[str]]) -> set[str]:
    """Snake_case endpoint names whose C++ `Historical` view exposes an
    `<endpoint>_stream` member.

    Reuses the already-collected C++ `{class: {method, ...}}` map. The
    server-stream companions live on the `client.historical()`
    `Historical` view body in `thetadatadx.hpp`. A member whose snake_case
    name ends in `_stream` is a server-stream terminal; strip the suffix
    to recover the endpoint name.
    """
    out: set[str] = set()
    methods = cpp_methods.get(_cpp_class_for("HistoricalView"), set())
    for method in methods:
        if method.endswith("_stream") and len(method) > len("_stream"):
            out.add(method[: -len("_stream")])
    return out


def _check_historical_streaming_rows(
    rows: list[dict[str, Any]],
    rust_stream: set[str],
    py_stream: set[str],
    ts_stream: set[str],
    cpp_stream: set[str],
    ffi_stream: set[str],
) -> list[str]:
    """Per-endpoint cross-binding gate for `[[historical_streaming]]` rows.

    Each row declares a snake_case endpoint `name` plus the expected
    server-stream presence on Rust / Python / TypeScript / C++ / the C ABI.
    The checker compares the declared state against the actual surface state
    and returns a list of mismatch strings (empty when every row matches).
    The Rust column is the source of truth — the `<Endpoint>Builder::stream`
    terminal the registry generates and every binding mirrors; a dropped or
    renamed Rust streaming endpoint trips here even if a binding still
    declares it.

    Beyond the per-row check, the collected sets are reconciled against
    the union of declared row names: an endpoint that streams on ANY
    surface but has no row at all trips the gate, so a newly-streamed
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
            ("rust", rust_stream, f"`{pascal}Builder::stream` terminal (registry of record)"),
            ("python", py_stream, f"`fn stream` on the `{pascal}Builder` pyclass"),
            ("typescript", ts_stream, f"`{camel}Stream` on the `Client` napi class"),
            ("cpp", cpp_stream, f"`{name}_stream(` on the C++ `Client` body"),
            ("ffi", ffi_stream, f"`thetadatadx_{name}_stream` extern \"C\" symbol"),
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
    # surface but has no row is undocumented drift.
    seen = rust_stream | py_stream | ts_stream | cpp_stream | ffi_stream
    for endpoint in sorted(seen - declared_names):
        on = sorted(
            lang
            for lang, s in (
                ("rust", rust_stream),
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


# ─── Historical async query surface ([[historical_async]]) ────────────
#
# Every buffered historical endpoint carries a non-blocking query
# companion so callers can run a request off the calling thread without
# managing their own threads. The shape is per-binding idiom:
#
#   * Python: an `<endpoint>_async` method on the historical surface,
#     returning an awaitable (generated into
#     `thetadatadx-py/src/_generated/historical_methods.rs`).
#   * TypeScript: the buffered `<endpoint>` method is itself `async` and
#     returns a `Promise`, so there is no separate `_async` name — the
#     presence of the buffered endpoint method on `HistoricalView` IS the
#     async surface.
#   * C++: an inline `<endpoint>_async` member on the `Historical` view
#     returning `std::future<std::vector<Row>>` over the blocking member.
#
# There is no C ABI row: the async surface is a binding-layer concern
# layered over the existing blocking `thetadatadx_<endpoint>_with_options`
# symbols. C has no awaitable type and its idiom is callback / poll, so
# the C ABI stays blocking-only by design and is not tracked here.
#
# Like the streaming family, these terminals are endpoint-named methods
# NOT covered by the `[[method]]` rows, so without this family a binding
# could ship the async query on some endpoints and silently omit it on
# others (or on a whole binding) with no checker noticing.


def _collect_python_async_endpoints(py_src: pathlib.Path) -> set[str]:
    """Snake_case endpoint names whose Python `<Endpoint>Builder` pyclass
    exposes a buffered `list_async` terminal.

    The Python async query surface is the awaitable `list_async()` terminal
    on each buffered endpoint's `<Endpoint>Builder` — the async twin of the
    blocking `list()` collect. It rides on the same builder the blocking
    query returns, so the builder's endpoint name (the CamelCase struct
    stem, lowered to snake_case) is the async-query presence signal. The
    server-stream `stream_async` terminal is a different surface (tracked by
    the `[[historical_streaming]]` family) and is not matched here.
    """
    out: set[str] = set()
    if not py_src.is_dir():
        return out
    impl_re = re.compile(r"impl\s+(\w+)Builder\s*\{")
    for rs in py_src.rglob("*.rs"):
        text = _read_source(rs)
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
            # The terminal carries a `<'py>` lifetime generic between the
            # name and the arg list (`fn list_async<'py>(`), so allow an
            # optional generic-parameter clause before the paren.
            if re.search(r"\bfn\s+list_async\s*(<[^>]*>)?\s*\(", body):
                out.add(_endpoint_method_to_snake(stem))
    return out


def _collect_typescript_async_endpoints(
    ts_methods: dict[str, set[str]],
) -> set[str]:
    """Snake_case endpoint names whose `HistoricalView` napi class exposes a
    buffered `<endpoint>` query method.

    Every buffered TypeScript data method is an `async fn` returning a
    `Promise`, so the buffered endpoint method IS the async surface — there
    is no separate `_async` spelling. Reuses the already-collected
    `{class: {method, ...}}` map: take the `HistoricalView` methods, drop
    the `<endpoint>Stream` server-stream companions, the `<endpoint>WithColumns`
    presence-carrying variants (tracked by the withColumns reachability gate),
    and the FPSS lifecycle methods, and snake-ify the remainder to recover the
    endpoint names.
    """
    out: set[str] = set()
    methods = ts_methods.get("HistoricalView", set())
    lifecycle = {"startStreaming", "stopStreaming", "isStreaming"}
    for method in methods:
        if method in lifecycle:
            continue
        if method.endswith("Stream") and len(method) > len("Stream"):
            continue
        if method.endswith("WithColumns") and len(method) > len("WithColumns"):
            continue
        out.add(_endpoint_method_to_snake(method))
    return out


def _collect_cpp_async_endpoints(cpp_methods: dict[str, set[str]]) -> set[str]:
    """Snake_case endpoint names whose C++ `Historical` view exposes an
    `<endpoint>_async` member.

    Reuses the already-collected C++ `{class: {method, ...}}` map. A member
    whose snake_case name ends in `_async` is a non-blocking query
    companion; strip the suffix to recover the endpoint name.
    """
    out: set[str] = set()
    methods = cpp_methods.get(_cpp_class_for("HistoricalView"), set())
    for method in methods:
        if method.endswith("_async") and len(method) > len("_async"):
            out.add(method[: -len("_async")])
    return out


def _check_historical_async_rows(
    rows: list[dict[str, Any]],
    rust_async: set[str],
    py_async: set[str],
    ts_async: set[str],
    cpp_async: set[str],
) -> list[str]:
    """Per-endpoint cross-binding gate for `[[historical_async]]` rows.

    Each row declares a snake_case endpoint `name` plus the expected async
    query presence on Rust / Python / TypeScript / C++. The checker compares
    the declared state against the actual surface state and returns a list of
    mismatch strings (empty when every row matches). The Rust column is the
    source of truth: every buffered endpoint in the registry carries a
    `HistoricalClient::<name>` async query method (the builder-await /
    `list_async` twin of the blocking collect), so a dropped or renamed Rust
    endpoint trips here even if a binding still declares it. There is no C
    ABI column — the async surface is a binding-layer concern over the
    blocking `_with_options` symbols (C has no awaitable type).

    Beyond the per-row check, the collected sets are reconciled against the
    union of declared row names: an endpoint that exposes an async query on
    ANY surface but has no row at all trips the gate, so a newly-async
    endpoint cannot slip in untracked.
    """
    errors: list[str] = []
    declared_names = {row.get("name") for row in rows if row.get("name")}
    for row in rows:
        name = row.get("name")
        if not name:
            errors.append(f"  [[historical_async]] row missing `name`: {row!r}")
            continue
        camel = _snake_to_camel(name)
        for lang, actual_set, hint in (
            ("rust", rust_async, f"`HistoricalClient::{name}` async query method (registry of record)"),
            ("python", py_async, f"`fn {name}_async` on the historical surface"),
            ("typescript", ts_async, f"`{camel}` async (Promise) method on `HistoricalView`"),
            ("cpp", cpp_async, f"`{name}_async(` on the C++ `Historical` view"),
        ):
            declared = row.get(lang, False)
            actual = name in actual_set
            if declared != actual:
                verb = "missing" if declared and not actual else "unexpected"
                errors.append(
                    f"  {name}.{lang}: declared={declared}, actual={actual} "
                    f"({verb} -- expected {hint})"
                )
    # Reverse-direction orphan check: any endpoint with an async query on a
    # surface but no row is undocumented drift.
    seen = rust_async | py_async | ts_async | cpp_async
    for endpoint in sorted(seen - declared_names):
        on = sorted(
            lang
            for lang, s in (
                ("rust", rust_async),
                ("python", py_async),
                ("typescript", ts_async),
                ("cpp", cpp_async),
            )
            if endpoint in s
        )
        errors.append(
            f"  {endpoint}: exposes an async query on {on} but has no "
            f"[[historical_async]] row. Add one declaring its "
            f"per-binding presence so the surface stays tracked."
        )
    return errors


# ─── Rust historical surface (registry of record) ────────────────────
#
# The historical-async / historical-streaming / historical-base families
# gate the managed bindings (Python / TypeScript / C++) and the C ABI, but
# the Rust core is the SOURCE OF TRUTH every one of them is generated from:
# the build pipeline reads `endpoint_surface.toml` and emits the
# `HistoricalClient::<endpoint>` buffered method, its async / streaming
# companions, and the `thetadatadx_<endpoint>_with_options` C-ABI base
# symbol. Without a Rust column + a collector that reads the actual Rust
# historical surface, a dropped or renamed Rust endpoint reaches none of
# the bindings yet trips no checker — the registry simply has one fewer
# entry and every downstream surface shrinks in lockstep, silently.
#
# The generated Rust method files live only in `OUT_DIR` (not committed),
# so the gate reads the registry of record directly — `endpoint_surface.toml`
# — which is the deterministic, no-build source the build pipeline itself
# consumes. Every `[[endpoints]]` entry generates a `HistoricalClient`
# method, so the registry IS the Rust historical surface.


def _collect_rust_buffered_endpoints(
    surface_toml: pathlib.Path,
) -> set[str]:
    """Snake_case names of every buffered historical endpoint in the
    registry of record.

    Parses `endpoint_surface.toml` and returns each `[[endpoints]]` `name`
    that is NOT a `*_stream` entry. The four `*_stream` entries are the
    FPSS real-time subscription endpoints (a distinct surface tracked by
    the streaming-client `[[method]]` rows); the remainder are the buffered
    historical endpoints (the 61-endpoint base surface). Each generates a
    `HistoricalClient::<name>` method on the Rust core, so this set is the
    Rust buffered + async query surface (every buffered endpoint carries an
    `.await` collect and the `list_async` / builder-await async twin).
    """
    if not surface_toml.is_file():
        return set()
    data = tomllib.loads(surface_toml.read_text(encoding="utf-8"))
    out: set[str] = set()
    for endpoint in data.get("endpoints", []):
        name = endpoint.get("name")
        if not name or name.endswith("_stream"):
            continue
        out.add(name)
    return out


def _endpoint_is_simple_list(endpoint: dict[str, Any]) -> bool:
    """True for a flat-list endpoint (`Vec<String>` return).

    Mirrors the build's `is_simple_list_endpoint` SSOT from the registry
    surface: a list endpoint declares a `list_column` (the field it
    projects) and/or returns the `StringList` collection. Flat lists return
    `Vec<String>`, not a typed row collection, so they get no server-stream
    terminal.
    """
    return "list_column" in endpoint or endpoint.get("returns") == "StringList"


def _endpoint_is_snapshot(endpoint: dict[str, Any]) -> bool:
    """True for a snapshot endpoint (bounded one-row-per-request result).

    Mirrors the build's `is_snapshot_endpoint` SSOT structurally from the
    registry: the snapshot endpoints carry the `_snapshot_` segment in their
    canonical name (`stock_snapshot_quote`, `option_snapshot_greeks_all`).
    A snapshot has nothing to drain incrementally, so it gets no
    server-stream terminal.
    """
    return "_snapshot_" in endpoint.get("name", "")


def _endpoint_is_calendar(endpoint: dict[str, Any]) -> bool:
    """True for a calendar endpoint (bounded handful of rows).

    Mirrors the build's calendar exclusion from the registry: the calendar
    endpoints are named under the `calendar_` prefix. Like snapshots, they
    return a bounded result and get no server-stream terminal.
    """
    return endpoint.get("name", "").startswith("calendar")


def _collect_rust_columnar_buffered_endpoints(
    surface_toml: pathlib.Path,
) -> set[str]:
    """Snake_case names of every buffered COLUMNAR historical endpoint.

    The buffered set (`_collect_rust_buffered_endpoints`) minus the flat
    `StringList` list endpoints, which carry no column set. Each columnar
    endpoint's `.await` decode yields a core `Ticks<T>` carrying the response's
    `ColumnPresence`, so each must surface a presence-carrying variant on the
    managed bindings for the projected Arrow-IPC exit to be drivable from a live
    call. This is the expected set the TypeScript `withColumns` reachability gate
    holds the binding to (the same registry of record the streaming / base
    families read).
    """
    if not surface_toml.is_file():
        return set()
    data = tomllib.loads(surface_toml.read_text(encoding="utf-8"))
    out: set[str] = set()
    for endpoint in data.get("endpoints", []):
        name = endpoint.get("name")
        if not name or name.endswith("_stream"):
            continue
        if _endpoint_is_simple_list(endpoint):
            continue
        out.add(name)
    return out


def _collect_typescript_with_columns_endpoints(
    hist_methods_rs: pathlib.Path,
) -> set[str]:
    """Snake_case endpoint names whose TypeScript historical surface exposes a
    `<endpoint>WithColumns` presence-carrying query variant.

    Reads the committed generated napi source
    (`thetadatadx-ts/src/_generated/historical_methods.rs`) rather than the
    built `index.d.ts`, so the gate holds against the deterministic no-build
    source the napi compile lowers into the `.d.ts` — the same reason the
    streaming / buffered rows read `endpoint_surface.toml`. Each variant carries
    a `#[napi(js_name = "<camel>WithColumns")]` attribute; the camel stem
    snake-ifies back to the endpoint name. The method is emitted onto both the
    `HistoricalView` and `HistoricalClient` impl blocks, so the set dedups.
    """
    out: set[str] = set()
    if not hist_methods_rs.is_file():
        return out
    text = _read_source(hist_methods_rs)
    for m in re.finditer(r'#\[napi\(js_name = "(\w+?)WithColumns"\)\]', text):
        out.add(_endpoint_method_to_snake(m.group(1)))
    return out


def _check_typescript_with_columns_reachability(
    expected: set[str],
    actual: set[str],
) -> list[str]:
    """The TypeScript projected-Arrow reachability gate.

    Every buffered columnar endpoint (`expected`) must surface a
    `<endpoint>WithColumns` variant returning the rows plus the response's
    `presentColumns` / `symbol`, so the projected Arrow-IPC exit is reachable
    from a live call at parity with Python's presence-carrying `<Tick>List` and
    the C / C++ `_with_options` presence out-param. A missing variant leaves the
    only live-call path on that endpoint the full-schema `<tick>ToArrowIpc`,
    which emits always-present flag / contract-identity columns the wire never
    sent. An unexpected variant (present on TS but not a columnar buffered
    endpoint) is generator drift. Both trip the gate.
    """
    errors: list[str] = []
    for name in sorted(expected - actual):
        errors.append(
            f"  {name}: buffered columnar endpoint has no `{_snake_to_camel(name)}"
            f"WithColumns` TypeScript variant. Without it a live caller cannot "
            f"obtain the response's presentColumns / symbol, so the projected "
            f"Arrow-IPC export is unreachable (the #1072 always-zero-flag frame). "
            f"Regenerate the TypeScript historical surface."
        )
    for name in sorted(actual - expected):
        errors.append(
            f"  {name}: TypeScript exposes a `{_snake_to_camel(name)}WithColumns` "
            f"variant but it is not a buffered columnar endpoint. Remove it or "
            f"correct the endpoint registry."
        )
    return errors


def _collect_rust_streaming_endpoints(
    surface_toml: pathlib.Path,
) -> set[str]:
    """Snake_case names of every buffered endpoint that carries a Rust
    server-stream terminal (`<Endpoint>Builder::stream`).

    Mirrors the build's `endpoint_streams` SSOT predicate from the registry
    of record: a buffered endpoint streams unless it is a flat list (returns
    `Vec<String>`, no typed row chunks), a snapshot, or a calendar (both
    bounded, nothing to drain). The multi-day / full-universe history pulls
    — the ones whose buffered collect can exhaust memory — are exactly the
    streamable set.

    This is the Rust streaming surface independent of the managed bindings:
    a Rust endpoint dropped from the registry vanishes from this set even if
    a binding still (wrongly) declares it, so the per-row gate trips. The
    classification is pinned against the live generated Python `fn stream`
    surface by `_case_hist_stream_rust_mirrors_generated` so a future change
    to the build SSOT cannot silently desync the gate's mirror.
    """
    if not surface_toml.is_file():
        return set()
    data = tomllib.loads(surface_toml.read_text(encoding="utf-8"))
    out: set[str] = set()
    for endpoint in data.get("endpoints", []):
        name = endpoint.get("name")
        if not name or name.endswith("_stream"):
            continue
        if _endpoint_is_simple_list(endpoint):
            continue
        if _endpoint_is_snapshot(endpoint):
            continue
        if _endpoint_is_calendar(endpoint):
            continue
        out.add(name)
    return out


def _collect_cabi_base_endpoints(with_options_inc: pathlib.Path) -> set[str]:
    """Snake_case endpoint names whose `thetadatadx_<endpoint>_with_options`
    base symbol is declared in the SHIPPED C-ABI header fragment.

    Reads `thetadatadx-cpp/include/endpoint_with_options.h.inc` — the header
    `thetadatadx.h` includes — rather than the `thetadatadx-ffi/src` source, so a stale
    regenerated header that dropped (or renamed) a base symbol relative to
    the Rust source of truth is caught. The base family cross-checks this
    set against the Rust registry, so a header/registry divergence in EITHER
    direction trips.
    """
    out: set[str] = set()
    if not with_options_inc.is_file():
        return out
    text = _read_source(with_options_inc)
    for m in re.finditer(r"\bthetadatadx_(\w+)_with_options\s*\(", text):
        out.add(m.group(1))
    return out


def _collect_ffi_base_endpoints(ffi_src: pathlib.Path) -> set[str]:
    """Snake_case endpoint names whose `thetadatadx_<endpoint>_with_options`
    base symbol is DEFINED in the `thetadatadx-ffi/src` Rust source.

    The companion to `_collect_cabi_base_endpoints`: the shipped header
    declares the symbol, this source defines it. The base family asserts the
    two agree, so a header that declares a symbol the source never defines
    (a link-time failure invisible to a header-only scan), or a source
    symbol the header forgot to declare (invisible to `cargo build`), trips.
    """
    out: set[str] = set()
    if not ffi_src.is_dir():
        return out
    fn_re = re.compile(r"\bfn\s+thetadatadx_(\w+)_with_options\s*\(")
    for rs in ffi_src.rglob("*.rs"):
        text = _read_source(rs)
        for m in fn_re.finditer(text):
            out.add(m.group(1))
    return out


# ─── Historical buffered base surface ([[historical_base]]) ───────────
#
# Every buffered historical endpoint exposes a blocking query terminal on
# all five surfaces: the Rust `HistoricalClient::<endpoint>` method, the
# Python `<Endpoint>Builder.list()` collect, the TypeScript buffered
# `<endpoint>` method (it is itself async-returning, so the buffered and
# async surfaces coincide there), the C++ `Historical::<endpoint>(...)`
# member, and the C-ABI `thetadatadx_<endpoint>_with_options` base symbol.
#
# This is the core "every endpoint exists everywhere" guarantee. Before
# this family the base sync presence was asserted only TRANSITIVELY — via
# the async / stream companions on the managed bindings — and the 61 C-ABI
# `_with_options` base functions were checked by NO family at all. A
# `[[historical_base]]` row pins one endpoint's blocking-query presence
# across all five surfaces directly.


def _collect_python_buffered_endpoints(py_src: pathlib.Path) -> set[str]:
    """Snake_case endpoint names whose Python `<Endpoint>Builder` pyclass
    exposes a buffered `list` terminal.

    The Python buffered query surface is the `list()` collect on each
    endpoint's `<Endpoint>Builder` — the blocking twin of the awaitable
    `list_async()`. Walks every `impl <Name>Builder { ... }` block and
    records the builder's endpoint name (the CamelCase struct stem, lowered)
    when the body declares `fn list(`. Mirrors the async / stream collectors
    on the buffered terminal.
    """
    out: set[str] = set()
    if not py_src.is_dir():
        return out
    impl_re = re.compile(r"impl\s+(\w+)Builder\s*\{")
    for rs in py_src.rglob("*.rs"):
        text = _read_source(rs)
        for header in impl_re.finditer(text):
            stem = header.group(1)
            body = _balanced_body(text, header.end())
            if re.search(r"\bfn\s+list\s*\(", body):
                out.add(_endpoint_method_to_snake(stem))
    return out


def _check_historical_base_rows(
    rows: list[dict[str, Any]],
    rust_buffered: set[str],
    py_buffered: set[str],
    ts_buffered: set[str],
    cpp_buffered: set[str],
    cabi_base: set[str],
    ffi_base: set[str],
) -> list[str]:
    """Per-endpoint cross-binding gate for `[[historical_base]]` rows.

    Each row declares a snake_case endpoint `name` plus the expected
    buffered-query presence on all five surfaces: Rust (the registry-of-
    record buffered method), Python (`<Endpoint>Builder.list()`), TypeScript
    (the buffered `<endpoint>` method), C++ (`Historical::<endpoint>`), and
    the C-ABI `thetadatadx_<endpoint>_with_options` base symbol. The checker
    compares the declared state against the actual surface state and returns
    a list of mismatch strings (empty when every row matches).

    Two reconciliations beyond the per-row forward check:

    1. Reverse-direction orphan scan: an endpoint present on ANY surface but
       with no row at all trips, so a new endpoint cannot slip in untracked.
    2. Header/source/registry agreement: the shipped C-ABI header
       (`cabi_base`), the `thetadatadx-ffi/src` source (`ffi_base`), and the Rust
       registry (`rust_buffered`) must declare the same base set. A stale
       header that drifted from the source of truth trips here, without
       duplicating the broader `.so`↔header completeness checker.
    """
    errors: list[str] = []
    declared_names = {row.get("name") for row in rows if row.get("name")}
    for row in rows:
        name = row.get("name")
        if not name:
            errors.append(f"  [[historical_base]] row missing `name`: {row!r}")
            continue
        for lang, actual_set, hint in (
            ("rust", rust_buffered, f"`HistoricalClient::{name}` buffered method (registry of record)"),
            ("python", py_buffered, f"`fn list` on the `{name}` builder pyclass"),
            ("typescript", ts_buffered, f"buffered `{_snake_to_camel(name)}` method on `HistoricalView`"),
            ("cpp", cpp_buffered, f"`{name}(` on the C++ `Historical` view"),
            ("ffi", cabi_base, f"`thetadatadx_{name}_with_options` extern \"C\" symbol"),
        ):
            declared = row.get(lang, False)
            actual = name in actual_set
            if declared != actual:
                verb = "missing" if declared and not actual else "unexpected"
                errors.append(
                    f"  {name}.{lang}: declared={declared}, actual={actual} "
                    f"({verb} -- expected {hint})"
                )

    # Reverse-direction orphan check: any endpoint present on a surface but
    # with no row is undocumented drift.
    seen = (
        rust_buffered | py_buffered | ts_buffered | cabi_base
    )
    # C++ buffered members co-mingle with unrelated wrapper helpers, so the
    # orphan scan uses the four cleanly-enumerable surfaces; the C++ column
    # is still pinned forward per row.
    for endpoint in sorted(seen - declared_names):
        on = sorted(
            lang
            for lang, s in (
                ("rust", rust_buffered),
                ("python", py_buffered),
                ("typescript", ts_buffered),
                ("ffi", cabi_base),
            )
            if endpoint in s
        )
        errors.append(
            f"  {endpoint}: present on {on} but has no [[historical_base]] "
            f"row. Add one declaring its per-surface presence so the "
            f"buffered base surface stays tracked."
        )

    # Header / source / registry agreement on the C-ABI base set. The
    # shipped header is what downstream C / C++ consumers compile against;
    # it must declare exactly the base symbols the `thetadatadx-ffi/src` source defines
    # and the Rust registry generates. A divergence in any direction is a
    # stale-artifact defect that no per-row column would surface on its own.
    if cabi_base and ffi_base and cabi_base != ffi_base:
        for ep in sorted(ffi_base - cabi_base):
            errors.append(
                f"  {ep}: `thetadatadx_{ep}_with_options` is defined in thetadatadx-ffi/src "
                f"but missing from the shipped header "
                f"thetadatadx-cpp/include/endpoint_with_options.h.inc (stale header)."
            )
        for ep in sorted(cabi_base - ffi_base):
            errors.append(
                f"  {ep}: `thetadatadx_{ep}_with_options` is declared in the "
                f"shipped header but not defined in thetadatadx-ffi/src (dangling header "
                f"declaration)."
            )
    if cabi_base and rust_buffered and cabi_base != rust_buffered:
        for ep in sorted(rust_buffered - cabi_base):
            errors.append(
                f"  {ep}: buffered endpoint in the Rust registry has no "
                f"`thetadatadx_{ep}_with_options` base symbol in the shipped "
                f"C-ABI header."
            )
        for ep in sorted(cabi_base - rust_buffered):
            errors.append(
                f"  {ep}: C-ABI base symbol `thetadatadx_{ep}_with_options` "
                f"has no matching buffered endpoint in the Rust registry "
                f"(endpoint_surface.toml)."
            )
    return errors


# ─── Client construction-from-file surface ([[from_file]]) ────────────
#
# Every standalone client class exposes a one-call file-construction
# convenience that loads credentials from a two-line file and connects.
# The entry point is not a data method the `[[method]]` rows cover, and
# its spelling differs per binding (`from_file` on Python / C++,
# `connectFromFile` on TypeScript, `thetadatadx_<stem>_connect_from_file` on the
# C ABI), so a single `[[method]]` row cannot express it. These
# collectors harvest each binding's file-construction surface so the
# `[[from_file]]` rows can pin the cross-binding roster.
#
# `name` is the cross-binding client class identifier. The C ABI symbol
# stem differs from the class name (`thetadatadx_client_connect_from_file` for
# `Client`, `thetadatadx_historical_connect_from_file` for
# `HistoricalClient`, `thetadatadx_streaming_connect_from_file` for `StreamingClient`); the stem
# table below bridges the two.
#
# The family governs exactly the standalone clients that connect to the
# servers via a `thetadatadx_<stem>_connect_from_file` C ABI symbol. It does NOT
# govern `Credentials.from_file` (a credentials factory that returns a
# `Credentials`, not a connected client, surfaced over the distinct
# `thetadatadx_credentials_from_file` symbol) nor the Python-only
# `AsyncClient.from_file` (no C ABI / managed-binding twin —
# its presence is tracked by the `AsyncClient` `[[class]]`
# row). Scoping the collectors to the governed roster keeps those
# unrelated `from_file` entry points out of this family while still
# tripping on a new governed client that forgets a row.


# Parity class name → C ABI symbol stem for the
# `thetadatadx_<stem>_connect_from_file` extern "C" symbol. The keys are the
# governed client roster: every class this family tracks, and the only
# classes the collectors below consider.
FROM_FILE_FFI_STEMS: dict[str, str] = {
    "Client": "client",
    "HistoricalClient": "historical",
    "StreamingClient": "streaming",
}

# The governed client roster (the C++ class spelling of each, so the
# C++ collector can match the harvested class name before folding it
# back to the cross-binding identifier).
_FROM_FILE_GOVERNED: frozenset[str] = frozenset(FROM_FILE_FFI_STEMS)


def _collect_python_from_file_classes(py_methods: dict[str, set[str]]) -> set[str]:
    """Governed client classes whose Python pyclass exposes a `from_file`
    staticmethod.

    Reuses the already-collected `{pyclass: {method, ...}}` map; a
    governed class carrying `from_file` exposes the file-construction
    convenience. Scoped to `_FROM_FILE_GOVERNED` so the credentials
    factory (`Credentials.from_file`) and the Python-only async twin are
    not mistaken for governed clients.
    """
    return {
        cls
        for cls, methods in py_methods.items()
        if cls in _FROM_FILE_GOVERNED and "from_file" in methods
    }


def _collect_typescript_from_file_classes(ts_methods: dict[str, set[str]]) -> set[str]:
    """Governed client classes whose TypeScript napi class exposes a
    `connectFromFile` factory.

    Reuses the already-collected `{class: {method, ...}}` map; the napi
    factory's `js_name` is the camelCase `connectFromFile`. Scoped to the
    governed roster (the `Credentials.fromFile` factory is excluded).
    """
    return {
        cls
        for cls, methods in ts_methods.items()
        if cls in _FROM_FILE_GOVERNED and "connectFromFile" in methods
    }


def _collect_cpp_from_file_classes(cpp_methods: dict[str, set[str]]) -> set[str]:
    """Governed client classes whose C++ wrapper exposes a `from_file`
    static member.

    Reuses the already-collected `{class: {method, ...}}` map (which
    inlines the generator-emitted `*.inc` member declarations), folds the
    C++ alias names back to the cross-binding class identifier, and scopes
    to the governed roster so `Credentials::from_file` is not counted.
    """
    reverse_alias = {v: k for k, v in CPP_ALIASES.items()}
    out: set[str] = set()
    for cls, methods in cpp_methods.items():
        if "from_file" in methods:
            canonical = reverse_alias.get(cls, cls)
            if canonical in _FROM_FILE_GOVERNED:
                out.add(canonical)
    return out


def _collect_ffi_from_file_stems(ffi_src: pathlib.Path) -> set[str]:
    """C ABI symbol stems whose `thetadatadx_<stem>_connect_from_file` extern "C"
    symbol exists in `thetadatadx-ffi/src/`.

    Returns the bare stems (`client` / `historical` / `streaming`); the
    checker maps each parity class name to its stem via
    `FROM_FILE_FFI_STEMS`.
    """
    out: set[str] = set()
    if not ffi_src.is_dir():
        return out
    fn_re = re.compile(r"\bfn\s+thetadatadx_(\w+)_connect_from_file\s*\(")
    for rs in ffi_src.rglob("*.rs"):
        text = _read_source(rs)
        for m in fn_re.finditer(text):
            out.add(m.group(1))
    return out


def _check_from_file_rows(
    rows: list[dict[str, Any]],
    py_from_file: set[str],
    ts_from_file: set[str],
    cpp_from_file: set[str],
    ffi_stems: set[str],
) -> list[str]:
    """Per-client-class cross-binding gate for `[[from_file]]` rows.

    Each row declares a client class `name` plus the expected
    file-construction presence in Python / TypeScript / C++ / the C ABI.
    The checker verifies the actual binding state against the declared
    state and returns a list of mismatch strings (empty when every row
    matches).

    Beyond the per-row check, the collected sets are reconciled against
    the union of declared row names: a client that exposes
    file-construction on ANY binding but has no row at all trips the
    gate, so a newly-added `from_file` cannot slip in untracked.
    """
    errors: list[str] = []
    declared_names = {row.get("name") for row in rows if row.get("name")}
    for row in rows:
        name = row.get("name")
        if not name:
            errors.append(f"  [[from_file]] row missing `name`: {row!r}")
            continue
        stem = FROM_FILE_FFI_STEMS.get(name)
        if stem is None:
            errors.append(
                f"  [[from_file]] row `{name}` has no C ABI stem mapping; "
                f"add it to FROM_FILE_FFI_STEMS."
            )
            continue
        ffi_present = stem in ffi_stems
        for lang, actual, hint in (
            ("python", name in py_from_file, f"`from_file` staticmethod on the `{name}` pyclass"),
            (
                "typescript",
                name in ts_from_file,
                f"`connectFromFile` napi factory on the `{name}` class",
            ),
            ("cpp", name in cpp_from_file, f"`from_file(` static member on the `{name}` C++ class"),
            ("ffi", ffi_present, f"`thetadatadx_{stem}_connect_from_file` extern \"C\" symbol"),
        ):
            declared = row.get(lang, False)
            if declared != actual:
                verb = "missing" if declared and not actual else "unexpected"
                errors.append(
                    f"  {name}.{lang}: declared={declared}, actual={actual} "
                    f"({verb} -- expected {hint})"
                )
    # Reverse-direction orphan check: any client exposing file
    # construction on a binding but lacking a row is undocumented drift.
    ffi_classes = {
        cls for cls, stem in FROM_FILE_FFI_STEMS.items() if stem in ffi_stems
    }
    seen = py_from_file | ts_from_file | cpp_from_file | ffi_classes
    for client in sorted(seen - declared_names):
        on = sorted(
            lang
            for lang, s in (
                ("python", py_from_file),
                ("typescript", ts_from_file),
                ("cpp", cpp_from_file),
                ("ffi", ffi_classes),
            )
            if client in s
        )
        errors.append(
            f"  {client}: exposes file construction on {on} but has no "
            f"[[from_file]] row. Add one declaring its per-binding "
            f"presence so the surface stays tracked."
        )
    return errors


# ─── Client construction (connect) surface ([[connect]]) ──────────────
#
# The base construction entry point: connect to the servers with an
# in-memory `Credentials` + `Config`. The `[[from_file]]` family above
# pins the file-loading convenience; this family pins the construction
# call itself, which `[[method]]` rows cannot express because its
# spelling differs per binding:
#   * Python — a `#[new]` constructor (`Client(creds, config)` /
#     `HistoricalClient(...)` / `StreamingClient(...)`, plus the
#     Python-only `AsyncClient(...)`). The method collector filters
#     `new`/`__new__`, so the constructor needs its own detector.
#   * TypeScript — a `connect` static factory (`Client.connect(...)`,
#     async-shaped: it returns the connected client).
#   * C++ — a `connect` static member (`Client::connect(...)`).
#   * C ABI — a `thetadatadx_<stem>_connect` extern "C" symbol.
#
# The governed roster is the standalone clients reachable over a
# `thetadatadx_<stem>_connect` C symbol. `AsyncClient` is Python-only
# (it wraps `Client`); it has no C ABI / TS / C++ twin, so its row
# carries no stem and is gated on Python alone.

# Parity class name → C ABI symbol stem for the `thetadatadx_<stem>_connect`
# extern "C" symbol. `AsyncClient` has no stem (Python-only); it is held
# in the governed roster separately so its constructor is still scanned.
CONNECT_FFI_STEMS: dict[str, str] = {
    "Client": "client",
    "HistoricalClient": "historical",
    "StreamingClient": "streaming",
}

# Python-only governed client(s) with no C ABI stem. Tracked so the
# constructor orphan scan still enrols them, but gated on Python alone.
CONNECT_PY_ONLY: frozenset[str] = frozenset({"AsyncClient"})

# Full governed roster (C-ABI-backed clients + Python-only twins).
_CONNECT_GOVERNED: frozenset[str] = frozenset(CONNECT_FFI_STEMS) | CONNECT_PY_ONLY


def _collect_python_connect_classes(py_src: pathlib.Path) -> set[str]:
    """Governed client classes whose Python pyclass exposes a `#[new]`
    constructor.

    The cross-binding method collector filters `new`/`__new__`, so the
    construction entry point needs its own scan. Walks every `impl <Path>`
    block, and if its body carries a `#[new]` attribute records the bare
    last-segment class name. Scoped to `_CONNECT_GOVERNED` so a `#[new]`
    on `Credentials` / `Config` / a tick struct is not mistaken for a
    client constructor.
    """
    out: set[str] = set()
    if not py_src.is_dir():
        return out
    impl_re = re.compile(
        r"impl\s+(?:[A-Za-z_][A-Za-z0-9_]*::)*([A-Za-z_][A-Za-z0-9_]*)\s*\{"
    )
    for rs in py_src.rglob("*.rs"):
        text = _read_source(rs)
        for header in impl_re.finditer(text):
            class_name = header.group(1)
            if class_name not in _CONNECT_GOVERNED:
                continue
            body = _balanced_body(text, header.end())
            if "#[new]" in body:
                out.add(class_name)
    return out


def _collect_typescript_connect_classes(ts_methods: dict[str, set[str]]) -> set[str]:
    """Governed client classes whose TypeScript napi class exposes a
    `connect` static factory.

    Reuses the collected `{class: {method, ...}}` map; the napi factory's
    name is the bare `connect`. Scoped to the C-ABI-backed roster (the
    Python-only `AsyncClient` is never a TS class).
    """
    return {
        cls
        for cls, methods in ts_methods.items()
        if cls in CONNECT_FFI_STEMS and "connect" in methods
    }


def _collect_cpp_connect_classes(cpp_methods: dict[str, set[str]]) -> set[str]:
    """Governed client classes whose C++ wrapper exposes a `connect`
    static member.

    Folds the C++ alias names back to the cross-binding identifier and
    scopes to the C-ABI-backed roster.
    """
    reverse_alias = {v: k for k, v in CPP_ALIASES.items()}
    out: set[str] = set()
    for cls, methods in cpp_methods.items():
        if "connect" in methods:
            canonical = reverse_alias.get(cls, cls)
            if canonical in CONNECT_FFI_STEMS:
                out.add(canonical)
    return out


def _collect_ffi_connect_stems(ffi_src: pathlib.Path) -> set[str]:
    """C ABI symbol stems whose `thetadatadx_<stem>_connect` extern "C"
    symbol exists in `thetadatadx-ffi/src/`.

    The `_connect_from_file` convenience shares the `_connect` prefix, so
    the regex anchors on a `(` immediately after `_connect` to match only
    the base construction symbol, not `_connect_from_file`.
    """
    out: set[str] = set()
    if not ffi_src.is_dir():
        return out
    fn_re = re.compile(r"\bfn\s+thetadatadx_(\w+?)_connect\s*\(")
    for rs in ffi_src.rglob("*.rs"):
        text = _read_source(rs)
        for m in fn_re.finditer(text):
            out.add(m.group(1))
    return out


def _check_connect_rows(
    rows: list[dict[str, Any]],
    py_connect: set[str],
    ts_connect: set[str],
    cpp_connect: set[str],
    ffi_stems: set[str],
) -> list[str]:
    """Per-client-class cross-binding gate for `[[connect]]` rows.

    Each row declares a client class `name` plus the expected construction
    presence in Python / TypeScript / C++ / the C ABI. A Python-only
    governed client (`AsyncClient`) carries no C ABI stem; its `ffi`
    column must be `false` and is gated on Python alone. The collected
    sets are reconciled against the declared row names so a client that
    constructs on ANY binding without a row trips the gate.
    """
    errors: list[str] = []
    declared_names = {row.get("name") for row in rows if row.get("name")}
    for row in rows:
        name = row.get("name")
        if not name:
            errors.append(f"  [[connect]] row missing `name`: {row!r}")
            continue
        if name not in _CONNECT_GOVERNED:
            errors.append(
                f"  [[connect]] row `{name}` is not a governed client; "
                f"add it to CONNECT_FFI_STEMS (C-ABI-backed) or "
                f"CONNECT_PY_ONLY (Python-only)."
            )
            continue
        stem = CONNECT_FFI_STEMS.get(name)
        ffi_present = stem in ffi_stems if stem is not None else False
        checks = [
            ("python", name in py_connect, f"`#[new]` constructor on the `{name}` pyclass"),
            (
                "typescript",
                name in ts_connect,
                f"`connect` static factory on the `{name}` napi class",
            ),
            ("cpp", name in cpp_connect, f"`connect(` static member on the `{name}` C++ class"),
            (
                "ffi",
                ffi_present,
                f"`thetadatadx_{stem}_connect` extern \"C\" symbol"
                if stem is not None
                else "no C ABI stem (Python-only client)",
            ),
        ]
        for lang, actual, hint in checks:
            declared = row.get(lang, False)
            if declared != actual:
                verb = "missing" if declared and not actual else "unexpected"
                errors.append(
                    f"  {name}.{lang}: declared={declared}, actual={actual} "
                    f"({verb} -- expected {hint})"
                )
    # Reverse-direction orphan check: any client constructing on a binding
    # but lacking a row is undocumented drift.
    ffi_classes = {
        cls for cls, stem in CONNECT_FFI_STEMS.items() if stem in ffi_stems
    }
    seen = py_connect | ts_connect | cpp_connect | ffi_classes
    for client in sorted(seen - declared_names):
        on = sorted(
            lang
            for lang, s in (
                ("python", py_connect),
                ("typescript", ts_connect),
                ("cpp", cpp_connect),
                ("ffi", ffi_classes),
            )
            if client in s
        )
        errors.append(
            f"  {client}: constructs a connected client on {on} but has no "
            f"[[connect]] row. Add one declaring its per-binding presence "
            f"so the construction surface stays tracked."
        )
    return errors


# ─── Credentials factory surface (reverse-orphan over `Credentials`) ──
#
# The `Credentials` class is the single auth handle every binding builds
# and hands to a connect call. Its factories (`fromFile`, `fromEmail`,
# `fromApiKey`, `fromApiKeyWithEmail`, `fromEnvOrFile`, `fromDotenv`) ride `[[method]]`
# rows, but a forward `[[method]]` check only fires when a row already
# exists — a binding that grows a NEW credentials factory the others lack
# reaches none of them and no row is there to trip. This family closes
# that blind spot at its source: it harvests every `Credentials` factory
# from each binding, folds the per-binding spelling to the canonical
# cross-binding name the rows use, and trips when a harvested factory has
# no `[[method]]` row OR when a known factory is absent from a binding the
# roster says should carry it.
#
# Per-binding factory spelling → canonical cross-binding name:
#   * Python  — `from_env_or_file` snake_case staticmethod  → `fromEnvOrFile`
#   * TS      — `fromEnvOrFile` napi factory (already camel) → `fromEnvOrFile`
#   * C++     — `from_env_or_file` snake_case static member  → `fromEnvOrFile`
#   * C ABI   — `thetadatadx_credentials_from_env_or_file`   → `fromEnvOrFile`
#
# `fromEmail` is C++/C-ABI only: Python and TypeScript build email +
# password credentials through the class constructor, not a factory, so
# the roster records that asymmetry rather than tripping on it.

# Canonical cross-binding name → the bindings that MUST expose the
# factory. The constructor-only email path (`fromEmail`) is C++/C-ABI
# only; every other factory is four-way. This roster is the governed set:
# a harvested factory outside it, or a roster member missing from a
# binding it lists, trips the gate.
CREDENTIALS_FACTORY_ROSTER: dict[str, frozenset[str]] = {
    "fromFile": frozenset({"python", "typescript", "cpp", "ffi"}),
    "fromEmail": frozenset({"cpp", "ffi"}),
    "fromApiKey": frozenset({"python", "typescript", "cpp", "ffi"}),
    "fromApiKeyWithEmail": frozenset({"python", "typescript", "cpp", "ffi"}),
    "fromEnvOrFile": frozenset({"python", "typescript", "cpp", "ffi"}),
    "fromDotenv": frozenset({"python", "typescript", "cpp", "ffi"}),
}


def _credentials_factory_camel(snake: str) -> str:
    """Fold a snake_case `Credentials` factory name (Python staticmethod /
    C++ static member / the `from_*` tail of a C ABI symbol) to the
    camelCase cross-binding name the `[[method]]` rows use."""
    head, *rest = snake.split("_")
    return head + "".join(part[:1].upper() + part[1:] for part in rest)


# `Credentials` members that are not factories: the constructor, the
# borrow accessor + its backing handle, the email getter, and the
# redaction hooks. Harvested by the generic method collectors but never a
# cross-binding factory, so they are exempt before the camelCase fold.
# Names are the raw per-binding spellings (TS `new` / `toString`; C++
# `email` / `get` / `handle_`; Python dunders).
CREDENTIALS_NON_FACTORY_MEMBERS: frozenset[str] = frozenset(
    {
        "new",
        "get",
        "handle_",
        "email",
        "to_string",
        "toString",
        "__repr__",
        "__str__",
    }
)


def _collect_python_credentials_factories(py_methods: dict[str, set[str]]) -> set[str]:
    """Canonical cross-binding names of every `Credentials` factory the
    Python pyclass exposes (snake_case staticmethods folded to camelCase).
    """
    members = py_methods.get("Credentials", set())
    return {
        _credentials_factory_camel(m)
        for m in members
        if m not in CREDENTIALS_NON_FACTORY_MEMBERS
    }


def _collect_typescript_credentials_factories(ts_methods: dict[str, set[str]]) -> set[str]:
    """Canonical cross-binding names of every `Credentials` factory the
    TypeScript napi class exposes (the `js_name` is already camelCase)."""
    members = ts_methods.get("Credentials", set())
    return {m for m in members if m not in CREDENTIALS_NON_FACTORY_MEMBERS}


def _collect_cpp_credentials_factories(cpp_methods: dict[str, set[str]]) -> set[str]:
    """Canonical cross-binding names of every `Credentials` factory the
    C++ wrapper exposes (snake_case static members folded to camelCase,
    inlining the generator-emitted `*.inc` members)."""
    reverse_alias = {v: k for k, v in CPP_ALIASES.items()}
    out: set[str] = set()
    for cls, members in cpp_methods.items():
        if reverse_alias.get(cls, cls) != "Credentials":
            continue
        out |= {
            _credentials_factory_camel(m)
            for m in members
            if m not in CREDENTIALS_NON_FACTORY_MEMBERS
        }
    return out


def _collect_ffi_credentials_factories(ffi_src: pathlib.Path) -> set[str]:
    """Canonical cross-binding names of every `Credentials` factory the C
    ABI exposes (`thetadatadx_credentials_<tail>` extern "C" symbols,
    excluding the `free` lifecycle hook)."""
    out: set[str] = set()
    if not ffi_src.is_dir():
        return out
    fn_re = re.compile(r"\bfn\s+thetadatadx_credentials_(\w+)\s*\(")
    for rs in ffi_src.rglob("*.rs"):
        text = _read_source(rs)
        for m in fn_re.finditer(text):
            tail = m.group(1)
            if tail == "free":
                continue
            out.add(_credentials_factory_camel(tail))
    return out


def _check_credentials_factory_rows(
    rows: list[dict[str, Any]],
    py_factories: set[str],
    ts_factories: set[str],
    cpp_factories: set[str],
    ffi_factories: set[str],
) -> list[str]:
    """Cross-binding gate for the `Credentials` factory surface.

    `rows` is the `[[method]]` roster filtered to `class = "Credentials"`.
    The check is two-directional:

      * Reverse-orphan — every `Credentials` factory harvested from ANY
        binding must carry a `[[method]]` row. A binding that grows a new
        factory the matrix does not track trips the gate even when nobody
        adds the row, closing the blind spot a forward-only check leaves.

      * Roster completeness — every factory in
        `CREDENTIALS_FACTORY_ROSTER` must be present on each binding the
        roster lists, so an asymmetric drop (a factory removed from one
        binding) trips even if its row is also (wrongly) deleted.

    Returns a list of mismatch strings (empty when the surface is
    symmetric and fully tracked).
    """
    errors: list[str] = []
    declared_names = {
        row.get("name")
        for row in rows
        if row.get("class") == "Credentials" and row.get("name")
    }
    per_binding: dict[str, set[str]] = {
        "python": py_factories,
        "typescript": ts_factories,
        "cpp": cpp_factories,
        "ffi": ffi_factories,
    }

    # Reverse-orphan: a harvested factory with no [[method]] row.
    seen = py_factories | ts_factories | cpp_factories | ffi_factories
    for factory in sorted(seen - declared_names):
        on = sorted(lang for lang, s in per_binding.items() if factory in s)
        errors.append(
            f"  Credentials.{factory}: factory present on {on} but has no "
            f"[[method]] row (class = \"Credentials\"). Add one declaring "
            f"its per-binding presence so the auth surface stays tracked."
        )

    # Roster completeness: a governed factory missing from a binding the
    # roster says must carry it.
    for factory, required in sorted(CREDENTIALS_FACTORY_ROSTER.items()):
        for lang in sorted(required):
            if factory not in per_binding[lang]:
                errors.append(
                    f"  Credentials.{factory}: governed factory missing from "
                    f"the {lang} binding (roster requires {sorted(required)}). "
                    f"Add it so the auth surface stays symmetric."
                )
    return errors


# ─── Main gate ──────────────────────────────────────────────────────


# Recognized suffixes for a dotted "documentation anchor" row — a row
# whose struct prefix is NOT a field-bearing config struct
# (`STRUCT_TO_PREFIX`). These rows record a cross-binding fact about a
# whole class (a tick wrapper tracked by the `*Tick` catch-all, or a known
# name divergence) rather than a per-field setter. The suffix must be one
# of these AND the struct part must name a real binding class, or the row
# is treated as a typo and fails the gate.
ANCHOR_ROW_SUFFIXES: frozenset[str] = frozenset(
    {
        "cross_binding_anchor",
        "cross_binding_name_divergence",
    }
)


def _check_dotted_rows(
    rows: list[dict[str, Any]],
    py_setters: set[str],
    ts_setters: set[str],
    cpp_setters: set[str],
    ffi_setters: set[str],
    anchor_classes: set[str] | None = None,
) -> list[str]:
    """Per-field / per-setter granularity (issue #595).

    Returns a list of human-readable error strings. An empty list
    means every dotted row in `parity.toml` matches the actual binding
    state of each SDK.

    `anchor_classes` is the universe of real binding class names (every
    pyclass / TS class / C++ class plus the implicitly-tracked tick
    wrappers). When provided, a dotted row whose struct prefix is not a
    field-bearing config struct is validated as a documentation-anchor row:
    its suffix must be in `ANCHOR_ROW_SUFFIXES` AND its struct part must
    name a class in `anchor_classes`. A row that satisfies neither is a
    probable typo (a misspelled struct, a stray dotted name) and fails the
    gate, closing the silent-skip blind spot. When `anchor_classes` is
    `None` (the selftest call shape), anchor rows are skipped as before so
    synthetic per-field matrices stay hermetic.
    """
    errors: list[str] = []
    for row in rows:
        name = row["name"]
        if "." not in name:
            continue
        struct_name, suffix = name.split(".", 1)
        prefix = STRUCT_TO_PREFIX.get(struct_name)
        if prefix is None:
            # Not a field-bearing config struct — this must be a
            # documentation-anchor row (a whole-class cross-binding fact,
            # e.g. `GreeksEodTick.cross_binding_anchor`). When the class
            # universe is known, validate it so a typo cannot slip through
            # silently; otherwise (selftest) skip as a non-field row.
            if anchor_classes is None:
                continue
            if suffix not in ANCHOR_ROW_SUFFIXES:
                errors.append(
                    f"  {name}: dotted row on non-config struct uses an "
                    f"unrecognized suffix `{suffix}`. A documentation-anchor "
                    f"row must use one of {sorted(ANCHOR_ROW_SUFFIXES)}; a "
                    f"field-level row must name a struct in STRUCT_TO_PREFIX. "
                    f"This is almost certainly a typo."
                )
                continue
            if struct_name not in anchor_classes:
                errors.append(
                    f"  {name}: anchor row names struct `{struct_name}`, which "
                    f"is not a known binding class. Fix the spelling or remove "
                    f"the row — a typo'd anchor silently asserts nothing."
                )
            continue
        # Allow rows to override the auto-derived setter name. Used
        # when a single struct has a mix of prefixed / unprefixed
        # binding-side names (e.g. `HistoricalConfig.host` binds as
        # `historical_host` because the bare `host` name would collide with
        # nothing meaningful and the `historical_` prefix clarifies intent).
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
                    f"FFI symbol `thetadatadx_config_set_{canonical}` is "
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


VALUE_FIELD_PY_SRC = REPO_ROOT / "thetadatadx-py" / "src"
VALUE_FIELD_TS_SRC = REPO_ROOT / "thetadatadx-ts" / "src"


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
        text = _read_source(path)
        for m in struct_re.finditer(text):
            fm = field_re.search(m.group(1))
            if fm:
                return fm.group(1).strip()
    return None


def _cpp_struct_field_type(hpp: pathlib.Path, struct: str, field: str) -> str | None:
    """Declared C++ type of `field` on `struct` in the C++ wrapper header.

    Mirrors [`_struct_field_type`] for the hand-written C++ value structs
    (`OptionContract`, etc.) whose field types live in `thetadatadx.hpp`
    rather than a Rust binding crate. Returns `None` when the struct or
    field is absent. A `cpp` key on a `[[value_field]]` row pins the
    type this returns, closing the gap that let a C++ value struct
    surface a raw wire integer the other bindings decode.
    """
    text = _read_source(hpp)
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


# The generated C-ABI header fragment carrying the `#[repr(C)]` streaming
# value structs. Its types are declared in the `typedef struct { ... }
# <Name>;` C idiom, NOT the `struct <Name> { ... }` C++ idiom the wrapper
# header uses, so it needs its own reader. The streaming contract payload
# (`ThetaDataDxContract`) lives here and carries the unit-bearing wire
# fields (`strike` dollars + `strike_thousandths` the raw integer).
CPP_C_STRUCT_INC = REPO_ROOT / "thetadatadx-cpp" / "include" / "fpss_event_structs.h.inc"

# Parity-toml value `class` → the generated C-ABI struct that backs it,
# for fields whose C-ABI shape is the source of truth rather than a C++
# wrapper struct. The streaming contract payload is the cross-binding
# `ContractRef` (Python) / `Contract` (TypeScript); both decode the same
# `#[repr(C)] ThetaDataDxContract` on the C side.
VALUE_FIELD_C_STRUCT_ALIASES: dict[str, str] = {
    "ContractRef": "ThetaDataDxContract",
    "Contract": "ThetaDataDxContract",
}


def _c_abi_struct_field_type(inc: pathlib.Path, struct: str, field: str) -> str | None:
    """Declared C type of `field` on a `typedef struct { ... } <struct>;`
    in the generated C-ABI header fragment.

    The streaming value structs are emitted in the C `typedef struct`
    idiom (the type name follows the closing brace), which
    `_cpp_struct_field_type`'s `struct <Name> {` regex cannot read. This
    reader matches the C form so a `cpp` key on a `[[value_field]]` row
    whose class is C-ABI-backed (`ContractRef` / `Contract`) is validated
    against the actual generated C struct member type — closing the gap
    that let the C side silently skip a unit-bearing field the other
    bindings pin. Returns `None` when the struct or field is absent.
    """
    if not inc.is_file():
        return None
    text = _read_source(inc)
    struct_re = re.compile(
        r"typedef\s+struct\s*\{(.*?)\}\s*" + re.escape(struct) + r"\s*;",
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
        # header, not a Rust crate, so they get their own reader. A class
        # whose C-side shape is the generated `#[repr(C)]` struct (the
        # streaming contract payload) is read from the C-ABI header
        # fragment instead — the `typedef struct { ... } <Name>;` idiom the
        # C++-struct reader cannot parse.
        declared_cpp = row.get("cpp")
        if declared_cpp is not None:
            c_struct = VALUE_FIELD_C_STRUCT_ALIASES.get(cls)
            if c_struct is not None:
                actual_cpp = _c_abi_struct_field_type(CPP_C_STRUCT_INC, c_struct, field)
            else:
                actual_cpp = _cpp_struct_field_type(CPP_HPP, _cpp_class_for(cls), field)
            if actual_cpp != declared_cpp:
                errors.append(
                    f"{cls}.{field}.cpp: declared type `{declared_cpp}`, "
                    f"actual `{actual_cpp or '<field missing>'}`"
                )
    return errors


# ─── Value-field exhaustiveness (reverse-direction roster) ────────────
#
# The per-row `_check_value_field_rows` gate verifies each DECLARED field
# resolves to the right type. It does not gate the reverse direction: a
# unit- or identity-bearing field that lands on a binding value struct
# without a `[[value_field]]` row trips nothing, because the row simply
# does not exist (the exact `strike_thousandths` defect — the wire integer
# shipped on the streaming payloads with zero matrix rows). Two
# complementary scans close that blind spot without pinning every trivial
# column:
#
#  1. A roster check: the load-bearing value classes each carry a fixed
#     set of unit/identity-bearing fields that MUST be pinned. A missing
#     entry trips, so dropping a row for one of these protected surfaces
#     fails the gate.
#  2. A vocabulary scan: any binding struct field whose NAME matches the
#     unit/identity grammar (`strike` / `strike_thousandths` / `right` /
#     `*_timestamp_ms`) on a class already in the matrix, but which has no
#     `[[value_field]]` row under that name on ANY class, trips. A brand-new
#     unit-bearing field name (a future `settlement_timestamp_ms`, a
#     re-typed `strike_micros`) cannot ship untracked.
#
# Both scans are intentionally scoped to the unit/identity-bearing surface
# the matrix exists to protect — the flat data columns (`price`, `size`,
# the Greeks scalars) are projected from one schema source and stay in
# lockstep by construction, so they are out of scope here.

# Load-bearing value classes → the unit/identity-bearing fields that must
# carry a `[[value_field]]` row. Keyed by the cross-binding class name the
# matrix uses (`ContractRef` is the Python streaming payload, `Contract`
# the TypeScript one). Each listed `(class, field)` must appear in the
# matrix or the roster check trips.
VALUE_FIELD_ROSTER: dict[str, tuple[str, ...]] = {
    "ContractRef": ("strike", "strike_thousandths", "right"),
    "Contract": ("strike", "strike_thousandths", "right"),
    "OptionContract": ("right",),
    "TradeTick": ("strike", "right"),
    "EodTick": ("created_ms_of_day", "last_trade_ms_of_day"),
}

# The unit/identity-bearing field-name grammar the vocabulary scan
# protects. A field whose name matches this — and which has no
# `[[value_field]]` row under that name anywhere — is a new unit-bearing
# surface that must be enrolled.
_VALUE_FIELD_UNIT_NAME_RE = re.compile(
    r"^(?:strike|strike_thousandths|right)$|_timestamp_ms$|^timestamp_ms$"
)

# Field-name aliases between the Python source spelling and the matrix /
# TypeScript spelling. The Python keyword escape `lambda_` is the matrix
# `lambda` on the TS side; neither is unit-bearing, so this is only here
# to document the one cross-binding rename the scans must treat as equal
# should the grammar ever widen to cover it.
_VALUE_FIELD_NAME_ALIASES: dict[str, str] = {"lambda_": "lambda"}


def _collect_value_struct_fields(
    src_dir: pathlib.Path, struct: str
) -> set[str]:
    """Return the `pub` field names of `struct` across a binding crate.

    Mirrors `_struct_field_type`'s struct-body scan but harvests every
    field name rather than one field's type. Used by the reverse
    exhaustiveness scan to enumerate a value struct's actual surface.
    """
    out: set[str] = set()
    if not src_dir.is_dir():
        return out
    struct_re = re.compile(
        r"(?:pub(?:\(crate\))?\s+)?struct\s+" + re.escape(struct) + r"\s*\{(.*?)\n\}",
        re.S,
    )
    field_re = re.compile(r"(?:#\[[^\]]*\]\s*)*pub\s+(\w+)\s*:", re.M)
    for path in sorted(src_dir.rglob("*.rs")):
        text = _read_source(path)
        for m in struct_re.finditer(text):
            for fm in field_re.finditer(m.group(1)):
                out.add(fm.group(1))
    return out


def _check_value_field_roster(rows: list[dict[str, Any]]) -> list[str]:
    """Reverse-direction exhaustiveness for the `[[value_field]]` matrix.

    Two scans (see the section header):

      * roster — every `(class, field)` in `VALUE_FIELD_ROSTER` must have a
        matrix row;
      * vocabulary — every binding struct field matching the unit/identity
        grammar on a matrix class must have a row under that name somewhere.

    Returns human-readable error strings (empty when complete).
    """
    errors: list[str] = []
    present_pairs: set[tuple[str, str]] = {
        (row["class"], row["name"]) for row in rows
    }
    present_names: set[str] = {row["name"] for row in rows}

    for cls, fields in VALUE_FIELD_ROSTER.items():
        for field in fields:
            if (cls, field) not in present_pairs:
                errors.append(
                    f"  {cls}.{field}: load-bearing unit/identity field has "
                    f"no `[[value_field]]` row. Add one pinning its type on "
                    f"the bindings that carry it (the matrix exists to keep "
                    f"this surface honest)."
                )

    # Vocabulary scan: every class already in the matrix, every binding
    # struct field whose name matches the unit grammar, must have a row
    # under that name on some class.
    matrix_classes = sorted({row["class"] for row in rows})
    flagged: set[tuple[str, str]] = set()
    for cls in matrix_classes:
        for src_dir in (VALUE_FIELD_PY_SRC, VALUE_FIELD_TS_SRC):
            for field in _collect_value_struct_fields(src_dir, cls):
                canonical = _VALUE_FIELD_NAME_ALIASES.get(field, field)
                if not _VALUE_FIELD_UNIT_NAME_RE.search(canonical):
                    continue
                if canonical in present_names or field in present_names:
                    continue
                if (cls, field) in flagged:
                    continue
                flagged.add((cls, field))
                errors.append(
                    f"  {cls}.{field}: unit/identity-bearing field present on "
                    f"a binding value struct but no `[[value_field]]` row pins "
                    f"the name anywhere. Enroll it so its type cannot drift "
                    f"silently across bindings."
                )
    return errors


# ─── C-ABI symbol roster + reverse-orphan scan ────────────────────────
#
# The `[[method]]` / endpoint / config / utility families each gate one
# slice of the C ABI by SHAPE. None of them tracks the streaming-batch /
# borrowed-handle externs (the columnar reader, the flat-file Arrow bridge,
# the historical sub-handle), and nothing asserts the reverse direction:
# that EVERY `extern "C"` symbol belongs to some enrolled family. A new
# C-ABI symbol could ship — breaking the ABI contract the C++ wrappers and
# external FFI consumers depend on — with no row anywhere. Two checks close
# this: `[[ffi_symbol]]` rows pin the streaming-batch family by name, and
# the orphan scan subtracts every enrolled family + the memory-management
# exempt roster from the harvested universe and flags the remainder.

# Memory-management frees and panic-test hooks: C-ABI symbols that release
# an owned allocation (`*_free`) or exist only to exercise the panic boundary
# in tests. They carry no cross-binding method contract, so they are exempt
# from the FFI-symbol orphan scan. Bare names with the `thetadatadx_` prefix
# stripped.
#
# The `*_tick_array_free` / `*_array_free` block is the per-tick deallocator
# the `tick_array_free!` macro emits in `thetadatadx-ffi/src/types.rs` (one per tick
# wrapper, plus the `calendar_day_array_free` calendar variant). The symbol
# name is the macro's first argument, so the orphan scan only sees these once
# `_collect_ffi_all_symbols` harvests macro-invocation sites; each is a pure
# deallocator paired with a `*_ticks_to_arrow_ipc` enrolled `[[ffi_symbol]]`.
_FFI_TICK_ARRAY_FREES: frozenset[str] = frozenset(
    {
        "eod_tick_array_free",
        "ohlc_tick_array_free",
        "trade_tick_array_free",
        "quote_tick_array_free",
        "greeks_all_tick_array_free",
        "greeks_eod_tick_array_free",
        "greeks_first_order_tick_array_free",
        "greeks_second_order_tick_array_free",
        "greeks_third_order_tick_array_free",
        "trade_greeks_all_tick_array_free",
        "trade_greeks_first_order_tick_array_free",
        "trade_greeks_second_order_tick_array_free",
        "trade_greeks_third_order_tick_array_free",
        "trade_greeks_implied_volatility_tick_array_free",
        "iv_tick_array_free",
        "price_tick_array_free",
        "index_price_at_time_tick_array_free",
        "open_interest_tick_array_free",
        "market_value_tick_array_free",
        "calendar_day_array_free",
        "interest_rate_tick_array_free",
        "trade_quote_tick_array_free",
    }
)
FFI_SYMBOL_EXEMPT: frozenset[str] = frozenset(
    {
        "arrow_bytes_free",
        "string_free",
        "string_array_free",
        "subscription_array_free",
        "option_contract_array_free",
        "greeks_result_free",
        "flatfile_bytes_free",
        "flatfile_rowlist_free",
        "credentials_free",
        "historical_free",
        "config_free",
        "test_panic_str",
        "test_panic_string",
    }
    | _FFI_TICK_ARRAY_FREES
)

# The client / streaming observability + lifecycle roster: the C-ABI
# realization of the `StreamView` / `StreamingClient` surface, each
# independently enrolled and gated by the `[[method]]`, core-streaming, and
# connect families. Enumerated explicitly (not by a broad `client_*` /
# `streaming_*` prefix) so a NEW `thetadatadx_client_*` / `thetadatadx_streaming_*`
# symbol that nobody enrolled still falls through to the orphan scan. Bare
# `<stem>_<suffix>` names (prefix stripped); `_OBS_SUFFIXES` is shared by
# both the `client` and `streaming` stems.
_FFI_OBS_SUFFIXES: frozenset[str] = frozenset(
    {
        "active_subscriptions",
        "active_full_subscriptions",
        "await_drain",
        "dropped_events",
        "free",
        "is_authenticated",
        "is_streaming",
        "last_connected_addr",
        "last_event_received_at_unix_nanos",
        "millis_since_last_event",
        "panic_count",
        "reconnect",
        "ring_capacity",
        "ring_occupancy",
        "set_callback",
        "subscribe",
        "unsubscribe",
    }
)
_FFI_OBS_SYMBOLS: frozenset[str] = frozenset(
    f"{stem}_{suf}" for stem in ("client", "streaming") for suf in _FFI_OBS_SUFFIXES
) | frozenset(
    {
        "client_stop_streaming",
        "client_historical",
        "streaming_shutdown",
    }
)

# Config lifecycle symbols outside the `config_set_` / `config_get_` /
# `config_with_` shapes (the ctors / environment selectors / free).
_FFI_CONFIG_MISC: frozenset[str] = frozenset(
    {"config_dev", "config_stage", "config_production", "config_from_dotenv"}
)

# Client / streaming / historical construction symbols (the `[[connect]]` /
# `[[from_file]]` families).
_FFI_CONNECT_SYMBOLS: frozenset[str] = frozenset(
    f"{stem}_{suf}"
    for stem in ("client", "streaming", "historical")
    for suf in ("connect", "connect_from_file")
)

# The error-surface symbols (the `[[error]]` leaf/code family threads the
# higher bindings off these).
_FFI_ERROR_SYMBOLS: frozenset[str] = frozenset(
    {"last_error", "last_error_code", "last_error_retry_after_ms", "clear_error"}
)

# Standalone utility symbols (the `[[utility]]` family —
# condition/exchange/sequence/calendar lookups, the strike/timestamp
# converters).
_FFI_UTILITY_SYMBOLS: frozenset[str] = frozenset(
    {
        "condition_description",
        "condition_is_cancel",
        "condition_name",
        "condition_updates_volume",
        "quote_condition_description",
        "quote_condition_is_firm",
        "quote_condition_is_halted",
        "quote_condition_name",
        "exchange_name",
        "exchange_symbol",
        "sequence_signed_to_unsigned",
        "sequence_unsigned_to_signed",
        "calendar_status_name",
        "contract_strike_dollars",
        "timestamp_ms",
    }
)

# Flat-file request symbols other than the Arrow-IPC bridge (which is an
# enrolled `[[ffi_symbol]]`): the decoded-rows fetch, the blob-to-disk
# fetch, and the row count — governed by the flat-file fetch / namespace
# families.
_FFI_FLATFILE_SYMBOLS: frozenset[str] = frozenset(
    {
        "flatfile_request_decoded",
        "flatfile_request_to_path",
        "flatfile_rows_count",
    }
)


def _collect_ffi_all_symbols(ffi_src: pathlib.Path) -> set[str]:
    """Every `extern "C" fn thetadatadx_<name>` declared under `thetadatadx-ffi/src/**`,
    as the bare `<name>` (prefix stripped).

    This is the full C-ABI symbol universe the orphan scan reduces against
    the enrolled families. Two harvest shapes:

    * `fn thetadatadx_<name>(` — symbols spelled literally (the bulk).
    * `<macro>!(thetadatadx_<name>, ...)` — symbols whose name is a macro
      ARGUMENT, invisible to the literal-`fn` regex. `thetadatadx-ffi/src/types.rs`
      emits these through `tick_array_free!` (the per-tick `*_array_free`
      deallocators) and `tick_array_to_arrow_ipc!` (the per-tick
      `*_ticks_to_arrow_ipc` columnar terminals the C++
      `tick_arrow_ipc.hpp.inc` calls by name). Both name the extern as the
      macro's first argument, so one regex anchored on that arg position
      harvests every current and future name-as-arg extern emitter.
    """
    out: set[str] = set()
    if not ffi_src.is_dir():
        return out
    sym_re = re.compile(r"\bfn\s+thetadatadx_(\w+)\s*\(")
    macro_sym_re = re.compile(r"\w+!\s*\(\s*thetadatadx_(\w+)\s*,")
    for rs in ffi_src.rglob("*.rs"):
        text = _read_source(rs)
        for m in sym_re.finditer(text):
            out.add(m.group(1))
        for m in macro_sym_re.finditer(text):
            out.add(m.group(1))
    return out


def _ffi_symbol_governed(name: str, enrolled: frozenset[str]) -> bool:
    """True iff the bare C-ABI symbol `name` belongs to an enrolled family.

    `enrolled` is the set of `[[ffi_symbol]]` row names (the streaming-batch
    family). The remaining families are matched by their SSOT-backed shape
    (config / endpoint / credentials) or their explicit roster (observability
    / connect / error / utility / flat-file / config-misc). A symbol matching
    none of these — and not in `FFI_SYMBOL_EXEMPT` — is an orphan.
    """
    if name in enrolled:
        return True
    if name in FFI_SYMBOL_EXEMPT:
        return True
    # SSOT-backed shapes: config accessors, every endpoint's `_with_options`
    # base and `_stream` companion, the credentials factories.
    if (
        name.startswith("config_set_")
        or name.startswith("config_get_")
        or name.startswith("config_with_")
        or name.startswith("credentials_")
        or name.endswith("_with_options")
        or name.endswith("_stream")
    ):
        return True
    return name in (
        _FFI_OBS_SYMBOLS
        | _FFI_CONFIG_MISC
        | _FFI_CONNECT_SYMBOLS
        | _FFI_ERROR_SYMBOLS
        | _FFI_UTILITY_SYMBOLS
        | _FFI_FLATFILE_SYMBOLS
    )


def _check_ffi_symbol_rows(
    ffi_symbol_rows: list[dict[str, Any]],
    all_symbols: set[str],
    ffi_src: pathlib.Path,
) -> list[str]:
    """Forward check for `[[ffi_symbol]]` rows: each declared symbol must
    exist as an `extern "C"` declaration under `thetadatadx-ffi/src/**`.

    A row whose symbol vanished (renamed / removed) trips, so the
    streaming-batch / borrowed-handle ABI cannot silently break the C++
    wrappers that call these symbols by name.

    A row MAY carry an optional `[ffi_symbol.signature]` sub-table pinning the
    C param list + return (opt-in, exactly like `[method.signature]`). When
    present, the extern's declared signature is extracted and compared on the
    `ffi` lang via the shared signature engine — the C ABI is the lowest layer,
    so its opaque handle pointers / owned-struct returns compare by exact
    spelling (see `_sig_type_agrees`). The macro-generated per-tick
    `*_to_arrow_ipc` terminals carry no signature here: they are emitted from
    one `tick_array_to_arrow_ipc!` macro, so their shape is identical by
    construction (the data-plane by-construction guarantee), and the name-only
    row already pins their existence.
    """
    errors: list[str] = []
    for row in ffi_symbol_rows:
        name = row.get("name")
        if not name:
            errors.append(f"  [[ffi_symbol]] row missing `name`: {row!r}")
            continue
        if name not in all_symbols:
            errors.append(
                f"  thetadatadx_{name}: enrolled `[[ffi_symbol]]` row has no "
                f"matching `extern \"C\" fn thetadatadx_{name}` under thetadatadx-ffi/src/. "
                f"Either restore the symbol or drop the row."
            )
            continue
        signature = row.get("signature")
        if signature:
            spec = _sig_spec_for(signature, "ffi")
            if spec is not None:
                errors += _sig_compare_one(
                    f"thetadatadx_{name}", spec, _sig_extract_ffi(ffi_src, name), "ffi"
                )
    return errors


def _check_ffi_symbol_orphans(
    all_symbols: set[str], ffi_symbol_rows: list[dict[str, Any]]
) -> list[str]:
    """Reverse-direction orphan scan over the whole C ABI: every harvested
    `thetadatadx_*` symbol must belong to an enrolled family or be in
    `FFI_SYMBOL_EXEMPT`.

    This is the strongest single guarantee in the matrix — no C-ABI symbol
    ships without enrollment. A genuinely new symbol family (a symbol
    matching no config / endpoint / credentials shape and no explicit
    roster) trips, forcing a `[[ffi_symbol]]` row (or the appropriate
    family) before it can land.
    """
    enrolled = frozenset(
        row["name"] for row in ffi_symbol_rows if row.get("name")
    )
    errors: list[str] = []
    for name in sorted(all_symbols):
        if _ffi_symbol_governed(name, enrolled):
            continue
        errors.append(
            f"  thetadatadx_{name}: C-ABI symbol belongs to no enrolled family. "
            f"Either add a `[[ffi_symbol]]` row (or enroll it in the matching "
            f"endpoint/config/utility family), or add it to FFI_SYMBOL_EXEMPT "
            f"if it is a memory-management free with no cross-binding contract."
        )
    return errors


# ─── Request-options SSOT roster (the C++ + FFI generated consumers) ──
#
# The endpoint request-options surface is generated from
# `endpoint_surface.toml` into two consumers: the C++ fluent `with_*`
# setters (`endpoint_options.hpp.inc`) and the FFI `#[repr(C)]` bridge
# struct `ThetaDataDxEndpointRequestOptions` (with a `has_*` presence flag
# per scalar). Both are emitted from the same option roster, so they must
# carry the same option set; a hand-edit or a generator drift that adds a
# `with_X` without the FFI field (or vice versa), or a scalar field without
# its `has_X` flag, breaks the C++ → C bridge silently. This checks the two
# generated consumers agree, anchored on the SSOT global (`timeout_ms`).
# `_check_request_options_roster` holds the NAME/roster level; the companion
# `_check_request_options_types` adds the per-option TYPE level — every
# option's declared type must agree across the SSOT, the C++ `with_*`
# parameter, and the FFI struct field, via `REQUEST_OPTION_TYPE_MAP`.
# `with_deadline` is a `std::chrono` convenience alias of `timeout_ms`, not
# a distinct option, and is exempt.
REQUEST_OPTIONS_WITH_EXEMPT: frozenset[str] = frozenset({"deadline"})


def _collect_endpoint_request_options(surface_toml: pathlib.Path) -> set[str]:
    """The request-options SSOT anchor: the `[[request_options_global]]`
    names from `endpoint_surface.toml`.

    These cross-cutting options (today: `timeout_ms`) must appear in BOTH
    generated consumers. The full builder-option roster is read from the
    generated `with_*` set directly (the authoritative emitted roster),
    which the cross-consumer equality check then holds the FFI struct to.
    """
    out: set[str] = set()
    if not surface_toml.is_file():
        return out
    data = tomllib.loads(surface_toml.read_text(encoding="utf-8"))
    for opt in data.get("request_options_global", []):
        name = opt.get("name")
        if name:
            out.add(name)
    return out


def _collect_cpp_with_options(hpp_inc: pathlib.Path) -> set[str]:
    """The `with_<name>` setter roster from the generated C++ options header,
    as the bare `<name>` (the `with_` prefix stripped)."""
    if not hpp_inc.is_file():
        return set()
    text = _read_source(hpp_inc)
    return {
        m.group(1)
        for m in re.finditer(r"\bEndpointRequestOptions&\s+with_(\w+)\s*\(", text)
    }


def _collect_ffi_request_option_fields(
    rs: pathlib.Path,
) -> tuple[set[str], set[str]]:
    """The `ThetaDataDxEndpointRequestOptions` struct's option fields and its
    `has_*` presence flags, as two bare-name sets.

    Returns `(option_fields, has_flags)` where `has_flags` are the bare field
    names a `has_<field>` flag exists for. A scalar option is applied through
    its presence flag; a string option uses a null pointer, so it carries no
    `has_`. The caller asserts every scalar field has its flag.
    """
    fields: set[str] = set()
    has_flags: set[str] = set()
    if not rs.is_file():
        return fields, has_flags
    text = _read_source(rs)
    m = re.search(
        r"struct\s+ThetaDataDxEndpointRequestOptions\s*\{(.*?)\n\}", text, re.S
    )
    if not m:
        return fields, has_flags
    for fm in re.finditer(r"\bpub\s+(\w+)\s*:", m.group(1)):
        name = fm.group(1)
        if name.startswith("has_"):
            has_flags.add(name[len("has_") :])
        else:
            fields.add(name)
    return fields, has_flags


def _check_request_options_roster(
    ssot_global: set[str],
    cpp_withs: set[str],
    ffi_fields: set[str],
    ffi_has_flags: set[str],
) -> list[str]:
    """Assert the two generated request-options consumers agree.

    * The C++ `with_*` setter roster (minus the `with_deadline` alias) equals
      the FFI struct's option-field set.
    * Every FFI scalar option field (one carrying a `has_*` flag) round-trips
      — the check enforces presence-flag completeness by requiring each
      `has_<field>` to name a real field.
    * The SSOT global anchor (`timeout_ms`) appears in both consumers AND
      carries its `has_<name>` presence flag in the FFI struct — without the
      flag the scalar value is never applied, so the C++ → C bridge would
      silently drop the option while every roster still matched.
    """
    errors: list[str] = []
    cpp_roster = cpp_withs - REQUEST_OPTIONS_WITH_EXEMPT

    missing_in_ffi = sorted(cpp_roster - ffi_fields)
    for name in missing_in_ffi:
        errors.append(
            f"  request-options `{name}`: C++ `with_{name}` setter exists but "
            f"the FFI `ThetaDataDxEndpointRequestOptions` struct has no `{name}` "
            f"field — the C++ → C bridge would drop the option."
        )
    missing_in_cpp = sorted(ffi_fields - cpp_roster)
    for name in missing_in_cpp:
        errors.append(
            f"  request-options `{name}`: FFI struct field `{name}` has no C++ "
            f"`with_{name}` setter — the option is unreachable from the C++ "
            f"fluent surface."
        )
    for name in sorted(ffi_has_flags - ffi_fields):
        errors.append(
            f"  request-options `{name}`: FFI `has_{name}` presence flag has no "
            f"matching `{name}` option field."
        )
    for name in sorted(ssot_global):
        if name not in cpp_roster:
            errors.append(
                f"  request-options `{name}`: SSOT [[request_options_global]] "
                f"option absent from the C++ `with_*` roster."
            )
        if name not in ffi_fields:
            errors.append(
                f"  request-options `{name}`: SSOT [[request_options_global]] "
                f"option absent from the FFI options struct."
            )
        elif name not in ffi_has_flags:
            # The field exists but its presence flag was dropped: a scalar is
            # applied only when `has_<name> = 1`, so a missing flag makes the
            # value unreachable even though the rosters still match.
            errors.append(
                f"  request-options `{name}`: SSOT [[request_options_global]] "
                f"scalar option has no `has_{name}` presence flag in the FFI "
                f"options struct — the value would never be applied."
            )
    return errors


# ─── Request-options TYPE parity (signature level) ───────────────────
#
# Route A: the request-options surface is SSOT-generated, so its types
# agree by construction today. This makes that machine-enforced — a future
# hand-edit or codegen change that drifts a type (a `with_X` parameter, or
# the FFI field) away from the SSOT `param_type` fails the gate.
#
# The SSOT canonical type is the `param_type` (or the global option's
# `type`). The codegen (`builder_value_type_name` / `ffi_option_value_type`)
# collapses every `param_type` into one of five categories; the map below
# mirrors that exactly, pinning the C++ `with_*` parameter spelling and the
# FFI `#[repr(C)]` field spelling each category MUST take. A `param_type` the
# map does not cover, or an actual type that disagrees, fails with the option
# named — so a drift cannot hide behind a still-matching roster.
#
# `with_deadline` is exempt (the `std::chrono::milliseconds` alias of the
# `timeout_ms` `u64`); the roster check already excludes it.
REQUEST_OPTION_TYPE_MAP: dict[str, tuple[str, str]] = {
    # SSOT param_type → (C++ `with_*` parameter type, FFI field type).
    "Int": ("int32_t", "i32"),
    "Float": ("double", "f64"),
    "Bool": ("bool", "i32"),  # bool is C-unfriendly over FFI; encoded as i32.
    "u64": ("uint64_t", "u64"),  # the `timeout_ms` global's `type`.
}
# Every string-like `param_type` decodes to the same C++/FFI spelling; the
# codegen's catch-all arm. Listed explicitly so an unknown new `param_type`
# fails closed (rather than silently assuming a string) — a numeric option
# mis-tagged string would otherwise slip the gate.
REQUEST_OPTION_STRING_TYPES: frozenset[str] = frozenset(
    {"Str", "Strike", "Right", "Interval", "Date", "Symbol", "Venue", "Version", "RateType"}
)
_REQUEST_OPTION_STRING_SPELLING: tuple[str, str] = ("std::string", "*const c_char")


def _request_option_canonical_types(param_type: str) -> tuple[str, str] | None:
    """`(cpp_with_param_type, ffi_field_type)` the SSOT `param_type` must take,
    or `None` if the `param_type` is outside the known roster."""
    if param_type in REQUEST_OPTION_STRING_TYPES:
        return _REQUEST_OPTION_STRING_SPELLING
    return REQUEST_OPTION_TYPE_MAP.get(param_type)


def _collect_ssot_request_option_types(surface_toml: pathlib.Path) -> dict[str, str]:
    """Map each request-option name to its canonical SSOT `param_type`.

    A request-option is any `binding = "builder"` param (defined in a
    `[param_groups.*]` group or inline on an endpoint) plus every
    `[[request_options_global]]` (whose canonical type is its `type` key).
    Returns `{name: param_type}`. A name appearing under two `param_type`s
    (a real SSOT inconsistency) is reported by the caller via the per-name
    type comparison, so only the first is recorded here.
    """
    out: dict[str, str] = {}
    if not surface_toml.is_file():
        return out
    data = tomllib.loads(surface_toml.read_text(encoding="utf-8"))

    def scan(params: list[dict[str, Any]]) -> None:
        for p in params:
            if "use" in p or p.get("binding") != "builder":
                continue
            name, pt = p.get("name"), p.get("param_type")
            if name and pt:
                out.setdefault(name, pt)

    for group in data.get("param_groups", {}).values():
        scan(group.get("params", []))
    for endpoint in data.get("endpoints", []):
        scan(endpoint.get("params", []))
    for opt in data.get("request_options_global", []):
        name, ty = opt.get("name"), opt.get("type")
        if name and ty:
            out.setdefault(name, ty)
    return out


def _collect_cpp_with_option_types(hpp_inc: pathlib.Path) -> dict[str, str]:
    """Map each `with_<name>` setter to its declared C++ parameter type.

    Mirrors `_collect_cpp_with_options` but captures the single `value`
    parameter's type so it can be compared to the SSOT-implied spelling. The
    `with_deadline` alias (a `std::chrono::milliseconds` parameter) is read
    too; the type check excludes it via the exempt set.
    """
    if not hpp_inc.is_file():
        return {}
    text = _read_source(hpp_inc)
    return {
        m.group(1): m.group(2).strip()
        for m in re.finditer(
            r"EndpointRequestOptions&\s+with_(\w+)\s*\(\s*"
            r"([A-Za-z_][\w:<>\s]*?)\s+value\s*\)",
            text,
        )
    }


def _check_request_options_types(
    ssot_types: dict[str, str],
    cpp_with_types: dict[str, str],
    ffi_options_rs: pathlib.Path,
) -> list[str]:
    """Assert each request-option's type agrees across SSOT, C++, and FFI.

    For every SSOT request-option (excluding the `deadline` alias), map its
    `param_type` through `REQUEST_OPTION_TYPE_MAP` to the C++ `with_*`
    parameter spelling and the FFI struct-field spelling it must take, then
    compare against the actual declared types. A `param_type` outside the map,
    a C++ parameter that differs, or an FFI field that differs each fails with
    the option named. The FFI field type is read with the same
    `_struct_field_type` machinery the `[[value_field]]` gate uses.
    """
    errors: list[str] = []
    for name in sorted(ssot_types):
        if name in REQUEST_OPTIONS_WITH_EXEMPT:
            continue
        param_type = ssot_types[name]
        expected = _request_option_canonical_types(param_type)
        if expected is None:
            errors.append(
                f"  request-options `{name}`: SSOT param_type `{param_type}` is "
                f"outside REQUEST_OPTION_TYPE_MAP — add the canonical C++/FFI "
                f"spelling for it (or correct the param_type)."
            )
            continue
        exp_cpp, exp_ffi = expected
        actual_cpp = cpp_with_types.get(name)
        if actual_cpp != exp_cpp:
            errors.append(
                f"  request-options `{name}`: SSOT param_type `{param_type}` "
                f"implies C++ `with_{name}({exp_cpp})`, but the generated setter "
                f"takes `{actual_cpp or '<setter missing>'}`."
            )
        actual_ffi = _struct_field_type(
            ffi_options_rs.parent, "ThetaDataDxEndpointRequestOptions", name
        )
        if actual_ffi != exp_ffi:
            errors.append(
                f"  request-options `{name}`: SSOT param_type `{param_type}` "
                f"implies FFI field `{name}: {exp_ffi}`, but the generated struct "
                f"declares `{actual_ffi or '<field missing>'}`."
            )
    return errors


# ─── Cross-language method SIGNATURE parity (Route B) ────────────────
#
# The control / lifecycle plane is hand-written per binding (it is
# idiomatically divergent, so it is NOT codegen'd from a single SSOT the
# way the data plane is). A `[[method]]` row carries only per-binding
# PRESENCE booleans — name-level. This adds an OPTIONAL `[method.signature]`
# sub-table that pins the actual params + return across the enrolled
# bindings, extracts each binding's declared signature from source, and
# compares through a canonical-Rust → per-binding TYPE_MAP. A row WITHOUT a
# `[method.signature]` keeps the name-only check unchanged — fully opt-in,
# exactly like the `rust` column.
#
# The map's logical key is the canonical Rust type the spec writes. Each
# `(canonical, binding)` cell lists every accepted binding spelling. The
# comparison is FORWARD (`_sig_type_agrees`): the canonical name selects the
# cell, and the binding's extracted spelling must be one of that cell's
# accepted values. A reverse lookup would be ambiguous — `f64` is an accepted
# napi spelling for BOTH `usize` (the `usize`→`f64` widening napi-rs applies)
# and `f64` — so the spec's canonical type is always the source of truth.
#
# `Option<T>` is structural: a canonical `Option<inner>` agrees with each
# binding's idiomatic optional wrapping of `inner` (`inner | null` in
# `.d.ts`, `std::optional<inner>` in C++, `Option<inner>` in the Rust-typed
# Python / napi / FFI surfaces). The FFI `_explicit (has_value, n)` ABI split
# injects an extra `bool` PARAM, so it is encoded with an `ffi_params`
# override on the row, not by the structural Option rule.
#
# The six signature "languages" (the TYPE_MAP cell keys) are binding VIEWS,
# NOT the four public bindings: `ts_napi` (the napi Rust `fn` — authoritative
# for TypeScript) and `ts_dts` (the generated `.d.ts` — a secondary
# cross-check) are distinct columns over the one TypeScript binding.
SIGNATURE_TYPE_MAP: dict[str, dict[str, tuple[str, ...]]] = {
    # canonical Rust type → {signature-lang: (accepted spelling, ...)}.
    # `usize` is platform-width. Its C++ / C-ABI cells accept ONLY the
    # platform-width spellings (`size_t` / `usize`), never the fixed-width
    # `uint64_t` / `u64` — those belong to `u64` below, and accepting them
    # here would let a `usize` row that drifts to a fixed-width return pass.
    # A binding that DELIBERATELY widens `usize` to a fixed-width boundary
    # type (the C ABI is fixed-width by design; C++ mirrors the ABI it wraps)
    # pins `u64` for that lang via a per-row `<lang>_returns` override, not a
    # loose cell here.
    "usize": {
        "python": ("usize",),
        # The `.pyi` stub spells every integer width as Python's unbounded
        # `int` — the runtime exposes no NewType per width, so `usize` / `u64`
        # / `i64` / `u32` / `i32` all read `int` in the stub. A width drift is
        # therefore invisible to THIS lane (it is the pyo3-source `python`
        # lane's job, which carries the exact width); the stub lane pins that
        # the parameter / return stays an integer at all and does not drift to
        # a non-integer Python type.
        "python_pyi": ("int",),
        "ts_napi": ("f64", "BigInt", "u32", "i64"),
        "ts_dts": ("number", "bigint"),
        "cpp": ("size_t", "std::size_t"),
        "rust": ("usize",),
        "ffi": ("usize", "size_t"),
    },
    # `u64` is fixed-width. Its C++ / C-ABI cells accept ONLY the fixed-width
    # spellings (`uint64_t` / `u64`), never the platform-width `size_t` /
    # `usize` — those belong to `usize` above. Zero overlap with `usize` so a
    # `u64` row that drifts to a platform-width return fails closed.
    "u64": {
        "python": ("u64",),
        "python_pyi": ("int",),
        "ts_napi": ("BigInt", "f64"),
        "ts_dts": ("bigint", "number"),
        "cpp": ("uint64_t", "std::uint64_t"),
        "rust": ("u64",),
        "ffi": ("u64", "uint64_t"),
    },
    "i64": {
        "python": ("i64",),
        "python_pyi": ("int",),
        "ts_napi": ("i64", "BigInt"),
        "ts_dts": ("number", "bigint"),
        "cpp": ("int64_t",),
        "rust": ("i64",),
        "ffi": ("i64", "int64_t"),
    },
    "u32": {
        "python": ("u32",),
        "python_pyi": ("int",),
        "ts_napi": ("u32",),
        "ts_dts": ("number",),
        "cpp": ("uint32_t",),
        "rust": ("u32",),
        "ffi": ("u32", "uint32_t"),
    },
    "i32": {
        "python": ("i32",),
        "python_pyi": ("int",),
        "ts_napi": ("i32",),
        "ts_dts": ("number",),
        "cpp": ("int32_t", "int"),
        "rust": ("i32",),
        "ffi": ("i32", "int32_t"),
    },
    "f64": {
        "python": ("f64",),
        "python_pyi": ("float", "int"),
        "ts_napi": ("f64",),
        "ts_dts": ("number",),
        "cpp": ("double",),
        "rust": ("f64",),
        "ffi": ("f64", "double"),
    },
    "bool": {
        "python": ("bool",),
        "python_pyi": ("bool",),
        "ts_napi": ("bool",),
        "ts_dts": ("boolean",),
        "cpp": ("bool",),
        "rust": ("bool",),
        # bool is C-unfriendly across the ABI; it is encoded as `i32`.
        "ffi": ("bool", "i32"),
    },
    "String": {
        "python": ("String", "&str"),
        # The stub spells a free string return / param as `str`. A Config knob
        # that CONSTRAINS the value set spells it as a `Literal["a", "b", ...]`;
        # that is NOT folded to `str` here — its row pins the exact value set via
        # a `python_pyi_returns` Literal override (compared by value set in
        # `_sig_type_agrees`). So a bare `String` spec accepts only `str`, and a
        # Literal actual against a bare `String` fails closed (forcing the pin).
        "python_pyi": ("str",),
        "ts_napi": ("String", "&str"),
        "ts_dts": ("string",),
        "cpp": ("std::string", "const std::string&"),
        "rust": ("String", "&str"),
        "ffi": ("*const c_char", "*const c_uchar"),
    },
    # A wall-clock span. The Rust core takes `std::time::Duration`; bindings
    # carry the integer / chrono spelling they expose to users.
    "Duration": {
        "python": ("u64", "f64", "Duration"),
        "python_pyi": ("int", "float"),
        "ts_napi": ("f64", "BigInt"),
        "ts_dts": ("number",),
        "cpp": ("uint64_t", "std::chrono::milliseconds", "double"),
        "rust": ("Duration", "std::time::Duration"),
        "ffi": ("u64", "uint64_t"),
    },
    # The built fluent subscription's nested contract / sec-type handle. Each
    # binding hands back its own value-object wrapper of the same logical
    # entity; the managed surfaces wrap it in `Option`, so these are the inner
    # types of an `Option<Contract>` / `Option<SecType>` canonical. C++ flattens
    # the contract instead (see the per-binding `Subscription` rows), so it has
    # no cell here.
    "Contract": {
        "python": ("PyContract",),
        # The stub names the fluent contract by its public Python class
        # `Contract` (the inner type of `Subscription.contract`'s
        # `Optional[Contract]`), not the Rust pyclass `PyContract`.
        "python_pyi": ("Contract",),
        "ts_napi": ("ContractRef",),
        # The napi `.d.ts` spells the fluent contract `ContractRef` (the
        # `Contract` symbol is the streaming event-payload class); the wrapper
        # re-exports `Contract = ContractRef`, so both names denote the surface.
        "ts_dts": ("Contract", "ContractRef"),
    },
    "SecType": {
        "python": ("PySecType",),
        "python_pyi": ("SecType",),
        "ts_napi": ("SecType",),
        "ts_dts": ("SecType",),
    },
    # The flat-file request dispatcher's typed enum params on the Rust core
    # only: `FlatFiles::request(SecType, ReqType, &str)`. The managed bindings
    # pass plain Strings (no typed-enum twin), so these carry a `rust` cell
    # alone — used by the `rust_params` override on the `request` row.
    "FlatFileSecType": {
        "rust": ("crate::flatfiles::SecType", "SecType"),
    },
    "FlatFileReqType": {
        "rust": ("crate::flatfiles::ReqType", "ReqType"),
    },
    # The wire-format selector on `flatFileToPath`'s last param: a String on the
    # managed bindings (the format name, optional on Python / TypeScript), the
    # typed `FlatFileFormat` enum on the Rust core. Per-lang param override only.
    "FlatFileFormat": {
        "rust": ("crate::flatfiles::FlatFileFormat", "FlatFileFormat"),
    },
    # The destination-path param on `flatFileToPath` (the Rust core takes an
    # `impl AsRef<Path>`; the managed / C++ bindings take their String spelling).
    "DestPath": {
        "rust": ("impl AsRef<std::path::Path>", "impl AsRef<Path>"),
    },
    # C++-only typed enums on `FluentSubscription`: the full/per-contract
    # `scope()` discriminant the managed bindings model as the `isFull` bool
    # instead. No managed cell — a row pinning `Scope` for Python / TypeScript
    # fails closed (those bindings carry the bool, not the enum).
    "Scope": {"cpp": ("Scope", "FluentSubscription::Scope")},
    "Kind": {"cpp": ("Kind", "FluentSubscription::Kind")},
    # A single built subscription handed to subscribe / unsubscribe. Each
    # binding takes its own value-object spelling: Python the polymorphic
    # `&Bound<PyAny>` it coerces internally (a `Subscription` or a contract
    # spec), the napi `&Subscription` handle (qualified `&fluent::Subscription`
    # when imported through the module path), the `.d.ts` `Subscription`, the
    # C++ `const FluentSubscription&`.
    "Subscription": {
        "python": ("&Bound<'_, PyAny>", "PyObject", "Py<PyAny>"),
        # The pyo3 source takes the polymorphic `&Bound<PyAny>` it coerces, but
        # the stub presents the typed `Subscription` the user actually passes.
        "python_pyi": ("Subscription",),
        "ts_napi": ("&fluent::Subscription", "&Subscription"),
        "ts_dts": ("Subscription",),
        "cpp": ("const class FluentSubscription&", "const FluentSubscription&"),
    },
    # The iterable of subscriptions handed to subscribeMany / unsubscribeMany.
    # Python takes the same polymorphic `&Bound<PyAny>` (an iterable it
    # coerces); the napi `Vec<&Subscription>`; the `.d.ts` `Array<Subscription>`;
    # the C++ `std::initializer_list<FluentSubscription>`.
    "SubscriptionList": {
        "python": ("&Bound<'_, PyAny>",),
        # The stub presents the iterable as a typed `List[Subscription]`
        # (a `Sequence[Subscription]` is the equally-valid read-only spelling).
        "python_pyi": ("List[Subscription]", "Sequence[Subscription]"),
        "ts_napi": ("Vec<&fluent::Subscription>", "Vec<&Subscription>"),
        "ts_dts": ("Array<Subscription>",),
        "cpp": (
            "std::initializer_list<class FluentSubscription>",
            "std::initializer_list<FluentSubscription>",
        ),
    },
    # The per-event streaming callback handed to startStreaming / setCallback.
    # Python takes a `Py<PyAny>` callable; C++ a `std::function`. The napi
    # `ThreadsafeFunction<...>` carries a long generic argument list that is
    # pure binding machinery (queue depth, status), so the napi callback param
    # is left to a `skip_langs = ["ts_napi"]` on the row rather than pinned
    # here — there is no cross-binding contract in those generics.
    "Callback": {
        "python": ("Py<PyAny>", "PyObject"),
        # The stub names the per-event handler via its `EventCallback` alias
        # (`Callable[[<event-union>], None]`), the documented streaming-callback
        # type; the alias is the cross-binding handler contract.
        "python_pyi": ("EventCallback",),
        "cpp": ("std::function<void(const StreamEvent&)>",),
        # The `.d.ts` per-event handler type. napi-rs generates the parameter
        # name `arg`; the cross-binding contract is the `StreamEvent` payload
        # shape, so a drift to a different event type still fails closed. The
        # hand-written `streaming(...)` wrapper names the same type via its
        # `StreamEventCallback` alias.
        "ts_dts": (
            "((arg: StreamEvent) => void)",
            "(arg: StreamEvent) => void",
            "StreamEventCallback",
        ),
    },
    # The optional batch-reader tuning bag on `StreamView.batches`. The managed
    # bindings pass a single options object (the napi / `.d.ts` `BatchesOptions`
    # struct); Python and C++ spread it into positional / keyword params
    # instead, so this cell covers only the object-passing bindings.
    "BatchesOptions": {
        "ts_napi": ("BatchesOptions", "crate::streaming_batches::BatchesOptions"),
        "ts_dts": ("BatchesOptions",),
    },
    # The C++-only back-pressure mode enum on `StreamView.batches` (block vs
    # drop-oldest); the managed bindings pass the mode as the `BatchesOptions`
    # field instead, so only the C++ cell exists.
    "Backpressure": {"cpp": ("Backpressure",)},
}

# Return-position canonical types (the result / unit wrappers). Kept apart
# from the param map because their spellings are return-only — the fallible
# result wrapper is `PyResult<...>` in Python, a thrown error (`napi::Result`)
# in TypeScript, an exception in C++, and an `i32` status in the C ABI.
SIGNATURE_RETURN_TYPE_MAP: dict[str, dict[str, tuple[str, ...]]] = {
    "()": {
        "python": ("()", "PyResult<()>"),
        # A no-result method's stub return annotation is `None` (the extractor
        # also yields `None` for a `def`-with-no-`->`, the pyo3 unit method).
        "python_pyi": ("None",),
        "ts_napi": ("()", "Result<()>", "napi::Result<()>"),
        "ts_dts": ("void", "Promise<void>"),
        "cpp": ("void",),
        "rust": ("()",),
        "ffi": ("()", "i32"),
    },
    # Arrow IPC stream bytes (the columnar exit). Each binding hands back its
    # idiomatic owned-bytes container: a Python object (`bytes` / `memoryview`,
    # typed as the opaque `Py<PyAny>`), a Node `Buffer`, a C++ byte vector, the
    # C-ABI owned-bytes struct (`ThetaDataDxArrowBytes` for the streaming
    # reader, `ThetaDataDxFlatFileBytes` for the flat-file rows).
    "Bytes": {
        "python": ("PyResult<Py<PyAny>>", "Py<PyAny>"),
        # The stub types the opaque owned-bytes object as `Any` (the runtime
        # hands back a `bytes` / `memoryview`); `bytes` is accepted for a stub
        # that names the concrete container.
        "python_pyi": ("Any", "bytes"),
        "ts_napi": ("napi::Result<Buffer>", "Buffer"),
        "ts_dts": ("Buffer", "Uint8Array"),
        "cpp": ("std::vector<uint8_t>", "std::vector<std::uint8_t>"),
        "ffi": ("ThetaDataDxArrowBytes", "ThetaDataDxFlatFileBytes"),
    },
    # The fixed Arrow schema a record-batch reader yields. The managed bindings
    # expose an Arrow schema object (an opaque `Py<PyAny>` on Python, the
    # `apache-arrow` `Schema` on TypeScript), C++ the shared Arrow schema
    # pointer.
    "Schema": {
        "python": ("PyResult<Py<PyAny>>", "Py<PyAny>"),
        # The stub types the Arrow schema object as `Any` (it is a runtime
        # `pyarrow.Schema`, not statically modelled).
        "python_pyi": ("Any",),
        "ts_napi": ("Schema",),
        "ts_dts": ("Schema", "import('apache-arrow').Schema"),
        "cpp": ("std::shared_ptr<arrow::Schema>",),
    },
    # A materialiser that hands back an arbitrary Python object (a pandas /
    # polars frame, a list of dicts). Python-only — no cross-binding twin — so
    # only the Python cell exists; a row pinning it for another binding fails
    # closed.
    "PyObject": {
        "python": ("PyResult<Py<PyAny>>", "Py<PyAny>"),
        # The stub types the arbitrary-object materialiser as `Any`
        # (`to_pandas` / `to_polars` — a frame) or `List[Any]` (`to_list` — a
        # list of row dicts); both are the opaque Python-object return.
        "python_pyi": ("Any", "List[Any]"),
    },
    # A `Credentials` factory return: the auth handle itself. Python spells it
    # `Self` (or `PyResult<Self>` for the fallible file / env factories),
    # TypeScript `Credentials` (or `napi::Result<Credentials>`), C++ the value
    # type `Credentials`. Both the fallible and infallible spellings are
    # accepted — fallibility is a per-factory property, not a cross-binding
    # contract the signature gate pins.
    "Credentials": {
        "python": ("Self", "PyResult<Self>", "Credentials", "PyResult<Credentials>"),
        # The stub resolves the factory return to the concrete `Credentials`
        # (it does not spell `Self` on a `@staticmethod`).
        "python_pyi": ("Credentials",),
        "ts_napi": ("Credentials", "napi::Result<Credentials>"),
        "ts_dts": ("Credentials",),
        "cpp": ("Credentials",),
    },
    # The per-contract active-subscription snapshot. Each binding returns its
    # own collection shape: Python a `Vec<Subscription>` of typed handles (the
    # `StreamView` surface wraps it in `PyResult`, unwrapped before the
    # compare), the napi a `serde_json::Value` JSON tree (`.d.ts` `any`), C++ a
    # `std::vector<Subscription>`. There is no single scalar twin, so the
    # divergence is encoded per binding here rather than forced symmetric.
    "Subscriptions": {
        "python": ("Vec<crate::fluent::PySubscription>", "Vec<PySubscription>"),
        # The stub types the active-subscription snapshot as `List[Subscription]`.
        "python_pyi": ("List[Subscription]",),
        "ts_napi": ("serde_json::Value",),
        "ts_dts": ("any",),
        "cpp": ("std::vector<Subscription>",),
    },
    # The full-stream active-subscription snapshot. Same per-binding collection
    # shapes as `Subscriptions`, except the C++ element type is the full-stream
    # `FullSubscription` (the managed bindings return the same typed list / JSON
    # tree as the per-contract variant).
    "FullSubscriptions": {
        "python": ("Vec<crate::fluent::PySubscription>", "Vec<PySubscription>"),
        "python_pyi": ("List[Subscription]",),
        "ts_napi": ("serde_json::Value",),
        "ts_dts": ("any",),
        "cpp": ("std::vector<FullSubscription>",),
    },
    # The pull-based columnar reader returned by `StreamView.batches`. Python
    # hands back the `RecordBatchStream` pyclass (wrapped in `PyResult`,
    # unwrapped before the compare), the napi the `RecordBatchStreamHandle`
    # transport, the `.d.ts` the `Promise<RecordBatchStream>` JS wrapper, C++
    # the `std::shared_ptr<RecordBatchStream>` (an `arrow::RecordBatchReader`).
    "RecordBatchStream": {
        "python": (
            "crate::streaming_batches::RecordBatchStream",
            "RecordBatchStream",
        ),
        # The stub presents the public `RecordBatchStream` wrapper class — the
        # coverage stubtest cannot give (a compiled pyo3 return carries no
        # runtime annotation), so a stub drift on this return is THIS lane's
        # to catch.
        "python_pyi": ("RecordBatchStream",),
        "ts_napi": (
            "crate::streaming_batches::RecordBatchStreamHandle",
            "RecordBatchStreamHandle",
        ),
        "ts_dts": ("Promise<RecordBatchStream>", "RecordBatchStream"),
        "cpp": ("std::shared_ptr<RecordBatchStream>",),
    },
    # The three `Client` data-plane VIEW accessors. Each is a zero-arg getter
    # whose RETURN TYPE is the cross-binding contract — the view the binding
    # hands back. The type name differs per binding (the managed bindings name
    # the view directly; the Rust core returns a borrowed view), so each is a
    # per-binding cell. A drop / rename of the returned view on any binding
    # trips the gate.
    "HistoricalView": {
        "python": ("HistoricalView",),
        "python_pyi": ("HistoricalView",),
        "ts_napi": ("HistoricalView",),
        "ts_dts": ("HistoricalView",),
        "cpp": ("Historical",),
        "rust": ("&HistoricalClient",),
    },
    "StreamView": {
        "python": ("StreamView",),
        "python_pyi": ("StreamView",),
        "ts_napi": ("StreamView",),
        "ts_dts": ("StreamView",),
        "cpp": ("Stream",),
        "rust": ("StreamSurface<>",),
    },
    "FlatFilesNamespace": {
        "python": ("FlatFilesNamespace",),
        "python_pyi": ("FlatFilesNamespace",),
        "ts_napi": ("FlatFilesNamespace",),
        "ts_dts": ("FlatFilesNamespace",),
        "cpp": ("FlatFiles",),
        "rust": ("FlatFiles<>",),
    },
    # The fluent client builder returned by `Client::builder()`. C++ + Rust
    # only (the managed bindings reach the same surface through the inline
    # constructor / options-object factory, not a builder), so no managed cell.
    "ClientBuilder": {
        "cpp": ("ClientBuilder",),
        "rust": ("crate::client_builder::ClientBuilder", "ClientBuilder"),
    },
    # The decoded flat-file row collection returned by the `FlatFilesNamespace`
    # fetch methods. The managed bindings return the `FlatFileRowList` value
    # object (wrapped in the binding's fallible result, unwrapped before the
    # compare); the Rust core returns the owned `Vec<FlatFileRow>`.
    "FlatFileRowList": {
        "python": ("FlatFileRowList",),
        "python_pyi": ("FlatFileRowList",),
        "ts_napi": ("FlatFileRowList",),
        "ts_dts": ("FlatFileRowList",),
        "cpp": ("FlatFileRowList",),
        "rust": ("Vec<crate::flatfiles::FlatFileRow>", "Vec<FlatFileRow>"),
    },
    # The built fluent subscription returned by the per-contract / full-stream
    # builders (`Contract.quote()`, `SecType.fullTrades()`). Each binding hands
    # back its own subscription value object. Distinct from the `Subscription`
    # PARAM canonical (the handle passed to subscribe), which carries the
    # by-reference spellings — this is the by-value return shape.
    "BuiltSubscription": {
        "python": ("PySubscription",),
        # The stub presents the built subscription as the public `Subscription`
        # class (not the Rust pyclass `PySubscription`).
        "python_pyi": ("Subscription",),
        "ts_napi": ("Subscription",),
        "ts_dts": ("Subscription",),
        "cpp": ("FluentSubscription",),
    },
    # The `flatFileToPath` blob-to-disk return. The managed bindings hand back
    # the written path as a String; C++ writes in place and returns `void`; the
    # Rust core returns the owned `PathBuf`.
    "WrittenPath": {
        "python": ("String",),
        "python_pyi": ("str",),
        "ts_napi": ("String",),
        "ts_dts": ("string",),
        "cpp": ("void",),
        "rust": ("std::path::PathBuf", "PathBuf"),
    },
    # The context-managed streaming session returned by `Client.streaming`.
    # Python hands back the `StreamingSession` pyclass (`Py<StreamingSession>`,
    # the `PyResult` unwrapped before the compare); the hand-written `.d.ts`
    # wrapper returns the `StreamingSession` interface. The napi layer has no
    # `fn` (the session is a JS wrapper), so the row skips `ts_napi`.
    "StreamingSession": {
        "python": ("Py<StreamingSession>", "StreamingSession"),
        "python_pyi": ("StreamingSession",),
        "ts_dts": ("StreamingSession",),
    },
    # The inline-construction options object + its `Client` return on the
    # TypeScript `connectWith` factory (TypeScript-only — Python uses the
    # constructor kwargs, Rust / C++ the fluent builder).
    "ClientConnectOptions": {
        "ts_napi": ("ClientConnectOptions",),
        "ts_dts": ("ClientConnectOptions",),
    },
    "Client": {
        "ts_napi": ("Client",),
        "ts_dts": ("Client",),
    },
    # The active-subscription snapshot returned by `Client.subscriptionInfo`
    # (Python + Rust only). Python materialises it as a `Vec<(root, kind)>`
    # tuple list; the Rust core returns the opaque `SubscriptionInfo` struct.
    "SubscriptionInfo": {
        "python": ("Vec<(String, String)>", "Vec<(String,String)>"),
        # The stub materialises the snapshot as `List[Tuple[str, str]]`
        # (whitespace-folded before the compare).
        "python_pyi": ("List[Tuple[str,str]]",),
        "rust": ("SubscriptionInfo",),
    },
    # A `Config` factory return (the env-tier presets `production` / `dev` /
    # `stage`, the `fromDotenv` loader). Python / TypeScript spell it `Self`
    # (the pyo3 / napi constructor return), the napi `.d.ts` resolves the
    # `Self` to the concrete `Config`, C++ the value type `Config`.
    "Config": {
        "python": ("Self", "Config"),
        # The stub resolves the env-tier factory return to the concrete
        # `Config` (a `@staticmethod` does not spell `Self`).
        "python_pyi": ("Config",),
        "ts_napi": ("Self", "Config"),
        "ts_dts": ("Config",),
        "cpp": ("Config",),
    },
}


_PYI_STR_LITERAL_RE = re.compile(
    r"Literal\[\s*(?:\"[^\"]*\"|'[^']*')(?:\s*,\s*(?:\"[^\"]*\"|'[^']*'))*\s*\]"
)
# One quoted member inside a `Literal[...]` — double- or single-quoted.
_PYI_LITERAL_MEMBER_RE = re.compile(r"\"([^\"]*)\"|'([^']*)'")


def _sig_literal_value_set(spelling: str) -> frozenset[str] | None:
    """The set of string members of a `.pyi` `Literal["a", "b", ...]`, or None
    when `spelling` is not such a Literal. The set is order-insensitive —
    `Literal["a", "b"]` and `Literal["b", "a"]` are the same Python type — so a
    Literal spec and a Literal actual compare by VALUE SET, catching an added,
    removed, or renamed member that a fold-to-`str` would hide."""
    if not _PYI_STR_LITERAL_RE.fullmatch(spelling.strip()):
        return None
    return frozenset(d or s for d, s in _PYI_LITERAL_MEMBER_RE.findall(spelling))


def _sig_canon_type(raw: str) -> str:
    """Fold a type spelling to its comparison form: drop Rust lifetimes
    (`&'static str` → `&str`), drop the napi prelude module path
    (`napi::bindgen_prelude::BigInt` → `BigInt`), then whitespace-fold. The
    lifetime / napi-path qualifiers are surface noise the binding adds, never a
    cross-binding type difference.

    A `.pyi` `Literal["a", "b"]` is NOT folded to `str`: a constrained Config
    knob (`flush_mode: Literal["batched", "immediate"]`) carries an exact value
    set that is part of the client-facing contract, and folding it would hide a
    value-set drift (adding / removing / renaming a member). Such rows pin the
    Literal directly via a `python_pyi_returns` override, and `_sig_type_agrees`
    compares the two Literals by value set. The whitespace-fold below still
    normalises spacing inside the `Literal[...]`, so `Literal["a","b"]` and
    `Literal["a", "b"]` canonicalise identically."""
    # Strip the lifetime pattern with the Literal members masked out, so a
    # single-quoted member (`Literal['a', 'b']`) is never consumed by the
    # lifetime regex (`'\w+`) and corrupted.
    s = raw.strip()
    masked = _PYI_STR_LITERAL_RE.sub(lambda m: m.group(0).replace("'", '"'), s)
    masked = re.sub(r"'\w+\s*", "", masked)
    masked = re.sub(r"\bnapi::bindgen_prelude::", "", masked)
    return re.sub(r"\s+", "", masked)


def _sig_unwrap_result(spelling: str) -> str:
    """Strip one fallible-result wrapper from a RETURN spelling: `PyResult<T>`
    / `napi::Result<T>` / `Result<T>` → `T`. An optional module qualifier on
    the wrapper is tolerated (`pyo3::PyResult<T>`, the fully-qualified form one
    streaming accessor writes), so the inner `T` is compared regardless of how
    the binding spells the path. Fallibility is a per-binding surface property
    (pyo3 `PyResult`, a thrown napi error), not a cross-binding return-type
    difference the signature gate pins. A non-wrapped spelling is returned
    unchanged.

    A `.d.ts` async method returns `Promise<T>`; the promise is the TypeScript
    fallible/async wrapper (the same role `napi::Result` plays on the napi
    side), so it is unwrapped too — the awaited `T` is the cross-binding
    return contract, not the promise envelope."""
    s = spelling.strip()
    pm = re.fullmatch(r"Promise\s*<\s*(.+)\s*>", s)
    if pm:
        s = pm.group(1).strip()
    m = re.fullmatch(r"(?:[A-Za-z_][\w:]*::)?(?:Py)?Result\s*<\s*(.+)\s*>", s)
    if not m:
        return s
    inner = m.group(1).strip()
    # A Rust `Result<T, E>` carries the error arm explicitly; the cross-binding
    # contract is the ok type `T`, so drop a trailing `, E` at top level (commas
    # inside `T`'s own generics are not split — depth-aware).
    return _sig_split_params(inner)[0].strip()


def _sig_option_inner(spelling: str, lang: str) -> str | None:
    """If `spelling` is `lang`'s idiomatic optional wrapper, the inner type;
    else None. `Option<T>` for the Rust-typed surfaces, `std::optional<T>` for
    C++, `T | null` for `.d.ts`. The FFI `_explicit` split is param-list level,
    so its inner is the bare `T` (matched via the `ffi_params` override)."""
    s = spelling.strip()
    if lang in ("python", "ts_napi", "rust", "ffi"):
        m = re.fullmatch(r"Option\s*<\s*(.+)\s*>", s)
        if m:
            return m.group(1).strip()
    elif lang == "cpp":
        m = re.fullmatch(r"std::optional\s*<\s*(.+)\s*>", s)
        if m:
            return m.group(1).strip()
    elif lang == "ts_dts":
        # `.d.ts` optionals are `T | null`, and napi-rs emits a nullable PARAM
        # as `T | undefined | null` (either union order). Strip the trailing
        # `| null` / `| undefined` members to recover the inner `T`.
        m = re.fullmatch(r"(.+?)\s*(?:\|\s*(?:null|undefined)\s*)+", s)
        if m:
            return m.group(1).strip()
    elif lang == "python_pyi":
        # The stub spells an optional as `Optional[T]` or the PEP 604 union
        # `T | None`. Either recovers the inner `T`.
        m = re.fullmatch(r"Optional\s*\[\s*(.+)\s*\]", s)
        if m:
            return m.group(1).strip()
        m = re.fullmatch(r"(.+?)\s*\|\s*None", s)
        if m:
            return m.group(1).strip()
    return None


def _sig_type_agrees(canonical: str, actual: str, lang: str) -> bool:
    """Does the binding's `actual` type spelling satisfy the `canonical` spec
    type under signature-`lang`?

    Forward map only — the canonical name selects the cell, `actual` must be
    one of its accepted spellings. `Option<inner>` is structural (it agrees
    with `lang`'s idiomatic optional wrapping of `inner`). An unknown canonical
    name, or an `actual` outside the cell, fails closed so an unmapped
    divergence can never silently pass.
    """
    # A `Literal["a", "b", ...]` canonical (a `python_pyi_returns` override on a
    # value-constrained Config knob) pins the EXACT value set: the actual stub
    # type must be a Literal over the same members (order-insensitive). A drift
    # that adds, removes, or renames a member fails — the coverage the old
    # fold-to-`str` discarded. A non-Literal actual (the stub widened the knob
    # to a bare `str`) fails too: the constraint is part of the contract.
    spec_set = _sig_literal_value_set(canonical)
    if spec_set is not None:
        return _sig_literal_value_set(actual) == spec_set
    cm = re.fullmatch(r"Option\s*<\s*(.+)\s*>", canonical.strip())
    if cm:
        unwrapped = _sig_option_inner(actual, lang)
        if unwrapped is None:
            return False
        return _sig_type_agrees(cm.group(1).strip(), unwrapped, lang)
    cell = SIGNATURE_TYPE_MAP.get(canonical) or SIGNATURE_RETURN_TYPE_MAP.get(canonical)
    if cell is None:
        # A raw C-ABI handle has no higher-level canonical to translate from —
        # the C type IS the canonical. The `ffi` lang's opaque pointers
        # (`*const ThetaDataDxClient`) and owned-struct returns
        # (`ThetaDataDxArrowBytes`), and the C++ raw-handle escape hatch
        # (`const ThetaDataDxFlatFileRowList*` from `FlatFileRowList::get`),
        # therefore compare by exact whitespace-folded spelling. This fires
        # ONLY for the C ABI (always) and for a C++ canonical that is itself a
        # raw pointer / `ThetaDataDx*` struct spelling — a managed scalar
        # divergence still fails closed everywhere else, so the escape can
        # never silently pass an unmapped `usize`/`bool`/etc.
        if lang == "ffi" or (
            lang == "cpp"
            and ("*" in canonical or _sig_canon_type(canonical).startswith("ThetaDataDx"))
        ):
            return _sig_canon_type(canonical) == _sig_canon_type(actual)
        return False
    accepted = {_sig_canon_type(s) for s in cell.get(lang, ())}
    return _sig_canon_type(actual) in accepted


def _sig_split_params(arglist: str) -> list[str]:
    """Split a comma-separated parameter list at TOP-LEVEL commas only (commas
    inside `<...>` / `(...)` / `[...]` are nested generic / fn-pointer args)."""
    out: list[str] = []
    depth = 0
    cur = ""
    for ch in arglist:
        if ch in "<([":
            depth += 1
            cur += ch
        elif ch in ">)]":
            depth -= 1
            cur += ch
        elif ch == "," and depth == 0:
            out.append(cur)
            cur = ""
        else:
            cur += ch
    if cur.strip():
        out.append(cur)
    return out


def _sig_balanced_parens(text: str, start: int) -> tuple[str, int]:
    """`(inner, after)` for the paren list whose `(` sits just before `start`
    (so `start` is the index immediately after the `(`). `inner` is the
    balanced content; `after` is the index past the matching `)`."""
    depth = 1
    i = start
    while i < len(text) and depth > 0:
        c = text[i]
        if c == "(":
            depth += 1
        elif c == ")":
            depth -= 1
            if depth == 0:
                return text[start:i], i + 1
        i += 1
    return text[start:i], i


def _sig_rust_param_type(param: str) -> str:
    """The type half of a Rust `name: Type` parameter (or the whole token when
    untyped — a bare `self`)."""
    s = param.strip()
    if ":" in s:
        return s.split(":", 1)[1].strip()
    return s


def _sig_is_rust_receiver(param: str) -> bool:
    """True for a `self` receiver or a `py: Python<...>` GIL token — neither is
    a cross-binding parameter, so both are stripped before the type compare.

    pyo3 also spells the receiver by VALUE as the bound-instance smart pointers
    `Py<Self>` / `PyRef<'_, Self>` / `PyRefMut<'_, Self>` / `Bound<'_, Self>`
    (a `#[pymethods] fn streaming(slf: Py<Self>, ...)`), which carry the
    instance the same way `&self` does — they are receivers, not cross-binding
    params, so they are stripped too."""
    s = param.strip()
    if s in ("self", "&self", "&mut self") or s.startswith(("&self", "&mut self", "self")):
        return True
    if ":" in s:
        ty = s.split(":", 1)[1]
        if re.search(r"\bPython\b", ty):
            return True
        if re.search(r"\b(?:Py|PyRef|PyRefMut|Bound)\s*<[^>]*\bSelf\b", ty):
            return True
    return False


def _sig_rust_fn(body: str, name: str, *, require_pub: bool = False) -> tuple[list[str], str] | None:
    """`(stripped_param_types, return_type)` for `fn name` (optionally
    `pub`/`pub async`) inside a Rust `impl` body. Receivers / the `py` token
    are stripped; the return defaults to `()` (unit) when no `-> T` follows."""
    prefix = r"\bpub\s+(?:async\s+)?fn\s+" if require_pub else r"\bfn\s+"
    m = re.search(prefix + re.escape(name) + r"\s*(?:<[^>]*>)?\s*\(", body)
    if not m:
        return None
    arglist, after = _sig_balanced_parens(body, m.end())
    rm = re.match(r"\s*->\s*([^\{;]+)", body[after:])
    ret = rm.group(1).strip() if rm else "()"
    params = [
        _sig_rust_param_type(p)
        for p in _sig_split_params(arglist)
        if not _sig_is_rust_receiver(p)
    ]
    return params, ret


def _sig_extract_python(py_src: pathlib.Path, cls: str, method: str) -> tuple[list[str], str] | None:
    """Python (pyo3) signature: the `fn method` inside a `#[pymethods] impl
    cls` (or qualified-path impl), receivers + the `py: Python` token stripped.
    The pyo3 `#[pyo3(signature = ...)]` only adjusts default-arg arity, which
    the fn sig itself already carries, so the fn sig is authoritative.

    A `#[getter]` readback accessor carries a `get_` prefix on its Rust fn name
    (`fn get_reconnect_policy`) while pyo3 strips the prefix so the Python
    property name stays bare (`config.reconnect_policy`); the bare `method`
    falls back to `get_<method>`, exactly as the forward presence check
    accepts `fn <snake>` or `fn get_<snake>`."""
    if not py_src.is_dir():
        return None
    impl_re = re.compile(r"impl\s+(?:[A-Za-z_]\w*::)*" + re.escape(cls) + r"\s*\{")
    for rs in py_src.rglob("*.rs"):
        text = _read_source(rs)
        for h in impl_re.finditer(text):
            body = _balanced_body(text, h.end())
            sig = _sig_rust_fn(body, method) or _sig_rust_fn(body, f"get_{method}")
            if sig is not None:
                return sig
    return None


def _pyi_class_bodies(pyi_path: pathlib.Path, cls: str) -> list[str]:
    """Every `class cls(...):` body text in the PEP 561 stub. A class body runs
    from the header line to the next column-0 non-blank line (the file's top
    level), so the indented members — and any nested `class`/`def` — are
    captured. A class is rarely redeclared in a stub, but all bodies are
    returned for symmetry with the `.d.ts` surface scan."""
    if not pyi_path.is_file():
        return []
    text = pyi_path.read_text(encoding="utf-8")
    out: list[str] = []
    cls_re = re.compile(r"(?m)^class\s+" + re.escape(cls) + r"\b[^\n]*:[ \t]*$")
    for m in cls_re.finditer(text):
        nl = text.find("\n", m.start())
        if nl == -1:
            continue
        body_lines: list[str] = []
        for line in text[nl + 1 :].splitlines(keepends=True):
            if line.strip() and not line[0].isspace():
                break
            body_lines.append(line)
        out.append("".join(body_lines))
    return out


def _sig_pyi_member(body: str, member: str) -> tuple[list[str], str] | None:
    """Parse `(params, ret)` for `member` inside a single stub class `body`, in
    either form the pinned Python surface uses:

      * a method  `def member(self, p: T, ...) -> R:` (the receiver `self`/`cls`
        and any `*` / `/` separator and `*args`/`**kwargs` are dropped; a
        default `= ...` is stripped off the param type; a `def` with no `->`
        defaults to the `None` unit return), or
      * a `@property` / bare read-write annotation `member: T` → a zero-arg
        signature returning `T` (the `Config` knobs and the `Subscription`
        accessors are stub properties, not methods).

    The method body may span lines (the keyword-only `batches(self, *, ...)`
    form), so the param list is read with a balanced-paren scan. Returns None
    when the member is absent from this body."""
    dm = re.search(r"(?m)^[ \t]+def[ \t]+" + re.escape(member) + r"[ \t]*\(", body)
    if dm:
        open_paren = body.index("(", dm.start())
        arglist, after = _sig_balanced_parens(body, open_paren + 1)
        rm = re.match(r"\s*->\s*(.+?)\s*:", body[after:], re.DOTALL)
        ret = rm.group(1).strip() if rm else "None"
        params: list[str] = []
        for p in _sig_split_params(arglist):
            tok = p.strip()
            # Drop the receiver, the keyword-only / positional-only markers, and
            # any *args / **kwargs — none is a cross-binding parameter.
            if not tok or tok in ("self", "cls", "*", "/") or tok.startswith("*"):
                continue
            if ":" in tok:
                ty = tok.split(":", 1)[1]
                if "=" in ty:  # strip a default value off the annotation
                    ty = ty.split("=", 1)[0]
                params.append(ty.strip())
            else:
                params.append(tok)
        return params, ret
    # Property / bare read-write annotation. Scan a copy with every `(...)` run
    # blanked so a deeper-indented `def` PARAMETER that happens to share the
    # member's name (`def batches(self, batch_size: Optional[int] = None)`
    # while looking up a `batch_size` property) cannot be misread as a class
    # property — only a genuine class-level `member: T` survives the mask.
    pm = re.search(
        r"(?m)^[ \t]+" + re.escape(member) + r"[ \t]*:[ \t]*(.+?)[ \t]*$",
        _pyi_blank_parens(body),
    )
    if pm:
        # Read the annotation from the ORIGINAL body at the matched span (the
        # mask only gates WHERE a property may match, never its text).
        return [], body[pm.start(1) : pm.end(1)].strip()
    return None


def _pyi_blank_parens(text: str) -> str:
    """Replace every balanced `(...)` run with spaces of equal length (newlines
    preserved), so a member-property scan never descends into a `def`'s
    parameter list. Length / line structure is preserved so match offsets line
    up with the original text."""
    out = list(text)
    depth = 0
    for i, ch in enumerate(text):
        if ch == "(":
            depth += 1
            out[i] = " "
        elif ch == ")":
            if depth > 0:
                out[i] = " "
                depth -= 1
        elif depth > 0 and ch != "\n":
            out[i] = " "
    return "".join(out)


def _sig_extract_python_pyi(
    pyi_path: pathlib.Path, cls: str, member_snake: str
) -> tuple[list[str], str] | None:
    """Python `.pyi` stub signature: the member `member_snake` inside
    `class cls` of the shipped PEP 561 stub — a method or a property
    (zero-arg). The stub is the client-facing type surface mypy / pyright
    consumers see.

    This lane verifies the stub against the cross-binding SPEC (the
    `python_pyi` type-map column), which is a DIFFERENT axis from Gate 6's
    stubtest: stubtest compares the stub against the RUNTIME and pins the
    parameter list / arity, but a compiled pyo3 method exposes no runtime
    return annotation, so stubtest cannot see a stub RETURN drift. This lane
    pins both the params (defence in depth with stubtest) AND the return (the
    coverage stubtest lacks) against the spec.

    Returns None when the stub is absent or the member is not declared in the
    class; `_sig_pyi_public_member_missing` then decides whether the absence is
    a dropped public member (fail) or a member legitimately served off the
    stub (degrades to the pyo3-source `python` lane + stubtest as authority)."""
    for body in _pyi_class_bodies(pyi_path, cls):
        sig = _sig_pyi_member(body, member_snake)
        if sig is not None:
            return sig
    return None


def _sig_pyi_public_member_missing(
    pyi_path: pathlib.Path, cls: str, member_snake: str
) -> bool:
    """Is `cls.member_snake` part of the hand-maintained public stub surface yet
    missing its declaration? True only when the CLASS is declared in the stub,
    carries NO class-level `__getattr__` escape, and the MEMBER is absent — a
    member that belongs on a fully-enumerated stub class was dropped.

    False when the class is absent from the stub (a generator-emitted class the
    stub deliberately omits — the 100+ historical builders / `<Tick>List`
    wrappers reached via the module-level `__getattr__ -> Any`), OR when the
    class carries its own `__getattr__` fallback (`AsyncClient`,
    `HistoricalClient`, `StreamingSession` route extras to `Any`). In both
    degrade cases the pyo3-source `python` lane + stubtest remain the authority
    — exactly the way `_sig_dts_public_member_missing` degrades a class absent
    from the `.d.ts` surface to napi-as-authority."""
    bodies = _pyi_class_bodies(pyi_path, cls)
    if not bodies:
        return False
    if any(re.search(r"(?m)^[ \t]+def[ \t]+__getattr__\b", b) for b in bodies):
        return False
    return _sig_extract_python_pyi(pyi_path, cls, member_snake) is None


def _sig_extract_ts_napi(ts_src: pathlib.Path, cls: str, method_camel: str) -> tuple[list[str], str] | None:
    """TypeScript signature (authoritative): the napi Rust `fn` inside `impl
    cls` whose camelCased name (or a `js_name`) is `method_camel`. The napi
    Rust fn is the source the `.d.ts` is generated from, so its param + return
    types are the binding contract."""
    if not ts_src.is_dir():
        return None
    impl_re = re.compile(r"impl\s+(?:[A-Za-z_]\w*::)*" + re.escape(cls) + r"\s*\{")
    js_name_re = re.compile(r'\bjs_name\s*=\s*"([A-Za-z_]\w*)"')
    for rs in ts_src.rglob("*.rs"):
        text = _read_source(rs)
        for h in impl_re.finditer(text):
            body = _balanced_body(text, h.end())
            fn_starts = [fm.start() for fm in re.finditer(r"\bfn\s+[a-z_]", body)]
            for idx, fm in enumerate(re.finditer(r"\bfn\s+([a-z_][a-z0-9_]*)\s*[(<]", body)):
                fn_name = fm.group(1)
                # The js_name (if any) overrides the auto-camelCased fn name. The
                # attribute window is bounded by the PREVIOUS `fn` so a prior
                # method's `js_name` can never bleed onto this one; the CLOSEST
                # (last) js_name in the window is the one decorating this fn.
                win_lo = fn_starts[idx - 1] if idx > 0 else 0
                names = js_name_re.findall(body[win_lo : fm.start()])
                js = names[-1] if names else _snake_to_camel(fn_name)
                if js != method_camel and fn_name != method_camel:
                    continue
                sig = _sig_rust_fn(body, fn_name)
                if sig is not None:
                    return sig
    return None


def _ts_dts_files(dts: pathlib.Path) -> list[pathlib.Path]:
    """The package `.d.ts` entry plus every sibling it re-exports with
    `export * from './X'`. The published entry (`streaming-session.d.ts`)
    layers its augmentations on top of the napi-generated `index.d.ts` via
    `export * from './index'`, so the public type surface a consumer imports
    is the UNION of both — the gate must read both to see a method the napi
    layer declares but the wrapper does not redeclare."""
    if not dts.is_file():
        return []
    out = [dts]
    seen = {dts.resolve()}
    text = _read_source(dts)
    for rel in re.findall(r"""export\s+\*\s+from\s+['"](\.[^'"]+)['"]""", text):
        cand = (dts.parent / rel).with_suffix(".d.ts")
        if cand.is_file() and cand.resolve() not in seen:
            out.append(cand)
            seen.add(cand.resolve())
    return out


def _ts_dts_class_bodies_with_src(dts: pathlib.Path, cls: str) -> list[tuple[pathlib.Path, str]]:
    """`(source file, body)` for every `class cls` / `interface cls` across the
    public `.d.ts` surface, in PRECEDENCE order: the package entry first (its
    `declare module` augmentation overrides), then each re-exported sibling
    (the napi-generated `index.d.ts`). The source file is carried so a conflict
    between an entry augmentation and a re-exported generated declaration can be
    reported with both locations."""
    out: list[tuple[pathlib.Path, str]] = []
    cls_re = re.compile(r"\b(?:class|interface)\s+" + re.escape(cls) + r"\b[^{]*\{")
    for f in _ts_dts_files(dts):
        text = _read_source(f)
        for m in cls_re.finditer(text):
            out.append((f, _balanced_body(text, m.end())))
    return out


def _ts_dts_class_bodies(dts: pathlib.Path, cls: str) -> list[str]:
    """Every `class cls` / `interface cls` body across the public `.d.ts`
    surface (entry + re-exported siblings). A class can appear more than once
    — the napi `index.d.ts` declaration plus a `declare module` augmentation
    in the wrapper — so all bodies are returned and the member is looked up in
    each."""
    bodies = [body for _, body in _ts_dts_class_bodies_with_src(dts, cls)]
    return bodies


# A member declaration in DECLARATION position inside a `.d.ts` class body:
# line-leading (after any leading modifiers), never a member-access
# (`foo.method`). The napi `index.d.ts` separates members by newline (no `;`),
# the hand-written augmentations by `;`, so the anchor is line-start under
# MULTILINE rather than a fixed terminator char. The modifier prefix covers
# every shape napi-rs / the augmentations emit: `static` factories
# (`static fromFile(...)`), `get`/`set` accessors (`get kind(): string` — the
# napi `#[getter]` form), and `readonly` properties. The char right after the
# name picks the form: `(` → method (a `get`/`set` accessor reads as a zero-arg
# method, which is the correct property shape), `?`/`:` → bare property.
def _ts_dts_member_decl_re(method_camel: str) -> re.Pattern[str]:
    return re.compile(
        r"(?m)^\s*(?:(?:static|readonly|get|set|public|abstract|declare)\s+)*"
        + re.escape(method_camel)
        + r"\s*(?P<kind>[(?:])"
    )


def _sig_extract_ts_dts(dts: pathlib.Path, cls: str, method_camel: str) -> tuple[list[str], str] | None:
    """TypeScript `.d.ts` signature: the member named `method_camel` inside
    `class cls` / `interface cls`, anywhere across the public `.d.ts` surface
    (the package entry plus the `index.d.ts` it re-exports), in either form —

      * a method call:    `methodCamel(<params>): <ret>` → those params + ret,
      * a property:       `[readonly] methodCamel: <Type>` → a zero-arg
        signature returning `<Type>` (the columnar reader's `readonly schema`
        / `readonly dropped` accessors are properties, not `fn`s).

    Returns None when the `.d.ts` is absent or the class / member is not
    found. Absence is NOT silently a pass: `_sig_dts_public_member_missing`
    promotes a missing declaration to an error for a row whose member IS part
    of the public `.d.ts` surface, so dropping a declaration fails the gate;
    only a member genuinely absent from the public `.d.ts` (a napi-only
    streaming row whose `.d.ts` shape is not redeclared) degrades to
    napi-as-authority."""
    decls = _sig_dts_all_decls(dts, cls, method_camel)
    # Precedence: the package-entry augmentation (declared first across the
    # surface) overrides the re-exported generated declaration, matching how
    # `_ts_dts_class_bodies_with_src` orders the surface.
    return decls[0][1] if decls else None


def _sig_parse_dts_member(body: str, dm: "re.Match[str]") -> tuple[list[str], str]:
    """Parse the `(params, ret)` of the member matched by `dm` inside a single
    `.d.ts` class body — the method (`name(p: T): R`) or property (`name: T`,
    zero-arg) form. Shared by the single-decl extractor and the all-decls
    conflict scan so both read the surface identically.

    A `?` optional marker is canonicalised to the `T | undefined` union so it
    flows through the SAME `_sig_option_inner` path the napi-emitted
    `T | undefined | null` already uses: an `Option<T>` spec then agrees with
    `name?: T` while a `name: T` (required) stays bare and fails it. Preserving
    optionality is what makes a required-vs-optional drift visible — without it
    `options?: T` and `options: T` both reduce to `T`."""
    if dm.group("kind") == "(":
        arglist, after = _sig_balanced_parens(body, dm.end())
        rm = re.match(r"\s*:\s*([^;{\n]+)", body[after:])
        ret = rm.group(1).strip() if rm else "void"
        params: list[str] = []
        for p in _sig_split_params(arglist):
            # `name: Type` / `name?: Type` / `...name: Type` (rest). Keep the
            # Type half; carry the `?` as `| undefined` so optionality survives.
            pm = re.match(r"\s*(?:\.\.\.)?[A-Za-z_]\w*\s*(\??)\s*:\s*(.+)", p)
            if pm:
                params.append(_sig_dts_apply_optional(pm.group(2).strip(), bool(pm.group(1))))
            else:
                params.append(p.strip())
        return params, ret
    # Property form: a zero-arg accessor (`name: T`, optionally `readonly` /
    # `name?: T`). The match's `kind` group is `?` or `:`; an optional property
    # carries the same `| undefined` so a required→optional property drift fails.
    colon = body.index(":", dm.start())
    rm = re.match(r"\s*([^;{\n]+)", body[colon + 1 :])
    ret = rm.group(1).strip() if rm else "void"
    return [], _sig_dts_apply_optional(ret, dm.group("kind") == "?")


def _sig_dts_apply_optional(ty: str, optional: bool) -> str:
    """Canonicalise an optional `.d.ts` type to `T | undefined` so it flows
    through `_sig_option_inner`. A type that already ends in a nullable union
    member (`T | undefined` / `T | null`, the napi spelling) is left as-is — the
    `?` is redundant with the explicit union, and a doubled `| undefined` would
    be noise."""
    if optional and not re.search(r"\|\s*(?:undefined|null)\s*$", ty):
        return f"{ty} | undefined"
    return ty


def _sig_dts_all_decls(
    dts: pathlib.Path, cls: str, method_camel: str
) -> list[tuple[pathlib.Path, tuple[list[str], str]]]:
    """EVERY `(source file, (params, ret))` declaration of `cls.method_camel`
    across the public `.d.ts` surface, in precedence order (entry first). A TS
    `declare module` interface→class augmentation MERGES with the generated
    class declaration as OVERLOADS rather than replacing it, so a pinned member
    can legitimately resolve to its entry-augmentation while a stale generated
    overload of a different return still rides along in `index.d.ts`. Returning
    all of them lets the gate enforce that the surviving public surface carries
    no conflicting declaration.

    EVERY declaration in each body is collected (`finditer`, not `search`):
    overloads of one method can sit in the same `interface`/`class` body
    (`batches(o?): Promise<RecordBatchStream>;` then a stale
    `batches(o?): Promise<Handle>;`), so a per-body single match would examine
    only the first and let a conflicting in-body sibling pass."""
    decl_re = _ts_dts_member_decl_re(method_camel)
    out: list[tuple[pathlib.Path, tuple[list[str], str]]] = []
    for src, body in _ts_dts_class_bodies_with_src(dts, cls):
        for dm in decl_re.finditer(body):
            out.append((src, _sig_parse_dts_member(body, dm)))
    return out


def _sig_dts_conflicting_decls(
    dts: pathlib.Path, cls: str, method_camel: str, spec: tuple[list[str], str]
) -> list[tuple[pathlib.Path, tuple[list[str], str]]]:
    """The SIBLING public-surface declarations of `cls.method_camel` that do NOT
    satisfy `spec`. The precedence-winning declaration (index 0) is compared
    against the spec by the caller, so it is skipped here; this catches a
    drifting sibling — e.g. a generated `index.d.ts` overload still returning the
    raw handle while the entry augmentation presents the wrapper. Because the two
    merge as overloads, the raw one re-leaks even though the resolved type looks
    correct, so any sibling that disagrees with the spec must fail the gate."""
    spec_params, spec_ret = spec
    bad: list[tuple[pathlib.Path, tuple[list[str], str]]] = []
    for src, (params, ret) in _sig_dts_all_decls(dts, cls, method_camel)[1:]:
        if len(params) != len(spec_params) or not all(
            _sig_type_agrees(sp, ap, "ts_dts") for sp, ap in zip(spec_params, params)
        ):
            bad.append((src, (params, ret)))
            continue
        if not _sig_type_agrees(spec_ret, _sig_unwrap_result(ret), "ts_dts"):
            bad.append((src, (params, ret)))
    return bad


def _sig_dts_public_member_missing(dts: pathlib.Path, cls: str, method_camel: str) -> bool:
    """Is `cls.method_camel` part of the public `.d.ts` surface yet missing
    its declaration? True only when the CLASS is declared somewhere in the
    public surface but the MEMBER is not — i.e. a member that belongs in the
    `.d.ts` was dropped. False when the class itself is absent (a row that is
    legitimately napi-only at the `.d.ts` level — its class is not part of the
    declared surface, so there is nothing to drop and the check degrades to
    napi-as-authority)."""
    return bool(_ts_dts_class_bodies(dts, cls)) and (
        _sig_extract_ts_dts(dts, cls, method_camel) is None
    )


def _sig_extract_cpp(hpp: pathlib.Path, cls: str, method: str) -> tuple[list[str], str] | None:
    """C++ signature: the `method(<params>)` *declaration* inside `class cls`.
    The return type is read from the text immediately left of the method name,
    bounded by the previous statement terminator / brace / access specifier.

    The class header is matched line-anchored (`^class cls`), not as a bare
    `\\bclass cls`: an elaborated-type-specifier param like
    `const class FluentSubscription& sub` would otherwise be the first match
    and `[^{]*\\{` would bridge to an unrelated method body, resolving the
    wrong class. The method is matched in DECLARATION position (a return-type
    token immediately precedes the name) so a member-access CALL inside an
    earlier inline body — `handle_.get()` inside `size()` — never shadows the
    real `get()` declaration, mirroring `_collect_cpp_class_methods`."""
    if not hpp.is_file():
        return None
    text = _read_cpp_expanded(hpp)
    cls_re = re.compile(r"^class\s+" + re.escape(cls) + r"\b\s*(?::[^{]*)?\{", re.MULTILINE)
    m = cls_re.search(text)
    if not m:
        return None
    body = _balanced_body(text, m.end())
    # Try the bare member name, then the `get_`-prefixed readback form: a C++
    # `Config` readback getter carries a uniform `get_` prefix (`get_flush_mode`)
    # against the bare `flush_mode` row, exactly as the forward presence check
    # accepts `<snake>` or `get_<snake>`.
    for candidate in (method, f"get_{method}"):
        sig = _sig_extract_cpp_member(body, candidate)
        if sig is not None:
            return sig
    return None


def _sig_extract_cpp_member(body: str, method: str) -> tuple[list[str], str] | None:
    """The `method(<params>)` *declaration* within a resolved C++ class body.

    Match in DECLARATION position: a return-type token (an identifier, or a
    `>`/`*`/`&`/`]` ending a templated/pointer/reference/array return) sits just
    before the method name. A bare `.method(` call or a keyword-fronted
    `return method(` is rejected — the same discipline the method collector uses
    to defeat the in-body call-site shadow (the G11 bypass class)."""
    decl_re = re.compile(
        r"(?P<prev>[A-Za-z_]\w*|[>*&\]])\s+" + re.escape(method) + r"\s*\("
    )
    for cm in decl_re.finditer(body):
        if cm.group("prev") in _SIG_CPP_CALL_PREV_KEYWORDS:
            continue
        # The return-type run ends where the method name begins.
        name_at = body.rfind(method, cm.start(), cm.end())
        ret = _sig_cpp_return_before(body[:name_at])
        if ret is None:
            continue
        arglist, _ = _sig_balanced_parens(body, cm.end())
        return [_sig_cpp_param_type(p) for p in _sig_split_params(arglist)], ret
    return None


_SIG_CPP_ACCESS_KEYWORDS = ("public", "private", "protected")


def _sig_cpp_return_before(prefix: str) -> str | None:
    """The C++ return-type run ending where the method name begins (the end of
    `prefix`). Bounded left by the last `;`/`{`/`}` or access-specifier label,
    with a leading `virtual`/`static`/`inline`/`constexpr` specifier dropped.
    Returns None when nothing precedes (a constructor)."""
    seg = re.split(r"[;{}]", prefix)[-1]
    # A preprocessor directive (`#ifdef THETADATADX_CPP_ARROW`, `#endif`)
    # carries no `;`/`{`/`}`, so a method guarded by one — the Arrow-gated
    # `Stream::batches` — would otherwise prepend the whole directive run to
    # its return type. The directive line is a boundary, not a return token.
    seg = re.sub(r"(?m)^\s*#.*$", "", seg)
    for kw in _SIG_CPP_ACCESS_KEYWORDS:
        seg = re.sub(rf".*\b{kw}\s*:", "", seg, flags=re.S)
    ret = re.sub(
        r"^(?:virtual|static|inline|constexpr)\s+", "", seg.strip()
    ).strip()
    return ret or None


def _sig_cpp_param_type(param: str) -> str:
    """The type half of a C++ `Type name` / `Type name = default` parameter
    (the trailing identifier + any default dropped, `*`/`&`/`const` kept)."""
    s = re.sub(r"=.*$", "", param.strip()).strip()
    m = re.match(r"(.+?)\s+[A-Za-z_]\w*\s*$", s)
    return m.group(1).strip() if m else s


def _sig_extract_rust(client_rs: pathlib.Path, struct: str, method: str) -> tuple[list[str], str] | None:
    """Rust core signature: the `pub fn method` / `pub async fn method` inside
    an `impl struct` block in `client.rs`. Receivers / the `py` token stripped;
    the return is the `-> T` (defaulting to `()`)."""
    if not client_rs.is_file():
        return None
    text = _read_source(client_rs)
    impl_re = re.compile(r"impl\s+" + re.escape(struct) + r"\b[^{]*\{")
    for h in impl_re.finditer(text):
        sig = _sig_rust_fn(_balanced_body(text, h.end()), method, require_pub=True)
        if sig is not None:
            return sig
    return None


def _sig_extract_ffi(ffi_src: pathlib.Path, symbol: str) -> tuple[list[str], str] | None:
    """FFI signature: the `extern "C" fn thetadatadx_<symbol>(<params>) -> ret`
    parameter list under `thetadatadx-ffi/src/**`. No receiver stripping — a C ABI fn has
    none; the return defaults to `()` (no `-> T`)."""
    if not ffi_src.is_dir():
        return None
    sym = "thetadatadx_" + symbol
    for rs in ffi_src.rglob("*.rs"):
        text = _read_source(rs)
        m = re.search(r"\bfn\s+" + re.escape(sym) + r"\s*\(", text)
        if not m:
            continue
        arglist, after = _sig_balanced_parens(text, m.end())
        rm = re.match(r"\s*->\s*([^\{;]+)", text[after:])
        ret = rm.group(1).strip() if rm else "()"
        return [_sig_rust_param_type(p) for p in _sig_split_params(arglist)], ret
    return None


# A `[method.signature]` row pins, per signature-lang, the params + return.
# The canonical `params` / `returns` apply to every enrolled lang; a
# `<lang>_params` / `<lang>_returns` override replaces them for that lang
# (type-map-justified divergence the map alone cannot encode — a napi arity
# change, the FFI `_explicit (has_value, n)` split). A lang the spec pins
# NEITHER canonical nor override for is not signature-checked. A lang named in
# the spec's `skip_langs` list is also not checked even when canonical params /
# returns are present: the binding exposes the member, but not in a form this
# lang's extractor reads — the `ts_napi` view of a JS-wrapper-only class
# (`RecordBatchStream`), which ships as a `.d.ts` interface with NO napi Rust
# `fn` for the extractor to find.
def _sig_spec_for(signature: dict[str, Any], lang: str) -> tuple[list[str], str] | None:
    if lang in signature.get("skip_langs", ()):
        return None
    # Override-key precedence per lang. `python_pyi` (the stub lane) shares the
    # one TypeScript-like split with the pyo3-source `python` lane: a
    # `python_*` override is a per-binding divergence both python views inherit,
    # so the stub lane falls back `python_pyi_*` → `python_*` → canonical. Every
    # other lang reads `<lang>_*` → canonical.
    key_chain = (
        ("python_pyi", "python") if lang == "python_pyi" else (lang,)
    )

    def _resolve(suffix: str) -> Any:
        for key in key_chain:
            if f"{key}_{suffix}" in signature:
                return signature[f"{key}_{suffix}"]
        return signature.get(suffix)

    params = _resolve("params")
    returns = _resolve("returns")
    if params is None and returns is None:
        return None
    return list(params or []), returns if returns is not None else "()"


def _sig_compare_one(
    label: str,
    spec: tuple[list[str], str],
    actual: tuple[list[str], str] | None,
    lang: str,
) -> list[str]:
    """Compare one binding's extracted `(params, ret)` against `spec` via the
    type map. Surfaces arity, per-position type (order-sensitive), and return
    drift, each as a human-readable string."""
    spec_params, spec_ret = spec
    if actual is None:
        return [
            f"  {label}.{lang}: `[method.signature]` pins this binding but no "
            f"`{lang}` declaration was extracted (method absent / unparsable)."
        ]
    act_params, act_ret = actual
    errors: list[str] = []
    if len(spec_params) != len(act_params):
        errors.append(
            f"  {label}.{lang}: arity mismatch — spec pins {len(spec_params)} "
            f"param(s) {spec_params!r}, actual declares {len(act_params)} "
            f"{act_params!r}."
        )
    else:
        for idx, (sp, ap) in enumerate(zip(spec_params, act_params)):
            if not _sig_type_agrees(sp, ap, lang):
                errors.append(
                    f"  {label}.{lang}: param #{idx} type mismatch — spec `{sp}` "
                    f"is not satisfied by actual `{ap}` under the {lang} type map."
                )
    # The actual return is unwrapped of its fallible-result wrapper first
    # (`PyResult<T>` / `napi::Result<T>` → `T`): fallibility is a per-binding
    # surface property, not a cross-binding return-type contract.
    if not _sig_type_agrees(spec_ret, _sig_unwrap_result(act_ret), lang):
        errors.append(
            f"  {label}.{lang}: return mismatch — spec `{spec_ret}` is not "
            f"satisfied by actual `{act_ret}` under the {lang} type map."
        )
    return errors


# Which signature-langs a `[[method]]` row can be signature-checked on, mapped
# from the row's presence booleans. `ts_napi` + `ts_dts` both derive from the
# row's `typescript` flag; `rust` from the `rust` flag (its class must also be
# Rust-mappable). The extractor for each lang resolves the row's class to the
# per-binding lookup the forward presence check already uses.
def _sig_check_method_signatures(
    method_rows: list[dict[str, Any]],
    *,
    py_src: pathlib.Path,
    pyi_path: pathlib.Path,
    ts_src: pathlib.Path,
    ts_dts: pathlib.Path,
    cpp_hpp: pathlib.Path,
    client_rs: pathlib.Path,
    ffi_src: pathlib.Path,
) -> list[str]:
    """Signature-level gate for `[[method]]` rows. FAIL-CLOSED enrollment: every
    `[[method]]` row MUST carry a `[method.signature]` sub-table OR a
    `NAME_ONLY_METHOD_ALLOWLIST` entry — a row with neither fails the gate, so a
    new row can never be silently name-only (its signature drift would otherwise
    hide while the gate stays green). For each row WITH a sub-table, extract
    every enrolled binding's declared signature and verify it satisfies the spec
    through the TYPE_MAP + per-binding overrides.

    A row's enrolled signature-langs are derived from its presence booleans
    (`python` → python + python_pyi, `typescript` → ts_napi + ts_dts, `cpp` →
    cpp, `rust` → rust) intersected with what the spec actually pins (canonical
    or a `<lang>_params`/`<lang>_returns` override). The FFI symbol is checked
    only when the row supplies an `ffi_symbol` key naming the extern (a method
    row has no FFI presence boolean — its C-ABI shape is a `[[ffi_symbol]]`
    concern), so an FFI signature is opt-in within the opt-in.

    The `python` flag drives TWO lanes: `python` reads the pyo3 Rust source
    (the runtime contract), `python_pyi` reads the shipped PEP 561 stub (the
    client-facing type surface). The stub lane checks params + RETURN against
    the cross-binding spec; its return check is coverage Gate 6's stubtest
    cannot give (a compiled pyo3 method has no runtime return annotation, so
    stubtest validates only the stub-vs-runtime parameter list / arity).
    """
    errors: list[str] = []
    for row in method_rows:
        class_name = row.get("class")
        camel = row.get("name")
        signature = row.get("signature")
        if not signature:
            # Fail-closed: a name-only row is allowed ONLY if it is explicitly
            # enrolled in the allowlist (with its stated reason). Otherwise its
            # signature is unpinned and a drift would hide — that is a gate
            # failure, so the row must either grow a `[method.signature]` or
            # earn an allowlist entry.
            if (class_name, camel) not in NAME_ONLY_METHOD_ALLOWLIST:
                errors.append(
                    f"  {class_name}.{camel}: `[[method]]` row has neither a "
                    f"`[method.signature]` sub-table nor a "
                    f"`NAME_ONLY_METHOD_ALLOWLIST` entry — pin its signature "
                    f"(preferred) or allowlist it with a documented reason so "
                    f"no row is silently name-only."
                )
            continue
        if not class_name or not camel:
            errors.append(
                f"  [[method]] row with `[method.signature]` missing `class`/"
                f"`name`: {row!r}"
            )
            continue
        snake = _camel_to_snake(camel)
        label = f"{class_name}.{camel}"
        override = METHOD_BINDING_OVERRIDES.get((class_name, camel))

        # Resolve each enrolled binding's (class/member) the same way the
        # forward presence check does, then extract + compare its signature.
        if row.get("python"):
            spec = _sig_spec_for(signature, "python")
            if spec is not None:
                py_cls, py_member = (
                    override["python"] if override and "python" in override
                    else (_py_class_for(class_name), snake)
                )
                errors += _sig_compare_one(
                    label, spec, _sig_extract_python(py_src, py_cls, py_member), "python"
                )
            # The PEP 561 stub lane. The stub uses the PUBLIC Python class
            # names (`Contract`, not the pyo3 pyclass `PyContract`), so resolve
            # against the parity-toml class directly; a `python` override
            # already targets the public stub class + member (e.g.
            # `flatFileToPath` → `Client.flatfile_to_path`,
            # `count` → `FlatFileRowList.__len__`), so reuse it when present.
            # DIVISION OF LABOUR: Gate 6's stubtest checks the stub against the
            # RUNTIME (params + arity); this lane checks it against the
            # cross-binding SPEC (params + RETURN) — the return is the coverage
            # stubtest lacks, since a compiled pyo3 method exposes no runtime
            # return annotation.
            spec_pyi = _sig_spec_for(signature, "python_pyi")
            if spec_pyi is not None and (class_name, camel) not in PYI_SETTER_PROPERTY_ROWS:
                pyi_cls, pyi_member = (
                    override["python"] if override and "python" in override
                    else (class_name, snake)
                )
                actual_pyi = _sig_extract_python_pyi(pyi_path, pyi_cls, pyi_member)
                if actual_pyi is not None:
                    errors += _sig_compare_one(label, spec_pyi, actual_pyi, "python_pyi")
                elif _sig_pyi_public_member_missing(pyi_path, pyi_cls, pyi_member):
                    # The class is a fully-enumerated public stub class (no
                    # `__getattr__` escape) but the member's declaration is
                    # gone — a dropped public stub member fails the gate. (A
                    # class absent from the stub, or one with a `__getattr__`
                    # fallback, degrades to the `python` lane + stubtest.)
                    errors.append(
                        f"  {label}.python_pyi: `[method.signature]` pins this "
                        f"binding and `{pyi_cls}` is a fully-enumerated public "
                        f"stub class, but no `{pyi_member}` declaration was "
                        f"found in `__init__.pyi` — a removed public stub "
                        f"declaration must fail the gate."
                    )
        if row.get("typescript"):
            ts_cls, ts_member = (
                override["typescript"] if override and "typescript" in override
                else (_ts_class_for(class_name), camel)
            )
            spec_napi = _sig_spec_for(signature, "ts_napi")
            if spec_napi is not None:
                errors += _sig_compare_one(
                    label, spec_napi, _sig_extract_ts_napi(ts_src, ts_cls, ts_member), "ts_napi"
                )
            spec_dts = _sig_spec_for(signature, "ts_dts")
            if spec_dts is not None:
                actual_dts = _sig_extract_ts_dts(ts_dts, ts_cls, ts_member)
                if actual_dts is not None:
                    errors += _sig_compare_one(label, spec_dts, actual_dts, "ts_dts")
                    # The precedence-winning declaration above is the resolved
                    # public type, but a `declare module` augmentation MERGES
                    # with the generated class declaration as overloads — so a
                    # stale re-exported declaration of a different return rides
                    # along and re-leaks. Every public-surface declaration must
                    # satisfy the spec; report each one that drifts.
                    for src, (p_act, r_act) in _sig_dts_conflicting_decls(
                        ts_dts, ts_cls, ts_member, spec_dts
                    ):
                        errors.append(
                            f"  {label}.ts_dts: conflicting public declaration in "
                            f"`{src.name}` — `{ts_member}({', '.join(p_act)}): "
                            f"{r_act}` does not satisfy `[method.signature]`. A "
                            f"`.d.ts` augmentation merges with the generated "
                            f"declaration as overloads, so this drifting "
                            f"declaration re-exposes a non-client-facing return "
                            f"alongside the intended one — the public surface "
                            f"must carry a single consistent signature."
                        )
                elif _sig_dts_public_member_missing(ts_dts, ts_cls, ts_member):
                    # The class IS part of the public `.d.ts` surface but the
                    # member's declaration is gone — dropping a pinned public
                    # declaration fails the gate. (A row whose class is not in
                    # the declared surface degrades to napi-as-authority.)
                    errors.append(
                        f"  {label}.ts_dts: `[method.signature]` pins this "
                        f"binding and `{ts_cls}` is declared in the public "
                        f".d.ts, but no `{ts_member}` declaration was found — "
                        f"a removed public declaration must fail the gate."
                    )
        if row.get("cpp"):
            spec = _sig_spec_for(signature, "cpp")
            if spec is not None:
                cpp_cls, cpp_member = (
                    override["cpp"] if override and "cpp" in override
                    else (_cpp_class_for(class_name), snake)
                )
                errors += _sig_compare_one(
                    label, spec, _sig_extract_cpp(cpp_hpp, cpp_cls, cpp_member), "cpp"
                )
        if row.get("rust"):
            spec = _sig_spec_for(signature, "rust")
            if spec is not None:
                if override and "rust" in override:
                    rust_cls, rust_member = override["rust"]
                elif class_name in RUST_METHOD_CLASS:
                    rust_cls, rust_member = RUST_METHOD_CLASS[class_name], snake
                else:
                    rust_cls, rust_member = None, snake
                if rust_cls is not None:
                    errors += _sig_compare_one(
                        label, spec, _sig_extract_rust(client_rs, rust_cls, rust_member), "rust"
                    )
        ffi_symbol = row.get("ffi_symbol")
        if ffi_symbol:
            spec = _sig_spec_for(signature, "ffi")
            if spec is not None:
                errors += _sig_compare_one(
                    label, spec, _sig_extract_ffi(ffi_src, ffi_symbol), "ffi"
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
    historical_async_rows: list[dict[str, Any]] = data.get(
        "historical_async", []
    )
    historical_base_rows: list[dict[str, Any]] = data.get("historical_base", [])
    from_file_rows: list[dict[str, Any]] = data.get("from_file", [])
    connect_rows: list[dict[str, Any]] = data.get("connect", [])
    ffi_symbol_rows: list[dict[str, Any]] = data.get("ffi_symbol", [])
    if not rows:
        print("parity.toml has no [[class]] rows", file=sys.stderr)
        return 1

    py_classes = collect_python_classes(PY_SRC)
    # The TypeScript surface is read in three parts so a row's class-presence
    # verdict rests on what actually ships at runtime, not on a declaration the
    # runtime entry may have dropped. `ts_classes` (the union) is retained only
    # for the name-universe consumers (anchor rows, the vocab scan).
    ts_declared_classes, ts_declared_interfaces = _collect_ts_dts_class_kinds(TS_DTS)
    ts_runtime_exports = _collect_ts_runtime_classes(TS_DTS)
    ts_classes = collect_typescript_classes(TS_DTS)
    cpp_classes = collect_cpp_classes(CPP_HPP)

    py_setters = _collect_python_setters(PY_SRC)
    ts_setters = _collect_typescript_setters(TS_SRC)
    cpp_setters = _collect_cpp_setters(CPP_HPP, CPP_H)
    ffi_setters = _collect_ffi_setters(FFI_SRC)

    rust_fields = _collect_rust_pub_fields(CONFIG_DIR)

    py_class_methods = _collect_python_class_methods(PY_SRC)
    ts_class_methods = _collect_typescript_class_methods(TS_SRC, TS_PKG_DIR)
    cpp_class_methods = _collect_cpp_class_methods(CPP_HPP)
    rust_view_methods = _collect_rust_view_methods(CORE_CLIENT_RS)

    declared_names: set[str] = {row["name"] for row in rows}

    # Class-level mismatches (non-dotted rows). The TypeScript column is held
    # to the runtime export, not the declaration, so a class dropped from the
    # shipped package surface trips even while the `.d.ts` still declares it.
    class_mismatches = _check_class_rows(
        rows,
        py_classes,
        cpp_classes,
        ts_declared_classes,
        ts_declared_interfaces,
        ts_runtime_exports,
    )

    # Field-level mismatches (dotted rows / #595). The class universe lets
    # the dotted-row check validate documentation-anchor rows (an anchor on
    # a typo'd / nonexistent struct fails instead of silently passing).
    anchor_classes = py_classes | ts_classes | cpp_classes
    field_errors = _check_dotted_rows(
        rows, py_setters, ts_setters, cpp_setters, ffi_setters, anchor_classes
    )

    # Method-level mismatches (per-method `[[method]]` rows on the
    # load-bearing user-facing classes — `Client`,
    # `StreamingClient`, `Credentials`, `Config`).
    method_errors = _check_method_rows(
        method_rows,
        py_class_methods,
        ts_class_methods,
        cpp_class_methods,
        rust_view_methods,
    )

    # Signature-level method parity: a `[[method]]` row may carry an optional
    # `[method.signature]` sub-table pinning the actual params + return across
    # the enrolled bindings (Route B). The check extracts each binding's
    # declared signature from source and verifies it through the canonical-Rust
    # → per-binding TYPE_MAP. It is opt-in per row, so it is a no-op on every
    # row that does not (yet) carry a `[method.signature]` — the name-only
    # check is unchanged for the rest.
    method_signature_errors = _sig_check_method_signatures(
        method_rows,
        py_src=PY_SRC,
        pyi_path=PY_PYI,
        ts_src=TS_SRC,
        ts_dts=TS_DTS,
        cpp_hpp=CPP_HPP,
        client_rs=CORE_CLIENT_RS,
        ffi_src=FFI_SRC,
    )

    # Reverse-direction orphan check on the core streaming surfaces: a
    # public observability accessor (counter / ring telemetry / the
    # slow-callback threshold setter) wired onto the core `StreamSurface`
    # view or the standalone `StreamingClient` MUST carry a `[[method]]`
    # row. Without this, a wired-but-unenrolled knob reaches none of the
    # bindings and no forward check fires, because the row simply does not
    # exist. This closes that blind spot at its source — the Rust surface.
    core_streaming_methods = _collect_core_streaming_observability_methods(
        CORE_CLIENT_RS, CORE_FPSS_MOD_RS
    )
    core_streaming_errors = _check_core_streaming_method_rows(
        core_streaming_methods, method_rows
    )

    # Orphan Rust pub fields (no parity row).
    orphan_errors = _check_orphan_rust_fields(rust_fields, rows)

    # Value-field TYPE parity ([[value_field]] rows).
    value_field_errors = _check_value_field_rows(value_field_rows)

    # Value-field exhaustiveness: the reverse direction of the per-row type
    # check. A load-bearing class missing a pinned unit/identity field, or a
    # new unit-bearing field name shipped on a binding value struct without
    # any enrolling row, trips here — the `strike_thousandths` defect class.
    value_field_roster_errors = _check_value_field_roster(value_field_rows)

    # Free-function (utility) parity ([[utility]] rows) — the standalone
    # condition / exchange / calendar / sequence lookups and converters are
    # free functions / namespace methods, tracked here because they are not
    # methods on any class the `[[method]]` rows cover.
    py_utils = _collect_python_utility_functions(PY_SRC)
    # The TS utility surface spans napi free functions and the `Util`
    # namespace-class static methods (the lookups); merge both so every
    # cross-binding row resolves regardless of TS shape.
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
    # roster, so none drifts untracked).
    utility_roster_errors = _check_utility_roster_complete(
        utility_rows, py_utils, ts_utils
    )

    # Historical server-stream surface ([[historical_streaming]] rows) —
    # the `.stream(handler)` / `<endpoint>Stream` / `thetadatadx_<endpoint>_stream`
    # terminal per endpoint. These live on per-endpoint builders or as
    # endpoint-named methods, NOT on a class the `[[method]]` rows cover,
    # so they would otherwise drift silently across bindings.
    # The Rust historical surface is the registry of record
    # (`endpoint_surface.toml`): the build pipeline generates every binding's
    # historical method from it, so a dropped / renamed Rust endpoint must
    # trip the gate. The buffered set is the async + base surface; the
    # streamable subset (mirroring the build's `endpoint_streams` SSOT) is
    # the streaming surface.
    rust_buffered = _collect_rust_buffered_endpoints(ENDPOINT_SURFACE_TOML)
    rust_stream = _collect_rust_streaming_endpoints(ENDPOINT_SURFACE_TOML)

    py_stream = _collect_python_streaming_endpoints(PY_SRC)
    ts_stream = _collect_typescript_streaming_endpoints(ts_class_methods)
    cpp_stream = _collect_cpp_streaming_endpoints(cpp_class_methods)
    ffi_stream = _collect_ffi_streaming_endpoints(FFI_SRC)
    historical_streaming_errors = _check_historical_streaming_rows(
        historical_streaming_rows,
        rust_stream,
        py_stream,
        ts_stream,
        cpp_stream,
        ffi_stream,
    )

    # Historical async query surface ([[historical_async]] rows) — the
    # non-blocking `<endpoint>_async` / Promise companion per endpoint.
    # Python / C++ name an `<endpoint>_async` method; TypeScript's buffered
    # method is itself async (Promise). No C ABI row: the async surface is
    # a binding-layer concern over the existing blocking C symbols.
    py_async = _collect_python_async_endpoints(PY_SRC)
    ts_async = _collect_typescript_async_endpoints(ts_class_methods)
    cpp_async = _collect_cpp_async_endpoints(cpp_class_methods)
    historical_async_errors = _check_historical_async_rows(
        historical_async_rows, rust_buffered, py_async, ts_async, cpp_async
    )

    # TypeScript projected-Arrow reachability — every buffered columnar endpoint
    # must surface a `<endpoint>WithColumns` variant returning the response's
    # presentColumns / symbol alongside the rows, so the projected Arrow-IPC
    # exit is drivable from a live call (parity with Python's presence-carrying
    # `<Tick>List` and the C / C++ `_with_options` presence out-param). Read
    # from the committed generated napi source, the deterministic no-build
    # counterpart of the streaming / base families' `endpoint_surface.toml`.
    ts_with_columns = _collect_typescript_with_columns_endpoints(
        TS_HISTORICAL_METHODS_RS
    )
    rust_columnar_buffered = _collect_rust_columnar_buffered_endpoints(
        ENDPOINT_SURFACE_TOML
    )
    ts_with_columns_errors = _check_typescript_with_columns_reachability(
        rust_columnar_buffered, ts_with_columns
    )

    # Historical buffered base surface ([[historical_base]] rows) — the
    # blocking query terminal per endpoint on ALL FIVE surfaces: the Rust
    # `HistoricalClient::<endpoint>` method (registry of record), the Python
    # `<Endpoint>Builder.list()` collect, the buffered TypeScript method, the
    # C++ `Historical::<endpoint>` member, and the C-ABI
    # `thetadatadx_<endpoint>_with_options` base symbol read from the SHIPPED
    # header. This is the core "every endpoint exists everywhere" guarantee,
    # previously only implicit via the async / stream companions and unchecked
    # entirely on the C-ABI base.
    py_buffered = _collect_python_buffered_endpoints(PY_SRC)
    ts_buffered = _collect_typescript_async_endpoints(ts_class_methods)
    cpp_buffered = cpp_class_methods.get(_cpp_class_for("HistoricalView"), set())
    cabi_base = _collect_cabi_base_endpoints(ENDPOINT_WITH_OPTIONS_INC)
    ffi_base = _collect_ffi_base_endpoints(FFI_SRC)
    historical_base_errors = _check_historical_base_rows(
        historical_base_rows,
        rust_buffered,
        py_buffered,
        ts_buffered,
        cpp_buffered,
        cabi_base,
        ffi_base,
    )

    # Client construction-from-file surface ([[from_file]] rows) — the
    # one-call `from_file` / `connectFromFile` / `thetadatadx_*_connect_from_file`
    # convenience per standalone client class. Its spelling differs per
    # binding, so it cannot ride a `[[method]]` row; this family pins it
    # across Python / TypeScript / C++ / the C ABI.
    py_from_file = _collect_python_from_file_classes(py_class_methods)
    ts_from_file = _collect_typescript_from_file_classes(ts_class_methods)
    cpp_from_file = _collect_cpp_from_file_classes(cpp_class_methods)
    ffi_from_file_stems = _collect_ffi_from_file_stems(FFI_SRC)
    from_file_errors = _check_from_file_rows(
        from_file_rows, py_from_file, ts_from_file, cpp_from_file, ffi_from_file_stems
    )

    # Client construction surface ([[connect]] rows) — the base
    # `Client(creds, config)` / `Client.connect(...)` / `Client::connect(...)`
    # / `thetadatadx_<stem>_connect` entry point per standalone client. Its
    # spelling differs per binding (Python `#[new]` constructor vs the
    # `connect` factory elsewhere), so it cannot ride a `[[method]]` row;
    # this family pins it across Python / TypeScript / C++ / the C ABI plus
    # the Python-only `AsyncClient`.
    py_connect = _collect_python_connect_classes(PY_SRC)
    ts_connect = _collect_typescript_connect_classes(ts_class_methods)
    cpp_connect = _collect_cpp_connect_classes(cpp_class_methods)
    ffi_connect_stems = _collect_ffi_connect_stems(FFI_SRC)
    connect_errors = _check_connect_rows(
        connect_rows, py_connect, ts_connect, cpp_connect, ffi_connect_stems
    )

    # Inline builder parity: every public Rust `ClientBuilder` fluent
    # setter must exist on the C++ `ClientBuilder`, and vice versa.
    rust_client_builder_setters = _collect_rust_client_builder_setters(
        RUST_CLIENT_BUILDER_RS
    )
    cpp_client_builder_setters = _collect_cpp_client_builder_setters(CPP_HPP)
    client_builder_setter_errors = _check_client_builder_setter_parity(
        rust_client_builder_setters, cpp_client_builder_setters
    )

    # TypeScript inline connect roster: `Client.connectWith(...)` must
    # continue to expose exactly the canonical option fields.
    ts_connect_with_fields = _collect_typescript_connect_with_fields(TS_LIB_RS)
    connect_with_field_errors = _check_typescript_connect_with_field_roster(
        ts_connect_with_fields
    )

    # Credentials factory surface — the auth-handle factories
    # (`fromFile` / `fromEmail` / `fromApiKey` / `fromApiKeyWithEmail` /
    # `fromEnvOrFile` / `fromDotenv`). Each rides a `[[method]]` row, but a forward
    # `[[method]]` check only fires when the row exists; this reverse
    # scan harvests every `Credentials` factory from all four bindings
    # and trips when one has no row, or when a governed factory is absent
    # from a binding the roster lists. A new asymmetric factory cannot
    # slip in untracked.
    py_cred_factories = _collect_python_credentials_factories(py_class_methods)
    ts_cred_factories = _collect_typescript_credentials_factories(ts_class_methods)
    cpp_cred_factories = _collect_cpp_credentials_factories(cpp_class_methods)
    ffi_cred_factories = _collect_ffi_credentials_factories(FFI_SRC)
    credentials_factory_errors = _check_credentials_factory_rows(
        method_rows,
        py_cred_factories,
        ts_cred_factories,
        cpp_cred_factories,
        ffi_cred_factories,
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
    # `THETADATADX_ERR_*` code in the C ABI. Asserts the full leaf roster + code
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

    # C-ABI symbol roster: the streaming-batch / borrowed-handle externs
    # pinned by `[[ffi_symbol]]` rows must exist, and EVERY harvested
    # `thetadatadx_*` symbol must belong to some enrolled family (the reverse
    # orphan scan). No C-ABI symbol ships untracked.
    ffi_all_symbols = _collect_ffi_all_symbols(FFI_SRC)
    ffi_symbol_errors = _check_ffi_symbol_rows(ffi_symbol_rows, ffi_all_symbols, FFI_SRC)
    ffi_symbol_orphan_errors = _check_ffi_symbol_orphans(
        ffi_all_symbols, ffi_symbol_rows
    )

    # Request-options roster: the two generated consumers of the
    # `endpoint_surface.toml` request-options SSOT (the C++ `with_*` setters
    # and the FFI `ThetaDataDxEndpointRequestOptions` struct) must carry the
    # same option set, with a `has_*` flag per scalar, anchored on the SSOT
    # global. Name/roster level; per-option TYPE parity is a later phase.
    request_options_global = _collect_endpoint_request_options(ENDPOINT_SURFACE_TOML)
    cpp_with_options = _collect_cpp_with_options(ENDPOINT_OPTIONS_HPP_INC)
    ffi_opt_fields, ffi_opt_has = _collect_ffi_request_option_fields(
        ENDPOINT_REQUEST_OPTIONS_RS
    )
    request_options_errors = _check_request_options_roster(
        request_options_global, cpp_with_options, ffi_opt_fields, ffi_opt_has
    )
    # Signature level: every option's type must agree across the SSOT, the C++
    # `with_*` parameter, and the FFI struct field (Route A — SSOT-generated, so
    # the types agree today; the check makes a future drift fail).
    request_options_errors += _check_request_options_types(
        _collect_ssot_request_option_types(ENDPOINT_SURFACE_TOML),
        _collect_cpp_with_option_types(ENDPOINT_OPTIONS_HPP_INC),
        ENDPOINT_REQUEST_OPTIONS_RS,
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
            f"mismatch(es) vs parity.toml:"
        )
        for e in class_mismatches:
            print(e)
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

    if method_signature_errors:
        had_errors = True
        print(
            f"check_binding_parity: {len(method_signature_errors)} method "
            f"SIGNATURE mismatch(es) (`[method.signature]` params/return):"
        )
        for e in method_signature_errors:
            print(e)
        print()

    if core_streaming_errors:
        had_errors = True
        print(
            f"check_binding_parity: {len(core_streaming_errors)} core "
            f"streaming observability accessor(s) lack a `[[method]]` row:"
        )
        for e in core_streaming_errors:
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

    if value_field_roster_errors:
        had_errors = True
        print(
            f"check_binding_parity: {len(value_field_roster_errors)} "
            f"value-field roster gap(s) (unit/identity field unpinned):"
        )
        for e in value_field_roster_errors:
            print(e)
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

    if historical_async_errors:
        had_errors = True
        print(
            f"check_binding_parity: {len(historical_async_errors)} "
            f"historical-async mismatch(es) (per-endpoint "
            f"`[[historical_async]]` granularity):"
        )
        for e in historical_async_errors:
            print(e)
        print()

    if historical_base_errors:
        had_errors = True
        print(
            f"check_binding_parity: {len(historical_base_errors)} "
            f"historical-base mismatch(es) (per-endpoint "
            f"`[[historical_base]]` granularity):"
        )
        for e in historical_base_errors:
            print(e)
        print()

    if ts_with_columns_errors:
        had_errors = True
        print(
            f"check_binding_parity: {len(ts_with_columns_errors)} TypeScript "
            f"projected-Arrow reachability mismatch(es) (per-endpoint "
            f"`<endpoint>WithColumns` variant):"
        )
        for e in ts_with_columns_errors:
            print(e)
        print()

    if from_file_errors:
        had_errors = True
        print(
            f"check_binding_parity: {len(from_file_errors)} "
            f"client construction-from-file mismatch(es) (per-client "
            f"`[[from_file]]` granularity):"
        )
        for e in from_file_errors:
            print(e)
        print()

    if connect_errors:
        had_errors = True
        print(
            f"check_binding_parity: {len(connect_errors)} "
            f"client construction mismatch(es) (per-client "
            f"`[[connect]]` granularity):"
        )
        for e in connect_errors:
            print(e)
        print()

    if client_builder_setter_errors:
        had_errors = True
        print(
            f"check_binding_parity: {len(client_builder_setter_errors)} "
            f"ClientBuilder fluent-setter divergence(s) between Rust and C++:"
        )
        for e in client_builder_setter_errors:
            print(e)
        print()

    if connect_with_field_errors:
        had_errors = True
        print(
            f"check_binding_parity: {len(connect_with_field_errors)} "
            f"TypeScript connectWith option-field divergence(s):"
        )
        for e in connect_with_field_errors:
            print(e)
        print()

    if credentials_factory_errors:
        had_errors = True
        print(
            f"check_binding_parity: {len(credentials_factory_errors)} "
            f"Credentials factory mismatch(es) (cross-binding auth-handle "
            f"surface):"
        )
        for e in credentials_factory_errors:
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

    if ffi_symbol_errors:
        had_errors = True
        print(
            f"check_binding_parity: {len(ffi_symbol_errors)} `[[ffi_symbol]]` "
            f"row(s) lack a matching C-ABI declaration:"
        )
        for e in ffi_symbol_errors:
            print(e)
        print()

    if ffi_symbol_orphan_errors:
        had_errors = True
        print(
            f"check_binding_parity: {len(ffi_symbol_orphan_errors)} C-ABI "
            f"symbol(s) belong to no enrolled family:"
        )
        for e in ffi_symbol_orphan_errors:
            print(e)
        print()

    if request_options_errors:
        had_errors = True
        print(
            f"check_binding_parity: {len(request_options_errors)} "
            f"request-options divergence(s) (roster or type) across the SSOT, "
            f"C++, and FFI generated consumers:"
        )
        for e in request_options_errors:
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
            "parity.toml to reflect the intended state. Every "
            "cross-binding asymmetry must be explicit + tracked."
        )
        return 1

    n_dotted = sum(1 for row in rows if "." in row["name"])
    n_class = len(rows) - n_dotted
    n_fields = sum(len(v) for v in rust_fields.values())
    n_methods = len(method_rows)
    n_method_sigs = sum(1 for r in method_rows if r.get("signature"))
    n_value_fields = len(value_field_rows)
    n_utilities = len(utility_rows)
    n_hist_stream = len(historical_streaming_rows)
    n_hist_async = len(historical_async_rows)
    n_hist_base = len(historical_base_rows)
    n_from_file = len(from_file_rows)
    n_connect = len(connect_rows)
    n_ffi_symbol = len(ffi_symbol_rows)
    print(
        f"check_binding_parity: clean "
        f"({n_class} class rows + {n_dotted} field rows + "
        f"{n_methods} method rows ({n_method_sigs} signature-pinned) + "
        f"{n_value_fields} value-field rows + "
        f"{n_utilities} utility rows + "
        f"{n_hist_stream} historical-streaming rows + "
        f"{n_hist_async} historical-async rows + "
        f"{n_hist_base} historical-base rows + "
        f"{n_from_file} from-file rows + "
        f"{n_connect} connect rows + "
        f"{n_ffi_symbol} ffi-symbol rows + "
        f"{n_fields} rust pub fields checked; "
        f"ffi_symbols={len(ffi_all_symbols)} "
        f"request_options={len(cpp_with_options - REQUEST_OPTIONS_WITH_EXEMPT)}; "
        f"py_classes={len(py_classes)} ts_classes={len(ts_classes)} "
        f"cpp_classes={len(cpp_classes)} "
        f"py_setters={len(py_setters)} ts_setters={len(ts_setters)} "
        f"cpp_setters={len(cpp_setters)} ffi_setters={len(ffi_setters)}; "
        f"getters py={len(py_getters)} ts={len(ts_getters)} "
        f"cpp={len(cpp_getters)} ffi={len(ffi_getters)}; "
        f"builder_setters rust={len(rust_client_builder_setters)} "
        f"cpp={len(cpp_client_builder_setters)}; "
        f"connectWith_fields={len(ts_connect_with_fields)}; "
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
    does not touch the live binding tree, so running it never depends
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
                "name": "HistoricalConfig.host",
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
                "name": "StreamingConfig.timeout_ms",
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
                "name": "StreamingConfig.timeout_ms",
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
                "name": "StreamingConfig.timeout_ms",
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
        # FFI emits `thetadatadx_config_set_tokio_worker_threads_explicit`;
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
                "class": "Client",
                "name": "panicCount",
                "python": True,
                "typescript": True,
                "cpp": True,
            }
        ]
        py_methods = {"Client": {"panic_count"}}
        ts_methods = {"Client": {"panicCount"}}
        cpp_methods = {"Client": {"panic_count"}}
        errors = _check_method_rows(rows, py_methods, ts_methods, cpp_methods)
        assert errors == [], f"method positive case must be silent; got {errors!r}"

    def _case_method_python_missing() -> None:
        """Declared on Python but not present in source — trips."""
        rows = [
            {
                "class": "Client",
                "name": "panicCount",
                "python": True,
                "typescript": True,
                "cpp": True,
            }
        ]
        py_methods: dict[str, set[str]] = {"Client": set()}
        ts_methods = {"Client": {"panicCount"}}
        cpp_methods = {"Client": {"panic_count"}}
        errors = _check_method_rows(rows, py_methods, ts_methods, cpp_methods)
        assert any("python" in e and "missing" in e for e in errors), (
            f"missing Python method must trip the gate; got {errors!r}"
        )

    def _case_method_typescript_missing() -> None:
        """Declared on TS but no matching `js_name` in source — trips."""
        rows = [
            {
                "class": "Client",
                "name": "activeFullSubscriptions",
                "python": True,
                "typescript": True,
                "cpp": True,
            }
        ]
        py_methods = {"Client": {"active_full_subscriptions"}}
        ts_methods: dict[str, set[str]] = {}
        cpp_methods = {"Client": {"active_full_subscriptions"}}
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
                "class": "Client",
                "name": "panicCount",
                "python": False,
                "typescript": False,
                "cpp": False,
            }
        ]
        py_methods = {"Client": {"panic_count"}}
        ts_methods = {"Client": {"panicCount"}}
        cpp_methods = {"Client": {"panic_count"}}
        errors = _check_method_rows(rows, py_methods, ts_methods, cpp_methods)
        # All three columns are stale — every binding now exposes the
        # method but the row still says `false`.
        assert any("unexpected" in e for e in errors), (
            f"stale `false` rows must trip the gate; got {errors!r}"
        )

    def _case_method_row_missing_class_or_name() -> None:
        """Malformed row — gate surfaces a clear error."""
        rows = [
            {"class": "Client", "python": True},
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
        coincidentally share a method name (`subscribe` on both classes).
        The decoy holder is `DecoyClass` (a class no reverse-orphan scan
        covers) so the forward-isolation behaviour is exercised without
        entangling any reverse scan.
        """
        rows = [
            {
                "class": "StreamingClient",  # StreamingClient not on TS
                "name": "subscribe",
                "python": True,
                "typescript": False,
                "cpp": True,
            }
        ]
        # `subscribe` exists on `DecoyClass` (TS) but NOT on
        # `StreamingClient` (TS). Class-scoped lookup must respect that.
        py_methods = {"StreamingClient": {"subscribe"}}
        ts_methods = {"DecoyClass": {"subscribe"}}
        cpp_methods = {"StreamingClient": {"subscribe"}}
        errors = _check_method_rows(rows, py_methods, ts_methods, cpp_methods)
        assert errors == [], (
            f"class-scoped TS lookup must not leak across classes; got {errors!r}"
        )

    def _case_core_streaming_positive_all_enrolled() -> None:
        """Every core observability accessor has a `[[method]]` row —
        gate is silent. Covers the `dropped_count` -> `droppedEventCount`
        rename plus the direct camelCase mappings, including a
        `set_*_threshold`-shaped setter that maps by default camelCase."""
        core_methods = {
            "StreamSurface": {
                "dropped_event_count",
                "ring_occupancy",
                "ring_capacity",
                "panic_count",
                "set_example_threshold",
            },
            "StreamingClient": {
                "dropped_count",
                "ring_occupancy",
                "ring_capacity",
                "panic_count",
                "set_example_threshold",
            },
        }
        rows = [
            {"class": "StreamView", "name": n}
            for n in (
                "droppedEventCount",
                "ringOccupancy",
                "ringCapacity",
                "panicCount",
                "setExampleThreshold",
            )
        ] + [
            {"class": "StreamingClient", "name": n}
            for n in (
                "droppedEventCount",
                "ringOccupancy",
                "ringCapacity",
                "panicCount",
                "setExampleThreshold",
            )
        ]
        errors = _check_core_streaming_method_rows(core_methods, rows)
        assert errors == [], (
            f"fully-enrolled core observability surface must be silent; got {errors!r}"
        )

    def _case_core_streaming_negative_unenrolled_getter() -> None:
        """A wired core counter with no `[[method]]` row trips the gate —
        the exact blind spot this check closes."""
        core_methods = {"StreamSurface": {"example_count"}}
        rows: list[dict[str, Any]] = []  # no enrolling row
        errors = _check_core_streaming_method_rows(core_methods, rows)
        assert any("example_count" in e and "exampleCount" in e for e in errors), (
            f"unenrolled core counter must trip the gate; got {errors!r}"
        )

    def _case_core_streaming_negative_unenrolled_setter() -> None:
        """A wired core threshold setter with no row trips, mapped to the
        default camelCase binding row name."""
        core_methods = {"StreamingClient": {"set_example_threshold"}}
        rows: list[dict[str, Any]] = []
        errors = _check_core_streaming_method_rows(core_methods, rows)
        assert any("setExampleThreshold" in e for e in errors), (
            f"unenrolled core setter must trip the gate; got {errors!r}"
        )

    def _case_core_streaming_internal_hook_ignored() -> None:
        """`record_panic` (the internal fault-injection hook) is not an
        observability accessor, so the harvester never surfaces it and it
        never needs a row. The predicate is the filter; assert it
        directly. Lifecycle / subscription methods are likewise excluded."""
        assert not _is_core_observability_accessor("record_panic"), (
            "record_panic must be excluded from the observability surface"
        )
        for non_obs in ("subscribe", "stop_streaming", "reconnect", "is_streaming"):
            assert not _is_core_observability_accessor(non_obs), (
                f"{non_obs} must not be treated as an observability accessor"
            )
        # The shape predicate accepts the genuine observability accessors:
        # cumulative counters (`*_count`), ring telemetry (`ring_*`), and a
        # `set_*_threshold`-shaped setter.
        for obs in (
            "dropped_count",
            "ring_occupancy",
            "panic_count",
            "example_count",
            "set_example_threshold",
        ):
            assert _is_core_observability_accessor(obs), (
                f"{obs} must be treated as an observability accessor"
            )

    _case(
        "core-streaming positive — every observability accessor enrolled",
        _case_core_streaming_positive_all_enrolled,
    )
    _case(
        "core-streaming negative — unenrolled counter trips",
        _case_core_streaming_negative_unenrolled_getter,
    )
    _case(
        "core-streaming negative — unenrolled threshold setter trips",
        _case_core_streaming_negative_unenrolled_setter,
    )
    _case(
        "core-streaming positive — internal record_panic hook ignored",
        _case_core_streaming_internal_hook_ignored,
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

    def _case_client_reverse_orphan_generalized_trips() -> None:
        """A NEW Client helper (not one of the historical view accessors)
        present on Python + TypeScript but carrying no `[[method]]` row trips
        the generalized reverse-orphan scan."""
        rows: list[dict[str, Any]] = []  # no Client rows at all
        py_methods = {"Client": {"streaming"}}
        ts_methods = {"Client": {"streaming"}}
        cpp_methods: dict[str, set[str]] = {}
        errors = _check_method_rows(rows, py_methods, ts_methods, cpp_methods)
        assert any(
            "Client.streaming" in e and "no `[[method]]` row" in e
            for e in errors
        ), f"a new unenrolled Client helper must trip; got {errors!r}"

    def _case_client_reverse_orphan_enrolled_silent() -> None:
        """An enrolled Client helper does NOT trip the reverse-orphan scan."""
        rows = [
            {
                "class": "Client",
                "name": "streaming",
                "python": True,
                "typescript": True,
                "cpp": False,
                "rust": False,
            }
        ]
        py_methods = {"Client": {"streaming"}}
        ts_methods = {"Client": {"streaming"}}
        cpp_methods: dict[str, set[str]] = {}
        errors = _check_method_rows(rows, py_methods, ts_methods, cpp_methods)
        assert errors == [], f"enrolled Client helper must be silent; got {errors!r}"

    def _case_client_reverse_orphan_exempt_silent() -> None:
        """An exempt Client member (a connection factory in
        `CLIENT_REVERSE_ORPHAN_EXEMPT_MEMBERS`) does NOT trip the
        reverse-orphan scan. The auth-time readbacks `session_uuid` /
        `subscription_info` are now enrolled as Python-only `[[method]]`
        rows (covered by their own enrolled-silent case below) rather than
        exempted, so they appear here as enrolled rows, not bare members."""
        rows: list[dict[str, Any]] = [
            {"class": "Client", "name": "sessionUuid", "python": True,
             "typescript": False, "cpp": False},
            {"class": "Client", "name": "subscriptionInfo", "python": True,
             "typescript": False, "cpp": False},
        ]
        py_methods = {
            "Client": {"from_file", "from_env", "from_dotenv", "session_uuid", "subscription_info"}
        }
        ts_methods = {"Client": {"connect", "connectFromFile"}}
        cpp_methods: dict[str, set[str]] = {}
        errors = _check_method_rows(rows, py_methods, ts_methods, cpp_methods)
        assert errors == [], (
            f"exempt + enrolled Client members must not trip; got {errors!r}"
        )

    def _case_client_reverse_orphan_override_home_silent() -> None:
        """The blob-to-disk helper lives on the Client class per binding but is
        enrolled against a `FlatFilesNamespace` row through
        `METHOD_BINDING_OVERRIDES`; neither it nor its Python `_async` twin
        trips the Client reverse-orphan scan."""
        rows = [
            {
                "class": "FlatFilesNamespace",
                "name": "flatFileToPath",
                "python": True,
                "typescript": True,
                "cpp": True,
            }
        ]
        py_methods = {"Client": {"flatfile_to_path", "flatfile_to_path_async"}}
        ts_methods = {"Client": {"flatFileToPath"}}
        cpp_methods = {"FlatFiles": {"to_path"}}
        errors = _check_method_rows(rows, py_methods, ts_methods, cpp_methods)
        assert errors == [], (
            f"override-home Client member must not trip; got {errors!r}"
        )

    _case("client reverse-orphan - new helper trips (beyond the triple)", _case_client_reverse_orphan_generalized_trips)
    _case("client reverse-orphan - enrolled helper silent", _case_client_reverse_orphan_enrolled_silent)
    _case("client reverse-orphan - exempt members silent", _case_client_reverse_orphan_exempt_silent)
    _case("client reverse-orphan - override-home member silent", _case_client_reverse_orphan_override_home_silent)

    # ── Universal class-reverse-orphan scan (the unified helper) ──

    def _case_class_reverse_orphan_new_method_trips() -> None:
        """A NEW method on an enrolled non-Client class (here a Subscription
        getter) with no `[[method]]` row trips the universal reverse scan."""
        rows = [
            {"class": "Subscription", "name": "kind", "python": True,
             "typescript": True, "cpp": True},
        ]
        # `expiry` is a brand-new getter on the Python pyclass with no row.
        py_methods = {"PySubscription": {"kind", "expiry"}}
        ts_methods: dict[str, set[str]] = {}
        cpp_methods: dict[str, set[str]] = {}
        errors = _check_method_rows(rows, py_methods, ts_methods, cpp_methods)
        assert any(
            "Subscription.expiry" in e and "no `[[method]]` row" in e
            for e in errors
        ), f"a new unenrolled Subscription getter must trip; got {errors!r}"

    def _case_class_reverse_orphan_exempt_idiom_silent() -> None:
        """Per-language idioms in the exempt roster (the RecordBatchStream
        async-iterator dunders + the C++ private `create`) do NOT trip."""
        rows = [
            {"class": "RecordBatchStream", "name": n, "python": True,
             "typescript": True, "cpp": True}
            for n in ("close", "schema", "dropped")
        ]
        py_methods = {"RecordBatchStream": {
            "close", "schema", "dropped", "__aiter__", "__anext__",
            "__iter__", "__next__", "__aenter__", "__aexit__",
        }}
        ts_methods = {"RecordBatchStream": {"close", "schema", "dropped"}}
        cpp_methods = {"RecordBatchStream": {
            "close", "schema", "dropped", "create", "decode_one",
            "decode_schema", "open_ipc",
        }}
        errors = _check_method_rows(rows, py_methods, ts_methods, cpp_methods)
        assert errors == [], (
            f"exempt idioms must not trip the reverse scan; got {errors!r}"
        )

    def _case_class_reverse_orphan_override_spelling_silent() -> None:
        """The idiomatic per-binding spelling of an override-enrolled method
        (FlatFileRowList `count` -> C++ `size`, Python `__len__`,
        TypeScript `len`) is resolved as enrolled, not flagged as an orphan."""
        rows = [
            {"class": "FlatFileRowList", "name": "count", "python": True,
             "typescript": True, "cpp": True},
            {"class": "FlatFileRowList", "name": "toArrowIpc", "python": True,
             "typescript": True, "cpp": True},
        ]
        py_methods = {"FlatFileRowList": {"__len__", "to_arrow", "__bool__"}}
        ts_methods = {"FlatFileRowList": {"len", "toArrowIpc", "isEmpty"}}
        cpp_methods = {"FlatFileRowList": {"size", "to_arrow_ipc", "out"}}
        errors = _check_method_rows(rows, py_methods, ts_methods, cpp_methods)
        assert errors == [], (
            f"override-spelled + idiom members must be silent; got {errors!r}"
        )

    def _case_class_reverse_orphan_camel_source_member_trips() -> None:
        """A camelCase member harvested from a DISCOVERY binding whose snake
        re-derivation does not round back to the raw spelling must still trip:
        the source binding always counts as present, so an unenrolled member
        can never be silently dropped by an empty `present_on`."""
        errors = _check_class_reverse_orphans(
            "Subscription",
            {"cpp": {"newGetter"}},  # camelCase cpp member, snake≠raw
            enrolled=set(),
            exempt=frozenset(),
            discover_langs=frozenset({"cpp"}),
            strip_async=False,
            report_camel=False,
        )
        assert any("newGetter" in e for e in errors), (
            f"a camelCase source-binding member must trip, not drop; got {errors!r}"
        )
        assert any("['cpp']" in e for e in errors), (
            f"the error must report presence on the source binding; got {errors!r}"
        )

    _case("class reverse-orphan - new non-Client method trips", _case_class_reverse_orphan_new_method_trips)
    _case("class reverse-orphan - exempt idiom silent", _case_class_reverse_orphan_exempt_idiom_silent)
    _case("class reverse-orphan - override spelling silent", _case_class_reverse_orphan_override_spelling_silent)
    _case(
        "class reverse-orphan - camelCase source member trips",
        _case_class_reverse_orphan_camel_source_member_trips,
    )

    # ── C-ABI symbol roster + reverse-orphan scan ──

    def _case_ffi_symbol_orphan_trips() -> None:
        """A harvested C-ABI symbol matching no enrolled family and not in
        `FFI_SYMBOL_EXEMPT` trips the orphan scan."""
        symbols = {
            "config_set_worker_threads",      # config family — governed
            "option_history_trade_with_options",  # endpoint family — governed
            "arrow_bytes_free",               # exempt
            "client_batches_open",            # enrolled [[ffi_symbol]]
            "frobnicate_widget",              # ORPHAN — no family, not exempt
        }
        rows = [{"name": "client_batches_open"}]
        errors = _check_ffi_symbol_orphans(symbols, rows)
        assert any("frobnicate_widget" in e for e in errors), (
            f"an ungoverned C-ABI symbol must trip; got {errors!r}"
        )
        assert not any(
            "config_set_worker_threads" in e
            or "arrow_bytes_free" in e
            or "client_batches_open" in e
            for e in errors
        ), f"governed / exempt / enrolled symbols must stay silent; got {errors!r}"

    def _case_ffi_symbol_row_missing_decl_trips() -> None:
        """A `[[ffi_symbol]]` row whose symbol is absent from the harvested
        universe trips the existence check."""
        rows = [
            {"name": "client_batches_open"},
            {"name": "record_batch_stream_vanished"},
        ]
        symbols = {"client_batches_open"}
        errors = _check_ffi_symbol_rows(rows, symbols, pathlib.Path("/nonexistent"))
        assert any("record_batch_stream_vanished" in e for e in errors), (
            f"a row with no matching extern must trip; got {errors!r}"
        )
        assert not any("client_batches_open" in e for e in errors), (
            f"a present symbol must stay silent; got {errors!r}"
        )

    def _case_ffi_macro_symbol_harvested_and_governed() -> None:
        """A C-ABI extern whose name is a MACRO ARGUMENT (the
        `tick_array_free!` / `tick_array_to_arrow_ipc!` shape in
        `thetadatadx-ffi/src/types.rs`) is harvested by `_collect_ffi_all_symbols`, and an
        unenrolled one trips the orphan scan — so the macro blindness that hid
        44 externs from the literal-`fn` regex cannot regress."""
        with tempfile.TemporaryDirectory() as tmp:
            ffi_dir = pathlib.Path(tmp) / "ffi"
            ffi_dir.mkdir()
            (ffi_dir / "types.rs").write_text(
                # A literal-`fn` extern (still seen) ...
                'pub unsafe extern "C" fn thetadatadx_string_free(s: i32) {}\n'
                # ... the macro DEFINITIONS ($fn_name is not a real symbol) ...
                "macro_rules! tick_array_free { ($fn_name:ident, $t:ident) => {\n"
                '    pub unsafe extern "C" fn $fn_name(a: $t) {} }; }\n'
                "macro_rules! tick_array_to_arrow_ipc { ($fn_name:ident, $t:ident) => {\n"
                '    pub unsafe extern "C" fn $fn_name(r: *const $t, n: usize) {} }; }\n'
                # ... and the macro INVOCATIONS where the name lives as an arg.
                "tick_array_free!(thetadatadx_eod_tick_array_free, EodTick);\n"
                "tick_array_to_arrow_ipc!(thetadatadx_eod_ticks_to_arrow_ipc, EodTick);\n"
                "tick_array_to_arrow_ipc!(\n"
                "    thetadatadx_frobnicate_ticks_to_arrow_ipc, FrobTick);\n",
                encoding="utf-8",
            )
            syms = _collect_ffi_all_symbols(ffi_dir)
            assert "string_free" in syms, (
                f"a literal-`fn` extern must still be harvested; got {syms!r}"
            )
            assert "eod_tick_array_free" in syms, (
                f"a `tick_array_free!`-emitted extern must be harvested; got {syms!r}"
            )
            assert "eod_ticks_to_arrow_ipc" in syms, (
                f"a `tick_array_to_arrow_ipc!`-emitted extern must be harvested; "
                f"got {syms!r}"
            )
            assert "fn_name" not in syms, (
                f"the macro's `$fn_name` metavariable must not be harvested as a "
                f"symbol; got {syms!r}"
            )
            # The deallocator is exempt and the enrolled terminal is governed;
            # the unenrolled macro extern (no row, no exempt) must trip.
            rows = [{"name": "eod_ticks_to_arrow_ipc"}]
            orphans = _check_ffi_symbol_orphans(syms, rows)
            assert any("frobnicate_ticks_to_arrow_ipc" in e for e in orphans), (
                f"an unenrolled macro-emitted extern must trip the orphan scan; "
                f"got {orphans!r}"
            )
            assert not any(
                "eod_tick_array_free" in e or "eod_ticks_to_arrow_ipc" in e
                for e in orphans
            ), f"exempt free + enrolled terminal must stay silent; got {orphans!r}"

    _case("ffi-symbol orphan - ungoverned symbol trips", _case_ffi_symbol_orphan_trips)
    _case("ffi-symbol row - missing declaration trips", _case_ffi_symbol_row_missing_decl_trips)
    _case(
        "ffi-symbol macro - name-as-arg extern harvested + governed",
        _case_ffi_macro_symbol_harvested_and_governed,
    )

    # ── Request-options roster (the two generated consumers) ──

    def _case_request_options_consumers_agree_silent() -> None:
        """When the C++ `with_*` roster (minus the deadline alias) and the FFI
        option fields match, with the SSOT global present, the check is silent."""
        ssot = {"timeout_ms"}
        cpp = {"strike", "right", "timeout_ms", "deadline"}  # deadline exempt
        ffi_fields = {"strike", "right", "timeout_ms"}
        ffi_has = {"timeout_ms"}  # scalar presence flag
        errors = _check_request_options_roster(ssot, cpp, ffi_fields, ffi_has)
        assert errors == [], (
            f"agreeing request-options consumers must be silent; got {errors!r}"
        )

    def _case_request_options_consumer_drift_trips() -> None:
        """A `with_X` setter with no matching FFI field (and vice versa) trips."""
        ssot = {"timeout_ms"}
        cpp = {"strike", "timeout_ms"}            # `with_strike`, no FFI field
        ffi_fields = {"right", "timeout_ms"}      # `right` field, no `with_right`
        ffi_has = {"timeout_ms"}
        errors = _check_request_options_roster(ssot, cpp, ffi_fields, ffi_has)
        assert any("strike" in e for e in errors) and any(
            "right" in e for e in errors
        ), f"a request-options consumer drift must trip; got {errors!r}"

    def _case_request_options_scalar_global_missing_has_flag_trips() -> None:
        """A scalar SSOT global present as a field but missing its `has_<name>`
        presence flag trips — without the flag the value is never applied, so
        the C++ → C bridge silently drops the option while the rosters match."""
        ssot = {"timeout_ms"}
        cpp = {"timeout_ms", "deadline"}      # roster matches the FFI field ...
        ffi_fields = {"timeout_ms"}           # ... field present ...
        ffi_has: set[str] = set()             # ... but `has_timeout_ms` dropped.
        errors = _check_request_options_roster(ssot, cpp, ffi_fields, ffi_has)
        assert any(
            "timeout_ms" in e and "has_timeout_ms" in e for e in errors
        ), (
            f"a scalar global missing its `has_` flag must trip even when the "
            f"rosters match; got {errors!r}"
        )

    _case("request-options - consumers agree silent", _case_request_options_consumers_agree_silent)
    _case("request-options - consumer drift trips", _case_request_options_consumer_drift_trips)
    _case(
        "request-options - scalar global missing has_ flag trips",
        _case_request_options_scalar_global_missing_has_flag_trips,
    )

    # ── Request-options TYPE parity (signature level, Route A) ──

    def _write_ffi_options_struct(tmp: pathlib.Path, fields: dict[str, str]) -> pathlib.Path:
        """A minimal `ThetaDataDxEndpointRequestOptions` source for the FFI
        field-type reader; `fields` maps option name → declared Rust type."""
        body = "".join(f"    pub {n}: {t},\n" for n, t in fields.items())
        path = tmp / "endpoint_request_options.rs"
        path.write_text(
            "#[repr(C)]\n"
            "pub struct ThetaDataDxEndpointRequestOptions {\n"
            f"{body}}}\n",
            encoding="utf-8",
        )
        return path

    def _case_request_options_types_real_surface_silent() -> None:
        """The actual SSOT / C++ `.inc` / FFI struct types agree (Route A is
        by-construction today), so the type check is silent on the real repo."""
        errors = _check_request_options_types(
            _collect_ssot_request_option_types(ENDPOINT_SURFACE_TOML),
            _collect_cpp_with_option_types(ENDPOINT_OPTIONS_HPP_INC),
            ENDPOINT_REQUEST_OPTIONS_RS,
        )
        assert errors == [], (
            f"the agreeing real request-options surface must be silent; got {errors!r}"
        )

    def _case_request_options_cpp_type_drift_trips() -> None:
        """A C++ `with_X` parameter type that differs from the SSOT-implied
        spelling (here `int32_t` where the SSOT says `Str` → `std::string`)
        trips, with the option named."""
        with tempfile.TemporaryDirectory() as tmp:
            ffi = _write_ffi_options_struct(
                pathlib.Path(tmp), {"strike": "*const c_char"}
            )
            errors = _check_request_options_types(
                {"strike": "Str"},
                {"strike": "int32_t"},  # drifted: should be std::string
                ffi,
            )
        assert any(
            "strike" in e and "with_strike" in e for e in errors
        ), f"a C++ with_* type drift must trip; got {errors!r}"

    def _case_request_options_ffi_type_drift_trips() -> None:
        """An FFI struct field type that differs from the SSOT-implied spelling
        (here `i32` where the SSOT says `Float` → `f64`) trips."""
        with tempfile.TemporaryDirectory() as tmp:
            ffi = _write_ffi_options_struct(
                pathlib.Path(tmp), {"rate_value": "i32"}  # drifted: should be f64
            )
            errors = _check_request_options_types(
                {"rate_value": "Float"},
                {"rate_value": "double"},
                ffi,
            )
        assert any(
            "rate_value" in e and "FFI field" in e for e in errors
        ), f"an FFI field type drift must trip; got {errors!r}"

    def _case_request_options_unknown_param_type_trips() -> None:
        """A `param_type` outside REQUEST_OPTION_TYPE_MAP fails closed — a new
        or mis-tagged type cannot slip the gate by defaulting to a string."""
        with tempfile.TemporaryDirectory() as tmp:
            ffi = _write_ffi_options_struct(
                pathlib.Path(tmp), {"mystery": "f64"}
            )
            errors = _check_request_options_types(
                {"mystery": "Decimal128"},  # not in the map
                {"mystery": "double"},
                ffi,
            )
        assert any(
            "mystery" in e and "REQUEST_OPTION_TYPE_MAP" in e for e in errors
        ), f"an unknown param_type must fail closed; got {errors!r}"

    def _case_request_options_types_deadline_exempt() -> None:
        """The `with_deadline` alias is exempt — its `std::chrono` parameter is
        not held to a SSOT type, so it never trips the type check."""
        with tempfile.TemporaryDirectory() as tmp:
            ffi = _write_ffi_options_struct(
                pathlib.Path(tmp), {"timeout_ms": "u64"}
            )
            errors = _check_request_options_types(
                {"timeout_ms": "u64", "deadline": "Duration"},
                {"timeout_ms": "uint64_t", "deadline": "std::chrono::milliseconds"},
                ffi,
            )
        assert errors == [], (
            f"the deadline alias must be exempt from the type check; got {errors!r}"
        )

    _case(
        "request-options type - real surface silent",
        _case_request_options_types_real_surface_silent,
    )
    _case(
        "request-options type - C++ with_* type drift trips",
        _case_request_options_cpp_type_drift_trips,
    )
    _case(
        "request-options type - FFI field type drift trips",
        _case_request_options_ffi_type_drift_trips,
    )
    _case(
        "request-options type - unknown param_type fails closed",
        _case_request_options_unknown_param_type_trips,
    )
    _case(
        "request-options type - deadline alias exempt",
        _case_request_options_types_deadline_exempt,
    )

    def _case_ts_entry_resolves_from_package_json() -> None:
        """The TS class collector resolves the entry from `package.json`
        `types`, follows `export * from './index'`, and harvests `export class`
        / `export declare const` leaves plus runtime `Object.assign` exports
        from the `main` entry."""
        with tempfile.TemporaryDirectory() as tmp:
            pkg = pathlib.Path(tmp)
            (pkg / "package.json").write_text(
                '{"main": "entry.js", "types": "entry.d.ts"}\n', encoding="utf-8"
            )
            (pkg / "index.d.ts").write_text(
                "export declare class NapiClient {}\n"
                "export interface NapiThing {}\n",
                encoding="utf-8",
            )
            (pkg / "entry.d.ts").write_text(
                "export * from './index';\n"
                "export class WrapperError extends Error {}\n"
                "export declare const WrapperSession: { new (): WrapperSession };\n"
                "export interface WrapperSession {}\n",
                encoding="utf-8",
            )
            (pkg / "entry.js").write_text(
                "const native = require('./index');\n"
                "module.exports = Object.assign({}, native, "
                "{ WrapperError, WrapperSession });\n",
                encoding="utf-8",
            )
            (pkg / "index.js").write_text(
                "exports.NapiClient = class {};\n", encoding="utf-8"
            )
            found = collect_typescript_classes(_resolve_ts_entry(pkg, "types", "index.d.ts"))
            for want in ("NapiClient", "NapiThing", "WrapperError", "WrapperSession"):
                assert want in found, f"{want} must be seen via the resolved entry; got {sorted(found)}"

    def _case_ts_entry_falls_back_to_index() -> None:
        """With no `types`/`main` keys, the resolver falls back to `index.*`."""
        with tempfile.TemporaryDirectory() as tmp:
            pkg = pathlib.Path(tmp)
            (pkg / "package.json").write_text("{}\n", encoding="utf-8")
            (pkg / "index.d.ts").write_text(
                "export declare class OnlyClient {}\n", encoding="utf-8"
            )
            found = collect_typescript_classes(_resolve_ts_entry(pkg, "types", "index.d.ts"))
            assert "OnlyClient" in found, f"fallback to index.d.ts must work; got {sorted(found)}"

    def _case_ts_wrapper_augmentation_method_seen() -> None:
        """A wrapper-side `declare module './index' { interface Client { ... } }`
        augmentation and its `Client.prototype.<name>` runtime addition are
        harvested under `Client`."""
        with tempfile.TemporaryDirectory() as tmp:
            pkg = pathlib.Path(tmp)
            (pkg / "package.json").write_text(
                '{"main": "entry.js", "types": "entry.d.ts"}\n', encoding="utf-8"
            )
            (pkg / "entry.d.ts").write_text(
                "declare module './index' {\n"
                "  interface Client {\n"
                "    streaming(cb: unknown): Promise<void>;\n"
                "  }\n"
                "}\n",
                encoding="utf-8",
            )
            (pkg / "entry.js").write_text(
                "native.Client.prototype.streaming = async function streaming(cb) {};\n",
                encoding="utf-8",
            )
            wrapper = _collect_ts_wrapper_class_methods(pkg)
            assert wrapper.get("Client") == {"streaming"}, (
                f"wrapper augmentation must surface Client.streaming; got {wrapper!r}"
            )

    _case("ts entry - resolves from package.json + follows re-exports", _case_ts_entry_resolves_from_package_json)
    _case("ts entry - falls back to index.* when keys absent", _case_ts_entry_falls_back_to_index)
    _case("ts entry - wrapper Client augmentation harvested", _case_ts_wrapper_augmentation_method_seen)

    def _materialize_ts_pkg(
        tmp: str, *, dts_body: str, js_body: str
    ) -> tuple[set[str], set[str], set[str]]:
        """Write a synthetic TypeScript package and return the three TS sets the
        class-row check consumes: declared classes, declared interfaces, runtime
        exports. The package declares its entries through `package.json` so the
        resolver path under test runs exactly as it does on the real tree."""
        pkg = pathlib.Path(tmp)
        (pkg / "package.json").write_text(
            '{"main": "entry.js", "types": "entry.d.ts"}\n', encoding="utf-8"
        )
        (pkg / "entry.d.ts").write_text(dts_body, encoding="utf-8")
        (pkg / "entry.js").write_text(js_body, encoding="utf-8")
        dts_path = _resolve_ts_entry(pkg, "types", "index.d.ts")
        declared_classes, declared_interfaces = _collect_ts_dts_class_kinds(dts_path)
        runtime = _collect_ts_runtime_classes(dts_path)
        return declared_classes, declared_interfaces, runtime

    def _case_ts_class_row_runtime_present_silent() -> None:
        """A class declared in `.d.ts` AND exported at runtime is silent."""
        rows = [{"name": "Widget", "python": False, "typescript": True, "cpp": False}]
        with tempfile.TemporaryDirectory() as tmp:
            dc, di, rt = _materialize_ts_pkg(
                tmp,
                dts_body="export declare class Widget {}\n",
                js_body="module.exports = Object.assign({}, { Widget });\n",
            )
            errors = _check_class_rows(rows, set(), set(), dc, di, rt)
        assert errors == [], (
            f"declared + runtime-exported class must be silent; got {errors!r}"
        )

    def _case_ts_class_row_runtime_drop_trips() -> None:
        """Dropping ONLY the JS runtime export (the class still declared in the
        `.d.ts`) must trip the gate. This is the audit-proven hole: a
        declaration-side hit must never stand in for the shipped surface."""
        rows = [{"name": "Widget", "python": False, "typescript": True, "cpp": False}]
        with tempfile.TemporaryDirectory() as tmp:
            dc, di, rt = _materialize_ts_pkg(
                tmp,
                # Still declared as a runtime class on the typed surface ...
                dts_body="export declare class Widget {}\n",
                # ... but absent from the runtime export object.
                js_body="module.exports = Object.assign({}, {});\n",
            )
            errors = _check_class_rows(rows, set(), set(), dc, di, rt)
        assert any(
            "Widget.typescript" in e and "missing" in e and "runtime export" in e
            for e in errors
        ), f"dropped JS runtime export must trip the gate; got {errors!r}"

    def _case_ts_class_row_dts_drop_caught_as_typing_gap() -> None:
        """Dropping ONLY the `.d.ts` declaration (the class still exported at
        runtime) is caught as a typing/parity gap, not passed silently."""
        rows = [{"name": "Widget", "python": False, "typescript": True, "cpp": False}]
        with tempfile.TemporaryDirectory() as tmp:
            dc, di, rt = _materialize_ts_pkg(
                tmp,
                # No declaration of any kind on the typed surface ...
                dts_body="export declare class Other {}\n",
                # ... yet present at runtime.
                js_body="module.exports = Object.assign({}, { Widget });\n",
            )
            errors = _check_class_rows(rows, set(), set(), dc, di, rt)
        assert any(
            "Widget.typescript" in e and "no matching .d.ts declaration" in e
            for e in errors
        ), f"runtime export with no declaration must be flagged; got {errors!r}"

    def _case_ts_interface_row_no_runtime_required() -> None:
        """An `interface`-declared object type (the napi `#[napi(object)]`
        plain-object shape) is satisfied by the declaration alone; it has no
        runtime constructor to export, so a true claim stays silent."""
        rows = [
            {"name": "PlainShape", "python": False, "typescript": True, "cpp": False}
        ]
        with tempfile.TemporaryDirectory() as tmp:
            dc, di, rt = _materialize_ts_pkg(
                tmp,
                dts_body="export interface PlainShape { x: number }\n",
                # The runtime ships no `PlainShape` constructor, and that is
                # correct for an erased interface.
                js_body="module.exports = Object.assign({}, {});\n",
            )
            errors = _check_class_rows(rows, set(), set(), dc, di, rt)
        assert errors == [], (
            f"interface-only object type must not require a runtime export; "
            f"got {errors!r}"
        )

    def _case_ts_runtime_export_via_object_literal_comments() -> None:
        """`_collect_js_exports` reads every key of a `module.exports =
        Object.assign(..., { ... })` literal even when comments interleave the
        keys; this covers the alternating-drop and comment-anchor defects that
        previously hid half the runtime names."""
        with tempfile.TemporaryDirectory() as tmp:
            pkg = pathlib.Path(tmp)
            js = pkg / "entry.js"
            js.write_text(
                "module.exports = Object.assign({}, native, {\n"
                "  Alpha,\n"
                "  // a comment between two keys must not hide the next key\n"
                "  Beta,\n"
                "  Gamma: native.GammaImpl,\n"
                "  /* block comment */ Delta,\n"
                "  Epsilon,\n"
                "});\n",
                encoding="utf-8",
            )
            found = _collect_js_exports(js)
        for want in ("Alpha", "Beta", "Gamma", "Delta", "Epsilon"):
            assert want in found, (
                f"runtime export {want} must be harvested despite comments; "
                f"got {sorted(found)}"
            )

    def _case_ts_const_alias_class_runtime_present_silent() -> None:
        """A `const` runtime-class alias (`export const X: typeof Y`) backed by a
        same-name `interface` is satisfied ONLY when the alias is exported at
        runtime. With the runtime export present the row stays silent — this is
        the shipped `Contract` shape (alias of `ContractRef`, plus the streaming
        event-payload `interface Contract`)."""
        rows = [{"name": "Contract", "python": False, "typescript": True, "cpp": False}]
        with tempfile.TemporaryDirectory() as tmp:
            dc, di, rt = _materialize_ts_pkg(
                tmp,
                dts_body=(
                    "export declare class ContractRef {}\n"
                    "export const Contract: typeof ContractRef;\n"
                    "export type Contract = ContractRef;\n"
                    "export interface Contract { symbol: string }\n"
                ),
                js_body=(
                    "module.exports = Object.assign({}, "
                    "{ ContractRef, Contract: ContractRef });\n"
                ),
            )
            assert "Contract" in dc, (
                f"the `typeof` alias must classify Contract as a runtime class; "
                f"got declared_classes={sorted(dc)}"
            )
            errors = _check_class_rows(rows, set(), set(), dc, di, rt)
        assert errors == [], (
            f"const-alias runtime class with a runtime export must be silent; "
            f"got {errors!r}"
        )

    def _case_ts_const_alias_class_runtime_drop_trips() -> None:
        """Dropping ONLY the runtime `Contract` alias from the package entry
        (while the same-name `interface Contract` and the `ContractRef` class
        both remain) MUST trip the gate. The `typeof` alias is a real runtime
        constructor, so the interface fallback must not stand in for the dropped
        runtime export — the alias-bypass hole this fix closes."""
        rows = [{"name": "Contract", "python": False, "typescript": True, "cpp": False}]
        with tempfile.TemporaryDirectory() as tmp:
            dc, di, rt = _materialize_ts_pkg(
                tmp,
                # `Contract` is still declared as a runtime-class alias and as a
                # same-name interface; `ContractRef` still ships ...
                dts_body=(
                    "export declare class ContractRef {}\n"
                    "export const Contract: typeof ContractRef;\n"
                    "export interface Contract { symbol: string }\n"
                ),
                # ... but the runtime `Contract` alias is gone (only ContractRef
                # is exported), so `import { Contract }` resolves to undefined.
                js_body="module.exports = Object.assign({}, { ContractRef });\n",
            )
            errors = _check_class_rows(rows, set(), set(), dc, di, rt)
        assert any(
            "Contract.typescript" in e and "missing" in e and "runtime export" in e
            for e in errors
        ), (
            f"a dropped runtime const-alias must trip despite the same-name "
            f"interface; got {errors!r}"
        )

    def _case_ts_inline_ctor_const_class_runtime_drop_trips() -> None:
        """The inline-constructor `const` shape
        (`export declare const X: { new (...): X }`, the shipped
        `StreamingSession`) is also held to a runtime export: dropping it from
        the runtime trips the gate."""
        rows = [
            {"name": "StreamingSession", "python": False, "typescript": True, "cpp": False}
        ]
        with tempfile.TemporaryDirectory() as tmp:
            dc, di, rt = _materialize_ts_pkg(
                tmp,
                dts_body=(
                    "export declare const StreamingSession: {\n"
                    "  new (client: unknown): StreamingSession;\n"
                    "  prototype: StreamingSession;\n"
                    "};\n"
                    "export interface StreamingSession { drainNow(): void }\n"
                ),
                js_body="module.exports = Object.assign({}, {});\n",
            )
            assert "StreamingSession" in dc, (
                f"the inline-ctor const must classify as a runtime class; "
                f"got declared_classes={sorted(dc)}"
            )
            errors = _check_class_rows(rows, set(), set(), dc, di, rt)
        assert any(
            "StreamingSession.typescript" in e and "missing" in e and "runtime export" in e
            for e in errors
        ), f"a dropped inline-ctor const-class must trip; got {errors!r}"

    def _case_ts_value_const_not_a_runtime_class() -> None:
        """A plain value `const` export (`export declare const VERSION: string`)
        is NOT a runtime class: it carries no constructor, so it must never be
        forced to have a runtime constructor export. The over-broad earlier
        pattern matched any `export declare const X:` and wrongly demanded one.
        A `const enum` is likewise not a runtime class here."""
        with tempfile.TemporaryDirectory() as tmp:
            dc, di, rt = _materialize_ts_pkg(
                tmp,
                dts_body=(
                    "export declare const VERSION: string;\n"
                    "export const BUILD: number = 7;\n"
                    "export declare const enum Venue { Nasdaq = 0 }\n"
                ),
                js_body='module.exports = Object.assign({}, { VERSION, BUILD });\n',
            )
            for value_const in ("VERSION", "BUILD", "Venue"):
                assert value_const not in dc, (
                    f"value const / const enum {value_const} must NOT classify as a "
                    f"runtime class; got declared_classes={sorted(dc)}"
                )
            # A row that claims `VERSION` ships on TypeScript is satisfied by the
            # runtime export alone (flagged only as an untyped-runtime typing
            # gap), never rejected for a missing constructor.
            rows = [
                {"name": "VERSION", "python": False, "typescript": True, "cpp": False}
            ]
            errors = _check_class_rows(rows, set(), set(), dc, di, rt)
        assert not any("missing" in e for e in errors), (
            f"a value const exported at runtime must not be reported missing as a "
            f"runtime class; got {errors!r}"
        )

    def _case_ts_runtime_helper_require_not_exported() -> None:
        """A name defined only in a helper module that is `require`d for its side
        effects but NOT re-exported must NOT count as a runtime export. The
        runtime surface is what is placed on this module's `module.exports`;
        following arbitrary `require(...)` calls would let a helper-only class
        masquerade as a shipped export and bypass the runtime-export gate. With
        the require unfollowed, a row claiming `typescript = true` for the
        helper-only class trips."""
        rows = [{"name": "HelperOnly", "python": False, "typescript": True, "cpp": False}]
        with tempfile.TemporaryDirectory() as tmp:
            pkg = pathlib.Path(tmp)
            (pkg / "package.json").write_text(
                '{"main": "entry.js", "types": "entry.d.ts"}\n', encoding="utf-8"
            )
            # Declared on the typed surface as a runtime class ...
            (pkg / "entry.d.ts").write_text(
                "export declare class HelperOnly {}\n", encoding="utf-8"
            )
            # ... but the helper holding it is imported for side effects only and
            # is NOT re-exported, so it never reaches `module.exports`.
            (pkg / "entry.js").write_text(
                "const helper = require('./helper');\n"
                "void helper;\n"
                "module.exports = Object.assign({}, {});\n",
                encoding="utf-8",
            )
            (pkg / "helper.js").write_text(
                "class HelperOnly {}\nmodule.exports = { HelperOnly };\n",
                encoding="utf-8",
            )
            dts_path = _resolve_ts_entry(pkg, "types", "index.d.ts")
            dc, di = _collect_ts_dts_class_kinds(dts_path)
            rt = _collect_ts_runtime_classes(dts_path)
            assert "HelperOnly" not in rt, (
                f"a helper-only side-effect require must not count as a runtime "
                f"export; got runtime={sorted(rt)}"
            )
            errors = _check_class_rows(rows, set(), set(), dc, di, rt)
        assert any(
            "HelperOnly.typescript" in e and "missing" in e and "runtime export" in e
            for e in errors
        ), f"helper-only class must trip the runtime-export gate; got {errors!r}"

    def _case_ts_runtime_real_reexport_chain_collected() -> None:
        """The genuine re-export chain still resolves: a wrapper entry binding
        the napi module (`const native = require('./index')`) and spreading it
        into `module.exports = Object.assign({}, native, { ... })` MUST still
        surface the napi names AND the wrapper-added names. This mirrors the
        shipped `streaming-session.js` -> `index.js` chain, so the bound-name
        re-export must keep being followed while the helper-only require above is
        excluded."""
        with tempfile.TemporaryDirectory() as tmp:
            pkg = pathlib.Path(tmp)
            (pkg / "package.json").write_text(
                '{"main": "entry.js", "types": "entry.d.ts"}\n', encoding="utf-8"
            )
            (pkg / "entry.d.ts").write_text(
                "export * from './index';\n"
                "export declare const StreamingSession: { new (): StreamingSession };\n",
                encoding="utf-8",
            )
            (pkg / "entry.js").write_text(
                "const native = require('./index');\n"
                "class StreamingSession {}\n"
                "module.exports = Object.assign({}, native, { StreamingSession });\n",
                encoding="utf-8",
            )
            (pkg / "index.js").write_text(
                "module.exports.Contract = class {};\n"
                "module.exports.ThetaDataError = class {};\n",
                encoding="utf-8",
            )
            (pkg / "index.d.ts").write_text(
                "export declare class Contract {}\n"
                "export declare class ThetaDataError {}\n",
                encoding="utf-8",
            )
            dts_path = _resolve_ts_entry(pkg, "types", "index.d.ts")
            rt = _collect_ts_runtime_classes(dts_path)
        # The require-bound name `native` is a spread SOURCE, not an export.
        assert "native" not in rt, (
            f"the require-binding identifier must not read as a runtime export; "
            f"got runtime={sorted(rt)}"
        )
        for want in ("Contract", "ThetaDataError", "StreamingSession"):
            assert want in rt, (
                f"the real re-export chain must still surface {want}; "
                f"got runtime={sorted(rt)}"
            )

    def _case_ts_runtime_require_value_not_wholesale() -> None:
        """A require-bound name used as a property VALUE
        (`module.exports = Object.assign({}, { alias: sub })`) re-exports the
        single key (`alias`), NOT the required module's whole surface. The
        required module's own names must therefore stay out of the runtime set
        — only a bare-argument / object-spread forward is wholesale."""
        with tempfile.TemporaryDirectory() as tmp:
            pkg = pathlib.Path(tmp)
            (pkg / "package.json").write_text(
                '{"main": "entry.js", "types": "entry.d.ts"}\n', encoding="utf-8"
            )
            (pkg / "entry.d.ts").write_text("export {};\n", encoding="utf-8")
            (pkg / "entry.js").write_text(
                "const sub = require('./sub');\n"
                "module.exports = Object.assign({}, { alias: sub });\n",
                encoding="utf-8",
            )
            (pkg / "sub.js").write_text(
                "module.exports.Buried = class {};\n", encoding="utf-8"
            )
            dts_path = _resolve_ts_entry(pkg, "types", "index.d.ts")
            rt = _collect_ts_runtime_classes(dts_path)
        assert "alias" in rt, f"the re-exported key must be present; got {sorted(rt)}"
        assert "Buried" not in rt, (
            f"a require-bound name used as a property value must not pull in the "
            f"required surface wholesale; got runtime={sorted(rt)}"
        )

    _case("ts class row - declared + runtime export silent", _case_ts_class_row_runtime_present_silent)
    _case("ts class row - JS runtime export drop trips", _case_ts_class_row_runtime_drop_trips)
    _case("ts class row - .d.ts drop caught as typing gap", _case_ts_class_row_dts_drop_caught_as_typing_gap)
    _case("ts class row - interface-only needs no runtime export", _case_ts_interface_row_no_runtime_required)
    _case("ts runtime - object-literal keys read past comments", _case_ts_runtime_export_via_object_literal_comments)
    _case("ts const-alias class - typeof alias + runtime export silent", _case_ts_const_alias_class_runtime_present_silent)
    _case("ts const-alias class - dropped runtime alias trips past interface", _case_ts_const_alias_class_runtime_drop_trips)
    _case("ts const-class - dropped inline-ctor runtime export trips", _case_ts_inline_ctor_const_class_runtime_drop_trips)
    _case("ts value const - not forced to be a runtime class", _case_ts_value_const_not_a_runtime_class)
    _case("ts runtime - helper-only side-effect require not exported", _case_ts_runtime_helper_require_not_exported)
    _case("ts runtime - real bound-name re-export chain still collected", _case_ts_runtime_real_reexport_chain_collected)
    _case("ts runtime - require bound as property value is not wholesale", _case_ts_runtime_require_value_not_wholesale)

    def _case_flatfiles_namespace_fetch_resolves() -> None:
        """A `FlatFilesNamespace` fetch row resolves through the C++ alias to the
        `FlatFiles` class and against the snake/camel per-binding spellings."""
        rows = [
            {
                "class": "FlatFilesNamespace",
                "name": "optionTradeQuote",
                "python": True,
                "typescript": True,
                "cpp": True,
            }
        ]
        py_methods = {"FlatFilesNamespace": {"option_trade_quote"}}
        ts_methods = {"FlatFilesNamespace": {"optionTradeQuote"}}
        # The row says `FlatFilesNamespace`; the C++ class is `FlatFiles`.
        cpp_methods = {"FlatFiles": {"option_trade_quote"}}
        errors = _check_method_rows(rows, py_methods, ts_methods, cpp_methods)
        assert errors == [], (
            f"flat-file fetch row must resolve across bindings; got {errors!r}"
        )

    def _case_flatfiles_to_path_override_resolves() -> None:
        """The blob-to-disk helper resolves through `METHOD_BINDING_OVERRIDES`:
        Python `Client.flatfile_to_path`, TS `Client.flatFileToPath`, C++
        `FlatFiles::to_path` — three divergent home classes / names, one row."""
        rows = [
            {
                "class": "FlatFilesNamespace",
                "name": "flatFileToPath",
                "python": True,
                "typescript": True,
                "cpp": True,
            }
        ]
        py_methods = {"Client": {"flatfile_to_path"}}
        ts_methods = {"Client": {"flatFileToPath"}}
        cpp_methods = {"FlatFiles": {"to_path"}}
        errors = _check_method_rows(rows, py_methods, ts_methods, cpp_methods)
        assert errors == [], (
            f"to-path override must resolve each binding's home class; got {errors!r}"
        )

    def _case_flatfiles_to_path_override_drop_trips() -> None:
        """Dropping the C++ `to_path` member trips the overridden row — the
        override must still gate, not silently pass."""
        rows = [
            {
                "class": "FlatFilesNamespace",
                "name": "flatFileToPath",
                "python": True,
                "typescript": True,
                "cpp": True,
            }
        ]
        py_methods = {"Client": {"flatfile_to_path"}}
        ts_methods = {"Client": {"flatFileToPath"}}
        cpp_methods = {"FlatFiles": set()}  # `to_path` dropped on C++
        errors = _check_method_rows(rows, py_methods, ts_methods, cpp_methods)
        assert any("flatFileToPath" in e and ".cpp" in e for e in errors), (
            f"dropped C++ to_path must trip the overridden row; got {errors!r}"
        )

    def _case_flatfiles_namespace_orphan_trips() -> None:
        """A fetch method present on the namespace class with no enrolling row
        trips the reverse-orphan scan."""
        rows = [
            {
                "class": "FlatFilesNamespace",
                "name": "optionTradeQuote",
                "python": True,
                "typescript": True,
                "cpp": True,
            }
        ]
        # `indexEod` is present on the namespace but carries no row.
        py_methods = {"FlatFilesNamespace": {"option_trade_quote", "index_eod"}}
        ts_methods = {"FlatFilesNamespace": {"optionTradeQuote", "indexEod"}}
        cpp_methods = {"FlatFiles": {"option_trade_quote", "index_eod"}}
        errors = _check_method_rows(rows, py_methods, ts_methods, cpp_methods)
        assert any("index_eod" in e and "no `[[method]]` row" in e for e in errors), (
            f"unenrolled namespace fetch method must trip the gate; got {errors!r}"
        )

    def _case_flatfiles_namespace_artifacts_exempt() -> None:
        """Collector artifacts on the namespace class (C++ `handle_` / the FFI
        externs, Python `pull_decoded`, the override-enrolled C++ `to_path`) do
        NOT trip the reverse-orphan scan."""
        rows = [
            {"class": "FlatFilesNamespace", "name": n, "python": True, "typescript": True, "cpp": True}
            for n in sorted(FLATFILES_NAMESPACE_METHODS)
        ] + [
            {"class": "FlatFilesNamespace", "name": "flatFileToPath", "python": True, "typescript": True, "cpp": True}
        ]
        py_methods = {
            "FlatFilesNamespace": {_camel_to_snake(n) for n in FLATFILES_NAMESPACE_METHODS} | {"pull_decoded"},
            "Client": {"flatfile_to_path"},
        }
        ts_methods = {"FlatFilesNamespace": set(FLATFILES_NAMESPACE_METHODS), "Client": {"flatFileToPath"}}
        cpp_methods = {
            "FlatFiles": {_camel_to_snake(n) for n in FLATFILES_NAMESPACE_METHODS}
            | {"handle_", "to_path", "thetadatadx_flatfile_request_decoded", "thetadatadx_flatfile_request_to_path"},
        }
        errors = _check_method_rows(rows, py_methods, ts_methods, cpp_methods)
        assert errors == [], (
            f"namespace collector artifacts must stay exempt; got {errors!r}"
        )

    _case("flatfiles positive — namespace fetch row resolves via C++ alias", _case_flatfiles_namespace_fetch_resolves)
    _case("flatfiles positive — to-path override resolves divergent homes", _case_flatfiles_to_path_override_resolves)
    _case("flatfiles negative — dropped C++ to_path trips overridden row", _case_flatfiles_to_path_override_drop_trips)
    _case("flatfiles negative — unenrolled namespace fetch method trips", _case_flatfiles_namespace_orphan_trips)
    _case("flatfiles positive — namespace collector artifacts exempt", _case_flatfiles_namespace_artifacts_exempt)

    def _case_method_rust_column_resolves() -> None:
        """A `FlatFilesNamespace` fetch row with `rust = true` resolves to the
        Rust `FlatFiles` view via `RUST_METHOD_CLASS` and the snake_case name."""
        rows = [
            {
                "class": "FlatFilesNamespace",
                "name": "optionTradeQuote",
                "python": True,
                "typescript": True,
                "cpp": True,
                "rust": True,
            }
        ]
        py_methods = {"FlatFilesNamespace": {"option_trade_quote"}}
        ts_methods = {"FlatFilesNamespace": {"optionTradeQuote"}}
        cpp_methods = {"FlatFiles": {"option_trade_quote"}}
        rust_methods = {"FlatFiles": {"option_trade_quote"}}
        errors = _check_method_rows(rows, py_methods, ts_methods, cpp_methods, rust_methods)
        assert errors == [], (
            f"rust-column fetch row must resolve to the FlatFiles view; got {errors!r}"
        )

    def _case_method_rust_column_missing_trips() -> None:
        """`rust = true` but the method is absent on the Rust view — trips."""
        rows = [
            {
                "class": "FlatFilesNamespace",
                "name": "optionTradeQuote",
                "python": True,
                "typescript": True,
                "cpp": True,
                "rust": True,
            }
        ]
        py_methods = {"FlatFilesNamespace": {"option_trade_quote"}}
        ts_methods = {"FlatFilesNamespace": {"optionTradeQuote"}}
        cpp_methods = {"FlatFiles": {"option_trade_quote"}}
        rust_methods: dict[str, set[str]] = {"FlatFiles": set()}
        errors = _check_method_rows(rows, py_methods, ts_methods, cpp_methods, rust_methods)
        assert any("optionTradeQuote" in e and ".rust" in e and "missing" in e for e in errors), (
            f"missing Rust view method must trip the rust column; got {errors!r}"
        )

    def _case_method_rust_to_path_override_resolves() -> None:
        """The blob-to-disk row resolves its Rust target through
        `METHOD_BINDING_OVERRIDES` to `FlatFiles::to_path`, alongside the
        divergent Python / TS / C++ homes."""
        rows = [
            {
                "class": "FlatFilesNamespace",
                "name": "flatFileToPath",
                "python": True,
                "typescript": True,
                "cpp": True,
                "rust": True,
            }
        ]
        py_methods = {"Client": {"flatfile_to_path"}}
        ts_methods = {"Client": {"flatFileToPath"}}
        cpp_methods = {"FlatFiles": {"to_path"}}
        rust_methods = {"FlatFiles": {"to_path"}}
        errors = _check_method_rows(rows, py_methods, ts_methods, cpp_methods, rust_methods)
        assert errors == [], (
            f"rust to_path override must resolve to FlatFiles::to_path; got {errors!r}"
        )

    def _case_method_rust_column_opt_in() -> None:
        """A row whose class is not in `RUST_METHOD_CLASS` and omits `rust` is
        not Rust-gated — the Rust column stays opt-in and never weakens the
        existing three-binding checks."""
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
        cpp_methods = {"FluentContract": {"quote"}}
        # No rust_methods passed at all (defaults to None) — must stay silent.
        errors = _check_method_rows(rows, py_methods, ts_methods, cpp_methods)
        assert errors == [], (
            f"unmapped class without rust column must not be Rust-gated; got {errors!r}"
        )

    _case("method positive — rust column resolves to the FlatFiles view", _case_method_rust_column_resolves)
    _case("method negative — rust column missing on view trips", _case_method_rust_column_missing_trips)
    _case("method positive — rust to_path override resolves", _case_method_rust_to_path_override_resolves)
    _case("method positive — rust column is opt-in for unmapped classes", _case_method_rust_column_opt_in)

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
            hpp = pathlib.Path(tmp) / "thetadatadx.hpp"
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
            hpp = pathlib.Path(tmp) / "thetadatadx.hpp"
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

    # ── Subscription-builder method resolution selftests ───────────

    def _case_builder_method_py_ts_aliases_resolve() -> None:
        """A `Contract` builder row resolves against the Python `PyContract`
        struct key and the TypeScript `ContractRef` impl key."""
        rows = [
            {
                "class": "Contract",
                "name": "quote",
                "python": True,
                "typescript": True,
                "cpp": True,
            }
        ]
        # Production collectors key Python by the Rust struct and TS by the
        # napi impl — the row uses the canonical `Contract` name.
        py_methods = {"PyContract": {"quote"}}
        ts_methods = {"ContractRef": {"quote"}}
        cpp_methods = {"FluentContract": {"quote"}}
        errors = _check_method_rows(rows, py_methods, ts_methods, cpp_methods)
        assert errors == [], (
            f"Contract builder must resolve through Py/TS aliases; got {errors!r}"
        )

    def _case_builder_method_sectype_full_stream_resolves() -> None:
        """`SecType.fullTrades` resolves on `PySecType` (py) + `SecType` (ts)."""
        rows = [
            {
                "class": "SecType",
                "name": "fullTrades",
                "python": True,
                "typescript": True,
                "cpp": True,
            }
        ]
        py_methods = {"PySecType": {"full_trades"}}
        ts_methods = {"SecType": {"fullTrades"}}
        cpp_methods = {"FluentSecType": {"full_trades"}}
        errors = _check_method_rows(rows, py_methods, ts_methods, cpp_methods)
        assert errors == [], (
            f"SecType full-stream builder must resolve; got {errors!r}"
        )

    def _case_builder_method_drop_trips() -> None:
        """Dropping a builder on one binding trips the per-method gate."""
        rows = [
            {
                "class": "Contract",
                "name": "quote",
                "python": True,
                "typescript": True,
                "cpp": True,
            }
        ]
        py_methods = {"PyContract": {"quote"}}
        ts_methods: dict[str, set[str]] = {"ContractRef": set()}  # dropped on TS
        cpp_methods = {"FluentContract": {"quote"}}
        errors = _check_method_rows(rows, py_methods, ts_methods, cpp_methods)
        assert any("typescript" in e and "missing" in e for e in errors), (
            f"a dropped builder must trip the gate; got {errors!r}"
        )

    _case("builder method — Contract Py/TS aliases resolve", _case_builder_method_py_ts_aliases_resolve)
    _case("builder method — SecType full-stream resolves", _case_builder_method_sectype_full_stream_resolves)
    _case("builder method — dropped builder on TS trips", _case_builder_method_drop_trips)

    # ── Client construction (connect) selftests ────────────────────

    def _case_connect_positive_all_bound() -> None:
        """A C-ABI-backed client present on every surface is silent."""
        rows = [
            {
                "name": "Client",
                "python": True,
                "typescript": True,
                "cpp": True,
                "ffi": True,
            }
        ]
        errors = _check_connect_rows(
            rows, {"Client"}, {"Client"}, {"Client"}, {"client"}
        )
        assert errors == [], f"all-bound connect row must be silent; got {errors!r}"

    def _case_connect_python_only_async_client() -> None:
        """`AsyncClient` is Python-only — no stem, ffi=false, silent."""
        rows = [
            {
                "name": "AsyncClient",
                "python": True,
                "typescript": False,
                "cpp": False,
                "ffi": False,
            }
        ]
        errors = _check_connect_rows(rows, {"AsyncClient"}, set(), set(), set())
        assert errors == [], (
            f"Python-only AsyncClient connect row must be silent; got {errors!r}"
        )

    def _case_connect_missing_python_constructor_trips() -> None:
        """Row claims a Python constructor but the pyclass has none — trips."""
        rows = [
            {
                "name": "Client",
                "python": True,
                "typescript": True,
                "cpp": True,
                "ffi": True,
            }
        ]
        errors = _check_connect_rows(
            rows, set(), {"Client"}, {"Client"}, {"client"}
        )
        assert any("python" in e and "missing" in e for e in errors), (
            f"a missing #[new] constructor must trip; got {errors!r}"
        )

    def _case_connect_orphan_untracked_trips() -> None:
        """A client constructing on a binding with no row is undocumented."""
        rows: list[dict[str, Any]] = []
        errors = _check_connect_rows(
            rows, {"Client"}, {"Client"}, set(), {"client"}
        )
        assert any("no [[connect]] row" in e for e in errors), (
            f"an untracked connect surface must trip; got {errors!r}"
        )

    def _case_connect_ffi_stem_collector_skips_from_file() -> None:
        """The FFI connect collector matches `_connect(` but not
        `_connect_from_file`."""
        with tempfile.TemporaryDirectory() as tmp:
            ffi_dir = pathlib.Path(tmp) / "ffi"
            ffi_dir.mkdir()
            (ffi_dir / "c.rs").write_text(
                'pub unsafe extern "C" fn thetadatadx_client_connect(a: i32) {}\n'
                'pub unsafe extern "C" fn thetadatadx_client_connect_from_file(p: i32) {}\n',
                encoding="utf-8",
            )
            stems = _collect_ffi_connect_stems(ffi_dir)
            assert "client" in stems, f"base connect stem must be seen; got {stems!r}"
            assert "client_connect_from" not in stems, (
                f"from_file must not leak into the connect stems; got {stems!r}"
            )

    _case("connect positive — all-bound client silent", _case_connect_positive_all_bound)
    _case("connect positive — Python-only AsyncClient silent", _case_connect_python_only_async_client)
    _case("connect negative — missing #[new] constructor trips", _case_connect_missing_python_constructor_trips)
    _case("connect negative — untracked connect surface trips", _case_connect_orphan_untracked_trips)
    _case("connect — FFI stem collector skips _connect_from_file", _case_connect_ffi_stem_collector_skips_from_file)

    # ── TypeScript connectWith option-field roster selftests ───────

    def _case_connect_with_field_collector_camelizes() -> None:
        """The field collector lifts Rust snake_case to JS camelCase."""
        with tempfile.TemporaryDirectory() as tmp:
            lib = pathlib.Path(tmp) / "lib.rs"
            lib.write_text(
                "#[napi(object)]\n"
                "pub struct ClientConnectOptions {\n"
                "    pub api_key_from_env: Option<bool>,\n"
                "    pub credentials_file: Option<String>,\n"
                "}\n",
                encoding="utf-8",
            )
            fields = _collect_typescript_connect_with_fields(lib)
            assert fields == {"apiKeyFromEnv", "credentialsFile"}, (
                f"collector must camelCase fields; got {fields!r}"
            )

    def _case_connect_with_field_roster_missing_field_trips() -> None:
        """A dropped/renamed connectWith field trips the roster."""
        actual = set(TYPESCRIPT_CONNECT_WITH_FIELD_ROSTER) - {"historicalType"}
        errors = _check_typescript_connect_with_field_roster(actual)
        assert any("historicalType" in e and "missing" in e for e in errors), (
            f"a missing connectWith field must trip; got {errors!r}"
        )

    def _case_connect_with_field_roster_unexpected_field_trips() -> None:
        """An extra connectWith field trips the roster."""
        actual = set(TYPESCRIPT_CONNECT_WITH_FIELD_ROSTER) | {"stageAlias"}
        errors = _check_typescript_connect_with_field_roster(actual)
        assert any("stageAlias" in e and "unexpected" in e for e in errors), (
            f"an unexpected connectWith field must trip; got {errors!r}"
        )

    def _case_connect_with_field_roster_live_source_clean() -> None:
        """The shipped `ClientConnectOptions` fields equal the pinned roster."""
        errors = _check_typescript_connect_with_field_roster(
            _collect_typescript_connect_with_fields(TS_LIB_RS)
        )
        assert errors == [], f"live connectWith field roster must be clean; got {errors!r}"

    _case("connectWith fields — collector camelCases Rust fields", _case_connect_with_field_collector_camelizes)
    _case("connectWith fields — missing field trips", _case_connect_with_field_roster_missing_field_trips)
    _case("connectWith fields — unexpected field trips", _case_connect_with_field_roster_unexpected_field_trips)
    _case("connectWith fields — live source clean", _case_connect_with_field_roster_live_source_clean)

    # ── C-ABI value-field + roster selftests ───────────────────────

    def _case_c_abi_struct_reader_reads_typedef_struct() -> None:
        """The C-ABI reader parses `typedef struct {...} <Name>;`."""
        with tempfile.TemporaryDirectory() as tmp:
            inc = pathlib.Path(tmp) / "structs.h.inc"
            inc.write_text(
                "typedef struct {\n"
                "const char *symbol;\n"
                "double strike;\n"
                "int32_t strike_thousandths;\n"
                "} ThetaDataDxContract;\n",
                encoding="utf-8",
            )
            assert (
                _c_abi_struct_field_type(inc, "ThetaDataDxContract", "strike_thousandths")
                == "int32_t"
            )
            assert (
                _c_abi_struct_field_type(inc, "ThetaDataDxContract", "strike") == "double"
            )

    def _case_value_field_roster_missing_trips() -> None:
        """A load-bearing class missing its pinned field trips the roster."""
        # `OptionContract.right` is in VALUE_FIELD_ROSTER; an empty matrix
        # must surface it (plus the other roster entries) as a gap.
        errors = _check_value_field_roster([])
        assert any("OptionContract.right" in e for e in errors), (
            f"roster gap must surface the unpinned field; got {errors!r}"
        )

    def _case_value_field_roster_complete_silent_on_roster() -> None:
        """With every roster pair present, the roster scan adds nothing
        for those pairs (vocabulary scan reads the live tree, so only the
        roster half is asserted here against a synthetic matrix)."""
        rows = [
            {"class": cls, "name": field}
            for cls, fields in VALUE_FIELD_ROSTER.items()
            for field in fields
        ]
        errors = _check_value_field_roster(rows)
        # No roster-half error should mention a roster pair.
        for cls, fields in VALUE_FIELD_ROSTER.items():
            for field in fields:
                assert not any(
                    f"{cls}.{field}: load-bearing" in e for e in errors
                ), f"roster pair {cls}.{field} must not be flagged; got {errors!r}"

    _case("value_field — C-ABI typedef-struct reader", _case_c_abi_struct_reader_reads_typedef_struct)
    _case("value_field — roster gap trips on missing pin", _case_value_field_roster_missing_trips)
    _case("value_field — roster silent when complete", _case_value_field_roster_complete_silent_on_roster)

    # ── Dotted anchor-row typo selftests ───────────────────────────

    def _case_anchor_row_valid_passes() -> None:
        """A recognized anchor suffix on a real class passes."""
        rows = [{"name": "GreeksEodTick.cross_binding_anchor", "python": True}]
        errors = _check_dotted_rows(
            rows, set(), set(), set(), set(), {"GreeksEodTick"}
        )
        assert errors == [], f"valid anchor must pass; got {errors!r}"

    def _case_anchor_row_typod_struct_trips() -> None:
        """An anchor on a nonexistent (typo'd) struct fails."""
        rows = [{"name": "GreksEodTick.cross_binding_anchor", "python": True}]
        errors = _check_dotted_rows(
            rows, set(), set(), set(), set(), {"GreeksEodTick"}
        )
        assert any("not a known binding class" in e for e in errors), (
            f"a typo'd anchor struct must trip; got {errors!r}"
        )

    def _case_anchor_row_bad_suffix_trips() -> None:
        """A dotted row on a non-config struct with an unknown suffix fails."""
        rows = [{"name": "GreeksEodTick.bogus_suffix", "python": True}]
        errors = _check_dotted_rows(
            rows, set(), set(), set(), set(), {"GreeksEodTick"}
        )
        assert any("unrecognized suffix" in e for e in errors), (
            f"an unrecognized anchor suffix must trip; got {errors!r}"
        )

    def _case_anchor_row_skipped_without_universe() -> None:
        """Without a class universe (selftest shape), anchor rows skip."""
        rows = [{"name": "Whatever.cross_binding_anchor", "python": True}]
        errors = _check_dotted_rows(rows, set(), set(), set(), set())
        assert errors == [], (
            f"anchor rows must skip when no universe is given; got {errors!r}"
        )

    _case("anchor row — valid suffix + real class passes", _case_anchor_row_valid_passes)
    _case("anchor row — typo'd struct trips", _case_anchor_row_typod_struct_trips)
    _case("anchor row — unrecognized suffix trips", _case_anchor_row_bad_suffix_trips)
    _case("anchor row — skipped without class universe", _case_anchor_row_skipped_without_universe)

    # ── StreamingSession exemption selftests ───────────────────────

    def _case_streaming_session_dunders_exempt() -> None:
        """The async-iterator / context dunders carry no contract — silent."""
        rows: list[dict[str, Any]] = []
        py_methods = {
            "StreamingSession": {"__aiter__", "__anext__", "__aenter__", "__aexit__"}
        }
        errors = _check_method_rows(rows, py_methods, {}, {})
        assert not any("StreamingSession" in e for e in errors), (
            f"session dunders must be exempt; got {errors!r}"
        )

    def _case_streaming_session_first_class_method_trips() -> None:
        """A first-class session method with no row trips the orphan scan."""
        rows: list[dict[str, Any]] = []
        py_methods = {"StreamingSession": {"drain_now"}}
        errors = _check_method_rows(rows, py_methods, {}, {})
        assert any("StreamingSession.drain_now" in e for e in errors), (
            f"an unenrolled first-class session method must trip; got {errors!r}"
        )

    _case("StreamingSession — async/context dunders exempt", _case_streaming_session_dunders_exempt)
    _case("StreamingSession — unenrolled first-class method trips", _case_streaming_session_first_class_method_trips)

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
                "impl Client {\n"
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
        """The FFI collector strips the `thetadatadx_` prefix to the bare name."""
        with tempfile.TemporaryDirectory() as tmp:
            ffi_dir = pathlib.Path(tmp) / "ffi"
            ffi_dir.mkdir()
            (ffi_dir / "utility.rs").write_text(
                'pub unsafe extern "C" fn thetadatadx_all_greeks() {}\n'
                'pub unsafe extern "C" fn thetadatadx_implied_volatility() {}\n',
                encoding="utf-8",
            )
            found = _collect_ffi_utility_functions(ffi_dir)
            assert {"all_greeks", "implied_volatility"} <= found, (
                f"thetadatadx_-prefixed symbols must map to bare names; got {found!r}"
            )

    _case("utility positive — all four bindings expose the calculator", _case_utility_positive_all_four_bound)
    _case("utility negative — calculator missing on TS trips", _case_utility_negative_missing_on_ts)
    _case("utility negative — unexpected C++ decl trips", _case_utility_negative_unexpected)
    _case("utility — TS collector skips impl methods", _case_utility_ts_free_fn_collector_skips_methods)
    _case("utility — FFI collector strips thetadatadx_ prefix", _case_utility_ffi_collector_strips_prefix)

    # ── Historical server-stream surface selftests ────────────────

    def _case_hist_stream_positive_all_bound() -> None:
        """An endpoint streaming on every declared surface is silent."""
        rows = [
            {
                "name": "option_history_trade",
                "rust": True,
                "python": True,
                "typescript": True,
                "cpp": True,
                "ffi": True,
            }
        ]
        s = {"option_history_trade"}
        errors = _check_historical_streaming_rows(rows, s, s, s, s, s)
        assert errors == [], f"all-bound row must be silent; got {errors!r}"

    def _case_hist_stream_missing_on_cpp_trips() -> None:
        """Row claims C++ streams but the C++ member is absent — trips."""
        rows = [
            {
                "name": "option_history_trade",
                "rust": True,
                "python": True,
                "typescript": True,
                "cpp": True,
                "ffi": True,
            }
        ]
        bound = {"option_history_trade"}
        errors = _check_historical_streaming_rows(
            rows, bound, bound, bound, set(), bound
        )
        assert any("cpp" in e and "missing" in e for e in errors), (
            f"missing C++ stream member must trip; got {errors!r}"
        )

    def _case_hist_stream_missing_on_rust_trips() -> None:
        """Row claims Rust streams but the registry has no such builder —
        the dropped / renamed Rust endpoint defect this column closes.
        """
        rows = [
            {
                "name": "option_history_trade",
                "rust": True,
                "python": True,
                "typescript": True,
                "cpp": True,
                "ffi": True,
            }
        ]
        bound = {"option_history_trade"}
        # Rust set is empty — the endpoint vanished from the registry of
        # record while the bindings still declare it.
        errors = _check_historical_streaming_rows(
            rows, set(), bound, bound, bound, bound
        )
        assert any("rust" in e and "missing" in e for e in errors), (
            f"a dropped Rust streaming endpoint must trip; got {errors!r}"
        )

    def _case_hist_stream_ts_only_state_is_silent() -> None:
        """The managed-only ship state (rust+py+ts true, cpp+ffi false) is
        silent when Rust + Python + TS stream and C++ + FFI do not — the
        documented `option_list_contracts` language-only variant the matrix
        tracks.
        """
        rows = [
            {
                "name": "option_list_contracts",
                "rust": True,
                "python": True,
                "typescript": True,
                "cpp": False,
                "ffi": False,
            }
        ]
        bound = {"option_list_contracts"}
        errors = _check_historical_streaming_rows(
            rows, bound, bound, bound, set(), set()
        )
        assert errors == [], f"managed-only state must be silent; got {errors!r}"

    def _case_hist_stream_untracked_orphan_trips() -> None:
        """An endpoint streaming on a surface with no row at all trips
        the reverse-direction orphan check.
        """
        errors = _check_historical_streaming_rows(
            [], {"option_history_trade"}, set(), set(), set(), set()
        )
        assert any(
            "option_history_trade" in e and "no [[historical_streaming]] row" in e
            for e in errors
        ), f"untracked streaming endpoint must trip; got {errors!r}"

    def _case_hist_stream_rust_mirrors_generated() -> None:
        """The Rust streaming classification (registry-of-record mirror of
        the build's `endpoint_streams` SSOT) must equal the live generated
        Python `fn stream` surface — both are emitted from the same
        registry, so any desync between the gate's mirror and the build's
        predicate is a real drift the gate must surface.
        """
        rust_stream = _collect_rust_streaming_endpoints(ENDPOINT_SURFACE_TOML)
        py_stream = _collect_python_streaming_endpoints(PY_SRC)
        assert rust_stream == py_stream, (
            f"Rust streaming mirror must equal the generated Python stream "
            f"surface; rust-only={sorted(rust_stream - py_stream)!r}, "
            f"py-only={sorted(py_stream - rust_stream)!r}"
        )

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

    _case("hist-stream positive — all five surfaces stream", _case_hist_stream_positive_all_bound)
    _case("hist-stream negative — missing C++ member trips", _case_hist_stream_missing_on_cpp_trips)
    _case("hist-stream negative — dropped Rust endpoint trips", _case_hist_stream_missing_on_rust_trips)
    _case("hist-stream positive — managed-only ship state is silent", _case_hist_stream_ts_only_state_is_silent)
    _case("hist-stream negative — untracked streaming endpoint trips", _case_hist_stream_untracked_orphan_trips)
    _case("hist-stream — Rust mirror equals generated Python stream surface", _case_hist_stream_rust_mirrors_generated)
    _case("hist-stream — initialism-aware inverse agrees across bindings", _case_hist_stream_initialism_inverse)

    # ── Historical async query surface selftests ──────────────────

    def _case_hist_async_positive_all_bound() -> None:
        """An endpoint with an async query on every declared surface is
        silent."""
        rows = [
            {
                "name": "stock_history_eod",
                "rust": True,
                "python": True,
                "typescript": True,
                "cpp": True,
            }
        ]
        s = {"stock_history_eod"}
        errors = _check_historical_async_rows(rows, s, s, s, s)
        assert errors == [], f"all-bound async row must be silent; got {errors!r}"

    def _case_hist_async_missing_on_cpp_trips() -> None:
        """Row claims C++ exposes the async query but the `_async` member is
        absent — trips."""
        rows = [
            {
                "name": "stock_history_eod",
                "rust": True,
                "python": True,
                "typescript": True,
                "cpp": True,
            }
        ]
        bound = {"stock_history_eod"}
        errors = _check_historical_async_rows(rows, bound, bound, bound, set())
        assert any("cpp" in e and "missing" in e for e in errors), (
            f"missing C++ async member must trip; got {errors!r}"
        )

    def _case_hist_async_missing_on_rust_trips() -> None:
        """Row claims Rust exposes the async query but the registry has no
        such buffered endpoint — the dropped / renamed Rust endpoint defect
        this column closes."""
        rows = [
            {
                "name": "stock_history_eod",
                "rust": True,
                "python": True,
                "typescript": True,
                "cpp": True,
            }
        ]
        bound = {"stock_history_eod"}
        errors = _check_historical_async_rows(rows, set(), bound, bound, bound)
        assert any("rust" in e and "missing" in e for e in errors), (
            f"a dropped Rust async endpoint must trip; got {errors!r}"
        )

    def _case_hist_async_untracked_orphan_trips() -> None:
        """An endpoint with an async query on a surface but no row at all
        trips the reverse-direction orphan check."""
        errors = _check_historical_async_rows(
            [], {"stock_history_eod"}, set(), set(), set()
        )
        assert any(
            "stock_history_eod" in e and "no [[historical_async]] row" in e
            for e in errors
        ), f"untracked async endpoint must trip; got {errors!r}"

    def _case_hist_async_cpp_collector_strips_suffix() -> None:
        """The C++ collector recovers the endpoint name from an `_async`
        member and ignores the blocking member of the same name."""
        cpp_methods = {
            "Historical": {"stock_history_eod", "stock_history_eod_async"}
        }
        found = _collect_cpp_async_endpoints(cpp_methods)
        assert found == {"stock_history_eod"}, (
            f"collector must strip `_async` and ignore the blocking twin; "
            f"got {found!r}"
        )

    _case("hist-async positive — all four surfaces expose async query", _case_hist_async_positive_all_bound)
    _case("hist-async negative — missing C++ member trips", _case_hist_async_missing_on_cpp_trips)
    _case("hist-async negative — dropped Rust endpoint trips", _case_hist_async_missing_on_rust_trips)
    _case("hist-async negative — untracked async endpoint trips", _case_hist_async_untracked_orphan_trips)
    _case("hist-async — C++ collector strips `_async` suffix", _case_hist_async_cpp_collector_strips_suffix)

    # ── Historical buffered base surface selftests ────────────────

    def _case_hist_base_positive_all_five() -> None:
        """An endpoint present on all five surfaces is silent."""
        rows = [
            {
                "name": "stock_history_eod",
                "rust": True,
                "python": True,
                "typescript": True,
                "cpp": True,
                "ffi": True,
            }
        ]
        s = {"stock_history_eod"}
        errors = _check_historical_base_rows(rows, s, s, s, s, s, s)
        assert errors == [], f"all-five-surface base row must be silent; got {errors!r}"

    def _case_hist_base_missing_on_cabi_trips() -> None:
        """Row claims the C-ABI base symbol exists but the shipped header
        does not declare it — the exact 61-symbol blind spot this family
        closes."""
        rows = [
            {
                "name": "stock_history_eod",
                "rust": True,
                "python": True,
                "typescript": True,
                "cpp": True,
                "ffi": True,
            }
        ]
        s = {"stock_history_eod"}
        # `cabi_base` AND `ffi_base` empty — the `_with_options` base symbol
        # is gone everywhere on the C side.
        errors = _check_historical_base_rows(rows, s, s, s, s, set(), set())
        assert any("ffi" in e and "missing" in e for e in errors), (
            f"missing C-ABI base symbol must trip; got {errors!r}"
        )

    def _case_hist_base_missing_on_rust_trips() -> None:
        """Row claims the Rust buffered method exists but the registry of
        record has no such endpoint — the dropped / renamed Rust endpoint
        defect."""
        rows = [
            {
                "name": "stock_history_eod",
                "rust": True,
                "python": True,
                "typescript": True,
                "cpp": True,
                "ffi": True,
            }
        ]
        s = {"stock_history_eod"}
        errors = _check_historical_base_rows(rows, set(), s, s, s, s, s)
        assert any("rust" in e and "missing" in e for e in errors), (
            f"a dropped Rust buffered endpoint must trip; got {errors!r}"
        )

    def _case_hist_base_header_source_divergence_trips() -> None:
        """A shipped header that dropped a base symbol the `thetadatadx-ffi/src` source
        still defines (a stale regenerated header) trips, independent of any
        per-row column."""
        rows = [
            {
                "name": "stock_history_eod",
                "rust": True,
                "python": True,
                "typescript": True,
                "cpp": True,
                "ffi": True,
            }
        ]
        s = {"stock_history_eod"}
        # Header is missing `option_history_eod` that the source defines.
        cabi = {"stock_history_eod"}
        ffi = {"stock_history_eod", "option_history_eod"}
        errors = _check_historical_base_rows(rows, s, s, s, s, cabi, ffi)
        assert any(
            "option_history_eod" in e and "stale header" in e for e in errors
        ), f"a header/source divergence must trip; got {errors!r}"

    def _case_hist_base_untracked_orphan_trips() -> None:
        """An endpoint present on a surface but with no row at all trips the
        reverse-direction orphan scan."""
        errors = _check_historical_base_rows(
            [], {"stock_history_eod"}, set(), set(), set(), set(), set()
        )
        assert any(
            "stock_history_eod" in e and "no [[historical_base]] row" in e
            for e in errors
        ), f"untracked base endpoint must trip; got {errors!r}"

    def _case_hist_base_live_sources_clean() -> None:
        """The live buffered base surface is symmetric across all five
        surfaces: every one of the 61 endpoints present on Rust / Python /
        TypeScript / C++ / the C-ABI base, and the shipped header agrees with
        the `thetadatadx-ffi/src` source and the Rust registry."""
        data = tomllib.loads(PARITY_TOML.read_text(encoding="utf-8"))
        rows = data.get("historical_base", [])
        assert rows, "live parity.toml must declare [[historical_base]] rows"
        ts_methods = _collect_typescript_class_methods(TS_SRC)
        cpp_methods = _collect_cpp_class_methods(CPP_HPP)
        errors = _check_historical_base_rows(
            rows,
            _collect_rust_buffered_endpoints(ENDPOINT_SURFACE_TOML),
            _collect_python_buffered_endpoints(PY_SRC),
            _collect_typescript_async_endpoints(ts_methods),
            cpp_methods.get(_cpp_class_for("HistoricalView"), set()),
            _collect_cabi_base_endpoints(ENDPOINT_WITH_OPTIONS_INC),
            _collect_ffi_base_endpoints(FFI_SRC),
        )
        assert errors == [], f"live buffered base surface must be clean; got {errors!r}"

    def _case_hist_base_registry_count() -> None:
        """The registry of record yields exactly the 61 buffered endpoints
        (the four `*_stream` FPSS subscription endpoints excluded), and the
        shipped C-ABI header declares the same 61 base symbols."""
        rust = _collect_rust_buffered_endpoints(ENDPOINT_SURFACE_TOML)
        cabi = _collect_cabi_base_endpoints(ENDPOINT_WITH_OPTIONS_INC)
        assert len(rust) == 61, f"registry must yield 61 buffered endpoints; got {len(rust)}"
        assert rust == cabi, (
            f"registry buffered set must equal the C-ABI base set; "
            f"rust-only={sorted(rust - cabi)!r}, cabi-only={sorted(cabi - rust)!r}"
        )

    _case("hist-base positive — all five surfaces present", _case_hist_base_positive_all_five)
    _case("hist-base negative — missing C-ABI base symbol trips", _case_hist_base_missing_on_cabi_trips)
    _case("hist-base negative — dropped Rust endpoint trips", _case_hist_base_missing_on_rust_trips)
    _case("hist-base negative — header/source divergence trips", _case_hist_base_header_source_divergence_trips)
    _case("hist-base negative — untracked endpoint trips", _case_hist_base_untracked_orphan_trips)
    _case("hist-base — live sources clean", _case_hist_base_live_sources_clean)
    _case("hist-base — registry yields 61 endpoints matching C-ABI base", _case_hist_base_registry_count)

    # ── Client construction-from-file surface selftests ───────────

    def _case_from_file_positive_all_bound() -> None:
        """A client exposing file construction on every declared binding
        (with the idiomatic per-binding spelling) is silent."""
        rows = [
            {
                "name": "HistoricalClient",
                "python": True,
                "typescript": True,
                "cpp": True,
                "ffi": True,
            }
        ]
        errors = _check_from_file_rows(
            rows,
            {"HistoricalClient"},
            {"HistoricalClient"},
            {"HistoricalClient"},
            {"historical"},
        )
        assert errors == [], f"all-bound row must be silent; got {errors!r}"

    def _case_from_file_missing_on_ffi_trips() -> None:
        """Row claims the C ABI exposes file construction but the
        `thetadatadx_<stem>_connect_from_file` symbol is absent — trips."""
        rows = [
            {
                "name": "StreamingClient",
                "python": True,
                "typescript": True,
                "cpp": True,
                "ffi": True,
            }
        ]
        errors = _check_from_file_rows(
            rows,
            {"StreamingClient"},
            {"StreamingClient"},
            {"StreamingClient"},
            set(),
        )
        assert any("ffi" in e and "missing" in e for e in errors), (
            f"missing C ABI symbol must trip; got {errors!r}"
        )

    def _case_from_file_untracked_orphan_trips() -> None:
        """A client exposing file construction on a binding with no row
        at all trips the reverse-direction orphan check."""
        errors = _check_from_file_rows(
            [], {"Client"}, set(), set(), set()
        )
        assert any(
            "Client" in e and "no [[from_file]] row" in e
            for e in errors
        ), f"untracked file-construction client must trip; got {errors!r}"

    def _case_from_file_ffi_stem_maps_class_name() -> None:
        """The FFI stem table bridges the class name to its C ABI symbol
        stem: `Client` resolves via `client`, not its own
        name. A row declaring ffi=true is silent only when the matching
        stem symbol exists."""
        rows = [
            {
                "name": "Client",
                "python": False,
                "typescript": False,
                "cpp": False,
                "ffi": True,
            }
        ]
        # The class name itself in the stem set must NOT satisfy the row;
        # only the mapped `client` stem does.
        errors_wrong = _check_from_file_rows(
            rows, set(), set(), set(), {"theta_data_dx"}
        )
        assert any("ffi" in e for e in errors_wrong), (
            f"class-name stem must not satisfy the row; got {errors_wrong!r}"
        )
        errors_right = _check_from_file_rows(rows, set(), set(), set(), {"client"})
        assert errors_right == [], (
            f"mapped `client` stem must satisfy the row; got {errors_right!r}"
        )

    def _case_from_file_live_sources_clean() -> None:
        """The shipped bindings expose `from_file` (idiomatic spelling) on
        every client the live `[[from_file]]` rows declare."""
        data = tomllib.loads(PARITY_TOML.read_text(encoding="utf-8"))
        rows = data.get("from_file", [])
        assert rows, "live parity.toml must declare [[from_file]] rows"
        py_methods = _collect_python_class_methods(PY_SRC)
        ts_methods = _collect_typescript_class_methods(TS_SRC)
        cpp_methods = _collect_cpp_class_methods(CPP_HPP)
        errors = _check_from_file_rows(
            rows,
            _collect_python_from_file_classes(py_methods),
            _collect_typescript_from_file_classes(ts_methods),
            _collect_cpp_from_file_classes(cpp_methods),
            _collect_ffi_from_file_stems(FFI_SRC),
        )
        assert errors == [], f"live from_file sources must be clean; got {errors!r}"

    _case("from-file positive — all four bindings construct", _case_from_file_positive_all_bound)
    _case("from-file negative — missing C ABI symbol trips", _case_from_file_missing_on_ffi_trips)
    _case("from-file negative — untracked client trips", _case_from_file_untracked_orphan_trips)
    _case("from-file — FFI stem table maps class name", _case_from_file_ffi_stem_maps_class_name)
    _case("from-file — live sources clean", _case_from_file_live_sources_clean)

    # ── Credentials factory surface selftests ──────────────────────

    def _case_credentials_factory_camel_folds() -> None:
        """The snake_case → camelCase fold matches the row spelling."""
        assert _credentials_factory_camel("from_env_or_file") == "fromEnvOrFile"
        assert _credentials_factory_camel("from_api_key_with_email") == "fromApiKeyWithEmail"
        assert _credentials_factory_camel("from_file") == "fromFile"

    def _case_credentials_factory_positive_all_bound() -> None:
        """Every roster factory present on each required binding, with a
        matching `[[method]]` row, is silent."""
        rows = [
            {"class": "Credentials", "name": name}
            for name in CREDENTIALS_FACTORY_ROSTER
        ]
        four = {
            "fromFile",
            "fromApiKey",
            "fromApiKeyWithEmail",
            "fromEnvOrFile",
            "fromDotenv",
        }
        py = set(four)
        ts = set(four)
        cpp = four | {"fromEmail"}
        ffi = four | {"fromEmail"}
        errors = _check_credentials_factory_rows(rows, py, ts, cpp, ffi)
        assert errors == [], f"fully-bound credentials surface must be silent; got {errors!r}"

    def _case_credentials_factory_untracked_orphan_trips() -> None:
        """A `Credentials` factory harvested from a binding with no
        `[[method]]` row trips the reverse-orphan scan."""
        errors = _check_credentials_factory_rows(
            [],  # no rows declared at all
            {"fromEnvOrFile"},
            set(),
            set(),
            set(),
        )
        assert any(
            "fromEnvOrFile" in e and "no [[method]] row" in e for e in errors
        ), f"untracked credentials factory must trip; got {errors!r}"

    def _case_credentials_factory_roster_gap_trips() -> None:
        """A governed factory missing from a binding the roster lists trips
        even when its row still exists (asymmetric removal)."""
        rows = [
            {"class": "Credentials", "name": name}
            for name in CREDENTIALS_FACTORY_ROSTER
        ]
        four = {
            "fromFile",
            "fromApiKey",
            "fromApiKeyWithEmail",
            "fromEnvOrFile",
            "fromDotenv",
        }
        py = four - {"fromEnvOrFile"}  # dropped from Python only
        ts = set(four)
        cpp = four | {"fromEmail"}
        ffi = four | {"fromEmail"}
        errors = _check_credentials_factory_rows(rows, py, ts, cpp, ffi)
        assert any(
            "fromEnvOrFile" in e and "missing from the python binding" in e
            for e in errors
        ), f"roster gap on a binding must trip; got {errors!r}"

    def _case_credentials_factory_live_sources_clean() -> None:
        """The shipped bindings expose a symmetric, fully-tracked
        `Credentials` factory surface."""
        data = tomllib.loads(PARITY_TOML.read_text(encoding="utf-8"))
        method_rows = data.get("method", [])
        py_methods = _collect_python_class_methods(PY_SRC)
        ts_methods = _collect_typescript_class_methods(TS_SRC)
        cpp_methods = _collect_cpp_class_methods(CPP_HPP)
        errors = _check_credentials_factory_rows(
            method_rows,
            _collect_python_credentials_factories(py_methods),
            _collect_typescript_credentials_factories(ts_methods),
            _collect_cpp_credentials_factories(cpp_methods),
            _collect_ffi_credentials_factories(FFI_SRC),
        )
        assert errors == [], f"live credentials factory surface must be clean; got {errors!r}"

    _case("credentials — camelCase fold matches row spelling", _case_credentials_factory_camel_folds)
    _case("credentials positive — all bindings + rows silent", _case_credentials_factory_positive_all_bound)
    _case("credentials negative — untracked factory trips", _case_credentials_factory_untracked_orphan_trips)
    _case("credentials negative — roster gap on a binding trips", _case_credentials_factory_roster_gap_trips)
    _case("credentials — live sources clean", _case_credentials_factory_live_sources_clean)

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
        NEVER trip — `HistoricalClient`, `historical_host`,
        `setStreamingRingSize`, `streaming_ring_size` are all clean.
        """
        errors = _check_public_surface_vocab(
            {"HistoricalClient", "StreamingClient", "StreamEvent"},
            set(),
            set(),
            {"historical_host", "historical_port", "streaming_ring_size", "streaming_host_selection"},
            {"streaming_ring_size"},
            set(),
            set(),
            {"StreamingClient": {"subscribe"}},
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
        assert _normalize_setter("streaming_host_shuffle_seed_explicit") == "streaming_host_shuffle_seed"

    def _case_setter_set_parity_positive_after_normalize() -> None:
        """The four sets, spelled in their per-binding idioms, compare
        equal after normalization — the gate is silent.
        """
        py = {"worker_threads", "flatfiles_jitter", "flush_mode"}
        ts = {"worker_threads_explicit", "flat_files_jitter", "flatfiles_jitter", "flush_mode"}
        cpp = {"worker_threads_explicit", "flatfiles_jitter", "flush_mode"}
        ffi = {"worker_threads_explicit", "flatfiles_jitter", "flush_mode"}
        errors = _check_setter_set_parity(py, ts, cpp, ffi, exempt={})
        assert errors == [], (
            f"normalized-equal sets must be silent; got {errors!r}"
        )

    def _case_setter_set_parity_missing_on_one_binding_trips() -> None:
        """A knob bound on three bindings but absent from TS trips — the
        `flush_mode`-missing-on-TS defect class.
        """
        py = {"flush_mode"}
        ts: set[str] = set()
        cpp = {"flush_mode"}
        ffi = {"flush_mode"}
        errors = _check_setter_set_parity(py, ts, cpp, ffi, exempt={})
        assert any("flush_mode" in e and "typescript" in e for e in errors), (
            f"missing-on-TS knob must trip the set-parity gate; got {errors!r}"
        )

    def _case_setter_set_parity_honours_exemption() -> None:
        """A Python-only knob listed in the exemption map does NOT trip
        — the documented per-language-idiom carve-out.
        """
        py = {"historical_host", "shared"}
        ts = {"shared"}
        cpp = {"shared"}
        ffi = {"shared"}
        errors = _check_setter_set_parity(
            py, ts, cpp, ffi, exempt={"historical_host": "Python-only advanced override"}
        )
        assert errors == [], (
            f"exempted Python-only knob must not trip; got {errors!r}"
        )

    def _case_setter_set_parity_stale_exemption_trips() -> None:
        """An exempted knob that is now uniformly bound on every binding
        is a stale carve-out and trips so the list never rots.
        """
        allfour = {"historical_host"}
        errors = _check_setter_set_parity(
            allfour,
            allfour,
            allfour,
            allfour,
            exempt={"historical_host": "Python-only advanced override"},
        )
        assert any("historical_host" in e and "stale" in e for e in errors), (
            f"uniformly-bound exemption must surface as stale; got {errors!r}"
        )

    def _case_setter_set_parity_shipped_exemption_is_live() -> None:
        """The shipped `SETTER_PARITY_EXEMPT` carve-outs must be live
        against the real binding sources — `historical_host` /
        `historical_port` present on every binding, so the carve-out map
        stays empty and the live gate stays silent on them.
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
            {"streaming_ring_size"},
            {"streaming_ring_size"},
            {"streaming_ring_size"},
            set(),
            exempt={},
        )
        assert any("streaming_ring_size" in e and "ffi" in e for e in errors), (
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
            "#[pymethods]\nimpl Config {\n    #[getter] fn get_streaming_ring_size(&self) -> usize { 0 }\n}\n"
            "#[pymethods]\nimpl QuoteTick {\n    #[getter] fn bid_price(&self) -> f64 { 0.0 }\n}\n"
        )
        bodies = _iter_impl_config_bodies(py_text)
        assert len(bodies) == 1, f"only the Config impl body must be picked; got {bodies!r}"
        assert "get_streaming_ring_size" in bodies[0]
        assert "bid_price" not in bodies[0]

    _case("getter-set — normalized-equal sets are silent", _case_getter_set_parity_positive)
    _case("getter-set — missing-on-FFI getter trips", _case_getter_set_parity_missing_on_ffi_trips)
    _case("getter-set — live sources clean", _case_getter_set_parity_live_sources_clean)
    _case("getter collectors — scope to impl Config only", _case_getter_collectors_scope_to_config)

    # ── ClientBuilder fluent-setter parity selftests ──────────────

    def _case_client_builder_setter_parity_positive() -> None:
        """Matching Rust/C++ builder setter rosters are silent."""
        errors = _check_client_builder_setter_parity(
            {"api_key", "environment", "from_dotenv"},
            {"api_key", "environment", "from_dotenv"},
            exempt={},
        )
        assert errors == [], f"matching builder setter sets must be silent; got {errors!r}"

    def _case_client_builder_setter_missing_on_cpp_trips() -> None:
        """A Rust builder setter missing from C++ trips."""
        errors = _check_client_builder_setter_parity(
            {"api_key", "environment"},
            {"api_key"},
            exempt={},
        )
        assert any("environment" in e and "cpp" in e for e in errors), (
            f"a dropped C++ builder setter must trip; got {errors!r}"
        )

    def _case_client_builder_setter_stale_exemption_trips() -> None:
        """A stale builder-setter exemption is an error."""
        errors = _check_client_builder_setter_parity(
            {"from_dotenv"},
            {"from_dotenv"},
            exempt={"from_dotenv": "legacy C++ gap"},
        )
        assert any("from_dotenv" in e and "stale" in e for e in errors), (
            f"a stale builder-setter exemption must surface; got {errors!r}"
        )

    def _case_client_builder_setter_live_sources_clean() -> None:
        """The shipped Rust and C++ `ClientBuilder` setter rosters match."""
        errors = _check_client_builder_setter_parity(
            _collect_rust_client_builder_setters(RUST_CLIENT_BUILDER_RS),
            _collect_cpp_client_builder_setters(CPP_HPP),
        )
        assert errors == [], f"live builder setter parity must be clean; got {errors!r}"

    _case("ClientBuilder setters — matching rosters silent", _case_client_builder_setter_parity_positive)
    _case("ClientBuilder setters — missing on C++ trips", _case_client_builder_setter_missing_on_cpp_trips)
    _case("ClientBuilder setters — stale exemption trips", _case_client_builder_setter_stale_exemption_trips)
    _case("ClientBuilder setters — live sources clean", _case_client_builder_setter_live_sources_clean)

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
        bad_codes["THETADATADX_ERR_STREAM"] = 99
        errors = _check_error_leaf_parity(
            leaves,
            leaves,
            leaves,
            bad_codes,
            set(CANONICAL_ERROR_CODES),
            bad_codes,
        )
        assert any("ffi" in e and "THETADATADX_ERR_STREAM" in e for e in errors), (
            f"a renumbered FFI code must trip; got {errors!r}"
        )

    def _case_error_leaf_header_drift_trips() -> None:
        """A C ABI header `#define` disagreeing with the FFI Rust constant
        (invisible to `cargo build`) trips.
        """
        leaves = set(CANONICAL_ERROR_LEAVES)
        header_codes = dict(CANONICAL_ERROR_CODES)
        header_codes["THETADATADX_ERR_CONFIG"] = 42
        errors = _check_error_leaf_parity(
            leaves,
            leaves,
            leaves,
            dict(CANONICAL_ERROR_CODES),
            set(CANONICAL_ERROR_CODES),
            header_codes,
        )
        assert any("cpp header" in e and "THETADATADX_ERR_CONFIG" in e for e in errors), (
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
        `thetadatadx_condition_is_cancel` on the C ABI.
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

    # ── Source comment-stripping selftests ─────────────────────────
    # The `.rs` / `.hpp` / `.h` / `.inc` collectors strip C-style
    # comments before their symbol regexes run, so a symbol that survives
    # only inside a comment (a deleted-but-not-removed declaration like
    # `// removed: historical()`) no longer reads as present. Without the
    # strip, a `DOTALL` collector regex spans the comment markers and a
    # ghost symbol passes — masking real cross-binding drift.

    def _case_strip_source_comments_helper() -> None:
        """`_read_source` removes both line and block comments."""
        raw = (
            "pub fn live() {}\n"
            "// pub fn ghost_line() {}\n"
            "/* pub fn ghost_block() {} */\n"
            "/// removed: historical()\n"
        )
        with tempfile.TemporaryDirectory() as td:
            p = pathlib.Path(td) / "x.rs"
            p.write_text(raw, encoding="utf-8")
            stripped = _read_source(p)
        assert "live" in stripped, "a live declaration must survive stripping"
        for ghost in ("ghost_line", "ghost_block", "historical"):
            assert ghost not in stripped, (
                f"`{ghost}` survived only in a comment but was not stripped"
            )

    def _case_strip_pyclass_in_comment_ignored() -> None:
        """A commented-out `#[pyclass]` is NOT collected, while a live one
        is — the deleted-symbol-in-comment hole, proven end-to-end through
        the real `collect_python_classes` collector.
        """
        src = (
            "#[pyclass]\n"
            "pub struct LiveClass {}\n"
            "\n"
            "// #[pyclass]\n"
            "// pub struct GhostLineClass {}\n"
            "\n"
            "/* #[pyclass]\n"
            "   pub struct GhostBlockClass {} */\n"
        )
        with tempfile.TemporaryDirectory() as td:
            root = pathlib.Path(td)
            (root / "lib.rs").write_text(src, encoding="utf-8")
            classes = collect_python_classes(root)
        assert "LiveClass" in classes, (
            f"a live #[pyclass] must be collected; got {sorted(classes)!r}"
        )
        assert "GhostLineClass" not in classes, (
            "a #[pyclass] surviving only in a line comment was collected — "
            f"the comment-strip hole is open; got {sorted(classes)!r}"
        )
        assert "GhostBlockClass" not in classes, (
            "a #[pyclass] surviving only in a block comment was collected — "
            f"the comment-strip hole is open; got {sorted(classes)!r}"
        )

    def _case_strip_cpp_includes_comments() -> None:
        """`_read_cpp_expanded` inlines `.inc` fragments AND strips
        comments from both the host header and the inlined fragment, so a
        commented-out declaration in either is not seen.
        """
        with tempfile.TemporaryDirectory() as td:
            root = pathlib.Path(td)
            (root / "frag.inc").write_text(
                "void thetadatadx_live_inc(void);\n"
                "// void thetadatadx_ghost_inc(void);\n",
                encoding="utf-8",
            )
            hpp = root / "header.hpp"
            hpp.write_text(
                '#include "frag.inc"\n'
                "void thetadatadx_live_hpp(void);\n"
                "/* void thetadatadx_ghost_hpp(void); */\n",
                encoding="utf-8",
            )
            text = _read_cpp_expanded(hpp)
        assert "thetadatadx_live_inc" in text and "thetadatadx_live_hpp" in text, (
            "live declarations from the header and the inlined .inc must "
            f"survive; got: {text!r}"
        )
        for ghost in ("thetadatadx_ghost_inc", "thetadatadx_ghost_hpp"):
            assert ghost not in text, (
                f"`{ghost}` survived only in a comment after include "
                "expansion but was not stripped"
            )

    _case("comment-strip — _read_source drops line + block comments", _case_strip_source_comments_helper)
    _case("comment-strip — commented-out #[pyclass] not collected", _case_strip_pyclass_in_comment_ignored)
    _case("comment-strip — _read_cpp_expanded strips host + .inc comments", _case_strip_cpp_includes_comments)

    # ── Route-B method-signature infrastructure (Phase 3) ──────────
    # The signature gate is opt-in per row, so it governs ZERO rows on the
    # real surface today (a verified no-op until Phase 4 authors specs).
    # These probes ARE the proof the infrastructure works: each extractor
    # reads a synthetic source, and the orchestrator catches every drift
    # axis (type / arity / order / return) while honouring a type-map
    # sanction and a per-binding override.

    def _case_sig_type_map_forward_and_sanction() -> None:
        """Forward map: a canonical type is satisfied by its accepted binding
        spellings, including the `usize`→napi-`f64` widening sanction. A
        spelling outside the cell, or an unknown canonical name, fails closed."""
        assert _sig_type_agrees("usize", "usize", "python")
        assert _sig_type_agrees("usize", "f64", "ts_napi"), "usize→f64 sanction"
        assert _sig_type_agrees("usize", "size_t", "cpp")
        assert _sig_type_agrees("String", "const std::string&", "cpp")
        assert _sig_type_agrees("bool", "i32", "ffi"), "bool→i32 over the ABI"
        assert not _sig_type_agrees("i32", "f64", "ts_napi"), "wrong-cell spelling"
        assert not _sig_type_agrees("usize", "HashMap<u8, u8>", "cpp"), "unmapped"
        assert not _sig_type_agrees("MysteryType", "size_t", "cpp"), "unknown canon"
        # Zero overlap between platform-width `usize` and fixed-width `u64` on
        # the C++ / C-ABI cells: a `u64` row that drifts to the platform-width
        # spelling fails closed, and vice-versa. Before the split both spellings
        # were accepted for both canonicals, so either drift passed silently.
        assert _sig_type_agrees("u64", "uint64_t", "cpp")
        assert not _sig_type_agrees("u64", "size_t", "cpp"), "u64 ≠ size_t (cpp)"
        assert not _sig_type_agrees("usize", "uint64_t", "cpp"), "usize ≠ uint64_t (cpp)"
        assert _sig_type_agrees("u64", "u64", "ffi")
        assert not _sig_type_agrees("u64", "size_t", "ffi"), "u64 ≠ size_t (ffi)"
        assert not _sig_type_agrees("usize", "u64", "ffi"), "usize ≠ u64 (ffi)"

    def _case_sig_option_structural() -> None:
        """`Option<T>` agrees with each binding's idiomatic optional wrapping;
        a non-optional actual under an `Option<T>` spec fails."""
        assert _sig_type_agrees("Option<u64>", "std::optional<uint64_t>", "cpp")
        assert _sig_type_agrees("Option<String>", "string | null", "ts_dts")
        assert _sig_type_agrees("Option<u64>", "Option<u64>", "ffi")
        assert not _sig_type_agrees("Option<u64>", "u64", "cpp"), "must be optional"

    def _case_sig_extract_python() -> None:
        """Python extractor reads the pyo3 `fn` sig, stripping `&self` + `py`."""
        with tempfile.TemporaryDirectory() as tmp:
            src = pathlib.Path(tmp)
            (src / "m.rs").write_text(
                "#[pymethods]\nimpl Foo {\n"
                "    #[pyo3(signature = (n, name))]\n"
                "    pub fn bar(&self, py: Python<'_>, n: usize, name: String)"
                " -> PyResult<()> { Ok(()) }\n}\n",
                encoding="utf-8",
            )
            got = _sig_extract_python(src, "Foo", "bar")
            assert got == (["usize", "String"], "PyResult<()>"), got

    def _case_sig_extract_python_py_self_receiver() -> None:
        """A pyo3 by-value bound-self receiver (`slf: Py<Self>`) is stripped
        like `&self` — it carries the instance, not a cross-binding param. The
        `Client.streaming(slf: Py<Self>, py, callback)` shape surfaced this."""
        with tempfile.TemporaryDirectory() as tmp:
            src = pathlib.Path(tmp)
            (src / "m.rs").write_text(
                "#[pymethods]\nimpl Foo {\n"
                "    fn streaming(slf: Py<Self>, py: Python<'_>, callback: Py<PyAny>)"
                " -> PyResult<Py<Bar>> { todo!() }\n}\n",
                encoding="utf-8",
            )
            got = _sig_extract_python(src, "Foo", "streaming")
            assert got == (["Py<PyAny>"], "PyResult<Py<Bar>>"), got

    def _case_sig_result_two_arg_unwrap() -> None:
        """A Rust `Result<T, E>` return unwraps to the ok type `T` (the error
        arm is a per-binding surface property), depth-aware so a comma inside
        `T`'s generics is not split. Single-arg `Result<T>` / `Promise<T>` also
        unwrap. The flat-file `Result<Vec<FlatFileRow>, Error>` surfaced this."""
        assert _sig_unwrap_result("Result<Vec<crate::flatfiles::FlatFileRow>, Error>") == "Vec<crate::flatfiles::FlatFileRow>"
        assert _sig_unwrap_result("Result<std::path::PathBuf, Error>") == "std::path::PathBuf"
        assert _sig_unwrap_result("PyResult<()>") == "()"
        assert _sig_unwrap_result("Promise<boolean>") == "boolean"

    def _case_sig_extract_ts_napi() -> None:
        """TS-napi extractor reads the napi Rust `fn`, matching the camelCased
        name (and honouring an explicit `js_name`)."""
        with tempfile.TemporaryDirectory() as tmp:
            src = pathlib.Path(tmp)
            (src / "l.rs").write_text(
                "#[napi]\nimpl Foo {\n"
                '    #[napi(js_name = "barBaz")]\n'
                "    pub fn bar_baz(&self, n: f64) -> napi::Result<()> { Ok(()) }\n}\n",
                encoding="utf-8",
            )
            got = _sig_extract_ts_napi(src, "Foo", "barBaz")
            assert got == (["f64"], "napi::Result<()>"), got

    def _case_sig_extract_ts_dts() -> None:
        """`.d.ts` extractor reads the declared `method(p: T): R` shape."""
        with tempfile.TemporaryDirectory() as tmp:
            dts = pathlib.Path(tmp) / "index.d.ts"
            dts.write_text(
                "export class Foo {\n"
                "  barBaz(n: number): void\n"
                "}\n",
                encoding="utf-8",
            )
            got = _sig_extract_ts_dts(dts, "Foo", "barBaz")
            assert got == (["number"], "void"), got

    def _case_sig_extract_ts_dts_property_and_modifiers() -> None:
        """The `.d.ts` extractor reads PROPERTY declarations (`readonly name:
        T`) as zero-arg accessors, follows napi's getter / static modifiers,
        and unwraps a `Promise<T>` async return. Without these the columnar
        reader's `readonly dropped: number` / a `get`-accessor / a `static`
        factory all returned None and the gate passed on absence."""
        with tempfile.TemporaryDirectory() as tmp:
            dts = pathlib.Path(tmp) / "index.d.ts"
            dts.write_text(
                "export class Foo {\n"
                "  readonly dropped: number\n"
                "  get kind(): string\n"
                "  static fromFile(path: string): Foo\n"
                "  awaitDrain(timeoutMs: number): Promise<boolean>\n"
                "  setCpu(n: number | undefined | null): void\n"
                "}\n",
                encoding="utf-8",
            )
            assert _sig_extract_ts_dts(dts, "Foo", "dropped") == ([], "number")
            assert _sig_extract_ts_dts(dts, "Foo", "kind") == ([], "string")
            assert _sig_extract_ts_dts(dts, "Foo", "fromFile") == (["string"], "Foo")
            # `Promise<boolean>` unwraps to `boolean`, agreeing with a `bool` spec.
            assert _sig_type_agrees(
                "bool", _sig_unwrap_result(_sig_extract_ts_dts(dts, "Foo", "awaitDrain")[1]), "ts_dts"
            )
            # `number | undefined | null` is the idiomatic `Option<usize>` param.
            assert _sig_type_agrees(
                "Option<usize>", _sig_extract_ts_dts(dts, "Foo", "setCpu")[0][0], "ts_dts"
            )

    def _case_sig_ts_dts_follows_reexport() -> None:
        """The extractor reads the package entry AND the `index.d.ts` it
        re-exports with `export * from './index'`, so a member declared only on
        the napi layer is still found (the real surface is the union)."""
        with tempfile.TemporaryDirectory() as tmp:
            d = pathlib.Path(tmp)
            (d / "index.d.ts").write_text(
                "export class Config {\n  get workerThreads(): number | null\n}\n",
                encoding="utf-8",
            )
            (d / "entry.d.ts").write_text(
                "export * from './index'\n", encoding="utf-8"
            )
            got = _sig_extract_ts_dts(d / "entry.d.ts", "Config", "workerThreads")
            assert got == ([], "number | null"), got

    def _case_sig_ts_dts_conflicting_overload() -> None:
        """The package entry's `declare module` augmentation OVERRIDES the
        re-exported generated declaration for precedence (the resolved type),
        but the two MERGE as overloads — so a re-exported declaration that
        drifts to a non-client-facing return must be reported as a conflict even
        though the resolved type looks correct. Mirrors the `StreamView.batches`
        leak: the entry presents `Promise<RecordBatchStream>` while a generated
        `index.d.ts` overload returns the raw `Promise<RecordBatchStreamHandle>`.
        The optional `options?` param canonicalises to `BatchesOptions |
        undefined`, so the spec pins it as `Option<BatchesOptions>`."""
        spec = (["Option<BatchesOptions>"], "RecordBatchStream")
        with tempfile.TemporaryDirectory() as tmp:
            d = pathlib.Path(tmp)
            entry = d / "entry.d.ts"
            entry.write_text(
                "export * from './index'\n"
                "declare module './index' {\n"
                "  interface StreamView {\n"
                "    batches(options?: BatchesOptions): Promise<RecordBatchStream>;\n"
                "  }\n"
                "}\n",
                encoding="utf-8",
            )
            # Drift case: the re-exported generated declaration still returns the
            # raw handle. Precedence resolves to the wrapper, but the stale
            # overload must trip the gate.
            (d / "index.d.ts").write_text(
                "export declare class StreamView {\n"
                "  batches(options?: BatchesOptions | undefined | null): "
                "Promise<RecordBatchStreamHandle>\n"
                "}\n",
                encoding="utf-8",
            )
            # Precedence winner is the entry augmentation (the wrapper return);
            # the optional param survives as `BatchesOptions | undefined`.
            assert _sig_extract_ts_dts(entry, "StreamView", "batches") == (
                ["BatchesOptions | undefined"], "Promise<RecordBatchStream>"
            )
            conflicts = _sig_dts_conflicting_decls(entry, "StreamView", "batches", spec)
            assert len(conflicts) == 1, conflicts
            assert conflicts[0][0].name == "index.d.ts", conflicts
            assert conflicts[0][1] == (
                ["BatchesOptions | undefined | null"], "Promise<RecordBatchStreamHandle>"
            ), conflicts
            # FINDING 5: a SECOND overload in the SAME entry interface body (not a
            # cross-file sibling) — `finditer` must collect it so the conflict
            # scan sees the in-body drift, which `search` (first-match-only) hid.
            entry.write_text(
                "export * from './index'\n"
                "declare module './index' {\n"
                "  interface StreamView {\n"
                "    batches(options?: BatchesOptions): Promise<RecordBatchStream>;\n"
                "    batches(options?: BatchesOptions): Promise<RecordBatchStreamHandle>;\n"
                "  }\n"
                "}\n",
                encoding="utf-8",
            )
            (d / "index.d.ts").write_text(
                "export declare class StreamView {\n  isStreaming(): boolean\n}\n",
                encoding="utf-8",
            )
            assert len(_sig_dts_all_decls(entry, "StreamView", "batches")) == 2
            in_body = _sig_dts_conflicting_decls(entry, "StreamView", "batches", spec)
            assert len(in_body) == 1, in_body
            assert in_body[0][1][1] == "Promise<RecordBatchStreamHandle>", in_body
            # FIX case: the generated declaration is suppressed (skip_typescript)
            # and only the wrapper overload remains — no conflict.
            entry.write_text(
                "export * from './index'\n"
                "declare module './index' {\n"
                "  interface StreamView {\n"
                "    batches(options?: BatchesOptions): Promise<RecordBatchStream>;\n"
                "  }\n"
                "}\n",
                encoding="utf-8",
            )
            assert _sig_dts_conflicting_decls(entry, "StreamView", "batches", spec) == []

    def _case_sig_ts_dts_param_optionality() -> None:
        """FINDING 6: the `?` optional-param marker is preserved as `T |
        undefined`, so a required-vs-optional drift on a `.d.ts` param FAILS the
        type compare instead of collapsing both spellings to `T`. Pins the
        `StreamView.batches(options?: BatchesOptions)` row: `Option<...>` spec
        accepts the optional form and rejects the required form (and vice
        versa)."""
        with tempfile.TemporaryDirectory() as tmp:
            d = pathlib.Path(tmp)
            dts = d / "index.d.ts"
            spec_opt = (["Option<BatchesOptions>"], "()")
            spec_req = (["BatchesOptions"], "()")
            # Optional declaration → `BatchesOptions | undefined`.
            dts.write_text(
                "export class StreamView {\n  batches(options?: BatchesOptions): void\n}\n",
                encoding="utf-8",
            )
            opt = _sig_extract_ts_dts(dts, "StreamView", "batches")
            assert opt == (["BatchesOptions | undefined"], "void"), opt
            # Required declaration → bare `BatchesOptions`.
            dts.write_text(
                "export class StreamView {\n  batches(options: BatchesOptions): void\n}\n",
                encoding="utf-8",
            )
            req = _sig_extract_ts_dts(dts, "StreamView", "batches")
            assert req == (["BatchesOptions"], "void"), req
            # The two are now DISTINGUISHED (the FINDING-6 bug collapsed them).
            assert opt != req
            # Legitimate matches pass; the representable drift fails.
            assert _sig_compare_one("X.batches", spec_opt, opt, "ts_dts") == []
            assert _sig_compare_one("X.batches", spec_req, req, "ts_dts") == []
            assert _sig_compare_one("X.batches", spec_opt, req, "ts_dts")  # required ≠ optional
            assert _sig_compare_one("X.batches", spec_req, opt, "ts_dts")  # optional ≠ required

    def _case_sig_ts_dts_surface_forms() -> None:
        """Every `.d.ts` declaration FORM present in the pinned public TS surface
        parses, and a representable drift in each fails. Bounds the hardening to
        the forms actually in use (enumerated from the `ts_dts`-pinned rows);
        forms that never appear (e.g. TS rest params, generic methods) are noted
        below as deliberately unhandled."""
        with tempfile.TemporaryDirectory() as tmp:
            d = pathlib.Path(tmp)
            dts = d / "index.d.ts"
            dts.write_text(
                "export class Foo {\n"
                # plain method, Promise return
                "  awaitDrain(timeoutMs: number): Promise<boolean>\n"
                # nullable PARAM (napi union spelling) — the Option<usize> form
                "  setCpu(n: number | undefined | null): void\n"
                # nullable RETURN — Option<...> in return position
                "  contract(): ContractRef | null\n"
                # readonly property (RecordBatchStream.schema/dropped shape)
                "  readonly dropped: number\n"
                # getter accessor (napi #[getter])
                "  get kind(): string\n"
                # static factory (Credentials.fromFile shape)
                "  static fromFile(path: string): Foo\n"
                # callback / fn-type param (startStreaming shape)
                "  startStreaming(cb: (e: StreamEvent) => void): Promise<void>\n"
                # Array<T> param (subscribeMany shape)
                "  subscribeMany(subs: Array<Subscription>): void\n"
                # import-type return (RecordBatchStream.schema shape)
                "  schema(): import('apache-arrow').Schema\n"
                "}\n",
                encoding="utf-8",
            )
            E = _sig_extract_ts_dts
            # Each form parses to the expected (params, ret).
            assert E(dts, "Foo", "awaitDrain") == (["number"], "Promise<boolean>")
            assert E(dts, "Foo", "setCpu") == (["number | undefined | null"], "void")
            assert E(dts, "Foo", "contract") == ([], "ContractRef | null")
            assert E(dts, "Foo", "dropped") == ([], "number")
            assert E(dts, "Foo", "kind") == ([], "string")
            assert E(dts, "Foo", "fromFile") == (["string"], "Foo")
            assert E(dts, "Foo", "startStreaming") == (["(e: StreamEvent) => void"], "Promise<void>")
            assert E(dts, "Foo", "subscribeMany") == (["Array<Subscription>"], "void")
            assert E(dts, "Foo", "schema") == ([], "import('apache-arrow').Schema")
            # A representable drift in each form FAILS its compare. (TS has no
            # integer-width distinction, so an int-vs-int param is NOT drift —
            # the probes pin types that genuinely disagree under the map.)
            assert _sig_compare_one("Foo.awaitDrain", (["number"], "String"),
                                    E(dts, "Foo", "awaitDrain"), "ts_dts")  # bool return ≠ String
            assert _sig_compare_one("Foo.setCpu", (["String"], "()"),
                                    E(dts, "Foo", "setCpu"), "ts_dts")  # wrong inner type
            assert _sig_compare_one("Foo.contract", ([], "SecType"),
                                    E(dts, "Foo", "contract"), "ts_dts")  # wrong return enum
            assert _sig_compare_one("Foo.dropped", ([], "String"),
                                    E(dts, "Foo", "dropped"), "ts_dts")  # number ≠ String cell
            assert _sig_compare_one("Foo.subscribeMany", (["Subscription"], "()"),
                                    E(dts, "Foo", "subscribeMany"), "ts_dts")  # Array<T> ≠ scalar
            # ponytail: TS rest params (`...args: T[]`) and generic methods
            # (`m<T>()`) do not appear in the pinned surface; the param regex
            # carries a `...` prefix so a rest param's type is still extracted,
            # but no row pins one, so there is no probe. Add when a pinned row
            # introduces one.

    def _case_sig_ts_dts_absence_promotion() -> None:
        """A MISSING `.d.ts` declaration is an error when the class IS part of
        the public surface (a dropped public member), but degrades to
        napi-as-authority when the class itself is absent (a legitimately
        napi-only row). This is the FINDING-2 fail-on-absence rule."""
        with tempfile.TemporaryDirectory() as tmp:
            d = pathlib.Path(tmp)
            (d / "index.d.ts").write_text(
                "export class StreamView {\n  isStreaming(): boolean\n}\n",
                encoding="utf-8",
            )
            # Class present, member present → not missing.
            assert not _sig_dts_public_member_missing(d / "index.d.ts", "StreamView", "isStreaming")
            # Class present, member dropped → missing (must fail the gate).
            assert _sig_dts_public_member_missing(d / "index.d.ts", "StreamView", "ringCapacity")
            # Class absent entirely → NOT promoted (napi-only degradation).
            assert not _sig_dts_public_member_missing(d / "index.d.ts", "RecordBatchStream", "dropped")

    def _case_sig_extract_python_pyi_forms() -> None:
        """The `.pyi` extractor reads every declaration FORM the pinned Python
        surface uses: a multi-line keyword-only method (`batches(self, *, ...)
        -> RecordBatchStream`), a `@property` / bare read-write annotation
        (zero-arg returning the annotated type), a `@staticmethod`, an
        `Optional[T]` / `T | None` return, and a `def` with no `->` (the unit
        `None` return). Each form must parse to the right `(params, ret)`."""
        with tempfile.TemporaryDirectory() as tmp:
            pyi = pathlib.Path(tmp) / "__init__.pyi"
            pyi.write_text(
                "class Foo:\n"
                "    def batches(\n"
                "        self,\n"
                "        *,\n"
                "        batch_size: Optional[int] = None,\n"
                "        backpressure: Optional[str] = None,\n"
                "    ) -> RecordBatchStream:\n"
                "        ...\n"
                "    @property\n"
                "    def contract(self) -> Optional[Contract]:\n"
                "        ...\n"
                "    kind: Literal[\"quote\", \"trade\"]\n"
                "    consumer_cpu: Optional[int]\n"
                "    @staticmethod\n"
                "    def from_file(path: str) -> Credentials:\n"
                "        ...\n"
                "    def stop(self) -> None:\n"
                "        ...\n"
                "    def reconnect(self):\n"
                "        ...\n",
                encoding="utf-8",
            )
            assert _sig_extract_python_pyi(pyi, "Foo", "batches") == (
                ["Optional[int]", "Optional[str]"], "RecordBatchStream"
            ), _sig_extract_python_pyi(pyi, "Foo", "batches")
            # `@property def` form → zero-arg returning the annotated type.
            assert _sig_extract_python_pyi(pyi, "Foo", "contract") == ([], "Optional[Contract]")
            # Bare read-write annotations (the Config-knob property shape).
            assert _sig_extract_python_pyi(pyi, "Foo", "kind") == ([], 'Literal["quote", "trade"]')
            assert _sig_extract_python_pyi(pyi, "Foo", "consumer_cpu") == ([], "Optional[int]")
            # `@staticmethod` → no receiver to strip; the lone `str` param survives.
            assert _sig_extract_python_pyi(pyi, "Foo", "from_file") == (["str"], "Credentials")
            # `-> None` and a `def` with no `->` both yield the unit `None`.
            assert _sig_extract_python_pyi(pyi, "Foo", "stop") == ([], "None")
            assert _sig_extract_python_pyi(pyi, "Foo", "reconnect") == ([], "None")
            # A member genuinely absent from the class → None.
            assert _sig_extract_python_pyi(pyi, "Foo", "missing") is None
            # REGRESSION: a deeper-indented `def` PARAMETER that shares a
            # member's name (`batch_size` inside `batches(...)`) must NOT be
            # misread as a class property — the paren-mask gates it out, so a
            # lookup of a name that exists ONLY as a param returns None.
            assert _sig_extract_python_pyi(pyi, "Foo", "batch_size") is None
            assert _sig_extract_python_pyi(pyi, "Foo", "backpressure") is None

    def _case_sig_python_pyi_type_map_and_literal() -> None:
        """The `python_pyi` type map agrees on the stub spellings and a
        representable drift fails. A `Literal["a", "b"]` canonical (a
        `python_pyi_returns` override on a value-constrained Config knob) pins
        the EXACT value set — order-insensitive — so an added / removed /
        renamed member, or a widen to bare `str`, fails. An `Optional[T]` /
        `T | None` satisfies an `Option<T>` spec while the bare required form
        does NOT (optionality drift is visible)."""
        # Integer widths all read `int`; a non-integer drift fails.
        assert _sig_type_agrees("usize", "int", "python_pyi")
        assert not _sig_type_agrees("usize", "str", "python_pyi")
        # A `Literal[...]` canonical compares by EXACT value set (the fold to
        # `str` is gone): identity and reordering pass; add / remove / rename a
        # member fails; widening to bare `str` fails. This is the FINDING-9 win.
        lit = 'Literal["batched", "immediate"]'
        assert _sig_type_agrees(lit, 'Literal["batched", "immediate"]', "python_pyi")
        assert _sig_type_agrees(lit, 'Literal["immediate", "batched"]', "python_pyi")  # order-insensitive
        assert not _sig_type_agrees(lit, 'Literal["batched", "immediate", "x"]', "python_pyi")  # added
        assert not _sig_type_agrees(lit, 'Literal["batched"]', "python_pyi")  # removed
        assert not _sig_type_agrees(lit, 'Literal["batched", "flush"]', "python_pyi")  # renamed
        assert not _sig_type_agrees(lit, "str", "python_pyi")  # widened to bare str
        # Single-quoted members canonicalise identically to double-quoted.
        assert _sig_type_agrees("Literal['PROD', 'STAGE']", 'Literal["PROD", "STAGE"]', "python_pyi")
        # A bare `String` spec no longer silently accepts a Literal actual (the
        # fold is removed) — a genuine `str` property still passes.
        assert not _sig_type_agrees("String", 'Literal["batched", "immediate"]', "python_pyi")
        assert _sig_type_agrees("String", "str", "python_pyi")
        assert not _sig_type_agrees("String", "int", "python_pyi")
        # Optionality: both `Optional[T]` and PEP 604 `T | None` satisfy the
        # `Option<T>` spec; the required form does not, and vice versa.
        assert _sig_type_agrees("Option<String>", "Optional[str]", "python_pyi")
        assert _sig_type_agrees("Option<u64>", "int | None", "python_pyi")
        assert not _sig_type_agrees("Option<String>", "str", "python_pyi")  # required ≠ optional
        assert not _sig_type_agrees("String", "Optional[str]", "python_pyi")  # optional ≠ required
        # The wrapper return the stub presents (the coverage stubtest lacks).
        assert _sig_type_agrees("RecordBatchStream", "RecordBatchStream", "python_pyi")
        assert not _sig_type_agrees("RecordBatchStream", "Any", "python_pyi")

    def _case_sig_python_pyi_literal_value_set_drift() -> None:
        """FINDING-9 probe end-to-end: a `python_pyi_returns` Literal override
        pins the EXACT value set through the orchestrator. The correct set
        passes; adding, removing, or changing one member of the `.pyi` property
        FAILS the `python_pyi` lane (while ts / cpp keep checking the canonical
        `String`). Drives the real `_sig_check_method_signatures`."""
        def _row():
            return [{"class": "Config", "name": "flushMode", "python": True,
                     "signature": {"returns": "String",
                                   "python_pyi_returns": 'Literal["batched", "immediate"]'}}]
        with tempfile.TemporaryDirectory() as tmp:
            root = pathlib.Path(tmp)
            py = root / "py"; py.mkdir()
            # pyo3 getter returns `&str` (the `python` lane stays clean).
            (py / "m.rs").write_text(
                "#[pymethods]\nimpl Config {\n"
                "    #[getter] fn flush_mode(&self) -> &str { \"batched\" }\n}\n",
                encoding="utf-8",
            )
            pyi = root / "__init__.pyi"
            paths = dict(py_src=py, pyi_path=pyi, ts_src=root / "none_ts",
                         ts_dts=root / "none.d.ts", cpp_hpp=root / "none.hpp",
                         client_rs=root / "none.rs", ffi_src=root / "none_ffi")
            # Correct value set → silent.
            pyi.write_text(
                'class Config:\n    flush_mode: Literal["batched", "immediate"]\n',
                encoding="utf-8",
            )
            assert _sig_check_method_signatures(_row(), **paths) == [], \
                _sig_check_method_signatures(_row(), **paths)
            # Reordered set still passes (order-insensitive).
            pyi.write_text(
                'class Config:\n    flush_mode: Literal["immediate", "batched"]\n',
                encoding="utf-8",
            )
            assert _sig_check_method_signatures(_row(), **paths) == [], \
                _sig_check_method_signatures(_row(), **paths)
            # ADD a member → return mismatch on the `python_pyi` lane.
            pyi.write_text(
                'class Config:\n    flush_mode: Literal["batched", "immediate", "bogus"]\n',
                encoding="utf-8",
            )
            errs = _sig_check_method_signatures(_row(), **paths)
            assert any("python_pyi" in e and "return mismatch" in e for e in errs), errs
            # REMOVE a member → fails.
            pyi.write_text(
                'class Config:\n    flush_mode: Literal["batched"]\n', encoding="utf-8",
            )
            errs = _sig_check_method_signatures(_row(), **paths)
            assert any("python_pyi" in e and "return mismatch" in e for e in errs), errs
            # CHANGE a member value → fails.
            pyi.write_text(
                'class Config:\n    flush_mode: Literal["batched", "flush"]\n',
                encoding="utf-8",
            )
            errs = _sig_check_method_signatures(_row(), **paths)
            assert any("python_pyi" in e and "return mismatch" in e for e in errs), errs
            # WIDEN to bare `str` (drop the constraint) → fails.
            pyi.write_text("class Config:\n    flush_mode: str\n", encoding="utf-8")
            errs = _sig_check_method_signatures(_row(), **paths)
            assert any("python_pyi" in e and "return mismatch" in e for e in errs), errs

    def _case_sig_python_pyi_lane_drifts_and_presence() -> None:
        """The orchestrator's `.pyi` lane: a return drift, a param drift, an
        optionality drift, and a dropped pinned declaration each FAIL; a clean
        stub passes; a member served only via a class `__getattr__` (or a class
        absent from the stub) does NOT false-fail. Proves the lane is wired and
        its presence policy mirrors the `.d.ts` degrade-to-authority rule."""
        def _row(**sig):
            base = {"params": ["Option<usize>"], "returns": "RecordBatchStream"}
            base.update(sig)
            return [{"class": "StreamView", "name": "batches",
                     "python": True, "signature": base}]
        with tempfile.TemporaryDirectory() as tmp:
            root = pathlib.Path(tmp)
            # Minimal pyo3 source so the `python` lane stays clean while we probe
            # the `.pyi` lane (StreamView → no PY_CLASS_ALIASES entry, member
            # `batches`).
            py = root / "py"; py.mkdir()
            (py / "m.rs").write_text(
                "#[pymethods]\nimpl StreamView {\n"
                "    pub fn batches(&self, n: Option<usize>) -> "
                "PyResult<crate::streaming_batches::RecordBatchStream> { todo!() }\n}\n",
                encoding="utf-8",
            )
            pyi = root / "__init__.pyi"
            paths = dict(py_src=py, pyi_path=pyi, ts_src=root / "none_ts",
                         ts_dts=root / "none.d.ts", cpp_hpp=root / "none.hpp",
                         client_rs=root / "none.rs", ffi_src=root / "none_ffi")
            # Clean stub → silent.
            pyi.write_text(
                "class StreamView:\n"
                "    def batches(self, n: Optional[int]) -> RecordBatchStream:\n"
                "        ...\n",
                encoding="utf-8",
            )
            assert _sig_check_method_signatures(_row(), **paths) == [], \
                _sig_check_method_signatures(_row(), **paths)
            # RETURN drift (the stubtest-blind axis) → fails.
            pyi.write_text(
                "class StreamView:\n"
                "    def batches(self, n: Optional[int]) -> Any:\n        ...\n",
                encoding="utf-8",
            )
            errs = _sig_check_method_signatures(_row(), **paths)
            assert any("python_pyi" in e and "return mismatch" in e for e in errs), errs
            # PARAM-TYPE drift → fails.
            pyi.write_text(
                "class StreamView:\n"
                "    def batches(self, n: Optional[str]) -> RecordBatchStream:\n        ...\n",
                encoding="utf-8",
            )
            errs = _sig_check_method_signatures(_row(), **paths)
            assert any("python_pyi" in e and "param #0 type mismatch" in e for e in errs), errs
            # OPTIONALITY drift (required where Optional pinned) → fails.
            pyi.write_text(
                "class StreamView:\n"
                "    def batches(self, n: int) -> RecordBatchStream:\n        ...\n",
                encoding="utf-8",
            )
            errs = _sig_check_method_signatures(_row(), **paths)
            assert any("python_pyi" in e and "param #0 type mismatch" in e for e in errs), errs
            # DROPPED pinned declaration on a fully-enumerated stub class → fails.
            pyi.write_text(
                "class StreamView:\n"
                "    def is_streaming(self) -> bool:\n        ...\n",
                encoding="utf-8",
            )
            errs = _sig_check_method_signatures(_row(), **paths)
            assert any("python_pyi" in e and "removed public stub" in e for e in errs), errs
            # Class carries a `__getattr__` escape → the absent member degrades
            # to the `python` lane + stubtest (no `.pyi` false-fail).
            pyi.write_text(
                "class StreamView:\n"
                "    def is_streaming(self) -> bool:\n        ...\n"
                "    def __getattr__(self, name: str) -> Any:\n        ...\n",
                encoding="utf-8",
            )
            assert _sig_check_method_signatures(_row(), **paths) == [], \
                _sig_check_method_signatures(_row(), **paths)
            # Class wholly absent from the stub (a generator-emitted class) →
            # degrades too.
            pyi.write_text("class Unrelated:\n    ...\n", encoding="utf-8")
            assert _sig_check_method_signatures(_row(), **paths) == [], \
                _sig_check_method_signatures(_row(), **paths)

    def _case_sig_python_pyi_setter_property_degrade() -> None:
        """A Config `#[setter]` row's `.pyi` surface is the assignable property,
        not a `def set_x`; such rows are in `PYI_SETTER_PROPERTY_ROWS` and the
        `.pyi` lane does NOT fail on the (correctly) absent `set_x`, while the
        matching GETTER row IS `.pyi`-checked. Uses the live stub so the real
        property annotation is exercised."""
        # The 9 enrolled setters are the only pinned-python rows absent from the
        # real stub — assert that membership matches reality (a NEW absent
        # pinned setter must be enrolled or it fails).
        data = tomllib.loads(PARITY_TOML.read_text(encoding="utf-8"))
        setter_errs = _sig_check_method_signatures(
            [r for r in data.get("method", [])
             if (r.get("class"), r.get("name")) in PYI_SETTER_PROPERTY_ROWS],
            py_src=PY_SRC, pyi_path=PY_PYI, ts_src=TS_SRC, ts_dts=TS_DTS,
            cpp_hpp=CPP_HPP, client_rs=CORE_CLIENT_RS, ffi_src=FFI_SRC,
        )
        # The setter rows still get their pyo3-source `python` / ts / cpp / ffi
        # checks; only the `.pyi` lane is exempt. So no `python_pyi` error.
        assert not any("python_pyi" in e for e in setter_errs), setter_errs
        # The matching getter (`flushMode` → property `flush_mode`) IS checked.
        assert _sig_extract_python_pyi(PY_PYI, "Config", "flush_mode") == (
            [], 'Literal["batched", "immediate"]'
        ), _sig_extract_python_pyi(PY_PYI, "Config", "flush_mode")
        # FINDING-8 closure: the exemption is sound ONLY if EVERY exempt setter
        # has a getter twin whose `.pyi` property type the `python_pyi` lane
        # checks. Resolve each setter's twin from the real spec and assert (a) a
        # checked getter `[[method]]` row exists for it and (b) the stub declares
        # the property — so no exempt setter rides on an unchecked property.
        # `setStreamingRingSize` → `streamingRingSize` was the gap this closes.
        setter_to_getter = {
            "setFlushMode": ("flushMode", "flush_mode"),
            "setConsumerCpu": ("consumerCpu", "consumer_cpu"),
            "setReconnectPolicy": ("reconnectPolicy", "reconnect_policy"),
            "setStreamingRingSize": ("streamingRingSize", "streaming_ring_size"),
            "setWorkerThreads": ("workerThreads", "worker_threads"),
        }
        assert {("Config", s) for s in setter_to_getter} == PYI_SETTER_PROPERTY_ROWS, (
            "the setter→getter twin table must cover exactly the exempt setters"
        )
        getter_rows = {
            (r.get("class"), r.get("name")): r for r in data.get("method", [])
        }
        for setter, (getter, prop) in setter_to_getter.items():
            row = getter_rows.get(("Config", getter))
            assert row is not None and row.get("python") and "signature" in row, (
                f"{setter}'s twin getter `{getter}` must be a python-checked "
                f"`[[method]]` row so its property type is pinned"
            )
            assert _sig_extract_python_pyi(PY_PYI, "Config", prop) is not None, (
                f"the stub must declare the `{prop}` property the `{getter}` "
                f"getter row pins"
            )

    def _case_sig_extract_cpp() -> None:
        """C++ extractor reads the in-class decl, return type bounded by the
        prior access specifier (no `public:` leak into the return)."""
        with tempfile.TemporaryDirectory() as tmp:
            hpp = pathlib.Path(tmp) / "h.hpp"
            hpp.write_text(
                "class Foo {\npublic:\n"
                "    void bar(size_t n, const std::string& name) const;\n"
                "};\n",
                encoding="utf-8",
            )
            got = _sig_extract_cpp(hpp, "Foo", "bar")
            assert got == (["size_t", "const std::string&"], "void"), got

    def _case_sig_extract_rust() -> None:
        """Rust-core extractor reads `pub fn` / `pub async fn` in the impl."""
        with tempfile.TemporaryDirectory() as tmp:
            rs = pathlib.Path(tmp) / "client.rs"
            rs.write_text(
                "impl StreamSurface {\n"
                "    pub fn dropped(&self) -> u64 { 0 }\n"
                "    pub async fn close(&self, n: usize) -> Result<()> { Ok(()) }\n}\n",
                encoding="utf-8",
            )
            assert _sig_extract_rust(rs, "StreamSurface", "dropped") == ([], "u64")
            assert _sig_extract_rust(rs, "StreamSurface", "close") == (["usize"], "Result<()>")

    def _case_sig_extract_ffi() -> None:
        """FFI extractor reads the `extern "C" fn thetadatadx_<sym>` param list."""
        with tempfile.TemporaryDirectory() as tmp:
            src = pathlib.Path(tmp)
            (src / "f.rs").write_text(
                'pub unsafe extern "C" fn thetadatadx_record_batch_stream_close'
                "(n: usize) -> i32 { 0 }\n",
                encoding="utf-8",
            )
            got = _sig_extract_ffi(src, "record_batch_stream_close")
            assert got == (["usize"], "i32"), got

    def _case_sig_extract_python_getter_prefix() -> None:
        """A pyo3 `#[getter] fn get_<name>` resolves against the bare `<name>`
        the row carries — pyo3 strips the `get_` prefix from the property name,
        so the extractor falls back to `get_<name>` (the `Config.reconnect_policy`
        / `worker_threads` readback shape that surfaced this)."""
        with tempfile.TemporaryDirectory() as tmp:
            src = pathlib.Path(tmp)
            (src / "m.rs").write_text(
                "#[pymethods]\nimpl Config {\n"
                "    #[getter]\n"
                "    fn get_worker_threads(&self) -> Option<usize> { None }\n}\n",
                encoding="utf-8",
            )
            assert _sig_extract_python(src, "Config", "worker_threads") == (
                [], "Option<usize>"
            ), _sig_extract_python(src, "Config", "worker_threads")

    def _case_sig_extract_cpp_getter_prefix() -> None:
        """A C++ `get_<name>(...)` readback resolves against the bare `<name>`,
        the same `get_`-prefix convention the forward presence check accepts."""
        with tempfile.TemporaryDirectory() as tmp:
            hpp = pathlib.Path(tmp) / "h.hpp"
            hpp.write_text(
                "class Config {\npublic:\n"
                "    std::optional<size_t> get_worker_threads() const;\n};\n",
                encoding="utf-8",
            )
            assert _sig_extract_cpp(hpp, "Config", "worker_threads") == (
                [], "std::optional<size_t>"
            )

    def _case_sig_extract_cpp_elaborated_type_param() -> None:
        """The C++ class header is line-anchored: a `const class X& p` ELABORATED
        TYPE used as a parameter earlier in the file must NOT be mistaken for the
        class definition (the real `class FluentSubscription { ... }` that the
        bare `\\bclass X[^{]*{` regex skipped, resolving the wrong body)."""
        with tempfile.TemporaryDirectory() as tmp:
            hpp = pathlib.Path(tmp) / "h.hpp"
            hpp.write_text(
                # An earlier method takes the class by elaborated-type ref, with
                # its own body brace the broken regex would bridge to.
                "class Other {\npublic:\n"
                "    void use(const class Sub& s) const { (void)s; }\n};\n"
                "class Sub {\npublic:\n"
                "    std::string kind_string() const;\n};\n",
                encoding="utf-8",
            )
            assert _sig_extract_cpp(hpp, "Sub", "kind_string") == ([], "std::string")

    def _case_sig_extract_cpp_in_body_call_shadow() -> None:
        """A member-access CALL inside an earlier inline body (`handle_.get()`
        inside `size()`) must NOT shadow the real `get()` declaration: the
        method is matched in DECLARATION position only (the `FlatFileRowList::get`
        raw-handle accessor that returned a parsed method body as its type)."""
        with tempfile.TemporaryDirectory() as tmp:
            hpp = pathlib.Path(tmp) / "h.hpp"
            hpp.write_text(
                "class Rows {\npublic:\n"
                "    size_t size() const { return handle_ ? count(handle_.get()) : 0; }\n"
                "    const Handle* get() const noexcept { return handle_.get(); }\n};\n",
                encoding="utf-8",
            )
            assert _sig_extract_cpp(hpp, "Rows", "get") == ([], "const Handle*")

    def _case_sig_extract_cpp_preprocessor_guarded_return() -> None:
        """A method behind a `#ifdef` (the Arrow-gated `Stream::batches`) must
        extract its real return type — the preprocessor directive carries no
        `;`/`{`/`}`, so without stripping it the directive run prepends onto the
        return spelling (`#ifdef ... std::shared_ptr<...>`) and the type compare
        fails closed on a legitimate signature."""
        with tempfile.TemporaryDirectory() as tmp:
            hpp = pathlib.Path(tmp) / "h.hpp"
            hpp.write_text(
                "class Stream {\npublic:\n"
                "    bool is_streaming() const;\n"
                "#ifdef THETADATADX_CPP_ARROW\n"
                "    std::shared_ptr<RecordBatchStream> batches(size_t n) const;\n"
                "#endif\n};\n",
                encoding="utf-8",
            )
            assert _sig_extract_cpp(hpp, "Stream", "batches") == (
                ["size_t"],
                "std::shared_ptr<RecordBatchStream>",
            )

    def _case_sig_ffi_opaque_pointer_exact() -> None:
        """For the `ffi` lang an unmapped canonical (a raw handle pointer / a
        C-ABI owned struct) compares by EXACT spelling — the C ABI is the
        lowest layer, so the C type IS the canonical. A managed lang still fails
        closed on the same unmapped spelling."""
        assert _sig_type_agrees(
            "*const ThetaDataDxClient", "*const ThetaDataDxClient", "ffi"
        )
        assert _sig_type_agrees(
            "ThetaDataDxFlatFileBytes", "ThetaDataDxFlatFileBytes", "ffi"
        )
        assert not _sig_type_agrees(
            "*const ThetaDataDxClient", "*const ThetaDataDxConfig", "ffi"
        ), "a different opaque pointer must not match"
        assert not _sig_type_agrees(
            "*const ThetaDataDxClient", "*const ThetaDataDxClient", "python"
        ), "a managed lang must still fail closed on an unmapped raw type"

    def _case_sig_cpp_raw_handle_exact() -> None:
        """A C++ canonical that is itself a raw `ThetaDataDx*` handle pointer
        compares by exact spelling (the `FlatFileRowList::get` escape hatch),
        while a plain unmapped C++ scalar still fails closed."""
        assert _sig_type_agrees(
            "const ThetaDataDxFlatFileRowList*", "const ThetaDataDxFlatFileRowList*", "cpp"
        )
        assert not _sig_type_agrees("SomeRandomScalar", "SomeRandomScalar", "cpp"), (
            "a non-pointer unmapped C++ type must fail closed"
        )

    def _case_sig_return_result_unwrap_and_lifetime() -> None:
        """A return spelling is unwrapped of its fallible-result wrapper
        (`napi::Result<T>` / `PyResult<T>` → `T`) and Rust lifetimes / the napi
        prelude path are folded away before the type compare — the
        `reconnectPolicy` (`&'static str`), `workerThreads`
        (`napi::Result<Option<u32>>`), and `setStreamingRingSize`
        (`napi::bindgen_prelude::BigInt`) shapes that surfaced these."""
        # `&'static str` folds to `&str` and satisfies the String canonical.
        errs = _sig_compare_one(
            "X.reconnectPolicy", ([], "String"), ([], "&'static str"), "python"
        )
        assert errs == [], errs
        # `napi::Result<&'static str>` unwraps + folds to satisfy String.
        errs = _sig_compare_one(
            "X.reconnectPolicy", ([], "String"), ([], "napi::Result<&'static str>"), "ts_napi"
        )
        assert errs == [], errs
        # `napi::Result<Option<u32>>` unwraps to the structural Option compare.
        errs = _sig_compare_one(
            "X.workerThreads", ([], "Option<usize>"), ([], "napi::Result<Option<u32>>"), "ts_napi"
        )
        assert errs == [], errs
        # A fully-qualified `pyo3::PyResult<T>` (the `activeFullSubscriptions`
        # view spelling) unwraps past the module qualifier to its inner `T`.
        assert _sig_unwrap_result("pyo3::PyResult<Vec<Subscription>>") == "Vec<Subscription>"
        # The qualified napi BigInt param folds to the bare `BigInt` cell.
        errs = _sig_compare_one(
            "X.setRing", (["usize"], "()"), (["napi::bindgen_prelude::BigInt"], "napi::Result<()>"), "ts_napi"
        )
        assert errs == [], errs

    def _case_sig_opaque_payload_returns() -> None:
        """The opaque cross-binding payload return canonicals (`Bytes` / `Schema`
        / `PyObject` / `Credentials`) accept each binding's idiomatic spelling —
        the Arrow-IPC / schema / materialiser / auth-handle returns the
        FlatFileRowList / RecordBatchStream / Credentials surfaces carry."""
        assert _sig_type_agrees("Bytes", "napi::Result<Buffer>", "ts_napi") or \
            _sig_type_agrees("Bytes", _sig_unwrap_result("napi::Result<Buffer>"), "ts_napi")
        assert _sig_type_agrees("Bytes", "std::vector<uint8_t>", "cpp")
        assert _sig_type_agrees("Schema", "std::shared_ptr<arrow::Schema>", "cpp")
        assert _sig_type_agrees("Credentials", "Credentials", "cpp")
        # PyObject is Python-only — a row pinning it for C++ fails closed.
        assert not _sig_type_agrees("PyObject", "anything", "cpp")

    # An end-to-end synthetic source tree the orchestrator drives. The row
    # pins canonical (usize, String) -> (); napi overrides usize→f64. Each
    # negative variant mutates ONE binding source so exactly that drift axis
    # trips, proving the gate is not vacuously silent.
    def _write_sig_tree(root: pathlib.Path, *, py_params: str, ts_ret: str,
                        cpp_params: str, rust_ret: str, ffi_params: str) -> dict:
        py = root / "py"; py.mkdir()
        (py / "m.rs").write_text(
            "#[pymethods]\nimpl Widget {\n"
            f"    pub fn resize(&self, {py_params}) -> PyResult<()> {{ Ok(()) }}\n}}\n",
            encoding="utf-8",
        )
        ts = root / "ts"; ts.mkdir()
        (ts / "l.rs").write_text(
            "#[napi]\nimpl Widget {\n"
            f"    #[napi]\n    pub fn resize(&self, n: f64, name: String) -> {ts_ret} {{ Ok(()) }}\n}}\n",
            encoding="utf-8",
        )
        hpp = root / "w.hpp"
        hpp.write_text(
            "class Widget {\npublic:\n"
            f"    void resize({cpp_params});\n}};\n",
            encoding="utf-8",
        )
        client = root / "client.rs"
        client.write_text(
            "impl Widget {\n"
            f"    pub fn resize(&self, n: usize, name: String) -> {rust_ret} {{ todo!() }}\n}}\n",
            encoding="utf-8",
        )
        ffi = root / "ffi"; ffi.mkdir()
        (ffi / "f.rs").write_text(
            f'pub extern "C" fn thetadatadx_widget_resize({ffi_params}) -> i32 {{ 0 }}\n',
            encoding="utf-8",
        )
        # The `.pyi` lane is inert in these orchestrator cases (no stub written
        # → extractor returns None → degrades), keeping them focused on the
        # pyo3-source / napi / cpp / rust / ffi axes; the `.pyi` lane has its
        # own dedicated cases.
        return dict(py_src=py, pyi_path=root / "none.pyi", ts_src=ts,
                    ts_dts=root / "none.d.ts",
                    cpp_hpp=hpp, client_rs=client, ffi_src=ffi)

    def _sig_row(**sig_extra) -> list[dict]:
        signature = {"params": ["usize", "String"], "returns": "()",
                     "ts_napi_params": ["f64", "String"]}
        signature.update(sig_extra)
        return [{
            "class": "Widget", "name": "resize",
            "python": True, "typescript": True, "cpp": True, "rust": True,
            "ffi_symbol": "widget_resize",
            "signature": signature,
        }]

    def _case_sig_positive_all_axes_clean() -> None:
        """All bindings match the spec (incl. the napi f64 override + the
        canonical usize→cpp size_t / ffi usize sanctions) — gate is silent."""
        with tempfile.TemporaryDirectory() as tmp:
            root = pathlib.Path(tmp)
            paths = _write_sig_tree(
                root,
                py_params="n: usize, name: String",
                ts_ret="napi::Result<()>",
                cpp_params="size_t n, const std::string& name",
                rust_ret="()",
                ffi_params="n: usize, name: *const c_char",
            )
            # Rust column needs the class mapped; use an override entry.
            METHOD_BINDING_OVERRIDES[("Widget", "resize")] = {
                "rust": ("Widget", "resize")
            }
            try:
                errs = _sig_check_method_signatures(_sig_row(), **paths)
            finally:
                METHOD_BINDING_OVERRIDES.pop(("Widget", "resize"), None)
            assert errs == [], f"positive case must be silent; got {errs!r}"

    def _case_sig_type_drift_fails() -> None:
        """A param TYPE drift (Python `usize`→`bool`) trips."""
        with tempfile.TemporaryDirectory() as tmp:
            root = pathlib.Path(tmp)
            paths = _write_sig_tree(
                root,
                py_params="n: bool, name: String",  # drift
                ts_ret="napi::Result<()>",
                cpp_params="size_t n, const std::string& name",
                rust_ret="()",
                ffi_params="n: usize, name: *const c_char",
            )
            errs = _sig_check_method_signatures(_sig_row(), **paths)
            assert any("python" in e and "param #0 type mismatch" in e for e in errs), errs

    def _case_sig_arity_drift_fails() -> None:
        """An ARITY drift (an extra Python param) trips."""
        with tempfile.TemporaryDirectory() as tmp:
            root = pathlib.Path(tmp)
            paths = _write_sig_tree(
                root,
                py_params="n: usize, name: String, extra: bool",  # extra arg
                ts_ret="napi::Result<()>",
                cpp_params="size_t n, const std::string& name",
                rust_ret="()",
                ffi_params="n: usize, name: *const c_char",
            )
            errs = _sig_check_method_signatures(_sig_row(), **paths)
            assert any("python" in e and "arity mismatch" in e for e in errs), errs

    def _case_sig_order_drift_fails() -> None:
        """A param-ORDER drift (String, usize swapped on Python) trips — the
        per-position compare catches a reorder a set-based check would miss."""
        with tempfile.TemporaryDirectory() as tmp:
            root = pathlib.Path(tmp)
            paths = _write_sig_tree(
                root,
                py_params="name: String, n: usize",  # swapped
                ts_ret="napi::Result<()>",
                cpp_params="size_t n, const std::string& name",
                rust_ret="()",
                ffi_params="n: usize, name: *const c_char",
            )
            errs = _sig_check_method_signatures(_sig_row(), **paths)
            assert any("python" in e and "param #0 type mismatch" in e for e in errs), errs

    def _case_sig_return_drift_fails() -> None:
        """A RETURN drift (C++ `void`→`int32_t`) trips."""
        with tempfile.TemporaryDirectory() as tmp:
            root = pathlib.Path(tmp)
            paths = _write_sig_tree(
                root,
                py_params="n: usize, name: String",
                ts_ret="napi::Result<()>",
                cpp_params="size_t n, const std::string& name",
                rust_ret="()",
                ffi_params="n: usize, name: *const c_char",
            )
            # Rewrite the C++ return to a non-void type.
            hpp = paths["cpp_hpp"]
            hpp.write_text(
                "class Widget {\npublic:\n"
                "    int32_t resize(size_t n, const std::string& name);\n};\n",
                encoding="utf-8",
            )
            errs = _sig_check_method_signatures(_sig_row(), **paths)
            assert any("cpp" in e and "return mismatch" in e for e in errs), errs

    def _case_sig_override_honoured() -> None:
        """A per-binding override is honoured: the FFI `_explicit (has_value, n)`
        ABI split (an extra leading `bool`) PASSES under an `ffi_params`
        override, while the SAME 2-arity FFI source TRIPS against the bare
        canonical 1-param spec — proving the override is load-bearing."""
        with tempfile.TemporaryDirectory() as tmp:
            root = pathlib.Path(tmp)
            paths = _write_sig_tree(
                root,
                py_params="n: usize, name: String",
                ts_ret="napi::Result<()>",
                cpp_params="size_t n, const std::string& name",
                rust_ret="()",
                # The FFI side splits Option<usize> into (has_value, n).
                ffi_params="has_n: bool, n: usize, name: *const c_char",
            )
            # WITH the override naming the split shape: clean.
            ok = _sig_check_method_signatures(
                _sig_row(ffi_params=["bool", "usize", "String"]), **paths
            )
            assert ok == [], f"override path must pass; got {ok!r}"
            # WITHOUT the override the FFI spec is the canonical (usize, String)
            # 2-arity, which TRIPS against the 3-arity split source.
            bad = _sig_check_method_signatures(_sig_row(), **paths)
            assert any("ffi" in e and "arity mismatch" in e for e in bad), bad

    def _case_sig_name_only_fails_closed() -> None:
        """FINDING-1 fail-closed enrollment: a `[[method]]` row WITHOUT a
        `[method.signature]` FAILS unless it is in `NAME_ONLY_METHOD_ALLOWLIST`.
        A synthetic unpinned + unlisted row trips; the same row listed passes.
        This is the guarantee that no NEW row can be silently name-only."""
        rows = [{"class": "Widget", "name": "resize",
                 "python": True, "typescript": True, "cpp": True}]
        with tempfile.TemporaryDirectory() as tmp:
            root = pathlib.Path(tmp)
            paths = _write_sig_tree(
                root, py_params="n: bool", ts_ret="x", cpp_params="x y",
                rust_ret="()", ffi_params="z: bool",
            )
            # Not pinned, not allowlisted → must fail closed.
            errs = _sig_check_method_signatures(rows, **paths)
            assert any("neither a `[method.signature]`" in e for e in errs), errs
            # Allowlisted → passes (no signature checked, enrollment satisfied).
            NAME_ONLY_METHOD_ALLOWLIST[("Widget", "resize")] = "selftest fixture"
            try:
                ok = _sig_check_method_signatures(rows, **paths)
            finally:
                del NAME_ONLY_METHOD_ALLOWLIST[("Widget", "resize")]
            assert ok == [], f"allowlisted name-only row must pass; got {ok!r}"

    def _case_sig_skip_langs_opts_lang_out() -> None:
        """`skip_langs` opts a present binding out of the signature check even
        when canonical params/returns are pinned — the `ts_napi` view of a
        JS-wrapper-only class (`RecordBatchStream`) that ships no napi Rust
        `fn`. Without the skip the absent napi declaration would trip; with it
        the gate is silent, while the OTHER bindings stay checked."""
        with tempfile.TemporaryDirectory() as tmp:
            root = pathlib.Path(tmp)
            paths = _write_sig_tree(
                root, py_params="n: usize, name: String", ts_ret="napi::Result<()>",
                cpp_params="size_t n, const std::string& name", rust_ret="()",
                ffi_params="n: usize, name: *const c_char",
            )
            # Blow away the napi fn so its extractor returns None.
            (paths["ts_src"] / "l.rs").write_text(
                "#[napi]\nimpl Widget {\n}\n", encoding="utf-8"
            )
            # WITHOUT skip: the absent napi declaration trips.
            bad = _sig_check_method_signatures(_sig_row(), **paths)
            assert any("ts_napi" in e and "no `ts_napi`" in e for e in bad), bad
            # WITH skip: ts_napi is not checked; the rest stay clean.
            METHOD_BINDING_OVERRIDES[("Widget", "resize")] = {"rust": ("Widget", "resize")}
            try:
                ok = _sig_check_method_signatures(
                    _sig_row(skip_langs=["ts_napi"]), **paths
                )
            finally:
                METHOD_BINDING_OVERRIDES.pop(("Widget", "resize"), None)
            assert ok == [], f"skip_langs must silence ts_napi; got {ok!r}"

    def _case_ffi_symbol_signature_checked() -> None:
        """An `[[ffi_symbol]]` row carrying a `[ffi_symbol.signature]` has its
        extern's C signature extracted + compared (opaque pointers exact, scalars
        mapped); a drift in the pinned shape trips, and a row without a signature
        stays name-only."""
        with tempfile.TemporaryDirectory() as tmp:
            src = pathlib.Path(tmp)
            (src / "f.rs").write_text(
                'pub unsafe extern "C" fn thetadatadx_client_batches_open('
                "handle: *const ThetaDataDxClient, n: usize) "
                "-> *mut ThetaDataDxRecordBatchStream { core::ptr::null_mut() }\n",
                encoding="utf-8",
            )
            symbols = {"client_batches_open"}
            good = [{"name": "client_batches_open", "signature": {
                "params": ["*const ThetaDataDxClient", "usize"],
                "returns": "*mut ThetaDataDxRecordBatchStream"}}]
            assert _check_ffi_symbol_rows(good, symbols, src) == []
            # A return drift (wrong opaque pointer) trips by exact spelling.
            bad = [{"name": "client_batches_open", "signature": {
                "params": ["*const ThetaDataDxClient", "usize"],
                "returns": "*mut ThetaDataDxArrowBytes"}}]
            errs = _check_ffi_symbol_rows(bad, symbols, src)
            assert any("return mismatch" in e for e in errs), errs
            # No signature → name-only, silent.
            assert _check_ffi_symbol_rows(
                [{"name": "client_batches_open"}], symbols, src
            ) == []

    def _case_sig_live_surface_is_clean() -> None:
        """The LIVE parity.toml now carries real `[method.signature]` specs (the
        non-streaming enrolled surfaces). The signature gate extracts every
        enrolled binding's declared signature from the real sources and must
        find them all satisfying the specs — the engaged-and-clean invariant
        that replaces Phase 3's gated-to-zero landing."""
        data = tomllib.loads(PARITY_TOML.read_text(encoding="utf-8"))
        method_rows = data.get("method", [])
        pinned = [r for r in method_rows if r.get("signature")]
        assert pinned, (
            "Phase 4a engages the signature gate: at least one [[method]] row "
            "must carry a [method.signature] sub-table against the real sources."
        )
        errs = _sig_check_method_signatures(
            method_rows, py_src=PY_SRC, pyi_path=PY_PYI, ts_src=TS_SRC,
            ts_dts=TS_DTS, cpp_hpp=CPP_HPP, client_rs=CORE_CLIENT_RS,
            ffi_src=FFI_SRC,
        )
        assert errs == [], f"live signature gate must be clean; got {errs!r}"

    def _case_sig_live_ffi_symbols_clean() -> None:
        """The LIVE `[[ffi_symbol]]` rows carry real `[ffi_symbol.signature]`
        specs for the hand-written streaming-batch family; the gate extracts
        each extern's C signature and must find them satisfying the specs."""
        data = tomllib.loads(PARITY_TOML.read_text(encoding="utf-8"))
        ffi_rows = data.get("ffi_symbol", [])
        pinned = [r for r in ffi_rows if r.get("signature")]
        assert pinned, (
            "Phase 4a pins the hand-written FFI streaming symbols' signatures."
        )
        errs = _check_ffi_symbol_rows(
            ffi_rows, _collect_ffi_all_symbols(FFI_SRC), FFI_SRC
        )
        assert errs == [], f"live FFI signature gate must be clean; got {errs!r}"

    _case("sig type-map — forward map + usize→f64 sanction + fail-closed", _case_sig_type_map_forward_and_sanction)
    _case("sig type-map — Option<T> structural per binding", _case_sig_option_structural)
    _case("sig extractor — Python pyo3 fn sig", _case_sig_extract_python)
    _case("sig extractor — Python Py<Self> receiver stripped", _case_sig_extract_python_py_self_receiver)
    _case("sig type-map — Result<T,E> / Promise<T> return unwrap", _case_sig_result_two_arg_unwrap)
    _case("sig extractor — TS napi Rust fn sig", _case_sig_extract_ts_napi)
    _case("sig extractor — TS .d.ts decl", _case_sig_extract_ts_dts)
    _case("sig extractor — TS .d.ts property + getter/static/Promise", _case_sig_extract_ts_dts_property_and_modifiers)
    _case("sig extractor — TS .d.ts follows export-* re-export", _case_sig_ts_dts_follows_reexport)
    _case("sig gate — TS .d.ts conflicting merged overload fails", _case_sig_ts_dts_conflicting_overload)
    _case("sig gate — TS .d.ts param optionality (?) drift fails", _case_sig_ts_dts_param_optionality)
    _case("sig gate — TS .d.ts pinned surface forms all parse + drift fails", _case_sig_ts_dts_surface_forms)
    _case("sig gate — TS .d.ts absence promoted for public member", _case_sig_ts_dts_absence_promotion)
    _case("sig extractor — Python .pyi stub declaration forms", _case_sig_extract_python_pyi_forms)
    _case("sig type-map — python_pyi spellings + Literal exact value set", _case_sig_python_pyi_type_map_and_literal)
    _case("sig gate — python_pyi Literal value-set drift (add/remove/change) fails", _case_sig_python_pyi_literal_value_set_drift)
    _case("sig gate — Python .pyi lane drifts fail + presence degrades", _case_sig_python_pyi_lane_drifts_and_presence)
    _case("sig gate — Python .pyi setter-property rows degrade, getter checked", _case_sig_python_pyi_setter_property_degrade)
    _case("sig extractor — C++ in-class decl", _case_sig_extract_cpp)
    _case("sig extractor — Rust core impl fn", _case_sig_extract_rust)
    _case("sig extractor — FFI extern fn", _case_sig_extract_ffi)
    _case("sig extractor — Python get_ getter prefix", _case_sig_extract_python_getter_prefix)
    _case("sig extractor — C++ get_ getter prefix", _case_sig_extract_cpp_getter_prefix)
    _case("sig extractor — C++ elaborated-type param not class def", _case_sig_extract_cpp_elaborated_type_param)
    _case("sig extractor — C++ in-body call shadow rejected", _case_sig_extract_cpp_in_body_call_shadow)
    _case("sig extractor — C++ #ifdef-guarded return stripped", _case_sig_extract_cpp_preprocessor_guarded_return)
    _case("sig type-map — FFI opaque pointer exact match", _case_sig_ffi_opaque_pointer_exact)
    _case("sig type-map — C++ raw handle exact match", _case_sig_cpp_raw_handle_exact)
    _case("sig type-map — return result-unwrap + lifetime/napi-path fold", _case_sig_return_result_unwrap_and_lifetime)
    _case("sig type-map — opaque payload returns (Bytes/Schema/PyObject/Credentials)", _case_sig_opaque_payload_returns)
    _case("sig orchestrator — all axes clean (sanction + override)", _case_sig_positive_all_axes_clean)
    _case("sig orchestrator — TYPE drift fails", _case_sig_type_drift_fails)
    _case("sig orchestrator — ARITY drift fails", _case_sig_arity_drift_fails)
    _case("sig orchestrator — param-ORDER drift fails", _case_sig_order_drift_fails)
    _case("sig orchestrator — RETURN drift fails", _case_sig_return_drift_fails)
    _case("sig orchestrator — per-binding override honoured", _case_sig_override_honoured)
    _case("sig orchestrator — name-only row fails closed unless allowlisted", _case_sig_name_only_fails_closed)
    _case("sig orchestrator — skip_langs opts a present lang out", _case_sig_skip_langs_opts_lang_out)
    _case("sig orchestrator — [ffi_symbol.signature] checked + drift fails", _case_ffi_symbol_signature_checked)
    _case("sig orchestrator — live method surface engaged + clean", _case_sig_live_surface_is_clean)
    _case("sig orchestrator — live FFI symbol signatures clean", _case_sig_live_ffi_symbols_clean)

    print(f"check_binding_parity --selftest: {n_pass} passed, {n_fail} failed")
    return 0 if n_fail == 0 else 1


if __name__ == "__main__":
    sys.exit(main())
