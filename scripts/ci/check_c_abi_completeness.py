#!/usr/bin/env python3
"""C ABI completeness check (Gate 4 / issue #547).

Every exported `thetadatadx_*` C ABI symbol that ends up in the compiled
shared library `libthetadatadx_ffi.so` MUST appear as a function
DECLARATION in `thetadatadx-cpp/include/thetadatadx.h` (or one of its `.inc`
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
`thetadatadx-ffi/src/**/*.rs`. The regex pass missed macro-emitted symbols
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
FFI_SRC = REPO_ROOT / "thetadatadx-ffi" / "src"
CPP_INCLUDE = REPO_ROOT / "thetadatadx-cpp" / "include"
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
# A header symbol counts as DECLARED only when it appears in C function
# *declaration position*: the symbol name preceded by a return-type token
# (an identifier, a `*`/`&` pointer-or-reference marker, a closing `>`
# from a templated/typedef'd return, or an `extern` linkage keyword) and
# immediately followed (modulo whitespace) by `(`. A bare mention in a
# comment or a prose wildcard like `thetadatadx_exchange_*` is NOT a
# declaration, and — crucially — neither is a *call site* such as
# `const char* err = thetadatadx_last_error();` or the statement-leading
# `thetadatadx_string_array_free(arr);` inside a C++ inline body. Counting
# a call as a declaration let a genuinely-missing C decl masquerade as
# present: the C++ inline wrappers (`.hpp` / `.hpp.inc`) call every C
# symbol, so a `thetadatadx_foo(` call in a wrapper body would satisfy the
# forward `rust - header` check even after the real prototype was deleted
# from `thetadatadx.h`. The leading return-type requirement is what
# separates a prototype (`int32_t thetadatadx_foo(`) from a call
# (`= thetadatadx_foo(` / `return thetadatadx_foo(` / a bare statement).
#
# The token immediately before the symbol must be a return-type fragment.
# Tokens that can only precede a *call* — `=`, `.`, `->`, `return`, `,`,
# `(`, `&&`, `||`, `!`, `?`, `:` — are excluded via a negative lookbehind
# group, and a statement-leading call (nothing but `;`/`{`/`}`/start before
# the name) is excluded by requiring a preceding type token on the line.
_DECL_PREFIX = (
    # A return-type token: a C identifier (`void`, `int32_t`,
    # `ThetaDataDxClient`), optionally trailed by pointer/reference markers
    # and whitespace, or a closing `>` (a templated return), or the
    # `extern` linkage keyword that fronts the `.h.inc` prototypes.
    r"(?:"
    r"(?<![A-Za-z0-9_])extern\b[^\n;{}]*?"      # `extern [linkage] <type> `
    r"|[A-Za-z_][A-Za-z0-9_]*\s*[\*&]*\s+"       # `<ident> ` (return type)
    r"|[\*&>]\s*"                                  # pointer / templated return
    r")"
)
# Reject the obvious call-context lead-ins that the `<ident>\s+` arm could
# otherwise match (`return thetadatadx_x(`): a declaration's return type is
# never the `return` keyword.
_CALL_LEAD_KEYWORDS = re.compile(r"\b(return|sizeof|case)\s*$")
HEADER_DECL_RE = re.compile(
    r"(?P<lead>" + _DECL_PREFIX + r")(?P<name>thetadatadx_\w+)\s*\("
)
# Retained for the diagnostic-only fallback path and documentation: the
# bare `name(` shape with no decl-position guard. Not used by the
# declaration collector, which requires decl position via HEADER_DECL_RE.
HEADER_NAME_RE = re.compile(r"\b(thetadatadx_\w+)\s*\(")
# C / C++ comment AND string-literal strippers. Run before the
# declaration scan so a function name surviving only inside a comment OR a
# string literal cannot read as declared. A bare mention in a comment
# (`// removed thetadatadx_foo()`) and a name inside a string literal
# (`const char* m = "call thetadatadx_foo()"`) are BOTH non-declarations;
# counting either let a genuinely-missing prototype masquerade as present
# (the forward `rust - header` check) and could pollute the reverse
# `header - rust` delta. The decl-position regex already rejects most call
# shapes, but a string payload can embed an arbitrary `<word> thetadatadx_x(`
# fragment that mimics declaration position, so the contents must be
# removed outright.
#
# Comments and string/char literals are stripped in a SINGLE alternation
# pass: matching them together (rather than comments-then-strings) is what
# keeps a `//` inside a string (`"http://x"`) from being mis-read as a
# line comment, and a `"` inside a comment from opening a phantom string.
# Each construct is replaced with a single space so adjacent tokens do not
# fuse. Order matters inside the alternation: block comment, line comment,
# then char, then string — the first match at each position wins.
_STRIP_RE = re.compile(
    r"/\*.*?\*/"                 # block comment
    r"|//[^\n]*"                # line comment
    r"|'(?:\\.|[^'\\])*'"      # char literal (handles '\'' and '\\')
    r"|\"(?:\\.|[^\"\\])*\"",  # string literal (handles \" and \\)
    re.DOTALL,
)


def _strip_c_comments(text: str) -> str:
    """Remove comments AND string/char literals from C/C++ text.

    (Name kept for call-site stability; it now strips string and char
    literals too — see `_STRIP_RE`.) A name surviving only inside a
    comment or a string literal must not be collected as a declaration.
    """
    return _STRIP_RE.sub(" ", text)


def _header_decls_from_text(text: str) -> set[str]:
    """Function-declaration symbols in one header's text.

    Comments are stripped first, then `<return-type> thetadatadx_<name>(`
    declaration sites are collected. A *call site* in a C++ inline body
    (`= thetadatadx_foo(`, `return thetadatadx_foo(`, the statement-leading
    `thetadatadx_foo(arg);`) is NOT in declaration position and is
    rejected, so a wrapper body that calls a C symbol cannot mask a
    deleted prototype. Factored out so the self-test can exercise the
    comment-vs-declaration-vs-call logic on synthetic header text.
    """
    stripped = _strip_c_comments(text)
    decls: set[str] = set()
    for m in HEADER_DECL_RE.finditer(stripped):
        lead = m.group("lead")
        # The `<ident>\s+` arm of the prefix would also match
        # `return thetadatadx_x(`; a real return type is never the
        # `return` / `sizeof` / `case` keyword. Reject those lead-ins.
        if _CALL_LEAD_KEYWORDS.search(lead):
            continue
        decls.add(m.group("name"))
    return decls
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
    """Fallback: scan `thetadatadx-ffi/src/**/*.rs` for literal
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


