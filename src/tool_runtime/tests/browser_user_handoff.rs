use super::super::kernel::{
    HostFileImportTrust, ToolCallContext, ToolCallOutcome, ToolCallRequest, ToolTransport,
};
use super::super::permissions::{AuthorityMode, PermissionEvaluator};
use super::super::sessions::{
    self, PostSessionMessageInput, SessionMessageKind, SessionMessagePriority,
};
use super::super::{SessionMode, ToolRuntime};
use super::support::*;
use crate::auth::AuthContext;
use crate::runner_protocol::{RunnerCapabilities, RunnerPollRequest};
use serde_json::{json, Value};

fn session(runtime: &ToolRuntime, auth: &AuthContext) -> String {
    let owner =
        super::super::session_context::workflow_session_authority_fingerprint(Some(auth)).unwrap();
    runtime
        .sessions
        .start_session_with_options(
            sessions::SessionCreateOptions::new(
                None,
                Some("browser handoff".into()),
                SessionMode::Normal,
                sessions::SessionGuards::default(),
            )
            .with_owner_authority_fingerprint(Some(owner)),
        )
        .unwrap()
        .session_id
}

fn message(runtime: &ToolRuntime, session_id: &str, kind: SessionMessageKind) -> String {
    runtime
        .sessions
        .post_message(PostSessionMessageInput {
            session_id: session_id.into(),
            kind,
            message: "Complete the visible browser step".into(),
            tags: vec![],
            reply_to: None,
            priority: SessionMessagePriority::Normal,
        })
        .unwrap()
        .message_id
}

async fn call(runtime: &ToolRuntime, auth: &AuthContext, arguments: Value) -> ToolCallOutcome {
    super::super::specialized::try_dispatch_specialized_gateway(
        runtime,
        &ToolCallRequest {
            tool_name: "control_browser".into(),
            arguments,
        },
        ToolCallContext {
            transport: ToolTransport::Api,
            session_id: None,
            auth: Some(auth),
            window: None,
            record_oauth_scope_denials: true,
            host_file_import_trust: HostFileImportTrust::Untrusted,
        },
    )
    .await
    .unwrap()
}

#[tokio::test]
async fn browser_user_handoff_requires_exact_open_attention_and_session_owner_before_dispatch() {
    let references = tempfile::tempdir().unwrap();
    let runtime = test_runtime()
        .with_project_reference_database(std::sync::Arc::new(
            crate::Database::open(&references.path().join("handoff-refs.db")).unwrap(),
        ))
        .with_permission_evaluator(PermissionEvaluator::with_mode(AuthorityMode::TrustedAgent));
    let client_id = "handoff-authority";
    let mut auth = shared_key_auth_context("handoff-owner");
    auth.scopes.extend([
        crate::auth::SCOPE_BROWSER_CONTROL.into(),
        crate::auth::SCOPE_COMPUTER_READ.into(),
    ]);
    register_agent_projects_for_auth(
        &runtime,
        client_id,
        &auth,
        RunnerCapabilities {
            browser_control: true,
            browser_surface_handoff: true,
            computer_observe: true,
            browser_user_handoff: true,
            ..Default::default()
        },
        vec![],
    )
    .await;
    let valid = session(&runtime, &auth);
    let question = message(&runtime, &valid, SessionMessageKind::Question);
    let guidance = message(&runtime, &valid, SessionMessageKind::Guidance);
    let resolved = message(&runtime, &valid, SessionMessageKind::Todo);
    runtime
        .sessions
        .resolve_message(&valid, &resolved, Some("already done".into()))
        .unwrap();
    let foreign_auth = shared_key_auth_context("foreign-owner");
    let foreign = session(&runtime, &foreign_auth);
    let foreign_question = message(&runtime, &foreign, SessionMessageKind::Question);
    let foreign_ref = runtime
        .session_reference_for_id(&foreign, Some(&foreign_auth))
        .unwrap();
    let closed = session(&runtime, &auth);
    let closed_question = message(&runtime, &closed, SessionMessageKind::Question);
    runtime.sessions.close_session(&closed).unwrap();
    for (session_id, message_id) in [
        (valid.as_str(), guidance.as_str()),
        (valid.as_str(), resolved.as_str()),
        (valid.as_str(), foreign_question.as_str()),
        (foreign.as_str(), foreign_question.as_str()),
        (foreign_ref.as_str(), foreign_question.as_str()),
        (closed.as_str(), closed_question.as_str()),
        ("session_missing", question.as_str()),
    ] {
        let result = call(
            &runtime,
            &auth,
            json!({"action":"begin_user_handoff", "client_id":client_id,
            "browser_id":"browser_fixture", "session_id":session_id, "message_id":message_id}),
        )
        .await;
        assert!(
            !result.success,
            "unexpected authority for {session_id}/{message_id}"
        );
        assert!(
            runtime
                .runner_registry
                .poll(RunnerPollRequest {
                    client_id: client_id.into(),
                    runner_instance_id: format!("inst-{client_id}")
                })
                .await
                .unwrap()
                .is_none(),
            "denial enqueued an effect"
        );
    }
    let mut missing_scope = auth.clone();
    missing_scope
        .scopes
        .retain(|scope| scope != crate::auth::SCOPE_RUNTIME_READ);
    assert!(
        !call(
            &runtime,
            &missing_scope,
            json!({"action":"begin_user_handoff", "client_id":client_id,
        "browser_id":"browser_fixture", "session_id":valid, "message_id":question})
        )
        .await
        .success
    );
    assert!(runtime
        .runner_registry
        .poll(RunnerPollRequest {
            client_id: client_id.into(),
            runner_instance_id: format!("inst-{client_id}")
        })
        .await
        .unwrap()
        .is_none());
}

