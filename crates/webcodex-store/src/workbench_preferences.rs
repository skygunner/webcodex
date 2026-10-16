use super::store_primitives::{
    validate_communication_principal, validate_id, CommunicationPrincipal, CommunicationStoreError,
};
use super::Database;
use rusqlite::{params, OptionalExtension};

const MAX_WORKBENCH_SUBMISSIONS_PER_PRINCIPAL: i64 = 1_000;

const MAX_DISPLAY_NAME_CHARS: usize = 80;
const MAX_DISPLAY_NAME_BYTES: usize = 320;
const AGENT_TASK_ID_PREFIX: &str = "wc_agent_task_";

fn normalize_display_name(name: &str) -> Result<String, CommunicationStoreError> {
    let name = name.trim();
    if name.is_empty()
        || name.chars().count() > MAX_DISPLAY_NAME_CHARS
        || name.len() > MAX_DISPLAY_NAME_BYTES
        || name.chars().any(char::is_control)
    {
        return Err(CommunicationStoreError::new(
            "invalid_workbench_display_name",
            "Workbench display name must be non-empty, contain no control characters, and be at most 80 characters and 320 UTF-8 bytes",
        ));
    }
    Ok(name.to_owned())
}

fn store_error(error: rusqlite::Error) -> CommunicationStoreError {
    tracing::warn!(error = %error, "Workbench preference store operation failed");
    CommunicationStoreError::new(
        "workbench_preferences_unavailable",
        "Workbench preferences are unavailable",
    )
}

impl Database {
    /// Reserve a principal-bound submission key before composite Workbench effects.
    /// Retained rows are never pruned because that could make an old retry new again.
    pub fn begin_workbench_submission(
        &self,
        principal: &CommunicationPrincipal,
        key: &str,
        request_hash: &str,
    ) -> Result<(), CommunicationStoreError> {
        validate_communication_principal(principal)?;
        if key.is_empty() || key.chars().count() > 128 || key.chars().any(char::is_control) {
            return Err(CommunicationStoreError::new(
                "invalid_workbench_submission_key",
                "Workbench submission key must contain 1 to 128 characters and no control characters",
            ));
        }
        if request_hash.len() != 64
            || !request_hash
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        {
            return Err(CommunicationStoreError::new(
                "invalid_workbench_submission_hash",
                "Workbench submission request hash must be 64 lowercase hexadecimal characters",
            ));
        }

        let mut conn = self.lock_connection(crate::StoreDomain::Workbench);
        let tx = conn
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(store_error)?;
        let existing: Option<String> = tx
            .query_row(
                "SELECT request_hash FROM workbench_submissions
                 WHERE principal_kind = ?1 AND principal_digest = ?2 AND submission_key = ?3",
                params![principal.kind.trim(), principal.digest, key],
                |row| row.get(0),
            )
            .optional()
            .map_err(store_error)?;
        if let Some(existing_hash) = existing {
            if existing_hash == request_hash {
                tx.commit().map_err(store_error)?;
                return Ok(());
            }
            return Err(CommunicationStoreError::new(
                "workbench_submission_conflict",
                "Workbench submission key was already used with different request content",
            ));
        }

        let count: i64 = tx
            .query_row(
                "SELECT COUNT(*) FROM workbench_submissions
                 WHERE principal_kind = ?1 AND principal_digest = ?2",
                params![principal.kind.trim(), principal.digest],
                |row| row.get(0),
            )
            .map_err(store_error)?;
        if count >= MAX_WORKBENCH_SUBMISSIONS_PER_PRINCIPAL {
            return Err(CommunicationStoreError::new(
                "workbench_submission_capacity_reached",
                "Workbench submission key capacity is full for this principal",
            ));
        }
        tx.execute(
            "INSERT INTO workbench_submissions
             (principal_kind, principal_digest, submission_key, request_hash)
             VALUES (?1, ?2, ?3, ?4)",
            params![principal.kind.trim(), principal.digest, key, request_hash],
        )
        .map_err(store_error)?;
        tx.commit().map_err(store_error)
    }

