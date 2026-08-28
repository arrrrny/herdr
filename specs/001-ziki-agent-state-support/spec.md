# Feature Specification: Ziki Agent-State Support

**Feature Branch**: `035-ziki-agent-state-support`

**Created**: 2026-08-27

**Status**: Approved

**Input**: User description: "Herdr-side partner for the Ziki agent-state sync contract (ziki spec 011-herdr-ziki-state-sync, ziki PR #13): recognize Ziki panes (Agent::Ziki), ingest the state Ziki publishes via the HTTP push API (POST /api/v1/pane/report/agent, source herdr:ziki, stale-seq guard), fall back to Ziki's screen markers ([ziki-state: <state>]) and OSC title (ziki:<state>) when HERDR_PANE_ID is unset, resolve dead Ziki panes to unknown, surface working/blocked/idle via sidebar badges and herdr agent read/explain."

## User Scenarios & Testing *(mandatory)*

### User Story 1 - Ziki pane is classified as the Ziki agent (Priority: P1)

A user runs the `ziki` binary inside a Herdr pane. Herdr's foreground-process detection recognizes the process name `ziki` and classifies the pane as the Ziki agent (label `ziki`) instead of leaving it unknown. The label is visible through the query surface (`herdr agent read`, `herdr agent explain`) and the sidebar agent entry.

**Why this priority**: Every other behavior (push ingestion, badges, explain) depends on the pane being classified as a known agent; this is the foundation slice.

**Independent Test**: Run a process named `ziki` as the foreground process of a pane; `herdr agent explain <pane> --json` reports agent `ziki`; the process-name lookup (`identify_agent("ziki")`) resolves to the Ziki agent in unit tests.

**Acceptance Scenarios**:

1. **Given** a pane whose foreground process name is `ziki`, **When** Herdr scans the foreground job, **Then** the pane's detected agent label is `ziki`.
2. **Given** a report body with `"agent": "ziki"`, **When** it is ingested, **Then** the reported agent label normalizes to `ziki` (accepted, not rejected as invalid).

---

### User Story 2 - Ziki state pushes are ingested with authority and ordering (Priority: P1)

Ziki (launched with `HERDR_PANE_ID` exported and `HERDR_API_URL=http://localhost:7878`) pushes state transitions to Herdr over HTTP. Herdr accepts `POST /api/v1/pane/report/agent` with the contract §2 JSON body, treats `source: "herdr:ziki"` as full lifecycle-report authority (mirroring `herdr:kimi` / `herdr:kilo`), stores state and `seq`, and rejects stale (non-increasing `seq`) reports so an old report can never overwrite a newer state.

**Why this priority**: The push path is the authoritative state source; without it, state sync only works through screen scraping.

**Independent Test**: Start the HTTP push listener, POST a working report with `seq: 2`, then POST a blocked report with `seq: 1`; the pane keeps the state from `seq: 2`. A unit test drives the seq guard through the terminal-state ingestion path with source `herdr:ziki`.

**Acceptance Scenarios**:

1. **Given** the HTTP push listener is running on 127.0.0.1:7878, **When** a client POSTs a valid §2 body to `/api/v1/pane/report/agent`, **Then** the response is a success status and the pane's detected agent state becomes the reported state.
2. **Given** a pane whose source `herdr:ziki` last accepted `seq: 5` reported `blocked`, **When** a report with `seq: 4` and state `working` arrives, **Then** the pane's state remains `blocked`.
3. **Given** a report with an unknown pane id, **When** it is POSTed, **Then** the response reports the pane was not found (HTTP 404-class error), not a silent success.
4. **Given** a request to an unknown path or with an invalid method/body, **When** it is received, **Then** the listener answers with the matching client-error status and does not crash.

---

### User Story 3 - Screen-marker and OSC-title fallback detection (Priority: P1)

When `HERDR_PANE_ID` is unset, Ziki still emits its state on stdout: a plain-ASCII screen marker line (`[ziki-state: <state>]`, optionally followed by a message) on every state change, and an OSC title `\x1b]2;ziki:<state>\x07`. Herdr's bundled `ziki.toml` detection manifest matches both: the screen markers in `bottom_lines(N)` / `whole_recent` regions and the OSC title in the `osc_title` region, so detection works in screen/OSC-only mode.

**Why this priority**: The fallback keeps state sync alive when the push path is unavailable (no pane id exported, custom API URL, or push failure).

