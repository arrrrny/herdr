# Bug Fix: Hermes /resume session not persisted across restart

- Slug: hermes-resume-session-not-persisted-across
- Fixed: 2026-08-23
- Assessment: ./assessment.md
- Status: applied

## Summary

Inside a Hermes **TUI** pane, running `/resume` to switch from conversation A to
conversation B changes the active session, but Herdr kept associating the pane
with A. After a restart the pane resumed A, not B.

Herdr's core persistence path is correct: `pane report-agent-session` →
`AppEvent::AgentSessionReported` →
`TerminalState::set_agent_session_ref_for_session_start`
(`src/terminal/state.rs:1395`) updates `self.persisted_agent_session`
(`src/terminal/state.rs:1605`) and returns a `TerminalStateMutation` with
`session_ref_changed = true`, which marks the session dirty and triggers a
debounced save. Hermes is a `session_identity_only_integration`
(`src/detect/mod.rs:307`) but NOT a `full_lifecycle_hook_authority`, so reports
flow through the normal `accept_hook_report` path, which accepts
strictly-increasing `seq` (`src/terminal/state.rs:1667`). So **if** Hermes reports
the new session, Herdr persists it.

The gap is in the Hermes integration asset `src/integration/assets/hermes/__init__.py`.
It registers three hooks:

- `on_session_start` → reports source `"startup"`
- `on_session_reset` → reports source `"new"`
- `pre_llm_call` → `_session_observed`, which ONLY reported when
  `platform == "cli"`

Inside a Hermes **TUI** (`platform` `"tui"`/`"desktop"`/`"acp"`), a plain
`/resume` does not fire `on_session_start`/`on_session_reset`, so the only hook
that sees the new `session_id` is `pre_llm_call` — and that was gated to `cli`
only. The TUI resume was therefore never reported, and the persisted session
stayed on A. This mirrors the upstream opencode fix (commit ccccda54) which added
TUI root-session reporting.

Fix: `_session_observed` now reports the resumed session for every interactive
platform (`tui`/`desktop`/`acp`/`cli`) instead of only `cli`. A module-level
`_LAST_REPORTED` dedup cache keeps `_report_session` from re-spamming an identical
`session_id` for the same `platform:start_source` combo on every `pre_llm_call`.
The effect: Hermes now reports the TUI-selected session on the next LLM call,
Herdr learns the new session id, and persists it via the existing resume-replacement
path (the later, higher-seq `resume` report wins over A because
`session_report_allows_session_replacement("herdr:hermes","hermes",Some("resume"))`
is true — `src/terminal/state.rs:1344`).

## Changes

| File | Change | Notes |
| --- | --- | --- |
| `src/integration/assets/hermes/__init__.py:14-18` | Add `_LAST_REPORTED: dict[str, str]` dedup cache | Keyed by `"platform:start_source"`, avoids duplicate reports per `pre_llm_call` |
| `src/integration/assets/hermes/__init__.py:52-62` | `_report_session` records the reported id and skips a repeat for the same `platform:start_source` combo | Keeps the existing interactive-platform early return; still calls `_send_session` on a new id |
| `src/integration/assets/hermes/__init__.py:69-71` | `_session_observed` now calls `_report_session("resume", **kwargs)` unconditionally (removed the `platform == "cli"` gate) | TUI/desktop/acp resumes are now reported to Herdr |
| `src/terminal/state.rs:2673-2732` | Add regression test `hermes_tui_resume_session_persists_across_restart` | Guards the persistence-across-restart contract: a later `resume` report replaces the earlier `startup` persisted session |

## Tests Added or Updated

In `src/terminal/state.rs` `#[cfg(test)] mod tests`:

1. `hermes_tui_resume_session_persists_across_restart` — builds a `TerminalState`
   via `test_terminal()`, sets `detected_agent = Some(Agent::Hermes)` with
   `recent_agent_process_exit = None` (so `process_present` is true), then:
   - reports session `A` with `seq 1`, source `startup`; asserts
     `persisted_agent_session` is `Some` with id `A`;
   - reports session `B` with `seq 2`, source `resume`; asserts
     `persisted_agent_session` is now `Some` with id `B` (the later resume report
     wins — the persistence-across-restart contract);
   - asserts `current_session_identity_for_persistence()` reflects
     `("herdr:hermes", "hermes", AgentSessionRefKind::Id, "B")`.

   This is the direct regression guard for fork issue #12.

## Local Verification

```
export PATH="/workspace/herdr/.cache/zig/zig-x86_64-linux-0.15.2:$PATH"

cargo test --bin herdr terminal::state::tests::hermes_tui_resume_session_persists_across_restart
  -> test result: ok. 1 passed; 0 failed

cargo test --bin herdr terminal::state::tests
  -> test result: ok. 102 passed; 0 failed  (no regression in the module)

cargo build --bin herdr
  -> Finished (dev profile), compiled against vendored libghostty-vt via Zig 0.15.2

python3 -c "import ast; ast.parse(open('src/integration/assets/hermes/__init__.py').read())"
  -> hermes __init__.py: valid python
```

The new test fails before the asset fix (a `cli`-gated `_session_observed` never
reports the TUI resume, so `persisted_agent_session` stays on `A`) and passes after
it; the asset change is exercised at the unit level through the existing
`pane report-agent-session` → `set_agent_session_ref_for_session_start` core path.

## Deviations from Assessment

None. The prescribed fix matches the root cause and the new regression test
confirms the resumed session now persists (the later `resume` report replaces the
earlier `startup` persisted identity through the existing
`session_report_allows_session_replacement` allow-list — no core-path change was
needed).
