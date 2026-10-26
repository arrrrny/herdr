---
feature: 83-ci-conventional-commits-fails-on-every-pr-merge-into-master
verdict: PASS_WITH_GAPS
standard: .specify/extensions/tdd/templates/tdd-test-quality-rubric.md # rubric graded against
verified_at: 8a9b2d88 # short SHA audited (HEAD of master history + uncommitted fix branch work)
behaviors: 2
proven: 1
likely: 1
test_after: 0
no_test: 0
high_smells: 0
criteria_total: 4
criteria_covered: 4
mutation_score: 100 # scope: scripts/conventional_commits.py only, 4 deliberate mutants (no mutation tool configured for Python in this repo), all killed
mutants_survived: 0
suite: 11 passed, 0 failed (scripts.test_conventional_commits, 0.28s); 206 passed, 0 failed (maintenance-test unittest neighbours, 41.9s)
verified_utc: 2026-10-26T22:27:27Z
---

# TDD Verification: ci conventional-commits merge-subject exemption (#83)

**Verdict: PASS_WITH_GAPS.** Every acceptance criterion has a test through the
real entry point (the script invoked exactly as CI invokes it), no HIGH smells,
and all 4 deliberate mutants were killed. The gap: the test-first red for the
new behavior is recorded in the work-order session (quoted below), not in a
committed cycle-log artifact, and the fix and its tests land in a single commit,
so git history alone cannot corroborate the ordering.

## Scope

Chore touching one behavior-bearing file: `scripts/conventional_commits.py`
(+ new `scripts/test_conventional_commits.py`, + one-line registration in the
justfile `maintenance-test` recipe). No Rust, no workflow YAML, no runtime code
was changed.

## Test-first evidence

| Behavior | Class  | Evidence                                                                                                                        |
| -------- | ------ | ------------------------------------------------------------------------------------------------------------------------------- |
| B1: git-generated merge/revert subjects are exempt from validation | LIKELY | Red recorded before the fix: pre-fix run of the real script on a real merge range printed `invalid commit subject(s): Merge branch 'feat/widget'` and exited 1; the same range exits 0 post-fix (commands and outputs quoted below). No cycle-log file exists for work-order chores; test and source land in one commit, so history cannot corroborate order independently |
| B2: author-typed non-conventional subjects still fail | PROVEN | Behavior pre-exists the change; pinned by two tests at the real entry point; both author-subject mutants killed in the mutation pass; full neighbour suite green after the change |

Red-phase record (session, pre-fix, repository `/tmp/repro-83`, before any
change to `scripts/conventional_commits.py`):

```text
$ git log --pretty=format:%s "<before>..<after>"
Merge branch 'feat/widget'
feat: add widget
$ python3 scripts/conventional_commits.py --range "<before>..<after>"
invalid commit subject(s):
  Merge branch 'feat/widget'
...
exit=1
$ python3 scripts/conventional_commits.py "Merge pull request #74 from arrrrny/feat/widget"
invalid commit subject(s):
  Merge pull request #74 from arrrrny/feat/widget
...
exit=1
```

Post-fix record (same range, plus the exact subject flagged on the #81 push):
`exit=0`; author junk subject `updated stuff` still exits 1.

## Findings

| # | Severity | Finding | Evidence |
| --- | -------- | ------- | -------- |
| 1 | MED | Test-first red for B1 lives in the session record (quoted above), not in a committed `cycle-log.md`; a future auditor cannot distinguish it from a fabricated red except by re-running the reproduction | this file, "Red-phase record" |
| 2 | LOW | `SubjectArgumentTests` asserts exit codes plus one `stdout` containment; exit-code-only failure messages would be less diagnostic | `scripts/test_conventional_commits.py:81-84` — mitigated by content assertions on the failure path |

No HIGH smells. The existing test files were not touched; no assertion was
removed, loosened, renamed, or skipped anywhere in the diff.

## Test smell catalogue (new/changed file: `scripts/test_conventional_commits.py`)

Checked every catalogue item against the repo exemplars
(`scripts/test_changelog.py`, `scripts/test_fork_owned_guard.py`,
`scripts/test_release.py`). Cleared items worth noting:

- *Conditional logic in the test*: subject lists are iterated with
  `self.subTest(subject=...)` — every iteration asserts, failures are labeled
  per subject; the loop decides nothing about whether to assert. Matches the
  repo's existing `subTest` convention (`test_release.py`,
  `test_unix_installer.py`).
