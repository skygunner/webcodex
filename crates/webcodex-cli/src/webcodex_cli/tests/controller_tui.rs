use super::*;
use ratatui::{backend::TestBackend, Terminal};
use serde_json::json;
use std::os::unix::process::ExitStatusExt;

/// Re-exec under an owned PTY: no developer terminal or real Controller is used.
/// The observer executable is this libtest binary, whose unsupported Controller
/// arguments fail locally instead of contacting a service.
#[cfg(target_os = "linux")]
#[test]
fn controller_tui_sigterm_restores_terminal() {
    use std::os::{fd::FromRawFd, unix::process::CommandExt};
    const FIXTURE: &str = "WEBCODEX_TUI_PTY_FIXTURE";
    if std::env::var_os(FIXTURE).is_some() {
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(1)
            .enable_all()
            .build()
            .unwrap()
            .block_on(run(
                "/missing/controller.toml".into(),
                "/missing/controller.service".into(),
                None,
            ))
            .unwrap();
        return;
    }
    let mut master_fd = -1;
    let mut slave_fd = -1;
    let size = libc::winsize {
        ws_row: 30,
        ws_col: 100,
        ws_xpixel: 0,
        ws_ypixel: 0,
    };
    // SAFETY: valid output pointers; returned descriptors are exclusively owned below.
    assert_eq!(
        unsafe {
            libc::openpty(
                &mut master_fd,
                &mut slave_fd,
                std::ptr::null_mut(),
                std::ptr::null(),
                &size,
            )
        },
        0
    );
    let mut master = unsafe { std::fs::File::from_raw_fd(master_fd) };
    let slave = unsafe { std::fs::File::from_raw_fd(slave_fd) };
    let mut original = std::mem::MaybeUninit::<libc::termios>::uninit();
    assert_eq!(
        unsafe { libc::tcgetattr(slave_fd, original.as_mut_ptr()) },
        0
    );
    let original = unsafe { original.assume_init() };
    let flags = unsafe { libc::fcntl(master_fd, libc::F_GETFL) };
    assert!(flags >= 0);
    assert_eq!(
        unsafe { libc::fcntl(master_fd, libc::F_SETFL, flags | libc::O_NONBLOCK) },
        0
    );
    let mut command = Command::new(std::env::current_exe().unwrap());
    command
        .args([
            "--exact",
            "webcodex_cli::controller::tui::tests::controller_tui_sigterm_restores_terminal",
            "--nocapture",
            "--test-threads=1",
        ])
        .env(FIXTURE, "1")
        .env("TOKIO_WORKER_THREADS", "1")
        .stdin(Stdio::from(slave.try_clone().unwrap()))
        .stdout(Stdio::from(slave.try_clone().unwrap()))
        .stderr(Stdio::from(slave.try_clone().unwrap()));
    // SAFETY: the child only performs async-signal-safe session/terminal syscalls.
    unsafe {
        command.pre_exec(|| {
            if libc::setsid() < 0 || libc::ioctl(0, libc::TIOCSCTTY, 0) < 0 {
                return Err(io::Error::last_os_error());
            }
            Ok(())
        });
    }
    struct Fixture(std::process::Child);
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
    let mut child = Fixture(command.spawn().unwrap());
    let mut output = Vec::new();
    let read = |master: &mut std::fs::File, output: &mut Vec<u8>| {
        let mut bytes = [0; 8192];
        match master.read(&mut bytes) {
            Ok(count) => {
                output.extend_from_slice(&bytes[..count]);
                assert!(output.len() <= OUTPUT_LIMIT);
            }
            Err(e)
                if e.kind() == io::ErrorKind::WouldBlock
                    || e.kind() == io::ErrorKind::Interrupted => {}
            Err(e) => panic!("PTY read failed: {e}"),
        }
    };
    let deadline = Instant::now() + Duration::from_secs(5);
    while output
        .windows(6)
        .filter(|bytes| *bytes == b"\x1b[?25l")
        .count()
        < 2
    {
        assert!(Instant::now() < deadline, "TUI did not render two frames");
        assert!(
            child.0.try_wait().unwrap().is_none(),
            "TUI fixture exited before rendering"
        );
        read(&mut master, &mut output);
        thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(unsafe { libc::kill(child.0.id() as i32, libc::SIGTERM) }, 0);
    let deadline = Instant::now() + Duration::from_secs(5);
    let status = loop {
        read(&mut master, &mut output);
        if let Some(status) = child.0.try_wait().unwrap() {
            break status;
        }
        assert!(Instant::now() < deadline, "SIGTERM did not exit the TUI");
        thread::sleep(Duration::from_millis(10));
    };
    read(&mut master, &mut output);
    assert!(status.success());
    let mut restored = std::mem::MaybeUninit::<libc::termios>::uninit();
    assert_eq!(
        unsafe { libc::tcgetattr(slave_fd, restored.as_mut_ptr()) },
        0
    );
    let restored = unsafe { restored.assume_init() };
    assert_eq!(
        (
            restored.c_iflag,
            restored.c_oflag,
            restored.c_cflag,
            restored.c_lflag,
            restored.c_cc
        ),
        (
            original.c_iflag,
            original.c_oflag,
            original.c_cflag,
            original.c_lflag,
            original.c_cc
        )
    );
    assert!(output.windows(8).any(|bytes| bytes == b"\x1b[?1049l"));
    assert!(output.windows(6).any(|bytes| bytes == b"\x1b[?25h"));
}

fn status() -> Value {
    json!({"source":"foreground","service":{"unit":"controller.service","loaded":"not-found","active":"inactive","enabled":"disabled"},
        "runtime":{"controller":"running","server":{"phase":"ready","pid":123,"restart_count":2},
            "runner":{"phase":"ready","pid":124,"restart_count":0},"tunnel":{"phase":"disabled","pid":null,"restart_count":0}}})
}

fn projects() -> Value {
    json!({"projects":[{"id":"agent:demo","path":"/repo/demo"}],"truncated":true,"project_inventory":{"sync_state":"syncing"}})
}

fn render(app: &mut Overview, width: u16, height: u16) -> String {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    terminal.draw(|frame| draw(frame, app)).unwrap();
    terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect()
}

#[test]
fn runner_observations_render_from_the_daemon_without_project_state_inference() {
    for phase in ["ready", "running_offline", "running_unverified"] {
        let mut observed = status();
        observed["runtime"]["runner"]["phase"] = json!(phase);
        observed["runtime"]["runner"]["observed_at"] = json!("2026-10-16T01:02:03Z");
        if phase == "running_unverified" {
            observed["runtime"]["runner"]["observation_error"] = json!("project_access_denied");
        }
        let mut app = Overview::new();
        app.status.update(Ok(observed));
        let screen = render(&mut app, 160, 30);
        assert!(screen.contains(phase));
        assert!(screen.contains("2026-10-16T01:02:03Z"));
        if phase == "running_unverified" {
            assert!(screen.contains("project_access_denied"));
        }
    }
}

#[test]
fn controller_tui_parser_preserves_paths_and_rejects_mutation_flags() {
    use super::super::{parse_controller_command, ControllerCommand};
    let command = parse_controller_command(
        &[
            "tui",
            "--config",
            "/tmp/custom config",
            "--service-file",
            "/tmp/controller.service",
            "--user-token-file",
            "/tmp/user-token",
        ]
        .map(String::from),
    )
    .unwrap();
    assert_eq!(
        command,
        ControllerCommand::Tui {
            config: "/tmp/custom config".into(),
            service_file: "/tmp/controller.service".into(),
            user_token_file: Some("/tmp/user-token".into())
        }
    );
    for flag in [
        "--json",
        "--confirm",
        "--overwrite",
        "--no-start",
        "-f",
        "--environment-file",
    ] {
        assert!(parse_controller_command(&["tui".into(), flag.into()])
            .unwrap_err()
            .contains("unknown controller tui option"));
    }
    assert!(parse_controller_command(&["tui".into(), "--config".into()])
        .unwrap_err()
        .contains("requires PATH"));
}

#[test]
fn overview_renders_components_inventory_and_logs_without_healthy_defaults() {
    let mut app = Overview::new();
    app.status.update(Ok(status()));
    app.projects.update(Ok(projects()));
    app.log("[runner] ready");
    let screen = render(&mut app, 100, 30);
    for value in [
        "foreground",
        "running",
        "controller.service",
        "not-found",
        "123",
        "124",
        "2",
        "—",
        "agent:demo",
        "/repo/demo",
        "sync=syncing",
        "truncated=yes",
        "[runner] ready",
    ] {
        assert!(screen.contains(value), "missing {value}: {screen}");
    }
    app.status.update(Ok(json!({"source":"systemd","runtime":null,"service":{"unit":"controller.service","loaded":"loaded","active":"inactive","enabled":"enabled"}})));
    let screen = render(&mut app, 100, 30);
    assert!(screen.contains("not reachable"));
    assert!(screen.contains("unavailable"));
    assert!(!screen.contains("123"));
}

#[test]
fn failed_queries_keep_last_data_visibly_stale_and_render_small_windows() {
    let mut app = Overview::new();
    app.status.update(Ok(status()));
    app.projects.update(Ok(projects()));
    app.status.update(Err("socket unavailable".into()));
    app.projects.update(Err("runner_offline".into()));
    let screen = render(&mut app, 120, 35);
    for value in [
        "stale",
        "socket unavailable",
        "runner_offline",
        "last success",
        "agent:demo",
    ] {
        assert!(screen.contains(value), "missing {value}");
    }
    assert!(render(&mut app, 40, 12).contains("80×24"));
    for (width, height) in [(1, 1), (80, 24), (160, 50)] {
        render(&mut app, width, height);
    }
}

#[test]
fn scroll_pause_refresh_and_exit_are_read_only() {
    let mut app = Overview::new();
    for index in 0..100 {
        app.log(&index.to_string());
    }
    render(&mut app, 100, 30);
    let key = |code| KeyEvent::new(code, KeyModifiers::NONE);
    app.key(key(KeyCode::Char(' ')));
    assert!(!app.follow_logs);
    let offset = app.log_offset;
    app.log("new");
    render(&mut app, 100, 30);
    assert_eq!(app.log_offset, offset);
    app.key(key(KeyCode::PageUp));
    assert!(app.log_offset < offset);
    app.key(key(KeyCode::Tab));
    assert!(!app.focus_logs);
    app.key(key(KeyCode::Char(' ')));
    assert!(app.follow_logs);
    assert!(!app.key(key(KeyCode::Char('r'))));
    assert!(app.key(key(KeyCode::Char('q'))));
    assert!(app.key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL)));
    assert!(require_terminal(false, true).is_err());
    assert!(require_terminal(true, false).is_err());
    assert!(require_terminal(true, true).is_ok());
}

