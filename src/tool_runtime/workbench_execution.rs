//! MCP Workbench execution orchestration. The durable Task/Attempt and existing
//! backends own all effects; the adapter stores only immutable replay intent.
use super::kernel::{
    check_runtime_tool_scope, HostFileImportTrust, ToolCallContext, ToolCallRequest, ToolTransport,
};
use super::{ToolResult, ToolRuntime};
use crate::auth::AuthContext;
use crate::client_window::ClientWindow;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Submission {
    project: String,
    title: String,
    instruction: String,
    execution: Execution,
    goal_id: Option<String>,
    idempotency_key: String,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Execution {
    kind: String,
    agent_id: String,
    provider_id: Option<String>,
    endpoint_id: Option<String>,
    expected_controller_generation: Option<i64>,
}

fn failure(kind: &str, message: &str) -> ToolResult {
    ToolResult::err_with_output(message, json!({"error_kind":kind,"state_changed":false}))
}

fn child_key(key: &str, step: &str) -> String {
    format!("workbench:{step}:{:x}", Sha256::digest(key.as_bytes()))
}

impl ToolRuntime {
    pub(crate) async fn workbench_call(
        &self,
        name: &str,
        arguments: Value,
        auth: Option<&AuthContext>,
        window: Option<&ClientWindow>,
    ) -> ToolResult {
        let outcome = Box::pin(self.call_tool_with_context(
            ToolCallRequest {
                tool_name: name.into(),
                arguments,
            },
            ToolCallContext {
                transport: ToolTransport::Api,
                session_id: None,
                auth,
                window,
                record_oauth_scope_denials: false,
                host_file_import_trust: HostFileImportTrust::Untrusted,
            },
        ))
        .await;
        outcome.result.unwrap_or_else(|| {
            failure(
                "workbench_domain_denied",
                "The domain operation is unavailable under current authority",
            )
        })
    }

    pub(crate) async fn enrich_workbench_execution(
        &self,
        output: &mut Value,
        project: Option<&str>,
        auth: Option<&AuthContext>,
        window: Option<&ClientWindow>,
    ) {
        let Ok(principal) = super::communication::communication_principal(auth) else {
            return;
        };
        let Some(db) = self.communication_db.as_ref() else {
            return;
        };
        let can_create = check_runtime_tool_scope(auth, "create_agent_task").is_ok()
            && check_runtime_tool_scope(auth, "start_agent_task_attempt").is_ok();
        let mut options = Vec::new();
        let runner_available = if let Some(project) = project {
            match self.resolve_project_input_for_auth(project, auth).await {
                Ok(resolved) if resolved.config.allow_patch => self
                    .runner_registry
                    .get_runner_semantic_view_for_auth(
                        &resolved.config.client_id,
                        crate::runner_http::runner_access_from_auth(auth).as_ref(),
                    )
                    .await
                    .is_some_and(|runner| {
                        runner.view.connected
                            && runner.supports(crate::runner_http::RunnerFeature::CodingAgentRuns)
                    }),
                _ => false,
            }
        } else {
            false
        };
        if can_create {
            if let Some(agents) = output.pointer("/agents/agents").and_then(Value::as_array) {
                for agent in agents.iter().take(50) {
                    if options.len() >= 100 {
                        break;
                    }
                    let Some(agent_id) = agent.get("agent_id").and_then(Value::as_str) else {
                        continue;
                    };
                    let display = agent
                        .get("display_name")
                        .and_then(Value::as_str)
                        .unwrap_or(agent_id);
                    if check_runtime_tool_scope(auth, "start_agent_task_endpoint_continuation")
                        .is_ok()
                    {
                        if let (Some(window), Some(controller)) =
                            (window, self.agent_continuations.as_ref())
                        {
                            if let Some((endpoint_id, generation)) = controller
                                .workbench_window_endpoint(&principal, agent_id, window.key())
                            {
                                options.push(json!({"kind":"chat","agent_id":agent_id,"display_name":display,"endpoint_id":endpoint_id,"expected_controller_generation":generation}));
                            }
                        }
                    }
                    if runner_available
                        && output.get("project").and_then(Value::as_str).is_some()
                        && check_runtime_tool_scope(auth, "start_agent_task_coding_run").is_ok()
                    {
                        if let Some(providers) = output.get("providers").and_then(Value::as_array) {
                            for provider in providers.iter().take(16) {
                                if let Some(id) =
                                    provider.get("provider_id").and_then(Value::as_str)
                                {
                                    if options.len() < 100 {
                                        options.push(json!({"kind":"runner","agent_id":agent_id,"display_name":display,"provider_id":id}));
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
        output["execution_options"] = json!(options);
        // The form may open before a Project is selected; creation authority is
        // distinct from whether that explicit selection currently has an executor.
        output["capabilities"]["submit_work"] = json!(can_create);
        let mut source_error = None;
        if let Some(tasks) = output
            .pointer_mut("/tasks/tasks")
            .and_then(Value::as_array_mut)
        {
            for task in tasks.iter_mut().take(50) {
                let Some(task_id) = task
                    .get("task_id")
                    .and_then(Value::as_str)
                    .map(str::to_owned)
                else {
                    continue;
                };
                match db.is_workbench_submission_task(&principal, &task_id) {
                    Ok(true) => task["workbench_source"] = json!("form"),
                    Ok(false) => {}
                    Err(error) => {
                        source_error =
                            Some(json!({"error_kind":error.code(),"message":error.message()}))
                    }
                }
                let Some(attempt_id) = task
                    .pointer("/latest_attempt/attempt_id")
                    .and_then(Value::as_str)
                else {
                    continue;
                };
                let Ok(binding) =
                    db.read_agent_task_coding_run_binding(&principal, &task_id, attempt_id)
                else {
                    continue;
                };
                if self
                    .resolve_project_input_for_auth(&binding.runtime_project_id, auth)
                    .await
                    .is_err()
                {
                    continue;
                }
                let mut actions = json!({});
                if check_runtime_tool_scope(auth, "reconcile_agent_task_coding_run").is_ok() {
                    actions["reconcile"] =
                        json!({"task_id":binding.task_id,"attempt_id":binding.attempt_id});
                }
                task["workbench_actions"] = actions.clone();
                if check_runtime_tool_scope(auth, "observe_coding_agent").is_err() {
                    continue;
                }
                // Reuse the authority-checked retained snapshot/Runner inventory.
                // Homepage reads must never enqueue one blocking Observe per card.
                let Ok(Some(snapshot)) = self
                    .reconcile_coding_agent_run_snapshot(&binding.run_id, auth)
                    .await
                else {
                    task["current_action"] = json!("Run observation unavailable; reconcile the original Attempt before further execution");
                    continue;
                };
                actions["observe"] =
                    json!({"project":binding.runtime_project_id,"run_id":binding.run_id});
                if !snapshot.state.terminal()
                    && binding.terminal_at_unix_ms.is_none()
                    && binding.dispatch_state == crate::db::AgentTaskCodingRunDispatchState::Bound
                    && check_runtime_tool_scope(auth, "cancel_coding_agent").is_ok()
                {
                    actions["cancel"] =
                        json!({"project":binding.runtime_project_id,"run_id":binding.run_id});
                }
                task["workbench_actions"] = actions;
                task["execution_observation"] = json!({
                    "state": snapshot.state.as_str(),
                    "execution_state": snapshot.execution_state.as_str(),
                    "updated_at": snapshot.updated_at,
                    "source": "retained_runner_inventory",
                });
                task["current_action"] = json!(format!(
                    "{} · {}",
                    snapshot.state.as_str(),
                    snapshot.execution_state.as_str()
                ));
            }
        }
        if let Some(error) = source_error {
            output["errors"]["task_sources"] = error;
        }
    }

    pub(crate) async fn submit_workbench_work(
        &self,
        arguments: Value,
        auth: Option<&AuthContext>,
        window: Option<&ClientWindow>,
    ) -> ToolResult {
        let input: Submission = match serde_json::from_value(arguments) {
            Ok(value) => value,
            Err(_) => {
                return failure(
                    "invalid_workbench_submission",
                    "Choose an explicit Project, assignee, executor and task instruction",
                )
            }
        };
        if input.title.trim().is_empty()
            || input.title.chars().count() > 200
            || input.instruction.trim().is_empty()
            || input.instruction.len() > 8192
            || input.idempotency_key.is_empty()
            || input.idempotency_key.len() > 128
            || input.idempotency_key.chars().any(char::is_control)
        {
            return failure(
                "invalid_workbench_submission",
                "Task title, instruction or submission key is invalid",
            );
        }
        if !matches!(input.execution.kind.as_str(), "chat" | "runner")
            || (input.execution.kind == "runner"
                && (input.execution.provider_id.is_none()
                    || input.execution.endpoint_id.is_some()
                    || input.execution.expected_controller_generation.is_some()))
            || (input.execution.kind == "chat"
                && (input.execution.provider_id.is_some()
                    || input.execution.endpoint_id.is_none()
                    || input.execution.expected_controller_generation.is_none()))
        {
            return failure(
                "invalid_workbench_executor",
                "Choose one exact available execution route",
            );
        }
        for name in [
            "create_agent_task",
            "start_agent_task_attempt",
            if input.execution.kind == "chat" {
                "start_agent_task_endpoint_continuation"
            } else {
                "start_agent_task_coding_run"
            },
        ] {
            if check_runtime_tool_scope(auth, name).is_err() {
                return failure(
                    "workbench_execution_denied",
                    "The selected execution route is not authorized",
                );
            }
        }
        let principal = match super::communication::communication_principal(auth) {
            Ok(p) => p,
            Err(e) => return e,
        };
        let Some(db) = self.communication_db.as_ref() else {
            return failure(
                "agent_task_store_unavailable",
                "Durable work storage is unavailable",
            );
        };
        let resolved = match self
            .resolve_project_input_for_auth(&input.project, auth)
            .await
        {
            Ok(p) => p,
            Err(_) => return failure("project_unavailable", "The selected Project is unavailable"),
        };
        let agent = self
            .workbench_call(
                "list_agent_identities",
                json!({"agent_id":input.execution.agent_id,"limit":1}),
                auth,
                window,
            )
            .await;
        if !agent.success {
            return agent;
        }
        if !agent
            .output
            .get("agents")
            .and_then(Value::as_array)
            .is_some_and(|a| a.len() == 1)
        {
            return failure(
                "workbench_assignee_unavailable",
                "The selected Agent is unavailable",
            );
        }
        if let Some(goal_id) = input.goal_id.as_ref() {
            let goal = self
                .workbench_call("get_goal", json!({"goal_id":goal_id}), auth, window)
                .await;
            if !goal.success {
                return goal;
            }
        }
        if input.execution.kind == "chat" {
            let route = window
                .zip(self.agent_continuations.as_ref())
                .and_then(|(w, c)| {
                    c.workbench_window_endpoint(&principal, &input.execution.agent_id, w.key())
                });
            if route.as_ref().is_none_or(|(endpoint, generation)| {
                Some(endpoint) != input.execution.endpoint_id.as_ref()
                    || Some(*generation) != input.execution.expected_controller_generation
            }) {
                return failure("workbench_chat_unavailable","This chat's exact continuation route changed or is unavailable; refresh available executors");
            }
        }
        let mut intent =
            serde_json::to_value(&input).expect("typed Workbench submission serializes");
        intent.as_object_mut().unwrap().remove("idempotency_key");
        let request_hash = format!("{:x}", Sha256::digest(serde_json::to_vec(&intent).unwrap()));
        if let Err(error) =
            db.begin_workbench_submission(&principal, &input.idempotency_key, &request_hash)
        {
            return failure(error.code(), error.message());
        }
        let created=self.workbench_call("create_agent_task",json!({"title":input.title,"instruction":input.instruction,"assignee_agent_id":input.execution.agent_id,"referenced_project_id":resolved.resolved_id,"idempotency_key":child_key(&input.idempotency_key,"create")}),auth,window).await;
        if !created.success {
            return created;
        }
        let Some(task_id) = created
            .output
            .pointer("/task/summary/task_id")
            .and_then(Value::as_str)
            .map(str::to_owned)
        else {
            return failure(
                "workbench_invalid_task_result",
                "The Task result did not include exact identity; retry unchanged submission only",
            );
        };
        if let Err(error) =
            db.bind_workbench_submission_task(&principal, &input.idempotency_key, &task_id)
        {
            return submission_failure(
                failure(error.code(), error.message()),
                &task_id,
                "source_association",
            );
        }
        if let Some(goal_id) = input.goal_id {
            let associated=self.workbench_call("associate_goal_agent_task",json!({"goal_id":goal_id,"task_id":task_id,"idempotency_key":child_key(&input.idempotency_key,"goal")}),auth,window).await;
            if !associated.success {
                return submission_failure(associated, &task_id, "goal_association");
            }
        }
        if created
            .output
            .pointer("/task/summary/state")
            .and_then(Value::as_str)
            .is_some_and(|s| matches!(s, "succeeded" | "failed"))
        {
            return ToolResult::ok(
                json!({"task_id":task_id,"task":created.output["task"],"replayed":true}),
            );
        }
        let attempt=self.workbench_call("start_agent_task_attempt",json!({"task_id":task_id,"assignee_agent_id":input.execution.agent_id,"idempotency_key":child_key(&input.idempotency_key,"attempt")}),auth,window).await;
        if !attempt.success {
            return submission_failure(attempt, &task_id, "attempt");
        }
        let Some(attempt_ref) = attempt.output.get("attempt_ref").and_then(Value::as_str) else {
            return submission_failure(
                failure(
                    "workbench_attempt_unavailable",
                    "The exact Attempt selector is unavailable; reconcile the original Task",
                ),
                &task_id,
                "attempt",
            );
        };
        let dispatched = if input.execution.kind == "chat" {
            self.start_workbench_agent_task_endpoint_continuation(
                auth,
                attempt_ref.to_owned(),
                input
                    .execution
                    .endpoint_id
                    .expect("validated chat endpoint"),
                input
                    .execution
                    .expected_controller_generation
                    .expect("validated chat generation"),
            )
        } else {
            self.workbench_call(
                "start_agent_task_coding_run",
                json!({"project":input.project,"attempt_ref":attempt_ref,"provider_id":input.execution.provider_id}),
                auth,
                window,
            ).await
        };
        if !dispatched.success {
            return submission_failure(dispatched, &task_id, "dispatch");
        }
        ToolResult::ok(
            json!({"task_id":task_id,"attempt":attempt.output["attempt"],"execution":dispatched.output,"replayed":created.output["replayed"]}),
        )
    }
}

fn submission_failure(mut result: ToolResult, task_id: &str, phase: &str) -> ToolResult {
    result.output["workbench_submission"] =
        json!({"task_id":task_id,"phase":phase,"retry":"same_key_and_unchanged_input_only"});
    result
}
