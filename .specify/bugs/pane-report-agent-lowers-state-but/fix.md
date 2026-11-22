# Bug Fix: pane report-agent --state idle never lowers agent_status

- Slug: pane-report-agent-lowers-state-but
- Fixed: 2026-08-23
- Assessment: ./assessment.md
- Status: applied

## Summary

A pane whose agent is registered via `pane report-agent-session` and then driven
with `pane report-agent --state working` keeps reporting `agent_status: "working"`
forever: a later `pane report-agent --state idle` for the same pane returns `ok`
but `agent list` still says `working`, while `agent explain` (screen detection)
reports `idle`. Raising (`working -> blocked`) worked; only lowering (`-> idle`)
was ignored.

`pane report-agent` flows through `AppEvent::HookStateReported` →
`TerminalState::set_hook_authority_with_session_ref` →
`TerminalState::set_hook_authority_at`. For a live, session-anchored,
full-lifecycle hook (e.g. `herdr:pi`) the routing in
`route_full_lifecycle_hook_report` accepts the report; the only remaining gate
is `accept_hook_report`.

`accept_hook_report` (the previous code) handled an absent `--seq` as:

```rust
let Some(seq) = seq else {
    return !self.hook_report_sequences.contains_key(source);
};
```

Because the prior `report-agent --state working` (and `report-agent-session`)
register a sequence number for the source, `hook_report_sequences` contains the
source, so every later unsequenced `report-agent` returned `false` and was
silently dropped — exactly a state-*lowering* push, since the realistic
forklift/kimi integration pushes `--state` with `--agent-session-id` but no
`--seq`. Lowering was therefore never applied; the hook authority stayed at
`Working`, and `recompute_effective_state` kept `terminal.state` (hence
`agent_status`) at `Working`. Raising appeared to work because it was tested with
an explicit higher `--seq`, which the `seq <= last_seq` check allows through.

Fix: an unsequenced report represents the agent's *current* state and is
authoritative in real time, so `accept_hook_report` now accepts it without
establishing a numeric sequence baseline (it does not insert into
`hook_report_sequences`). The documented sequence-ordering guarantee for
*sequenced* reports is preserved untouched: a sequenced report with
`seq <= last_seq` is still rejected (stale/out-of-order), and a sequenced report
with a higher `seq` still advances the baseline. Unsequenced reports therefore
lower (and raise) `agent_status` correctly, and the existing
`AgentState` arbitration in `recompute_effective_state` applies the new state
because the reported agent is the pane's foreground agent.

## Changes

| File | Change | Notes |
| --- | --- | --- |
| `src/terminal/state.rs:1667-1682` | `accept_hook_report` now accepts an unsequenced (`None`) report instead of rejecting it once a sequence exists for the source | One-branch change; does not insert into `hook_report_sequences`, so sequenced ordering is unaffected |

## Diff Highlights

```rust
// src/terminal/state.rs  (fn accept_hook_report)
fn accept_hook_report(&mut self, source: &str, seq: Option<u64>) -> bool {
    let Some(seq) = seq else {
-       return !self.hook_report_sequences.contains_key(source);
+       // An unsequenced report represents the agent's current state and is
+       // authoritative in real time. Accept it without establishing a
+       // numeric sequence baseline, so a later sequenced (or unsequenced)
+       // report still orders correctly. Previously an unsequenced report
+       // was rejected once any sequence had been recorded for the source,
+       // which silently dropped state-lowering reports (e.g. working ->
+       // idle) from callers that do not send --seq. See issue arrrrny/herdr#9.
+       return true;
    };

    if self
        .hook_report_sequences
        .get(source)
        .is_some_and(|last_seq| seq <= *last_seq)
    {
        return false;
    }

    self.hook_report_sequences.insert(source.to_string(), seq);
    true
}
```

## Tests Added or Updated

In `src/terminal/state.rs` `#[cfg(test)] mod tests`:

1. `report_lower_state_with_explicit_seq_lowers_effective_state` — registers a
   `herdr:pi` session, reports `Working` (seq 2) then `Idle` (seq 3), and asserts
   `terminal.state == Idle`. Guards the *raising already worked / lowering
   already worked with seq* contract and pins the expected end state.
2. `report_lower_state_without_seq_lowers_effective_state` — same setup but the
   second report omits `--seq` (the no-seq forklift/kimi push pattern). Asserts
   `terminal.state == Idle`. This is the direct regression guard for issue #9:
   before the fix the unsequenced idle report was dropped and `state` stayed
   `Working`.

Both mirror the existing `startup_session_claim_activates_full_lifecycle_integrations`
setup (session-anchored `herdr:pi`) and assert on the effective `terminal.state`
that `agent_status` is derived from via `pane_agent_status`.

## Local Verification

```
export ZIG=/workspace/herdr/.cache/zig/zig-x86_64-linux-0.15.2/zig

cargo test --bin herdr terminal::state::
  -> test result: ok. 116 passed; 0 failed  (includes both new tests)

cargo test --bin herdr -- report_agent report-agent handle_pane_report_agent \
  hook_authority agent_status
  -> test result: ok. 31 passed; 0 failed
```

The two new tests fail before the fix (state stuck at `Working`) and pass after
(file compiled with the vendored libghostty-vt via the exported `ZIG`).

## Deviations from Assessment

The assessment's `Suspected Code Paths`, `Root Cause Hypothesis`, `Reproduction`,
and `Proposed Remediation` were all `[NEEDS CLARIFICATION]`. The candidate lead
named in the task (`set_agent_reported_state`, and a `max`/`Ord` threshold over
`AgentState`) was investigated:

- There is **no** `Ord`/`PartialOrd`/`max` on `AgentState`, and
  `recompute_effective_state` applies the hook authority state directly via
  `.map(|authority| authority.state)` — it never drops a lower value. So the
  threshold hypothesis did not hold; the bug was upstream of arbitration, in the
  report-acceptance gate.
- `set_agent_reported_state` does not exist in this codebase; the report path is
  `HookStateReported` → `set_hook_authority_with_session_ref` →
  `set_hook_authority_at` → `accept_hook_report`.

The root cause is the `seq = None` branch of `accept_hook_report`, confirmed by a
failing-then-passing reproduction rather than by the upstream reporter's
"even with an explicit high --seq" claim (in this codebase explicit-high-seq
lowering already worked; the break is specifically the *unsequenced* push).

## Follow-ups

- **Hook authority vs screen state**: this fix makes the reported (hook)
  authority win for the reported foreground agent, which is the desired
  contract. The headless `FULL_LIFECYCLE_HOOK_STALE_THRESHOLD` staleness bound
  still lets screen detection take over once the hook goes quiet, so a
  never-updated hook will not freeze `agent_status` indefinitely — unchanged.
- **Broader unsequenced acceptance**: `accept_hook_report` is also called by
  `clear_hook_authority` and `release_agent`. A no-seq clear/release is now
  accepted even after a sequence was recorded; this is the expected "clear now"
  semantics and no existing test regressed. Confirm this is acceptable for the
  forklift lifecycle hooks before relying on seq-gated clears elsewhere.
- **End-to-end**: an integration/cli test driving
  `herdr pane report-agent-session` + `report-agent working` + `report-agent idle`
  through the real daemon (and asserting `agent list` shows `idle`) would lock
  this in at the API surface; the unit tests here cover the arbitration logic
  directly.
