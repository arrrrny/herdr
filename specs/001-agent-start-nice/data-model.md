# Data Model: agent start nice level

## AgentStartPriority (value object)

The scheduling priority of a launch. Not a stored entity: it is a validated
value that exists only for the duration of a launch decision.

| Field      | Type    | Range          | Meaning                                            |
| ---------- | ------- | -------------- | -------------------------------------------------- |
| `value`    | integer | -20 ..= 19     | The requested CPU scheduling priority.              |
| `source`   | enum    | `Requested` / `Configured` / `None` | Where the effective value came from. |

**Validation rules**

- A `Requested` value outside -20..=19 is rejected before launch, with a
  non-zero exit status and a message naming the value and the valid range.
  It is never clamped.
- A `Configured` value outside -20..=19 is reported as a configuration
  diagnostic; the effective default falls back to "no priority prefix". It is
  never clamped.
- A value of `0` is valid and means "no priority prefix": the launch line is
  identical to the pre-feature launch line.

**State transitions**

```text
        operator supplies --nice          configuration supplies [agent.start].nice
                 │                                        │
                 ▼                                        ▼
            Requested(value)                        Configured(value)
                 │                                        │
                 │  requested wins                       │ invalid → diagnostic + fall back
                 ▼                                        ▼
              effective value ────────────────► launch line gets a priority prefix
                                                       │
                                                       │ value == 0 or no value
                                                       ▼
                                                  launch line unchanged
```

## ConfiguredDefault (state field)

The machine-wide default, held in the server's application state and refreshed
whenever the configuration is loaded or reloaded.

| Field | Type    | Range      | Default | Meaning                                 |
| ----- | ------- | ---------- | ------- | --------------------------------------- |
| `nice`| integer | -20 ..= 19 | `0`     | The default priority for fresh launches. |

`0` is the default, and it means "no priority prefix", so the feature is inert
until an operator opts in.

## LaunchLine (derived value)

The command text typed into the target pane's shell.

| Field    | Type                | Meaning                                        |
| -------- | ------------------- | ---------------------------------------------- |
| `tokens` | ordered token list  | The agent executable followed by the agent's own arguments. |
| `prefix` | ordered token list  | Empty, or the priority program and its value.   |

**Invariant**: when `prefix` is empty, the launch line is byte-for-byte the
launch line produced before this feature existed. This is the property the
acceptance test pins, and it is the reason the prefix is prepended as tokens
rather than as a formatted string.

## PaneProcessInfoProcess (extended entity)

The reported information about a foreground process in a pane. The scheduling
priority is a new optional field.

| Field      | Type    | Required | Meaning                                                     |
| ---------- | ------- | -------- | ----------------------------------------------------------- |
| `pid`      | integer | yes      | The process id.                                              |
| `name`     | text    | yes      | The executable name.                                         |
| `nice`     | integer | **no**   | The scheduling priority, when the operating system reports it. |

**Invariant**: `nice` is absent when the operating system cannot determine it
— never `0` as a substitute, because `0` is a real priority value.

## Relationships

- `ConfiguredDefault` → `LaunchLine`: supplies the fallback value when no
  priority was requested.
- `RequestedPriority` → `LaunchLine`: supplies the value when the operator
  asked for one, overriding the configured default.
- `LaunchLine` → the agent process: the priority prefix applies before the
  agent's first fork, so descendants inherit it.
- `PaneProcessInfoProcess.nice` → the agent process: reports the priority the
  operating system actually applied, which is not necessarily the requested
  one (a privileged requirement may be refused).
