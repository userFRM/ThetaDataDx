#!/usr/bin/env python3
"""Move every version pin of the release in lockstep, or check that they agree.

Usage:
    scripts/release/bump_version.py 9.9.9     # rewrite every pin
    scripts/release/bump_version.py --check   # fail if any pin disagrees

The canonical version is `[workspace.package] version` in the root
`Cargo.toml`, which every member crate inherits. The other pins are the npm
packages (the TypeScript SDK launcher and its platform packages, the MCP server
launcher and its platform packages, the launchers' `optionalDependencies`), the
napi loader `thetadatadx-ts/index.js` and the TypeScript `package-lock.json`,
the C++ `project(... VERSION ...)`, the published OpenAPI `info.version`, and
every `thetadatadx-rs = "<version>"` pin in the Markdown docs.

A bump validates every pin before writing any, so it writes all of them or
none. `--check` runs the same validation against the canonical version and
writes nothing; CI runs it on every push. This is the only supported way to
bump the version: by hand, `thetadatadx-ts/` reliably gets missed, which once
left npm three releases behind.
"""

from __future__ import annotations

import json
import re
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent.parent

MEMBER_CARGOS = [
    ROOT / "thetadatadx-rs" / "Cargo.toml",
    ROOT / "thetadatadx-ffi" / "Cargo.toml",
    ROOT / "tools" / "mcp" / "Cargo.toml",
    ROOT / "tools" / "server" / "Cargo.toml",
    ROOT / "thetadatadx-py" / "Cargo.toml",
    ROOT / "thetadatadx-ts" / "Cargo.toml",
]

# `thetadatadx-rs = "<version>"` and `thetadatadx-rs = { version = "<version>", ... }`.
DOC_PIN_RE = re.compile(r'thetadatadx-rs\s*=\s*(?:"([^"]+)"|\{\s*version\s*=\s*"([^"]+)")')
# The changelogs, release notes and migration guides pin past versions by design.
_DOC_PIN_EXCLUDE_FRAGMENTS = ("/target/", "/node_modules/", "/.git/", "/.github/", "/migration/", "/worktrees/")
_DOC_PIN_EXCLUDE_NAMES = frozenset({"CHANGELOG.md", "changelog.md"})


class PinError(Exception):
    """A pin that does not hold the version the bump expects."""


def parse_semver(value: str) -> None:
    # A pre-release or build suffix (`9.9.9-rc.1`) rides through: Cargo and npm
    # take it as-is, maturin normalises it to PEP 440, and CMake gets the core.
    if not re.fullmatch(r"\d+\.\d+\.\d+(?:[-+][0-9A-Za-z.\-]+)?", value):
        sys.exit(f"not a semver MAJOR.MINOR.PATCH[-prerelease]: '{value}'")


def current_canonical() -> str:
    match = re.search(r'^version\s*=\s*"([^"]+)"', (ROOT / "Cargo.toml").read_text(), re.MULTILINE)
    if not match:
        sys.exit("could not parse `[workspace.package] version` from Cargo.toml")
    return match.group(1)


def bump_cargo(text: str, current: str, target: str) -> str:
    new_text, count = re.subn(
        rf'^version\s*=\s*"{re.escape(current)}"', f'version = "{target}"', text, count=1, flags=re.MULTILINE
    )
    if count != 1:
        raise PinError(f'expected one `version = "{current}"` line')
    return new_text


def require_inherited_version(text: str, current: str, target: str) -> str:
    """A member that pins its own version leaves the lockstep silently."""
    if not re.search(r"^version\.workspace\s*=\s*true\s*$", text, re.MULTILINE):
        raise PinError("does not inherit `version.workspace = true`")
    return text


def _platform_names(npm_dir: Path, launcher: str | None = None) -> set[str]:
    return {
        json.loads(pkg.read_text())["name"]
        for pkg in npm_dir.glob("*/package.json")
        if pkg.parent.name != launcher
    }


