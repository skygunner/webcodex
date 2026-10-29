use super::support::*;
use crate::runner_protocol::{
    RunnerCapabilities, RunnerPollRequest, RunnerRequest, RunnerResultRequest,
};
use crate::tool_runtime::{ToolCall, ToolRuntime};
use serde_json::{json, Value};

const CLIENT: &str = "native-selection";

fn capabilities() -> RunnerCapabilities {
    RunnerCapabilities {
        computer_observe: true,
        computer_control: true,
        computer_selection: true,
        computer_text_input: true,
        computer_accessibility_observe: true,
        ..Default::default()
    }
}

async fn setup(caps: RunnerCapabilities) -> ToolRuntime {
    let runtime = runtime_with_agent_project(CLIENT);
    register_agent_projects_for_auth(
        &runtime,
        CLIENT,
        &bootstrap_auth_context(),
        caps,
        vec![registered_project("agent-proj", "/tmp/agent-proj")],
    )
    .await;
    runtime
}

fn call() -> ToolCall {
    ToolCall::from_tool_name(
        "control_computer",
        json!({"action":"select", "client_id":CLIENT,
        "surface_id":"surface_test", "element_id":"element_test"}),
    )
    .unwrap()
}

async fn complete(
    runtime: &ToolRuntime,
    request: RunnerRequest,
    output: Option<Value>,
    error: Option<&str>,
) {
    runtime
        .runner_registry
        .complete(RunnerResultRequest {
            client_id: CLIENT.into(),
            runner_instance_id: format!("inst-{CLIENT}"),
            request_id: request.request_id,
            exit_code: Some(if error.is_none() { 0 } else { 1 }),
            stdout: output.map(|v| v.to_string()),
            stderr: None,
            stdout_truncated: false,
            stderr_truncated: false,
            duration_ms: Some(1),
            error: error.map(str::to_string),
        })
        .await
        .unwrap();
}

async fn no_more_requests(runtime: &ToolRuntime) {
    assert!(runtime
        .runner_registry
        .poll(RunnerPollRequest {
            client_id: CLIENT.into(),
            runner_instance_id: format!("inst-{CLIENT}")
        })
        .await
        .unwrap()
        .is_none());
}

#[tokio::test]
async fn computer_selection_requires_its_own_capability_before_dispatch() {
    let mut caps = capabilities();
    caps.computer_selection = false;
    let runtime = setup(caps).await;
    let auth = bootstrap_auth_context();
    let result = runtime.dispatch_computer_tool(call(), Some(&auth)).await;
    assert!(!result.success);
    assert_eq!(result.output["execution_state"], "not_started");
    no_more_requests(&runtime).await;
}

#[tokio::test]
async fn computer_selection_routes_once_and_preserves_uncertain_effects() {
    for error in [None, Some("outcome_unknown: Select confirmation failed")] {
        let runtime = setup(capabilities()).await;
        let auth = bootstrap_auth_context();
        let mut tool = Box::pin(runtime.dispatch_computer_tool(call(), Some(&auth)));
        assert!(futures_util::poll!(tool.as_mut()).is_pending());
        let request = wait_for_runner_request_for_client(&runtime, CLIENT).await;
        assert_eq!(request.kind, "computer_control");
        assert_eq!(
            serde_json::from_str::<Value>(request.stdin.as_deref().unwrap()).unwrap(),
            json!({"action":"select","surface_id":"surface_test","element_id":"element_test"})
        );
        complete(&runtime, request, error.is_none().then(|| json!({"platform":"windows",
            "surface_id":"surface_test","element_id":"element_test","action":"select","success":true})), error).await;
        let result = tool.await;
        assert_eq!(result.success, error.is_none(), "{result:?}");
        if error.is_some() {
            assert_eq!(result.output["execution_state"], "outcome_unknown");
        }
        no_more_requests(&runtime).await;
    }
}

#[tokio::test(start_paused = true)]
async fn computer_selection_lost_response_never_replays() {
    let runtime = setup(capabilities()).await;
    let auth = bootstrap_auth_context();
    let mut tool = Box::pin(runtime.dispatch_computer_tool(call(), Some(&auth)));
    assert!(futures_util::poll!(tool.as_mut()).is_pending());
    let _request = wait_for_runner_request_for_client(&runtime, CLIENT).await;
    tokio::time::advance(std::time::Duration::from_secs(64)).await;
    let result = tool.await;
    assert!(!result.success);
    assert_eq!(result.output["execution_state"], "outcome_unknown");
    no_more_requests(&runtime).await;
}

#[tokio::test]
async fn computer_selection_state_requests_negotiate_optional_fields() {
    for supported in [false, true] {
        let mut caps = capabilities();
        caps.computer_element_state = true;
        caps.computer_selection = supported;
        let runtime = setup(caps).await;
        let auth = bootstrap_auth_context();
        let call = ToolCall::from_tool_name(
            "observe_computer",
            json!({"action":"element_state",
            "client_id":CLIENT,"surface_id":"surface_test","element_id":"element_test"}),
        )
        .unwrap();
        let mut tool = Box::pin(runtime.dispatch_computer_tool(call, Some(&auth)));
        assert!(futures_util::poll!(tool.as_mut()).is_pending());
        let request = wait_for_runner_request_for_client(&runtime, CLIENT).await;
        let payload: Value = serde_json::from_str(request.stdin.as_deref().unwrap()).unwrap();
        assert_eq!(
            payload.get("include_selection"),
            if supported {
                Some(&Value::Bool(true))
            } else {
                None
            }
        );
        let mut output = json!({"platform":"windows", "surface_id":"surface_test", "element_id":"element_test",
            "observation_generation":1,"enabled":true,"focused":false,"protected":false,
            "value_empty":null,"can_press":false,"can_focus":false,"can_input_text":false});
        if supported {
            output["can_select"] = json!(true);
            output["selected"] = json!(false);
        }
        complete(&runtime, request, Some(output), None).await;
        let result = tool.await;
        assert!(result.success, "{result:?}");
        assert_eq!(result.output.get("can_select").is_some(), supported);
        no_more_requests(&runtime).await;
    }
}