#[test]
fn log_cache_bounds_count_bytes_and_escapes_terminal_controls() {
    let mut app = Overview::new();
    for _ in 0..501 {
        app.log("same");
    }
    assert_eq!(app.logs.len(), 500);
    assert_eq!(app.evicted, 1);
    for _ in 0..20 {
        app.log(&"x".repeat(LOG_LINE_LIMIT));
    }
    assert!(app.log_bytes <= OUTPUT_LIMIT);
    assert_eq!(
        app.log_bytes,
        app.logs.iter().map(String::len).sum::<usize>()
    );
    app.log("\x1b[31mred\r\t\x07");
    assert_eq!(app.logs.back().unwrap(), "\\u{1b}[31mred\\r\\t\\u{7}");
    assert!(!app.logs.back().unwrap().chars().any(char::is_control));
}

#[test]
fn observer_commands_are_exact_read_only_argv_and_scope_credentials() {
    let target = Target {
        executable: "/opt/web codex/webcodex".into(),
        config: "/tmp/custom config".into(),
        service_file: "/tmp/custom.service".into(),
        user_token_file: Some("/tmp/user-token".into()),
    };
    for (query, expected) in [
        (
            Some(Query::Status),
            vec![
                "controller",
                "status",
                "--json",
                "--config",
                "/tmp/custom config",
                "--service-file",
                "/tmp/custom.service",
            ],
        ),
        (
            Some(Query::Projects),
            vec![
                "controller",
                "project",
                "list",
                "--json",
                "--config",
                "/tmp/custom config",
                "--user-token-file",
                "/tmp/user-token",
            ],
        ),
        (
            None,
            vec![
                "controller",
                "logs",
                "-f",
                "--lines",
                "100",
                "--config",
                "/tmp/custom config",
                "--service-file",
                "/tmp/custom.service",
            ],
        ),
    ] {
        let command = target.command(query);
        assert_eq!(command.get_program(), "/opt/web codex/webcodex");
        assert_eq!(
            command.get_args().collect::<Vec<_>>(),
            expected
                .iter()
                .map(std::ffi::OsStr::new)
                .collect::<Vec<_>>()
        );
    }
}

