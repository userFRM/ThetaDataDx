"""Proves ``Config.worker_threads`` is wired through to the embedded
async runtime rather than being a no-op.

The async runtime is a process-global singleton built once, on the first
client connect. To observe the worker pool deterministically the probe
runs in a fresh subprocess (fresh singleton), sets ``worker_threads`` to a
small N well below the host CPU count, triggers the lazy runtime build via
a connect attempt, then counts the live tokio worker OS threads by name.

The runtime is seeded from the client config *before* the connect
handshake runs, so the worker pool is already sized to N by the time the
connect resolves. A no-op knob would instead leave the pool at tokio's
default sizing (one worker per logical CPU), which on any multi-core CI
host is far larger than N — that is the signal this test keys on.
"""

from __future__ import annotations

import os
import subprocess
import sys

# Probe run in its own interpreter so it owns a fresh process-global
# runtime. Sets worker_threads=N, builds the runtime via a connect
# attempt, and prints the count of tokio worker OS threads. tokio names
# multi-thread workers ``tokio-rt-worker``; the count tracks N exactly
# (tokio's own ``num_workers()`` metric is asserted == N by the Rust FFI
# test ``worker_threads_sizes_the_embedded_runtime``). Authenticating the
# connect spins at most one auxiliary tokio thread, so the OS-thread count
# is allowed a +1 slack here while still proving the pool is sized to N
# and not to the host CPU count.
_PROBE = r"""
import collections
import glob

import thetadatadx as tdx

N = 2

cfg = tdx.Config.dev()
cfg.worker_threads = N

try:
    # Any client constructor seeds the process-global runtime from
    # `cfg.runtime` before the connect handshake runs. The connect itself
    # is expected to fail on throwaway credentials — we only care that the
    # runtime was built at the configured size.
    tdx.MddsClient(tdx.Credentials("nobody@example.invalid", "x"), cfg)
except Exception:
    pass

counts = collections.Counter()
for comm_path in glob.glob("/proc/self/task/*/comm"):
    try:
        with open(comm_path) as fh:
            counts[fh.read().strip()] += 1
    except OSError:
        continue

workers = counts.get("tokio-rt-worker", 0)
print(workers)
"""


def test_worker_threads_sizes_the_embedded_runtime() -> None:
    """``worker_threads = 2`` must size the embedded runtime to ~2 workers.

    A no-op knob would leave the pool at tokio's default size (one worker
    per logical CPU), so on any host with more than three cores this test
    fails if the value is ignored.
    """
    if not sys.platform.startswith("linux"):
        import pytest

        pytest.skip("thread-name probe reads /proc, Linux-only")

    n = 2
    cpu_count = os.cpu_count() or 1
    if cpu_count <= n + 1:
        import pytest

        pytest.skip(f"host has only {cpu_count} CPUs; default sizing is indistinguishable from N={n}")

    result = subprocess.run(
        [sys.executable, "-c", _PROBE],
        capture_output=True,
        text=True,
        timeout=30,
    )
    assert result.returncode == 0, (
        f"probe exited non-zero: stdout={result.stdout!r} stderr={result.stderr!r}"
    )
    workers = int(result.stdout.strip().splitlines()[-1])
    # Honored: pool tracks N (allowing one auxiliary tokio thread). A
    # no-op knob would land at cpu_count (one worker per logical CPU).
    assert n <= workers <= n + 1, (
        f"expected ~{n} tokio worker threads for worker_threads={n}, got {workers} "
        f"(host has {cpu_count} CPUs — a no-op knob would sit near {cpu_count})"
    )
