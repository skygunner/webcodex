//! Read-only terminal adapter over the existing Controller CLI contracts.
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::{
    layout::{Constraint, Layout, Rect},
    style::{Color, Style},
    widgets::{Block, Borders, Cell, Paragraph, Row, Table, Wrap},
    Frame,
};
use serde_json::Value;
use std::{
    collections::VecDeque,
    io::{self, IsTerminal, Read},
    path::PathBuf,
    process::{Command, ExitStatus, Stdio},
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        mpsc::{self, SyncSender, TrySendError},
        Arc,
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};
use webcodex_process::ManagedChild;

const OUTPUT_LIMIT: usize = 1024 * 1024;
const LOG_LINE_LIMIT: usize = 64 * 1024;
const LOG_LIMIT: usize = 500;
const STATUS_INTERVAL: Duration = Duration::from_secs(2);
const PROJECT_INTERVAL: Duration = Duration::from_secs(10);
const STATUS_TIMEOUT: Duration = Duration::from_secs(10);
const PROJECT_TIMEOUT: Duration = Duration::from_secs(180);

#[derive(Clone)]
struct Target {
    executable: PathBuf,
    config: PathBuf,
    service_file: PathBuf,
    user_token_file: Option<PathBuf>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Query {
    Status,
    Projects,
}

impl Target {
    fn command(&self, query: Option<Query>) -> Command {
        let mut command = Command::new(&self.executable);
        command.env("TOKIO_WORKER_THREADS", "1");
        command.arg("controller");
        match query {
            Some(Query::Status) => {
                command.args(["status", "--json"]);
            }
            Some(Query::Projects) => {
                command.args(["project", "list", "--json"]);
            }
            None => {
                command.args(["logs", "-f", "--lines", "100"]);
            }
        }
        command.arg("--config").arg(&self.config);
        if query == Some(Query::Projects) {
            if let Some(path) = &self.user_token_file {
                command.arg("--user-token-file").arg(path);
            }
        } else {
            command.arg("--service-file").arg(&self.service_file);
        }
        command
    }
}

enum Message {
    Query(Query, Result<Value, String>),
    Fatal(String),
    Log(String),
    LogsEnded { error: String, unsupported: bool },
}

fn send(tx: &SyncSender<Message>, cancel: &AtomicBool, mut message: Message) -> bool {
    loop {
        if cancel.load(Ordering::Acquire) {
            return false;
        }
        match tx.try_send(message) {
            Ok(()) => return true,
            Err(TrySendError::Disconnected(_)) => return false,
            Err(TrySendError::Full(value)) => message = value,
        }
        thread::sleep(Duration::from_millis(10));
    }
}

struct Workers {
    cancel: Arc<AtomicBool>,
    handles: Vec<JoinHandle<()>>,
    tx: SyncSender<Message>,
    target: Target,
}

impl Workers {
    fn new(target: Target, tx: SyncSender<Message>) -> Self {
        Self {
            target,
            tx,
            cancel: Arc::new(AtomicBool::new(false)),
            handles: Vec::new(),
        }
    }

    fn query(&mut self, query: Query) {
        let mut command = self.target.command(Some(query));
        let tx = self.tx.clone();
        let cancel = self.cancel.clone();
        self.handles.push(thread::spawn(move || {
            let timeout = match query {
                Query::Status => STATUS_TIMEOUT,
                Query::Projects => PROJECT_TIMEOUT,
            };
            let result = observe(&mut command, Some(timeout), None, &cancel)
                .and_then(|output| decode_query(query, output));
            if let Err(error) = &result {
                if cleanup_failed(error) {
                    send(&tx, &cancel, Message::Fatal(error.clone()));
                    return;
                }
            }
            send(&tx, &cancel, Message::Query(query, result));
        }));
    }

    fn logs(&mut self) {
        let mut command = self.target.command(None);
        let tx = self.tx.clone();
        let cancel = self.cancel.clone();
        self.handles.push(thread::spawn(move || {
            let message = log_result(observe(&mut command, None, Some(tx.clone()), &cancel));
            send(&tx, &cancel, message);
        }));
    }

