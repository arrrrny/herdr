//! Loopback HTTP listener for agent state-report pushes.
//!
//! Ziki (and agents following the same contract,
//! `specs/011-herdr-ziki-state-sync` §2 on the ziki repo) publishes its agent
//! state to `POST ${HERDR_API_URL}/api/v1/pane/report/agent` with a JSON body.
//! This listener bridges that single endpoint onto the existing JSON-RPC
//! `pane.report_agent` dispatch, so HTTP pushes share the exact ingestion
//! semantics of the local socket path (agent-label normalization, full
//! lifecycle authority for `herdr:ziki`, the per-source stale-`seq` guard,
//! and the pane-not-found / invalid-agent error surface).
//!
//! The listener is deliberately minimal: one endpoint, one request per
//! connection, `Content-Length`-framed bodies, no keep-alive. Agents treat the
//! push as best-effort and fall back to screen-marker / OSC-title detection
//! when it is unavailable, so a bind failure only logs a warning and never
//! prevents Herdr from starting.

use std::io::{self, BufRead, BufReader, Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream, ToSocketAddrs};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use tracing::{debug, info, warn};

use crate::api::schema::{Method, PaneReportAgentParams, Request};
use crate::api::ApiRequestSender;

/// Environment override for the push listener bind address, mirroring
/// `HERDR_SOCKET_PATH`. Takes precedence over `[server].agent_push_listen_addr`.
/// An empty value disables the listener.
pub(crate) const HTTP_PUSH_LISTEN_ADDR_ENV_VAR: &str = "HERDR_API_LISTEN_ADDR";

/// Default bind address. The Ziki contract pins the client-side default to
/// `http://localhost:7878`, so Herdr listens on the loopback match of that
/// port out of the box.
pub(crate) const DEFAULT_HTTP_PUSH_LISTEN_ADDR: &str = "127.0.0.1:7878";

/// The single endpoint this listener serves.
pub(crate) const AGENT_REPORT_PATH: &str = "/api/v1/pane/report/agent";

const MAX_BODY_BYTES: usize = 1024 * 1024;
const MAX_REQUEST_LINE_BYTES: usize = 8 * 1024;
const MAX_HEADER_BYTES: usize = 16 * 1024;
const CONNECTION_READ_TIMEOUT: Duration = Duration::from_secs(10);
const RESPONSE_TIMEOUT: Duration = Duration::from_secs(5);

/// Resolve the push listener bind address: `HERDR_API_LISTEN_ADDR` wins over
/// `[server].agent_push_listen_addr`; an empty value disables the listener.
/// Bind failures stay non-fatal (the caller logs and continues), so an
/// unparseable address only disables the push path.
pub(crate) fn resolved_http_push_listen_addr() -> Option<SocketAddr> {
    if let Ok(value) = std::env::var(HTTP_PUSH_LISTEN_ADDR_ENV_VAR) {
        let value = value.trim();
        if value.is_empty() {
            return None;
        }
        return parse_listen_addr(value, HTTP_PUSH_LISTEN_ADDR_ENV_VAR);
    }
    let configured = crate::config::Config::load()
        .config
        .server
        .agent_push_listen_addr;
    let configured = configured.trim();
    if configured.is_empty() {
        return None;
    }
    parse_listen_addr(configured, "[server].agent_push_listen_addr")
}

fn parse_listen_addr(value: &str, source: &str) -> Option<SocketAddr> {
    if let Ok(addr) = value.parse::<SocketAddr>() {
        if !addr.ip().is_loopback() {
            warn!(
                source,
                value,
                "agent push listen address must be loopback (127.0.0.1 or ::1) for unauthenticated endpoint"
            );
            return None;
        }
        return Some(addr);
    }
    // Hostnames such as `localhost:7878` are accepted for parity with the
    // contract's URL-shaped client default.
    match value.to_socket_addrs() {
        Ok(mut addrs) => {
            let addr = addrs.find(|addr| addr.is_ipv4()).or_else(|| addrs.next());
            match addr {
                Some(addr) => {
                    if !addr.ip().is_loopback() {
                        warn!(
                            source,
                            value,
                            "agent push listen address must be loopback (127.0.0.1 or ::1) for unauthenticated endpoint"
                        );
                        return None;
                    }
                    Some(addr)
                }
                None => {
                    warn!(
                        source,
                        value, "agent push listen address resolved to nothing"
                    );
                    None
                }
            }
        }
        Err(err) => {
            warn!(source, value, err = %err, "invalid agent push listen address");
            None
        }
    }
}

