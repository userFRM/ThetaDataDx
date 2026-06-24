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

This gate scans the entire publicly-rendered tree — every text file with
a known public-facing extension anywhere in the repository — for that
framing and fails with `file:line` on any hit. Walking the whole tree
(rather than a hand-picked glob list) is deliberate: a hand-picked list
goes stale every time a new source directory or binding lands, and the
provenance leak this gate exists to stop has slipped through exactly that
gap before. Only build output and vendored trees are excluded.

Run::

    python3 scripts/ci/check_no_re_framing.py

Exit codes:

* ``0`` — clean.
* ``1`` — at least one source file frames the protocol as reverse-engineered.

Selftest::

    python3 scripts/ci/check_no_re_framing.py --selftest

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

REPO_ROOT = pathlib.Path(__file__).resolve().parents[2]


# Every text file with one of these extensions is scanned, anywhere in
# the repository, unless it sits under an exempt path (below). This is the
# whole publicly-rendered surface: Rust / FFI / binding source and its
# doc comments (crates.io + docs.rs), C++ headers and source, Python and
# its stubs, TypeScript and its declaration files, every flavour of
# JavaScript module, every `.toml` manifest / schema descriptor, all
# Markdown (top-level and the docs site), and the CI workflows whose text
# is echoed into release artifacts. Walking by extension rather than by a
# curated glob list means a new source directory or binding is covered the
# moment it lands, with no edit to this gate.
SCAN_EXTENSIONS = frozenset(
    {
        ".rs",
        ".py",
        ".pyi",
        ".ts",
        ".d.ts",
        ".mjs",
        ".cjs",
        ".js",
        ".hpp",
        ".h",
        ".inc",
        ".cpp",
        ".cc",
        ".cxx",
        ".md",
        ".toml",
        ".yml",
        ".yaml",
    }
)


# Path fragments that exclude a candidate from the walk: build output,
# vendored deps, and version-control metadata never carry our public
# prose. Everything else with a scanned extension is in scope.
EXEMPT_PATH_FRAGMENTS = (
    "/node_modules/",
    "/target/",
    "/.git/",
    # Vendored / third-party trees never carry our prose.
    "/vendor/",
    "/dist/",
    "/build/",
    # Build-support codegen that strips upstream provenance at build time.
    # It is not packaged into any crate (absent from the `include` list)
    # and intentionally contains the provenance filter terms it removes;
    # scanning it would flag the very guard that protects the public
    # surface.
    "/build_support_bin/",
    # Pinned upstream / CI input data, not our publicly-rendered source.
    # `scripts/ci/data/` holds the verbatim vendor OpenAPI snapshot
    # (`upstream_openapi.yaml`, refreshed wholesale from
    # `docs.thetadata.us/openapiv3.yaml`) and gate allow-lists. The vendor
    # spec legitimately carries the vendor's own `x-java-package:
    # net.thetadata.*` field — that is THEIR published metadata, not our
    # provenance leak, and it is never shipped in any crate or rendered on
    # docs.rs. This directory is the same category as `build_support_bin/`:
    # CI input, not public prose.
    "/scripts/ci/data/",
)