    /// Bind a reserved Workbench submission to its exact owner Task. The link
    /// is immutable so replay can recover a partially completed orchestration.
    pub fn bind_workbench_submission_task(
        &self,
        principal: &CommunicationPrincipal,
        key: &str,
        task_id: &str,
    ) -> Result<(), CommunicationStoreError> {
        validate_communication_principal(principal)?;
        if key.is_empty() || key.chars().count() > 128 || key.chars().any(char::is_control) {
            return Err(CommunicationStoreError::new(
                "invalid_workbench_submission_key",
                "Workbench submission key must contain 1 to 128 characters and no control characters",
            ));
        }
        validate_id(task_id, AGENT_TASK_ID_PREFIX, "invalid_agent_task_id")?;

        let mut conn = self.lock_connection(crate::StoreDomain::Workbench);
        let tx = conn
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(store_error)?;
        let task_owned: bool = tx
            .query_row(
                "SELECT EXISTS(
                    SELECT 1 FROM wc_agent_tasks
                    WHERE task_id = ?1 AND owner_principal_kind = ?2 AND owner_principal_digest = ?3
                )",
                params![task_id, principal.kind.trim(), principal.digest],
                |row| row.get(0),
            )
            .map_err(store_error)?;
        if !task_owned {
            return Err(CommunicationStoreError::new(
                "agent_task_not_found",
                "AgentTask was not found",
            ));
        }

