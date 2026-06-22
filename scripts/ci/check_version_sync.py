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

The published Python wheel is covered too. ``sdks/python/pyproject.toml``
declares ``dynamic = ["version"]`` with the maturin backend, so the wheel
version is read from ``sdks/python/Cargo.toml`` ``[package].version`` at
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
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent.parent
CANONICAL_CARGO = ROOT / "crates" / "thetadatadx" / "Cargo.toml"
CMAKE_LISTS = ROOT / "sdks" / "cpp" / "CMakeLists.txt"
PY_INIT = ROOT / "sdks" / "python" / "python" / "thetadatadx" / "__init__.py"
# The published Python wheel version is NOT pinned in `pyproject.toml`
# (it declares `dynamic = ["version"]` with `build-backend = "maturin"`);
# maturin reads it from the binding crate's `[package].version`. The gate
# must therefore assert THIS file against the canonical version, or the
# wheel uploaded to PyPI can age independently of every other artifact.
PY_CARGO = ROOT / "sdks" / "python" / "Cargo.toml"

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
# `thetadatadx = "<version>"` shape. Drift between the canonical Cargo
# version and a doc pin (README.md, docs-site quickstart /
# installation) silently aged the docs across v9 → v10 — this gate
# now catches that case.
DOC_PIN_PATHS = (
    ROOT / "README.md",
    ROOT / "docs-site" / "docs" / "articles" / "getting-started.md",
    ROOT / "docs-site" / "docs" / "examples" / "dataframes.md",
)
# Match `thetadatadx = "<VERSION>"` (Cargo.toml-ish pin) and
# `thetadatadx = { version = "<VERSION>", ... }` (Cargo.toml feature
# pin). Capture the FULL quoted literal — including any pre-release
# suffix (`13.0.0-rc.5`) — not just the major. A major-only capture let
# a stale pre-release pin (`13.0.0-rc.1`) pass against a canonical
# `13.0.0-rc.5` because both share the major `13`; comparing the full
# version closes that hole so a doc that pins an aged release fails the
# gate.
DOC_PIN_RE = re.compile(
    r'thetadatadx\s*=\s*(?:"([^"]+)"|\{\s*version\s*=\s*"([^"]+)")'
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
    issues: list[str] = []
    for path in DOC_PIN_PATHS:
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

    # Published Python wheel version. `sdks/python/pyproject.toml` is
    # `dynamic = ["version"]` + maturin, so the wheel version is taken
    # from `sdks/python/Cargo.toml` `[package].version` at build time, not
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
