#!/usr/bin/env python3
"""C ABI completeness check (Gate 4 / issue #547).

Every exported `thetadatadx_*` C ABI symbol that ends up in the compiled
shared library `libthetadatadx_ffi.so` MUST appear by name in
`sdks/cpp/include/thetadatadx.h` (or one of its `.inc` includes).
Drift on the C-side header is invisible to `cargo build` because the
headers are hand-maintained and the link contract only breaks at the
user's compile time, after they've already pip-installed or fetched
the C++ SDK.

The symbol inventory is sourced from the compiled library via
`nm -D --defined-only` rather than a regex pass over
`ffi/src/**/*.rs`. The regex pass missed macro-emitted symbols
(e.g. `thetadatadx_*_tick_array_free` emitted by the `tick_array_free!`
macro), so a macro-generated free fn that the C++ headers did not
ship would link-error on user builds. The nm-based inventory is the
ground truth: a symbol present in the .so but absent from the
headers is a real ABI gap.

Test-helper symbols prefixed `thetadatadx_test_*` are skipped — they are
build-time-only helpers exposed for FFI integration tests and not
part of the published C ABI.

Exits non-zero on any gap. Run from the repo root.

Usage:
    cargo build -p thetadatadx-ffi --release
    python3 scripts/ci/check_c_abi_completeness.py
"""

from __future__ import annotations

import pathlib
import re
import subprocess
import sys


REPO_ROOT = pathlib.Path(__file__).resolve().parents[2]
FFI_SRC = REPO_ROOT / "ffi" / "src"
CPP_INCLUDE = REPO_ROOT / "sdks" / "cpp" / "include"
# Linux / macOS share the .so / .dylib output naming; the loader
# helper below tries each in turn so the gate runs unchanged on
# macOS hosts during local development.
_SO_NAMES = ("libthetadatadx_ffi.so", "libthetadatadx_ffi.dylib")


# Fallback regex (used only when nm cannot run — e.g. on a Windows
# host before the C ABI gate is added to the Windows CI matrix). The
# regex misses macro-emitted symbols, which is exactly the gap C4
# closes. Kept for diagnostic-only paths; the production gate prefers
# `nm`.
EXTERN_RE = re.compile(r'extern\s+"C"\s+fn\s+(thetadatadx_\w+)')
SYMBOL_RE = re.compile(r"\bthetadatadx_\w+\b")
# `nm -D --defined-only` lines look like:
#   `0000000000123456 T thetadatadx_some_symbol_name`
# We match the trailing identifier on `T` (text/code) entries — those
# are the externally-callable symbols. `B` / `D` (bss / data) entries
# would also be exported but we don't ship static globals through
# the C ABI; restricting to `T` keeps the gate scoped to actual fns.
NM_LINE_RE = re.compile(r"^\s*[0-9a-fA-F]+\s+T\s+(thetadatadx_\w+)\s*$")


def _so_path() -> pathlib.Path | None:
    """Locate the compiled FFI shared library under
    `target/release/`. Returns `None` if the library is not present —
    caller falls back to the regex pass and prints a warning.
    """
    for name in _SO_NAMES:
        candidate = REPO_ROOT / "target" / "release" / name
        if candidate.is_file():
            return candidate
    return None


def collect_ffi_symbols_via_nm() -> set[str] | None:
    """Return the set of exported `thetadatadx_*` symbols in the compiled
    shared library, or `None` if the library is missing or `nm`
    fails. Excludes `thetadatadx_test_*` helpers.
    """
    so = _so_path()
    if so is None:
        return None
    try:
        out = subprocess.check_output(
            ["nm", "-D", "--defined-only", str(so)],
            text=True,
        )
    except (FileNotFoundError, subprocess.CalledProcessError):
        return None
    syms: set[str] = set()
    for line in out.splitlines():
        match = NM_LINE_RE.match(line)
        if not match:
            continue
        name = match.group(1)
        if name.startswith("thetadatadx_test_"):
            continue
        syms.add(name)
    return syms