- *Non-deterministic / mystery guest*: range tests build throwaway git repos in
  `tempfile.mkdtemp` with a fixed committer identity, no network, no sleeps,
  removed in `tearDown`. The only external binary is `git` itself, which is the
  subject's real collaborator, not a fixture side channel.
- *Doubled subject*: no stubs or doubles; `run_script` executes the actual
  script with `sys.executable` exactly as `ci.yml` does
  (`python3 scripts/conventional_commits.py ...`).
- *Magic values*: the subject literals are the specification itself (the exact
  default forms `git merge`, `gh pr merge`, and `git revert` emit, including the
  `sync/fork-sync-resolution-2026-10-15` form flagged on the #81 push).

## Test strength: deliberate mutants

No Python mutation tool is configured in this repo, so per the rubric's
fallback, deliberate mutants were applied one at a time to
`scripts/conventional_commits.py`, running the full
`scripts.test_conventional_commits` module each time; the file was restored and
byte-compared after every run (restore verified: working tree identical to the
intended fix).

| Mutant | Behavior | Survived | Judgment |
| ------ | -------- | -------- | -------- |
| `EXEMPT_SUBJECT_PREFIXES` drops `"Merge "` (Revert-only exemption) | B1 | No | Killed: `test_merge_commit_subjects_pass_range_validation` and the merge-form predicate tests fail |
| `EXEMPT_SUBJECT_PREFIXES` drops `'Revert "'` (Merge-only exemption) | B1 | No | Killed: `test_git_revert_default_title_passes` and the revert predicate assertion fail |
| `exempt_subject()` returns `False` (exemption disabled) | B1 | No | Killed: same merge-form entry-point tests fail |
| `exempt_subject()` returns `True` (swallows every subject) | B2, B1 | No | Killed: `test_author_subject_still_fails_range_validation`, `test_non_conventional_title_still_fails`, `test_author_typed_subjects_are_not_exempt` fail |

Sampled: all 4 risk-bearing points of the changed code (the prefix tuple's two
elements and the predicate's two degenerate forms); this is exhaustive for the
diff, not a sample. Score: 4/4 killed, 0 survived.

## Traceability: criteria to tests

| Criterion (from issue #83) | Tests | End to end |
| -------------------------- | ----- | ---------- |
| AC-1: `--range` push validation does not fail on git-generated merge subjects (`Merge pull request #NN from …`, `Merge remote-tracking branch …`, `Merge branch '…'`) | `RangeValidationTests.test_merge_commit_subjects_pass_range_validation`, `SubjectArgumentTests.test_pr_merge_default_title_passes` | Yes — real script, real git repo with real merge commits, invoked as CI invokes it |
| AC-2: `Revert "<subject>"` git-generated form is exempt | `ExemptSubjectTests.test_git_generated_merge_and_revert_forms_are_exempt`, `SubjectArgumentTests.test_git_revert_default_title_passes` | Yes at the CLI entry point; the range path shares the same `main()` filter |
| AC-3: author-typed non-conventional subjects still fail (preview-note hygiene preserved) | `RangeValidationTests.test_author_subject_still_fails_range_validation`, `SubjectArgumentTests.test_non_conventional_title_still_fails`, `ExemptSubjectTests.test_author_typed_subjects_are_not_exempt` | Yes |
| AC-4: conventional subjects remain valid in every path | `ValidSubjectTests.test_conventional_subjects_are_valid`, `MessageFileTests.test_message_file_first_subject_is_read`, plus conventional commits inside the AC-1 end-to-end range | Yes |

Untested criteria: none. Tests tracing to nothing: none.

## What was not audited

- `cargo build --locked` could not complete on this host for a pre-existing,
  unrelated reason: `crates/ghostty-vt/build.rs` requires Zig 0.16.0; the build
  was attempted with Zig provided and was still compiling when this audit was
  written. The chore changes no Rust code; `cargo fmt --check` passes.
- `bun test scripts/release-workflows.test.ts scripts/sync-upstream-workflow.test.ts`:
  12 pass / 1 fail — the failure (`release arguments are not interpolated into
  executable shell text`, `release-workflows.test.ts:56`) reproduces identically
  on a pristine `master` clone and is unrelated to this chore.
- Neighbour scope was the `maintenance-test` unittest set (206 tests) plus the
  bun workflow tests; the Rust integration suite (`cargo nextest`) was not run
  because no behavior-bearing Rust code was touched.
