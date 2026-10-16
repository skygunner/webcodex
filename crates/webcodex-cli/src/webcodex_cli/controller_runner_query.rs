//! One bounded, authenticated Runner observation shared by management and supervision.
use serde_json::{json, Value};
use std::time::Duration;
use webcodex_admin::ServerHttpOptions;

pub(super) const QUERY_TIMEOUT: Duration = Duration::from_secs(3);
const RESPONSE_LIMIT: usize = 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum QueryError {
    Unreachable,
    AccessDenied(u16),
    ApiUnavailable,
    InvalidResponse,
    NotVisible,
    Oversized,
}

impl QueryError {
    pub(super) fn code(self) -> &'static str {
        match self {
            Self::Unreachable => "server_unreachable",
            Self::AccessDenied(_) => "project_access_denied",
            Self::ApiUnavailable => "project_api_unavailable",
            Self::InvalidResponse => "invalid_runner_response",
            Self::NotVisible => "runner_not_visible",
            Self::Oversized => "runner_response_too_large",
        }
    }
}

impl std::fmt::Display for QueryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::AccessDenied(status) => write!(
                f,
                "{}: Server returned HTTP {status}; check the user credential and permissions",
                self.code()
            ),
            _ => write!(f, "{}: cannot verify the configured Runner", self.code()),
        }
    }
}

#[derive(Debug)]
pub(super) struct Observation {
    pub online: bool,
    pub project_inventory: Value,
}

fn parse(body: &Value, client_id: &str) -> Result<Observation, QueryError> {
    if body["success"] != true {
        return Err(QueryError::InvalidResponse);
    }
    let runners = body["output"]["runners"]
        .as_array()
        .ok_or(QueryError::InvalidResponse)?;
    let mut matching = runners
        .iter()
        .filter(|runner| runner["client_id"].as_str() == Some(client_id));
    let runner = matching.next().ok_or(QueryError::NotVisible)?;
    if matching.next().is_some() {
        return Err(QueryError::InvalidResponse);
    }
    let online = match runner["status"].as_str() {
        Some("online") => true,
        Some("stale" | "offline") => false,
        _ => return Err(QueryError::InvalidResponse),
    };
    Ok(Observation {
        online,
        project_inventory: runner["project_inventory"].clone(),
    })
}

pub(super) async fn query(
    server: &str,
    client_id: &str,
    token: &str,
    options: &ServerHttpOptions,
) -> Result<Observation, QueryError> {
    tokio::time::timeout(QUERY_TIMEOUT, async {
        options.validate().map_err(|_| QueryError::Unreachable)?;
        let mut builder = reqwest::Client::builder().redirect(reqwest::redirect::Policy::none());
        if options.no_system_proxy {
            builder = builder.no_proxy();
        } else if let Some(proxy) = &options.proxy {
            builder = builder
                .no_proxy()
                .proxy(reqwest::Proxy::all(proxy).map_err(|_| QueryError::Unreachable)?);
        }
        let client = builder.build().map_err(|_| QueryError::Unreachable)?;
        let mut response = client
            .post(format!("{server}/api/tools/call"))
            .bearer_auth(token)
            .json(&json!({"tool":"list_runners","params":{
                "client_id":client_id,"summary_only":false,"include_projects":false
            }}))
            .send()
            .await
            .map_err(|_| QueryError::Unreachable)?;
        match response.status().as_u16() {
            status @ (401 | 403) => return Err(QueryError::AccessDenied(status)),
            404 | 405 => return Err(QueryError::ApiUnavailable),
            200..=299 => {}
            _ => return Err(QueryError::InvalidResponse),
        }
        if response
            .content_length()
            .is_some_and(|length| length > RESPONSE_LIMIT as u64)
        {
            return Err(QueryError::Oversized);
        }
        let mut bytes = Vec::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|_| QueryError::Unreachable)?
        {
            if chunk.len() > RESPONSE_LIMIT.saturating_sub(bytes.len()) {
                return Err(QueryError::Oversized);
            }
            bytes.extend_from_slice(&chunk);
        }
        let body = serde_json::from_slice(&bytes).map_err(|_| QueryError::InvalidResponse)?;
        parse(&body, client_id)
    })
    .await
    .map_err(|_| QueryError::Unreachable)?
}

#[cfg(test)]
#[path = "tests/controller_runner_query.rs"]
mod tests;
