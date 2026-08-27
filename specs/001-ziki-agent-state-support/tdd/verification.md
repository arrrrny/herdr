# TDD Verification: Ziki Agent-State Support

**Feature**: 001-ziki-agent-state-support | **Verified**: 2026-08-27 | **Auditor**: /speckit.tdd.verify (executed by the session agent)

**Verdict: PASS** — every behavior on `tdd/test-list.md` is DONE, the suite is green on top of the recorded pre-existing baseline failures, and four targeted mutants on the changed files were killed by the new tests.

## 1. Test-first evidence (git history discipline)

The red-green-refactor loop ran cycle by cycle with evidence in `tdd/cycle-log.md`:

| Cycle | Behavior(s) | Red evidence | Green |
| ----- | ----------- | ------------ | ----- |
| 1 | U1–U4 (Agent::Ziki classification) | compile error: 8 × `E0599 no variant 'Ziki'` | enum + label/executable/lookup/authority tables |
| 2 | U5–U10 (ziki.toml manifest) | `ziki_manifest_bundled_and_parseable` + `ziki_in_screen_manifest_agents` failed (no manifest) | manifest created + registered + SCREEN_MANIFEST_AGENTS |
| 3 | U11–U14, A2, A5 (ingestion + death window) | `left: Unknown, right: Working` — first herdr:ziki push ignored (no session anchor) | `is_official_agent_source` + router anchor for session-carrying state reports |
| 4 | U15–U21, A4 (HTTP listener + config) | config test `E0609 no field agent_push_listen_addr`; listener tests red before module existed | `src/api/http_push.rs` + ServerConfig key + server wiring |
| 5 | Query surface (A1/A3 evidence) | test added after manifest (label path trivially follows parse); covered by `ziki_query_surface_explains_from_agent_label` | green |

Red evidence for each cycle is recorded verbatim in `tdd/cycle-log.md` (commands, failure
output, what turned it green, refactor notes).

## 2. Test-smell rubric

- **One behavior per test**: each test asserts one observable outcome (classification,
  marker ⇒ state, status mapping, seq guard); multi-step flow tests
  (`ziki_reports_drive_pane_state_through_the_api`) name each leg in comments.
- **No test after the fact without red**: the manifest tests and listener tests were
  written before their implementation (cycles 2 and 4); the config test failed to
  compile before the field existed.
- **No assertion-free tests**: every test asserts state, flags, rule ids, or HTTP statuses.
- **No shared mutable fixtures**: each test builds its own terminal/app/listener; the
  HTTP tests bind ephemeral ports (`127.0.0.1:0`) and use a per-test fake app channel.
- **Deterministic**: no sleeps on the critical path (channel recv timeouts are generous
  upper bounds, not synchronization); no wall-clock dependence.
- **Honest doubles**: the HTTP tests use a real TCP socket and the real dispatch channel;
  the only double is the app-side responder, which is the unit boundary under test.

## 3. Mutation evidence (cargo-mutants unavailable — manual mutants)

The stack profile records cargo-mutants as not installed (`null`), so four targeted
manual mutants were applied to the changed files, run against the focused filter, and
reverted (each revert verified by re-running the suite):

| Mutant | Change | Killed by |
| ------ | ------ | --------- |
| A | Removed `("herdr:ziki", "ziki")` from `full_lifecycle_hook_authority` | `detect::tests::ziki_is_full_lifecycle_hook_authority` (FAILED) |
| B | Disabled the router's ziki session-anchor branch (`if false && …`) | `app::api::tests::ziki_reports_drive_pane_state_through_the_api` (first push ignored ⇒ `Unknown != Working`) |
| C | `ziki.toml` `screen_marker_blocked` state changed to `idle` | `detect::manifest::tests::ziki_manifest_screen_marker_blocked` (FAILED) |
| D | HTTP status mapping: `pane_not_found` → 200 | `api::http_push::tests::http_push_unknown_pane_not_found` (FAILED) |

All four mutants were killed. After the final revert, all 21 `ziki_*`/`http_push_*`
tests pass.

## 4. Acceptance-criteria coverage

| Criterion | Evidence |
| --------- | -------- |
| SC-001 (ziki classified) | `identify_ziki_process`, `ziki_label_and_executable`, `ziki_in_agent_enumerations`, `all_bundled_manifests_parse_and_validate` (suite) |
| SC-002 (push authority + stale-seq guard) | `ziki_reports_drive_pane_state_through_the_api`: working(1) drives state, blocked(2) transitions, stale(1) ignored |
| SC-003 (markers + OSC classify) | `ziki_manifest_screen_marker_{blocked,working,idle}`, `ziki_manifest_osc_title_states`, `ziki_manifest_osc_outranks_screen`, `ziki_manifest_newest_marker_wins` |
| SC-004 (HTTP endpoint end-to-end) | `http_push_accepts_contract_report` (real TCP, exact §2 body), `http_push_unknown_pane_not_found` (404), `http_push_malformed_body_bad_request` (400), `http_push_routing_statuses` (404/405), `http_push_survives_bad_requests`, `http_push_bind_conflict_is_reported` |
| SC-005 (death ⇒ unknown window) | exit leg of `ziki_reports_drive_pane_state_through_the_api`: stale blocked state no longer wins, agent label released |
| SC-006 (fmt + clippy + nextest) | `cargo fmt --check` clean; `cargo clippy --all-targets --locked -- -D warnings` clean (the 6 pre-existing master lints were fixed in a separate `chore` commit so the gate can run); `cargo nextest run --locked --no-fail-fast` ⇒ 3620 run: 3619 passed, 1 failed, 1 skipped — the single failure is the pre-existing `cases::agent_wait::agent_wait_exits_when_done_status_matches`, reproduced deterministically on pristine master before any change |
| SC-007 (maintenance + config-reference scripts) | `agent_detection_manifest_check.py` (default and `--require-website`) ok; `config_reference_check.py` exit 0; `test_changelog`/`test_docs_translation_parity`/`test_config_reference_check`/`test_agent_detection_manifest_check` all OK. `scripts.test_hermes_integration_asset` fails — pre-existing on master, unrelated |

## 5. Out-of-scope notes (recorded, not dropped silently)

- Sidebar badges (FR-007/T015): verified by inspection — `state_icon_symbol`/`state_label`
  in `src/ui/status.rs` render per-pane state generically (blocked/working/idle dots,
  symbols, labels, colors) from the pane's effective `AgentState`; the ziki state flows
  into that path through the generic terminal state, so no pane-type-specific test exists
  by design (per AGENTS.md: no agent-specific full-screen fixture suites).
- Live end-to-end run with a real `ziki` binary is not possible in this sandbox (ziki is
  not installable here); the contract-pinned surfaces (§2 body shape, §3 markers, §4 OSC)
  are covered by the tests above.
- Windows-target lint (`just windows-lint`) requires the MSVC target unavailable in this
  environment; CI covers it. The changed code uses `std::net` and has no OS-specific cfg.

## 6. Baseline honesty

Pre-existing failures on pristine master (verified before any feature change, recorded in
`.specify/memory/tdd-profile.md`): the `agent_wait_exits_when_done_status_matches` nextest
failure, six clippy lints (fixed here in a dedicated chore commit), and the hermes
integration-asset script test. No feature test depends on these; no new failure was
introduced.
