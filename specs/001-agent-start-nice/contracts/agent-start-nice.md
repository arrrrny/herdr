# Contract: agent start nice level

Three surfaces change: the command line, the configuration file, and the
socket interface. The command line and the configuration file are additive.
The socket interface gains one optional request field and one optional
response field.

## 1. Command line

```
herdr agent start <NAME> --kind <KIND> --pane <ID> [--timeout MS] [--nice N] [-- <AGENT-ARGS>...]
```

### `--nice N`

| Aspect      | Value                                                        |
| ----------- | ------------------------------------------------------------ |
| Type        | Signed integer, decimal                                      |
| Range       | -20 ..= 19 inclusive                                         |
| Required    | No. Absent means "use the configured default".               |
| Default     | Absent                                                        |
| Repeats     | Last occurrence wins, matching the other flag handling in this command. |

**Behavior**

- A value inside the range is sent to the server as the requested priority.
- A value outside the range is rejected locally, before any request is sent:
  a message on standard error naming the offending value and the valid
  range, and a non-zero exit status. No agent is started.
- A value of `0` is valid and produces a launch line identical to the
  pre-feature launch line.
- Negative values require elevated privileges. The operating system refuses
  them for an unprivileged Herdr, and the refusal is surfaced as the launch
  error.
- The agent runs at exactly the requested priority, on every platform that
  has a Unix scheduling priority.

**Trailing arguments**: the priority argument is parsed before the `--`
separator. Agent arguments after `--` are forwarded to the agent executable
unchanged and are never reordered by the priority prefix.

### Help text and completions

`herdr agent start --help` and the generated shell completions list the new
argument with the same wording as the other optional arguments of the command.

## 2. Configuration file

```toml
[agent.start]
nice = 19
```

| Key    | Type    | Range      | Default | Meaning                                     |
| ------ | ------- | ---------- | ------- | ------------------------------------------- |
| `nice` | integer | -20 ..= 19 | `0`     | Default scheduling priority for fresh launches. |

**Behavior**

- The section is optional. When it is absent, the default is `0`, which means
  "no priority prefix".
- The value is validated when the configuration is loaded. An out-of-range
  value produces a configuration diagnostic and the effective default falls
  back to `0`. The value is never clamped.
- The value is refreshed when the configuration is reloaded, and applies to
  launches started after the reload.

**Interaction with the command line**: a priority given on the command line
overrides the configured default for that launch. The configured default is
not changed by the command line.

## 3. Socket interface

### Request

The agent-start request gains one optional field.

| Field  | Type    | Required | Range      | Meaning                     |
| ------ | ------- | -------- | ---------- | --------------------------- |
| `nice` | integer | **no**   | -20 ..= 19 | The requested priority.     |

- Absent means "use the configured default".
- A value outside the range is rejected by the server with an error response,
  because the request may come from a client that is not the Herdr CLI.
- A launch whose priority program is not on the server's search path is
  rejected with `agent_nice_unavailable` and no agent is started.
- A client that predates this field sends no `nice` and is unaffected.

### Response

The process-information response gains one optional field per foreground
process.

| Field  | Type    | Required | Meaning                                          |
| ------ | ------- | -------- | ------------------------------------------------ |
| `nice` | integer | **no**   | The scheduling priority the operating system reports. |

- Absent when the operating system cannot determine the priority.
- Never `0` as a substitute for "unknown", because `0` is a real priority.

### Compatibility

Both fields are optional and additive. No published codec changes, so the
protocol version is not bumped. An older client ignores the new response
field; an older client sends no request field and keeps the pre-feature
behavior.

## 4. Platform behavior

| Platform      | Launch line with a requested priority      | Reported priority |
| ------------- | ----------------------------------------- | ----------------- |
| Linux         | gains a priority prefix                   | reported          |
| macOS         | gains a priority prefix                   | reported          |
| Windows       | unchanged                                 | absent            |
| other         | unchanged                                 | absent            |

**The requested priority is the priority the agent runs at**, on every
platform above that reports one. Where the platform's priority program sets
the priority outright, the requested value is passed through. Where it
adjusts the caller's priority instead, the launch line carries the difference
between the requested priority and the priority the pane's shell already
runs at, so the agent still ends up at the requested priority.
