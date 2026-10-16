use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
#[cfg(unix)]
use std::collections::VecDeque;
use std::path::{Path, PathBuf};
#[cfg(unix)]
use std::process::Stdio;
#[cfg(unix)]
use std::sync::{Arc, Mutex};
#[cfg(unix)]
use std::time::{Duration, Instant};
#[cfg(unix)]
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
#[cfg(any(unix, test))]
use tokio::process::Command;
#[cfg(unix)]
use tokio::process::{Child, ChildStdin};
#[cfg(unix)]
use tokio::sync::{broadcast, watch};

#[path = "controller_projects.rs"]
mod projects;

#[cfg(unix)]
#[path = "controller_runner_observation.rs"]
mod runner_observation;
#[path = "controller_runner_query.rs"]
mod runner_query;
#[cfg(unix)]
use runner_observation::RunnerMonitor;

#[cfg(target_os = "linux")]
#[path = "controller_wsl2_nat.rs"]
mod wsl2_nat;
#[cfg(unix)]
#[path = "controller_wsl2_nat_startup.rs"]
mod wsl2_startup;
#[cfg(unix)]
use wsl2_startup::{Wsl2NatSnapshot, Wsl2NatStartup};

#[cfg(unix)]
#[path = "controller_tui.rs"]
mod tui;

#[cfg(unix)]
use tokio::net::{UnixListener, UnixStream};

