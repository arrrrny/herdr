# Bug Fix: agent wait returns early when a child process finishes (issue #5)

- Slug: agent-wait-returns-early-child-processes
- Fixed: 2026-08-23
- Assessment: ./assessment.md
- Status: applied

## Summary

`herdr agent wait <pane>` returned almost immediately (claiming the agent was
done) while the agent was still working, whenever a child process running inside
the same pane finished. The root cause was in `src/api/wait.rs`:
`wait_for_resolved_agent` resolved the wait directly from a
`PaneAgentDetected { released: true, final_status }` *event* without probing the
real, stable terminal state or checking the reporting process identity. A child
process exiting in the same pane emits that exact event, so the wait ended on the
child's release — not on the foreground agent's end-of-turn. This is the
"child processes overwrite pane agent state" symptom from issue #5 (mirror of
upstream herdrdev/herdr#2851).

The already-merged commit `1980fc6b` ("fix(agent wait): reject transient status
events from child processes (#18)") addressed the *transient flicker* half of the
bug (the `PaneAgentStatusChanged` path) by setting `accept_transient_status:
false` and removing the early `Matched` return driven by transient status. It did
**not** touch the `PaneAgentDetected { released }` path, which had the same
early-return defect for process-release events. This change closes that gap.

After this fix, `agent wait` only ever resolves on the real, probed terminal
state (`agent get`), never on a transient status change or a release event
alone. A child process finishing — or any transient flicker — can no longer end
the wait early; the answer returned is always the real, current state.

## Changes

| File | Change | Notes |
| --- | --- | --- |
| `src/api/wait.rs` | `PaneAgentDetected { released: true }` now sets `should_probe = true` instead of returning `Matched`/`agent_wait_not_running` directly from the event's `final_status`. | The wait only resolves after probing the real terminal state, so a child-process release no longer ends the wait early. |
| `src/api/wait.rs` | Removed the now-dead `matched_event_status` local and the `accept_transient_status` field on `ResolvedAgentWait` (and its three construction sites). | After #18, transient status is never trusted, so the flag and the transient-status accumulator were dead. Keeps the change minimal and warning-free. |
| `src/api/wait.rs` | Added regression tests in `mod tests::child_process_regression` (`agent_wait_ignores_child_release_flicker`, `agent_wait_returns_on_real_stable_done`). | Deterministic unit tests driving `wait_for_resolved_agent` with a mock agent backend and an in-memory `EventHub`. |

## Diff Highlights

Key behavioral change in the event loop of `wait_for_resolved_agent`:

```rust
EventData::PaneAgentDetected { pane_id: event_pane, agent, released, .. }
    if event_pane == pane_id =>
{
    if released {
        // A release (process exit) may come from a child process running inside
        // the same pane, not the foreground agent. Never resolve the wait on the
        // transient release signal alone; only the real, probed terminal state
        // settles it, so a child process finishing cannot end the wait early (#5).
        should_probe = true;
    } else if agent.is_some() && expected_agent.is_some() && agent != expected_agent {
        return agent_wait_not_running(request_id).map(AgentWaitOutcome::Response).map(Some);
    } else {
        should_probe = true;
    }
}
```

The loop still probes the real state on `should_probe` and resolves via
`agent_wait_matches(&current, &wait.until, wait.after_state_change_seq)`. For a
genuine foreground-agent release the probe reads the settled status and the wait
returns `Matched`; for a child-process release the probe still reads the running
foreground agent and the wait continues.

## Tests Added or Updated

- `src/api/wait.rs` → `tests::child_process_regression::agent_wait_ignores_child_release_flicker`
  Pushes a `PaneAgentDetected { released: true, final_status: Done }` event while
  the mocked real agent state stays `Working`. Asserts the wait does **not**
  return `Matched` and instead times out. This directly pins "wait does not
  return early when a child process finishes".
- `src/api/wait.rs` → `tests::child_process_regression::agent_wait_returns_on_real_stable_done`
  Pushes a status change while the mocked real agent state is `Done`. Asserts the
  wait returns `Matched(Done)`. Positive control proving the wait still resolves
  on the real, stable state.

Both tests use a deterministic harness: an in-memory `EventHub`, a mock
`ApiRequestSender` (tokio unbounded channel + background thread answering
`agent get` with a shared live status), and a connected `UnixStream` pair to
satisfy the connection-liveness probe. No live panes.

Verification that the tests are meaningful: reverting the fix (restoring the
pre-fix `Matched`-on-release behavior) makes
`agent_wait_ignores_child_release_flicker` FAIL (panics "agent wait must not
return early on a child process release"), while the fix makes it PASS.

`cargo test --bin herdr child_process_regression` → 2 passed, 0 failed.
`cargo test --bin herdr agent_wait` (broader filter) → 4 passed, 0 failed.

## Local Verification

```
export ZIG=/workspace/herdr/.cache/zig/zig-x86_64-linux-0.15.2/zig
cargo test --bin herdr child_process_regression
# running 2 tests
# test agent_wait_returns_on_real_stable_done ... ok
# test agent_wait_ignores_child_release_flicker ... ok
# test result: ok. 2 passed; 0 failed
```

`cargo check --bin herdr` is clean (no warnings).

## Deviations from Assessment

The assessment was a stub ("NEEDS CLARIFICATION" for symptom/reproduction/root
cause). Investigation concluded the concrete mechanism is the `released`-event
early return in `src/api/wait.rs`, not a deeper state-attribution rewrite. The
fix is scoped to the wait primitive as requested ("minimal changes", "if a real
gap remains, fix it minimally").

## Follow-ups

- **Deeper pid/token attribution hardening (state.rs):** This fix makes the wait
  layer correct regardless of attribution. The desirable "stable,
  pid/token-tagged agent state so child processes cannot impersonate the pane's
  agent" (from the issue) lives in `src/terminal/state.rs`
  (`set_hook_authority_with_session_ref`, `live_full_lifecycle_hook_authority_conflicts_with_session`).
  If a child process reuses the foreground agent's `agent_session_id`/path, its
  `report-agent` could still overwrite the pane's *effective* state; the wait
  layer would then read that overwritten state. The existing session-ref conflict
  logic mitigates different-session children, but a fully pid-aware ownership
  check is a separate, larger change worth tracking as its own issue.
- Consider a cross-check test in `tests/` that drives a real `App` + `EventHub`
  through `report-agent` from a child session id to confirm attribution rejects
  it, complementing this wait-layer regression test.
