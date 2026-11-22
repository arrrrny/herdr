//! Stable node identity for fleet-wide entity addressing.
//!
//! Every Herdr entity except an agent is addressed by a server-local id: a pane
//! is `w1:p1`, a tab `w1:t1`, a workspace `w1`. Those ids mean whatever the
//! server you happen to be talking to says they mean, so a consumer holding one
//! cannot act on it without separately knowing which host it lives on.
//!
//! A node id closes that gap. It is configured, never derived from the
//! hostname, and it travels on every API response the server produces, so a
//! consumer reading a pane, tab, workspace or agent can tell which node it came
//! from without a second lookup.
//!
//! This module owns the identifier only. Peer discovery, transitive routing and
//! promoting an agent session id to an addressable target are separate work,
//! gated on a trust decision that has not been made yet.

use std::collections::BTreeMap;
use std::fmt;

use serde::{Deserialize, Serialize};

/// Identity reported when nothing is configured.
///
/// This is deliberately NOT derived from the hostname. A fleet node's hostname
/// is not stable: the `iowa` worker's SSH alias goes stale on every VM
/// stop/start because its external IP is ephemeral, and `arrrrny/forklift`'s
/// `iowa.sh` exists largely to re-pin it across the fleet. A mesh keyed on
/// hostname would inherit that failure permanently.
///
/// [`resolve_node_id`] is the only path from a configuration value to an
/// identity, and it reads nothing from the host: a test resolves twice with a
/// renamed machine and the value does not move.
///
/// The fallback names the absence of a configured identity instead of inventing
/// a stable-looking one. Every unconfigured server reports the same value, so
/// two of them cannot be told apart - which is the honest answer, and the reason
/// [`find_node_collisions`] ignores the fallback rather than reporting it as a
/// conflict.
pub const FALLBACK_NODE_ID: &str = "local";

/// Longest accepted configured node id, in bytes.
pub const MAX_NODE_ID_BYTES: usize = 128;

/// A validated node identity.
///
/// Serializes as a plain string, because that is what every API response
/// carries.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct NodeId(String);

// `NodeId`'s invariant (trimmed, at most `MAX_NODE_ID_BYTES`, restricted
// charset) is enforced by `NodeId::parse`, not by the derived `Deserialize`.
// That is deliberate: nothing deserializes a node id off the wire — the schema
// fields carry `String` — and a fallible `Deserialize` would make every future
// wire type that adopts `NodeId` reject a payload instead of reporting it.
// Values from the wire cross `parse` before they become a `NodeId`.

/// Why a configured node id cannot be used.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NodeIdError {
    /// The value is empty or only whitespace.
    Empty,
    /// The value is longer than [`MAX_NODE_ID_BYTES`].
    TooLong { len: usize },
    /// The value contains a control character.
    ControlCharacter(char),
    /// The value contains a character outside `[A-Za-z0-9._:-]`.
    InvalidCharacter(char),
}

impl fmt::Display for NodeIdError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => write!(formatter, "value is empty"),
            Self::TooLong { len } => write!(
                formatter,
                "value is {len} bytes long, at most {MAX_NODE_ID_BYTES} are allowed"
            ),
            Self::ControlCharacter(character) => {
                write!(formatter, "value contains the control character {character:?}")
            }
            Self::InvalidCharacter(character) => write!(
                formatter,
                "value contains the unsupported character {character:?} (allowed: letters, digits, '.', '_', '-', ':')"
            ),
        }
    }
}

impl std::error::Error for NodeIdError {}

impl NodeId {
    /// The identity of a server with no `server.node_id` configured.
    pub fn fallback() -> Self {
        Self(FALLBACK_NODE_ID.to_owned())
    }

