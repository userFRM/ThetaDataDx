#!/usr/bin/env python3
"""Gate documented config defaults against the Rust source of truth.

Every binding surface (FFI doc comments, the Python ``.pyi`` and PyO3
docstrings, the NAPI docstrings that generate ``index.d.ts``, the C++
Doxygen, and the config-crate docstrings themselves) advertises a
default value for each tuning knob: ``Default 250``, ``defaults to 30``,
``default 30_000``, and so on. The single source of truth for those
values is the Rust config crate ``crates/thetadatadx/src/config/*.rs`` —
the ``impl Default`` / ``production_defaults`` constructors and the const
bounds. A documented default that disagrees with the constructor is a
shipped lie: it tells a caller the SDK behaves one way while the binary
behaves another.

This gate closes that gap by construction:

* Parse the canonical default for each config field straight out of the
  Rust constructors, normalising ``_`` digit separators and the
  ``Duration::from_secs`` / ``Duration::from_millis`` unit wrappers into
  a plain integer.
* For each field a binding surface documents, locate the ``Default N``
  token next to its setter / field / accessor and assert it equals the
  canonical value, in the unit that surface documents.
* Exit non-zero with ``file:line`` for every mismatch.

A field whose default is genuinely environment-dependent (no single
literal — e.g. ``concurrent_requests = 0`` meaning "auto-detect from the
subscription tier") is registered with an explicit skip reason rather
than forced into a false match.

Run::

    python3 scripts/check_doc_defaults.py

Exit codes:

* ``0`` — every documented default matches the source of truth.
* ``1`` — at least one documented default drifted, or a field expected
  to be documented on a surface could not be found (a silent-drop
  regression).

Selftest::

    python3 scripts/check_doc_defaults.py --selftest

The selftest plants a wrong documented default in a synthetic tree,
confirms the gate catches it, then confirms the gate passes once the
plant is corrected.
"""

from __future__ import annotations

import argparse
import pathlib
import re
import sys
from dataclasses import dataclass, field
from typing import Callable, Optional

REPO_ROOT = pathlib.Path(__file__).resolve().parents[1]

CONFIG_DIR = pathlib.Path("crates/thetadatadx/src/config")


# ── Canonical-value parsing ──────────────────────────────────────────


def _norm_int(literal: str) -> int:
    """Normalise a Rust integer literal: strip ``_`` separators and any
    type suffix (``30_000u64`` → ``30000``)."""
    cleaned = literal.replace("_", "")
    m = re.match(r"^(\d+)", cleaned)
    if not m:
        raise ValueError(f"not an integer literal: {literal!r}")
    return int(m.group(1))


# A canonical-value extractor reads a logical field's default out of the
# Rust source. Each returns an integer in the unit the *bindings*
# document for that field (ms for `*_ms` knobs, seconds for `*_secs`,
# bytes for byte knobs, a plain count otherwise).
CanonExtractor = Callable[[dict[str, str]], int]


@dataclass
class CanonField:
    """One canonical config default, sourced from the Rust constructors."""

    field_id: str
    # Human-facing unit, only used in diagnostics.
    unit: str
    extractor: CanonExtractor


def _read(rel: pathlib.Path, root: pathlib.Path) -> str:
    return (root / rel).read_text(encoding="utf-8")


def _struct_literal_fields(body: str) -> dict[str, str]:
    """Map ``field: <expr>,`` assignments inside a struct literal body to
    the raw right-hand-side expression text (one logical line each)."""
    out: dict[str, str] = {}
    for m in re.finditer(
        r"^\s*([a-z_][a-z0-9_]*)\s*:\s*(.+?),\s*(?://.*)?$",
        body,
        re.MULTILINE,
    ):
        out[m.group(1)] = m.group(2).strip()
    return out


def _block_after(text: str, anchor: str) -> str:
    """Return the balanced ``{ ... }`` block that follows ``anchor`` in
    ``text`` (the constructor body)."""
    idx = text.index(anchor)
    brace = text.index("{", idx)
    depth = 0
    for i in range(brace, len(text)):
        c = text[i]
        if c == "{":
            depth += 1
        elif c == "}":
            depth -= 1
            if depth == 0:
                return text[brace + 1 : i]
    raise ValueError(f"unbalanced block after anchor {anchor!r}")


def _duration_secs(expr: str) -> int:
    m = re.search(r"Duration::from_secs\(\s*([0-9_]+)\s*\)", expr)
    if not m:
        raise ValueError(f"not a from_secs duration: {expr!r}")
    return _norm_int(m.group(1))


def _duration_millis(expr: str) -> int:
    m = re.search(r"Duration::from_millis\(\s*([0-9_]+)\s*\)", expr)
    if not m:
        raise ValueError(f"not a from_millis duration: {expr!r}")
    return _norm_int(m.group(1))


