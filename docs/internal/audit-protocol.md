# ThetaDataDx Audit + Review Protocol

> Standing rulebook. Every contributor (human or LLM-assisted) reads this before
> opening a PR. Every reviewer uses it as the rubric. Future audits cite the
> sections here as the bar a change had to clear.

This document is the floor, not the ceiling. It captures the lessons paid for
in past PRs and codifies the patterns that keep the repo defensible. When in
doubt, default to the stricter reading.

---

## 1. The Bar

Every change must clear two reviewers at the same time:

1. A **senior open-source maintainer** in the style of Nicholas Marriott's
   review of `disruptor-rs` PR #35 — allergic to LLM bloat, math-formula
   comments, hand-copied algorithm replicas in tests, one-method traits with
   one impl, stamped boilerplate, and over-confident docs.
2. A **trading-system SWE** in the style of the build teams behind Bloomberg
   TOS, LSEG / Refinitiv RTDS, and Databento — allergic to hot-path allocations,
   locks held across `.await`, unguarded FDs, unbounded retries, leaked
   credentials in tracing output, and wire-protocol drift.

If either reviewer would revert the change, do not ship. Open a follow-up issue
and address the concern before merge.

---

## 2. First Principles

Twelve non-negotiables. Each is quotable; cite the number in PR review
comments.

1. **No defined-but-not-connected public surface.** If a class, function, or
   symbol is defined, it must be reachable from a fresh `pip install` /
   `cargo add` / `npm install`. (Precedent: v10.0.0 `Contract` collision —
   two pyclasses both bound to Python name `Contract`, second registration
   silently shadowed the first. Closed by Gate 1 + Gate 8.)
