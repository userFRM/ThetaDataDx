#!/usr/bin/env python3
"""Verify that user-visible package metadata stays in lockstep with Cargo.toml.

The TypeScript SDK ships through npm and pins its version in
``sdks/typescript/package.json`` plus three per-platform packages under
``sdks/typescript/npm/`` plus three ``optionalDependencies`` entries.
The Rust workspace bumps its Cargo.toml independently. When any of those
fall out of sync (which happened across v8.0.27 / v8.0.28 / v8.0.29 and
left npm stuck on v8.0.26 because the publish workflow keys off
``package.json`` rather than ``Cargo.toml``), the npm package silently
ages while git tags advance.

This script fails CI when any tracked version disagrees with the
canonical ``crates/thetadatadx/Cargo.toml`` version.
"""

from __future__ import annotations

import json
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
CANONICAL_CARGO = ROOT / "crates" / "thetadatadx" / "Cargo.toml"
CMAKE_LISTS = ROOT / "sdks" / "cpp" / "CMakeLists.txt"
PY_INIT = ROOT / "sdks" / "python" / "python" / "thetadatadx" / "__init__.py"

# Accepted values for the `__version__` fallback in the Python SDK
# `__init__.py`. `"unknown"` is the canonical sentinel — anything that
# happens to look like a semver literal (e.g. `"10.0.0"`) drifts
# silently whenever `Cargo.toml` advances, so the gate warns the
# operator immediately.
PY_FALLBACK_OK = ("unknown",)
PY_FALLBACK_RE = re.compile(r'__version__\s*=\s*"([^"]+)"')


def cargo_version(path: Path) -> str:
    text = path.read_text()
    match = re.search(r'^version\s*=\s*"([^"]+)"', text, re.MULTILINE)
    if not match:
        sys.exit(f"could not parse `version` from {path}")
    return match.group(1)


def package_json_version(path: Path) -> str:
    return json.loads(path.read_text())["version"]


def package_json_optional_deps(path: Path) -> dict[str, str]:
    return json.loads(path.read_text()).get("optionalDependencies", {})


def cmake_project_version(path: Path) -> str | None:
    """Extract the `project(... VERSION x.y.z ...)` value from a
    CMakeLists.txt. U4 closure: the version-sync check previously
    returned clean even when `sdks/cpp/CMakeLists.txt` still pinned
    `VERSION 8.0.23` against a Cargo-side v10. Match the `VERSION
    x.y.z` substring inside the `project(...)` call so a future CMake
    drift fails this gate.
    """
    if not path.is_file():
        return None
    text = path.read_text()
    match = re.search(
        r"project\s*\(\s*[\w\-]+\s+VERSION\s+(\d+\.\d+\.\d+)",
        text,
        re.IGNORECASE,
    )
    if not match:
        return None
    return match.group(1)


# U5 closure: the same gate scans documentation pins for the
# `thetadatadx = "<major>"` shape. Drift between the canonical Cargo
# version and a doc pin (README.md, docs-site quickstart /
# installation) silently aged the docs across v9 → v10 — this gate
# now catches that case.
DOC_PIN_PATHS = (
    ROOT / "README.md",
    ROOT / "docs-site" / "docs" / "articles" / "getting-started.md",
    ROOT / "docs-site" / "docs" / "examples" / "dataframes.md",
)
# Match `thetadatadx = "<MAJOR>"` (Cargo.toml-ish pin) and
# `thetadatadx = { version = "<MAJOR>", ... }` (Cargo.toml feature
# pin). Both shapes have a single major-version-only literal that
# must match the canonical major.
DOC_PIN_RE = re.compile(
    r'thetadatadx\s*=\s*(?:"(\d+)(?:[.,)\'" ]|$)|\{\s*version\s*=\s*"(\d+)(?:[.,)\'" ]|$))'
)


