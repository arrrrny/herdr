//! End-to-end coverage for the CPU scheduling priority of a freshly started
//! agent.
//!
//! The `tests/cli/` suite only runs on Linux, so the acceptance behaviors live
//! here in a Unix-gated file that runs on every Unix CI job and can be
//! reproduced on a developer machine.

#![cfg(unix)]

pub mod support;

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::{Mutex, MutexGuard, OnceLock};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use portable_pty::{native_pty_system, Child, CommandBuilder, MasterPty, PtySize};
use support::{
    cleanup_test_base, register_runtime_dir, register_spawned_herdr_pid,
    unregister_spawned_herdr_pid,
};

fn unique_test_dir() -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    PathBuf::from(format!("/tmp/hprio-{}-{nanos}", std::process::id()))
}

struct SpawnedHerdr {
    _master: Box<dyn MasterPty + Send>,
    child: Box<dyn Child + Send + Sync>,
}

impl SpawnedHerdr {
    fn pid(&self) -> Option<u32> {
        self.child.process_id()
    }
}

impl Drop for SpawnedHerdr {
    fn drop(&mut self) {
        let pid = self.child.process_id();
        support::stop_spawned_herdr(&mut *self.child);

        if let Some(pid) = pid {
            let deadline = Instant::now() + Duration::from_secs(10);
            while Instant::now() < deadline {
                let mut status = 0;
                let result =
                    unsafe { libc::waitpid(pid as libc::pid_t, &mut status, libc::WNOHANG) };
                if result == pid as libc::pid_t || result == -1 {
                    break;
                }
                thread::sleep(Duration::from_millis(20));
            }

            unregister_spawned_herdr_pid(Some(pid));
        }
    }
}

fn cleanup_spawned_herdr(spawned: SpawnedHerdr, base: PathBuf) {
    drop(spawned);
    cleanup_test_base(&base);
}

fn test_lock() -> MutexGuard<'static, ()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn wait_for_socket(path: &Path, timeout: Duration) {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if path.exists() && std::os::unix::net::UnixStream::connect(path).is_ok() {
            return;
        }
        thread::sleep(Duration::from_millis(25));
    }
    panic!("socket did not appear at {}", path.display());
}

/// The configuration directory the Herdr binary under test reads.
///
/// A debug build keeps its configuration in `herdr-dev`, a release build in
/// `herdr`.
fn app_dir_name() -> &'static str {
    if cfg!(debug_assertions) {
        "herdr-dev"
    } else {
        "herdr"
    }
}

/// Spawns a Herdr server with the given configuration and a private `PATH`.
///
/// The private `PATH` holds the fake agent and a `nice` shim, so the launch
/// line resolves exactly the way it would on a real machine.
fn spawn_herdr(
    config_home: &Path,
    runtime_dir: &Path,
    socket_path: &Path,
    path_override: &Path,
    config_toml: &str,
) -> SpawnedHerdr {
    spawn_herdr_at_priority(
        config_home,
        runtime_dir,
        socket_path,
        path_override,
        config_toml,
        0,
    )
}

/// Spawns a Herdr server that runs at the given scheduling priority.
///
/// The pane's shell inherits the server's priority, so this is how a test
/// reproduces a host whose shell does not run at the default priority.
fn spawn_herdr_at_priority(
    config_home: &Path,
    runtime_dir: &Path,
    socket_path: &Path,
    path_override: &Path,
    config_toml: &str,
    priority: i32,
) -> SpawnedHerdr {
    fs::create_dir_all(config_home.join(app_dir_name())).unwrap();
    fs::create_dir_all(runtime_dir).unwrap();
    register_runtime_dir(runtime_dir);
    fs::write(
        config_home.join(app_dir_name()).join("config.toml"),
        config_toml,
    )
    .unwrap();

    let pair = native_pty_system()
        .openpty(PtySize {
            rows: 24,
            cols: 80,
            pixel_width: 0,
            pixel_height: 0,
        })
        .unwrap();

    let mut cmd = if priority == 0 {
        let mut cmd = CommandBuilder::new(env!("CARGO_BIN_EXE_herdr"));
        cmd.arg("server");
        cmd
    } else {
        let mut cmd = CommandBuilder::new(nice_binary());
        cmd.arg("-n");
        cmd.arg(priority.to_string());
        cmd.arg(env!("CARGO_BIN_EXE_herdr"));
        cmd.arg("server");
        cmd
    };
    support::isolate_herdr_test_process(&mut cmd);
    cmd.env("XDG_CONFIG_HOME", config_home);
    cmd.env("XDG_RUNTIME_DIR", runtime_dir);
    cmd.env("HERDR_SOCKET_PATH", socket_path);
    cmd.env_remove("HERDR_CLIENT_SOCKET_PATH");
    cmd.env("SHELL", "/bin/sh");
    cmd.env_remove("HERDR_ENV");
    // An inherited agent socket points at a live Herdr session. The spawned
    // server would share that socket with it, which stalls the startup of
    // every server the suite runs in parallel.
    cmd.env_remove("SSH_AUTH_SOCK");
    cmd.env("PATH", path_override);

    let child = pair.slave.spawn_command(cmd).unwrap();
    register_spawned_herdr_pid(child.process_id());
    SpawnedHerdr {
        _master: pair.master,
        child,
    }
}