def bump_launcher_package_json(platforms: set[str]):
    """The launcher's version and its `optionalDependencies` pins, which must
    name every platform package the repository publishes: a missing one never
    installs the native binary on that platform."""

    def bump(text: str, current: str, target: str) -> str:
        data = json.loads(text)
        if data.get("version") != current:
            raise PinError(f"version is {data.get('version')!r}, expected {current!r}")
        data["version"] = target
        deps = data.get("optionalDependencies", {})
        if not platforms or set(deps) != platforms:
            raise PinError(f"optionalDependencies name {sorted(deps)}, the platform packages are {sorted(platforms)}")
        for name, pinned in deps.items():
            if pinned != current:
                raise PinError(f"optionalDependencies['{name}'] is {pinned!r}, expected {current!r}")
            deps[name] = target
        return json.dumps(data, indent=2, ensure_ascii=False) + "\n"

    return bump


def bump_platform_package_json(text: str, current: str, target: str) -> str:
    data = json.loads(text)
    if data.get("version") != current:
        raise PinError(f"version is {data.get('version')!r}, expected {current!r}")
    data["version"] = target
    return json.dumps(data, indent=2, ensure_ascii=False) + "\n"


def bump_napi_index_js(text: str, current: str, target: str) -> str:
    """The version napi bakes into every native-binding load path of the
    generated, published `index.js`. Left behind, the loader rejects its own
    binary once NAPI_RS_ENFORCE_VERSION_CHECK is set. Every semantic version in
    the file is that one pin, so a textual replace is what regenerating gives."""
    # The suffix is part of the version: a pattern that stopped at the patch
    # digit would read a loader still pinned to `0.5.1-rc.1` as `0.5.1`.
    found = set(re.findall(r"\b\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?", text))
    if found != {current}:
        raise PinError(f"pins {sorted(found)}, expected only {current!r}; regenerate it with `npm run build`")
    return text.replace(current, target)


def bump_package_lock(first_party_prefix: str):
    """This package's version, and its own platform packages, in a lockfile.

    The launcher pins the platform packages through `optionalDependencies`, so
    a lockfile left behind makes `npm ci` install the previous release's
    binaries under the new launcher. Third-party packages at the same version
    are left alone.
    """

    def bump(text: str, current: str, target: str) -> str:
        data = json.loads(text)
        root_pkg = data.get("packages", {}).get("", {})
        for where, holder in (("version", data), ('packages[""].version', root_pkg)):
            if holder.get("version") != current:
                raise PinError(f"{where} is {holder.get('version')!r}, expected {current!r}")
            holder["version"] = target

        moved = 0
        for dep, pinned in list(root_pkg.get("optionalDependencies", {}).items()):
            if dep.startswith(first_party_prefix):
                if pinned != current:
                    raise PinError(f'packages[""].optionalDependencies[{dep!r}] is {pinned!r}, expected {current!r}')
                root_pkg["optionalDependencies"][dep] = target
        for name, entry in data.get("packages", {}).items():
            if not name.startswith("node_modules/") or not name.rsplit("/", 1)[-1].startswith(first_party_prefix):
                continue
            if entry.get("version") != current:
                raise PinError(f"{name} is {entry.get('version')!r}, expected {current!r}")
            resolved = entry.get("resolved")
            if resolved and f"-{current}.tgz" not in resolved:
                raise PinError(f"{name} is {current} but resolves {resolved.rsplit('/', 1)[-1]}")
            entry["version"] = target
            if target != current:
                # The new release's tarballs do not exist until it publishes;
                # carrying these over labels the old artifact with the new
                # version. npm resolves the entry again on the next install.
                entry.pop("resolved", None)
                entry.pop("integrity", None)
            moved += 1
        if moved == 0:
            raise PinError(f"no `node_modules/{first_party_prefix}*` entry")
        return json.dumps(data, indent=2, ensure_ascii=False) + "\n"

    return bump


def bump_cmake(text: str, current: str, target: str) -> str:
    """CMake rejects anything but a numeric version, so the pin carries only
    the numeric core and a pre-release suffix is dropped on both sides."""
    needle = f"VERSION {re.split(r'[-+]', current, maxsplit=1)[0]}"
    if text.count(needle) != 1:
        raise PinError(f"expected exactly one {needle!r}, found {text.count(needle)}")
    return text.replace(needle, f"VERSION {re.split(r'[-+]', target, maxsplit=1)[0]}")


