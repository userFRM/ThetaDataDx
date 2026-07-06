#!/usr/bin/env python3
"""Client-facing vocabulary gate: the prose surface says historical/streaming.

The two server channels are named after the data they serve on every surface
a user reads: the public API symbols, the CLI, the environment variables, the
error strings, the published binding type declarations, the examples, the
READMEs, the docs site, and the OpenAPI document. The internal transport
vocabulary (`fpss` / `mdds`) is permitted only in places a user never reads:
the transport source, the wire protocol, the proto package, generated-file
provenance banners, the real DNS hostnames, and the operator metric namespace.

`check_public_surface_leak.py` enforces the *symbol* surface (the exported API
identifiers). This gate is its prose complement: it greps the client-facing
*documentation* surface case-insensitively for `fpss` / `mdds` and fails on any
hit outside the encoded allow-list. Without it the historical/streaming
invariant on README / example / `.d.ts` prose is on the honor system, which is
exactly where manual review (not CI) caught the regressions this gate prevents.

Run::

    python3 scripts/ci/check_client_facing_vocab.py            # gate
    python3 scripts/ci/check_client_facing_vocab.py --selftest # in-process smoke test

Exit 0 + "client-facing vocab: clean" when there are no non-exempt hits; exit 1
plus a `file:line` list otherwise.
"""

from __future__ import annotations

import argparse
import pathlib
import re
import sys
import tempfile
from typing import Iterable

REPO_ROOT = pathlib.Path(__file__).resolve().parents[2]

SELF_NAME = pathlib.Path(__file__).name


# ── The swept client-facing surface ────────────────────────────────────────
#
# Glob patterns (relative to the repo root) for every file a user reads as
# product documentation. Listed explicitly rather than walked-by-extension
# because the client-facing surface is a curated subset: a TS source file
# under `thetadatadx-ts/src/` is internal and keeps its transport names,
# while the published `index.d.ts` / `streaming-session.d.ts` it generates is
# client-facing and must not. The exempt rules below carve the few internal
# files that happen to match these globs.
SWEEP_GLOBS = (
    # READMEs the integrator reads.
    "README.md",
    "thetadatadx-py/README.md",
    "thetadatadx-ts/README.md",
    "thetadatadx-cpp/README.md",
    "thetadatadx-ffi/README.md",
    "tools/mcp/README.md",
    "tools/server/README.md",
    # Published examples, every language.
    "thetadatadx-py/examples/**/*",
    "thetadatadx-ts/examples/**/*",
    "thetadatadx-cpp/examples/**/*",
    # Published type declarations (the IDE-hover surface).
    "thetadatadx-py/**/*.pyi",
    "thetadatadx-ts/**/*.d.ts",
    "thetadatadx-cpp/include/**/*.h",
    "thetadatadx-cpp/include/**/*.hpp",
    # User-facing docs site + the published OpenAPI contract.
    "docs-site/docs/**/*.md",
    "docs-site/docs/public/**/*.yaml",
    # User-editable config + the developer helper scripts shipped in-tree.
    "thetadatadx-rs/config.default.toml",
    "scripts/dev/*.py",
)


# ── Path-level exemptions ───────────────────────────────────────────────────
#
# Files that match a sweep glob but are NOT the client-facing prose surface:
# append-only history, contributor / internal docs, the wire/proto contract,
# generated includes, CI config, internal source, and the guard scripts. A
# substring match against `/<relpath>/` exempts the whole file.
EXEMPT_PATH_FRAGMENTS = (
    # Append-only history: the old names are the accurate record of a past
    # release and must not be rewritten.
    "/CHANGELOG.md",
    "/docs-site/docs/changelog.md",
    "/docs-site/docs/migration/",
    "/.github/release-notes/",
    # Contributor / internal documentation (not the product-usage surface).
    "/CONTRIBUTING.md",
    "/SECURITY.md",
    "/thetadatadx-rs/proto/MAINTENANCE.md",
    "/thetadatadx-rs/benches/README.md",
    # CI config, internal source trees, generated wire includes, build output.
    "/.github/workflows/",
    "/src/",
    "/_generated/",
    "/node_modules/",
    "/target/",
    "/.git/",
)


