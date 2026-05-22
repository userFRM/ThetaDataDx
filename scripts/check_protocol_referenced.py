#!/usr/bin/env python3
"""Gate: every CLAUDE.md in the repo links to docs/internal/audit-protocol.md.

The audit protocol (`docs/internal/audit-protocol.md`) is the standing
rulebook for every contributor and every LLM-assisted change. The repo
has at least one top-level ``CLAUDE.md``; per-SDK and per-crate
``CLAUDE.md`` files may land later. Every one of them must surface the
protocol link near the top so the first thing an LLM agent reads on
attaching to the repo points it at the rules.

This gate walks the tree, collects every ``CLAUDE.md``, and asserts the
relative path ``docs/internal/audit-protocol.md`` (or an absolute
``/docs/internal/audit-protocol.md`` style link) appears in the file.

Exit codes
----------

* ``0`` — every CLAUDE.md references the protocol.
* ``1`` — at least one CLAUDE.md is missing the reference.
* ``2`` — the protocol file itself does not exist.

Selftest
--------

Run with ``--selftest`` to simulate a regression and verify the
detector flags it.
"""

from __future__ import annotations

import argparse
import pathlib
import sys
import tempfile
from typing import Iterable


PROTOCOL_PATH = pathlib.Path("docs/internal/audit-protocol.md")

# Path fragments anywhere in the relative path that mark vendored or
# generated trees the gate must not police.
EXEMPT_PATH_FRAGMENTS = (
    "/.venv/",
    "/.venv-test/",
    "/venv/",
    "/node_modules/",
    "/target/",
    "/__pycache__/",
    "/.git/",
    "/.mypy_cache/",
    "/.pytest_cache/",
    "/build/",
    "/dist/",
)


def _iter_claude_md(root: pathlib.Path) -> Iterable[pathlib.Path]:
    for path in root.rglob("CLAUDE.md"):
        rel = path.relative_to(root).as_posix()
        rel_with_sep = "/" + rel
        if any(frag in rel_with_sep for frag in EXEMPT_PATH_FRAGMENTS):
            continue
        yield path


def _references_protocol(text: str) -> bool:
    needle_rel = "docs/internal/audit-protocol.md"
    needle_abs = "/docs/internal/audit-protocol.md"
    return needle_rel in text or needle_abs in text


def _check(root: pathlib.Path) -> list[pathlib.Path]:
    """Return the list of CLAUDE.md files missing the protocol reference."""
    missing: list[pathlib.Path] = []
    for path in _iter_claude_md(root):
        try:
            text = path.read_text(encoding="utf-8")
        except UnicodeDecodeError:
            text = path.read_text(encoding="utf-8", errors="replace")
        if not _references_protocol(text):
            missing.append(path)
    return missing


def _selftest() -> int:
    """Simulate a regression and verify the detector flags it."""
    with tempfile.TemporaryDirectory() as tmp:
        root = pathlib.Path(tmp)
        (root / "docs" / "internal").mkdir(parents=True)
        (root / "docs" / "internal" / "audit-protocol.md").write_text(
            "protocol body\n", encoding="utf-8"
        )

        # Case 1: clean — links to the protocol.
        clean = root / "CLAUDE.md"
        clean.write_text(
            "Read [the protocol](docs/internal/audit-protocol.md)\n",
            encoding="utf-8",
        )
        if _check(root):
            print("selftest FAILED: clean tree reported missing", file=sys.stderr)
            return 1

        # Case 2: regression — a per-crate CLAUDE.md without the link.
        crate = root / "crates" / "foo"
        crate.mkdir(parents=True)
        bad = crate / "CLAUDE.md"
        bad.write_text("no link here\n", encoding="utf-8")
        missing = _check(root)
        if not missing or bad not in missing:
            print(
                "selftest FAILED: regression not detected",
                file=sys.stderr,
            )
            return 1

        # Case 3: vendored CLAUDE.md is exempt.
        vendored = root / "node_modules" / "pkg" / "CLAUDE.md"
        vendored.parent.mkdir(parents=True)
        vendored.write_text("no link here either\n", encoding="utf-8")
        missing = _check(root)
        if vendored in missing:
            print(
                "selftest FAILED: vendored CLAUDE.md not exempted",
                file=sys.stderr,
            )
            return 1

    print("selftest OK")
    return 0


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--selftest",
        action="store_true",
        help="run the built-in regression selftest and exit",
    )
    parser.add_argument(
        "--root",
        default=".",
        help="repo root to scan (default: current working directory)",
    )
    args = parser.parse_args(argv)

    if args.selftest:
        return _selftest()

    root = pathlib.Path(args.root).resolve()
    protocol = root / PROTOCOL_PATH
    if not protocol.is_file():
        print(
            f"::error::audit protocol file missing at {PROTOCOL_PATH}",
            file=sys.stderr,
        )
        return 2

    missing = _check(root)
    if missing:
        print(
            "::error::CLAUDE.md files missing reference to "
            f"{PROTOCOL_PATH}:",
            file=sys.stderr,
        )
        for path in missing:
            print(f"  {path.relative_to(root)}", file=sys.stderr)
        print(
            "fix: add a link to "
            f"`{PROTOCOL_PATH}` near the top of each file above",
            file=sys.stderr,
        )
        return 1

    print("protocol-referenced: clean")
    return 0


if __name__ == "__main__":
    sys.exit(main())
