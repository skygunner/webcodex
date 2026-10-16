use super::super::diagnose::{project_live_options, read_bounded_diagnostic_frame};
use super::super::protocol::{wait_response, AcpFailure, ReaderEvent};
use super::*;

const PRIVATE_DIAGNOSTIC: &str = "synthetic-secret-/private/operator/auth-session";

#[test]
fn unadvertised_select_current_never_leaks_into_diagnostic() {
    // A provider may send a private currentValue not offered as a selectable
    // identity. Never reflect it just because it fits a byte-length budget.
    let secret = "private_token_not_in_advertised_choices_123456";
    let option: SessionConfigOption = serde_json::from_value(json!({
        "id": "model", "name": "Model", "type": "select",
        "currentValue": secret,
        "options": [{"value": "gpt-6", "name": "GPT 6"}],
    }))
    .unwrap();
    let (projected, truncated) = project_live_options(vec![option]);
    assert!(!truncated);
    assert_eq!(projected.len(), 1);
    assert_eq!(projected[0].id, "model");
    assert_eq!(projected[0].choices.len(), 1);
    assert_eq!(projected[0].choices[0].value, "gpt-6");
    assert_eq!(projected[0].current, None);
    assert!(!serde_json::to_string(&projected).unwrap().contains(secret));
}

#[test]
fn live_diagnostic_reader_caps_unterminated_frame_before_allocating_unbounded_memory() {
    // A hostile ACP provider can stream an arbitrarily long line without
    // sending the newline. The reader must stop after the declared frame bound.
    let mut input = std::io::Cursor::new(vec![b'x'; ACP_MESSAGE_MAX_BYTES * 4]);
    assert!(matches!(
        read_bounded_diagnostic_frame(&mut input),
        Some(ReaderEvent::TooLarge)
    ));
    assert!(input.position() <= (ACP_MESSAGE_MAX_BYTES + 1) as u64);
    let mut valid = std::io::Cursor::new(b"{\"id\":1,\"result\":{}}\n".to_vec());
    assert!(matches!(
        read_bounded_diagnostic_frame(&mut valid),
        Some(ReaderEvent::Message(_))
    ));
    assert!(read_bounded_diagnostic_frame(&mut valid).is_none());
}

#[test]
fn response_faults_keep_safe_distinct_causes() {
    let cases = [
        (ReaderEvent::Eof, AcpFailure::OutputClosed),
        (ReaderEvent::Malformed, AcpFailure::Malformed),
        (ReaderEvent::TooLarge, AcpFailure::TooLarge),
        (
            ReaderEvent::Io(std::io::ErrorKind::PermissionDenied),
            AcpFailure::ReadIo(std::io::ErrorKind::PermissionDenied),
        ),
        (
            ReaderEvent::Message(json!({"id": 1})),
            AcpFailure::MissingResult,
        ),
        (
            ReaderEvent::Message(
                json!({"id": 1, "error": {"code": -32001, "message": PRIVATE_DIAGNOSTIC, "data": PRIVATE_DIAGNOSTIC}}),
            ),
            AcpFailure::RpcRejected(Some(-32001)),
        ),
    ];
    let mut messages = std::collections::BTreeSet::new();
    for (event, expected) in cases {
        let (tx, rx) = mpsc::sync_channel(1);
        tx.send(event).unwrap();
        let error = wait_response(&rx, 1, Duration::from_secs(1), None).unwrap_err();
        assert_eq!(error, expected);
        assert!(!error.safe_message().contains(PRIVATE_DIAGNOSTIC));
        assert!(
            messages.insert(error.safe_message()),
            "faults must remain distinguishable"
        );
    }
    let (tx, rx) = mpsc::sync_channel(1);
    assert_eq!(
        wait_response(&rx, 1, Duration::ZERO, None),
        Err(AcpFailure::TimedOut)
    );
    let cancelled = AtomicBool::new(true);
    assert_eq!(
        wait_response(&rx, 1, Duration::from_secs(1), Some(&cancelled)),
        Err(AcpFailure::Interrupted)
    );
    drop(tx);
    assert_eq!(
        wait_response(&rx, 1, Duration::from_secs(1), None),
        Err(AcpFailure::Disconnected)
    );
}

