# v7.0.0 Phase 0 — Cleanup Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Land the five non-breaking cleanup items (L1, L2, L3, L4, L14) from the v7.0.0 scorched-earth spec. Phase 0 is quick-win territory: delete dead code, fix the two shipped-broken CI gates, and remove stale migration docs. No breaking changes.

**Architecture:** All work happens on the `release/v7.0` branch. Each kill-list item is one commit. The phase gate runs after every commit; a failing gate blocks the next task.

**Tech Stack:** Rust (cargo fmt/clippy/test), Python (regex in `check_docs_consistency.py`), markdown deletions, git.

**Spec:** `docs/superpowers/specs/2026-04-10-v7-scorched-earth-design.md`

---

## Pre-flight

Verify you are on `release/v7.0`:

```bash
git rev-parse --abbrev-ref HEAD    # expect: release/v7.0
git status                          # expect: clean working tree
```

If not, abort and switch:

```bash
git checkout release/v7.0
```

---

## Task 1: Delete commented-out Python methods (L1)

**Files:**
- Modify: `sdks/python/src/lib.rs` — delete lines `1135-2269` (a single `/* ... */` block of 1,134 dead lines)

**Context:** These are hand-written Python methods that have already been replaced by the generated `include!("generated_historical_methods.rs")` at line 2334. They're wrapped in a C-style comment so they don't compile, but they take up 1,134 lines of noise. Verified the block boundaries via `grep -n '^\s*\*/\|^\s*/\*'` → `1135:    /*` and `2269:    */`.

- [ ] **Step 1: Confirm the block still exists and has the expected shape**

Run:
```bash
sed -n '1135p;2269p' sdks/python/src/lib.rs
```

Expected output:
```
    /*
    */
```

If the boundaries differ, STOP and re-read the file to find the current comment block bounds. Do not blindly delete by line number if the file has changed.

- [ ] **Step 2: Delete the block with `sed`**

```bash
sed -i '1135,2269d' sdks/python/src/lib.rs
```

- [ ] **Step 3: Verify the file still compiles**

```bash
cargo check --manifest-path sdks/python/Cargo.toml 2>&1 | tail -5
```

Expected: `Finished \`dev\` profile [...] target(s)` (may take ~30s first run).

If it fails, inspect `sdks/python/src/lib.rs` — the `include!("generated_historical_methods.rs")` line should now be around line 1200, inside the `#[pymethods] impl ThetaDataDx` block. The generated methods must still be in scope.

- [ ] **Step 4: Verify line count dropped by ~1,134**

```bash
wc -l sdks/python/src/lib.rs
```

Expected: ~1,265 lines (was 2,399). If the number is wrong, the block deletion missed something.

- [ ] **Step 5: Commit**

```bash
git add sdks/python/src/lib.rs
git commit -m "$(cat <<'EOF'
chore(py): delete 1,134 lines of commented-out legacy Python methods (L1)

These were hand-written #[pymethods] functions that have been
replaced by include!("generated_historical_methods.rs"). The block
was wrapped in /* ... */ comments rather than removed. Delete for
real in preparation for v7.0.0.

Part of v7.0.0 scorched earth refactor (Phase 0).

Co-Authored-By: Claude Opus 4.6 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 2: Delete `public-api-redesign.md` (L2)

**Files:**
- Delete: `docs/public-api-redesign.md` (339 lines, unimplemented aspirational plan)

**Context:** The user confirmed this document was never implemented. It proposes a two-layer "exact + ergonomic" API that doesn't exist in code. It's being deleted to eliminate confusion between agents reading it as a source of truth vs. as a draft.

- [ ] **Step 1: Check for references to the file**

```bash
grep -rn 'public-api-redesign' --include='*.md' --include='*.ts' --include='*.yml' --include='*.rs' --include='*.toml' . 2>/dev/null | grep -v node_modules | grep -v target
```

Expected: zero matches OR only matches inside the spec doc and the file itself.

If there are references in `docs-site/docs/.vitepress/config.ts`, `README.md`, or other docs, they need to be removed as part of this task.

- [ ] **Step 2: Delete the file**

```bash
git rm docs/public-api-redesign.md
```

- [ ] **Step 3: Remove any references found in Step 1**

For each reference found, remove the line (or the containing nav entry). Example for a vitepress nav:

```bash
# If grep showed: docs-site/docs/.vitepress/config.ts:N: { text: 'Public API Redesign', link: '/public-api-redesign' }
# Open the file and delete that line
```

If no references existed, skip this step.

- [ ] **Step 4: Verify no dangling references**

```bash
grep -rn 'public-api-redesign' --include='*.md' --include='*.ts' --include='*.yml' --include='*.rs' --include='*.toml' . 2>/dev/null | grep -v node_modules | grep -v target
```

Expected: zero matches (the spec mention doesn't count if it's quoting the filename — that's allowed).

- [ ] **Step 5: Commit**

```bash
git add -A
git commit -m "$(cat <<'EOF'
docs: delete public-api-redesign.md (L2)

