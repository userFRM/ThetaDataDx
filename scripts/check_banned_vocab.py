#!/usr/bin/env python3
"""Banned-vocabulary scrubber (Gate 11 / issue #554).

Scans source files, doc strings, code comments, recent commit subjects,
and (when run inside a CI PR) the current PR title + body for marketing
or internal-process jargon that should never reach a public artifact.

Hits return a non-zero exit code. False positives can be silenced inline
with a ``VOCAB-OK: <one-line reason>`` annotation on the same line as
the offending phrase, or by listing the file under ``EXEMPT_PATHS``
below — historical changelogs and release notes are append-only ledgers
of past releases and rewriting them would falsify the record.

Run from the repo root::

    python3 scripts/check_banned_vocab.py
"""

from __future__ import annotations

import os
import pathlib
import re
import subprocess
import sys
from typing import Iterable


REPO_ROOT = pathlib.Path(__file__).resolve().parents[1]


# Phrases that have to disappear from any newly-authored public surface.
# Matched case-insensitively as whole tokens; multi-word phrases are
# matched as-is (whitespace-collapsed).
BANNED = [
    "institutional",
    "bulletproof",
    "enterprise-grade",
    "enterprise grade",
    "audit fixes",
    "NICE-TO-HAVE",
    "nice to have",
    "production-ready",
    "production ready",
    "world-class",
    "world class",
    "next-generation",
    "next generation",
    "minimum vs complete",
]


# File globs walked relative to repo root.
SCAN_GLOBS = [
    "crates/**/*.rs",
    "crates/**/*.toml",
    "ffi/**/*.rs",
    "ffi/**/*.toml",
    "sdks/**/*.rs",
    "sdks/**/*.py",
    "sdks/**/*.pyi",
    "sdks/**/*.ts",
    "sdks/**/*.js",
    "sdks/**/*.hpp",
    "sdks/**/*.cpp",
    "sdks/**/*.h",
    "sdks/**/*.inc",
    "sdks/**/*.toml",
    "sdks/**/*.md",
    "tools/**/*.rs",
    "tools/**/*.toml",
    "docs/**/*.md",
    "scripts/**/*.py",
    ".github/**/*.yml",
    "README.md",
    "CONTRIBUTING.md",
    "SECURITY.md",
]


# Paths skipped by the file walk. CHANGELOG.md and the docs-site
# changelog mirror are append-only ledgers of historical releases;
# rewriting them would falsify the public record. The gate still
# blocks new content elsewhere from referencing the same phrases.
EXEMPT_PATHS = {
    "CHANGELOG.md",
    "docs-site/docs/changelog.md",
    "scripts/check_banned_vocab.py",
    "scripts/__pycache__",
    ".github/release-notes",
}


# Directory name fragments anywhere in the path that mark vendored or
# generated content the gate must not police: third-party wheels in a
# local venv, package-manager caches, build outputs, etc.
EXEMPT_PATH_FRAGMENTS = (
    "/.venv/",
    "/.venv-test/",
    "/venv/",
    "/node_modules/",
    "/target/",
    "/__pycache__/",
    "/.git/",
)


# Compile once. Word-boundary check on alphanumeric tokens; multi-word
# phrases match as a literal collapsed-whitespace sequence.
def _compile_patterns() -> list[tuple[str, re.Pattern[str]]]:
    out: list[tuple[str, re.Pattern[str]]] = []
    for phrase in BANNED:
        if " " in phrase or "-" in phrase:
            # Multi-token: tolerate one-or-more whitespace / hyphen runs
            # between tokens so "enterprise grade" and "enterprise-grade"
            # both flag the same rule.
            tokens = re.split(r"[\s-]+", phrase)
            pattern = r"\b" + r"[\s-]+".join(re.escape(t) for t in tokens) + r"\b"
        else:
            pattern = r"\b" + re.escape(phrase) + r"\b"
        out.append((phrase, re.compile(pattern, re.IGNORECASE)))
    return out


PATTERNS = _compile_patterns()
ALLOW_RE = re.compile(r"VOCAB-OK\s*:")


def _is_exempt(rel_path: pathlib.Path) -> bool:
    parts = rel_path.as_posix()
    for exempt in EXEMPT_PATHS:
        if parts == exempt or parts.startswith(exempt + "/"):
            return True
    fragment_target = "/" + parts + "/"
    for fragment in EXEMPT_PATH_FRAGMENTS:
        if fragment in fragment_target:
            return True
    return False


