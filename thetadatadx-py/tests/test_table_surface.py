"""The built module against the core tables the PyO3 crate expands.

stubtest does not report a member the runtime has and the type stub lacks,
such as a ``stream`` terminal on the builder of an endpoint that does not
stream, and sees the ``Config`` accessors as plain attributes, so not whether a
property can be written. These hold the module itself to the endpoint and
config tables.
"""

from __future__ import annotations

import inspect
import pathlib
import re

import pytest

thetadatadx = pytest.importorskip(
    "thetadatadx",
    reason="native thetadatadx extension not built -- run `maturin develop` from thetadatadx-py/",
)

SURFACE = pathlib.Path(__file__).resolve().parents[2] / "thetadatadx-rs" / "src" / "surface"


def test_builders_follow_the_endpoint_table() -> None:
    """Every endpoint has a builder with ``list`` and ``list_async``,
    ``stream`` and ``stream_async`` on exactly the ``chunked`` ones, and a
    setter for every keyword argument its method takes."""
    text = (SURFACE / "endpoints.rs").read_text(encoding="utf-8")
    table = text[text.index("macro_rules! __endpoints") : text.index("macro_rules! __stream_endpoints")]
    shapes = {
        name: shape
        for shape, name in re.findall(r"^\s*(list|buffered|chunked)\s+(\w+)\s*\{", table, re.MULTILINE)
    }
    assert shapes, "no endpoint entries read from endpoints.rs"

    wrong = []
    for name, shape in sorted(shapes.items()):
        builder_name = "".join(part.capitalize() for part in name.split("_")) + "Builder"
        builder = getattr(thetadatadx, builder_name, None)
        if builder is None:
            wrong.append(f"{name}: no `{builder_name}` on the module")
            continue
        expected = {"list", "list_async"} | ({"stream", "stream_async"} if shape == "chunked" else set())
        actual = {t for t in ("list", "list_async", "stream", "stream_async") if hasattr(builder, t)}
        if actual != expected:
            wrong.append(f"{builder_name}: has {sorted(actual)}, a {shape} endpoint has {sorted(expected)}")
        parameters = inspect.signature(getattr(thetadatadx.MarketDataView, name)).parameters.values()
        keywords = [p.name for p in parameters if p.kind is p.KEYWORD_ONLY]
        missing = [k for k in keywords if not hasattr(builder, k)]
        if "timeout_ms" not in keywords or missing:
            wrong.append(f"{builder_name}: no setter for {missing} of the keywords `{name}()` takes ({keywords})")
    assert not wrong, "\n".join(wrong)


def test_config_follows_the_config_table() -> None:
    """Every ``get`` entry is a readable ``Config`` property, and every
    ``set`` entry takes back the value its getter returns."""
    entries = re.findall(
        r"^\s*(set|get) thetadatadx_config_(?:set|get)_(\w+)\(",
        (SURFACE / "config.rs").read_text(encoding="utf-8"),
        re.MULTILINE,
    )
    assert entries, "no config entries read from config.rs"

    config = thetadatadx.Config.production()
    wrong = []
    for direction, name in entries:
        try:
            value = getattr(config, name)
            if direction == "set":
                setattr(config, name, value)
        except (AttributeError, TypeError, ValueError) as exc:
            wrong.append(f"{direction} {name}: {exc}")
    assert not wrong, "\n".join(wrong)
