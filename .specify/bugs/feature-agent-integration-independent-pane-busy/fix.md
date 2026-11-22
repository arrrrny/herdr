# Feature: agent-integration-independent pane busy signal + `pane wait --idle`

- Slug: feature-agent-integration-independent-pane-busy
- Fixed: 2026-08-23
- Assessment: ./assessment.md
- Issues: arrrrny/herdr#14, arrrrny/herdr#15
- Status: applied

## Summary

Adds a busy signal that does **not** depend on any agent integration. A pane is
*busy* when a foreground process group other than the interactive shell is present
on its pty, and *idle* (busy = false) when only the shell is running.

Two coupled surfaces:

1. A non-agent `busy` boolean on `PaneProcessInfo` in the process-info API
   (`herdr pane process-info`), computed purely from pty/process facts
   (`foreground_process_group_id` vs `shell_pid`).
2. A blocking `herdr pane wait --idle [--timeout MS]` CLI primitive that polls the
   `busy` field and returns when the pane has been idle across two ~1s samples.

The two-sample (≥2 consecutive idle samples ~1s apart) confirmation lives in the
new `pane wait --idle` poller so a single idle sample cannot race a just-typed
command's startup. See the #15 fix report for the rationale and the note about
`agent wait`, which is a different (server-side, status-driven) primitive.

## Changes

| File | Change | Notes |
| --- | --- | --- |
| `src/api/schema/panes.rs` | Add `pub busy: bool` (with `#[serde(default)]`) to `PaneProcessInfo` | Documents the agent-independent busy signal; defaults to `false` so old clients ignore it |
| `src/app/api/panes.rs` | Compute `busy` in `handle_pane_process_info` and pass it into the `PaneProcessInfo` construction | `busy = foreground_process_group_id.zip(shell_pid).is_some_and(\|(fg, sp)\| fg != sp)` |
| `src/cli/pane.rs` | Add `pane wait --idle` command: dispatch arm, `pane_wait`, `idle_streak_after_sample`, `parse_pane_wait_args`, and tests | Mirrors `pane wait-output` / `parse_pane_wait_output_args`; polls `PaneProcessInfo.busy` every 1s, requires 2 consecutive idle samples |
| `src/cli/spec.rs` | Add `wait` subcommand to `pane` (required `pane_id`, required `--idle`, optional `--timeout`) | Mirrors existing `required`/`flag`/`option` helper usage |
| `docs/next/api/herdr-api.schema.json` | Regenerate so `PaneProcessInfo` includes `busy` | Regenerated via `HERDR_UPDATE_API_SCHEMA=1` for `api::schema::tests::generated_protocol_schema_artifact_is_current` |

`busy` semantics note: a background job launched with `cmd &` also lands in the
foreground process group here, so it counts as busy too. That is acceptable for
the idle signal and is recorded in a code comment at the computation site.

## Tests Added

In `src/cli/pane.rs` `#[cfg(test)] mod tests`:

1. `idle_streak_after_sample_resets_on_busy` — a busy sample resets the streak to 0.
2. `idle_streak_after_sample_reaches_two_on_consecutive_idle` — two consecutive
   idle samples reach a streak of 2 (the return threshold).
3. `idle_streak_after_sample_busy_between_idles_resets` — a busy sample between
   two idle samples resets, so idle is only confirmed after two consecutive idles.
4. `parse_pane_wait_args_parses_pane_id_idle_and_timeout` — parses
   `issue-1 --idle --timeout 5000`.
5. `parse_pane_wait_args_accepts_equals_timeout` — accepts `--timeout=5000`.
6. `parse_pane_wait_args_requires_idle` — errors (usage) without `--idle`.
7. `parse_pane_wait_args_requires_pane_id` — errors (usage) without a pane id.

In `src/cli/spec.rs` `#[cfg(test)] mod tests`:

8. `spec_matches_refactored_agent_and_pane_commands` — augmented to assert the
   `pane` command exposes the `wait` subcommand (mirrors the existing
   `wait-output` assertion).

In `src/api/schema/tests.rs`:

9. `generated_protocol_schema_artifact_is_current` — now re-validates the
   regenerated artifact, which contains `busy` under `PaneProcessInfo`.

## Local Verification

```
export ZIG=$(ls -d /workspace/herdr/.cache/zig/zig-x86_64-linux-* | head -1)

cargo build --bin herdr
  -> Finished; binary compiles with vendored libghostty-vt (Zig 0.15.2)

cargo test --bin herdr cli::pane
  -> test result: ok. 49 passed; 0 failed

cargo test --bin herdr cli::spec
  -> test result: ok. 18 passed; 0 failed

cargo test --bin herdr api::schema
  -> test result: ok. 42 passed; 0 failed
  (includes generated_protocol_schema_artifact_is_current, which now embeds busy)
```

`cargo fmt -- --check` is clean for all touched files. `herdr pane wait --help`
prints `herdr pane wait [OPTIONS] --idle <PANE_ID>` with `--idle` (required) and
`--timeout <MS>`.

## Deviations from Assessment

The assessment files for this bug were scaffolded (assessment/issue stubs). The
implementation follows the precise change list from the task rather than a
navigated root cause; no deviation from that list beyond the two notes below.

- **`parse_pane_wait_args` enforces `--idle`.** The provided `parse_pane_wait_args`
  skeleton did not reject a missing `--idle`; the upstream test requirement
  ("errors without `--idle`/pane_id") and the codebase pattern (e.g.
  `parse_pane_wait_output_args` returns `"missing required --match or --regex"`)
  both expect the parser to enforce required flags. I added
  `if !idle { return Err(USAGE.into()); }` after pane-id resolution. The
  defensive `if !params.idle` usage check in `pane_wait` is kept as-is (provided)
  and is now redundant but harmless.
- **Schema regenerated via the test harness, not `api schema --output`.** `herdr
  api schema --output PATH` only writes the already-checked-in `include_str!`
  constant, so it cannot reflect source changes. The artifact was regenerated by
  running `generated_protocol_schema_artifact_is_current` with
  `HERDR_UPDATE_API_SCHEMA=1`, which is the authoritative path used by CI.

## Follow-ups

- An end-to-end CLI test driving `herdr pane wait --idle` against a live daemon
  (typing a command, letting it finish, asserting return 0; and a `--timeout`
  expiry asserting return 1) would lock the polling/timeout behavior at the API
  surface. Unit tests here cover the parser and the streak logic directly.
- `agent wait` is intentionally NOT covered by the two-sample rule (see the #15
  fix report); do not merge the pty-based confirmation into the agent-wait path.
