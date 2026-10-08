//! `herdr app focus` — jump the running client shell to a saved machine's target.
//!
//! The jump itself belongs to the client shell: only it can switch the endpoint
//! it presents. This command asks the local server to forward the request to a
//! running shell (`client.activate_endpoint`) and keeps today's server-side
//! focus as the fallback when no shell accepts it.

use crate::api::schema::{
    ClientActivateEndpointParams, ClientActivateEndpointTarget, Method, PaneTarget, Request,
    TabTarget, WorkspaceTarget,
};
use crate::client::endpoint::EndpointCatalog;

pub(super) const APP_FOCUS_USAGE: &str =
    "usage: herdr app focus [--machine <label-or-id>] (--pane <id> | --tab <id> | --workspace <id>)";

pub(super) const FOCUSED_APP: &str = "focused (app)";
pub(super) const FOCUSED_SERVER_SIDE: &str = "focused (server-side)";
pub(super) const FOCUSED_LOCAL: &str = "focused (local)";

pub(super) fn run_app_command(args: &[String]) -> std::io::Result<i32> {
    let Some(subcommand) = args.first().map(String::as_str) else {
        print_app_help();
        return Ok(2);
    };

    match subcommand {
        "focus" => app_focus(&args[1..]),
        "help" | "--help" | "-h" => {
            print_app_help();
            Ok(0)
        }
        _ => {
            print_app_help();
            Ok(2)
        }
    }
}

struct FocusRequest {
    machine: Option<String>,
    target: ClientActivateEndpointTarget,
}

fn parse_focus_args(args: &[String]) -> Result<FocusRequest, String> {
    let mut machine = None;
    let mut target = None;
    let mut index = 0;
    while index < args.len() {
        let flag = args[index].as_str();
        match flag {
            "--machine" | "--pane" | "--tab" | "--workspace" => {
                let value = args
                    .get(index + 1)
                    .ok_or_else(|| format!("missing value for {flag}"))?;
                if value.starts_with('-') {
                    return Err(format!("missing value for {flag}"));
                }
                match flag {
                    "--machine" => {
                        if machine.is_some() {
                            return Err("--machine can only be specified once".into());
                        }
                        if value.trim().is_empty() {
                            return Err(
                                "--machine requires a saved machine label or profile ID".into()
                            );
                        }
                        machine = Some(value.clone());
                    }
                    "--pane" | "--tab" | "--workspace" => {
                        if target.is_some() {
                            return Err(
                                "specify exactly one of --pane, --tab, or --workspace".into()
                            );
                        }
                        target = Some(match flag {
                            "--pane" => ClientActivateEndpointTarget::Pane(value.clone()),
                            "--tab" => ClientActivateEndpointTarget::Tab(value.clone()),
                            _ => ClientActivateEndpointTarget::Workspace(value.clone()),
                        });
                    }
                    _ => unreachable!("matched flag"),
                }
                index += 2;
            }
            other => return Err(format!("unknown option: {other}")),
        }
    }
    let target =
        target.ok_or_else(|| "specify one of --pane, --tab, or --workspace".to_string())?;
    Ok(FocusRequest { machine, target })
}

fn focus_method(target: &ClientActivateEndpointTarget) -> Method {
    match target {
        ClientActivateEndpointTarget::Workspace(workspace_id) => {
            Method::WorkspaceFocus(WorkspaceTarget {
                workspace_id: workspace_id.clone(),
            })
        }
        ClientActivateEndpointTarget::Tab(tab_id) => Method::TabFocus(TabTarget {
            tab_id: tab_id.clone(),
        }),
        ClientActivateEndpointTarget::Pane(pane_id) => Method::PaneFocus(PaneTarget {
            pane_id: pane_id.clone(),
        }),
    }
}

fn probe_method(target: &ClientActivateEndpointTarget) -> Method {
    match target {
        ClientActivateEndpointTarget::Workspace(workspace_id) => {
            Method::WorkspaceGet(WorkspaceTarget {
                workspace_id: workspace_id.clone(),
            })
        }
        ClientActivateEndpointTarget::Tab(tab_id) => Method::TabGet(TabTarget {
            tab_id: tab_id.clone(),
        }),
        ClientActivateEndpointTarget::Pane(pane_id) => Method::PaneGet(PaneTarget {
            pane_id: pane_id.clone(),
        }),
    }
}

/// Whether a running client shell accepted the request to jump.
pub(super) fn activation_delivered(response: &serde_json::Value) -> bool {
    response["result"]["delivered"].as_bool() == Some(true)
}

