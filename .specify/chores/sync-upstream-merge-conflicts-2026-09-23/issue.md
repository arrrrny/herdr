# Chore Issue: sync: upstream merge conflicts require manual resolution

- **Slug**: sync-upstream-merge-conflicts-2026-09-23
- **Fetched**: 2026-09-23
- **Issue**: 62
- **URL**: https://github.com/arrrrny/herdr/issues/62
- **State**: open
- **Author**: app/github-actions
- **Labels**: sync

## Body

The daily upstream sync encountered **17** merge conflict(s) and aborted per the fork-upstream-sync policy (AGENTS.md).

Auto-resolving in favor of upstream is **not** permitted — these conflicts contain fork-owned features that must be preserved.

## Conflicted files

- `distribution/preview.json`
- `docs/preview/website/src/content/docs/connecting-machines.mdx`
- `docs/preview/website/src/content/docs/integrations.mdx`
- `docs/preview/website/src/content/docs/ja/connecting-machines.mdx`
- `docs/preview/website/src/content/docs/ja/integrations.mdx`
- `docs/preview/website/src/content/docs/ja/persistence-remote.mdx`
- `docs/preview/website/src/content/docs/ja/session-state.mdx`
- `docs/preview/website/src/content/docs/session-state.mdx`
- `docs/preview/website/src/content/docs/zh-cn/connecting-machines.mdx`
- `docs/preview/website/src/content/docs/zh-cn/integrations.mdx`
- `docs/preview/website/src/content/docs/zh-cn/persistence-remote.mdx`
- `docs/preview/website/src/content/docs/zh-cn/session-state.mdx`
- `scripts/test_windows_input.ps1`
- `scripts/windows_input/Native.cs`
- `src/detect/manifest/tests.rs`
- `src/platform/unix_common.rs`
- `src/server/client_shell.rs`

## Resolution procedure

1. Create a branch from upstream and merge the fork master.
2. Resolve each conflict by hand, preserving fork-owned code and upstream changes.
3. Run `just check` and the fork-owned test suites.
4. Open a PR titled `chore: sync upstream 2026-09-23` and reference this issue.
5. After the PR merges, the next scheduled sync should be a no-op.

## Commit refs

- local master: `81e1152e1f44c1ecf72cc1d9f42faf8ec6d0222f`
- upstream/master recorded by the issue: `9903007e5c4407ded451cac0b8164e3d69deb891`
- current upstream/master fetched for the resolution: `c958833fa81333bf07a255f7d1325e6892daca1e`

cc: the arrrrny fork maintainer.