fn run_cli(socket_path: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_herdr"))
        .args(args)
        .env("HERDR_SOCKET_PATH", socket_path)
        .env_remove("HERDR_CLIENT_SOCKET_PATH")
        .output()
        .expect("run the herdr CLI")
}

fn run_cli_json(socket_path: &Path, args: &[&str]) -> serde_json::Value {
    let output = run_cli(socket_path, args);
    assert!(
        output.status.success(),
        "command failed: herdr {}\nstatus: {:?}\nstderr: {}",
        args.join(" "),
        output.status.code(),
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).expect("the CLI must answer with JSON")
}

/// Installs a fake `pi` agent and a `nice` shim into a private bin directory.
///
/// The fake agent records the arguments it was called with, its own process
/// id, and the process id of a child it forks, so a test can read the
/// operating system's view of the priority independently of Herdr.
///
/// Without the `nice` shim the private directory holds only the agent, which
/// is the search path of a host that has no priority program at all.
fn install_fake_agent(
    base: &Path,
    args_file: &Path,
    pid_file: &Path,
    child_pid_file: &Path,
    install_priority_program: bool,
) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;

    let bin = base.join("bin");
    fs::create_dir_all(&bin).unwrap();
    let fake_pi = bin.join("pi");
    fs::write(
        &fake_pi,
        format!(
            "#!/bin/sh\nprintf '%s\\n' \"$@\" >> '{args}'\nprintf '%s\\n' \"$$\" > '{pid}'\nexport HERDR_AGENT=pi\n'{herdr}' pane report-agent \"$HERDR_PANE_ID\" --source custom:fake-pi --agent pi --state idle >/dev/null\n( while : ; do /bin/sleep 1 ; done ) &\nprintf '%s\\n' \"$!\" > '{child}'\nwhile IFS= read -r _prompt; do\n  '{herdr}' pane report-agent \"$HERDR_PANE_ID\" --source custom:fake-pi --agent pi --state working >/dev/null\n  '{herdr}' pane report-agent \"$HERDR_PANE_ID\" --source custom:fake-pi --agent pi --state idle >/dev/null\ndone\n",
            args = args_file.display(),
            pid = pid_file.display(),
            child = child_pid_file.display(),
            herdr = env!("CARGO_BIN_EXE_herdr"),
        ),
    )
    .unwrap();
    fs::set_permissions(&fake_pi, fs::Permissions::from_mode(0o755)).unwrap();

    if install_priority_program {
        std::os::unix::fs::symlink(nice_binary(), bin.join("nice")).unwrap();
    }
    bin
}

/// The `nice` binary of the host.
fn nice_binary() -> &'static Path {
    ["/usr/bin/nice", "/bin/nice"]
        .iter()
        .map(Path::new)
        .find(|candidate| candidate.exists())
        .expect("the host must provide a nice binary")
}

/// Reads a file the fake agent writes, waiting for it to appear.
fn wait_for_agent_file(path: &Path, timeout: Duration) -> String {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if let Ok(contents) = fs::read_to_string(path) {
            let contents = contents.trim().to_string();
            if !contents.is_empty() {
                return contents;
            }
        }
        thread::sleep(Duration::from_millis(25));
    }
    panic!("{} was never written", path.display());
}