# This gate's own source carries every forbidden pattern as a literal so
# the regexes can be defined; scanning it would flag the guard itself.
SELF_NAME = pathlib.Path(__file__).name


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
    # `java`-then-`terminal` with any (or no) separator: catches
    # `Java terminal`, `Java-terminal`, `java_terminal`, AND the
    # separator-free camelCase `JavaTerminal`. The separator is
    # zero-or-more (`*`) precisely so the glued camelCase spelling cannot
    # dodge the gate. `JVM terminal` / `Theta Terminal` remain
    # allow-listed and are stripped before matching.
    re.compile(r"java[\s\-_]*terminal", re.IGNORECASE),
    re.compile(r"\.toBytes\b", re.IGNORECASE),
    re.compile(r"\.fromBytes\b", re.IGNORECASE),
    re.compile(r"\bFITReader\.java\b", re.IGNORECASE),
    re.compile(r"\bFIE\.java\b", re.IGNORECASE),
    # Any `<identifier>.java` source reference, in any casing
    # (`Contract.java`, `CONTRACT.JAVA`): a named Java source is
    # provenance a reader could only have from decompilation.
    re.compile(r"\b\w+\.java\b", re.IGNORECASE),
    # A dotted `net.thetadata.*` Java fully-qualified class name
    # (`net.thetadata.types.tick.MarketValueTick`): the package path is
    # provenance a reader could only have from the decompiled terminal,
    # and unlike a `Contract.java` source reference it carries no `.java`
    # suffix, so the `<identifier>.java` pattern above never sees it. Only
    # the DOTTED form is forbidden — `net.thetadata` must be immediately
    # followed by a `.`-separated path. The bare class name on its own
    # (`MarketValueTick`) is a legitimate public SDK type that ships across
    # every binding and must stay clean, so the package prefix is required.
    re.compile(r"\bnet\.thetadata\.[\w.]+", re.IGNORECASE),
    re.compile(r"\bjavap\b", re.IGNORECASE),
    re.compile(r"decompil", re.IGNORECASE),
    # The reverse-engineering vocabulary itself, with any separator
    # (`reverse-engineer`, `reverse engineer`, `reverse_engineer`,
    # `reversed engineering`). `revers(e|ed)` + separator(s) + `engineer`.
    re.compile(r"revers(?:e|ed)[\s\-_]+engineer", re.IGNORECASE),
    # The abbreviated form: `RE'd` / `RE-d` ("we RE'd the wire format").
    # Bounded to the uppercase `RE` token followed by a straight or curly
    # apostrophe or a hyphen and a trailing `d`, so ordinary contractions
    # (`we're`, `they'd`) and words like `pre-d...` do not trip: the `RE`
    # must stand alone (`(?<![A-Za-z])RE`) and the match must end on a
    # word boundary.
    re.compile(r"(?<![A-Za-z])RE['’\-]d\b"),
    re.compile(r"jar[\s\-_]+build", re.IGNORECASE),
    re.compile(r"verified-live\s+against\s+terminal", re.IGNORECASE),
    # Jar-provenance: the act of citing the vendor jar, the terminal jar,
    # the local extraction path it was pulled from, or any bare `.jar`
    # mention. The wire format is described factually against the
    # allow-listed `JVM terminal` / `Theta Terminal` parity reference,
    # never against a jar artifact.
    re.compile(r"\bvendor\s+jar\b", re.IGNORECASE),
    re.compile(r"\bThetaTerminal\s+jar\b", re.IGNORECASE),
    re.compile(r"ThetaTerminal/\S*(?:downloads|jar)", re.IGNORECASE),
    re.compile(r"\.jar\b", re.IGNORECASE),
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
    if any(fragment in parts for fragment in EXEMPT_PATH_FRAGMENTS):
        return True
    # The gate's own source carries the forbidden literals by necessity.
    return rel_path.name == SELF_NAME


def _is_scanned(rel_path: pathlib.Path) -> bool:
    """True when the file's extension is in the public-facing scan set."""
    name = rel_path.name
    # `.d.ts` would otherwise read as a bare `.ts`; both are scanned, so
    # the suffix check covers it, but spell it out for clarity.
    if name.endswith(".d.ts"):
        return ".d.ts" in SCAN_EXTENSIONS or ".ts" in SCAN_EXTENSIONS
    return rel_path.suffix.lower() in SCAN_EXTENSIONS


_EXEMPT_DIR_NAMES = frozenset(
    fragment.strip("/") for fragment in EXEMPT_PATH_FRAGMENTS
)