def bump_openapi_yaml(text: str, current: str, target: str) -> str:
    """`info.version`, the only unindented-block `version:` the spec carries."""
    needle = f"\n  version: {current}\n"
    if text.count(needle) != 1:
        raise PinError(f"expected exactly one info `version: {current}`")
    return text.replace(needle, f"\n  version: {target}\n")


def bump_doc_pins(text: str, current: str, target: str) -> str:
    stale = sorted({m.group(1) or m.group(2) for m in DOC_PIN_RE.finditer(text)} - {current})
    if stale:
        raise PinError(f"pins thetadatadx-rs at {stale}, expected {current!r}")
    return DOC_PIN_RE.sub(lambda m: m.group(0).replace(f'"{current}"', f'"{target}"'), text)


def _doc_pin_files() -> list[Path]:
    """Every Markdown file that should carry the current pin, discovered so a
    new install snippet is covered the moment it lands."""
    return sorted(
        path
        for path in ROOT.rglob("*.md")
        if not any(frag in "/" + path.relative_to(ROOT).as_posix() for frag in _DOC_PIN_EXCLUDE_FRAGMENTS)
        and path.name not in _DOC_PIN_EXCLUDE_NAMES
    )


def pins() -> list[tuple[Path, object]]:
    ts = ROOT / "thetadatadx-ts"
    mcp_npm = ROOT / "tools" / "mcp" / "npm"
    mcp_launcher = "thetadatadx-mcp-server"
    out: list[tuple[Path, object]] = [(ROOT / "Cargo.toml", bump_cargo)]
    out += [(cargo, require_inherited_version) for cargo in MEMBER_CARGOS]
    out.append((ts / "package.json", bump_launcher_package_json(_platform_names(ts / "npm"))))
    out += [(pkg, bump_platform_package_json) for pkg in sorted((ts / "npm").glob("*/package.json"))]
    out.append((ts / "index.js", bump_napi_index_js))
    out.append((ts / "package-lock.json", bump_package_lock("thetadatadx-ts")))
    # The MCP server ships to npm too (`npx -y thetadatadx-mcp-server`), laid
    # out like the TypeScript SDK.
    out.append(
        (mcp_npm / mcp_launcher / "package.json", bump_launcher_package_json(_platform_names(mcp_npm, mcp_launcher)))
    )
    out += [
        (pkg, bump_platform_package_json)
        for pkg in sorted(mcp_npm.glob("*/package.json"))
        if pkg.parent.name != mcp_launcher
    ]
    out.append((ROOT / "thetadatadx-cpp" / "CMakeLists.txt", bump_cmake))
    out.append((ROOT / "docs-site" / "docs" / "public" / "thetadatadx.yaml", bump_openapi_yaml))
    out += [(doc, bump_doc_pins) for doc in _doc_pin_files()]
    return out


def rewrite_all(current: str, target: str) -> tuple[dict[Path, str], list[str]]:
    rewritten: dict[Path, str] = {}
    errors: list[str] = []
    for path, bump in pins():
        try:
            rewritten[path] = bump(path.read_text(), current, target)
        except (PinError, OSError) as exc:
            errors.append(f"{path.relative_to(ROOT)}: {exc}")
    return rewritten, errors


def main(argv: list[str]) -> int:
    if len(argv) != 2:
        sys.exit("usage: scripts/release/bump_version.py <new-version> | --check")
    current = current_canonical()
    target = current if argv[1] == "--check" else argv[1]
    parse_semver(target)

    rewritten, errors = rewrite_all(current, target)
    if errors:
        print(f"version pins disagree with Cargo.toml ({current}):", file=sys.stderr)
        for error in errors:
            print(f"  - {error}", file=sys.stderr)
        return 1
    if argv[1] == "--check":
        print(f"version pins: {len(rewritten)} files at {current}")
        return 0
    if current == target:
        sys.exit(f"current version is already {current}; nothing to bump")

    print(f"bumping {current} -> {target}")
    for path, text in rewritten.items():
        if text != path.read_text():
            path.write_text(text)
            print(f"  bumped {path.relative_to(ROOT)}")
    print("refreshing Cargo.lock ...")
    subprocess.run(["cargo", "update", "--workspace"], cwd=ROOT, check=True)
    print(f"version bump to {target} complete; review `git diff` before committing")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
