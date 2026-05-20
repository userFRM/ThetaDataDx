#!/usr/bin/env python3
"""C ABI completeness check (Gate 4 / issue #547).

Every `extern "C" fn tdx_<name>` declared in `ffi/src/**/*.rs` must
appear by name in `sdks/cpp/include/thetadx.h` or one of its `.inc`
includes. Drift on the C-side header is invisible to `cargo build`
because the headers are hand-maintained and the link contract only
breaks at the user's compile time, after they've already pip-installed
or fetched the C++ SDK.

Test-helper symbols prefixed `tdx_test_*` are skipped — they are
build-time-only helpers exposed for FFI integration tests and not
part of the published C ABI.

Exits non-zero on any gap. Run from the repo root.
"""

from __future__ import annotations

import pathlib
import re
import sys


REPO_ROOT = pathlib.Path(__file__).resolve().parents[1]
FFI_SRC = REPO_ROOT / "ffi" / "src"
CPP_INCLUDE = REPO_ROOT / "sdks" / "cpp" / "include"


EXTERN_RE = re.compile(r'extern\s+"C"\s+fn\s+(tdx_\w+)')
SYMBOL_RE = re.compile(r"\btdx_\w+\b")


def collect_ffi_symbols() -> set[str]:
    out: set[str] = set()
    for rs in FFI_SRC.rglob("*.rs"):
        text = rs.read_text(encoding="utf-8")
        for match in EXTERN_RE.finditer(text):
            name = match.group(1)
            if name.startswith("tdx_test_"):
                continue
            out.add(name)
    return out


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
