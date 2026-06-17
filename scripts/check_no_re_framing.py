#!/usr/bin/env python3
"""Gate the publicly-rendered SDK source against reverse-engineering framing.

The Rust, FFI, and binding source ships verbatim: it compiles into the
crates published on crates.io and its `///` / `//!` doc comments render on
docs.rs. Any sentence that frames the protocol work as reverse-engineering
the vendor's JVM terminal — naming a decompiled class or method, citing a
jar build the wire layout was checked against, or using the words
"reverse-engineered" / "decompiled" — ships to every reader who opens the
docs.

The approved story is the no-JVM SDK closing the parity gap with the
vendor's JVM terminal. The terminal is a legitimate parity reference, so
"Theta Terminal" and "JVM terminal" stay. What must never appear is the
provenance of how the wire format was learned: named internal Java
identifiers, jar-build verification notes, and the reverse-engineering /
decompilation vocabulary itself.

This gate scans the publicly-rendered source trees — the Rust crate
source (including generated `.rs`), the FFI source, and the Python and
TypeScript binding source — for that framing and fails with `file:line`
on any hit.

Run::

    python3 scripts/check_no_re_framing.py

Exit codes:

* ``0`` — clean.
* ``1`` — at least one source file frames the protocol as reverse-engineered.

Selftest::

    python3 scripts/check_no_re_framing.py --selftest

The selftest plants a `reverse-engineered the Java terminal` line in a
synthetic source file and confirms the gate flags it, then confirms a
clean file (one that names only the allow-listed "JVM terminal" /
"Theta Terminal" parity reference) passes.
"""

from __future__ import annotations

import argparse
import pathlib
import re
import sys
from typing import Iterable

REPO_ROOT = pathlib.Path(__file__).resolve().parents[1]


# Globs describing the publicly-rendered source, relative to the repo
# root. Every file here either compiles into a published crate or ships
# its doc comments to docs.rs. Generated `.rs` is included on purpose:
# the checked-in generated tick classes render exactly like hand-written
# source, so they must clear the same bar.
#
# The non-`.rs` schema descriptors below ride the crate `include` list in
# `crates/thetadatadx/Cargo.toml`, so their comments land in the crates.io
# tarball exactly like source. The `examples/` and `tests/` trees are not
# packaged but are GitHub-visible, and their doc comments are read as
# authoritative protocol notes — they clear the same bar.
SCAN_GLOBS = (
    "crates/thetadatadx/src/**/*.rs",
    "ffi/src/**/*.rs",
    "sdks/python/src/**/*.rs",
    "sdks/typescript/src/**/*.rs",
    "sdks/typescript/src/**/*.ts",
    # Schema / surface descriptors shipped via the crate `include` list.
    "crates/thetadatadx/tick_schema.toml",
    "crates/thetadatadx/endpoint_surface.toml",
    "crates/thetadatadx/sdk_surface.toml",
    "crates/thetadatadx/fpss_event_schema.toml",
    "crates/thetadatadx/data/*.toml",
    "sdks/parity.toml",
    # GitHub-visible examples and regression tests.
    "crates/thetadatadx/examples/**/*.rs",
    "crates/thetadatadx/tests/**/*.rs",
    "crates/thetadatadx/tests/**/*.toml",
    # Publicly-rendered prose. The docs site re-publishes these straight
    # from `main`, and the top-level Markdown is the first thing a reader
    # sees on the repo. The same provenance bar applies: these render to
    # every reader exactly like the crate doc comments, so a leak here
    # ships just as widely. This is the surface the `.rs`-only scan missed.
    "docs-site/docs/**/*.md",
    "SECURITY.md",
    "CHANGELOG.md",
    "README.md",
    "tools/server/README.md",
)


# Path fragments that exclude a candidate even when it matches a scan
# glob: vendored deps and build output never carry our prose.
EXEMPT_PATH_FRAGMENTS = (
    "/node_modules/",
    "/target/",
    "/.git/",
)


# Reverse-engineering framing that must never appear in the rendered
# source. Each entry is a compiled regex matched case-insensitively:
#
# * Named internal Java identifiers a reader could only learn from
#   decompilation (`Foo.toBytes()`, `Contract.java`, `FITReader.java`,
#   `javap`).
# * The verification-provenance note that records which terminal jar
#   build a wire layout was checked against.
# * The reverse-engineering / decompilation vocabulary itself.
#
# "Java terminal" is forbidden too: the approved spelling is "JVM
# terminal" (allow-listed below), so an explicit "Java terminal" hit is
# a drift signal, not a parity reference.
FORBIDDEN_PATTERNS = (
    re.compile(r"java\s+terminal", re.IGNORECASE),
    re.compile(r"\.toBytes\b"),
    re.compile(r"\.fromBytes\b"),
    re.compile(r"\bFITReader\.java\b"),
    re.compile(r"\bFIE\.java\b"),
    re.compile(r"\b\w+\.java\b"),
    re.compile(r"\bjavap\b", re.IGNORECASE),
    re.compile(r"decompil", re.IGNORECASE),
    re.compile(r"reverse[- ]engineer", re.IGNORECASE),
    re.compile(r"jar\s+build", re.IGNORECASE),
    re.compile(r"verified-live\s+against\s+terminal", re.IGNORECASE),
)


