use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use interprocess::local_socket::traits::Stream as _;

use crate::ipc::LocalStream;

pub const SESSION_ENV_VAR: &str = "HERDR_SESSION";
pub const DEFAULT_SESSION_NAME: &str = "default";

const MAX_SESSION_NAME_LEN: usize = 64;
const STOP_WAIT_TIMEOUT: Duration = Duration::from_secs(15);
const STOP_WAIT_POLL: Duration = Duration::from_millis(25);
const MIN_SOCKET_TIMEOUT: Duration = Duration::from_millis(1);

static EXPLICIT_SESSION_REQUESTED: AtomicBool = AtomicBool::new(false);

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct SessionInfo {
    pub name: String,
    pub default: bool,
    pub running: bool,
    pub socket_path: String,
    pub session_dir: String,
}

pub fn configure_from_args(args: &[String]) -> Result<Vec<String>, String> {
    let mut cleaned = Vec::with_capacity(args.len());
    if let Some(program) = args.first() {
        cleaned.push(program.clone());
    }

    if args.get(1).map(String::as_str) == Some("session")
        && args.get(2).map(String::as_str) == Some("attach")
    {
        if matches!(
            args.get(3).map(String::as_str),
            Some("help" | "--help" | "-h")
        ) {
            return Ok(args.to_vec());
        }
        let Some(name) = args.get(3) else {
            return Err("usage: herdr session attach <name>".to_string());
        };
        if args.len() != 4 {
            return Err("usage: herdr session attach <name>".to_string());
        }
        apply_explicit_name(name)?;
        return Ok(cleaned);
    }

    let mut requested_session = None;
    let mut index = 1;
    while index < args.len() {
        let arg = &args[index];
        if arg == "--" {
            cleaned.extend_from_slice(&args[index..]);
            break;
        }
        if arg == "--session" {
            let Some(value) = args.get(index + 1) else {
                return Err("missing value for --session".to_string());
            };
            requested_session = Some(value.clone());
            index += 2;
            continue;
        }
        if let Some(value) = arg.strip_prefix("--session=") {
            requested_session = Some(value.to_string());
            index += 1;
            continue;
        }

        cleaned.push(arg.clone());
        index += 1;
    }

    if let Some(session) = requested_session {
        apply_explicit_name(&session)?;
    } else if std::env::var_os(crate::api::SOCKET_PATH_ENV_VAR).is_some() {
        EXPLICIT_SESSION_REQUESTED.store(false, Ordering::Relaxed);
    } else if let Ok(session) = std::env::var(SESSION_ENV_VAR) {
        if normalize_name(&session)?.is_none() {
            std::env::remove_var(SESSION_ENV_VAR);
        }
        EXPLICIT_SESSION_REQUESTED.store(false, Ordering::Relaxed);
    } else {
        EXPLICIT_SESSION_REQUESTED.store(false, Ordering::Relaxed);
    }

    Ok(cleaned)
}

pub fn active_name() -> Option<String> {
    std::env::var(SESSION_ENV_VAR)
        .ok()
        .filter(|name| name != DEFAULT_SESSION_NAME)
        .filter(|name| validate_name(name).is_ok())
}

pub fn local_attach_command() -> String {
    match active_name() {
        Some(name) => format!("herdr session attach {name}"),
        None => "herdr".to_string(),
    }
}

pub fn local_stop_command() -> String {
    stop_command_for(active_name().as_deref())
}

pub fn stop_command_for(name: Option<&str>) -> String {
    match name {
        Some(name) => format!("herdr session stop {name}"),
        None => "herdr server stop".to_string(),
    }
}

pub fn restart_after_update_guidance(stop_command: &str, attach_command: Option<&str>) -> String {
    let restart = match attach_command {
        Some(command) => format!("Run `{stop_command}`, then run `{command}` again."),
        None => format!("Run `{stop_command}`, then restart Herdr with the same socket override."),
    };
    format!(
        "Stop the old server to use the new version.\nStopping exits pane processes.\n{restart}"
    )
}

pub fn active_restart_after_update_guidance() -> String {
    if !explicit_session_requested() {
        if let Ok(socket_path) = std::env::var(crate::api::SOCKET_PATH_ENV_VAR) {
            return restart_after_update_guidance(
                &format!(
                    "{}={} herdr server stop",
                    crate::api::SOCKET_PATH_ENV_VAR,
                    socket_path
                ),
                None,
            );
        }
    }

    restart_after_update_guidance(&local_stop_command(), Some(&local_attach_command()))
}

pub fn explicit_session_requested() -> bool {
    EXPLICIT_SESSION_REQUESTED.load(Ordering::Relaxed)
}

#[cfg(test)]
pub(crate) fn clear_explicit_session_for_test() {
    EXPLICIT_SESSION_REQUESTED.store(false, Ordering::Relaxed);
}

pub fn data_dir() -> PathBuf {
    data_dir_for(active_name().as_deref())
}

pub fn data_dir_for(name: Option<&str>) -> PathBuf {
    let config_dir = crate::config::config_dir();
    match name {
        Some(name) => config_dir.join("sessions").join(name),
        None => config_dir,
    }
}

pub fn api_socket_path_for(name: Option<&str>) -> PathBuf {
    data_dir_for(name).join("herdr.sock")
}

pub fn active_api_socket_path() -> PathBuf {
    if explicit_session_requested() {
        return api_socket_path_for(active_name().as_deref());
    }
    if let Ok(path) = std::env::var(crate::api::SOCKET_PATH_ENV_VAR) {
        return PathBuf::from(path);
    }
    api_socket_path_for(active_name().as_deref())
}

pub fn client_socket_path_for(name: Option<&str>) -> PathBuf {
    data_dir_for(name).join("herdr-client.sock")
}

