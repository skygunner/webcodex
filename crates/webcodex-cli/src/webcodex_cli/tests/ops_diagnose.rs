use super::support::*;
use crate::webcodex_cli::ops::ops_exit_code;
use crate::webcodex_cli::run_ops_command;
use std::time::Duration;

fn diagnose_opts(server_url: String) -> OpsDiagnoseProviderOptions {
    OpsDiagnoseProviderOptions {
        common: OpsCommonOptions {
            server_url,
            server_http: direct_server_http(),
            env_file: None,
            token_file: None,
            token: None,
            json: false,
            strict: false,
        },
        project: "agent:mini:demo".to_string(),
        provider: "codex".to_string(),
        include_live_acp: false,
    }
}

fn diagnose_response_body() -> String {
    json!({
        "success": true,
        "output": {
            "project": "agent:mini:demo",
            "provider_id": "codex",
            "env_presence": {"CODEX_PATH": true, "CODEX_HOME": false},
            "live_acp": null
        }
    })
    .to_string()
}

// The CLI is only a consumer of the authorized Server entry: the fixture pins
// the exact tool name and typed arguments, and proves no credential is printed.
#[tokio::test]
async fn diagnose_provider_calls_the_canonical_server_entry_and_renders_presence() {
    use std::io::BufRead as _;
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let address = listener.local_addr().unwrap();
    let (stop_tx, stop_rx) = std::sync::mpsc::channel();
    let server = thread::spawn(move || {
        let mut bodies = Vec::new();
        loop {
            match listener.accept() {
                Ok((mut stream, _)) => {
                    stream.set_nonblocking(false).unwrap();
                    stream
                        .set_read_timeout(Some(Duration::from_secs(5)))
                        .unwrap();
                    let mut reader = std::io::BufReader::new(&mut stream);
                    let mut length = 0;
                    loop {
                        let mut line = String::new();
                        reader.read_line(&mut line).unwrap();
                        if line == "\r\n" || line.is_empty() {
                            break;
                        }
                        if let Some(value) =
                            line.to_ascii_lowercase().strip_prefix("content-length:")
                        {
                            length = value.trim().parse::<usize>().unwrap();
                        }
                    }
                    let mut body = vec![0u8; length];
                    reader.read_exact(&mut body).unwrap();
                    let request: Value = serde_json::from_slice(&body).unwrap();
                    bodies.push(request.clone());
                    let response = diagnose_response_body();
                    write!(
                        stream,
                        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                        response.len(),
                        response
                    )
                    .unwrap();
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    if stop_rx.try_recv().is_ok() {
                        break;
                    }
                    thread::sleep(Duration::from_millis(5));
                }
                Err(error) => panic!("fixture accept: {error}"),
            }
        }
        bodies
    });
    let mut opts = diagnose_opts(format!("http://{address}"));
    opts.common.token = Some("wc_pat_diagnose_fixture".into());
    let result = run_ops_command(OpsCommand::DiagnoseProvider(opts))
        .await
        .unwrap();
    stop_tx.send(()).unwrap();
    let bodies = server.join().unwrap();
    assert_eq!(bodies.len(), 1);
    assert_eq!(bodies[0]["tool"], "diagnose_coding_agent");
    assert_eq!(bodies[0]["params"]["project"], "agent:mini:demo");
    assert_eq!(bodies[0]["params"]["provider_id"], "codex");
    assert_eq!(bodies[0]["params"]["include_live_acp"], false);
    // Human rendering names presence only; the token and any env value never
    // appear (the fixture response carries no values, and the renderer prints
    // only the boolean mapping).
    assert!(result.stdout.contains("CODEX_PATH present"));
    assert!(result.stdout.contains("CODEX_HOME ABSENT"));
    assert!(result.stdout.contains("Overall: WARN"));
    assert!(result.stdout.contains("env_source_absent"));
    assert!(!result.stdout.contains("wc_pat_diagnose_fixture"));
}

