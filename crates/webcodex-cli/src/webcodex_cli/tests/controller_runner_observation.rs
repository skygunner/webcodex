use super::*;
use tokio::net::TcpListener;

fn fixture(server: &str, mode: ServerMode) -> (tempfile::TempDir, ControllerRuntime) {
    let tmp = tempfile::tempdir().unwrap();
    let runner = tmp.path().join("runner.toml");
    let token = tmp.path().join("webcodex-user-token");
    std::fs::write(
        &runner,
        format!("server_url = '{server}'\nclient_id = 'observation-fixture'\n"),
    )
    .unwrap();
    std::fs::write(&token, "first-fixture-token").unwrap();
    let mut cfg: ControllerConfig = toml::from_str(&format!(
        "[server]\nmode='remote'\nurl='{server}'\n[runner]\nconfig={runner:?}\nuser_token_file={token:?}\n[controller]\nrestart=false\nruntime_dir={:?}\n",tmp.path()
    )).unwrap();
    cfg.server.mode = mode;
    let mut runtime = ControllerRuntime::new(cfg);
    runtime.runner.phase = "starting";
    if mode == ServerMode::Wsl2Nat {
        runtime.wsl2_startup.snapshot.generation = Some("fixture-target-generation".into());
        runtime.wsl2_startup.snapshot.effective_url = Some(server.into());
    }
    runtime.runner_monitor.start(
        server.into(),
        runner_view(&runner).unwrap(),
        runtime.wsl2_startup.snapshot.generation.clone(),
    );
    (tmp, runtime)
}

fn attach_process(runtime: &mut ControllerRuntime) {
    // EOF, not the invoking terminal, owns this disposable process lifetime.
    // Use an absolute POSIX utility path: parallel test PATH changes must not
    // make the mock Runner fail to spawn before observation is exercised.
    let mut command = Command::new("/bin/cat");
    command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    runtime.runner.process =
        Some(spawn_process(command, "runner", runtime.log.clone(), None).unwrap());
}

