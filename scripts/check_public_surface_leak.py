#!/usr/bin/env python3
"""Gate the shipped public SDK surface against internal-name leaks.

The C ABI completeness gate (`check_c_abi_completeness.py`) checks the
*exported symbols* of the compiled FFI library, but says nothing about
the hand-written comments and type stubs that ship alongside it. A
banner comment, a docstring, or a `.pyi` annotation can name an internal
Rust module (`tdbe`), a runtime crate (`tokio`, `crossbeam`,
`parking_lot`), or an internal dispatch primitive (`disruptor`,
`block_on`, `os_pipe`, `allow_threads`) without ever touching an
exported symbol — and that text ships verbatim to every user who opens
the header or reads the stub.

This gate closes that gap. It scans only the *shipped user-facing
surface* — the published C/C++ headers, the typed Python stubs, and the
distributed TypeScript type/dist files — for the forbidden internal-name
tokens and fails with `file:line` on any hit.

Scope is deliberately narrow. It excludes everything that legitimately
references internals:

* Rust `src/` — the implementation; it is *supposed* to name its crates.
* `tests/`, `benches/`, `__tests__/` — e.g. the no-GIL audit test that
  asserts `block_on` is absent from a hot path, or the helper test that
  mirrors `crates/.../tdbe/` source paths. Flagging those would punish
  the very gates that keep the surface clean.
* `node_modules/`, build/target directories.

Vendor names that are *intentionally* part of the public vocabulary
(`fpss`, `mdds`, `FIC`, `FIT`, `Theta Terminal`, `Interp3`) are
allow-listed: the forbidden-token match
is suppressed when the only reason a line matched was a vendor term.

Run::

    python3 scripts/check_public_surface_leak.py

Exit codes:

* ``0`` — clean.
* ``1`` — at least one shipped surface file leaks an internal name.

Selftest::

    python3 scripts/check_public_surface_leak.py --selftest

The selftest plants `tdbe`/`tokio` in a synthetic shipped stub and
confirms the gate flags it, then confirms a clean stub (and one that
contains only allow-listed vendor names) passes.
"""

from __future__ import annotations

import argparse
import pathlib
import re
import sys
from typing import Iterable

REPO_ROOT = pathlib.Path(__file__).resolve().parents[1]


# Globs describing the *shipped* user-facing surface, relative to the
# repo root. These are the files a user actually receives: the published
# C/C++ headers (every file under the public include dir ships via the
# CMake PUBLIC include path), the typed Python package, and the four
# distributed TypeScript files named in the npm package `files` field.
SCAN_GLOBS = (
    "sdks/cpp/include/*.h",
    "sdks/cpp/include/*.hpp",
    "sdks/cpp/include/*.inc",
    "sdks/python/python/thetadatadx/**/*.py",
    "sdks/python/python/thetadatadx/**/*.pyi",
    "sdks/typescript/index.d.ts",
    "sdks/typescript/index.js",
    "sdks/typescript/streaming-session.d.ts",
    "sdks/typescript/streaming-session.js",
)


# Path fragments that, if present, exclude a candidate even when it
# otherwise matches a scan glob. These cover the directories that
# legitimately reference internals (tests, benches, vendored deps,
# build output) and the Rust implementation. The globs above are
# already narrow, but the exclusions make the intent explicit and keep
# the gate correct if a glob is ever broadened.
EXEMPT_PATH_FRAGMENTS = (
    "/tests/",
    "/benches/",
    "/__tests__/",
    "/node_modules/",
    "/target/",
    "/.git/",
    "/src/",
)


# Internal-name tokens that must never appear in the shipped surface.
# Matched case-sensitively on a word boundary so `mddS` vendor casing
# and unrelated substrings do not trip. The list mirrors the standing
# public-prose ban: internal Rust module names, runtime crates, and
# dispatch primitives.
FORBIDDEN_TOKENS = (
    "tdbe",
    "tokio",
    "crossbeam",
    "parking_lot",
    "disruptor",
    "block_on",
    "os_pipe",
    "allow_threads",
)

FORBIDDEN_RE = re.compile(
    r"(?<![A-Za-z0-9_])(" + "|".join(re.escape(t) for t in FORBIDDEN_TOKENS) + r")(?![A-Za-z0-9_])"
)


# Vendor / methodology names that are intentionally public. A matched
# line is exonerated only if removing every allow-listed term would
# leave no forbidden token behind — i.e. the allow-list never masks a
# genuine leak that merely happens to share a line with a vendor name.
ALLOWLISTED_VENDOR_NAMES = (
    "fpss",
    "mdds",
    "FIC",
    "FIT",
    "Theta Terminal",
    "Interp3",
)


def _is_exempt(rel_path: pathlib.Path) -> bool:
    parts = "/" + rel_path.as_posix() + "/"
    return any(fragment in parts for fragment in EXEMPT_PATH_FRAGMENTS)


