#!/usr/bin/env python3
"""Verify that user-visible package metadata stays in lockstep with Cargo.toml.

The TypeScript SDK ships through npm and pins its version in
``thetadatadx-ts/package.json`` plus three per-platform packages under
``thetadatadx-ts/npm/`` plus three ``optionalDependencies`` entries.
The Rust workspace bumps its Cargo.toml independently. When any of those
fall out of sync (which happened across v8.0.27 / v8.0.28 / v8.0.29 and
left npm stuck on v8.0.26 because the publish workflow keys off
``package.json`` rather than ``Cargo.toml``), the npm package silently
ages while git tags advance.

This script fails CI when any tracked version disagrees with the
canonical ``thetadatadx-rs/Cargo.toml`` version.

The published Python wheel is covered too. ``thetadatadx-py/pyproject.toml``
declares ``dynamic = ["version"]`` with the maturin backend, so the wheel
version is read from ``thetadatadx-py/Cargo.toml`` ``[package].version`` at
build time rather than from any literal in ``pyproject.toml``; the gate
asserts that crate version against the canonical one.

Run::

    python3 scripts/ci/check_version_sync.py

Selftest::

    python3 scripts/ci/check_version_sync.py --selftest

The selftest exercises the two checks whose coverage was widened — the
full-version documentation-pin comparison and the Python-wheel
``Cargo.toml`` version read — against synthetic inputs, proving a stale
pre-release pin and a drifted wheel crate version are both caught.
"""

from __future__ import annotations

import argparse
import json
import re
import sys
import tempfile
import tomllib
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent.parent
CANONICAL_CARGO = ROOT / "thetadatadx-rs" / "Cargo.toml"
CMAKE_LISTS = ROOT / "thetadatadx-cpp" / "CMakeLists.txt"
# The published REST contract. Its `info.version` is the version a
# generated client stamps into its user-agent / about strings, yet it was
# governed by no gate: `check_version_sync` never read this file and
# `check_docs_consistency` validates only the servers / paths / security
# of the same YAML, never `info.version`. So the published OpenAPI spec
# could fall a full release behind canonical undetected. The gate now
# asserts `info.version` against the canonical version.
OPENAPI_YAML = ROOT / "docs-site" / "docs" / "public" / "thetadatadx.yaml"
PY_INIT = ROOT / "thetadatadx-py" / "python" / "thetadatadx" / "__init__.py"
# The published Python wheel version is NOT pinned in `pyproject.toml`
# (it declares `dynamic = ["version"]` with `build-backend = "maturin"`);
# maturin reads it from the binding crate's `[package].version`. The gate
# must therefore assert THIS file against the canonical version, or the
# wheel uploaded to PyPI can age independently of every other artifact.
PY_CARGO = ROOT / "thetadatadx-py" / "Cargo.toml"

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


# Every workspace crate's published / advertised version must move in
# lockstep with the canonical one. The previous gate asserted only the
# canonical `thetadatadx-rs/Cargo.toml` and `thetadatadx-py/Cargo.toml`
# manifests, leaving the FFI, CLI, server, MCP, and TypeScript-binding
# crates — plus EVERY `Cargo.lock` — unscanned. The server and MCP
# binaries advertise their version at runtime via `CARGO_PKG_VERSION`
# (sourced from their own manifests), and a lockfile that pins a stale
# `thetadatadx` entry ships an aged dependency graph; both drifted
# silently. The two helpers below discover every manifest and lockfile so
# any `thetadatadx*` version that falls out of sync trips the gate.
#
# Build output and vendored trees never carry a first-party manifest, so
# they are pruned from the walk.
_MANIFEST_EXCLUDE_FRAGMENTS = ("/target/", "/node_modules/", "/.git/")


def _is_excluded_manifest(path: Path) -> bool:
    rel = "/" + path.relative_to(ROOT).as_posix()
    return any(frag in rel for frag in _MANIFEST_EXCLUDE_FRAGMENTS)