#[test]
fn decoder_rejects_incomplete_inventory_instead_of_showing_empty_success() {
    let decode = |value: Value| {
        decode_query(
            Query::Projects,
            (
                ExitStatus::from_raw(0),
                serde_json::to_vec(&value).unwrap(),
                Vec::new(),
            ),
        )
    };
    assert!(decode(projects()).is_ok());
    assert!(decode(json!({"projects":[]})).is_err());
    assert!(decode(json!({"success":false,"error":"runner_offline"})).is_err());
    let error = decode_query(
        Query::Projects,
        (
            ExitStatus::from_raw(256),
            Vec::new(),
            b"runner_offline".to_vec(),
        ),
    )
    .unwrap_err();
    assert_eq!(error, "runner_offline");
}

fn fixture(script: &str) -> Command {
    let mut command = Command::new("/bin/sh");
    command.args(["-c", script]);
    command
}

#[test]
fn observer_drains_both_pipes_closes_stdin_and_enforces_output_bound() {
    let cancel = Arc::new(AtomicBool::new(false));
    let output = observe(
        &mut fixture("read value && exit 7; printf '%0200000d' 0; printf '%0200000d' 0 >&2"),
        Some(Duration::from_secs(5)),
        None,
        &cancel,
    )
    .unwrap();
    assert!(output.0.success());
    assert_eq!(output.1.len(), 200000);
    assert_eq!(output.2.len(), 200000);
    let error = observe(
        &mut fixture("printf '%01048577d' 0"),
        Some(Duration::from_secs(5)),
        None,
        &cancel,
    )
    .unwrap_err();
    assert!(error.contains("output limit exceeded"));
}

