# Tasks: agent start nice level

**Input**: Design documents from `specs/001-agent-start-nice/`
**Prerequisites**: `plan.md`, `spec.md`, `research.md`, `data-model.md`,
`contracts/agent-start-nice.md`, `quickstart.md`

**Tests**: Included. Every behavior change is driven by a test that failed
first. The test tasks below are mandatory and precede their implementation
tasks.

**Organization**: Tasks are grouped by user story so each story is an
independently testable increment. Behavior ids refer to
`specs/001-agent-start-nice/tdd/test-list.md`.

## Phase 1: Setup

- [x] T001 Create the feature directory layout and pin the feature in `.specify/feature.json` (`specs/001-agent-start-nice/`, `.specify/feature.json`)
- [x] T002 Write the stack profile with proven commands (`.specify/memory/tdd-profile.md`)

## Phase 2: Foundational

- [x] T003 [behavior: U14] Prove the platform priority prefix on a Unix platform (`src/platform/mod.rs`, `src/platform/macos.rs`, `src/platform/linux.rs`)
- [x] T004 [behavior: U15] Prove the platform without a Unix scheduling priority produces no prefix (`src/platform/windows.rs`, `src/platform/fallback.rs`)
- [x] T005 [behavior: U10] Prove `[agent.start] nice = 19` is parsed (`src/config/model.rs`)
- [x] T006 [behavior: U11] Prove a configuration without an `[agent.start]` section resolves to zero (`src/config/model.rs`)
- [x] T007 [behavior: U12] Prove an out-of-range configured value is reported and falls back to zero (`src/config/model.rs`, `src/config.rs`)
- [x] T008 [behavior: U13] Prove the `agent` section is a known top-level section (`src/config/io.rs`)

## Phase 3: User Story 1 - Launch an agent at a chosen scheduling priority (P1)

**Goal**: An operator can request a scheduling priority for a fresh agent
launch, and the priority covers the agent and every child it forks.

**Independent test**: Start an agent with a requested priority into a fresh
pane and read the pane's foreground process priority.

- [x] T009 [behavior: U1] Test that `--nice <N>` inside the supported range is sent to the server as the requested priority (`src/cli/agent.rs`)
- [x] T010 [behavior: U2] Test that `--nice` outside the supported range is rejected before any request is sent (`src/cli/agent.rs`)
- [x] T011 [behavior: U3] Test that omitting `--nice` sends no priority field (`src/cli/agent.rs`)
- [x] T012 [behavior: U4] Test that agent arguments after `--` are forwarded unchanged alongside a requested priority (`src/cli/agent.rs`)
- [x] T013 [behavior: U5] Test that the requested priority is used when the request carries one (`src/app/agents.rs`)
- [x] T014 [behavior: U7] Test that the launch line is unchanged when the effective priority is zero (`src/app/agents.rs`)
- [x] T015 [behavior: U8] Test that the launch line gains a priority prefix when the effective priority is non-zero (`src/app/agents.rs`)
- [x] T016 [behavior: U9] Test that an out-of-range requested priority is rejected as an invalid agent argument (`src/app/agents.rs`)
- [x] T017 Carry the optional priority on the agent-start request (`src/api/schema/agents.rs`, `src/api/schema.rs`)
- [x] T018 Build the launch line with the resolved priority prefix (`src/app/agents.rs`)
- [x] T019 Document the priority argument in the command help and the command-spec tree (`src/cli/agent.rs`, `src/cli/spec.rs`)

## Phase 4: User Story 2 - Set a machine-wide default priority (P2)

**Goal**: An operator sets one default in the configuration and every fresh
launch inherits it, while a per-launch request still wins.

**Independent test**: Configure a default, launch without a flag, observe the
default; then launch with a flag and observe the flag.

- [x] T020 [behavior: U6] Test that the configured default is used when the request carries none (`src/app/agents.rs`, `src/app/state.rs`, `src/app/mod.rs`)
- [x] T021 Hold the configured default in the application state and refresh it on configuration load and live reload (`src/app/state.rs`, `src/app/mod.rs`)
- [x] T022 Register the `agent` configuration section for live reload (`src/config/io.rs`)
- [x] T023 Document the `[agent.start]` section in the default configuration file (`src/main.rs`)

## Phase 5: User Story 3 - Verify the resulting priority (P3)

**Goal**: A caller can read the priority of a pane's foreground process
instead of assuming the request was honored.

**Independent test**: Launch an agent with a priority, read the pane's process
information, compare.

- [x] T024 [behavior: U16] Test that the reported process information carries the scheduling priority when it is known (`src/api/schema/panes.rs`, `src/app/api/panes.rs`)
- [x] T025 [behavior: U17] Test that the reported scheduling priority is absent when it cannot be determined (`src/api/schema/panes.rs`, `src/app/api/panes.rs`)
- [x] T026 Read the scheduling priority of a process per platform (`src/platform/macos.rs`, `src/platform/linux.rs`, `src/platform/windows.rs`, `src/platform/fallback.rs`)

