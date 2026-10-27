//! Project-first OAuth subject helpers used by `webcodex share`.
//!
//! The OAuth transport credential is session-fenced, while Connector durable
//! identity remains the stable project grant. A share restart therefore makes
//! old authorization codes/access tokens/refresh tokens unusable without
//! changing ownership of existing project tasks.

use std::path::PathBuf;

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

/// Canonical parent-root grants for project-share OAuth subjects.
///
/// Roots are canonicalized so later containment checks compare real directory
/// components, not textual prefixes. A configured root that no longer resolves
/// (deleted or unreadable since server start) is dropped rather than failing
/// authentication: dropping it only removes authority, never grants more.
/// Relative configured roots are dropped for the same reason: `canonicalize`
/// would resolve them against the Server process cwd and silently grant
/// whatever that lands on. Only the `--project-roots` CLI flag and the
/// persisted grant file reach this function through
/// `validate_share_project_root`, which already requires absolute paths; this
/// check closes the remaining `WEBCODEX_OAUTH2_PROJECT_SHARE_ROOTS` env path.
/// Symlinks inside the root are resolved by canonicalization, so a link that
/// escapes the root cannot pass the fence through its link name.
pub(crate) fn project_share_root_scopes_from_config(config: &crate::Config) -> Vec<PathBuf> {
    config
        .oauth2
        .project_share_roots
        .iter()
        .filter_map(|root| {
            if !root.is_absolute() {
                tracing::warn!(
                    root = %root.display(),
                    "configured project-share root is relative; ignored"
                );
                return None;
            }
            match root.canonicalize() {
                Ok(canonical) if canonical.is_dir() => Some(canonical),
                Ok(_) => {
                    tracing::warn!("configured project-share root is not a directory; ignored");
                    None
                }
                Err(error) => {
                    tracing::warn!(%error, "configured project-share root did not resolve; ignored");
                    None
                }
            }
        })
        .collect()
}

pub(crate) fn project_share_scopes_are_bounded(scopes: &str) -> bool {
    scopes
        .split_ascii_whitespace()
        .all(|scope| PROJECT_SHARE_OAUTH_SCOPES.contains(&scope) || scope == "offline_access")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config_with_roots(roots: Vec<PathBuf>) -> crate::Config {
        crate::Config {
            addr: "127.0.0.1:0".to_string(),
            data_dir: PathBuf::from("./data"),
            token: Some("bootstrap-token".to_string()),
            max_text_size: 2 * 1024 * 1024,
            oauth2: crate::OAuth2Config {
                project_share_roots: roots,
                ..crate::OAuth2Config::default()
            },
        }
    }

    #[test]
    fn project_share_root_scopes_drop_relative_and_unresolvable_roots() {
        let dir = tempfile::tempdir().unwrap();
        let granted = dir.path().join("granted");
        std::fs::create_dir(&granted).unwrap();

        // Core policy, platform-independent: one admitted absolute directory,
        // one relative spelling that would resolve from any cwd, one missing
        // absolute path (dropped as before).
        let config = config_with_roots(vec![
            granted.clone(),
            PathBuf::from(".."),
            dir.path().join("missing"),
        ]);
        let scopes = project_share_root_scopes_from_config(&config);
        assert_eq!(scopes, vec![granted.canonicalize().unwrap()]);
    }

    #[cfg(unix)]
    #[test]
    fn project_share_root_scopes_resolve_a_symlinked_root_to_its_target() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("granted");
        std::fs::create_dir(&target).unwrap();
        // A symlinked spelling of a configured root resolves to its target:
        // the authority is the real directory, never the link name.
        let link = dir.path().join("link-to-granted");
        std::os::unix::fs::symlink(&target, &link).unwrap();

        let config = config_with_roots(vec![link]);
        let scopes = project_share_root_scopes_from_config(&config);
        assert_eq!(scopes, vec![target.canonicalize().unwrap()]);
    }
}
