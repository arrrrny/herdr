# Test List: Ziki Agent-State Support

---
feature: 001-ziki-agent-state-support
loop: outside-in
profile: .specify/memory/tdd-profile.md
spec_criteria: 7
planned_at: 1f8db18
updated_at: 1f8db18
suite_baseline: red
---

Baseline red is pre-existing on master and unrelated to this feature (see
`.specify/memory/tdd-profile.md`): one nextest failure
(`agent_wait_exits_when_done_status_matches`) and six clippy lints. The loop
below runs against that baseline; every behavior must pass on top of it.

## Outer loop: acceptance behaviors

One per success criterion in `spec.md`. Each stays red until the feature works
through its real entry point (manifest engine, terminal-state ingestion, HTTP
listener over a real socket).

| id  | behavior                                                                                     | traces         | kind    | state   | test                                                                  |
| --- | -------------------------------------------------------------------------------------------- | -------------- | ------- | ------- | --------------------------------------------------------------------- |
| A1  | A foreground process named `ziki` classifies the pane as agent `ziki` (label + executable)    | SC-001, FR-001 | example | DONE    | `src/detect/mod.rs::identify_ziki_process` + `ziki_label_and_executable`                         |
| A2  | A `herdr:ziki` report sets the pane state and a lower-`seq` report cannot overwrite it        | SC-002, FR-004, FR-005 | example | DONE    | `src/app/api.rs::ziki_reports_drive_pane_state_through_the_api`     |
| A3  | Screen markers and OSC title classify blocked/working/idle without the push path              | SC-003, FR-002 | example | DONE    | `src/detect/manifest/tests.rs::ziki_manifest_*` (7 tests)                        |
| A4  | `POST /api/v1/pane/report/agent` over TCP ingests the §2 body; errors map to HTTP statuses     | SC-004, FR-003, FR-009, FR-010 | example | DONE    | `src/api/http_push.rs::http_push_*` (7 tests)                                    |
| A5  | A dead Ziki pane (process exit, no terminal push) leaves the stale hook state ineffective     | SC-005, FR-006 | example | DONE    | `src/app/api.rs::ziki_reports_drive_pane_state_through_the_api` (exit leg)     |
| A6  | fmt + clippy + nextest gates pass with the new tests (no new lint/test failures)              | SC-006         | example | DONE    | (gate run; see verification.md)                                        |
| A7  | Manifest maintenance + config-reference scripts pass with ziki.toml and the new config key    | SC-007         | example | DONE    | (gate run; see verification.md)                                        |

## Inner loop: unit behaviors

Grouped by the component from `plan.md` that owns them.

### `src/detect/mod.rs` (Agent classification)

| id  | behavior                                                              | traces             | kind    | state   | test                                                            |
| --- | --------------------------------------------------------------------- | ------------------ | ------- | ------- | --------------------------------------------------------------- |
| U1  | `identify_agent("ziki")` and case variants resolve to `Agent::Ziki`   | FR-001, SC-001     | example | DONE    | `src/detect/mod.rs::identify_ziki_process`                       |
| U2  | `agent_label(Agent::Ziki)` is `ziki`; executable table covers Ziki    | FR-001, SC-001     | example | DONE    | `src/detect/mod.rs::ziki_label_and_executable`                   |
| U3  | `full_lifecycle_hook_authority("herdr:ziki", "ziki")` is true; neighboring pairs stay false | FR-004, SC-002 | example | DONE    | `src/detect/mod.rs::ziki_is_full_lifecycle_hook_authority`       |
| U4  | Ziki is in `Agent::ALL` and `Agent::SCREEN_MANIFEST_AGENTS`           | FR-001, FR-002     | example | DONE    | `src/detect/mod.rs::ziki_in_agent_enumerations` / `ziki_in_screen_manifest_agents`                  |

### `src/detect/manifests/ziki.toml` (detection rules)

| id  | behavior                                                                     | traces         | kind    | state   | test                                                                |
| --- | ---------------------------------------------------------------------------- | -------------- | ------- | ------- | ------------------------------------------------------------------- |
| U5  | `[ziki-state: blocked] <msg>` in the bottom lines detects blocked + visible_blocker | FR-002, SC-003 | example | DONE    | `src/detect/manifest/tests.rs::ziki_manifest_screen_marker_blocked` |
| U6  | `[ziki-state: working]` detects working + visible_working                     | FR-002, SC-003 | example | DONE    | `src/detect/manifest/tests.rs::ziki_manifest_screen_marker_working` |
| U7  | `[ziki-state: idle]` at the bottom detects idle + visible_idle                | FR-002, SC-003 | example | DONE    | `src/detect/manifest/tests.rs::ziki_manifest_screen_marker_idle`   |
| U8  | OSC title `ziki:<state>` detects the matching state                          | FR-002, SC-003 | example | DONE    | `src/detect/manifest/tests.rs::ziki_manifest_osc_title_states`     |
| U9  | A current OSC title outranks a stale screen marker (OSC blocked beats old blocked marker; OSC working beats old blocked marker) | FR-002, SC-003 | example | DONE    | `src/detect/manifest/tests.rs::ziki_manifest_osc_outranks_screen` |
| U10 | A newer working marker below an older blocked marker detects working (marker ordering) | FR-002, SC-003 | example | DONE    | `src/detect/manifest/tests.rs::ziki_manifest_newest_marker_wins` |

