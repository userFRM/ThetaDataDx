#!/usr/bin/env python3
"""Fail if a Windows binary imports the Visual C++ runtime.

The Windows release binaries link the MSVC C runtime statically, so they
start on a machine without the Visual C++ redistributable. GitHub's Windows
runners ship the redistributable, so a build that falls back to the dynamic
runtime still passes every test there and only fails on a clean Windows
install, with a missing VCRUNTIME140.dll. This reads the PE import table of
each binary given and fails on any vcruntime*.dll or msvcp*.dll import.

A binary with no imports at all is refused too: every Windows executable
imports at least KERNEL32.dll, so an empty list means the table was not read.

Usage:
  python scripts/ci/check_windows_crt_static.py BINARY [BINARY ...]
"""

from __future__ import annotations

import argparse
import struct
import sys
from pathlib import Path

REDISTRIBUTABLE_PREFIXES = ("vcruntime", "msvcp")


def imported_dlls(data: bytes) -> list[str]:
    """The DLL names in the import directory of a PE32 or PE32+ image."""
    if data[:2] != b"MZ":
        raise ValueError("not a PE image (no MZ header)")
    pe = struct.unpack_from("<I", data, 0x3C)[0]
    if data[pe : pe + 4] != b"PE\0\0":
        raise ValueError("not a PE image (no PE signature)")
    sections, optional_size = struct.unpack_from("<H12xH", data, pe + 6)
    optional = pe + 24
    magic = struct.unpack_from("<H", data, optional)[0]
    directories = {0x10B: optional + 96, 0x20B: optional + 112}.get(magic)
    if directories is None:
        raise ValueError(f"unknown optional header magic {magic:#x}")
    import_rva = struct.unpack_from("<I", data, directories + 8)[0]

    table = optional + optional_size

    def offset(rva: int) -> int:
        for i in range(sections):
            vsize, va, raw_size, raw = struct.unpack_from("<4I", data, table + 40 * i + 8)
            if va <= rva < va + max(vsize, raw_size):
                return raw + rva - va
        raise ValueError(f"RVA {rva:#x} lies in no section")

    names = []
    descriptor = offset(import_rva) if import_rva else None
    while descriptor is not None and any(data[descriptor : descriptor + 20]):
        name = offset(struct.unpack_from("<I", data, descriptor + 12)[0])
        names.append(data[name : data.index(b"\0", name)].decode("ascii"))
        descriptor += 20
    return names


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("binaries", nargs="+", type=Path)
    failed = False
    for path in parser.parse_args().binaries:
        dlls = imported_dlls(path.read_bytes())
        print(f"{path}: imports {', '.join(dlls) or 'nothing'}")
        runtime = [d for d in dlls if d.lower().startswith(REDISTRIBUTABLE_PREFIXES)]
        if not dlls:
            print(f"::error::{path}: no imports read from the PE import table", file=sys.stderr)
            failed = True
        elif runtime:
            print(
                f"::error::{path} imports the Visual C++ runtime ({', '.join(runtime)}), "
                "so it does not start on a Windows install without the redistributable. "
                "Link it with -C target-feature=+crt-static.",
                file=sys.stderr,
            )
            failed = True
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
