# Branch protection (rulesets as code)

`main-branch-protection.json` is the branch-protection ruleset for `main`, kept in the repo so the required-checks policy is reviewable and version-controlled instead of living only in the GitHub UI.

## Canonical required status checks

Exactly four checks must pass before a PR can merge to `main`:

| Check | Workflow | Rolls up |
|-------|----------|----------|
| `CI gate` | `ci.yml` | every fast-lane + heavy job in the core CI workflow |
| `Python gate` | `python.yml` | the wheel matrix and its stub / smoke / nogil / freethreaded jobs |
| `TypeScript gate` | `typescript.yml` | the napi build + test + publish jobs |
| `Perf gate` | `perf-gate.yml` | the decode-allocations regression gate |

Each of the four is an `if: always()` aggregator that depends on every job in its workflow and fails unless every dependency finished `success` or `skipped`. Requiring the four aggregators — instead of a hand-maintained list of individual job names — means two things stay true automatically:

- A heavy job that legitimately skips on a path-irrelevant PR (e.g. the Python wheel matrix on a docs-only change) counts as a pass, so a conditionally-skipped check can never leave the merge gate permanently unsatisfiable.
- A newly added job is covered the moment it lands in its workflow's aggregator `needs:` list — no ruleset edit, and no risk of a new job silently escaping the gate.

Do not require the individual jobs (`fmt`, `clippy`, `Build wheel (...)`, etc.) directly. They are covered transitively by the aggregators, and pinning them by name reintroduces the skip-counts-as-unsatisfiable problem the aggregators exist to solve.

Each required check is pinned to `integration_id: 15368`, the GitHub Actions app ID. That keeps a different status producer from satisfying a required context by reusing the same check name.

The pull-request review count is intentionally `0` for solo-maintainer operation. The merge queue plus app-pinned aggregate checks are the mandatory merge protection.

## Applying it

Importing or updating a ruleset is a repo-admin action on GitHub; it cannot be done from a PR. To apply this file:

- GitHub UI: **Settings -> Rules -> Rulesets -> New ruleset -> Import a ruleset**, then select this JSON.
- Or with the API:

  ```bash
  gh api -X POST repos/userFRM/ThetaDataDx/rulesets --input .github/rulesets/main-branch-protection.json
  ```

  To update an existing ruleset, `PUT repos/userFRM/ThetaDataDx/rulesets/<id>` with the same body.

After applying, confirm the four contexts above are listed under the ruleset's required status checks, and that the previous single-check requirement (`CI gate` only) is replaced by all four.

## Keeping this file honest

The check names here must match the aggregator job `name:` values in the workflows (`CI gate`, `Python gate`, `TypeScript gate`, `Perf gate`). If an aggregator is renamed, update both the workflow and this file, then re-import.
