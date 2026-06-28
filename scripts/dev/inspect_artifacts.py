#!/usr/bin/env python3
"""Artifact contents inspection (Gate 12 / issue #555).

Verifies that built wheels, npm tarballs, and cmake-installed cpp
trees contain exactly the files we expect — no accidental inclusion
of `.env`, `creds.txt`, `target/`, `node_modules/`, `__pycache__/`,
`.venv*/`, or internal docs. Wheels and npm tarballs are public
distribution channels; one slipped credential is permanent.

Usage::

    # auto-discover artifacts in dist/ + thetadatadx-ts/
    python3 scripts/dev/inspect_artifacts.py

    # inspect a specific wheel / tarball / directory
    python3 scripts/dev/inspect_artifacts.py dist/thetadatadx-10.0.0-py3-none-any.whl
    python3 scripts/dev/inspect_artifacts.py thetadatadx-ts/thetadatadx-10.0.0.tgz
"""

from __future__ import annotations

import argparse
import fnmatch
import pathlib
import sys
import tarfile
import zipfile
from typing import Iterable


REPO_ROOT = pathlib.Path(__file__).resolve().parents[2]


# Patterns that absolutely must not appear in a distributed artifact.
# Globs run against the path-inside-archive (forward slashes).
FORBIDDEN_GLOBS = (
    ".env",
    "*/.env",
    # `creds.txt` (the local dev credentials file we tell users to keep
    # outside the repo) and its siblings. Scoped to the `.txt`,
    # `.json`, `.yaml`, `.yml`, `.toml` extensions so the pattern does
    # not falsely flag well-named Rust source like
    # `thetadatadx-rs/src/auth/creds.rs` which is shipped on
    # purpose inside the published sdist.
    "creds*.txt",
    "creds*.json",
    "creds*.yaml",
    "creds*.yml",
    "creds*.toml",
    "*/creds*.txt",
    "*/creds*.json",
    "*/creds*.yaml",
    "*/creds*.yml",
    "*/creds*.toml",
    "credentials*.txt",
    "credentials*.json",
    "*/credentials*.txt",
    "*/credentials*.json",
    "*/.venv/*",
    "*/.venv-test/*",
    "*/.venv-pr*/*",
    "*/__pycache__/*",
    "*.pyc",
    "*/target/*",
    "*/node_modules/*",
    "*/.git/*",
    "*/private/*",
    "*/todo.md",
    "*/.DS_Store",
    "*/*.key",
    "*/*.pem",
)


def _archive_members(path: pathlib.Path) -> list[str]:
    """List filenames inside a wheel (.whl) or npm tarball (.tgz / .tar.gz)."""
    suffix = path.suffix.lower()
    if suffix == ".whl" or suffix == ".zip":
        with zipfile.ZipFile(path) as zf:
            return [info.filename for info in zf.infolist() if not info.is_dir()]
    if suffix in (".tgz", ".gz") or path.name.endswith(".tar.gz") or path.name.endswith(".tar"):
        with tarfile.open(path) as tf:
            return [member.name for member in tf.getmembers() if member.isfile()]
    raise ValueError(f"unrecognised archive format: {path}")


def _dir_members(path: pathlib.Path) -> list[str]:
    """List all regular files under a directory (cmake install tree, etc.)."""
    out: list[str] = []
    for entry in path.rglob("*"):
        if entry.is_file():
            out.append(entry.relative_to(path).as_posix())
    return out


def _check_members(members: Iterable[str]) -> list[tuple[str, str]]:
    hits: list[tuple[str, str]] = []
    for name in members:
        for pat in FORBIDDEN_GLOBS:
            if fnmatch.fnmatch(name, pat):
                hits.append((name, pat))
                break
    return hits


def _discover() -> list[pathlib.Path]:
    candidates: list[pathlib.Path] = []
    for d in (
        REPO_ROOT / "dist",
        REPO_ROOT / "thetadatadx-py" / "dist",
        REPO_ROOT / "thetadatadx-ts",
    ):
        if not d.is_dir():
            continue
        for f in d.iterdir():
            if not f.is_file():
                continue
            name = f.name.lower()
            if name.endswith(".whl") or name.endswith(".tgz") or name.endswith(".tar.gz"):
                candidates.append(f)
    return candidates


def inspect(target: pathlib.Path) -> int:
    print(f"inspecting {target}")
    if target.is_dir():
        members = _dir_members(target)
    else:
        try:
            members = _archive_members(target)
        except ValueError as exc:
            print(f"  ERROR: {exc}")
            return 1
    if not members:
        print(f"  WARN: {target} contained zero file members")
        return 0
    hits = _check_members(members)
    if hits:
        print(f"  forbidden content in {target}:")
        for name, pat in hits:
            print(f"    {name} (matched {pat})")
        return 1
    print(f"  ok: {len(members)} file(s) scanned, no forbidden content")
    return 0


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "targets",
        nargs="*",
        help="wheels / tarballs / directories to inspect. Empty = auto-discover.",
    )
    args = parser.parse_args()

    targets: list[pathlib.Path]
    if args.targets:
        targets = [pathlib.Path(t).resolve() for t in args.targets]
    else:
        targets = _discover()

    if not targets:
        print("inspect_artifacts: no candidates found; nothing to scan", file=sys.stderr)
        return 0

    bad = 0
    for t in targets:
        rc = inspect(t)
        if rc != 0:
            bad += 1

    return 1 if bad else 0


if __name__ == "__main__":
    sys.exit(main())
