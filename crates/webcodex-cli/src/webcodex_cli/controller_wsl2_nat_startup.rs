//! Controller-owned startup attempts and safe endpoint snapshots.
use super::*;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct Binding {
    runner_config: PathBuf,
    client_id: String,
    enrolled_server_url: String,
    interface: String,
    scheme: String,
    port: u16,
}

impl Binding {
    pub(super) fn read(config: &ControllerConfig) -> Result<Self, String> {
        let view = runner_view(&config.runner.config)?;
        validate_runner(&config.runner.config)?;
        Ok(Self {
            runner_config: config
                .runner
                .config
                .canonicalize()
                .map_err(|_| "runner_config_unavailable")?,
            client_id: view.client_id,
            enrolled_server_url: super::super::connections::canonical_server_url(&view.server_url)
                .map_err(|_| "runner_server_url_invalid")?
                .url,
            interface: config.server.wsl2_interface().into(),
            scheme: config.server.wsl2_scheme().into(),
            port: config.server.port.ok_or("server.port is required")?,
        })
    }
}

fn validate_runner(path: &Path) -> Result<(), String> {
    // Inspect only transport selection; never project credentials into diagnostics.
    let text = std::fs::read_to_string(path).map_err(|_| "runner_config_unavailable")?;
    let value: toml::Value = toml::from_str(&text).map_err(|_| "runner_config_invalid")?;
    let transport = value
        .get("transport")
        .map(|v| v.as_str().unwrap_or("invalid"))
        .unwrap_or("websocket")
        .trim();
    if !matches!(transport, "websocket" | "polling" | "auto" | "") || value.get("quic").is_some() {
        return Err(
            "wsl2_nat requires websocket, polling or auto without a fixed QUIC endpoint".into(),
        );
    }
    Ok(())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct Wsl2NatSnapshot {
    pub effective_url: Option<String>,
    pub generation: Option<String>,
    pub binding: Option<Binding>,
    pub ifindex: Option<u32>,
    pub gateway: Option<std::net::Ipv4Addr>,
    pub resolution_state: String,
    pub probe_state: String,
    pub source: String,
    pub sampled_at: Option<String>,
    pub error: Option<String>,
}

impl Default for Wsl2NatSnapshot {
    fn default() -> Self {
        Self {
            effective_url: None,
            generation: None,
            binding: None,
            ifindex: None,
            gateway: None,
            resolution_state: "unresolved".into(),
            probe_state: "not_probed".into(),
            source: "controller_active".into(),
            sampled_at: None,
            error: None,
        }
    }
}

pub(super) struct Wsl2NatStartup {
    pub snapshot: Wsl2NatSnapshot,
    task: Option<tokio::task::JoinHandle<Wsl2NatSnapshot>>,
    pub retry_at: Option<Instant>,
    retry_secs: u64,
}

impl Default for Wsl2NatStartup {
    fn default() -> Self {
        Self {
            snapshot: Wsl2NatSnapshot::default(),
            task: None,
            retry_at: None,
            retry_secs: 3,
        }
    }
}

impl Wsl2NatStartup {
    pub fn cancel(&mut self) {
        if let Some(task) = self.task.take() {
            task.abort();
        }
        self.retry_at = None;
        self.snapshot = Wsl2NatSnapshot::default();
    }
}

impl Drop for Wsl2NatStartup {
    fn drop(&mut self) {
        self.cancel();
    }
}

pub(super) fn configure_runner_command(
    command: &mut Command,
    server_url: &str,
) -> Result<(), String> {
    let url = url::Url::parse(server_url).map_err(|_| "server_url_invalid")?;
    let host = url.host_str().ok_or("server_url_invalid")?;
    command.args(["--server-url", server_url]);
    // Child-only additions preserve explicit operator proxy exclusions.
    for name in ["NO_PROXY", "no_proxy"] {
        let mut value = std::env::var_os(name).unwrap_or_default();
        if !value.is_empty() {
            value.push(",");
        }
        value.push(host);
        command.env(name, value);
    }
    Ok(())
}

async fn resolve(config: &ControllerConfig) -> Result<(String, u32, std::net::Ipv4Addr), String> {
    #[cfg(target_os = "linux")]
    {
        let target =
            tokio::time::timeout(Duration::from_secs(3), wsl2_nat::resolve(&config.server))
                .await
                .map_err(|_| "netlink_timeout")??;
        Ok((target.server_url, target.ifindex, target.gateway))
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = config;
        Err("wsl2_nat requires Linux inside WSL2 NAT".into())
    }
}

async fn probe(url: &str) -> Result<(), String> {
    let client = reqwest::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(2))
        .build()
        .map_err(|_| "server_probe_failed")?;
    let response = client
        .post(format!("{url}/api/runtime/status"))
        .json(&json!({}))
        .send()
        .await
        .map_err(|_| "server_unreachable")?;
    if response.status().is_success() || matches!(response.status().as_u16(), 401 | 403) {
        Ok(())
    } else {
        Err("server_unreachable".into())
    }
}