This document proposed an "exact + ergonomic" two-layer API that was
never implemented. It's being deleted to eliminate confusion with the
actual v7 direction, which is a single generated surface across all
SDKs. If a future ergonomic layer is desired, it will be spec'd
separately when it's actually being built.

Part of v7.0.0 scorched earth refactor (Phase 0).

Co-Authored-By: Claude Opus 4.6 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 3: Fix `cargo fmt` failure in `build_support/endpoints.rs` (L3)

**Files:**
- Modify: `crates/thetadatadx/build_support/endpoints.rs` (~16 whitespace hunks)

**Context:** Codex shipped 1,009 lines of new code in this file without running `cargo fmt`. `cargo fmt --all -- --check` fails with ~16 diffs around lines 1827, 1947, 2149, 2165, 2179, and beyond. All are whitespace — no logic changes.

- [ ] **Step 1: Confirm the failure state**

```bash
cargo fmt --all -- --check 2>&1 | grep -c '^Diff in'
```

Expected: a number `>= 1` (there are pending fmt diffs). If zero, the file is already formatted and this task is a no-op — skip to Step 4.

- [ ] **Step 2: Apply formatting**

```bash
cargo fmt --all
```

- [ ] **Step 3: Verify the file now passes the check**

```bash
cargo fmt --all -- --check
echo "exit=$?"
```

Expected: `exit=0` with no diff output.

- [ ] **Step 4: Verify tests still pass (fmt should not change behavior)**

```bash
cargo check --workspace 2>&1 | tail -3
```

Expected: `Finished` line with no errors.

- [ ] **Step 5: Commit**

