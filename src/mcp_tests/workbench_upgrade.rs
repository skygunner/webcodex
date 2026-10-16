use super::*;

fn auth(user: &str, manage: bool) -> crate::auth::AuthContext {
    let mut value = crate::auth::AuthContext::new(crate::auth::AuthKind::ApiToken);
    value.user_id = Some(user.into());
    value.scopes = vec![
        crate::auth::SCOPE_RUNTIME_READ.into(),
        crate::auth::SCOPE_COMMUNICATION_READ.into(),
    ];
    if manage {
        value
            .scopes
            .push(crate::auth::SCOPE_COMMUNICATION_MANAGE.into());
    }
    value
}

fn params(value: Value) -> Value {
    let mut value = value;
    value["_meta"] = json!({"io.modelcontextprotocol/protocolVersion":MCP_STATELESS_PROTOCOL_VERSION,"io.modelcontextprotocol/clientCapabilities":{"extensions":{"io.modelcontextprotocol/ui":{"mimeTypes":["text/html;profile=mcp-app"]}}}});
    value
}

async fn request(
    runtime: &ToolRuntime,
    auth: Option<&crate::auth::AuthContext>,
    method: &str,
    value: Value,
    enabled: bool,
) -> Value {
    let outcome = handle_with_app_policy(
        runtime,
        serde_json::from_value(
            json!({"jsonrpc":"2.0","id":1,"method":method,"params":params(value)}),
        )
        .unwrap(),
        auth,
        enabled,
    )
    .await;
    match outcome {
        McpOutcome::Ok(value) => value,
        _ => panic!("request failed"),
    }
}

#[tokio::test]
async fn workbench_upgrade_personal_titles_are_private_and_clearable_through_mcp() {
    let temp = tempfile::tempdir().unwrap();
    let db = Arc::new(crate::Database::open(&temp.path().join("workbench.db")).unwrap());
    let runtime = ToolRuntime::new_for_tests().with_communication_database(db);
    let owner = auth("owner", true);
    let other = auth("other", true);
    let updated = request(
        &runtime,
        Some(&owner),
        "tools/call",
        json!({"name":"update_workbench_settings","arguments":{"display_name":"Lab Workbench"}}),
        true,
    )
    .await;
    assert_eq!(
        updated["result"]["structuredContent"]["success"], true,
        "{updated}"
    );
    for (who, title) in [(&owner, "Lab Workbench"), (&other, "WebCodex")] {
        let listed = request(&runtime, Some(who), "tools/list", json!({}), true).await;
        assert_eq!(listed["result"]["cacheScope"], "private");
        let tools = listed["result"]["tools"].as_array().unwrap();
        let launcher = tools
            .iter()
            .find(|t| t["name"] == "open_webcodex_workbench")
            .unwrap();
        assert_eq!(launcher["title"], title);
        assert_eq!(
            launcher.pointer("/_meta/ui/resourceUri"),
            Some(&json!(MCP_WORKBENCH_UI_RESOURCE_URI))
        );
    }
    let missing = request(
        &runtime,
        Some(&owner),
        "tools/call",
        json!({"name":"update_workbench_settings","arguments":{}}),
        true,
    )
    .await;
    assert_eq!(missing["result"]["structuredContent"]["success"], false);
    let preserved = runtime.workbench_settings(Some(&owner));
    assert_eq!(preserved.output["display_name"], "Lab Workbench");
    let cleared = request(
        &runtime,
        Some(&owner),
        "tools/call",
        json!({"name":"update_workbench_settings","arguments":{"display_name":null}}),
        true,
    )
    .await;
    assert_eq!(
        cleared["result"]["structuredContent"]["output"]["display_name"],
        "WebCodex"
    );
}

#[tokio::test]
async fn workbench_settings_restricted_authority_denies_writes_but_keeps_reads() {
    use crate::tool_runtime::permissions::{AuthorityMode, PermissionEvaluator};
    use std::sync::atomic::{AtomicUsize, Ordering};

    let temp = tempfile::tempdir().unwrap();
    let db = Arc::new(crate::Database::open(&temp.path().join("restricted-workbench.db")).unwrap());
    let evaluations = Arc::new(AtomicUsize::new(0));
    let runtime = ToolRuntime::new_for_tests()
        .with_communication_database(db.clone())
        .with_permission_evaluator(
            PermissionEvaluator::with_mode(AuthorityMode::Restricted)
                .with_eval_counter(evaluations.clone()),
        );
    let owner = auth("restricted-owner", true);
    for name in [json!("Must not persist"), Value::Null] {
        let denied = request(
            &runtime,
            Some(&owner),
            "tools/call",
            json!({"name":"update_workbench_settings","arguments":{"display_name":name}}),
            true,
        )
        .await;
        let result = &denied["result"]["structuredContent"];
        assert_eq!(result["success"], false, "{denied}");
        assert_eq!(result["output"]["error_kind"], "permission_denied");
        assert_eq!(result["output"]["permission"]["status"], "denied");
        assert_eq!(result["output"]["state_changed"], false);
    }
    assert_eq!(evaluations.load(Ordering::SeqCst), 2);
    assert_eq!(
        db.conn_for_tests()
            .query_row("SELECT COUNT(*) FROM workbench_preferences", [], |row| row
                .get::<_, i64>(
                0
            ))
            .unwrap(),
        0,
    );
    let read = request(
        &runtime,
        Some(&owner),
        "tools/call",
        json!({"name":"read_workbench_state","arguments":{}}),
        true,
    )
    .await;
    assert_eq!(
        read["result"]["structuredContent"]["success"], true,
        "{read}"
    );
    assert_eq!(
        evaluations.load(Ordering::SeqCst),
        2,
        "reads must not require approval"
    );
}

