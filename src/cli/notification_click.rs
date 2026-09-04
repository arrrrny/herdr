//! Hidden macOS system-notification click handler.
//!
//! When `ui.toast.delivery = "system"` is set on macOS, `terminal-notifier`
//! is invoked with `-execute <herdr_binary>` and a sidecar marker
//! file at `<config_dir>/notification_click_target.json` containing the
//! public pane id of the originating pane (see `src/platform/macos.rs`).
//!
//! Clicking the notification's "Show" action launches a fresh herdr binary
//! via macOS's `NSWorkspace openURL:`. The fresh binary has no args and no
//! controlling TTY on stdin, so `maybe_run` calls
//! [`handle_if_click_launch`] before normal CLI dispatch.
//!
//! On a successful click-launch, this module reads the marker file, sends a
//! `pane.focus` API request to the running TUI's Unix socket via the
//! existing CLI `send_request` path, and exits. The running TUI's
//! `handle_pane_focus` then calls `focus_pane_in_workspace` — the same
//! primitive the in-app Herdr toast uses — which is the validated
//! focus-switch primitive identified in arrrrny/herdr#27.

use std::io::Write;
use std::path::PathBuf;
use std::time::Duration;

use crate::api::schema::{Method, PaneTarget, Request};

/// Marker file freshness window. A marker older than this is treated as
/// stale (the notification was probably long-since dismissed).
const NOTIFICATION_CLICK_TARGET_MAX_AGE: Duration = Duration::from_secs(5 * 60);

/// Maximum log file size before rotation (64 KB).
const LOG_MAX_BYTES: u64 = 64 * 1024;

/// Append a timestamped line to `<config_dir>/notification_click.log`.
///
/// The click-launched binary has no TTY, so this is the only observable
/// trace of whether a macOS notification click was detected and what it did.
/// Best-effort: a failed write is silently ignored. The file is rotated to
/// `notification_click.log.1` when it exceeds 64 KB to prevent unbounded growth.
fn log_click(msg: impl Into<String>) {
    let msg = msg.into();
    let path = crate::config::config_dir().join("notification_click.log");
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    // Rotate when the log file exceeds 64 KB.
    let _ = std::fs::metadata(&path)
        .ok()
        .filter(|m| m.len() > LOG_MAX_BYTES)
        .and_then(|_| {
            let rotated = path.with_extension("log.1");
            std::fs::rename(&path, rotated).ok()
        });
    let _ = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .and_then(|mut f| writeln!(f, "[{ts}] {msg}"));
}

/// Entry point for the click-launch heuristic.
///
/// Returns `Ok(Some(exit_code))` if this process is a click-handler launch
/// (caller should exit with the given code); `Ok(None)` if it is a normal
/// launch (caller should proceed with the usual TUI/CLI dispatch).
pub(crate) fn handle_if_click_launch() -> std::io::Result<Option<i32>> {
    if !is_click_launch() {
        return Ok(None);
    }
    log_click("click-launch detected; running handler");
    // Best-effort: never bubble errors into the user's terminal — the
    // click-handler is a background process with no TTY attached.
    let exit_code = match run_click_handler() {
        Ok(code) => {
            log_click(format!("handler finished exit={code}"));
            code
        }
        Err(err) => {
            log_click(format!("handler error: {err}"));
            1
        }
    };
    Ok(Some(exit_code))
}

/// Heuristic: this is a click-handler launch if all of:
///   1. No CLI subcommand was passed (argv has just the binary path).
///   2. stdin is not a TTY (terminal-notifier spawns with no TTY).
///   3. A fresh marker file exists at the canonical click-target path.
fn is_click_launch() -> bool {
    let args: Vec<String> = std::env::args().collect();
    if args.len() != 1 {
        log_click(format!("not click-launch: args.len()={}", args.len()));
        return false;
    }
    use std::io::IsTerminal;
    if std::io::stdin().is_terminal() {
        log_click("not click-launch: stdin is a TTY");
        return false;
    }
    let Some(path) = click_target_path() else {
        log_click("not click-launch: no marker path on this platform");
        return false;
    };
    let Ok(metadata) = std::fs::metadata(&path) else {
        log_click("not click-launch: marker file missing");
        return false;
    };
    let Ok(modified) = metadata.modified() else {
        return false;
    };
    let age = std::time::SystemTime::now()
        .duration_since(modified)
        .unwrap_or_default();
    if age > NOTIFICATION_CLICK_TARGET_MAX_AGE {
        log_click(format!("not click-launch: marker stale {}s", age.as_secs()));
        return false;
    }
    log_click("click-launch checks passed");
    true
}

