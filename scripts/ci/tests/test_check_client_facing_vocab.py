#!/usr/bin/env python3
"""Test suite for `scripts/ci/check_client_facing_vocab.py`.

Drives the production gate over synthetic tempdir trees and asserts:
  * a would-be leak ("FPSS event" prose in a swept README / example / .d.ts) is
    flagged;
  * a tree whose only `fpss`/`mdds` occurrences are exempt (DNS hosts, the
    metric namespace, generated-file banners, wire-include filenames, history /
    migration / internal-source paths) is clean;
  * a real leak that shares a line with an exempt span still trips;
  * an internal `src/` file with the transport names is NOT swept.

Runs as plain Python (no pytest) so CI can wire it into the same `ci.yml`
invocation as the production gate. The exit code is the total failure count.

Run::

    python3 scripts/ci/tests/test_check_client_facing_vocab.py

The production script's `--selftest` runs an in-process subset for a fast smoke
check; this file is the fuller fixture matrix.
"""

from __future__ import annotations

import pathlib
import sys

# Import the production module so the suite exercises the gate's own code path.
GATE_DIR = pathlib.Path(__file__).resolve().parent.parent
sys.path.insert(0, str(GATE_DIR))
import check_client_facing_vocab as gate  # noqa: E402

import tempfile  # noqa: E402

_fails: list[str] = []
_total = 0


def _expect(name: str, cond: bool) -> None:
    global _total
    _total += 1
    if not cond:
        _fails.append(name)
        print(f"FAIL: {name}", file=sys.stderr)


def _write(root: pathlib.Path, rel: str, body: str) -> None:
    path = root / rel
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(body, encoding="utf-8")


def _hits(root: pathlib.Path) -> list[tuple[pathlib.Path, int, str]]:
    return gate._scan(root)


def _flagged(root: pathlib.Path, rel: str) -> bool:
    return any(h[0].as_posix() == rel for h in _hits(root))


# ── Negative cases: a leak in each swept surface is flagged ─────────────────


def test_leak_in_readme_flagged() -> None:
    with tempfile.TemporaryDirectory() as tmp:
        root = pathlib.Path(tmp)
        _write(root, "thetadatadx-ffi/README.md", "Reconnect FPSS, drain the previous generation.\n")
        _expect("README leak flagged", _flagged(root, "thetadatadx-ffi/README.md"))


def test_leak_in_example_flagged() -> None:
    with tempfile.TemporaryDirectory() as tmp:
        root = pathlib.Path(tmp)
        _write(
            root,
            "thetadatadx-py/examples/quote.py",
            '"""Match-case dispatch on typed FPSS event classes."""\n',
        )
        _expect("example leak flagged", _flagged(root, "thetadatadx-py/examples/quote.py"))


def test_leak_in_dts_flagged() -> None:
    with tempfile.TemporaryDirectory() as tmp:
        root = pathlib.Path(tmp)
        _write(
            root,
            "thetadatadx-ts/streaming-session.d.ts",
            "/** tear the FPSS session down. */\nexport declare class X {}\n",
        )
        _expect(
            "published .d.ts leak flagged",
            _flagged(root, "thetadatadx-ts/streaming-session.d.ts"),
        )


def test_leak_in_openapi_flagged() -> None:
    with tempfile.TemporaryDirectory() as tmp:
        root = pathlib.Path(tmp)
        _write(
            root,
            "docs-site/docs/public/openapi.yaml",
            "paths:\n  /v3/system/fpss/status: {}\n",
        )
        _expect(
            "OpenAPI leak flagged",
            _flagged(root, "docs-site/docs/public/openapi.yaml"),
        )


def test_leak_in_config_default_flagged() -> None:
    with tempfile.TemporaryDirectory() as tmp:
        root = pathlib.Path(tmp)
        _write(
            root,
            "thetadatadx-rs/config.default.toml",
            "# THETADATA_FPSS_TYPE = prod\n",
        )
        _expect(
            "config.default.toml leak flagged",
            _flagged(root, "thetadatadx-rs/config.default.toml"),
        )


