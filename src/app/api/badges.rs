//! Handlers for the `badge.set` / `badge.clear` / `badge.list` JSON-RPC methods.
//!
//! These mutate / read the in-memory [`crate::badges::BadgeStore`] on
//! [`crate::app::state::AppState`]. File-sourced badges (`run/badge.json`)
//! are NOT touched here — they are read fresh on every render and merged
//! with these at display time.

use crate::api::schema::{BadgeClearParams, BadgeInfo, BadgeSetParams, ResponseResult};
use crate::app::App;

use super::responses::{encode_error, encode_success};

impl App {
    /// `badge.set` — set or replace a badge by key.
    ///
    /// Validation: `key` must be non-empty (after trimming). `text` and
    /// `color` are accepted as-is — invalid colors fall through to herdr's
    /// color parser default at render time, never panicking.
    pub(super) fn handle_badge_set(&mut self, id: String, params: BadgeSetParams) -> String {
        let key = params.key.trim().to_string();
        if key.is_empty() {
            return encode_error(id, "invalid_params", "badge key must be non-empty");
        }
        // Truncate text defensively so a runaway controller cannot push a
        // multi-megabyte string into memory. The renderer truncates further
        // for display, but we cap storage too.
        const MAX_STORED_TEXT_LEN: usize = 256;
        const MAX_STORED_COLOR_LEN: usize = 64;
        const MAX_STORED_KEY_LEN: usize = 64;
        if key.len() > MAX_STORED_KEY_LEN {
            return encode_error(
                id,
                "invalid_params",
                format!("badge key exceeds {MAX_STORED_KEY_LEN} bytes"),
            );
        }
        let text = if params.text.len() > MAX_STORED_TEXT_LEN {
            params.text.chars().take(MAX_STORED_TEXT_LEN).collect()
        } else {
            params.text
        };
        let color = if params.color.len() > MAX_STORED_COLOR_LEN {
            params.color.chars().take(MAX_STORED_COLOR_LEN).collect()
        } else {
            params.color
        };

        let (stored_key, badge) = crate::api::schema::badge_from_set_params(&BadgeSetParams {
            key: key.clone(),
            text: text.clone(),
            color: color.clone(),
        });
        self.state.badges.set(stored_key, badge);

        // Re-render is triggered by `request_changes_ui` matching BadgeSet
        // in src/api/mod.rs — no explicit signal needed here.

        encode_success(id, ResponseResult::BadgeSet { key, text, color })
    }

    /// `badge.clear` — remove a single badge by key. No-op (still success)
    /// if the key is not present.
    pub(super) fn handle_badge_clear(&mut self, id: String, params: BadgeClearParams) -> String {
        let key = params.key.trim().to_string();
        if key.is_empty() {
            return encode_error(id, "invalid_params", "badge key must be non-empty");
        }
        let existed = self.state.badges.clear(&key).is_some();
        // Re-render is triggered by `request_changes_ui` matching BadgeClear.
        encode_success(id, ResponseResult::BadgeClear { key, existed })
    }