/// Reads a file the fake agent writes, waiting until it holds the expected
/// content.
///
/// The agent can be detected before its first write reaches the disk, so a
/// direct read races with the launch.
fn wait_for_agent_file_content(path: &Path, expected: &str, timeout: Duration) -> String {
    let deadline = Instant::now() + timeout;
    let mut seen = String::new();
    while Instant::now() < deadline {
        if let Ok(contents) = fs::read_to_string(path) {
            if contents == expected {
                return contents;
            }
            seen = contents;
        }
        thread::sleep(Duration::from_millis(25));
    }
    panic!(
        "{} never held {expected:?}, last held {seen:?}",
        path.display()
    );
}

/// Reads the CPU scheduling priority the operating system applied to a
/// process, independently of anything Herdr reports.
fn applied_priority(pid: u32) -> Option<i32> {
    let output = Command::new("ps")
        .args(["-o", "ni=", "-p", &pid.to_string()])
        .output()
        .ok()?;
    String::from_utf8_lossy(&output.stdout).trim().parse().ok()
}

/// The priority the pane's shell runs at, which is the baseline a launch line
/// starts from.
///
/// The pane's shell is a child of the server, so it runs at the server's
/// priority.
fn launch_baseline(server: &SpawnedHerdr) -> i32 {
    let pid = server.pid().expect("the server must have a pid");
    applied_priority(pid).expect("the host must report a priority")
}

/// The adjustment a requested priority puts on the launch line.
///
/// GNU `nice -n` sets the priority outright, so the requested value is passed
/// through. macOS `nice -n` adjusts the caller's priority, so the difference
/// between the request and the priority the pane's shell runs at is applied.
fn expected_adjustment(nice: i32, baseline: i32) -> String {
    if cfg!(target_os = "macos") {
        (nice - baseline).to_string()
    } else {
        nice.to_string()
    }
}

/// Creates a workspace and returns the id of its root pane.
fn create_root_pane(socket_path: &Path, base: &Path) -> String {
    let created = run_cli_json(
        socket_path,
        &["workspace", "create", "--cwd", base.to_str().unwrap()],
    );
    created["result"]["root_pane"]["pane_id"]
        .as_str()
        .unwrap()
        .to_string()
}

/// Reads the foreground processes of a pane.
fn foreground_processes(socket_path: &Path, pane_id: &str) -> Vec<serde_json::Value> {
    let info = run_cli_json(socket_path, &["pane", "process-info", "--pane", pane_id]);
    info["result"]["process_info"]["foreground_processes"]
        .as_array()
        .expect("the pane must report its foreground processes")
        .clone()
}

/// Waits until the pane reports the given process and returns its entry.
fn wait_for_reported_process(
    socket_path: &Path,
    pane_id: &str,
    pid: u32,
    timeout: Duration,
) -> serde_json::Value {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        let processes = foreground_processes(socket_path, pane_id);
        if let Some(process) = processes
            .iter()
            .find(|process| process["pid"].as_u64() == Some(u64::from(pid)))
        {
            return process.clone();
        }
        thread::sleep(Duration::from_millis(50));
    }
    panic!(
        "the pane never reported the process {pid}: {:?}",
        foreground_processes(socket_path, pane_id)
    );
}

struct AgentFixture {
    base: PathBuf,
    socket_path: PathBuf,
    args_file: PathBuf,
    pid_file: PathBuf,
    child_pid_file: PathBuf,
    bin: PathBuf,
}

impl AgentFixture {
    fn new() -> AgentFixture {
        AgentFixture::with_priority_program(true)
    }

    /// A fixture whose private search path holds no priority program.
    fn without_priority_program() -> AgentFixture {
        AgentFixture::with_priority_program(false)
    }

    fn with_priority_program(install_priority_program: bool) -> AgentFixture {
        let base = unique_test_dir();
        let runtime_dir = base.join("runtime");
        let socket_path = runtime_dir.join("herdr.sock");
        let args_file = base.join("pi-args");
        let pid_file = base.join("pi-pid");
        let child_pid_file = base.join("pi-child-pid");
        let bin = install_fake_agent(
            &base,
            &args_file,
            &pid_file,
            &child_pid_file,
            install_priority_program,
        );
        AgentFixture {
            base,
            socket_path,
            args_file,
            pid_file,
            child_pid_file,
            bin,
        }
    }

    fn spawn(&self) -> SpawnedHerdr {
        let herdr = spawn_herdr(
            &self.base.join("config"),
            &self.base.join("runtime"),
            &self.socket_path,
            &self.bin,
            "onboarding = false\n",
        );
        wait_for_socket(&self.socket_path, Duration::from_secs(10));
        herdr
    }
}

