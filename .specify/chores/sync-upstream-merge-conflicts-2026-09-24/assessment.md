# Chore Assessment: Resolve 2026-09-24 upstream sync conflicts

- **Slug**: sync-upstream-merge-conflicts-2026-09-24
- **Created**: 2026-09-24
- **Source**: https://github.com/arrrrny/herdr/issues/65
- **Verdict**: in scope
- **Size**: small (the conflicts themselves are isolated lines in two files; the PR carries a 2-commit upstream sync)

## Report

The daily upstream sync (`.github/workflows/sync-upstream.yml`) aborted with 2 merge conflicts:
- `src/pane.rs`
- `src/pane/agent_detection.rs`

Upstream commits being synced:
- `21fd121a` fix: prevent cursor flicker on status redraw (#4554)
- `9c96f7dd` fix: avoid inferring codex idle from terminal output (#4563)

## Conflict Analysis & Proposed Approach

1. `src/pane.rs`: Both sides added local variables to detection loops.
   - Upstream: `let mut last_codex_prompt_ready = false;`
   - Fork: `let mut last_lifecycle_authority_scan_at: Option<Instant> = None;`
   - Resolution: Keep the union of both variables across both loop spawn sites.

2. `src/pane/agent_detection.rs`: Both sides added unit tests for detection screen reading.
   - Upstream: `screen_read_skips_unchanged_ambiguous_codex_but_not_new_content_or_replacement`
   - Fork: `lifecycle_force_refresh_reads_unchanged_idle_bottom_buffer`
   - Resolution: Keep both unit tests.

All fork-owned features and markers from `.github/FORK_OWNED_FILES` must be preserved.
