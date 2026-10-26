# Bug Assessment: agent start fails on freshly created panes

- **Slug**: agent-start-fails-on-freshly-created-panes
- **Created**: 2026-10-26
- **Source**: https://github.com/arrrrny/herdr/issues/86
- **Verdict**: valid
- **Severity**: high

## Report (verbatim or summarized)

GitHub issue arrrrny/herdr#86. On a long-lived headless server (herdr 0.9.5 stable, macOS 15.7.3 x86_64, ~29h uptime, ~30 tabs / 5 workspaces, concurrent lane agents), `agent start` against a pane created seconds earlier by `tab create` fails two ways back to back:

1. Refused immediately with `agent_pane_busy` — "agent target pane ... is not an available shell" — for panes the server just spawned whose shell is at its prompt (`pane get` reads `agent_status: unknown`, `foreground_cwd` set).
2. When accepted, the CLI readiness wait never sees the agent: every `agent.get` poll (~250 ms) errors, ending at timeout; the pane is already gone by then (`pane_not_found` / `tab_not_found`).

Frequency: 44 x `agent_pane_busy` and 1 x `agent:start:timeout` in one dispatcher log; the timeout case later produced `agent_name_taken` for the same name while `agent.get <name>` reported nothing. Tabs pile up with recycled ids (e.g. a new tab numbered 32 returned `wF6:t0`).

## Symptom

- `agent start` on a pane the server created moments earlier either (a) is refused with `agent_pane_busy` even though the pane sits at its prompt with no foreground process, or (b) starts but the CLI never observes the agent becoming ready and the pane disappears before the caller can retry/read it.
- A genuine start failure tears down the pane, so the caller must recreate a tab.
- A timed-out start leaves the agent name "taken" so a retry with the same name fails with `agent_name_taken` even though `agent.get <name>` reports nothing.

## Reproduction

1. Run a long-lived headless herdr server with several workspaces/tabs.
2. `herdr tab create --workspace <ws> --cwd <dir> --label task-<id>`
3. `herdr agent start <name> --kind <kind> --pane <root_pane> --timeout 90000 -- --auto`
4. Under load, observe `agent_pane_busy` (case 1) or timeout + vanished pane (case 2).

Marked unknowns: exact load threshold and whether the shell process actually exits vs. the pane id becomes stale; whether detection ever fires on a headless server with no attached TUI client.

## Suspected Code Paths

- `src/app/agents.rs:237` — `terminal.is_agent_terminal() || terminal.managed_agent_kind().is_some()` gate returns `TargetBusy`.
- `src/app/agents.rs:244` — `available_shell_name(runtime)` (delegates to `platform::available_pane_shell`) returns `None` for a freshly spawned shell whose foreground process group is not yet settled, producing `agent_pane_busy`.
- `src/platform/macos.rs:449` `available_pane_shell` / `foreground_job` — reads `e_tpgid`; a brand-new shell may report 0 or a transient pgid, so `foreground_job` returns `None` → busy.
- `src/cli/agent.rs:31` `agent_start` retry loop — retries on busy only while `pane_shell_is_initializing` (requires `foreground_process_group_id == shell_pid`); a pane whose fg pgid is not yet settled fails that check, so no retry and busy is surfaced immediately.
- `src/cli/agent.rs:380` `wait_for_named_agent` — polls `agent.get` by name then by pane id; returns timeout without ensuring the server has released the managed-agent name, so a retry hits `agent_name_taken`.
- `src/app/actions.rs:2059` `handle_pane_died` — removes the pane (and terminal if unattached); the shell exiting is what makes the pane vanish. Managed-agent state on a surviving terminal is not explicitly cleared here.
- `src/terminal/state.rs:2318` `reconcile_managed_agent_at` — clears the name only when `now >= deadline` (the full timeout), so a CLI timeout that fires before the server's deadline leaves the name taken.

## Root Cause Hypothesis

Confidence: medium.

Three coupled causes:

1. **Racy busy detection on fresh panes.** `available_pane_shell` treats "no settled foreground process group yet" as busy. A just-spawned shell has not yet become its own foreground process group leader (or `e_tpgid` reads 0 transiently), so the check fails even though no foreground command is running. The CLI's retry guard mirrors the same strict condition, so it gives up immediately instead of waiting for the shell to settle.

