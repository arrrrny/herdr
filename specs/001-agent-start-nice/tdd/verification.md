---
feature: 001-agent-start-nice
profile: .specify/memory/tdd-profile.md
audited_at: 2026-10-21
baseline_commit: a69eee85
verdict: pass
---

# Verification: agent start nice level

## Verdict

**Pass.** Every behavior in `tdd/test-list.md` has a named test, every test
passes against the committed source, and no deliberate mutant of the
feature's behavior survived.

## What was audited

The feature has six load-bearing decisions. Each one was broken on purpose
and the pinning test was observed to fail, which is the evidence that the
test actually holds the behavior down.

| # | Mutant | Where | Killed by | Result |
| - | ------ | ----- | --------- | ------ |
| 1 | The resolution ignores the requested priority and always uses the configured default | `src/agent_priority.rs` | `tests/agent_priority.rs::agent_start_honors_the_configured_default_and_lets_the_flag_override_it` | killed |
| 2 | The launch line drops the priority prefix | `src/app/agents.rs` | `tests/agent_priority.rs::agent_start_applies_a_requested_priority` | killed |
| 3 | The range check accepts every value | `src/agent_priority.rs` | `src/cli/agent/bindings_tests.rs::agent_start_rejects_an_out_of_range_priority_before_any_request` and `tests/agent_priority.rs::agent_start_rejects_an_out_of_range_priority` | killed |
| 4 | The reported process priority is dropped | `src/app/api/panes.rs` | `tests/agent_priority.rs::pane_process_info_reports_the_applied_priority` | killed |
| 5 | The `agent` section is not a known top-level configuration section | `src/config/io.rs` | `src/config/io.rs::load_live_config_parses_the_agent_start_section` | killed |
| 6 | The CLI drops the requested priority while parsing | `src/cli/agent/bindings.rs` | `src/cli/agent/bindings_tests.rs::agent_start_parses_a_requested_priority` | killed |

**Surviving mutants: 0.**

The audit script is `/tmp/agent_start_nice_mutants.sh`. It applies one
mutant, runs the pinning test with `cargo nextest run --locked -E
'test(<name>)'`, restores the source, and fails the audit when the test
still passes. The raw output is reproduced in `tdd/cycle-log.md`.

## Suite results

```
cargo nextest run --locked -E 'binary(agent_priority)'
  Summary [  14.873s] 12 tests run: 12 passed, 0 skipped

cargo nextest run --locked -E 'test(agent_start_carries_the_scheduling_priority_over_the_socket)'
  Summary [   4.019s] 1 test run: 1 passed, 4077 skipped
```

Both runs were taken against a build produced from the committed source after
the stale-build incident described in `tdd/cycle-log.md` (cycle 7) was
resolved.

## Coverage of the success criteria

| Criterion | Covered by |
| --------- | ---------- |
| SC-001 a requested priority applies to the agent and its children | `agent_start_applies_a_requested_priority` |
| SC-002 no priority leaves the launch line unchanged | `agent_start_without_a_priority_keeps_the_launch_line_unchanged` |
| SC-003 the configured default applies and the flag overrides it | `agent_start_honors_the_configured_default_and_lets_the_flag_override_it` |
| SC-004 an out-of-range priority is rejected before any agent starts | `agent_start_rejects_an_out_of_range_priority` |
| SC-005 detection still works for a priority-requested agent | `agent_start_with_a_priority_still_detects_the_agent` |
| SC-006 every agent kind accepts the priority, including trailing `--` arguments | `agent_start_passes_trailing_agent_arguments_with_a_priority` |
| SC-007 the reported priority matches the applied priority | `pane_process_info_reports_the_applied_priority` |

The same fields also travel over the raw socket, which
`tests/api_ping.rs::agent_start_carries_the_scheduling_priority_over_the_socket`
covers separately.

## Maintenance checks

```
python3 scripts/config_reference_check.py     # green
python3 scripts/docs_translation_parity.py    # green
```

## Caveats

- `tests/cli/` is gated `#![cfg(all(unix, not(target_os = "macos")))]` and
  does not run on this macOS machine. The acceptance evidence therefore comes
  from `tests/agent_priority.rs`, which is gated `#![cfg(unix)]` and runs on
  every Unix CI job.
- `tests/live_handoff.rs::live_handoff_preserves_keyboard_protocol_for_client_input`
  fails on this machine with `failed to openpty: Device not configured`. It
  is present in the baseline and is unrelated to this feature.
- `tests/remote_attach.rs::ssh_check_message_is_visible_while_authentication_waits`
  fails on this machine with `timed out waiting for fake ssh`. It was
  confirmed to fail on the baseline as well, with this feature's changes
  stashed, so it is not caused by this feature.
- No mutation or coverage tooling is installed on this machine, so the audit
  above uses hand-applied mutants instead of an automated mutation run.
- Windows is not covered by the acceptance suite. The Windows platform keeps
  its current behavior: no priority prefix, and no priority reported, per
  `spec.md` "Out of Scope".
