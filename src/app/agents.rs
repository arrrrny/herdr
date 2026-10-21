use std::time::{Duration, Instant};

use bytes::Bytes;

use super::{terminal_targets::TerminalTargetError, App};
use crate::api::schema::AgentStartParams;

const DEFAULT_AGENT_START_TIMEOUT: Duration = Duration::from_secs(30);
pub(crate) const MAX_AGENT_START_TIMEOUT: Duration = Duration::from_secs(300);
pub(crate) const AGENT_START_SETTLE_DELAY: Duration = Duration::from_secs(3);
const INVALID_AGENT_TIMEOUT_MESSAGE: &str =
    "agent start timeout must be greater than 3000ms and at most 300000ms";
const INVALID_AGENT_NAME_MESSAGE: &str = "agent name must start with a lowercase letter and contain only lowercase letters, digits, '-' or '_' (1-32 characters)";

fn valid_agent_name(name: &str) -> bool {
    let mut chars = name.chars();
    matches!(chars.next(), Some('a'..='z'))
        && name.len() <= 32
        && chars.all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || matches!(ch, '-' | '_'))
}

/// Validates a requested CPU scheduling priority for an agent launch.
///
/// An out-of-range value is rejected rather than clamped, so an agent is
/// never started at a priority the operator did not ask for.
pub(crate) fn validate_agent_nice(nice: i32) -> Result<i32, AgentStartError> {
    if crate::agent_priority::is_valid(nice) {
        Ok(nice)
    } else {
        Err(AgentStartError::InvalidNice(nice))
    }
}

/// Builds the launch line for an agent.
///
/// The scheduling priority is part of the command the pane's shell runs, so
/// the agent inherits it before it forks any child and every descendant keeps
/// it. Without a priority the launch line is exactly the agent executable
/// followed by the agent's own arguments.
pub(crate) fn agent_launch_argv(
    executable: String,
    args: Vec<String>,
    nice: Option<i32>,
) -> Vec<String> {
    let mut argv = match nice {
        Some(nice) => crate::platform::agent_nice_prefix(nice),
        None => Vec::new(),
    };
    argv.push(executable);
    argv.extend(args);
    argv
}

/// Returns the priority program the launch line names, if it names one.
fn priority_program(argv: &[String]) -> Option<&'static str> {
    let program = crate::platform::agent_nice_program()?;
    match argv.first() {
        Some(first) if first == program => Some(program),
        _ => None,
    }
}

impl App {
    pub(super) fn collect_agent_infos(&self) -> Vec<crate::api::schema::AgentInfo> {
        self.state
            .workspaces
            .iter()
            .enumerate()
            .flat_map(|(ws_idx, ws)| {
                ws.tabs.iter().flat_map(move |tab| {
                    tab.layout
                        .pane_ids()
                        .into_iter()
                        .filter_map(move |pane_id| self.agent_info(ws_idx, pane_id))
                })
            })
            .collect()
    }

    pub(super) fn reconcile_managed_agent_target(&mut self, target: &str) {
        let Ok(resolved) = self.resolve_agent_target(target) else {
            return;
        };
        let Some(terminal_id) = self
            .state
            .workspaces
            .get(resolved.ws_idx)
            .and_then(|workspace| workspace.terminal_id(resolved.pane_id))
            .cloned()
        else {
            return;
        };
        let changed = self
            .state
            .terminals
            .get_mut(&terminal_id)
            .is_some_and(|terminal| terminal.reconcile_managed_agent_at(Instant::now(), false));
        if changed {
            self.state.mark_session_dirty();
            self.schedule_session_save();
            self.emit_pane_updated(resolved.ws_idx, resolved.pane_id);
        }
    }

    pub(super) fn agent_info_for_target(
        &self,
        target: &str,
    ) -> Result<crate::api::schema::AgentInfo, TerminalTargetError> {
        let resolved = self.resolve_agent_target(target)?;
        self.agent_info(resolved.ws_idx, resolved.pane_id)
            .ok_or_else(|| TerminalTargetError::NotFound {
                target: target.to_string(),
            })
    }

    pub(super) fn focus_agent_target(
        &mut self,
        target: &str,
    ) -> Result<crate::api::schema::AgentInfo, TerminalTargetError> {
        let resolved = self.resolve_agent_target(target)?;
        self.state
            .focus_pane_in_workspace(resolved.ws_idx, resolved.pane_id);
        self.state.mark_active_tab_seen();
        self.state.mode = crate::app::Mode::Terminal;
        self.agent_info(resolved.ws_idx, resolved.pane_id)
            .ok_or_else(|| TerminalTargetError::NotFound {
                target: target.to_string(),
            })
    }

