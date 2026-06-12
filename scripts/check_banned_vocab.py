#!/usr/bin/env python3
"""Banned-vocabulary scrubber (Gate 11 / issue #554).

Scans source files, doc strings, code comments, recent commit subjects,
and (when run inside a CI PR) the current PR title + body for marketing
or internal-process jargon that should never reach a public artifact.

Hits return a non-zero exit code. False positives can be silenced inline
with a ``VOCAB-OK: <one-line reason>`` annotation on the same line as
the offending phrase, or by listing the file under ``EXEMPT_PATHS``
below — historical changelogs and release notes are append-only ledgers
of past releases and rewriting them would falsify the record.

Run from the repo root::

    python3 scripts/check_banned_vocab.py
"""

from __future__ import annotations

import os
import pathlib
import re
import subprocess
import sys
from typing import Iterable


REPO_ROOT = pathlib.Path(__file__).resolve().parents[1]


# Phrases that have to disappear from any newly-authored public surface.
# Matched case-insensitively as whole tokens; multi-word phrases are
# matched as-is (whitespace-collapsed).
BANNED = [
    "institutional",
    "bulletproof",
    "enterprise-grade",
    "enterprise grade",
    "audit fixes",
    "NICE-TO-HAVE",
    "nice to have",
    "production-ready",
    "production ready",
    "world-class",
    "world class",
    "next-generation",
    "next generation",
    "minimum vs complete",
    # Release-cycle / review-process vocabulary that must not leak
    # into source comments, docstrings, commit prose, or PR text.
    # Version numbers in explanatory prose belong only in the
    # CHANGELOG and release notes (already exempted below).
    #
    # Word-boundary matching on the bare tokens catches `v11`,
    # `v11.0.0`, `(v11)`, `v11-` etc. without needing surrounding
    # whitespace.
    "v11",
    "v12",
    "codex",
    "Codex",
    "CODEX",
    "BLOCKER",
    "SERIOUS #",
    # Cover every round number a future audit cycle might produce
    # so the gate fires on the next leak without script edits.
    "round-2",
    "round-3",
    "round-4",
    "round-5",
    "round-6",
    "round-7",
    "round-8",
    "round-9",
    "round-10",
    "round-11",
    "round-12",
    # Competitor SDK / vendor names must not appear in client-facing
    # docs, doc comments, or PR text. Engine-internal source (the
    # `thetadatadx-engine` crate) is exempted via `EXEMPT_PATHS`
    # below — engineering benchmarks may reference vendor behaviour
    # without leaking into the public surface.
    "databento",
    "Bloomberg",
    "blpapi",
    "Refinitiv",
    "LSEG",
    # ConnectionClosed regression closure: ban the wrong-cause
    # vocabulary so future PRs cannot reintroduce the misattribution.
    # `cascade` is the noun; `h2-cascade` / `UpstreamCascade` / the
    # field-count quote layouts are the artifacts.
    "cascade",
    "6-field NBBO",
    "11-field NBBO",
    "12-field NBBO",
    "normalizeData",
    "UpstreamCascade",
    "RestOnH2Disconnect",
    "RestAlwaysForDateRange",
    "h2-cascade",
    "legacy quote investigation",
    "theta-terminal-re/patches",
    # Kairos/SDK internal architecture vocabulary that must never appear
    # in user-facing surfaces (doc comments, public READMEs, marketing).
    # Protocol/vendor names (FPSS, MDDS) are ALLOW-listed — these banned
    # items are Rust impl-detail names.
    #
    # NOTE on identifier-embedded leaks: the patterns below match as
    # whole tokens (`\bword\b`). An underscore is a regex word
    # character, so a banned token buried inside a snake_case /
    # camelCase PUBLIC identifier (`set_tokio_worker_threads`,
    # `TokioWorkerThreadsSetting`) is intentionally NOT caught here — a
    # substring scan over every source file would false-positive on the
    # engine's and bindings' legitimate INTERNAL use of the same tokens
    # (`tokio::runtime`, `crossbeam::channel`, `parking_lot::Mutex`,
    # `Runtime::block_on`). That leak class is caught structurally,
    # without false positives, by `check_binding_parity.py`
    # (`_check_public_surface_vocab`), which inspects only the declared
    # PUBLIC client identifiers the parity collectors harvest. The two
    # guards are complementary: this scrubber owns standalone / phrase
    # vocabulary across all prose; the parity guard owns
    # identifier-embedded tokens on the public API surface.
    "MDDS gRPC",
    "FPSS TCP",
    "FIT nibble",
    "disruptor",
    "LMAX",
    "SPMC",
    "tonic",
    "crossbeam",
    "parking_lot",
    "os_pipe",
    "block_on",
    "allow_threads",
    "Python::detach",
]