#[test]
fn agent_start_applies_a_requested_priority() {
    let _lock = test_lock();
    let fixture = AgentFixture::new();
    let herdr = fixture.spawn();
    let socket_path = fixture.socket_path.clone();
    let pane_id = create_root_pane(&socket_path, &fixture.base);

    let started = run_cli_json(
        &socket_path,
        &[
            "agent",
            "start",
            "worker",
            "--kind",
            "pi",
            "--pane",
            &pane_id,
            "--timeout",
            "8000",
            "--nice",
            "19",
        ],
    );

    assert_eq!(
        started["result"]["argv"],
        serde_json::json!([
            "nice",
            "-n",
            expected_adjustment(19, launch_baseline(&herdr)),
            "pi"
        ])
    );

    let agent_pid: u32 = wait_for_agent_file(&fixture.pid_file, Duration::from_secs(10))
        .parse()
        .unwrap();
    let child_pid: u32 = wait_for_agent_file(&fixture.child_pid_file, Duration::from_secs(10))
        .parse()
        .unwrap();

    // The operating system applied the requested priority to the agent ...
    assert_eq!(applied_priority(agent_pid), Some(19));
    // ... and every child the agent forks inherits it.
    assert_eq!(applied_priority(child_pid), Some(19));

    cleanup_spawned_herdr(herdr, fixture.base);
}

#[test]
fn the_requested_priority_is_reached_from_a_shell_at_another_priority() {
    // The pane's shell does not have to run at the default priority, and
    // `nice` adjusts the caller's priority on some platforms. The agent must
    // still land on the requested priority.
    let _lock = test_lock();
    let fixture = AgentFixture::new();
    let shell_priority = 5;
    let host_priority =
        applied_priority(std::process::id()).expect("the host must report a priority");
    // macOS `nice -n` adjusts the caller's priority, so the server's
    // priority is reached with an adjustment from the host's. GNU `nice -n`
    // sets it outright.
    let adjustment = if cfg!(target_os = "macos") {
        shell_priority - host_priority
    } else {
        shell_priority
    };
    let herdr = spawn_herdr_at_priority(
        &fixture.base.join("config"),
        &fixture.base.join("runtime"),
        &fixture.socket_path,
        &fixture.bin,
        "onboarding = false\n",
        adjustment,
    );
    wait_for_socket(&fixture.socket_path, Duration::from_secs(10));
    let socket_path = fixture.socket_path.clone();
    let pane_id = create_root_pane(&socket_path, &fixture.base);

    let started = run_cli_json(
        &socket_path,
        &[
            "agent",
            "start",
            "worker",
            "--kind",
            "pi",
            "--pane",
            &pane_id,
            "--timeout",
            "8000",
            "--nice",
            "19",
        ],
    );

    let baseline = launch_baseline(&herdr);
    assert_eq!(
        baseline, shell_priority,
        "the pane's shell must run at another priority"
    );
    assert_eq!(
        started["result"]["argv"],
        serde_json::json!(["nice", "-n", expected_adjustment(19, baseline), "pi"])
    );

    let agent_pid: u32 = wait_for_agent_file(&fixture.pid_file, Duration::from_secs(10))
        .parse()
        .unwrap();

    // The agent lands on the requested priority, not on the shell's priority
    // plus the request.
    assert_eq!(applied_priority(agent_pid), Some(19));

    cleanup_spawned_herdr(herdr, fixture.base);
}

#[test]
fn agent_start_without_a_priority_keeps_the_launch_line_unchanged() {
    let _lock = test_lock();
    let fixture = AgentFixture::new();
    let herdr = fixture.spawn();
    let socket_path = fixture.socket_path.clone();
    let pane_id = create_root_pane(&socket_path, &fixture.base);

    let started = run_cli_json(
        &socket_path,
        &[
            "agent",
            "start",
            "worker",
            "--kind",
            "pi",
            "--pane",
            &pane_id,
            "--timeout",
            "8000",
        ],
    );

    assert_eq!(started["result"]["argv"], serde_json::json!(["pi"]));

    let agent_pid: u32 = wait_for_agent_file(&fixture.pid_file, Duration::from_secs(10))
        .parse()
        .unwrap();
    let baseline = launch_baseline(&herdr);
    assert_eq!(applied_priority(agent_pid), Some(baseline));

    cleanup_spawned_herdr(herdr, fixture.base);
}