2. **Pane vanishing is the shell exiting.** Under load (resource pressure / many concurrent PTYs) the freshly spawned shell can exit, and `PaneDied` removes the pane. `agent start` has already sent the launch line; if the shell dies mid-flight the pane is gone by the time readiness polling finishes. Nothing in the start path guarantees the pane survives a failed/late start.

3. **Stale managed-agent name after timeout.** The CLI timeout (`timeout`) and the server's managed-agent deadline (`server_now + timeout`) are computed on different clocks; the CLI can time out and return before the server clears the name, so the next `agent start` with the same name reports `agent_name_taken` while `agent.get <name>` finds nothing. The timeout path's single `resolve_agent_target_unchecked` reconcile call does not wait for the server to actually release the name.

## Proposed Remediation

**Preferred**:

- Relax the busy signal for not-yet-settled shells: in `available_pane_shell` (unix), when `foreground_job` returns `None` but the child pid itself is alive and is a recognized pane shell, treat the pane as available (idle) instead of busy. Mirror this in the CLI's `pane_shell_is_initializing` so the retry loop waits for settlement rather than giving up.
- Make the CLI `agent start` timeout path wait (bounded, ~1s) for the server to release the managed-agent name before returning, so retries with the same name succeed.
- In `handle_pane_died`, explicitly clear the terminal's managed-agent state/agent name when the pane's terminal is being removed, so a dead pane cannot leave a stale "taken" name behind.
- Keep the pane alive on a failed start: do not close/remove the pane as a side effect of a start failure; the existing code already avoids this, but ensure `clear_agent_name` on input failure and the timeout reconcile path do not trigger pane removal.

**Alternatives**:

- Add a settle delay / readiness gate in `tab create` so a pane is only reported as created once its shell is the foreground process group leader. Trade-off: delays tab creation; cleaner but larger.
- Force-clear the managed agent on any `agent.get` reconcile when the caller is the timeout path. Trade-off: requires a new request flag or a separate method.

**Files likely to change**:

- `src/platform/macos.rs` (`available_pane_shell` / `foreground_job`)
- `src/platform/linux.rs` (same)
- `src/cli/agent.rs` (`pane_shell_is_initializing`, `process_info_shows_shell_initialization`, `agent_start` timeout path)
- `src/app/actions.rs` (`handle_pane_died`)
- `src/terminal/state.rs` (`reconcile_managed_agent_at` — possibly add a force-clear entry point)
- `src/app/agents.rs` (`start_agent` busy gate)

**Tests to add or update**:

- Unit: `available_pane_shell` returns `Some` when foreground info is missing but the child is a shell (macOS/Linux gated, synthetic).
- Unit: `process_info_shows_shell_initialization` returns true when `foreground_process_group_id` is absent/0 but `shell_pid` present and a shell process is listed.
- Unit: `handle_pane_died` clears managed-agent name on the removed terminal (AppState-level, with `AppState::test_new`).
- CLI/integration: timeout path releases the name so a second `agent start` with the same name succeeds.

## Risks & Considerations

- Relaxing the busy check could let `agent start` proceed on a pane whose shell is genuinely busy but whose fg pgid hasn't been read yet; mitigated by requiring the child pid to be a recognized pane shell and by the existing managed-agent/launch-pending gates.
- Clock-skew fix must remain bounded so a stuck server cannot hang the CLI indefinitely.
- Detection on headless servers (no TUI client) is unverified; if detection never fires, managed agents never become `Active` and every start times out. This may be a separate, larger issue requiring a detection-on-server path; flagged as [NEEDS CLARIFICATION].

## Open Questions

- [NEEDS CLARIFICATION: does agent detection run on a headless server with no attached TUI client? If not, managed starts can never succeed and this bug is partly a detection-lifecycle issue.]
- [NEEDS CLARIFICATION: is the pane's shell actually exiting under load, or is the pane id going stale due to tab-id recycling? Reproduction on a loaded host is needed to distinguish.]
