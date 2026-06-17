"""Regression guard against `#[pyclass]` Python-name collisions.

Two pyclasses registered under the same Python name silently shadow each
other — `m.add_class` is last-write-wins, so whichever helper runs second
in `lib.rs` wins, and the user-facing surface loses whatever methods the
earlier registration carried (factory methods, getters, etc.).

This test scans every `#[pyclass]` in the Python SDK's Rust sources and
asserts no two structs map to the same Python-facing name. The mapping
mirrors how pyo3 resolves the public name:
    1. If the attribute carries `name = "..."`, that wins.
    2. Otherwise the bare struct identifier becomes the Python name,
       with the leading `Py` prefix stripped (project convention used by
       `PyContract`, `PySubscription`, etc.).

If this fires, add an explicit `name = "..."` to the offending pyclass
to disambiguate. Do NOT silence the test — name collisions break the
fluent surface even when the build is green.
"""

from __future__ import annotations

import collections
import pathlib
import re

# `#[pyclass(...)] <attrs> pub(crate)? struct <Name>`
# The attribute block can contain commas, quoted strings, balanced brackets,
# and span multiple lines — we tolerate all of that and pull out the
# optional `name = "..."` override plus the struct identifier.
PYCLASS_RE = re.compile(
    r"#\[pyclass(?:\(([^)]*)\))?\][^{]*?"
    r"(?:pub(?:\(crate\))?\s+)?struct\s+(\w+)",
    re.MULTILINE | re.DOTALL,
)
NAME_ATTR_RE = re.compile(r'name\s*=\s*"([^"]+)"')

SRC_DIR = pathlib.Path(__file__).resolve().parents[1] / "src"


def _python_name(attrs: str | None, struct_name: str) -> str:
    """Resolve the Python-facing class name from the `#[pyclass]` attrs."""
    if attrs:
        match = NAME_ATTR_RE.search(attrs)
        if match:
            return match.group(1)
    # Convention: bare struct named `PyFoo` → Python sees it as `Foo`.
    return struct_name.removeprefix("Py")


def _collect_python_names() -> dict[str, list[str]]:
    seen: dict[str, list[str]] = collections.defaultdict(list)
    for rust_src in SRC_DIR.rglob("*.rs"):
        text = rust_src.read_text(encoding="utf-8")
        for match in PYCLASS_RE.finditer(text):
            attrs, struct_name = match.group(1), match.group(2)
            py_name = _python_name(attrs, struct_name)
            rel = rust_src.relative_to(SRC_DIR)
            seen[py_name].append(f"{rel}::{struct_name}")
    return seen


def test_no_pyclass_name_collisions() -> None:
    names = _collect_python_names()
    collisions = {k: v for k, v in names.items() if len(v) > 1}
    assert not collisions, (
        "Multiple #[pyclass] structs register the same Python name. "
        "pyo3's m.add_class is last-write-wins, so the second registration "
        "silently shadows the first, dropping the earlier registration's "
        "methods from the user-facing surface. Fix by adding an explicit "
        '`name = "..."` attribute to disambiguate.\n'
        f"Collisions: {collisions}"
    )


def test_scanner_finds_expected_pyclasses() -> None:
    """Sanity check: the regex actually matches the pyclasses we know
    are there. Without this, a future regex regression could pass the
    collision check by finding *nothing*."""
    names = _collect_python_names()
    # `Contract` (fluent builder) and `ContractRef` (event payload) — the
    # two that motivated this guard. Plus a couple of generic SDK types
    # to anchor the scanner against accidental zero-match silence.
    for expected in ("Contract", "ContractRef", "Credentials", "Subscription"):
        assert expected in names, (
            f"scanner failed to locate #[pyclass] for {expected!r}; "
            "regex regression?"
        )