def load_canonical(root: pathlib.Path = REPO_ROOT) -> dict[str, int]:
    """Parse every canonical config default out of the Rust constructors.

    Returns ``{field_id: value_in_documented_unit}``. Raises if a field
    the gate expects cannot be parsed — a constructor refactor that
    drops or renames a field surfaces as a gate failure rather than a
    silently stale canonical map.
    """
    canon: dict[str, int] = {}

    # FpssConfig::production_defaults — millisecond / second / count
    # knobs documented verbatim by the bindings.
    streaming = _struct_literal_fields(
        _block_after(_read(CONFIG_DIR / "fpss.rs", root), "fn production_defaults()")
    )
    canon["streaming.timeout_ms"] = _norm_int(streaming["timeout_ms"])
    canon["streaming.ring_size"] = _norm_int(streaming["ring_size"])
    canon["streaming.ping_interval_ms"] = _norm_int(streaming["ping_interval_ms"])
    canon["streaming.connect_timeout_ms"] = _norm_int(streaming["connect_timeout_ms"])
    canon["streaming.io_read_slice_ms"] = _norm_int(streaming["io_read_slice_ms"])
    canon["streaming.data_watchdog_ms"] = _norm_int(streaming["data_watchdog_ms"])
    canon["streaming.keepalive_idle_secs"] = _norm_int(streaming["keepalive_idle_secs"])
    canon["streaming.keepalive_interval_secs"] = _norm_int(streaming["keepalive_interval_secs"])
    canon["streaming.keepalive_retries"] = _norm_int(streaming["keepalive_retries"])

    # FpssConfig bounds — the documented FLATFILES / FPSS validation
    # range that a doc comment also asserts.
    streaming_src = _read(CONFIG_DIR / "fpss.rs", root)

    # FlatFilesConfig::production_defaults — second-unit Durations + count.
    ff = _struct_literal_fields(
        _block_after(
            _read(CONFIG_DIR / "flatfiles.rs", root), "fn production_defaults()"
        )
    )
    canon["flatfiles.max_attempts"] = _norm_int(ff["max_attempts"])
    canon["flatfiles.initial_backoff_secs"] = _duration_secs(ff["initial_backoff"])
    canon["flatfiles.max_backoff_secs"] = _duration_secs(ff["max_backoff"])

    # FlatFilesConfig bounds — `MAX_ATTEMPTS: ... = 1..=100`.
    ff_src = _read(CONFIG_DIR / "flatfiles.rs", root)
    m = re.search(r"MAX_ATTEMPTS:[^=]*=\s*([0-9_]+)\s*..=\s*([0-9_]+)", ff_src)
    if not m:
        raise ValueError("could not parse flatfiles MAX_ATTEMPTS bounds")
    canon["flatfiles.max_attempts.range_lo"] = _norm_int(m.group(1))
    canon["flatfiles.max_attempts.range_hi"] = _norm_int(m.group(2))

    # ReconnectConfig::production_defaults — millisecond cadence knobs.
    rc = _struct_literal_fields(
        _block_after(
            _read(CONFIG_DIR / "reconnect.rs", root), "fn production_defaults()"
        )
    )
    canon["reconnect.wait_ms"] = _norm_int(rc["wait_ms"])
    canon["reconnect.wait_max_ms"] = _norm_int(rc["wait_max_ms"])
    canon["reconnect.wait_rate_limited_ms"] = _norm_int(rc["wait_rate_limited_ms"])
    canon["reconnect.wait_server_restart_ms"] = _norm_int(rc["wait_server_restart_ms"])
    canon["reconnect.replay_burst_size"] = _norm_int(rc["replay_burst_size"])
    canon["reconnect.replay_pace_ms"] = _norm_int(rc["replay_pace_ms"])

    # ReconnectAttemptLimits::default — per-class attempt budgets + windows.
    ral = _struct_literal_fields(
        _block_after(
            _read(CONFIG_DIR / "reconnect.rs", root),
            "impl Default for ReconnectAttemptLimits",
        )
    )
    canon["reconnect.max_attempts"] = _norm_int(ral["max_attempts"])
    canon["reconnect.max_rate_limited_attempts"] = _norm_int(
        ral["max_rate_limited_attempts"]
    )
    canon["reconnect.max_server_restart_attempts"] = _norm_int(
        ral["max_server_restart_attempts"]
    )
    canon["reconnect.max_elapsed_secs"] = _duration_secs(ral["max_elapsed"])
    canon["reconnect.stable_window_secs"] = _duration_secs(ral["stable_window"])

    # RetryPolicy::default — MDDS historical retry ladder.
    rp = _struct_literal_fields(
        _block_after(
            _read(CONFIG_DIR / "retry.rs", root), "impl Default for RetryPolicy"
        )
    )
    canon["retry.initial_delay_ms"] = _duration_millis(rp["initial_delay"])
    canon["retry.max_delay_ms"] = _duration_secs(rp["max_delay"]) * 1000
    canon["retry.max_attempts"] = _norm_int(rp["max_attempts"])
    canon["retry.max_elapsed_secs"] = _duration_secs(rp["max_elapsed"])

    # MddsConfig::production_defaults — byte / second knobs.
    historical = _struct_literal_fields(
        _block_after(
            _read(CONFIG_DIR / "mdds.rs", root), "fn production_defaults()"
        )
    )
    canon["historical.connect_timeout_secs"] = _norm_int(historical["connect_timeout_secs"])
    canon["historical.keepalive_secs"] = _norm_int(historical["keepalive_secs"])
    canon["historical.keepalive_timeout_secs"] = _norm_int(historical["keepalive_timeout_secs"])
    canon["historical.window_size_kb"] = _norm_int(historical["window_size_kb"])
    canon["historical.connection_window_size_kb"] = _norm_int(
        historical["connection_window_size_kb"]
    )
    # warn_on_buffered_threshold_bytes = 100 * 1024 * 1024 (100 MiB).
    wob = historical["warn_on_buffered_threshold_bytes"]
    m = re.match(r"^([0-9_]+)\s*\*\s*1024\s*\*\s*1024$", wob)
    if not m:
        raise ValueError(f"unexpected warn-threshold literal: {wob!r}")
    canon["historical.warn_on_buffered_threshold_mib"] = _norm_int(m.group(1))

    del streaming_src  # parsed lazily above; kept for future bound checks.
    return canon