#[tokio::test]
async fn diagnose_provider_reports_server_side_failures_as_fail() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let response = json!({
        "success": false,
        "error": "include_live_acp=true requires start_coding_agent authority: missing required scope: project:write",
        "output": {
            "error_kind": "coding_agent_live_probe_authority_required",
            "run_id": null,
            "execution_state": "not_started"
        }
    })
    .to_string();
    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        stream.set_nonblocking(false).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        let mut length = 0;
        {
            let mut reader = std::io::BufReader::new(&stream);
            use std::io::BufRead as _;
            loop {
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                if line == "\r\n" || line.is_empty() {
                    break;
                }
                if let Some(value) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                    length = value.trim().parse::<usize>().unwrap();
                }
            }
            let mut body = vec![0u8; length];
            reader.read_exact(&mut body).unwrap();
            let request: Value = serde_json::from_slice(&body).unwrap();
            assert_eq!(request["tool"], "diagnose_coding_agent");
            assert_eq!(request["params"]["include_live_acp"], true);
        }
        stream
            .write_all(
                format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    response.len(),
                    response
                )
                .as_bytes(),
            )
            .unwrap();
    });
    let mut opts = diagnose_opts(format!("http://{address}"));
    opts.common.token = Some("wc_pat_diagnose_fixture".into());
    opts.common.strict = true;
    opts.include_live_acp = true;
    let result = run_ops_command(OpsCommand::DiagnoseProvider(opts))
        .await
        .unwrap();
    server.join().unwrap();
    assert_eq!(result.status, "fail");
    assert!(result
        .stdout
        .contains("coding_agent_live_probe_authority_required"));
    assert_eq!(
        ops_exit_code(true, "fail"),
        2,
        "--strict must map a FAIL diagnose to exit 2"
    );
}

#[test]
fn diagnose_provider_parses_flags_and_requires_project_and_provider() {
    match cli_action([
        "ops",
        "diagnose-provider",
        "--project",
        "agent:mini:demo",
        "--provider",
        "codex",
        "--include-live-acp",
        "--json",
        "--strict",
    ]) {
        CliAction::Ops(OpsCommand::DiagnoseProvider(opts)) => {
            assert_eq!(opts.project, "agent:mini:demo");
            assert_eq!(opts.provider, "codex");
            assert!(opts.include_live_acp);
            assert!(opts.common.json);
            assert!(opts.common.strict);
        }
        other => panic!("diagnose-provider did not parse: {other:?}"),
    }
    match cli_action(["ops", "diagnose-provider", "--project", "p"]) {
        CliAction::Exit { code, stderr, .. } => {
            assert_eq!(code, 2);
            assert!(stderr.contains("--provider is required"), "{stderr}");
        }
        other => panic!("missing provider must fail parsing: {other:?}"),
    }
    match cli_action(["ops", "diagnose-provider"]) {
        CliAction::Exit { code, stderr, .. } => {
            assert_eq!(code, 2);
            assert!(stderr.contains("--project is required"), "{stderr}");
        }
        other => panic!("missing project must fail parsing: {other:?}"),
    }
    match cli_action(["ops", "diagnose-provider", "--bogus"]) {
        CliAction::Exit { code, stderr, .. } => {
            assert_eq!(code, 2);
            assert!(
                stderr.contains("unknown ops diagnose-provider flag"),
                "{stderr}"
            );
        }
        other => panic!("unknown flag must fail parsing: {other:?}"),
    }
}

#[test]
fn diagnose_provider_help_is_available() {
    let out = cli_exit(["ops", "diagnose-provider", "--help"]).unwrap();
    assert!(out.contains("Usage: webcodex ops diagnose-provider"));
    assert!(out.contains("--include-live-acp"));
    let ops_help = cli_exit(["ops", "--help"]).unwrap();
    assert!(ops_help.contains("diagnose-provider"));
}
