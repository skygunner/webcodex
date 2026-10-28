use super::support::*;
use crate::auth::{AuthContext, AuthKind};
use crate::runner_protocol::{RunnerCapabilities, RunnerRegisterRequest};
use crate::tool_runtime::coding_agent::{
    diagnose_response_lost, diagnose_result_from_response, live_acp_bound_violation,
};
use crate::tool_runtime::permissions::{AuthorityMode, PermissionEvaluator};
use crate::tool_runtime::{ToolCall, ToolRuntime};
use serde_json::{json, Value};
use webcodex_core::coding_agent::{
    CodingAgentDiagnoseOutcome, CodingAgentDiagnosePhase, CodingAgentDispatchState,
    CodingAgentLiveAcpDiagnostic, CodingAgentLiveConfigChoice, CodingAgentLiveConfigOption,
    CodingAgentProvider, CodingAgentResponse, CodingAgentResponsePayload,
    CODING_AGENT_MAX_CONFIG_KEY_BYTES, CODING_AGENT_MAX_CONFIG_LABEL_BYTES,
    CODING_AGENT_MAX_CONFIG_OPTIONS, CODING_AGENT_MAX_CONFIG_SELECT_CHOICES,
    CODING_AGENT_MAX_ENV_MAPPINGS,
};

fn oauth(scopes: &[&str]) -> AuthContext {
    AuthContext {
        user_id: Some("u".to_string()),
        username: Some("alice".to_string()),
        role: Some("user".to_string()),
        scopes: scopes.iter().map(|s| s.to_string()).collect(),
        token_kind: Some("oauth2".to_string()),
        ..AuthContext::new(AuthKind::OAuth2Token)
    }
}

fn provider(id: &str) -> CodingAgentProvider {
    CodingAgentProvider {
        provider_id: id.into(),
        provider_instance_id: format!("private-instance-{id}"),
        name: format!("{id} Agent"),
    }
}

async fn register(
    runtime: &ToolRuntime,
    client_id: &str,
    providers: Option<Vec<CodingAgentProvider>>,
) {
    let has_providers = providers
        .as_ref()
        .is_some_and(|providers| !providers.is_empty());
    runtime
        .runner_registry
        .register_with_auth(
            RunnerRegisterRequest {
                coding_agent_inventory: has_providers.then(Default::default),
                coding_agent_providers: providers,
                ..crate::test_support::runner_registration(
                    client_id,
                    &format!("inst-{client_id}"),
                    RunnerCapabilities {
                        coding_agent_runs: has_providers,
                        ..Default::default()
                    },
                )
            },
            None,
        )
        .await
        .unwrap();
}

async fn register_runner_with_project(
    client_id: &str,
    providers: Option<Vec<CodingAgentProvider>>,
) -> ToolRuntime {
    let runtime = test_runtime();
    register(&runtime, client_id, providers).await;
    crate::test_support::apply_project_inventory_snapshot(
        &runtime.runner_registry,
        client_id,
        &format!("inst-{client_id}"),
        vec![registered_project("demo", "/tmp/acp-diagnose-demo")],
    )
    .await;
    runtime
}

fn live_ok(options: Vec<CodingAgentLiveConfigOption>) -> CodingAgentLiveAcpDiagnostic {
    CodingAgentLiveAcpDiagnostic {
        outcome: CodingAgentDiagnoseOutcome::Ok,
        phase: CodingAgentDiagnosePhase::Projection,
        protocol_version: Some(1),
        options,
        options_truncated: false,
    }
}

fn select_option(id: &str, choices: Vec<String>) -> CodingAgentLiveConfigOption {
    CodingAgentLiveConfigOption {
        id: id.to_string(),
        label: Some(format!("{id} label")),
        kind: "select".to_string(),
        current: Some(choices.first().cloned().unwrap_or_default()),
        choices: choices
            .into_iter()
            .map(|value| CodingAgentLiveConfigChoice { value, label: None })
            .collect(),
        choices_truncated: false,
    }
}

fn success_response(
    env_presence: serde_json::Map<String, Value>,
    live: Option<CodingAgentLiveAcpDiagnostic>,
) -> CodingAgentResponse {
    CodingAgentResponse {
        dispatch_state: CodingAgentDispatchState::Completed,
        payload: Some(CodingAgentResponsePayload::Diagnose {
            provider_id: "codex".to_string(),
            provider_instance_id: "private-instance-codex".to_string(),
            env_presence: env_presence
                .into_iter()
                .map(|(name, value)| {
                    (
                        name,
                        value.as_bool().expect("test env presence must be boolean"),
                    )
                })
                .collect(),
            live_acp: live,
        }),
        error: None,
    }
}

