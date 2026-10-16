use super::*;

fn fixture() -> (tempfile::TempDir, ControllerConfig) {
    let tmp = tempfile::tempdir().unwrap();
    let runner = tmp.path().join("runner.toml");
    std::fs::write(&runner, "server_url = 'http://172.20.0.1:8080'\nclient_id = 'wsl-runner'\ntoken = 'private-test-token'\n").unwrap();
    let config = toml::from_str(&format!("[server]\nmode = 'wsl2_nat'\nport = 8080\n[runner]\nconfig = {:?}\n[controller]\nruntime_dir = {:?}\n", runner, tmp.path())).unwrap();
    (tmp, config)
}

#[cfg(target_os = "linux")]
#[test]
fn config_defaults_and_explicit_values_are_validated() {
    let (_tmp, mut cfg) = fixture();
    validate_config(&cfg).unwrap();
    assert_eq!(cfg.server.wsl2_interface(), "eth0");
    assert_eq!(cfg.server.wsl2_scheme(), "http");
    cfg.server.interface = Some("eth1".into());
    cfg.server.scheme = Some("https".into());
    validate_config(&cfg).unwrap();
    assert_eq!(cfg.server.wsl2_interface(), "eth1");
    assert_eq!(cfg.server.wsl2_scheme(), "https");
    for value in ["", "bad interface", "../../x", "1234567890123456"] {
        cfg.server.interface = Some(value.into());
        assert!(validate_config(&cfg).is_err());
    }
    cfg.server.interface = None;
    cfg.server.scheme = Some("".into());
    assert!(validate_config(&cfg).is_err());
    cfg.server.scheme = None;
    cfg.server.port = None;
    assert!(validate_config(&cfg).is_err());
    cfg.server.port = Some(0);
    assert!(validate_config(&cfg).is_err());
    cfg.server.port = Some(8080);
    cfg.server.url = Some("".into());
    assert!(validate_config(&cfg).is_err());
    cfg.server.url = None;
    cfg.tunnel.enabled = true;
    assert!(validate_config(&cfg).is_err());
    cfg.tunnel.enabled = false;
    cfg.runner.enabled = false;
    assert!(validate_config(&cfg).is_err());
}

#[test]
fn quic_endpoints_cannot_bypass_dynamic_target() {
    let (_tmp, cfg) = fixture();
    let original = std::fs::read_to_string(&cfg.runner.config).unwrap();
    std::fs::write(
        &cfg.runner.config,
        format!("{original}transport = 'auto'\n"),
    )
    .unwrap();
    Binding::read(&cfg).unwrap();
    for transport in ["quic", "invalid"] {
        std::fs::write(
            &cfg.runner.config,
            format!("{original}transport = '{transport}'\n"),
        )
        .unwrap();
        assert!(Binding::read(&cfg).is_err());
    }
    std::fs::write(
        &cfg.runner.config,
        format!("{original}[quic]\nserver_addr = 'other:443'\n"),
    )
    .unwrap();
    assert!(Binding::read(&cfg).is_err());
}

#[tokio::test]
async fn waiting_discovery_does_not_block_ipc_and_restart_cancels_old_attempt() {
    let (_tmp, cfg) = fixture();
    let mut runtime = ControllerRuntime::new(cfg);
    runtime.schedule_wsl2_start().unwrap();
    let task = tokio::spawn(std::future::pending::<Wsl2NatSnapshot>());
    let abort = task.abort_handle();
    runtime.wsl2_startup.task = Some(task);
    runtime.wsl2_startup.retry_at = None;
    runtime.poll_wsl2_start().await;
    let (server, mut client) = UnixStream::pair().unwrap();
    client
        .write_all(b"{\"method\":\"status\"}\n")
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_millis(200), handle_ipc(&mut runtime, server))
        .await
        .unwrap()
        .unwrap();
    let mut response = String::new();
    BufReader::new(client)
        .read_line(&mut response)
        .await
        .unwrap();
    let response: Value = serde_json::from_str(&response).unwrap();
    assert_eq!(response["result"]["runner"]["phase"], "waiting_for_server");
    assert_eq!(runtime.runner.restart_count, 0);
    assert!(runtime
        .restart_component(Some(Component::Server))
        .await
        .is_err());
    runtime
        .restart_component(Some(Component::Runner))
        .await
        .unwrap();
    tokio::task::yield_now().await;
    assert!(abort.is_finished());
    runtime.stop_all().await;
    assert!(runtime.wsl2_startup.retry_at.is_none());
}

#[tokio::test]
async fn failed_attempt_waits_without_consuming_process_restart_budget() {
    let (_tmp, cfg) = fixture();
    let mut runtime = ControllerRuntime::new(cfg);
    runtime.schedule_wsl2_start().unwrap();
    let task = tokio::spawn(async {
        Wsl2NatSnapshot {
            error: Some("gateway_missing".into()),
            ..Default::default()
        }
    });
    while !task.is_finished() {
        tokio::task::yield_now().await;
    }
    runtime.wsl2_startup.task = Some(task);
    runtime.wsl2_startup.retry_at = None;
    runtime.poll_wsl2_start().await;
    assert_eq!(runtime.runner.restart_count, 0);
    assert_eq!(runtime.runner.phase, "waiting_for_server");
    assert_eq!(
        runtime.wsl2_startup.snapshot.error.as_deref(),
        Some("gateway_missing")
    );
    assert!(runtime.wsl2_startup.retry_at.unwrap() > Instant::now());
    assert!(runtime.wsl2_startup.task.is_none());
}

