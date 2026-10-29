# Feature Specification: local machine label

**Feature Branch**: `feat/issue-90`

**Created**: 2026-10-29

**Status**: Implemented

**Input**: User description: "The sidebar shows a fixed, built-in label `Local` for the local machine. With several saved machines, every machine in the sidebar has a meaningful name except the local one, and there is no supported way to rename it. Any of the following would fix it: a config key, `herdr machine rename local --label i9`, or a UI rename action on the machine group heading. Source: https://github.com/arrrrny/herdr/issues/90"

## User Scenarios & Testing *(mandatory)*

### User Story 1 - Name the local machine like the rest of the fleet (Priority: P1)

An operator runs several machines in one window and has given every saved
machine a meaningful name. The machine they are sitting at is the only one
still called `Local`, so they cannot tell at a glance that it is the host they
named, for example, `i9`. They set one value in the machine's configuration
and the sidebar shows that name everywhere the local machine is presented.

**Why this priority**: This is the whole request. Every other surface in the
window already uses a user-chosen name, so the unnamed local machine breaks the
one mental model the sidebar exists to support: machine first, work second.

**Independent Test**: Set the local machine name in the configuration, start
the client, and confirm the machine row for the local machine shows the chosen
name instead of `Local`, while saved machines keep their own names.

**Acceptance Scenarios**:

1. **Given** a configuration that names the local machine `i9`, **When** the
   client renders the sidebar, **Then** the local machine row is labelled `i9`
   and saved machines keep their configured names.
2. **Given** no local machine name is configured, **When** the client renders
   the sidebar, **Then** the local machine row is labelled `Local`, exactly as
   before this feature existed.
3. **Given** a configured name that is empty or only whitespace, **When** the
   client renders the sidebar, **Then** the local machine row is labelled
   `Local` rather than showing a blank row.
4. **Given** a configured name with surrounding whitespace, **When** the
   client renders the sidebar, **Then** the label shows the trimmed name.

---

### User Story 2 - The chosen name follows the client everywhere (Priority: P2)

An operator who has named the local machine expects that name wherever the
local machine is presented, not only in the one row they were looking at.

**Why this priority**: A name that appears in one surface and not another
re-creates the confusion the request is about, on a smaller scale.

**Independent Test**: With a configured name, inspect every place the local
machine is presented (sidebar machine row, the new-workspace footer of the
active machine, the mobile machine switcher, and any sidebar token that shows
the machine value) and confirm they all show the configured name.

**Acceptance Scenarios**:

1. **Given** a configured local machine name, **When** the local machine is the
   active machine, **Then** the footer and mobile surfaces that name the active
   machine use the configured name.
2. **Given** a configured local machine name, **When** a sidebar token renders
   the machine value, **Then** the token value is the configured name, so
   existing label rules can match it.

---

### User Story 3 - Rename without restarting the client (Priority: P3)

An operator edits the configuration while the client is running and expects the
renamed machine to appear without restarting.

**Why this priority**: Nice to have. Every other appearance setting in the same
configuration section already applies on reload.

**Independent Test**: Run the client with the default label, edit the
configuration to a new local machine name, reload the configuration, and
confirm the sidebar row updates without a restart.

**Acceptance Scenarios**:

1. **Given** a running client showing `Local`, **When** the local machine name
   changes in the configuration and the configuration is reloaded, **Then** the
   sidebar row shows the new name.
2. **Given** a running client showing a custom local machine name, **When** the
   configured name is cleared to an empty value and the configuration is
   reloaded, **Then** the sidebar row shows `Local` again.

---

### Edge Cases

- A machine list refresh (a saved machine is added, removed, or reconnected)
  must not reset the local machine row to `Local`.
- The configured name is presentation only: it must not change the local
  machine's identity, its saved session data, nor the identity of any saved
  machine.
- No local machine name is invented from the host name; the default stays
  `Local` so existing setups look unchanged.
- A name longer than the sidebar width is truncated by the existing row
  truncation rules, not by this feature.

## Requirements *(mandatory)*

### Functional Requirements

- **FR-001**: The system MUST accept a user-configured display name for the
  local machine in the client-side UI configuration surface.
- **FR-002**: When no name is configured, the local machine MUST be presented
  as `Local`.
- **FR-003**: A configured name that is empty or whitespace-only MUST be
  treated as unset, so the local machine is presented as `Local`.
- **FR-004**: A configured name MUST be presented trimmed of surrounding
  whitespace.
- **FR-005**: The configured name MUST be used everywhere the local machine is
  presented to the user, including sidebar machine rows, active-machine
  footers, mobile machine surfaces, and sidebar machine token values.
- **FR-006**: A machine list refresh MUST preserve the configured local machine
  name.
- **FR-007**: A configuration reload MUST apply a changed or cleared local
  machine name without restarting the client.
- **FR-008**: The configured name MUST NOT alter machine identity, saved
  machine names, session persistence, or any server-owned state.
- **FR-009**: The new configuration key MUST be documented in the configuration
  reference.

### Key Entities

- **Local machine**: the machine the client is connected to over its local
  socket. It is always present in the machine list and is not a saved
  connection profile, so it has no profile id to rename through.
- **Saved machine label**: the user-chosen name of a remote connection profile.
  Unchanged by this feature.
- **Local machine label**: the user-chosen presentation name of the local
  machine. Defaults to `Local`.

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-001**: An operator can rename the local machine with a single
  configuration value, without editing code, hostname, or saved profiles.
- **SC-002**: With the value unset, every surface named in FR-005 presents the
  local machine exactly as before this feature (label `Local`), so no existing
  configuration changes its appearance.
- **SC-003**: After changing the value and reloading the configuration, the new
  name is visible in the sidebar with no client restart.
- **SC-004**: 100% of the surfaces named in FR-005 show the configured name in a
  single client session; no surface keeps the built-in label.

## Assumptions

- The request lists three acceptable solutions (a configuration value, a CLI
  rename, or a sidebar rename action). This specification adopts the
  configuration value: it is the only option that works for a machine without a
  saved profile, it needs no new CLI verb, and it keeps the change client-local.
- The local machine label is client presentation state, not a shared
  runtime/session fact, so it is owned by the client-side UI configuration and
  is not exposed through the server API.
- Renaming does not rewrite a user's existing sidebar label rules: rules match
  the configured name, so a rule written for `Local` stops matching once the
  machine is renamed. That is the expected consequence of renaming, and the
  rule text is the user's to update.
- The collapsed-sidebar single-letter marker and the short machine markers of
  the mobile layout are position markers for a machine slot, not names, and are
  out of scope.

## Out of Scope

- Renaming the local machine through `herdr machine rename` or any new CLI verb.
- A sidebar rename action (inline editor, context menu) on the machine group
  heading.
- Renaming saved machines, or changing how saved machine labels are stored.
- Deriving a name from the host name.
- Per-session or per-connection overrides of the local machine name.
