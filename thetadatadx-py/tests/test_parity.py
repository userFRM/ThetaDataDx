"""The built module against the cross-binding manifest ``parity.json``.

The manifest names the hand-written surface the Python, TypeScript and C++
bindings share. This holds the Python column of it, both ways: every listed
class, member and function exists, and a listed class has no public member
the manifest does not name.
"""

from __future__ import annotations

import json
import pathlib
import re

import pytest

thetadatadx = pytest.importorskip(
    "thetadatadx",
    reason="native thetadatadx extension not built -- run `maturin develop` from thetadatadx-py/",
)

ROOT = pathlib.Path(__file__).resolve().parents[2]
MANIFEST = json.loads((ROOT / "parity.json").read_text(encoding="utf-8"))
# The config table writes the remaining `Config` properties.
CONFIG_TABLE = set(
    re.findall(
        r"^\s*(?:set|get) thetadatadx_config_(?:set|get)_(\w+)\(",
        (ROOT / "thetadatadx-rs" / "src" / "surface" / "config.rs").read_text(encoding="utf-8"),
        re.MULTILINE,
    )
)


def python_name(name: str, spec: str | dict) -> str | None:
    """The Python spelling of a manifest entry, or ``None`` when Python lacks it."""
    if isinstance(spec, dict):
        return spec.get("python")
    if spec != "*" and "python" not in spec.split():
        return None
    return re.sub(r"^[gs]et_", "", name)


def public(obj: object) -> set[str]:
    return {n for n in dir(obj) if not n.startswith("_")}


def test_classes_and_members_match_the_manifest() -> None:
    config = thetadatadx.Config.production()
    wrong = []
    for name, entry in MANIFEST["classes"].items():
        py_name = entry.get("python", name)
        if py_name is None:
            if hasattr(thetadatadx, name):
                wrong.append(f"{name}: on the module, but the manifest lists no Python class")
            continue
        cls = getattr(thetadatadx, py_name, None)
        if not isinstance(cls, type):
            wrong.append(f"{name}: no class `{py_name}` on the module")
            continue
        expected = set()
        for member, spec in entry["members"].items():
            attr = python_name(member, spec)
            if attr is None:
                continue
            expected.add(attr)
            if not hasattr(cls, attr):
                wrong.append(f"{py_name}.{attr}: missing")
            elif member.startswith("set_"):
                # A property's descriptor exists whether or not it has a setter;
                # only `Config` carries setters.
                try:
                    setattr(config, attr, object())
                except AttributeError:
                    wrong.append(f"{py_name}.{attr}: not writable")
                except (TypeError, ValueError):
                    pass
        if "python" in entry.get("open", "").split() or entry.get("open") == "*":
            continue
        if cls is thetadatadx.Config:
            expected |= CONFIG_TABLE
        if not issubclass(cls, BaseException):
            wrong += [f"{py_name}.{attr}: not in parity.json" for attr in sorted(public(cls) - expected)]
    assert not wrong, "\n".join(wrong)


def test_functions_match_the_manifest() -> None:
    wrong = []
    for name, spec in MANIFEST["functions"].items():
        attr = python_name(name, spec)
        if attr is None:
            if hasattr(thetadatadx.util, name) or hasattr(thetadatadx, name):
                wrong.append(f"{name}: present, but the manifest lists no Python function")
        elif not callable(getattr(thetadatadx.util, attr, None) or getattr(thetadatadx, attr, None)):
            wrong.append(f"{attr}: missing from `thetadatadx.util` and the module")
    listed = {python_name(n, s) for n, s in MANIFEST["functions"].items()}
    wrong += [f"util.{attr}: not in parity.json" for attr in sorted(public(thetadatadx.util) - listed)]
    assert not wrong, "\n".join(wrong)


def test_subscription_kinds_match_the_manifest() -> None:
    contract = thetadatadx.Contract.stock("AAPL")
    kinds = [getattr(contract, k)().kind for k in ("quote", "trade", "open_interest", "market_value")]
    sec_type = thetadatadx.SecType.OPTION
    kinds += [getattr(sec_type, k)().kind for k in ("full_trades", "full_open_interest")]
    assert kinds == MANIFEST["subscription_kinds"]


def test_no_public_name_embeds_an_implementation_detail() -> None:
    names = public(thetadatadx) | public(thetadatadx.util)
    for name in list(names):
        if isinstance(getattr(thetadatadx, name, None), type):
            names |= public(getattr(thetadatadx, name))
    leaked = sorted(n for n in names for token in MANIFEST["banned_identifier_tokens"] if token in n.lower())
    assert not leaked, leaked