pub fn list_sessions() -> std::io::Result<Vec<SessionInfo>> {
    let mut sessions = vec![session_info(None)];
    let sessions_dir = crate::config::config_dir().join("sessions");
    let entries = match std::fs::read_dir(&sessions_dir) {
        Ok(entries) => entries,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(sessions),
        Err(err) => return Err(err),
    };

    let mut names = Vec::new();
    for entry in entries {
        let entry = entry?;
        if !entry.file_type()?.is_dir() {
            continue;
        }
        let Some(name) = entry.file_name().to_str().map(str::to_string) else {
            continue;
        };
        if name != DEFAULT_SESSION_NAME && validate_name(&name).is_ok() {
            names.push(name);
        }
    }
    names.sort();
    sessions.extend(names.iter().map(|name| session_info(Some(name))));
    Ok(sessions)
}

pub fn session_info(name: Option<&str>) -> SessionInfo {
    let default = name.is_none();
    let display_name = name.unwrap_or(DEFAULT_SESSION_NAME).to_string();
    let socket_path = api_socket_path_for(name);
    let session_dir = data_dir_for(name);
    SessionInfo {
        name: display_name,
        default,
        running: is_running_at(&socket_path),
        socket_path: socket_path.display().to_string(),
        session_dir: session_dir.display().to_string(),
    }
}

pub fn parse_target_name(name: &str) -> Result<Option<String>, String> {
    normalize_name(name)
}

pub fn stop_session(name: Option<&str>) -> Result<SessionInfo, String> {
    stop_session_with_timeout(name, STOP_WAIT_TIMEOUT)
}

pub(crate) fn stop_active_server() -> Result<(), String> {
    let socket_path = active_api_socket_path();
    let client_socket_path = crate::server::socket_paths::client_socket_path();
    stop_socket_with_timeout(
        socket_path.clone(),
        vec![socket_path, client_socket_path],
        STOP_WAIT_TIMEOUT,
        "server",
    )
}

fn stop_session_with_timeout(name: Option<&str>, timeout: Duration) -> Result<SessionInfo, String> {
    let socket_path = api_socket_path_for(name);
    let client_socket_path = client_socket_path_for(name);
    let label = format!("session {}", name.unwrap_or(DEFAULT_SESSION_NAME));
    stop_socket_with_timeout(
        socket_path.clone(),
        vec![socket_path, client_socket_path],
        timeout,
        &label,
    )?;
    Ok(session_info(name))
}