# File globs walked relative to repo root.
#
# The scrubber covers `proto/**/*.proto` (gRPC wire schema comments),
# `crates/**/*.md` (per-crate READMEs and inline maintenance guides),
# `tools/**/*.md` (CLI / MCP / server crate READMEs), and
# `docs/**/*.md` (top-level docs, pinned redundantly so a future glob
# trim still includes them). Marketing or internal-process vocabulary
# that lands inline in any of those files trips the gate.
SCAN_GLOBS = [
    "crates/**/*.rs",
    "crates/**/*.toml",
    "crates/**/*.md",
    "ffi/**/*.rs",
    "ffi/**/*.toml",
    "ffi/**/*.md",
    # Proto files live under each crate (`crates/<name>/proto/*.proto`),
    # not in a top-level `proto/` directory. The earlier `proto/**/*.proto`
    # glob always expanded to an empty list and quietly skipped wire-spec
    # files from the banned-vocab scan.
    "crates/**/*.proto",
    "sdks/**/*.rs",
    "sdks/**/*.py",
    "sdks/**/*.pyi",
    "sdks/**/*.ts",
    "sdks/**/*.js",
    "sdks/**/*.hpp",
    "sdks/**/*.cpp",
    "sdks/**/*.h",
    "sdks/**/*.inc",
    "sdks/**/*.toml",
    "sdks/**/*.md",
    "tools/**/*.rs",
    "tools/**/*.toml",
    "tools/**/*.md",
    "docs/**/*.md",
    "docs-site/**/*.md",
    "docs-site/**/*.ts",
    "docs-site/**/*.vue",
    "scripts/**/*.py",
    ".github/**/*.yml",
    "README.md",
    "CONTRIBUTING.md",
    "SECURITY.md",
]


# Paths skipped by the file walk. CHANGELOG.md and the docs-site
# changelog mirror are append-only ledgers of historical releases;
# rewriting them would falsify the public record. The gate still
# blocks new content elsewhere from referencing the same phrases.
EXEMPT_PATHS = {
    "CHANGELOG.md",
    "docs-site/docs/changelog.md",
    "scripts/check_banned_vocab.py",
    # The cross-binding parity gate is the SECOND vocabulary policy
    # file: its `BANNED_SURFACE_TOKENS` list enumerates the same
    # impl-detail token names, and its selftest fixtures spell them out
    # to prove the public-surface guard fires. A file whose job is to
    # name the banned tokens cannot be scrubbed for containing them —
    # identical rationale to the `check_banned_vocab.py` self-exemption
    # above. Its companion test file follows for the same reason.
    "scripts/check_binding_parity.py",
    "scripts/test_check_binding_parity.py",
    "scripts/__pycache__",
    ".github/release-notes",
    # The `tdbe` crate is an independent library with its own release cycle.
    # Its internal codec terminology (FIT nibble = Financial Information Tick
    # encoding unit) is out of scope for the SDK surface vocabulary gate.
    "crates/tdbe",
    # Per-version migration ledgers are append-only artefacts that
    # name the versions they transition between. They join the
    # CHANGELOG and release-notes as historical references.
    "docs-site/docs/migration",
}