    pub(super) fn rename_agent_target(
        &mut self,
        target: &str,
        name: Option<String>,
    ) -> Result<crate::api::schema::AgentInfo, AgentRenameError> {
        let resolved = self
            .resolve_agent_target(target)
            .map_err(AgentRenameError::Target)?;
        let normalized_name = match name {
            Some(name) if valid_agent_name(&name) => Some(name),
            Some(_) => return Err(AgentRenameError::InvalidName),
            None => None,
        };

        if let Some(name) = normalized_name.as_deref() {
            let conflicts = self.agent_name_conflicts(name, &resolved.terminal_id);
            if !conflicts.is_empty() {
                return Err(AgentRenameError::DuplicateName {
                    name: name.to_string(),
                    candidates: conflicts,
                });
            }
        }

        let Some(terminal) = self
            .state
            .terminals
            .values_mut()
            .find(|terminal| terminal.id.to_string() == resolved.terminal_id)
        else {
            return Err(AgentRenameError::Target(TerminalTargetError::NotFound {
                target: target.to_string(),
            }));
        };
        if terminal.managed_agent_launch_pending() {
            return Err(AgentRenameError::PendingLaunch);
        }
        if terminal.effective_agent_label().is_none() {
            return Err(AgentRenameError::NotAgent);
        }
        match normalized_name {
            Some(name) => terminal.set_agent_name(name),
            None => terminal.clear_agent_name(),
        }
        self.state.mark_session_dirty();
        self.schedule_session_save();
        self.emit_pane_updated(resolved.ws_idx, resolved.pane_id);
        self.agent_info(resolved.ws_idx, resolved.pane_id)
            .ok_or_else(|| {
                AgentRenameError::Target(TerminalTargetError::NotFound {
                    target: target.to_string(),
                })
            })
    }

    pub(super) fn start_agent(
        &mut self,
        params: AgentStartParams,
    ) -> Result<(crate::api::schema::AgentInfo, Vec<String>), AgentStartError> {
        let name = params.name;
        if !valid_agent_name(&name) {
            return Err(AgentStartError::InvalidName);
        }
        let Some(kind) = crate::detect::parse_agent_label(&params.kind) else {
            return Err(AgentStartError::UnsupportedKind(params.kind));
        };
        if params
            .args
            .iter()
            .any(|arg| arg.chars().any(char::is_control))
        {
            return Err(AgentStartError::InvalidArgument);
        }
        if let Some(nice) = params.nice {
            validate_agent_nice(nice)?;
        }
        let nice = crate::agent_priority::resolve(params.nice, self.state.agent_start_nice);
        let persisted_agent_session =
            crate::agent_resume::persisted_session_from_launch_args(kind, &params.args);
        let conflicts = self.agent_name_conflicts(&name, "");
        if !conflicts.is_empty() {
            return Err(AgentStartError::DuplicateName {
                name,
                candidates: conflicts,
            });
        }
        let Some((ws_idx, pane_id)) = self.parse_current_public_pane_id(&params.pane_id) else {
            return Err(AgentStartError::TargetNotFound(params.pane_id));
        };
        let terminal_id = self
            .state
            .workspaces
            .get(ws_idx)
            .and_then(|workspace| workspace.terminal_id(pane_id))
            .cloned()
            .ok_or_else(|| AgentStartError::TargetNotFound(params.pane_id.clone()))?;
        let terminal = self
            .state
            .terminals
            .get(&terminal_id)
            .ok_or_else(|| AgentStartError::TargetNotFound(params.pane_id.clone()))?;
        if terminal.is_agent_terminal() || terminal.managed_agent_kind().is_some() {
            return Err(AgentStartError::TargetBusy(params.pane_id));
        }
        let runtime = self
            .terminal_runtimes
            .get(&terminal_id)
            .ok_or_else(|| AgentStartError::TargetUnavailable(params.pane_id.clone()))?;
        let shell_name = available_shell_name(runtime)
            .ok_or_else(|| AgentStartError::TargetBusy(params.pane_id.clone()))?;

        let argv = agent_launch_argv(
            crate::detect::interactive_agent_executable(kind).to_string(),
            params.args,
            nice,
        );
        // The launch line names the priority program, so the pane's shell has
        // to find it. A missing program is reported instead of leaving the
        // operator with a launch that times out.
        if let Some(program) = priority_program(&argv) {
            if !crate::agent_priority::program_on_path(program) {
                return Err(AgentStartError::NiceUnavailable(program.to_string()));
            }
        }
        let command = crate::platform::interactive_shell_command(&argv, &shell_name)
            .ok_or(AgentStartError::InvalidArgument)?;
        let bytes = crate::app::api_helpers::encode_api_submission(runtime, &command);
        let timeout = Duration::from_millis(
            params
                .timeout_ms
                .unwrap_or(DEFAULT_AGENT_START_TIMEOUT.as_millis() as u64),
        );
        if timeout <= AGENT_START_SETTLE_DELAY || timeout > MAX_AGENT_START_TIMEOUT {
            return Err(AgentStartError::InvalidTimeout);
        }

        runtime
            .validate_input_submission(&bytes)
            .map_err(|err| AgentStartError::InputFailed(err.to_string()))?;

        let now = Instant::now();
        let terminal = self
            .state
            .terminals
            .get_mut(&terminal_id)
            .ok_or_else(|| AgentStartError::TargetUnavailable(params.pane_id.clone()))?;
        terminal.begin_managed_agent(name.clone(), kind, now, AGENT_START_SETTLE_DELAY, timeout);
        if let Err(err) = runtime.try_send_bytes(Bytes::from(bytes)) {
            terminal.clear_agent_name();
            return Err(AgentStartError::InputFailed(err.to_string()));
        }
        if let Some(session) = persisted_agent_session {
            terminal.set_managed_agent_launch_session(session);
        }
        self.state.mark_session_dirty();
        self.schedule_session_save();

        let agent = self
            .agent_info(ws_idx, pane_id)
            .ok_or(AgentStartError::TargetUnavailable(params.pane_id))?;
        Ok((agent, argv))
    }

