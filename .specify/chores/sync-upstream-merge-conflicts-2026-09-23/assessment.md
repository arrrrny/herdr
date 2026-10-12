# Chore Assessment: Resolve the 2026-09-23 upstream sync conflicts

- **Slug**: sync-upstream-merge-conflicts-2026-09-23
- **Created**: 2026-09-23
- **Source**: https://github.com/arrrrny/herdr/issues/62
- **Verdict**: in scope
- **Size**: large (17 conflicted paths, broad upstream merge; high blast radius)

## Summary

Issue #62 reports that the daily upstream sync aborted because 17 paths conflicted. The conflicts include the generated preview manifest and localized preview documentation, Windows input test infrastructure, detection tests, Unix platform helpers, and the server client-shell snapshot/rendering path. The resolution must be a hand-written merge that preserves the fork's macOS notification, login-shell PATH, badge, agent lifecycle, Ziki detection, and config-write behavior while retaining upstream's newer changes.

The issue records upstream commit `9903007e`, but the current upstream ref fetched during assessment is `c958833f` (a later descendant). The resolution branch is therefore based on the current `upstream/master` and merges the current `origin/master`; this keeps the PR current and makes the next scheduled sync a no-op if the merge is accepted.

## Protected behavior

- Fork-owned markers in `.github/FORK_OWNED_FILES` must remain intact.
- `src/platform/unix_common.rs` must retain `config_file_link_count`, symlink-safe config writes, and upstream FD/SSH helpers.
- `src/server/client_shell.rs` must retain badge projection and upstream completion/atomic-render changes.
- `src/terminal/state.rs`, `src/pane.rs`, `src/client/shell/sidebar.rs`, `src/api/server.rs`, and `src/detect/mod.rs` require semantic review because they are fork-guarded and receive upstream changes.
- The published `docs/preview/website/` tree and `distribution/preview.json` are an atomic generated snapshot. Restore the fork's published snapshot wholesale; do not blend individual preview documents or let an upstream preview commit overwrite the fork's published pointer.
- Do not use `git merge -X theirs`, `-s ours`, or automated conflict side-picking.

## Planned approach

1. Build `sync/fork-sync-resolution-2026-09-23` from current `upstream/master` and merge current `origin/master` with `--no-ff`.
2. Restore the fork's published preview snapshot and manifest as one generated unit, accepting the fork-side deletion of preview-only `connecting-machines` pages.
3. Hand-union the Windows gauntlet script and native helper: retain fork `herdr-remote`/non-key cleanup behavior and upstream retryable reads, owner-aware clipboard/image handling, and native-record cleanup.
4. Use the upstream detection test stage as the base, because current `AGENTS.md` prohibits agent-specific screen/rule-ID tests; retain runtime Ziki support and validate its generic manifest coverage.
5. Hand-union Unix helpers and client-shell code, preserving all fork markers and upstream render/completion behavior.
6. Audit clean-but-merge-sensitive files, run the fork guard, preview documentation validator, and `just check`; document any unavailable Windows validation.

## Validation

- `git diff --check`
- `python3 scripts/fork_owned_guard.py`
- `node scripts/docs/preview.mjs check`
- `just check`
- Windows cross-lint and native gauntlet validation if the configured SDK/toolchain is available; otherwise record the limitation and rely on CI.
- Confirm no unresolved conflict markers remain and the branch is based on current `origin/master` immediately before PR creation.

## Risks

- The generated preview tree has clean auto-merges in addition to the 17 listed conflicts; restoring the whole fork snapshot is required to avoid a mixed release contract.
- The sync changes protocol/client-shell and Windows input code, so a compile or behavioral regression can affect more than the conflicted files.
- `src/detect/manifest/tests.rs` contains fork-specific Ziki tests that conflict with the repository's current testing policy; runtime Ziki support must remain even if those screen-specific tests are not retained.
- The issue's recorded upstream ref is stale relative to the fetched ref; the current merge must be documented in `implement.md` as a deviation from the issue snapshot.
