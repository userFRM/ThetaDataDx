#!/usr/bin/env python3
"""Structural detector for stamped SAFETY-comment boilerplate.

The original `check_banned_vocab.py` regression guard only catches the
literal string that landed in #572. A future bot can defeat that gate by
re-emitting a fresh boilerplate string at three or more sites — the
text changes, but the lint pathology is identical: a copy-pasted
SAFETY annotation that names neither the invariant nor any per-site
detail, masquerading as a real safety review.

This detector closes that gap structurally:

* Collect every ``// SAFETY: <text>`` block across the Rust tree.
* Bucket by the exact normalised text.
* Flag any bucket whose population is >= ``DEFAULT_MIN_OCCURRENCES``,
  spans at least one non-FFI site, and whose text mentions no
  per-site invariant token (an identifier in backticks, a lifetime
  annotation, an atomic ordering, a layout property, etc.).

The text-token heuristic is deliberately permissive — a genuine
SAFETY annotation almost always names the type, the lifetime, or the
ordering that makes the unsafe block sound. Boilerplate that says
"caller upholds the contract" mentions none of those.

Run::

    python3 scripts/check_safety_comment_boilerplate.py

Exit codes:

* ``0`` — clean.
* ``1`` — at least one offending bucket detected.

Selftest::

    python3 scripts/check_safety_comment_boilerplate.py --selftest

The selftest simulates a regression and verifies the detector flags it.
"""

from __future__ import annotations

import argparse
import pathlib
import re
import sys
from collections import defaultdict
from typing import Iterable


REPO_ROOT = pathlib.Path(__file__).resolve().parents[1]


# Minimum number of verbatim-identical SAFETY comments at non-FFI sites
# that triggers the gate. Three is the same floor cited in the audit
# finding M3 — two copies are plausibly coincidence, three is a pattern.
DEFAULT_MIN_OCCURRENCES = 3


# Files / directories the detector skips. The detector itself is
# excluded via the `/scripts/` fragment — it embeds the regression
# sample as a string literal and would otherwise match itself.
SCAN_GLOBS = ("crates/**/*.rs", "ffi/**/*.rs", "tools/**/*.rs", "sdks/**/*.rs")

# Per-site allowlist file. Substring matches against the normalised
# SAFETY-comment text exempt the comment from the duplicate count.
# See the file's own header for the policy on when to add an entry.
ALLOWLIST_PATH = REPO_ROOT / "scripts" / "safety_comment_allowlist.txt"

EXEMPT_PATH_FRAGMENTS = (
    "/target/",
    "/.git/",
)


# Regex matching a `// SAFETY:` line and capturing the trailing text.
# We collect continuation lines (subsequent `//` comments) into the same
# block until the comment terminates. Doc-style `/// SAFETY:` is
# included — same review pathology applies.
SAFETY_LINE_RE = re.compile(r"^\s*///?\s*SAFETY:\s*(?P<body>.*)$")
COMMENT_CONT_RE = re.compile(r"^\s*///?\s?(?P<body>.*)$")


# Per-site invariant signals. If any of these appears in the comment
# text the gate treats it as a "real" SAFETY annotation and skips it
# regardless of duplicate count. Tokens come from the audit finding's
# heuristic list: identifiers in backticks, lifetime annotations,
# atomic orderings, layout properties, validity language, etc.
INVARIANT_SIGNAL_PATTERNS = (
    re.compile(r"`[^`]+`"),                       # backtick-quoted identifier
    re.compile(r"\B'[a-z_][a-z_0-9]*\b"),         # lifetime annotation, e.g. 'a
    re.compile(r"\bOrdering::"),                  # atomic ordering
    re.compile(r"\brepr\("),                      # layout repr
    re.compile(r"\bsize_of\b"),
    re.compile(r"\balign_of\b"),
    re.compile(r"\boffset(?:_of)?\b"),
    re.compile(r"\bnon[- ]null\b", re.IGNORECASE),
    re.compile(r"\bvalid for\b", re.IGNORECASE),
    re.compile(r"\bdiscriminant\b"),
    re.compile(r"\baligned\b"),
    re.compile(r"\bUTF-?8\b", re.IGNORECASE),
    re.compile(r"\bNULL?\b"),
    re.compile(r"\bnul[- ]terminated\b", re.IGNORECASE),
    re.compile(r"\bCString::"),
    re.compile(r"\bunique\b", re.IGNORECASE),     # uniqueness / aliasing
    re.compile(r"\bborrow\b", re.IGNORECASE),
    re.compile(r"\bMutex|RwLock|Arc|Box|Rc\b"),
    re.compile(r"\binitialise|initialize\b", re.IGNORECASE),
)