# Directory name fragments anywhere in the path that mark vendored or
# generated content the gate must not police: third-party wheels in a
# local venv, package-manager caches, build outputs, etc.
EXEMPT_PATH_FRAGMENTS = (
    "/.venv/",
    "/.venv-test/",
    "/venv/",
    "/node_modules/",
    "/target/",
    "/__pycache__/",
    "/.git/",
    # C++ build tree's `_deps/` carries the unpacked Catch2 source
    # tarball plus other third-party deps the CMake fetch step
    # materialises into the build dir. Their markdown / source
    # contains vocabulary the gate has no opinion on.
    "/build_tests/",
    "/_deps/",
    # Criterion bench harnesses are internal tooling, not public-surface
    # documentation. They may reference crate names (disruptor, etc.)
    # in measurement descriptions without those names leaking to users.
    "/benches/",
    # Build-time codegen helpers are never in the published rlib surface.
    "/build_support/",
    "/build_support_bin/",
    # The grpc module is `pub(crate)` in shipped builds and `#[doc(hidden)]`
    # when re-opened by `__test-helpers`. Its implementation comments may
    # legitimately reference the previous transport name for historical
    # comparison. Not a user-facing surface.
    "/grpc/",
    # fpss/ring.rs is pub(crate) implementation for the streaming
    # event-dispatch pipeline. Its doc comments may name internal
    # mechanisms that never reach the public SDK surface.
    "/fpss/ring.rs",
    # io_loop/ is a pub(crate) subdirectory of the FPSS module — its
    # implementation docs are never rendered to the public SDK surface.
    "/io_loop/",
    # mdds/macros.rs and mdds/endpoint_args.rs host the generated endpoint
    # macro-expansion helpers. These are #[doc(hidden)] and pub(crate); their
    # implementation comments may reference the old transport name for
    # correctness comparisons.
    "/mdds/macros.rs",
    "/mdds/endpoint_args.rs",
    # ffi/src/ is the C ABI shim (`publish = false`). Its implementation uses
    # `tokio::Runtime::block_on` (not PyO3 `allow_threads`) and may name the
    # internal event-ring mechanism. None of these symbols appear in user docs.
    "ffi/src/",
    # tests/ is not part of the published crate surface.
    "/tests/",
    # proto/MAINTENANCE.md is an internal developer guide, not user-facing.
    "/proto/",
    # _generated/ directories contain codegen-emitted Rust source — the
    # generator template is what the vocabulary gate should police, not
    # individual generated files whose identifiers come from schema defns.
    "/_generated/",
    # async_runtime.rs is an internal PyO3 bridge file; its `block_on`
    # calls are `tokio::Runtime::block_on` in the runtime glue, not the
    # PyO3 GIL-holding pattern the rule targets.
    "/async_runtime.rs",
    # streaming_session.rs is a pub(crate) PyO3 glue type — not user docs.
    "/streaming_session.rs",
    # sdks/python/src/lib.rs is the PyO3 shim entry point (`publish = false`).
    # Its `runtime().block_on` calls are tokio-runtime bridge, documented as
    # VOCAB-OK on the preceding comment line — not the PyO3 GIL-hold pattern.
    "sdks/python/src/lib.rs",
    # sdks/typescript/src/lib.rs is the NAPI shim (`publish = false`).
    # Its `.block_on(...)` calls are tokio-runtime bridge, not PyO3 GIL-hold.
    "sdks/typescript/src/lib.rs",
    # tools/server/src/ws/broadcast.rs uses `tokio::Runtime::block_on` in
    # a test body (server binary, `publish = false`). Not user-facing.
    "tools/server/src/ws/broadcast.rs",
    # Cargo.toml comment lines referencing dep crate names are not user-facing.
    # (Files themselves are scanned; VOCAB-OK annotations handle the remainder.)
)


# Compile once. Word-boundary check on alphanumeric tokens; multi-word
# phrases match as a literal collapsed-whitespace sequence.
def _compile_patterns() -> list[tuple[str, re.Pattern[str]]]:
    out: list[tuple[str, re.Pattern[str]]] = []
    for phrase in BANNED:
        if " " in phrase or "-" in phrase:
            # Multi-token: tolerate one-or-more whitespace / hyphen runs
            # between tokens so "enterprise grade" and "enterprise-grade"
            # both flag the same rule.
            tokens = re.split(r"[\s-]+", phrase)
            pattern = r"\b" + r"[\s-]+".join(re.escape(t) for t in tokens) + r"\b"
        else:
            pattern = r"\b" + re.escape(phrase) + r"\b"
        out.append((phrase, re.compile(pattern, re.IGNORECASE)))
    return out


PATTERNS = _compile_patterns()
ALLOW_RE = re.compile(r"VOCAB-OK\s*:")


def _is_exempt(rel_path: pathlib.Path) -> bool:
    parts = rel_path.as_posix()
    for exempt in EXEMPT_PATHS:
        if parts == exempt or parts.startswith(exempt + "/"):
            return True
    fragment_target = "/" + parts + "/"
    for fragment in EXEMPT_PATH_FRAGMENTS:
        if fragment in fragment_target:
            return True
    return False


