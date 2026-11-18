# Feature Specification: stable configured node identity

**Feature Branch**: `feat/issue-93`

**Created**: 2026-11-18

**Status**: Implemented

**Input**: User description: "Every Herdr entity except an agent is addressed by a server-local id. A pane is `w1:p1`, a tab `w1:t1`, a workspace `w1` — and `w1` means whatever the server you happen to be talking to says it means. Forklift works around this today by joining every host's dispatcher state file over SSH to recover the owning machine before it can build a focus command (arrrrny/forklift#1006). The mesh needs the unique handle promoted to a first-class target, and the same treatment extended to the entities that only have local ids. Source: https://github.com/arrrrny/herdr/issues/93"

## User Scenarios & Testing *(mandatory)*

### User Story 1 - Tell which node an entity came from (Priority: P1)

An operator works across several Herdr servers and holds references from all of
them. A pane id, tab id or workspace id is server-local, so a reference on its
own does not say where it lives, and the consumer has to know the host
separately. With a node identity stamped onto every entity, the answer travels
with the reference.

**Why this priority**: This is the whole request. Every other story is a
consequence of the identifier existing.

**Independent Test**: Configure a node id, start a server, read any pane, tab,
workspace or agent through the JSON API, and confirm each response carries the
configured id.

**Acceptance Scenarios**:

1. **Given** a server configured with `node_id = "i7"`, **When** a consumer reads
   a pane, tab, workspace or agent from it, **Then** every entity carries
   `node_id: "i7"`.
2. **Given** the same server, **When** a consumer calls `session.snapshot`, **Then**
   the snapshot carries the same node id.
3. **Given** the same server, **When** a consumer calls `ping`, **Then** the
   reply reports the same node id.
4. **Given** a server with no node id configured, **When** a consumer reads an
   entity, **Then** the entity carries the documented `local` fallback.

---

### User Story 2 - The identity survives a hostname or address change (Priority: P1)

A fleet node's hostname is not stable: a worker's SSH alias goes stale on every
VM stop/start because its external IP is ephemeral. An identity derived from the
hostname would inherit that failure permanently, so the identity is configured,
never derived.

**Why this priority**: A node id that moves with the hostname solves nothing the
issue is asking for.

**Independent Test**: Configure a node id, then change the machine's hostname
and address. The id the server reports is unchanged.

**Acceptance Scenarios**:

1. **Given** a configured node id, **When** the host's hostname or address
   changes, **Then** the server reports the same node id.
2. **Given** no configured node id, **When** the server reports an identity,
   **Then** it reports the fallback rather than the hostname.

---

### User Story 3 - A colliding node id is reported, not silently accepted (Priority: P2)

Two servers configured with the same node id are a configuration error that no
server can see on its own. The identity has to be visible to the consumer
holding both references, and the collision has to be nameable.

**Why this priority**: The identifier is still correct without this; it is the
difference between detecting a misconfiguration and debugging it later.

**Independent Test**: Feed the node ids of two servers to the collision report
and confirm it names both claimants.

**Acceptance Scenarios**:

1. **Given** two servers configured with the same node id, **When** a consumer
   reports node id collisions across them, **Then** the report names the node id
   and both claimants.
2. **Given** three servers sharing one node id, **When** collisions are
   reported, **Then** every claimant is named.
3. **Given** two servers with no configured node id, **When** collisions are
   reported, **Then** nothing is reported: the fallback is shared by design.
4. **Given** a configured node id that cannot be used (empty, over-long,
   control characters, unsupported characters), **When** the configuration is
   loaded, **Then** the value is reported as a diagnostic and the server answers
   with the fallback instead of a half-validated identity.

---

### Edge Cases

- A node id padded with whitespace behaves like the same value written without
  padding.
- A node id of exactly the maximum accepted length is accepted; one byte longer
  is rejected.
- A configured id that spells the fallback (`local`) is treated as the fallback:
  it is not a stable identity, and reporting it as a collision between two such
  servers would be noise.
- A server built before node identity existed sends no value; a consumer reads
  that as an unknown identity, never as a stable one.
- Node identity is per server, not per session: every session on one host answers
  with the same configured id, because the id names the node, not the session.

## Requirements *(mandatory)*

### Functional Requirements

- **FR-001**: A Herdr server MUST have a node identity configured as
  `server.node_id` in `config.toml`.
- **FR-002**: The node identity MUST NOT be derived from the hostname, the IP
  address, or any other property of the host that can change.
- **FR-003**: The node identity MUST be validated on load. A value that is empty
  after trimming, longer than 128 bytes, or containing a control character or a
  character outside `[A-Za-z0-9._:-]` MUST be reported as a configuration
  diagnostic and MUST NOT be used.
- **FR-004**: An absent or blank `server.node_id` MUST resolve to a documented
  fallback, `local`, which MUST be described as the absence of a configured
  identity rather than a stable one.
- **FR-005**: Every entity read through the JSON API (`workspace`, `tab`,
  `pane`, `agent`) MUST carry the node identity in its response.
- **FR-006**: The session snapshot and the `ping` reply MUST carry the node
  identity, so a consumer can learn it without reading an entity.
- **FR-007**: Adding the field MUST NOT break an older client: a consumer that
  does not know the field MUST still be able to read every response.
- **FR-008**: The API MUST expose a way to report node ids claimed by more than
  one server, naming every claimant, and MUST NOT report the fallback as a
  collision.
- **FR-009**: `herdr status` MUST report the node identity of the server it talks
  to, and MUST say when the server reports none.

### Key Entities

- **Node identity**: a validated, configured name for one Herdr server. It
  qualifies every entity that server reports and is the value a future routing
  layer will resolve against.
- **Node claim**: one server's node identity as a consumer holding several sees
  it — the identity plus the name the consumer knows that server by.
- **Node collision**: one node identity claimed by more than one server, with
  every claimant named.

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-001**: A consumer holding a pane id read from a remote server can tell
  which node it belongs to from the response alone, with no second lookup.
- **SC-002**: Changing a node's hostname or address does not change the node
  identity it reports.
- **SC-003**: Two servers configured with the same node id are named by the
  collision report, with no manual diffing of configuration files.
- **SC-004**: An operator can see the node identity of the running server with
  `herdr status server`.
- **SC-005**: No existing configuration changes meaning: a server that sets no
  node id reports `local` and nothing else about its API responses changes.

## Assumptions

- The node identity names the **server**, not the session. Two sessions on one
  host answer with the same id, because the id exists to locate the machine, not
  to distinguish sessions on it.
- The identifier is a plain string, not a structured address. A routing layer
  that needs more (a transport, a port, a trust level) can carry it alongside
  without changing this one.
- Duplicate detection is reported where the consumer already holds several
  servers. The multi-machine client path speaks the frozen endpoint codec, which
  this issue must not change, so the report is a pure function over the node ids
  a consumer collected plus the per-server value `herdr status` prints.
- The fallback is `local`, matching the sidebar's existing default label for the
  unnamed local machine, so the two surfaces do not invent different words for
  the same idea.

## Out of Scope

- Peer discovery or gossip between servers.
- Transitive routing (A→B→C when A has no direct route to C).
- Promoting an agent session id to an addressable target.
- The trust decision that gates all of the above: whether peers may exchange
  discovery only while data keeps flowing over the caller's own SSH, or may
  route each other's reads.
- Per-session or per-connection node id overrides.
- Deriving an identity from the hostname, even as a labelled weak fallback.