def cargo_manifest_package_version(path: Path) -> tuple[str, str] | None:
    """`([package].name, [package].version)` for a Cargo manifest, or
    `None` for a virtual manifest (a `[workspace]` root with no
    `[package]`). Parsed with `tomllib` so a `version` under
    `[dependencies]` is never mistaken for the package version.
    """
    data = tomllib.loads(path.read_text())
    pkg = data.get("package")
    if not isinstance(pkg, dict) or "version" not in pkg:
        return None
    version = pkg["version"]
    # `version.workspace = true` inheritance renders as a dict; the
    # thetadatadx workspace does NOT hoist `version` (each member carries
    # its own literal), so a dict here means the manifest defers to the
    # workspace and there is no literal to assert.
    if not isinstance(version, str):
        return None
    return str(pkg.get("name", "")), version


def cargo_manifest_mismatches(canonical: str) -> list[str]:
    """Every first-party (`thetadatadx*`) crate manifest whose
    `[package].version` disagrees with the canonical version.

    `publish = false` crates (the binding crates, the server/MCP/CLI
    tools) are included on purpose: the server and MCP binaries surface
    their version at runtime through `CARGO_PKG_VERSION`, so a drifted
    manifest there ships a wrong version string even though nothing is
    uploaded to a registry.
    """
    issues: list[str] = []
    for path in sorted(ROOT.rglob("Cargo.toml")):
        if _is_excluded_manifest(path):
            continue
        parsed = cargo_manifest_package_version(path)
        if parsed is None:
            continue
        name, version = parsed
        if not name.startswith("thetadatadx"):
            continue
        if version != canonical:
            issues.append(
                f"{path.relative_to(ROOT)} [package] version (crate "
                f"`{name}`) is {version}, expected {canonical}"
            )
    return issues


def cargo_lock_mismatches(canonical: str) -> list[str]:
    """Every `Cargo.lock` whose pinned `thetadatadx*` package entries
    disagree with the canonical version.

    A lockfile is valid TOML with a top-level `[[package]]` array; the
    first-party crates appear there with their resolved version. A stale
    pin here means the resolved dependency graph ships an aged crate even
    when the manifest is correct, so every lockfile in the tree (the
    excluded-from-workspace SDK / tool lockfiles included) is scanned.
    """
    issues: list[str] = []
    for path in sorted(ROOT.rglob("Cargo.lock")):
        if _is_excluded_manifest(path):
            continue
        data = tomllib.loads(path.read_text())
        for pkg in data.get("package", []):
            name = pkg.get("name", "")
            if not name.startswith("thetadatadx"):
                continue
            version = pkg.get("version")
            if version != canonical:
                issues.append(
                    f"{path.relative_to(ROOT)} pins `{name}` at {version}, "
                    f"expected {canonical}"
                )
    return issues


def package_json_version(path: Path) -> str:
    return json.loads(path.read_text())["version"]


def package_json_optional_deps(path: Path) -> dict[str, str]:
    return json.loads(path.read_text()).get("optionalDependencies", {})