# The only parity-reference spellings that may name the vendor's product.
# A matched line is exonerated when removing every allow-listed phrase
# leaves no forbidden pattern behind, so the allow-list never masks a
# genuine leak that merely shares a line with the parity reference.
ALLOWLISTED_PHRASES = (
    "JVM terminal",
    "Theta Terminal",
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
    """Remove every allow-listed parity-reference phrase from `line`.

    Forbidden-pattern matching runs against the residue. A line whose
    only matches come from the allow-listed phrases is thereby cleared,
    while a real leak sharing a line with a parity reference still trips.
    """
    residue = line
    for phrase in ALLOWLISTED_PHRASES:
        residue = re.sub(re.escape(phrase), " ", residue, flags=re.IGNORECASE)
    return residue


def _scan_line(line: str) -> list[str]:
    """Return the forbidden patterns that survive allow-list stripping."""
    residue = _strip_allowlisted(line)
    return [pat.search(residue).group(0) for pat in FORBIDDEN_PATTERNS if pat.search(residue)]


def _scan(root: pathlib.Path) -> list[tuple[pathlib.Path, int, str, str]]:
    """Return (rel_path, lineno, matched, line_text) for every hit found."""
    hits: list[tuple[pathlib.Path, int, str, str]] = []
    for path in _iter_files(root):
        rel = path.relative_to(root)
        try:
            text = path.read_text(encoding="utf-8")
        except (OSError, UnicodeDecodeError):
            continue
        for lineno, line in enumerate(text.splitlines(), start=1):
            for matched in _scan_line(line):
                hits.append((rel, lineno, matched, line.strip()))
    return hits


def _selftest() -> int:
    """Plant RE framing in a synthetic source file and confirm the gate fires.

    Four cases:

    * A file with `reverse-engineered the Java terminal` plus a jar-build
      provenance note — must be flagged.
    * A shipped non-`.rs` schema descriptor (`tick_schema.toml`) carrying
      a `jar build NNN` provenance comment — must be flagged. This is the
      class of leak that previously evaded the `.rs`-only scan and shipped
      in the crates.io tarball.
    * A clean file that names only the allow-listed "JVM terminal" /
      "Theta Terminal" parity reference — must pass.
    * A clean file whose factual wire description shares a line with the
      allow-listed parity reference — must pass.
    """
    import tempfile

    leaky = (
        "//! We reverse-engineered the Java terminal to learn this layout.\n"
        "/// Wire layout verified-live against terminal jar build `202605221`.\n"
        "/// Source: `Contract.toBytes()` in `Contract.java`.\n"
    )
    leaky_schema = (
        'doc = """OHLC tick -- 9 fields.\n'
        "Wire layout verified-live against terminal jar build `202605221`.\n"
        '"""\n'
    )
    clean = (
        "//! Matches the JVM terminal byte-for-byte on the wire.\n"
        "/// Parity reference: the JVM terminal connects with a 2000 ms deadline.\n"
    )
    vendor_line = (
        "/// The wire format caps the encoded root at 16 bytes, matching the\n"
        "/// JVM terminal. Decoded against the Theta Terminal parity reference.\n"
    )

    with tempfile.TemporaryDirectory() as td:
        root = pathlib.Path(td)

        leaky_path = root / "crates" / "thetadatadx" / "src" / "leaky.rs"
        leaky_path.parent.mkdir(parents=True, exist_ok=True)
        leaky_path.write_text(leaky, encoding="utf-8")

        schema_path = root / "crates" / "thetadatadx" / "tick_schema.toml"
        schema_path.parent.mkdir(parents=True, exist_ok=True)
        schema_path.write_text(leaky_schema, encoding="utf-8")

        clean_path = root / "ffi" / "src" / "clean.rs"
        clean_path.parent.mkdir(parents=True, exist_ok=True)
        clean_path.write_text(clean, encoding="utf-8")

        vendor_path = root / "sdks" / "python" / "src" / "vendor.rs"
        vendor_path.parent.mkdir(parents=True, exist_ok=True)
        vendor_path.write_text(vendor_line, encoding="utf-8")

        hits = _scan(root)

        leaky_hits = [h for h in hits if h[0].name == "leaky.rs"]
        if not leaky_hits:
            print("selftest FAILED: the planted RE-framing line was not flagged")
            return 1
        schema_hits = [h for h in hits if h[0].name == "tick_schema.toml"]
        if not schema_hits:
            print(
                "selftest FAILED: the planted jar-build line in the shipped "
                "schema descriptor was not flagged"
            )
            return 1
        if any(rel.name == "clean.rs" for (rel, _, _, _) in hits):
            print("selftest FAILED: a clean JVM-terminal file was flagged")
            return 1
        if any(rel.name == "vendor.rs" for (rel, _, _, _) in hits):
            print("selftest FAILED: an allow-listed parity-reference file was flagged")
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
        print("no-re-framing: clean")
        return 0
    print(
        f"no-re-framing: {len(hits)} reverse-engineering framing leak(s) in "
        "the publicly-rendered source"
    )
    for rel, lineno, matched, line in hits:
        print(f"  {rel}:{lineno}: frames as `{matched}`")
        print(f"    {line}")
    print(
        "  -> The source renders on docs.rs. Describe the wire behavior "
        "factually and reference the parity target only as the "
        "`JVM terminal` / `Theta Terminal`; never name a decompiled "
        "identifier, a jar build, or the reverse-engineering act."
    )
    return 1


if __name__ == "__main__":
    sys.exit(main())
