# Chore Implementation: Resolve the 2026-09-23 upstream sync conflicts

- **Slug**: sync-upstream-merge-conflicts-2026-09-23
- **Issue**: [#62](https://github.com/arrrrny/herdr/issues/62)
- **Branch**: `sync/fork-sync-resolution-2026-09-23`
- **Status**: applied

## Applied

- Merged current `origin/master` into current `upstream/master` with a manual conflict resolution.
- Preserved fork-owned notification, login-shell PATH, badges, Ziki detection, config-write, and agent-lifecycle behavior.
- Restored the fork's generated preview snapshot and manifest as one unit, including the removal of fork-deleted `connecting-machines` pages.
- Hand-unioned the Windows input gauntlet, Unix helpers, and server client-shell changes so upstream behavior and fork-owned markers coexist.
- Kept the upstream detection test policy while retaining Ziki runtime support.
- Forced periodic lifecycle scans to bypass unchanged-content skips and refreshed the screen fallback while fresh hook authority remains effective.
- Closed the server PTY master before waiting in the unavailable-pane restore integration test, matching the existing shutdown-test pattern.
- Extended the fork-owned survival-marker list for the changed detection and Windows input/report paths.

## Deviations from Assessment

- The issue recorded upstream `9903007e`; the resolution used the later fetched `c958833f`, as documented in the assessment.
- The lifecycle refresh fix was added after review identified a stale-fallback path not covered by the original conflict resolution.
- The Windows client trace reader now uses the shared retry helper and resets its cursor when the rotating log is recreated.
- Matrix-drift reporting keeps arbitrary out-of-matrix observation identities without sorting them, avoiding mixed-type exceptions before `report.json` is written.

## Validation

- `git diff --check` and `git diff --cached --check`: passed.
- `python3 scripts/fork_owned_guard.py`: passed with 39 entries.
- `python3 -m unittest scripts.test_windows_input`: 23 tests passed.
- `python3 -m unittest scripts.test_fork_owned_guard`: 15 tests passed.
- `node scripts/docs/preview.mjs check`: passed.
- Focused lifecycle refresh tests: passed.
- All `terminal::state::tests`: 107 tests passed.
- `unavailable_restored_pane_keeps_saved_cwd_in_server`: passed.
- `ssh_check_message_is_visible_while_authentication_waits`: passed when rerun independently.
- Full `just check`: passed, including 3,658 Rust tests, 160 maintenance tests, integration-assets, Windows cross-lint, and documentation contract tests.
- Windows native qualification remains an interactive Windows-only check; no native desktop was available in this macOS environment.