## Phase 6: Acceptance

- [x] T027 [behavior: A1] Test that an agent started with a requested priority of 19 runs at 19 and its children inherit 19 (`tests/agent_priority.rs`)
- [x] T028 [behavior: A2] Test that a launch with no requested priority and no configured default is byte-identical to the pre-feature launch (`tests/agent_priority.rs`)
- [x] T029 [behavior: A3] Test that the configured default applies when no priority is requested and is overridden by an explicit value (`tests/agent_priority.rs`)
- [x] T030 [behavior: A4] Test that an out-of-range requested priority is rejected with a non-zero exit, a naming message, and no agent started (`tests/agent_priority.rs`)
- [x] T031 [behavior: A5] Test that a priority-requested agent still reaches a ready state and still accepts a delivered prompt (`tests/agent_priority.rs`)
- [x] T032 [behavior: A6] Test that every supported agent kind accepts the priority, including trailing `--` arguments (`tests/agent_priority.rs`)
- [x] T033 [behavior: A7] Test that the reported priority matches the priority the operating system applied (`tests/agent_priority.rs`)

## Phase 7: Polish & cross-cutting

- [x] T034 Register the new configuration key in the published configuration reference (`docs/next/website/src/data/config-reference.json`)
- [x] T035 Document the priority argument and the configuration key in the website documentation, English and Japanese (`docs/next/website/src/content/docs/cli-reference.mdx`, `docs/next/website/src/content/docs/configuration.mdx`, `docs/next/website/src/content/docs/ja/`)
- [x] T036 Document the optional request and response fields in the socket API documentation (`docs/next/website/src/content/docs/socket-api.mdx`, `docs/next/website/src/content/docs/ja/`)
- [x] T037 Run the full validation gate (`just check`)

## Phase 8: Review follow-ups

Raised by the review of the first push.

- [x] T038 [behavior: U25] Reject the launch before it is submitted when the priority program is not on the server's search path, naming the program (`src/app/agents.rs`)
- [x] T039 [behavior: A9] Test that a launch on a host without the priority program is rejected with an error naming the program (`tests/agent_priority.rs`)
- [x] T040 [behavior: A10] Test that a launch without a priority still succeeds on a host without the priority program (`tests/agent_priority.rs`)
- [x] T041 [behavior: A11] Test that an out-of-range requested priority is rejected over the raw socket (`tests/api_ping.rs`)
- [x] T042 Derive the expected socket launch line from the priority the host's shell runs at, instead of asserting a fixed value (`tests/api_ping.rs`)

## Dependencies

- T001, T002 must complete before any behavior task.
- T003-T008 (Phase 2) are the shared foundation and block all user stories.
- Phase 3 (US1) is the MVP; it depends on Phase 2 only.
- Phase 4 (US2) depends on Phase 3 (a default only means something once a
  request can be made).
- Phase 5 (US3) depends on Phase 3.
- Phase 6 depends on Phases 3, 4 and 5.
- Phase 7 depends on everything above.

## Parallel execution examples

- T003, T004, T005, T006, T007, T008 can run in parallel with each other
  (different files, no shared unfinished dependency).
- T009-T012 are in one file and are sequential.
- T013-T016 are in one file and are sequential.
- T017-T019 are in distinct files and can run in parallel once T013-T016
  are green.
- T020-T023 touch distinct files and can run in parallel.
- T024-T026 are in distinct files and can run in parallel.
- T027-T033 are in one file and are sequential.
- T034, T035, T036 are in distinct files and can run in parallel.

## Implementation strategy

1. **MVP (US1)**: Phase 2 plus Phase 3 deliver the feature's core value — a
   caller can request a scheduling priority for a fresh agent launch.
2. **US2**: adds the machine-wide default, which is what makes the feature
   usable for fleet dispatchers.
3. **US3**: adds the verification surface, which lets callers confirm what
   happened instead of assuming it.
4. **Polish**: publishes the configuration reference and the documentation, and
   runs the full gate.

## Deviations from the plan

- **T027-T033 live in `tests/agent_priority.rs`, not `tests/cli/agents.rs`.**
  The `tests/cli/` suite is gated `#![cfg(all(unix, not(target_os = "macos")))]`,
  so a test placed there never runs on macOS and cannot be reproduced on a
  developer machine. The new file is gated `#![cfg(unix)]` and drives the real
  `herdr` CLI against a real server, so the acceptance evidence exists on every
  Unix CI job and on a macOS workstation.
- **An extra acceptance test was added** for the raw socket path:
  `tests/api_ping.rs::agent_start_carries_the_scheduling_priority_over_the_socket`.
  The CLI is one caller of the socket API; a second caller proves the optional
  field travels over the wire.
- **The acceptance tests poll for the expected state.** `agent.start` answers
  as soon as the launch command is submitted to the pane's shell, so the agent
  process may not have been exec'd yet when the answer arrives. The tests wait
  for the process they expect instead of assuming it is already there.