    pub(super) fn agent_start_error_body(
        &self,
        err: AgentStartError,
    ) -> crate::api::schema::ErrorBody {
        match err {
            AgentStartError::InvalidName => crate::api::schema::ErrorBody {
                code: "invalid_agent_name".into(),
                message: INVALID_AGENT_NAME_MESSAGE.into(),
            },
            AgentStartError::UnsupportedKind(kind) => crate::api::schema::ErrorBody {
                code: "unsupported_agent_kind".into(),
                message: format!("unsupported interactive agent kind {kind}"),
            },
            AgentStartError::InvalidArgument => crate::api::schema::ErrorBody {
                code: "invalid_agent_argument".into(),
                message: "agent arguments cannot be encoded safely for the target shell".into(),
            },
            AgentStartError::InvalidTimeout => crate::api::schema::ErrorBody {
                code: "invalid_agent_timeout".into(),
                message: INVALID_AGENT_TIMEOUT_MESSAGE.into(),
            },
            AgentStartError::InvalidNice(nice) => crate::api::schema::ErrorBody {
                code: "invalid_agent_nice".into(),
                message: format!(
                    "agent nice value {nice} is outside the supported range {}..={}",
                    crate::agent_priority::MIN,
                    crate::agent_priority::MAX
                ),
            },
            AgentStartError::NiceUnavailable(program) => crate::api::schema::ErrorBody {
                code: "agent_nice_unavailable".into(),
                message: format!(
                    "the {program} program is not on the search path, so the agent was not started"
                ),
            },
            AgentStartError::TargetNotFound(target) => crate::api::schema::ErrorBody {
                code: "agent_pane_not_found".into(),
                message: format!("agent target pane {target} not found"),
            },
            AgentStartError::TargetBusy(target) => crate::api::schema::ErrorBody {
                code: "agent_pane_busy".into(),
                message: format!("agent target pane {target} is not an available shell"),
            },
            AgentStartError::TargetUnavailable(target) => crate::api::schema::ErrorBody {
                code: "agent_pane_unavailable".into(),
                message: format!("agent target pane {target} has no live terminal"),
            },
            AgentStartError::InputFailed(message) => crate::api::schema::ErrorBody {
                code: "agent_start_input_failed".into(),
                message,
            },
            AgentStartError::DuplicateName { name, candidates } => crate::api::schema::ErrorBody {
                code: "agent_name_taken".into(),
                message: format!(
                    "agent name {name} is already used; candidates: {}",
                    candidates
                        .into_iter()
                        .map(|candidate| format!(
                            "terminal_id={} pane_id={} workspace_id={} tab_id={} cwd={} status={:?}",
                            candidate.terminal_id,
                            candidate.pane_id,
                            candidate.workspace_id,
                            candidate.tab_id,
                            candidate.cwd.unwrap_or_else(|| "unknown".into()),
                            candidate.agent_status,
                        ))
                        .collect::<Vec<_>>()
                        .join("; ")
                ),
            },
        }
    }

