use super::*;

fn app_params(value: Value) -> Value {
    mcp_2026_ui_params(value)
}

async fn http_rpc(service: &Service, method: &str, params: Value) -> (StatusCode, Value) {
    let tool_name = (method == "tools/call")
        .then(|| params.get("name").and_then(Value::as_str))
        .flatten();
    let mut request = TestClient::post("http://localhost/mcp")
        .bearer_auth("secret")
        .add_header(
            MCP_PROTOCOL_VERSION_HEADER,
            MCP_STATELESS_PROTOCOL_VERSION,
            true,
        )
        .add_header(MCP_METHOD_HEADER, method, true);
    if let Some(name) = tool_name {
        request = request.add_header(MCP_NAME_HEADER, name, true);
    }
    let mut response = request
        .json(&json!({"jsonrpc":"2.0","id":1,"method":method,"params":app_params(params)}))
        .send(service)
        .await;
    let status = effective_status(&response);
    let body = response.take_json::<Value>().await.unwrap();
    (status, body)
}

async fn call(service: &Service, name: &str, arguments: Value) -> Value {
    let (status, body) = http_rpc(
        service,
        "tools/call",
        json!({"name":name,"arguments":arguments}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    body
}

#[tokio::test]
async fn workbench_http_app_discovery_and_read_settings_calls() {
    let (_temp, db) = test_db();
    let runtime = Arc::new(
        test_runtime_with_mcp_settings(false, true).with_communication_database(db.clone()),
    );
    let service = Service::new(build_test_router(test_config(Some("secret")), db, runtime));

    let (status, listed) = http_rpc(&service, "tools/list", json!({})).await;
    assert_eq!(status, StatusCode::OK, "{listed}");
    let tools = listed["result"]["tools"]
        .as_array()
        .expect("tools/list tools");
    for name in [
        "read_workbench_state",
        "update_workbench_settings",
        "submit_workbench_work",
        "list_runners",
    ] {
        let descriptor = tools
            .iter()
            .find(|tool| tool["name"] == name)
            .unwrap_or_else(|| panic!("missing {name}: {listed}"));
        assert_eq!(
            descriptor.pointer("/_meta/ui/visibility"),
            Some(&json!(["app"])),
            "{name}"
        );
    }
    let launcher = tools
        .iter()
        .find(|tool| tool["name"] == "open_webcodex_workbench")
        .expect("launcher descriptor");
    assert_eq!(launcher["title"], "WebCodex");
    assert_eq!(
        launcher.pointer("/_meta/ui/resourceUri"),
        Some(&json!(MCP_WORKBENCH_UI_RESOURCE_URI))
    );

    let runners = call(
        &service,
        "list_runners",
        json!({"summary_only":true,"include_projects":false,"limit":10}),
    )
    .await;
    assert_eq!(
        runners["result"]["structuredContent"]["success"], true,
        "{runners}"
    );

    let state = call(&service, "read_workbench_state", json!({})).await;
    assert_eq!(
        state["result"]["structuredContent"]["success"], true,
        "{state}"
    );
    assert!(
        state["result"]["structuredContent"]["output"]["tasks"]["tasks"].is_array(),
        "{state}"
    );
    assert_eq!(
        state["result"]["structuredContent"]["output"]["settings"]["display_name"],
        "WebCodex"
    );

    let changed = call(
        &service,
        "update_workbench_settings",
        json!({"display_name":"HTTP Workbench"}),
    )
    .await;
    assert_eq!(
        changed["result"]["structuredContent"]["success"], true,
        "{changed}"
    );
    let after = call(&service, "read_workbench_state", json!({})).await;
    assert_eq!(
        after["result"]["structuredContent"]["output"]["settings"]["display_name"],
        "HTTP Workbench"
    );

    let cleared = call(
        &service,
        "update_workbench_settings",
        json!({"display_name":null}),
    )
    .await;
    assert_eq!(
        cleared["result"]["structuredContent"]["success"], true,
        "{cleared}"
    );
}
