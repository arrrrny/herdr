# Phase 0 Research: agent start nice level

**Feature**: 001-agent-start-nice | **Date**: 2026-10-21

## R1 — Where does the priority have to be applied?

**Decision**: Prefix the launch line with the priority program and its value,
ahead of the agent executable, at the point where the launch line is built for
the target pane's shell.

**Rationale**: The launch line is a plain shell command typed into the pane.
A prefix on that line lands on the agent process before its first `fork`, and
scheduling priority is inherited by every child. The alternative — re-prioritizing
the process after the agent becomes ready — has a birth window: every child
forked before the re-prioritization inherits the default priority instead. The
issue author's measured incident depends exactly on that window, because a test
suite's fan-out happens in it.

**Alternatives considered**:
- *Post-launch re-prioritization* — rejected: it cannot cover the birth window.
- *Set the priority from inside the agent* — rejected: the agent does not know
  it is managed, and the request must work for every supported agent kind.
- *A scheduler/cgroup wrapper* — rejected: out of scope, and it does not
  compose with the existing single-shell-command launch model.

## R2 — Should the prefix be platform-specific?

**Decision**: Yes. The prefix is produced by a platform-level function, and
the platform that has no Unix scheduling priority produces no prefix at all.

**Rationale**: The project keeps OS-specific behavior in `src/platform/<os>.rs`
and keeps core modules free of target conditionals. The launch-line builder is
already platform-specific (POSIX quoting on Unix, a PowerShell wrapper on
Windows), so the priority prefix belongs in the same boundary. A bare
`cfg!(windows)` check in the agent module would violate the project's
platform-isolation rule.

**Alternatives considered**:
- *Emit the prefix unconditionally and let Windows fail* — rejected: it would
  break every Windows launch.
- *Reject the request on Windows* — rejected: it would make the flag
  unusable there and would break callers that set a fleet-wide default on a
  mixed host. Ignoring it is the behavior the spec pins.

## R3 — How is the request carried from the CLI to the server?

**Decision**: An optional field on the agent-start request. The CLI sends it
only when the operator asked for one; the server falls back to the configured
default when the field is absent.

**Rationale**: The agent-start call already travels over the JSON socket
interface as a structured request, so the request is the natural carrier. The
field is optional and defaults to absent, so an older client that never sends
it keeps working, and the change is additive on the wire.

**Alternatives considered**:
- *Fold the prefix into the agent arguments* — rejected: the agent arguments
  are the agent's own arguments and are forwarded verbatim to the agent
  executable; a prefix there would be handed to the agent as an argument.
- *A separate "launch wrapper" request field* — rejected: it is the same
  concept, so a separate field would be redundant.

## R4 — How is the configured default applied?

**Decision**: The configured default lives in the server's application state,
refreshed whenever the configuration is loaded or reloaded, and is read at
launch time.

**Rationale**: The project already stores configuration values in the
application state and refreshes them on live reload, rather than re-reading
the configuration file inside a request handler. A live reload of the default
must take effect for the next launch, so the value has to live in state.

**Alternatives considered**:
- *Read the configuration file inside the launch handler* — rejected: it
  breaks live reload and does filesystem I/O on a latency-sensitive path.
- *Resolve the default in the CLI* — rejected: a caller that uses the socket
  interface directly would then miss the default, and the default is a
  machine-wide operator setting, not a caller setting.

## R5 — What happens to an out-of-range value?

**Decision**: Two different failure modes, because the two values have
different owners.

- A *requested* value (from the operator) is rejected before anything is
  launched, with a non-zero exit status and a message naming the offending
  value and the valid range. It is never clamped.
- A *configured* value (from the configuration file) is reported as a
  configuration diagnostic, and the effective default falls back to "no
  priority prefix". It is never clamped either.

**Rationale**: Clamping silently would launch an agent at a priority the
operator did not ask for — the opposite of what an explicit request means. A
bad configuration value must not stop the server from starting or reloading,
which is the established pattern for invalid configuration in this project.

**Alternatives considered**:
- *Clamp both to the nearest valid value* — rejected: a silent wrong answer.
- *Fail server startup on a bad configured value* — rejected: it would make a
  typo in one key take the whole session down.

## R6 — How is the resulting priority reported?

**Decision**: The pane's process information gains an optional scheduling
priority per foreground process, sourced from the operating system.