// ---------------------------------------------------------------------------
// Authority: the live probe never inherits observation authority.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn diagnose_live_probe_requires_start_authority_before_any_resolution() {
    let runtime = test_runtime();
    // The scope gate fires before Project resolution or Runner observation:
    // an unknown project must not leak resolution details for an
    // under-authorized live probe either.
    let run_only = oauth(&["coding_agent:run"]);
    let result = runtime
        .coding_agent_diagnose(
            "agent:mini:demo".into(),
            "codex".into(),
            true,
            Some(&run_only),
        )
        .await;
    assert!(!result.success);
    assert_eq!(
        result.output["error_kind"],
        "coding_agent_live_probe_authority_required"
    );
    assert!(result
        .error
        .as_deref()
        .unwrap()
        .contains("start_coding_agent"));
    let write_only = oauth(&["project:write"]);
    let result = runtime
        .coding_agent_diagnose(
            "agent:mini:demo".into(),
            "codex".into(),
            true,
            Some(&write_only),
        )
        .await;
    assert_eq!(
        result.output["error_kind"],
        "coding_agent_live_probe_authority_required"
    );
}

#[tokio::test]
async fn diagnose_presence_only_needs_no_start_authority_and_authorized_live_probe_passes_the_gate()
{
    let runtime = test_runtime();
    // Presence-only under run-only authority proceeds past the authority gate;
    // the unregistered Project then fails at resolution instead.
    let run_only = oauth(&["coding_agent:run"]);
    let result = runtime
        .coding_agent_diagnose(
            "agent:mini:demo".into(),
            "codex".into(),
            false,
            Some(&run_only),
        )
        .await;
    assert!(!result.success);
    assert_ne!(
        result.output["error_kind"],
        "coding_agent_live_probe_authority_required"
    );
    // Fully authorized live probe passes the gate too (same resolver failure).
    let start_allowed = oauth(&["coding_agent:run", "project:write"]);
    let result = runtime
        .coding_agent_diagnose(
            "agent:mini:demo".into(),
            "codex".into(),
            true,
            Some(&start_allowed),
        )
        .await;
    assert_ne!(
        result.output["error_kind"],
        "coding_agent_live_probe_authority_required"
    );
}

#[tokio::test]
async fn diagnose_live_probe_is_permission_gated_per_project() {
    let client_id = "diagnose-restricted";
    let runtime = test_runtime()
        .with_permission_evaluator(PermissionEvaluator::with_mode(AuthorityMode::Restricted));
    register(&runtime, client_id, Some(vec![provider("codex")])).await;
    crate::test_support::apply_project_inventory_snapshot(
        &runtime.runner_registry,
        client_id,
        &format!("inst-{client_id}"),
        vec![registered_project("demo", "/tmp/acp-diagnose-demo")],
    )
    .await;
    let bootstrap = auth_context(Some("operator"), true);
    let result = runtime
        .coding_agent_diagnose(
            format!("agent:{client_id}:demo"),
            "codex".into(),
            true,
            Some(&bootstrap),
        )
        .await;
    assert!(!result.success);
    assert_eq!(result.output["error_kind"], "permission_denied");
    assert_eq!(result.output["permission"]["status"], "denied");
    // Presence-only stays permission-free even under restricted mode: the
    // unadvertised provider fails at inventory resolution, not at a gate.
    let result = runtime
        .coding_agent_diagnose(
            format!("agent:{client_id}:demo"),
            "missing".into(),
            false,
            Some(&bootstrap),
        )
        .await;
    assert!(!result.success);
    assert_eq!(
        result.output["error_kind"],
        "coding_agent_provider_unavailable"
    );
    assert!(result.output.get("permission").is_none());
}

// ---------------------------------------------------------------------------
// Exact Runner/provider generation resolution.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn diagnose_rejects_runner_without_coding_agent_capability() {
    let runtime = register_runner_with_project("plain-diagnose", None).await;
    let bootstrap = auth_context(Some("operator"), true);
    let result = runtime
        .coding_agent_diagnose(
            "agent:plain-diagnose:demo".into(),
            "codex".into(),
            false,
            Some(&bootstrap),
        )
        .await;
    assert!(!result.success);
    assert_eq!(result.output["error_kind"], "coding_agent_unsupported");
}

