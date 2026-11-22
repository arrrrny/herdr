# Quickstart: agent start nice level

Runnable validation scenarios that prove the feature works end to end.

## Prerequisites

- A built Herdr binary (`cargo build` or `cargo run -- ...`).
- A running Herdr server with at least one pane.
- On Linux/macOS: a `nice` program on the search path (standard on both).
- Nothing else. No privileges are needed for the positive-value scenarios.

## Scenario 1 — A launch without a priority is unchanged

This is the compatibility baseline: the feature must be inert by default.

```bash
# Start an agent with no priority and capture the launch line the server used.
herdr agent start compat --kind kimi --pane "$PANE"
```

Expected: the agent starts, and the reported launch arguments are exactly the
agent executable followed by the agent's own arguments — no priority tokens.

## Scenario 2 — A requested priority is applied and inherited

```bash
herdr agent start niced --kind kimi --pane "$PANE" --nice 19
```

Then read the pane's process information:

```bash
herdr pane process-info --pane "$PANE"
```

Expected:

- The launch arguments include the priority prefix ahead of the agent
  executable.
- The reported scheduling priority of the agent's foreground process is `19`.
- Any child the agent forks afterwards reports `19` as well.

## Scenario 3 — The configured default is honored and overridden

```bash
cat > "$HERDR_CONFIG_PATH" <<'TOML'
[agent.start]
nice = 19
TOML
herdr server reload-agent-manifests >/dev/null 2>&1 || true   # any config-touching command
herdr agent start defaulted --kind kimi --pane "$PANE"
herdr pane process-info --pane "$PANE"
```

Expected: the agent runs at priority `19`.

Now override it:

```bash
herdr agent start overridden --kind kimi --pane "$PANE" --nice 5
herdr pane process-info --pane "$PANE"
```

Expected: the agent runs at priority `5`, not `19`.

Remove the `[agent.start]` section and confirm the launch returns to the
Scenario 1 behavior.

## Scenario 4 — An out-of-range value is rejected, not clamped

```bash
herdr agent start bad --kind kimi --pane "$PANE" --nice 20; echo "exit=$?"
herdr agent start bad --kind kimi --pane "$PANE" --nice -21; echo "exit=$?"
```

Expected: a non-zero exit status, a message on standard error naming the
offending value and the valid range `-20..19`, and no agent started.

## Scenario 5 — Detection and prompt delivery are unaffected

```bash
herdr agent start ready --kind kimi --pane "$PANE" --nice 19
herdr agent prompt ready --text "hello"
```

Expected: the agent reaches a ready state and the prompt is delivered, exactly
as for a launch without a priority.

## Scenario 6 — Every agent kind accepts the priority

Repeat Scenario 2 with each supported kind, including one that takes trailing
arguments:

```bash
herdr agent start withargs --kind kimi --pane "$PANE" --nice 19 -- --auto
```

Expected: the agent's own arguments are forwarded unchanged after the
priority prefix, and the priority is applied.

## Scenario 7 — A bad configured default is reported, not fatal

```bash
cat > "$HERDR_CONFIG_PATH" <<'TOML'
[agent.start]
nice = 99
TOML
herdr agent start baddefault --kind kimi --pane "$PANE"
```

Expected: the configuration is reported as invalid for that key, the server
keeps running, and the launch behaves as if no default were configured.