**Rationale**: The issue asks for verification rather than assumption. The
operating system is the only authority on the actual priority, so the reported
value must come from there and not from the value that was requested. The
field is optional so that a platform without the concept, or a process the
operating system will not report about, produces an absent value rather than a
wrong one.

**Alternatives considered**:
- *Echo the requested value back in the launch response* — rejected: it
  reports the request, not the outcome, so it cannot detect a refused request.
- *Add a dedicated "nice" command* — rejected: it duplicates the existing
  process-information surface.

## R7 — Does the priority affect agent detection?

**Decision**: No. Detection is unchanged, and a test pins that a
priority-requested agent still becomes ready and still accepts a prompt.

**Rationale**: The priority changes the process's scheduling attributes, not
its executable, its title, or its screen output. Detection recognizes the
agent in the pane, so it is unaffected. The test exists because "should be
unaffected" is exactly the kind of claim that silently rots.

**Alternatives considered**: none — the requirement is to keep the existing
behavior and prove it.

## R8 — What is the valid range, and what are the privilege semantics?

**Decision**: The supported range is -20 to 19 inclusive, matching the
operating system's own range. Negative values need elevated privileges; the
operating system refuses them for an unprivileged Herdr, and the refusal is
surfaced to the operator.

**Rationale**: The range is the operating system's, not an arbitrary
restriction, so the feature cannot express a value the operating system would
reject for range reasons. Privilege errors are the operating system's to
report, and the documentation states the requirement up front rather than
letting the operator discover it by failure.

**Alternatives considered**:
- *Restrict the range to 0..19* — rejected: it would make the feature unable
  to express what the operating system can express, and would hide a real
  capability.

## R9 — Does the platform's priority program mean the same thing everywhere?

**Decision**: No, and Herdr compensates for it. On GNU systems `nice -n N`
sets the priority to `N`; on macOS `nice -n N` *adjusts* the caller's
priority by `N`, and the result is capped at 20. The launch line therefore
carries the difference between the requested priority and the priority the
pane's shell already runs at, on the platforms whose program adjusts.

**Rationale**: The operator asked for a priority, not for an adjustment. A host
whose shell does not run at the default priority — a machine whose processes
are already scheduled aggressively, a CI runner that elevates its job
processes — would otherwise land the agent somewhere else entirely, and the
reported priority would silently disagree with the request. The pane's shell
is the process the launch line runs in, so its priority is the baseline, and
Herdr reads it from the operating system before building the line.

**Alternatives considered**:
- *Keep the plain `nice -n N` prefix everywhere and document the difference* —
  rejected: the agent would not run at the requested priority, which is the
  one thing the flag promises.
- *Compute the difference inside the shell, with a nested `sh -c`* — rejected:
  it puts a `ps` call and a nested shell into every launch line, and it makes
  the line depend on a program the pane may not have.
- *Rename the flag to say "adjust by"* — rejected: the operator wants a
  priority.

## R10 — What happens when the priority program is not installed?

**Decision**: The launch is rejected before any agent starts, with an error
that names the missing program (`agent_nice_unavailable`). A launch that
requests no priority is unaffected.

**Rationale**: The launch line runs in the pane's shell, which inherits the
server's environment, so the server's search path decides whether the
priority program is found there. If it is missing, the shell reports
`command not found` inside the pane and the agent simply never appears. The
operator is left with a launch that timed out, and nothing in the error says
that the machine is missing `nice`. Checking the program before the launch
line is submitted turns that silence into a named error, and it leaves the
pane untouched.

The check is only meaningful when a priority is actually in effect: a
priority-free launch line names no priority program, so it must keep working
on a host that does not have one.

**Alternatives considered**:
- *Let the launch fail in the pane and surface the shell's own error* —
  rejected: the shell's error is written into the pane, not returned to the
  caller, and the operator sees a timeout instead of the cause.
- *Fall back to an unprioritized launch* — rejected: silently dropping the
  requested priority is worse than refusing the launch; the operator asked for
  a specific priority.
- *Require the program at startup* — rejected: the priority is opt-in, so a
  host without the program must keep working for every other launch.

## Resolved clarifications

All `NEEDS CLARIFICATION` markers from the technical context were resolved
into R1-R10 above. No marker remains open.