# ── Surface checks ───────────────────────────────────────────────────


@dataclass
class Mismatch:
    surface: str
    rel_path: str
    lineno: int
    field_id: str
    documented: int
    canonical: int
    detail: str = ""


@dataclass
class SurfaceField:
    """One documented default the gate expects to find on a surface.

    ``anchor`` is a regex that uniquely locates the setter / field /
    accessor for ``field_id``; the gate searches forward (and, for
    accessor-style getters, the same line) for the ``Default N`` token
    and compares it to ``canonical[field_id]``.
    """

    field_id: str
    anchor: re.Pattern
    # Some surfaces document the same field twice (a setter doc + a
    # getter doc). Both must match; ``min_hits`` guards against a
    # silent drop where neither is found.
    min_hits: int = 1


# Matches a documented default in any of the shipped phrasings, with the
# value possibly carrying `_` separators and (TS) a BigInt `n` suffix.
DEFAULT_TOKEN_RE = re.compile(
    r"(?:[Dd]efault(?:s)?(?:\s+(?:is|of|to))?|[Dd]efault\s+`?)\s*`?"
    r"(?P<value>\d[\d_]*)\s*n?\b"
)


@dataclass
class Surface:
    name: str
    rel_path: pathlib.Path
    # Where the documentation that carries the ``Default`` token sits
    # relative to the declaration the anchor matches. Rust / C++ doc
    # comments precede the item (``"above"``); the ``.pyi`` attribute
    # docstring follows the typed attribute (``"below"``). Declaring the
    # direction removes the ambiguity of a previous field's docstring
    # sitting immediately above the next field's declaration.
    doc_direction: str = "above"
    fields: list[SurfaceField] = field(default_factory=list)
    # Fields documented on this surface that are deliberately NOT a
    # single literal: {field_id: reason}.
    skips: dict[str, str] = field(default_factory=dict)


def _find_defaults_for_anchor(
    lines: list[str], anchor: re.Pattern, direction: str = "above"
) -> list[tuple[int, int]]:
    """Return ``(lineno, value)`` for each ``Default N`` token that
    annotates the *declaration* of ``anchor``.

    The anchor must land on a declaration line — a Rust ``fn`` /
    extern signature, a C declaration, or a typed ``.pyi`` attribute —
    not on an incidental mention inside another item's doc comment
    (``capped at thetadatadx_config_set_retry_max_delay_ms``). Once anchored, the
    gate reads the ``Default`` token from the documentation attached to
    that declaration, on the side named by ``direction``:

    * ``"above"`` — Rust / C++ doc comments precede the item; the gate
      walks up through the contiguous comment block, skipping ``#[...]``
      attribute lines that separate the doc from the ``fn``.
    * ``"below"`` — the ``.pyi`` attribute docstring follows the typed
      attribute; the gate reads the immediately-following comment block.

    A declaration that legitimately documents no default yields no hits
    and the caller's ``min_hits`` guard decides whether that is a
    regression.
    """
    hits: list[tuple[int, int]] = []
    for i, line in enumerate(lines):
        if not anchor.search(line):
            continue
        if _is_doc_or_comment(line):
            # Anchor matched inside a doc comment (a cross-reference to
            # another setter), not on a real declaration. Skip it.
            continue
        window: list[tuple[int, str]] = [(i, line)]
        if direction == "above":
            j = i - 1
            while j >= 0 and (
                _is_doc_or_comment(lines[j]) or _is_attribute(lines[j])
            ):
                window.append((j, lines[j]))
                j -= 1
        else:  # "below"
            k = i + 1
            while k < len(lines) and _is_doc_or_comment(lines[k]):
                window.append((k, lines[k]))
                k += 1
        # Per-line scan catches the common single-line ``Default N``.
        per_line: set[tuple[int, int]] = set()
        for lineno, text in window:
            for m in DEFAULT_TOKEN_RE.finditer(text):
                per_line.add((lineno + 1, _norm_int(m.group("value"))))
        # A doc comment may wrap ``Default`` onto one line and the value
        # onto the next (``... Default`` / ``` `10`. Validated ...```).
        # Re-scan the de-commented, whitespace-collapsed block so the
        # wrapped form is caught too; attribute the hit to the anchor
        # line since the exact wrap point is not load-bearing.
        joined = _join_comment_block(window)
        for m in DEFAULT_TOKEN_RE.finditer(joined):
            value = _norm_int(m.group("value"))
            if not any(v == value for (_, v) in per_line):
                per_line.add((i + 1, value))
        hits.extend(sorted(per_line))
    return hits