    pub(super) fn agent_target_error_body(
        &self,
        err: TerminalTargetError,
    ) -> crate::api::schema::ErrorBody {
        match err {
            TerminalTargetError::NotFound { target } => crate::api::schema::ErrorBody {
                code: "agent_not_found".into(),
                message: format!("agent target {target} not found"),
            },
            TerminalTargetError::Ambiguous { target, candidates } => {
                crate::api::schema::ErrorBody {
                    code: "agent_target_ambiguous".into(),
                    message: format!(
                        "agent target {target} is ambiguous; candidates: {}",
                        candidates
                            .into_iter()
                            .map(|candidate| format!(
                                "terminal_id={} pane_id={} workspace_id={} tab_id={} cwd={} status={:?}",
                                candidate.terminal_id,
                                candidate.pane_id,
                                candidate.workspace_id,
                                candidate.tab_id,
                                candidate.cwd.unwrap_or_else(|| "unknown".into()),
                                candidate.agent_status,
                            ))
                            .collect::<Vec<_>>()
                            .join("; ")
                    ),
                }
            }
        }
    }

    pub(super) fn agent_rename_error_body(
        &self,
        err: AgentRenameError,
    ) -> crate::api::schema::ErrorBody {
        match err {
            AgentRenameError::Target(err) => self.agent_target_error_body(err),
            AgentRenameError::InvalidName => crate::api::schema::ErrorBody {
                code: "invalid_agent_name".into(),
                message: INVALID_AGENT_NAME_MESSAGE.into(),
            },
            AgentRenameError::NotAgent => crate::api::schema::ErrorBody {
                code: "agent_not_found".into(),
                message: "agent target does not currently host an agent".into(),
            },
            AgentRenameError::PendingLaunch => crate::api::schema::ErrorBody {
                code: "agent_launch_pending".into(),
                message: "agent name cannot change while startup is pending".into(),
            },
            AgentRenameError::DuplicateName { name, candidates } => crate::api::schema::ErrorBody {
                code: "agent_name_taken".into(),
                message: format!(
                    "agent name {name} is already used; candidates: {}",
                    candidates
                        .into_iter()
                        .map(|candidate| format!(
                            "terminal_id={} pane_id={} workspace_id={} tab_id={} cwd={} status={:?}",
                            candidate.terminal_id,
                            candidate.pane_id,
                            candidate.workspace_id,
                            candidate.tab_id,
                            candidate.cwd.unwrap_or_else(|| "unknown".into()),
                            candidate.agent_status,
                        ))
                        .collect::<Vec<_>>()
                        .join("; ")
                ),
            },
        }
    }

    pub(super) fn agent_info(
        &self,
        ws_idx: usize,
        pane_id: crate::layout::PaneId,
    ) -> Option<crate::api::schema::AgentInfo> {
        let ws = self.state.workspaces.get(ws_idx)?;
        let pane_state = ws.pane_state(pane_id)?;
        let terminal = self.state.terminals.get(&pane_state.attached_terminal_id)?;
        if !terminal.is_agent_terminal() {
            return None;
        }
        let pane = self.pane_metadata(ws_idx, pane_id)?;
        Some(crate::api::schema::AgentInfo {
            terminal_id: pane.terminal_id,
            name: terminal.agent_name.clone(),
            agent: pane.agent,
            title: pane.title,
            terminal_title: pane.terminal_title,
            terminal_title_stripped: pane.terminal_title_stripped,
            display_agent: pane.display_agent,
            agent_status: pane.agent_status,
            screen_detection_skipped: terminal.full_lifecycle_hook_authority_active(),
            state_labels: pane.state_labels,
            tokens: pane.tokens,
            agent_session: pane.agent_session,
            workspace_id: pane.workspace_id,
            tab_id: pane.tab_id,
            pane_id: pane.pane_id,
            focused: pane.focused,
            launch_pending: terminal.managed_agent_launch_pending(),
            interactive_ready: terminal.managed_agent_interactive_ready(),
            state_change_seq: terminal.last_agent_state_change_seq.unwrap_or(0),
            completion_seq: terminal.last_agent_completion_seq,
            cwd: pane.cwd,
            foreground_cwd: pane.foreground_cwd,
            revision: pane.revision,
        })
    }

