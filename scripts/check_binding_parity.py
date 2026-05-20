#!/usr/bin/env python3
"""Cross-binding parity check (Gate 2 / issue #545).

Reads `sdks/parity.toml` — the declared cross-binding presence matrix
— and compares each row's `python` / `typescript` / `cpp` claims to
the actual binding state extracted from:

- Python: every `m.add_class::<T>()` registered in `lib.rs` + helper
  `register_*` calls, expanded statically by parsing the Rust source.
  Mirrors the regex powering `test_no_pyclass_name_collisions.py`.
- TypeScript: `export declare class X` / `export class X` declarations
  in `sdks/typescript/index.d.ts`.
- C++: `^class X` / `^struct X` declarations in
  `sdks/cpp/include/thetadx.hpp`. The `.h` header is C-only and not
  considered for parity.

Exits non-zero on any mismatch. Run from the repo root.
"""

from __future__ import annotations

import pathlib
import re
import sys
import tomllib
from typing import Any


REPO_ROOT = pathlib.Path(__file__).resolve().parents[1]
PARITY_TOML = REPO_ROOT / "sdks" / "parity.toml"
PY_SRC = REPO_ROOT / "sdks" / "python" / "src"
TS_DTS = REPO_ROOT / "sdks" / "typescript" / "index.d.ts"
CPP_HEADER = REPO_ROOT / "sdks" / "cpp" / "include" / "thetadx.hpp"


PYCLASS_RE = re.compile(
    r"#\[pyclass(?:\(([^)]*)\))?\][^{]*?"
    r"(?:pub(?:\(crate\))?\s+)?struct\s+(\w+)",
    re.MULTILINE | re.DOTALL,
)
NAME_ATTR_RE = re.compile(r'name\s*=\s*"([^"]+)"')


def _python_name(attrs: str | None, struct_name: str) -> str:
    if attrs:
        m = NAME_ATTR_RE.search(attrs)
        if m:
            return m.group(1)
    return struct_name.removeprefix("Py")


def collect_python() -> set[str]:
    """Python-side pyclasses, in the same way `m.add_class::<T>()`
    would surface them."""
    out: set[str] = set()
    for rs in PY_SRC.rglob("*.rs"):
        text = rs.read_text(encoding="utf-8")
        for m in PYCLASS_RE.finditer(text):
            out.add(_python_name(m.group(1), m.group(2)))
    # The errors module registers each exception via `m.add(...)` not
    # `m.add_class`; parse the matching helper.
    errors_rs = (PY_SRC / "errors.rs").read_text(encoding="utf-8")
    for m in re.finditer(r'm\.add\(\s*"(\w+)"\s*,', errors_rs):
        out.add(m.group(1))
    return out


TS_CLASS_RE = re.compile(r"export\s+declare\s+class\s+(\w+)")
TS_ALIAS_RE = re.compile(r"export\s+const\s+(\w+)\s*=\s*(\w+)")
TS_INTERFACE_RE = re.compile(r"export\s+(?:declare\s+)?interface\s+(\w+)")


def collect_typescript() -> set[str]:
    out: set[str] = set()
    if not TS_DTS.is_file():
        return out
    text = TS_DTS.read_text(encoding="utf-8")
    for m in TS_CLASS_RE.finditer(text):
        out.add(m.group(1))
    for m in TS_INTERFACE_RE.finditer(text):
        out.add(m.group(1))
    # `index.js` post-processor emits `Contract = ContractRef` aliases;
    # capture them so the parity check sees the user-facing name.
    js_path = TS_DTS.with_name("index.js")
    if js_path.is_file():
        for m in re.finditer(r"exports\.(\w+)\s*=\s*\w+", js_path.read_text(encoding="utf-8")):
            out.add(m.group(1))
    return out


CPP_CLASS_RE = re.compile(r"^(?:class|struct)\s+(\w+)", re.MULTILINE)
CPP_USING_RE = re.compile(r"^using\s+(\w+)\s*=", re.MULTILINE)


def collect_cpp() -> set[str]:
    out: set[str] = set()
    if not CPP_HEADER.is_file():
        return out
    text = CPP_HEADER.read_text(encoding="utf-8")
    for m in CPP_CLASS_RE.finditer(text):
        out.add(m.group(1))
    # `using X = ...` aliases (e.g. `using QuoteTick = TdxQuoteTick;`).
    for m in CPP_USING_RE.finditer(text):
        out.add(m.group(1))
    return out


# C++ uses some renamed identifiers — record the equivalences here so
# the parity check accepts "C++ ships an equivalent under a different
# class name" without requiring a rename of the production header.
CPP_ALIASES: dict[str, str] = {
    "ThetaDataDxClient": "UnifiedClient",
    "FlatFilesNamespace": "FlatFiles",
    "StreamingIterSession": "UnifiedFpssIterSession",
    "Contract": "FluentContract",
    "Subscription": "FluentSubscription",
    "SecType": "FluentSecType",
}


def cpp_has(symbol: str, cpp: set[str]) -> bool:
    if symbol in cpp:
        return True
    alias = CPP_ALIASES.get(symbol)
    if alias and alias in cpp:
        return True
    return False


def main() -> int:
    if not PARITY_TOML.is_file():
        print(f"missing parity matrix: {PARITY_TOML}", file=sys.stderr)
        return 1

    data: dict[str, Any] = tomllib.loads(PARITY_TOML.read_text(encoding="utf-8"))
    rows: list[dict[str, Any]] = data.get("class", [])
    if not rows:
        print("parity.toml has no [[class]] rows", file=sys.stderr)
        return 1

    py = collect_python()
    ts = collect_typescript()
    cpp = collect_cpp()

    mismatches: list[tuple[str, str, bool, bool]] = []
    for row in rows:
        name = row["name"]
        for lang, declared in (("python", row["python"]), ("typescript", row["typescript"]), ("cpp", row["cpp"])):
            if lang == "python":
                actual = name in py
            elif lang == "typescript":
                actual = name in ts
            else:
                actual = cpp_has(name, cpp)
            if actual != declared:
                mismatches.append((name, lang, declared, actual))

    if mismatches:
        print(f"check_binding_parity: {len(mismatches)} mismatch(es) vs sdks/parity.toml:")
        for name, lang, declared, actual in mismatches:
            verb = "missing" if declared and not actual else "unexpected"
            print(f"  {name}.{lang}: declared={declared}, actual={actual} ({verb})")
        print(
            "\nFix: either land the missing binding, or update "
            "sdks/parity.toml to reflect the intended state. Every "
            "cross-binding asymmetry must be explicit + tracked."
        )
        return 1

    print(
        f"check_binding_parity: clean "
        f"({len(rows)} rows checked, py={len(py)} ts={len(ts)} cpp={len(cpp)})"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
