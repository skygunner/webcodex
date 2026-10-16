//! Process-bound observations. They inform status, never execution or restart authority.
use super::*;

const POLL_INTERVAL: Duration = Duration::from_secs(10);
const MAX_AGE: Duration = Duration::from_secs(30);

#[derive(Clone, Debug, PartialEq, Eq)]
struct Target {
    process_generation: uuid::Uuid,
    server: String,
    view: RunnerConfigView,
    wsl2_generation: Option<String>,
}

struct Sample {
    target: Target,
    at: Instant,
    observed_at: String,
    result: Result<bool, &'static str>,
}

#[derive(Default)]
pub(super) struct RunnerMonitor {
    target: Option<Target>,
    task: Option<tokio::task::JoinHandle<Sample>>,
    next_poll: Option<Instant>,
    sample: Option<Sample>,
    last_logged: Option<(&'static str, Option<&'static str>)>,
}

impl RunnerMonitor {
    pub(super) fn reset(&mut self) {
        if let Some(task) = self.task.take() {
            task.abort();
        }
        self.target = None;
        self.next_poll = None;
        self.sample = None;
        self.last_logged = None;
    }

    pub(super) fn start(
        &mut self,
        server: String,
        view: RunnerConfigView,
        wsl2_generation: Option<String>,
    ) {
        self.reset();
        self.target = Some(Target {
            process_generation: uuid::Uuid::new_v4(),
            server,
            view,
            wsl2_generation,
        });
        self.next_poll = Some(Instant::now());
    }

    fn projection(&self, now: Instant) -> (&'static str, Option<&'static str>) {
        let Some(sample) = &self.sample else {
            return ("starting", None);
        };
        if now.saturating_duration_since(sample.at) >= MAX_AGE {
            return ("running_unverified", Some("runner_observation_expired"));
        }
        match sample.result {
            Ok(true) => ("ready", None),
            Ok(false) => ("running_offline", None),
            Err(error) => ("running_unverified", Some(error)),
        }
    }

    pub(super) fn decorate(&self, snapshot: &mut ComponentSnapshot, now: Instant) {
        if snapshot.pid.is_none()
            || self.target.is_none()
            || !matches!(
                snapshot.phase,
                "starting" | "ready" | "running_unverified" | "running_offline"
            )
        {
            return;
        }
        let (phase, error) = self.projection(now);
        snapshot.phase = phase;
        snapshot.observed_at = self
            .sample
            .as_ref()
            .map(|sample| sample.observed_at.clone());
        snapshot.observation_error = error;
    }
}

impl Drop for RunnerMonitor {
    fn drop(&mut self) {
        self.reset();
    }
}

fn credential_error(error: &str) -> &'static str {
    if error.starts_with("Ambiguous") {
        "user_credential_ambiguous"
    } else if error.contains("is empty") || error.contains("Runner transport token") {
        "user_credential_invalid"
    } else {
        "user_credential_unavailable"
    }
}

fn sample(target: Target, result: Result<bool, &'static str>) -> Sample {
    Sample {
        target,
        at: Instant::now(),
        observed_at: chrono::Utc::now().to_rfc3339(),
        result,
    }
}

async fn observe(config: ControllerConfig, target: Target) -> Sample {
    let result = async {
        if runner_view(&config.runner.config).ok().as_ref() != Some(&target.view) {
            return Err("runner_identity_changed");
        }
        let token = projects::read_user_token(&config, &target.view, None)
            .map_err(|error| credential_error(&error))?;
        let options = webcodex_admin::ServerHttpOptions {
            no_system_proxy: config.server.mode == ServerMode::Wsl2Nat
                || super::super::project::server_url_is_loopback(&target.server),
            ..Default::default()
        };
        runner_query::query(&target.server, &target.view.client_id, &token, &options)
            .await
            .map(|observed| observed.online)
            .map_err(|error| error.code())
    }
    .await;
    sample(target, result)
}

