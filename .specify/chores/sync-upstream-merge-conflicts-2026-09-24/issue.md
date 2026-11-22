# Chore Issue: sync: upstream merge conflicts require manual resolution

- **Slug**: sync-upstream-merge-conflicts-2026-09-24
- **Fetched**: 2026-09-24
- **Issue**: 65
- **URL**: https://github.com/arrrrny/herdr/issues/65
- **State**: open
- **Author**: app/github-actions
- **Labels**: sync

## Body

The daily upstream sync encountered **2** merge conflict(s) and aborted per the fork-upstream-sync policy (AGENTS.md).

Auto-resolving in favor of upstream is **not** permitted — these conflicts contain fork-owned features that must be preserved.

## Conflicted files

- `src/pane.rs`
- `src/pane/agent_detection.rs`

## Resolution procedure

1. Create a branch: `git switch -c sync/fork-sync-resolution upstream/master`
2. Merge master into it: `git merge master --no-ff`
3. Resolve each conflict, preserving fork-owned code (macOS notification click handler, login-shell PATH, badge CLI verb, agent lifecycle hooks, ziki detection, etc.).
4. Run `just check` and the fork-owned test suites (see `.github/FORK_OWNED_FILES` for the markers to grep for).
5. Open a PR titled `chore: sync upstream $(date +%Y-%m-%d)` and reference this issue.
6. After the PR merges, the next scheduled sync will be a no-op.

## Commit refs

- local master: `0b159859790c79d1fcb8e1f204d38020472f6fd4`
- upstream/master: `9c96f7ddb3be2cc575a159d4d1f1d49fb10d7006`

cc: the arrrrny fork maintainer.

## Comments

None.