**Independent Test**: Unit tests feed synthetic screens: a bottom buffer containing `[ziki-state: blocked] criterion not satisfied after retries` detects blocked; an OSC title `ziki:working` detects working; a screen whose newest marker is `[ziki-state: idle]` detects idle.

**Acceptance Scenarios**:

1. **Given** a Ziki pane whose bottom lines contain `[ziki-state: blocked]`, **When** screen detection runs, **Then** the detected state is blocked with a visible blocker.
2. **Given** a Ziki pane whose terminal title is `ziki:working`, **When** OSC-title detection runs, **Then** the detected state is working.
3. **Given** a Ziki pane whose bottom lines end with `[ziki-state: idle]`, **When** screen detection runs, **Then** the detected state is idle.
4. **Given** a newer working marker followed by output lines while an older blocked marker scrolled out of the bottom window, **When** screen detection runs, **Then** the detected state is working.

---

### User Story 4 - Dead Ziki panes resolve to unknown (Priority: P2)

A Ziki pane that exits without a terminal idle push must not stay stuck on its last reported state. Once Herdr observes the process exit (foreground control returns to the shell), the hook authority for `herdr:ziki` stops being effective and the pane's effective state resolves away from the stale hook state (to unknown/screen fallback) after the existing short detection window.

**Why this priority**: Prevents permanently stale badges; it is a correctness guard rather than the primary path (Ziki pushes terminal idle on clean exit).

**Independent Test**: Unit tests: set hook authority via `herdr:ziki` report, then apply a process-exit state update; the effective agent label is released and the state no longer comes from the stale hook authority.

**Acceptance Scenarios**:

1. **Given** a Ziki pane whose state came from a `herdr:ziki` push, **When** the Ziki process exits and the shell returns, **Then** the pane's effective agent label is released and the stale hook state no longer wins.

---

### User Story 5 - Sidebar badges and query surface (Priority: P2)

Users see per-pane working/blocked/idle badges in the sidebar for Ziki panes derived from the detected state, and machine consumers (Forklift) can read the same state through `herdr agent read <pane> --source detection --format json` and `herdr agent explain <pane> --json`.

**Why this priority**: Presentation and read-only query surface ride on top of slices 1-3.

**Independent Test**: Sidebar rendering is driven by the pane's effective `AgentState` (existing per-state icons/labels); unit tests assert the ziki label round-trips through the agent query schema (state labels map for the ziki agent).

**Acceptance Scenarios**:

1. **Given** a Ziki pane detected as blocked (via push or screen), **When** the sidebar renders the pane, **Then** the blocked indicator (dot/color/label) is shown for that pane.
2. **Given** a Ziki pane with detected state, **When** `herdr agent explain <pane> --json` runs, **Then** the output names agent `ziki` and the current detected state.

---

### Edge Cases

- What happens when port 7878 is already taken by another process? The listener fails to bind, Herdr logs a warning and keeps running; Ziki's pushes fail best-effort and detection falls back to screen/OSC markers (contract: absent push path is never fatal).
- What happens when the HTTP body is not valid JSON or misses required fields? The listener answers with a 400-class client error and never panics.
- What happens when two different Ziki panes push concurrently? Each pane's `seq` sequence is keyed per source; pane identity comes from the body's `pane_id`, so reports are independent.
- What happens when a stale report has a *higher* seq than the pane has seen (e.g. agent restarted with a fresh seq counter)? Ziki's seq is strictly increasing per process lifetime; a restart resets the pane association, and the existing process-exit/re-anchor machinery handles sequence resets on agent restart.
- What happens when the marker string appears in unrelated output (e.g. a user `echo`s it)? Blocked/idle markers require the marker at the bottom of the buffer where Ziki actually emits it; incidental scrollback matches are bounded by the priority design and self-correct on the next scan.

## Requirements *(mandatory)*

### Functional Requirements