fn stop_socket_with_timeout(
    socket_path: PathBuf,
    stopped_socket_paths: Vec<PathBuf>,
    timeout: Duration,
    label: &str,
) -> Result<(), String> {
    let deadline = Instant::now() + timeout;
    let request = serde_json::json!({
        "id": "cli:session:stop",
        "method": "server.stop",
        "params": {}
    });
    // Try the normal API path first: connect to the status API socket and
    // send `server.stop`. If the socket is missing or stale (partial-shutdown
    // state, issue #11), fall back to finding the server PID via a live
    // client socket and signaling it directly so `herdr server stop` can
    // recover without a manual kill.
    match crate::ipc::connect_local_stream(&socket_path) {
        Ok(stream) => {
            let stop_response = send_stop_request(stream, &request, deadline)?;
            if let Some(response) = stop_response {
                if let Some(error) = response.get("error") {
                    return Err(error.to_string());
                }
            }
        }
        Err(err) if stop_socket_connect_error_allows_fallback(err.kind()) => {
            // Partial-shutdown recovery (issue #11): the status API socket is
            // missing or stale, but the server process may still be alive on
            // the client socket. Try to find the server PID and SIGTERM it.
            let recovered = try_recover_partial_shutdown(&stopped_socket_paths, deadline)?;
            if !recovered {
                // No client socket alive either — server is genuinely not running.
                return Err(format!(
                    "{label} is not running or cannot be reached at {}: {err}",
                    socket_path.display()
                ));
            }
        }
        Err(err) => {
            return Err(format!(
                "{label} is not running or cannot be reached at {}: {err}",
                socket_path.display()
            ));
        }
    }
    if !wait_until_stopped_until(&stopped_socket_paths, deadline) {
        let reachable = reachable_socket_paths(&stopped_socket_paths);
        return Err(format!(
            "{label} did not stop within {}ms; sockets are still reachable at {}",
            timeout.as_millis(),
            reachable
                .iter()
                .map(|path| path.display().to_string())
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    Ok(())
}

/// Returns true if a missing or stale status API socket should trigger the
/// partial-shutdown recovery path (find the server PID via the client socket
/// and SIGTERM it). `NotFound` means the socket file is gone; `ConnectionRefused`
/// means the socket file exists but nobody is listening (stale); `TimedOut`
/// means the server isn't accepting in a reasonable time.
fn stop_socket_connect_error_allows_fallback(kind: std::io::ErrorKind) -> bool {
    matches!(
        kind,
        std::io::ErrorKind::NotFound
            | std::io::ErrorKind::ConnectionRefused
            | std::io::ErrorKind::TimedOut
    )
}

/// Attempts to recover from the partial-shutdown state (issue #11) by finding
/// the server's PID via a live socket and sending SIGTERM. Returns `Ok(true)`
/// if a live server was found and signaled; `Ok(false)` if no server appears
/// to be running (no live socket found, or PID lookup unsupported/failed).
fn try_recover_partial_shutdown(
    stopped_socket_paths: &[PathBuf],
    deadline: Instant,
) -> Result<bool, String> {
    // Find any socket that's still alive (server is listening on it).
    let live_socket = stopped_socket_paths.iter().find(|path| is_running_at(path));
    let Some(live_socket) = live_socket else {
        return Ok(false);
    };

    // Find the PID of the process listening on the live socket.
    let Some(pid) = crate::platform::find_unix_socket_owner_pid(live_socket) else {
        return Ok(false);
    };

    // Don't signal ourselves — this would happen if the test process or a
    // server running in the same process tried to stop itself via the
    // fallback path. The normal API path handles same-process stops.
    if pid == std::process::id() {
        return Err(format!(
            "refusing to signal current process (pid {pid}); the server may be this process"
        ));
    }

    // Send SIGTERM to the server process. The server's ctrlc handler sets
    // should_quit and the main loop exits cleanly (complete_shutdown removes
    // the client socket; api_server Drop removes the API socket).
    crate::platform::signal_processes(&[pid], crate::platform::Signal::Terminate);

    // Wait for the process to exit (poll process_exists).
    while Instant::now() < deadline {
        if !crate::platform::process_exists(pid) {
            break;
        }
        std::thread::sleep(STOP_WAIT_POLL.min(time_until(deadline)));
    }

    // If the server is still listening after the deadline, recovery failed: it
    // ignored SIGTERM and survives. Don't remove the socket files — the server
    // is still running, and removing them would let the caller's
    // `wait_until_stopped_until` falsely conclude the server stopped. Report
    // the real failure so the outer timeout path surfaces the error.
    //
    // We check socket liveness rather than `process_exists(pid)` because a
    // cleanly-exited child may linger as a zombie (not yet reaped) and would
    // otherwise be mistaken for a surviving server.
    if stopped_socket_paths.iter().any(|path| is_running_at(path)) {
        return Ok(false);
    }

    // Clean up any leftover socket files. The server should have removed them
    // on graceful exit, but if it was killed mid-shutdown they may remain.
    for path in stopped_socket_paths {
        let _ = std::fs::remove_file(path);
    }

    Ok(true)
}

pub fn delete_session(name: &str) -> Result<SessionInfo, String> {
    if name == DEFAULT_SESSION_NAME {
        return Err("deleting the default session is not supported".to_string());
    }
    validate_name(name)?;
    let socket_path = api_socket_path_for(Some(name));
    if is_running_at(&socket_path) {
        return Err(format!(
            "session {name} is running; stop it before deleting"
        ));
    }
    let info = session_info(Some(name));
    let dir = data_dir_for(Some(name));
    match std::fs::remove_dir_all(&dir) {
        Ok(()) => Ok(info),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(info),
        Err(err) => Err(err.to_string()),
    }
}

fn send_stop_request(
    mut stream: LocalStream,
    request: &serde_json::Value,
    deadline: Instant,
) -> Result<Option<serde_json::Value>, String> {
    let Some(write_timeout) = socket_timeout_until(deadline) else {
        return Ok(None);
    };
    if let Err(err) = stream.set_send_timeout(Some(write_timeout)) {
        if !stop_timeout_error_allows_wait(&err) {
            return Err(err.to_string());
        }
    }

    let response = send_stop_request_inner(&mut stream, request, deadline);
    match response {
        Ok(Some(line)) => serde_json::from_str(&line)
            .map(Some)
            .map_err(|err| err.to_string()),
        Ok(None) => Ok(None),
        Err(err) if stop_request_error_allows_wait(&err) => Ok(None),
        Err(err) => Err(err.to_string()),
    }
}

fn send_stop_request_inner(
    stream: &mut LocalStream,
    request: &serde_json::Value,
    deadline: Instant,
) -> std::io::Result<Option<String>> {
    stream.write_all(request.to_string().as_bytes())?;
    stream.write_all(b"\n")?;
    stream.flush()?;

    let Some(read_timeout) = socket_timeout_until(deadline) else {
        return Ok(None);
    };
    if let Err(err) = stream.set_recv_timeout(Some(read_timeout)) {
        if stop_timeout_error_allows_wait(&err) {
            return Ok(None);
        }
        return Err(err);
    }

    let mut line = String::new();
    let bytes_read = BufReader::new(stream).read_line(&mut line)?;
    if bytes_read == 0 {
        return Ok(None);
    }
    Ok(Some(line))
}

fn stop_timeout_error_allows_wait(err: &std::io::Error) -> bool {
    err.kind() == std::io::ErrorKind::InvalidInput
        || (cfg!(windows) && err.kind() == std::io::ErrorKind::Unsupported)
}

fn stop_request_error_allows_wait(err: &std::io::Error) -> bool {
    matches!(
        err.kind(),
        std::io::ErrorKind::BrokenPipe
            | std::io::ErrorKind::ConnectionReset
            | std::io::ErrorKind::UnexpectedEof
            | std::io::ErrorKind::NotConnected
            | std::io::ErrorKind::TimedOut
            | std::io::ErrorKind::WouldBlock
    )
}

fn is_running_at(socket_path: &Path) -> bool {
    socket_path.exists() && crate::ipc::connect_local_stream(socket_path).is_ok()
}

fn wait_until_stopped_until(socket_paths: &[PathBuf], deadline: Instant) -> bool {
    while Instant::now() < deadline {
        if socket_paths.iter().all(|path| !is_running_at(path)) {
            return true;
        }
        std::thread::sleep(STOP_WAIT_POLL.min(time_until(deadline)));
    }
    socket_paths.iter().all(|path| !is_running_at(path))
}

fn reachable_socket_paths(socket_paths: &[PathBuf]) -> Vec<PathBuf> {
    socket_paths
        .iter()
        .filter(|path| is_running_at(path))
        .cloned()
        .collect()
}

fn time_until(deadline: Instant) -> Duration {
    deadline.saturating_duration_since(Instant::now())
}

fn socket_timeout_until(deadline: Instant) -> Option<Duration> {
    socket_timeout_from_remaining(time_until(deadline))
}

fn socket_timeout_from_remaining(remaining: Duration) -> Option<Duration> {
    if remaining.is_zero() {
        return None;
    }
    Some(remaining.max(MIN_SOCKET_TIMEOUT))
}

pub fn validate_name(name: &str) -> Result<(), String> {
    if name.is_empty() {
        return Err("session name cannot be empty".to_string());
    }
    if name.len() > MAX_SESSION_NAME_LEN {
        return Err(format!(
            "session name cannot be longer than {MAX_SESSION_NAME_LEN} bytes"
        ));
    }
    if name == "." || name == ".." {
        return Err("session name cannot be . or ..".to_string());
    }
    if !name
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
    {
        return Err(
            "session name may only contain ASCII letters, numbers, '.', '_' and '-'".to_string(),
        );
    }
    Ok(())
}

fn apply_explicit_name(name: &str) -> Result<(), String> {
    let session = normalize_name(name)?;
    if let Some(session) = session {
        std::env::set_var(SESSION_ENV_VAR, session);
    } else {
        std::env::remove_var(SESSION_ENV_VAR);
    }
    EXPLICIT_SESSION_REQUESTED.store(true, Ordering::Relaxed);
    Ok(())
}

fn normalize_name(name: &str) -> Result<Option<String>, String> {
    if name == DEFAULT_SESSION_NAME {
        return Ok(None);
    }
    validate_name(name)?;
    Ok(Some(name.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(unix)]
    use interprocess::local_socket::traits::Listener as _;
    use std::sync::{Mutex, OnceLock};

    fn env_lock() -> &'static Mutex<()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
    }

    #[cfg(unix)]
    fn unique_test_path(name: &str) -> std::path::PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("herdr-{name}-{}-{nanos}", std::process::id()))
    }

    #[cfg(unix)]
    fn local_stream_pair(name: &str) -> (LocalStream, LocalStream, std::path::PathBuf) {
        let path = unique_test_path(name);
        let listener = crate::ipc::bind_local_listener(&path).unwrap();
        let client = crate::ipc::connect_local_stream(&path).unwrap();
        let server = listener.accept().unwrap();
        (client, server, path)
    }

    #[test]
    fn stop_wait_timeout_allows_slow_graceful_shutdown() {
        assert_eq!(STOP_WAIT_TIMEOUT, Duration::from_secs(15));
    }

    #[test]
    fn stop_request_errors_wait_for_socket_state() {
        for kind in [
            std::io::ErrorKind::BrokenPipe,
            std::io::ErrorKind::ConnectionReset,
            std::io::ErrorKind::UnexpectedEof,
            std::io::ErrorKind::NotConnected,
            std::io::ErrorKind::TimedOut,
            std::io::ErrorKind::WouldBlock,
        ] {
            let err = std::io::Error::from(kind);
            assert!(stop_request_error_allows_wait(&err), "{kind:?}");
        }
    }

    #[test]
    fn stop_timeout_invalid_input_waits_for_socket_state() {
        let err = std::io::Error::from(std::io::ErrorKind::InvalidInput);

        assert!(stop_timeout_error_allows_wait(&err));
    }

    #[test]
    fn stop_timeout_unsupported_waits_for_socket_state_only_on_windows() {
        let err = std::io::Error::from(std::io::ErrorKind::Unsupported);

        assert_eq!(stop_timeout_error_allows_wait(&err), cfg!(windows));
    }

    #[test]
    fn socket_timeouts_are_never_zero_duration() {
        assert_eq!(socket_timeout_from_remaining(Duration::ZERO), None);
        assert_eq!(
            socket_timeout_from_remaining(Duration::from_nanos(1)),
            Some(MIN_SOCKET_TIMEOUT)
        );
        assert_eq!(
            socket_timeout_from_remaining(Duration::from_millis(10)),
            Some(Duration::from_millis(10))
        );
    }

    #[cfg(unix)]
    #[test]
    fn stop_request_empty_response_waits_for_socket_state() {
        let (client, server, _path) = local_stream_pair("stop-empty-response");
        let handle = std::thread::spawn(move || {
            let mut request = String::new();
            let _ = BufReader::new(server).read_line(&mut request);
            request
        });
        let request = serde_json::json!({
            "id": "cli:session:stop",
            "method": "server.stop",
            "params": {}
        });

        assert_eq!(
            send_stop_request(
                client,
                &request,
                Instant::now() + Duration::from_millis(100)
            )
            .unwrap(),
            None
        );
        assert!(handle.join().unwrap().contains("server.stop"));
    }

    #[cfg(unix)]
    #[test]
    fn stop_session_times_out_when_socket_stays_open_without_response() {
        let _guard = env_lock().lock().unwrap();
        let config_home = PathBuf::from(format!("/tmp/hs-stop-open-{}", std::process::id()));
        std::env::set_var("XDG_CONFIG_HOME", &config_home);
        let session_name = "silent";
        let socket_path = api_socket_path_for(Some(session_name));
        std::fs::create_dir_all(socket_path.parent().unwrap()).unwrap();
        let _ = std::fs::remove_file(&socket_path);
        let listener = std::os::unix::net::UnixListener::bind(&socket_path).unwrap();
        listener.set_nonblocking(true).unwrap();
        let keep_running = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true));
        let keep_running_for_thread = keep_running.clone();
        let handle = std::thread::spawn(move || {
            let mut held_streams = Vec::new();
            while keep_running_for_thread.load(Ordering::Relaxed) {
                match listener.accept() {
                    Ok((stream, _)) => {
                        if let Ok(reader_stream) = stream.try_clone() {
                            let mut request = String::new();
                            match BufReader::new(reader_stream).read_line(&mut request) {
                                Ok(0) => continue,
                                Ok(_) if request.contains("server.stop") => {
                                    held_streams.push(stream)
                                }
                                Ok(_) => {}
                                Err(_) => continue,
                            }
                        }
                    }
                    Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(5));
                    }
                    Err(_) => break,
                }
            }
        });

        let err = stop_session_with_timeout(Some(session_name), Duration::from_millis(75))
            .expect_err("silent session should fail after timeout");

        assert!(err.contains("did not stop"), "{err}");
        keep_running.store(false, Ordering::Relaxed);
        handle.join().unwrap();
        let _ = std::fs::remove_dir_all(&config_home);
        std::env::remove_var("XDG_CONFIG_HOME");
    }

    #[test]
    fn stop_socket_connect_error_allows_fallback_for_stale_or_missing_socket() {
        for kind in [
            std::io::ErrorKind::NotFound,
            std::io::ErrorKind::ConnectionRefused,
            std::io::ErrorKind::TimedOut,
        ] {
            let err = std::io::Error::from(kind);
            assert!(
                stop_socket_connect_error_allows_fallback(err.kind()),
                "{kind:?} should allow partial-shutdown fallback"
            );
        }
    }

    #[test]
    fn stop_socket_connect_error_does_not_fallback_for_permission_denied() {
        let err = std::io::Error::from(std::io::ErrorKind::PermissionDenied);
        assert!(
            !stop_socket_connect_error_allows_fallback(err.kind()),
            "PermissionDenied should surface the raw error, not trigger PID-signal fallback"
        );
    }

    #[cfg(unix)]
    #[test]
    fn try_recover_partial_shutdown_returns_false_when_no_socket_alive() {
        let api_socket = PathBuf::from(format!(
            "/tmp/herdr-stop-no-api-{}-{}.sock",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let client_socket = PathBuf::from(format!(
            "/tmp/herdr-stop-no-client-{}-{}.sock",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _ = std::fs::remove_file(&api_socket);
        let _ = std::fs::remove_file(&client_socket);
        // Neither socket exists → no live socket → Ok(false), no recovery.
        let recovered = try_recover_partial_shutdown(
            &[api_socket.clone(), client_socket.clone()],
            Instant::now() + Duration::from_millis(100),
        )
        .unwrap();
        assert!(!recovered);
    }

    /// Integration test for issue #11: `herdr server stop` should recover from
    /// the partial-shutdown state where the API socket is missing but a server
    /// process is still alive on the client socket. Spawns a child Python
    /// process that binds a Unix socket (simulating the surviving server),
    /// leaves the API socket path empty, and verifies that
    /// `stop_socket_with_timeout` finds the child PID and SIGTERMs it.
    #[cfg(unix)]
    #[test]
    fn stop_socket_with_timeout_recovers_partial_shutdown_via_sigterm() {
        use std::process::{Command, Stdio};

        let python = ["python3", "python"]
            .iter()
            .find(|cmd| Command::new(cmd).arg("--version").output().is_ok());
        let Some(python) = python else {
            // Python isn't installed — skip the integration test rather than fail.
            eprintln!("stop_socket_with_timeout_recovers_partial_shutdown_via_sigterm: python3 not found, skipping");
            return;
        };

        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let api_socket = PathBuf::from(format!(
            "/tmp/herdr-stop-api-{stamp}-{}.sock",
            std::process::id()
        ));
        let client_socket = PathBuf::from(format!(
            "/tmp/herdr-stop-client-{stamp}-{}.sock",
            std::process::id()
        ));
        let _ = std::fs::remove_file(&api_socket);
        let _ = std::fs::remove_file(&client_socket);

        // Spawn a child that binds the "client" socket and accepts connections
        // until SIGTERM arrives (python's default handler exits the process).
        let script = format!(
            r#"
import os, signal, socket, sys, time
path = sys.argv[1]
if os.path.exists(path):
    os.unlink(path)
s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
s.bind(path)
s.listen(5)
signal.signal(signal.SIGTERM, lambda *_: sys.exit(0))
while True:
    conn, _ = s.accept()
    conn.close()
"#,
        );
        let mut child = Command::new(python)
            .arg("-c")
            .arg(&script)
            .arg(client_socket.to_str().unwrap())
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn python child");

        // Wait for the child to bind the socket (poll up to 2s).
        let ready_deadline = Instant::now() + Duration::from_secs(2);
        while Instant::now() < ready_deadline {
            if is_running_at(&client_socket) {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(
            is_running_at(&client_socket),
            "child failed to bind client socket"
        );
        // API socket is intentionally absent — this is the partial-shutdown
        // state from issue #11.
        assert!(!api_socket.exists());

        let result = stop_socket_with_timeout(
            api_socket.clone(),
            vec![api_socket.clone(), client_socket.clone()],
            Duration::from_secs(5),
            "test server",
        );

        // The fallback should succeed — child was found and SIGTERM'd.
        assert!(result.is_ok(), "fallback failed: {:?}", result.err());

        // The child process should have exited (either gracefully via its
        // SIGTERM handler, like the real herdr server's ctrlc handler, or
        // killed by signal). The important thing is that it stopped.
        let child_status = child.try_wait().expect("child not waited on");
        assert!(child_status.is_some(), "child still running after stop");

        // Sockets should be cleaned up.
        assert!(!client_socket.exists() || !is_running_at(&client_socket));
        let _ = std::fs::remove_file(&api_socket);
        let _ = std::fs::remove_file(&client_socket);
    }

    /// Regression test for the kimi.ai review finding on PR #19:
    /// `try_recover_partial_shutdown` must NOT report success (or remove the
    /// socket files) when the signaled process ignores SIGTERM and survives
    /// past the deadline. Reporting success there lets the caller's
    /// `wait_until_stopped_until` falsely believe the server stopped, while
    /// it is actually still running.
    #[cfg(unix)]
    #[test]
    fn try_recover_partial_shutdown_returns_false_when_process_ignores_sigterm() {
        use std::process::{Command, Stdio};

        let python = ["python3", "python"]
            .iter()
            .find(|cmd| Command::new(cmd).arg("--version").output().is_ok());
        let Some(python) = python else {
            // Python isn't installed — skip the integration test rather than fail.
            eprintln!("try_recover_partial_shutdown_returns_false_when_process_ignores_sigterm: python3 not found, skipping");
            return;
        };

        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let client_socket = PathBuf::from(format!(
            "/tmp/herdr-stop-ignore-{stamp}-{}.sock",
            std::process::id()
        ));
        let _ = std::fs::remove_file(&client_socket);

        // Child binds the socket and ignores SIGTERM — it survives the signal.
        let script = format!(
            r#"
import os, signal, socket, sys
path = sys.argv[1]
if os.path.exists(path):
    os.unlink(path)
signal.signal(signal.SIGTERM, signal.SIG_IGN)
s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
s.bind(path)
s.listen(5)
while True:
    conn, _ = s.accept()
    conn.close()
"#,
        );
        let mut child = Command::new(python)
            .arg("-c")
            .arg(&script)
            .arg(client_socket.to_str().unwrap())
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn python child");

        // Wait for the child to bind the socket (poll up to 2s).
        let ready_deadline = Instant::now() + Duration::from_secs(2);
        while Instant::now() < ready_deadline {
            if is_running_at(&client_socket) {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(
            is_running_at(&client_socket),
            "child failed to bind client socket"
        );

        let recovered = try_recover_partial_shutdown(
            &[client_socket.clone()],
            Instant::now() + Duration::from_millis(500),
        )
        .unwrap();

        // The process survived SIGTERM, so recovery must report failure.
        assert!(
            !recovered,
            "should return false when process ignores SIGTERM"
        );

        // The socket file must NOT be removed — the server is still running.
        assert!(
            client_socket.exists(),
            "socket file must not be removed when recovery fails"
        );

        child.kill().expect("kill child");
        let _ = child.wait();
        let _ = std::fs::remove_file(&client_socket);
    }

    #[test]
    fn configure_from_args_removes_global_session_option() {
        let _guard = env_lock().lock().unwrap();
        std::env::remove_var(SESSION_ENV_VAR);
        clear_explicit_session_for_test();
        let args = vec![
            "herdr".to_string(),
            "--session".to_string(),
            "work".to_string(),
            "workspace".to_string(),
            "list".to_string(),
        ];

        let cleaned = configure_from_args(&args).unwrap();

        assert_eq!(std::env::var(SESSION_ENV_VAR).as_deref(), Ok("work"));
        assert!(explicit_session_requested());
        assert_eq!(cleaned, vec!["herdr", "workspace", "list"]);
        std::env::remove_var(SESSION_ENV_VAR);
        clear_explicit_session_for_test();
    }

    #[test]
    fn configure_from_args_accepts_equals_form() {
        let _guard = env_lock().lock().unwrap();
        std::env::remove_var(SESSION_ENV_VAR);
        clear_explicit_session_for_test();
        let args = vec![
            "herdr".to_string(),
            "server".to_string(),
            "stop".to_string(),
            "--session=api".to_string(),
        ];

        let cleaned = configure_from_args(&args).unwrap();

        assert_eq!(std::env::var(SESSION_ENV_VAR).as_deref(), Ok("api"));
        assert!(explicit_session_requested());
        assert_eq!(cleaned, vec!["herdr", "server", "stop"]);
        std::env::remove_var(SESSION_ENV_VAR);
        clear_explicit_session_for_test();
    }

    #[test]
    fn configure_from_args_preserves_child_session_option_after_separator() {
        let _guard = env_lock().lock().unwrap();
        std::env::remove_var(SESSION_ENV_VAR);
        clear_explicit_session_for_test();
        let args = vec![
            "herdr".to_string(),
            "agent".to_string(),
            "start".to_string(),
            "repro".to_string(),
            "--".to_string(),
            "/bin/echo".to_string(),
            "--session".to_string(),
            "child-session".to_string(),
        ];

        let cleaned = configure_from_args(&args).unwrap();

        assert_eq!(cleaned, args);
        assert!(std::env::var(SESSION_ENV_VAR).is_err());
        assert!(!explicit_session_requested());
    }

    #[test]
    fn configure_from_args_preserves_child_session_equals_option_after_separator() {
        let _guard = env_lock().lock().unwrap();
        std::env::remove_var(SESSION_ENV_VAR);
        clear_explicit_session_for_test();
        let args = vec![
            "herdr".to_string(),
            "agent".to_string(),
            "start".to_string(),
            "repro".to_string(),
            "--".to_string(),
            "/bin/echo".to_string(),
            "--session=child-session".to_string(),
        ];

        let cleaned = configure_from_args(&args).unwrap();

        assert_eq!(cleaned, args);
        assert!(std::env::var(SESSION_ENV_VAR).is_err());
        assert!(!explicit_session_requested());
    }

    #[test]
    fn configure_from_args_rewrites_session_attach_to_default_launch() {
        let _guard = env_lock().lock().unwrap();
        std::env::set_var(SESSION_ENV_VAR, "bad/name");
        std::env::set_var(crate::api::SOCKET_PATH_ENV_VAR, "/tmp/inherited.sock");
        clear_explicit_session_for_test();
        let args = vec![
            "herdr".to_string(),
            "session".to_string(),
            "attach".to_string(),
            "work".to_string(),
        ];

        let cleaned = configure_from_args(&args).unwrap();

        assert_eq!(std::env::var(SESSION_ENV_VAR).as_deref(), Ok("work"));
        assert!(explicit_session_requested());
        assert_eq!(cleaned, vec!["herdr"]);
        std::env::remove_var(SESSION_ENV_VAR);
        std::env::remove_var(crate::api::SOCKET_PATH_ENV_VAR);
        clear_explicit_session_for_test();
    }

    #[test]
    fn configure_from_args_leaves_session_attach_help_for_cli_dispatch() {
        let _guard = env_lock().lock().unwrap();
        std::env::remove_var(SESSION_ENV_VAR);
        clear_explicit_session_for_test();
        let args = vec![
            "herdr".to_string(),
            "session".to_string(),
            "attach".to_string(),
            "-h".to_string(),
        ];

        let cleaned = configure_from_args(&args).unwrap();

        assert_eq!(cleaned, args);
        assert!(!explicit_session_requested());
    }

    #[test]
    fn configure_from_args_maps_default_session_name_to_default_path() {
        let _guard = env_lock().lock().unwrap();
        let config_home =
            std::env::temp_dir().join(format!("herdr-session-default-{}", std::process::id()));
        std::env::set_var("XDG_CONFIG_HOME", &config_home);
        std::env::set_var(SESSION_ENV_VAR, "work");
        clear_explicit_session_for_test();
        std::env::set_var(crate::api::SOCKET_PATH_ENV_VAR, "/tmp/inherited.sock");
        let args = vec![
            "herdr".to_string(),
            "--session".to_string(),
            DEFAULT_SESSION_NAME.to_string(),
            "workspace".to_string(),
            "list".to_string(),
        ];

        let cleaned = configure_from_args(&args).unwrap();

        assert_eq!(cleaned, vec!["herdr", "workspace", "list"]);
        assert!(std::env::var(SESSION_ENV_VAR).is_err());
        assert!(explicit_session_requested());
        assert_eq!(
            active_api_socket_path(),
            config_home
                .join(crate::config::app_dir_name())
                .join("herdr.sock")
        );
        std::env::remove_var("XDG_CONFIG_HOME");
        std::env::remove_var(SESSION_ENV_VAR);
        clear_explicit_session_for_test();
        std::env::remove_var(crate::api::SOCKET_PATH_ENV_VAR);
    }

    #[test]
    fn env_session_does_not_mark_session_explicit() {
        let _guard = env_lock().lock().unwrap();
        std::env::set_var(SESSION_ENV_VAR, "env-session");
        EXPLICIT_SESSION_REQUESTED.store(true, Ordering::Relaxed);
        let args = vec![
            "herdr".to_string(),
            "workspace".to_string(),
            "list".to_string(),
        ];

        let cleaned = configure_from_args(&args).unwrap();

        assert_eq!(cleaned, vec!["herdr", "workspace", "list"]);
        assert_eq!(std::env::var(SESSION_ENV_VAR).as_deref(), Ok("env-session"));
        assert!(!explicit_session_requested());
        std::env::remove_var(SESSION_ENV_VAR);
    }

    #[test]
    fn env_default_session_name_uses_default_path() {
        let _guard = env_lock().lock().unwrap();
        let config_home =
            std::env::temp_dir().join(format!("herdr-env-session-default-{}", std::process::id()));
        std::env::set_var("XDG_CONFIG_HOME", &config_home);
        std::env::remove_var(crate::api::SOCKET_PATH_ENV_VAR);
        std::env::set_var(SESSION_ENV_VAR, DEFAULT_SESSION_NAME);
        EXPLICIT_SESSION_REQUESTED.store(true, Ordering::Relaxed);
        let args = vec![
            "herdr".to_string(),
            "workspace".to_string(),
            "list".to_string(),
        ];

        let cleaned = configure_from_args(&args).unwrap();

        assert_eq!(cleaned, vec!["herdr", "workspace", "list"]);
        assert!(std::env::var(SESSION_ENV_VAR).is_err());
        assert!(!explicit_session_requested());
        assert_eq!(
            active_api_socket_path(),
            config_home
                .join(crate::config::app_dir_name())
                .join("herdr.sock")
        );
        std::env::remove_var("XDG_CONFIG_HOME");
        std::env::remove_var(SESSION_ENV_VAR);
        std::env::remove_var(crate::api::SOCKET_PATH_ENV_VAR);
        clear_explicit_session_for_test();
    }

    #[test]
    fn local_attach_command_uses_default_launch_for_default_session() {
        let _guard = env_lock().lock().unwrap();
        std::env::remove_var(SESSION_ENV_VAR);

        assert_eq!(local_attach_command(), "herdr");
    }

    #[test]
    fn local_attach_command_uses_session_attach_for_named_session() {
        let _guard = env_lock().lock().unwrap();
        std::env::set_var(SESSION_ENV_VAR, "work");

        assert_eq!(local_attach_command(), "herdr session attach work");

        std::env::remove_var(SESSION_ENV_VAR);
    }

    #[test]
    fn local_stop_command_uses_server_stop_for_default_session() {
        let _guard = env_lock().lock().unwrap();
        std::env::remove_var(SESSION_ENV_VAR);

        assert_eq!(local_stop_command(), "herdr server stop");

        std::env::remove_var(SESSION_ENV_VAR);
    }

    #[test]
    fn local_stop_command_uses_session_stop_for_named_session() {
        let _guard = env_lock().lock().unwrap();
        std::env::set_var(SESSION_ENV_VAR, "work");

        assert_eq!(local_stop_command(), "herdr session stop work");

        std::env::remove_var(SESSION_ENV_VAR);
    }

    #[test]
    fn restart_after_update_guidance_names_stop_and_attach_commands() {
        assert_eq!(
            restart_after_update_guidance(
                "herdr session stop work",
                Some("herdr session attach work")
            ),
            "Stop the old server to use the new version.\nStopping exits pane processes.\nRun `herdr session stop work`, then run `herdr session attach work` again."
        );
    }

    #[test]
    fn active_restart_after_update_guidance_respects_socket_override() {
        let _guard = env_lock().lock().unwrap();
        std::env::set_var(crate::api::SOCKET_PATH_ENV_VAR, "/tmp/custom-herdr.sock");
        std::env::remove_var(SESSION_ENV_VAR);
        clear_explicit_session_for_test();

        assert_eq!(
            active_restart_after_update_guidance(),
            "Stop the old server to use the new version.\nStopping exits pane processes.\nRun `HERDR_SOCKET_PATH=/tmp/custom-herdr.sock herdr server stop`, then restart Herdr with the same socket override."
        );

        std::env::remove_var(crate::api::SOCKET_PATH_ENV_VAR);
    }

    #[test]
    fn explicit_session_socket_ignores_inherited_socket_override() {
        let _guard = env_lock().lock().unwrap();
        let config_home =
            std::env::temp_dir().join(format!("herdr-session-precedence-{}", std::process::id()));
        std::env::set_var("XDG_CONFIG_HOME", &config_home);
        std::env::set_var(SESSION_ENV_VAR, "work");
        EXPLICIT_SESSION_REQUESTED.store(true, Ordering::Relaxed);
        std::env::set_var(crate::api::SOCKET_PATH_ENV_VAR, "/tmp/inherited.sock");

        let path = active_api_socket_path();

        assert_eq!(
            path,
            config_home
                .join(crate::config::app_dir_name())
                .join("sessions")
                .join("work")
                .join("herdr.sock")
        );
        std::env::remove_var("XDG_CONFIG_HOME");
        std::env::remove_var(SESSION_ENV_VAR);
        clear_explicit_session_for_test();
        std::env::remove_var(crate::api::SOCKET_PATH_ENV_VAR);
    }

    #[test]
    fn env_socket_override_wins_without_explicit_session() {
        let _guard = env_lock().lock().unwrap();
        std::env::set_var(SESSION_ENV_VAR, "work");
        clear_explicit_session_for_test();
        std::env::set_var(crate::api::SOCKET_PATH_ENV_VAR, "/tmp/explicit.sock");

        assert_eq!(
            active_api_socket_path(),
            PathBuf::from("/tmp/explicit.sock")
        );

        std::env::remove_var(SESSION_ENV_VAR);
        clear_explicit_session_for_test();
        std::env::remove_var(crate::api::SOCKET_PATH_ENV_VAR);
    }

    #[test]
    fn env_socket_override_skips_invalid_env_session_validation_without_explicit_session() {
        let _guard = env_lock().lock().unwrap();
        std::env::set_var(SESSION_ENV_VAR, "bad/name");
        clear_explicit_session_for_test();
        std::env::set_var(crate::api::SOCKET_PATH_ENV_VAR, "/tmp/herdr.sock");
        let args = vec![
            "herdr".to_string(),
            "workspace".to_string(),
            "list".to_string(),
        ];

        let cleaned = configure_from_args(&args).unwrap();

        assert_eq!(cleaned, vec!["herdr", "workspace", "list"]);
        assert!(!explicit_session_requested());
        assert_eq!(active_api_socket_path(), PathBuf::from("/tmp/herdr.sock"));
        assert_eq!(std::env::var(SESSION_ENV_VAR).as_deref(), Ok("bad/name"));

        std::env::remove_var(SESSION_ENV_VAR);
        clear_explicit_session_for_test();
        std::env::remove_var(crate::api::SOCKET_PATH_ENV_VAR);
    }

    #[cfg(unix)]
    #[test]
    fn stop_session_fails_when_socket_remains_reachable_after_timeout() {
        let _guard = env_lock().lock().unwrap();
        let config_home = PathBuf::from(format!("/tmp/hs-stop-{}", std::process::id()));
        std::env::set_var("XDG_CONFIG_HOME", &config_home);
        let session_name = "slow";
        let socket_path = api_socket_path_for(Some(session_name));
        std::fs::create_dir_all(socket_path.parent().unwrap()).unwrap();
        let _ = std::fs::remove_file(&socket_path);
        let listener = std::os::unix::net::UnixListener::bind(&socket_path).unwrap();
        listener.set_nonblocking(true).unwrap();
        let keep_running = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true));
        let keep_running_for_thread = keep_running.clone();
        let handle = std::thread::spawn(move || {
            while keep_running_for_thread.load(Ordering::Relaxed) {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        if let Ok(reader_stream) = stream.try_clone() {
                            let mut request = String::new();
                            match BufReader::new(reader_stream).read_line(&mut request) {
                                Ok(0) => continue,
                                Ok(_) if request.trim().is_empty() => continue,
                                Ok(_) => {}
                                Err(_) => continue,
                            }
                        }
                        let _ = stream.write_all(b"{\"id\":\"cli:session:stop\",\"result\":{}}\n");
                        let _ = stream.flush();
                    }
                    Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(5));
                    }
                    Err(_) => break,
                }
            }
        });

        let err = stop_session_with_timeout(Some(session_name), Duration::from_millis(75))
            .expect_err("still-running session should fail");

        assert!(err.contains("did not stop"), "{err}");
        assert!(
            err.contains(socket_path.to_string_lossy().as_ref()),
            "{err}"
        );
        keep_running.store(false, Ordering::Relaxed);
        handle.join().unwrap();
        let _ = std::fs::remove_dir_all(&config_home);
        std::env::remove_var("XDG_CONFIG_HOME");
    }

    #[test]
    fn invalid_names_are_rejected() {
        let _guard = env_lock().lock().unwrap();
        assert!(validate_name("../prod").is_err());
        assert!(validate_name("").is_err());
        assert!(validate_name("work session").is_err());
    }

    #[test]
    fn parse_default_target_name_maps_to_default_session() {
        assert_eq!(parse_target_name(DEFAULT_SESSION_NAME).unwrap(), None);
        assert_eq!(parse_target_name("work").unwrap(), Some("work".to_string()));
    }

    #[test]
    fn delete_default_session_is_rejected() {
        assert!(delete_session(DEFAULT_SESSION_NAME).is_err());
    }

    #[test]
    fn list_sessions_skips_reserved_default_directory() {
        let _guard = env_lock().lock().unwrap();
        let config_home =
            std::env::temp_dir().join(format!("herdr-session-list-{}", std::process::id()));
        let sessions_dir = config_home
            .join(crate::config::app_dir_name())
            .join("sessions");
        std::fs::create_dir_all(sessions_dir.join(DEFAULT_SESSION_NAME)).unwrap();
        std::fs::create_dir_all(sessions_dir.join("work")).unwrap();
        std::env::set_var("XDG_CONFIG_HOME", &config_home);
        std::env::remove_var(SESSION_ENV_VAR);
        clear_explicit_session_for_test();

        let sessions = list_sessions().unwrap();
        let names: Vec<_> = sessions
            .iter()
            .map(|session| session.name.as_str())
            .collect();

        assert_eq!(names, vec![DEFAULT_SESSION_NAME, "work"]);
        std::fs::remove_dir_all(&config_home).unwrap();
        std::env::remove_var("XDG_CONFIG_HOME");
    }
}