#[tokio::test]
async fn workbench_upgrade_scope_and_actual_runner_call_match_descriptors() {
    let runtime = ToolRuntime::new_for_tests();
    let reader = auth("reader", false);
    let listed = request(&runtime, Some(&reader), "tools/list", json!({}), true).await;
    let tools = listed["result"]["tools"].as_array().unwrap();
    assert!(tools.iter().any(|t| t["name"] == "list_runners"
        && t.pointer("/_meta/ui/visibility") == Some(&json!(["app"]))));
    assert!(!tools
        .iter()
        .any(|t| t["name"] == "update_workbench_settings"));
    assert!(!tools.iter().any(|t| t["name"] == "submit_workbench_work"));
    let denied = request(
        &runtime,
        Some(&reader),
        "tools/call",
        json!({"name":"update_workbench_settings","arguments":{"display_name":"Other"}}),
        true,
    )
    .await;
    assert_eq!(denied["result"]["structuredContent"]["success"], false);
    let runners=request(&runtime,Some(&reader),"tools/call",json!({"name":"list_runners","arguments":{"summary_only":true,"include_projects":false,"limit":10}}),true).await;
    assert_eq!(
        runners["result"]["structuredContent"]["success"], true,
        "{runners}"
    );
    assert!(runners["result"]["content"]
        .as_array()
        .unwrap()
        .iter()
        .any(|c| c["type"] == "text" && c["text"].as_str().is_some_and(|s| s.contains("success"))));
}

#[tokio::test]
async fn workbench_upgrade_disabled_and_invalid_adapters_fail_closed() {
    let runtime = ToolRuntime::new_for_tests();
    let outcome=handle_with_app_policy(&runtime,serde_json::from_value(json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":params(json!({"name":"read_workbench_state","arguments":{}}))})).unwrap(),None,false).await;
    assert!(matches!(outcome, McpOutcome::BadRequest(_)));
    for args in [
        json!({"limit":0}),
        json!({"limit":51}),
        json!({"principal":"foreign"}),
    ] {
        let result = request(
            &runtime,
            None,
            "tools/call",
            json!({"name":"read_workbench_state","arguments":args}),
            true,
        )
        .await;
        assert_eq!(
            result["result"]["structuredContent"]["success"], false,
            "{result}"
        );
    }
    let result = request(
        &runtime,
        None,
        "tools/call",
        json!({"name":"read_workbench_state","arguments":{}}),
        true,
    )
    .await;
    assert_eq!(result["result"]["structuredContent"]["success"], true);
    assert!(result["result"]["structuredContent"]["output"]["errors"]["tasks"].is_object());
}

#[tokio::test]
async fn workbench_upgrade_reports_unsupported_development_goal_id_without_rewriting_or_hiding_it()
{
    let temp = tempfile::tempdir().unwrap();
    let db = Arc::new(crate::Database::open(&temp.path().join("legacy-development.db")).unwrap());
    let runtime = ToolRuntime::new_for_tests().with_communication_database(db.clone());
    let owner = auth("development-owner", true);
    let created=request(&runtime,Some(&owner),"tools/call",json!({"name":"create_goal","arguments":{"title":"Preserve historical data","objective":"Keep the original database available for diagnosis","idempotency_key":"legacy-fixture"}}),true).await;
    let id = created["result"]["structuredContent"]["output"]["goal"]["summary"]["goal_id"]
        .as_str()
        .unwrap();
    let historical = "wc_goal_bd51c130bad143d58f3303c30d5f5b82";
    db.conn_for_tests()
        .execute(
            "UPDATE wc_goals SET goal_id=?1 WHERE goal_id=?2",
            rusqlite::params![historical, id],
        )
        .unwrap();
    let state = request(
        &runtime,
        Some(&owner),
        "tools/call",
        json!({"name":"read_workbench_state","arguments":{}}),
        true,
    )
    .await;
    let output = &state["result"]["structuredContent"]["output"];
    assert_eq!(state["result"]["structuredContent"]["success"], true);
    assert_eq!(output["errors"]["goals"]["error_kind"], "invalid_goal_id");
    assert!(output["errors"]["goals"]["message"]
        .as_str()
        .unwrap()
        .contains(historical));
    assert_eq!(output["goals"]["goals"], json!([]));
    assert!(output["errors"]["tasks"].is_null());
    let preserved: String = db
        .conn_for_tests()
        .query_row("SELECT goal_id FROM wc_goals", [], |row| row.get(0))
        .unwrap();
    assert_eq!(preserved, historical);
}
