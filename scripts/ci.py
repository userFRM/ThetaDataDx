#!/usr/bin/env python3
"""Single entry point for the repository gate suite.

`python scripts/ci.py all` runs every gate in order and exits non-zero if
any gate fails. `python scripts/ci.py <gate>` runs one named gate. The
gate names match the modules under `scripts/ci/`; each maps to the same
command CI runs for that gate, including any `--selftest` pre-pass and
threshold flags, so the dispatcher and the workflow stay in lockstep.

This is a thin runner: it shells the individual gate scripts under
`scripts/ci/` (and the test files under `scripts/ci/tests/`) with the
current interpreter. It does not reimplement any gate logic.

Usage:
    python scripts/ci.py all
    python scripts/ci.py list
    python scripts/ci.py binding_parity
    python scripts/ci.py docs_consistency
"""

from __future__ import annotations

import subprocess
import sys
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent
CI_DIR = REPO_ROOT / "scripts" / "ci"
TESTS_DIR = CI_DIR / "tests"


def _ci(name: str, *args: str) -> list[list[str]]:
    """One invocation of a gate script under scripts/ci/."""
    return [[sys.executable, str(CI_DIR / f"{name}.py"), *args]]


def _test(name: str) -> list[list[str]]:
    """One invocation of a test under scripts/ci/tests/."""
    return [[sys.executable, str(TESTS_DIR / f"{name}.py")]]


# Gate name -> ordered list of commands. The command set for each gate
# mirrors exactly what the corresponding CI job runs (a `--selftest`
# pre-pass where the gate has one, plus the production invocation with
# the same thresholds CI uses). `all` runs them in this dict's order.
GATES: dict[str, list[list[str]]] = {
    "c_abi_completeness": (
        _ci("check_c_abi_completeness", "--selftest")
        + _ci("check_c_abi_completeness")
    ),
    "binding_parity": (
        _ci("check_binding_parity", "--selftest")
        + _test("test_check_binding_parity")
        + _ci("check_binding_parity")
    ),
    "safety_comment_boilerplate": (
        _ci("check_safety_comment_boilerplate", "--selftest")
        + _ci("check_safety_comment_boilerplate")
    ),
    "public_surface_leak": (
        _ci("check_public_surface_leak", "--selftest")
        + _ci("check_public_surface_leak")
    ),
    "doc_defaults": (
        _ci("check_doc_defaults", "--selftest") + _ci("check_doc_defaults")
    ),
    "no_re_framing": (
        _ci("check_no_re_framing", "--selftest") + _ci("check_no_re_framing")
    ),
    "bench_regression": (
        _ci("check_bench_regression", "--selftest")
        + _ci("check_bench_regression", "--threshold", "25")
    ),
    "perf_gate": _ci("check_perf_gate", "--threshold", "10"),
    "docs_consistency": (
        _ci("check_docs_consistency", "--selftest")
        + _ci("check_docs_consistency")
    ),
    "version_sync": (
        _ci("check_version_sync", "--selftest") + _ci("check_version_sync")
    ),
    "lockfile_drift": _ci("check_lockfile_drift"),
    "tier_badges": _ci("check_tier_badges"),
    "agreement": _test("test_check_agreement"),
}


def run_gate(name: str) -> int:
    commands = GATES[name]
    for cmd in commands:
        print(f"::: {name} -> {' '.join(Path(c).name if i == 1 else c for i, c in enumerate(cmd))}")
        result = subprocess.run(cmd, cwd=REPO_ROOT, check=False)
        if result.returncode != 0:
            print(f"::: gate '{name}' FAILED (exit {result.returncode})", file=sys.stderr)
            return result.returncode
    return 0


def main(argv: list[str]) -> int:
    if len(argv) != 2:
        print(__doc__)
        print(f"gates: {', '.join(GATES)}", file=sys.stderr)
        return 2

    target = argv[1]

    if target == "list":
        for name in GATES:
            print(name)
        return 0

    if target == "all":
        failed: list[str] = []
        for name in GATES:
            if run_gate(name) != 0:
                failed.append(name)
        if failed:
            print(f"\n::: {len(failed)} gate(s) failed: {', '.join(failed)}", file=sys.stderr)
            return 1
        print("\n::: all gates passed")
        return 0

    if target not in GATES:
        print(f"unknown gate '{target}'; known gates: {', '.join(GATES)}", file=sys.stderr)
        return 2

    return run_gate(target)


if __name__ == "__main__":
    sys.exit(main(sys.argv))
