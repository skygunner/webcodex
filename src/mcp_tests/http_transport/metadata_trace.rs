use super::*;

#[test]
fn metadata_trace_captures_direct_gateway_and_invalid_envelopes_without_payloads() {
    std::thread::Builder::new()
        .name("mcp-metadata-trace".into())
        .stack_size(8 * 1024 * 1024)
        .spawn(|| {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap()
                .block_on(exercise())
        })
        .unwrap()
        .join()
        .unwrap();
}

async fn exercise() {
    let root = tempfile::tempdir().unwrap();
    let mut env = crate::test_support::TestEnvGuard::new();
    env.set("WEBCODEX_TOOL_REQUEST_TRACE", "metadata");
    env.set(
        "WEBCODEX_TOOL_REQUEST_TRACE_DIR",
        root.path().to_str().unwrap(),
    );
    env.set("WEBCODEX_TOOL_REQUEST_TRACE_MAX_TOTAL_BYTES", "8388608");
    let config = test_config(Some("ingress-secret-never-copy"));
    let (_tmp, db) = test_db();
    let runtime = Arc::new(test_runtime());
    let service = Service::new(build_test_router(config, db, runtime));
    let cases = [
        (
            "read_files",
            json!({"project":"agent:missing:demo","items":[{"path":"src/example.rs","start_line":4,"limit":7}],"_wc":{"context":["project.instructions"]}}),
            StatusCode::OK,
        ),
        (
            "call_runtime_tool",
            json!({"tool":"run_script","arguments":{"project":"agent:missing:demo","language":"python","script":"body-only-private-sentinel".repeat(1_000)},"_wc":{"context":["project.instructions"]}}),
            StatusCode::OK,
        ),
        (
            "work_on_project",
            json!({"client_id":"missing","path":"/workspace/demo","instruction":"instruction-body-private-sentinel","_wc":{"context":"wrong-shape"}}),
            StatusCode::BAD_REQUEST,
        ),
    ];
    for (id, (name, arguments, expected)) in cases.into_iter().enumerate() {
        let (status, body) = stateless_2026_tool_call(
            &service,
            "ingress-secret-never-copy",
            101 + id as i64,
            name,
            arguments,
            None,
        )
        .await;
        assert_eq!(status, expected, "{name}: {body}");
    }
    crate::tool_request_trace::flush_full_trace_writer();
    let mut supplied = Vec::new();
    let mut phases = std::collections::BTreeSet::new();
    for entry in std::fs::read_dir(root.path()).unwrap().flatten() {
        if !entry.file_type().unwrap().is_dir() {
            continue;
        }
        assert!(!entry.path().join("payloads").exists());
        let bytes = std::fs::read_to_string(entry.path().join("events.jsonl")).unwrap();
        for sentinel in [
            "ingress-secret-never-copy",
            "body-only-private-sentinel",
            "instruction-body-private-sentinel",
        ] {
            assert!(
                !bytes.contains(sentinel),
                "body or ingress credential escaped metadata projection"
            );
        }
        for line in bytes.lines() {
            let event: Value = serde_json::from_str(line).unwrap();
            if let Some(phase) = event["phase"].as_str() {
                phases.insert(phase.to_owned());
            }
            if event["phase"] == "supplied_arguments" {
                supplied.push(event["diagnostic"]["value"].clone());
            }
            assert_ne!(event["event"], "tool_trace_payload_captured");
        }
    }
    assert_eq!(supplied.len(), 3);
    let direct = supplied
        .iter()
        .find(|item| item["tool"] == "read_files")
        .unwrap();
    assert_eq!(direct["arguments"]["items"][0]["path"], "src/example.rs");
    assert!(direct["arguments"].get("_wc").is_none());
    assert_eq!(
        direct["invocation"]["context"],
        json!(["project.instructions"])
    );
    let gateway = supplied
        .iter()
        .find(|item| item["tool"] == "run_script")
        .unwrap();
    assert_eq!(gateway["entry_tool"], "call_runtime_tool");
    assert_eq!(gateway["arguments"]["script"]["omitted"], "body");
    assert_eq!(
        supplied
            .iter()
            .find(|item| item["tool"] == "work_on_project")
            .unwrap()["invocation"]["context"],
        "wrong-shape"
    );
    assert!(phases.contains("kernel_arguments"));
    assert!(phases.contains("response_summary"));
}
