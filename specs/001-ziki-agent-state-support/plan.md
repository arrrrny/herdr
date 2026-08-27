# Implementation Plan: Ziki Agent-State Support

**Branch**: `035-ziki-agent-state-support` | **Date**: 2026-08-27 | **Spec**: [spec.md](spec.md)

**Input**: Feature specification from `/specs/001-ziki-agent-state-support/spec.md`

## Summary

Herdr learns the Ziki agent end-to-end so Forklift v-next can coordinate Ziki goal-panes through Herdr panes: a new `Agent::Ziki` variant classifies `ziki` foreground processes; a bundled `ziki.toml` detection manifest matches Ziki's contract §3 screen markers (`[ziki-state: <state>]`) and §4 OSC title (`ziki:<state>`) so detection works without the push path; a small loopback HTTP listener accepts the contract §2 push (`POST /api/v1/pane/report/agent`) and bridges it into the existing JSON-RPC `pane.report_agent` ingestion (full lifecycle authority for `herdr:ziki`, per-source stale-`seq` guard); and the existing generic release/staleness machinery resolves dead Ziki panes away from stale hook state. Sidebar badges and `herdr agent read/explain` ride on the detected state with no pane-type-specific code.

## Technical Context

**Language/Version**: Rust 1.96.1 (rust-toolchain.toml pinned), edition 2021.

**Primary Dependencies**: Cargo (locked), tokio (rt, sync, mpsc — already a dependency), serde/serde_json, toml; agent-detection manifests (`src/detect/manifests/*.toml`) with engine version 3; pane-report push API implemented as a minimal hand-rolled HTTP/1.1 listener on `std::net::TcpListener` bridging into the existing local-socket JSON-RPC server (`pane.report_agent`), consistent with the repo's dependency-conservative style (no axum/hyper added); OSC-title + screen-marker detection via the existing manifest engine regions (`osc_title`, `bottom_lines(N)`, `whole_recent`).

**Storage**: N/A (in-memory pane/terminal state; manifest is a bundled TOML asset compiled in via `include_str!`).

**Testing**: cargo-nextest (unit tests live next to the code in `#[cfg(test)] mod tests`); maintenance script tests (`python3 -m unittest scripts.test_*`); `just check` gates CI. HTTP listener tested over a real loopback TCP socket with `std::net::TcpStream` clients.

**Target Platform**: Linux/macOS/Windows (TUI desktop app + headless server; the HTTP listener uses `std::net`, portable across all three).

**Project Type**: terminal workspace manager for AI coding agents (ratatui TUI + local-socket JSON-RPC server + headless session server).

**Performance Goals**: The HTTP push path is cold (one request per Ziki state change, tiny JSON bodies) — no measurable render-loop impact; the listener thread is parked in `accept()` and each connection is handled on its own short-lived thread, mirroring the existing socket server pattern. Detection adds one manifest to the hot-reload cache (rule count stays minimal).

