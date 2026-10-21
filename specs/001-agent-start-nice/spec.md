# Feature Specification: agent start nice level

**Feature Branch**: `issue/77-agent-start-nice`

**Created**: 2026-10-21

**Status**: Draft

**Input**: User description: "herdr agent start should launch the agent at a configurable Unix nice level so shared-host services are not starved by agent work. Source: https://github.com/arrrrny/herdr/issues/77"

## User Scenarios & Testing *(mandatory)*

### User Story 1 - Launch an agent at a chosen scheduling priority (Priority: P1)

An operator who runs coding agents on a shared machine wants the agent process,
and every child it later forks, to run at a lower CPU priority than the host's
own services. The operator asks for that priority once, at launch time, and
expects the whole agent process tree to inherit it.

**Why this priority**: This is the core capability. Without it, a single
un-capped agent workload can starve co-located services for minutes. Everything
else in this specification is a convenience or a verification surface for this
one behavior.

**Independent Test**: Launch an agent with a requested priority into a fresh
pane, then read that pane's foreground process priority and confirm it matches
the requested value.

**Acceptance Scenarios**:

1. **Given** a fresh pane with an idle shell, **When** the operator starts an
   agent with a requested priority of 19, **Then** the agent process runs at
   priority 19 and every child it subsequently forks also runs at 19.
2. **Given** a fresh pane with an idle shell, **When** the operator starts an
   agent without a requested priority, **Then** the launch line is exactly the
   one produced before this feature existed (no priority prefix is added).
3. **Given** a requested priority of 0, **When** the operator starts an agent,
   **Then** the launch line is exactly the one produced before this feature
   existed.

---

### User Story 2 - Set a machine-wide default priority (Priority: P2)

An operator who dispatches agents from their own tooling does not want to add
a flag to every call site. They set one value in the machine's Herdr
configuration and every agent they start inherits it, while still being able
to override it for a single launch.

**Why this priority**: It removes the per-call-site change for fleet
dispatchers, which is the dominant launch path on the affected hosts. It
depends on Story 1 and is not useful without it.

**Independent Test**: Set a default priority in the configuration file, start
an agent without a flag, confirm the default applied; then start an agent with
an explicit different value and confirm the flag wins.

**Acceptance Scenarios**:

1. **Given** a configured default priority of 19, **When** the operator starts
   an agent without specifying one, **Then** the agent runs at priority 19.
2. **Given** a configured default priority of 19, **When** the operator starts
   an agent specifying a different priority, **Then** the explicit value wins
   and the configured default is ignored for that launch.
3. **Given** no configured default, **When** the operator starts an agent
   without specifying one, **Then** the launch line is exactly the one
   produced before this feature existed.

---

### User Story 3 - Verify the resulting priority (Priority: P3)

A dispatcher that cannot observe the priority of what it launched has to
assume the request was honored. The operator wants to read the priority of a
pane's foreground process directly, so a request can be verified rather than
assumed.

**Why this priority**: It is an observability surface for the two stories above.
It does not change what is launched, so it can ship after them.

**Independent Test**: Start an agent with a requested priority, then read the
pane's process information and confirm the reported priority matches.

**Acceptance Scenarios**:

1. **Given** a pane running an agent started with a requested priority of 19,
   **When** the operator reads the pane's process information, **Then** the
   agent's reported priority is 19.
2. **Given** a pane running a process whose priority cannot be determined,
   **When** the operator reads the pane's process information, **Then** the
   priority is reported as absent rather than as a wrong number.

---

### Edge Cases

- A requested priority outside the supported range is rejected with a clear
  error, before anything is launched; it is never silently clamped to the
  nearest valid value.
- A requested priority that requires elevated privileges is rejected by the
  operating system when the agent is launched; the operator is told that
  negative values need privileges.
- A configured default priority outside the supported range is reported as an
  invalid configuration value, and the default falls back to "no priority
  prefix" rather than to a clamped number.
- The target operating system has no Unix scheduling-priority concept: the
  launch line is unchanged and no priority is reported.
- The agent is launched with trailing arguments after `--`: the priority prefix
  must not swallow or reorder them.
- The agent is resumed from a restored session rather than started fresh: the
  feature covers fresh launches only.
- A platform where the priority program is not on the search path: the launch
  is rejected before the agent starts, with an error that names the priority
  program, so the operator sees which program was missing instead of a
  readiness timeout. When the pane's own search path differs from the
  server's, the operating system's own error still reaches the pane, and the
  readiness error names the priority program as well.

## Requirements *(mandatory)*

### Functional Requirements

- **FR-001**: The `agent start` command MUST accept an optional priority
  argument that requests a specific CPU scheduling priority for the agent
  process and its descendants.
- **FR-002**: The supported priority range MUST be validated before launch.
  Values outside the range MUST be rejected with a clear error and MUST NOT be
  clamped.
- **FR-003**: When no priority is requested and no default is configured, the
  launch MUST be byte-for-byte identical to the launch produced before this
  feature existed.
- **FR-004**: When the requested priority is zero, the launch MUST be
  byte-for-byte identical to the launch produced before this feature existed.