def cmake_project_version(path: Path) -> str | None:
    """Extract the `project(... VERSION x.y.z ...)` value from a
    CMakeLists.txt. U4 closure: the version-sync check previously
    returned clean even when `thetadatadx-cpp/CMakeLists.txt` still pinned
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


def openapi_info_version(path: Path) -> str | None:
    """The `info.version` value from an OpenAPI YAML, or `None` if the
    file or the key is absent.

    Block-scoped: the spec carries other `version:` keys inside response
    schemas (deeper-indented), so a naive first-`version:` grab would read
    the wrong value. This walks the top-level `info:` block — from the
    unindented `info:` line to the next top-level (column-0) key — and
    returns the `version:` found within it. Hand-rolled in the same style
    as `check_docs_consistency._openapi_server_urls` to keep the gate
    dependency-light (no YAML parser).
    """
    if not path.is_file():
        return None
    in_info = False
    for line in path.read_text().splitlines():
        if re.match(r"^info:\s*$", line):
            in_info = True
            continue
        if in_info:
            # A new top-level key (no indentation) ends the info block.
            if re.match(r"^\S", line):
                break
            m = re.match(r"\s+version:\s*(\S+)\s*$", line)
            if m:
                return m.group(1).strip().strip('"').strip("'")
    return None


# U5 closure: the same gate scans documentation pins for the
# `thetadatadx = "<version>"` shape. Drift between the canonical Cargo
# version and a doc pin silently aged the docs across v9 → v10, and again
# across the rc series: `thetadatadx-rs/README.md` ships verbatim to
# docs.rs (it is the crate's `readme = "README.md"`) yet a hand-picked
# three-path list never scanned it, so it pinned `13.0.0-rc.1` against a
# canonical `13.0.0-rc.5` undetected. The gate now discovers EVERY `*.md`
# in the tree (minus the exclusions below) so a new install snippet in any
# Markdown file is covered the moment it lands, with no edit here.
#
# Excluded from the doc-pin scan, because they pin historical versions by
# design and would false-positive against the canonical version:
#
#   * `CHANGELOG.md` + `docs-site/docs/changelog.md` — the changelog and
#     its published mirror enumerate every shipped version.
#   * `.github/release-notes/*.md` (and the whole `.github/` tree) — each
#     release note pins the version it documents.
#   * `docs-site/docs/migration/*.md` — migration guides pin the old→new
#     version pair as before/after examples (`thetadatadx = "11"` → `"12"`).
#
# Build output, vendored deps, and VCS metadata are excluded too.
_DOC_PIN_EXCLUDE_FRAGMENTS = (
    "/target/",
    "/node_modules/",
    "/.git/",
    "/.github/",
    "/migration/",
)
_DOC_PIN_EXCLUDE_NAMES = frozenset({"CHANGELOG.md", "changelog.md"})


def _discover_doc_pin_files() -> tuple[Path, ...]:
    """Every `*.md` under the repo root that should carry a current pin.

    Walks the tree and drops the historical-pin files (changelog, release
    notes, migration guides) and the build/vendor/VCS trees. Sorted for
    deterministic diagnostics.
    """
    out: list[Path] = []
    for path in ROOT.rglob("*.md"):
        rel = "/" + path.relative_to(ROOT).as_posix()
        if any(frag in rel for frag in _DOC_PIN_EXCLUDE_FRAGMENTS):
            continue
        if path.name in _DOC_PIN_EXCLUDE_NAMES:
            continue
        out.append(path)
    return tuple(sorted(out))


# Retained as an override hook: when `None` (production), the doc-pin scan
# discovers the file set via `_discover_doc_pin_files()`. The selftest sets
# this to an explicit synthetic list to stay hermetic.
DOC_PIN_PATHS: tuple[Path, ...] | None = None
# Match `thetadatadx = "<VERSION>"` (Cargo.toml-ish pin) and
# `thetadatadx = { version = "<VERSION>", ... }` (Cargo.toml feature
# pin). Capture the FULL quoted literal — including any pre-release
# suffix (`13.0.0-rc.5`) — not just the major. A major-only capture let
# a stale pre-release pin (`13.0.0-rc.1`) pass against a canonical
# `13.0.0-rc.5` because both share the major `13`; comparing the full
# version closes that hole so a doc that pins an aged release fails the
# gate.
DOC_PIN_RE = re.compile(
    r'thetadatadx-rs\s*=\s*(?:"([^"]+)"|\{\s*version\s*=\s*"([^"]+)")'
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


def doc_pin_mismatches(canonical: str) -> list[str]:
    paths = DOC_PIN_PATHS if DOC_PIN_PATHS is not None else _discover_doc_pin_files()
    issues: list[str] = []
    for path in paths:
        if not path.is_file():
            continue
        for lineno, line in enumerate(path.read_text().splitlines(), start=1):
            match = DOC_PIN_RE.search(line)
            if not match:
                continue
            pinned = match.group(1) or match.group(2)
            if pinned != canonical:
                issues.append(
                    f"{path.relative_to(ROOT)}:{lineno} pins "
                    f'`thetadatadx = "{pinned}"`, expected {canonical}'
                )
    return issues


def main() -> int:
    canonical = cargo_version(CANONICAL_CARGO)
    print(f"canonical version (Cargo.toml): {canonical}")

    failures: list[str] = []

    ts_root = ROOT / "thetadatadx-ts" / "package.json"
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

    # The MCP server ships to npm as well (`npx -y thetadatadx-mcp`): a
    # launcher package plus one prebuilt-binary package per platform, all
    # under `tools/mcp/npm/`. They pin the canonical version exactly like
    # the TypeScript packages and are bumped by `bump_version.py` in the
    # same pass; scan every `package.json` (launcher + platforms) and the
    # launcher's optionalDependencies so a missed bump trips this gate
    # instead of shipping a launcher that depends on a stale binary
    # package.
    for mcp_pkg in (ROOT / "tools" / "mcp" / "npm").glob("*/package.json"):
        if package_json_version(mcp_pkg) != canonical:
            failures.append(
                f"{mcp_pkg.relative_to(ROOT)} version is "
                f"{package_json_version(mcp_pkg)}, expected {canonical}"
            )
        for name, pinned in package_json_optional_deps(mcp_pkg).items():
            if pinned != canonical:
                failures.append(
                    f"{mcp_pkg.relative_to(ROOT)} optionalDependencies"
                    f"['{name}'] is {pinned}, expected {canonical}"
                )

    # Published Python wheel version. `thetadatadx-py/pyproject.toml` is
    # `dynamic = ["version"]` + maturin, so the wheel version is taken
    # from `thetadatadx-py/Cargo.toml` `[package].version` at build time, not
    # from any literal in `pyproject.toml`. Asserting that crate version
    # is what stops the PyPI wheel from silently aging while every other
    # tracked artifact advances.
    if PY_CARGO.is_file():
        py_cargo_version = cargo_version(PY_CARGO)
        if py_cargo_version != canonical:
            failures.append(
                f"{PY_CARGO.relative_to(ROOT)} [package] version is "
                f"{py_cargo_version}, expected {canonical} "
                "(this is the version maturin stamps on the published "
                "Python wheel via pyproject.toml `dynamic = [\"version\"]`)"
            )
    else:
        failures.append(
            f"{PY_CARGO.relative_to(ROOT)}: not found — the published "
            "Python wheel version cannot be verified"
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

    # U5 closure: documentation pins must match the canonical version in
    # full (including any pre-release suffix), not merely the major.
    failures.extend(doc_pin_mismatches(canonical))

    # Every first-party crate manifest and every lockfile must agree with
    # the canonical version — the FFI / CLI / server / MCP / TS-binding
    # crates and all `Cargo.lock` files were previously unscanned, so the
    # server + MCP runtime `CARGO_PKG_VERSION` advertisement and the
    # resolved dependency graph could drift undetected.
    failures.extend(cargo_manifest_mismatches(canonical))
    failures.extend(cargo_lock_mismatches(canonical))

    # The published OpenAPI contract's `info.version` must match canonical.
    # A generated client reads it for its about / user-agent strings, yet
    # neither this gate nor the docs-consistency gate previously asserted
    # it, so the published REST spec could age a full release behind.
    openapi_version = openapi_info_version(OPENAPI_YAML)
    if openapi_version is None:
        failures.append(
            f"{OPENAPI_YAML.relative_to(ROOT)}: could not read `info.version`"
        )
    elif openapi_version != canonical:
        failures.append(
            f"{OPENAPI_YAML.relative_to(ROOT)} info.version is "
            f"{openapi_version}, expected {canonical}"
        )

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


def _selftest() -> int:
    """Exercise the two widened checks against synthetic inputs.

    Cases:

    * A doc file that pins a stale pre-release (`thetadatadx =
      "13.0.0-rc.1"`) against canonical `13.0.0-rc.5` — must be flagged.
      The old major-only capture passed it because both share major `13`;
      the full-version capture must now catch it. The feature-pin shape
      (`{ version = "..." }`) is covered too.
    * A doc file pinning the exact canonical version — must pass.
    * A synthetic Python binding `Cargo.toml` whose `[package].version`
      drifts from canonical — `cargo_version` must read the full
      pre-release literal and the mismatch must be detected. A matching
      crate version must compare equal. This is the published-wheel
      version that `pyproject.toml`'s `dynamic = ["version"]` + maturin
      stamps onto the PyPI upload.
    * Doc-pin discovery (G3): against a synthetic temp ROOT, an install
      doc (`thetadatadx-py/README.md`) is discovered while a changelog, a release
      note, and a migration guide are excluded; a stale pin in the install
      doc is flagged and the historical-pin files never leak in.
    * Manifest + lockfile coverage (G4): a drifted first-party crate
      manifest (`thetadatadx-server`) and a drifted lockfile entry are
      flagged, a third-party crate is ignored, a `[dependencies]` version
      is not mistaken for the package version, and a virtual workspace root
      with no `[package]` does not crash.
    * OpenAPI info.version (G12): the block-scoped reader returns the
      `info.version` and not a deeper-indented response-schema `version:`
      key; a drifted value is read verbatim and a matching one compares
      equal to canonical.
    """
    global DOC_PIN_PATHS

    canonical = "13.0.0-rc.5"
    failures: list[str] = []

    with tempfile.TemporaryDirectory() as td:
        root = Path(td)

        # --- Documentation pin: full-version comparison ---------------
        stale_doc = root / "README.md"
        stale_doc.write_text(
            "Install with\n\n"
            "```toml\n"
            '[dependencies]\n'
            'thetadatadx = "13.0.0-rc.1"\n'
            "```\n",
            encoding="utf-8",
        )
        stale_feature_doc = root / "features.md"
        stale_feature_doc.write_text(
            'thetadatadx = { version = "13.0.0-rc.1", features = ["frames"] }\n',
            encoding="utf-8",
        )
        clean_doc = root / "clean.md"
        clean_doc.write_text(
            'thetadatadx = "13.0.0-rc.5"\n',
            encoding="utf-8",
        )

        saved_paths = DOC_PIN_PATHS
        DOC_PIN_PATHS = (stale_doc, stale_feature_doc, clean_doc)
        try:
            # `doc_pin_mismatches` formats paths relative to the real
            # ROOT; the synthetic temp paths are not under ROOT, so derive
            # the diagnostics from the regex + comparison directly here to
            # keep the selftest hermetic, while still proving the captured
            # literal is the FULL version (not the major).
            pin_issues: list[str] = []
            for path in DOC_PIN_PATHS:
                for line in path.read_text().splitlines():
                    m = DOC_PIN_RE.search(line)
                    if not m:
                        continue
                    pinned = m.group(1) or m.group(2)
                    if pinned != canonical:
                        pin_issues.append((path.name, pinned))
        finally:
            DOC_PIN_PATHS = saved_paths

        stale_flagged = {name for name, _ in pin_issues}
        if "README.md" not in stale_flagged:
            failures.append(
                "doc-pin: stale plain pre-release pin `13.0.0-rc.1` was not "
                "flagged against canonical 13.0.0-rc.5 (full-version "
                "comparison regressed)"
            )
        if "features.md" not in stale_flagged:
            failures.append(
                "doc-pin: stale feature pin `{ version = \"13.0.0-rc.1\" }` "
                "was not flagged"
            )
        if any(pinned != "13.0.0-rc.1" for _, pinned in pin_issues):
            failures.append(
                f"doc-pin: captured an unexpected literal: {pin_issues!r} "
                "(the full pre-release string must be captured verbatim)"
            )
        if "clean.md" in stale_flagged:
            failures.append(
                "doc-pin: a pin matching the canonical version was wrongly "
                "flagged"
            )

        # --- Python wheel version (maturin reads Cargo.toml) ----------
        drifted_cargo = root / "py_cargo_drift.toml"
        drifted_cargo.write_text(
            '[package]\n'
            'name = "thetadatadx-py"\n'
            'version = "13.0.0-rc.1"\n'
            'edition = "2021"\n',
            encoding="utf-8",
        )
        matched_cargo = root / "py_cargo_ok.toml"
        matched_cargo.write_text(
            '[package]\n'
            'name = "thetadatadx-py"\n'
            'version = "13.0.0-rc.5"\n'
            'edition = "2021"\n',
            encoding="utf-8",
        )
        if cargo_version(drifted_cargo) != "13.0.0-rc.1":
            failures.append(
                "py-wheel: cargo_version did not read the full pre-release "
                f"literal (got {cargo_version(drifted_cargo)!r})"
            )
        if cargo_version(drifted_cargo) == canonical:
            failures.append(
                "py-wheel: a drifted Python crate version compared equal to "
                "canonical (the wheel-version assertion would not fire)"
            )
        if cargo_version(matched_cargo) != canonical:
            failures.append(
                "py-wheel: a matching Python crate version did not compare "
                "equal to canonical"
            )

    # --- Doc-pin discovery: recursive *.md scan with exclusions ---------
    # A synthetic repo tree: a stale pin in an arbitrary install doc must
    # be discovered, while the same stale pin in a changelog / release-note
    # / migration guide must be excluded (those legitimately pin history).
    global ROOT
    saved_root = ROOT
    with tempfile.TemporaryDirectory() as td:
        root = Path(td)
        ROOT = root
        DOC_PIN_PATHS = None  # force production discovery against the temp ROOT
        try:
            install_doc = root / "thetadatadx-py" / "README.md"
            install_doc.parent.mkdir(parents=True, exist_ok=True)
            install_doc.write_text('thetadatadx = "13.0.0-rc.1"\n', encoding="utf-8")

            changelog = root / "CHANGELOG.md"
            changelog.write_text('thetadatadx = "13.0.0-rc.1"\n', encoding="utf-8")

            release_note = root / ".github" / "release-notes" / "v13.0.0-rc.1.md"
            release_note.parent.mkdir(parents=True, exist_ok=True)
            release_note.write_text('thetadatadx = "13.0.0-rc.1"\n', encoding="utf-8")

            migration = root / "docs-site" / "docs" / "migration" / "v11-to-v12.md"
            migration.parent.mkdir(parents=True, exist_ok=True)
            migration.write_text('thetadatadx = "11"\n', encoding="utf-8")

            discovered = {p.relative_to(root).as_posix() for p in _discover_doc_pin_files()}
            if "thetadatadx-py/README.md" not in discovered:
                failures.append(
                    "doc-pin-discovery: an install doc outside the old "
                    "hardcoded list was not discovered (the recursive *.md "
                    "scan regressed — this is the crate-README drift bypass)"
                )
            for excluded in (
                "CHANGELOG.md",
                ".github/release-notes/v13.0.0-rc.1.md",
                "docs-site/docs/migration/v11-to-v12.md",
            ):
                if excluded in discovered:
                    failures.append(
                        f"doc-pin-discovery: {excluded} was scanned but must be "
                        "excluded (it pins historical versions by design)"
                    )

            mismatches = doc_pin_mismatches(canonical)
            if not any("thetadatadx-py/README.md" in m for m in mismatches):
                failures.append(
                    "doc-pin-discovery: the stale pin in the discovered install "
                    "doc was not flagged as a mismatch"
                )
            if any("CHANGELOG.md" in m or "migration" in m for m in mismatches):
                failures.append(
                    "doc-pin-discovery: a historical-pin file leaked into the "
                    "mismatch list"
                )
        finally:
            DOC_PIN_PATHS = None
            ROOT = saved_root

    # --- Manifest + lockfile coverage (G4) -----------------------------
    # A synthetic repo with a drifted first-party manifest, a drifted
    # lockfile entry, a third-party manifest that must be IGNORED, and a
    # virtual workspace root with no `[package]` (must not crash).
    with tempfile.TemporaryDirectory() as td:
        root = Path(td)
        ROOT = root
        try:
            # Virtual workspace root — no `[package]`.
            (root / "Cargo.toml").write_text(
                '[workspace]\nmembers = ["server"]\n', encoding="utf-8"
            )
            # First-party crate manifest, drifted.
            srv = root / "server" / "Cargo.toml"
            srv.parent.mkdir(parents=True, exist_ok=True)
            srv.write_text(
                '[package]\nname = "thetadatadx-server"\n'
                'version = "13.0.0-rc.1"\nedition = "2021"\n\n'
                '[dependencies]\n'
                # A dependency `version` must NOT be mistaken for the
                # package version — tomllib scoping proves this.
                'serde = { version = "1.0.0" }\n',
                encoding="utf-8",
            )
            # Third-party crate manifest — must be ignored (name prefix).
            other = root / "other" / "Cargo.toml"
            other.parent.mkdir(parents=True, exist_ok=True)
            other.write_text(
                '[package]\nname = "some-other-crate"\n'
                'version = "0.1.0"\nedition = "2021"\n',
                encoding="utf-8",
            )
            # Lockfile with a drifted first-party entry + an unrelated one.
            lock = root / "Cargo.lock"
            lock.write_text(
                'version = 3\n\n'
                '[[package]]\nname = "thetadatadx"\nversion = "13.0.0-rc.1"\n\n'
                '[[package]]\nname = "serde"\nversion = "1.0.0"\n',
                encoding="utf-8",
            )

            man_issues = cargo_manifest_mismatches(canonical)
            if not any("thetadatadx-server" in m for m in man_issues):
                failures.append(
                    "manifest: a drifted first-party crate manifest "
                    "(thetadatadx-server) was not flagged — the FFI / CLI / "
                    "server / MCP manifests were the unscanned bypass"
                )
            if any("some-other-crate" in m for m in man_issues):
                failures.append(
                    "manifest: a third-party crate was flagged (the scan must "
                    "be scoped to `thetadatadx*` crates)"
                )

            lock_issues = cargo_lock_mismatches(canonical)
            if not any("thetadatadx" in m for m in lock_issues):
                failures.append(
                    "lockfile: a drifted first-party lock entry was not flagged "
                    "— every Cargo.lock was previously unscanned"
                )
            if any("serde" in m for m in lock_issues):
                failures.append(
                    "lockfile: an unrelated lock entry (serde) was flagged"
                )
        finally:
            ROOT = saved_root

    # --- OpenAPI info.version (G12) -------------------------------------
    # The block-scoped reader must pull the `info.version` and ignore the
    # deeper-indented `version:` keys inside response schemas (the trap the
    # task flagged). A drifted info.version must be detected; a matching one
    # must compare equal.
    with tempfile.TemporaryDirectory() as td:
        root = Path(td)
        drift_yaml = root / "drift.yaml"
        drift_yaml.write_text(
            "openapi: 3.1.0\n"
            "info:\n"
            "  title: Theta Data v3\n"
            "  version: 13.0.0-rc.1\n"
            "  contact:\n"
            "    name: x\n"
            "paths:\n"
            "  /v3/thing:\n"
            "    get:\n"
            "      responses:\n"
            "        '200':\n"
            "          content:\n"
            "            application/json:\n"
            "              schema:\n"
            "                properties:\n"
            "                  version:\n"
            "                    type: string\n",
            encoding="utf-8",
        )
        got = openapi_info_version(drift_yaml)
        if got != "13.0.0-rc.1":
            failures.append(
                "openapi-info-version: block-scoped read returned "
                f"{got!r}, expected '13.0.0-rc.1' (a deeper-indented "
                "response-schema `version:` key may have been grabbed instead "
                "of info.version)"
            )
        ok_yaml = root / "ok.yaml"
        ok_yaml.write_text(
            "info:\n  title: x\n  version: 13.0.0-rc.5\n",
            encoding="utf-8",
        )
        if openapi_info_version(ok_yaml) != canonical:
            failures.append(
                "openapi-info-version: a matching info.version did not compare "
                "equal to canonical"
            )

    if failures:
        print("check_version_sync --selftest: FAILED")
        for f in failures:
            print(f"  - {f}")
        return 1

    print("check_version_sync --selftest: ok")
    return 0


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument(
        "--selftest",
        action="store_true",
        help="Run the embedded self-test and exit.",
    )
    args = parser.parse_args()
    sys.exit(_selftest() if args.selftest else main())