**Constraints**: `cargo fmt --check`, `cargo clippy --all-targets --locked -- -D warnings`, `cargo nextest run --locked` must pass. No new third-party crates (vendored/portable-pty patch discipline and the repo's conservative dependency policy). Bind failure of the HTTP listener must never abort Herdr startup (Ziki degrades to screen/OSC-only mode per contract).

**Scale/Scope**: One agent enum variant + one manifest (7 rules), one config key, one new `src/api/http_push.rs` module (~300 lines with tests), a one-line authority addition, docs/changelog entries, and ~15 focused tests.

## Constitution Check

*GATE: Must pass before Phase 0 research. Re-check after Phase 1 design.*

- **State is separated from runtime**: the push report flows through `AppEvent::HookStateReported` into `TerminalState` exactly like kimi/kilo reports; the HTTP listener is a transport, not state. ✅
- **Detection is decoupled / evidence-based**: ziki.toml encodes the contract's invariant markers (own-line marker at the bottom buffer, OSC title) with explicit priorities; no whole-pane incidental matching. ✅
- **Runtime/client boundary guardrail**: the push endpoint is server/runtime state exposed through the neutral JSON-RPC ingestion path (not TUI-only); no UI-surface names in server code. ✅
- **Platform code is isolated**: the listener uses `std::net` with no `#[cfg(target_os)]`. ✅
- **UI patterns are reused**: sidebar badges use the existing per-pane state indicators; no new rendering. ✅
- **Multiplicative paths**: no new work inside render/parse/resize loops; the listener thread blocks on `accept`. ✅

## Project Structure

### Documentation (this feature)

```text
specs/001-ziki-agent-state-support/
├── plan.md              # This file
├── tasks.md             # Phase 2 output (/speckit-tasks command)
├── checklists/
│   └── requirements.md  # Spec quality checklist
└── tdd/
    ├── test-list.md     # TDD extension output
    └── verification.md  # TDD extension output
```

### Source Code (repository root)

```text
src/
├── detect/
│   ├── mod.rs                    # Agent enum + lookup + authority tables (Ziki variant)
│   ├── manifest.rs               # BUNDLED_MANIFESTS list (+ ziki.toml)
│   ├── manifest/tests.rs         # Manifest behavior tests
│   └── manifests/
│       └── ziki.toml             # NEW: §3 marker + §4 OSC rules
├── api/
│   ├── mod.rs                    # HTTP_PUSH_LISTEN_ADDR env var + re-exports
│   ├── server.rs                 # Start HTTP push listener beside the JSON-RPC socket server
│   └── http_push.rs              # NEW: loopback HTTP listener → pane.report_agent bridge
├── config/
│   └── model.rs                  # [server] agent_push_listen_addr (default 127.0.0.1:7878)
├── terminal/
│   └── state.rs                  # (no change expected — herdr:ziki rides existing machinery)
└── ...

docs/next/
├── CHANGELOG.md                  # User-facing entry
└── website/src/data/config-reference.json   # Documented config key (checked by script)
```

## Phase 0/1: Research & Design Decisions

**D1 — HTTP listener instead of a full web framework.** Ziki's publisher (ziki `src/agent/herdr.zig`) POSTs real HTTP/1.1 with `Content-Type: application/json` and a `Content-Length` body via `std.http.Client`. Herdr currently has no HTTP server (local-socket JSON-RPC only). A minimal `std::net::TcpListener` loopback server that parses the request line + headers + body and bridges to the existing `handle_request` dispatch adds the contract endpoint without pulling axum/hyper/tower into a dependency-conservative repo. Alternative (rejected): reusing the local socket from Ziki — the contract pins HTTP and Ziki is already implemented/tested against it.

**D2 — Bridge into `pane.report_agent`, not a parallel ingestion path.** The JSON-RPC method `pane.report_agent` already: normalizes the agent label, resolves the pane, emits `AppEvent::HookStateReported`, applies the per-source stale-`seq` guard (`accept_hook_report`), and returns structured errors (pane_not_found, invalid_agent). The HTTP layer parses the body into `PaneReportAgentParams`, forwards one `Request` through `api_tx`, and maps the JSON-RPC response to HTTP statuses (200 success, 404 pane_not_found, 400 invalid body/agent, 503 dispatch failure). One seam, all ingestion semantics reused and already battle-tested by kimi/kilo.

**D3 — `full_lifecycle_hook_authority` gains `("herdr:ziki", "ziki")`.** This makes `herdr:ziki` reports authoritative like `herdr:kimi`/`herdr:kilo`, which also engages the existing full-lifecycle routing (session anchoring, process-exit suppression, staleness threshold) that implements the death→unknown window.

**D4 — Manifest rule design (evidence-based ordering).** Ziki emits each marker on its own line at publish time; the newest marker sits at the bottom of the bottom buffer. OSC title is always current (no accumulation), so OSC rules take the highest priorities (1200/1100/1000 blocked/working/idle). Screen-marker rules use `bottom_lines(4)` for blocked (500) and idle (450) — Ziki prints nothing after a blocked/idle marker while it waits/finishes — and `whole_recent` for working (400), because tool output follows the working marker. Higher-priority blocked/idle rules beat an older working marker in the window; a stale blocked/idle marker leaves the 4-line window within a few output lines. No `not` gates: they suppress correct working detections more often than they protect.

**D5 — Default bind 127.0.0.1:7878, fail-soft.** The contract default (`HERDR_API_URL=http://localhost:7878`) implies Herdr listens there out of the box. `[server] agent_push_listen_addr` overrides (any `host:port`); `""` disables; `HERDR_API_LISTEN_ADDR` env var overrides for tests/remapping, mirroring the `HERDR_SOCKET_PATH` pattern. Bind failure logs a `warn!` and Herdr continues (Ziki falls back to §3/§4).

**D6 — Query surface and badges need no new code.** `herdr agent read/explain` and the sidebar consume the pane's effective agent label/state; FR-008/SC-006 are proven by tests that the ziki label parses/normalizes/round-trips and by existing generic rendering.

## Rollout / Migration

Single PR, all changes additive: new enum variant, new manifest, new module, new config key with a default that preserves the contract. No migration; existing users only see a new loopback listener (disable via `[server] agent_push_listen_addr = ""`). Docs: `docs/next/CHANGELOG.md` entry + config-reference row; unreleased docs stay under `docs/next/` per AGENTS.md.