    fn agent_name_conflicts(
        &self,
        name: &str,
        except_terminal_id: &str,
    ) -> Vec<crate::api::schema::AgentInfo> {
        self.collect_agent_infos()
            .into_iter()
            .filter(|agent| {
                agent.name.as_deref() == Some(name) && agent.terminal_id != except_terminal_id
            })
            .collect()
    }
}

pub(super) fn available_shell_name(runtime: &crate::terminal::TerminalRuntime) -> Option<String> {
    #[cfg(test)]
    if runtime.child_pid().is_none() {
        return Some("sh".into());
    }
    crate::platform::available_pane_shell(runtime.child_pid()?)
}

pub(super) fn runtime_hosts_agent(
    runtime: &crate::terminal::TerminalRuntime,
    expected: crate::detect::Agent,
) -> bool {
    #[cfg(test)]
    if runtime.child_pid().is_none() {
        return true;
    }
    live_runtime_agent(runtime) == Some(expected)
}

fn live_runtime_agent(runtime: &crate::terminal::TerminalRuntime) -> Option<crate::detect::Agent> {
    let job = crate::detect::foreground_job(runtime.child_pid()?)?;
    crate::detect::identify_agent_in_job(&job)
        .map(|(agent, _)| agent)
        .or_else(|| {
            job.processes
                .iter()
                .find_map(|process| crate::platform::process_agent_hint(process.pid))
        })
}

pub(super) enum AgentStartError {
    InvalidName,
    UnsupportedKind(String),
    InvalidArgument,
    InvalidTimeout,
    InvalidNice(i32),
    NiceUnavailable(String),
    TargetNotFound(String),
    TargetBusy(String),
    TargetUnavailable(String),
    InputFailed(String),
    DuplicateName {
        name: String,
        candidates: Vec<crate::api::schema::AgentInfo>,
    },
}

pub(super) enum AgentRenameError {
    Target(TerminalTargetError),
    InvalidName,
    NotAgent,
    PendingLaunch,
    DuplicateName {
        name: String,
        candidates: Vec<crate::api::schema::AgentInfo>,
    },
}

#[cfg(test)]
mod tests {
    use super::valid_agent_name;

    #[test]
    fn agent_names_use_a_small_cli_safe_grammar() {
        for name in ["a", "reviewer-one", "reviewer_2", &"a".repeat(32)] {
            assert!(valid_agent_name(name), "expected {name:?} to be valid");
        }
        for name in [
            "",
            " reviewer",
            "reviewer ",
            "reviewer one",
            "Reviewer",
            "1reviewer",
            "reviewer.one",
            &"a".repeat(33),
        ] {
            assert!(!valid_agent_name(name), "expected {name:?} to be invalid");
        }
    }

    #[test]
    fn the_launch_line_is_unchanged_without_a_priority() {
        assert_eq!(
            super::agent_launch_argv("pi".to_string(), vec!["--auto".to_string()], None),
            vec!["pi".to_string(), "--auto".to_string()]
        );
    }

    #[test]
    fn the_launch_line_puts_the_priority_before_the_agent_executable() {
        let argv = super::agent_launch_argv("pi".to_string(), vec!["--auto".to_string()], Some(19));
        let executable = argv
            .iter()
            .position(|arg| arg == "pi")
            .expect("the agent executable is part of the launch line");
        assert_eq!(argv.last().map(String::as_str), Some("--auto"));
        if cfg!(any(target_os = "linux", target_os = "macos")) {
            assert_eq!(argv[..executable], ["nice", "-n", "19"]);
        } else {
            assert_eq!(executable, 0);
        }
    }

    #[test]
    fn an_out_of_range_requested_priority_is_rejected_and_not_clamped() {
        assert!(matches!(super::validate_agent_nice(19), Ok(19)));
        assert!(matches!(super::validate_agent_nice(-20), Ok(-20)));
        for nice in [20, -21, i32::MAX, i32::MIN] {
            assert!(
                matches!(super::validate_agent_nice(nice), Err(super::AgentStartError::InvalidNice(value)) if value == nice),
                "expected {nice} to be rejected"
            );
        }
    }
}