#[tokio::test]
async fn diagnose_unadvertised_provider_returns_safe_inventory_and_schema_valid_recovery() {
    let runtime =
        register_runner_with_project("mini", Some(vec![provider("pi"), provider("codex")])).await;
    let bootstrap = auth_context(Some("operator"), true);
    let result = runtime
        .coding_agent_diagnose(
            "agent:mini:demo".into(),
            "missing".into(),
            false,
            Some(&bootstrap),
        )
        .await;
    assert!(!result.success);
    assert_eq!(
        result.output["error_kind"],
        "coding_agent_provider_unavailable"
    );
    let inventory = &result.output["available_providers"];
    assert_eq!(
        inventory,
        &json!([
            {"provider_id": "pi", "name": "pi Agent"},
            {"provider_id": "codex", "name": "codex Agent"}
        ])
    );
    assert!(!result.output.to_string().contains("private-instance"));
    let suggested = &result.output["suggested_call"];
    webcodex_tool_contracts::test_support::validate_generated_tool_call_against_registered_input_schema(
        suggested,
    )
    .expect("diagnose inventory recovery must pass get_runtime_status registered inputSchema");
}

// ---------------------------------------------------------------------------
// Response mapping: identity, bounds, closed failures, non-disclosure.
// ---------------------------------------------------------------------------

#[test]
fn diagnose_response_with_mismatched_provider_identity_fails_closed() {
    let mut response = success_response(
        serde_json::Map::from_iter([("CODEX_PATH".to_string(), json!(true))]),
        None,
    );
    if let Some(CodingAgentResponsePayload::Diagnose {
        provider_instance_id,
        ..
    }) = response.payload.as_mut()
    {
        *provider_instance_id = "replaced-instance".to_string();
    }
    let result = diagnose_result_from_response(
        response,
        "agent:mini:demo",
        "codex",
        "private-instance-codex",
        "wc_agent_run_x",
        false,
    );
    assert!(!result.success);
    assert_eq!(result.output["error_kind"], "invalid_runner_response");
}

#[test]
fn diagnose_response_maps_runner_failures_to_closed_reasons() {
    let response = CodingAgentResponse::error(
        CodingAgentDispatchState::NotStarted,
        "stale_coding_agent_provider",
        "ACP provider instance was replaced",
        Some("stale_state"),
        Some("reobserve"),
    );
    let result = diagnose_result_from_response(
        response,
        "agent:mini:demo",
        "codex",
        "private-instance-codex",
        "wc_agent_run_x",
        false,
    );
    assert!(!result.success);
    assert_eq!(result.output["error_kind"], "stale_coding_agent_provider");
    assert_eq!(result.output["execution_state"], "not_started");

    let response = CodingAgentResponse {
        dispatch_state: CodingAgentDispatchState::Completed,
        payload: None,
        error: None,
    };
    let result = diagnose_result_from_response(
        response,
        "agent:mini:demo",
        "codex",
        "private-instance-codex",
        "wc_agent_run_x",
        false,
    );
    assert_eq!(result.output["error_kind"], "invalid_runner_response");
}

#[test]
fn diagnose_success_projection_is_bounded_and_discloses_nothing_sensitive() {
    let env_presence = serde_json::Map::from_iter([
        ("CODEX_PATH".to_string(), json!(true)),
        ("CODEX_HOME".to_string(), json!(false)),
    ]);
    let result = diagnose_result_from_response(
        success_response(
            env_presence,
            Some(live_ok(vec![select_option(
                "model",
                vec!["gpt-6-sol".to_string(), "gpt-6.1-sol".to_string()],
            )])),
        ),
        "agent:mini:demo",
        "codex",
        "private-instance-codex",
        "wc_agent_run_x",
        true,
    );
    assert!(result.success);
    let serialized = result.output.to_string();
    for forbidden in [
        "provider_instance_id",
        "private-instance",
        "CODEX_PATH=",
        "executable",
    ] {
        assert!(!serialized.contains(forbidden), "leaked: {forbidden}");
    }
    // Advertised live choice values (the model catalog) are the point of the
    // live projection and are intentionally visible.
    assert_eq!(result.output["project"], "agent:mini:demo");
    assert_eq!(result.output["provider_id"], "codex");
    assert_eq!(
        result.output["env_presence"],
        json!({"CODEX_PATH": true, "CODEX_HOME": false})
    );
    assert_eq!(result.output["live_acp"]["outcome"], "ok");
    assert_eq!(result.output["live_acp"]["phase"], "projection");
    assert_eq!(result.output["live_acp"]["protocol_version"], 1);
    // Presence-only projection: no live section beyond null.
    let result = diagnose_result_from_response(
        success_response(serde_json::Map::new(), None),
        "agent:mini:demo",
        "codex",
        "private-instance-codex",
        "wc_agent_run_x",
        false,
    );
    assert_eq!(result.output["live_acp"], Value::Null);

    // The full success payload validates against the registered output schema.
    let schema = webcodex_tool_contracts::registry::registered_tool_specs()
        .into_iter()
        .find(|spec| spec.name == "diagnose_coding_agent")
        .expect("diagnose_coding_agent must be a registered tool")
        .output_schema;
    let live = live_ok(vec![select_option("model", vec!["m1".to_string()])]);
    let payload = json!({
        "project": "agent:mini:demo",
        "provider_id": "codex",
        "env_presence": {"CODEX_PATH": true},
        "live_acp": serde_json::to_value(live).unwrap(),
    });
    let wrapped = json!({"success": true, "output": payload});
    webcodex_tool_contracts::test_support::validate_schema_instance(&wrapped, &schema)
        .expect("diagnose projection must match its registered output schema");
}

