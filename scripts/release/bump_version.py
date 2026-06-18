#!/usr/bin/env python3
"""Bump every user-visible version pin in lockstep.

Usage:
    scripts/release/bump_version.py 8.0.30

Reads the canonical version from ``crates/thetadatadx/Cargo.toml`` (only
to print "from -> to" for context), then walks every file that pins a
version of the published artifact and rewrites it. All four npm
``package.json`` files, the seven member Cargo.toml files (thetadatadx +
ffi + tools/cli + tools/mcp + tools/server + sdks/python +
sdks/typescript), and the three ``optionalDependencies`` pins inside
``sdks/typescript/package.json``. Cargo.lock files are refreshed via
``cargo update --workspace`` against every manifest that carries its own
lockfile.

After the bump, ``scripts/ci/check_version_sync.py`` runs to verify nothing
got missed. Exits non-zero if anything is out of sync.

This is the only supported way to bump the SDK version. Doing it by
hand reliably misses ``sdks/typescript/`` files (lesson from npm being
stuck at v8.0.26 across v8.0.27 / v8.0.28 / v8.0.29 releases).
"""

from __future__ import annotations

import json
import re
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent.parent

WORKSPACE_CARGOS = [
    ROOT / "crates" / "thetadatadx" / "Cargo.toml",
    ROOT / "ffi" / "Cargo.toml",
    ROOT / "tools" / "cli" / "Cargo.toml",
    ROOT / "tools" / "mcp" / "Cargo.toml",
    ROOT / "tools" / "server" / "Cargo.toml",
    ROOT / "sdks" / "python" / "Cargo.toml",
    ROOT / "sdks" / "typescript" / "Cargo.toml",
]

SUB_LOCK_MANIFESTS = [
    ROOT / "tools" / "mcp" / "Cargo.toml",
    ROOT / "tools" / "server" / "Cargo.toml",
    ROOT / "sdks" / "python" / "Cargo.toml",
    ROOT / "sdks" / "typescript" / "Cargo.toml",
]


def parse_semver(value: str) -> tuple[int, int, int]:
    # Accept an optional SemVer pre-release / build suffix (e.g.
    # `13.0.0-rc.1`) so a release candidate can be bumped with the same
    # tool. The numeric core is returned; the suffix rides through on the
    # version strings the bump writes. Cargo and npm take `-rc.1` as-is;
    # maturin normalises it to the PEP 440 form, and CMake (operator-set)
    # carries the numeric core.
    match = re.fullmatch(r"(\d+)\.(\d+)\.(\d+)(?:[-+][0-9A-Za-z.\-]+)?", value)
    if not match:
        sys.exit(f"not a semver MAJOR.MINOR.PATCH[-prerelease]: '{value}'")
    return int(match.group(1)), int(match.group(2)), int(match.group(3))


def current_canonical() -> str:
    text = (ROOT / "crates" / "thetadatadx" / "Cargo.toml").read_text()
    match = re.search(r'^version\s*=\s*"([^"]+)"', text, re.MULTILINE)
    if not match:
        sys.exit("could not parse current version from crates/thetadatadx/Cargo.toml")
    return match.group(1)


def bump_cargo(path: Path, current: str, target: str) -> None:
    text = path.read_text()
    pattern = re.compile(rf'^version\s*=\s*"{re.escape(current)}"', re.MULTILINE)
    new_text, count = pattern.subn(f'version = "{target}"', text, count=1)
    if count != 1:
        sys.exit(
            f"{path.relative_to(ROOT)}: expected one `version = \"{current}\"` line, "
            f"found {count}"
        )
    path.write_text(new_text)


def bump_root_package_json(path: Path, current: str, target: str) -> None:
    data = json.loads(path.read_text())
    if data.get("version") != current:
        sys.exit(
            f"{path.relative_to(ROOT)}: version is {data.get('version')!r}, "
            f"expected {current!r}"
        )
    data["version"] = target
    deps = data.get("optionalDependencies", {})
    for name, pinned in list(deps.items()):
        if pinned == current:
            deps[name] = target
        else:
            sys.exit(
                f"{path.relative_to(ROOT)} optionalDependencies['{name}'] is "
                f"{pinned!r}, expected {current!r}"
            )
    path.write_text(json.dumps(data, indent=2, ensure_ascii=False) + "\n")


def bump_platform_package_json(path: Path, current: str, target: str) -> None:
    data = json.loads(path.read_text())
    if data.get("version") != current:
        sys.exit(
            f"{path.relative_to(ROOT)}: version is {data.get('version')!r}, "
            f"expected {current!r}"
        )
    data["version"] = target
    path.write_text(json.dumps(data, indent=2, ensure_ascii=False) + "\n")


def cargo_update(manifest: Path) -> None:
    subprocess.run(
        ["cargo", "update", "--manifest-path", str(manifest), "--workspace"],
        cwd=ROOT,
        check=True,
    )


def main(argv: list[str]) -> int:
    if len(argv) != 2:
        sys.exit("usage: scripts/release/bump_version.py <new-version>")
    target = argv[1]
    parse_semver(target)
    current = current_canonical()
    if current == target:
        sys.exit(f"current version is already {current}; nothing to bump")
    print(f"bumping {current} -> {target}")

    for cargo in WORKSPACE_CARGOS:
        bump_cargo(cargo, current, target)
        print(f"  bumped {cargo.relative_to(ROOT)}")

    bump_root_package_json(
        ROOT / "sdks" / "typescript" / "package.json", current, target
    )
    print("  bumped sdks/typescript/package.json (+ optionalDependencies)")

    for platform_pkg in (ROOT / "sdks" / "typescript" / "npm").glob("*/package.json"):
        bump_platform_package_json(platform_pkg, current, target)
        print(f"  bumped {platform_pkg.relative_to(ROOT)}")

    print("refreshing Cargo.lock files ...")
    cargo_update(WORKSPACE_CARGOS[0])
    for manifest in SUB_LOCK_MANIFESTS:
        cargo_update(manifest)

    print("verifying with scripts/ci/check_version_sync.py ...")
    subprocess.run(
        [sys.executable, str(ROOT / "scripts" / "ci" / "check_version_sync.py")],
        cwd=ROOT,
        check=True,
    )
    print(f"version bump to {target} complete; review `git diff` before committing")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
