# Implementation Plan: agent start nice level

**Branch**: `issue/77-agent-start-nice` | **Date**: 2026-10-21 | **Spec**: [spec.md](./spec.md)

**Input**: Feature specification from `specs/001-agent-start-nice/spec.md`

## Summary

Add an optional CPU scheduling priority to `herdr agent start`. The operator
passes `--nice <N>` on the command line, or an operator sets a machine-wide
default under `[agent.start] nice` in the Herdr configuration file. The
requested priority becomes a prefix on the launch line that is typed into the
target pane, so the agent process — and every child it forks — inherits it
before the first fork. The pane's process information additionally reports the
scheduling priority of each foreground process, so a caller can verify the
result instead of assuming it.

## Technical Context

**Language/Version**: Rust (edition pinned by `Cargo.toml`; toolchain per
`rust-toolchain.toml`)

**Primary Dependencies**: `libc` (already a dependency, used for the existing
per-OS process accessors), `serde` / `toml` (already used by the config
layer), `clap` (already used for the command-spec tree that drives help text
and completions)

**Storage**: The Herdr configuration file (`~/.config/herdr/config.toml`,
overridable via `HERDR_CONFIG_PATH`)

**Testing**: `cargo nextest` via the `just test` / `just check` recipes;
integration tests under `tests/`, unit tests in `#[cfg(test)] mod tests` next
to the code

**Target Platform**: Unix-like (Linux, macOS) for the priority behavior;
Windows keeps its current behavior unchanged

**Project Type**: Desktop terminal multiplexer with a server/TUI split; the
feature touches the CLI layer, the server runtime, the config layer, and the
platform layer

**Performance Goals**: No measurable impact on the render or layout paths. The
launch path gains one integer comparison and, at most, three extra tokens on
a command line that is typed once.

**Constraints**: The launch path must not add filesystem I/O or a
configuration re-read; the configured default is resolved from state that is
already loaded.

**Scale/Scope**: One CLI flag, one config key, one optional request field, one
optional response field, and one per-OS accessor per platform.

## Constitution Check

*GATE: Must pass before Phase 0 research. Re-check after Phase 1 design.*

The project constitution is `AGENTS.md` in the repository root. Applicable
principles and how this plan satisfies them:

| Principle | How the plan satisfies it |
| --- | --- |
| Platform code is isolated | The priority prefix is produced by a platform-level function, one implementation per operating system, re-exported through `src/platform/mod.rs`. No `cfg!(windows)` check appears in `src/app/`. |
| State is separated from runtime | The configured default lives in `AppState` as data; the launch path reads it and does not mutate it. |
| No god objects | The change is additive: one config struct, one platform function, one request field. No module gains a second responsibility. |
| Stable client endpoint contract | The new request field is optional and defaults to absent; the new response field is optional. No published codec changes, so no protocol version bump. |
| UI patterns should be reused | No new UI surface. The feature is a CLI flag plus a config key, following the existing `--timeout` / `[session]` precedents. |
| Screen detection is evidence-based | The feature does not change detection. A test pins that a priority-requested agent is still detected and still accepts a prompt. |

**Post-design re-check**: no violation found. The platform boundary holds
(R2), the runtime/client boundary holds (R3), and no published codec changes.

## Project Structure

### Documentation (this feature)

```text
specs/001-agent-start-nice/
├── spec.md                 # the specification
├── plan.md                 # this file
├── research.md             # Phase 0 output
├── data-model.md           # Phase 1 output
├── quickstart.md           # Phase 1 output
├── contracts/
│   └── agent-start-nice.md # Phase 1 output: CLI, config, socket contracts
├── checklists/
│   └── requirements.md     # spec quality checklist
├── tasks.md                # Phase 2 output
└── tdd/
    ├── test-list.md        # the behavior plan
    ├── cycle-log.md        # append-only red/green evidence
    └── verification.md     # the audit report
```

### Source Code (repository root)

```text
src/
├── cli/
│   ├── agent.rs            # hand-rolled `agent start` parser: --nice parsing
│   └── spec.rs             # clap command tree: --nice arg for help/completions
├── app/
│   ├── agents.rs           # start_agent(): resolves the effective priority,
│   │                       # builds the launch line, validates the range
│   ├── state.rs            # AppState field for the configured default
│   ├── mod.rs              # assigns + live-reloads the configured default
│   └── api/
│       ├── agents.rs       # handle_agent_start(): unchanged, new field flows
│       └── panes.rs        # handle_pane_process_info(): reports the priority
├── api/schema/
│   ├── agents.rs           # AgentStartParams.nice (optional)
│   └── panes.rs            # PaneProcessInfoProcess.nice (optional)
├── config/
│   ├── model.rs            # AgentConfig / AgentStartConfig structs
│   └── io.rs               # known top-level key + live-reload section
├── platform/
│   ├── mod.rs              # shared trait/contract + re-exports
│   ├── macos.rs            # priority prefix + priority reader
│   ├── linux.rs            # priority prefix + priority reader
│   ├── windows.rs          # no prefix, no priority reported
│   └── fallback.rs         # no prefix, no priority reported
└── main.rs                 # DEFAULT_CONFIG: documented [agent.start] section

tests/
├── cli/agents.rs           # CLI-level acceptance for --nice
└── api_ping.rs             # socket-level acceptance for the request field
```

**Structure Decision**: the existing single-crate layout is kept. The feature
adds no new module: it extends the CLI parser, the agent-start request/response
schema, the config model, the platform boundary, and the process-information
producer. The platform boundary is the only place that knows which operating
systems have a Unix scheduling priority.

## Complexity Tracking

> **Fill ONLY if Constitution Check has violations that must be justified**

| Violation | Why Needed | Simpler Alternative Rejected Because |
|-----------|------------|-------------------------------------|
| _none_ | — | — |
