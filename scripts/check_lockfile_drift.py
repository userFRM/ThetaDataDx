#!/usr/bin/env python3
"""Detect cross-Cargo.lock dependency-version drift.

The repository tracks five independent `Cargo.lock` files:
  - `Cargo.lock` (workspace root: core crates + ffi + tools/cli)
  - `sdks/python/Cargo.lock` (pyo3 bindings)
  - `sdks/typescript/Cargo.lock` (napi bindings)
  - `tools/server/Cargo.lock` (HTTP server)
  - `tools/mcp/Cargo.lock` (MCP harness)

Each can resolve a shared transitive dependency to a different
version. For most deps that's fine; for security-sensitive deps
(`rustls`, `webpki-roots`, `h2`, `tokio`, `reqwest`) and for
direct-API deps (`thetadatadx`, `tdbe`) we want them pinned to
the same version everywhere so a workspace-root patch propagates
to every shipped binding.

Failure mode this catches:
  - workspace bumps `tokio = "1.52"` to `1.53.0`
  - sdks/python/Cargo.lock keeps tokio 1.52.3 because nothing
    re-resolved it
  - the Python wheel ships an older runtime with a known issue

Usage:
  python3 scripts/check_lockfile_drift.py [--strict]

`--strict` fails on ANY shared-dep drift; default mode only fails
on the curated security/API list below.
"""

from __future__ import annotations

import argparse
import sys
from pathlib import Path

try:
    # `tomllib` is stdlib on 3.11+. Older runners fall back to `toml`.
    import tomllib  # type: ignore[import-not-found]

    def _load(p: Path):
        with p.open("rb") as f:
            return tomllib.load(f)

except ImportError:
    import toml  # type: ignore[import-not-found]

    def _load(p: Path):
        return toml.loads(p.read_text())


# Deps where cross-lockfile drift breaks the release.
SECURITY_CRITICAL = {
    "rustls",
    "webpki-roots",
    "h2",
    "tokio",
    "reqwest",
    "hyper",
    "prost",
    "tokio-rustls",
    "rustls-pki-types",
}

# Direct-API crates: anything that user code links against and that
# we own. A version drift here means two bindings expose the same
# user-facing surface backed by different SDK builds.
SDK_OWNED = {
    "thetadatadx",
    "tdbe",
    "thetadatadx-ffi",
}

# Binding-critical infrastructure crates. Drift here means two SDK
# bindings ship with different ABI assumptions (pyo3 macros expanding
# against different pyo3-ffi headers, arrow buffers laid out by
# different arrow-schema versions, napi shims compiled against
# divergent napi cores, gRPC envelopes encoded by divergent prost /
# old transport schemas). The default-mode drift check used to pass while
# `--strict` flagged `pyo3 0.28.2 vs 0.28.3` and `arrow-ipc /
# arrow-select 58.2.0 vs 58.3.0` between workspace root and
# sdks/python Cargo.lock. Pinning these in the default set keeps the
# gate strict enough to catch the binding-ABI class of drift without
# burning operators on legitimately-divergent leaf transitives.
BINDING_CRITICAL = {
    "pyo3",
    "pyo3-build-config",
    "pyo3-ffi",
    "pyo3-macros",
    "arrow-ipc",
    "arrow-select",
    "arrow-array",
    "arrow-buffer",
    "arrow-schema",
    "napi",
    "napi-derive",
}

WATCHED = SECURITY_CRITICAL | SDK_OWNED | BINDING_CRITICAL


def lockfile_versions(path: Path) -> dict[str, set[str]]:
    """Return `{crate_name: {version, ...}}` for every package in a lock."""
    data = _load(path)
    out: dict[str, set[str]] = {}
    for pkg in data.get("package", []):
        name = pkg.get("name")
        version = pkg.get("version")
        if not name or not version:
            continue
        out.setdefault(name, set()).add(version)
    return out


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument(
        "--strict",
        action="store_true",
        help="Fail on ANY shared-dep drift, not just the watched list.",
    )
    args = ap.parse_args()

    repo_root = Path(__file__).resolve().parent.parent
    lockfiles = [
        repo_root / "Cargo.lock",
        repo_root / "sdks" / "python" / "Cargo.lock",
        repo_root / "sdks" / "typescript" / "Cargo.lock",
        repo_root / "tools" / "server" / "Cargo.lock",
        repo_root / "tools" / "mcp" / "Cargo.lock",
    ]

    # `{crate: {version: [lockfiles_that_have_it]}}`
    crate_map: dict[str, dict[str, list[str]]] = {}
    for lock in lockfiles:
        if not lock.exists():
            print(f"SKIP: {lock} (missing)")
            continue
        rel = lock.relative_to(repo_root).as_posix()
        for crate, versions in lockfile_versions(lock).items():
            for v in versions:
                crate_map.setdefault(crate, {}).setdefault(v, []).append(rel)

    drift_critical: list[tuple[str, dict[str, list[str]]]] = []
    drift_other: list[tuple[str, dict[str, list[str]]]] = []
    for crate, versions_to_files in sorted(crate_map.items()):
        if len(versions_to_files) <= 1:
            continue
        # Crate appears at multiple versions across lockfiles.
        if crate in WATCHED:
            drift_critical.append((crate, versions_to_files))
        else:
            drift_other.append((crate, versions_to_files))

    if drift_critical:
        print("ERROR: cross-lockfile drift on security-critical / SDK-owned deps:")
        for crate, versions_to_files in drift_critical:
            print(f"  {crate}:")
            for v, files in sorted(versions_to_files.items()):
                print(f"    {v} -> {', '.join(files)}")
        if not args.strict:
            print(
                "\nFix: run `cargo update -p <crate>` in the lagging "
                "lockfile, or regenerate via "
                "`cargo generate-lockfile` from each binding root."
            )

    if drift_other and args.strict:
        print("\nERROR (--strict): cross-lockfile drift on other shared deps:")
        for crate, versions_to_files in drift_other:
            print(f"  {crate}:")
            for v, files in sorted(versions_to_files.items()):
                print(f"    {v} -> {', '.join(files)}")

    if drift_critical or (drift_other and args.strict):
        return 1
    if drift_other:
        # Informational only — many transitive deps legitimately
        # resolve differently across bindings (e.g. `windows-sys`
        # narrowing on a binding that doesn't pull in `tokio-net`).
        print(
            f"OK: {len(drift_other)} transitive deps drift across "
            "lockfiles; none on the watched list. Run with --strict "
            "to enumerate."
        )
    else:
        print(
            f"OK: all {len(crate_map)} crates resolve to the same "
            f"version across {sum(1 for l in lockfiles if l.exists())} lockfiles."
        )
    return 0


if __name__ == "__main__":
    sys.exit(main())