#[test]
fn agent_start_honors_the_configured_default_and_lets_the_flag_override_it() {
    let _lock = test_lock();
    let fixture = AgentFixture::new();
    let herdr = spawn_herdr(
        &fixture.base.join("config"),
        &fixture.base.join("runtime"),
        &fixture.socket_path,
        &fixture.bin,
        "onboarding = false\n[agent.start]\nnice = 19\n",
    );
    wait_for_socket(&fixture.socket_path, Duration::from_secs(10));
    let socket_path = fixture.socket_path.clone();

    let defaulted_pane = create_root_pane(&socket_path, &fixture.base);
    let defaulted = run_cli_json(
        &socket_path,
        &[
            "agent",
            "start",
            "defaulted",
            "--kind",
            "pi",
            "--pane",
            &defaulted_pane,
            "--timeout",
            "8000",
        ],
    );
    assert_eq!(
        defaulted["result"]["argv"],
        serde_json::json!([
            "nice",
            "-n",
            expected_adjustment(19, launch_baseline(&herdr)),
            "pi"
        ])
    );

    let overridden_pane = create_root_pane(&socket_path, &fixture.base);
    let overridden = run_cli_json(
        &socket_path,
        &[
            "agent",
            "start",
            "overridden",
            "--kind",
            "pi",
            "--pane",
            &overridden_pane,
            "--timeout",
            "8000",
            "--nice",
            "5",
        ],
    );
    assert_eq!(
        overridden["result"]["argv"],
        serde_json::json!([
            "nice",
            "-n",
            expected_adjustment(5, launch_baseline(&herdr)),
            "pi"
        ])
    );

    // An explicit zero means "today's behavior" and wins over the default.
    let disabled_pane = create_root_pane(&socket_path, &fixture.base);
    let disabled = run_cli_json(
        &socket_path,
        &[
            "agent",
            "start",
            "disabled",
            "--kind",
            "pi",
            "--pane",
            &disabled_pane,
            "--timeout",
            "8000",
            "--nice",
            "0",
        ],
    );
    assert_eq!(disabled["result"]["argv"], serde_json::json!(["pi"]));

    cleanup_spawned_herdr(herdr, fixture.base);
}

#[test]
fn agent_start_rejects_an_out_of_range_priority() {
    let _lock = test_lock();
    let fixture = AgentFixture::new();
    let herdr = fixture.spawn();
    let socket_path = fixture.socket_path.clone();
    let pane_id = create_root_pane(&socket_path, &fixture.base);

    let rejected = run_cli(
        &socket_path,
        &[
            "agent", "start", "invalid", "--kind", "pi", "--pane", &pane_id, "--nice", "20",
        ],
    );

    assert!(!rejected.status.success());
    let stderr = String::from_utf8_lossy(&rejected.stderr);
    assert!(stderr.contains("--nice"), "must name the option: {stderr}");
    assert!(stderr.contains("20"), "must name the value: {stderr}");
    assert!(
        stderr.contains("-20..=19"),
        "must name the supported range: {stderr}"
    );

    // Nothing was started.
    let listed = run_cli_json(&socket_path, &["agent", "list"]);
    assert!(
        listed["result"]["agents"].as_array().unwrap().is_empty(),
        "no agent may have been started: {listed}"
    );
    assert!(!fixture.pid_file.exists(), "the agent must never have run");

    cleanup_spawned_herdr(herdr, fixture.base);
}

#[test]
fn agent_start_with_a_priority_still_detects_the_agent() {
    let _lock = test_lock();
    let fixture = AgentFixture::new();
    let herdr = fixture.spawn();
    let socket_path = fixture.socket_path.clone();
    let pane_id = create_root_pane(&socket_path, &fixture.base);

    let started = run_cli_json(
        &socket_path,
        &[
            "agent",
            "start",
            "worker",
            "--kind",
            "pi",
            "--pane",
            &pane_id,
            "--timeout",
            "8000",
            "--nice",
            "19",
        ],
    );

    assert_eq!(started["result"]["type"], "agent_started");
    assert_eq!(started["result"]["agent"]["agent"], "pi");
    assert_eq!(started["result"]["agent"]["name"], "worker");
    assert_eq!(started["result"]["agent"]["interactive_ready"], true);

    let listed = run_cli_json(&socket_path, &["agent", "list"]);
    assert_eq!(listed["result"]["agents"][0]["name"], "worker");

    let prompted = run_cli(
        &socket_path,
        &["agent", "prompt", "worker", "hello there", "--wait"],
    );
    assert!(
        prompted.status.success(),
        "a priority-requested agent must still accept a prompt: {}",
        String::from_utf8_lossy(&prompted.stderr)
    );

    cleanup_spawned_herdr(herdr, fixture.base);
}