async fn finish_poll(runtime: &mut ControllerRuntime) {
    runtime.runner_monitor.next_poll = Some(Instant::now());
    runtime.poll_runner_observation().await;
    tokio::time::timeout(Duration::from_secs(5), async {
        while !runtime.runner_monitor.task.as_ref().unwrap().is_finished() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    runtime.poll_runner_observation().await;
}

fn online_body(status: &str) -> String {
    json!({"success":true,"output":{"runners":[{"client_id":"observation-fixture","status":status}]}}).to_string()
}

async fn receive(stream: tokio::net::TcpStream) -> (BufReader<tokio::net::TcpStream>, String) {
    let mut reader = BufReader::new(stream);
    let mut first = String::new();
    reader.read_line(&mut first).await.unwrap();
    let mut length = 0;
    let mut bearer = String::new();
    loop {
        let mut line = String::new();
        reader.read_line(&mut line).await.unwrap();
        if line == "\r\n" {
            break;
        }
        if let Some(value) = line.to_lowercase().strip_prefix("content-length:") {
            length = value.trim().parse::<usize>().unwrap();
        }
        if line.to_lowercase().starts_with("authorization:") {
            bearer = line.split_once(':').unwrap().1.trim().to_owned();
        }
    }
    let mut bytes = vec![0; length];
    reader.read_exact(&mut bytes).await.unwrap();
    let request: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(request["tool"], "list_runners");
    assert_eq!(request["params"]["client_id"], "observation-fixture");
    (reader, bearer)
}

#[tokio::test]
async fn all_modes_share_online_offline_failure_and_recovery_observations() {
    for mode in [ServerMode::Local, ServerMode::Remote, ServerMode::Wsl2Nat] {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let server = format!("http://{}", listener.local_addr().unwrap());
        let (_tmp, mut runtime) = fixture(&server, mode);
        attach_process(&mut runtime);
        assert_eq!(runtime.snapshot().runner.phase, "starting");
        let responses = [
            (200, online_body("online")),
            (200, online_body("stale")),
            (401, "private-fixture-response".into()),
            (200, "invalid".into()),
            (
                200,
                json!({"success":true,"output":{"runners":[]}}).to_string(),
            ),
            (200, online_body("online")),
        ];
        let task = tokio::spawn(async move {
            for (index, (status, body)) in responses.into_iter().enumerate() {
                let (stream, _) = listener.accept().await.unwrap();
                let (mut reader, bearer) = receive(stream).await;
                assert_eq!(
                    bearer,
                    if index == 5 {
                        "Bearer rotated-fixture-token"
                    } else {
                        "Bearer first-fixture-token"
                    }
                );
                reader.get_mut().write_all(format!("HTTP/1.1 {status} Test\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len()).as_bytes()).await.unwrap();
            }
        });
        for (index, (phase, error)) in [
            ("ready", None),
            ("running_offline", None),
            ("running_unverified", Some("project_access_denied")),
            ("running_unverified", Some("invalid_runner_response")),
            ("running_unverified", Some("runner_not_visible")),
            ("ready", None),
        ]
        .into_iter()
        .enumerate()
        {
            if index == 5 {
                std::fs::write(
                    runtime.config.runner.user_token_file.as_ref().unwrap(),
                    "rotated-fixture-token",
                )
                .unwrap();
            }
            finish_poll(&mut runtime).await;
            let snapshot = runtime.snapshot();
            assert_eq!(snapshot.runner.phase, phase);
            assert_eq!(snapshot.runner.observation_error, error);
            assert!(snapshot.runner.observed_at.is_some());
            assert_eq!(runtime.runner.restart_count, 0);
            assert!(runtime.wsl2_startup.retry_at.is_none());
            assert!(
                runtime.runner_monitor.next_poll.unwrap()
                    >= runtime.runner_monitor.sample.as_ref().unwrap().at + POLL_INTERVAL
            );
            runtime.poll_runner_observation().await;
            assert!(runtime.runner_monitor.task.is_none());
            assert!(!serde_json::to_string(&snapshot)
                .unwrap()
                .contains("fixture-token"));
        }
        task.await.unwrap();
        runtime.stop_all().await;
        assert!(runtime.runner_monitor.target.is_none());
        assert_eq!(runtime.snapshot().runner.phase, "stopped");
        assert!(runtime.snapshot().runner.observed_at.is_none());
    }
}

#[test]
fn observations_expire_with_a_monotonic_clock_and_cannot_override_lifecycle() {
    let (_tmp, mut runtime) = fixture("http://127.0.0.1:1", ServerMode::Remote);
    let target = runtime.runner_monitor.target.clone().unwrap();
    runtime.runner_monitor.sample = Some(sample(target, Ok(true)));
    let at = runtime.runner_monitor.sample.as_ref().unwrap().at;
    assert_eq!(
        runtime
            .runner_monitor
            .projection(at + MAX_AGE - Duration::from_nanos(1)),
        ("ready", None)
    );
    assert_eq!(
        runtime.runner_monitor.projection(at + MAX_AGE),
        ("running_unverified", Some("runner_observation_expired"))
    );
    for phase in [
        "stopped",
        "failed",
        "waiting_for_server",
        "restarting",
        "disabled",
    ] {
        let mut snapshot = ComponentSnapshot {
            enabled: true,
            phase,
            pid: Some(123),
            restart_count: 0,
            observed_at: None,
            observation_error: None,
        };
        runtime.runner_monitor.decorate(&mut snapshot, at);
        assert_eq!(snapshot.phase, phase);
    }
}

#[tokio::test]
async fn missing_invalid_and_rotated_credentials_update_without_restarting() {
    let (_tmp, mut runtime) = fixture("http://127.0.0.1:1", ServerMode::Remote);
    attach_process(&mut runtime);
    let path = runtime.config.runner.user_token_file.clone().unwrap();
    for (value, error) in [
        (None, "user_credential_unavailable"),
        (Some(""), "user_credential_invalid"),
        (Some("wc_agent_fixture"), "user_credential_invalid"),
    ] {
        if let Some(value) = value {
            std::fs::write(&path, value).unwrap();
        } else {
            std::fs::remove_file(&path).unwrap();
        }
        finish_poll(&mut runtime).await;
        assert_eq!(runtime.snapshot().runner.phase, "running_unverified");
        assert_eq!(runtime.snapshot().runner.observation_error, Some(error));
        assert_eq!(runtime.runner.restart_count, 0);
    }
    runtime.stop_all().await;
}

#[tokio::test]
async fn in_flight_observation_does_not_block_ipc_overlap_or_survive_stop() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let server = format!("http://{}", listener.local_addr().unwrap());
    let (_tmp, mut runtime) = fixture(&server, ServerMode::Remote);
    attach_process(&mut runtime);
    let (tx, rx) = tokio::sync::oneshot::channel();
    let peer = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let (stream, _) = receive(stream).await;
        tx.send(()).unwrap();
        std::future::pending::<()>().await;
        drop(stream);
    });
    runtime.poll_runner_observation().await;
    rx.await.unwrap();
    let id = runtime.runner_monitor.task.as_ref().unwrap().id();
    let abort = runtime.runner_monitor.task.as_ref().unwrap().abort_handle();
    runtime.runner_monitor.next_poll = Some(Instant::now());
    runtime.poll_runner_observation().await;
    assert_eq!(runtime.runner_monitor.task.as_ref().unwrap().id(), id);
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
    assert_eq!(
        serde_json::from_str::<Value>(&response).unwrap()["result"]["runner"]["phase"],
        "starting"
    );
    runtime.stop_component(Component::Runner).await;
    tokio::task::yield_now().await;
    assert!(abort.is_finished());
    assert!(runtime.runner_monitor.sample.is_none());
    peer.abort();
}

#[tokio::test]
async fn stale_generation_identity_changes_and_process_exit_cannot_report_ready() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let server = format!("http://{}", listener.local_addr().unwrap());
    let (_tmp, mut runtime) = fixture(&server, ServerMode::Remote);
    attach_process(&mut runtime);
    let old = runtime.runner_monitor.target.clone().unwrap();
    runtime.runner_monitor.start(server, old.view.clone(), None);
    let old_task = tokio::spawn(async move { sample(old, Ok(true)) });
    while !old_task.is_finished() {
        tokio::task::yield_now().await;
    }
    runtime.runner_monitor.task = Some(old_task);
    runtime.poll_runner_observation().await;
    assert_ne!(runtime.snapshot().runner.phase, "ready");
    let original = std::fs::read_to_string(&runtime.config.runner.config).unwrap();
    std::fs::write(
        &runtime.config.runner.config,
        original.replace("observation-fixture", "replacement-fixture"),
    )
    .unwrap();
    runtime.poll_runner_observation().await;
    assert_eq!(
        runtime.snapshot().runner.observation_error,
        Some("runner_identity_changed")
    );
    assert!(runtime.runner_monitor.task.is_none());
    runtime.stop_component(Component::Runner).await;
    let mut command = Command::new("sh");
    command
        .args(["-c", "exit 77"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    let mut process = spawn_process(command, "runner", runtime.log.clone(), None).unwrap();
    process.child.wait().await.unwrap();
    runtime.runner.process = Some(process);
    let target = Target {
        process_generation: uuid::Uuid::new_v4(),
        server: "unused".into(),
        view: runner_view(&runtime.config.runner.config).unwrap(),
        wsl2_generation: None,
    };
    runtime.runner_monitor.target = Some(target.clone());
    runtime.runner_monitor.sample = Some(sample(target, Ok(true)));
    runtime.reap_and_recover().await;
    assert_eq!(runtime.snapshot().runner.phase, "failed");
    assert!(runtime.snapshot().runner.observed_at.is_none());
    assert!(runtime.runner_monitor.target.is_none());
}

#[tokio::test]
async fn doctor_and_text_views_use_the_same_live_snapshot_and_offline_is_unknown() {
    let (_tmp, mut runtime) = fixture("http://127.0.0.1:1", ServerMode::Remote);
    attach_process(&mut runtime);
    let target = runtime.runner_monitor.target.clone().unwrap();
    runtime.runner_monitor.sample = Some(sample(target, Ok(true)));
    let snapshot = serde_json::to_value(runtime.snapshot()).unwrap();
    let listener = UnixListener::bind(controller_socket(&runtime.config).unwrap()).unwrap();
    let sent = snapshot.clone();
    let peer = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let mut request = String::new();
        {
            let mut reader = BufReader::new(&mut stream);
            reader.read_line(&mut request).await.unwrap();
        }
        stream
            .write_all(format!("{}\n", json!({"ok":true,"result":sent})).as_bytes())
            .await
            .unwrap();
    });
    let observed = live_snapshot(&runtime.config).await.unwrap();
    let mut report = json!({"runner":{"enabled":true,"config_file":true,"binary":true,"server_matches_controller":true}});
    attach_runtime(&mut report, observed);
    assert_eq!(report["runtime"], snapshot);
    assert_eq!(report["runner"]["phase"], "ready");
    assert_eq!(
        report["runner"]["observed_at"],
        snapshot["runner"]["observed_at"]
    );
    assert_eq!(runner_doctor_text(&report), "ready (configuration: ok)");
    assert!(runner_observation_text(&snapshot).contains("Runner observed:"));
    peer.await.unwrap();
    attach_runtime(&mut report, None);
    assert_eq!(report["runner"]["phase"], "unknown");
    assert!(report["runtime"].is_null());
    assert!(report["runner"].get("observed_at").is_none());
    runtime.stop_all().await;
}
