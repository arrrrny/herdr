use super::*;

fn connect_shell(
    server: &mut HeadlessServer,
    client_id: u64,
    activate_endpoint: bool,
) -> std::sync::mpsc::Receiver<Vec<u8>> {
    let (writer, control_rx, _render_rx) = test_client_writer();
    assert!(
        server.handle_server_event(ServerEvent::ClientShellConnected {
            client_id,
            surface_cols: 80,
            surface_rows: 24,
            cell_width_px: 8,
            cell_height_px: 16,
            pixel_mouse: false,
            direct_graphics: false,
            endpoint_keybindings: false,
            mouse_capture: false,
            surface_active: true,
            surface_reuse: false,
            surface_delta: false,
            surface_scroll: false,
            activate_endpoint,
            writer,
        })
    );
    control_rx
}

fn activate_endpoint_request() -> api::schema::Request {
    api::schema::Request {
        id: "activate-endpoint".into(),
        method: api::schema::Method::ClientActivateEndpoint(
            api::schema::ClientActivateEndpointParams {
                machine: "mac-mini".into(),
                target: api::schema::ClientActivateEndpointTarget::Pane("w1D:p4Y".into()),
            },
        ),
    }
}

fn dispatch(server: &mut HeadlessServer, request: api::schema::Request) -> serde_json::Value {
    let (respond_to, response_rx) = std::sync::mpsc::channel();
    server.handle_api_request_with_shutdown_check(api::ApiRequestMessage {
        request,
        respond_to,
        response_write_complete: None,
    });
    let response = response_rx
        .recv_timeout(Duration::from_millis(500))
        .expect("api response");
    serde_json::from_str(&response).expect("api response json")
}

fn read_activate_endpoint_control(
    control_rx: &std::sync::mpsc::Receiver<Vec<u8>>,
) -> api::schema::ClientActivateEndpointParams {
    let deadline = std::time::Instant::now() + Duration::from_millis(500);
    while std::time::Instant::now() < deadline {
        let Ok(bytes) = control_rx.recv_timeout(Duration::from_millis(50)) else {
            continue;
        };
        let ServerMessage::EndpointControl { kind, data } = read_server_message(bytes) else {
            continue;
        };
        if kind == crate::protocol::endpoint::ACTIVATE_ENDPOINT_KIND {
            return serde_json::from_str(&data).expect("activate endpoint params");
        }
    }
    panic!("no activate endpoint control reached the shell");
}

#[tokio::test]
async fn activate_endpoint_request_reaches_a_capable_client_shell() {
    let mut server = test_headless_server();
    let control_rx = connect_shell(&mut server, 1, true);

    let response = dispatch(&mut server, activate_endpoint_request());

    assert_eq!(response["result"]["type"], "client_activate_endpoint");
    assert_eq!(response["result"]["delivered"], true);
    let params = read_activate_endpoint_control(&control_rx);
    assert_eq!(params.machine, "mac-mini");
    assert_eq!(
        params.target,
        api::schema::ClientActivateEndpointTarget::Pane("w1D:p4Y".into())
    );
}

#[tokio::test]
async fn activate_endpoint_request_without_a_capable_shell_is_not_delivered() {
    let mut server = test_headless_server();
    let control_rx = connect_shell(&mut server, 1, false);

    let response = dispatch(&mut server, activate_endpoint_request());

    assert_eq!(response["result"]["type"], "client_activate_endpoint");
    assert_eq!(response["result"]["delivered"], false);
    assert!(
        control_rx
            .recv_timeout(Duration::from_millis(200))
            .ok()
            .is_none_or(|bytes| {
                !matches!(
                    read_server_message(bytes),
                    ServerMessage::EndpointControl { kind, .. }
                        if kind == crate::protocol::endpoint::ACTIVATE_ENDPOINT_KIND
                )
            }),
        "an incapable shell must not receive the control"
    );
}

#[tokio::test]
async fn activate_endpoint_request_without_clients_is_not_delivered() {
    let mut server = test_headless_server();

    let response = dispatch(&mut server, activate_endpoint_request());

    assert_eq!(response["result"]["type"], "client_activate_endpoint");
    assert_eq!(response["result"]["delivered"], false);
}