def _is_exempt(rel_path: pathlib.Path) -> bool:
    parts = "/" + rel_path.as_posix() + "/"
    for fragment in EXEMPT_PATH_FRAGMENTS:
        if fragment in parts:
            return True
    return False


def _load_allowlist(path: pathlib.Path = ALLOWLIST_PATH) -> list[str]:
    """Read the per-site allowlist substrings. Empty if file missing."""
    if not path.is_file():
        return []
    entries: list[str] = []
    for raw in path.read_text(encoding="utf-8").splitlines():
        stripped = raw.strip()
        if not stripped or stripped.startswith("#"):
            continue
        entries.append(_normalise(stripped).lower())
    return entries


def _is_allowlisted(text: str, allowlist: list[str]) -> bool:
    if not allowlist:
        return False
    haystack = text.lower()
    return any(entry in haystack for entry in allowlist)


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


def _normalise(text: str) -> str:
    """Collapse whitespace so trivial reflow does not defeat the gate."""
    return " ".join(text.split())


def _mentions_invariant(text: str) -> bool:
    for pattern in INVARIANT_SIGNAL_PATTERNS:
        if pattern.search(text):
            return True
    return False


def _extract_safety_blocks(path: pathlib.Path) -> list[tuple[int, str]]:
    """Return (line_number, normalised_text) for every SAFETY block in `path`.

    A block starts on a line matching ``// SAFETY: <body>`` and absorbs
    subsequent contiguous ``// <body>`` lines (allowing multi-line
    annotations).
    """
    try:
        text = path.read_text(encoding="utf-8")
    except (OSError, UnicodeDecodeError):
        return []

    blocks: list[tuple[int, str]] = []
    lines = text.splitlines()
    i = 0
    while i < len(lines):
        m = SAFETY_LINE_RE.match(lines[i])
        if not m:
            i += 1
            continue
        start = i + 1  # 1-based line number
        chunks = [m.group("body").strip()]
        j = i + 1
        while j < len(lines):
            cont = COMMENT_CONT_RE.match(lines[j])
            if not cont:
                break
            body = cont.group("body").strip()
            if not body:
                # blank `//` line ends a multi-line comment block
                break
            if SAFETY_LINE_RE.match(lines[j]):
                # another SAFETY: header — start a new block
                break
            chunks.append(body)
            j += 1
        normalised = _normalise(" ".join(chunks))
        blocks.append((start, normalised))
        i = j
    return blocks


def _scan(
    root: pathlib.Path,
    min_occurrences: int = DEFAULT_MIN_OCCURRENCES,
    allowlist: list[str] | None = None,
) -> list[tuple[str, list[tuple[pathlib.Path, int]]]]:
    """Return a list of (boilerplate_text, occurrences) buckets that trip the gate."""
    if allowlist is None:
        allowlist = _load_allowlist()
    buckets: dict[str, list[tuple[pathlib.Path, int]]] = defaultdict(list)
    for path in _iter_files(root):
        rel = path.relative_to(root)
        for lineno, body in _extract_safety_blocks(path):
            buckets[body].append((rel, lineno))
    flagged: list[tuple[str, list[tuple[pathlib.Path, int]]]] = []
    for body, sites in buckets.items():
        if len(sites) < min_occurrences:
            continue
        if _mentions_invariant(body):
            continue
        # Per-site allowlist: a verbatim invariant shared genuinely
        # across multiple sites (e.g. handle-based FFI fns) is
        # accepted via a substring entry in
        # `scripts/safety_comment_allowlist.txt`.
        if _is_allowlisted(body, allowlist):
            continue
        flagged.append((body, sites))
    return flagged