def _strip_comment_prefix(line: str) -> str:
    """Remove a leading Rust / C++ / docstring comment marker so the
    prose reflows into a single sentence for the wrapped-default scan."""
    s = line.strip()
    for prefix in ("///", "//!", "//", "*/", "/**", "/*", "*", 'r"""', '"""'):
        if s.startswith(prefix):
            s = s[len(prefix):]
            break
    return s.strip().rstrip('"')


def _join_comment_block(window: list[tuple[int, str]]) -> str:
    # ``window`` is anchor-first then upward (or downward) order; sort by
    # line number so reflowed prose reads top-to-bottom.
    ordered = sorted(window, key=lambda pair: pair[0])
    return " ".join(_strip_comment_prefix(text) for _, text in ordered)


def _is_attribute(line: str) -> bool:
    s = line.strip()
    return s.startswith("#[") or s.startswith("#![")


def _is_doc_or_comment(line: str) -> bool:
    s = line.strip()
    if not s:
        return False
    return (
        s.startswith("///")
        or s.startswith("//")
        or s.startswith("*")
        or s.startswith("/*")
        or s.startswith('"""')
        or s.startswith('r"""')
    )


def check_surface(
    surface: Surface, canon: dict[str, int], root: pathlib.Path
) -> list[Mismatch]:
    mismatches: list[Mismatch] = []
    text = (root / surface.rel_path).read_text(encoding="utf-8")
    lines = text.splitlines()
    rel = surface.rel_path.as_posix()
    for sf in surface.fields:
        if sf.field_id in surface.skips:
            continue
        canonical = canon.get(sf.field_id)
        if canonical is None:
            mismatches.append(
                Mismatch(
                    surface.name,
                    rel,
                    0,
                    sf.field_id,
                    -1,
                    -1,
                    detail="no canonical value parsed for this field",
                )
            )
            continue
        hits = _find_defaults_for_anchor(lines, sf.anchor, surface.doc_direction)
        if len(hits) < sf.min_hits:
            mismatches.append(
                Mismatch(
                    surface.name,
                    rel,
                    0,
                    sf.field_id,
                    -1,
                    canonical,
                    detail=(
                        f"expected >= {sf.min_hits} documented 'Default' "
                        f"token(s) for this field, found {len(hits)} "
                        "(silent-drop regression?)"
                    ),
                )
            )
            continue
        for lineno, value in hits:
            if value != canonical:
                mismatches.append(
                    Mismatch(
                        surface.name, rel, lineno, sf.field_id, value, canonical
                    )
                )
    return mismatches


# ── Surface registry ─────────────────────────────────────────────────


def _re(pattern: str) -> re.Pattern:
    return re.compile(pattern)


