"""The type stub's stub-only base classes against the built module.

Several stub classes take their methods from private bases that exist only in
the stub: the tick lists from ``_TickList``, the builders from ``_Builder``,
``MarketDataView`` its awaitable methods from ``_AsyncMarketData`` and
``StreamView`` its methods from ``_StreamMethods``. stubtest checks such an
inherited method only when the runtime class has it, so one the runtime
dropped would go unreported there.
"""

from __future__ import annotations

import ast
import pathlib

import pytest

thetadatadx = pytest.importorskip(
    "thetadatadx",
    reason="native thetadatadx extension not built -- run `maturin develop` from thetadatadx-py/",
)

STUB = pathlib.Path(__file__).resolve().parents[1] / "python" / "thetadatadx" / "__init__.pyi"


def test_stub_only_bases_declare_only_runtime_methods() -> None:
    classes = {
        node.name: node
        for node in ast.parse(STUB.read_text(encoding="utf-8")).body
        if isinstance(node, ast.ClassDef)
    }

    def inherited(node: ast.ClassDef) -> set[str]:
        names: set[str] = set()
        for base in node.bases:
            if isinstance(base, ast.Subscript):
                base = base.value
            if isinstance(base, ast.Name) and base.id.startswith("_") and base.id in classes:
                parent = classes[base.id]
                names |= {item.name for item in parent.body if isinstance(item, ast.FunctionDef)}
                names |= inherited(parent)
        return names

    # `AsyncClient` reaches what it inherits through `__getattr__`; the classes
    # that define those methods are checked here.
    checked = {
        name: inherited(node)
        for name, node in classes.items()
        if not name.startswith("_") and name != "AsyncClient"
    }
    assert any(checked.values()), "no class in the stub inherits from a stub-only base"

    missing = sorted(
        f"{name}.{method}"
        for name, methods in checked.items()
        for method in methods
        if not hasattr(getattr(thetadatadx, name), method)
    )
    assert not missing, f"the stub declares methods the runtime classes lack: {missing}"