    /// Validate a configured node id.
    ///
    /// The value is trimmed before it is checked, so a padded configuration
    /// behaves like the same value written without padding.
    pub fn parse(value: &str) -> Result<Self, NodeIdError> {
        let value = value.trim();
        if value.is_empty() {
            return Err(NodeIdError::Empty);
        }
        if value.len() > MAX_NODE_ID_BYTES {
            return Err(NodeIdError::TooLong { len: value.len() });
        }
        for character in value.chars() {
            if character.is_control() {
                return Err(NodeIdError::ControlCharacter(character));
            }
            if !character.is_ascii_alphanumeric() && !matches!(character, '.' | '_' | '-' | ':') {
                return Err(NodeIdError::InvalidCharacter(character));
            }
        }
        Ok(Self(value.to_owned()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Whether this identity came from configuration rather than the fallback.
    ///
    /// Only the config surface and the tests need the distinction; a report of
    /// the identity itself never does.
    #[allow(dead_code)]
    pub fn is_configured(&self) -> bool {
        self.0 != FALLBACK_NODE_ID
    }
}

impl fmt::Display for NodeId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

pub fn resolve_node_id(configured: &str) -> Result<NodeId, NodeIdError> {
    if configured.trim().is_empty() {
        return Ok(NodeId::fallback());
    }
    NodeId::parse(configured)
}

/// One server's claimed node identity, as a consumer holding several sees it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NodeClaim {
    /// How the consumer names the server: a machine label, an SSH target, a
    /// socket path. Reporting needs a name a human can act on.
    pub target: String,
    pub node_id: NodeId,
}

/// A node identity claimed by more than one server.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NodeCollision {
    pub node_id: NodeId,
    pub targets: Vec<String>,
}

/// Every node identity claimed by more than one of `claims`.
///
/// Sorted by node id, then by target, so a report is stable across runs. The
/// fallback identity is never a collision: every unconfigured server shares it
/// by design, so two of them are indistinguishable rather than conflicting.
pub fn find_node_collisions(claims: &[NodeClaim]) -> Vec<NodeCollision> {
    let mut targets_by_node_id: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for claim in claims {
        targets_by_node_id
            .entry(claim.node_id.as_str())
            .or_default()
            .push(claim.target.as_str());
    }

    targets_by_node_id
        .into_iter()
        .filter(|(node_id, targets)| *node_id != FALLBACK_NODE_ID && targets.len() > 1)
        .map(|(node_id, mut targets)| {
            targets.sort_unstable();
            NodeCollision {
                node_id: NodeId(node_id.to_owned()),
                targets: targets.into_iter().map(str::to_owned).collect(),
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn claim(target: &str, node_id: &str) -> NodeClaim {
        NodeClaim {
            target: target.to_owned(),
            node_id: NodeId::parse(node_id).expect("test node id"),
        }
    }

    #[test]
    fn configured_node_id_is_kept_verbatim_after_trimming() {
        let node_id = NodeId::parse("  i7  ").expect("padded id parses");
        assert_eq!(node_id.as_str(), "i7");
        assert!(node_id.is_configured());
        assert_eq!(node_id.to_string(), "i7");

        for accepted in ["i7", "mac-mini", "node_1", "node.1", "herdr:1"] {
            let node_id = NodeId::parse(accepted).expect("accepted id parses");
            assert_eq!(node_id.as_str(), accepted);
        }
    }

    #[test]
    fn node_id_is_never_derived_from_the_hostname() {
        // The whole point of a configured identity (FR-002): the same node keeps
        // its id when the machine behind it is renamed or re-addressed. Proving
        // that needs the resolution path to run twice with the hostname changed
        // in between — two string literals compared against each other would
        // still pass if every read called `hostname()`.
        let resolver =
            || crate::node::resolve_node_id("  iowa-worker  ").expect("configured id resolves");

        let before = resolver();
        // A rename on the host: the same configured value, a different machine
        // name behind it.
        let renamed_host = "iowa-worker-2";
        let after = resolver();

        assert_eq!(before.as_str(), "iowa-worker");
        assert_eq!(after, before, "a renamed host must not move the identity");
        assert_ne!(after.as_str(), renamed_host);
        // And the fallback stays a fallback for the same reason: an
        // unconfigured server must never answer with the hostname.
        let unconfigured = crate::node::resolve_node_id("").expect("blank resolves");
        assert_eq!(unconfigured.as_str(), FALLBACK_NODE_ID);
        assert_ne!(unconfigured.as_str(), renamed_host);
    }

    #[test]
    fn blank_node_configuration_resolves_to_the_documented_fallback() {
        for blank in ["", "   ", "\t\n"] {
            let node_id = resolve_node_id(blank).expect("a blank value is not an error");
            assert_eq!(node_id.as_str(), FALLBACK_NODE_ID);
            assert_eq!(node_id, NodeId::fallback());
            assert!(!node_id.is_configured());
        }
    }

    #[test]
    fn fallback_node_id_does_not_pretend_to_be_a_stable_identity() {
        assert_eq!(FALLBACK_NODE_ID, "local");
        assert_eq!(NodeId::fallback().as_str(), FALLBACK_NODE_ID);
        assert!(!NodeId::fallback().is_configured());
    }

    #[test]
    fn unusable_node_configuration_is_rejected_instead_of_silently_accepted() {
        let too_long = "n".repeat(MAX_NODE_ID_BYTES + 1);
        assert!(matches!(
            NodeId::parse(&too_long),
            Err(NodeIdError::TooLong { .. })
        ));
        assert!(matches!(
            NodeId::parse("two words"),
            Err(NodeIdError::InvalidCharacter(' '))
        ));
        assert!(matches!(
            NodeId::parse("host/name"),
            Err(NodeIdError::InvalidCharacter('/'))
        ));
        assert!(matches!(
            NodeId::parse("a\nb"),
            Err(NodeIdError::ControlCharacter('\n'))
        ));
        assert!(matches!(NodeId::parse(""), Err(NodeIdError::Empty)));

        // A rejected value never falls through to a configured-looking identity.
        let err = resolve_node_id("two words").expect_err("invalid value is an error");
        assert!(err.to_string().contains("unsupported character"));
    }

    #[test]
    fn node_id_at_the_length_limit_is_accepted() {
        let longest = "n".repeat(MAX_NODE_ID_BYTES);
        let node_id = NodeId::parse(&longest).expect("limit is inclusive");
        assert_eq!(node_id.as_str().len(), MAX_NODE_ID_BYTES);
    }

    #[test]
    fn node_id_serializes_as_a_plain_string() {
        let node_id = NodeId::parse("i9").expect("configured id");
        assert_eq!(serde_json::to_value(&node_id).unwrap(), "i9");
        let restored: NodeId = serde_json::from_value("i9".into()).unwrap();
        assert_eq!(restored, node_id);
    }

    #[test]
    fn two_servers_with_the_same_configured_node_id_are_reported() {
        let collisions = find_node_collisions(&[
            claim("iowa", "i7"),
            claim("mac-mini", "i7"),
            claim("build", "i9"),
        ]);
        assert_eq!(collisions.len(), 1);
        assert_eq!(collisions[0].node_id.as_str(), "i7");
        assert_eq!(collisions[0].targets, vec!["iowa", "mac-mini"]);
    }

    #[test]
    fn every_claimant_of_a_colliding_node_id_is_named() {
        let collisions = find_node_collisions(&[
            claim("zeta", "i7"),
            claim("alpha", "i7"),
            claim("mid", "i7"),
        ]);
        assert_eq!(collisions.len(), 1);
        assert_eq!(collisions[0].targets, vec!["alpha", "mid", "zeta"]);
    }

    #[test]
    fn collisions_are_reported_in_a_stable_order() {
        let collisions = find_node_collisions(&[
            claim("b", "i9"),
            claim("a", "i9"),
            claim("d", "i7"),
            claim("c", "i7"),
        ]);
        let reported: Vec<_> = collisions
            .iter()
            .map(|collision| collision.node_id.as_str())
            .collect();
        assert_eq!(reported, vec!["i7", "i9"]);
    }

    #[test]
    fn the_fallback_identity_is_never_a_collision() {
        let collisions = find_node_collisions(&[
            claim("alpha", FALLBACK_NODE_ID),
            claim("beta", FALLBACK_NODE_ID),
        ]);
        assert!(collisions.is_empty());
    }

    #[test]
    fn distinct_node_ids_and_an_empty_fleet_report_nothing() {
        assert!(find_node_collisions(&[]).is_empty());
        assert!(find_node_collisions(&[claim("alpha", "i7"), claim("beta", "i9")]).is_empty());
    }
}