def build_surfaces() -> list[Surface]:
    """Declare which documented defaults the gate verifies on each
    binding surface, keyed to the canonical field id.

    The anchor regexes target the setter symbol (FFI/PyO3/NAPI/C++) or
    the typed attribute (``.pyi``). Getter/setter pairs that both carry
    a ``Default`` token are covered by the window search in
    ``_find_defaults_for_anchor`` — the gate compares every token it
    finds in the field's doc/accessor block.
    """
    surfaces: list[Surface] = []

    # FFI doc comments (`thetadatadx_config_set_*`). The setters carry the
    # canonical doc; many getters echo `(default N)` inline and are
    # caught by the same anchor's window.
    ffi = Surface("ffi", pathlib.Path("ffi/src/auth.rs"))
    ffi.fields = [
        SurfaceField("reconnect.max_attempts", _re(r"set_reconnect_max_attempts\b")),
        SurfaceField(
            "reconnect.max_rate_limited_attempts",
            _re(r"set_reconnect_max_rate_limited_attempts\b"),
        ),
        SurfaceField(
            "reconnect.max_server_restart_attempts",
            _re(r"set_reconnect_max_server_restart_attempts\b"),
        ),
        SurfaceField(
            "reconnect.stable_window_secs",
            _re(r"set_reconnect_stable_window_secs\b"),
        ),
        SurfaceField("reconnect.wait_ms", _re(r"set_reconnect_wait_ms\b")),
        SurfaceField(
            "reconnect.wait_rate_limited_ms",
            _re(r"set_reconnect_wait_rate_limited_ms\b"),
        ),
        SurfaceField(
            "reconnect.max_elapsed_secs", _re(r"set_reconnect_max_elapsed_secs\b")
        ),
        SurfaceField("reconnect.wait_max_ms", _re(r"set_reconnect_wait_max_ms\b")),
        SurfaceField(
            "reconnect.wait_server_restart_ms",
            _re(r"set_reconnect_wait_server_restart_ms\b"),
        ),
        SurfaceField(
            "reconnect.replay_burst_size", _re(r"set_reconnect_replay_burst_size\b")
        ),
        SurfaceField(
            "reconnect.replay_pace_ms", _re(r"set_reconnect_replay_pace_ms\b")
        ),
        SurfaceField("streaming.timeout_ms", _re(r"set_streaming_timeout_ms\b")),
        SurfaceField("streaming.connect_timeout_ms", _re(r"set_streaming_connect_timeout_ms\b")),
        SurfaceField("streaming.ping_interval_ms", _re(r"set_streaming_ping_interval_ms\b")),
        SurfaceField("streaming.io_read_slice_ms", _re(r"set_streaming_io_read_slice_ms\b")),
        SurfaceField("streaming.data_watchdog_ms", _re(r"set_streaming_data_watchdog_ms\b")),
        SurfaceField(
            "streaming.keepalive_idle_secs", _re(r"set_streaming_keepalive_idle_secs\b")
        ),
        SurfaceField(
            "streaming.keepalive_interval_secs", _re(r"set_streaming_keepalive_interval_secs\b")
        ),
        SurfaceField("streaming.keepalive_retries", _re(r"set_streaming_keepalive_retries\b")),
        SurfaceField("retry.initial_delay_ms", _re(r"set_retry_initial_delay_ms\b")),
        SurfaceField("retry.max_delay_ms", _re(r"set_retry_max_delay_ms\b")),
        SurfaceField("retry.max_attempts", _re(r"set_retry_max_attempts\b")),
        SurfaceField("retry.max_elapsed_secs", _re(r"set_retry_max_elapsed_secs\b")),
        SurfaceField(
            "flatfiles.max_attempts", _re(r"set_flatfiles_max_attempts\b")
        ),
        SurfaceField(
            "flatfiles.initial_backoff_secs",
            _re(r"set_flatfiles_initial_backoff_secs\b"),
        ),
        SurfaceField(
            "flatfiles.max_backoff_secs", _re(r"set_flatfiles_max_backoff_secs\b")
        ),
    ]
    surfaces.append(ffi)

    # C++ Doxygen — the `.h` (C ABI) header.
    cpp_h = Surface("cpp.h", pathlib.Path("sdks/cpp/include/thetadx.h"))
    cpp_h.fields = [
        SurfaceField("reconnect.max_attempts", _re(r"set_reconnect_max_attempts\b")),
        SurfaceField(
            "reconnect.max_rate_limited_attempts",
            _re(r"set_reconnect_max_rate_limited_attempts\b"),
        ),
        SurfaceField(
            "reconnect.max_server_restart_attempts",
            _re(r"set_reconnect_max_server_restart_attempts\b"),
        ),
        SurfaceField(
            "reconnect.stable_window_secs",
            _re(r"set_reconnect_stable_window_secs\b"),
        ),
        SurfaceField("reconnect.wait_ms", _re(r"set_reconnect_wait_ms\b")),
        SurfaceField(
            "reconnect.wait_rate_limited_ms",
            _re(r"set_reconnect_wait_rate_limited_ms\b"),
        ),
        SurfaceField(
            "reconnect.max_elapsed_secs", _re(r"set_reconnect_max_elapsed_secs\b")
        ),
        SurfaceField("reconnect.wait_max_ms", _re(r"set_reconnect_wait_max_ms\b")),
        SurfaceField(
            "reconnect.wait_server_restart_ms",
            _re(r"set_reconnect_wait_server_restart_ms\b"),
        ),
        SurfaceField(
            "reconnect.replay_burst_size", _re(r"set_reconnect_replay_burst_size\b")
        ),
        SurfaceField(
            "reconnect.replay_pace_ms", _re(r"set_reconnect_replay_pace_ms\b")
        ),
        SurfaceField("streaming.timeout_ms", _re(r"set_streaming_timeout_ms\b")),
        SurfaceField("streaming.connect_timeout_ms", _re(r"set_streaming_connect_timeout_ms\b")),
        SurfaceField("streaming.ping_interval_ms", _re(r"set_streaming_ping_interval_ms\b")),
        SurfaceField("streaming.io_read_slice_ms", _re(r"set_streaming_io_read_slice_ms\b")),
        SurfaceField("streaming.data_watchdog_ms", _re(r"set_streaming_data_watchdog_ms\b")),
        SurfaceField(
            "streaming.keepalive_idle_secs", _re(r"set_streaming_keepalive_idle_secs\b")
        ),
        SurfaceField(
            "streaming.keepalive_interval_secs", _re(r"set_streaming_keepalive_interval_secs\b")
        ),
        SurfaceField("streaming.keepalive_retries", _re(r"set_streaming_keepalive_retries\b")),
        SurfaceField("retry.initial_delay_ms", _re(r"set_retry_initial_delay_ms\b")),
        SurfaceField("retry.max_delay_ms", _re(r"set_retry_max_delay_ms\b")),
        SurfaceField("retry.max_attempts", _re(r"set_retry_max_attempts\b")),
        SurfaceField("retry.max_elapsed_secs", _re(r"set_retry_max_elapsed_secs\b")),
        SurfaceField(
            "flatfiles.max_attempts", _re(r"set_flatfiles_max_attempts\b")
        ),
        SurfaceField(
            "flatfiles.initial_backoff_secs",
            _re(r"set_flatfiles_initial_backoff_secs\b"),
        ),
        SurfaceField(
            "flatfiles.max_backoff_secs", _re(r"set_flatfiles_max_backoff_secs\b")
        ),
    ]
    surfaces.append(cpp_h)

    # TypeScript NAPI docstrings (generate index.d.ts).
    ts = Surface("typescript", pathlib.Path("sdks/typescript/src/config_class.rs"))
    ts.fields = [
        SurfaceField("reconnect.max_attempts", _re(r"setReconnectMaxAttempts\b")),
        SurfaceField(
            "reconnect.max_rate_limited_attempts",
            _re(r"setReconnectMaxRateLimitedAttempts\b"),
        ),
        SurfaceField("reconnect.wait_ms", _re(r"setReconnectWaitMs\b")),
        SurfaceField(
            "reconnect.wait_rate_limited_ms", _re(r"setReconnectWaitRateLimitedMs\b")
        ),
        SurfaceField("reconnect.wait_max_ms", _re(r"setReconnectWaitMaxMs\b")),
        SurfaceField(
            "reconnect.wait_server_restart_ms",
            _re(r"setReconnectWaitServerRestartMs\b"),
        ),
        SurfaceField(
            "reconnect.replay_burst_size", _re(r"setReconnectReplayBurstSize\b")
        ),
        SurfaceField(
            "reconnect.replay_pace_ms", _re(r"setReconnectReplayPaceMs\b")
        ),
        SurfaceField("streaming.timeout_ms", _re(r"setStreamingTimeoutMs\b")),
        SurfaceField("streaming.connect_timeout_ms", _re(r"setStreamingConnectTimeoutMs\b")),
        SurfaceField("streaming.ping_interval_ms", _re(r"setStreamingPingIntervalMs\b")),
        SurfaceField("streaming.io_read_slice_ms", _re(r"setStreamingIoReadSliceMs\b")),
        SurfaceField("streaming.data_watchdog_ms", _re(r"setStreamingDataWatchdogMs\b")),
        SurfaceField(
            "streaming.keepalive_idle_secs", _re(r"setStreamingKeepaliveIdleSecs\b")
        ),
        SurfaceField(
            "streaming.keepalive_interval_secs", _re(r"setStreamingKeepaliveIntervalSecs\b")
        ),
        SurfaceField("streaming.keepalive_retries", _re(r"setStreamingKeepaliveRetries\b")),
        SurfaceField("retry.max_attempts", _re(r"setRetryMaxAttempts\b")),
        SurfaceField("flatfiles.max_attempts", _re(r"setFlatfilesMaxAttempts\b")),
        SurfaceField(
            "flatfiles.initial_backoff_secs",
            _re(r"setFlatfilesInitialBackoffSecs\b"),
        ),
        SurfaceField(
            "flatfiles.max_backoff_secs", _re(r"setFlatfilesMaxBackoffSecs\b")
        ),
    ]
    surfaces.append(ts)

    # Shipped Python type stub (`.pyi`). The default sits inline on the
    # attribute docstring; the anchor is the typed attribute line.
    pyi = Surface(
        "python.pyi",
        pathlib.Path("sdks/python/python/thetadatadx/__init__.pyi"),
        doc_direction="below",
    )
    pyi.fields = [
        SurfaceField("reconnect.max_attempts", _re(r"^\s*reconnect_max_attempts:")),
        SurfaceField(
            "reconnect.max_server_restart_attempts",
            _re(r"^\s*reconnect_max_server_restart_attempts:"),
        ),
        SurfaceField(
            "reconnect.max_elapsed_secs", _re(r"^\s*reconnect_max_elapsed_secs:")
        ),
        SurfaceField("reconnect.wait_ms", _re(r"^\s*reconnect_wait_ms:")),
        SurfaceField("reconnect.wait_max_ms", _re(r"^\s*reconnect_wait_max_ms:")),
        SurfaceField(
            "reconnect.wait_rate_limited_ms",
            _re(r"^\s*reconnect_wait_rate_limited_ms:"),
        ),
        SurfaceField(
            "reconnect.wait_server_restart_ms",
            _re(r"^\s*reconnect_wait_server_restart_ms:"),
        ),
        SurfaceField(
            "reconnect.replay_burst_size", _re(r"^\s*reconnect_replay_burst_size:")
        ),
        SurfaceField(
            "reconnect.replay_pace_ms", _re(r"^\s*reconnect_replay_pace_ms:")
        ),
        SurfaceField("retry.initial_delay_ms", _re(r"^\s*retry_initial_delay_ms:")),
        SurfaceField("retry.max_delay_ms", _re(r"^\s*retry_max_delay_ms:")),
        SurfaceField("retry.max_attempts", _re(r"^\s*retry_max_attempts:")),
        SurfaceField("retry.max_elapsed_secs", _re(r"^\s*retry_max_elapsed_secs:")),
        SurfaceField("flatfiles.max_attempts", _re(r"^\s*flatfiles_max_attempts:")),
        SurfaceField(
            "flatfiles.initial_backoff_secs",
            _re(r"^\s*flatfiles_initial_backoff_secs:"),
        ),
        SurfaceField(
            "flatfiles.max_backoff_secs", _re(r"^\s*flatfiles_max_backoff_secs:")
        ),
        SurfaceField("streaming.timeout_ms", _re(r"^\s*streaming_timeout_ms:")),
        SurfaceField("streaming.data_watchdog_ms", _re(r"^\s*streaming_data_watchdog_ms:")),
        SurfaceField(
            "streaming.keepalive_idle_secs", _re(r"^\s*streaming_keepalive_idle_secs:")
        ),
        SurfaceField(
            "streaming.keepalive_interval_secs", _re(r"^\s*streaming_keepalive_interval_secs:")
        ),
        SurfaceField("streaming.keepalive_retries", _re(r"^\s*streaming_keepalive_retries:")),
        SurfaceField("historical.concurrent_requests", _re(r"^\s*concurrent_requests:")),
    ]
    # `concurrent_requests` has no single-literal default: the
    # constructor seeds `0`, which is the "auto-detect from the
    # subscription tier returned by Nexus auth" sentinel rather than a
    # caller-facing fixed value. Every surface documents it as
    # "0 = auto-detect", not "Default 0"; the gate registers the field
    # so the skip is explicit rather than silently absent, but does not
    # demand a literal match against the sentinel.
    pyi.skips["historical.concurrent_requests"] = (
        "default 0 is the auto-detect-from-tier sentinel, not a fixed literal"
    )
    surfaces.append(pyi)

    return surfaces