def _iter_files() -> Iterable[pathlib.Path]:
    seen: set[pathlib.Path] = set()
    for pattern in SCAN_GLOBS:
        for candidate in REPO_ROOT.glob(pattern):
            if not candidate.is_file():
                continue
            rel = candidate.relative_to(REPO_ROOT)
            if _is_exempt(rel):
                continue
            if rel in seen:
                continue
            seen.add(rel)
            yield candidate


def _scan_file(path: pathlib.Path) -> list[tuple[int, str, str]]:
    hits: list[tuple[int, str, str]] = []
    try:
        text = path.read_text(encoding="utf-8")
    except (OSError, UnicodeDecodeError):
        return hits
    for lineno, line in enumerate(text.splitlines(), start=1):
        if ALLOW_RE.search(line):
            continue
        for phrase, regex in PATTERNS:
            if regex.search(line):
                hits.append((lineno, phrase, line.rstrip()))
                break
    return hits


def _scan_commit_subjects() -> list[tuple[str, str, str]]:
    """Inspect commit subjects unique to the current branch.

    Limits the scan to ``origin/main..HEAD`` so already-merged history
    on ``main`` stays immutable — squashing or rewording landed commits
    would falsify the public git record. If the comparison ref cannot
    be resolved (shallow clone, first-commit scenarios, no remote) we
    fall back to the last 50 commits on ``HEAD``.
    """
    spec = "origin/main..HEAD"
    rev_check = subprocess.run(
        ["git", "rev-parse", "--verify", "origin/main"],
        cwd=REPO_ROOT,
        capture_output=True,
        text=True,
        check=False,
    )
    if rev_check.returncode != 0:
        spec = "-50"
    try:
        out = subprocess.check_output(
            ["git", "log", spec, "--pretty=%H%x09%s"],
            cwd=REPO_ROOT,
            text=True,
        )
    except (subprocess.CalledProcessError, FileNotFoundError):
        return []
    hits: list[tuple[str, str, str]] = []
    for line in out.splitlines():
        if "\t" not in line:
            continue
        sha, subject = line.split("\t", 1)
        for phrase, regex in PATTERNS:
            if regex.search(subject):
                hits.append((sha[:12], phrase, subject))
                break
    return hits


def _scan_pr_metadata() -> list[tuple[str, str, str]]:
    """Inspect the current PR title + body, if available via GitHub Actions."""
    pr_title = os.environ.get("PR_TITLE", "")
    pr_body = os.environ.get("PR_BODY", "")
    hits: list[tuple[str, str, str]] = []
    if pr_title:
        for phrase, regex in PATTERNS:
            if regex.search(pr_title):
                hits.append(("PR title", phrase, pr_title))
                break
    if pr_body:
        for line in pr_body.splitlines():
            if ALLOW_RE.search(line):
                continue
            for phrase, regex in PATTERNS:
                if regex.search(line):
                    hits.append(("PR body", phrase, line))
                    break
    return hits


def main() -> int:
    total = 0

    file_hits: list[tuple[pathlib.Path, int, str, str]] = []
    for path in _iter_files():
        for lineno, phrase, line in _scan_file(path):
            file_hits.append((path.relative_to(REPO_ROOT), lineno, phrase, line))

    if file_hits:
        total += len(file_hits)
        print(f"banned-vocab: {len(file_hits)} hit(s) in tracked files")
        for rel, lineno, phrase, line in file_hits:
            print(f"  {rel}:{lineno} [{phrase}] {line[:160]}")

    commit_hits = _scan_commit_subjects()
    if commit_hits:
        total += len(commit_hits)
        print(f"banned-vocab: {len(commit_hits)} hit(s) in last 50 commit subjects")
        for sha, phrase, subject in commit_hits:
            print(f"  {sha} [{phrase}] {subject[:160]}")

    pr_hits = _scan_pr_metadata()
    if pr_hits:
        total += len(pr_hits)
        print(f"banned-vocab: {len(pr_hits)} hit(s) in PR metadata")
        for where, phrase, line in pr_hits:
            print(f"  {where} [{phrase}] {line[:160]}")

    if total:
        print(
            "\nFix: either rephrase, or add `VOCAB-OK: <reason>` on the "
            "same line for genuinely-legitimate uses (e.g. quoting a "
            "third-party standard name)."
        )
        return 1

    print("banned-vocab: clean")
    return 0


if __name__ == "__main__":
    sys.exit(main())
