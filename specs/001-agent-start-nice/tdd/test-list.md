---
feature: 001-agent-start-nice
loop: outside-in
profile: .specify/memory/tdd-profile.md
spec_criteria: 9
planned_at: a69eee85
updated_at: 90df45cc
suite_baseline: green
---

# Test List: agent start nice level

## Outer loop: acceptance behaviors

One per success criterion in `spec.md`. Each stays red until the feature
works end to end through its real entry point.

The acceptance suite lives in `tests/agent_priority.rs` rather than in
`tests/cli/agents.rs`. The `tests/cli/` suite is gated
`#![cfg(all(unix, not(target_os = "macos")))]`, so a test placed there never
runs on macOS and cannot be reproduced on a developer machine. The new file is
gated `#![cfg(unix)]` and drives the real `herdr` CLI against a real server, so
the same acceptance evidence exists on every Unix CI job.

| id  | behavior                                                                                          | traces | kind    | state | test                                                    |
| --- | ------------------------------------------------------------------------------------------------- | ------ | ------- | ----- | ------------------------------------------------------- |
| A1  | An agent started with a requested priority of 19 runs at 19, and every child it forks inherits 19   | SC-001 | example | DONE  | `tests/agent_priority.rs::agent_start_applies_a_requested_priority` |
| A2  | A launch with no requested priority and no configured default is byte-identical to the pre-feature launch | SC-002 | example | DONE  | `tests/agent_priority.rs::agent_start_without_a_priority_keeps_the_launch_line_unchanged` |
| A3  | A configured default of 19 applies when no priority is requested, and an explicit value overrides it | SC-003 | example | DONE  | `tests/agent_priority.rs::agent_start_honors_the_configured_default_and_lets_the_flag_override_it` |
| A4  | An out-of-range requested priority is rejected with a non-zero exit, a naming message, and no agent started | SC-004 | example | DONE  | `tests/agent_priority.rs::agent_start_rejects_an_out_of_range_priority` |
| A5  | A priority-requested agent still reaches a ready state and still accepts a delivered prompt           | SC-005 | example | DONE  | `tests/agent_priority.rs::agent_start_with_a_priority_still_detects_the_agent` |
| A6  | Every supported agent kind accepts the priority, including kinds invoked with trailing `--` arguments | SC-006 | example | DONE  | `tests/agent_priority.rs::agent_start_passes_trailing_agent_arguments_with_a_priority` |
| A7  | The reported priority matches the priority the operating system actually applied                   | SC-007 | example | DONE  | `tests/agent_priority.rs::pane_process_info_reports_the_applied_priority` |
| A8  | An agent requested at priority 19 runs at 19 even when the pane's shell runs at another priority | SC-008 | example | DONE  | `tests/agent_priority.rs::the_requested_priority_is_reached_from_a_shell_at_another_priority` |
| A9  | A launch on a host without the priority program is rejected with an error naming the program, and no agent is started | SC-009 | example | DONE  | `tests/agent_priority.rs::agent_start_reports_a_missing_priority_program` |
| A10 | A launch that requests no priority still succeeds on a host without the priority program | SC-009 | example | DONE  | `tests/agent_priority.rs::a_launch_without_a_priority_does_not_need_the_priority_program` |
| A11 | An out-of-range requested priority is rejected over the raw socket, and no agent is started | SC-004 | example | DONE  | `tests/api_ping.rs::agent_start_rejects_an_out_of_range_priority_over_the_socket` |

The same request and response fields also travel over the raw socket, which
is covered separately by
`tests/api_ping.rs::agent_start_carries_the_scheduling_priority_over_the_socket`.

## Inner loop: unit behaviors

Grouped by the component from `plan.md` that owns them. Each line names one
observable result.

### `src/agent_priority.rs`

| id  | behavior                                                                       | traces      | kind     | state | test                                                |
| --- | ------------------------------------------------------------------------------ | ----------- | -------- | ----- | --------------------------------------------------- |
| U18 | The supported range matches the operating system's own range                    | FR-002      | example  | DONE  | `src/agent_priority.rs::the_supported_range_matches_the_operating_system_range` |
| U19 | An out-of-range value is rejected, names the value and the range, and is never clamped | FR-002 | example | DONE  | `src/agent_priority.rs::an_out_of_range_value_is_rejected_and_never_clamped` |
| U20 | A value inside the range is accepted, including both boundaries               | FR-001      | example  | DONE  | `src/agent_priority.rs::a_value_inside_the_range_is_accepted` |
| U21 | A non-numeric value is rejected                                               | FR-002      | example  | DONE  | `src/agent_priority.rs::a_non_numeric_value_is_rejected` |
| U22 | An explicit zero overrides the configured default                             | FR-004      | example  | DONE  | `src/agent_priority.rs::an_explicit_zero_overrides_the_configured_default` |
| U24 | A program that is not on the search path is reported as missing                | FR-018      | example  | DONE  | `src/agent_priority.rs::a_program_that_is_not_on_the_search_path_is_reported_as_missing` |

### `src/cli/agent/bindings.rs`

| id  | behavior                                                                        | traces      | kind     | state | test                                              |
| --- | ------------------------------------------------------------------------------- | ----------- | -------- | ----- | ------------------------------------------------- |
| U1  | `--nice <N>` inside the supported range is sent to the server as the requested priority | SC-001, FR-001 | example | DONE  | `src/cli/agent/bindings_tests.rs::agent_start_parses_a_requested_priority` |
| U2  | `--nice` outside the supported range is rejected before any request is sent, naming the value and the range | SC-004, FR-002 | example | DONE  | `src/cli/agent/bindings_tests.rs::agent_start_rejects_an_out_of_range_priority_before_any_request` |
| U3  | Omitting `--nice` sends no priority field at all                                 | SC-002, FR-003 | example | DONE  | `src/cli/agent/bindings_tests.rs::agent_start_sends_no_priority_when_the_flag_is_absent` |
| U4  | Agent arguments after `--` are forwarded unchanged alongside a requested priority | SC-006, FR-016 | example | DONE  | `src/cli/agent/bindings_tests.rs::agent_start_keeps_trailing_agent_arguments_with_a_priority` |

