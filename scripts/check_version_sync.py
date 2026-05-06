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


def main() -> int:
    canonical = cargo_version(CANONICAL_CARGO)
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

    if failures:
        print("version-sync errors:")
        for f in failures:
            print(f"  - {f}")
        return 1

    print("version sync: ok")
    return 0


if __name__ == "__main__":
    sys.exit(main())