#[tokio::test]
async fn browser_user_handoff_dispatch_preserves_exact_association_and_finish_does_not_require_open_attention(
) {
    let references = tempfile::tempdir().unwrap();
    let runtime = test_runtime()
        .with_project_reference_database(std::sync::Arc::new(
            crate::Database::open(&references.path().join("handoff-refs.db")).unwrap(),
        ))
        .with_permission_evaluator(PermissionEvaluator::with_mode(AuthorityMode::TrustedAgent));
    let client_id = "handoff-dispatch";
    let mut auth = shared_key_auth_context("handoff-dispatch-owner");
    auth.scopes.extend([
        crate::auth::SCOPE_BROWSER_CONTROL.into(),
        crate::auth::SCOPE_COMPUTER_READ.into(),
    ]);
    register_agent_projects_for_auth(
        &runtime,
        client_id,
        &auth,
        RunnerCapabilities {
            browser_control: true,
            browser_surface_handoff: true,
            computer_observe: true,
            browser_user_handoff: true,
            ..Default::default()
        },
        vec![],
    )
    .await;
    let session_id = session(&runtime, &auth);
    let message_id = message(&runtime, &session_id, SessionMessageKind::Todo);
    let session_ref = runtime
        .session_reference_for_id(&session_id, Some(&auth))
        .unwrap();
    for finish in [false, true] {
        if finish {
            runtime
                .sessions
                .resolve_message(&session_id, &message_id, Some("human finished".into()))
                .unwrap();
            auth.scopes.retain(|scope| {
                scope != crate::auth::SCOPE_RUNTIME_READ
                    && scope != crate::auth::SCOPE_COMPUTER_READ
            });
        }
        let mut arguments = json!({"action":if finish {"finish_user_handoff"} else {"begin_user_handoff"},
            "client_id":client_id,"browser_id":"browser_fixture","session_id":session_ref});
        if finish {
            arguments["handoff_fence"] = json!("handoff_fixture");
        } else {
            arguments["message_id"] = json!(message_id);
        }
        let task = tokio::spawn({
            let runtime = runtime.clone();
            let auth = auth.clone();
            async move { call(&runtime, &auth, arguments).await }
        });
        let request = wait_for_runner_request_for_client(&runtime, client_id).await;
        assert_eq!(
            request.kind,
            if finish {
                "browser_finish_user_handoff"
            } else {
                "browser_begin_user_handoff"
            }
        );
        let payload: Value = serde_json::from_str(request.stdin.as_deref().unwrap()).unwrap();
        assert_eq!(payload["browser_id"], "browser_fixture");
        assert_eq!(payload["session_id"], session_id);
        assert_eq!(
            payload[if finish {
                "handoff_fence"
            } else {
                "message_id"
            }],
            if finish {
                "handoff_fixture"
            } else {
                message_id.as_str()
            }
        );
        complete_patch_agent_request_for_instance(
            &runtime,
            client_id,
            &format!("inst-{client_id}"),
            &request.request_id,
            0,
            &json!({"ok":true,"execution_state":"completed","state_changed":true,
                "result":{"browser_id":"browser_fixture","session_id":session_id,"needs_snapshot":true}})
            .to_string(),
            "",
        )
        .await;
        assert!(task.await.unwrap().success);
    }
}
