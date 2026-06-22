#!/usr/bin/env python3
"""C ABI completeness check (Gate 4 / issue #547).

Every exported `thetadatadx_*` C ABI symbol that ends up in the compiled
shared library `libthetadatadx_ffi.so` MUST appear as a function
DECLARATION in `sdks/cpp/include/thetadatadx.h` (or one of its `.inc`
includes). Drift on the C-side header is invisible to `cargo build`
because the headers are hand-maintained and the link contract only
breaks at the user's compile time, after they've already pip-installed
or fetched the C++ SDK.

"Declaration" is the operative word: the header scan strips C/C++
comments first and then collects only `thetadatadx_<name>(` declaration
sites. A symbol name surviving in a comment, or a prose wildcard such as
`thetadatadx_exchange_*`, is NOT a declaration and does not count — so it
can neither make a genuinely-missing prototype look present (the forward
`rust - header` check) nor pollute the reverse `header - rust` delta. A
prior raw-text scan counted comment mentions, which is why a
`thetadatadx_exchange_` allow-list workaround was once needed; that entry
is gone now that only real declarations are collected.

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

Selftest:
    python3 scripts/ci/check_c_abi_completeness.py --selftest

The selftest plants a synthetic header that declares one symbol and
mentions another only in a comment, and confirms the comment mention is
not collected as declared (so the forward completeness check would still
flag it as missing).
"""

from __future__ import annotations

import argparse
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
# A header symbol counts as DECLARED only when it appears as a function
# declaration — the symbol name immediately followed (modulo whitespace)
# by an opening parenthesis. A bare mention in a comment or a prose
# wildcard like `thetadatadx_exchange_*` is NOT a declaration and must
# not satisfy the forward `rust - header` check; counting it let a
# genuinely-missing decl masquerade as present (the very gap the
# `thetadatadx_exchange_` allow-list entry was a workaround for).
HEADER_DECL_RE = re.compile(r"\b(thetadatadx_\w+)\s*\(")
# C / C++ comment strippers. Run before the declaration scan so a
# function name surviving only inside a comment cannot read as declared.
# (Both directions benefit: a commented-out prototype neither satisfies
# `rust - header` nor trips `header - rust`.) String-literal contents are
# not stripped, which is harmless here: the scanned names are bare C
# identifiers in declaration position, never string payloads.
_BLOCK_COMMENT_RE = re.compile(r"/\*.*?\*/", re.DOTALL)
_LINE_COMMENT_RE = re.compile(r"//[^\n]*")


def _strip_c_comments(text: str) -> str:
    """Remove block (`/* */`) and line (`//`) comments from C/C++ text."""
    return _LINE_COMMENT_RE.sub("", _BLOCK_COMMENT_RE.sub("", text))


def _header_decls_from_text(text: str) -> set[str]:
    """Function-declaration symbols in one header's text.

    Comments are stripped first, then `thetadatadx_<name>(` declaration
    sites are collected. Factored out so the self-test can exercise the
    comment-vs-declaration logic on synthetic header text.
    """
    stripped = _strip_c_comments(text)
    return {m.group(1) for m in HEADER_DECL_RE.finditer(stripped)}
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
    """Symbols DECLARED as functions in the shipped C/C++ headers.

    Comments are stripped first, then only `thetadatadx_<name>(`
    declaration sites are collected. A name that survives only in a
    comment, or a `thetadatadx_foo_*` wildcard mentioned in prose, is not
    a declaration and is intentionally not collected — so it can neither
    satisfy the forward `rust - header` completeness check nor pollute
    the reverse `header - rust` delta.
    """
    out: set[str] = set()
    for header in list(CPP_INCLUDE.rglob("*.h")) + list(CPP_INCLUDE.rglob("*.hpp")) + list(CPP_INCLUDE.rglob("*.inc")):
        out |= _header_decls_from_text(header.read_text(encoding="utf-8"))
    return out


# Header-only `thetadatadx_*` symbols that legitimately have no Rust
# counterpart — e.g. a function-like macro spelled `thetadatadx_foo(...)`
# that expands inline rather than linking to an exported symbol. Add a
# comment when extending this list.
#
# The former `thetadatadx_exchange_` entry has been removed: it existed
# only because the old raw-text scan captured the prose wildcard
# `thetadatadx_exchange_*` from a comment. The declaration-shape scan
# (`thetadatadx_<name>(`) over comment-stripped text no longer sees it,
# so no allow-listing is required for that family. Its real exports
# (`thetadatadx_exchange_name` / `thetadatadx_exchange_symbol`) are
# matched normally as genuine declarations.
HEADER_ONLY_ALLOWLIST: set[str] = set()


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


def _selftest() -> int:
    """Prove a comment-only symbol mention does not count as declared.

    Cases (all driven through the real `_header_decls_from_text` +
    `rust - header` / `header - rust` logic on synthetic inputs):

    * A header that DECLARES `thetadatadx_alpha(...)` but mentions
      `thetadatadx_beta` only inside a `//` comment and a `/* */` block,
      plus the prose wildcard `thetadatadx_gamma_*`. Only `alpha` may be
      collected as declared.
    * Forward check: a Rust symbol set `{alpha, beta}` against that header
      must report `beta` missing — the comment mention must NOT satisfy
      completeness (this is the gameable hole being closed).
    * Reverse check: a header declaring `thetadatadx_delta(...)` with no
      Rust counterpart must surface `delta` as an extra — proving real
      declarations are still collected after comment stripping.
    """
    header_text = (
        "// thetadatadx_beta is documented here but never declared.\n"
        "/* The family thetadatadx_gamma_* is described in prose only. */\n"
        "void thetadatadx_alpha(int x);\n"
        "/* thetadatadx_beta(int) — intentionally only in this comment */\n"
    )
    declared = _header_decls_from_text(header_text)

    failures: list[str] = []
    if declared != {"thetadatadx_alpha"}:
        failures.append(
            "comment-stripping: expected exactly {thetadatadx_alpha} to be "
            f"collected as declared, got {sorted(declared)} (a comment-only "
            "mention or a prose wildcard leaked in as a declaration)"
        )

    # Forward direction: the comment-only `beta` must read as MISSING.
    rust = {"thetadatadx_alpha", "thetadatadx_beta"}
    missing = rust - declared
    if "thetadatadx_beta" not in missing:
        failures.append(
            "forward check: a symbol present only in a header comment was "
            "treated as declared — the gate would pass a real missing decl"
        )
    if "thetadatadx_alpha" in missing:
        failures.append(
            "forward check: a genuinely declared symbol was reported missing"
        )

    # Reverse direction: a real header decl with no Rust impl is an extra.
    reverse_text = "double thetadatadx_delta(void);\n"
    reverse_declared = _header_decls_from_text(reverse_text)
    extras = (reverse_declared - {"thetadatadx_other"}) - HEADER_ONLY_ALLOWLIST
    if "thetadatadx_delta" not in extras:
        failures.append(
            "reverse check: a real header declaration absent from Rust was "
            "not surfaced as an extra"
        )

    if failures:
        print("check_c_abi_completeness --selftest: FAILED")
        for f in failures:
            print(f"  - {f}")
        return 1
    print("check_c_abi_completeness --selftest: ok")
    return 0


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument(
        "--selftest",
        action="store_true",
        help="Run the embedded self-test and exit.",
    )
    args = parser.parse_args()
    sys.exit(_selftest() if args.selftest else main())
