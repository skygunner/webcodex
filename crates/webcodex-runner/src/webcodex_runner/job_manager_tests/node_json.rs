use super::*;
use webcodex_core::project_validation::{
    ProjectValidationAction, ProjectValidationAdapter, ProjectValidationRequest,
};
use webcodex_core::runner_operation::{
    RunnerInvocationMetadata, RunnerJobValidationOperation, RunnerOperation,
};
use webcodex_core::runner_protocol::{ShellJobContext, ShellJobValidationMetadata};

#[cfg(unix)]
#[test]
#[ignore = "real Node fixture: set WEBCODEX_NODE_FIXTURE_MODULES to disposable installed Vitest/Jest node_modules"]
fn node_json_real_framework_jobs_retain_counts_and_reject_interrupted_reports() {
    let modules = PathBuf::from(
        std::env::var_os("WEBCODEX_NODE_FIXTURE_MODULES")
            .expect("explicit isolated fixture dependencies"),
    );
    for framework in ["vitest", "jest"] {
        for initial_scenario in ["pass", "fail", "zero", "todo", "mixed", "timeout", "cancel"] {
            let temp = tempfile::tempdir().unwrap();
            let root = temp.path().join("project");
            let registry = temp.path().join("registry");
            std::fs::create_dir_all(&root).unwrap();
            std::fs::create_dir_all(&registry).unwrap();
            std::os::unix::fs::symlink(&modules, root.join("node_modules")).unwrap();
            // The failed case is repaired in this same project, then admitted as a new Job.
            let attempts = if initial_scenario == "fail" {
                vec!["fail", "repair"]
            } else {
                vec![initial_scenario]
            };
            for scenario in attempts {
                let job_id = format!("node-json-{scenario}");
                std::fs::write(
                    root.join("package.json"),
                    serde_json::to_vec(&json!({"scripts":{"test":framework}})).unwrap(),
                )
                .unwrap();
                let import = if framework == "vitest" {
                    "const {test,expect}=await import('vitest');"
                } else {
                    ""
                };
                let body=match scenario {
                "pass" | "repair"=>"test('works',()=>{console.log('arbitrary tests passed 999');expect(1).toBe(1)});",
                "fail"=>"test('fails',()=>{expect(1).toBe(2)});",
                "zero"=>"test.skip('skipped',()=>{});",
                "todo"=>"test.todo('planned');",
                "mixed"=>"test('works',()=>{expect(1).toBe(1)});test.skip('skipped',()=>{});test.todo('planned');",
                _=>"test('waits',async()=>{await new Promise(r=>setTimeout(r,30000))},60000);",
            };
                let source = format!("{import}\n{body}");
                std::fs::write(
                    root.join(if framework == "vitest" {
                        "sample.test.mjs"
                    } else {
                        "sample.test.cjs"
                    }),
                    source,
                )
                .unwrap();
                std::fs::write(
                    registry.join("demo.toml"),
                    format!(
                        "id='demo'\nname='Demo'\npath={}\nallow_patch=true\n",
                        serde_json::to_string(root.to_str().unwrap()).unwrap()
                    ),
                )
                .unwrap();
                let policy = RunnerPolicy {
                    allowed_roots: vec![root.clone()],
                    ..Default::default()
                };
                let request = ProjectValidationRequest {
                    project_id: "demo".into(),
                    cwd: None,
                    action: ProjectValidationAction::Test,
                    adapter: ProjectValidationAdapter::Node,
                    scope: None,
                    dependency_policy: None,
                    test: None,
                };
                let (plan, cwd) =
                    crate::webcodex_runner::validation::project::plan(&policy, &registry, &request)
                        .unwrap();
                let timeout_secs = if scenario == "timeout" { 1 } else { 60 };
                let metadata = ShellJobValidationMetadata {
                    project_validation: Some(plan.provenance),
                    source_fence: None,
                    tool: "project_validate".into(),
                    kind: "test".into(),
                    steps: vec![plan.step.clone()],
                    effective_timeout_secs: timeout_secs,
                    sync_wait_secs: 1,
                    adapter: plan.adapter,
                    validation_target_id: Some(plan.validation_target_id),
                    minimum_tests: Some(1),
                    require_tests: Some(true),
                    no_run: None,
                };
                assert!(metadata.is_valid());
                let context = ShellJobContext {
                    runtime_project_id: Some("agent:validation-agent:demo".into()),
                    validation: Some(metadata),
                    workflow_session_id: None,
                    ssh_resource: None,
                    project_cwd: Some(".".into()),
                    cwd: Some(cwd.to_string_lossy().into_owned()),
                    purpose: Some("validation".into()),
                    shell: None,
                    command_preview: "project validation".into(),
                    validation_steps: vec!["test".into()],
                    structured_execution: None,
                };
                let operation = RunnerJobOperation::StartValidation(RunnerJobValidationOperation {
                    job_id: job_id.clone(),
                    cwd: Some(cwd.to_string_lossy().into_owned()),
                    steps: vec![plan.step],
                    timeout_secs,
                    context,
                });
                let wire = RunnerRequest::from_operation(
                    RunnerInvocationMetadata {
                        request_id: format!("node-request-{scenario}"),
                        client_id: "validation-agent".into(),
                        requested_by: "test".into(),
                        created_at: chrono::Utc::now().timestamp(),
                    },
                    RunnerOperation::Job(operation),
                )
                .unwrap();
                let manager = JobManager::new(1);
                let (sink, _rx) = structured_test_sink("validation-agent", "validation-instance");
                manager.enqueue(
                    sink,
                    PendingJobStart::from_wire(
                        1,
                        policy,
                        ShellConfig::default(),
                        SshConfig::default(),
                        registry.clone(),
                        wire,
                    ),
                );
                if scenario == "cancel" {
                    let deadline = Instant::now() + Duration::from_secs(10);
                    while lock_unpoison(&manager.jobs)
                        .get(&job_id)
                        .unwrap()
                        .snapshot
                        .status
                        != "running"
                        && Instant::now() < deadline
                    {
                        std::thread::sleep(Duration::from_millis(20));
                    }
                    manager.stop_all();
                }
                assert!(
                    manager.wait_for_workers(Instant::now() + Duration::from_secs(30)),
                    "{framework}/{scenario} did not terminate"
                );
                let snapshot = lock_unpoison(&manager.jobs)
                    .get(&job_id)
                    .unwrap()
                    .snapshot
                    .clone();
                manager.stop_all();
                let evidence = snapshot
                    .test_count_evidence
                    .as_ref()
                    .expect("durable report evidence");
                match scenario {
                    "pass" | "repair" | "mixed" => {
                        assert_eq!(snapshot.status, "completed", "{framework}: {snapshot:?}");
                        assert_eq!(evidence.tests_run_count, Some(1));
                        assert_eq!(evidence.tests_passed, Some(1));
                    }
                    "fail" => {
                        assert_eq!(snapshot.status, "failed", "{framework}: {snapshot:?}");
                        assert_eq!(evidence.tests_run_count, Some(1));
                        assert_eq!(evidence.tests_failed, Some(1));
                    }
                    "zero" | "todo" => {
                        assert_eq!(
                            evidence.tests_run_count,
                            Some(0),
                            "{framework}: {snapshot:?}"
                        );
                    }
                    _ => {
                        assert!(
                            matches!(snapshot.status.as_str(), "timeout" | "stopped"),
                            "{framework}: {snapshot:?}"
                        );
                        assert_eq!(evidence.tests_run_count, None);
                    }
                }
                // Reconnect inventory carries the exact aggregate, not report bytes/path.
                let restored: webcodex_core::runner_protocol::ShellJobSnapshot =
                    serde_json::from_slice(&serde_json::to_vec(&snapshot).unwrap()).unwrap();
                assert_eq!(restored.test_count_evidence, snapshot.test_count_evidence);
                assert!(!serde_json::to_string(evidence)
                    .unwrap()
                    .contains("webcodex-node-report-"));
            }
        }
    }
}