def _iter_files() -> Iterable[pathlib.Path]:
    seen: set[pathlib.Path] = set()
    for pattern in SCAN_GLOBS:
        for candidate in REPO_ROOT.glob(pattern):
            if not candidate.is_file():
                continue
            rel = candidate.relative_to(REPO_ROOT)
            if _is_exempt(rel):
                continue
            if rel in seen:
                continue
            seen.add(rel)
            yield candidate


def _scan_file(path: pathlib.Path) -> list[tuple[int, str, str]]:
    hits: list[tuple[int, str, str]] = []
    try:
        text = path.read_text(encoding="utf-8")
    except (OSError, UnicodeDecodeError):
        return hits
    for lineno, line in enumerate(text.splitlines(), start=1):
        if ALLOW_RE.search(line):
            continue
        for phrase, regex in PATTERNS:
            if regex.search(line):
                hits.append((lineno, phrase, line.rstrip()))
                break
    return hits


VOCAB_OK_BODY_RE = re.compile(
    r"VOCAB-OK\s*:\s*([0-9a-f]{7,40})\s+(.+)",
    re.IGNORECASE,
)


def _scan_commit_subjects() -> list[tuple[str, str, str]]:
    """Inspect commit subjects unique to the current branch.

    Limits the scan to ``origin/main..HEAD`` so already-merged history
    on ``main`` stays immutable — squashing or rewording landed commits
    would falsify the public git record. If the comparison ref cannot
    be resolved (shallow clone, first-commit scenarios, no remote) we
    fall back to the last 50 commits on ``HEAD``.

    Honors a body-level ``VOCAB-OK: <sha-prefix> <reason>`` escape
    hatch. The annotation may live in any commit body within the same
    range; when a banned hit lands on a subject whose SHA is prefixed
    by a declared exemption SHA, the hit is suppressed and the
    exemption is logged to stdout for transparency. Mirrors the inline
    ``VOCAB-OK: <reason>`` escape hatch the file walker already
    honors, scoped here to past commit subjects we cannot rewrite
    without a force push.
    """
    spec = "origin/main..HEAD"
    rev_check = subprocess.run(
        ["git", "rev-parse", "--verify", "origin/main"],
        cwd=REPO_ROOT,
        capture_output=True,
        text=True,
        check=False,
    )
    if rev_check.returncode != 0:
        spec = "-50"
    try:
        out = subprocess.check_output(
            [
                "git",
                "log",
                spec,
                "--pretty=format:%H%x09%s%x1e%b%x1f",
            ],
            cwd=REPO_ROOT,
            text=True,
        )
    except (subprocess.CalledProcessError, FileNotFoundError):
        return []

    records: list[tuple[str, str, str]] = []
    exemptions: list[tuple[str, str]] = []
    for record in out.split("\x1f"):
        record = record.strip("\n")
        if not record or "\t" not in record:
            continue
        head, _, body = record.partition("\x1e")
        sha, _, subject = head.partition("\t")
        if not sha:
            continue
        records.append((sha, subject, body))
        for line in body.splitlines():
            match = VOCAB_OK_BODY_RE.search(line)
            if match:
                exemptions.append((match.group(1).lower(), match.group(2).strip()))

    hits: list[tuple[str, str, str]] = []
    for sha, subject, _body in records:
        for phrase, regex in PATTERNS:
            if regex.search(subject):
                exempted_reason: str | None = None
                sha_lower = sha.lower()
                for ex_prefix, ex_reason in exemptions:
                    if sha_lower.startswith(ex_prefix):
                        exempted_reason = ex_reason
                        break
                if exempted_reason is not None:
                    print(
                        f"vocab-ok: {sha[:12]} [{phrase}] {exempted_reason[:160]}"
                    )
                else:
                    hits.append((sha[:12], phrase, subject))
                break
    return hits


