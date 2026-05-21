#!/usr/bin/env python3
"""C ABI completeness check (Gate 4 / issue #547).

Every exported `tdx_*` C ABI symbol that ends up in the compiled
shared library `libthetadatadx_ffi.so` MUST appear by name in
`sdks/cpp/include/thetadx.h` (or one of its `.inc` includes).
Drift on the C-side header is invisible to `cargo build` because the
headers are hand-maintained and the link contract only breaks at the
user's compile time, after they've already pip-installed or fetched
the C++ SDK.

C4 closure: the symbol inventory is sourced from the compiled
library via `nm -D --defined-only` rather than a regex pass over
`ffi/src/**/*.rs`. The regex pass missed macro-emitted symbols
(e.g. `tdx_*_tick_array_free` emitted by the `tick_array_free!`
macro), so a macro-generated free fn that the C++ headers did not
ship would link-error on user builds. The nm-based inventory is the
ground truth: a symbol present in the .so but absent from the
headers is a real ABI gap.

Test-helper symbols prefixed `tdx_test_*` are skipped — they are
build-time-only helpers exposed for FFI integration tests and not
part of the published C ABI.

Exits non-zero on any gap. Run from the repo root.

Usage:
    cargo build -p thetadatadx-ffi --release
    python3 scripts/check_c_abi_completeness.py
"""

from __future__ import annotations

import pathlib
import re
import subprocess
import sys


REPO_ROOT = pathlib.Path(__file__).resolve().parents[1]
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
EXTERN_RE = re.compile(r'extern\s+"C"\s+fn\s+(tdx_\w+)')
SYMBOL_RE = re.compile(r"\btdx_\w+\b")
# `nm -D --defined-only` lines look like:
#   `0000000000123456 T tdx_some_symbol_name`
# We match the trailing identifier on `T` (text/code) entries — those
# are the externally-callable symbols. `B` / `D` (bss / data) entries
# would also be exported but we don't ship static globals through
# the C ABI; restricting to `T` keeps the gate scoped to actual fns.
NM_LINE_RE = re.compile(r"^\s*[0-9a-fA-F]+\s+T\s+(tdx_\w+)\s*$")


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
    """Return the set of exported `tdx_*` symbols in the compiled
    shared library, or `None` if the library is missing or `nm`
    fails. Excludes `tdx_test_*` helpers.
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
        if name.startswith("tdx_test_"):
            continue
        syms.add(name)
    return syms


def collect_ffi_symbols_via_regex() -> set[str]:
    """Fallback: scan `ffi/src/**/*.rs` for literal
    `extern "C" fn tdx_<name>` declarations. This MISSES macro-emitted
    symbols (`tick_array_free!`, etc.) and is intentionally only the
    diagnostic-fallback path — `collect_ffi_symbols_via_nm` is the
    SSOT when the .so is available.
    """
    out: set[str] = set()
    for rs in FFI_SRC.rglob("*.rs"):
        text = rs.read_text(encoding="utf-8")
        for match in EXTERN_RE.finditer(text):
            name = match.group(1)
            if name.startswith("tdx_test_"):
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


def main() -> int:
    rust = collect_ffi_symbols()
    header = collect_header_symbols()
    missing = sorted(rust - header)
    if missing:
        print(f"check_c_abi_completeness: {len(missing)} symbol(s) defined in ffi/src but absent from C headers:")
        for name in missing:
            print(f"  {name}")
        print(
            "\nFix: add the missing decl(s) to sdks/cpp/include/thetadx.h "
            "(or the appropriate `*.inc` include). The C++ wrapper "
            "compiles against these headers — a missing decl breaks "
            "user builds at the link step, not at `cargo build`."
        )
        return 1
    print(f"check_c_abi_completeness: clean ({len(rust)} symbols, all present in headers)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
