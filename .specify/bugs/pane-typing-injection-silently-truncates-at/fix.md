# Bug Fix: pane typing injection silently truncates at 1024 bytes

- Slug: pane-typing-injection-silently-truncates-at
- Fixed: 2026-08-23
- Assessment: ./assessment.md
- Status: applied

## Summary

The 1024-byte cap is **not** in Herdr's Rust code. Every userspace layer on the
`agent start` typing path passes the payload through whole:

- `src/cli/agent.rs` → `Method::AgentStart { args: Vec<String> }` — no cap.
- `src/app/api/agents.rs:145` `start_agent` → `interactive_shell_command` +
  `encode_api_submission` — no cap.
- `src/app/api_helpers.rs:25` `encode_api_text` — bracketed-paste wrap only.
- `src/api/server.rs` / `src/api/client.rs` — byte-at-a-time and
  `BufReader::read_line` framing; `MAX_INITIAL_REQUEST_BYTES` is 1 MB.
- `src/server/client_transport.rs:41` `MAX_INPUT_PAYLOAD` is 1 MB, and it
  *rejects loudly* rather than truncating.
- `src/pty/actor/unix.rs` `ACTOR_COMMAND_BUFFER = 1024` is the mpsc channel
  capacity **in messages**, not bytes.

The cap is the **kernel TTY line discipline**. `src/pty/actor/unix.rs:363` sets
the PTY master fd non-blocking, and `flush_pending_writes_once` then issued a
**single `write()` for the entire payload**. On BSD/Darwin, `sys/tty.h` defines
`TTYHOG = 1024`: `ttyinput()` **flushes the whole pending input queue** once one
write pushes it past that bound, instead of short-writing. So a >1024-byte
one-shot write to the master silently loses everything past the first 1024
bytes — exactly the reported symptom, exactly the reported byte count, and
macOS-only (Linux `N_TTY_BUF_SIZE` is 4096, which is why this never reproduced
on Linux).

This also explains the other reported properties: the CLI reports success
(userspace `write()` returned the full count), no error is surfaced (the discard
happens inside the line discipline), and re-running appends another truncated
copy rather than self-healing.

Fix: cap each individual `write()` to the master at `PTY_WRITE_CHUNK_BYTES = 512`,
safely under `TTYHOG`. This keeps the kernel's own `EWOULDBLOCK` backpressure
intact — the existing poll/`POLLOUT` loop then waits for the child to drain and
resumes from `current_write_offset`, so the full payload is delivered rather than
silently clipped. No new error path was needed: delivering the complete payload
is the preferred remediation named in the issue, and the infrastructure (a
resumable `pending_writes` queue with a write offset) already supported it.

## Changes

| File | Change | Notes |
| --- | --- | --- |
| `src/pty/actor/unix.rs:21-31` | Added `PTY_WRITE_CHUNK_BYTES: usize = 512` with a comment documenting the BSD/Darwin `TTYHOG` overflow-flush behaviour | New constant next to the existing actor constants |
| `src/pty/actor/unix.rs:731-733` | `flush_pending_writes_once` now slices the remaining payload to at most `PTY_WRITE_CHUNK_BYTES` per `write()` | Two-line change; partial-write/offset handling was already correct and is unchanged |
| `src/pty/actor/unix.rs:1462-1573` | Added `actor_runner_over_datagram_pair()` helper plus two regression tests | Test-only |

## Diff Highlights

```rust
// src/pty/actor/unix.rs
+// BSD/Darwin line discipline keeps at most `TTYHOG` (1024) bytes queued for the
+// child. `ttyinput` discards the whole pending input queue once a single write
+// pushes it past that bound, so a large one-shot write to the master silently
+// loses everything after the first 1024 bytes instead of short-writing. Feeding
+// the master in sub-TTYHOG slices keeps the kernel's own EWOULDBLOCK
+// backpressure intact: the poll loop then waits for write readiness (i.e. for
+// the child to drain) and resumes at `current_write_offset`, so long injected
+// input such as `agent start` command lines arrives complete.
+const PTY_WRITE_CHUNK_BYTES: usize = 512;

 fn flush_pending_writes_once(&mut self) {
     while let Some(bytes) = self.pending_writes.front() {
-        let chunk = &bytes[self.current_write_offset..];
+        let remaining = &bytes[self.current_write_offset..];
+        let chunk = &remaining[..remaining.len().min(PTY_WRITE_CHUNK_BYTES)];
         match self.file.write(chunk) {
```

