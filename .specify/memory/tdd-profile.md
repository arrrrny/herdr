# TDD Stack Profile — herdr

Detected by `/speckit.tdd.setup` on 2026-08-27 from the repository manifests
(`Cargo.toml`, `rust-toolchain.toml`, `justfile`, `.github/workflows/ci.yml`).
Every command below was executed in this repository before being recorded.

## Stack

- **Ecosystem**: Rust 1.96.1 (pinned by `rust-toolchain.toml`; CI pins the same).
- **Working directory**: repository root (`/home/z/my-project/herdr`).
- **Test runner**: cargo-nextest 0.9.143 (`cargo nextest`). Unit tests live next
  to the code in `#[cfg(test)] mod tests`; integration tests live in `tests/`.
- **Environment**: `ZIG=<path to zig 0.15.2>` must be set for `build.rs`
  (vendored libghostty-vt); obtained via the `ziglang` PyPI package or a Zig
  0.15.2 install. `source ~/.cargo/env` for cargo on PATH.

## Commands (verified)

- **Run one test by name**: `cargo nextest run --locked -E 'test(<exact test name>)'`
  (verified: `cargo nextest run --locked -E 'test(all_bundled_manifests_parse_and_validate)'`
  → 1 passed).
- **Full suite**: `cargo nextest run --locked --no-fail-fast --status-level fail --final-status-level fail --failure-output final`
  (CI's `just ci` uses `cargo nextest run --locked -E "<filter>"` after `just lint`).
- **Lint gate**: `cargo fmt --check && cargo clippy --all-targets --locked -- -D warnings`.
- **Maintenance scripts**: `python3 -m unittest scripts.test_agent_detection_manifest_check scripts.test_changelog scripts.test_config_reference_check scripts.test_docs_translation_parity scripts.test_hermes_integration_asset scripts.test_package_windows_conpty scripts.test_preview scripts.test_unix_installer scripts.test_vendor_libghostty_vt scripts.test_vendor_portable_pty`
  (run with cargo on PATH).

## Baseline at profile time

- Full suite: 3598 tests run: **3597 passed, 1 failed, 1 skipped**.
  Pre-existing failure (deterministic, reproduces on pristine master):
  `herdr::cli cases::agent_wait::agent_wait_exits_when_done_status_matches`
  (`agent_not_running` panic in `tests/cli/agent_wait.rs`).
- `cargo clippy --all-targets --locked -- -D warnings` fails on pristine master
  with 6 pre-existing mechanical lints (src/cli/agent.rs:953 unused import,
  src/cli/notification_click.rs:40 manual unwrap_or, src/cli/pane.rs:1259
  needless borrow, src/session.rs:810/904 useless format!, src/session.rs:943
  clone→from_ref). Recorded as baseline red; the feature must add zero new ones.
- Maintenance scripts: 98 tests, 1 pre-existing failure
  (`scripts.test_hermes_integration_asset` hermes asset vs test drift on master).

## Capabilities

- Run one test by name: **yes** (nextest `-E 'test(...)'`).
- Run whole suite: **yes** (nextest).
- Report failures usefully: **yes** (nextest failure output).
- Coverage: **no** (cargo-llvm-cov/tarpaulin not installed; not required).
- Mutation testing: **no** (cargo-mutants not installed — recorded as `null`;
  the TDD audit substitutes targeted manual mutants; see verification.md).
- Property-based testing: **no** (proptest/quickcheck not in Cargo.lock).

## Test utilities to reuse

- `AppState::test_new()` / `Workspace::test_new()` for state tests without PTYs.
- `TerminalState::new(TerminalId::alloc(), path)` for terminal-state ingestion tests.
- `crate::detect::manifest::tests` helpers (`explain`, `bundled_manifest`) for
  manifest behavior tests (`src/detect/manifest/tests.rs`).
- `crate::api::server` test pattern (`#[cfg(test)] handle_connection`) for
  request-dispatch tests; `std::net::TcpStream` for real HTTP socket tests.

## Constraints

- The suite spawns processes and binds local sockets; safe to run in this
  workspace. Tests that bind ports must use ephemeral ports (`127.0.0.1:0`).
- `just check` additionally cross-lints a Windows target (`windows-lint`) — not
  runnable in this Linux sandbox without the MSVC target; CI covers it.