    fn reap(&mut self) -> Result<(), String> {
        let mut index = 0;
        while index < self.handles.len() {
            if self.handles[index].is_finished() {
                self.handles
                    .swap_remove(index)
                    .join()
                    .map_err(|_| "Controller TUI observer failed".to_string())?;
            } else {
                index += 1;
            }
        }
        Ok(())
    }
}

impl Drop for Workers {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Release);
        for handle in self.handles.drain(..) {
            let _ = handle.join();
        }
    }
}

type CapturedOutput = (ExitStatus, Vec<u8>, Vec<u8>);

fn log_result(result: Result<CapturedOutput, String>) -> Message {
    let error = match result {
        Ok((status, _, stderr)) => {
            let error = String::from_utf8_lossy(&stderr).trim().to_string();
            if error.is_empty() {
                format!("Log stream disconnected ({status})")
            } else {
                error
            }
        }
        Err(error) => error,
    };
    if cleanup_failed(&error) {
        return Message::Fatal(error);
    }
    let unsupported = error.contains("does not support log following");
    Message::LogsEnded { error, unsupported }
}

fn cleanup_failed(error: &str) -> bool {
    error.starts_with("Cannot clean up Controller observation")
        || error == "Controller observation process tree did not exit"
}

/// Own the observation tree and drain both pipes concurrently, including during cleanup.
fn observe(
    command: &mut Command,
    timeout: Option<Duration>,
    logs: Option<SyncSender<Message>>,
    cancel: &Arc<AtomicBool>,
) -> Result<CapturedOutput, String> {
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child =
        ManagedChild::spawn(command).map_err(|e| format!("Cannot start Controller query: {e}"))?;
    let total = Arc::new(AtomicUsize::new(0));
    let overflow = Arc::new(AtomicBool::new(false));
    let stdout = pipe_reader(
        child.child_mut().stdout.take().unwrap(),
        logs,
        total.clone(),
        overflow.clone(),
        cancel.clone(),
    );
    let stderr = pipe_reader(
        child.child_mut().stderr.take().unwrap(),
        None,
        total,
        overflow.clone(),
        cancel.clone(),
    );
    let started = Instant::now();
    let result = loop {
        if cancel.load(Ordering::Acquire) {
            break Err("Controller query cancelled".to_string());
        }
        if overflow.load(Ordering::Acquire) {
            break Err("Controller output limit exceeded".to_string());
        }
        if timeout.is_some_and(|timeout| started.elapsed() >= timeout) {
            break Err("Controller query timed out".to_string());
        }
        match child.try_wait() {
            Ok(Some(status)) => break Ok(status),
            Ok(None) => thread::sleep(Duration::from_millis(20)),
            Err(e) => break Err(format!("Cannot wait for Controller query: {e}")),
        }
    };
    // A command leader can exit while its journal/helper descendant still owns a pipe.
    let cleanup = child
        .terminate_tree()
        .and_then(|_| child.wait_tree_exit(Duration::from_secs(2)));
    match cleanup {
        Ok(true) => {}
        Ok(false) => return Err("Controller observation process tree did not exit".to_string()),
        Err(e) => return Err(format!("Cannot clean up Controller observation: {e}")),
    }
    let stdout = stdout
        .join()
        .map_err(|_| "Controller stdout reader failed".to_string())?
        .map_err(|e| format!("Cannot read Controller stdout: {e}"))?;
    let stderr = stderr
        .join()
        .map_err(|_| "Controller stderr reader failed".to_string())?
        .map_err(|e| format!("Cannot read Controller stderr: {e}"))?;
    if overflow.load(Ordering::Acquire) {
        return Err("Controller output limit exceeded".to_string());
    }
    result.map(|status| (status, stdout, stderr))
}