# ── Driver ───────────────────────────────────────────────────────────


def run(root: pathlib.Path = REPO_ROOT) -> list[Mismatch]:
    canon = load_canonical(root)
    mismatches: list[Mismatch] = []
    for surface in build_surfaces():
        if not (root / surface.rel_path).is_file():
            mismatches.append(
                Mismatch(
                    surface.name,
                    surface.rel_path.as_posix(),
                    0,
                    "<surface>",
                    -1,
                    -1,
                    detail="surface file not found",
                )
            )
            continue
        mismatches.extend(check_surface(surface, canon, root))
    return mismatches


def _print_mismatches(mismatches: list[Mismatch]) -> None:
    print(f"doc-defaults: {len(mismatches)} documented default(s) drifted")
    for m in mismatches:
        if m.detail:
            print(f"  {m.rel_path}:{m.lineno} [{m.surface}] {m.field_id}: {m.detail}")
        else:
            print(
                f"  {m.rel_path}:{m.lineno} [{m.surface}] {m.field_id}: "
                f"documented {m.documented}, source of truth {m.canonical}"
            )
    print(
        "  -> Correct the doc comment to the Rust constructor value in "
        "crates/thetadatadx/src/config/*.rs."
    )


# ── Selftest ─────────────────────────────────────────────────────────


