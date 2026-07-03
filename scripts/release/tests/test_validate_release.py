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


class ValidateReleaseTests(unittest.TestCase):
    def run_release(self, python_stub: str, cpp_stub: str) -> subprocess.CompletedProcess[str]:
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

            bin_dir = tmp / "bin"
            bin_dir.mkdir()
            write_executable(bin_dir / "python3", python_stub)

            env = os.environ.copy()
            env["PATH"] = f"{bin_dir}:{env['PATH']}"
            env.pop("PYTHON_BIN", None)

            return subprocess.run(
                ["bash", str(repo / "scripts" / "release" / "validate_release.sh"), str(creds)],
                cwd=repo,
                env=env,
                stdout=subprocess.PIPE,
                stderr=subprocess.STDOUT,
                text=True,
                check=False,
            )

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


if __name__ == "__main__":
    unittest.main()
