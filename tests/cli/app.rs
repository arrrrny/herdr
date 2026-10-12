use super::harness::*;

fn run_app_cli(base: &Path, socket_path: &Path, args: &[&str]) -> std::process::Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_herdr"));
    command
        .args(args)
        .env("HERDR_SOCKET_PATH", socket_path)
        .env("XDG_STATE_HOME", base.join("state"))
        .env("XDG_CONFIG_HOME", base.join("config"))
        .env_remove("HERDR_SESSION")
        .env_remove("HERDR_CLIENT_SOCKET_PATH")
        .env_remove("HERDR_ENV")
        .output()
        .unwrap()
}

#[test]
fn app_focus_without_a_machine_uses_the_local_focus_path() {
    let base = unique_test_dir();
    fs::create_dir_all(&base).unwrap();
    let socket_path = base.join("herdr.sock");
    let listener = UnixListener::bind(&socket_path).unwrap();

    let server = thread::spawn(move || {
        let (mut stream, line) = accept_fake_cli_operation(&listener);
        stream
            .write_all(br#"{"id":"cli:app:focus","result":{"type":"ok"}}"#)
            .unwrap();
        stream.write_all(b"\n").unwrap();
        stream.flush().unwrap();
        line
    });

    let output = run_app_cli(&base, &socket_path, &["app", "focus", "--pane", "1-1"]);

    let line = server.join().unwrap();
    let request: serde_json::Value = serde_json::from_str(&line).unwrap();
    assert_eq!(request["method"], "pane.focus");
    assert_eq!(request["params"]["pane_id"], "1-1");
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&output.stdout), "focused (local)\n");
    cleanup_test_base(&base);
}

#[test]
fn app_focus_rejects_an_unknown_machine_without_touching_the_socket() {
    let base = unique_test_dir();
    fs::create_dir_all(&base).unwrap();
    let socket_path = base.join("herdr.sock");

    let output = run_app_cli(
        &base,
        &socket_path,
        &[
            "app",
            "focus",
            "--machine",
            "no-such-machine-issue-73",
            "--pane",
            "1-1",
        ],
    );

    assert_eq!(output.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("unknown machine 'no-such-machine-issue-73'"),
        "stderr: {stderr}"
    );
    cleanup_test_base(&base);
}

#[test]
fn app_focus_requires_exactly_one_target() {
    let base = unique_test_dir();
    fs::create_dir_all(&base).unwrap();
    let socket_path = base.join("herdr.sock");

    for args in [
        &["app", "focus"][..],
        &["app", "focus", "--pane", "1-1", "--tab", "1-2"][..],
        &["app", "focus", "--machine"][..],
        &["app", "focus", "--pane", "1-1", "--json"][..],
    ] {
        let output = run_app_cli(&base, &socket_path, args);
        assert_eq!(output.status.code(), Some(2), "{args:?}");
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(stderr.contains("herdr app focus"), "{args:?}: {stderr}");
    }
    cleanup_test_base(&base);
}
