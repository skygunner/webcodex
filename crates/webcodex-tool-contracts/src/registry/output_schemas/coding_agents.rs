use serde_json::{json, Value};
use webcodex_core::coding_agent::{
    CodingAgentEventKind, CodingAgentExecutionState, CodingAgentLiveAcpDiagnostic,
    CodingAgentProviderSummary, CodingAgentRunState, CODING_AGENT_MAX_ENV_MAPPINGS,
};

use crate::schema_generation::typed_host_schema;

use super::common::{
    array_schema, nullable_schema, schema_type, suggested_tool_call_schema, wrapped_output_schema,
};

pub(super) fn provider_inventory_schema() -> Value {
    json!({
        "type": "array",
        "description": "Current exact Runner ACP discovery; logical IDs and names only. Does not select a default or grant execution authority.",
        "maxItems": webcodex_core::coding_agent::CODING_AGENT_MAX_PROVIDERS,
        "items": typed_host_schema::<CodingAgentProviderSummary>()
    })
}

fn terminal_schema() -> Value {
    json!({
        "anyOf": [
            {
                "type":"object",
                "additionalProperties":false,
                "properties":{
                    "stop_reason": nullable_schema("string", "Correlated stable ACP v1 stop reason when available."),
                    "error_code": nullable_schema("string", "Bounded protocol/provider terminal error code when available."),
                    "message": nullable_schema("string", "Bounded terminal diagnostic; never reasoning or a transcript."),
                    "completed_at": schema_type("integer", "Unix terminal timestamp.")
                },
                "required":["stop_reason","error_code","message","completed_at"]
            },
            {"type":"null"}
        ]
    })
}

fn usage_schema() -> Value {
    json!({
        "type":"object",
        "additionalProperties":false,
        "properties":{
            "used_tokens":{"type":"integer","minimum":0},
            "context_window_tokens":{"type":"integer","minimum":0},
            "cost_amount":{"type":"string"},
            "cost_currency":{"type":"string"}
        }
    })
}

fn event_schema() -> Value {
    json!({
        "type":"object",
        "additionalProperties":false,
        "properties":{
            "sequence":{"type":"integer","minimum":1},
            "kind": typed_host_schema::<CodingAgentEventKind>(),
            "text":nullable_schema("string", "Bounded normalized text when this event kind carries text."),
            "label":nullable_schema("string", "Bounded normalized activity label when present."),
            "status":nullable_schema("string", "Bounded normalized activity status when present."),
            "usage":{"anyOf":[usage_schema(),{"type":"null"}]}
        },
        "required":["sequence","kind","text","label","status","usage"]
    })
}

fn common_run_fields() -> Vec<(&'static str, Value)> {
    vec![
        ("run_id", schema_type("string", "Opaque CodingAgentRun id.")),
        (
            "project",
            schema_type("string", "Exact registered runtime Project id."),
        ),
        (
            "provider_id",
            schema_type("string", "Logical operator-configured provider id."),
        ),
        ("state", typed_host_schema::<CodingAgentRunState>()),
        (
            "execution_state",
            typed_host_schema::<CodingAgentExecutionState>(),
        ),
        ("terminal", terminal_schema()),
        (
            "error_kind",
            schema_type(
                "string",
                "Bounded failure classification when unsuccessful.",
            ),
        ),
    ]
}