def _is_c_decl_header(path: pathlib.Path) -> bool:
    """True for the C-ABI *declaration* headers, false for the C++
    inline-body headers.

    The C `extern "C"` prototypes — the SSOT for the link contract — live
    only in `thetadatadx.h` and the `*.h.inc` fragments it `#include`s.
    The C++ convenience wrappers (`thetadatadx.hpp` and the `*.hpp.inc`
    fragments) carry no prototypes; they *call* every C symbol from inline
    bodies. Scanning a wrapper for declarations is exactly the gap C4
    closes: a `thetadatadx_foo(` call there would otherwise count as a
    decl and mask a prototype deleted from `thetadatadx.h`. The
    decl-position regex already rejects those calls, but pruning the
    wrapper files here makes the scope explicit and keeps the gate fast.

    `.h.inc` ends in `.inc`, so a plain suffix check cannot tell it from
    `.hpp.inc`; match on the compound suffix instead.
    """
    name = path.name
    if name.endswith(".hpp") or name.endswith(".hpp.inc"):
        return False
    return name.endswith(".h") or name.endswith(".h.inc")


def collect_header_symbols() -> set[str]:
    """Symbols DECLARED as functions in the shipped C-ABI headers.

    Only the C *declaration* headers are scanned — `thetadatadx.h` and the
    `*.h.inc` prototype fragments. The C++ inline-body wrappers
    (`thetadatadx.hpp` / `*.hpp.inc`) are excluded: they declare nothing
    and merely call the C symbols, so a call there must not read as a
    declaration (the bypass C4 closes). Within the scanned files, comments
    are stripped first and only `<return-type> thetadatadx_<name>(`
    declaration sites are collected. A name that survives only in a
    comment, a `thetadatadx_foo_*` wildcard mentioned in prose, or a call
    site is not a declaration and is intentionally not collected — so it
    can neither satisfy the forward `rust - header` completeness check nor
    pollute the reverse `header - rust` delta.
    """
    out: set[str] = set()
    for header in sorted(CPP_INCLUDE.rglob("*")):
        if not header.is_file() or not _is_c_decl_header(header):
            continue
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
            "in thetadatadx-ffi/src but absent from C headers:"
        )
        for name in missing_in_header:
            print(f"  {name}")
        print(
            "\nFix: add the missing decl(s) to thetadatadx-cpp/include/thetadatadx.h "
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
            "headers but absent from thetadatadx-ffi/src:"
        )
        for name in sorted(extras):
            print(f"  {name}")
        print(
            "\nFix: either implement the symbol with "
            "`#[no_mangle] pub extern \"C\"` in `thetadatadx-ffi/src/`, or remove "
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
    """Prove a comment-only mention, a prose wildcard, and a C++ inline
    *call site* are all rejected, while real prototypes (including
    multi-line ones) are collected.

    Cases (all driven through the real `_header_decls_from_text` +
    `rust - header` / `header - rust` logic on synthetic inputs):

    * A header that DECLARES `thetadatadx_alpha(...)` but mentions
      `thetadatadx_beta` only inside a `//` comment and a `/* */` block,
      plus the prose wildcard `thetadatadx_gamma_*`. Only `alpha` may be
      collected as declared.
    * Forward check: a Rust symbol set `{alpha, beta}` against that header
      must report `beta` missing — the comment mention must NOT satisfy
      completeness (this is the original gameable hole).
    * Reverse check: a header declaring `thetadatadx_delta(...)` with no
      Rust counterpart must surface `delta` as an extra — proving real
      declarations are still collected after comment stripping.
    * Call-site rejection (the C4 bypass): a C++ inline body that *calls*
      `thetadatadx_epsilon` three ways — assignment (`= thetadatadx_epsilon(`),
      `return thetadatadx_epsilon(`, and the statement-leading
      `thetadatadx_epsilon(arg);` — must collect NOTHING. A wrapper body
      calling a C symbol must never read as that symbol's declaration, or
      a prototype deleted from `thetadatadx.h` would still pass while the
      `.hpp` wrapper that calls it survives.
    * String-literal rejection (the .h/.inc string bypass): a
      `thetadatadx_<name>(` fragment inside a C string / char literal must
      collect NOTHING, even when the surrounding string text mimics
      declaration position (`"use thetadatadx_x()"`). A `//`-bearing URL
      string must not be mis-stripped as a line comment, and a char literal
      must not swallow a following real declaration.
    * Multi-line prototype: a declaration whose argument list wraps across
      lines (`int32_t thetadatadx_zeta(\n  const T* a,\n  size_t n);`) must
      still be collected — the decl-position scan must not regress the real
      wrapped prototypes in `thetadatadx.h`.
    * File-scope exclusion: a `.hpp` and a `.hpp.inc` wrapper file that
      call C symbols, plus a `.h` and a `.h.inc` file that declare them,
      run through `_is_c_decl_header` — only the `.h` / `.h.inc` files are
      in scope, so a prototype deleted from `thetadatadx.h` is caught even
      when the wrapper still calls it.
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

    # Call-site rejection: a C++ inline body calling a C symbol must
    # collect nothing. This is the C4 bypass — a wrapper call masquerading
    # as a declaration.
    call_body = (
        "inline int wrap_epsilon(int a) {\n"
        "    const int r = thetadatadx_epsilon(a);\n"
        "    thetadatadx_epsilon(a);\n"
        "    return thetadatadx_epsilon(a);\n"
        "}\n"
    )
    call_decls = _header_decls_from_text(call_body)
    if call_decls:
        failures.append(
            "call-site rejection: a C++ inline body that only CALLS "
            f"thetadatadx_epsilon was read as declaring {sorted(call_decls)} "
            "(a wrapper call must not count as a declaration — this is the "
            "deleted-prototype bypass)"
        )

    # A genuinely-missing prototype whose only surviving mention is a
    # wrapper call must read as MISSING in the forward direction.
    rust_eps = {"thetadatadx_epsilon"}
    if "thetadatadx_epsilon" not in (rust_eps - call_decls):
        failures.append(
            "forward check: a symbol present only as a wrapper call site was "
            "treated as declared — the gate would pass a deleted prototype"
        )

    # String-literal rejection: a `thetadatadx_<name>(` fragment embedded in
    # a C string or char literal must NOT be collected. A string payload
    # can mimic declaration position (`"use thetadatadx_x() carefully"` puts
    # the word `use` before the symbol), so the contents must be stripped
    # outright before scanning. Covers a plain assignment, an initializer
    # list, a `//`-containing URL string (which must not be mis-stripped as
    # a line comment), and a char literal preceding a real decl (which MUST
    # survive). This is the .h/.inc string-literal bypass.
    string_text = (
        'const char* msg = "call thetadatadx_strghost(x) now";\n'
        'static const char* names[] = { "use thetadatadx_arrghost() here" };\n'
        'const char* url = "thetadatadx_urlghost(http://x)";\n'
        "char sep = ';';\n"
        "void thetadatadx_strreal(int x);\n"
    )
    string_decls = _header_decls_from_text(string_text)
    ghosts = {d for d in string_decls if d.endswith("ghost")}
    if ghosts:
        failures.append(
            "string-literal rejection: symbols embedded only in C string "
            f"literals were collected as declarations: {sorted(ghosts)} "
            "(a string payload mimicking declaration position must be "
            "stripped — this is the .h/.inc string-literal bypass)"
        )
    if "thetadatadx_strreal" not in string_decls:
        failures.append(
            "string-literal rejection: a real declaration following string / "
            "char literals was dropped (the strip removed too much)"
        )

    # Multi-line prototype: the wrapped-arg-list declarations in
    # thetadatadx.h must still be collected.
    multiline_text = (
        "int32_t thetadatadx_zeta(\n"
        "    const ThetaDataDxClient* a,\n"
        "    size_t n);\n"
    )
    if "thetadatadx_zeta" not in _header_decls_from_text(multiline_text):
        failures.append(
            "multi-line decl: a prototype whose argument list wraps across "
            "lines was not collected (real wrapped prototypes in "
            "thetadatadx.h would read as missing)"
        )

    # File-scope: only `.h` / `.h.inc` declaration headers are in scope;
    # the `.hpp` / `.hpp.inc` C++ wrappers are excluded.
    scope_cases = {
        "thetadatadx.h": True,
        "endpoint_with_options.h.inc": True,
        "thetadatadx.hpp": False,
        "tick_arrow_ipc.hpp.inc": False,
    }
    for fname, expected in scope_cases.items():
        got = _is_c_decl_header(pathlib.Path("thetadatadx-cpp/include") / fname)
        if got != expected:
            failures.append(
                f"file-scope: _is_c_decl_header({fname!r}) returned {got}, "
                f"expected {expected} (a C++ wrapper in scope would let a "
                "wrapper call mask a deleted prototype; a declaration header "
                "out of scope would blind the gate)"
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