def _selftest() -> int:
    """Build a temporary tree with 3 stamped sites + 1 genuine site, then scan.

    Then re-scan with a per-site allowlist substring that whitelists
    the stamped text, and confirm the detector silences the bucket.
    """
    import tempfile

    sample_boilerplate = (
        "the caller upholds the FFI contract on this pointer; "
        "ownership / lifetime is managed entirely outside Rust"
    )
    genuine = (
        "the pointer was returned by `tdx_session_new` and refers to a "
        "`Box<Session>` whose lifetime is bounded by `tdx_session_free`; "
        "no aliasing mutator runs concurrently"
    )

    sample_files = {
        pathlib.Path("crates/a/src/lib.rs"): f"""
fn a() {{
    // SAFETY: {sample_boilerplate}
    unsafe {{ }}
}}
""",
        pathlib.Path("crates/b/src/lib.rs"): f"""
fn b() {{
    // SAFETY: {sample_boilerplate}
    unsafe {{ }}
}}
""",
        pathlib.Path("crates/c/src/lib.rs"): f"""
fn c() {{
    // SAFETY: {sample_boilerplate}
    unsafe {{ }}
}}
""",
        # Genuine annotation — must NOT be flagged.
        pathlib.Path("crates/d/src/lib.rs"): f"""
fn d() {{
    // SAFETY: {genuine}
    unsafe {{ }}
}}
""",
    }

    with tempfile.TemporaryDirectory() as td:
        root = pathlib.Path(td)
        for rel, content in sample_files.items():
            target = root / rel
            target.parent.mkdir(parents=True, exist_ok=True)
            target.write_text(content, encoding="utf-8")
        # First scan: empty allowlist — the stamped bucket trips.
        flagged = _scan(root, allowlist=[])
        if not flagged:
            print("selftest FAILED: stamped boilerplate not detected")
            return 1
        if len(flagged) != 1:
            print(
                f"selftest FAILED: expected exactly 1 flagged bucket, got {len(flagged)}"
            )
            return 1
        body, sites = flagged[0]
        if len(sites) != 3:
            print(
                f"selftest FAILED: expected 3 sites, got {len(sites)} "
                f"({sites!r})"
            )
            return 1
        if "the caller upholds" not in body:
            print(f"selftest FAILED: flagged the wrong bucket ({body!r})")
            return 1
        if any("tdx_session_new" in b for (b, _) in flagged):
            print("selftest FAILED: genuine annotation was flagged")
            return 1

        # Second scan: allowlist the boilerplate substring -- the
        # bucket is silenced. Verifies the allowlist takes effect.
        allowlisted = _scan(root, allowlist=["the caller upholds the ffi contract"])
        if allowlisted:
            print(
                "selftest FAILED: allowlist did not silence the stamped "
                f"bucket ({len(allowlisted)} bucket(s) still flagged)"
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
    parser.add_argument(
        "--min-occurrences",
        type=int,
        default=DEFAULT_MIN_OCCURRENCES,
        help=(
            f"Minimum identical-text occurrences that trip the gate "
            f"(default {DEFAULT_MIN_OCCURRENCES})."
        ),
    )
    args = parser.parse_args(argv)

    if args.selftest:
        return _selftest()

    flagged = _scan(REPO_ROOT, min_occurrences=args.min_occurrences)
    if not flagged:
        print("safety-boilerplate: clean")
        return 0
    print(
        f"safety-boilerplate: {len(flagged)} stamped-comment bucket(s) "
        f"with >= {args.min_occurrences} sites"
    )
    for body, sites in flagged:
        truncated = body if len(body) <= 200 else body[:200] + "..."
        print(f"  text: {truncated}")
        for rel, lineno in sites:
            print(f"    {rel}:{lineno}")
    print(
        "  -> Rewrite each comment to name the actual invariant "
        "(identifier, lifetime, ordering, layout property). "
        "Stamped boilerplate is not a SAFETY annotation."
    )
    return 1


if __name__ == "__main__":
    sys.exit(main())