async fn attempt(config: ControllerConfig, binding: Binding) -> Wsl2NatSnapshot {
    let mut state = Wsl2NatSnapshot {
        binding: Some(binding),
        sampled_at: Some(chrono::Utc::now().to_rfc3339()),
        ..Default::default()
    };
    match resolve(&config).await {
        Err(error) => {
            state.resolution_state = "failed".into();
            state.error = Some(error);
        }
        Ok((url, index, gateway)) => {
            state.resolution_state = "resolved".into();
            state.ifindex = Some(index);
            state.gateway = Some(gateway);
            match probe(&url).await {
                Ok(()) => {
                    state.probe_state = "reachable".into();
                    state.effective_url = Some(url);
                    state.generation = Some(uuid::Uuid::new_v4().to_string());
                }
                Err(error) => {
                    state.probe_state = "unreachable".into();
                    state.error = Some(error);
                }
            }
        }
    }
    state
}

impl ControllerRuntime {
    pub(super) fn schedule_wsl2_start(&mut self) -> Result<(), String> {
        Binding::read(&self.config)?;
        self.wsl2_startup.cancel();
        self.wsl2_startup.retry_secs = 3;
        self.wsl2_startup.retry_at = Some(Instant::now());
        self.server.phase = "waiting_for_desktop_server";
        self.runner.phase = "waiting_for_server";
        Ok(())
    }

    pub(super) async fn poll_wsl2_start(&mut self) {
        if self.shutdown {
            return;
        }
        if self.runner.process.is_some() {
            return; // Never rediscover or probe while a Runner is alive.
        }
        if self.wsl2_startup.snapshot.effective_url.is_none() {
            self.server.phase = "waiting_for_desktop_server";
        }
        if self
            .wsl2_startup
            .task
            .as_ref()
            .is_some_and(|task| task.is_finished())
        {
            let task = self.wsl2_startup.task.take().expect("finished task");
            match task.await {
                Ok(state) => {
                    if self.wsl2_startup.snapshot.error != state.error {
                        if let Some(error) = &state.error {
                            log_line(&self.log, "controller", error);
                        }
                    }
                    self.wsl2_startup.snapshot = state;
                }
                Err(_) => {
                    self.wsl2_startup.snapshot.error = Some("startup_task_failed".into());
                }
            }
            if self.wsl2_startup.snapshot.effective_url.is_some() {
                // A file edit during discovery cannot silently change Runner identity.
                let current = Binding::read(&self.config);
                if current.ok().as_ref() != self.wsl2_startup.snapshot.binding.as_ref() {
                    self.wsl2_startup.cancel();
                    self.wsl2_startup.snapshot.error = Some("runner_identity_changed".into());
                    self.runner.phase = "failed";
                    return;
                }
                self.server.phase = "remote_ready";
                if let Err(error) = self.start_component(Component::Runner).await {
                    log_line(&self.log, "controller", error);
                    self.wsl2_startup.cancel();
                    self.wsl2_startup.snapshot.error = Some("runner_start_failed".into());
                    self.runner.phase = "failed";
                }
                return;
            }
            let jitter = (uuid::Uuid::new_v4().as_u128() % 250) as u64;
            self.wsl2_startup.retry_at = Some(
                Instant::now()
                    + Duration::from_millis(
                        (self.wsl2_startup.retry_secs * 1000 + jitter).min(30_000),
                    ),
            );
            self.wsl2_startup.retry_secs = (self.wsl2_startup.retry_secs * 2).min(30);
        }
        if self.wsl2_startup.task.is_none()
            && self
                .wsl2_startup
                .retry_at
                .is_some_and(|at| Instant::now() >= at)
        {
            self.wsl2_startup.retry_at = None;
            match Binding::read(&self.config) {
                Ok(binding) => {
                    self.wsl2_startup.task =
                        Some(tokio::spawn(attempt(self.config.clone(), binding)))
                }
                Err(error) => {
                    self.wsl2_startup.snapshot.error = Some(error);
                    self.runner.phase = "failed";
                }
            }
        }
    }
}