#[test]
fn diagnose_response_bounds_fail_closed() {
    let oversized = |count: usize, choices: usize| -> Vec<CodingAgentLiveConfigOption> {
        (0..count)
            .map(|index| {
                select_option(
                    &format!("option-{index}"),
                    (0..choices)
                        .map(|choice| format!("c{choice}"))
                        .collect::<Vec<_>>(),
                )
            })
            .collect()
    };
    for (label, live) in [
        (
            "too many options",
            live_ok(oversized(CODING_AGENT_MAX_CONFIG_OPTIONS + 1, 1)),
        ),
        (
            "too many choices",
            live_ok(vec![select_option(
                "model",
                (0..CODING_AGENT_MAX_CONFIG_SELECT_CHOICES + 1)
                    .map(|choice| format!("c{choice}"))
                    .collect::<Vec<_>>(),
            )]),
        ),
        ("success without options", live_ok(Vec::new())),
    ] {
        let result = diagnose_result_from_response(
            success_response(serde_json::Map::new(), Some(live)),
            "agent:mini:demo",
            "codex",
            "private-instance-codex",
            "wc_agent_run_x",
            true,
        );
        assert!(!result.success, "{label}");
        assert_eq!(result.output["error_kind"], "invalid_runner_response");
    }
    // A non-success outcome must never carry projected options.
    let mut failed = live_ok(vec![select_option("model", vec!["m1".to_string()])]);
    failed.outcome = CodingAgentDiagnoseOutcome::Timeout;
    failed.phase = CodingAgentDiagnosePhase::Initialize;
    let result = diagnose_result_from_response(
        success_response(serde_json::Map::new(), Some(failed)),
        "agent:mini:demo",
        "codex",
        "private-instance-codex",
        "wc_agent_run_x",
        true,
    );
    assert_eq!(result.output["error_kind"], "invalid_runner_response");
    // Oversized environment-presence map.
    let mut response = success_response(serde_json::Map::new(), None);
    if let Some(CodingAgentResponsePayload::Diagnose { env_presence, .. }) =
        response.payload.as_mut()
    {
        for index in 0..=CODING_AGENT_MAX_ENV_MAPPINGS {
            env_presence.insert(format!("KEY_{index}"), true);
        }
    }
    let result = diagnose_result_from_response(
        response,
        "agent:mini:demo",
        "codex",
        "private-instance-codex",
        "wc_agent_run_x",
        false,
    );
    assert_eq!(result.output["error_kind"], "invalid_runner_response");
}

#[test]
fn live_acp_bound_violation_rejects_unstable_identities() {
    let mut option = select_option("model", vec!["m1".to_string()]);
    option.kind = "freeform".to_string();
    assert!(live_acp_bound_violation(&live_ok(vec![option])).is_some());

    let mut option = select_option("model", vec!["m1".to_string()]);
    option.id = "x".repeat(CODING_AGENT_MAX_CONFIG_KEY_BYTES + 1);
    assert!(live_acp_bound_violation(&live_ok(vec![option])).is_some());

    let mut option = select_option("model", vec!["m1".to_string()]);
    option.label = Some("x".repeat(CODING_AGENT_MAX_CONFIG_LABEL_BYTES + 1));
    assert!(live_acp_bound_violation(&live_ok(vec![option])).is_some());

    let mut option = select_option("model", vec!["m1".to_string()]);
    option.choices[0].value = String::new();
    assert!(live_acp_bound_violation(&live_ok(vec![option])).is_some());

    let mut diagnostic = live_ok(vec![select_option("model", vec!["m1".to_string()])]);
    diagnostic.options_truncated = true;
    assert!(live_acp_bound_violation(&diagnostic).is_none());

    assert!(live_acp_bound_violation(&live_ok(vec![select_option(
        "model",
        vec!["m1".to_string()]
    )]))
    .is_none());
}

