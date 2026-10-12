use super::*;

#[tokio::test]
async fn console_trace_route_keeps_admin_and_same_origin_boundaries() {
    let runtime = Arc::new(ToolRuntime::new_for_tests());
    // Test-config shared keys are Bootstrap; inject an actual non-admin
    // principal so this checks the handler boundary rather than that fixture.
    let ordinary = crate::auth::shared_key_context("ordinary");
    let service = Service::new(
        Router::new()
            .hoop(affix_state::inject(runtime.clone()))
            .hoop(affix_state::inject(ordinary))
            .push(Router::with_path("api").push(routes())),
    );
    let mut response = TestClient::post("http://localhost/api/runtime-console/trace")
        .bearer_auth("ordinary")
        .json(&json!({}))
        .send(&service)
        .await;
    assert_eq!(
        response.status_code.unwrap_or(StatusCode::OK),
        StatusCode::FORBIDDEN
    );
    let body = response.take_string().await.unwrap();
    assert!(!body.contains("payload"));

    // Inject only canonical administrator identity while retaining the actual
    // Console handler's same-origin/JSON and runtime authorization checks.
    let service = Service::new(
        Router::new()
            .hoop(affix_state::inject(runtime))
            .hoop(affix_state::inject(test_bootstrap_auth()))
            .push(Router::with_path("api").push(routes())),
    );
    let mut response = TestClient::post("http://localhost/api/runtime-console/trace")
        .json(&json!({"query":{"since_ms":2,"until_ms":1}}))
        .send(&service)
        .await;
    assert_eq!(
        response.status_code.unwrap_or(StatusCode::OK),
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        response.take_json::<Value>().await.unwrap()["error_kind"],
        "invalid_trace_request"
    );
    let response = TestClient::post("http://localhost/api/runtime-console/trace")
        .add_header("Origin", "https://other.example", true)
        .json(&json!({}))
        .send(&service)
        .await;
    assert_eq!(
        response.status_code.unwrap_or(StatusCode::OK),
        StatusCode::FORBIDDEN
    );
}