#[test]
fn child_url_and_proxy_exclusions_are_process_local() {
    let mut command = Command::new("webcodex-runner");
    configure_runner_command(&mut command, "http://172.29.32.1:8080").unwrap();
    let args: Vec<_> = command
        .as_std()
        .get_args()
        .map(|s| s.to_str().unwrap())
        .collect();
    assert_eq!(args, ["--server-url", "http://172.29.32.1:8080"]);
    for (name, value) in command.as_std().get_envs() {
        if name == "NO_PROXY" || name == "no_proxy" {
            assert!(value.unwrap().to_string_lossy().ends_with("172.29.32.1"));
        }
    }
}

#[tokio::test]
async fn project_target_requires_live_matching_controller() {
    let (_tmp, cfg) = fixture();
    assert!(active_target(&cfg)
        .await
        .unwrap_err()
        .contains("controller_unavailable"));
}

async fn snapshot_server(
    cfg: &ControllerConfig,
    states: Vec<Value>,
) -> tokio::task::JoinHandle<()> {
    let listener = UnixListener::bind(controller_socket(cfg).unwrap()).unwrap();
    tokio::spawn(async move {
        tokio::time::timeout(Duration::from_secs(5), async move {
            for state in states {
                let (stream, _) = listener.accept().await.unwrap();
                let mut reader = BufReader::new(stream);
                let mut line = String::new();
                reader.read_line(&mut line).await.unwrap();
                reader
                    .get_mut()
                    .write_all(format!("{}\n", json!({"ok":true,"result":state})).as_bytes())
                    .await
                    .unwrap();
            }
        })
        .await
        .unwrap();
    })
}

fn active_snapshot(cfg: &ControllerConfig, generation: &str) -> Value {
    json!({"server_mode":"wsl2_nat", "runner":{"phase":"running_unverified"},
    "wsl2_nat_target": Wsl2NatSnapshot {
        effective_url: Some("http://172.29.32.1:8080".into()), generation: Some(generation.into()),
        binding: Some(Binding::read(cfg).unwrap()), probe_state: "reachable".into(),
        ..Default::default()
    }})
}

#[tokio::test]
async fn doctor_uses_active_snapshot_and_project_identity_mismatch_fails_closed() {
    let (_tmp, cfg) = fixture();
    let state = active_snapshot(&cfg, "generation");
    let task = snapshot_server(&cfg, vec![state.clone(), state]).await;
    let mut report = doctor_report(&cfg, &PathBuf::new()).unwrap();
    doctor(&cfg, &mut report).await.unwrap();
    assert_eq!(report["server"]["url"], "http://172.29.32.1:8080");
    assert_eq!(report["wsl2_nat_target"]["source"], "controller_active");
    let original = std::fs::read_to_string(&cfg.runner.config).unwrap();
    std::fs::write(
        &cfg.runner.config,
        original.replace("wsl-runner", "replacement-runner"),
    )
    .unwrap();
    assert!(active_target(&cfg)
        .await
        .unwrap_err()
        .starts_with("controller_target_changed"));
    task.await.unwrap();
}

#[tokio::test]
async fn invalid_live_doctor_response_never_falls_back_to_discovery() {
    let (_tmp, cfg) = fixture();
    let listener = UnixListener::bind(controller_socket(&cfg).unwrap()).unwrap();
    let task = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut reader = BufReader::new(stream);
        let mut request = String::new();
        reader.read_line(&mut request).await.unwrap();
        reader
            .get_mut()
            .write_all(b"invalid response\n")
            .await
            .unwrap();
    });
    let mut report = doctor_report(&cfg, &PathBuf::new()).unwrap();
    assert!(doctor(&cfg, &mut report)
        .await
        .unwrap_err()
        .contains("invalid live snapshot"));
    assert!(report.get("wsl2_nat_target").is_none());
    task.await.unwrap();
}

#[tokio::test]
async fn live_runner_prevents_discovery_and_auth_exit_does_not_restart() {
    let (_tmp, cfg) = fixture();
    let mut runtime = ControllerRuntime::new(cfg);
    // Dedicated fixture owns stdin; EOF stops cat, and both output pipes are drained.
    // Avoid ambient PATH, which other CLI tests temporarily override.
    let mut command = Command::new("/bin/cat");
    command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    runtime.runner.process =
        Some(spawn_process(command, "runner", runtime.log.clone(), None).unwrap());
    runtime.wsl2_startup.retry_at = Some(Instant::now());
    runtime.poll_wsl2_start().await;
    assert!(runtime.wsl2_startup.task.is_none());
    assert!(runtime
        .runner
        .process
        .as_mut()
        .unwrap()
        .child
        .try_wait()
        .unwrap()
        .is_none());
    runtime.stop_component(Component::Runner).await;
    assert!(runtime.runner.process.is_none());
    let mut command = Command::new("/bin/sh");
    command
        .args(["-c", "exit 77"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    let mut process = spawn_process(command, "runner", runtime.log.clone(), None).unwrap();
    process.child.wait().await.unwrap();
    runtime.runner.process = Some(process);
    runtime.reap_and_recover().await;
    assert_eq!(
        runtime.wsl2_startup.snapshot.error.as_deref(),
        Some("runner_auth_failed")
    );
    assert_eq!(runtime.runner.restart_count, 0);
    assert!(runtime.wsl2_startup.retry_at.is_none());
}

#[tokio::test]
async fn server_auth_status_proves_only_reachability_and_redirects_are_rejected() {
    for status in [200, 401, 403, 302, 500] {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let task = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut request = [0; 4096];
            stream.read(&mut request).await.unwrap();
            stream.write_all(format!("HTTP/1.1 {status} Test\r\nContent-Length: 0\r\nLocation: http://127.0.0.1:1\r\nConnection: close\r\n\r\n").as_bytes()).await.unwrap();
        });
        assert_eq!(probe(&url).await.is_ok(), matches!(status, 200 | 401 | 403));
        task.await.unwrap();
    }
}