/// Path to the marker file on macOS; `None` on other platforms (no
/// click-back channel is wired there, so no marker is ever written).
fn click_target_path() -> Option<PathBuf> {
    if cfg!(target_os = "macos") {
        Some(crate::config::config_dir().join("notification_click_target.json"))
    } else {
        None
    }
}

/// Reads the marker file, deletes it (one-shot), and sends a
/// `pane.focus` API request to the running TUI. Returns the exit code
/// the binary should use.
fn run_click_handler() -> std::io::Result<i32> {
    let path = click_target_path().ok_or_else(|| {
        std::io::Error::other("no notification click target path on this platform")
    })?;
    let body = std::fs::read_to_string(&path)?;
    // Delete the marker first so a concurrent re-launch (e.g. the user
    // clicks the notification twice) can't double-fire the focus request.
    let _ = std::fs::remove_file(&path);

    let pane_id = parse_pane_id_from_marker(&body)?;
    if pane_id.is_empty() {
        log_click("handler: empty pane_id, skipping");
        return Ok(0);
    }

    log_click(format!(
        "handler: parsed pane_id={pane_id}; sending pane.focus"
    ));
    let request = Request {
        id: "macos-notification-click".into(),
        method: Method::PaneFocus(PaneTarget { pane_id }),
    };
    // Best-effort dispatch. If the running TUI is unreachable (e.g. it has
    // since exited), there's nothing useful to print to a non-TTY stdout —
    // exit silently with the underlying error code.
    match super::send_request(&request) {
        Ok(_) => {
            log_click("handler: pane.focus sent OK");
            Ok(0)
        }
        Err(err) => {
            log_click(format!("handler: pane.focus FAILED: {err}"));
            Ok(1)
        }
    }
}

fn parse_pane_id_from_marker(body: &str) -> std::io::Result<String> {
    let value: serde_json::Value = serde_json::from_str(body)
        .map_err(|err| std::io::Error::other(format!("invalid marker file: {err}")))?;
    let pane_id = value
        .get("pane_id")
        .and_then(|v| v.as_str())
        .ok_or_else(|| std::io::Error::other("marker file missing pane_id"))?;
    Ok(pane_id.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn click_target_path_returns_none_off_macos() {
        // The path is only meaningful on macOS. On other platforms the
        // function returns None so the heuristic never triggers.
        let path = click_target_path();
        if cfg!(target_os = "macos") {
            assert!(path.is_some(), "macOS should expose a marker path");
        } else {
            assert!(path.is_none(), "non-macOS should not expose a marker path");
        }
    }

    #[test]
    fn parse_pane_id_from_marker_reads_canonical_shape() {
        let body = r#"{"pane_id":"wA:pB","written_at_ms":1700000000000}"#;
        let pane_id = parse_pane_id_from_marker(body).expect("marker should parse");
        assert_eq!(pane_id, "wA:pB");
    }

    #[test]
    fn parse_pane_id_from_marker_rejects_missing_pane_id() {
        let body = r#"{"written_at_ms":1700000000000}"#;
        let result = parse_pane_id_from_marker(body);
        assert!(result.is_err(), "marker without pane_id should error");
    }

    #[test]
    fn parse_pane_id_from_marker_rejects_invalid_json() {
        let body = "not json";
        let result = parse_pane_id_from_marker(body);
        assert!(result.is_err(), "invalid JSON should error");
    }
}