#[test]
fn writer_discards_raw_io_diagnostics() {
    struct SecretErrorWriter;
    impl Write for SecretErrorWriter {
        fn write(&mut self, _: &[u8]) -> std::io::Result<usize> {
            Err(std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                PRIVATE_DIAGNOSTIC,
            ))
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let threads = BackgroundThreads::default();
    let writer = AcpOutboundWriter::spawn(SecretErrorWriter, &threads).unwrap();
    let pending = writer.start_frame(b"test\n".to_vec()).unwrap();
    let outcome = wait_outbound_write(pending, Instant::now() + Duration::from_secs(1), None, None);
    let OutboundWriteOutcome::Failed(error) = outcome else {
        panic!("writer failure was lost")
    };
    assert_eq!(
        error,
        AcpFailure::WriteIo(std::io::ErrorKind::PermissionDenied)
    );
    assert_eq!(error.safe_message(), "ACP stdin write failed");
    assert!(writer.wait_finished_until(Instant::now() + Duration::from_secs(1)));
}

#[test]
#[cfg(unix)]
fn process_exit_requires_pre_cleanup_exit_evidence() {
    use std::os::unix::process::ExitStatusExt;
    let status = std::process::ExitStatus::from_raw(23 << 8);
    assert_eq!(
        AcpFailure::OutputClosed.diagnose("run", "initialize", None),
        AcpFailure::OutputClosed
    );
    assert_eq!(
        AcpFailure::OutputClosed.diagnose("run", "initialize", Some(status)),
        AcpFailure::ProcessExited
    );
    assert_eq!(
        AcpFailure::Malformed.diagnose("run", "initialize", Some(status)),
        AcpFailure::Malformed
    );
    assert!(!AcpFailure::ProcessExited.safe_message().contains("23"));
}

#[cfg(unix)]
fn diagnostic_agent(temp: &TempDir, stage: &str, fault: &str) -> (String, Vec<String>) {
    let path = temp.path().join("diagnostic-acp.py");
    let script = r#"#!/usr/bin/env python3
import json, os, sys
stage, fault = sys.argv[1:3]
secret = 'synthetic-secret-/private/operator/auth-session'
options = [{'id':'mode','name':'Mode','type':'select','currentValue':'read-only','options':[{'value':'read-only','name':'Read only'}]}]
def send(value):
 print(json.dumps(value), flush=True)
for line in sys.stdin:
 request = json.loads(line)
 method, rid = request.get('method'), request.get('id')
 if method == 'session/prompt':
  open(os.path.join(os.path.dirname(__file__), 'prompt-dispatched'), 'w').close()
 if method == stage:
  if fault == 'block':
   open(os.path.join(os.path.dirname(__file__), 'setup-blocked'), 'w').close()
  elif fault == 'rpc':
   # More than pipe capacity: a missing stderr drain would deadlock this response.
   sys.stderr.write((secret + '\n') * 4096); sys.stderr.flush()
   send({'jsonrpc':'2.0','id':rid,'error':{'code':-32001,'message':secret,'data':{'sessionId':secret,'path':secret}}})
  elif fault == 'malformed':
   print(secret, flush=True)
  elif fault == 'oversized':
   print('x' * (1024 * 1024 + 1), flush=True)
  elif fault == 'read_io':
   os.write(1, b'\xff\n')
  elif fault == 'eof':
   os.close(1)
  sys.stdin.read()
  break
 elif method == 'initialize': send({'jsonrpc':'2.0','id':rid,'result':{'protocolVersion':1}})
 elif method == 'session/new': send({'jsonrpc':'2.0','id':rid,'result':{'sessionId':'private-session','configOptions':options}})
 elif method == 'session/set_config_option': send({'jsonrpc':'2.0','id':rid,'result':{'configOptions':options}})
 elif method == 'session/prompt': send({'jsonrpc':'2.0','id':rid,'result':{'stopReason':'end_turn'}})
"#;
    fs::write(&path, script).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
    (
        path.to_string_lossy().into_owned(),
        vec![stage.into(), fault.into()],
    )
}

#[test]
#[cfg(unix)]
fn setup_stages_preserve_fault_class_and_never_dispatch_prompt() {
    for (stage, code) in [
        ("initialize", "coding_agent_initialize_failed"),
        ("session/new", "coding_agent_session_new_failed"),
        ("session/set_config_option", "coding_agent_config_failed"),
    ] {
        for (fault, expected) in [
            ("malformed", "ACP returned malformed JSON"),
            ("oversized", "ACP message exceeded the size limit"),
            ("read_io", "ACP stdout read failed"),
            ("eof", "ACP stdout closed before responding; check the executable and explicit Runner environment mappings"),
        ] {
            let temp = crate::tests::executable_tempdir();
            let (exe,args) = diagnostic_agent(&temp, stage, fault);
            let manager = CodingAgentManager::with_store(&fake_config(exe,args), temp.path().join("store")).unwrap();
            let projects = project_fixture(&temp);
            let run = "wc_agent_run_diagnosticstage01";
            let config = BTreeMap::from([("mode".into(), CodingAgentConfigValue::String("read-only".into()))]);
            assert!(manager.handle(start_request(&manager, &temp.path().join("repo"), run, config), &projects).error.is_none());
            let snapshot = wait_for_snapshot(&manager, run, |snapshot| snapshot.state.terminal());
            assert_eq!(snapshot.state, CodingAgentRunState::Failed, "{stage}/{fault}");
            assert_eq!(snapshot.execution_state, CodingAgentExecutionState::NotStarted);
            let terminal = snapshot.terminal.unwrap();
            assert_eq!(terminal.error_code.as_deref(), Some(code));
            assert_eq!(terminal.message.as_deref(), Some(expected));
            assert!(!temp.path().join("prompt-dispatched").exists());
            assert_eq!(manager.drain_workers_until(Instant::now()+Duration::from_secs(3)).timed_out, 0);
        }
    }
}

#[test]
#[cfg(unix)]
fn rpc_and_stderr_secrets_never_reach_observations_inventory_or_durable_state() {
    for (stage, code) in [
        ("initialize", "coding_agent_initialize_failed"),
        ("session/new", "coding_agent_session_new_failed"),
        ("session/set_config_option", "coding_agent_config_failed"),
        ("session/prompt", "prompt_error"),
    ] {
        let temp = crate::tests::executable_tempdir();
        let (exe, args) = diagnostic_agent(&temp, stage, "rpc");
        let manager =
            CodingAgentManager::with_store(&fake_config(exe, args), temp.path().join("store"))
                .unwrap();
        let projects = project_fixture(&temp);
        let run = "wc_agent_run_diagnosticprivacy01";
        let config = BTreeMap::from([(
            "mode".into(),
            CodingAgentConfigValue::String("read-only".into()),
        )]);
        assert!(manager
            .handle(
                start_request(&manager, &temp.path().join("repo"), run, config),
                &projects
            )
            .error
            .is_none());
        let snapshot = wait_for_snapshot(&manager, run, |snapshot| snapshot.state.terminal());
        assert_eq!(
            snapshot.terminal.as_ref().unwrap().error_code.as_deref(),
            Some(code)
        );
        assert_eq!(
            snapshot.terminal.as_ref().unwrap().message.as_deref(),
            Some("ACP provider rejected the request")
        );
        let entry = manager.runs.lock().unwrap().get(run).unwrap().clone();
        let observation = entry.observe(None, 64, 0).unwrap();
        let response =
            CodingAgentResponse::success(CodingAgentResponsePayload::Observe { observation });
        let inventory = manager.inventory();
        let durable = fs::read_to_string(manager.store.state_path(run)).unwrap();
        for output in [
            serde_json::to_string(&response).unwrap(),
            serde_json::to_string(&inventory).unwrap(),
            durable,
        ] {
            assert!(
                !output.contains(PRIVATE_DIAGNOSTIC),
                "{stage} leaked private diagnostic"
            );
            assert!(
                !output.contains("-32001"),
                "numeric RPC detail belongs only in Runner diagnostics"
            );
        }
        assert_eq!(
            manager
                .drain_workers_until(Instant::now() + Duration::from_secs(3))
                .timed_out,
            0
        );
        assert_eq!(
            temp.path().join("prompt-dispatched").exists(),
            stage == "session/prompt"
        );
    }
}

#[test]
#[cfg(unix)]
fn explicit_path_mapping_restores_an_interpreter_outside_default_search_paths() {
    let outer = crate::tests::executable_tempdir();
    let runtime = outer.path().join("runtime");
    fs::create_dir(&runtime).unwrap();
    crate::tests::IsolatedEnv::new()
        .set("ACP_TEST_RUNTIME_PATH", runtime.to_str().unwrap())
        .run("inherited", || {
            let temp = crate::tests::executable_tempdir();
            let runtime = PathBuf::from(std::env::var_os("ACP_TEST_RUNTIME_PATH").unwrap());
            let (exe, args) = fake_agent(&temp, "end");
            // A unique interpreter name prevents system Node from masking this
            // /usr/bin/env lookup regression on CI hosts.
            let interpreter = format!("wc-acp-node-{}", Uuid::new_v4().simple());
            let python = std::env::split_paths(&std::env::var_os("PATH").unwrap())
                .map(|directory| directory.join("python3"))
                .find(|path| path.is_file())
                .expect("existing ACP fixtures require Python");
            std::os::unix::fs::symlink(python, runtime.join(&interpreter)).unwrap();
            let script = fs::read_to_string(&exe).unwrap().replacen(
                "#!/usr/bin/env python3",
                &format!("#!/usr/bin/env {interpreter}"),
                1,
            );
            fs::write(&exe, script).unwrap();
            let projects = project_fixture(&temp);
            let root = temp.path().join("repo");
            let mut cfg = fake_config(exe, args);
            let manager =
                CodingAgentManager::with_store(&cfg, temp.path().join("unmapped-store")).unwrap();
            let run = "wc_agent_run_unmappedruntime01";
            assert!(manager
                .handle(
                    start_request(&manager, &root, run, BTreeMap::new()),
                    &projects
                )
                .error
                .is_none());
            let failed = wait_for_snapshot(&manager, run, |snapshot| snapshot.state.terminal());
            assert_eq!(failed.state, CodingAgentRunState::Failed);
            assert_eq!(
                failed.execution_state,
                CodingAgentExecutionState::NotStarted
            );
            assert!(wire_log(&temp).is_empty());
            assert_eq!(
                manager
                    .drain_workers_until(Instant::now() + Duration::from_secs(3))
                    .timed_out,
                0
            );
            cfg.agents[0].env_from_env =
                BTreeMap::from([("PATH".into(), "ACP_TEST_RUNTIME_PATH".into())]);
            let manager =
                CodingAgentManager::with_store(&cfg, temp.path().join("mapped-store")).unwrap();
            let run = "wc_agent_run_mappedruntime01";
            assert!(manager
                .handle(
                    start_request(&manager, &root, run, BTreeMap::new()),
                    &projects
                )
                .error
                .is_none());
            let completed = wait_for_snapshot(&manager, run, |snapshot| snapshot.state.terminal());
            assert_eq!(completed.state, CodingAgentRunState::Completed);
            assert_eq!(prompt_count(&temp), 1);
            assert_eq!(
                manager
                    .drain_workers_until(Instant::now() + Duration::from_secs(3))
                    .timed_out,
                0
            );
        });
}

#[test]
#[cfg(unix)]
fn each_setup_stage_cancels_and_reports_its_deadline_without_prompt_dispatch() {
    for stage in ["initialize", "session/new", "session/set_config_option"] {
        for cancel in [true, false] {
            let temp = crate::tests::executable_tempdir();
            let (exe, args) = diagnostic_agent(&temp, stage, "block");
            let manager =
                CodingAgentManager::with_store(&fake_config(exe, args), temp.path().join("store"))
                    .unwrap();
            let projects = project_fixture(&temp);
            let run = "wc_agent_run_setupinterrupt01";
            let config = BTreeMap::from([(
                "mode".into(),
                CodingAgentConfigValue::String("read-only".into()),
            )]);
            let budget = if cancel { 10 } else { 1 };
            assert!(manager
                .handle(
                    start_request_with_timeout(
                        &manager,
                        &temp.path().join("repo"),
                        run,
                        config,
                        budget
                    ),
                    &projects
                )
                .error
                .is_none());
            let deadline = Instant::now() + Duration::from_secs(3);
            while !temp.path().join("setup-blocked").exists() {
                assert!(
                    Instant::now() < deadline,
                    "provider did not reach setup stage"
                );
                thread::sleep(Duration::from_millis(10));
            }
            if cancel {
                assert!(manager
                    .handle(
                        CodingAgentRequest::Cancel(CodingAgentCancelRequest { run_id: run.into() }),
                        &projects
                    )
                    .error
                    .is_none());
            }
            let terminal = wait_for_snapshot(&manager, run, |snapshot| snapshot.state.terminal());
            assert_eq!(
                terminal.execution_state,
                CodingAgentExecutionState::NotStarted
            );
            if cancel {
                assert_eq!(terminal.state, CodingAgentRunState::Cancelled);
                assert_eq!(terminal.terminal.unwrap().error_code, None);
            } else {
                assert_eq!(terminal.state, CodingAgentRunState::Failed);
                let error = terminal.terminal.unwrap();
                assert_eq!(
                    error.error_code.as_deref(),
                    Some("coding_agent_setup_timeout")
                );
                assert!(error
                    .message
                    .unwrap()
                    .contains(&format!("during ACP {stage} before prompt dispatch")));
            }
            assert!(!temp.path().join("prompt-dispatched").exists());
            assert_eq!(
                manager
                    .drain_workers_until(Instant::now() + Duration::from_secs(3))
                    .timed_out,
                0
            );
        }
    }
}

#[test]
#[cfg(unix)]
fn natural_process_exit_is_observed_before_cleanup() {
    let mut command = Command::new("/bin/sh");
    command
        .args(["-c", "exit 23"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    let mut child = ManagedChild::spawn(&mut command).unwrap();
    let deadline = Instant::now() + Duration::from_secs(3);
    let status = loop {
        if let Some(status) = child.try_wait().unwrap() {
            break status;
        }
        assert!(Instant::now() < deadline);
        thread::sleep(Duration::from_millis(10));
    };
    assert_eq!(status.code(), Some(23));
    let failure = AcpFailure::OutputClosed.diagnose("run", "initialize", Some(status));
    assert_eq!(failure, AcpFailure::ProcessExited);
    assert!(!failure.safe_message().contains("23"));
}

#[test]
fn declared_acp_environment_presence_only_returns_configured_boolean_destinations() {
    let provider = AcpAgentConfig {
        id: "codex".into(),
        name: "Codex".into(),
        executable: "never-spawn-this-binary".into(),
        args: vec!["secret-command-argument".into()],
        env_from_env: BTreeMap::from([
            ("API_KEY".into(), "PRIVATE_SECRET_ENV".into()),
            ("CODEX_PATH".into(), "ACP_CONFIG_SOURCE".into()),
        ]),
        allowed_config_options: Vec::new(),
    };
    let mut probed = Vec::new();
    let result = declared_env_source_presence(&provider, |source| {
        probed.push(source.to_owned());
        source == "ACP_CONFIG_SOURCE"
    });
    assert_eq!(
        result,
        BTreeMap::from([("API_KEY".into(), false), ("CODEX_PATH".into(), true),])
    );
    assert_eq!(probed, vec!["PRIVATE_SECRET_ENV", "ACP_CONFIG_SOURCE"]);
    let serialized = serde_json::to_string(&result).unwrap();
    for forbidden in [
        "PRIVATE_SECRET_ENV",
        "ACP_CONFIG_SOURCE",
        "never-spawn-this-binary",
        "secret-command-argument",
    ] {
        assert!(!serialized.contains(forbidden));
    }
}

#[test]
fn acp_environment_presence_does_not_probe_unlisted_sources() {
    let provider = AcpAgentConfig {
        id: "codex".into(),
        name: "Codex".into(),
        executable: "unused".into(),
        args: Vec::new(),
        env_from_env: BTreeMap::new(),
        allowed_config_options: Vec::new(),
    };
    let result = declared_env_source_presence(&provider, |_| {
        panic!("unlisted environment sources must not be inspected")
    });
    assert!(result.is_empty());
}

#[test]
fn provider_env_presence_uses_explicit_provider_id_and_omits_values() {
    let temp = TempDir::new().unwrap();
    let mut config = fake_config("do-not-launch".into(), Vec::new());
    config.agents[0].env_from_env = BTreeMap::from([
        ("PATH_DESTINATION".into(), "PATH".into()),
        (
            "MISSING_DESTINATION".into(),
            "WEBCODEX_TEST_NEVER_SET_ENV_994".into(),
        ),
    ]);
    let manager = CodingAgentManager::with_store(&config, temp.path().join("store")).unwrap();
    assert!(manager.declared_provider_env_presence("unknown").is_none());
    let readiness = manager.declared_provider_env_presence("codex").unwrap();
    assert_eq!(readiness.get("PATH_DESTINATION"), Some(&true));
    assert_eq!(readiness.get("MISSING_DESTINATION"), Some(&false));
    assert_eq!(readiness.len(), 2);
    let output = serde_json::to_string(&readiness).unwrap();
    assert!(!output.contains("WEBCODEX_TEST_NEVER_SET_ENV_994"));
    assert!(!output.contains("PATH="));
    assert!(!output.contains("do-not-launch"));
}

#[test]
#[cfg(unix)]
fn acp_diagnose_runner_request_is_identity_fenced_read_only_and_no_spawn() {
    use webcodex_core::coding_agent::{validate_response_for_request, CodingAgentDiagnoseRequest};
    let temp = crate::tests::executable_tempdir();
    let projects = project_fixture(&temp);
    let project_root = temp.path().join("repo");
    let mut config = fake_config("never-launch-this-acp-binary".into(), vec![]);
    config.agents[0].env_from_env = BTreeMap::from([
        ("KNOWN_DEST".into(), "PATH".into()),
        (
            "MISSING_DEST".into(),
            "WEBCODEX_TEST_994_MISSING_SOURCE".into(),
        ),
    ]);
    let manager = CodingAgentManager::with_store(&config, temp.path().join("store")).unwrap();
    let instance = manager.providers()[0].provider_instance_id.clone();
    let make_request = || {
        CodingAgentRequest::Diagnose(CodingAgentDiagnoseRequest {
            run_id: "wc_agent_run_diagnosis_readonly_994".into(),
            runtime_project_id: "agent:test:demo".into(),
            project_root: project_root.to_string_lossy().to_string(),
            provider_id: "codex".into(),
            provider_instance_id: instance.clone(),
            include_live_acp: false,
        })
    };
    let valid_request = make_request();
    let response = manager.handle(valid_request.clone(), &projects);
    validate_response_for_request(&valid_request, &response).unwrap();
    let Some(CodingAgentResponsePayload::Diagnose {
        provider_id,
        provider_instance_id,
        env_presence,
        live_acp,
    }) = response.payload
    else {
        panic!(
            "diagnostic should return typed presence only: {:?}",
            response.error
        );
    };
    assert_eq!(live_acp, None);
    assert_eq!(provider_id, "codex");
    assert_eq!(provider_instance_id, instance);
    assert!(env_presence["KNOWN_DEST"]);
    assert!(!env_presence["MISSING_DEST"]);
    let serialized = serde_json::to_string(&env_presence).unwrap();
    assert!(!serialized.contains("WEBCODEX_TEST_994_MISSING_SOURCE"));
    assert!(!serialized.contains("never-launch-this-acp-binary"));
    assert!(manager.runs.lock().unwrap().is_empty());

    let mut stale = make_request();
    let CodingAgentRequest::Diagnose(stale_request) = &mut stale else {
        unreachable!()
    };
    stale_request.provider_instance_id = "acp_stale_generation".into();
    let reply = manager.handle(stale.clone(), &projects);
    assert_eq!(
        reply.error.as_ref().unwrap().code,
        "stale_coding_agent_provider"
    );
    validate_response_for_request(&stale, &reply).unwrap();

    let mut wrong_project = make_request();
    let CodingAgentRequest::Diagnose(wrong) = &mut wrong_project else {
        unreachable!()
    };
    wrong.runtime_project_id = "agent:test:other_project".into();
    let reply = manager.handle(wrong_project.clone(), &projects);
    assert_eq!(
        reply.error.as_ref().unwrap().code,
        "stale_coding_agent_project"
    );
    validate_response_for_request(&wrong_project, &reply).unwrap();

    // A read-only Project may be inspected; the same binding must not grant
    // the separate Start operation writable authority.
    std::fs::write(
        projects.join("p.toml"),
        format!(
            "id = \"demo\"\npath = {:?}\nallow_patch = false\n",
            project_root.to_string_lossy()
        ),
    )
    .unwrap();
    let readonly_request = make_request();
    let readonly_reply = manager.handle(readonly_request.clone(), &projects);
    validate_response_for_request(&readonly_request, &readonly_reply).unwrap();
    assert!(matches!(
        readonly_reply.payload,
        Some(CodingAgentResponsePayload::Diagnose { .. })
    ));
    assert!(!project_binding_matches(
        &projects,
        "test",
        "agent:test:demo",
        project_root.to_string_lossy().as_ref()
    ));
    assert!(manager.runs.lock().unwrap().is_empty());
}

#[cfg(unix)]
fn live_diagnose_request(
    manager: &CodingAgentManager,
    root: &Path,
    run: &str,
    include_live_acp: bool,
) -> CodingAgentRequest {
    use webcodex_core::coding_agent::CodingAgentDiagnoseRequest;
    let provider = manager.providers().remove(0);
    CodingAgentRequest::Diagnose(CodingAgentDiagnoseRequest {
        run_id: run.to_string(),
        runtime_project_id: "agent:test:demo".to_string(),
        project_root: root.to_string_lossy().to_string(),
        provider_id: provider.provider_id,
        provider_instance_id: provider.provider_instance_id,
        include_live_acp,
    })
}

#[test]
#[cfg(unix)]
fn concurrent_live_probe_is_rejected_without_unbounded_mutex_wait() {
    use webcodex_core::coding_agent::{CodingAgentDiagnoseOutcome, CodingAgentDiagnosePhase};
    let temp = crate::tests::executable_tempdir();
    let manager = CodingAgentManager::with_store(
        &fake_config("not-a-real-executable".into(), Vec::new()),
        temp.path().join("store"),
    )
    .unwrap();
    let projects = project_fixture(&temp);
    let request = live_diagnose_request(
        &manager,
        &temp.path().join("repo"),
        "wc_agent_run_diagnosebusy01",
        true,
    );
    // Pretend a different live probe already owns the single process permit.
    let permit = manager.live_probe_permit.lock().unwrap();
    let (tx, rx) = std::sync::mpsc::sync_channel(1);
    let worker = {
        let manager = Arc::clone(&manager);
        std::thread::spawn(move || {
            let _ = tx.send(manager.handle(request, &projects));
        })
    };
    let promptly = rx.recv_timeout(Duration::from_millis(500));
    drop(permit); // Release even if the candidate blocks, so the test can end.
    worker.join().unwrap();
    let response = promptly.expect("a live probe must not queue without a work deadline");
    let Some(CodingAgentResponsePayload::Diagnose {
        live_acp: Some(live),
        ..
    }) = response.payload
    else {
        panic!("busy response needs a bounded diagnostic payload");
    };
    assert_eq!(live.outcome, CodingAgentDiagnoseOutcome::Busy);
    assert_eq!(live.phase, CodingAgentDiagnosePhase::Spawn);
    assert!(live.options.is_empty());
}

#[test]
#[cfg(unix)]
fn acp_diagnose_live_probe_reports_live_options_with_only_initialize_and_session_new() {
    use webcodex_core::coding_agent::{
        validate_response_for_request, CodingAgentDiagnoseOutcome, CodingAgentDiagnosePhase,
    };
    let temp = crate::tests::executable_tempdir();
    let (exe, args) = fake_agent(&temp, "end");
    let mut config = fake_config(exe.clone(), args);
    // The declared source exists in the Runner process, but its name and
    // value must never appear in the live projection either.
    config.agents[0].env_from_env =
        BTreeMap::from([("CODEX_PATH".to_string(), "PATH".to_string())]);
    let manager = CodingAgentManager::with_store(&config, temp.path().join("store")).unwrap();
    let projects = project_fixture(&temp);
    let root = temp.path().join("repo");
    let request = live_diagnose_request(&manager, &root, "wc_agent_run_diagnoselive01", true);
    let response = manager.handle(request.clone(), &projects);
    validate_response_for_request(&request, &response).unwrap();
    let serialized = serde_json::to_string(&response).unwrap();
    assert!(!serialized.contains("never-spawn-this-binary"));
    let Some(CodingAgentResponsePayload::Diagnose {
        env_presence,
        live_acp,
        ..
    }) = response.payload
    else {
        panic!(
            "live diagnose should return a typed payload: {:?}",
            response.error
        );
    };
    assert!(env_presence["CODEX_PATH"]);
    let live = live_acp.expect("explicitly authorized live probe result");
    assert_eq!(live.outcome, CodingAgentDiagnoseOutcome::Ok);
    assert_eq!(live.phase, CodingAgentDiagnosePhase::Projection);
    assert_eq!(live.protocol_version, Some(1));
    assert!(!live.options_truncated);
    assert_eq!(live.options.len(), 1);
    assert_eq!(live.options[0].id, "mode");
    assert_eq!(live.options[0].kind, "select");
    assert_eq!(live.options[0].current.as_deref(), Some("agent"));
    assert_eq!(live.options[0].choices.len(), 2);
    assert_eq!(live.options[0].choices[0].value, "agent");
    // Only the two setup methods were ever sent: no prompt, no config write,
    // and no provider tool call.
    assert_eq!(
        received_methods(&wire_log(&temp)),
        vec!["initialize".to_string(), "session/new".to_string()]
    );
    // No CodingAgentRun was created or persisted by the probe.
    assert!(manager.inventory().runs.is_empty());
    // The ephemeral provider process did not survive the probe.
    #[cfg(target_os = "linux")]
    {
        let startup = wire_log(&temp)[0]
            .get("startup_pid")
            .and_then(Value::as_u64)
            .expect("fake provider logged its startup pid");
        wait_for_proc_exit(startup);
    }
    assert_eq!(
        manager
            .drain_workers_until(Instant::now() + Duration::from_secs(3))
            .timed_out,
        0
    );
}

#[test]
#[cfg(unix)]
fn acp_diagnose_live_probe_missing_env_source_fails_closed_without_spawn() {
    use webcodex_core::coding_agent::{
        validate_response_for_request, CodingAgentDiagnoseOutcome, CodingAgentDiagnosePhase,
    };
    let temp = crate::tests::executable_tempdir();
    let mut config = fake_config("never-launch-this-acp-binary".into(), Vec::new());
    config.agents[0].env_from_env = BTreeMap::from([(
        "CODEX_PATH".to_string(),
        "WEBCODEX_TEST_994_MISSING_SOURCE".to_string(),
    )]);
    let manager = CodingAgentManager::with_store(&config, temp.path().join("store")).unwrap();
    let projects = project_fixture(&temp);
    let request = live_diagnose_request(
        &manager,
        &temp.path().join("repo"),
        "wc_agent_run_diagnoseenvmiss01",
        true,
    );
    let response = manager.handle(request.clone(), &projects);
    validate_response_for_request(&request, &response).unwrap();
    let Some(CodingAgentResponsePayload::Diagnose { live_acp, .. }) = response.payload else {
        panic!(
            "diagnose should return a typed payload: {:?}",
            response.error
        );
    };
    let live = live_acp.expect("explicitly authorized live probe result");
    assert_eq!(live.outcome, CodingAgentDiagnoseOutcome::EnvUnavailable);
    assert_eq!(live.phase, CodingAgentDiagnosePhase::Environment);
    assert!(live.options.is_empty());
    let serialized = serde_json::to_string(&live).unwrap();
    assert!(!serialized.contains("WEBCODEX_TEST_994_MISSING_SOURCE"));
    assert!(!serialized.contains("never-launch-this-acp-binary"));
    assert!(wire_log(&temp).is_empty());
    assert!(manager.runs.lock().unwrap().is_empty());
}

#[test]
#[cfg(unix)]
fn acp_diagnose_live_probe_faults_stay_classified_bounded_and_prompt_free() {
    use webcodex_core::coding_agent::{
        validate_response_for_request, CodingAgentDiagnoseOutcome, CodingAgentDiagnosePhase,
    };
    let transport = CodingAgentDiagnoseOutcome::TransportFailed;
    let cases = [
        (
            "initialize",
            "rpc",
            CodingAgentDiagnoseOutcome::Rejected,
            CodingAgentDiagnosePhase::Initialize,
        ),
        (
            "session/new",
            "rpc",
            CodingAgentDiagnoseOutcome::Rejected,
            CodingAgentDiagnosePhase::SessionNew,
        ),
        (
            "initialize",
            "malformed",
            transport,
            CodingAgentDiagnosePhase::Initialize,
        ),
        (
            "initialize",
            "oversized",
            transport,
            CodingAgentDiagnosePhase::Initialize,
        ),
        (
            "initialize",
            "eof",
            transport,
            CodingAgentDiagnosePhase::Initialize,
        ),
        (
            "session/new",
            "malformed",
            transport,
            CodingAgentDiagnosePhase::SessionNew,
        ),
        (
            "session/new",
            "oversized",
            transport,
            CodingAgentDiagnosePhase::SessionNew,
        ),
        (
            "session/new",
            "eof",
            transport,
            CodingAgentDiagnosePhase::SessionNew,
        ),
    ];
    for (stage, fault, expected_outcome, expected_phase) in cases {
        let temp = crate::tests::executable_tempdir();
        let (exe, args) = diagnostic_agent(&temp, stage, fault);
        let manager =
            CodingAgentManager::with_store(&fake_config(exe, args), temp.path().join("store"))
                .unwrap();
        let projects = project_fixture(&temp);
        let request = live_diagnose_request(
            &manager,
            &temp.path().join("repo"),
            "wc_agent_run_diagnosefault01",
            true,
        );
        let response = manager.handle(request.clone(), &projects);
        validate_response_for_request(&request, &response).unwrap();
        let Some(CodingAgentResponsePayload::Diagnose { live_acp, .. }) = response.payload else {
            panic!(
                "diagnose should return a typed payload: {:?}",
                response.error
            );
        };
        let live = live_acp.expect("explicitly authorized live probe result");
        assert_eq!(live.outcome, expected_outcome, "{stage}/{fault}");
        assert_eq!(live.phase, expected_phase, "{stage}/{fault}");
        assert!(live.options.is_empty(), "{stage}/{fault}");
        let serialized = serde_json::to_string(&live).unwrap();
        assert!(
            !serialized.contains(PRIVATE_DIAGNOSTIC),
            "{stage}/{fault} leaked provider-controlled secret text"
        );
        assert!(
            !serialized.contains("-32001"),
            "{stage}/{fault} leaked numeric RPC detail"
        );
        // The diagnostic never dispatches a prompt and never creates a Run.
        assert!(!temp.path().join("prompt-dispatched").exists());
        assert!(manager.runs.lock().unwrap().is_empty());
        assert_eq!(
            manager
                .drain_workers_until(Instant::now() + Duration::from_secs(3))
                .timed_out,
            0
        );
    }
}

#[test]
#[cfg(unix)]
fn acp_diagnose_live_probe_timeout_terminates_the_whole_provider_tree() {
    use webcodex_core::coding_agent::{
        validate_response_for_request, CodingAgentDiagnoseOutcome, CodingAgentDiagnosePhase,
    };
    let temp = crate::tests::executable_tempdir();
    let (exe, args) = fake_agent(&temp, "block_initialize_tree");
    let manager =
        CodingAgentManager::with_store(&fake_config(exe, args), temp.path().join("store")).unwrap();
    *manager.probe_test_budget.lock().unwrap() = Some(Duration::from_millis(400));
    let projects = project_fixture(&temp);
    let started = Instant::now();
    let request = live_diagnose_request(
        &manager,
        &temp.path().join("repo"),
        "wc_agent_run_diagnosetimeout01",
        true,
    );
    let response = manager.handle(request.clone(), &projects);
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "the probe must honor its bounded budget, took {:?}",
        started.elapsed()
    );
    validate_response_for_request(&request, &response).unwrap();
    let Some(CodingAgentResponsePayload::Diagnose { live_acp, .. }) = response.payload else {
        panic!(
            "diagnose should return a typed payload: {:?}",
            response.error
        );
    };
    let live = live_acp.expect("explicitly authorized live probe result");
    assert_eq!(live.outcome, CodingAgentDiagnoseOutcome::Timeout);
    assert_eq!(live.phase, CodingAgentDiagnosePhase::Initialize);
    assert!(live.options.is_empty());
    assert!(manager.inventory().runs.is_empty());
    // The whole ephemeral process tree, including the spawned descendant, is
    // already gone by the time the classified result is observable.
    #[cfg(target_os = "linux")]
    {
        let log = wire_log(&temp);
        let startup = log[0]
            .get("startup_pid")
            .and_then(Value::as_u64)
            .expect("fake provider logged its startup pid");
        wait_for_proc_exit(startup);
        let descendant = log
            .iter()
            .find_map(|entry| entry.get("descendant_pid"))
            .expect("blocked provider logged its descendant pid")
            .as_u64()
            .expect("descendant pid");
        wait_for_proc_exit(descendant);
    }
    assert_eq!(
        manager
            .drain_workers_until(Instant::now() + Duration::from_secs(3))
            .timed_out,
        0
    );
}

#[test]
#[cfg(unix)]
fn acp_diagnose_live_probe_provider_exit_is_classified_without_status_leak() {
    use webcodex_core::coding_agent::{
        validate_response_for_request, CodingAgentDiagnoseOutcome, CodingAgentDiagnosePhase,
    };
    let temp = crate::tests::executable_tempdir();
    let (exe, args) = fake_agent(&temp, "crash_before_prompt");
    let manager =
        CodingAgentManager::with_store(&fake_config(exe, args), temp.path().join("store")).unwrap();
    let projects = project_fixture(&temp);
    let request = live_diagnose_request(
        &manager,
        &temp.path().join("repo"),
        "wc_agent_run_diagnosecrash01",
        true,
    );
    let response = manager.handle(request.clone(), &projects);
    validate_response_for_request(&request, &response).unwrap();
    let Some(CodingAgentResponsePayload::Diagnose { live_acp, .. }) = response.payload else {
        panic!(
            "diagnose should return a typed payload: {:?}",
            response.error
        );
    };
    let live = live_acp.expect("explicitly authorized live probe result");
    // Closing stdout can race process reaping: without a pre-cleanup exit
    // observation the honest closed outcome is transport_failed. Both are
    // safe, and the separate natural-exit test proves the classified upgrade.
    assert!(matches!(
        live.outcome,
        CodingAgentDiagnoseOutcome::ProcessExited | CodingAgentDiagnoseOutcome::TransportFailed
    ));
    assert_eq!(live.phase, CodingAgentDiagnosePhase::Initialize);
    assert!(live.options.is_empty());
    let serialized = serde_json::to_string(&live).unwrap();
    assert!(
        !serialized.contains('9'),
        "exit status must not be projected"
    );
    assert!(manager.inventory().runs.is_empty());
    assert_eq!(
        manager
            .drain_workers_until(Instant::now() + Duration::from_secs(3))
            .timed_out,
        0
    );
}

#[test]
#[cfg(unix)]
fn acp_diagnose_live_probe_projects_advertised_options_within_closed_bounds() {
    use webcodex_core::coding_agent::{
        validate_response_for_request, CodingAgentDiagnoseOutcome, CodingAgentDiagnosePhase,
        CODING_AGENT_MAX_CONFIG_LABEL_BYTES, CODING_AGENT_MAX_CONFIG_OPTIONS,
        CODING_AGENT_MAX_CONFIG_SELECT_CHOICES,
    };
    let temp = crate::tests::executable_tempdir();
    let (exe, args) = fake_agent(&temp, "diag_oversized");
    let manager =
        CodingAgentManager::with_store(&fake_config(exe, args), temp.path().join("store")).unwrap();
    let projects = project_fixture(&temp);
    let request = live_diagnose_request(
        &manager,
        &temp.path().join("repo"),
        "wc_agent_run_diagnoselarge01",
        true,
    );
    let response = manager.handle(request.clone(), &projects);
    // Response validation itself enforces the closed budgets end to end.
    validate_response_for_request(&request, &response).unwrap();
    let Some(CodingAgentResponsePayload::Diagnose { live_acp, .. }) = response.payload else {
        panic!(
            "diagnose should return a typed payload: {:?}",
            response.error
        );
    };
    let live = live_acp.expect("explicitly authorized live probe result");
    assert_eq!(live.outcome, CodingAgentDiagnoseOutcome::Ok);
    assert_eq!(live.phase, CodingAgentDiagnosePhase::Projection);
    assert_eq!(live.options.len(), CODING_AGENT_MAX_CONFIG_OPTIONS);
    assert!(live.options_truncated);
    let option = &live.options[0];
    assert_eq!(option.id, "opt_0");
    assert_eq!(option.kind, "select");
    assert!(option
        .label
        .as_ref()
        .is_some_and(|label| label.len() <= CODING_AGENT_MAX_CONFIG_LABEL_BYTES));
    assert_eq!(option.choices.len(), CODING_AGENT_MAX_CONFIG_SELECT_CHOICES);
    assert!(option.choices_truncated);
    assert_eq!(option.choices[0].value, "v0_0");
    assert!(option.choices[0]
        .label
        .as_ref()
        .is_some_and(|label| label.len() <= CODING_AGENT_MAX_CONFIG_LABEL_BYTES));
    assert_eq!(
        received_methods(&wire_log(&temp)),
        vec!["initialize".to_string(), "session/new".to_string()]
    );
    assert!(manager.inventory().runs.is_empty());
    assert_eq!(
        manager
            .drain_workers_until(Instant::now() + Duration::from_secs(3))
            .timed_out,
        0
    );
}
