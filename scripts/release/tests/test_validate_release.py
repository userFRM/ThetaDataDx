#!/usr/bin/env python3
"""Offline tests for validate_release.sh fail-closed behavior."""

from __future__ import annotations

import os
import shutil
import stat
import subprocess
import tempfile
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[3]


def write_executable(path: Path, text: str) -> None:
    path.write_text(text, encoding="utf-8")
    path.chmod(path.stat().st_mode | stat.S_IXUSR)


OK_STUB = """#!/usr/bin/env bash
exit 0
"""


class ValidateReleaseTests(unittest.TestCase):
    def run_release(
        self,
        python_stub: str,
        cpp_stub: str,
        cmake_stub: str = OK_STUB,
        node_stub: str = OK_STUB,
        stale_artifacts: tuple[str, ...] = (),
        on_result=None,
    ) -> subprocess.CompletedProcess[str]:
        with tempfile.TemporaryDirectory() as raw_tmp:
            tmp = Path(raw_tmp)
            repo = tmp / "repo"
            shutil.copytree(ROOT / "scripts" / "release", repo / "scripts" / "release")

            creds = repo / "creds.txt"
            creds.write_text("email@example.test\nnot-a-real-password\n", encoding="utf-8")

            ffi_dir = repo / "target" / "release"
            ffi_dir.mkdir(parents=True)
            (ffi_dir / "libthetadatadx_ffi.so").write_text("", encoding="utf-8")

            cpp_validator = repo / "thetadatadx-cpp" / "build" / "thetadatadx_validate"
            cpp_validator.parent.mkdir(parents=True)
            write_executable(cpp_validator, cpp_stub)

            # Artifacts an earlier run would have left behind.
            artifacts = repo / "artifacts"
            artifacts.mkdir(parents=True)
            for name in stale_artifacts:
                (artifacts / name).write_text('{"stale": true}', encoding="utf-8")

            bin_dir = tmp / "bin"
            bin_dir.mkdir()
            write_executable(bin_dir / "python3", python_stub)
            # The script now rebuilds every artifact it validates rather than
            # reusing what is on disk, so the build tools are on the stubbed
            # PATH too and the tested path is the one that ships.
            write_executable(bin_dir / "cargo", OK_STUB)
            write_executable(bin_dir / "cmake", cmake_stub)
            write_executable(bin_dir / "node", node_stub)

            env = os.environ.copy()
            env["PATH"] = f"{bin_dir}:{env['PATH']}"
            # Without this the script compiles the extension from source; these
            # tests are about fail-closed reporting, not the bootstrap.
            env["PYTHON_BIN"] = str(bin_dir / "python3")

            proc = subprocess.run(
                ["bash", str(repo / "scripts" / "release" / "validate_release.sh"), str(creds)],
                cwd=repo,
                env=env,
                stdout=subprocess.PIPE,
                stderr=subprocess.STDOUT,
                text=True,
                check=False,
            )
            if on_result is not None:
                on_result(repo, proc)
            return proc

    def test_successful_stubbed_validators_pass(self) -> None:
        proc = self.run_release(
            python_stub="""#!/usr/bin/env bash
case "$1" in
  -c) exit 0 ;;
  */check_python.py) printf 'COUNTS:2:0:0\\n'; exit 0 ;;
  */check_agreement.py) printf 'agreement ok\\n'; exit 0 ;;
esac
exit 64
""",
            cpp_stub="""#!/usr/bin/env bash
printf 'COUNTS:3:0:0\\n'
exit 0
""",
        )

        self.assertEqual(proc.returncode, 0, proc.stdout)
        self.assertIn("RELEASE OK", proc.stdout)

    def test_agreement_nonzero_exit_blocks_release(self) -> None:
        proc = self.run_release(
            python_stub="""#!/usr/bin/env bash
case "$1" in
  -c) exit 0 ;;
  */check_python.py) printf 'COUNTS:2:0:0\\n'; exit 0 ;;
  */check_agreement.py) printf 'agreement failed\\n'; exit 7 ;;
esac
exit 64
""",
            cpp_stub="""#!/usr/bin/env bash
printf 'COUNTS:3:0:0\\n'
exit 0
""",
        )

        self.assertEqual(proc.returncode, 1, proc.stdout)
        self.assertIn("agreement failed", proc.stdout)
        self.assertIn("RELEASE BLOCKED", proc.stdout)

    def test_missing_python_counts_blocks_release(self) -> None:
        proc = self.run_release(
            python_stub="""#!/usr/bin/env bash
case "$1" in
  -c) exit 0 ;;
  */check_python.py) printf 'python crashed before counts\\n'; exit 0 ;;
  */check_agreement.py) printf 'agreement ok\\n'; exit 0 ;;
esac
exit 64
""",
            cpp_stub="""#!/usr/bin/env bash
printf 'COUNTS:3:0:0\\n'
exit 0
""",
        )

        self.assertEqual(proc.returncode, 1, proc.stdout)
        self.assertIn("Python validator did not emit COUNTS:p:s:f.", proc.stdout)
        self.assertIn("RELEASE BLOCKED", proc.stdout)

    def test_nonzero_cpp_validator_exit_blocks_release(self) -> None:
        proc = self.run_release(
            python_stub="""#!/usr/bin/env bash
case "$1" in
  -c) exit 0 ;;
  */check_python.py) printf 'COUNTS:2:0:0\\n'; exit 0 ;;
  */check_agreement.py) printf 'agreement ok\\n'; exit 0 ;;
esac
exit 64
""",
            cpp_stub="""#!/usr/bin/env bash
printf 'COUNTS:3:0:0\\n'
exit 9
""",
        )

        self.assertEqual(proc.returncode, 1, proc.stdout)
        self.assertIn("C++ validator exited with status 9.", proc.stdout)
        self.assertIn("RELEASE BLOCKED", proc.stdout)


    def test_stale_artifact_from_an_earlier_run_is_cleared(self) -> None:
        # `check_agreement.py` reads whatever sits in artifacts/. A file from an
        # earlier run, or from another branch, would be diffed against today's
        # output and counted as agreement between two bindings never built
        # together. The run clears the directory before producing anything.
        seen: dict[str, bool] = {}

        def check(repo: Path, _proc: subprocess.CompletedProcess[str]) -> None:
            seen["left"] = (repo / "artifacts" / "validator_cpp.json").exists()

        self.run_release(
            python_stub="""#!/usr/bin/env bash
case "$1" in
  -c) exit 0 ;;
  */check_python.py) printf 'COUNTS:2:0:0\\n'; exit 0 ;;
  */check_agreement.py) printf 'agreement ok\\n'; exit 0 ;;
esac
exit 64
""",
            cpp_stub="""#!/usr/bin/env bash
printf 'COUNTS:3:0:0\\n'
exit 0
""",
            stale_artifacts=("validator_cpp.json",),
            on_result=check,
        )

        self.assertFalse(
            seen["left"],
            "an artifact left by an earlier run survived into the agreement step",
        )

    def test_failed_cpp_build_does_not_fall_through_to_a_stale_binary(self) -> None:
        # The validator binary is present and would answer COUNTS:3:0:0, but it
        # was built from code that is no longer in the tree. A failing build has
        # to block the release rather than report that binary's numbers.
        proc = self.run_release(
            python_stub="""#!/usr/bin/env bash
case "$1" in
  -c) exit 0 ;;
  */check_python.py) printf 'COUNTS:2:0:0\\n'; exit 0 ;;
  */check_agreement.py) printf 'agreement ok\\n'; exit 0 ;;
esac
exit 64
""",
            cpp_stub="""#!/usr/bin/env bash
printf 'COUNTS:3:0:0\\n'
exit 0
""",
            cmake_stub="""#!/usr/bin/env bash
echo "cmake: no such target" >&2
exit 1
""",
        )

        self.assertEqual(proc.returncode, 1, proc.stdout)
        self.assertIn("C++ validator build failed", proc.stdout)
        self.assertNotIn("COUNTS:3:0:0", proc.stdout)
        self.assertIn("RELEASE BLOCKED", proc.stdout)

    def test_typescript_manifest_emit_failure_blocks_release(self) -> None:
        # The TS shape manifest is one of the three artifacts
        # `--require-all-sdks` demands. If it is not emitted the agreement step
        # has nothing to compare the TS surface against, so the emit failing
        # has to be a release failure and not a silent gap.
        proc = self.run_release(
            python_stub="""#!/usr/bin/env bash
case "$1" in
  -c) exit 0 ;;
  */check_python.py) printf 'COUNTS:2:0:0\\n'; exit 0 ;;
  */check_agreement.py) printf 'agreement ok\\n'; exit 0 ;;
esac
exit 64
""",
            cpp_stub="""#!/usr/bin/env bash
printf 'COUNTS:3:0:0\\n'
exit 0
""",
            node_stub="""#!/usr/bin/env bash
echo "SyntaxError: unexpected token" >&2
exit 1
""",
        )

        self.assertEqual(proc.returncode, 1, proc.stdout)
        self.assertIn("TypeScript shape manifest emit failed", proc.stdout)
        self.assertIn("RELEASE BLOCKED", proc.stdout)


if __name__ == "__main__":
    unittest.main()