# ── Positive cases: exempt-only trees are clean ─────────────────────────────


def test_exempt_only_tokens_clean() -> None:
    with tempfile.TemporaryDirectory() as tmp:
        root = pathlib.Path(tmp)
        _write(
            root,
            "README.md",
            "Market-data host mdds-01.thetadata.us, staging mdds-stage.thetadata.us.\n"
            "Scrape thetadatadx.fpss.dropped.\n"
            "<!-- @generated from fpss_event_schema.toml -->\n"
            '#include "fpss_event_structs.h.inc"\n',
        )
        _expect("exempt-only tokens clean", _hits(root) == [])


def test_history_paths_exempt() -> None:
    with tempfile.TemporaryDirectory() as tmp:
        root = pathlib.Path(tmp)
        _write(root, "CHANGELOG.md", "Renamed THETADATA_FPSS_TYPE.\n")
        _write(root, "docs-site/docs/changelog.md", "Renamed Error::Fpss.\n")
        _write(
            root,
            "docs-site/docs/migration/v11-to-v12.md",
            "v11 used the FpssEventPoller and THETADATADX_FPSS_QUOTE.\n",
        )
        _expect("history + migration paths exempt", _hits(root) == [])


def test_contributor_and_internal_docs_exempt() -> None:
    with tempfile.TemporaryDirectory() as tmp:
        root = pathlib.Path(tmp)
        _write(root, "CONTRIBUTING.md", "Commit scope feat(fpss) / feat(mdds).\n")
        _write(root, "SECURITY.md", "The FPSS TLS connection pins the SPKI.\n")
        _write(
            root,
            "thetadatadx-rs/proto/MAINTENANCE.md",
            "The mdds.proto wire contract.\n",
        )
        _write(
            root,
            "thetadatadx-rs/benches/README.md",
            "FPSS Framing (`fpss/framing.rs`) benchmarks.\n",
        )
        _expect("contributor + internal docs exempt", _hits(root) == [])


def test_internal_src_not_swept() -> None:
    with tempfile.TemporaryDirectory() as tmp:
        root = pathlib.Path(tmp)
        # An internal binding-crate source file: keeps transport names, and is
        # not part of the swept client-facing surface.
        _write(
            root,
            "thetadatadx-ts/src/fpss_client.rs",
            "/// Start the FPSS streaming connection.\n",
        )
        _expect("internal src/ not swept", _hits(root) == [])


def test_generated_path_exempt() -> None:
    with tempfile.TemporaryDirectory() as tmp:
        root = pathlib.Path(tmp)
        _write(
            root,
            "thetadatadx-ts/src/_generated/fpss_event_classes.rs",
            "/// FPSS Quote tick.\n",
        )
        _expect("_generated path exempt", _hits(root) == [])


# ── Shared-line + clean-real-surface cases ──────────────────────────────────


def test_leak_sharing_line_with_exempt_span_trips() -> None:
    with tempfile.TemporaryDirectory() as tmp:
        root = pathlib.Path(tmp)
        _write(
            root,
            "README.md",
            "Connect to mdds-01.thetadata.us over the FPSS stream.\n",
        )
        _expect(
            "leak sharing a line with an exempt span trips",
            _flagged(root, "README.md"),
        )


def test_clean_channel_prose_passes() -> None:
    with tempfile.TemporaryDirectory() as tmp:
        root = pathlib.Path(tmp)
        _write(
            root,
            "thetadatadx-ffi/README.md",
            "Reconnect streaming, drain the previous generation.\n"
            "GET /v3/system/historical/status returns the historical status.\n",
        )
        _expect("clean channel prose passes", _hits(root) == [])


def main() -> int:
    for name, fn in sorted(globals().items()):
        if name.startswith("test_") and callable(fn):
            fn()
    if _fails:
        print(
            f"\ntest_check_client_facing_vocab: {len(_fails)}/{_total} cases FAILED",
            file=sys.stderr,
        )
        return len(_fails)
    print(f"test_check_client_facing_vocab: all {_total} cases passed")
    return 0


if __name__ == "__main__":
    sys.exit(main())