### `src/app/agents.rs`

| id  | behavior                                                                  | traces             | kind     | state | test                                            |
| --- | ------------------------------------------------------------------------- | ------------------ | -------- | ----- | ----------------------------------------------- |
| U5  | The requested priority is used when the request carries one                | FR-006, FR-008    | example  | DONE  | `src/app/agents.rs::the_launch_line_puts_the_priority_before_the_agent_executable` |
| U6  | The configured default is used when the request carries none               | FR-005             | example  | DONE  | `src/agent_priority.rs::an_absent_request_falls_back_to_the_configured_default` |
| U7  | The launch line is unchanged when the effective priority is zero           | SC-002, FR-004     | example  | DONE  | `src/app/agents.rs::the_launch_line_is_unchanged_without_a_priority` |
| U8  | The launch line gains a priority prefix when the effective priority is non-zero | FR-008        | example  | DONE  | `src/app/agents.rs::the_launch_line_puts_the_priority_before_the_agent_executable` |
| U9  | An out-of-range requested priority is rejected as an invalid agent argument | FR-002           | example  | DONE  | `src/app/agents.rs::an_out_of_range_requested_priority_is_rejected_and_not_clamped` |
| U25 | A launch naming a priority program the server cannot find is rejected before the launch line is submitted, and the error names the program | FR-018, FR-019 | example | DONE  | `tests/agent_priority.rs::agent_start_reports_a_missing_priority_program` |

### `src/config/`

| id  | behavior                                                                       | traces      | kind     | state | test                                                   |
| --- | ------------------------------------------------------------------------------ | ----------- | -------- | ----- | ------------------------------------------------------ |
| U10 | `[agent.start] nice = 19` is parsed into the configuration                    | FR-005      | example  | DONE  | `src/config/io.rs::load_live_config_parses_the_agent_start_section` |
| U11 | A configuration without an `[agent.start]` section resolves to a default of zero | FR-003    | example  | DONE  | `src/config/io.rs::load_live_config_defaults_the_agent_start_priority_to_zero` |
| U12 | An out-of-range configured value is reported as a diagnostic and falls back to zero | FR-007 | example  | DONE  | `src/config/io.rs::load_live_config_reports_an_out_of_range_agent_start_priority` |
| U13 | The `agent` section is a known top-level configuration section                | FR-005      | example  | DONE  | `src/config/io.rs::the_agent_section_is_a_known_top_level_section` |

### `src/platform/`

| id  | behavior                                                            | traces      | kind     | state | test                                                     |
| --- | ------------------------------------------------------------------- | ----------- | -------- | ----- | -------------------------------------------------------- |
| U14 | A Unix platform produces a priority prefix for a non-zero priority  | FR-001      | example  | DONE  | `src/platform/macos.rs::the_priority_prefix_precedes_the_agent_executable` |
| U15 | A platform without a Unix scheduling priority produces no prefix   | FR-015      | example  | DONE  | `src/platform/macos.rs::a_zero_priority_produces_no_prefix` |
| U23 | The operating system's own priority is readable for a live process   | FR-012      | example  | DONE  | `src/platform/macos.rs::the_current_process_reports_its_priority` |
| U26 | The platform names the priority program its launch line uses        | FR-018      | example  | DONE  | `src/platform/macos.rs::the_platform_names_the_priority_program_it_launches_with` |

### `src/api/schema/panes.rs`

| id  | behavior                                                                     | traces             | kind     | state | test                                            |
| --- | ---------------------------------------------------------------------------- | ------------------ | -------- | ----- | ----------------------------------------------- |
| U16 | The reported process information carries the scheduling priority when it is known | FR-012, SC-007 | example  | DONE  | `src/api/schema/tests.rs::pane_process_info_carries_the_scheduling_priority` |
| U17 | The reported scheduling priority is absent when it cannot be determined      | FR-013             | example  | DONE  | `src/api/schema/tests.rs::pane_process_info_omits_an_unknown_scheduling_priority` |

## Invariants and edge cases still to place

Behaviors that belong to the feature but do not yet have a home component. Each
must become a numbered line above before the feature is done, or be dropped
with a reason.

- A negative requested priority is accepted by the range check and refused by
  the operating system, which is reported to the operator rather than clamped.
  (Covered by the U19/U20 range tests plus the platform's own refusal; the
  refusal path belongs to the operating system, not to Herdr.)
- A configured default of exactly `-20` and exactly `19` are both accepted.
  (Boundary values; sampled by the U12 range test and the U20 boundary test.)

## Out of scope

- Re-prioritizing an already-running agent: the feature covers launch time
  only, per `spec.md` "Out of Scope".
- Applying the configured default to agents resumed from a restored session:
  a different launch path, explicitly excluded by `spec.md`.
- Windows priority classes: the platform keeps its current behavior, per
  `spec.md` "Out of Scope".
- I/O priority, CPU affinity, cgroup placement: never part of this feature.

## Verification commands

Copied verbatim from `.specify/memory/tdd-profile.md` at planning time, so this
file is readable on its own:

- Single test: `cargo nextest run --locked -E 'test(<name>)'`
- Whole file: `cargo nextest run --locked -E 'binary(agent_priority)'`
- Full suite: `cargo nextest run --locked --status-level fail --final-status-level fail --failure-output final --success-output never`
- Mutation (changed files): none available; the audit uses deliberate mutants.
