# Bug Fix: a saved SSH machine stays dead after a transient remote failure

- **Slug**: `saved-ssh-machine-stops-reconnecting`
- **Fixed**: 2026-09-18
- **Assessment**: ./assessment.md
- **Status**: applied

## Summary

A saved SSH machine stopped reconnecting permanently when a reconnect attempt could not *confirm*
a usable remote Herdr — for example while the remote was briefly unhealthy after the link dropped.
`find_installed_remote_herdr` collapsed "the remote answered with an unusable Herdr" and "the
probe could not be completed at all" into one `Unsupported` verdict, which
`saved_ssh_failure_needs_attention` reads as *needs attention*; `record_status(Attention)` then
clears `next_attempt`, so the supervisor never scheduled another attempt for the rest of the
client's life. The machine stayed `! attention` even after the remote was healthy again, forcing
the user to detach and restart the client.

The fix keeps the permanent verdict for positive evidence only (a candidate that answered with a
status failing the endpoint requirement, or a discovery that positively found no binary) and
returns a retryable `ConnectionAborted` — "could not confirm a usable Herdr on <target>;
reconnecting" — when the probe was inconclusive. The machine now keeps re-dialing on the existing
500ms → 120s backoff and reconnects by itself once the remote answers again.

## Changes

| File | Change | Notes |
| --- | --- | --- |
| `src/remote/attach.rs` | `remote_client_status` returns a new `RemoteClientProbe` (`Status` / `Unconfirmed`) instead of `Option<RemoteClientStatusJson>`. | Distinguishes a decoded client status from a probe that could not be completed. |
| `src/remote/attach.rs` | `find_installed_remote_herdr` now records both a positively outdated candidate and an unconfirmed probe, and delegates the verdict to a new `remote_herdr_unavailable`. | The retryable kind and message match no `saved_ssh_failure_needs_attention` needle, so the supervisor keeps scheduling attempts. |
| `src/remote/attach.rs` | `remote_herdr_unavailable(target, missing, outdated, unconfirmed)` keeps the existing `Unsupported` "not ready … install or update" error when discovery positively found nothing (`missing`) or every candidate answered and none was usable (`outdated`), and returns a retryable `ConnectionAborted` whenever **any** candidate could not be probed (`unconfirmed`). | One unresponsive candidate is enough to make the pass inconclusive: another candidate answering "too old" does not rule out that the silent one is the usable binary. |
| `src/remote/attach.rs` | Adapted the other two probe callers: `remote_binary_supports_endpoint_requirement` and `live_handoff_remote_server`. | Behavior unchanged — both only needed "is there a usable status". |
| `src/remote/attach.rs` | Added `only_positive_discovery_verdicts_need_attention_from_the_user`. | Drives the real verdict producer and asserts the classifier's decision, so a reworded or re-kinded verdict cannot silently stop the retry loop again. |
| `tests/client_mode.rs` | Added `saved_machine_recovers_from_a_silent_ssh_stall`. | A link that dies without an EOF (ssh child alive, bridge socket bound) must still be detected and re-dialed. Passed before the fix too; it closes a real coverage gap. |
| `tests/client_mode.rs` | Added `saved_machine_reconnects_after_a_transient_remote_probe_failure`. | The regression test for this bug: red before the fix, green after. |

## Diff Highlights

The verdict split in `find_installed_remote_herdr`:

```rust
let missing = candidates.is_empty();
let mut outdated = false;
let mut unconfirmed = false;
for mut candidate in candidates {
    match remote_client_status(ssh, &candidate)? {
        RemoteClientProbe::Status(status) => {
            if status.supports_endpoint_requirement(&candidate.platform, true) {
                candidate.bridge_idle_timeout = status.remote_bridge_idle_timeout;
                return Ok(candidate);
            }
            outdated = true;
        }
        RemoteClientProbe::Unconfirmed => unconfirmed = true,
    }
}
Err(remote_herdr_unavailable(ssh.target(), missing, outdated, unconfirmed))
```

```rust
fn remote_herdr_unavailable(target: &str, missing: bool, outdated: bool, unconfirmed: bool) -> io::Error {
    if !unconfirmed && (missing || outdated) {
        return io::Error::new(io::ErrorKind::Unsupported, /* … install or update … */);
    }
    io::Error::new(
        io::ErrorKind::ConnectionAborted,
        format!("could not confirm a usable Herdr on {target}; reconnecting"),
    )
}
```