```bash
git add crates/thetadatadx/build_support/endpoints.rs
git commit -m "$(cat <<'EOF'
style: cargo fmt build_support/endpoints.rs (L3)

Codex shipped ~1000 new lines in this file without running fmt,
causing CI to fail on the format check. Pure whitespace cleanup.

Part of v7.0.0 scorched earth refactor (Phase 0).

Co-Authored-By: Claude Opus 4.6 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 4: Fix `check_docs_consistency.py` regex drift (L4)

**Files:**
- Modify: `scripts/check_docs_consistency.py` line 217 (change target file from `sdks/go/client.go` to `sdks/go/generated_endpoint_options.go`)

**Context:** In commit `8582412` (Codex's "generate public sdk endpoint surfaces"), the Go `EndpointRequestOptions` struct was moved from `sdks/go/client.go` to `sdks/go/generated_endpoint_options.go`. The consistency checker still looks in the old location and fails with `missing expected struct pattern`.

- [ ] **Step 1: Confirm the failure state**

```bash
python3 scripts/check_docs_consistency.py 2>&1 | tail -3
```

Expected output contains:
```
docs consistency error: sdks/go/client.go missing expected struct pattern: 'type EndpointRequestOptions struct \{(.*?)\n\}'
```

If it doesn't fail with that exact message, STOP and re-read the script — the failure mode may be different from what the spec assumed.

- [ ] **Step 2: Confirm the struct now lives in the new file**

```bash
grep -l 'type EndpointRequestOptions struct' sdks/go/*.go
```

Expected: `sdks/go/generated_endpoint_options.go`

If the struct is in a different file, use that path in Step 3 instead.

- [ ] **Step 3: Update the script to look in the correct file**

Edit `scripts/check_docs_consistency.py` at line 217:

```python
# OLD:
    go_fields = extract_struct_fields(
        ROOT / "sdks/go/client.go",
        r"type EndpointRequestOptions struct \{(.*?)\n\}",
        r"^\s*([A-Z][A-Za-z0-9]+)\s+\*",
    )
```

Replace with:

```python
    go_fields = extract_struct_fields(
        ROOT / "sdks/go/generated_endpoint_options.go",
        r"type EndpointRequestOptions struct \{(.*?)\n\}",
        r"^\s*([A-Z][A-Za-z0-9]+)\s+\*",
    )
```

Also update the error message at line 226:

```python
# OLD:
        fail(
            "sdks/go/client.go EndpointRequestOptions fields drifted from endpoint_surface.toml. "
            f"missing={missing or '[]'} extra={extra or '[]'}"
        )
```

Replace with:

```python
        fail(
            "sdks/go/generated_endpoint_options.go EndpointRequestOptions fields drifted from endpoint_surface.toml. "
            f"missing={missing or '[]'} extra={extra or '[]'}"
        )
```

- [ ] **Step 4: Run the checker and verify it passes**

```bash
python3 scripts/check_docs_consistency.py
echo "exit=$?"
```

Expected: `exit=0` with no error output.

If it still fails — but with a DIFFERENT error message — that's fine for this task (L4 only fixes the Go drift); the new error is a separate L4-adjacent issue to record. Inspect the new error and update this task or file a follow-up.

- [ ] **Step 5: Commit**

```bash
git add scripts/check_docs_consistency.py
git commit -m "$(cat <<'EOF'
fix(ci): point docs consistency checker at generated_endpoint_options.go (L4)

In commit 8582412 the Go EndpointRequestOptions struct moved from
sdks/go/client.go to sdks/go/generated_endpoint_options.go, but the
consistency checker still looked in the old file. Update the regex
target so the check passes against Codex's own output.

Part of v7.0.0 scorched earth refactor (Phase 0).

Co-Authored-By: Claude Opus 4.6 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 5: Delete `migration-from-rest-ws.md` (L14)

**Files:**
- Delete: `docs-site/docs/getting-started/migration-from-rest-ws.md` (304 lines)
- Modify: `docs-site/docs/.vitepress/config.ts` (remove nav entry at line 51)

**Context:** This guide helps users migrate from the old REST/WS API to the current SDK. Since v7.0 will ship with a clean break and new docs, the old migration story is irrelevant. Also verified the nav reference exists via `grep -n 'migration-from-rest' docs-site/docs/.vitepress/config.ts` → line 51.

- [ ] **Step 1: Delete the markdown file**

```bash
git rm docs-site/docs/getting-started/migration-from-rest-ws.md
```

- [ ] **Step 2: Remove the nav entry from vitepress config**

Open `docs-site/docs/.vitepress/config.ts` and find the line (should be around line 51):

```ts
          { text: 'Migration from REST & WS', link: '/getting-started/migration-from-rest-ws' },
```

Delete that entire line. If the preceding or following line has a trailing comma that becomes problematic, fix it too.

- [ ] **Step 3: Verify no other references to the file**

```bash
grep -rn 'migration-from-rest-ws' docs-site/ 2>/dev/null | grep -v 'node_modules\|\.vitepress/dist\|\.vitepress/cache'
```

Expected: zero matches (ignoring stale build artifacts in `dist/` and `cache/`).

If there are other references (e.g., inline cross-links from another docs page), follow them and either update or remove them.

- [ ] **Step 4: Verify git status is clean after manual edit**

```bash
git status
```

Expected:
```
Changes to be committed:
  deleted: docs-site/docs/getting-started/migration-from-rest-ws.md
Changes not staged for commit:
  modified: docs-site/docs/.vitepress/config.ts
```

- [ ] **Step 5: Stage and commit**

```bash
git add docs-site/docs/.vitepress/config.ts
git commit -m "$(cat <<'EOF'
docs: delete migration-from-rest-ws.md (L14)

v7.0 is a clean break. The REST/WS → SDK migration story was written
for v5/v6 users and is no longer needed. v7 docs will start fresh
with a post-release migration guide from v6 → v7.

Part of v7.0.0 scorched earth refactor (Phase 0).

Co-Authored-By: Claude Opus 4.6 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 6: Phase 0 verification gate

**Files:** none (verification only)

**Context:** Every phase ends with a full verification gate per the spec. All five tasks must be on the branch and every gate command must pass before Phase 0 is considered complete.

- [ ] **Step 1: Run cargo fmt check**

```bash
cargo fmt --all -- --check
echo "fmt_exit=$?"
```

Expected: `fmt_exit=0`

- [ ] **Step 2: Run clippy with warnings-as-errors**

```bash
cargo clippy --workspace --all-targets -- -D warnings 2>&1 | tail -5
echo "clippy_exit=$?"
```

Expected: `clippy_exit=0`, no `error:` lines, ends with `Finished`.

- [ ] **Step 3: Run the full workspace test suite**

```bash
cargo test --workspace 2>&1 | grep 'test result' | tail -10
echo "test_exit=$?"
```

Expected: every `test result` line reads `ok. N passed; 0 failed`, and `test_exit=0`.

- [ ] **Step 4: Run the docs consistency checker**

```bash
python3 scripts/check_docs_consistency.py
echo "docs_exit=$?"
```

Expected: `docs_exit=0` with no error output.

- [ ] **Step 5: Run the FFI release build**

```bash
cargo build --release -p thetadatadx-ffi --locked 2>&1 | tail -3
echo "ffi_exit=$?"
```

Expected: `Finished \`release\` profile` and `ffi_exit=0`.

- [ ] **Step 6: Run Go SDK build and tests**

```bash
(cd sdks/go && go build ./... && go test ./...) 2>&1 | tail -5
echo "go_exit=$?"
```

Expected: `ok github.com/userFRM/thetadatadx/sdks/go` and `go_exit=0`.

- [ ] **Step 7: Run Python SDK cargo check**

```bash
cargo check --manifest-path sdks/python/Cargo.toml --locked 2>&1 | tail -3
echo "py_exit=$?"
```

Expected: `Finished \`dev\` profile` and `py_exit=0`.

- [ ] **Step 8: If all gates pass, push the branch**

```bash
git push origin release/v7.0
```

- [ ] **Step 9: Verify origin has all 5 Phase 0 commits**

```bash
git log --oneline origin/release/v7.0 ^origin/main
```

Expected: 5 commits with subjects:
```
docs: delete migration-from-rest-ws.md (L14)
fix(ci): point docs consistency checker at generated_endpoint_options.go (L4)
style: cargo fmt build_support/endpoints.rs (L3)
docs: delete public-api-redesign.md (L2)
chore(py): delete 1,134 lines of commented-out legacy Python methods (L1)
```
(plus the spec commit at the bottom)

If a gate fails, STOP. Diagnose the failure, fix it as a new commit (don't amend — the spec says "each phase is one or more commits, no squashing"), and re-run the gate from Step 1.

---

## Phase 0 completion criteria

Phase 0 is complete when:
1. All 5 kill-list items (L1, L2, L3, L4, L14) have landed as commits on `origin/release/v7.0`
2. Every gate in Task 6 exits 0
3. `cargo fmt --all -- --check` passes (no unformatted code)
4. `python3 scripts/check_docs_consistency.py` passes (no docs drift)

When Phase 0 is complete, the next step is to write the Phase 1 plan (Rust/SDK prune — L5, L7, L8, L13).

---

## Spec coverage check (self-review)

Phase 0 covers spec items: L1 ✓, L2 ✓, L3 ✓, L4 ✓, L14 ✓.
Phase 0 does NOT cover: L5, L6, L7, L8, L9, L10, L11, L12, L13, L15 (those are Phases 1–6).

No gaps in Phase 0 coverage.