#[cfg(unix)]
const READY_TIMEOUT: Duration = Duration::from_secs(30);
#[cfg(unix)]
const LOG_LIMIT: usize = 500;
#[cfg(unix)]
const LOG_FOLLOWER_LIMIT: usize = 16;
const CONTROLLER_SERVICE_UNIT: &str = "webcodex-controller.service";

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ControllerCommand {
    Tui {
        config: PathBuf,
        service_file: PathBuf,
        user_token_file: Option<PathBuf>,
    },
    Init {
        config: PathBuf,
        overwrite: bool,
    },
    Run {
        config: PathBuf,
    },
    Install {
        config: PathBuf,
        environment_file: PathBuf,
        service_file: PathBuf,
        overwrite: bool,
        no_start: bool,
    },
    Start {
        config: PathBuf,
        service_file: PathBuf,
    },
    Status {
        config: PathBuf,
        service_file: PathBuf,
        json: bool,
    },
    Doctor {
        config: PathBuf,
        environment_file: PathBuf,
        json: bool,
    },
    Stop {
        config: PathBuf,
        service_file: PathBuf,
    },
    Restart {
        config: PathBuf,
        service_file: PathBuf,
        component: Option<Component>,
    },
    Logs {
        config: PathBuf,
        service_file: PathBuf,
        lines: usize,
        follow: bool,
    },
    Uninstall {
        service_file: PathBuf,
        confirm: bool,
    },
    Project {
        config: PathBuf,
        action: ProjectAction,
        user_token_file: Option<PathBuf>,
        json: bool,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ProjectAction {
    List,
    Register { project: PathBuf },
    Remove { target: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Component {
    Server,
    Runner,
    Tunnel,
}

impl Component {
    fn parse(value: &str) -> Result<Self, String> {
        match value {
            "server" => Ok(Self::Server),
            "runner" => Ok(Self::Runner),
            "tunnel" => Ok(Self::Tunnel),
            _ => Err("component must be one of: server, runner, tunnel".to_string()),
        }
    }
    #[cfg(unix)]
    fn as_str(self) -> &'static str {
        match self {
            Self::Server => "server",
            Self::Runner => "runner",
            Self::Tunnel => "tunnel",
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct ControllerConfig {
    #[serde(default = "config_version")]
    version: u32,
    #[serde(default)]
    controller: ControllerSettings,
    server: ServerSettings,
    runner: RunnerSettings,
    #[serde(default)]
    tunnel: TunnelSettings,
}

fn config_version() -> u32 {
    1
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
struct ControllerSettings {
    restart: bool,
    restart_delay_secs: u64,
    max_restart_attempts: u32,
    runtime_dir: Option<PathBuf>,
}
impl Default for ControllerSettings {
    fn default() -> Self {
        Self {
            restart: true,
            restart_delay_secs: 3,
            max_restart_attempts: 5,
            runtime_dir: None,
        }
    }
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
enum ServerMode {
    #[default]
    Local,
    Remote,
    Wsl2Nat,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
struct ServerSettings {
    mode: ServerMode,
    #[serde(skip_serializing_if = "Option::is_none")]
    env_file: Option<PathBuf>,
    #[serde(skip_serializing_if = "Option::is_none")]
    url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    enabled: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    interface: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    scheme: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    port: Option<u16>,
}
impl Default for ServerSettings {
    fn default() -> Self {
        Self {
            mode: ServerMode::Local,
            env_file: None,
            url: None,
            enabled: None,
            interface: None,
            scheme: None,
            port: None,
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
struct RunnerSettings {
    enabled: bool,
    config: PathBuf,
    #[serde(skip_serializing_if = "Option::is_none")]
    user_token_file: Option<PathBuf>,
}
impl Default for RunnerSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            config: PathBuf::new(),
            user_token_file: None,
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
struct TunnelSettings {
    enabled: bool,
    provider: String,
    server_env_file: Option<PathBuf>,
}
impl Default for TunnelSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            provider: "openai".to_string(),
            server_env_file: None,
        }
    }
}

#[cfg(unix)]
#[derive(Debug, Clone, Serialize)]
struct ComponentSnapshot {
    enabled: bool,
    phase: &'static str,
    pid: Option<u32>,
    restart_count: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    observed_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    observation_error: Option<&'static str>,
}
#[cfg(unix)]
#[derive(Debug, Clone, Serialize)]
struct ControllerSnapshot {
    controller: &'static str,
    server_mode: ServerMode,
    server: ComponentSnapshot,
    runner: ComponentSnapshot,
    tunnel: ComponentSnapshot,
    #[serde(skip_serializing_if = "Option::is_none")]
    wsl2_nat_target: Option<Wsl2NatSnapshot>,
}

#[cfg(unix)]
struct ManagedProcess {
    child: Child,
    stdin: Option<ChildStdin>,
}
#[cfg(unix)]
struct RuntimeComponent {
    enabled: bool,
    phase: &'static str,
    process: Option<ManagedProcess>,
    restart_count: u32,
}
#[cfg(unix)]
impl RuntimeComponent {
    fn new(enabled: bool) -> Self {
        Self {
            enabled,
            phase: if enabled { "stopped" } else { "disabled" },
            process: None,
            restart_count: 0,
        }
    }
    fn snapshot(&self) -> ComponentSnapshot {
        ComponentSnapshot {
            enabled: self.enabled,
            phase: self.phase,
            pid: self.process.as_ref().and_then(|p| p.child.id()),
            restart_count: self.restart_count,
            observed_at: None,
            observation_error: None,
        }
    }
}

#[cfg(unix)]
struct ControllerRuntime {
    config: ControllerConfig,
    server: RuntimeComponent,
    runner: RuntimeComponent,
    tunnel: RuntimeComponent,
    log: Arc<Mutex<ControllerLog>>,
    log_followers: tokio::task::JoinSet<()>,
    shutdown: bool,
    tunnel_ready_rx: Option<watch::Receiver<bool>>,
    wsl2_startup: Wsl2NatStartup,
    runner_monitor: RunnerMonitor,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
struct RunnerConfigView {
    server_url: String,
    client_id: String,
}

fn controller_service_status(service_file: &Path) -> super::service::SystemdStatus {
    let unit = super::service_unit_name(service_file, CONTROLLER_SERVICE_UNIT);
    super::query_systemd_service_status_for_scope(crate::ServiceScope::User, &unit)
}

fn controller_service_loaded(status: &super::service::SystemdStatus) -> bool {
    status.loaded == "loaded"
}

fn uninstall_controller_service(service_file: &Path, confirm: bool) -> Result<String, String> {
    if !confirm {
        return Err(
            "controller uninstall requires --confirm; controller.toml and controller.env are never deleted"
                .to_string(),
        );
    }
    let service_file = absolute_path(service_file)?;
    super::validate_service_file_scope(crate::ServiceScope::User, &service_file)?;
    let unit = super::service_unit_name(&service_file, CONTROLLER_SERVICE_UNIT);
    let result = super::uninstall_unit_for_scope(crate::ServiceScope::User, &service_file, &unit)?;
    Ok(format!(
        "Controller service {}. Controller config and environment files were not deleted.\n",
        if result.removed {
            "uninstalled"
        } else {
            "was already absent"
        }
    ))
}

fn parse_controller_project_command(args: &[String]) -> Result<ControllerCommand, String> {
    let Some(action) = args.first().map(String::as_str) else {
        return Err(
            "Usage: webcodex controller project <list|register|remove> [TARGET] [--config PATH] [--user-token-file PATH] [--json]"
                .to_string(),
        );
    };
    let mut config = None;
    let mut user_token_file = None;
    let mut json = false;
    let mut positional = Vec::new();
    let mut iter = args[1..].iter();
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--config" => {
                config = Some(PathBuf::from(
                    iter.next()
                        .ok_or_else(|| "--config requires PATH".to_string())?,
                ));
            }
            "--user-token-file" => {
                user_token_file =
                    Some(PathBuf::from(iter.next().ok_or_else(|| {
                        "--user-token-file requires PATH".to_string()
                    })?));
            }
            "--json" => json = true,
            value if !value.starts_with('-') => positional.push(value.to_string()),
            other => return Err(format!("unknown controller project option: {other}")),
        }
    }
    let config = config
        .map(Ok)
        .unwrap_or_else(default_controller_config_path)?;
    let action = match action {
        "list" if positional.is_empty() => ProjectAction::List,
        "register" if positional.len() == 1 => ProjectAction::Register {
            project: PathBuf::from(&positional[0]),
        },
        "remove" if positional.len() == 1 => ProjectAction::Remove {
            target: positional.remove(0),
        },
        "list" => return Err("controller project list takes no TARGET".to_string()),
        "register" => {
            return Err("controller project register requires exactly one project PATH".to_string())
        }
        "remove" => {
            return Err(
                "controller project remove requires exactly one project ID or PATH".to_string(),
            )
        }
        other => return Err(format!("unknown controller project action: {other}")),
    };
    Ok(ControllerCommand::Project {
        config,
        action,
        user_token_file,
        json,
    })
}
#[cfg(unix)]
#[derive(Debug, Deserialize)]
struct IpcRequest {
    method: String,
    #[serde(default)]
    component: Option<Component>,
    #[serde(default)]
    lines: Option<usize>,
    #[serde(default)]
    follow: bool,
}

pub(crate) fn default_controller_config_path() -> Result<PathBuf, String> {
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .ok_or_else(|| "HOME is not set; pass --config PATH".to_string())?;
    Ok(home.join(".config/webcodex/controller.toml"))
}

fn default_controller_environment_path() -> Result<PathBuf, String> {
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .ok_or_else(|| "HOME is not set; pass --environment-file PATH".to_string())?;
    Ok(home.join(".config/webcodex/controller.env"))
}

fn default_controller_service_file() -> Result<PathBuf, String> {
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .ok_or_else(|| "HOME is not set; pass --service-file PATH".to_string())?;
    Ok(home.join(".config/systemd/user/webcodex-controller.service"))
}

fn absolute_path(path: &Path) -> Result<PathBuf, String> {
    if path.is_absolute() {
        return Ok(path.to_path_buf());
    }
    std::env::current_dir()
        .map(|cwd| cwd.join(path))
        .map_err(|e| format!("failed to resolve {}: {e}", path.display()))
}

fn render_controller_systemd_unit(
    executable: &Path,
    config: &Path,
    environment_file: &Path,
) -> Result<String, String> {
    let executable = super::encode_exec_program("ExecStart", executable)?;
    let controller = super::encode_exec_argument("ExecStart argument", "controller")?;
    let run = super::encode_exec_argument("ExecStart argument", "run")?;
    let config_flag = super::encode_exec_argument("ExecStart argument", "--config")?;
    let config = super::encode_exec_path_argument("ExecStart --config", config)?;
    let environment_file = super::encode_unit_path_value("EnvironmentFile", environment_file)?;
    Ok(format!(
        "[Unit]\n\
Description=WebCodex Controller\n\
After=network-online.target\n\
Wants=network-online.target\n\n\
[Service]\n\
Type=simple\n\
EnvironmentFile=-{environment_file}\n\
ExecStart={executable} {controller} {run} {config_flag} {config}\n\
Restart=on-failure\n\
RestartSec=3s\n\
KillMode=mixed\n\
TimeoutStopSec=30s\n\
StandardOutput=journal\n\
StandardError=journal\n\n\
[Install]\n\
WantedBy=default.target\n"
    ))
}

fn install_controller_service(
    config: &Path,
    environment_file: &Path,
    service_file: &Path,
    overwrite: bool,
    no_start: bool,
) -> Result<String, String> {
    let config = absolute_path(config)?;
    let environment_file = absolute_path(environment_file)?;
    let service_file = absolute_path(service_file)?;
    let cfg = read_config(&config)?;
    if !no_start {
        reject_existing_service_conflicts(&cfg)?;
    }
    super::validate_service_file_scope(crate::ServiceScope::User, &service_file)?;
    super::ensure_service_file_parent(&service_file)?;
    let executable =
        std::env::current_exe().map_err(|e| format!("cannot resolve webcodex executable: {e}"))?;
    let unit = render_controller_systemd_unit(&executable, &config, &environment_file)?;
    let unit_name = super::service_unit_name(&service_file, CONTROLLER_SERVICE_UNIT);
    let result = super::install_unit_for_scope(
        crate::ServiceScope::User,
        &service_file,
        &unit_name,
        &unit,
        overwrite,
        no_start,
    )?;
    Ok(format!(
        "Controller service installed.\n\n  service file: {}\n  service unit: {}\n  config: {}\n  environment file: {} (optional)\n  started: {}\n",
        service_file.display(),
        result.unit,
        config.display(),
        environment_file.display(),
        if result.started { "yes" } else { "no (--no-start)" }
    ))
}

fn control_controller_service(
    service_file: &Path,
    control: super::ServiceControl,
) -> Result<String, String> {
    let unit = super::service_unit_name(service_file, CONTROLLER_SERVICE_UNIT);
    super::control_service_for_scope(crate::ServiceScope::User, &unit, control)?;
    Ok(format!(
        "Controller service {} completed for {}.\n",
        control.as_str(),
        unit
    ))
}

fn default_runtime_dir() -> Result<PathBuf, String> {
    if let Some(path) = std::env::var_os("XDG_RUNTIME_DIR") {
        return Ok(PathBuf::from(path).join("webcodex"));
    }
    #[cfg(unix)]
    {
        return Ok(PathBuf::from(format!("/tmp/webcodex-{}", unsafe {
            libc::geteuid()
        })));
    }
    #[cfg(not(unix))]
    Err("Controller runtime directory is unavailable on this platform".to_string())
}
fn controller_socket(config: &ControllerConfig) -> Result<PathBuf, String> {
    Ok(config
        .controller
        .runtime_dir
        .clone()
        .unwrap_or(default_runtime_dir()?)
        .join("controller.sock"))
}

impl ServerSettings {
    fn wsl2_interface(&self) -> &str {
        self.interface.as_deref().unwrap_or("eth0")
    }
    fn wsl2_scheme(&self) -> &str {
        self.scheme.as_deref().unwrap_or("http")
    }

    fn is_local(&self) -> bool {
        self.mode == ServerMode::Local
    }

    fn local_env_file(&self) -> Result<&Path, String> {
        if !self.is_local() {
            return Err("remote Server has no local env_file".to_string());
        }
        self.env_file
            .as_deref()
            .filter(|path| !path.as_os_str().is_empty())
            .ok_or_else(|| "server.env_file is required when server.mode=\"local\"".to_string())
    }

    fn remote_url(&self) -> Result<String, String> {
        if self.mode != ServerMode::Remote {
            return Err("local Server has no remote url".to_string());
        }
        let raw = self
            .url
            .as_deref()
            .filter(|value| !value.trim().is_empty())
            .ok_or_else(|| "server.url is required when server.mode=\"remote\"".to_string())?;
        super::connections::canonical_server_url(raw).map(|server| server.url)
    }
}

fn validate_config(cfg: &ControllerConfig) -> Result<(), String> {
    if cfg
        .runner
        .user_token_file
        .as_ref()
        .is_some_and(|path| path.as_os_str().is_empty())
    {
        return Err("runner.user_token_file must not be empty".into());
    }
    if cfg.server.enabled == Some(false) {
        return Err(
            "server.enabled=false is no longer supported; select server.mode=\"local\" or \"remote\""
                .to_string(),
        );
    }
    if cfg.server.mode != ServerMode::Wsl2Nat
        && (cfg.server.interface.is_some()
            || cfg.server.scheme.is_some()
            || cfg.server.port.is_some())
    {
        return Err("server.interface/scheme/port require server.mode=\"wsl2_nat\"".into());
    }
    match cfg.server.mode {
        ServerMode::Local => {
            cfg.server.local_env_file()?;
            if cfg
                .server
                .url
                .as_deref()
                .is_some_and(|url| !url.trim().is_empty())
            {
                return Err("server.url must be omitted when server.mode=\"local\"".to_string());
            }
        }
        ServerMode::Wsl2Nat => {
            if !cfg!(target_os = "linux") {
                return Err("wsl2_nat requires a Linux Runner inside WSL2 NAT".into());
            }
            let interface = cfg.server.wsl2_interface();
            if interface.is_empty()
                || interface.len() > 15
                || interface == "."
                || interface == ".."
                || !interface
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"_.-".contains(&b))
            {
                return Err("server.interface must be a valid Linux interface name".into());
            }
            if !matches!(cfg.server.wsl2_scheme(), "http" | "https") {
                return Err("server.scheme must be http or https".into());
            }
            if cfg.server.port.is_none_or(|port| port == 0) {
                return Err("server.port is required and must be in 1..=65535".into());
            }
            if cfg.server.env_file.is_some() || cfg.server.url.is_some() {
                return Err("server.env_file and server.url must be omitted for wsl2_nat".into());
            }
            if cfg.tunnel.enabled || !cfg.runner.enabled {
                return Err(
                    "wsl2_nat requires runner.enabled=true and tunnel.enabled=false".into(),
                );
            }
        }
        ServerMode::Remote => {
            cfg.server.remote_url()?;
            if cfg
                .server
                .env_file
                .as_ref()
                .is_some_and(|path| !path.as_os_str().is_empty())
            {
                return Err(
                    "server.env_file must be omitted when server.mode=\"remote\"".to_string(),
                );
            }
            if cfg.tunnel.enabled {
                return Err("local Tunnel is not supported when server.mode=\"remote\"".to_string());
            }
        }
    }
    if cfg.runner.enabled && cfg.runner.config.as_os_str().is_empty() {
        return Err("runner.config is required when runner.enabled=true".to_string());
    }
    if cfg.tunnel.enabled && cfg.tunnel.provider != "openai" {
        return Err("Controller supports only tunnel.provider=\"openai\"".to_string());
    }
    if cfg.controller.max_restart_attempts == 0 {
        return Err("controller.max_restart_attempts must be greater than zero".to_string());
    }
    Ok(())
}

fn existing_service_conflicts(config: &ControllerConfig) -> Vec<String> {
    let mut conflicts = Vec::new();
    if config.server.is_local() {
        let service = super::query_systemd_service_status(super::SERVER_SERVICE_UNIT);
        let socket = super::query_systemd_socket_status(super::SERVER_SOCKET_UNIT);
        if service.active == "active" {
            conflicts.push(super::SERVER_SERVICE_UNIT.to_string());
        }
        if socket.active == "active" {
            conflicts.push(super::SERVER_SOCKET_UNIT.to_string());
        }
    }
    if config.runner.enabled {
        for scope in [crate::ServiceScope::User, crate::ServiceScope::System] {
            let status =
                super::query_systemd_service_status_for_scope(scope, super::RUNNER_SERVICE_UNIT);
            if status.active == "active" {
                conflicts.push(format!(
                    "{} ({})",
                    super::RUNNER_SERVICE_UNIT,
                    scope.as_str()
                ));
            }
        }
    }
    conflicts
}

fn reject_existing_service_conflicts(config: &ControllerConfig) -> Result<(), String> {
    let conflicts = existing_service_conflicts(config);
    if conflicts.is_empty() {
        return Ok(());
    }
    Err(format!(
        "Controller refuses to take ownership while existing WebCodex services are active: {}. Stop the existing services first; Controller made no changes.",
        conflicts.join(", ")
    ))
}
fn read_config(path: &Path) -> Result<ControllerConfig, String> {
    let text = std::fs::read_to_string(path)
        .map_err(|e| format!("failed to read Controller config {}: {e}", path.display()))?;
    let cfg: ControllerConfig = toml::from_str(&text)
        .map_err(|e| format!("failed to parse Controller config {}: {e}", path.display()))?;
    if cfg.version != 1 {
        return Err(format!(
            "unsupported Controller config version {}",
            cfg.version
        ));
    }
    validate_config(&cfg)?;
    Ok(cfg)
}

fn server_base_url(env_file: &Path) -> Result<String, String> {
    super::server::derive_regular_tunnel_server_url(env_file)
        .map_err(|error| format!("Controller requires a loopback Server: {error}"))
}

fn configured_server_url(config: &ControllerConfig) -> Result<String, String> {
    match config.server.mode {
        ServerMode::Local => server_base_url(config.server.local_env_file()?),
        ServerMode::Remote => config.server.remote_url(),
        ServerMode::Wsl2Nat => {
            Err("wsl2_nat_target_unresolved: use the active Controller target".into())
        }
    }
}

#[cfg(unix)]
fn server_token(env_file: &Path) -> Result<Option<String>, String> {
    if let Ok(value) = std::env::var("WEBCODEX_TOKEN") {
        if !value.trim().is_empty() {
            return Ok(Some(value));
        }
    }
    super::read_env_file_value(env_file, "WEBCODEX_TOKEN")
}

fn runner_view(path: &Path) -> Result<RunnerConfigView, String> {
    let text = std::fs::read_to_string(path)
        .map_err(|e| format!("failed to read Runner config {}: {e}", path.display()))?;
    toml::from_str(&text)
        .map_err(|_| "runner_config_invalid: could not parse Runner configuration".to_string())
}

fn same_server_origin(left: &str, right: &str) -> bool {
    match (
        super::connections::canonical_server_url(left),
        super::connections::canonical_server_url(right),
    ) {
        (Ok(left), Ok(right)) => left == right,
        _ => false,
    }
}

#[cfg(any(unix, test))]
fn validate_runner_target(
    view: &RunnerConfigView,
    controller_server_url: &str,
) -> Result<(), String> {
    if same_server_origin(&view.server_url, controller_server_url) {
        return Ok(());
    }
    Err("Runner server_url does not match Controller Server".into())
}

fn controller_environment_value(path: &Path, key: &str) -> Result<Option<String>, String> {
    if let Ok(value) = std::env::var(key) {
        if !value.trim().is_empty() {
            return Ok(Some(value));
        }
    }
    if !path.exists() {
        return Ok(None);
    }
    super::read_env_file_value(path, key)
}

fn tunnel_credentials_present(environment_file: &Path) -> Result<bool, String> {
    let tunnel_id = controller_environment_value(environment_file, "CONTROL_PLANE_TUNNEL_ID")?;
    let api_key = controller_environment_value(environment_file, "CONTROL_PLANE_API_KEY")?;
    Ok(tunnel_id.is_some_and(|value| !value.trim().is_empty())
        && api_key.is_some_and(|value| !value.trim().is_empty()))
}

#[cfg(any(unix, test))]
fn remove_controller_tunnel_credentials(command: &mut Command) {
    command
        .env_remove("CONTROL_PLANE_TUNNEL_ID")
        .env_remove("CONTROL_PLANE_API_KEY");
}
#[cfg(unix)]
struct ControllerLog {
    lines: VecDeque<String>,
    updates: broadcast::Sender<String>,
}
#[cfg(unix)]
impl ControllerLog {
    fn new() -> Self {
        Self {
            lines: VecDeque::new(),
            updates: broadcast::channel(LOG_LIMIT).0,
        }
    }
}
#[cfg(unix)]
fn log_line(log: &Arc<Mutex<ControllerLog>>, source: &str, line: impl AsRef<str>) {
    let mut guard = log.lock().unwrap_or_else(|p| p.into_inner());
    let line = format!("[{source}] {}", line.as_ref());
    guard.lines.push_back(line.clone());
    while guard.lines.len() > LOG_LIMIT {
        guard.lines.pop_front();
    }
    let _ = guard.updates.send(line);
}
#[cfg(unix)]
fn spawn_log_reader<R>(
    reader: R,
    source: &'static str,
    log: Arc<Mutex<ControllerLog>>,
    tunnel_tx: Option<watch::Sender<bool>>,
) where
    R: tokio::io::AsyncRead + Unpin + Send + 'static,
{
    tokio::spawn(async move {
        let mut lines = BufReader::new(reader).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            if let Some(tx) = tunnel_tx.as_ref() {
                if let Ok(value) = serde_json::from_str::<Value>(&line) {
                    let ready = value.get("event").and_then(Value::as_str) == Some("ready")
                        || (value.get("event").and_then(Value::as_str) == Some("health")
                            && value.get("tunnel_ready").and_then(Value::as_bool) == Some(true)
                            && value.get("local_mcp_ready").and_then(Value::as_bool) == Some(true));
                    if ready {
                        let _ = tx.send(true);
                    }
                }
            }
            log_line(&log, source, &line);
        }
    });
}
#[cfg(unix)]
fn spawn_process(
    mut command: Command,
    source: &'static str,
    log: Arc<Mutex<ControllerLog>>,
    tunnel_tx: Option<watch::Sender<bool>>,
) -> Result<ManagedProcess, String> {
    let mut child = command
        .spawn()
        .map_err(|e| format!("failed to start {source}: {e}"))?;
    let stdin = child.stdin.take();
    if let Some(stdout) = child.stdout.take() {
        spawn_log_reader(stdout, source, log.clone(), tunnel_tx);
    }
    if let Some(stderr) = child.stderr.take() {
        spawn_log_reader(stderr, source, log, None);
    }
    Ok(ManagedProcess { child, stdin })
}
#[cfg(unix)]
async fn wait_runtime_status(
    base: &str,
    token: Option<&str>,
    predicate: impl Fn(&Value) -> bool,
) -> Result<(), String> {
    let client = reqwest::Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(2))
        .build()
        .map_err(|e| e.to_string())?;
    let deadline = Instant::now() + READY_TIMEOUT;
    while Instant::now() < deadline {
        let mut request = client
            .post(format!("{}/api/runtime/status", base.trim_end_matches('/')))
            .json(&json!({}));
        if let Some(token) = token {
            request = request.bearer_auth(token);
        }
        if let Ok(response) = request.send().await {
            if response.status().is_success() {
                if let Ok(body) = response.json::<Value>().await {
                    let output = body.get("output").unwrap_or(&body);
                    if predicate(output) {
                        return Ok(());
                    }
                }
            }
        }
        tokio::time::sleep(Duration::from_millis(300)).await;
    }
    Err("runtime did not become ready within 30 seconds".to_string())
}

#[cfg(unix)]
async fn wait_remote_server_reachable(base: &str) -> Result<(), String> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(2))
        .build()
        .map_err(|e| e.to_string())?;
    let deadline = Instant::now() + READY_TIMEOUT;
    while Instant::now() < deadline {
        if let Ok(response) = client
            .post(format!("{}/api/runtime/status", base.trim_end_matches('/')))
            .json(&json!({}))
            .send()
            .await
        {
            let status = response.status().as_u16();
            if response.status().is_success() || matches!(status, 401 | 403) {
                return Ok(());
            }
        }
        tokio::time::sleep(Duration::from_millis(300)).await;
    }
    Err(format!(
        "remote Server {base} did not become reachable within 30 seconds"
    ))
}

#[cfg(unix)]
impl ControllerRuntime {
    fn new(config: ControllerConfig) -> Self {
        let mut server = RuntimeComponent::new(config.server.is_local());
        if !config.server.is_local() {
            server.phase = "remote";
        }
        Self {
            server,
            runner: RuntimeComponent::new(config.runner.enabled),
            tunnel: RuntimeComponent::new(config.tunnel.enabled),
            config,
            log: Arc::new(Mutex::new(ControllerLog::new())),
            log_followers: tokio::task::JoinSet::new(),
            shutdown: false,
            tunnel_ready_rx: None,
            wsl2_startup: Wsl2NatStartup::default(),
            runner_monitor: RunnerMonitor::default(),
        }
    }
    fn snapshot(&self) -> ControllerSnapshot {
        let mut runner = self.runner.snapshot();
        self.runner_monitor.decorate(&mut runner, Instant::now());
        ControllerSnapshot {
            controller: if self.shutdown { "stopping" } else { "running" },
            server_mode: self.config.server.mode,
            server: self.server.snapshot(),
            runner,
            tunnel: self.tunnel.snapshot(),
            wsl2_nat_target: (self.config.server.mode == ServerMode::Wsl2Nat)
                .then(|| self.wsl2_startup.snapshot.clone()),
        }
    }
    async fn start_all(&mut self) -> Result<(), String> {
        if self.config.server.mode == ServerMode::Wsl2Nat {
            return self.schedule_wsl2_start();
        }
        if self.config.server.is_local() {
            self.start_component(Component::Server).await?;
        } else {
            self.server.phase = "connecting";
            self.wait_server_ready().await?;
            self.server.phase = "remote_ready";
        }
        if self.runner.enabled {
            self.start_component(Component::Runner).await?;
        }
        if self.tunnel.enabled {
            self.start_component(Component::Tunnel).await?;
        }
        Ok(())
    }
    async fn start_component(&mut self, component: Component) -> Result<(), String> {
        match component {
            Component::Server => {
                if !self.config.server.is_local() {
                    return Err(
                        "remote Server is observed by Controller and cannot be restarted locally"
                            .to_string(),
                    );
                }
                self.stop_component(Component::Server).await;
                self.server.phase = "starting";
                let bin = super::discover_internal_binary("webcodex-server").ok_or_else(|| {
                    "webcodex-server was not found beside webcodex or in PATH".to_string()
                })?;
                let mut command = Command::new(bin);
                let env_file = self.config.server.local_env_file()?;
                command
                    .arg("--stop-on-stdin-eof")
                    .env("WEBCODEX_ENV_FILE", env_file)
                    .stdin(Stdio::piped())
                    .stdout(Stdio::piped())
                    .stderr(Stdio::piped())
                    .kill_on_drop(true);
                remove_controller_tunnel_credentials(&mut command);
                self.server.process =
                    Some(spawn_process(command, "server", self.log.clone(), None)?);
                self.wait_server_ready().await?;
                self.server.phase = "ready";
            }
            Component::Runner => {
                let view = runner_view(&self.config.runner.config)?;
                let server_url = if self.config.server.mode == ServerMode::Wsl2Nat {
                    self.wsl2_startup
                        .snapshot
                        .effective_url
                        .clone()
                        .ok_or("wsl2_nat_target_unresolved")?
                } else {
                    let url = configured_server_url(&self.config)?;
                    validate_runner_target(&view, &url)?;
                    url
                };
                if self.config.server.mode != ServerMode::Wsl2Nat {
                    self.stop_component(Component::Runner).await;
                }
                self.runner.phase = "starting";
                let bin = super::discover_internal_binary("webcodex-runner").ok_or_else(|| {
                    "webcodex-runner was not found beside webcodex or in PATH".to_string()
                })?;
                let mut command = Command::new(bin);
                command
                    .arg("--config")
                    .arg(&self.config.runner.config)
                    .arg("--stop-on-stdin-eof")
                    .stdin(Stdio::piped())
                    .stdout(Stdio::piped())
                    .stderr(Stdio::piped())
                    .kill_on_drop(true);
                if self.config.server.mode == ServerMode::Wsl2Nat {
                    wsl2_startup::configure_runner_command(&mut command, &server_url)?;
                }
                remove_controller_tunnel_credentials(&mut command);
                self.runner.process =
                    Some(spawn_process(command, "runner", self.log.clone(), None)?);
                self.runner_monitor.start(
                    server_url,
                    view,
                    self.wsl2_startup.snapshot.generation.clone(),
                );
                self.poll_runner_observation().await;
            }
            Component::Tunnel => {
                if !self.config.server.is_local() {
                    return Err("local Tunnel cannot be started for a remote Server".to_string());
                }
                self.stop_component(Component::Tunnel).await;
                self.tunnel.phase = "starting";
                let exe = std::env::current_exe()
                    .map_err(|e| format!("cannot resolve webcodex executable: {e}"))?;
                let default_env = self.config.server.local_env_file()?;
                let env_file = self
                    .config
                    .tunnel
                    .server_env_file
                    .as_deref()
                    .unwrap_or(default_env);
                let mut command = Command::new(exe);
                command
                    .args(["server", "tunnel", "--provider", "openai", "--env-file"])
                    .arg(env_file)
                    .args(["--json", "--stop-on-stdin-eof"])
                    .stdin(Stdio::piped())
                    .stdout(Stdio::piped())
                    .stderr(Stdio::piped())
                    .kill_on_drop(true);
                let (tx, rx) = watch::channel(false);
                self.tunnel.process = Some(spawn_process(
                    command,
                    "tunnel",
                    self.log.clone(),
                    Some(tx),
                )?);
                self.tunnel_ready_rx = Some(rx);
                self.wait_tunnel_ready().await?;
                self.tunnel.phase = "ready";
            }
        }
        let phase = match component {
            Component::Server => self.server.phase,
            Component::Runner => self.runner.phase,
            Component::Tunnel => self.tunnel.phase,
        };
        log_line(
            &self.log,
            "controller",
            format!("{} {phase}", component.as_str()),
        );
        Ok(())
    }
    async fn stop_component(&mut self, component: Component) {
        if component == Component::Runner {
            self.runner_monitor.reset();
        }
        if component == Component::Runner && self.config.server.mode == ServerMode::Wsl2Nat {
            self.wsl2_startup.cancel();
        }
        if component == Component::Server && !self.config.server.is_local() {
            self.server.phase = "remote";
            return;
        }
        let target = match component {
            Component::Server => &mut self.server,
            Component::Runner => &mut self.runner,
            Component::Tunnel => &mut self.tunnel,
        };
        if let Some(mut process) = target.process.take() {
            target.phase = "stopping";
            process.stdin.take();
            if tokio::time::timeout(Duration::from_secs(6), process.child.wait())
                .await
                .is_err()
            {
                let _ = process.child.start_kill();
                let _ = process.child.wait().await;
            }
        }
        target.phase = if target.enabled {
            "stopped"
        } else {
            "disabled"
        };
    }
    async fn stop_all(&mut self) {
        self.shutdown = true;
        self.stop_component(Component::Tunnel).await;
        self.stop_component(Component::Runner).await;
        self.stop_component(Component::Server).await;
    }
    async fn restart_component(&mut self, component: Option<Component>) -> Result<(), String> {
        if let Some(component) = component {
            if component == Component::Server && !self.config.server.is_local() {
                return Err(
                    "remote Server is observed by Controller and cannot be restarted locally"
                        .to_string(),
                );
            }
            let enabled = match component {
                Component::Server => self.server.enabled,
                Component::Runner => self.runner.enabled,
                Component::Tunnel => self.tunnel.enabled,
            };
            if !enabled {
                return Err(format!(
                    "{} is disabled by Controller config",
                    component.as_str()
                ));
            }
        }
        match component {
            None | Some(Component::Server) => {
                self.stop_component(Component::Tunnel).await;
                self.stop_component(Component::Runner).await;
                self.stop_component(Component::Server).await;
                self.start_all().await
            }
            Some(Component::Runner) if self.config.server.mode == ServerMode::Wsl2Nat => {
                self.stop_component(Component::Runner).await;
                self.schedule_wsl2_start()
            }
            Some(Component::Runner) => self.start_component(Component::Runner).await,
            Some(Component::Tunnel) => self.start_component(Component::Tunnel).await,
        }
    }
    async fn wait_server_ready(&self) -> Result<(), String> {
        let base = configured_server_url(&self.config)?;
        if self.config.server.is_local() {
            let token = server_token(self.config.server.local_env_file()?)?;
            wait_runtime_status(&base, token.as_deref(), |_| true).await
        } else {
            wait_remote_server_reachable(&base).await
        }
    }
    async fn wait_tunnel_ready(&mut self) -> Result<(), String> {
        let Some(mut rx) = self.tunnel_ready_rx.clone() else {
            return Err("Tunnel readiness channel unavailable".to_string());
        };
        if *rx.borrow() {
            return Ok(());
        }
        tokio::time::timeout(READY_TIMEOUT, async {
            while rx.changed().await.is_ok() {
                if *rx.borrow() {
                    return;
                }
            }
        })
        .await
        .map_err(|_| "Tunnel did not become ready within 30 seconds".to_string())?;
        if *rx.borrow() {
            Ok(())
        } else {
            Err("Tunnel exited before readiness".to_string())
        }
    }
    async fn reap_and_recover(&mut self) {
        if self.config.server.mode == ServerMode::Wsl2Nat {
            self.poll_wsl2_start().await;
        }
        for component in [Component::Server, Component::Runner, Component::Tunnel] {
            let target = match component {
                Component::Server => &mut self.server,
                Component::Runner => &mut self.runner,
                Component::Tunnel => &mut self.tunnel,
            };
            let exited = target
                .process
                .as_mut()
                .and_then(|p| p.child.try_wait().ok().flatten());
            let Some(status) = exited else {
                continue;
            };
            target.process = None;
            target.phase = "failed";
            if component == Component::Runner {
                self.runner_monitor.reset();
            }
            if component == Component::Runner && self.config.server.mode == ServerMode::Wsl2Nat {
                self.wsl2_startup.cancel();
                if matches!(status.code(), Some(2 | 77 | 78)) {
                    self.wsl2_startup.snapshot.error = Some(
                        if status.code() == Some(77) {
                            "runner_auth_failed"
                        } else {
                            "runner_configuration_failed"
                        }
                        .into(),
                    );
                    log_line(
                        &self.log,
                        "controller",
                        self.wsl2_startup
                            .snapshot
                            .error
                            .as_deref()
                            .unwrap_or("runner_failed"),
                    );
                    continue;
                }
            }
            log_line(
                &self.log,
                "controller",
                format!("{} exited unexpectedly: {status}", component.as_str()),
            );
            if self.shutdown
                || !self.config.controller.restart
                || !target.enabled
                || target.restart_count >= self.config.controller.max_restart_attempts
            {
                continue;
            }
            target.restart_count += 1;
            target.phase = "restarting";
            if component == Component::Runner && self.config.server.mode == ServerMode::Wsl2Nat {
                match self.schedule_wsl2_start() {
                    Ok(()) => {
                        self.wsl2_startup.retry_at = Some(
                            Instant::now()
                                + Duration::from_secs(
                                    self.config.controller.restart_delay_secs.min(30),
                                ),
                        )
                    }
                    Err(error) => {
                        log_line(&self.log, "controller", error);
                        self.wsl2_startup.snapshot.error =
                            Some("runner_configuration_failed".into());
                        self.runner.phase = "failed";
                    }
                }
                continue;
            }
            tokio::time::sleep(Duration::from_secs(
                self.config.controller.restart_delay_secs,
            ))
            .await;
            let _ = self
                .restart_component(Some(component))
                .await
                .map_err(|e| log_line(&self.log, "controller", e));
            if component == Component::Server {
                break;
            }
        }
        self.poll_runner_observation().await;
    }
}

fn render_default_config() -> Result<String, String> {
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .ok_or_else(|| "HOME is not set".to_string())?;
    toml::to_string_pretty(&ControllerConfig {
        version: 1,
        controller: ControllerSettings::default(),
        server: ServerSettings {
            mode: ServerMode::Local,
            env_file: Some(home.join(".config/webcodex/webcodex.env")),
            url: None,
            enabled: None,
            interface: None,
            scheme: None,
            port: None,
        },
        runner: RunnerSettings {
            user_token_file: None,
            enabled: true,
            config: home.join(".config/webcodex/runner.toml"),
        },
        tunnel: TunnelSettings::default(),
    })
    .map_err(|e| e.to_string())
}
fn doctor_report(config: &ControllerConfig, environment_file: &Path) -> Result<Value, String> {
    let server_env = config
        .server
        .env_file
        .as_ref()
        .is_some_and(|path| path.is_file());
    let runner_config = config.runner.config.is_file();
    let server_url = configured_server_url(config).ok();
    let server_bin = if config.server.is_local() {
        Some(super::discover_internal_binary("webcodex-server").is_some())
    } else {
        None
    };
    let runner_bin = super::discover_internal_binary("webcodex-runner").is_some();
    let runner_server_matches_controller = if !config.runner.enabled {
        Some(true)
    } else if runner_config {
        runner_view(&config.runner.config).ok().and_then(|view| {
            server_url
                .as_deref()
                .map(|server| same_server_origin(&view.server_url, server))
        })
    } else {
        None
    };
    let tunnel_credentials =
        !config.tunnel.enabled || tunnel_credentials_present(environment_file)?;
    let service_conflicts = existing_service_conflicts(config);
    let server_ok = match config.server.mode {
        ServerMode::Local => server_env && server_url.is_some() && server_bin == Some(true),
        ServerMode::Remote => server_url.is_some(),
        ServerMode::Wsl2Nat => false,
    };
    let ok = server_ok
        && (!config.runner.enabled
            || (runner_config && runner_bin && runner_server_matches_controller == Some(true)))
        && tunnel_credentials
        && service_conflicts.is_empty();
    Ok(json!({
        "ok": ok,
        "server": {
            "mode": config.server.mode,
            "env_file": if config.server.is_local() { json!(server_env) } else { Value::Null },
            "url": server_url,
            "binary": server_bin
        },
        "runner": {
            "enabled":config.runner.enabled,
            "config_file":runner_config,
            "binary":runner_bin,
            "server_matches_controller":runner_server_matches_controller
        },
        "tunnel": {
            "enabled":config.tunnel.enabled,
            "provider":config.tunnel.provider,
            "credentials_present":tunnel_credentials,
            "environment_file": environment_file.is_file()
        },
        "existing_service_conflicts": service_conflicts
    }))
}

fn runner_observation_text(runtime: &Value) -> String {
    let runner = &runtime["runner"];
    let mut text = String::new();
    if let Some(at) = runner["observed_at"].as_str() {
        text.push_str(&format!("Runner observed: {at}\n"));
    }
    if let Some(error) = runner["observation_error"].as_str() {
        text.push_str(&format!("Runner observation: {error}\n"));
    }
    text
}

fn runner_doctor_text(report: &Value) -> String {
    let runner = &report["runner"];
    if runner["enabled"] == false {
        return "disabled".into();
    }
    let configuration = if runner["config_file"] == true
        && runner["binary"] == true
        && runner["server_matches_controller"] == true
    {
        "ok"
    } else {
        "check required"
    };
    format!(
        "{} (configuration: {configuration})",
        runner["phase"].as_str().unwrap_or("unknown")
    )
}

fn wsl2_target_text(report: &Value) -> String {
    let Some(target) = report
        .get("wsl2_nat_target")
        .filter(|value| value.is_object())
    else {
        return String::new();
    };
    format!(
        "WSL2 NAT target: {} ({})\nDiscovery: {} / {}\n{}",
        target["effective_url"].as_str().unwrap_or("unresolved"),
        target["source"].as_str().unwrap_or("unknown"),
        target["resolution_state"].as_str().unwrap_or("unknown"),
        target["probe_state"].as_str().unwrap_or("unknown"),
        target["error"]
            .as_str()
            .map(|error| format!("Error: {error}\n"))
            .unwrap_or_default(),
    )
}

#[cfg(unix)]
fn prepare_runtime_dir(path: &Path) -> Result<(), String> {
    use std::os::unix::fs::{DirBuilderExt, MetadataExt, PermissionsExt};

    if !path.exists() {
        let mut builder = std::fs::DirBuilder::new();
        builder.recursive(true).mode(0o700);
        builder
            .create(path)
            .map_err(|e| format!("failed to create {}: {e}", path.display()))?;
    }
    let metadata = std::fs::symlink_metadata(path)
        .map_err(|e| format!("failed to inspect {}: {e}", path.display()))?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(format!(
            "Controller runtime path {} must be a real directory",
            path.display()
        ));
    }
    if metadata.uid() != unsafe { libc::geteuid() } {
        return Err(format!(
            "Controller runtime directory {} is not owned by the current user",
            path.display()
        ));
    }
    if metadata.permissions().mode() & 0o022 != 0 {
        return Err(format!(
            "Controller runtime directory {} must not be writable by group or others",
            path.display()
        ));
    }
    Ok(())
}

#[cfg(unix)]
fn remove_stale_controller_socket(path: &Path) -> Result<(), String> {
    use std::os::unix::fs::FileTypeExt;

    let metadata = std::fs::symlink_metadata(path).map_err(|e| {
        format!(
            "failed to inspect stale Controller socket {}: {e}",
            path.display()
        )
    })?;
    if !metadata.file_type().is_socket() {
        return Err(format!(
            "refusing to remove non-socket Controller path {}",
            path.display()
        ));
    }
    std::fs::remove_file(path)
        .map_err(|e| format!("failed to remove stale socket {}: {e}", path.display()))
}

#[cfg(unix)]
async fn ipc_call(socket: &Path, request: Value) -> Result<Value, String> {
    let stream = UnixStream::connect(socket)
        .await
        .map_err(|e| format!("Controller is not running at {}: {e}", socket.display()))?;
    ipc_exchange(stream, request).await
}

#[cfg(unix)]
async fn ipc_exchange(mut stream: UnixStream, request: Value) -> Result<Value, String> {
    // Status/control replies are small. A valid 500-line log tail can exceed
    // 64 KiB; preserve a finite larger limit only for that explicit operation.
    // Local restarts may include several child stops and readiness windows.
    let (reply_limit, reply_timeout) = match request["method"].as_str() {
        Some("logs") => (8 * 1024 * 1024usize, Duration::from_secs(3)),
        Some("restart") => (65_536usize, Duration::from_secs(120)),
        _ => (65_536usize, Duration::from_secs(3)),
    };
    stream
        .write_all(
            serde_json::to_string(&request)
                .map_err(|e| e.to_string())?
                .as_bytes(),
        )
        .await
        .map_err(|e| e.to_string())?;
    stream.write_all(b"\n").await.map_err(|e| e.to_string())?;
    let mut line = String::new();
    let mut reader = BufReader::new(stream).take(reply_limit as u64 + 1);
    tokio::time::timeout(reply_timeout, reader.read_line(&mut line))
        .await
        .map_err(|_| "Controller response timeout")?
        .map_err(|e| e.to_string())?;
    if line.len() > reply_limit || !line.ends_with('\n') {
        return Err("invalid or oversized Controller response".into());
    }
    serde_json::from_str(&line).map_err(|e| format!("invalid Controller response: {e}"))
}
#[cfg(unix)]
async fn write_log_response<W: tokio::io::AsyncWrite + Unpin>(
    write: &mut W,
    value: Value,
) -> Result<(), String> {
    let mut bytes = serde_json::to_vec(&value).map_err(|e| e.to_string())?;
    bytes.push(b'\n');
    write.write_all(&bytes).await.map_err(|e| e.to_string())
}

#[cfg(unix)]
async fn stream_log_updates<W: tokio::io::AsyncWrite + Unpin>(
    write: &mut W,
    initial: Vec<String>,
    mut updates: broadcast::Receiver<String>,
) -> Result<(), String> {
    write_log_response(
        write,
        json!({"ok":true,"result":{"lines":initial,"following":true}}),
    )
    .await?;
    loop {
        match updates.recv().await {
            Ok(line) => {
                write_log_response(write, json!({"ok":true,"result":{"lines":[line]}})).await?;
            }
            Err(broadcast::error::RecvError::Lagged(_)) => {
                return write_log_response(
                    write,
                    json!({"ok":false,"error":"Controller log follower fell behind; rerun controller logs -f to read the latest logs"}),
                )
                .await;
            }
            Err(broadcast::error::RecvError::Closed) => return Ok(()),
        }
    }
}

#[cfg(unix)]
async fn follow_controller_logs<W: tokio::io::AsyncWrite + Unpin>(
    mut stream: UnixStream,
    lines: usize,
    output: &mut W,
) -> Result<(), String> {
    write_log_response(
        &mut stream,
        json!({"method":"logs","lines":lines,"follow":true}),
    )
    .await?;
    let mut reader = BufReader::new(stream);
    let mut first = true;
    loop {
        let mut line = String::new();
        if reader
            .read_line(&mut line)
            .await
            .map_err(|e| e.to_string())?
            == 0
        {
            return Err("Controller log stream disconnected; rerun controller logs -f after the Controller is available".to_string());
        }
        let value: Value = serde_json::from_str(&line)
            .map_err(|e| format!("invalid Controller log response: {e}"))?;
        if value["ok"] != true {
            return Err(value["error"]
                .as_str()
                .unwrap_or("Controller log request failed")
                .to_string());
        }
        if first && value["result"]["following"] != true {
            return Err("The running Controller does not support log following; restart it with the updated binary".to_string());
        }
        first = false;
        let items = value["result"]["lines"]
            .as_array()
            .ok_or_else(|| "invalid Controller log response: missing lines".to_string())?;
        for item in items {
            let line = item
                .as_str()
                .ok_or_else(|| "invalid Controller log response: expected a string".to_string())?;
            output
                .write_all(line.as_bytes())
                .await
                .map_err(|e| e.to_string())?;
            output.write_all(b"\n").await.map_err(|e| e.to_string())?;
        }
        output.flush().await.map_err(|e| e.to_string())?;
    }
}

#[cfg(unix)]
async fn handle_ipc(runtime: &mut ControllerRuntime, stream: UnixStream) -> Result<(), String> {
    if stream
        .peer_cred()
        .map_err(|_| "Controller peer unavailable")?
        .uid()
        != unsafe { libc::geteuid() }
    {
        return Err("Controller IPC requires the same user".into());
    }
    let (read, mut write) = stream.into_split();
    let mut line = String::new();
    let mut read = BufReader::new(read).take(8193);
    tokio::time::timeout(Duration::from_secs(1), read.read_line(&mut line))
        .await
        .map_err(|_| "Controller request timeout")?
        .map_err(|e| e.to_string())?;
    if line.len() > 8192 || !line.ends_with('\n') {
        return Err("invalid or oversized Controller request".into());
    }
    let req: IpcRequest =
        serde_json::from_str(&line).map_err(|e| format!("invalid Controller request: {e}"))?;
    let result = match req.method.as_str() {
        "status" => serde_json::to_value(runtime.snapshot()).map_err(|e| e.to_string())?,
        "stop" => {
            runtime.shutdown = true;
            json!({"stopping":true})
        }
        "restart" => {
            runtime.restart_component(req.component).await?;
            serde_json::to_value(runtime.snapshot()).map_err(|e| e.to_string())?
        }
        "logs" => {
            let lines = req.lines.unwrap_or(100).min(LOG_LIMIT);
            // Subscribe while holding the same lock as log_line so the tail/update
            // boundary cannot duplicate or lose an entry.
            let (items, updates) = {
                let guard = runtime.log.lock().unwrap_or_else(|p| p.into_inner());
                let mut items: Vec<_> = guard.lines.iter().rev().take(lines).cloned().collect();
                items.reverse();
                (items, req.follow.then(|| guard.updates.subscribe()))
            };
            if let Some(updates) = updates {
                while runtime.log_followers.try_join_next().is_some() {}
                if runtime.log_followers.len() >= LOG_FOLLOWER_LIMIT {
                    write_log_response(
                        &mut write,
                        json!({"ok":false,"error":"Controller log follower limit reached"}),
                    )
                    .await?;
                    return Ok(());
                }
                runtime.log_followers.spawn(async move {
                    tokio::select! {
                        _ = read.read_u8() => {},
                        _ = stream_log_updates(&mut write, items, updates) => {},
                    }
                });
                return Ok(());
            }
            json!({"lines":items})
        }
        other => return Err(format!("unknown Controller method: {other}")),
    };
    let response =
        serde_json::to_string(&json!({"ok":true,"result":result})).map_err(|e| e.to_string())?;
    write
        .write_all(response.as_bytes())
        .await
        .map_err(|e| e.to_string())?;
    write.write_all(b"\n").await.map_err(|e| e.to_string())?;
    Ok(())
}
#[cfg(unix)]
async fn run_controller(config_path: &Path) -> Result<(), String> {
    let config = read_config(config_path)?;
    reject_existing_service_conflicts(&config)?;
    let socket = controller_socket(&config)?;
    let runtime_dir = socket.parent().ok_or_else(|| {
        format!(
            "Controller socket {} has no parent directory",
            socket.display()
        )
    })?;
    prepare_runtime_dir(runtime_dir)?;
    if socket.exists() {
        if UnixStream::connect(&socket).await.is_ok() {
            return Err(format!(
                "Controller is already running at {}",
                socket.display()
            ));
        }
        remove_stale_controller_socket(&socket)?;
    }
    let listener = UnixListener::bind(&socket)
        .map_err(|e| format!("failed to bind {}: {e}", socket.display()))?;
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&socket, std::fs::Permissions::from_mode(0o600))
            .map_err(|e| format!("failed to secure {}: {e}", socket.display()))?;
    }
    let mut runtime = ControllerRuntime::new(config);
    log_line(&runtime.log, "controller", "starting");
    if let Err(error) = runtime.start_all().await {
        runtime.stop_all().await;
        let _ = std::fs::remove_file(&socket);
        return Err(error);
    }
    log_line(&runtime.log, "controller", "ready");
    println!("WebCodex Controller is ready.");
    println!("Control socket: {}", socket.display());
    let mut tick = tokio::time::interval(Duration::from_secs(1));
    let mut terminate = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        .map_err(|e| format!("failed to register SIGTERM handler: {e}"))?;
    loop {
        tokio::select! {
            _ = tokio::signal::ctrl_c() => runtime.shutdown = true,
            _ = terminate.recv() => runtime.shutdown = true,
            _ = tick.tick() => runtime.reap_and_recover().await,
            accepted = listener.accept() => match accepted {
                Ok((stream, _)) => if let Err(error) = handle_ipc(&mut runtime, stream).await { log_line(&runtime.log, "controller", error); },
                Err(error) => log_line(&runtime.log, "controller", format!("IPC accept failed: {error}")),
            }
        }
        if runtime.shutdown {
            break;
        }
    }
    runtime.log_followers.abort_all();
    runtime.stop_all().await;
    let _ = std::fs::remove_file(&socket);
    Ok(())
}