#[test]
fn agent_start_passes_trailing_agent_arguments_with_a_priority() {
    let _lock = test_lock();
    let fixture = AgentFixture::new();
    let herdr = fixture.spawn();
    let socket_path = fixture.socket_path.clone();
    let pane_id = create_root_pane(&socket_path, &fixture.base);

    let started = run_cli_json(
        &socket_path,
        &[
            "agent",
            "start",
            "worker",
            "--kind",
            "pi",
            "--pane",
            &pane_id,
            "--timeout",
            "8000",
            "--nice",
            "19",
            "--",
            "--name",
            "scratch",
            "--no-session",
        ],
    );

    assert_eq!(
        started["result"]["argv"],
        serde_json::json!([
            "nice",
            "-n",
            expected_adjustment(19, launch_baseline(&herdr)),
            "pi",
            "--name",
            "scratch",
            "--no-session"
        ])
    );
    assert_eq!(
        wait_for_agent_file_content(
            &fixture.args_file,
            "--name\nscratch\n--no-session\n",
            Duration::from_secs(10)
        ),
        "--name\nscratch\n--no-session\n"
    );

    cleanup_spawned_herdr(herdr, fixture.base);
}

#[test]
fn agent_start_reports_a_missing_priority_program() {
    let _lock = test_lock();
    let fixture = AgentFixture::without_priority_program();
    let herdr = fixture.spawn();
    let socket_path = fixture.socket_path.clone();
    let pane_id = create_root_pane(&socket_path, &fixture.base);

    let rejected = run_cli(
        &socket_path,
        &[
            "agent", "start", "worker", "--kind", "pi", "--pane", &pane_id, "--nice", "19",
        ],
    );

    assert!(!rejected.status.success());
    let stderr = String::from_utf8_lossy(&rejected.stderr);
    assert!(
        stderr.contains("agent_nice_unavailable"),
        "must name the error: {stderr}"
    );
    assert!(
        stderr.contains("nice"),
        "must name the priority program: {stderr}"
    );

    // Nothing was started.
    let listed = run_cli_json(&socket_path, &["agent", "list"]);
    assert!(
        listed["result"]["agents"].as_array().unwrap().is_empty(),
        "no agent may have been started: {listed}"
    );
    assert!(!fixture.pid_file.exists(), "the agent must never have run");

    cleanup_spawned_herdr(herdr, fixture.base);
}

#[test]
fn a_launch_without_a_priority_does_not_need_the_priority_program() {
    // The priority program is only required when a priority is in effect.
    let _lock = test_lock();
    let fixture = AgentFixture::without_priority_program();
    let herdr = fixture.spawn();
    let socket_path = fixture.socket_path.clone();
    let pane_id = create_root_pane(&socket_path, &fixture.base);

    let started = run_cli_json(
        &socket_path,
        &[
            "agent",
            "start",
            "worker",
            "--kind",
            "pi",
            "--pane",
            &pane_id,
            "--timeout",
            "8000",
        ],
    );

    assert_eq!(started["result"]["argv"], serde_json::json!(["pi"]));

    cleanup_spawned_herdr(herdr, fixture.base);
}

#[test]
fn pane_process_info_reports_the_applied_priority() {
    let _lock = test_lock();
    let fixture = AgentFixture::new();
    let herdr = fixture.spawn();
    let socket_path = fixture.socket_path.clone();
    let pane_id = create_root_pane(&socket_path, &fixture.base);

    let started = run_cli_json(
        &socket_path,
        &[
            "agent",
            "start",
            "worker",
            "--kind",
            "pi",
            "--pane",
            &pane_id,
            "--timeout",
            "8000",
            "--nice",
            "19",
        ],
    );
    assert_eq!(started["result"]["type"], "agent_started");

    let agent_pid: u32 = wait_for_agent_file(&fixture.pid_file, Duration::from_secs(10))
        .parse()
        .unwrap();

    let reported =
        wait_for_reported_process(&socket_path, &pane_id, agent_pid, Duration::from_secs(10));
    assert_eq!(reported["nice"], 19);

    // The reported priority matches the one the operating system applied.
    assert_eq!(applied_priority(agent_pid), Some(19));

    cleanup_spawned_herdr(herdr, fixture.base);
}