def _iter_files(root: pathlib.Path) -> Iterable[pathlib.Path]:
    seen: set[pathlib.Path] = set()
    for pattern in SCAN_GLOBS:
        for candidate in root.glob(pattern):
            if not candidate.is_file():
                continue
            rel = candidate.relative_to(root)
            if _is_exempt(rel):
                continue
            if rel in seen:
                continue
            seen.add(rel)
            yield candidate


def _strip_allowlisted(line: str) -> str:
    """Remove every allow-listed vendor name from `line`.

    Forbidden-token matching runs against the residue. A line whose only
    matches come from vendor names is thereby cleared, while a real leak
    sharing a line with a vendor name still trips.
    """
    residue = line
    for name in ALLOWLISTED_VENDOR_NAMES:
        residue = residue.replace(name, " ")
    return residue


def _scan_line(line: str) -> list[str]:
    """Return the forbidden tokens that survive allow-list stripping."""
    residue = _strip_allowlisted(line)
    return [m.group(1) for m in FORBIDDEN_RE.finditer(residue)]


def _scan(root: pathlib.Path) -> list[tuple[pathlib.Path, int, str, str]]:
    """Return (rel_path, lineno, token, line_text) for every leak found."""
    hits: list[tuple[pathlib.Path, int, str, str]] = []
    for path in _iter_files(root):
        rel = path.relative_to(root)
        try:
            text = path.read_text(encoding="utf-8")
        except (OSError, UnicodeDecodeError):
            continue
        for lineno, line in enumerate(text.splitlines(), start=1):
            for token in _scan_line(line):
                hits.append((rel, lineno, token, line.strip()))
    return hits


def _selftest() -> int:
    """Plant leaks in synthetic shipped stubs and confirm the gate fires.

    Three cases:

    * A stub naming `tdbe` and `tokio` — must be flagged (2 hits).
    * A clean stub — must pass.
    * A stub mentioning only allow-listed vendor names — must pass.
    """
    import tempfile

    leaky_header = (
        "/* #[repr(C)] tick types — layout-compatible with Rust tdbe structs */\n"
        "/* dispatched on a tokio runtime */\n"
        "void thetadatadx_thing(void);\n"
    )
    clean_stub = (
        "def connect(host: str) -> Client:\n"
        '    """Open a session against the Theta Terminal (fpss feed)."""\n'
        "    ...\n"
    )
    vendor_only = (
        "// The fpss and mdds feeds expose FIC / FIT data.\n"
        "// Interp3 curve interpolation is part of the public surface.\n"
    )

    with tempfile.TemporaryDirectory() as td:
        root = pathlib.Path(td)

        leaky = root / "sdks" / "cpp" / "include" / "leaky.h"
        leaky.parent.mkdir(parents=True, exist_ok=True)
        leaky.write_text(leaky_header, encoding="utf-8")

        clean = root / "sdks" / "python" / "python" / "thetadatadx" / "__init__.pyi"
        clean.parent.mkdir(parents=True, exist_ok=True)
        clean.write_text(clean_stub, encoding="utf-8")

        vendor = root / "sdks" / "typescript" / "index.d.ts"
        vendor.parent.mkdir(parents=True, exist_ok=True)
        vendor.write_text(vendor_only, encoding="utf-8")

        # A test file naming internals must be ignored even though it
        # lives under the SDK tree — proves the exclusion works.
        test_file = root / "sdks" / "python" / "tests" / "test_no_gil.py"
        test_file.parent.mkdir(parents=True, exist_ok=True)
        test_file.write_text("assert 'block_on' not in source\n", encoding="utf-8")

        hits = _scan(root)

        tokens = sorted(token for (_, _, token, _) in hits)
        if tokens != ["tdbe", "tokio"]:
            print(
                "selftest FAILED: expected exactly tdbe + tokio from the "
                f"leaky header, got {tokens!r}"
            )
            return 1
        if any("tests/" in rel.as_posix() for (rel, _, _, _) in hits):
            print("selftest FAILED: a test file was scanned")
            return 1
        if any(rel.name == "__init__.pyi" for (rel, _, _, _) in hits):
            print("selftest FAILED: a clean stub was flagged")
            return 1
        if any(rel.name == "index.d.ts" for (rel, _, _, _) in hits):
            print("selftest FAILED: a vendor-only stub was flagged")
            return 1

    print("selftest: ok")
    return 0


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument(
        "--selftest",
        action="store_true",
        help="Run the embedded self-test and exit.",
    )
    args = parser.parse_args(argv)

    if args.selftest:
        return _selftest()

    hits = _scan(REPO_ROOT)
    if not hits:
        print("public-surface-leak: clean")
        return 0
    print(
        f"public-surface-leak: {len(hits)} internal-name leak(s) in the "
        "shipped public surface"
    )
    for rel, lineno, token, line in hits:
        print(f"  {rel}:{lineno}: leaks `{token}`")
        print(f"    {line}")
    print(
        "  -> The shipped surface must not name internal Rust modules, "
        "runtime crates, or dispatch primitives. Rewrite the text to "
        "describe the public contract, not the implementation."
    )
    return 1


if __name__ == "__main__":
    sys.exit(main())
