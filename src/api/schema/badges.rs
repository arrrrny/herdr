use serde::{Deserialize, Serialize};

use crate::badges::{Badge, BadgeEntry};

/// `badge.set` — set or replace a badge by key. Setting the same key again
/// replaces the previous badge. Multiple distinct keys stack.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct BadgeSetParams {
    /// Stable identifier for this badge. Replaces any existing badge with
    /// the same key. Non-empty.
    pub key: String,
    /// Arbitrary text to render. Truncated for display if too long.
    pub text: String,
    /// Any color string accepted by herdr's color parser: hex (`#rrggbb`,
    /// `#rgb`), `rgb(r,g,b)`, named colors, or `reset`/`default`/`none`.
    pub color: String,
}

/// `badge.clear` — remove a single badge by key. No-op if the key is not
/// present (returns success either way).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct BadgeClearParams {
    pub key: String,
}

/// `badge.list` — return all currently-set in-memory (IPC-set) badges.
/// File-sourced badges from `run/badge.json` are NOT included here; they
/// are read fresh on every render and merged with these at display time.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct BadgeInfo {
    pub key: String,
    pub text: String,
    pub color: String,
}

impl From<&BadgeEntry> for BadgeInfo {
    fn from(entry: &BadgeEntry) -> Self {
        Self {
            key: entry.key.clone(),
            text: entry.badge.text.clone(),
            color: entry.badge.color.clone(),
        }
    }
}

/// Construct an in-memory `(key, Badge)` pair from a `badge.set` request —
/// handlers feed this straight into [`crate::badges::BadgeStore::set`].
pub(crate) fn badge_from_set_params(params: &BadgeSetParams) -> (String, Badge) {
    (
        params.key.clone(),
        Badge {
            text: params.text.clone(),
            color: params.color.clone(),
        },
    )
}