def _scan_pr_metadata() -> list[tuple[str, str, str]]:
    """Inspect the current PR title + body, if available via GitHub Actions."""
    pr_title = os.environ.get("PR_TITLE", "")
    pr_body = os.environ.get("PR_BODY", "")
    hits: list[tuple[str, str, str]] = []
    if pr_title:
        for phrase, regex in PATTERNS:
            if regex.search(pr_title):
                hits.append(("PR title", phrase, pr_title))
                break
    if pr_body:
        for line in pr_body.splitlines():
            if ALLOW_RE.search(line):
                continue
            for phrase, regex in PATTERNS:
                if regex.search(line):
                    hits.append(("PR body", phrase, line))
                    break
    return hits


# Stamped SAFETY-comment string the bot-author landed across the
# codebase in lieu of writing a real SAFETY annotation. Keeping it as
# boilerplate at every `unsafe { ... }` site reduces SAFETY comments to
# noise — the whole point of the lint is that each unsafe block names
# the invariant the caller upholds. The exact verbatim string below is
# blocked everywhere EXCEPT inside `ffi/src/`, where every site IS an
# `extern "C" fn` raw-pointer deref whose caller contract is documented
# on the enclosing fn signature; rewriting each of those would just
# duplicate the function-level doc.
STAMPED_SAFETY = (
    "see FFI boundary doc on the enclosing fn "
    "— raw pointers satisfy the documented caller contract"
)
STAMPED_SAFETY_SCOPE_EXEMPT_PREFIXES = ("ffi/src/",)


def _scan_stamped_safety() -> list[tuple[pathlib.Path, int, str]]:
    """Reject the stamped SAFETY boilerplate outside `ffi/src/`.

    Regression guard against future bot-stamping. The string is exact
    (no fuzzy match) so any rewrite that names the actual invariant
    sails through.
    """
    hits: list[tuple[pathlib.Path, int, str]] = []
    needle = STAMPED_SAFETY
    for path in _iter_files():
        rel = path.relative_to(REPO_ROOT).as_posix()
        if any(rel.startswith(p) for p in STAMPED_SAFETY_SCOPE_EXEMPT_PREFIXES):
            continue
        try:
            text = path.read_text(encoding="utf-8")
        except (OSError, UnicodeDecodeError):
            continue
        if needle not in text:
            continue
        for lineno, line in enumerate(text.splitlines(), start=1):
            if needle in line:
                hits.append((path.relative_to(REPO_ROOT), lineno, line.rstrip()))
    return hits


def main() -> int:
    total = 0

    file_hits: list[tuple[pathlib.Path, int, str, str]] = []
    for path in _iter_files():
        for lineno, phrase, line in _scan_file(path):
            file_hits.append((path.relative_to(REPO_ROOT), lineno, phrase, line))

    if file_hits:
        total += len(file_hits)
        print(f"banned-vocab: {len(file_hits)} hit(s) in tracked files")
        for rel, lineno, phrase, line in file_hits:
            print(f"  {rel}:{lineno} [{phrase}] {line[:160]}")

    commit_hits = _scan_commit_subjects()
    if commit_hits:
        total += len(commit_hits)
        print(f"banned-vocab: {len(commit_hits)} hit(s) in last 50 commit subjects")
        for sha, phrase, subject in commit_hits:
            print(f"  {sha} [{phrase}] {subject[:160]}")

    pr_hits = _scan_pr_metadata()
    if pr_hits:
        total += len(pr_hits)
        print(f"banned-vocab: {len(pr_hits)} hit(s) in PR metadata")
        for where, phrase, line in pr_hits:
            print(f"  {where} [{phrase}] {line[:160]}")

    stamped_hits = _scan_stamped_safety()
    if stamped_hits:
        total += len(stamped_hits)
        print(
            f"banned-vocab: {len(stamped_hits)} stamped-SAFETY hit(s) "
            f"outside ffi/src/"
        )
        for rel, lineno, line in stamped_hits:
            print(f"  {rel}:{lineno} {line[:160]}")
        print(
            "  -> Rewrite the comment to name the actual invariant "
            "(what's true here that makes the unsafe block sound). "
            "Stamped boilerplate is not a SAFETY annotation."
        )

    if total:
        if file_hits or commit_hits or pr_hits:
            print(
                "\nFix: either rephrase, or add `VOCAB-OK: <reason>` on the "
                "same line for genuinely-legitimate uses (e.g. quoting a "
                "third-party standard name)."
            )
        return 1

    print("banned-vocab: clean")
    return 0


if __name__ == "__main__":
    sys.exit(main())