def collect_ffi_symbols_via_regex() -> set[str]:
    """Fallback: scan `ffi/src/**/*.rs` for literal
    `extern "C" fn thetadatadx_<name>` declarations. This MISSES macro-emitted
    symbols (`tick_array_free!`, etc.) and is intentionally only the
    diagnostic-fallback path — `collect_ffi_symbols_via_nm` is the
    SSOT when the .so is available.
    """
    out: set[str] = set()
    for rs in FFI_SRC.rglob("*.rs"):
        text = rs.read_text(encoding="utf-8")
        for match in EXTERN_RE.finditer(text):
            name = match.group(1)
            if name.startswith("thetadatadx_test_"):
                continue
            out.add(name)
    return out


def collect_ffi_symbols() -> set[str]:
    """Production entry point: prefer nm-based inventory; fall back
    to the regex pass with a loud warning if nm cannot run. The
    warning surfaces in CI logs so the operator sees that the gate
    is running in degraded mode.
    """
    via_nm = collect_ffi_symbols_via_nm()
    if via_nm is not None:
        print(
            f"check_c_abi_completeness: sourced symbol inventory from "
            f"compiled .so ({len(via_nm)} exported symbols)"
        )
        return via_nm
    print(
        "check_c_abi_completeness: WARNING — falling back to regex pass "
        "(libthetadatadx_ffi.{so,dylib} not found under `target/release/` "
        "or `nm` unavailable). Macro-emitted symbols may be missed. "
        "Build with `cargo build -p thetadatadx-ffi --release` before "
        "running this gate for full coverage.",
        file=sys.stderr,
    )
    return collect_ffi_symbols_via_regex()


def collect_header_symbols() -> set[str]:
    out: set[str] = set()
    for header in list(CPP_INCLUDE.rglob("*.h")) + list(CPP_INCLUDE.rglob("*.hpp")) + list(CPP_INCLUDE.rglob("*.inc")):
        text = header.read_text(encoding="utf-8")
        for match in SYMBOL_RE.finditer(text):
            out.add(match.group(0))
    return out


# Header-only `thetadatadx_*` symbols that legitimately have no Rust
# counterpart — e.g. opaque struct typedefs declared in C but
# implemented entirely in C++ wrapper code, or compile-time
# constants emitted via `#define THETADATADX_FOO 1`. Add a comment when
# extending this list.
HEADER_ONLY_ALLOWLIST: set[str] = {
    # `thetadatadx_exchange_` (trailing underscore) is the symbol-family
    # prose mention `thetadatadx_exchange_*` in `thetadatadx.h:785`. The
    # regex captures `thetadatadx_exchange_` from the wildcard form. Real
    # exports are `thetadatadx_exchange_name` / `thetadatadx_exchange_symbol`.
    "thetadatadx_exchange_",
}


def main() -> int:
    rust = collect_ffi_symbols()
    header = collect_header_symbols()
    missing_in_header = sorted(rust - header)
    if missing_in_header:
        print(
            f"check_c_abi_completeness: {len(missing_in_header)} symbol(s) defined "
            "in ffi/src but absent from C headers:"
        )
        for name in missing_in_header:
            print(f"  {name}")
        print(
            "\nFix: add the missing decl(s) to sdks/cpp/include/thetadatadx.h "
            "(or the appropriate `*.inc` include). The C++ wrapper "
            "compiles against these headers — a missing decl breaks "
            "user builds at the link step, not at `cargo build`."
        )
        return 1

    # Reverse delta: names declared in C headers but with no matching
    # `#[no_mangle] pub extern "C"` Rust counterpart.
    # These are link-time time bombs — the C++ wrapper compiles
    # against the header but the dylib symbol is missing.
    extras = (header - rust) - HEADER_ONLY_ALLOWLIST
    if extras:
        print(
            f"check_c_abi_completeness: {len(extras)} symbol(s) declared in C "
            "headers but absent from ffi/src:"
        )
        for name in sorted(extras):
            print(f"  {name}")
        print(
            "\nFix: either implement the symbol with "
            "`#[no_mangle] pub extern \"C\"` in `ffi/src/`, or remove "
            "the header decl. A header-only decl breaks consumer "
            "builds at LINK time (not compile), which is the "
            "regression mode that lands in CI nightly. If the symbol "
            "is intentionally header-only (typedef tag, macro "
            "constant), add it to `HEADER_ONLY_ALLOWLIST` above with "
            "a rationale comment."
        )
        return 1

    print(
        f"check_c_abi_completeness: clean ({len(rust)} symbols, "
        f"bidirectional rust<->header parity)"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
