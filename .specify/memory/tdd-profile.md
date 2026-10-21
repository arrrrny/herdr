# Stack Profile

Detected: 2026-10-21
Engine: raw (non-zuraffa) — `zfa` is not installed and this repository has no
`.zfa.json`.

This is a Rust project. The test stack is `cargo-nextest` for the Rust
suites, plus Python `unittest` for the repository maintenance contract tests
and `bun test` for the workflow TypeScript tests. There is no separate
"build" step: `cargo check --locked --all-targets` is the build.

## Commands (proven)

- single_test: `cargo nextest run --locked -E 'test(<name>)' --status-level fail --final-status-level fail --failure-output final --success-output never`
- whole_file: `cargo nextest run --locked <file> --status-level fail --final-status-level fail --failure-output final --success-output never`
- full_suite: `cargo nextest run --locked --status-level fail --final-status-level fail --failure-output final --success-output never`
- build: `export ZIG=$HOME/.local/share/herdr/zig-0.16.0/zig && cargo check --locked --all-targets`
- maintenance_tests: `python3 -m unittest scripts.test_agent_detection_manifest_check scripts.test_changelog scripts.test_config_reference_check scripts.test_docs_translation_parity scripts.test_fork_owned_guard scripts.test_generated_snapshot_guard scripts.test_hermes_integration_asset scripts.test_package_windows_conpty scripts.test_preview scripts.test_release scripts.test_sync_resolve_generated scripts.test_unix_installer scripts.test_vendor_libghostty_vt scripts.test_vendor_portable_pty scripts.test_windows_cross scripts.test_windows_input`
- workflow_tests: `bun test scripts/release-workflows.test.ts scripts/sync-upstream-workflow.test.ts`
- loop: `cargo nextest run --locked -E 'test(<name>)' --status-level fail --final-status-level fail --failure-output final --success-output never`
- verify: `just check`

`just test` and `just check` are the repository's own entry points; they run
the Rust suite, the maintenance tests, the UI hot-path architecture test, the
integration-asset test, and the docs contract test in that order.

## Notes

- **The vendored `libghostty-vt` build needs Zig 0.16.0.** The `zig` on `PATH`
  is 0.15.2 and fails with `Your Zig version v0.15.2 does not meet the
  required build version of v0.16.0`. Every cargo command in this repository
  must run with `ZIG=$HOME/.local/share/herdr/zig-0.16.0/zig` exported.
- **No mutation or coverage tooling is installed.** `cargo mutants`,
  `cargo llvm-cov`, `cargo tarpaulin` and `cargo proptest` are all absent
  from this machine. The TDD verify step therefore uses *deliberate* mutants
  — a test is proven meaningful by hand-editing the production code to break
  the behavior, confirming the test goes red, and restoring the code. No
  automated mutation score is claimed.
- **One known environmental flake.** `tests/live_handoff.rs::live_handoff_preserves_keyboard_protocol_for_client_input`
  fails on this machine with `failed to openpty: Device not configured`. It
  is a PTY-availability artifact of the host, not a product defect, and it is
  unrelated to any feature work. The baseline records it.
- **No separate constitution file.** This repository keeps its engineering
  rules in `AGENTS.md` at the repository root, which carries the testing
  rules ("Use `just` recipes by default", "Run `just check` before
  committing") and the rule that new behavior must be testable without PTYs
  where possible. The TDD principle is recorded in `AGENTS.md` rather than in
  a `constitution.md`, because introducing a second rules file would split
  the project's guidance in two.