fn parse_controller_tui_command(args: &[String]) -> Result<ControllerCommand, String> {
    let mut config = None;
    let mut service_file = None;
    let mut user_token_file = None;
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "-h" | "--help" => return Err(super::controller_usage().to_string()),
            "--config" | "--service-file" | "--user-token-file" => {
                let value =
                    PathBuf::from(iter.next().ok_or_else(|| format!("{arg} requires PATH"))?);
                match arg.as_str() {
                    "--config" => config = Some(value),
                    "--service-file" => service_file = Some(value),
                    _ => user_token_file = Some(value),
                }
            }
            other => return Err(format!("unknown controller tui option: {other}")),
        }
    }
    Ok(ControllerCommand::Tui {
        config: config
            .map(Ok)
            .unwrap_or_else(default_controller_config_path)?,
        service_file: service_file
            .map(Ok)
            .unwrap_or_else(default_controller_service_file)?,
        user_token_file,
    })
}

pub(crate) fn parse_controller_command(args: &[String]) -> Result<ControllerCommand, String> {
    let Some(command) = args.first().map(String::as_str) else {
        return Err(super::controller_usage().to_string());
    };
    if matches!(command, "--help" | "-h") {
        return Err(super::controller_usage().to_string());
    }
    if command == "project" {
        return parse_controller_project_command(&args[1..]);
    }
    if command == "tui" {
        return parse_controller_tui_command(&args[1..]);
    }
    let mut config = None;
    let mut environment_file = None;
    let mut service_file = None;
    let mut overwrite = false;
    let mut no_start = false;
    let mut confirm = false;
    let mut json = false;
    let mut component = None;
    let mut lines = 100usize;
    let mut follow = false;
    let mut iter = args[1..].iter();
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--config" => {
                config = Some(PathBuf::from(
                    iter.next()
                        .ok_or_else(|| "--config requires PATH".to_string())?,
                ))
            }
            "--environment-file" => {
                environment_file =
                    Some(PathBuf::from(iter.next().ok_or_else(|| {
                        "--environment-file requires PATH".to_string()
                    })?))
            }
            "--service-file" => {
                service_file = Some(PathBuf::from(
                    iter.next()
                        .ok_or_else(|| "--service-file requires PATH".to_string())?,
                ))
            }
            "--overwrite" => overwrite = true,
            "--no-start" => no_start = true,
            "--confirm" => confirm = true,
            "--json" => json = true,
            "-f" | "--follow" if command == "logs" => follow = true,
            "--lines" => {
                lines = iter
                    .next()
                    .ok_or_else(|| "--lines requires N".to_string())?
                    .parse()
                    .map_err(|_| "--lines must be a positive integer".to_string())?;
                if lines == 0 {
                    return Err("--lines must be greater than zero".to_string());
                }
            }
            value if command == "restart" && component.is_none() && !value.starts_with('-') => {
                component = Some(Component::parse(value)?)
            }
            other => return Err(format!("unknown controller {command} option: {other}")),
        }
    }
    let resolve_config = || {
        config
            .clone()
            .map(Ok)
            .unwrap_or_else(default_controller_config_path)
    };
    let resolve_environment_file = || {
        environment_file
            .clone()
            .map(Ok)
            .unwrap_or_else(default_controller_environment_path)
    };
    let resolve_service_file = || {
        service_file
            .clone()
            .map(Ok)
            .unwrap_or_else(default_controller_service_file)
    };
    match command {
        "init" => Ok(ControllerCommand::Init {
            config: resolve_config()?,
            overwrite,
        }),
        "run" => Ok(ControllerCommand::Run {
            config: resolve_config()?,
        }),
        "install" => Ok(ControllerCommand::Install {
            config: resolve_config()?,
            environment_file: resolve_environment_file()?,
            service_file: resolve_service_file()?,
            overwrite,
            no_start,
        }),
        "start" => Ok(ControllerCommand::Start {
            config: resolve_config()?,
            service_file: resolve_service_file()?,
        }),
        "status" => Ok(ControllerCommand::Status {
            config: resolve_config()?,
            service_file: resolve_service_file()?,
            json,
        }),
        "doctor" => Ok(ControllerCommand::Doctor {
            config: resolve_config()?,
            environment_file: resolve_environment_file()?,
            json,
        }),
        "stop" => Ok(ControllerCommand::Stop {
            config: resolve_config()?,
            service_file: resolve_service_file()?,
        }),
        "restart" => Ok(ControllerCommand::Restart {
            config: resolve_config()?,
            service_file: resolve_service_file()?,
            component,
        }),
        "logs" => Ok(ControllerCommand::Logs {
            config: resolve_config()?,
            service_file: resolve_service_file()?,
            lines,
            follow,
        }),
        "uninstall" => Ok(ControllerCommand::Uninstall {
            service_file: resolve_service_file()?,
            confirm,
        }),
        other => Err(format!(
            "unknown controller subcommand: {other}\n\n{}",
            super::controller_usage()
        )),
    }
}

