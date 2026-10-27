//! Project-first OAuth subject helpers used by `webcodex share`.
//!
//! The OAuth transport credential is session-fenced, while Connector durable
//! identity remains the stable project grant. A share restart therefore makes
//! old authorization codes/access tokens/refresh tokens unusable without
//! changing ownership of existing project tasks.

use super::{
    SCOPE_JOB_RUN, SCOPE_PROJECT_READ, SCOPE_PROJECT_WRITE, SCOPE_RUNTIME_READ,
    SCOPE_SESSION_COLLABORATE,
};
pub(crate) use webcodex_core::authority::{
    parse_project_share_subject_id, PROJECT_SHARE_OAUTH_SUBJECT_KIND,
};
use webcodex_core::authority::{project_share_subject_id, PROJECT_SHARE_SESSION_PREFIX};

pub(crate) const PROJECT_SHARE_OAUTH_TOKEN_KIND: &str = "oauth2_project";
pub(crate) const PROJECT_SHARE_OAUTH_SCOPES: &[&str] = &[
    SCOPE_RUNTIME_READ,
    SCOPE_SESSION_COLLABORATE,
    SCOPE_PROJECT_READ,
    SCOPE_PROJECT_WRITE,
    SCOPE_JOB_RUN,
];

pub(crate) fn generate_project_share_session_id() -> String {
    let mut random = String::with_capacity(64);
    while random.len() < 64 {
        random.push_str(&uuid::Uuid::new_v4().simple().to_string());
    }
    random.truncate(64);
    format!("{PROJECT_SHARE_SESSION_PREFIX}{random}")
}

/// Restart-stable session id derived from an already validated, persisted
/// project-share OAuth client id. An arbitrary caller string is never an
/// authority input; only the canonical generated client id format is accepted.
pub(crate) fn project_share_session_id_for_client(client_id: &str) -> Result<String, String> {
    let suffix = client_id
        .strip_prefix("wc_client_")
        .filter(|suffix| {
            suffix.len() == 64
                && suffix
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        })
        .ok_or_else(|| "invalid persisted project-share OAuth client ID".to_string())?;
    Ok(format!("{PROJECT_SHARE_SESSION_PREFIX}{suffix}"))
}

pub(crate) fn configured_project_share_subject(
    config: &crate::Config,
) -> Result<Option<String>, String> {
    match (
        config.oauth2.project_share_grant_id.as_deref(),
        config.oauth2.project_share_session_id.as_deref(),
    ) {
        (None, None) => Ok(None),
        (Some(grant_id), Some(session_id)) => {
            project_share_subject_id(grant_id, session_id).map(Some)
        }
        _ => Err("project-share OAuth configuration is incomplete".to_string()),
    }
}

pub(crate) fn validate_project_share_grant_subject(
    config: &crate::Config,
    subject_kind: &str,
    subject_id: &str,
) -> Result<(), String> {
    if subject_kind != PROJECT_SHARE_OAUTH_SUBJECT_KIND {
        return Ok(());
    }
    let expected = configured_project_share_subject(config)?
        .ok_or_else(|| "project-share OAuth session is not active".to_string())?;
    if expected != subject_id {
        return Err("project-share OAuth session is no longer active".to_string());
    }
    Ok(())
}

pub(crate) fn project_share_scopes_are_bounded(scopes: &str) -> bool {
    scopes
        .split_ascii_whitespace()
        .all(|scope| PROJECT_SHARE_OAUTH_SCOPES.contains(&scope) || scope == "offline_access")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn persisted_client_session_is_stable_without_weakening_subject_validation() {
        let client = format!("wc_client_{}", "a".repeat(64));
        let stable = project_share_session_id_for_client(&client).unwrap();
        assert_eq!(
            stable,
            format!("{PROJECT_SHARE_SESSION_PREFIX}{}", "a".repeat(64))
        );
        assert_eq!(
            project_share_session_id_for_client(&client).unwrap(),
            stable
        );
        assert_ne!(generate_project_share_session_id(), stable);
        for invalid in [
            "".to_string(),
            "wc_client_short".to_string(),
            format!("wc_client_{}", "g".repeat(64)),
            format!("wc_client_{}", "A".repeat(64)),
            format!("wc_csec_{}", "a".repeat(64)),
        ] {
            assert!(
                project_share_session_id_for_client(&invalid).is_err(),
                "{invalid}"
            );
        }
    }
}