    /// `badge.list` — return all in-memory (IPC-set) badges. File-sourced
    /// badges are not included (they are read fresh on render).
    pub(super) fn handle_badge_list(&mut self, id: String) -> String {
        let badges: Vec<BadgeInfo> = self
            .state
            .badges
            .to_vec()
            .iter()
            .map(BadgeInfo::from)
            .collect();
        encode_success(id, ResponseResult::BadgeList { badges })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::schema::{ErrorResponse, Method, Request, SuccessResponse};

    fn test_app() -> App {
        let (_api_tx, api_rx) = tokio::sync::mpsc::unbounded_channel();
        App::new(
            &crate::config::Config::default(),
            crate::app::AppPolicy::TEST,
            None,
            api_rx,
            crate::api::EventHub::default(),
        )
    }

    fn set(app: &mut App, key: &str, text: &str, color: &str) -> ResponseResult {
        let response = app.handle_api_request(Request {
            id: "set".into(),
            method: Method::BadgeSet(BadgeSetParams {
                key: key.into(),
                text: text.into(),
                color: color.into(),
            }),
        });
        let success: SuccessResponse = serde_json::from_str(&response).unwrap_or_else(|err| {
            panic!("badge.set response was not a SuccessResponse: {response} (err: {err})")
        });
        success.result
    }

    fn clear(app: &mut App, key: &str) -> ResponseResult {
        let response = app.handle_api_request(Request {
            id: "clear".into(),
            method: Method::BadgeClear(BadgeClearParams { key: key.into() }),
        });
        let success: SuccessResponse = serde_json::from_str(&response).unwrap_or_else(|err| {
            panic!("badge.clear response was not a SuccessResponse: {response} (err: {err})")
        });
        success.result
    }

    fn list(app: &mut App) -> Vec<BadgeInfo> {
        let response = app.handle_api_request(Request {
            id: "list".into(),
            method: Method::BadgeList(crate::api::schema::EmptyParams {}),
        });
        let success: SuccessResponse = serde_json::from_str(&response).unwrap();
        match success.result {
            ResponseResult::BadgeList { badges } => badges,
            other => panic!("expected BadgeList, got {other:?}"),
        }
    }

    #[test]
    fn badge_set_persists_and_lists() {
        let mut app = test_app();
        assert!(list(&mut app).is_empty());

        let result = set(&mut app, "pool", "LOCAL+CLOUD", "#89b4fa");
        assert!(matches!(
            result,
            ResponseResult::BadgeSet { ref key, ref text, ref color }
                if key == "pool" && text == "LOCAL+CLOUD" && color == "#89b4fa"
        ));

        let badges = list(&mut app);
        assert_eq!(badges.len(), 1);
        assert_eq!(badges[0].key, "pool");
        assert_eq!(badges[0].text, "LOCAL+CLOUD");
        assert_eq!(badges[0].color, "#89b4fa");
    }

    #[test]
    fn badge_set_replaces_existing_key() {
        let mut app = test_app();
        set(&mut app, "pool", "OLD", "red");
        set(&mut app, "pool", "NEW", "green");

        let badges = list(&mut app);
        assert_eq!(badges.len(), 1);
        assert_eq!(badges[0].key, "pool");
        assert_eq!(badges[0].text, "NEW");
        assert_eq!(badges[0].color, "green");
    }

    #[test]
    fn badge_set_multiple_keys_stack_alphabetically() {
        let mut app = test_app();
        set(&mut app, "zeta", "Z", "blue");
        set(&mut app, "alpha", "A", "red");
        set(&mut app, "mid", "M", "green");

        let badges = list(&mut app);
        assert_eq!(
            badges.iter().map(|b| b.key.as_str()).collect::<Vec<_>>(),
            vec!["alpha", "mid", "zeta"]
        );
    }

    #[test]
    fn badge_clear_removes_only_matching_key() {
        let mut app = test_app();
        set(&mut app, "a", "A", "red");
        set(&mut app, "b", "B", "blue");

        let result = clear(&mut app, "a");
        assert!(matches!(
            result,
            ResponseResult::BadgeClear { ref key, existed: true } if key == "a"
        ));

        let badges = list(&mut app);
        assert_eq!(badges.len(), 1);
        assert_eq!(badges[0].key, "b");
    }

    #[test]
    fn badge_clear_missing_key_reports_existed_false() {
        let mut app = test_app();
        set(&mut app, "present", "P", "red");

        let result = clear(&mut app, "missing");
        assert!(matches!(
            result,
            ResponseResult::BadgeClear { existed: false, .. }
        ));

        let badges = list(&mut app);
        assert_eq!(badges.len(), 1);
    }

    #[test]
    fn badge_set_empty_key_is_rejected() {
        let mut app = test_app();
        let response = app.handle_api_request(Request {
            id: "set".into(),
            method: Method::BadgeSet(BadgeSetParams {
                key: "   ".into(),
                text: "T".into(),
                color: "red".into(),
            }),
        });
        let err: ErrorResponse = serde_json::from_str(&response).unwrap();
        assert_eq!(err.error.code, "invalid_params");
        assert!(list(&mut app).is_empty());
    }

    #[test]
    fn badge_clear_empty_key_is_rejected() {
        let mut app = test_app();
        let response = app.handle_api_request(Request {
            id: "clear".into(),
            method: Method::BadgeClear(BadgeClearParams { key: "".into() }),
        });
        let err: ErrorResponse = serde_json::from_str(&response).unwrap();
        assert_eq!(err.error.code, "invalid_params");
    }

    #[test]
    fn badge_set_truncates_oversized_inputs() {
        let mut app = test_app();
        let huge_text = "x".repeat(1024);
        let huge_color = "#".to_string() + &"a".repeat(200);
        let result = set(&mut app, "big", &huge_text, &huge_color);
        // Should succeed (truncated), not error.
        assert!(matches!(result, ResponseResult::BadgeSet { .. }));

        let badges = list(&mut app);
        assert_eq!(badges.len(), 1);
        // Text is truncated to 256 chars.
        assert_eq!(badges[0].text.len(), 256);
        // Color is truncated to 64 chars.
        assert_eq!(badges[0].color.len(), 64);
    }

    #[test]
    fn badge_set_trims_whitespace_in_key() {
        let mut app = test_app();
        set(&mut app, "  pool  ", "LOCAL", "green");
        let badges = list(&mut app);
        assert_eq!(badges.len(), 1);
        assert_eq!(badges[0].key, "pool");
    }

    #[test]
    fn badge_set_response_returns_trimmed_key() {
        let mut app = test_app();
        let result = set(&mut app, "  pool  ", "LOCAL", "green");
        assert!(matches!(
            result,
            ResponseResult::BadgeSet { ref key, ref text, ref color }
                if key == "pool" && text == "LOCAL" && color == "green"
        ));
    }

    #[test]
    fn badge_clear_trims_whitespace_in_key() {
        let mut app = test_app();
        set(&mut app, "pool", "LOCAL", "green");
        let result = clear(&mut app, "  pool  ");
        assert!(matches!(
            result,
            ResponseResult::BadgeClear { ref key, existed: true } if key == "pool"
        ));
        assert!(list(&mut app).is_empty());
    }

    #[test]
    fn badge_set_rejects_oversized_key() {
        let mut app = test_app();
        let huge_key = "x".repeat(128);
        let response = app.handle_api_request(Request {
            id: "set".into(),
            method: Method::BadgeSet(BadgeSetParams {
                key: huge_key,
                text: "T".into(),
                color: "red".into(),
            }),
        });
        let err: ErrorResponse = serde_json::from_str(&response).unwrap();
        assert_eq!(err.error.code, "invalid_params");
        assert!(list(&mut app).is_empty());
    }
}
