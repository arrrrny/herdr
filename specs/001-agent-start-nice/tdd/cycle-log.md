---
feature: 001-agent-start-nice
profile: .specify/memory/tdd-profile.md
engine: manual (no zfa available on this machine)
baseline_commit: a69eee85
---

# Cycle Log: agent start nice level

The deterministic TDD engine (`zfa`) is not installed on this machine, and no
mutation or coverage tooling is available either (`cargo mutants`,
`cargo llvm-cov`, `cargo tarpaulin` and `cargo proptest` are all absent). The
loop was therefore driven by hand:

- The **red** side of every behavior is proven by a deliberate mutant: the
  behavior is broken in the production code, the pinning test is observed to
  fail, and the source is restored. That is the same evidence a mutation
  tool would collect, produced one mutant at a time.
- The **green** side is the ordinary suite run against the restored source.

Each cycle below names the mutant that produced the red state, the exact
failure the pinning test reported, and the command that returned to green.

## Baseline

```
cargo nextest run --locked --status-level fail --final-status-level fail \
  --failure-output final --success-output never
```

Green on `a69eee85` before the feature work started. The pre-existing suite
is the reference the new tests are compared against.

One environmental failure is present in the baseline on this machine and is
unrelated to the feature: `tests/live_handoff.rs::live_handoff_preserves_keyboard_protocol_for_client_input`
fails with `failed to openpty: Device not configured`.

## Cycle 1 — the requested priority reaches the launch line

**Red** (mutant "the launch line drops the priority prefix", `src/app/agents.rs`):

```
thread 'agent_start_applies_a_requested_priority' panicked at tests/agent_priority.rs:332:5:
assertion `left == right` failed
  left: Array [String("pi")]
 right: Array [String("nice"), String("-n"), String("19"), String("pi")]
```

**Green**:

```
cargo nextest run --locked -E 'binary(agent_priority)'
```

12 passed. The launch line keeps the agent executable and its arguments
unchanged and only gains the priority prefix in front of them.

## Cycle 2 — the configured default and the flag override

**Red** (mutant "resolve ignores the requested priority",
`src/agent_priority.rs`):

```
thread 'agent_start_honors_the_configured_default_and_lets_the_flag_override_it' panicked at tests/agent_priority.rs:438:5:
assertion `left == right` failed
  left: Array [String("nice"), String("-n"), String("19"), String("pi")]
 right: Array [String("nice"), String("-n"), String("5"), String("pi")]
```

**Green**: the same acceptance run, 12 passed.

## Cycle 3 — the range check rejects instead of clamping

**Red** (mutant "the range check accepts every value",
`src/agent_priority.rs`):

```
thread 'cli::agent::bindings_tests::agent_start_rejects_an_out_of_range_priority_before_any_request' panicked at src/cli/agent/bindings_tests.rs:251:13
thread 'agent_start_rejects_an_out_of_range_priority' panicked at tests/agent_priority.rs:489:5:
assertion failed: !rejected.status.success()
```

**Green**: the same acceptance run, 12 passed.

## Cycle 4 — the applied priority is reported

**Red** (mutant "the reported process priority is dropped",
`src/app/api/panes.rs`):

```
thread 'pane_process_info_reports_the_applied_priority' panicked at tests/agent_priority.rs:630:5:
assertion `left == right` failed
  left: Null
 right: 19
```

**Green**: the same acceptance run, 12 passed.

## Cycle 5 — the configuration section is known

**Red** (mutant "the agent section is not a known configuration section",
`src/config/io.rs`):

```
thread 'config::io::tests::load_live_config_parses_the_agent_start_section' panicked at src/config/io.rs:1224:9
```

**Green**: the same acceptance run, 12 passed.

## Cycle 6 — the CLI parses the requested priority

**Red** (mutant "the CLI drops the requested priority",
`src/cli/agent/bindings.rs`):

```
thread 'cli::agent::bindings_tests::agent_start_parses_a_requested_priority' panicked at src/cli/agent/bindings_tests.rs:218:5:
assertion `left == right` failed
  left: Object {"kind": String("codex"), "name": String("name"), "pane_id": String("1")}
 right: Object {"kind": String("codex"), "name": String("name"), "nice": Number(19), "pane_id": String("1")}
```