fn pipe_reader<R: Read + Send + 'static>(
    mut pipe: R,
    logs: Option<SyncSender<Message>>,
    total: Arc<AtomicUsize>,
    overflow: Arc<AtomicBool>,
    cancel: Arc<AtomicBool>,
) -> JoinHandle<io::Result<Vec<u8>>> {
    thread::spawn(move || {
        let mut output = Vec::new();
        let mut buffer = [0u8; 8192];
        loop {
            let count = match pipe.read(&mut buffer) {
                Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                result => result?,
            };
            if count == 0 {
                break;
            }
            if let Some(tx) = &logs {
                for &byte in &buffer[..count] {
                    if byte == b'\n' {
                        if !send(
                            tx,
                            &cancel,
                            Message::Log(String::from_utf8_lossy(&output).into_owned()),
                        ) {
                            return Ok(Vec::new());
                        }
                        output.clear();
                    } else if output.len() < LOG_LINE_LIMIT {
                        output.push(byte);
                    } else {
                        overflow.store(true, Ordering::Release);
                        return Ok(Vec::new());
                    }
                }
            } else {
                if total
                    .fetch_add(count, Ordering::AcqRel)
                    .saturating_add(count)
                    > OUTPUT_LIMIT
                {
                    overflow.store(true, Ordering::Release);
                    return Ok(output);
                }
                output.extend_from_slice(&buffer[..count]);
            }
        }
        if let Some(tx) = logs {
            if !output.is_empty() {
                send(
                    &tx,
                    &cancel,
                    Message::Log(String::from_utf8_lossy(&output).into_owned()),
                );
            }
            output.clear();
        }
        Ok(output)
    })
}

fn decode_query(query: Query, (status, stdout, stderr): CapturedOutput) -> Result<Value, String> {
    if !status.success() {
        let error = String::from_utf8_lossy(&stderr).trim().to_string();
        return Err(if error.is_empty() {
            format!("Controller query failed ({status})")
        } else {
            error
        });
    }
    let value: Value =
        serde_json::from_slice(&stdout).map_err(|e| format!("Invalid Controller response: {e}"))?;
    let valid = match query {
        Query::Status => {
            value["source"].is_string()
                && value["service"].is_object()
                && value
                    .get("runtime")
                    .is_some_and(|v| v.is_null() || v.is_object())
        }
        Query::Projects => {
            value["projects"].as_array().is_some_and(|rows| {
                rows.iter()
                    .all(|row| row["id"].is_string() && row["path"].is_string())
            }) && value["truncated"].is_boolean()
                && value["project_inventory"]["sync_state"].is_string()
        }
    };
    if !valid {
        return Err("Invalid Controller response: missing overview fields".to_string());
    }
    Ok(value)
}

fn visible(text: &str) -> String {
    text.chars()
        .flat_map(|c| {
            if c.is_control() {
                c.escape_default().collect::<Vec<_>>()
            } else {
                vec![c]
            }
        })
        .collect()
}

#[derive(Default)]
struct Observation {
    value: Option<Value>,
    error: Option<String>,
    pending: bool,
    succeeded: Option<Instant>,
}

impl Observation {
    fn update(&mut self, result: Result<Value, String>) {
        self.pending = false;
        match result {
            Ok(value) => {
                self.value = Some(value);
                self.error = None;
                self.succeeded = Some(Instant::now());
            }
            Err(error) => self.error = Some(visible(&error)),
        }
    }

    fn note(&self) -> String {
        if let Some(error) = &self.error {
            let age = self
                .succeeded
                .map(|at| format!("; last success {}s ago", at.elapsed().as_secs()))
                .unwrap_or_default();
            return format!("Unavailable / stale: {error}{age}");
        }
        if self.pending && self.value.is_none() {
            return "Loading…".to_string();
        }
        self.succeeded
            .map(|at| {
                format!(
                    "Updated {}s ago{}",
                    at.elapsed().as_secs(),
                    if self.pending { " (refreshing)" } else { "" }
                )
            })
            .unwrap_or_else(|| "Not queried".to_string())
    }
}

#[derive(Default)]
struct Overview {
    status: Observation,
    projects: Observation,
    logs: VecDeque<String>,
    log_bytes: usize,
    log_offset: usize,
    project_offset: usize,
    focus_logs: bool,
    follow_logs: bool,
    log_state: String,
    log_rows: usize,
    project_rows: usize,
    evicted: usize,
}

impl Overview {
    fn new() -> Self {
        Self {
            focus_logs: true,
            follow_logs: true,
            log_state: "Connecting…".into(),
            ..Self::default()
        }
    }

