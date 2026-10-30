"""No two registrations on the native module share a Python name.

`m.add_class` is last-write-wins: a second class registered under a name
already taken replaces the first, and the surface loses the first one's
methods even though the build is green. PyO3 appends every registered name
to the module's `__all__`, so a collision shows up there as a repeated name.
Reading the built module covers the classes the macro tables expand as well
as the hand-written ones, which no scan of the Rust source can see.
"""

from __future__ import annotations

import collections

import pytest

thetadatadx = pytest.importorskip(
    "thetadatadx",
    reason="native thetadatadx extension not built -- run `maturin develop` from thetadatadx-py/",
)


def test_no_two_registrations_share_a_python_name() -> None:
    names = thetadatadx.thetadatadx.__all__
    assert names, "the native module registered nothing"
    repeated = sorted(name for name, count in collections.Counter(names).items() if count > 1)
    assert not repeated, f"registered more than once on the native module: {repeated}"