and the probe result it is based on:

```rust
enum RemoteClientProbe {
    Status(RemoteClientStatusJson),
    Unconfirmed,
}
```

## Tests Added or Updated

- `tests/client_mode.rs::saved_machine_reconnects_after_a_transient_remote_probe_failure` —
  asserts the client keeps dialing while the remote cannot confirm a usable Herdr, and reconnects
  on its own once it can. Fails before the fix with the machine stuck in `! attention`.
- `tests/client_mode.rs::saved_machine_recovers_from_a_silent_ssh_stall` — asserts a stalled link
  (no EOF) is detected and re-dialed without user action.
- `src/remote/attach.rs::only_positive_discovery_verdicts_need_attention_from_the_user` — calls
  `remote_herdr_unavailable` and asserts `saved_ssh_failure_needs_attention` on its output, pinning
  the producer-to-classifier contract rather than a hand-copied message.

## Local Verification

Environment: macOS x86_64, `ZIG=$HOME/sdk/zig-x86_64-macos-0.16.0/zig` (the repo requires Zig
0.16.0 for the vendored libghostty-vt; `just`/`cargo-nextest` are not installed on this machine, so
the underlying `cargo` commands were run directly).

| Command | Result |
| --- | --- |
| `cargo test --test client_mode saved_machine_reconnects_after_a_transient_remote_probe_failure` | **failed before the fix** (`! attention`, dial count frozen), passes after |
| `cargo test --test client_mode` | 28 passed, 0 failed |
| `cargo test --bin herdr only_positive_discovery_verdicts_need_attention` | 1 passed (new producer/classifier contract test) |
| `cargo test --bin herdr platform::remote_bridge_tests` | 4 passed |
| `cargo fmt --check` | clean |
| CI on the pull request (`just check` on macOS, Linux and Windows, plus Windows ConPTY packaging, conventional commits, and the fork review bot) | all green |
| `cargo clippy --all-targets --locked -- -D warnings` (local shell only) | fails on 8 pre-existing lints from this machine's Rust 1.98.1 clippy (`chunks_exact_to_as_chunks` at `src/app/api/pane_graphics.rs:492`, `src/ghostty/mod.rs:744,758`, `src/remote/attach.rs:3256,4256`, `src/integration/tests.rs:28`, plus `src/server/handoff.rs:393`, `src/terminal_theme.rs:138`). None are in the changed regions; this change adds no clippy findings, and CI runs the same lint green on its pinned toolchain |
| `python3 -m unittest` (maintenance contract tests) | 137 passed |
| `python3 -m unittest scripts.test_ui_hot_path_architecture` | 6 passed |
| `bun test scripts/release-workflows.test.ts` | 4 pass, 1 fail — **pre-existing and unrelated** (`release arguments are not interpolated into executable shell text`); this change touches no workflow or release script |

Note: the full parallel `cargo test --bin herdr` run aborts with a SIGPIPE inside
`platform::remote_bridge_tests::bridge_child`, a pre-existing pipe-handling flake under this
toolchain; that test file is untouched by this change and `platform::remote_bridge_tests` passes
4/4 in isolation.

## Deviations from Assessment

None. The assessment proposed the retryable/permanent split that was implemented.

## Follow-ups

- The same permanent treatment still applies to `"protocol"`, `"handshake"`, and
  `"unsupported remote platform"` needles in `saved_ssh_failure_needs_attention`. Those can be
  transient too (a remote mid-update, or a remote shell that emits startup output and garbles
  platform detection — upstream #3789). Retry classification for those was left alone to keep this
  fix narrow.
- The behavior change is deliberately limited to *inconclusive* probes. A remote with no Herdr
  installed, or with candidates that all answered as too old, still stops retrying and still tells
  the user to install or update — that is positive evidence, which retrying cannot change.
- A remote whose Herdr answers as too old *while* another candidate stays unprobed now keeps
  retrying instead of stopping. That is the point of the `unconfirmed` override, but it does mean a
  half-broken machine can re-dial on the ≤120s backoff indefinitely; worth a second opinion in
  review.