    fn log(&mut self, line: &str) {
        let mut line = visible(line);
        if line.len() > LOG_LINE_LIMIT {
            let mut end = LOG_LINE_LIMIT;
            while !line.is_char_boundary(end) {
                end -= 1;
            }
            line.truncate(end);
            line.push_str(" [display truncated at 64 KiB]");
        }
        self.log_bytes += line.len();
        self.logs.push_back(line);
        while self.logs.len() > LOG_LIMIT || self.log_bytes > OUTPUT_LIMIT {
            self.log_bytes -= self.logs.pop_front().unwrap().len();
            self.log_offset = self.log_offset.saturating_sub(1);
            self.evicted += 1;
        }
    }

    fn key(&mut self, key: KeyEvent) -> bool {
        if key.kind == KeyEventKind::Release {
            return false;
        }
        if key.code == KeyCode::Char('q')
            || (key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL))
        {
            return true;
        }
        match key.code {
            KeyCode::Tab | KeyCode::BackTab => self.focus_logs = !self.focus_logs,
            KeyCode::Char(' ') => {
                if self.follow_logs {
                    self.log_offset = self.logs.len().saturating_sub(self.log_rows);
                }
                self.follow_logs = !self.follow_logs;
            }
            KeyCode::Up | KeyCode::Down | KeyCode::PageUp | KeyCode::PageDown => {
                let up = matches!(key.code, KeyCode::Up | KeyCode::PageUp);
                let page = matches!(key.code, KeyCode::PageUp | KeyCode::PageDown);
                let (offset, rows, count) = if self.focus_logs {
                    if self.follow_logs {
                        self.log_offset = self.logs.len().saturating_sub(self.log_rows);
                    }
                    self.follow_logs = false;
                    (&mut self.log_offset, self.log_rows, self.logs.len())
                } else {
                    (
                        &mut self.project_offset,
                        self.project_rows,
                        self.projects
                            .value
                            .as_ref()
                            .and_then(|v| v["projects"].as_array())
                            .map_or(0, Vec::len),
                    )
                };
                let amount = if page { rows.max(1) } else { 1 };
                *offset = if up {
                    offset.saturating_sub(amount)
                } else {
                    offset
                        .saturating_add(amount)
                        .min(count.saturating_sub(rows))
                };
            }
            _ => {}
        }
        false
    }
}

fn field(value: &Value, key: &str) -> String {
    match &value[key] {
        Value::String(text) => visible(text),
        Value::Number(number) => number.to_string(),
        _ => "—".to_string(),
    }
}

fn panel(title: impl Into<String>, focused: bool) -> Block<'static> {
    Block::default()
        .borders(Borders::ALL)
        .title(title.into())
        .border_style(Style::default().fg(if focused {
            Color::Cyan
        } else {
            Color::DarkGray
        }))
}

