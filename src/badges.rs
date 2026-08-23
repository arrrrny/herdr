//! Custom badges for agents / panes / the workspace sidebar.
//!
//! A badge is an arbitrary text string rendered with an arbitrary color. Badges
//! are keyed so multiple can stack and individual ones can be cleared.
//!
//! Two input paths (both best-effort, never panic):
//! 1. File: `run/badge.json` (relative to herdr's CWD). Re-read on every render.
//!    Accepted shapes:
//!      - `{ "<key>": {"text": "...", "color": "..."}, ... }`  (preferred map)
//!      - `{ "badges": [{"key":"...", "text":"...", "color":"..."}, ...] }`
//!      - `[{"key":"...", "text":"...", "color":"..."}, ...]`
//! 2. IPC: `badge.set` / `badge.clear` / `badge.list` JSON-RPC methods on
//!    herdr's existing local API socket (see `src/api/schema/badges.rs`).
//!
//! In-memory (IPC-set) badges win on key conflict over file-sourced badges.

use std::collections::BTreeMap;
use std::path::Path;

use ratatui::style::{Color, Style};
use serde::{Deserialize, Serialize};

use crate::config::parse_color;

/// Default file path (relative to CWD) for file-sourced badges.
pub const DEFAULT_BADGE_FILE: &str = "run/badge.json";

/// Defensive cap on the number of badges rendered in the sidebar header.
pub const MAX_RENDERED_BADGES: usize = 16;

/// Maximum text length per badge before truncation kicks in.
pub const MAX_BADGE_TEXT_LEN: usize = 32;

/// A single badge: arbitrary text + arbitrary color.
///
/// `color` accepts any string accepted by [`crate::config::parse_color`]:
/// hex (`#rrggbb`, `#rgb`), `rgb(r,g,b)`, named colors, or `reset`/`default`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct Badge {
    pub text: String,
    pub color: String,
}

/// A keyed badge entry — used by the IPC `badge.set` params and the
/// `run/badge.json` map shape (object mapping keys to `{text, color}`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct BadgeEntry {
    pub key: String,
    #[serde(flatten)]
    pub badge: Badge,
}

/// In-memory store of IPC-set badges. File badges are read fresh on every
/// render via [`load_file_badges`] and merged on top of this store
/// (in-memory badges win on key conflict).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BadgeStore {
    badges: BTreeMap<String, Badge>,
}

impl BadgeStore {
    pub fn new() -> Self {
        Self::default()
    }

    /// Set or replace a badge by key. Returns the previous badge if any.
    pub fn set(&mut self, key: impl Into<String>, badge: Badge) -> Option<Badge> {
        self.badges.insert(key.into(), badge)
    }

    /// Remove a single badge by key. Returns the removed badge if any.
    pub fn clear(&mut self, key: &str) -> Option<Badge> {
        self.badges.remove(key)
    }

    /// Remove all badges. Reserved for future use (e.g. a `badge.clear_all`
    /// IPC method or a config-reload hook).
    #[allow(dead_code)]
    pub fn clear_all(&mut self) {
        self.badges.clear();
    }

    /// Iterate over (key, badge) pairs in stable (alphabetical) order.
    pub fn iter(&self) -> impl Iterator<Item = (&str, &Badge)> {
        self.badges.iter().map(|(k, v)| (k.as_str(), v))
    }

    /// Number of in-memory badges. Reserved for future use.
    #[allow(dead_code)]
    pub fn len(&self) -> usize {
        self.badges.len()
    }

    /// Whether the in-memory store is empty. Reserved for future use.
    #[allow(dead_code)]
    pub fn is_empty(&self) -> bool {
        self.badges.is_empty()
    }

    /// Snapshot the in-memory badges as a Vec of [`BadgeEntry`] pairs.
    pub fn to_vec(&self) -> Vec<BadgeEntry> {
        self.badges
            .iter()
            .map(|(k, b)| BadgeEntry {
                key: k.clone(),
                badge: b.clone(),
            })
            .collect()
    }
}

/// Best-effort read of file-sourced badges from `run/badge.json` (or the
/// provided path). Returns an empty vec on any error — file is missing,
/// unreadable, or contains malformed JSON. Never panics.
pub fn load_file_badges(path: &Path) -> Vec<BadgeEntry> {
    let bytes = match std::fs::read(path) {
        Ok(b) => b,
        Err(_) => return Vec::new(),
    };
    parse_file_badges(&bytes).unwrap_or_default()
}

fn parse_file_badges(bytes: &[u8]) -> Option<Vec<BadgeEntry>> {
    let value: serde_json::Value = serde_json::from_slice(bytes).ok()?;
    // Shape 1: object map of key -> {text, color}
    if let Some(map) = value.as_object() {
        let mut out: Vec<BadgeEntry> = Vec::with_capacity(map.len());
        for (k, v) in map {
            // Skip a top-level "badges" array key — handled below.
            if k == "badges" {
                continue;
            }
            if let Some(entry) = parse_badge_value(k.clone(), v) {
                out.push(entry);
            }
        }
        if !out.is_empty() {
            return Some(out);
        }
    }
    // Shape 2: { "badges": [...] }
    if let Some(arr) = value.get("badges").and_then(|v| v.as_array()) {
        return Some(parse_badge_array(arr));
    }
    // Shape 3: top-level array
    if let Some(arr) = value.as_array() {
        return Some(parse_badge_array(arr));
    }
    Some(Vec::new())
}

