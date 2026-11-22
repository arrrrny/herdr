# Bug: agent start fails on freshly created panes

- Slug: agent-start-fails-on-freshly-created-panes
- Fixed: 2026-10-26
- Assessment: ./assessment.md
- Issue: arrrrny/herdr#86
- Status: applied

## Summary

Relaxes the "available shell" / busy detection so a freshly spawned pane whose
foreground process group has not yet settled is treated as available (idle)
instead of busy, and makes the CLI retry loop wait for settlement rather than
giving up. Clears stale managed-agent state when a pane dies so a timed-out
start does not leave the agent name "taken", and makes the CLI timeout path
wait briefly for the server to actually release the name before returning.

## Changes

| File | Change |
| --- | --- |
| `src/platform/macos.rs` | `available_pane_shell` falls back to inspecting the child process when `foreground_job` is `None` (freshly spawned shell not yet settled); returns `Some(name)` if the child is a recognized pane shell. |
| `src/platform/linux.rs` | Same fallback using `process_name_and_parent`. |
| `src/cli/agent.rs` | `process_info_shows_shell_initialization` treats a missing/zero foreground process group id as "still initializing" and treats absent foreground process info as initializing. |
| `src/cli/agent.rs` | `wait_for_named_agent` timeout path polls up to ~1s for the server to release the managed-agent name before returning. |
| `src/app/actions.rs` | `handle_pane_died` clears the dying pane's terminal managed-agent name before removal. |

## Tests

- `cargo build` succeeds.
- `cargo test --bin herdr cli::agent` — 10 passed.
- `cargo test --bin herdr app::actions` — 105 passed.
- `cargo test --bin herdr terminal::state` — 123 passed.
- Pre-existing failures in `platform::macos::launchd_spawn` (environment/control-char in plist value) are unrelated.

## Deviations from Assessment

- The "pane should not disappear" expectation is addressed only partially: the
  shell exiting under load is not directly preventable here; the fix prevents
  stale names and false busy refusals, and clears state on death. A genuine
  shell exit still removes the pane (existing behavior). Detection-on-headless-
  server question remains open.