**Green**: the same acceptance run, 12 passed.

## Cycle 7 — a real regression the unit tests alone did not catch

After the mutant audit finished, the acceptance suite went red against the
build on disk:

```
thread 'agent_start_passes_trailing_agent_arguments_with_a_priority' panicked at tests/agent_priority.rs:587:5:
assertion `left == right` failed
  left: Array [String("pi"), String("--name"), String("scratch"), String("--no-session")]
 right: Array [String("nice"), String("-n"), String("19"), String("pi"), String("--name"), String("scratch"), String("--no-session")]

thread 'agent_start_honors_the_configured_default_and_lets_the_flag_override_it' panicked at tests/agent_priority.rs:438:5:
assertion `left == right` failed
  left: Array [String("nice"), String("-n"), String("19"), String("pi")]
 right: Array [String("nice"), String("-n"), String("5"), String("pi")]
```

The cause was a stale build, not a source defect: the audit's last mutant was
compiled while the source was mutated, and the source was restored before
cargo recorded its fingerprint, so cargo treated the mutated artifact as
up to date. The mutated CLI still accepted `--nice` but dropped the value
before it reached the request.

The unit test that covers the CLI parse (`agent_start_parses_a_requested_priority`)
went red for the same reason, which is what identified the build as the
culprit rather than the code:

```
cargo nextest run --locked -E 'test(agent_start_parses_a_requested_priority)'
```

**Green** after rebuilding from the restored source:

```
touch src/agent_priority.rs src/app/agents.rs src/app/api/panes.rs \
  src/config/io.rs src/cli/agent/bindings.rs
cargo nextest run --locked -E 'binary(agent_priority)'
```

12 passed.

This cycle is the reason the acceptance suite lives outside the crate's unit
tests. The unit test builds the request from the parsed parameters and never
exercises `agent_start()` itself, so it stayed green against a build whose
CLI dropped the value on the way to the server. The acceptance test drives
the real binary over the real socket and caught the regression.

## Cycle 8 — the agent writes its arguments after it is detected

The full-suite run of `just check` failed the acceptance suite:

```
thread 'agent_start_passes_trailing_agent_arguments_with_a_priority' panicked at tests/agent_priority.rs:593:48:
called `Result::unwrap()` on an `Err` value: Os { code: 2, kind: NotFound, message: "No such file or directory" }
```

Herdr can detect the fake agent from its process before the fake agent's first
write reaches the disk, so a direct read of the arguments file races with
the launch. The read now waits for the file to hold the expected content.

**Green**: the acceptance run, 12 passed.

## Cycle 9 — the spawned server must not share the session's agent socket

The same full-suite run left the server-spawning tests failing with:

```
thread 'agent_start_applies_a_requested_priority' panicked at tests/agent_priority.rs:80:5:
socket did not appear at /tmp/hprio-.../runtime/herdr.sock
```

The server's log showed it reaching `api server listening` about five seconds
after the spawn, which is past the test's wait. The cause was the
environment, not the feature: the suite runs inside a Herdr session, so the
spawned server inherited `SSH_AUTH_SOCK` pointing at the live session's agent
socket. Every server the suite starts in parallel then contends for that one
socket, and the startup stalls.

The spawned server now drops the inherited `SSH_AUTH_SOCK` and serves its
own, so the suite is hermetic whether or not it runs inside a Herdr session:

```
cargo nextest run --locked -E 'binary(agent_priority)'
  Summary [  27.516s] 12 tests run: 12 passed, 0 skipped
```

The same run with `SSH_AUTH_SOCK` removed from the whole environment also
passes, which confirms the cause.

## Final state

```
cargo nextest run --locked -E 'binary(agent_priority)'
  Summary [  14.873s] 12 tests run: 12 passed, 0 skipped

cargo nextest run --locked -E 'test(agent_start_carries_the_scheduling_priority_over_the_socket)'
  Summary [   4.019s] 1 test run: 1 passed, 4077 skipped
```
