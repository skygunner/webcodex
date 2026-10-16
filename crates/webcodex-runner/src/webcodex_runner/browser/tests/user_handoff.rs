use super::*;

#[test]
fn browser_user_handoff_payloads_are_exact_and_closed() {
    assert!(parse::<BeginUserHandoffRequest>(r#"{"browser_id":"browser_exact","session_id":"session_owner","message_id":"message_attention"}"#).is_ok());
    assert!(parse::<FinishUserHandoffRequest>(r#"{"browser_id":"browser_exact","session_id":"session_owner","handoff_fence":"handoff_original"}"#).is_ok());
    for payload in [
        r#"{"browser_id":"browser_exact","session_id":"session_owner","message_id":"message_attention","renew":true}"#,
        r#"{"browser_id":"browser_exact","session_id":"session_owner","message_id":"message_attention","pid":42}"#,
    ] {
        assert!(parse::<BeginUserHandoffRequest>(payload).is_err());
    }
}

#[cfg(not(any(target_os = "macos", windows)))]
#[test]
fn browser_user_handoff_is_unavailable_on_linux_before_browser_or_native_lookup() {
    let operation = RunnerBrowserOperation {
        kind: RunnerBrowserOperationKind::BeginUserHandoff,
        payload: r#"{"browser_id":"browser_stale","session_id":"session_owner","message_id":"message_attention"}"#.into(),
        timeout_secs: 30,
    };
    let result = handle_browser_operation(
        &BrowserSupervisor::new(),
        &RunnerPolicy::default(),
        &operation,
    );
    let output: Value = serde_json::from_str(result.stdout.as_deref().unwrap()).unwrap();
    assert_eq!(output["execution_state"], "not_started");
    assert_eq!(output["error"]["kind"], "browser_user_handoff_unavailable");
}