def _iter_files(root: pathlib.Path) -> Iterable[pathlib.Path]:
    import os

    for dirpath, dirnames, filenames in os.walk(root):
        # Prune excluded subtrees in place so the walk never descends
        # into build output / vendored deps (e.g. `node_modules`).
        dirnames[:] = sorted(d for d in dirnames if d not in _EXEMPT_DIR_NAMES)
        for filename in sorted(filenames):
            candidate = pathlib.Path(dirpath) / filename
            rel = candidate.relative_to(root)
            if _is_exempt(rel):
                continue
            if not _is_scanned(rel):
                continue
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
    """Plant RE framing in synthetic source files and confirm the gate fires.

    Cases:

    * A file with `reverse-engineered the Java terminal` plus a jar-build
      provenance note — must be flagged.
    * A shipped non-`.rs` schema descriptor (`tick_schema.toml`) carrying
      a `jar build NNN` provenance comment — must be flagged. This is the
      class of leak that previously evaded the `.rs`-only scan and shipped
      in the crates.io tarball.
    * A file outside the old hand-picked glob list (`tools/server/src`)
      carrying a hyphenated `Java-terminal` — must be flagged. This proves
      the whole-tree walk plus the hyphen/underscore/whitespace pattern
      catches the spelling variants the curated scan missed.
    * A C++ header (`sdks/cpp/include`) naming a decompiled `.java` source
      — must be flagged, proving non-Rust extensions are in scope.
    * A file carrying the spelling variants the older patterns missed —
      the separator-free camelCase `JavaTerminal`, the abbreviated `RE'd`
      / `RE-d`, the `reversed engineering` conjugation, an uppercase
      `.JAVA` source, and a hyphenated `jar-build` note — every one must
      be flagged.
    * A file naming a dotted `net.thetadata.*` Java fully-qualified class
      name (`net.thetadata.types.tick.MarketValueTick`) — must be flagged.
      The package path has no `.java` suffix, so the `<identifier>.java`
      pattern never saw it; the dotted-FQCN pattern closes that gap.
    * A clean file naming only the BARE public SDK type (`MarketValueTick`,
      `QuoteTick`) with no package prefix — must pass, proving the dotted
      pattern does not punish the legitimate cross-binding type name.
    * A clean file that names only the allow-listed "JVM terminal" /
      "Theta Terminal" parity reference — must pass.
    * A clean file whose factual wire description shares a line with the
      allow-listed parity reference — must pass.
    * A clean file of ordinary contractions (`they'd`, `we're`,
      `PRE-decode`, `CORE-d`) — must pass, proving the tuned `RE'd`
      pattern does not false-positive.
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
    # Hyphenated spelling in a tree outside the old curated glob list.
    leaky_hyphen = (
        "/// Convert a shared endpoint output into the Java-terminal envelope.\n"
        "/// The java_terminal underscore spelling must trip too.\n"
    )
    # Non-Rust extension: a decompiled identifier named in a C++ header.
    leaky_cpp = (
        "// Field order mirrors `Contract.java` from the decompiled layout.\n"
        "struct OhlcTick { double open; };\n"
    )
    # Spelling variants the older patterns let through: the separator-free
    # camelCase `JavaTerminal`, the abbreviated `RE'd` / `RE-d`, the
    # `reversed engineering` conjugation, an uppercase `.JAVA` source, and
    # the hyphenated `jar-build` provenance note. Each must trip.
    leaky_variants = (
        "/// Mirrors the JavaTerminal envelope layout.\n"
        "/// We RE'd the framing, then RE-d the deadline handshake.\n"
        "/// Layout reversed engineering notes for the OHLC packet.\n"
        "/// Source: `Contract.JAVA` decompiled output.\n"
        "/// Checked against the terminal jar-build `202605221`.\n"
    )
    # A dotted Java fully-qualified class name from the decompiled terminal
    # package tree. It carries no `.java` suffix, so the older
    # `<identifier>.java` pattern never saw it. Must trip.
    leaky_fqcn = (
        "/// Field order mirrors net.thetadata.types.tick.MarketValueTick.\n"
        "/// Also seen as NET.THETADATA.Types.Tick.QuoteTick in another build.\n"
    )
    # The bare public SDK type name on its own — no package prefix. This is
    # a legitimate cross-binding type and must stay clean; the dotted-FQCN
    # pattern must NOT fire on it.
    clean_bare_type = (
        "/// Returns a MarketValueTick with the consolidated NBBO snapshot.\n"
        "/// The QuoteTick and TradeTick structs share the same header.\n"
    )
    clean = (
        "//! Matches the JVM terminal byte-for-byte on the wire.\n"
        "/// Parity reference: the JVM terminal connects with a 2000 ms deadline.\n"
    )
    vendor_line = (
        "/// The wire format caps the encoded root at 16 bytes, matching the\n"
        "/// JVM terminal. Decoded against the Theta Terminal parity reference.\n"
    )
    # Ordinary English contractions and unrelated `RE`-adjacent text that
    # must NOT trip the abbreviated `RE'd` pattern. Guards against the
    # false-positive class the tuned boundary is designed to avoid.
    clean_contractions = (
        "/// They'd already cached the handshake; we're done before the deadline.\n"
        "/// The PRE-decode buffer and a CORE-d helper stay untouched.\n"
    )

    with tempfile.TemporaryDirectory() as td:
        root = pathlib.Path(td)

        leaky_path = root / "crates" / "thetadatadx" / "src" / "leaky.rs"
        leaky_path.parent.mkdir(parents=True, exist_ok=True)
        leaky_path.write_text(leaky, encoding="utf-8")

        schema_path = root / "crates" / "thetadatadx" / "tick_schema.toml"
        schema_path.parent.mkdir(parents=True, exist_ok=True)
        schema_path.write_text(leaky_schema, encoding="utf-8")

        hyphen_path = root / "tools" / "server" / "src" / "hyphen.rs"
        hyphen_path.parent.mkdir(parents=True, exist_ok=True)
        hyphen_path.write_text(leaky_hyphen, encoding="utf-8")

        cpp_path = root / "sdks" / "cpp" / "include" / "tick.hpp"
        cpp_path.parent.mkdir(parents=True, exist_ok=True)
        cpp_path.write_text(leaky_cpp, encoding="utf-8")

        clean_path = root / "ffi" / "src" / "clean.rs"
        clean_path.parent.mkdir(parents=True, exist_ok=True)
        clean_path.write_text(clean, encoding="utf-8")

        vendor_path = root / "sdks" / "python" / "src" / "vendor.rs"
        vendor_path.parent.mkdir(parents=True, exist_ok=True)
        vendor_path.write_text(vendor_line, encoding="utf-8")

        variants_path = root / "crates" / "thetadatadx" / "src" / "variants.rs"
        variants_path.parent.mkdir(parents=True, exist_ok=True)
        variants_path.write_text(leaky_variants, encoding="utf-8")

        fqcn_path = root / "crates" / "thetadatadx" / "src" / "fqcn.rs"
        fqcn_path.parent.mkdir(parents=True, exist_ok=True)
        fqcn_path.write_text(leaky_fqcn, encoding="utf-8")

        bare_type_path = root / "sdks" / "cpp" / "include" / "bare_type.hpp"
        bare_type_path.parent.mkdir(parents=True, exist_ok=True)
        bare_type_path.write_text(clean_bare_type, encoding="utf-8")

        contractions_path = root / "ffi" / "src" / "contractions.rs"
        contractions_path.parent.mkdir(parents=True, exist_ok=True)
        contractions_path.write_text(clean_contractions, encoding="utf-8")

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
        hyphen_hits = [h for h in hits if h[0].name == "hyphen.rs"]
        if not any("terminal" in m.lower() for (_, _, m, _) in hyphen_hits):
            print(
                "selftest FAILED: the hyphen/underscore `Java-terminal` "
                "spelling outside the old glob list was not flagged"
            )
            return 1
        if not [h for h in hits if h[0].name == "tick.hpp"]:
            print(
                "selftest FAILED: a decompiled `.java` reference in a C++ "
                "header was not flagged"
            )
            return 1
        if any(rel.name == "clean.rs" for (rel, _, _, _) in hits):
            print("selftest FAILED: a clean JVM-terminal file was flagged")
            return 1
        if any(rel.name == "vendor.rs" for (rel, _, _, _) in hits):
            print("selftest FAILED: an allow-listed parity-reference file was flagged")
            return 1

        # A dotted `net.thetadata.*` FQCN must be flagged.
        fqcn_hits = [h for h in hits if h[0].name == "fqcn.rs"]
        if not any("net.thetadata" in m.lower() for (_, _, m, _) in fqcn_hits):
            print(
                "selftest FAILED: a dotted net.thetadata.* Java FQCN was not "
                "flagged (the package path is decompilation provenance)"
            )
            return 1
        # The bare public SDK type name (no package prefix) must stay clean.
        if any(rel.name == "bare_type.hpp" for (rel, _, _, _) in hits):
            bad = [
                (m, line)
                for (rel, _, m, line) in hits
                if rel.name == "bare_type.hpp"
            ]
            print(
                "selftest FAILED: the dotted-FQCN pattern false-positived on a "
                f"bare public type name: {bad!r}"
            )
            return 1

        # Spelling-variant coverage: every new variant must surface at
        # least one hit on its own line.
        variant_matches = {
            m for (rel, _, m, _) in hits if rel.name == "variants.rs"
        }
        variant_checks = {
            "JavaTerminal (camelCase, no separator)": any(
                "javaterminal" in m.lower().replace(" ", "") for m in variant_matches
            ),
            "RE'd / RE-d abbreviation": any(
                re.fullmatch(r"RE['’\-]d", m) for m in variant_matches
            ),
            "reversed engineering conjugation": any(
                m.lower().startswith("reversed") and "engineer" in m.lower()
                for m in variant_matches
            ),
            "uppercase .JAVA source": any(
                m.lower().endswith(".java") for m in variant_matches
            ),
            "jar-build hyphenated provenance": any(
                m.lower().startswith("jar") and "build" in m.lower()
                for m in variant_matches
            ),
        }
        unmet = [label for label, ok in variant_checks.items() if not ok]
        if unmet:
            print(
                "selftest FAILED: these RE-framing spelling variants slipped "
                f"through: {unmet!r} (matched: {sorted(variant_matches)!r})"
            )
            return 1

        # The tuned `RE'd` pattern must NOT fire on ordinary contractions
        # or `RE`-adjacent words.
        if any(rel.name == "contractions.rs" for (rel, _, _, _) in hits):
            bad = [
                (m, line)
                for (rel, _, m, line) in hits
                if rel.name == "contractions.rs"
            ]
            print(
                "selftest FAILED: the `RE'd` pattern false-positived on an "
                f"ordinary contraction: {bad!r}"
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