pub(crate) async fn run_controller_command(command: ControllerCommand) -> Result<String, String> {
    match command {
        ControllerCommand::Tui {
            config,
            service_file,
            user_token_file,
        } => {
            #[cfg(unix)]
            {
                tui::run(config, service_file, user_token_file).await?;
                Ok(String::new())
            }
            #[cfg(not(unix))]
            {
                let _ = (config, service_file, user_token_file);
                Err(
                    "webcodex controller tui is currently supported only on WSL/Linux/Unix"
                        .to_string(),
                )
            }
        }
        ControllerCommand::Init { config, overwrite } => {
            if config.exists() && !overwrite {
                return Err(format!(
                    "{} already exists; pass --overwrite to replace it",
                    config.display()
                ));
            }
            if let Some(parent) = config.parent() {
                std::fs::create_dir_all(parent)
                    .map_err(|e| format!("failed to create {}: {e}", parent.display()))?;
            }
            std::fs::write(&config, render_default_config()?)
                .map_err(|e| format!("failed to write {}: {e}", config.display()))?;
            Ok(format!(
                "Controller configuration written to {}\n",
                config.display()
            ))
        }
        ControllerCommand::Doctor {
            config,
            environment_file,
            json: as_json,
        } => {
            let cfg = read_config(&config)?;
            let report = doctor_report(&cfg, &environment_file)?;
            #[cfg(unix)]
            let report = {
                let mut report = report;
                if cfg.server.mode == ServerMode::Wsl2Nat {
                    wsl2_startup::doctor(&cfg, &mut report).await?;
                } else {
                    let runtime = runner_observation::live_snapshot(&cfg).await?;
                    runner_observation::attach_runtime(&mut report, runtime);
                }
                report
            };
            if as_json {
                return serde_json::to_string_pretty(&report).map_err(|e| e.to_string());
            }
            Ok(format!(
                "Controller doctor: {}\nServer: {}\nRunner: {}\nTunnel: {}\n{}",
                if report["ok"].as_bool() == Some(true) {
                    "PASS"
                } else {
                    "FAIL"
                },
                match report["server"]["mode"].as_str() {
                    Some("wsl2_nat") if report["server"]["url"].as_str().is_some() => "wsl2_nat",
                    Some("remote") if report["server"]["url"].as_str().is_some() => "remote",
                    Some("local")
                        if report["server"]["env_file"].as_bool() == Some(true)
                            && report["server"]["binary"].as_bool() == Some(true) =>
                    {
                        "local"
                    }
                    _ => "check required",
                },
                runner_doctor_text(&report),
                if report["tunnel"]["enabled"].as_bool() == Some(false) {
                    "disabled"
                } else if report["tunnel"]["credentials_present"].as_bool() == Some(true) {
                    "ok"
                } else {
                    "credentials missing"
                },
                format!(
                    "{}{}",
                    wsl2_target_text(&report),
                    runner_observation_text(&report["runtime"])
                ),
            ))
        }
        ControllerCommand::Run { config } => {
            #[cfg(unix)]
            {
                run_controller(&config).await?;
                Ok(String::new())
            }
            #[cfg(not(unix))]
            {
                let _ = config;
                Err("webcodex controller is currently supported only on WSL/Linux/Unix".to_string())
            }
        }
        ControllerCommand::Install {
            config,
            environment_file,
            service_file,
            overwrite,
            no_start,
        } => install_controller_service(
            &config,
            &environment_file,
            &service_file,
            overwrite,
            no_start,
        ),
        ControllerCommand::Start {
            config,
            service_file,
        } => {
            let cfg = read_config(&config)?;
            reject_existing_service_conflicts(&cfg)?;
            control_controller_service(&service_file, super::ServiceControl::Start)
        }
        ControllerCommand::Status {
            config,
            service_file,
            json: as_json,
        } => {
            let service = controller_service_status(&service_file);
            #[cfg(unix)]
            {
                if let Ok(cfg) = read_config(&config) {
                    let socket = controller_socket(&cfg)?;
                    if let Ok(value) = ipc_call(&socket, json!({"method":"status"})).await {
                        let result = &value["result"];
                        let source = if service.active == "active" {
                            "systemd"
                        } else {
                            "foreground"
                        };
                        if as_json {
                            return serde_json::to_string_pretty(&json!({
                                "source": source,
                                "runtime": result,
                                "service": {
                                    "unit": super::service_unit_name(&service_file, CONTROLLER_SERVICE_UNIT),
                                    "loaded": service.loaded,
                                    "active": service.active,
                                    "enabled": service.enabled,
                                }
                            }))
                            .map_err(|e| e.to_string());
                        }
                        return Ok(format!(
                            "Controller: {}\nMode: {}\nServer: {}\nRunner: {}\nTunnel: {}\nService: {} / {}\n{}",
                            result["controller"].as_str().unwrap_or("unknown"),
                            source,
                            result["server"]["phase"].as_str().unwrap_or("unknown"),
                            result["runner"]["phase"].as_str().unwrap_or("unknown"),
                            result["tunnel"]["phase"].as_str().unwrap_or("unknown"),
                            service.active,
                            service.enabled,
                            format!("{}{}", wsl2_target_text(result), runner_observation_text(result)),
                        ));
                    }
                }
                if as_json {
                    return serde_json::to_string_pretty(&json!({
                        "source": "systemd",
                        "runtime": Value::Null,
                        "service": {
                            "unit": super::service_unit_name(&service_file, CONTROLLER_SERVICE_UNIT),
                            "loaded": service.loaded,
                            "active": service.active,
                            "enabled": service.enabled,
                        }
                    }))
                    .map_err(|e| e.to_string());
                }
                Ok(format!(
                    "Controller: not reachable\nService: {} / {} / {}\n",
                    service.loaded, service.active, service.enabled
                ))
            }
            #[cfg(not(unix))]
            {
                let _ = (config, service_file, service, as_json);
                Err("webcodex controller is currently supported only on WSL/Linux/Unix".to_string())
            }
        }
        ControllerCommand::Stop {
            config,
            service_file,
        } => {
            #[cfg(not(unix))]
            let _ = config;
            #[cfg(unix)]
            {
                if let Ok(cfg) = read_config(&config) {
                    let socket = controller_socket(&cfg)?;
                    if ipc_call(&socket, json!({"method":"stop"})).await.is_ok() {
                        return Ok(
                            "Controller stop requested through the local control socket.\n"
                                .to_string(),
                        );
                    }
                }
            }
            let status = controller_service_status(&service_file);
            if controller_service_loaded(&status) {
                return control_controller_service(&service_file, super::ServiceControl::Stop);
            }
            Err("Controller is not running and no systemd user service is installed".to_string())
        }
        ControllerCommand::Restart {
            config,
            service_file,
            component,
        } => {
            if component.is_none() {
                let status = controller_service_status(&service_file);
                if controller_service_loaded(&status) || status.active == "active" {
                    return control_controller_service(
                        &service_file,
                        super::ServiceControl::Restart,
                    );
                }
                let cfg = read_config(&config)?;
                let socket = controller_socket(&cfg)?;
                #[cfg(unix)]
                {
                    ipc_call(&socket, json!({"method":"restart","component":Value::Null})).await?;
                    return Ok("Controller foreground runtime restart completed.\n".to_string());
                }
                #[cfg(not(unix))]
                {
                    let _ = socket;
                    return Err(
                        "Controller restart requires an installed systemd service on this platform"
                            .to_string(),
                    );
                }
            }
            let cfg = read_config(&config)?;
            let socket = controller_socket(&cfg)?;
            #[cfg(unix)]
            {
                let _ =
                    ipc_call(&socket, json!({"method":"restart","component":component})).await?;
                Ok(format!(
                    "Controller restart completed for {}.\n",
                    component.map(Component::as_str).unwrap_or("runtime")
                ))
            }
            #[cfg(not(unix))]
            {
                let _ = (socket, component);
                Err("webcodex controller is currently supported only on WSL/Linux/Unix".to_string())
            }
        }
        ControllerCommand::Logs {
            config,
            service_file,
            lines,
            follow,
        } => {
            #[cfg(unix)]
            {
                if let Ok(cfg) = read_config(&config) {
                    let socket = controller_socket(&cfg)?;
                    if follow {
                        if let Ok(stream) = UnixStream::connect(&socket).await {
                            let mut stdout = tokio::io::stdout();
                            tokio::select! {
                                result = follow_controller_logs(stream, lines, &mut stdout) => result?,
                                result = tokio::signal::ctrl_c() => result.map_err(|e| format!("failed to wait for Ctrl+C: {e}"))?,
                            }
                            return Ok(String::new());
                        }
                    } else if let Ok(value) =
                        ipc_call(&socket, json!({"method":"logs","lines":lines})).await
                    {
                        let mut out = String::new();
                        for item in value["result"]["lines"]
                            .as_array()
                            .cloned()
                            .unwrap_or_default()
                        {
                            if let Some(line) = item.as_str() {
                                out.push_str(line);
                                out.push('\n');
                            }
                        }
                        return Ok(out);
                    }
                }
                let status = controller_service_status(&service_file);
                if controller_service_loaded(&status) {
                    let unit = super::service_unit_name(&service_file, CONTROLLER_SERVICE_UNIT);
                    return super::run_logs_for_scope(
                        crate::ServiceScope::User,
                        &unit,
                        lines.min(u32::MAX as usize) as u32,
                        None,
                        follow,
                    );
                }
                Err(
                    "Controller is not running and no systemd user service is installed"
                        .to_string(),
                )
            }
            #[cfg(not(unix))]
            {
                let _ = (config, service_file, lines, follow);
                Err("webcodex controller is currently supported only on WSL/Linux/Unix".to_string())
            }
        }
        ControllerCommand::Uninstall {
            service_file,
            confirm,
        } => uninstall_controller_service(&service_file, confirm),
        ControllerCommand::Project {
            config,
            action,
            user_token_file,
            json: as_json,
        } => {
            let cfg =
                read_config(&config).map_err(|error| projects::format_error(error, as_json))?;
            projects::run(&cfg, action, user_token_file.as_deref(), as_json).await
        }
    }
}