impl ControllerRuntime {
    pub(super) async fn poll_runner_observation(&mut self) {
        if self.shutdown || self.runner.process.is_none() {
            self.runner_monitor.reset();
            return;
        }
        let Some(target) = self.runner_monitor.target.clone() else {
            return;
        };
        let binding_matches = runner_view(&self.config.runner.config).ok().as_ref()
            == Some(&target.view)
            && (self.config.server.mode != ServerMode::Wsl2Nat
                || (self.wsl2_startup.snapshot.generation == target.wsl2_generation
                    && self.wsl2_startup.snapshot.effective_url.as_ref() == Some(&target.server)));
        if !binding_matches {
            if let Some(task) = self.runner_monitor.task.take() {
                task.abort();
            }
            if self
                .runner_monitor
                .sample
                .as_ref()
                .is_none_or(|sample| sample.result != Err("runner_identity_changed"))
            {
                self.runner_monitor.sample = Some(sample(target, Err("runner_identity_changed")));
            }
            self.runner_monitor.next_poll = Some(Instant::now());
        } else {
            if self
                .runner_monitor
                .task
                .as_ref()
                .is_some_and(|task| task.is_finished())
            {
                let task = self
                    .runner_monitor
                    .task
                    .take()
                    .expect("finished observation");
                let observation = task
                    .await
                    .unwrap_or_else(|_| sample(target.clone(), Err("runner_observation_failed")));
                // A completed task is never evidence for a replacement process.
                if self.runner_monitor.target.as_ref() == Some(&observation.target) {
                    self.runner_monitor.next_poll = Some(observation.at + POLL_INTERVAL);
                    self.runner_monitor.sample = Some(observation);
                } else {
                    self.runner_monitor.next_poll = Some(Instant::now());
                }
            }
            if self.runner_monitor.task.is_none()
                && self
                    .runner_monitor
                    .next_poll
                    .is_some_and(|at| Instant::now() >= at)
            {
                self.runner_monitor.next_poll = None;
                self.runner_monitor.task = Some(tokio::spawn(observe(self.config.clone(), target)));
            }
        }
        let state = self.runner_monitor.projection(Instant::now());
        self.runner.phase = state.0;
        if self.runner_monitor.last_logged != Some(state) {
            log_line(
                &self.log,
                "controller",
                format!(
                    "runner {}{}",
                    state.0,
                    state
                        .1
                        .map(|error| format!(": {error}"))
                        .unwrap_or_default()
                ),
            );
            self.runner_monitor.last_logged = Some(state);
        }
    }
}

pub(super) async fn live_snapshot(config: &ControllerConfig) -> Result<Option<Value>, String> {
    let connection = tokio::time::timeout(
        Duration::from_secs(2),
        UnixStream::connect(controller_socket(config)?),
    )
    .await
    .map_err(|_| "controller_unavailable: connection timed out")?;
    let stream = match connection {
        Ok(stream) => stream,
        Err(error)
            if matches!(
                error.kind(),
                std::io::ErrorKind::NotFound | std::io::ErrorKind::ConnectionRefused
            ) =>
        {
            return Ok(None)
        }
        Err(_) => return Err("controller_unavailable: cannot access the control socket".into()),
    };
    let response = tokio::time::timeout(
        Duration::from_secs(2),
        ipc_exchange(stream, json!({"method":"status"})),
    )
    .await
    .map_err(|_| "controller_unavailable: live snapshot timed out")?
    .map_err(|_| "controller_unavailable: invalid live snapshot")?;
    let runtime = &response["result"];
    if response["ok"] != true
        || runtime["server_mode"] != json!(config.server.mode)
        || !runtime["runner"]["phase"].is_string()
    {
        return Err("controller_target_changed: invalid Controller snapshot".into());
    }
    Ok(Some(runtime.clone()))
}

pub(super) fn attach_runtime(report: &mut Value, runtime: Option<Value>) {
    if let Some(runner) = report["runner"].as_object_mut() {
        runner.remove("observed_at");
        runner.remove("observation_error");
    }
    if let Some(runtime) = runtime {
        for key in ["phase", "observed_at", "observation_error"] {
            if let Some(value) = runtime["runner"].get(key) {
                report["runner"][key] = value.clone();
            }
        }
        report["runtime"] = runtime;
    } else {
        report["runtime"] = Value::Null;
        report["runner"]["phase"] = json!(if report["runner"]["enabled"] == false {
            "disabled"
        } else {
            "unknown"
        });
    }
}

#[cfg(test)]
#[path = "tests/controller_runner_observation.rs"]
mod tests;