### `src/terminal/state.rs` (push ingestion + death window)

| id   | behavior                                                                       | traces              | kind    | state   | test                                                                  |
| ---- | ------------------------------------------------------------------------------ | ------------------- | ------- | ------- | --------------------------------------------------------------------- |
| U11  | A `herdr:ziki` blocked report (with seq) sets effective state blocked           | FR-004, SC-002      | example | DONE    | `src/app/api.rs::ziki_reports_drive_pane_state_through_the_api` (via router anchor)                        |
| U12  | A `herdr:ziki` report with seq ≤ last accepted seq is ignored (stale guard)     | FR-005, SC-002      | example | DONE    | `src/app/api.rs::ziki_reports_drive_pane_state_through_the_api` (stale-seq leg)                 |
| U13  | An unsequenced `herdr:ziki` report is accepted in real time                     | FR-005, SC-002      | example | DONE    | covered by generic `report_lower_state_without_seq_lowers_effective_state` semantics (accept_hook_report unsequenced path); ziki-specific unsequenced variant exercised in `ziki_reports_drive` API shape              |
| U14  | After a `herdr:ziki` report, process exit releases the agent label (stale hook state stops winning) | FR-006, SC-005 | example | DONE    | `src/app/api.rs::ziki_reports_drive_pane_state_through_the_api` (exit leg) |

### `src/api/http_push.rs` (HTTP push listener)

| id   | behavior                                                                          | traces                    | kind    | state   | test                                                                  |
| ---- | --------------------------------------------------------------------------------- | ------------------------- | ------- | ------- | --------------------------------------------------------------------- |
| U15  | A valid §2 POST dispatches `pane.report_agent` with the exact body and returns success | FR-003, SC-004      | example | DONE    | `src/api/http_push.rs::http_push_accepts_contract_report`              |
| U16  | An unknown pane maps to the not-found error status (JSON-RPC error surfaced)      | FR-003, FR-010, SC-004    | example | DONE    | `src/api/http_push.rs::http_push_unknown_pane_not_found`               |
| U17  | A malformed JSON body returns a bad-request status                               | FR-010, SC-004            | example | DONE    | `src/api/http_push.rs::http_push_malformed_body_bad_request`           |
| U18  | An unknown path returns not-found; a non-POST method returns method-not-allowed   | FR-010, SC-004            | example | DONE    | `src/api/http_push.rs::http_push_routing_statuses`                     |
| U19  | The listener survives malformed requests and keeps serving                       | FR-009, SC-004            | example | DONE    | `src/api/http_push.rs::http_push_survives_bad_requests`                |
| U20  | Bind failure on a taken port returns an error without panicking (fail-soft contract) | FR-009       | example | DONE    | `src/api/http_push.rs::http_push_bind_conflict_is_reported`            |

### `src/config/model.rs` + docs (config surface)

| id   | behavior                                                              | traces         | kind    | state   | test                                                            |
| ---- | --------------------------------------------------------------------- | -------------- | ------- | ------- | --------------------------------------------------------------- |
| U21  | `[server] agent_push_listen_addr` defaults to 127.0.0.1:7878; "" parses as disabled; garbage is rejected/ignored per serde rules | FR-003, SC-007 | example | DONE    | `src/config/model.rs::server_agent_push_listen_addr_config` |

## Invariants and edge cases still to place

- All placed above (marker ordering U10, stale seq U12, fail-soft U20, death window U14).

## Out of scope

- Ziki publisher behavior (§1 env vars, §2 wire format from the client side): implemented and tested in the ziki repo (PR #13, 66 tests).
- Windows-target clippy (`just windows-lint`): requires the MSVC target not available in this sandbox; CI covers it.
- Live end-to-end run with a real `ziki` binary inside a Herdr pane: ziki is not installable here; covered by the contract-pinned unit/acceptance tests above (per AGENTS.md, manifest work uses focused Rust tests rather than full-screen fixture suites).
- Sidebar badge rendering tests: badges are the existing generic per-pane state indicators; no pane-type-specific rendering exists to test (evidence recorded in verification.md).
- cargo-mutants mutation run: tool not installed (profile records `null`); verification.md substitutes targeted manual mutants on the changed files.

## Verification commands

Copied verbatim from `.specify/memory/tdd-profile.md`:

- Single test: `cargo nextest run --locked -E 'test(<exact test name>)'`
- Full suite: `cargo nextest run --locked --no-fail-fast --status-level fail --final-status-level fail --failure-output final`
- Lint gate: `cargo fmt --check && cargo clippy --all-targets --locked -- -D warnings`
- Maintenance scripts: `python3 -m unittest scripts.test_agent_detection_manifest_check scripts.test_changelog scripts.test_config_reference_check scripts.test_docs_translation_parity scripts.test_hermes_integration_asset scripts.test_package_windows_conpty scripts.test_preview scripts.test_unix_installer scripts.test_vendor_libghostty_vt scripts.test_vendor_portable_pty`