fn draw(frame: &mut Frame, app: &mut Overview) {
    let area = frame.area();
    if area.width < 80 || area.height < 24 {
        frame.render_widget(
            Paragraph::new(
                "Controller overview requires at least 80×24. Resize the terminal; q/Ctrl+C exits.",
            )
            .wrap(Wrap { trim: true })
            .block(panel("WebCodex Controller", false)),
            area,
        );
        return;
    }
    let areas = Layout::vertical([
        Constraint::Length(5),
        Constraint::Length(6),
        Constraint::Length((area.height / 4).clamp(6, 12)),
        Constraint::Min(6),
        Constraint::Length(1),
    ])
    .split(area);
    let null = Value::Null;
    let status = app.status.value.as_ref().unwrap_or(&null);
    let runtime = &status["runtime"];
    let service = &status["service"];
    let overview = format!(
        "Source: {}   Controller: {}\nService: {}   loaded={} active={} enabled={}\n{}",
        field(status, "source"),
        if runtime.is_object() {
            field(runtime, "controller")
        } else {
            "not reachable".into()
        },
        field(service, "unit"),
        field(service, "loaded"),
        field(service, "active"),
        field(service, "enabled"),
        app.status.note()
    );
    frame.render_widget(
        Paragraph::new(overview).block(panel("WebCodex Controller · read only", false)),
        areas[0],
    );
    let rows = ["server", "runner", "tunnel"].into_iter().map(|name| {
        let component = &runtime[name];
        let phase = if runtime.is_null() {
            "unavailable".into()
        } else {
            field(component, "phase")
        };
        Row::new(vec![
            name.to_string(),
            phase,
            field(component, "pid"),
            field(component, "restart_count"),
        ])
    });
    frame.render_widget(
        Table::new(
            rows,
            [
                Constraint::Length(12),
                Constraint::Min(22),
                Constraint::Length(14),
                Constraint::Length(14),
            ],
        )
        .header(
            Row::new(["Component", "Status", "PID", "Restarts"])
                .style(Style::default().fg(Color::Cyan)),
        )
        .block(
            panel(
                if app.status.error.is_some() {
                    "Components · stale"
                } else {
                    "Components"
                },
                false,
            )
            .title_bottom(
                super::runner_observation_text(runtime)
                    .trim()
                    .replace('\n', " · "),
            ),
        ),
        areas[1],
    );
    draw_projects(frame, app, areas[2]);
    let title = format!(
        "Logs · {} · {}{}",
        app.log_state,
        if app.follow_logs {
            "follow"
        } else {
            "scroll paused"
        },
        if app.evicted > 0 {
            format!(" · {} older lines discarded (cache limit)", app.evicted)
        } else {
            String::new()
        }
    );
    let block = panel(title, app.focus_logs);
    let inner = block.inner(areas[3]);
    app.log_rows = inner.height as usize;
    let max_offset = app.logs.len().saturating_sub(app.log_rows);
    app.log_offset = if app.follow_logs {
        max_offset
    } else {
        app.log_offset.min(max_offset)
    };
    let text = app
        .logs
        .iter()
        .skip(app.log_offset)
        .take(app.log_rows)
        .cloned()
        .collect::<Vec<_>>()
        .join("\n");
    frame.render_widget(block, areas[3]);
    frame.render_widget(Paragraph::new(text), inner);
    frame.render_widget(
        Paragraph::new(
            "Tab focus · ↑↓/PgUp/PgDn scroll · Space follow · r refresh · q/Ctrl+C exit",
        ),
        areas[4],
    );
}

fn draw_projects(frame: &mut Frame, app: &mut Overview, area: Rect) {
    let null = Value::Null;
    let projects = app.projects.value.as_ref().unwrap_or(&null);
    let title = format!(
        "Projects · sync={} · truncated={}{}",
        field(&projects["project_inventory"], "sync_state"),
        projects["truncated"]
            .as_bool()
            .map(|v| if v { "yes" } else { "no" })
            .unwrap_or("unknown"),
        if app.projects.error.is_some() {
            " · stale"
        } else {
            ""
        }
    );
    let block = panel(title, !app.focus_logs);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let parts = Layout::vertical([Constraint::Length(1), Constraint::Min(1)]).split(inner);
    frame.render_widget(Paragraph::new(app.projects.note()), parts[0]);
    let rows = projects["projects"].as_array();
    app.project_rows = parts[1].height.saturating_sub(1) as usize;
    app.project_offset = app
        .project_offset
        .min(rows.map_or(0, Vec::len).saturating_sub(app.project_rows));
    let rows = rows
        .into_iter()
        .flatten()
        .skip(app.project_offset)
        .take(app.project_rows)
        .map(|project| {
            Row::new([
                Cell::from(field(project, "id")),
                Cell::from(field(project, "path")),
            ])
        });
    frame.render_widget(
        Table::new(
            rows,
            [Constraint::Percentage(45), Constraint::Percentage(55)],
        )
        .header(Row::new(["Project ID", "Path"]).style(Style::default().fg(Color::Cyan))),
        parts[1],
    );
}

struct TerminalGuard;
impl Drop for TerminalGuard {
    fn drop(&mut self) {
        ratatui::restore();
    }
}

struct ExitSignals {
    requested: Arc<AtomicBool>,
    registrations: Vec<signal_hook::SigId>,
}

