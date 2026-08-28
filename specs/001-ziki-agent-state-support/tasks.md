# Tasks: Ziki Agent-State Support

**Input**: Design documents from `/specs/001-ziki-agent-state-support/` (spec.md, plan.md)

**Prerequisites**: plan.md (required), spec.md (required for user stories)

**Tests**: The TDD extension drives every behavioral task through the red-green-refactor loop (see `tdd/test-list.md`); test tasks below are mandatory, not optional.

**Organization**: Tasks are grouped by user story; MVP order is US1 → US2 → US3 → US4 → US5.

## Phase 1: Agent classification (US1 — foundation)

- [x] T001 Add `Ziki` variant to the `Agent` enum in `src/detect/mod.rs`: extend `Agent::ALL` (23 entries), `Agent::SCREEN_MANIFEST_AGENTS` (21 entries), `agent_label` → `"ziki"`, `interactive_agent_executable` → `"ziki"`, and `lookup_agent` with `"ziki"`.
- [x] T002 (test-first) Prove `identify_agent("ziki")`/`parse_agent_label("ziki")` resolve to the Ziki agent (case-insensitive), `agent_label(Agent::Ziki) == "ziki"`, and the executable table covers Ziki.

## Phase 2: Detection manifest (US3 — screen/OSC fallback)

- [x] T003 Create `src/detect/manifests/ziki.toml`: OSC-title rules (blocked 1200 / working 1100 / idle 1000, region `osc_title`, `contains = ["ziki:<state>"]`, visible_* flags) and screen-marker rules (blocked 500 / idle 450 in `bottom_lines(4)`, working 400 in `whole_recent`, `contains = ["[ziki-state: <state>]"]`), `aliases = ["herdr:ziki"]`, `min_engine_version = 1`.
- [x] T004 Register the manifest in `BUNDLED_MANIFESTS` (`src/detect/manifest.rs`) so the cache loads it for `Agent::Ziki`.
- [x] T005 (test-first) Add manifest behavior tests in `src/detect/manifest/tests.rs`: blocked marker with trailing message ⇒ blocked + visible_blocker; working marker ⇒ working; idle marker ⇒ idle; OSC titles `ziki:working`/`ziki:blocked`/`ziki:idle` ⇒ matching states; OSC outranks a stale screen marker; newer working marker after output ⇒ working (ordering proof).
- [x] T006 (test-first) Extend the maintenance-script expectations: `python3 scripts/agent_detection_manifest_check.py` passes with `ziki.toml` (no script change expected unless validation rejects a rule shape).

## Phase 3: Push ingestion (US2 — authority + HTTP bridge)

- [x] T007 Add `("herdr:ziki", "ziki")` to `full_lifecycle_hook_authority` in `src/detect/mod.rs` (mirrors `herdr:kimi`/`herdr:kilo`).
- [x] T008 (test-first) Prove ingestion semantics in `src/terminal/state.rs` tests: a `herdr:ziki` blocked report sets the effective state to blocked; a stale lower-`seq` report does not change the state; an unsequenced report is accepted in real time.
- [x] T009 Add `agent_push_listen_addr` to `ServerConfig` (`src/config/model.rs`) with default `"127.0.0.1:7878"` (empty string disables; `HERDR_API_LISTEN_ADDR` env override resolved at listener start).
- [x] T010 Create `src/api/http_push.rs`: `std::net::TcpListener` accept thread + per-connection handler; parse HTTP/1.1 request line, headers (Content-Length cap 1 MiB), body; route `POST /api/v1/pane/report/agent` → deserialize `PaneReportAgentParams` → forward `Request { method: Method::PaneReportAgent }` through `api_tx` (same channel contract as the socket server) → map the JSON-RPC response to HTTP statuses (200 / 404 pane_not_found / 400 invalid body+agent / 404 unknown path / 405 non-POST / 503 dispatch failure); drop/timeout guard on the response channel.
- [x] T011 Wire the listener into `start_server_inner` (`src/api/server.rs`) so every API-server start site (TUI monolithic, headless server, handoff restore) starts it; bind failure logs a `warn!` and continues; `ServerHandle` owns the thread lifecycle.
- [x] T012 (test-first) End-to-end listener tests over loopback TCP: valid POST returns success and the dispatched request carries the exact §2 body; unknown pane ⇒ 404-class error; malformed JSON ⇒ 400; wrong path ⇒ 404; wrong method ⇒ 405; concurrent requests keep the listener alive.

## Phase 4: Death window, badges, query surface (US4/US5)

- [x] T013 (test-first) Prove the death→unknown window in `src/terminal/state.rs` tests: after a `herdr:ziki` state report, a process-exit detection update releases the agent label and the stale hook state stops winning.
- [x] T014 (test-first) Prove the query surface: the ziki agent label normalizes through `normalize_reported_agent_label`, and `herdr agent explain --file <screen> --agent ziki` resolves the manifest (label parse round-trip; the JSON path is covered by existing generic schema tests).
- [x] T015 Verify sidebar badges need no new code (existing per-pane state indicators); record the evidence in `tdd/verification.md` rather than adding pane-type-specific tests.

## Phase 5: Docs, wiring, verification (non-behavioral)

- [x] T016 Add the `Some(Agent::Ziki) => AgentSoundSetting::Default` arm to `AgentSoundOverrides::for_agent` in `src/config/sound.rs` (exhaustive-match requirement; follows the Omp/Mastracode pattern — no new user-facing sound key).
- [x] T017 Document the new config key in `docs/next/website/src/data/config-reference.json` and add the user-facing entry to `docs/next/CHANGELOG.md`.
- [x] T018 Run `cargo fmt --check && cargo clippy --all-targets --locked -- -D warnings && cargo nextest run --locked` plus the maintenance script suite; flag any pre-existing unrelated failures (known: `scripts.test_hermes_integration_asset` fails on master).
- [x] T019 Commit spec-kit artifacts (spec.md, plan.md, tasks.md, tdd/test-list.md, tdd/verification.md) with the code per the repo's conventional-commit style (`feat: ...` + `refs #035`).

## Dependencies

- T001 blocks T002–T006 (enum must exist for labels/manifests) and T007–T014.
- T007 blocks T008/T013 (authority required before state tests mean anything).
- T009–T011 form the listener chain; T012 depends on T010–T011.
- T005 depends on T003–T004; T017 depends on T009; T018/T019 depend on everything.