pub(super) fn output_schema_for_tool(name: &str) -> Option<Value> {
    let mut fields = common_run_fields();
    match name {
        "start_coding_agent" => {
            fields.push(("available_providers", provider_inventory_schema()));
            fields.push((
                "suggested_call",
                suggested_tool_call_schema(
                    webcodex_core::runtime_contract::GeneratedFollowUpKind::FallbackRecovery,
                    "get_runtime_status",
                    json!({
                        "type": "object",
                        "additionalProperties": false,
                        "properties": {
                            "client_id": {"type": "string"},
                            "compact": {"type": "boolean", "const": true}
                        },
                        "required": ["client_id", "compact"]
                    }),
                    "Re-observe the exact Runner inventory when the requested logical coding-agent provider is unavailable. This is recovery, not an automatic provider-selection step.",
                ),
            ));
            fields.push((
                "observation_token",
                schema_type("string", "Opaque Run-bound observation token."),
            ));
            Some(wrapped_output_schema(fields))
        }
        "observe_coding_agent" => {
            fields.extend([
                ("events", array_schema(event_schema(), "Only-new retained normalized CodingAgentRun events; raw ACP JSON is never exposed.")),
                ("observation_token", schema_type("string", "Opaque Run-bound token for the next observation.")),
                ("has_more", schema_type("boolean", "True when retained newer events remain after this page.")),
                ("history_lost", schema_type("boolean", "True when the requested cursor predates retained history or the Server epoch rebaselined.")),
                ("first_retained_sequence", schema_type("integer", "First currently retained Runner event sequence.")),
            ]);
            Some(wrapped_output_schema(fields))
        }
        "cancel_coding_agent" => {
            fields.push((
                "cancel_requested",
                schema_type(
                    "boolean",
                    "True when cancellation was requested for a nonterminal Run.",
                ),
            ));
            Some(wrapped_output_schema(fields))
        }
        "diagnose_coding_agent" => {
            // No Run exists for a diagnostic, so common_run_fields does not
            // apply: the projection is the bounded diagnostic itself.
            let fields = vec![
                (
                    "project",
                    schema_type(
                        "string",
                        "Exact registered runtime Project id the diagnostic was authorized against.",
                    ),
                ),
                (
                    "provider_id",
                    schema_type(
                        "string",
                        "Logical operator-configured provider id that was diagnosed.",
                    ),
                ),
                (
                    "env_presence",
                    json!({
                        "type": "object",
                        "description": "Declared env_from_env destination names mapped to presence booleans read from the running Runner process. Never values and never unrelated environment names.",
                        "additionalProperties": {"type": "boolean"},
                        "maxProperties": CODING_AGENT_MAX_ENV_MAPPINGS
                    }),
                ),
                (
                    "live_acp",
                    json!({
                        "description": "Present only when the request explicitly authorized the live ACP probe; otherwise absent.",
                        "anyOf": [
                            typed_host_schema::<CodingAgentLiveAcpDiagnostic>(),
                            {"type": "null"}
                        ]
                    }),
                ),
            ];
            Some(wrapped_output_schema(fields))
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::validate_schema_instance;

    #[test]
    fn coding_agent_state_schemas_preserve_wire_vocabularies() {
        for tool in [
            "start_coding_agent",
            "observe_coding_agent",
            "cancel_coding_agent",
        ] {
            let schema = output_schema_for_tool(tool).unwrap();
            let fields = &schema["properties"]["output"]["properties"];
            assert_eq!(
                fields["state"],
                json!({"type":"string","enum":[
                    "starting", "running", "waiting_permission", "completed", "failed", "cancelled", "lost"
                ]})
            );
            assert_eq!(
                fields["execution_state"],
                json!({"type":"string","enum":[
                    "not_started", "started", "outcome_unknown", "completed"
                ]})
            );
            for wire in fields["state"]["enum"].as_array().unwrap() {
                let state: CodingAgentRunState = serde_json::from_value(wire.clone()).unwrap();
                assert_eq!(serde_json::to_value(state).unwrap(), *wire);
            }
            for wire in fields["execution_state"]["enum"].as_array().unwrap() {
                let state: CodingAgentExecutionState =
                    serde_json::from_value(wire.clone()).unwrap();
                assert_eq!(serde_json::to_value(state).unwrap(), *wire);
            }
            for field in ["state", "execution_state"] {
                assert!(validate_schema_instance(&json!("unknown"), &fields[field]).is_err());
                assert!(validate_schema_instance(&Value::Null, &fields[field]).is_err());
            }
        }
    }

    #[test]
    fn coding_agent_provider_schema_keeps_safe_fields_and_bounds() {
        use webcodex_core::coding_agent::{
            safe_provider_inventory, CodingAgentProvider, CODING_AGENT_MAX_PROVIDERS,
            CODING_AGENT_MAX_PROVIDER_ID_BYTES, CODING_AGENT_MAX_PROVIDER_NAME_BYTES,
        };

        let schema = provider_inventory_schema();
        let safe = safe_provider_inventory(Some(&[CodingAgentProvider {
            provider_id: "codex".into(),
            provider_instance_id: "private-instance".into(),
            name: "Codex".into(),
        }]));
        let sample = serde_json::to_value(safe).unwrap();
        validate_schema_instance(&sample, &schema).unwrap();
        assert_eq!(sample, json!([{"provider_id":"codex", "name":"Codex"}]));
        assert_eq!(schema["items"]["additionalProperties"], false);
        assert!(schema["items"]["properties"]
            .get("provider_instance_id")
            .is_none());
        for invalid in [
            json!([{"provider_id":"", "name":"Codex"}]),
            json!([{"provider_id":"codex", "name":""}]),
            json!([{"provider_id":"x".repeat(CODING_AGENT_MAX_PROVIDER_ID_BYTES + 1), "name":"Codex"}]),
            json!([{"provider_id":"codex", "name":"x".repeat(CODING_AGENT_MAX_PROVIDER_NAME_BYTES + 1)}]),
            json!([{"provider_id":"codex"}]),
            json!([{"provider_id":"codex", "name":"Codex", "provider_instance_id":"private-instance"}]),
            Value::Array(vec![sample[0].clone(); CODING_AGENT_MAX_PROVIDERS + 1]),
        ] {
            assert!(
                validate_schema_instance(&invalid, &schema).is_err(),
                "{invalid}"
            );
        }
        validate_schema_instance(
            &json!([{
                "provider_id":"x".repeat(CODING_AGENT_MAX_PROVIDER_ID_BYTES),
                "name":"x".repeat(CODING_AGENT_MAX_PROVIDER_NAME_BYTES)
            }]),
            &schema,
        )
        .unwrap();
    }

    #[test]
    fn coding_agent_event_schema_is_closed_and_terminal_is_nullable() {
        let event = event_schema();
        assert_eq!(event["additionalProperties"], false);
        assert_eq!(
            event["properties"]["kind"]["enum"],
            json!([
                "agent_message",
                "reasoning",
                "plan",
                "tool_activity",
                "file_change",
                "terminal_activity",
                "usage",
                "permission_request",
                "terminal"
            ])
        );
        assert_eq!(
            event["properties"]["usage"]["anyOf"][0]["additionalProperties"],
            false
        );
        assert_eq!(event["properties"]["usage"]["anyOf"][1]["type"], "null");

        let terminal = terminal_schema();
        assert_eq!(terminal["anyOf"][0]["additionalProperties"], false);
        assert_eq!(terminal["anyOf"][1]["type"], "null");

        let observe = output_schema_for_tool("observe_coding_agent").unwrap();
        let serialized = serde_json::to_string(&observe).unwrap();
        assert!(serialized.contains("agent_message"));
        assert!(!serialized.contains("agentmessage"));
    }
}