pub(crate) struct HttpPushServerHandle {
    running: Arc<AtomicBool>,
    #[cfg_attr(not(test), allow(dead_code))]
    addr: SocketAddr,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl HttpPushServerHandle {
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn addr(&self) -> SocketAddr {
        self.addr
    }
}

impl Drop for HttpPushServerHandle {
    fn drop(&mut self) {
        self.running.store(false, Ordering::Release);
        // Unblock the accept loop by connecting to the listener's address.
        // The loop will observe running == false after accept() returns and exit.
        if let Ok(unblock) = TcpStream::connect(self.addr) {
            let _ = unblock.shutdown(std::net::Shutdown::Both);
        }
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// Start the push listener. The accept thread is stopped and joined when the
/// handle is dropped, ensuring the TcpListener is released, mirroring the
/// JSON-RPC socket server's lifecycle.
pub(crate) fn start_http_push_server(
    api_tx: ApiRequestSender,
    listen_addr: SocketAddr,
) -> io::Result<HttpPushServerHandle> {
    let listener = TcpListener::bind(listen_addr)?;
    let addr = listener.local_addr()?;
    let running = Arc::new(AtomicBool::new(true));
    let thread_running = Arc::clone(&running);
    let thread = std::thread::Builder::new()
        .name("herdr-http-push".into())
        .spawn(move || {
            for stream in listener.incoming() {
                if !thread_running.load(Ordering::Acquire) {
                    break;
                }
                match stream {
                    Ok(stream) => {
                        let api_tx = api_tx.clone();
                        std::thread::Builder::new()
                            .name("herdr-http-push-conn".into())
                            .spawn(move || {
                                if let Err(err) = handle_connection(stream, &api_tx) {
                                    debug!(err = %err, "http push connection failed");
                                }
                            })
                            .ok();
                    }
                    Err(err) => {
                        warn!(err = %err, "http push listener accept failed");
                        break;
                    }
                }
            }
        })?;
    info!(addr = %addr, "agent push http server listening");
    Ok(HttpPushServerHandle {
        addr,
        running,
        thread: Some(thread),
    })
}

static REQUEST_COUNTER: AtomicU64 = AtomicU64::new(0);

struct HttpRequest {
    method: String,
    path: String,
    body: Vec<u8>,
}

struct HttpResponse {
    status: u16,
    reason: &'static str,
    body: String,
}

impl HttpResponse {
    fn json(status: u16, reason: &'static str, body: String) -> Self {
        Self {
            status,
            reason,
            body,
        }
    }

    fn error(status: u16, reason: &'static str, message: &str) -> Self {
        Self::json(
            status,
            reason,
            format!(
                "{{\"error\":{{\"code\":\"{reason}\",\"message\":{}}}}}",
                serde_json::to_string(message).unwrap_or_else(|_| "\"\"".into())
            ),
        )
    }

    fn write_to(&self, stream: &mut TcpStream) -> io::Result<()> {
        let head = format!(
            "HTTP/1.1 {} {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            self.status,
            self.reason,
            self.body.len()
        );
        stream.write_all(head.as_bytes())?;
        stream.write_all(self.body.as_bytes())?;
        stream.flush()
    }
}

fn handle_connection(mut stream: TcpStream, api_tx: &ApiRequestSender) -> io::Result<()> {
    // Set read timeout to bound total connection duration for headers and body
    stream.set_read_timeout(Some(CONNECTION_READ_TIMEOUT))?;

    let request = match read_request(&mut stream) {
        Ok(Some(request)) => request,
        Ok(None) => return Ok(()),
        Err(err) => {
            let response = HttpResponse::error(400, "bad_request", &err.to_string());
            let _ = response.write_to(&mut stream);
            return Err(err);
        }
    };

    let response = route_request(request, api_tx);
    let status = response.status;
    response.write_to(&mut stream)?;
    debug!(status, "http push request handled");
    Ok(())
}

fn read_request(stream: &mut TcpStream) -> io::Result<Option<HttpRequest>> {
    let mut reader = BufReader::new(stream.try_clone()?);

    let mut request_line = String::new();
    let bytes = reader.read_line(&mut request_line)?;
    if bytes == 0 {
        return Ok(None);
    }
    if request_line.len() > MAX_REQUEST_LINE_BYTES {
        return Err(io::Error::other("request line too long"));
    }
    let mut parts = request_line.split_whitespace();
    let method = parts.next().unwrap_or_default().to_string();
    let path = parts.next().unwrap_or_default().to_string();
    if method.is_empty() || path.is_empty() {
        return Err(io::Error::other("malformed request line"));
    }

    let mut content_length: Option<usize> = None;
    let mut header_bytes = 0usize;
    loop {
        let mut header_line = String::new();
        let bytes = reader.read_line(&mut header_line)?;
        if bytes == 0 {
            return Err(io::Error::other("unexpected end of headers"));
        }
        header_bytes += bytes;
        if header_bytes > MAX_HEADER_BYTES {
            return Err(io::Error::other("headers too long"));
        }
        let header_line = header_line.trim_end();
        if header_line.is_empty() {
            break;
        }
        if let Some((name, value)) = header_line.split_once(':') {
            if name.trim().eq_ignore_ascii_case("content-length") {
                let value = value.trim();
                content_length = Some(
                    value
                        .parse::<usize>()
                        .map_err(|_| io::Error::other("invalid content-length"))?,
                );
            }
        }
    }

    // A missing Content-Length means an empty body; body-hungry methods fail
    // later at JSON parsing, and bodyless methods (GET) still route.
    let content_length = content_length.unwrap_or(0);
    if content_length > MAX_BODY_BYTES {
        return Err(io::Error::other("body too large"));
    }

    let mut body = vec![0u8; content_length];
    reader.read_exact(&mut body)?;

    Ok(Some(HttpRequest { method, path, body }))
}

fn route_request(request: HttpRequest, api_tx: &ApiRequestSender) -> HttpResponse {
    if request.path != AGENT_REPORT_PATH {
        return HttpResponse::error(404, "not_found", "unknown path");
    }
    if !request.method.eq_ignore_ascii_case("POST") {
        return HttpResponse::error(405, "method_not_allowed", "use POST");
    }

    let params: PaneReportAgentParams = match serde_json::from_slice(&request.body) {
        Ok(params) => params,
        Err(err) => {
            return HttpResponse::error(400, "bad_request", &format!("invalid body: {err}"));
        }
    };

    let id = format!(
        "http-push-{}",
        REQUEST_COUNTER.fetch_add(1, Ordering::Relaxed)
    );
    let response = crate::api::server::dispatch_to_app_with_timeout(
        Request {
            id,
            method: Method::PaneReportAgent(params),
        },
        api_tx,
        Some(RESPONSE_TIMEOUT),
    );

    json_rpc_response_to_http(response)
}

fn json_rpc_response_to_http(response: String) -> HttpResponse {
    match serde_json::from_str::<serde_json::Value>(&response) {
        Ok(value) => {
            if value.get("error").is_some() {
                let code = value
                    .pointer("/error/code")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or_default();
                match code {
                    "pane_not_found" => HttpResponse::json(404, "not_found", response),
                    "invalid_agent" => HttpResponse::json(400, "bad_request", response),
                    "server_unavailable" => {
                        HttpResponse::json(503, "service_unavailable", response)
                    }
                    _ => HttpResponse::json(500, "internal_error", response),
                }
            } else {
                HttpResponse::json(200, "ok", response)
            }
        }
        Err(_) => HttpResponse::json(500, "internal_error", response),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;

    struct FakeApp {
        api_tx: ApiRequestSender,
        _thread: std::thread::JoinHandle<()>,
    }

    impl FakeApp {
        /// Reply to every `pane.report_agent` request with a canned JSON-RPC
        /// response and record the dispatched params.
        fn new(
            respond_with: impl Fn(usize) -> String + Send + 'static,
        ) -> (Self, mpsc::Receiver<PaneReportAgentParams>) {
            let (api_tx, mut api_rx) =
                tokio::sync::mpsc::unbounded_channel::<crate::api::ApiRequestMessage>();
            let (seen_tx, seen_rx) = mpsc::channel();
            let thread = std::thread::spawn(move || {
                while let Some(message) = api_rx.blocking_recv() {
                    if let Method::PaneReportAgent(params) = message.request.method {
                        seen_tx.send(params.clone()).ok();
                        let response = respond_with(params.pane_id.len());
                        message.respond_to.send(response).ok();
                    }
                }
            });
            (
                Self {
                    api_tx,
                    _thread: thread,
                },
                seen_rx,
            )
        }
    }

    fn http_request(stream: &mut TcpStream, raw: &str) -> String {
        use std::io::Read;
        stream.write_all(raw.as_bytes()).unwrap();
        stream.flush().unwrap();
        let mut response = String::new();
        let _ = stream.read_to_string(&mut response);
        response
    }

    fn post_body(pane_id: &str, state: &str, seq: u64) -> String {
        format!(
            r#"{{"pane_id":"{pane_id}","source":"herdr:ziki","agent":"ziki","state":"{state}","message":null,"seq":{seq},"agent_session_id":"goal-1","agent_session_path":"/wd/.ziki"}}"#
        )
    }

    fn post_request(addr: SocketAddr, body: &str) -> String {
        format!(
            "POST {AGENT_REPORT_PATH} HTTP/1.1\r\nHost: {addr}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        )
    }

    fn start_listener() -> (
        HttpPushServerHandle,
        FakeApp,
        mpsc::Receiver<PaneReportAgentParams>,
    ) {
        let (app, seen) = FakeApp::new(|_| r#"{"id":"x","result":{"type":"ok"}}"#.to_string());
        let handle = start_http_push_server(app.api_tx.clone(), "127.0.0.1:0".parse().unwrap())
            .expect("listener should bind an ephemeral port");
        (handle, app, seen)
    }

    fn status_of(response: &str) -> u16 {
        response
            .split_whitespace()
            .nth(1)
            .and_then(|code| code.parse().ok())
            .unwrap_or_default()
    }

    fn body_of(response: &str) -> &str {
        response
            .split_once("\r\n\r\n")
            .map(|(_, body)| body)
            .unwrap_or_default()
    }

    #[test]
    fn http_push_accepts_contract_report() {
        let (handle, _app, seen) = start_listener();
        let mut stream = TcpStream::connect(handle.addr()).unwrap();
        let body = post_body("pane-XYZ", "working", 3);
        let response = http_request(&mut stream, &post_request(handle.addr(), &body));

        assert_eq!(status_of(&response), 200, "response: {response}");
        assert!(
            body_of(&response).contains("\"result\""),
            "response: {response}"
        );
        let dispatched = seen
            .recv_timeout(Duration::from_secs(2))
            .expect("report dispatched");
        assert_eq!(dispatched.pane_id, "pane-XYZ");
        assert_eq!(dispatched.source, "herdr:ziki");
        assert_eq!(dispatched.agent, "ziki");
        assert_eq!(
            dispatched.state,
            crate::api::schema::PaneAgentState::Working
        );
        assert_eq!(dispatched.seq, Some(3));
        assert_eq!(dispatched.agent_session_id.as_deref(), Some("goal-1"));
        assert_eq!(dispatched.agent_session_path.as_deref(), Some("/wd/.ziki"));
    }

    #[test]
    fn http_push_unknown_pane_not_found() {
        let (app, _seen) = FakeApp::new(|pane_id_len| {
            if pane_id_len > 0 {
                r#"{"id":"x","error":{"code":"pane_not_found","message":"pane not found"}}"#
                    .to_string()
            } else {
                r#"{"id":"x","result":{"type":"ok"}}"#.to_string()
            }
        });
        let handle =
            start_http_push_server(app.api_tx.clone(), "127.0.0.1:0".parse().unwrap()).unwrap();
        let mut stream = TcpStream::connect(handle.addr()).unwrap();
        let body = post_body("missing-pane", "idle", 1);
        let response = http_request(&mut stream, &post_request(handle.addr(), &body));
        assert_eq!(status_of(&response), 404, "response: {response}");
        assert!(body_of(&response).contains("pane_not_found"));
    }

    #[test]
    fn http_push_malformed_body_bad_request() {
        let (handle, _app, _seen) = start_listener();
        let mut stream = TcpStream::connect(handle.addr()).unwrap();
        let response = http_request(
            &mut stream,
            &post_request(handle.addr(), "this is not json"),
        );
        assert_eq!(status_of(&response), 400, "response: {response}");
    }

    #[test]
    fn http_push_routing_statuses() {
        let (handle, _app, _seen) = start_listener();

        // Unknown path.
        let mut stream = TcpStream::connect(handle.addr()).unwrap();
        let response = http_request(
            &mut stream,
            &format!(
                "POST /api/v1/other HTTP/1.1\r\nHost: {}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                handle.addr()
            ),
        );
        assert_eq!(status_of(&response), 404, "response: {response}");

        // Wrong method on the report path.
        let mut stream = TcpStream::connect(handle.addr()).unwrap();
        let response = http_request(
            &mut stream,
            &format!(
                "GET {AGENT_REPORT_PATH} HTTP/1.1\r\nHost: {}\r\nConnection: close\r\n\r\n",
                handle.addr()
            ),
        );
        assert_eq!(status_of(&response), 405, "response: {response}");
    }

    #[test]
    fn http_push_survives_bad_requests() {
        let (handle, _app, _seen) = start_listener();

        for raw in [
            "GARBAGE\r\n\r\n".to_string(),
            format!("POST {AGENT_REPORT_PATH} HTTP/1.1\r\nContent-Length: 999999999\r\n\r\n"),
            format!("POST {AGENT_REPORT_PATH} HTTP/1.1\r\n\r\n"),
        ] {
            let mut stream = TcpStream::connect(handle.addr()).unwrap();
            let response = http_request(&mut stream, &raw);
            assert!(
                (400..500).contains(&status_of(&response)),
                "expected client error for {raw:?}, got: {response}"
            );
        }

        // The listener still serves a valid request afterwards.
        let mut stream = TcpStream::connect(handle.addr()).unwrap();
        let body = post_body("pane-1", "blocked", 1);
        let response = http_request(&mut stream, &post_request(handle.addr(), &body));
        assert_eq!(status_of(&response), 200, "response: {response}");
    }

    #[test]
    fn http_push_bind_conflict_is_reported() {
        let (app, _seen) = FakeApp::new(|_| r#"{"id":"x","result":{"type":"ok"}}"#.to_string());
        let first = start_http_push_server(app.api_tx.clone(), "127.0.0.1:0".parse().unwrap())
            .expect("first listener binds");
        let conflict = start_http_push_server(app.api_tx.clone(), first.addr());
        assert!(conflict.is_err(), "second bind on the same port must fail");
    }

    #[test]
    fn http_push_parses_listen_addr() {
        assert_eq!(
            parse_listen_addr("127.0.0.1:7878", "test").map(|addr| addr.to_string()),
            Some("127.0.0.1:7878".to_string())
        );
        assert!(parse_listen_addr("localhost:7878", "test").is_some());
        assert!(parse_listen_addr("not an address", "test").is_none());
        assert!(parse_listen_addr("", "test").is_none());
    }

    #[test]
    fn http_push_handle_drop_releases_port() {
        let (app, _seen) = FakeApp::new(|_| r#"{"id":"x","result":{"type":"ok"}}"#.to_string());
        let handle =
            start_http_push_server(app.api_tx.clone(), "127.0.0.1:0".parse().unwrap()).unwrap();
        let addr = handle.addr();
        drop(handle);
        // If the port was not released, this bind will fail.
        let rebound = TcpListener::bind(addr);
        assert!(
            rebound.is_ok(),
            "rebinding immediately after drop should succeed"
        );
    }
}
