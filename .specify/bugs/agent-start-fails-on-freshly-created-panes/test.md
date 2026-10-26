# Bug Test: agent start fails on freshly created panes

- Slug: agent-start-fails-on-freshly-created-panes
- Tested: 2026-10-26
- Result: verified (build + unit tests)

## Verification

- `cargo build` (offline, zig 0.16 cached) succeeded.
- `cargo test --bin herdr cli::agent` — 10 passed.
- `cargo test --bin herdr app::actions` — 105 passed.
- `cargo test --bin herdr terminal::state` — 123 passed.

Pre-existing failures (unrelated, environment): `platform::macos::launchd_spawn` (3 tests, control-character in plist value). Not caused by this change.

## PR

https://github.com/arrrrny/herdr/pull/87