fn parse_badge_array(arr: &[serde_json::Value]) -> Vec<BadgeEntry> {
    arr.iter()
        .filter_map(|item| {
            let key = item.get("key").and_then(|v| v.as_str())?.to_string();
            parse_badge_value(key, item)
        })
        .collect()
}

fn parse_badge_value(key: String, value: &serde_json::Value) -> Option<BadgeEntry> {
    // Reject empty keys (defensive — would create phantom badges).
    if key.is_empty() {
        return None;
    }
    let text = value.get("text").and_then(|v| v.as_str())?.to_string();
    let color = value
        .get("color")
        .and_then(|v| v.as_str())
        .unwrap_or("reset")
        .to_string();
    Some(BadgeEntry {
        key,
        badge: Badge { text, color },
    })
}

/// Resolve a badge color string into a ratatui [`Color`]. Empty strings map
/// to [`Color::Reset`] (preserves the surrounding style). Unknown names
/// fall through to [`parse_color`]'s default.
pub fn resolve_color(color: &str) -> Color {
    if color.is_empty() {
        return Color::Reset;
    }
    parse_color(color)
}

/// Resolve a badge color string into a ratatui [`Style`] (foreground only).
pub fn resolve_style(color: &str) -> Style {
    Style::default().fg(resolve_color(color))
}

/// Truncate badge text for display: keeps the first `max` chars, appends an
/// ellipsis `…` if truncation occurred. Returns the (possibly truncated)
/// text. `max == 0` returns an empty string.
pub fn truncate_text(text: &str, max: usize) -> String {
    let cells: Vec<char> = text.chars().collect();
    if cells.len() <= max {
        return text.to_string();
    }
    if max == 0 {
        return String::new();
    }
    let kept: String = cells.iter().take(max.saturating_sub(1)).collect();
    format!("{kept}…")
}

