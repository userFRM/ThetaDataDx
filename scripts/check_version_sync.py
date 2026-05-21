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
    ROOT / "docs-site" / "docs" / "getting-started" / "installation.md",
    ROOT / "docs-site" / "docs" / "getting-started" / "quickstart.md",
)
# Match `thetadatadx = "<MAJOR>"` (Cargo.toml-ish pin) and
# `thetadatadx = { version = "<MAJOR>", ... }` (Cargo.toml feature
# pin). Both shapes have a single major-version-only literal that
# must match the canonical major.
DOC_PIN_RE = re.compile(
    r'thetadatadx\s*=\s*(?:"(\d+)(?:[.,)\'" ]|$)|\{\s*version\s*=\s*"(\d+)(?:[.,)\'" ]|$))'
)


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

    # U4 closure: CMake project version must match the canonical
    # crates Cargo version exactly.
    cmake_version = cmake_project_version(CMAKE_LISTS)
    if cmake_version is None:
        failures.append(
            f"{CMAKE_LISTS.relative_to(ROOT)}: could not parse "
            "`project(... VERSION ...)` value"
        )
    elif cmake_version != canonical:
        failures.append(
            f"{CMAKE_LISTS.relative_to(ROOT)} VERSION is "
            f"{cmake_version}, expected {canonical}"
        )

    # U5 closure: documentation pins must match the canonical major.
    failures.extend(doc_pin_mismatches(canonical_major))

    if failures:
        print("version-sync errors:")
        for f in failures:
            print(f"  - {f}")
        return 1

    print("version sync: ok")
    return 0


if __name__ == "__main__":
    sys.exit(main())