# ── Line-level exemptions ───────────────────────────────────────────────────
#
# Within a swept file, a line whose only `fpss`/`mdds` occurrence is one of
# these is not a leak: it names a real internal artifact a user may legitimately
# encounter (a generated-file provenance banner, a wire-include filename, the
# real DNS hosts, the operator metric namespace). Matched case-insensitively;
# the line is cleared of these spans before the leak scan runs, so a real leak
# sharing a line with an exempt token still trips.
EXEMPT_LINE_PATTERNS = (
    # Generated-file provenance banners cite the schema they are emitted from.
    re.compile(r"@generated", re.IGNORECASE),
    re.compile(r"fpss_event_schema\.toml", re.IGNORECASE),
    # Wire-include filenames (the C/C++ event-struct includes).
    re.compile(r"fpss_event_structs\.h(?:\.inc)?", re.IGNORECASE),
    re.compile(r"fpss_layout_asserts\.hpp(?:\.inc)?", re.IGNORECASE),
    re.compile(r"fpss\.hpp\.inc", re.IGNORECASE),
    re.compile(r"fpss_event_classes", re.IGNORECASE),
    # Real DNS hostnames (the actual production / staging endpoints).
    re.compile(r"mdds-01\.thetadata\.us", re.IGNORECASE),
    re.compile(r"mdds-stage\.thetadata\.us", re.IGNORECASE),
    # Operator metric namespace (renaming breaks dashboards / alerts).
    re.compile(r"thetadatadx\.fpss\.", re.IGNORECASE),
    # The bundled server is an exact drop-in for the JVM terminal, which
    # publishes these two system routes verbatim (docs.thetadata.us). The
    # server mirrors them 1:1, so the literal route paths are permitted on the
    # public surface — exclusively as the route path. Any other fpss/mdds
    # occurrence on the line (a bare descriptor) still trips, so descriptions
    # use "streaming" / "historical", not the codename.
    re.compile(r"/v3/terminal/fpss/status", re.IGNORECASE),
    re.compile(r"/v3/terminal/mdds/status", re.IGNORECASE),
)


# The internal transport vocabulary forbidden on the client-facing surface.
# Matched as a plain case-insensitive substring, NOT word-bounded: `\b` treats
# `_` as a word character, so a `\bfpss\b` pattern would miss the very leaks
# that matter most — `THETADATA_FPSS_TYPE`, `THETADATADX_FPSS_QUOTE`,
# `MDDS_TYPE` — where the token is embedded in an underscore-delimited
# identifier. `fpss` / `mdds` are distinctive enough that substring matching
# does not collide with unrelated words.
LEAK_PATTERN = re.compile(r"(?:fpss|mdds)", re.IGNORECASE)


def _is_exempt_path(rel_path: pathlib.Path) -> bool:
    parts = "/" + rel_path.as_posix() + "/"
    if any(fragment in parts for fragment in EXEMPT_PATH_FRAGMENTS):
        return True
    return rel_path.name == SELF_NAME


def _iter_swept_files(root: pathlib.Path) -> Iterable[pathlib.Path]:
    seen: set[pathlib.Path] = set()
    for pattern in SWEEP_GLOBS:
        for candidate in sorted(root.glob(pattern)):
            if not candidate.is_file():
                continue
            rel = candidate.relative_to(root)
            if rel in seen:
                continue
            if _is_exempt_path(rel):
                continue
            seen.add(rel)
            yield candidate


def _strip_exempt_spans(line: str) -> str:
    """Blank every exempt span so a leak sharing the line still trips."""
    residue = line
    for pattern in EXEMPT_LINE_PATTERNS:
        residue = pattern.sub(" ", residue)
    return residue