pub(super) async fn active_target(config: &ControllerConfig) -> Result<Wsl2NatSnapshot, String> {
    let response = tokio::time::timeout(
        Duration::from_secs(2),
        ipc_call(&controller_socket(config)?, json!({"method":"status"})),
    )
    .await
    .map_err(|_| "controller_unavailable")?
    .map_err(|_| "controller_unavailable")?;
    if response["ok"] != true || response["result"]["server_mode"] != "wsl2_nat" {
        return Err("controller_unavailable: expected the configured WSL2 NAT Controller".into());
    }
    let state: Wsl2NatSnapshot =
        serde_json::from_value(response["result"]["wsl2_nat_target"].clone())
            .map_err(|_| "wsl2_nat_target_unresolved")?;
    if state.effective_url.is_none()
        || state.generation.is_none()
        || state.probe_state != "reachable"
        || !matches!(
            response["result"]["runner"]["phase"].as_str(),
            Some("starting" | "running_unverified" | "running_offline" | "ready")
        )
    {
        return Err("wsl2_nat_target_unresolved: Runner has no active target".into());
    }
    if state.binding.as_ref() != Some(&Binding::read(config)?) {
        return Err("controller_target_changed: config no longer matches the active Runner".into());
    }
    Ok(state)
}

pub(super) async fn doctor(config: &ControllerConfig, report: &mut Value) -> Result<(), String> {
    let runtime = runner_observation::live_snapshot(config).await?;
    let state = if let Some(runtime) = &runtime {
        serde_json::from_value::<Wsl2NatSnapshot>(runtime["wsl2_nat_target"].clone())
            .map_err(|_| "invalid_controller_snapshot")?
    } else {
        let mut state = attempt(config.clone(), Binding::read(config)?).await;
        state.source = "candidate".into();
        state.generation = None;
        state
    };
    runner_observation::attach_runtime(report, runtime);
    let binding_matches = state.binding.as_ref() == Some(&Binding::read(config)?);
    report["server"]["url"] = json!(state.effective_url);
    report["runner"]["server_matches_controller"] = json!(binding_matches);
    report["ok"] = json!(
        state.effective_url.is_some()
            && binding_matches
            && report["runner"]["config_file"] == true
            && report["runner"]["binary"] == true
            && report["existing_service_conflicts"]
                .as_array()
                .is_some_and(|v| v.is_empty())
    );
    report["wsl2_nat_target"] = serde_json::to_value(state).map_err(|e| e.to_string())?;
    Ok(())
}

#[cfg(test)]
#[path = "tests/controller_wsl2_nat_startup.rs"]
mod tests;