/// Merge file-sourced badges with in-memory IPC badges. In-memory wins on
/// key conflict. Returns a Vec in stable (alphabetical by key) order.
pub fn merge_badges(file: &[BadgeEntry], ipc: &BadgeStore) -> Vec<BadgeEntry> {
    let mut merged: BTreeMap<String, Badge> = BTreeMap::new();
    for entry in file {
        if !entry.key.is_empty() {
            merged.insert(entry.key.clone(), entry.badge.clone());
        }
    }
    for (key, badge) in ipc.iter() {
        merged.insert(key.to_string(), badge.clone());
    }
    merged
        .into_iter()
        .map(|(key, badge)| BadgeEntry { key, badge })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn tmp(suffix: &str) -> std::path::PathBuf {
        let mut p = std::env::temp_dir();
        p.push(format!("herdr-badges-{}-{suffix}.json", std::process::id()));
        p
    }

    #[test]
    fn store_set_clear_roundtrip() {
        let mut s = BadgeStore::new();
        assert!(s.is_empty());
        s.set(
            "pool",
            Badge {
                text: "LOCAL".into(),
                color: "green".into(),
            },
        );
        s.set(
            "branch",
            Badge {
                text: "dev".into(),
                color: "#89b4fa".into(),
            },
        );
        assert_eq!(s.len(), 2);
        let snap = s.to_vec();
        assert_eq!(snap.len(), 2);
        // BTreeMap ordering: branch before pool
        assert_eq!(snap[0].key, "branch");
        assert_eq!(snap[1].key, "pool");

        let removed = s.clear("pool");
        assert!(removed.is_some());
        assert_eq!(removed.unwrap().text, "LOCAL");
        assert_eq!(s.len(), 1);
    }

    #[test]
    fn store_clear_missing_key_is_noop() {
        let mut s = BadgeStore::new();
        s.set(
            "a",
            Badge {
                text: "A".into(),
                color: "red".into(),
            },
        );
        assert!(s.clear("missing").is_none());
        assert_eq!(s.len(), 1);
    }

    #[test]
    fn store_clear_all_empties() {
        let mut s = BadgeStore::new();
        s.set(
            "a",
            Badge {
                text: "A".into(),
                color: "red".into(),
            },
        );
        s.set(
            "b",
            Badge {
                text: "B".into(),
                color: "blue".into(),
            },
        );
        s.clear_all();
        assert!(s.is_empty());
    }

    #[test]
    fn file_map_shape_parses() {
        let p = tmp("map");
        let mut f = std::fs::File::create(&p).unwrap();
        write!(
            f,
            r##"{{"pool":{{"text":"LOCAL+CLOUD","color":"#89b4fa"}},"branch":{{"text":"dev","color":"green"}}}}"##
        )
        .unwrap();
        let badges = load_file_badges(&p);
        assert_eq!(badges.len(), 2);
        let pool = badges.iter().find(|b| b.key == "pool").unwrap();
        assert_eq!(pool.badge.text, "LOCAL+CLOUD");
        assert_eq!(pool.badge.color, "#89b4fa");
        let _ = std::fs::remove_file(&p);
    }

    #[test]
    fn file_array_shape_parses() {
        let p = tmp("arr");
        let mut f = std::fs::File::create(&p).unwrap();
        write!(
            f,
            r#"[{{"key":"a","text":"A","color":"red"}},{{"key":"b","text":"B","color":"blue"}}]"#
        )
        .unwrap();
        let badges = load_file_badges(&p);
        assert_eq!(badges.len(), 2);
        let _ = std::fs::remove_file(&p);
    }

    #[test]
    fn file_badges_key_shape_parses() {
        let p = tmp("badges");
        let mut f = std::fs::File::create(&p).unwrap();
        write!(
            f,
            r#"{{"badges":[{{"key":"x","text":"X","color":"cyan"}}]}}"#
        )
        .unwrap();
        let badges = load_file_badges(&p);
        assert_eq!(badges.len(), 1);
        assert_eq!(badges[0].key, "x");
        let _ = std::fs::remove_file(&p);
    }

    #[test]
    fn file_missing_returns_empty() {
        let badges = load_file_badges(std::path::Path::new(
            "/nonexistent/herdr-badges-xyz-987654.json",
        ));
        assert!(badges.is_empty());
    }

    #[test]
    fn file_malformed_returns_empty() {
        let p = tmp("malformed");
        let mut f = std::fs::File::create(&p).unwrap();
        write!(f, "{{not json").unwrap();
        let badges = load_file_badges(&p);
        assert!(badges.is_empty());
        let _ = std::fs::remove_file(&p);
    }

    #[test]
    fn file_entry_missing_color_defaults_to_reset() {
        let p = tmp("nocolor");
        let mut f = std::fs::File::create(&p).unwrap();
        write!(f, r#"{{"k":{{"text":"T"}}}}"#).unwrap();
        let badges = load_file_badges(&p);
        assert_eq!(badges.len(), 1);
        assert_eq!(badges[0].badge.color, "reset");
        let _ = std::fs::remove_file(&p);
    }

    #[test]
    fn file_entry_missing_text_is_dropped() {
        let p = tmp("notext");
        let mut f = std::fs::File::create(&p).unwrap();
        write!(f, r#"{{"k":{{"color":"red"}}}}"#).unwrap();
        let badges = load_file_badges(&p);
        assert!(badges.is_empty());
        let _ = std::fs::remove_file(&p);
    }

    #[test]
    fn file_empty_key_is_dropped() {
        let p = tmp("emptykey");
        let mut f = std::fs::File::create(&p).unwrap();
        write!(f, r#"{{"":{{"text":"T","color":"red"}}}}"#).unwrap();
        let badges = load_file_badges(&p);
        assert!(badges.is_empty());
        let _ = std::fs::remove_file(&p);
    }

    #[test]
    fn merge_ipc_overrides_file() {
        let file = vec![BadgeEntry {
            key: "pool".into(),
            badge: Badge {
                text: "FILE".into(),
                color: "red".into(),
            },
        }];
        let mut ipc = BadgeStore::new();
        ipc.set(
            "pool",
            Badge {
                text: "IPC".into(),
                color: "green".into(),
            },
        );
        ipc.set(
            "extra",
            Badge {
                text: "E".into(),
                color: "blue".into(),
            },
        );
        let merged = merge_badges(&file, &ipc);
        assert_eq!(merged.len(), 2);
        let pool = merged.iter().find(|b| b.key == "pool").unwrap();
        assert_eq!(pool.badge.text, "IPC"); // IPC wins
    }

    #[test]
    fn merge_drops_empty_file_keys() {
        let file = vec![BadgeEntry {
            key: "".into(),
            badge: Badge {
                text: "X".into(),
                color: "red".into(),
            },
        }];
        let ipc = BadgeStore::new();
        let merged = merge_badges(&file, &ipc);
        assert!(merged.is_empty());
    }

    #[test]
    fn resolve_color_handles_hex_named_reset() {
        assert_eq!(resolve_color("#89b4fa"), Color::Rgb(0x89, 0xb4, 0xfa));
        assert_eq!(resolve_color("#abc"), Color::Rgb(0xaa, 0xbb, 0xcc));
        assert_eq!(resolve_color("red"), Color::Red);
        assert_eq!(resolve_color("reset"), Color::Reset);
        assert_eq!(resolve_color(""), Color::Reset);
    }

    #[test]
    fn truncate_text_long_input_gets_ellipsis() {
        assert_eq!(truncate_text("hello", 10), "hello");
        assert_eq!(truncate_text("hello", 5), "hello");
        assert_eq!(truncate_text("hello", 4), "hel…");
        assert_eq!(truncate_text("hello", 1), "…");
        assert_eq!(truncate_text("hello", 0), "");
    }
}