2. **Tests drive the real code path.** No regression test may re-implement
   the algorithm it claims to verify. (Precedent: `replay_unary_macro_loop`
   test family — eight tests stamped a hand-copy of the dispatch macro and
   asserted against themselves. Closed by PR #570.)
3. **Every `// SAFETY:` names the actual invariant.** Pointer provenance,
   layout, ordering, lifetime, alignment, or initialisation — pick the one
   that makes the block sound and write it down. Stamped boilerplate fails
   Gate 14. (Precedent: PR #574 removed eight identical placeholder comments
   that satisfied `clippy::undocumented_unsafe_blocks` without explaining
   anything.)
4. **Comments describe intent, not arithmetic.** A comment that re-derives a
   formula the reader can read off the next line is noise. (Precedent:
   Nicholas's flag on `free_slots(2, -1) = -1 - (2 - 8) = 5` in
   `disruptor-rs` PR #35.)
5. **One-method traits with one impl get inlined.** A free function or an
   inherent method is the right answer; trait abstraction is a load-bearing
   tool, not a habit. (Precedent: `CloneRefAttached` — invented for a single
   call site, removed during the same-PR review.)
6. **`assert_eq!` over `assert!(x <= y)` when the invariant is `==`.** The
   weaker check passes for every wrong answer that happens to be smaller.
   (Precedent: Nicholas's finding #4 on `disruptor-rs` PR #35.)
7. **No test deadlocks the runner on failure.** Drop locks before asserting,
   or assert outside the critical section. (Precedent: Nicholas's finding
   #5 — assert-while-holding-the-Mutex pattern hangs CI on regression.)
8. **`cargo doc --no-deps --workspace` passes without warnings.** Broken
   intra-doc links, redundant explicit targets, unresolved references all
   fail Gate 13. (Precedent: PR #570 unblocked 33 hidden rustdoc errors that
   shipped because CI never ran `cargo doc`.)
9. **`cargo clippy --workspace --all-targets -- -D warnings` passes on every
   tier of the matrix.** Linux, macOS, Windows. (Precedent: PR #572 caught
   `libc::pipe2` and `libc::__errno_location` referenced on Windows without
   a `cfg(target_os = "linux")` gate.)
10. **No release until every CI gate is green and at least one independent
    audit cycle is clean.** "Looks good to me" is not a substitute for a
    second pair of eyes on the diff.
11. **No hot-path allocation.** Per-tick `Vec::new` / `String::from` /
    `HashMap::new` is a defect. Pre-allocate, reuse buffers, or push the
    allocation off the hot path entirely.
12. **No silent breaking change.** `cargo-semver-checks` runs on every PR
    (Gate 9). A breaking change requires an explicit `!` in the commit
    subject, a `### Changed` or `### Removed` bucket in `CHANGELOG.md`, and
    owner sign-off on the bump.

---

## 3. The CI Gates

Every gate below is wired into `.github/workflows/ci.yml`. New gates land
as a row in this table; old gates do not get retired without a follow-up
issue documenting the replacement.

| #  | Gate                                | Catches                                                | Source           | Script / job                                                                                          |
|----|-------------------------------------|--------------------------------------------------------|------------------|-------------------------------------------------------------------------------------------------------|
| 1  | Python pyclass registration         | Unregistered or shadow-registered surface              | issue #544       | `scripts/check_binding_parity.py` (parity job)                                                        |
| 2  | Cross-binding parity matrix         | Asymmetric Py / TS / C++ surface                       | issue #545       | `scripts/check_binding_parity.py` + `sdks/parity.toml`                                                |
| 3  | Doctest gate                        | Examples that lie                                      | issue #546       | `cargo test --doc --workspace` (test job)                                                             |
| 4  | C ABI completeness vs header        | Exported-but-unbound symbols                           | issue #547       | `scripts/check_c_abi_completeness.py` (c-abi job)                                                     |
| 5  | Wire schema drift                   | Snapshot vs codegen output divergence                  | issue #548       | `generate_sdk_surfaces --check` + `refresh_grpc_snapshot` + `git diff --exit-code` (wire-schema job)  |
| 6  | Stubtest `.pyi` completeness        | Runtime vs stub drift                                  | issue #549       | `stubtest` in the Python workflow                                                                     |
| 7  | TS `.d.ts` drift                    | Committed vs napi-emitted                              | issue #550       | TypeScript SDK build + test (`surfaces` job)                                                          |
| 8  | Fresh-install smoke                 | First-user-experience break                            | issue #551       | `scripts/validate_python.py` + `scripts/validate_cli.py`                                              |
| 9  | `cargo-semver-checks`               | Silent breaking change                                 | issue #552       | `obi1kenobi/cargo-semver-checks-action@v2` (semver job)                                               |
| 10 | Bench regression                    | Silent perf regression                                 | issue #553       | `scripts/check_bench_regression.py --threshold 25` (bench job)                                        |
| 11 | Banned vocabulary                   | Marketing speak in source / commits / PR metadata      | issue #554       | `scripts/check_banned_vocab.py` (banned-vocab job)                                                    |
| 12 | Artifact contents                   | Leaked `.env`, creds, cache files in published wheels  | issue #555       | `scripts/inspect_artifacts.py`                                                                        |
| 13 | Cargo doc                           | Broken intra-doc links (Nicholas-lens regression)      | PR #572          | `cargo doc --no-deps --workspace` + feature-gated re-run (rustdoc job)                                |
| 14 | SAFETY-comment boilerplate detector | Stamped LLM `SAFETY:` without invariant                | PR #574          | `scripts/check_safety_comment_boilerplate.py` (banned-vocab job)                                      |

When you add a new gate:

- File the issue first. The PR title carries `feat(ci): add gate N — <name>`.
- Add the row above. Link the script + workflow step.
- Write a self-test in the script (`--selftest` is the convention; see
  `check_safety_comment_boilerplate.py`).
- Document the catch case in section 7 with a one-paragraph postmortem.

---

## 4. Code-Review Smell Catalog

Patterns that get a PR reverted. Each entry: the smell in one line, the fix
in one line. Real instances anchored where possible.

### 4.1 LLM-bloat smells (Nicholas-lens)

- **Math-formula comments.** `// free_slots(2, -1) = -1 - (2 - 8) = 5` —
  the next line is the formula. Delete the comment, or rewrite as intent:
  "wrap-around: producer is ahead by one cycle."
- **One-method traits + one impl.** If exactly one type implements it and
  the trait is not consumed generically, the trait is decoration. Use a
  free function or `impl Type` instead. (Anchor: `CloneRefAttached` removed
  in PR #570.)
- **Hand-copied algorithm replicas in tests.** If the test re-implements
  the function under test, it asserts that two copies of the same bug
  agree. Drive the real path; use mocks at the I/O boundary, not at the
  algorithm boundary. (Anchor: eight `replay_unary_macro_loop_*` tests
  removed in PR #570.)
- **`assert!(x <= y)` where `==` is the invariant.** Use `assert_eq!`.
  Weak assertions hide off-by-one defects. (Anchor: Nicholas's finding
  #4 on `disruptor-rs` PR #35.)
- **`matches!(x, _)` where `assert!(matches!(x, Variant))` was meant.**
  Same family — the assertion has no teeth.
- **Tests that deadlock on assert failure.** Releasing the lock before
  the assertion lets the runner report the failure instead of timing out.
- **`as usize` / `as u32` on relaxed atomic loads.** Torn-snapshot UB on
  32-bit targets. Use `load(Ordering::Acquire)` into the natural type and
  bounds-check before cast.
- **Over-confident docs.** "guarantees", "always", "never" without a
  citation to the proof. The Bloomberg reviewer reads docs as a contract;
  if the contract is wider than the implementation, the contract is wrong.
- **Silently source-breaking supertrait bounds.** Adding
  `Send + Sync + 'static` to a public trait is a breaking change.
  `cargo-semver-checks` (Gate 9) catches it; do not paper over the diff.
- **Doc blocks stolen by the wrong item.** A `///` block before a `#[cfg]`
  block silently attaches to the next item that survives the cfg. Verify
  with `cargo doc --no-deps --open`.
- **Bloated test setup.** Mocking five layers when two suffice is a
  signal the test is testing the harness, not the code.
- **Test names that lie.** `test_quote_handles_zero_size` should fail
  when zero-size quotes break, not when an unrelated invariant moves.
- **`#[allow(dead_code)]` / `#[allow(unused)]`.** Banned. Delete the
  function until it has a caller, or wire the caller in the same PR.
- **Stamped SAFETY comments.** Every `// SAFETY:` names the invariant.
  "Caller upholds the contract" is boilerplate; Gate 14 fails the build.
- **Multi-paragraph LLM-narrative docstrings on internal types.** A
  `pub(crate)` struct does not need a five-paragraph essay. One line of
  intent plus the field-level notes is the budget.
- **Comments explaining WHAT instead of WHY.** Code says what; comments
  say why. If the code is unclear, fix the code first.

### 4.2 Trading-system smells (Bloomberg / Refinitiv / Databento lens)

- **Hot-path allocations.** Per-tick `Vec::new`, `String::from`,
  `HashMap::new`, `format!`. Pre-size, reuse, or push the alloc to the
  init path.
- **Format strings inside `tracing::*` hot path.** `tracing::info!("...
  {x}")` evaluates the format args even when the subscriber filters the
  level. Use `tracing::trace!` for per-event paths and gate the call site
  on `tracing::enabled!(Level::TRACE)` if the args are expensive.
- **Per-call counter handle lookup.** Looking up `metrics::counter!(name)`
  per event re-hashes the metric name. Hoist to `LazyLock<Counter>` once
  per call site.
- **Lock held across `.await`.** Any `MutexGuard`, `RwLockReadGuard`,
  `parking_lot::*Guard` that crosses `.await` will cause a hang or a
  starvation under load. Drop before await, or use `tokio::sync::Mutex`
  with intent.
- **FD allocated without an `OwnedFd` guard on a fallible path.**
  Wrap raw FDs in `OwnedFd` immediately after the syscall; the `Drop`
  closes on early-return.
- **`tokio::spawn` without a tracked `JoinHandle` + abort-in-Drop.**
  Detached tasks outlive the parent and leak the lifetime of every
  borrow. Track the handle; `abort()` it in `Drop`.
- **Per-event allocation that survives 1024-rate-limit pressure.** Soak
  your hot path with one tick per microsecond. If the allocator profile
  has work to do, you have a defect.
- **Unbounded retry on transient classification.** Distinguish transient
  / permanent / rate-limited. Cap retries, jitter the backoff, propagate
  permanent failures, observe `Retry-After`.
- **Atomic counters on adjacent struct fields read by different threads.**
  False sharing. Pad with `#[repr(align(64))]` wrappers or move the field
  to its own cache line.
- **Sequence number wraps into a wire sentinel.** If `-1` or `u64::MAX`
  means "absent" on the wire, the in-memory counter must saturate before
  reaching the sentinel.
- **Adversarial wire-byte without a bounds check.** Every offset, every
  length-prefix, every variable-length field gets a `checked_*` or
  `get(..)` before the read.
- **Time math without `saturating_*` or `checked_*`.** A negative
  duration after `Instant::now() - earlier` panics; a wrap on `u64` ms
  overflows the schema field. Use the checked variant on every subtraction
  the user can influence.
- **Secret in `Debug` impl or tracing output.** Wrap in
  `secrecy::Secret<String>` or `Zeroizing<String>`; write a custom
  `Debug` that elides the value.
- **`panic!` inside `Drop`.** A panic during unwinding from another
  panic aborts the process. Use `tracing::error!` and return.
- **`unsafe` without a SAFETY comment that names the invariant.** See
  First Principle 3.

### 4.3 Cross-platform smells (caught by PR #572)

- **`libc::pipe2` / `libc::__errno_location` / `libc::epoll_*` without
  `cfg(target_os = "linux")`.** Pull the call into a per-OS module.
- **`pthread_t` / `siginfo_t` / `ucontext_t` references reaching
  Windows.** POSIX-only types belong behind a `cfg(unix)` boundary.
- **`[target.'cfg(unix)'.dev-dependencies]` placed before the main
  `[dev-dependencies]` block.** Cargo TOML table headers absorb every
  key until the next header; the unix-only table will steal cross-platform
  dev-deps. Always place narrow targets *after* the broad block.
  (Anchor: PR #572 surfaced this by silently dropping `toml` and `pyo3`
  from non-unix builds.)
- **POSIX signal handlers, mmap-style atomics, or named pipes without an
  OS gate.** Add the `cfg` and the Windows alternative in the same PR.
- **Test fixture using a Linux-only syscall without
  `cfg(target_os = "linux")`.** The test still has to compile on macOS
  and Windows even if it skips at runtime.

---

## 5. Process Discipline

Things that are not code but matter.

- **Conventional Commits.** Required on every commit. Subject ≤ 70
  chars. Body explains WHY. Format: `type(scope): subject`. Types:
  `feat`, `fix`, `docs`, `chore`, `refactor`, `perf`, `test`, `style`.
  Scopes documented in `CONTRIBUTING.md`.
- **One PR, one scope.** No drive-by changes that are not announced in
  the PR title. If you found something else while you were in there,
  open a follow-up issue.
- **No `--no-verify` on commits.** The hooks exist for a reason. Fix the
  issue, do not bypass the check. Anyone reviewing your PR will see the
  bypass in the commit metadata.
- **No `--force-push` to `main`.** Branch protection blocks it. Admin
  override is reserved for squash-consolidation with a documented
  rationale in the consolidation commit.
- **Issue first.** Every code change references an issue OR a commit
  message body that explains why the unprompted scope was justified.
  Drive-by quality-of-life fixes are fine; drive-by API changes are not.
- **No mega-PRs.** When scope grows past ~800 LOC and 8 commits, split.
  The reviewer's attention budget is finite; respect it. Exception:
  squash-consolidation of an already-merged history.
- **Every `Cargo.toml` dep added is justified.** Place a comment above
  the `name = "..."` line: which feature it unlocks, why this crate
  over alternatives, what the closest stdlib option is.
- **Every `unsafe` block has a SAFETY comment that names the actual
  invariant.** Verified by Gate 14.
- **Every public method has an `# Errors` doc section** if it returns
  `Result`. Verified by `clippy::missing_errors_doc` (consider promoting
  to `deny` once the existing surface is annotated).
- **Every newly-exported user-facing type has an entry in
  `sdks/parity.toml`.** Gate 2 fails the build otherwise.
- **No `TODO` / `FIXME` / `HACK` / `XXX` without a linked issue.** If
  it is worth flagging, it is worth tracking. Format:
  `TODO(#issue): short description`.

---

## 6. Trading-system patterns to mirror

Concrete patterns the SDK already follows or should adopt. Cite this
section in design reviews.

- **Wire-protocol contracts versioned + length-prefixed.** Every
  revision is backward-compatible at the codec layer; the decoder
  tolerates the previous schema and the next.
- **Per-method latency histograms (p50 / p99 / p999) via the `metrics`
  crate.** Histogram handles hoisted to `LazyLock<Histogram>`. One
  observation per call. No histogram per-event in hot paths without a
  rate-limit guard.
- **Backpressure as a first-class config knob.** `block` /
  `drop_oldest` / `drop_newest` are the three modes the FPSS streaming
  builder exposes. Every bounded channel surfaces the same enum.
- **Reconnect policy split per failure class.** Transient (network /
  TLS) reconnects automatically; permanent (auth / unknown contract)
  surfaces an `Error`; rate-limited honours `Retry-After` and backs off.
- **Credentials never in tracing output.** `Credentials` wraps the
  password in `Zeroizing<String>`. The `Debug` impl elides. The
  serialiser elides. Any future field is gated by review.
- **Secret rotation via `SessionToken::refresh`.** Works even with
  `RetryPolicy::disabled()` because token refresh is not a retry.
- **Soak tests are bench-driven.** No human-driven "run for 30 minutes
  and see." Soak harnesses live under `scripts/fpss_soak.py` and are
  checked into CI on the live workflow.
- **Generated code carries a `@generated DO NOT EDIT` header pointing
  to the SSOT schema.** Verified by the wire-schema drift gate (Gate 5);
  re-run the generator instead of hand-editing.

---

## 7. Postmortems

The mistakes paid for in past PRs. Each is one paragraph. Cite the
postmortem number in PR review comments when the same pattern recurs.

### 7.1 v10.0.0 `Contract` collision

The v10.0.0 wheel registered two `pyclass`-annotated types with
Python name `Contract`. The second registration silently shadowed
the first; users who relied on the first one saw `AttributeError` on
import. Symptom shipped to production. Root cause: no completeness
check on pyclass registration + no fresh-install smoke. Closed by
Gates 1, 8, and the collision-detector test added in PR #560.

### 7.2 `replay_unary_macro_loop` tests

Eight regression tests hand-copied the dispatch macro and asserted
against themselves. They passed because both copies had the same bug.
Closed by PR #570: regression tests now drive the real macro and assert
on the public API surface.

### 7.3 Stamped SAFETY comments

PR #572 introduced eight `// SAFETY: caller upholds the contract`
lines on unsafe blocks where no caller contract existed. The comments
satisfied `clippy::undocumented_unsafe_blocks = "deny"` without
documenting anything. Closed by Gate 14 (structural detector) and by
the audit cycle in PR #574, which rewrote every comment to name the
actual invariant.

### 7.4 `cargo doc` broken silently

33 broken intra-doc links shipped on `main` because CI never ran
`cargo doc`. The Nicholas-lens audit of `disruptor-rs` PR #35
documented the same anti-pattern in another codebase; we caught ours
the same week. Closed by Gate 13.

### 7.5 TOML table heading absorbed dev-deps

A `[target.'cfg(unix)'.dev-dependencies]` block placed *before* the
main `[dev-dependencies]` block silently stole `toml` and `pyo3` from
the cross-platform dev surface. Cargo TOML heading absorption is
silent; the missing deps surface only on the next macOS / Windows
build. Closed by PR #572 and codified as smell 4.3.

### 7.6 OOM at 23 GB on buffered historical endpoints

A `Vec<Tick>` accumulator at the I/O layer collected an entire
multi-month historical response before yielding. A user hit 23 GB
RSS on a one-year stock-quote pull. Closed by reshaping the API
around bounded `mpsc` + a `MessageTooLarge` clamp; streaming-first
is now the default contract for every historical endpoint.

### 7.7 2022 6-field NBBO upstream crash

ThetaData's Terminal cascades the h2 stream on pre-extension 6-field
NBBO rows for some 2022 option contracts. The SDK now ships three
independent recovery paths: a REST transport with a `FallbackPolicy`
enum, a `local-terminal-patcher` CLI, and a lenient gRPC decoder that
picks up the eventual upstream fix without further SDK change. The
lesson: when an upstream defect lands in users' hands, the SDK covers
what upstream cannot. (Anchor: PR #573, issue #571.)

---

## 8. The Meta-rule

If reading this doc tempts you to skip a step because "it's clean
today," stop. The `Contract` collision, the silent FD leak, the
stamped SAFETY comments, the broken `cargo doc`, the TOML absorption,
the 23 GB OOM — every single one of these passed casual review. Every
single one was caught by an explicit gate or an independent audit
cycle.

Treat the gates as floor, not ceiling. Treat "we already audited this"
as a hypothesis, not a conclusion. Run the gates locally before you
push. Read your own diff as if you were the reviewer.

---

## 9. Pre-PR self-review checklist

Run this before opening any PR.

```text
[ ] Conventional Commit subject + body (subject <= 70 chars)
[ ] cargo fmt --all -- --check
[ ] cargo clippy --workspace --all-targets -- -D warnings
[ ] cargo test --workspace
[ ] cargo doc --no-deps --workspace
[ ] cargo test --doc --workspace
[ ] Maturin build + pytest passes for Python SDK changes
[ ] npm run build + npm test for TypeScript SDK changes
[ ] Every // SAFETY: names the actual invariant (not boilerplate)
[ ] Every new user-facing type has a sdks/parity.toml row
[ ] Every new public fn / method has a # Errors doc section
[ ] Every Cargo.toml dep change has a justification comment
[ ] Cross-platform: every libc::* call has cfg(target_os = "...") if non-portable
[ ] TOML: [target.'cfg(...)'.X] tables come AFTER the broad [X] table
[ ] No #[allow(dead_code)] / #[allow(unused)] introduced
[ ] No stamped boilerplate SAFETY comments introduced
[ ] Tests assert == not <= for deterministic outputs
[ ] No new TODO / FIXME / HACK / XXX without a linked issue
[ ] CHANGELOG.md updated under [Unreleased] if user-facing
[ ] scripts/check_banned_vocab.py passes (run locally)
[ ] scripts/check_safety_comment_boilerplate.py passes
[ ] scripts/check_binding_parity.py passes
```

---

## 10. References

- `CONTRIBUTING.md` — development setup, pre-commit checks, PR process.
- `SECURITY.md` — disclosure policy.
- `.github/workflows/ci.yml` — every gate above is wired here.
- `scripts/` — gate implementations.
- `sdks/parity.toml` — cross-binding parity matrix (Gate 2).
- `disruptor-rs` PR #35 — Nicholas-lens prior art:
  https://github.com/nicholassm/disruptor-rs/pull/35