- **FR-001**: Herdr MUST recognize a foreground process named `ziki` as the Ziki agent (canonical label `ziki`), including case-insensitive lookup and the interactive executable name `ziki`.
- **FR-002**: Herdr MUST ship a bundled `ziki.toml` detection manifest whose rules match the contract §3 screen markers (`[ziki-state: idle|working|blocked]`, with optional trailing message) in `bottom_lines(N)` / `whole_recent` regions and the contract §4 OSC title (`ziki:<state>`) in the `osc_title` region.
- **FR-003**: Herdr MUST accept `POST ${HERDR_API_URL}/api/v1/pane/report/agent` (JSON body per contract §2: `pane_id`, `source`, `agent`, `state`, optional `message`/`seq`/`agent_session_id`/`agent_session_path`) on an HTTP listener bound to `127.0.0.1:7878` by default, configurable via `[server] agent_push_listen_addr` (empty string disables the listener).
- **FR-004**: Herdr MUST treat reports with `source` exactly `herdr:ziki` and agent `ziki` as full lifecycle-report authority (mirroring `herdr:kimi` / `herdr:kilo`), mapping `idle`/`working`/`blocked` onto the pane's detected agent state.
- **FR-005**: Herdr MUST ignore any `herdr:ziki` report whose `seq` is not strictly greater than the last accepted `seq` for that source on the pane (stale-report guard), and MUST accept unsequenced reports as real-time authoritative.
- **FR-006**: Herdr MUST resolve a Ziki pane that exits without a terminal idle push away from its stale hook state after the existing short detection window (process-exit release), never leaving a dead pane stuck on `working`/`blocked`.
- **FR-007**: Herdr MUST surface the detected Ziki state through the existing per-pane sidebar state indicators (working/blocked/idle) without pane-type-specific rendering.
- **FR-008**: Herdr MUST return the detected Ziki state and agent label through `herdr agent read <pane> --source detection --format json` and `herdr agent explain <pane> --json`.
- **FR-009**: The HTTP listener MUST fail soft: bind failure logs a warning and leaves the rest of Herdr running; request handling errors never panic the server.
- **FR-010**: The HTTP listener MUST answer unknown paths with a not-found status, non-POST methods on the report path with a method-not-allowed status, and malformed bodies with a bad-request status.

### Key Entities *(include if feature involves data)*

- **Ziki pane report**: the contract §2 push body (pane id, source `herdr:ziki`, agent `ziki`, state, optional message, monotonically increasing seq, optional session identity).
- **Agent (Ziki)**: a new member of Herdr's known-agent set with label `ziki`, executable `ziki`, and full lifecycle hook authority for source `herdr:ziki`.
- **Detection manifest (ziki.toml)**: rules for the ziki screen markers and OSC title, versioned and hot-reloadable like other bundled manifests.

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-001**: `identify_agent("ziki")` resolves to the Ziki agent and `agent_label` returns `ziki`; a manifest parse test proves the bundled `ziki.toml` loads and every SCREEN_MANIFEST_AGENTS member (including Ziki) has a bundled manifest (existing suite).
- **SC-002**: Driving the existing ingestion path (`pane report agent`) with source `herdr:ziki`/agent `ziki` changes the pane's effective state to the reported state; a follow-up report with a lower `seq` leaves the state unchanged (stale-seq guard proved by test).
- **SC-003**: Unit tests prove: bottom-buffer `[ziki-state: blocked] …` ⇒ blocked; `[ziki-state: working]` ⇒ working; `[ziki-state: idle]` ⇒ idle; OSC title `ziki:<state>` ⇒ matching state; OSC rules outrank stale screen markers.
- **SC-004**: A test drives the HTTP listener end-to-end over a real TCP socket: a valid POST returns success, a stale-seq POST does not change the pane state, an unknown pane returns not-found, and malformed requests return client-error statuses without killing the listener.
- **SC-005**: After a `herdr:ziki` state report, a process-exit detection update releases the agent label so the stale hook state stops winning (death ⇒ unknown resolution window).
- **SC-006**: `cargo fmt --check`, `cargo clippy --all-targets --locked -- -D warnings`, and `cargo nextest run --locked` all pass with the new tests included (no new warnings).
- **SC-007**: The manifest maintenance script (`scripts/agent_detection_manifest_check.py`) and the config-reference check both pass with the new manifest and config key.

## Assumptions

- Ziki's publisher side is already implemented and merged (ziki PR #13, 66 tests); Herdr only implements the consumer side of the pinned contract.
- The contract's `HERDR_API_URL` default (`http://localhost:7878`) means Herdr's HTTP push listener binds loopback `127.0.0.1:7878` by default; remote agents are out of scope.
- Ziki always emits both the screen marker and the OSC title on every state change, so the OSC rules can be prioritized as the current-state signal while screen-marker rules cover marker-only captures.
- Sidebar presentation already renders per-pane state generically; no new sidebar rendering work is required beyond the state flowing into the pane.
- No repo-side changeset tooling is used (this fork follows Conventional Commits without changesets); user-facing changes are noted in `docs/next/CHANGELOG.md`.