## Tests Added or Updated

Both in `src/pty/actor/unix.rs` `#[cfg(test)] mod tests`:

1. `large_user_input_is_written_in_sub_ttyhog_chunks_without_truncation` — the
   direct regression guard for issue #3. Runs the real runner over a
   `UnixDatagram::pair()` so **every `write()` syscall is observable as one
   discrete message**, enqueues a 5000-byte payload, and asserts (a) each write
   is `<= PTY_WRITE_CHUNK_BYTES`, and (b) the reassembled bytes equal the payload
   exactly — i.e. no truncation at 1024. A `const _: () = assert!(...)` pins
   `PTY_WRITE_CHUNK_BYTES < TTYHOG` at compile time.
2. `chunked_user_input_resumes_after_backpressure` — fills the fd until
   `WouldBlock`, enqueues 4096 bytes, and asserts the payload stays queued (not
   dropped) and then resumes byte-exactly across repeated flush/drain cycles,
   ending with `current_write_offset == 0`. This pins the offset arithmetic that
   the chunking now exercises on every write.

Also added the `actor_runner_over_datagram_pair()` test helper, mirroring the
existing `actor_runner_for_unit_test()` style.

## Local Verification

```
export ZIG=/workspace/herdr/.cache/zig/zig-x86_64-linux-0.15.2/zig

cargo test --bin herdr pty::actor::unix
  -> test result: ok. 19 passed; 0 failed  (includes both new tests)

cargo test --bin herdr pty::
  -> test result: ok. 20 passed; 0 failed

cargo clippy --all-targets
  -> no errors (one redundant-`&&` error in the first draft of the test was fixed)

cargo fmt --check
  -> no diff in src/pty/actor/unix.rs
     (pre-existing diffs in src/integration/registry.rs and src/platform/linux.rs
      exist on `development` already and were left untouched)
```

Not verified: an end-to-end macOS reproduction. The environment is Linux, where
`N_TTY_BUF_SIZE = 4096` means the original bug does not reproduce. The root cause
is established from the code (non-blocking master + single unbounded `write()`)
plus the documented BSD/Darwin `TTYHOG` semantics; the fix is validated by
deterministic unit tests on the write-chunking behaviour rather than by observing
the macOS symptom disappear.

## Deviations from Assessment

The assessment's `Suspected Code Paths` and `Root Cause Hypothesis` were
`[NEEDS CLARIFICATION]`. The candidate leads supplied with the task
(`src/raw_input.rs`, `pane send-text` handlers, bracketed-paste buffering) were
all investigated and ruled out:

- `src/raw_input.rs:653` `scratch = [0u8; 1024]` is the **host stdin** read
  buffer, consumed in a `loop`, and is not on the injection path at all.
- `src/app/api/panes.rs:1501/1519` `handle_pane_send_text` /
  `handle_pane_send_input` pass their payload straight to `try_send_bytes`.
- No literal or computed 1024-byte cap exists anywhere on the typing path (the
  whole `src` tree was grepped for `1024`, `1_024`, `0x400`, and `MAX_*`
  byte-cap constants).

## Follow-ups

- **Windows**: `src/pty/actor/` Windows/ConPTY path was deliberately left alone —
  ConPTY has no `TTYHOG` equivalent. Confirm before assuming parity.
- **Chunk size**: 512 is a conservative half of `TTYHOG`. Throughput for very
  large pastes now costs more `write()` syscalls plus more `POLLOUT` round trips.
  Per `AGENTS.md`, the PTY write path is a multiplicative surface; if bulk-paste
  throughput regresses noticeably, consider applying the chunk cap only to
  `WriteUserInput` (leaving terminal responses unchunked), or raising the chunk
  toward 1023.
- **Verification on macOS**: worth a manual run of
  `herdr agent start ... -- <2 KB of args>` on macOS/zsh to confirm the full
  command now executes.
- **Loud-failure path**: the issue also asked for a visible error if delivery is
  incomplete. `flush_pending_writes_once` still `warn!`s and clears
  `pending_writes` on a hard write error without notifying the API caller. That
  gap is unchanged by this fix and could be surfaced separately.