#[test]
fn log_observer_delivers_duplicates_and_final_unterminated_line() {
    let cancel = Arc::new(AtomicBool::new(false));
    let (tx, rx) = mpsc::sync_channel(8);
    let output = observe(
        &mut fixture("printf 'same\\nsame\\nfinal'; printf 'warning' >&2"),
        Some(Duration::from_secs(5)),
        Some(tx),
        &cancel,
    )
    .unwrap();
    assert_eq!(output.2, b"warning");
    let lines = rx
        .try_iter()
        .map(|message| match message {
            Message::Log(line) => line,
            _ => panic!("unexpected query"),
        })
        .collect::<Vec<_>>();
    assert_eq!(lines, ["same", "same", "final"]);
}

#[test]
fn oversized_log_lines_fail_explicitly_and_old_controllers_stop_reconnecting() {
    let cancel = Arc::new(AtomicBool::new(false));
    let (tx, _rx) = mpsc::sync_channel(8);
    let error = observe(
        &mut fixture("printf '%065537d' 0"),
        Some(Duration::from_secs(5)),
        Some(tx),
        &cancel,
    )
    .unwrap_err();
    assert!(error.contains("output limit exceeded"));
    let output = observe(&mut fixture("printf 'The running Controller does not support log following; restart it with the updated binary' >&2; exit 1"), Some(Duration::from_secs(5)), None, &cancel).unwrap();
    assert!(matches!(
        log_result(Ok(output)),
        Message::LogsEnded {
            unsupported: true,
            ..
        }
    ));
    assert!(matches!(
        log_result(Ok((ExitStatus::from_raw(0), Vec::new(), Vec::new()))),
        Message::LogsEnded {
            unsupported: false,
            ..
        }
    ));
    assert!(matches!(
        log_result(Err(
            "Controller observation process tree did not exit".into()
        )),
        Message::Fatal(_)
    ));
}

#[test]
fn cancelling_a_full_log_queue_does_not_deadlock_cleanup() {
    let cancel = Arc::new(AtomicBool::new(false));
    let (tx, rx) = mpsc::sync_channel(1);
    let worker_cancel = cancel.clone();
    let worker = thread::spawn(move || {
        observe(
            &mut fixture("printf 'first\\nsecond\\nthird\\n'; sleep 30"),
            None,
            Some(tx),
            &worker_cancel,
        )
    });
    assert!(matches!(
        rx.recv_timeout(Duration::from_secs(5)).unwrap(),
        Message::Log(_)
    ));
    cancel.store(true, Ordering::Release);
    assert!(worker.join().unwrap().unwrap_err().contains("cancelled"));
}

#[cfg(target_os = "linux")]
#[test]
fn observer_timeout_cleans_up_descendants_and_reaps_the_leader() {
    let temp = tempfile::tempdir().unwrap();
    let pid_file = temp.path().join("pid");
    let mut command = fixture("sleep 30 & printf '%s' \"$!\" > \"$1\"; wait");
    command.arg("fixture").arg(&pid_file);
    let error = observe(
        &mut command,
        Some(Duration::from_millis(300)),
        None,
        &Arc::new(AtomicBool::new(false)),
    )
    .unwrap_err();
    assert!(error.contains("timed out"));
    let pid: u32 = std::fs::read_to_string(pid_file).unwrap().parse().unwrap();
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat"));
    assert!(
        stat.is_err()
            || stat
                .unwrap()
                .rsplit_once(')')
                .unwrap()
                .1
                .trim_start()
                .starts_with('Z'),
        "owned descendant is still running"
    );
}