- **FR-005**: A machine-wide default priority MUST be configurable in the
  Herdr configuration file, so an operator can set it once for every caller.
- **FR-006**: An explicitly requested priority MUST override the configured
  default for that launch.
- **FR-007**: An invalid configured default priority MUST be reported as a
  configuration diagnostic and MUST NOT prevent the Herdr server from starting
  or reloading its configuration.
- **FR-008**: The priority MUST apply to the agent process before it forks any
  child, so every descendant inherits it.
- **FR-009**: Agent readiness detection MUST be unaffected by the priority
  request: a priority-requested agent MUST still be detected, still reach a
  ready state, and still accept a delivered prompt.
- **FR-010**: The command's help text and generated shell completions MUST
  describe the new priority argument.
- **FR-011**: The command's documented usage MUST describe the new priority
  argument.
- **FR-012**: The reported process information MUST include the scheduling
  priority of each foreground process, when the operating system can report it.
- **FR-013**: The reported scheduling priority MUST be absent when the
  operating system cannot determine it, rather than reported as a wrong value.
- **FR-014**: The priority request MUST be carried over the programmatic
  interface to the running server as an optional field, so an existing client
  that does not send it keeps working.
- **FR-015**: On operating systems without a Unix scheduling priority, the
  launch MUST be unchanged and the priority MUST be reported as absent.
- **FR-016**: The feature MUST apply to every supported agent kind, including
  kinds that take trailing arguments after `--`.
- **FR-017**: The requested priority MUST be the priority the agent actually
  runs at, on every platform that has a Unix scheduling priority. Where the
  platform's priority program adjusts the caller's priority instead of
  setting it, Herdr MUST compensate for the priority the pane's shell
  already runs at.
- **FR-018**: When a priority is in effect and the platform's priority
  program cannot be found, the launch MUST be rejected before any agent
  starts, with an error that names the program, rather than submitting a
  launch line that cannot run.
- **FR-019**: When a launch names a priority program, a startup failure MUST
  name that program, so the operator can tell a missing program apart from
  an agent that simply did not become ready.

### Key Entities

- **Requested priority**: the scheduling priority an operator asks for on a
  single launch. Range: -20 (most aggressive) to 19 (most deferred). Absent
  means "use the configured default".
- **Configured default priority**: the machine-wide scheduling priority stored
  in the Herdr configuration. Absent means "no priority prefix".
- **Launch line**: the command text typed into the target pane's shell. When a
  priority applies, the line gains a priority prefix ahead of the agent
  executable.
- **Foreground process**: a process running in a pane. Its reported
  information now carries the scheduling priority when the operating system
  can report one.

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-001**: An agent started with a requested priority of 19 reports a
  priority of 19 when its pane's process information is read, and every child
  it subsequently forks reports 19 as well.
- **SC-002**: A launch with no requested priority and no configured default
  produces a launch line identical to the pre-feature launch line, compared
  byte for byte.
- **SC-003**: A configured default of 19 is honored by a launch that requests
  no priority, and is overridden by a launch that requests a different value.
- **SC-004**: Every out-of-range requested priority is rejected with a
  non-zero exit status and a message naming the offending value and the valid
  range, and no agent is started.
- **SC-005**: A priority-requested agent still reaches a ready state and still
  accepts a delivered prompt, identically to a priority-free launch.
- **SC-006**: Every supported agent kind accepts the priority argument,
  including kinds invoked with trailing arguments after `--`.
- **SC-007**: The reported scheduling priority matches the actual priority of
  the running process in every case where the operating system can report it.
- **SC-008**: An agent requested at priority 19 runs at priority 19 even when
  the pane's shell runs at another priority.
- **SC-009**: On a host whose search path holds no priority program, a launch
  that requests a priority is rejected with an error naming the program, and
  no agent is started. A launch that requests no priority still succeeds on
  the same host.

## Assumptions

- The feature targets Unix-like operating systems, where scheduling priority
  is the standard CPU-niceness concept. Other operating systems keep their
  current behavior.
- The priority program is usually available on the target machine's search
  path. When it is not, Herdr rejects the launch before any agent starts and
  names the missing program, instead of leaving the operator with a launch
  line that can only time out.
- Raising priority above the default requires elevated privileges; an
  unprivileged Herdr cannot grant it, and the operating system reports the
  refusal at launch.
- The platform's priority program is not spelled the same everywhere: GNU
  `nice -n` sets the priority, while the macOS program adjusts the caller's
  priority. Herdr hides that difference, and the agent always ends up at the
  requested priority.
- Fresh launches are in scope. Agents resumed from a restored session are
  launched by a different path and are explicitly out of scope.
- The existing "renice after readiness" workaround used by affected hosts can
  be removed once this lands, because the priority now applies before the
  first fork.

## Out of Scope

- Changing the priority of an already-running agent.
- Per-agent-kind priority defaults; the default is machine-wide.
- Restoring a configured default onto agents resumed from a restored session.
- I/O priority, CPU affinity, or cgroup placement.
- Windows priority classes.