def _selftest() -> int:
    """Plant a wrong documented default in a synthetic tree, confirm the
    gate catches it, then correct the plant and confirm the gate passes.

    The synthetic tree mirrors the real source-of-truth + one binding
    surface so the parser, the anchor search, and the comparison are all
    exercised end to end.
    """
    import tempfile

    fpss_rs = """
impl FpssConfig {
    pub fn production_defaults() -> Self {
        Self {
            timeout_ms: 3_000,
            ring_size: 131_072,
            ping_interval_ms: 250,
            connect_timeout_ms: 2_000,
            io_read_slice_ms: 25,
            data_watchdog_ms: 30_000,
            keepalive_idle_secs: 5,
            keepalive_interval_secs: 2,
            keepalive_retries: 2,
        }
    }
}
"""
    flatfiles_rs = """
impl FlatFilesConfig {
    pub fn production_defaults() -> Self {
        Self {
            max_attempts: 10,
            initial_backoff: Duration::from_secs(1),
            max_backoff: Duration::from_secs(30),
            jitter: true,
        }
    }
}
pub mod bounds {
    pub const MAX_ATTEMPTS: std::ops::RangeInclusive<u32> = 1..=100;
}
"""
    reconnect_rs = """
impl ReconnectConfig {
    pub fn production_defaults() -> Self {
        Self {
            wait_ms: 250,
            wait_max_ms: 30_000,
            wait_rate_limited_ms: 130_000,
            wait_server_restart_ms: 5_000,
            jitter: JitterMode::Full,
            replay_burst_size: 50,
            replay_pace_ms: 5,
            policy: ReconnectPolicy::Auto(ReconnectAttemptLimits::default()),
        }
    }
}
impl Default for ReconnectAttemptLimits {
    fn default() -> Self {
        Self {
            max_attempts: 30,
            max_rate_limited_attempts: 100,
            max_server_restart_attempts: 60,
            max_elapsed: Duration::from_secs(300),
            stable_window: Duration::from_secs(60),
        }
    }
}
"""
    retry_rs = """
impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            initial_delay: Duration::from_millis(250),
            max_delay: Duration::from_secs(30),
            max_attempts: 20,
            max_elapsed: Duration::from_secs(300),
            jitter: true,
        }
    }
}
"""
    mdds_rs = """
impl MddsConfig {
    pub fn production_defaults() -> Self {
        Self {
            concurrent_requests: 0,
            max_message_size: 4 * 1024 * 1024,
            keepalive_secs: 30,
            keepalive_timeout_secs: 10,
            window_size_kb: 64,
            connection_window_size_kb: 64,
            connect_timeout_secs: 10,
            warn_on_buffered_threshold_bytes: 100 * 1024 * 1024,
            override_tier_clamp: false,
        }
    }
}
"""

    # One synthetic FFI surface. The reconnect_wait_ms doc is PLANTED
    # wrong (2_000 vs canonical 250); the rest are correct.
    ffi_bad = """
/// Set the reconnect delay (ms). Default `2_000`.
pub unsafe extern "C" fn thetadatadx_config_set_reconnect_wait_ms() {}

/// Set the read timeout (ms). Default `3_000`.
pub unsafe extern "C" fn thetadatadx_config_set_streaming_timeout_ms() {}

/// Set the flatfile attempt budget. Default `10`. Validated to `[1, 100]`.
pub unsafe extern "C" fn thetadatadx_config_set_flatfiles_max_attempts() {}
"""
    ffi_good = ffi_bad.replace("Default `2_000`", "Default `250`")

    def write_tree(root: pathlib.Path, ffi_body: str) -> None:
        cfg = root / CONFIG_DIR
        cfg.mkdir(parents=True, exist_ok=True)
        (cfg / "fpss.rs").write_text(fpss_rs, encoding="utf-8")
        (cfg / "flatfiles.rs").write_text(flatfiles_rs, encoding="utf-8")
        (cfg / "reconnect.rs").write_text(reconnect_rs, encoding="utf-8")
        (cfg / "retry.rs").write_text(retry_rs, encoding="utf-8")
        (cfg / "mdds.rs").write_text(mdds_rs, encoding="utf-8")
        ffi_dir = root / "ffi" / "src"
        ffi_dir.mkdir(parents=True, exist_ok=True)
        (ffi_dir / "auth.rs").write_text(ffi_body, encoding="utf-8")

    # A minimal surface registry scoped to the synthetic FFI file.
    synthetic = Surface("ffi", pathlib.Path("ffi/src/auth.rs"))
    synthetic.fields = [
        SurfaceField("reconnect.wait_ms", _re(r"set_reconnect_wait_ms\b")),
        SurfaceField("streaming.timeout_ms", _re(r"set_streaming_timeout_ms\b")),
        SurfaceField("flatfiles.max_attempts", _re(r"set_flatfiles_max_attempts\b")),
    ]

    with tempfile.TemporaryDirectory() as td:
        root = pathlib.Path(td)

        # 1) Planted wrong default → gate must catch it.
        write_tree(root, ffi_bad)
        canon = load_canonical(root)
        if canon["reconnect.wait_ms"] != 250:
            print(
                "selftest FAILED: canonical reconnect.wait_ms parsed as "
                f"{canon['reconnect.wait_ms']}, expected 250"
            )
            return 1
        if canon["retry.max_delay_ms"] != 30_000:
            print(
                "selftest FAILED: canonical retry.max_delay_ms unit "
                f"conversion gave {canon['retry.max_delay_ms']}, expected 30000"
            )
            return 1
        bad = check_surface(synthetic, canon, root)
        planted = [m for m in bad if m.field_id == "reconnect.wait_ms"]
        if not planted:
            print("selftest FAILED: planted wrong default was not caught")
            return 1
        if planted[0].documented != 2000 or planted[0].canonical != 250:
            print(
                "selftest FAILED: planted mismatch reported wrong values "
                f"({planted[0].documented} vs {planted[0].canonical})"
            )
            return 1
        # The two correct fields must NOT be flagged.
        if any(m.field_id != "reconnect.wait_ms" for m in bad):
            print(
                "selftest FAILED: a correct default was flagged "
                f"({[m.field_id for m in bad]})"
            )
            return 1

        # 2) Corrected tree → gate must pass clean.
        write_tree(root, ffi_good)
        canon = load_canonical(root)
        good = check_surface(synthetic, canon, root)
        if good:
            print(
                "selftest FAILED: corrected tree still reports mismatches "
                f"({[(m.field_id, m.documented, m.canonical) for m in good]})"
            )
            return 1

    print("selftest: ok")
    return 0


def main(argv: Optional[list[str]] = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument(
        "--selftest",
        action="store_true",
        help="Run the embedded plant-and-revert self-test and exit.",
    )
    args = parser.parse_args(argv)

    if args.selftest:
        return _selftest()

    mismatches = run()
    if not mismatches:
        print("doc-defaults: clean")
        return 0
    _print_mismatches(mismatches)
    return 1


if __name__ == "__main__":
    sys.exit(main())