        let existing: Option<Option<String>> = tx
            .query_row(
                "SELECT task_id FROM workbench_submissions
                 WHERE principal_kind = ?1 AND principal_digest = ?2 AND submission_key = ?3",
                params![principal.kind.trim(), principal.digest, key],
                |row| row.get(0),
            )
            .optional()
            .map_err(store_error)?;
        match existing {
            None => {
                return Err(CommunicationStoreError::new(
                    "workbench_submission_not_found",
                    "Workbench submission key was not reserved",
                ));
            }
            Some(Some(bound)) if bound == task_id => {
                tx.commit().map_err(store_error)?;
                return Ok(());
            }
            Some(Some(_)) => {
                return Err(CommunicationStoreError::new(
                    "workbench_submission_task_conflict",
                    "Workbench submission is already bound to a different Task",
                ));
            }
            Some(None) => {}
        }
        tx.execute(
            "UPDATE workbench_submissions SET task_id = ?4
             WHERE principal_kind = ?1 AND principal_digest = ?2 AND submission_key = ?3 AND task_id IS NULL",
            params![principal.kind.trim(), principal.digest, key, task_id],
        )
        .map_err(store_error)?;
        tx.commit().map_err(store_error)
    }

    /// Check whether a canonical Task is linked to this principal's submission.
    /// Foreign Tasks and Tasks linked to another principal are indistinguishable.
    pub fn is_workbench_submission_task(
        &self,
        principal: &CommunicationPrincipal,
        task_id: &str,
    ) -> Result<bool, CommunicationStoreError> {
        validate_communication_principal(principal)?;
        validate_id(task_id, AGENT_TASK_ID_PREFIX, "invalid_agent_task_id")?;
        let conn = self.lock_connection(crate::StoreDomain::Workbench);
        conn.query_row(
            "SELECT EXISTS(
                SELECT 1 FROM workbench_submissions s
                JOIN wc_agent_tasks t ON t.task_id = s.task_id
                WHERE s.principal_kind = ?1 AND s.principal_digest = ?2
                  AND t.task_id = ?3
                  AND t.owner_principal_kind = ?1 AND t.owner_principal_digest = ?2
            )",
            params![principal.kind.trim(), principal.digest, task_id],
            |row| row.get(0),
        )
        .map_err(store_error)
    }

    /// Read the personal display-name override for one authenticated principal.
    pub fn read_workbench_display_name(
        &self,
        principal: &CommunicationPrincipal,
    ) -> Result<Option<String>, CommunicationStoreError> {
        validate_communication_principal(principal)?;
        let conn = self.lock_connection(crate::StoreDomain::Workbench);
        conn.query_row(
            "SELECT display_name FROM workbench_preferences
             WHERE principal_kind = ?1 AND principal_digest = ?2",
            params![principal.kind.trim(), principal.digest],
            |row| row.get(0),
        )
        .optional()
        .map_err(store_error)
    }

    /// Set or clear the personal display-name override for one authenticated principal.
    pub fn set_workbench_display_name(
        &self,
        principal: &CommunicationPrincipal,
        display_name: Option<&str>,
    ) -> Result<Option<String>, CommunicationStoreError> {
        validate_communication_principal(principal)?;
        let normalized = display_name.map(normalize_display_name).transpose()?;
        let conn = self.lock_connection(crate::StoreDomain::Workbench);
        if let Some(name) = normalized {
            conn.execute(
                "INSERT INTO workbench_preferences(principal_kind, principal_digest, display_name)
                 VALUES (?1, ?2, ?3)
                 ON CONFLICT(principal_kind, principal_digest)
                 DO UPDATE SET display_name = excluded.display_name",
                params![principal.kind.trim(), principal.digest, name],
            )
            .map_err(store_error)?;
            Ok(Some(name))
        } else {
            conn.execute(
                "DELETE FROM workbench_preferences
                 WHERE principal_kind = ?1 AND principal_digest = ?2",
                params![principal.kind.trim(), principal.digest],
            )
            .map_err(store_error)?;
            Ok(None)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn principal(kind: &str, suffix: char) -> CommunicationPrincipal {
        CommunicationPrincipal {
            kind: kind.to_owned(),
            digest: format!("wc_commprincipal_{}", suffix.to_string().repeat(64)),
        }
    }

    fn task_input(key: &str) -> crate::NewAgentTask {
        crate::NewAgentTask {
            title: "Workbench task".to_owned(),
            instruction: "A durable task for the Workbench source link test.".to_owned(),
            assignee_agent_id: None,
            source_conversation_id: None,
            source_message_id: None,
            referenced_project_id: None,
            idempotency_key: key.to_owned(),
        }
    }

    #[test]
    fn workbench_display_name_is_personal_persistent_validated_and_clearable() {
        let temp = tempdir().unwrap();
        let path = temp.path().join("preferences.db");
        let a = principal("oauth_user", 'a');
        let b = principal("oauth_user", 'b');
        {
            let db = Database::open(&path).unwrap();
            assert_eq!(db.read_workbench_display_name(&a).unwrap(), None);
            assert_eq!(
                db.set_workbench_display_name(&a, Some("  Personal 🧭  "))
                    .unwrap(),
                Some("Personal 🧭".into())
            );
            assert_eq!(
                db.set_workbench_display_name(&b, Some("Other personal name"))
                    .unwrap(),
                Some("Other personal name".into())
            );
            assert_eq!(
                db.read_workbench_display_name(&a).unwrap().as_deref(),
                Some("Personal 🧭")
            );
            assert_eq!(
                db.read_workbench_display_name(&b).unwrap().as_deref(),
                Some("Other personal name")
            );
            assert!(db.set_workbench_display_name(&a, Some(" \n ")).is_err());
            assert!(db
                .set_workbench_display_name(&a, Some("bad\u{7f}name"))
                .is_err());
            assert!(db
                .set_workbench_display_name(&a, Some(&"x".repeat(81)))
                .is_err());
            assert!(db
                .set_workbench_display_name(&a, Some(&"界".repeat(107)))
                .is_err());
        }
        let db = Database::open(&path).unwrap();
        assert_eq!(
            db.read_workbench_display_name(&a).unwrap().as_deref(),
            Some("Personal 🧭")
        );
        assert_eq!(db.set_workbench_display_name(&a, None).unwrap(), None);
        assert_eq!(db.read_workbench_display_name(&a).unwrap(), None);
        assert_eq!(
            db.read_workbench_display_name(&b).unwrap().as_deref(),
            Some("Other personal name")
        );
    }

    #[test]
    fn workbench_submission_keys_are_principal_bound_replay_safe_and_bounded() {
        let temp = tempdir().unwrap();
        let path = temp.path().join("submissions.db");
        let a = principal("oauth_user", 'a');
        let b = principal("oauth_user", 'b');
        let hash_a = "a".repeat(64);
        let hash_b = "b".repeat(64);
        {
            let db = Database::open(&path).unwrap();
            db.begin_workbench_submission(&a, "submit-1", &hash_a)
                .unwrap();
            db.begin_workbench_submission(&a, "submit-1", &hash_a)
                .unwrap();
            assert_eq!(
                db.begin_workbench_submission(&a, "submit-1", &hash_b)
                    .unwrap_err()
                    .code(),
                "workbench_submission_conflict"
            );
            db.begin_workbench_submission(&a, "submit-2", &hash_b)
                .unwrap();
            db.begin_workbench_submission(&b, "submit-1", &hash_b)
                .unwrap();
            assert!(db
                .begin_workbench_submission(&a, "bad\nkey", &hash_a)
                .is_err());
            assert!(db.begin_workbench_submission(&a, "", &hash_a).is_err());
            assert!(db
                .begin_workbench_submission(&a, "ok", "A".repeat(64).as_str())
                .is_err());
        }
        let db = Database::open(&path).unwrap();
        db.begin_workbench_submission(&a, "submit-1", &hash_a)
            .unwrap();
        assert_eq!(
            db.begin_workbench_submission(&a, "submit-1", &hash_b)
                .unwrap_err()
                .code(),
            "workbench_submission_conflict"
        );

        {
            let conn = db.conn_for_tests();
            let tx = conn.unchecked_transaction().unwrap();
            for index in 0..MAX_WORKBENCH_SUBMISSIONS_PER_PRINCIPAL {
                tx.execute(
                    "INSERT INTO workbench_submissions
                     (principal_kind, principal_digest, submission_key, request_hash)
                     VALUES (?1, ?2, ?3, ?4)",
                    params![a.kind, a.digest, format!("filled-{index}"), hash_a],
                )
                .unwrap();
            }
            tx.commit().unwrap();
        }
        // Exact historical retries still work at capacity; a new intent fails closed.
        db.begin_workbench_submission(&a, "submit-1", &hash_a)
            .unwrap();
        assert_eq!(
            db.begin_workbench_submission(&a, "one-too-many", &hash_a)
                .unwrap_err()
                .code(),
            "workbench_submission_capacity_reached"
        );
    }

    #[test]
    fn workbench_submission_task_link_is_owner_scoped_immutable_and_persistent() {
        let temp = tempdir().unwrap();
        let path = temp.path().join("submission-task-link.db");
        let owner = principal("oauth_user", 'a');
        let foreign = principal("oauth_user", 'b');
        let task_id;
        let foreign_task_id;
        let second_task_id;
        {
            let db = Database::open(&path).unwrap();
            db.begin_workbench_submission(&owner, "source-key", &"a".repeat(64))
                .unwrap();
            let task = db
                .create_agent_task(&owner, task_input("owner-task-1"))
                .unwrap();
            let second = db
                .create_agent_task(&owner, task_input("owner-task-2"))
                .unwrap();
            let foreign_task = db
                .create_agent_task(&foreign, task_input("foreign-task"))
                .unwrap();
            task_id = task.task.summary.task_id;
            second_task_id = second.task.summary.task_id;
            foreign_task_id = foreign_task.task.summary.task_id;

            assert_eq!(
                db.bind_workbench_submission_task(&owner, "missing-key", &task_id)
                    .unwrap_err()
                    .code(),
                "workbench_submission_not_found"
            );
            assert_eq!(
                db.bind_workbench_submission_task(&owner, "source-key", &foreign_task_id)
                    .unwrap_err()
                    .code(),
                "agent_task_not_found",
                "a foreign principal's Task must be hidden"
            );
            assert!(!db
                .is_workbench_submission_task(&owner, &foreign_task_id)
                .unwrap());

            db.bind_workbench_submission_task(&owner, "source-key", &task_id)
                .unwrap();
            db.bind_workbench_submission_task(&owner, "source-key", &task_id)
                .unwrap();
            assert!(db.is_workbench_submission_task(&owner, &task_id).unwrap());
            assert!(!db.is_workbench_submission_task(&foreign, &task_id).unwrap());
            assert_eq!(
                db.bind_workbench_submission_task(&owner, "source-key", &second_task_id)
                    .unwrap_err()
                    .code(),
                "workbench_submission_task_conflict"
            );
        }
        let db = Database::open(&path).unwrap();
        db.bind_workbench_submission_task(&owner, "source-key", &task_id)
            .unwrap();
        assert!(db.is_workbench_submission_task(&owner, &task_id).unwrap());
        assert!(!db
            .is_workbench_submission_task(&owner, &second_task_id)
            .unwrap());
    }
}