#[cfg(test)]
#[path = "tests/controller_logs.rs"]
mod logs_tests;

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parses_restart_component() {
        let cmd = parse_controller_command(&[
            "restart".into(),
            "tunnel".into(),
            "--config".into(),
            "/tmp/controller.toml".into(),
            "--service-file".into(),
            "/tmp/webcodex-controller.service".into(),
        ])
        .unwrap();
        assert_eq!(
            cmd,
            ControllerCommand::Restart {
                config: PathBuf::from("/tmp/controller.toml"),
                service_file: PathBuf::from("/tmp/webcodex-controller.service"),
                component: Some(Component::Tunnel)
            }
        );
    }

    #[cfg(unix)]
    #[test]
    fn controller_service_unit_runs_controller_with_exact_config() {
        let unit = render_controller_systemd_unit(
            Path::new("/opt/web codex/webcodex"),
            Path::new("/home/user/.config/webcodex/controller.toml"),
            Path::new("/home/user/.config/webcodex/controller.env"),
        )
        .unwrap();
        assert!(unit.contains("Description=WebCodex Controller"));
        assert!(unit.contains("controller"));
        assert!(unit.contains("run"));
        assert!(unit.contains("--config"));
        assert!(unit.contains("controller.toml"));
        assert!(unit.contains("EnvironmentFile=-"));
        assert!(unit.contains("KillMode=mixed"));
        assert!(unit.contains("TimeoutStopSec=30s"));
        assert!(unit.contains("WantedBy=default.target"));
    }
    #[test]
    fn config_rejects_non_openai_tunnel() {
        let cfg = ControllerConfig {
            version: 1,
            controller: ControllerSettings::default(),
            server: ServerSettings {
                mode: ServerMode::Local,
                env_file: Some("/tmp/server.env".into()),
                url: None,
                enabled: None,
                interface: None,
                scheme: None,
                port: None,
            },
            runner: RunnerSettings {
                user_token_file: None,
                enabled: false,
                config: PathBuf::new(),
            },
            tunnel: TunnelSettings {
                enabled: true,
                provider: "cloudflare".into(),
                server_env_file: None,
            },
        };
        assert!(validate_config(&cfg).unwrap_err().contains("openai"));
    }

    #[test]
    fn config_rejects_local_tunnel_for_remote_server() {
        let cfg = ControllerConfig {
            version: 1,
            controller: ControllerSettings::default(),
            server: ServerSettings {
                mode: ServerMode::Remote,
                env_file: None,
                url: Some("https://runtime.example.test".into()),
                enabled: None,
                interface: None,
                scheme: None,
                port: None,
            },
            runner: RunnerSettings {
                user_token_file: None,
                enabled: true,
                config: "/tmp/runner.toml".into(),
            },
            tunnel: TunnelSettings {
                enabled: true,
                provider: "openai".into(),
                server_env_file: None,
            },
        };
        assert!(validate_config(&cfg).unwrap_err().contains("remote"));
    }

    #[test]
    fn runner_target_must_match_controller_local_server() {
        let matching = RunnerConfigView {
            server_url: "http://127.0.0.1:8080/".into(),
            client_id: "special".into(),
        };
        validate_runner_target(&matching, "http://127.0.0.1:8080").unwrap();

        let canonical_remote = RunnerConfigView {
            server_url: "https://RUNTIME.example.test:443/".into(),
            client_id: "special".into(),
        };
        validate_runner_target(&canonical_remote, "https://runtime.example.test").unwrap();

        let remote = RunnerConfigView {
            server_url: "https://runtime.example.test".into(),
            client_id: "special".into(),
        };
        assert!(validate_runner_target(&remote, "http://127.0.0.1:8080")
            .unwrap_err()
            .contains("does not match"));
    }

    #[test]
    fn doctor_parser_preserves_controller_environment_file() {
        let cmd = parse_controller_command(&[
            "doctor".into(),
            "--config".into(),
            "/tmp/controller.toml".into(),
            "--environment-file".into(),
            "/tmp/controller.env".into(),
            "--json".into(),
        ])
        .unwrap();
        assert_eq!(
            cmd,
            ControllerCommand::Doctor {
                config: PathBuf::from("/tmp/controller.toml"),
                environment_file: PathBuf::from("/tmp/controller.env"),
                json: true,
            }
        );
    }

    #[test]
    fn start_parser_preserves_config_for_conflict_preflight() {
        let cmd = parse_controller_command(&[
            "start".into(),
            "--config".into(),
            "/tmp/controller.toml".into(),
            "--service-file".into(),
            "/tmp/webcodex-controller.service".into(),
        ])
        .unwrap();
        assert_eq!(
            cmd,
            ControllerCommand::Start {
                config: PathBuf::from("/tmp/controller.toml"),
                service_file: PathBuf::from("/tmp/webcodex-controller.service"),
            }
        );
    }

    #[test]
    fn server_and_runner_children_drop_tunnel_control_credentials() {
        let mut command = Command::new("webcodex-child");
        command
            .env("CONTROL_PLANE_TUNNEL_ID", "tunnel_fixture")
            .env("CONTROL_PLANE_API_KEY", "secret-fixture");
        remove_controller_tunnel_credentials(&mut command);
        let env = command
            .as_std()
            .get_envs()
            .map(|(key, value)| {
                (
                    key.to_string_lossy().to_string(),
                    value.map(|value| value.to_string_lossy().to_string()),
                )
            })
            .collect::<std::collections::BTreeMap<_, _>>();
        assert_eq!(env.get("CONTROL_PLANE_TUNNEL_ID"), Some(&None));
        assert_eq!(env.get("CONTROL_PLANE_API_KEY"), Some(&None));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn component_restart_rejects_disabled_component() {
        let cfg = ControllerConfig {
            version: 1,
            controller: ControllerSettings::default(),
            server: ServerSettings {
                mode: ServerMode::Local,
                env_file: Some("/tmp/server.env".into()),
                url: None,
                enabled: None,
                interface: None,
                scheme: None,
                port: None,
            },
            runner: RunnerSettings {
                user_token_file: None,
                enabled: false,
                config: PathBuf::new(),
            },
            tunnel: TunnelSettings::default(),
        };
        let mut runtime = ControllerRuntime::new(cfg);
        assert!(runtime
            .restart_component(Some(Component::Tunnel))
            .await
            .unwrap_err()
            .contains("disabled"));
    }

    #[cfg(unix)]
    #[test]
    fn runtime_dir_rejects_group_or_world_writable_directory() {
        use std::os::unix::fs::PermissionsExt;

        let temp = tempfile::tempdir().unwrap();
        let runtime = temp.path().join("runtime");
        std::fs::create_dir(&runtime).unwrap();
        std::fs::set_permissions(&runtime, std::fs::Permissions::from_mode(0o777)).unwrap();
        assert!(prepare_runtime_dir(&runtime)
            .unwrap_err()
            .contains("must not be writable"));
    }

    #[cfg(unix)]
    #[test]
    fn stale_controller_path_must_be_a_socket() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("controller.sock");
        std::fs::write(&path, b"not a socket").unwrap();
        assert!(remove_stale_controller_socket(&path)
            .unwrap_err()
            .contains("non-socket"));
        assert!(path.exists());
    }
}
