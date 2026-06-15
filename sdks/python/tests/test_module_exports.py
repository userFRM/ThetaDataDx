"""Gate 1 (issue #544): every `#[pyclass]` declared in the Python SDK's
Rust source must end up registered on the compiled module.

Production user on v10.0.0 hit `ImportError` for `from thetadatadx import
Contract` because the pyclass was defined but never registered via
`m.add_class::<...>()`. This test parametrises one assertion per pyclass
so the failure points at the missing name, not a generic "drift" message.

False positives — types that are pyclasses for FFI-internal reasons but
deliberately not exposed at the Python module level — go in
``DELIBERATELY_PRIVATE`` with a one-line reason. Start empty; add only
when there is a real "the runtime needs it as a class to pass typed
references around, but Python users never instantiate it" case.
"""

from __future__ import annotations

import importlib
import pathlib
import re
from typing import Iterable

import pytest


SRC_DIR = pathlib.Path(__file__).resolve().parents[1] / "src"


# `#[pyclass(...)] <attrs> pub(crate)? struct <Name>` — matches the
# same pyo3 sugar the no-collision regression test uses (file:
# `test_no_pyclass_name_collisions.py`), kept in lockstep so the same
# scanner powers both gates.
PYCLASS_RE = re.compile(
    r"#\[pyclass(?:\(([^)]*)\))?\][^{]*?"
    r"(?:pub(?:\(crate\))?\s+)?struct\s+(\w+)",
    re.MULTILINE | re.DOTALL,
)
NAME_ATTR_RE = re.compile(r'name\s*=\s*"([^"]+)"')


def _python_name(attrs: str | None, struct_name: str) -> str:
    """Resolve the Python-facing class name from the `#[pyclass]` attrs."""
    if attrs:
        match = NAME_ATTR_RE.search(attrs)
        if match:
            return match.group(1)
    return struct_name.removeprefix("Py")


# Pyclasses that exist for internal type-routing reasons but are NOT
# part of the user-facing Python module surface. Keep this list short:
# every entry is a class a future test must be told to ignore.
DELIBERATELY_PRIVATE: set[str] = set()


def _collect() -> set[str]:
    out: set[str] = set()
    for rust_src in SRC_DIR.rglob("*.rs"):
        text = rust_src.read_text(encoding="utf-8")
        for match in PYCLASS_RE.finditer(text):
            attrs, struct_name = match.group(1), match.group(2)
            out.add(_python_name(attrs, struct_name))
    return out


def _expected() -> Iterable[str]:
    return sorted(_collect() - DELIBERATELY_PRIVATE)


@pytest.mark.parametrize("name", _expected())
def test_pyclass_registered_on_module(name: str) -> None:
    mod = importlib.import_module("thetadatadx")
    assert hasattr(mod, name), (
        f"pyclass {name!r} is declared in sdks/python/src/**/*.rs but "
        f"never registered on the thetadatadx module via "
        f"`m.add_class::<{name}>()` (or via one of the `register_*` "
        f"helpers in lib.rs). Either register it, add a `register_*` "
        f"call to the `#[pymodule]` block, or — if the class is "
        f"deliberately internal — add it to `DELIBERATELY_PRIVATE` in "
        f"this test with a one-line justification."
    )


def test_scanner_finds_anchor_classes() -> None:
    """Guard against regex regression — if the scanner ever returns
    nothing, every parametrised case above would silently pass."""
    names = _collect()
    for anchor in ("Credentials", "Config", "Contract", "Subscription", "StreamingClient"):
        assert anchor in names, (
            f"scanner failed to locate `#[pyclass]` for {anchor!r}; "
            "regex regression in PYCLASS_RE?"
        )
