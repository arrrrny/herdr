# Hardening: wait primitives must confirm idle across two samples

- Slug: hardening-wait-primitives-confirm-idle-across
- Fixed: 2026-08-23
- Assessment: ./assessment.md
- Issues: arrrrny/herdr#15 (and arrrrny/herdr#14)
- Status: applied

## Summary

A single idle sample races a just-typed command's startup: the shell briefly looks
idle in the instant between a keystroke being sent and the spawned foreground
process appearing in the pty's foreground process group. Any wait primitive that
declares "idle" must therefore require idle to be confirmed across two samples
spaced ~1s apart before returning.

This hardening is applied to the new pty-based `herdr pane wait --idle` primitive
(see the #14 fix report): the poller samples `PaneProcessInfo.busy` once per second
and only returns once `idle_streak` (consecutive idle samples) reaches 2. A single
busy sample resets the streak, so a momentary idle dip during command startup does
not early-return.

## Changes

| File | Change | Notes |
| --- | --- | --- |
| `src/cli/pane.rs` | `pane_wait` polls every 1s; `idle_streak_after_sample` tracks consecutive idle samples; returns only when `idle_streak >= 2` | The two-sample rule lives entirely in the pty-based primitive |
| `src/cli/pane.rs` | `idle_streak_after_sample` helper + tests | Busy sample resets to 0; two consecutive idles reach 2 |
| `src/cli/pane.rs` | `parse_pane_wait_args` tests cover parse/required-flag behavior | Mirrors `wait-output` parser tests |

No change to the busy computation in `src/app/api/panes.rs` (added for #14) or to
the `PaneProcessInfo` schema; those are the data source the poller samples.

## Tests Added

In `src/cli/pane.rs` `#[cfg(test)] mod tests`:

1. `idle_streak_after_sample_resets_on_busy` — a busy sample resets the streak to 0.
2. `idle_streak_after_sample_reaches_two_on_consecutive_idle` — two consecutive
   idle samples reach a streak of 2 (the return threshold).
3. `idle_streak_after_sample_busy_between_idles_resets` — a busy sample between two
   idle samples resets, proving idle is only confirmed after two *consecutive*
   idle samples (the race guard).
4. `parse_pane_wait_args_parses_pane_id_idle_and_timeout`,
   `parse_pane_wait_args_accepts_equals_timeout`,
   `parse_pane_wait_args_requires_idle`,
   `parse_pane_wait_args_requires_pane_id` — parser coverage for the `wait` command.

## Local Verification

```
export ZIG=$(ls -d /workspace/herdr/.cache/zig/zig-x86_64-linux-* | head -1)

cargo test --bin herdr cli::pane
  -> test result: ok. 49 passed; 0 failed
  (includes the three idle_streak_after_sample_* tests above)
```

`cargo fmt -- --check` is clean for the touched file.

## Deviations from Assessment

The assessment for this bug was a stub. The implementation is driven by the precise
task change list. One explicit design decision is recorded here:

- **`agent wait` is NOT subject to the two-sample rule.** `agent wait` is a
  server-side, event/status-driven wait: it observes agent status transitions
  (Working → Done/Idle) delivered through the event hub, not a pty foreground-
  process poll. The two-sample "idle confirmed across ~1s" rule is specifically a
  guard against the pty foreground-process-group race (a command typed into the
  shell momentarily showing idle before its child appears). That race does not
  exist in the agent-wait path, so the rule does not map onto it and is not
  applied there. Only `pane wait --idle` (the pty-based primitive) enforces the
  two-sample confirmation.

## Follow-ups

- An end-to-end `pane wait --idle` test against a live daemon (type a command,
  let it finish, assert return 0 after two idle samples; and a `--timeout` expiry
  asserting return 1) would validate the real 1s cadence and the two-sample
  threshold. Unit tests here cover the streak logic deterministically.