def _scan(root: pathlib.Path) -> list[tuple[pathlib.Path, int, str]]:
    """Return (rel_path, lineno, line_text) for every non-exempt leak."""
    hits: list[tuple[pathlib.Path, int, str]] = []
    for path in _iter_swept_files(root):
        rel = path.relative_to(root)
        try:
            text = path.read_text(encoding="utf-8", errors="replace")
        except OSError:
            continue
        for lineno, line in enumerate(text.splitlines(), start=1):
            if LEAK_PATTERN.search(_strip_exempt_spans(line)):
                hits.append((rel, lineno, line.strip()))
    return hits


def _run(root: pathlib.Path) -> int:
    hits = _scan(root)
    if hits:
        print(
            "client-facing vocab: FAIL — internal transport names (fpss/mdds) "
            "found on the client-facing surface:",
            file=sys.stderr,
        )
        for rel, lineno, line in hits:
            print(f"  {rel}:{lineno}: {line}", file=sys.stderr)
        print(
            "\nThe client-facing surface says historical/streaming. If a hit is a "
            "real internal artifact (generated-file banner, wire-include filename, "
            "DNS host, metric namespace), add it to the exempt rules; otherwise "
            "rename it to the channel vocabulary.",
            file=sys.stderr,
        )
        return 1
    print("client-facing vocab: clean")
    return 0


# ── In-process selftest ─────────────────────────────────────────────────────


def _selftest() -> int:
    """Plant synthetic files and confirm the gate fires on a leak, not on exempt.

    Cases:
      * a fake README carrying "FPSS event" prose -> must be flagged;
      * a file whose only hits are exempt (DNS host, metric namespace,
        @generated banner, a leak sharing a line with an exempt span) ->
        must be clean, except the genuine leak sharing the exempt line.
    """
    failures = 0

    def check(name: str, cond: bool) -> None:
        nonlocal failures
        if not cond:
            failures += 1
            print(f"  selftest FAIL: {name}", file=sys.stderr)

    with tempfile.TemporaryDirectory() as tmp:
        root = pathlib.Path(tmp)

        # Leak fixture: a README with client-facing FPSS prose.
        (root / "README.md").write_text(
            "# Demo\n\nThe SDK delivers every typed FPSS event to your callback.\n",
            encoding="utf-8",
        )
        leak_hits = _scan(root)
        check(
            "client-facing FPSS prose in README is flagged",
            any(rel.as_posix() == "README.md" for rel, _, _ in leak_hits),
        )

        # Exempt-only fixture: every hit is allow-listed -> clean.
        (root / "README.md").write_text(
            "# Demo\n"
            "Market-data host is mdds-01.thetadata.us; staging is "
            "mdds-stage.thetadata.us.\n"
            "Scrape the thetadatadx.fpss.reconnects counter.\n"
            "<!-- @generated from fpss_event_schema.toml -->\n"
            '#include "fpss_event_structs.h.inc"\n',
            encoding="utf-8",
        )
        check("exempt-only README is clean", _scan(root) == [])

        # Path-exempt fixture: a migration doc may carry the old names.
        mig = root / "docs-site" / "docs" / "migration"
        mig.mkdir(parents=True)
        (mig / "v1.md").write_text("v12 used THETADATA_FPSS_TYPE.\n", encoding="utf-8")
        check("migration history path is exempt", _scan(root) == [])

        # Shared-line fixture: a real leak next to an exempt DNS host still trips.
        (root / "README.md").write_text(
            "Connect to mdds-01.thetadata.us over the FPSS stream.\n",
            encoding="utf-8",
        )
        shared = _scan(root)
        check(
            "a leak sharing a line with an exempt span still trips",
            any(rel.as_posix() == "README.md" for rel, _, _ in shared),
        )

    if failures:
        print(f"client-facing vocab selftest: {failures} case(s) FAILED", file=sys.stderr)
    else:
        print("client-facing vocab selftest: all cases passed")
    return failures


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--selftest",
        action="store_true",
        help="run the in-process synthetic fixtures instead of scanning the repo",
    )
    parser.add_argument(
        "--root",
        type=pathlib.Path,
        default=REPO_ROOT,
        help="repository root to scan (defaults to the repo this script lives in)",
    )
    args = parser.parse_args()
    if args.selftest:
        return _selftest()
    return _run(args.root)


if __name__ == "__main__":
    sys.exit(main())