impl ExitSignals {
    fn new() -> Result<Self, String> {
        let mut signals = Self {
            requested: Arc::new(AtomicBool::new(false)),
            registrations: Vec::new(),
        };
        for signal in [signal_hook::consts::SIGINT, signal_hook::consts::SIGTERM] {
            signals.registrations.push(
                signal_hook::flag::register(signal, signals.requested.clone())
                    .map_err(|e| format!("Cannot register TUI exit signal: {e}"))?,
            );
        }
        Ok(signals)
    }
}

impl Drop for ExitSignals {
    fn drop(&mut self) {
        for registration in self.registrations.drain(..) {
            signal_hook::low_level::unregister(registration);
        }
    }
}

pub(super) async fn run(
    config: PathBuf,
    service_file: PathBuf,
    user_token_file: Option<PathBuf>,
) -> Result<(), String> {
    require_terminal(io::stdin().is_terminal(), io::stdout().is_terminal())?;
    let target = Target {
        executable: std::env::current_exe().map_err(|e| e.to_string())?,
        config,
        service_file,
        user_token_file,
    };
    // The native terminal loop uses the same signal-hook registry as Crossterm.
    // Do not depend on Tokio signal delivery while polling native terminal input.
    let signals = ExitSignals::new()?;
    let _terminal_guard = TerminalGuard;
    let mut terminal =
        ratatui::try_init().map_err(|e| format!("Cannot initialize Controller TUI: {e}"))?;
    let (tx, rx) = mpsc::sync_channel(64);
    let mut workers = Workers::new(target, tx);
    let mut app = Overview::new();
    let mut status_due = Instant::now();
    let mut projects_due = Instant::now();
    let mut logs_due = Some(Instant::now());
    let mut logs_active = false;
    loop {
        if signals.requested.load(Ordering::Acquire) {
            return Ok(());
        }
        workers.reap()?;
        // Bound work per frame so busy logs cannot starve keys or signal handling.
        for message in rx.try_iter().take(64) {
            match message {
                Message::Fatal(error) => return Err(visible(&error)),
                Message::Query(Query::Status, result) => app.status.update(result),
                Message::Query(Query::Projects, result) => app.projects.update(result),
                Message::Log(line) => {
                    app.log_state = "connected".into();
                    app.log(&line);
                }
                Message::LogsEnded { error, unsupported } => {
                    logs_active = false;
                    app.log_state = if unsupported {
                        "update required"
                    } else {
                        "disconnected; retrying"
                    }
                    .into();
                    app.log(&format!("--- {error} ---"));
                    logs_due = (!unsupported).then(|| Instant::now() + STATUS_INTERVAL);
                }
            }
        }
        let now = Instant::now();
        if !app.status.pending && now >= status_due {
            app.status.pending = true;
            workers.query(Query::Status);
            status_due = now + STATUS_INTERVAL;
        }
        if !app.projects.pending && now >= projects_due {
            app.projects.pending = true;
            workers.query(Query::Projects);
            projects_due = now + PROJECT_INTERVAL;
        }
        if !logs_active && logs_due.is_some_and(|due| now >= due) {
            app.log_state = "following; waiting for output".into();
            logs_active = true;
            workers.logs();
        }
        terminal
            .draw(|frame| draw(frame, &mut app))
            .map_err(|e| e.to_string())?;
        // Read a bounded batch; releases do not repeat actions and continuous key
        // input cannot postpone cleanup or observations indefinitely.
        for _ in 0..16 {
            if !event::poll(Duration::ZERO).map_err(|e| e.to_string())? {
                break;
            }
            if let Event::Key(key) = event::read().map_err(|e| e.to_string())? {
                if app.key(key) {
                    return Ok(());
                }
                if key.kind != KeyEventKind::Release && key.code == KeyCode::Char('r') {
                    status_due = now;
                    projects_due = now;
                }
            }
        }
        thread::sleep(Duration::from_millis(50));
    }
}

fn require_terminal(stdin: bool, stdout: bool) -> Result<(), String> {
    if stdin && stdout {
        Ok(())
    } else {
        Err("controller tui requires interactive stdin and stdout; use controller status --json, project list --json, or logs -f instead".into())
    }
}

#[cfg(test)]
#[path = "tests/controller_tui.rs"]
mod tests;