#[test]
fn diagnose_response_lost_preserves_possible_live_launch() {
    let live = diagnose_response_lost(true, "wc_agent_run_x");
    assert!(!live.success);
    assert_eq!(
        live.output["error_kind"],
        "coding_agent_diagnose_response_lost"
    );
    assert_eq!(live.output["execution_state"], "outcome_unknown");
    let presence = diagnose_response_lost(false, "wc_agent_run_x");
    assert_eq!(presence.output["execution_state"], "not_started");
}

#[test]
fn diagnose_response_rejects_mode_mismatch_and_unbounded_env_names() {
    let live = live_ok(vec![select_option("model", vec!["m1".to_string()])]);
    for (response, requested_live) in [
        (success_response(serde_json::Map::new(), Some(live)), false),
        (success_response(serde_json::Map::new(), None), true),
    ] {
        let result = diagnose_result_from_response(
            response,
            "agent:mini:demo",
            "codex",
            "private-instance-codex",
            "wc_agent_run_x",
            requested_live,
        );
        assert_eq!(result.output["error_kind"], "invalid_runner_response");
    }
    for invalid in [
        "X".repeat(webcodex_core::coding_agent::CODING_AGENT_MAX_ENV_NAME_BYTES + 1),
        "HAS=EQUALS".to_string(),
        "\u{4e2d}".to_string(),
    ] {
        let response = success_response(serde_json::Map::from_iter([(invalid, json!(true))]), None);
        let result = diagnose_result_from_response(
            response,
            "agent:mini:demo",
            "codex",
            "private-instance-codex",
            "wc_agent_run_x",
            false,
        );
        assert_eq!(result.output["error_kind"], "invalid_runner_response");
    }
}

#[test]
fn live_diagnostic_current_must_be_from_advertised_choices() {
    let mut unknown = select_option("model", vec!["model-a".to_string()]);
    unknown.current = Some("unadvertised-private-value".into());
    assert!(live_acp_bound_violation(&live_ok(vec![unknown])).is_some());

    let mut bool_option = select_option("auto", vec![]);
    bool_option.kind = "boolean".into();
    bool_option.current = Some("true".into());
    assert!(live_acp_bound_violation(&live_ok(vec![bool_option.clone()])).is_none());
    bool_option.current = Some("secret".into());
    assert!(live_acp_bound_violation(&live_ok(vec![bool_option.clone()])).is_some());
    bool_option.current = Some("false".into());
    bool_option.choices.push(CodingAgentLiveConfigChoice {
        value: "unexpected".into(),
        label: None,
    });
    assert!(live_acp_bound_violation(&live_ok(vec![bool_option])).is_some());
}

// Full governance path: the router pre-resolves the Project, then the runtime
// authority gate must still refuse the live probe. Presence-only observation
// authority (coding_agent:run alone) must never launch the provider process.
#[tokio::test]
async fn diagnose_live_probe_authority_holds_through_full_dispatch() {
    let runtime = test_runtime();
    let run_only = oauth(&["coding_agent:run", "project:read"]);
    runtime
        .runner_registry
        .register_with_auth(
            RunnerRegisterRequest {
                owner: run_only.username.clone(),
                coding_agent_inventory: Some(Default::default()),
                coding_agent_providers: Some(vec![provider("codex")]),
                ..crate::test_support::runner_registration(
                    "mini",
                    "inst-mini",
                    RunnerCapabilities {
                        coding_agent_runs: true,
                        ..Default::default()
                    },
                )
            },
            Some(crate::test_support::runner_access(&run_only)).as_ref(),
        )
        .await
        .unwrap();
    crate::test_support::apply_project_inventory_snapshot(
        &runtime.runner_registry,
        "mini",
        "inst-mini",
        vec![registered_project("demo", "/tmp/acp-diagnose-demo")],
    )
    .await;
    let result = runtime
        .dispatch_with_auth(
            ToolCall::CodingAgentDiagnose {
                project: "agent:mini:demo".into(),
                provider_id: "codex".into(),
                include_live_acp: true,
            },
            Some(&run_only),
        )
        .await;
    assert!(!result.success);
    assert_eq!(
        result.output["error_kind"],
        "coding_agent_live_probe_authority_required"
    );
    assert!(result.error.as_deref().unwrap().contains("project:write"));
}
