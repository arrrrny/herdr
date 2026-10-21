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

## Cycle 10 — the requested priority must be the priority the agent runs at

The macOS CI job failed on the feature's own acceptance test:

```
thread 'agent_start_applies_a_requested_priority' panicked at tests/agent_priority.rs:377:5:
assertion `left == right` failed
  left: Some(9)
 right: Some(19)
```

The test passed on a developer machine and failed on the runner. The cause is
a platform difference in the priority program itself, measured on both:

```
/usr/bin/nice -n 5 /bin/sh -c '/usr/bin/nice -n 19 ...'   ->  20
/usr/bin/nice -n 5 /bin/sh -c '/usr/bin/nice -n 3 ...'    ->   8
```

macOS `nice -n` *adjusts* the caller's priority, while GNU `nice -n` sets it
outright. The runner's processes start at a priority of -10, so a request for
19 landed on 9. The operator asked for a priority, not for an adjustment, and
the feature promises the agent runs at the requested priority.

The launch line now carries the difference between the requested priority and
the priority the pane's shell already runs at, on the platforms whose program
adjusts. The shell's priority is read from the operating system before the
line is built, so the agent ends up at the requested priority on every
platform. On a host whose shell runs at the default priority, the launch line
is unchanged.

A new acceptance test starts the server at another priority and pins the
outcome:

```
cargo nextest run --locked -E 'binary(agent_priority)'
  Summary [  21.986s] 13 tests run: 13 passed (1 leaky), 0 skipped
```

The test is red without the fix — the deliberate mutant that restores the
plain `nice -n <request>` prefix fails it:

```
    thread 'the_requested_priority_is_reached_from_a_shell_at_another_priority' panicked at tests/agent_priority.rs:508:5:
    assertion `left == right` failed
      left: Array [String("nice"), String("-n"), String("19"), String("pi")]
     right: Array [String("nice"), String("-n"), String("14"), String("pi")]
```

## Cycle 11 — a missing priority program must not look like a successful launch

The review of the first push asked what happens on a host that does not have
the priority program. The answer was: nothing good. The launch line named a
program the pane's shell could not find, the shell wrote `command not found`
into the pane, the agent never appeared, and the caller waited out its
timeout with no hint about the cause.

The deliberate mutant that removed the availability check showed the same
failure from the caller's side, and it was worse than a timeout — it looked
like a success:

```
status=Some(0)
{"type":"agent_started","agent_status":"idle","interactive_ready":true,"argv":["nice","-n","19","pi"]}
```

The server accepted the launch and reported the agent as ready. The pane was
empty. The caller's only way to notice was to look at the pane.

The server now checks the program the launch line names against its own
search path before the line is submitted, and refuses the launch with
`agent_nice_unavailable`, naming the program. The rejection happens before
anything is written to the pane, so a refused launch leaves the pane
untouched.

Two acceptance tests pin this down:

```
cargo nextest run --locked -E 'binary(agent_priority)'
  Summary [  22.177s] 15 tests run: 15 passed, 0 skipped
```

- `agent_start_reports_a_missing_priority_program` — the launch is refused
  with an error that names the program, and no agent is started.
- `a_launch_without_a_priority_does_not_need_the_priority_program` — a
  priority-free launch still succeeds on that same host, because the check
  only applies when a priority is actually in effect.

## Cycle 12 — the socket path checks the range itself, and the launch line is not a constant

Two gaps showed up in the socket-level evidence.

The first was that the raw socket path had no test for an out-of-range
requested priority. The CLI rejects an out-of-range value before it sends
anything, but the socket can be driven by a client that is not the Herdr CLI,
so the server has to refuse the value itself. The server does refuse it, and
`tests/api_ping.rs::agent_start_rejects_an_out_of_range_priority_over_the_socket`
now pins that down; the deliberate mutant that drops the server-side range
check fails it.

The second was that the socket test asserted a fixed launch line:

```rust
serde_json::json!(["nice", "-n", "19", "pi"])
```

That is only true on a host whose processes start at the default priority.
The macOS CI runner starts its processes at a priority of -10, and macOS
`nice -n` adjusts the caller's priority, so the launch line there carries
`29`, not `19`. The test failed on the runner for a reason that had nothing
to do with the feature.

The assertion now derives the expected adjustment from the priority the
pane's shell actually runs at:

```rust
serde_json::json!([
    "nice",
    "-n",
    expected_adjustment(19, launch_baseline()),
    "pi"
])
```

`launch_baseline()` reads the priority of the test process, which the server
and the pane's shell inherit, and `expected_adjustment` applies the
platform's rule: the requested value on a platform that sets the priority,
the difference on a platform that adjusts it. The test now states the
invariant — the launch line carries the right adjustment for this host —
instead of a number that only holds on one of them.

## Final state

```
cargo nextest run --locked -E 'binary(agent_priority)'
  Summary [  22.177s] 15 tests run: 15 passed, 0 skipped

cargo nextest run --locked -E 'test(agent_start_carries_the_scheduling_priority_over_the_socket) or test(agent_start_rejects_an_out_of_range_priority_over_the_socket)'
  Summary [   3.605s] 2 tests run: 2 passed, 4084 skipped
```
