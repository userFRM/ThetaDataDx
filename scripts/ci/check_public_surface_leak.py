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
  mirrors `thetadatadx-rs/.../tdbe/` source paths. Flagging those would punish
  the very gates that keep the surface clean.
* `node_modules/`, build/target directories.

Vendor names that are *intentionally* part of the public vocabulary
(`fpss`, `mdds`, `FIC`, `FIT`, `Theta Terminal`, `Interp3`) are
allow-listed: the forbidden-token match
is suppressed when the only reason a line matched was a vendor term.

Run::

    python3 scripts/ci/check_public_surface_leak.py

Exit codes:

* ``0`` — clean.
* ``1`` — at least one shipped surface file leaks an internal name.

Selftest::

    python3 scripts/ci/check_public_surface_leak.py --selftest

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

REPO_ROOT = pathlib.Path(__file__).resolve().parents[2]


# Globs describing the *shipped* user-facing surface, relative to the
# repo root. These are the files a user actually receives: the published
# C/C++ headers (every file under the public include dir ships via the
# CMake PUBLIC include path), the typed Python package, and the four
# distributed TypeScript files named in the npm package `files` field.
SCAN_GLOBS = (
    "thetadatadx-cpp/include/*.h",
    "thetadatadx-cpp/include/*.hpp",
    "thetadatadx-cpp/include/*.inc",
    "thetadatadx-py/python/thetadatadx/**/*.py",
    "thetadatadx-py/python/thetadatadx/**/*.pyi",
    "thetadatadx-ts/index.d.ts",
    "thetadatadx-ts/index.js",
    "thetadatadx-ts/streaming-session.d.ts",
    "thetadatadx-ts/streaming-session.js",
    # Per-SDK READMEs ship verbatim as the package long-description: the
    # Python README becomes the PyPI page (`pyproject.toml` `readme =
    # "README.md"`), the TypeScript README is packed into the npm tarball
    # (npm always includes `README.md`), and the C++ README ships with the
    # source SDK. They are as user-facing as the headers and stubs, so an
    # internal-name leak in any of them reaches every reader of the
    # package page.
    "thetadatadx-py/README.md",
    "thetadatadx-ts/README.md",
    "thetadatadx-cpp/README.md",
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
# Matched case-insensitively on a word boundary, so a `Tokio` /
# `Crossbeam` / `TDBE` capitalisation slips through no more than the
# lower-case spelling does. The list mirrors the standing public-prose
# ban: internal Rust module names, runtime crates, and dispatch
# primitives. Single-token entries whose internal separator can be
# spelled with a space, hyphen, or underscore (`parking_lot` /
# `parking lot`, `block_on` / `block-on`, `os_pipe`, `crossbeam` /
# `cross beam`) carry a flexible `[\s_-]?` separator so a reflowed or
# re-hyphenated rendering cannot dodge the gate.
#
# Vendor terms (`fpss`, `mdds`) are deliberately ABSENT from this list
# and never matched here — case-insensitivity is therefore safe for
# them: there is no forbidden pattern a vendor word can satisfy. The
# allow-list below is a second, belt-and-braces guard for the case where
# an impl-IP token genuinely shares a line with a vendor name.
FORBIDDEN_PATTERNS = (
    r"tdbe",
    r"tokio",
    r"cross[\s_-]?beam",
    r"parking[\s_-]?lot",
    r"disruptor",
    r"block[\s_-]?on",
    r"os[\s_-]?pipe",
    r"allow_threads",
    r"Python::detach",
    r"ingest[\s_-]?ring",
    r"dispatch[\s_-]?consumer",
    # Extended DENY set — the rest of the standing impl-IP vocabulary that
    # describes the dispatch engine, the runtime data structures, and the
    # async bridge. Each carries the same flexible `[\s_-]?` separator as
    # the entries above so a reflowed or re-hyphenated rendering cannot
    # dodge the gate.
    r"firehose",                  # the streaming fan-out engine name
    r"routing[\s_-]?table",       # the contract-to-consumer routing table
    r"arc[\s_-]?swap",            # `arc_swap` / `arc-swap` hot-config cell
    r"dashmap",                   # the concurrent map crate
    r"rustc[\s_-]?hash",          # `rustc_hash` / `FxHashMap` hasher crate
    r"epoll",                     # the Linux readiness primitive
    r"kqueue",                    # the BSD/macOS readiness primitive
    r"pyo3-async-runtimes",       # the Python async bridge crate
    r"future_into_py",            # the pyo3 async bridge entry point
    r"SharedProducer",            # internal ring-producer handle type
    r"StateCell",                 # internal hot-swappable state cell type
    # NOTE: `worker[\s_-]?pool` is deliberately NOT in this list. The
    # async worker-thread count is a documented PUBLIC config knob
    # (`set_worker_threads` / the `worker_threads` property on every
    # binding); `check_binding_parity._check_public_surface_vocab` already
    # adjudicates worker-thread vocabulary as neutral public surface (it
    # strips the internal `tokio_` prefix and exposes `worker_threads`),
    # with an explicit `test_surface_vocab_allows_neutral_worker_threads`
    # asserting it stays clean. The knob's own doc comment necessarily
    # describes its process-global threading model, so banning "worker
    # pool" here would fire on a legitimate public-API description, not an
    # impl-IP leak. The internal runtime crate name (`tokio`) is already
    # banned above — that is the token that would actually leak the
    # implementation.
)

# Each pattern is wrapped in identifier-boundary guards so a token glued
# to a surrounding identifier (`mytokio`, `tokiox`, `block_once`) does
# not trip. `Python::detach` is bounded on both ends by the `Python`
# prefix and the `detach` suffix; the literal `::` sits safely inside the
# alternative, so the same boundary guards apply unchanged.
FORBIDDEN_RE = re.compile(
    r"(?<![A-Za-z0-9_])("
    + "|".join(f"(?:{p})" for p in FORBIDDEN_PATTERNS)
    + r")(?![A-Za-z0-9_])",
    re.IGNORECASE,
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


_ALLOWLISTED_RE = re.compile(
    "|".join(re.escape(name) for name in ALLOWLISTED_VENDOR_NAMES),
    re.IGNORECASE,
)


def _strip_allowlisted(line: str) -> str:
    """Remove every allow-listed vendor name from `line`.

    Forbidden-token matching runs against the residue. A line whose only
    matches come from vendor names is thereby cleared, while a real leak
    sharing a line with a vendor name still trips. The strip is
    case-insensitive to match the case-insensitive forbidden scan: a
    vendor name in any casing (`FPSS`, `Mdds`) is cleared just as the
    lower-case spelling is.
    """
    return _ALLOWLISTED_RE.sub(" ", line)


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

    Cases:

    * A header naming `tdbe` and `tokio` — must be flagged (2 hits).
    * A clean stub — must pass.
    * A stub mentioning only allow-listed vendor names (in mixed casing)
      — must pass.
    * A header carrying mixed-case and separator-variant impl-IP tokens
      (`Tokio`, `Crossbeam`, `Python::detach`, `TDBE`, `parking-lot`,
      `block on`, `cross beam`, `os-pipe`, `ingest ring`,
      `dispatch consumer`) — every one must be flagged, proving the
      case-insensitive scan, the new phrases, and the flexible
      separators all bite.
    * A header carrying the extended DENY set (`Firehose`, `routing table`,
      `arc-swap`, `DashMap`, `rustc-hash`, `epoll`, `kqueue`,
      `pyo3-async-runtimes`, `future_into_py`, `SharedProducer`,
      `StateCell`) — every one must be flagged. A planted `firehose` in a
      shipped `thetadatadx-cpp/include/*.h` is the canonical bypass this closes.
      (`worker pool` is intentionally absent — see the FORBIDDEN_PATTERNS
      note; worker-thread vocabulary is adjudicated public surface.)
    * A shipped README (`thetadatadx-py/README.md`) naming an internal
      runtime crate — must be flagged, proving the per-package README is in
      the scan set (the PyPI / npm long-description leak that previously
      went unscanned).
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
    # Vendor names in deliberately mixed casing: the case-insensitive
    # allow-list strip must clear them so the line passes.
    vendor_only = (
        "// The FPSS and Mdds feeds expose FIC / FIT data.\n"
        "// Interp3 curve interpolation is part of the public surface.\n"
    )
    # Each line carries exactly one impl-IP spelling the OLD gate let
    # through: a capitalised token, a `::`-bearing token, or a token
    # whose internal separator was respelled as a space or hyphen.
    case_and_separator_variants = (
        "/* dispatched on a Tokio runtime */\n"
        "/* a Crossbeam channel feeds the consumer */\n"
        "/* releases the GIL via Python::detach */\n"
        "/* layout-compatible with the TDBE engine */\n"
        "/* guarded by a parking-lot mutex */\n"
        "/* the reader will block on the socket: block on it */\n"
        "/* a cross beam queue */\n"
        "/* drained through an os-pipe */\n"
        "/* events arrive on the ingest ring */\n"
        "/* serviced by the dispatch consumer */\n"
    )
    # The extended DENY set, one spelling per line — the dispatch engine,
    # the runtime data structures, and the async bridge, in mixed casing
    # and with separator variants. Every one must be flagged.
    extended_deny_variants = (
        "/* events fan out through the Firehose */\n"
        "/* keyed by the routing table */\n"
        "/* hot config lives in an arc-swap cell */\n"
        "/* contracts indexed in a DashMap */\n"
        "/* hashed with rustc-hash */\n"
        "/* readiness via epoll on Linux */\n"
        "/* readiness via kqueue on macOS */\n"
        "/* bridged by pyo3-async-runtimes */\n"
        "/* the future_into_py entry point */\n"
        "/* writes go through a SharedProducer */\n"
        "/* swapped atomically in a StateCell */\n"
    )
    leaky_readme = (
        "# thetadatadx (Python)\n"
        "\n"
        "Ticks are decoded on a tokio runtime and handed to Python.\n"
    )

    with tempfile.TemporaryDirectory() as td:
        root = pathlib.Path(td)

        leaky = root / "thetadatadx-cpp" / "include" / "leaky.h"
        leaky.parent.mkdir(parents=True, exist_ok=True)
        leaky.write_text(leaky_header, encoding="utf-8")

        clean = root / "thetadatadx-py" / "python" / "thetadatadx" / "__init__.pyi"
        clean.parent.mkdir(parents=True, exist_ok=True)
        clean.write_text(clean_stub, encoding="utf-8")

        vendor = root / "thetadatadx-ts" / "index.d.ts"
        vendor.parent.mkdir(parents=True, exist_ok=True)
        vendor.write_text(vendor_only, encoding="utf-8")

        variants = root / "thetadatadx-cpp" / "include" / "variants.h"
        variants.parent.mkdir(parents=True, exist_ok=True)
        variants.write_text(case_and_separator_variants, encoding="utf-8")

        extended = root / "thetadatadx-cpp" / "include" / "extended.h"
        extended.parent.mkdir(parents=True, exist_ok=True)
        extended.write_text(extended_deny_variants, encoding="utf-8")

        readme = root / "thetadatadx-py" / "README.md"
        readme.parent.mkdir(parents=True, exist_ok=True)
        readme.write_text(leaky_readme, encoding="utf-8")

        # A test file naming internals must be ignored even though it
        # lives under the SDK tree — proves the exclusion works.
        test_file = root / "thetadatadx-py" / "tests" / "test_no_gil.py"
        test_file.parent.mkdir(parents=True, exist_ok=True)
        test_file.write_text("assert 'block_on' not in source\n", encoding="utf-8")

        hits = _scan(root)

        header_tokens = sorted(
            token.lower() for (rel, _, token, _) in hits if rel.name == "leaky.h"
        )
        if header_tokens != ["tdbe", "tokio"]:
            print(
                "selftest FAILED: expected exactly tdbe + tokio from the "
                f"leaky header, got {header_tokens!r}"
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

        # Every mixed-case / separator-variant spelling must be caught.
        variant_tokens = {
            re.sub(r"[\s_-]", "", token.lower())
            for (rel, _, token, _) in hits
            if rel.name == "variants.h"
        }
        expected_variants = {
            "tokio",
            "crossbeam",
            "python::detach",
            "tdbe",
            "parkinglot",
            "blockon",
            "ospipe",
            "ingestring",
            "dispatchconsumer",
        }
        missing = expected_variants - variant_tokens
        if missing:
            print(
                "selftest FAILED: case/separator-variant impl-IP tokens "
                f"slipped through: {sorted(missing)!r}"
            )
            return 1

        # Every entry in the extended DENY set must be flagged.
        extended_tokens = {
            re.sub(r"[\s_-]", "", token.lower())
            for (rel, _, token, _) in hits
            if rel.name == "extended.h"
        }
        expected_extended = {
            "firehose",
            "routingtable",
            "arcswap",
            "dashmap",
            "rustchash",
            "epoll",
            "kqueue",
            # `pyo3-async-runtimes` normalises to `pyo3asyncruntimes` once
            # the hyphens are stripped.
            "pyo3asyncruntimes",
            "futureintopy",
            "sharedproducer",
            "statecell",
        }
        missing_extended = expected_extended - extended_tokens
        if missing_extended:
            print(
                "selftest FAILED: extended-DENY-set tokens slipped through: "
                f"{sorted(missing_extended)!r}"
            )
            return 1

        # The shipped README must be scanned and its leak flagged.
        if not any(rel.name == "README.md" for (rel, _, _, _) in hits):
            print(
                "selftest FAILED: the impl-IP leak in the shipped "
                "per-package README long-description was not flagged"
            )
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