fn app_focus(args: &[String]) -> std::io::Result<i32> {
    let request = match parse_focus_args(args) {
        Ok(request) => request,
        Err(message) => {
            eprintln!("{message}");
            eprintln!("{APP_FOCUS_USAGE}");
            return Ok(2);
        }
    };

    let Some(selector) = request.machine.clone() else {
        return local_focus(&request.target);
    };

    let profiles = match EndpointCatalog::load_profiles() {
        Ok(profiles) => profiles,
        Err(error) => {
            eprintln!("error: {error}");
            return Ok(1);
        }
    };
    let profile = match super::target::resolve_machine(&profiles, &selector) {
        Ok(profile) => profile.clone(),
        Err(error) => {
            eprintln!("error: {error}");
            return Ok(2);
        }
    };

    // The machine owns its public ids, so prove the target exists there before
    // either path acts; an unresolvable id must fail here, not silently no-op.
    let validation = super::target::with_machine_profile(profile.clone(), || {
        super::send_request(&Request {
            id: "cli:app:focus:validate".into(),
            method: probe_method(&request.target),
        })
    });
    match validation {
        Ok(response) if response.get("error").is_none() => {}
        Ok(response) => {
            eprintln!("{}", serde_json::to_string(&response).unwrap());
            return Ok(1);
        }
        Err(error) => {
            eprintln!("error: {error}");
            return Ok(1);
        }
    }

    let activation = Request {
        id: "cli:app:focus".into(),
        method: Method::ClientActivateEndpoint(ClientActivateEndpointParams {
            machine: selector,
            target: request.target.clone(),
        }),
    };
    // No running shell (or one that cannot act on the request) falls through to
    // server-side focus, so an unreachable app is never an error by itself.
    if super::send_request_unchecked(&activation)
        .is_ok_and(|response| activation_delivered(&response))
    {
        println!("{FOCUSED_APP}");
        return Ok(0);
    }

    let response = super::target::with_machine_profile(profile, || {
        super::send_request(&Request {
            id: "cli:app:focus:server-side".into(),
            method: focus_method(&request.target),
        })
    })?;
    if response.get("error").is_some() {
        eprintln!("{}", serde_json::to_string(&response).unwrap());
        return Ok(1);
    }
    println!("{FOCUSED_SERVER_SIDE}");
    Ok(0)
}

fn local_focus(target: &ClientActivateEndpointTarget) -> std::io::Result<i32> {
    let response = super::send_request(&Request {
        id: "cli:app:focus".into(),
        method: focus_method(target),
    })?;
    if response.get("error").is_some() {
        eprintln!("{}", serde_json::to_string(&response).unwrap());
        return Ok(1);
    }
    println!("{FOCUSED_LOCAL}");
    Ok(0)
}

fn print_app_help() {
    eprintln!("herdr app commands:");
    eprintln!(
        "  herdr app focus [--machine <label-or-id>] (--pane <id> | --tab <id> | --workspace <id>)"
    );
    eprintln!("    Jump a running app to the target. With --machine it switches the app");
    eprintln!("    to that saved machine; without one it focuses the local target.");
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| (*value).into()).collect()
    }

    #[test]
    fn focus_args_require_exactly_one_target() {
        for input in [
            args(&[]),
            args(&["--machine", "mac"]),
            args(&["--pane", "w1:p1", "--tab", "w1:t1"]),
            args(&["--workspace", "w1", "--pane", "w1:p1"]),
        ] {
            assert!(parse_focus_args(&input).is_err(), "{input:?}");
        }

        let request = parse_focus_args(&args(&["--pane", "w1:p1"])).unwrap();
        assert_eq!(request.machine, None);
        assert_eq!(
            request.target,
            ClientActivateEndpointTarget::Pane("w1:p1".into())
        );

        let request = parse_focus_args(&args(&["--machine", "mac", "--tab", "w1:t1"])).unwrap();
        assert_eq!(request.machine.as_deref(), Some("mac"));
        assert_eq!(
            request.target,
            ClientActivateEndpointTarget::Tab("w1:t1".into())
        );
    }

    #[test]
    fn focus_args_reject_missing_values_and_unknown_options() {
        for input in [
            args(&["--machine"]),
            args(&["--pane"]),
            args(&["--machine", "--pane", "w1:p1"]),
            args(&["--pane", "w1:p1", "--machine"]),
            args(&["--machine", "mac", "--pane", "w1:p1", "--machine", "other"]),
            args(&["--machine", "mac", "--pane", "w1:p1", "--json"]),
        ] {
            assert!(parse_focus_args(&input).is_err(), "{input:?}");
        }
    }

    #[test]
    fn focus_targets_map_to_their_focus_and_probe_methods() {
        let targets = [
            ClientActivateEndpointTarget::Workspace("w1".into()),
            ClientActivateEndpointTarget::Tab("w1:t1".into()),
            ClientActivateEndpointTarget::Pane("w1:p1".into()),
        ];
        for target in targets {
            let focus = focus_method(&target);
            let probe = probe_method(&target);
            let expected = match &target {
                ClientActivateEndpointTarget::Workspace(_) => ("workspace.focus", "workspace.get"),
                ClientActivateEndpointTarget::Tab(_) => ("tab.focus", "tab.get"),
                ClientActivateEndpointTarget::Pane(_) => ("pane.focus", "pane.get"),
            };
            assert_eq!(crate::api::api_method_name(&focus), expected.0);
            assert_eq!(crate::api::api_method_name(&probe), expected.1);
        }
    }

    #[test]
    fn only_a_delivered_response_counts_as_an_app_jump() {
        assert!(activation_delivered(
            &serde_json::json!({"result": {"type": "client_activate_endpoint", "delivered": true}})
        ));
        assert!(!activation_delivered(
            &serde_json::json!({"result": {"type": "client_activate_endpoint", "delivered": false}})
        ));
        assert!(!activation_delivered(
            &serde_json::json!({"error": {"code": "not_implemented", "message": "no"}})
        ));
        assert!(!activation_delivered(&serde_json::json!({})));
    }
}
