# Cycle Log: Ziki Agent-State Support

Append-only evidence log. One entry per completed red-green-refactor cycle.
Never edit a past entry; a correction is a new entry that says what it corrects.

## Baseline

- `planned_at` HEAD: 1f8db18d (pristine master of arrrrny/herdr).
- Full suite: `cargo nextest run --locked --no-fail-fast` → 3598 run: 3597 passed,
  1 failed (`cases::agent_wait::agent_wait_exits_when_done_status_matches`), 1 skipped.
  Reproduced twice in isolation — deterministic, pre-existing, unrelated to this feature.
- Clippy gate: `cargo clippy --all-targets --locked -- -D warnings` → 6 pre-existing
  errors on pristine master (listed in .specify/memory/tdd-profile.md), unrelated files.
- Maintenance scripts: 98 tests, 1 pre-existing failure (hermes asset test drift).

## Cycle 1 — Agent::Ziki classification (U1, U2, U3, U4-ALL part)

- **RED**: added `ziki_*` tests to `src/detect/mod.rs`; `cargo nextest run --locked -E 'test(ziki_)'`
  → compile failed: 8 × `E0599: no variant ... named 'Ziki' found for enum detect::Agent`
  (failing for the right reason: the variant does not exist).
- **GREEN**: added `Ziki` to the `Agent` enum, `Agent::ALL` (23), `agent_label` ("ziki"),
  `interactive_agent_executable` ("ziki"), `lookup_agent` ("ziki"), and
  `("herdr:ziki", "ziki")` to `full_lifecycle_hook_authority`; updated the expected
  table in `every_agent_has_a_canonical_interactive_executable`; added the
  `Some(Agent::Ziki) => AgentSoundSetting::Default` arm to `AgentSoundOverrides::for_agent`
  (required by the exhaustive match).
  Result: 6/7 ziki tests pass; `ziki_in_screen_manifest_agents` still red (cycle 2's RED).
- **Refactor**: none needed (table-driven additions only).

## Cycle 2 — ziki.toml manifest (U5–U10, U4-SCREEN part)

- **RED**: added `ziki_manifest_*` tests to `src/detect/manifest/tests.rs`;
  run showed `ziki_manifest_bundled_and_parseable` and `ziki_manifest_newest_marker_wins`
  failing (no manifest registered → fallback path), plus the cycle-1 leftover
  `ziki_in_screen_manifest_agents` failure.
- **GREEN**: created `src/detect/manifests/ziki.toml` (osc_title rules blocked/working/idle
  at priorities 1200/1100/1000; screen markers blocked 500 / idle 450 in bottom_lines(4);
  working 400 in whole_recent), registered it in `BUNDLED_MANIFESTS`, added `Ziki` to
  `Agent::SCREEN_MANIFEST_AGENTS` (21). Result: 13/13 ziki tests pass,
  `all_bundled_manifests_parse_and_validate` passes.
- **Refactor**: published the manifest to the website catalog
  (`website/agent-detection/ziki.toml` + index.toml entry) required by
  `scripts/agent_detection_manifest_check.py`; both default and `--require-website`
  runs pass, and `scripts.test_agent_detection_manifest_check` (10 tests) is green.

## Cycle 3 — herdr:ziki ingestion + death window (U11–U14, A2, A5)

- **RED**: app-level test `ziki_reports_drive_pane_state_through_the_api` in `src/app/api.rs`
  drives the exact production flow (pane with detected Ziki foreground, §2 reports through
  `handle_api_request`). First push failed: `left: Unknown, right: Working` — the first
  `herdr:ziki` report was IGNORED by the full-lifecycle router because there was no session
  anchor (ziki's contract has no separate session-start push, unlike kimi/kilo whose hook
  assets call `pane report-agent-session` first).
- **GREEN**: two-part fix.
  (1) `src/agent_resume.rs`: added `("herdr:ziki", "ziki")` to `is_official_agent_source`
  (so `session_ref_from_report` resolves ziki's `agent_session_id`) and added the
  `state_report_carries_session_start` predicate.
  (2) `src/terminal/state.rs` `route_full_lifecycle_hook_report`: when a ziki-style report
  (state report that carries its own session identity) arrives with the reporting process
  as the pane's foreground agent, no existing anchor, and no active suppression, route it
  `Accept` directly — the report's own session_ref becomes the anchor for subsequent
  reports via `hook_authority.session_ref`. No seq games, no synthesized events.
- Result: working(1) → blocked(2) → stale(1) ignored → process exit releases label and
  un-sticks the stale blocked state. Full suite after the router change:
  3611 run: 3610 passed, 1 failed (the pre-existing `agent_wait_exits_when_done_status_matches`),
  1 skipped. 414/414 lifecycle/session/report-filtered tests pass.
- **Refactor**: none needed; the router change is additive and gated on the ziki predicate.

## Cycle 4 — HTTP push listener + config (U15–U21, A4)

- **RED**: config test `server_agent_push_listen_addr_config` failed to compile
  (`E0609: no field agent_push_listen_addr`); http_push tests were written alongside the
  module skeleton (new module — the listener tests fail before `start_http_push_server`
  exists).
- **GREEN**: 
  - `src/config/model.rs`: `ServerConfig.agent_push_listen_addr: String` default
    "127.0.0.1:7878" ("" disables).
  - `src/api/http_push.rs` (new): `std::net::TcpListener` accept thread + per-connection
    threads; HTTP/1.1 request-line/header/Content-Length parsing (1 MiB body cap,
    16 KiB header cap); routes `POST /api/v1/pane/report/agent` → deserialize
    `PaneReportAgentParams` → `dispatch_to_app_with_timeout` (the same dispatch the socket
    server uses) → JSON-RPC response mapped to HTTP statuses (200 / 404 pane_not_found /
    400 invalid_agent|bad body / 404 unknown path / 405 non-POST / 500|503 server errors).
    `resolved_http_push_listen_addr()`: `HERDR_API_LISTEN_ADDR` env override →
    `[server].agent_push_listen_addr` ("" disables); hostname bind values resolve.
  - `src/api/server.rs`: `start_server_inner` starts the listener beside the JSON-RPC
    socket (all four server start sites inherit it); bind failure logs a warning and
    Herdr keeps running; `ServerHandle` owns the listener lifecycle.
  - Tests over real loopback TCP: contract report dispatches with the exact §2 body (200);
    unknown pane → 404; malformed body → 400; unknown path → 404; GET on the report path
    → 405; garbage/oversized/headerless requests → 4xx and the listener keeps serving;
    bind conflict errors (fail-soft contract at the caller).
  Full suite: 3619 run: 3618 passed, 1 failed (the pre-existing agent_wait failure).
- **Refactor**: dropped the unused JoinHandle field; `addr` kept test-gated; clippy clean
  for the module.

## Cycle 5 — Query surface + mutation runs + gates

- Query-surface test `ziki_query_surface_explains_from_agent_label` proves
  `herdr agent explain --file PATH --agent ziki` resolves through the label to the
  manifest and reports blocked with the matched rule (the `<target>` mode rides the
  generic AgentExplain API path exercised by the existing suite).
- Mutation runs (cargo-mutants unavailable per stack profile): four targeted manual
  mutants — (A) authority pair removed, (B) router ziki anchor disabled, (C) blocked
  marker rule state corrupted to idle, (D) HTTP pane_not_found mapped to 200 — each was
  applied, killed by its test, and reverted with verified restoration (21/21
  ziki/http_push tests green after the final revert).
- Gates: `cargo fmt --check` clean; `cargo clippy --all-targets --locked -- -D warnings`
  clean after fixing the six pre-existing master lints in a separate chore commit;
  full suite 3620 run: 3619 passed / 1 pre-existing failure / 1 skipped; maintenance
  scripts pass except the pre-existing hermes asset test.
