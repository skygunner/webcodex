use super::*;

#[test]
fn metadata_index_is_readable_without_raw_payloads_and_after_capture_is_disabled() {
    let temp = tempfile::tempdir().unwrap();
    let mut env = crate::test_support::TestEnvGuard::new();
    env.set("WEBCODEX_TOOL_REQUEST_TRACE", "metadata");
    env.set(
        "WEBCODEX_TOOL_REQUEST_TRACE_DIR",
        temp.path().to_str().unwrap(),
    );
    let id = new_trace_id();
    let guard = ToolRequestLifecycle::new(
        "mcp",
        id.clone(),
        "1",
        "tools/call",
        Some("work_on_project".into()),
    );
    guard.received();
    guard.capture_request_diagnostic(
        "work_on_project",
        &json!({"path":"/root/git/demo","_wc":{"context":["project.instructions"]}}),
    );
    guard.capture_payload(
        "final_response",
        &json!({"success":true,"output":{"resolved_project":"agent:special:demo"}}),
    );
    guard.handler_returned(200, Some(100), Some(true), Some(true), "success");
    flush_full_trace_writer();
    assert!(!temp.path().join(&id).join("payloads").exists());
    let first = read_trace(&id, None, Some(2), None).unwrap();
    assert_eq!(first["trace_mode"], "metadata");
    assert_eq!(first["returned_count"], 2);
    assert_eq!(first["payload_count"], 0);
    let next = first["next_offset"].as_u64().unwrap() as usize;
    env.set("WEBCODEX_TOOL_REQUEST_TRACE", "off");
    let page = read_trace(&id, Some(next), Some(64), None).unwrap();
    assert_eq!(page["capture_mode"], "off");
    assert_eq!(page["status"], "available");
    assert_eq!(page["response_handoff_observed"], true);
    assert_eq!(
        page["capture_health"]["scope"],
        "server_process_since_start_not_per_trace"
    );
    assert!(page.to_string().contains("handler_returned"));
    assert!(read_trace("../elsewhere", None, None, None).is_err());
    let missing = read_trace(&new_trace_id(), None, None, None).unwrap();
    assert_eq!(missing["reason"], "trace_not_retained");
    assert_eq!(missing["coverage"], "unknown");
}

#[test]
fn metadata_reader_rejects_cross_trace_event_identity() {
    let temp = tempfile::tempdir().unwrap();
    let mut env = crate::test_support::TestEnvGuard::new();
    env.set("WEBCODEX_TOOL_REQUEST_TRACE", "metadata");
    env.set(
        "WEBCODEX_TOOL_REQUEST_TRACE_DIR",
        temp.path().to_str().unwrap(),
    );
    let id = new_trace_id();
    let event = base_event(&new_trace_id(), "wrong_trace");
    assert!(persist_metadata_event(&id, event).unwrap());
    assert_eq!(
        read_trace(&id, None, None, None).unwrap_err().kind,
        "trace_corrupt"
    );
}