def python_init_fallback_mismatches(canonical: str) -> list[str]:
    """Warn if the Python SDK `__version__` fallback drifted away from
    the `"unknown"` sentinel into a stale numeric literal.

    A literal fallback like `"10.0.0"` silently lies whenever the
    canonical `Cargo.toml` version advances — operators inspecting
    `thetadatadx.__version__` on a source-tree import see a stale
    number instead of an obvious "I cannot determine the version"
    signal. The accepted value is the sentinel; anything else either
    matches `canonical` (acceptable but fragile) or is stale.
    """
    if not PY_INIT.is_file():
        return []
    issues: list[str] = []
    for lineno, line in enumerate(PY_INIT.read_text().splitlines(), start=1):
        match = PY_FALLBACK_RE.search(line)
        if not match:
            continue
        literal = match.group(1)
        if literal in PY_FALLBACK_OK:
            continue
        if literal == canonical:
            # Matches today, but will drift on the next bump — warn.
            issues.append(
                f"{PY_INIT.relative_to(ROOT)}:{lineno} `__version__` "
                f'fallback is the numeric literal "{literal}" (matches '
                "canonical today but will drift on next version bump — "
                'prefer the "unknown" sentinel)'
            )
            continue
        issues.append(
            f"{PY_INIT.relative_to(ROOT)}:{lineno} `__version__` "
            f'fallback is "{literal}", expected the "unknown" sentinel '
            f"(canonical is {canonical} — a numeric literal here drifts "
            "silently)"
        )
    return issues


def doc_pin_mismatches(canonical_major: str) -> list[str]:
    issues: list[str] = []
    for path in DOC_PIN_PATHS:
        if not path.is_file():
            continue
        for lineno, line in enumerate(path.read_text().splitlines(), start=1):
            match = DOC_PIN_RE.search(line)
            if not match:
                continue
            pinned = match.group(1) or match.group(2)
            if pinned != canonical_major:
                issues.append(
                    f"{path.relative_to(ROOT)}:{lineno} pins "
                    f'`thetadatadx = "{pinned}"`, expected major {canonical_major}'
                )
    return issues


def main() -> int:
    canonical = cargo_version(CANONICAL_CARGO)
    canonical_major = canonical.split(".", 1)[0]
    print(f"canonical version (Cargo.toml): {canonical}")

    failures: list[str] = []

    ts_root = ROOT / "sdks" / "typescript" / "package.json"
    if package_json_version(ts_root) != canonical:
        failures.append(
            f"{ts_root.relative_to(ROOT)} version is "
            f"{package_json_version(ts_root)}, expected {canonical}"
        )

    for name, pinned in package_json_optional_deps(ts_root).items():
        if pinned != canonical:
            failures.append(
                f"{ts_root.relative_to(ROOT)} optionalDependencies['{name}'] "
                f"is {pinned}, expected {canonical}"
            )

    for platform_pkg in (ts_root.parent / "npm").glob("*/package.json"):
        if package_json_version(platform_pkg) != canonical:
            failures.append(
                f"{platform_pkg.relative_to(ROOT)} version is "
                f"{package_json_version(platform_pkg)}, expected {canonical}"
            )

    # CMake project version must match the canonical crates Cargo
    # version. CMake's `project(... VERSION ...)` only accepts a numeric
    # `major.minor.patch`, so on a pre-release (e.g. `13.0.0-rc.1`) it
    # carries the numeric base (`13.0.0`); compare against that base, not
    # the full pre-release string. A normal release has no suffix, so the
    # base equals the canonical version.
    canonical_base = canonical.split("-", 1)[0]
    cmake_version = cmake_project_version(CMAKE_LISTS)
    if cmake_version is None:
        failures.append(
            f"{CMAKE_LISTS.relative_to(ROOT)}: could not parse "
            "`project(... VERSION ...)` value"
        )
    elif cmake_version != canonical_base:
        failures.append(
            f"{CMAKE_LISTS.relative_to(ROOT)} VERSION is "
            f"{cmake_version}, expected {canonical_base}"
        )

    # U5 closure: documentation pins must match the canonical major.
    failures.extend(doc_pin_mismatches(canonical_major))

    # M4 closure (post-#572 audit): Python SDK `__version__` fallback
    # must be the `"unknown"` sentinel, not a numeric literal that
    # drifts silently.
    failures.extend(python_init_fallback_mismatches(canonical))

    if failures:
        print("version-sync errors:")
        for f in failures:
            print(f"  - {f}")
        return 1

    print("version sync: ok")
    return 0


if __name__ == "__main__":
    sys.exit(main())
