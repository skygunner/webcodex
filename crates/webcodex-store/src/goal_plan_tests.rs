use super::goal::{GoalLifecycle, GoalPatch, NewGoal};
use super::goal_plan::*;
use super::store_primitives::{CommunicationPrincipal, COMMUNICATION_PRINCIPAL_DIGEST_PREFIX};
use super::Database;
use rusqlite::params;

const T0: i64 = 10_000;

fn fixture() -> (tempfile::TempDir, Database, CommunicationPrincipal) {
    let temp = tempfile::tempdir().unwrap();
    let db = Database::open(&temp.path().join("goal-plan.db")).unwrap();
    let owner = CommunicationPrincipal {
        kind: "user".into(),
        digest: format!("{COMMUNICATION_PRINCIPAL_DIGEST_PREFIX}{}", "a".repeat(64)),
    };
    (temp, db, owner)
}

fn input(key: &str) -> NewGoal {
    NewGoal {
        title: "Goal workflow".into(),
        objective: "Implement, validate and review".into(),
        controller_agent_id: None,
        completion_conditions: vec!["Focused validation and independent review pass".into()],
        steps: ["inspect", "implement", "verify"]
            .into_iter()
            .map(|id| NewGoalStep {
                id: id.into(),
                title: id.into(),
            })
            .collect(),
        idempotency_key: key.into(),
    }
}

fn checkpoint(done: &[&str], current: Option<&str>, summary: &str) -> GoalCheckpoint {
    GoalCheckpoint {
        completed_step_ids: done.iter().map(|id| (*id).into()).collect(),
        current_step_id: current.map(str::to_string),
        summary: summary.into(),
    }
}

#[test]
fn goal_plan_create_read_checkpoint_and_exact_replay_survive_reopen() {
    let (temp, db, owner) = fixture();
    let created = db.create_goal_at(&owner, input("create"), T0).unwrap();
    let id = created.goal.summary.goal_id;
    assert_eq!(created.goal.plan.steps.len(), 3);
    assert!(created
        .goal
        .plan
        .steps
        .iter()
        .all(|step| step.status == GoalStepStatus::Pending && step.updated_at_unix_ms == T0));
    let first = checkpoint(
        &["inspect"],
        Some("implement"),
        "Inspection complete; implement the plan transaction",
    );
    let result = db
        .checkpoint_goal_at(&owner, &id, 1, first.clone(), "checkpoint", T0 + 1)
        .unwrap();
    assert_eq!(result.goal.summary.revision, 2);
    assert_eq!(result.goal.plan.completed_count(), 1);
    assert_eq!(result.goal.plan.current_step().unwrap().id, "implement");
    assert_eq!(result.goal.plan.checkpoint_at_unix_ms, Some(T0 + 1));
    assert_eq!(result.goal.plan.steps[0].updated_at_unix_ms, T0 + 1);
    let replay = db
        .checkpoint_goal_at(&owner, &id, 1, first.clone(), "checkpoint", T0 + 2)
        .unwrap();
    assert!(replay.replayed);
    assert!(!replay.state_changed);
    assert_eq!(replay.goal, result.goal);
    drop(db);
    let reopened = Database::open(&temp.path().join("goal-plan.db")).unwrap();
    assert_eq!(reopened.read_goal(&owner, &id).unwrap(), result.goal);
    let replay = reopened
        .checkpoint_goal_at(&owner, &id, 1, first.clone(), "checkpoint", T0 + 3)
        .unwrap();
    assert_eq!(replay.goal.summary.revision, 2);
    let mut changed = first;
    changed.summary.push_str(" changed");
    assert_eq!(
        reopened
            .checkpoint_goal_at(&owner, &id, 1, changed, "checkpoint", T0 + 4)
            .unwrap_err()
            .code(),
        "goal_idempotency_conflict"
    );
    assert_eq!(
        reopened
            .checkpoint_goal_at(
                &owner,
                &id,
                1,
                checkpoint(&[], None, "stale"),
                "new-key",
                T0 + 4
            )
            .unwrap_err()
            .code(),
        "goal_revision_changed"
    );
}

#[test]
fn goal_plan_invalid_batch_is_atomic_and_completed_steps_never_regress() {
    let (_temp, db, owner) = fixture();
    let created = db.create_goal_at(&owner, input("create"), T0).unwrap();
    let id = created.goal.summary.goal_id;
    let first = db
        .checkpoint_goal_at(
            &owner,
            &id,
            1,
            checkpoint(&["inspect"], Some("implement"), "Ready"),
            "first",
            T0 + 1,
        )
        .unwrap();
    for (index, bad) in [
        checkpoint(
            &["verify", "missing"],
            None,
            "unknown must not partially complete verify",
        ),
        checkpoint(&["verify", "verify"], None, "duplicate"),
        checkpoint(&["bad id"], None, "invalid id"),
        checkpoint(&[], Some("missing"), "unknown current"),
        checkpoint(&[], Some("verify"), "two in progress"),
        checkpoint(&[], Some("inspect"), "regression"),
        checkpoint(&["verify"], Some("verify"), "contradictory batch"),
        checkpoint(&[], None, "  "),
        checkpoint(&[], None, &"x".repeat(MAX_GOAL_PROGRESS_SUMMARY_BYTES + 1)),
    ]
    .into_iter()
    .enumerate()
    {
        assert_eq!(
            db.checkpoint_goal_at(&owner, &id, 2, bad, &format!("bad-{index}"), T0 + 2)
                .unwrap_err()
                .code(),
            "invalid_goal_checkpoint"
        );
        assert_eq!(db.read_goal(&owner, &id).unwrap(), first.goal);
    }
    let second = db
        .checkpoint_goal_at(
            &owner,
            &id,
            2,
            checkpoint(&["implement"], Some("verify"), "Implement complete"),
            "second",
            T0 + 3,
        )
        .unwrap();
    assert_eq!(second.goal.summary.revision, 3);
    assert_eq!(second.goal.plan.completed_count(), 2);
    assert_eq!(second.goal.plan.current_step().unwrap().id, "verify");
    // Repeating an already completed selection cannot change its completion time.
    let third = db
        .checkpoint_goal_at(
            &owner,
            &id,
            3,
            checkpoint(&["inspect"], None, "Recovery checkpoint"),
            "third",
            T0 + 4,
        )
        .unwrap();
    assert_eq!(third.goal.summary.revision, 4);
    assert_eq!(third.goal.plan.steps[0], first.goal.plan.steps[0]);
}

#[test]
fn goal_plan_gates_completion_and_terminal_checkpoint_mutation() {
    let (_temp, db, owner) = fixture();
    let goal = db.create_goal_at(&owner, input("create"), T0).unwrap().goal;
    let id = &goal.summary.goal_id;
    assert_eq!(
        db.update_goal_at(
            &owner,
            id,
            1,
            GoalPatch {
                lifecycle: Some(GoalLifecycle::Completed),
                ..Default::default()
            },
            "complete",
            T0 + 1
        )
        .unwrap_err()
        .code(),
        "goal_plan_incomplete"
    );
    assert_eq!(db.read_goal(&owner, id).unwrap(), goal);
    let batch = checkpoint(
        &["inspect", "implement", "verify"],
        None,
        "Fresh validation and review complete",
    );
    let checkpointed = db
        .checkpoint_goal_at(&owner, id, 1, batch.clone(), "all", T0 + 2)
        .unwrap();
    assert!(checkpointed.goal.plan.complete());
    let completed = db
        .update_goal_at(
            &owner,
            id,
            2,
            GoalPatch {
                lifecycle: Some(GoalLifecycle::Completed),
                ..Default::default()
            },
            "complete-after-plan",
            T0 + 3,
        )
        .unwrap();
    assert_eq!(completed.goal.summary.revision, 3);
    assert_eq!(
        db.checkpoint_goal_at(
            &owner,
            id,
            3,
            checkpoint(&[], None, "not allowed"),
            "after-terminal",
            T0 + 4
        )
        .unwrap_err()
        .code(),
        "goal_terminal"
    );
    // Historical exact replay is not a mutation of terminal truth.
    assert!(
        db.checkpoint_goal_at(&owner, id, 1, batch, "all", T0 + 5)
            .unwrap()
            .replayed
    );
    assert_eq!(db.read_goal(&owner, id).unwrap(), completed.goal);
    let cancelled = db
        .create_goal_at(&owner, input("cancel-create"), T0)
        .unwrap()
        .goal;
    db.update_goal_at(
        &owner,
        &cancelled.summary.goal_id,
        1,
        GoalPatch {
            lifecycle: Some(GoalLifecycle::Cancelled),
            ..Default::default()
        },
        "cancel",
        T0 + 1,
    )
    .unwrap();
    assert_eq!(
        db.checkpoint_goal_at(
            &owner,
            &cancelled.summary.goal_id,
            2,
            checkpoint(&[], None, "no"),
            "cancelled",
            T0 + 2
        )
        .unwrap_err()
        .code(),
        "goal_terminal"
    );
}

#[test]
fn goal_plan_create_bounds_and_changed_key_are_enforced() {
    let (_temp, db, owner) = fixture();
    let mut bad_inputs = Vec::new();
    let mut bad = input("bounds");
    bad.steps[1].id = bad.steps[0].id.clone();
    bad_inputs.push(bad);
    let mut bad = input("bounds");
    bad.steps[0].id = "x".repeat(MAX_GOAL_STEP_ID_BYTES + 1);
    bad_inputs.push(bad);
    let mut bad = input("bounds");
    bad.steps[0].title = "x".repeat(MAX_GOAL_STEP_TITLE_CHARS + 1);
    bad_inputs.push(bad);
    let mut bad = input("bounds");
    bad.steps = (0..=MAX_GOAL_STEPS)
        .map(|i| NewGoalStep {
            id: format!("s{i}"),
            title: "step".into(),
        })
        .collect();
    bad_inputs.push(bad);
    let mut bad = input("bounds");
    bad.completion_conditions = vec!["condition".into(); MAX_GOAL_COMPLETION_CONDITIONS + 1];
    bad_inputs.push(bad);
    let mut bad = input("bounds");
    bad.completion_conditions = vec!["x".repeat(MAX_GOAL_CONDITION_BYTES + 1)];
    bad_inputs.push(bad);
    for bad in bad_inputs {
        assert_eq!(
            db.create_goal_at(&owner, bad, T0).unwrap_err().code(),
            "invalid_goal_plan"
        );
    }
    let created = db.create_goal_at(&owner, input("bounds"), T0).unwrap();
    assert_eq!(created.goal.summary.revision, 1);
    assert!(
        db.create_goal_at(&owner, input("bounds"), T0 + 1)
            .unwrap()
            .replayed
    );
    let mut changed = input("bounds");
    changed.steps[0].title.push_str(" changed");
    assert_eq!(
        db.create_goal_at(&owner, changed, T0 + 2)
            .unwrap_err()
            .code(),
        "goal_idempotency_conflict"
    );
}

#[test]
fn goal_plan_malformed_persistence_fails_closed_for_reads_lists_and_mutations() {
    let (_temp, db, owner) = fixture();
    let goal = db.create_goal_at(&owner, input("create"), T0).unwrap().goal;
    let mut malformed = vec![
        "{}".to_string(),
        "null".into(),
        "not json".into(),
        "x".repeat(MAX_GOAL_PLAN_BYTES + 1),
    ];
    for mutation in ["duplicate", "status", "two_current", "timestamp", "summary"] {
        let mut value = serde_json::to_value(&goal.plan).unwrap();
        match mutation {
            "duplicate" => value["steps"][1]["id"] = value["steps"][0]["id"].clone(),
            "status" => value["steps"][0]["status"] = "unknown".into(),
            "two_current" => {
                value["steps"][0]["status"] = "in_progress".into();
                value["steps"][1]["status"] = "in_progress".into();
            }
            "timestamp" => value["steps"][0]["updated_at_unix_ms"] = (T0 + 1).into(),
            "summary" => value["progress_summary"] = "missing checkpoint time".into(),
            _ => unreachable!(),
        }
        malformed.push(value.to_string());
    }
    for raw in malformed {
        db.lock_connection(crate::StoreDomain::Goal)
            .execute(
                "UPDATE wc_goals SET plan_json = ?2 WHERE goal_id = ?1",
                params![goal.summary.goal_id, raw],
            )
            .unwrap();
        assert_eq!(
            db.read_goal(&owner, &goal.summary.goal_id)
                .unwrap_err()
                .code(),
            "goal_store_unavailable"
        );
        assert_eq!(
            db.list_goals(&owner, None, None, 0, 10).unwrap_err().code(),
            "goal_store_unavailable"
        );
        assert_eq!(
            db.checkpoint_goal_at(
                &owner,
                &goal.summary.goal_id,
                1,
                checkpoint(&[], None, "must not repair corruption"),
                "bad-store",
                T0 + 2
            )
            .unwrap_err()
            .code(),
            "goal_store_unavailable"
        );
    }
}

#[test]
fn goal_plan_revision_fence_serializes_competing_checkpoints() {
    let (_temp, db, owner) = fixture();
    let id = db
        .create_goal_at(&owner, input("create"), T0)
        .unwrap()
        .goal
        .summary
        .goal_id;
    let db = std::sync::Arc::new(db);
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
    let handles: Vec<_> = (0..2)
        .map(|i| {
            let (db, owner, id, barrier) = (db.clone(), owner.clone(), id.clone(), barrier.clone());
            std::thread::spawn(move || {
                barrier.wait();
                db.checkpoint_goal_at(
                    &owner,
                    &id,
                    1,
                    checkpoint(&[], Some("inspect"), "Start"),
                    &format!("race-{i}"),
                    T0 + 1,
                )
            })
        })
        .collect();
    let results: Vec<_> = handles
        .into_iter()
        .map(|handle| handle.join().unwrap())
        .collect();
    assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 1);
    assert_eq!(
        results
            .iter()
            .find_map(|result| result.as_ref().err())
            .unwrap()
            .code(),
        "goal_revision_changed"
    );
    assert_eq!(db.read_goal(&owner, &id).unwrap().summary.revision, 2);
}

#[test]
fn goal_plan_persisted_progress_requires_checkpoint_and_monotonic_step_time() {
    let (_temp, db, owner) = fixture();
    let goal = db
        .create_goal_at(&owner, input("progress-shape"), T0)
        .unwrap()
        .goal;
    for (status, step_time, checkpoint_time) in [
        ("completed", T0, None),
        ("in_progress", T0, None),
        ("pending", T0 + 4, Some(T0 + 2)),
        ("completed", T0 + 2, Some(T0 + 1)),
    ] {
        let mut value = serde_json::to_value(&goal.plan).unwrap();
        value["steps"][0]["status"] = status.into();
        value["steps"][0]["updated_at_unix_ms"] = step_time.into();
        if let Some(at) = checkpoint_time {
            value["progress_summary"] = "checkpoint".into();
            value["checkpoint_at_unix_ms"] = at.into();
        }
        db.lock_connection(crate::StoreDomain::Goal)
            .execute(
                "UPDATE wc_goals SET plan_json = ?2, updated_at_unix_ms = ?3 WHERE goal_id = ?1",
                params![goal.summary.goal_id, value.to_string(), T0 + 3],
            )
            .unwrap();
        assert_eq!(
            db.read_goal(&owner, &goal.summary.goal_id)
                .unwrap_err()
                .code(),
            "goal_store_unavailable"
        );
    }
}

fn replacement(steps: &[(&str, &str)], reason: &str) -> GoalPlanReplacement {
    GoalPlanReplacement {
        reason: reason.into(),
        steps: steps
            .iter()
            .map(|(id, title)| NewGoalStep {
                id: (*id).into(),
                title: (*title).into(),
            })
            .collect(),
    }
}

#[test]
fn revise_goal_plan_preserves_progress_records_prior_snapshot_and_survives_reopen() {
    let (temp, db, owner) = fixture();
    let goal = db.create_goal_at(&owner, input("create"), T0).unwrap().goal;
    let id = &goal.summary.goal_id;
    let checkpointed = db
        .checkpoint_goal_at(
            &owner,
            id,
            1,
            checkpoint(&["inspect"], Some("implement"), "Inspected"),
            "progress",
            T0 + 1,
        )
        .unwrap()
        .goal;
    let request = replacement(
        &[
            ("new", "New validation"),
            ("inspect", "inspect"),
            ("implement", "Adapt implementation"),
        ],
        "New evidence",
    );
    let revised = db
        .revise_goal_plan_at(&owner, id, 2, request.clone(), "revise", T0 + 2)
        .unwrap();
    assert_eq!(revised.goal.summary.revision, 3);
    assert_eq!(revised.goal.plan.steps[0].status, GoalStepStatus::Pending);
    assert_eq!(revised.goal.plan.steps[0].updated_at_unix_ms, T0 + 2);
    assert_eq!(revised.goal.plan.steps[1], checkpointed.plan.steps[0]);
    assert_eq!(revised.goal.plan.current_step().unwrap().id, "implement");
    assert_eq!(revised.goal.plan.steps[2].updated_at_unix_ms, T0 + 1);
    assert_eq!(
        revised.goal.plan.completion_conditions,
        checkpointed.plan.completion_conditions
    );
    assert_eq!(
        revised.goal.plan.progress_summary,
        checkpointed.plan.progress_summary
    );
    let history = &revised.goal.plan_history;
    assert_eq!(
        (
            history.total_count,
            history.returned_count,
            history.truncated
        ),
        (1, 1, false)
    );
    assert_eq!(history.revisions[0].plan, checkpointed.plan);
    assert_eq!(history.revisions[0].revision, 2);
    assert_eq!(history.revisions[0].reason, "New evidence");
    drop(db);
    let db = Database::open(&temp.path().join("goal-plan.db")).unwrap();
    assert_eq!(db.read_goal(&owner, id).unwrap(), revised.goal);
    let replay = db
        .revise_goal_plan_at(&owner, id, 2, request.clone(), "revise", T0 + 3)
        .unwrap();
    assert!(replay.replayed && !replay.state_changed);
    assert_eq!(replay.goal, revised.goal);
    let mut changed = request;
    changed.steps.reverse();
    assert_eq!(
        db.revise_goal_plan_at(&owner, id, 2, changed, "revise", T0 + 4)
            .unwrap_err()
            .code(),
        "goal_idempotency_conflict"
    );
    let removed = db
        .revise_goal_plan_at(
            &owner,
            id,
            3,
            replacement(&[("inspect", "inspect")], "Drop unfinished work"),
            "remove",
            T0 + 4,
        )
        .unwrap();
    assert!(removed.goal.plan.current_step().is_none());
    assert!(removed.goal.plan.complete());
}

#[test]
fn revise_goal_plan_invalid_replacements_and_stale_writes_are_atomic_and_private() {
    let (_temp, db, owner) = fixture();
    let id = db
        .create_goal_at(&owner, input("create"), T0)
        .unwrap()
        .goal
        .summary
        .goal_id;
    let current = db
        .checkpoint_goal_at(
            &owner,
            &id,
            1,
            checkpoint(&["inspect"], Some("implement"), "Inspected"),
            "progress",
            T0 + 1,
        )
        .unwrap()
        .goal;
    let mut invalid = vec![
        replacement(&[("implement", "implement")], "Remove completed"),
        replacement(&[("inspect", "renamed")], "Rename completed"),
        replacement(
            &[("inspect", "inspect"), ("inspect", "duplicate")],
            "Duplicate",
        ),
        replacement(&[("inspect", "inspect"), ("bad id", "bad")], "Invalid id"),
        replacement(&[("inspect", "inspect"), ("new", "")], "Empty title"),
        replacement(&[("inspect", "inspect")], " "),
        replacement(&[("inspect", "inspect")], &"界".repeat(683)),
        replacement(
            &[("inspect", "inspect"), ("new", &"界".repeat(121))],
            "Long title",
        ),
    ];
    let mut too_many = replacement(&[], "Too many");
    too_many.steps = (0..33)
        .map(|i| NewGoalStep {
            id: format!("s{i}"),
            title: "s".into(),
        })
        .collect();
    invalid.push(too_many);
    for request in invalid {
        assert_eq!(
            db.revise_goal_plan_at(&owner, &id, 2, request, "invalid", T0 + 2)
                .unwrap_err()
                .code(),
            "invalid_goal_plan"
        );
        assert_eq!(db.read_goal(&owner, &id).unwrap(), current);
    }
    let valid = replacement(&[("inspect", "inspect")], "Trim unfinished");
    let stale = db
        .revise_goal_plan_at(&owner, &id, 1, valid.clone(), "stale", T0 + 2)
        .unwrap_err();
    assert_eq!(stale.code(), "goal_revision_changed");
    assert_eq!(stale.current_revision(), Some(2));
    let foreign = CommunicationPrincipal {
        digest: format!("{COMMUNICATION_PRINCIPAL_DIGEST_PREFIX}{}", "b".repeat(64)),
        ..owner.clone()
    };
    assert_eq!(
        db.revise_goal_plan_at(&foreign, &id, 2, valid.clone(), "foreign", T0 + 2)
            .unwrap_err()
            .code(),
        "goal_not_found"
    );
    let accepted = db
        .revise_goal_plan_at(&owner, &id, 2, valid, "invalid", T0 + 2)
        .unwrap();
    assert_eq!(
        accepted.goal.plan_history.total_count, 1,
        "failed validations must not consume a key or retain history"
    );
}

#[test]
fn revise_goal_plan_history_retention_and_terminal_replay() {
    let (_temp, db, owner) = fixture();
    let id = db
        .create_goal_at(&owner, input("create"), T0)
        .unwrap()
        .goal
        .summary
        .goal_id;
    for i in 1..=12 {
        let result = db
            .revise_goal_plan_at(
                &owner,
                &id,
                i,
                replacement(&[("inspect", "inspect")], &format!("Revision {i}")),
                &format!("revision-{i}"),
                T0 + i,
            )
            .unwrap();
        let history = result.goal.plan_history;
        assert_eq!(history.total_count, i);
        assert_eq!(history.returned_count, (i as usize).min(8));
        assert_eq!(history.truncated, i > 8);
        assert_eq!(history.revisions[0].revision, i);
    }
    let stored: i64 = db
        .lock_connection(crate::StoreDomain::Goal)
        .query_row(
            "SELECT COUNT(*) FROM wc_goal_plan_revisions WHERE goal_id = ?1",
            [&id],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(stored, 8);
    let cancelled = db
        .update_goal_at(
            &owner,
            &id,
            13,
            GoalPatch {
                lifecycle: Some(GoalLifecycle::Cancelled),
                ..Default::default()
            },
            "cancel",
            T0 + 13,
        )
        .unwrap()
        .goal;
    assert_eq!(
        db.revise_goal_plan_at(
            &owner,
            &id,
            14,
            replacement(&[], "Terminal"),
            "terminal",
            T0 + 14
        )
        .unwrap_err()
        .code(),
        "goal_terminal"
    );
    let replay = db
        .revise_goal_plan_at(
            &owner,
            &id,
            1,
            replacement(&[("inspect", "inspect")], "Revision 1"),
            "revision-1",
            T0 + 15,
        )
        .unwrap();
    assert!(replay.replayed && !replay.state_changed);
    assert_eq!(
        replay.goal, cancelled,
        "pruning history must not prune replay identity"
    );
}

#[test]
fn revise_goal_plan_competing_connections_serialize_and_failed_history_insert_rolls_back() {
    let (temp, db, owner) = fixture();
    let id = db
        .create_goal_at(&owner, input("create"), T0)
        .unwrap()
        .goal
        .summary
        .goal_id;
    let other = Database::open(&temp.path().join("goal-plan.db")).unwrap();
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
    let handles: Vec<_> = [db, other]
        .into_iter()
        .enumerate()
        .map(|(i, db)| {
            let (owner, id, barrier) = (owner.clone(), id.clone(), barrier.clone());
            std::thread::spawn(move || {
                barrier.wait();
                db.revise_goal_plan_at(
                    &owner,
                    &id,
                    1,
                    replacement(&[("new", "new")], "Race"),
                    &format!("race-{i}"),
                    T0 + 1,
                )
            })
        })
        .collect();
    let results: Vec<_> = handles.into_iter().map(|h| h.join().unwrap()).collect();
    assert_eq!(results.iter().filter(|r| r.is_ok()).count(), 1);
    assert_eq!(
        results
            .iter()
            .find_map(|r| r.as_ref().err())
            .unwrap()
            .code(),
        "goal_revision_changed"
    );
    let db = Database::open(&temp.path().join("goal-plan.db")).unwrap();
    let before = db.read_goal(&owner, &id).unwrap();
    db.lock_connection(crate::StoreDomain::Goal).execute_batch("CREATE TRIGGER reject_plan_history BEFORE INSERT ON wc_goal_plan_revisions BEGIN SELECT RAISE(ABORT, 'injected failure'); END;").unwrap();
    assert_eq!(
        db.revise_goal_plan_at(
            &owner,
            &id,
            2,
            replacement(&[], "Fail atomically"),
            "failure",
            T0 + 2
        )
        .unwrap_err()
        .code(),
        "goal_store_unavailable"
    );
    assert_eq!(db.read_goal(&owner, &id).unwrap(), before);
    // Fail after plan/history mutation, proving the same transaction rolls them back.
    db.lock_connection(crate::StoreDomain::Goal).execute_batch("DROP TRIGGER reject_plan_history; CREATE TRIGGER reject_plan_replay BEFORE INSERT ON wc_goal_idempotency WHEN NEW.operation = 'revise_goal_plan' BEGIN SELECT RAISE(ABORT, 'injected replay failure'); END;").unwrap();
    assert_eq!(
        db.revise_goal_plan_at(
            &owner,
            &id,
            2,
            replacement(&[], "Fail replay atomically"),
            "late-failure",
            T0 + 2
        )
        .unwrap_err()
        .code(),
        "goal_store_unavailable"
    );
    assert_eq!(db.read_goal(&owner, &id).unwrap(), before);
}

#[test]
fn revise_goal_plan_large_history_read_budget_and_corrupt_rows_fail_closed() {
    let (_temp, db, owner) = fixture();
    let mut large = input("large");
    large.steps = (0..32)
        .map(|i| NewGoalStep {
            id: format!("step{i}"),
            title: "😀".repeat(120),
        })
        .collect();
    large.completion_conditions = vec!["c".repeat(512); 8];
    let id = db
        .create_goal_at(&owner, large.clone(), T0)
        .unwrap()
        .goal
        .summary
        .goal_id;
    for i in 1..=8 {
        db.revise_goal_plan_at(
            &owner,
            &id,
            i,
            GoalPlanReplacement {
                steps: large.steps.clone(),
                reason: "r".repeat(2048),
            },
            &format!("large-{i}"),
            T0 + i,
        )
        .unwrap();
    }
    let goal = db.read_goal(&owner, &id).unwrap();
    assert_eq!(goal.plan_history.total_count, 8);
    assert!(goal.plan_history.returned_count < 8 && goal.plan_history.returned_count > 0);
    assert!(goal.plan_history.truncated);
    let bytes: usize = goal
        .plan_history
        .revisions
        .iter()
        .map(|r| serde_json::to_vec(r).unwrap().len())
        .sum();
    assert!(bytes <= MAX_GOAL_PLAN_HISTORY_READ_BYTES);
    assert!(
        serde_json::to_vec(&goal).unwrap().len()
            < MAX_GOAL_PLAN_HISTORY_READ_BYTES + MAX_GOAL_PLAN_BYTES + 12_288
    );
    // Even an old snapshot omitted by the presentation budget must fail closed if corrupt.
    db.lock_connection(crate::StoreDomain::Goal)
        .execute(
            "UPDATE wc_goal_plan_revisions SET plan_json = ?2 WHERE goal_id = ?1 AND revision = 1",
            params![id, "x".repeat(MAX_GOAL_PLAN_BYTES + 1)],
        )
        .unwrap();
    assert_eq!(
        db.read_goal(&owner, &id).unwrap_err().code(),
        "goal_store_unavailable"
    );
}

#[test]
fn revise_goal_plan_cannot_mutate_completed_goal_or_resurrect_progress() {
    let (_temp, db, owner) = fixture();
    let id = db
        .create_goal_at(&owner, input("create"), T0)
        .unwrap()
        .goal
        .summary
        .goal_id;
    let request = replacement(&[("inspect", "inspect")], "Reduce unfinished plan");
    db.revise_goal_plan_at(&owner, &id, 1, request.clone(), "revision", T0 + 1)
        .unwrap();
    db.checkpoint_goal_at(
        &owner,
        &id,
        2,
        checkpoint(&["inspect"], None, "Validation and review passed"),
        "finish-progress",
        T0 + 2,
    )
    .unwrap();
    let terminal = db
        .update_goal_at(
            &owner,
            &id,
            3,
            GoalPatch {
                lifecycle: Some(GoalLifecycle::Completed),
                ..Default::default()
            },
            "finish",
            T0 + 3,
        )
        .unwrap()
        .goal;
    assert_eq!(
        db.revise_goal_plan_at(
            &owner,
            &id,
            4,
            replacement(&[("new", "new")], "Attempt resurrection"),
            "after-completion",
            T0 + 4
        )
        .unwrap_err()
        .code(),
        "goal_terminal"
    );
    assert_eq!(
        db.revise_goal_plan_at(&owner, &id, 1, request, "revision", T0 + 5)
            .unwrap()
            .goal,
        terminal
    );
}

#[test]
fn revise_goal_plan_additive_migration_preserves_existing_goal_without_history() {
    let (temp, db, owner) = fixture();
    let goal = db
        .create_goal_at(&owner, input("pre-history-create"), T0)
        .unwrap()
        .goal;
    // Reproduce the exact pre-revision Goal schema, retaining its canonical plan.
    db.lock_connection(crate::StoreDomain::Goal).execute_batch(
        "DROP TABLE wc_goal_plan_revisions; ALTER TABLE wc_goals DROP COLUMN plan_revision_count;"
    ).unwrap();
    drop(db);
    let db = Database::open(&temp.path().join("goal-plan.db")).unwrap();
    assert_eq!(db.read_goal(&owner, &goal.summary.goal_id).unwrap(), goal);
    let revised = db
        .revise_goal_plan_at(
            &owner,
            &goal.summary.goal_id,
            1,
            replacement(&[("new", "New step")], "Plan after upgrade"),
            "post-upgrade-revise",
            T0 + 1,
        )
        .unwrap();
    assert_eq!(revised.goal.plan_history.total_count, 1);
    assert_eq!(revised.goal.plan_history.revisions[0].plan, goal.plan);
}

#[test]
fn revise_goal_plan_reads_pin_current_plan_and_history_to_one_snapshot() {
    let (temp, db, owner) = fixture();
    let id = db
        .create_goal_at(&owner, input("create"), T0)
        .unwrap()
        .goal
        .summary
        .goal_id;
    let writer = Database::open(&temp.path().join("goal-plan.db")).unwrap();
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
    let handle = {
        let (owner, id, barrier) = (owner.clone(), id.clone(), barrier.clone());
        std::thread::spawn(move || {
            barrier.wait();
            for revision in 1..=16 {
                writer
                    .revise_goal_plan_at(
                        &owner,
                        &id,
                        revision,
                        replacement(&[("inspect", "inspect")], "Concurrent revision"),
                        &format!("revision-{revision}"),
                        T0 + revision,
                    )
                    .unwrap();
            }
        })
    };
    barrier.wait();
    for _ in 0..64 {
        let goal = db.read_goal(&owner, &id).unwrap();
        assert_eq!(goal.plan_history.total_count + 1, goal.summary.revision);
        if let Some(prior) = goal.plan_history.revisions.first() {
            assert_eq!(prior.revision + 1, goal.summary.revision);
        }
        assert_eq!(
            db.list_goals(&owner, None, None, 0, 10)
                .unwrap()
                .goals
                .len(),
            1
        );
    }
    handle.join().unwrap();
    assert_eq!(db.read_goal(&owner, &id).unwrap().summary.revision, 17);
}

#[test]
fn goal_completion_conditions_edit_preserves_progress_replay_and_restart() {
    let (temp, db, owner) = fixture();
    let id = db
        .create_goal_at(&owner, input("conditions-create"), T0)
        .unwrap()
        .goal
        .summary
        .goal_id;
    let prior = db
        .checkpoint_goal_at(
            &owner,
            &id,
            1,
            checkpoint(&["inspect"], Some("implement"), "Keep this recovery point"),
            "conditions-checkpoint",
            T0 + 1,
        )
        .unwrap()
        .goal;
    let patch = GoalPatch {
        completion_conditions: Some(vec!["  Revised acceptance  ".into(), "验证通过".into()]),
        ..Default::default()
    };
    let edited = db
        .update_goal_at(&owner, &id, 2, patch.clone(), "conditions-edit", T0 + 2)
        .unwrap();
    assert!(edited.state_changed);
    assert_eq!(edited.goal.summary.revision, 3);
    assert_eq!(edited.goal.summary.updated_at_unix_ms, T0 + 2);
    assert_eq!(
        edited.goal.plan.completion_conditions,
        ["Revised acceptance", "验证通过"]
    );
    assert_eq!(edited.goal.plan.steps, prior.plan.steps);
    assert_eq!(
        edited.goal.plan.progress_summary,
        prior.plan.progress_summary
    );
    assert_eq!(
        edited.goal.plan.checkpoint_at_unix_ms,
        prior.plan.checkpoint_at_unix_ms
    );
    let replay = db
        .update_goal_at(&owner, &id, 2, patch.clone(), "conditions-edit", T0 + 3)
        .unwrap();
    assert!(replay.replayed);
    assert!(!replay.state_changed);
    assert_eq!(replay.goal, edited.goal);
    let changed = GoalPatch {
        completion_conditions: Some(vec!["Different".into()]),
        ..Default::default()
    };
    assert_eq!(
        db.update_goal_at(&owner, &id, 2, changed.clone(), "conditions-edit", T0 + 3)
            .unwrap_err()
            .code(),
        "goal_idempotency_conflict"
    );
    assert_eq!(
        db.update_goal_at(&owner, &id, 2, changed, "conditions-stale", T0 + 3)
            .unwrap_err()
            .code(),
        "goal_revision_changed"
    );
    assert_eq!(db.read_goal(&owner, &id).unwrap(), edited.goal);
    drop(db);
    let db = Database::open(&temp.path().join("goal-plan.db")).unwrap();
    assert_eq!(db.read_goal(&owner, &id).unwrap(), edited.goal);
    assert!(
        db.update_goal_at(&owner, &id, 2, patch, "conditions-edit", T0 + 4)
            .unwrap()
            .replayed
    );
    let omitted = db
        .update_goal_at(
            &owner,
            &id,
            3,
            GoalPatch {
                title: Some("Renamed".into()),
                ..Default::default()
            },
            "conditions-omitted",
            T0 + 5,
        )
        .unwrap();
    assert_eq!(omitted.goal.plan, edited.goal.plan);
    let same = db
        .update_goal_at(
            &owner,
            &id,
            4,
            GoalPatch {
                completion_conditions: Some(vec!["Revised acceptance".into(), "验证通过".into()]),
                ..Default::default()
            },
            "conditions-same-new-key",
            T0 + 6,
        )
        .unwrap();
    assert!(!same.state_changed);
    assert!(!same.replayed);
    assert_eq!(same.goal, omitted.goal);
    let cleared = db
        .update_goal_at(
            &owner,
            &id,
            4,
            GoalPatch {
                completion_conditions: Some(vec![]),
                ..Default::default()
            },
            "conditions-clear",
            T0 + 7,
        )
        .unwrap();
    assert!(cleared.goal.plan.completion_conditions.is_empty());
    assert_eq!(cleared.goal.summary.revision, 5);
    assert_eq!(cleared.goal.plan.steps, prior.plan.steps);
    assert_eq!(
        cleared.goal.plan.progress_summary,
        prior.plan.progress_summary
    );
}

#[test]
fn goal_completion_conditions_edit_has_creation_bounds_and_atomic_failure() {
    let (_temp, db, owner) = fixture();
    let goal = db
        .create_goal_at(&owner, input("conditions-bounds-create"), T0)
        .unwrap()
        .goal;
    for (i, conditions) in [
        vec!["x".into(); MAX_GOAL_COMPLETION_CONDITIONS + 1],
        vec![" ".into()],
        vec![String::new()],
        vec!["x".repeat(MAX_GOAL_CONDITION_BYTES + 1)],
        vec!["界".repeat(MAX_GOAL_CONDITION_BYTES / 3 + 1)],
    ]
    .into_iter()
    .enumerate()
    {
        let mut create = input(&format!("conditions-invalid-{i}"));
        create.completion_conditions = conditions.clone();
        let create_code = db.create_goal_at(&owner, create, T0 + 1).unwrap_err();
        let error = db
            .update_goal_at(
                &owner,
                &goal.summary.goal_id,
                1,
                GoalPatch {
                    title: Some("Must not apply".into()),
                    completion_conditions: Some(conditions),
                    ..Default::default()
                },
                &format!("conditions-invalid-edit-{i}"),
                T0 + 1,
            )
            .unwrap_err();
        assert_eq!(error.code(), create_code.code());
        assert_eq!(db.read_goal(&owner, &goal.summary.goal_id).unwrap(), goal);
    }
    let conditions = vec!["x".repeat(MAX_GOAL_CONDITION_BYTES); MAX_GOAL_COMPLETION_CONDITIONS];
    let accepted = db
        .update_goal_at(
            &owner,
            &goal.summary.goal_id,
            1,
            GoalPatch {
                completion_conditions: Some(conditions.clone()),
                ..Default::default()
            },
            "conditions-max",
            T0 + 2,
        )
        .unwrap();
    assert_eq!(accepted.goal.plan.completion_conditions, conditions);
    // Foreign-owner edits retain existence hiding and do not mutate the owner record.
    let mut foreign = owner.clone();
    foreign.digest = format!("{COMMUNICATION_PRINCIPAL_DIGEST_PREFIX}{}", "b".repeat(64));
    assert_eq!(
        db.update_goal_at(
            &foreign,
            &goal.summary.goal_id,
            2,
            GoalPatch {
                completion_conditions: Some(vec![]),
                ..Default::default()
            },
            "conditions-foreign",
            T0 + 3
        )
        .unwrap_err()
        .code(),
        "goal_not_found"
    );
    assert_eq!(
        db.read_goal(&owner, &goal.summary.goal_id).unwrap(),
        accepted.goal
    );
}

#[test]
fn goal_completion_conditions_terminal_immutability_and_historical_replay() {
    let (_temp, db, owner) = fixture();
    for lifecycle in [GoalLifecycle::Completed, GoalLifecycle::Cancelled] {
        let mut create = input(&format!("conditions-terminal-{lifecycle:?}"));
        create.steps.clear();
        let goal = db.create_goal_at(&owner, create, T0).unwrap().goal;
        let patch = GoalPatch {
            completion_conditions: Some(vec!["Final intent".into()]),
            ..Default::default()
        };
        db.update_goal_at(
            &owner,
            &goal.summary.goal_id,
            1,
            patch.clone(),
            &format!("conditions-before-{lifecycle:?}"),
            T0 + 1,
        )
        .unwrap();
        let terminal = db
            .update_goal_at(
                &owner,
                &goal.summary.goal_id,
                2,
                GoalPatch {
                    lifecycle: Some(lifecycle),
                    ..Default::default()
                },
                &format!("conditions-finish-{lifecycle:?}"),
                T0 + 2,
            )
            .unwrap()
            .goal;
        assert_eq!(
            db.update_goal_at(
                &owner,
                &goal.summary.goal_id,
                3,
                GoalPatch {
                    completion_conditions: Some(vec![]),
                    ..Default::default()
                },
                &format!("conditions-after-{lifecycle:?}"),
                T0 + 3
            )
            .unwrap_err()
            .code(),
            "goal_terminal"
        );
        let replay = db
            .update_goal_at(
                &owner,
                &goal.summary.goal_id,
                1,
                patch,
                &format!("conditions-before-{lifecycle:?}"),
                T0 + 3,
            )
            .unwrap();
        assert!(replay.replayed);
        assert_eq!(replay.goal, terminal);
    }
}

#[test]
fn goal_completion_conditions_omission_keeps_pre_extension_replay_hash() {
    use super::store_primitives::digest_json;
    let (_temp, db, owner) = fixture();
    let goal = db
        .create_goal_at(&owner, input("conditions-old-create"), T0)
        .unwrap()
        .goal;
    let patch = GoalPatch {
        title: Some("Rename".into()),
        ..Default::default()
    };
    db.update_goal_at(
        &owner,
        &goal.summary.goal_id,
        1,
        patch.clone(),
        "conditions-old-update",
        T0 + 1,
    )
    .unwrap();
    let old_hash = digest_json("webcodex.goal.request.v2", &serde_json::json!({
        "goal_id": goal.summary.goal_id, "expected_revision": 1, "title": "Rename",
        "objective": null, "controller_agent_id": null, "lifecycle": null, "terminal_reason": null
    })).unwrap();
    // Replace the hash with the exact format produced before completion-condition edits existed.
    db.lock_connection(crate::StoreDomain::Goal).execute(
        "UPDATE wc_goal_idempotency SET request_hash = ?1 WHERE goal_id = ?2 AND operation = 'update_goal'",
        params![old_hash, goal.summary.goal_id]).unwrap();
    assert!(
        db.update_goal_at(
            &owner,
            &goal.summary.goal_id,
            1,
            patch,
            "conditions-old-update",
            T0 + 2
        )
        .unwrap()
        .replayed
    );
}

#[test]
fn completion_conditions_and_plan_revision_preserve_each_others_history_and_progress() {
    let (_temp, db, owner) = fixture();
    let created = db
        .create_goal_at(&owner, input("cross-operation"), T0)
        .unwrap()
        .goal;
    let id = &created.summary.goal_id;
    let checkpointed = db
        .checkpoint_goal_at(
            &owner,
            id,
            1,
            checkpoint(&["inspect"], Some("implement"), "Inspected"),
            "cross-checkpoint",
            T0 + 1,
        )
        .unwrap()
        .goal;
    let conditions = vec!["Review and validation required".to_string()];
    let edited = db
        .update_goal_at(
            &owner,
            id,
            2,
            GoalPatch {
                completion_conditions: Some(conditions.clone()),
                ..Default::default()
            },
            "cross-conditions",
            T0 + 2,
        )
        .unwrap()
        .goal;
    assert_eq!(edited.plan.steps, checkpointed.plan.steps);
    let revised = db
        .revise_goal_plan_at(
            &owner,
            id,
            3,
            replacement(
                &[
                    ("inspect", "inspect"),
                    ("implement", "Implement safely"),
                    ("new", "New step"),
                ],
                "Revise unfinished work",
            ),
            "cross-revision",
            T0 + 3,
        )
        .unwrap()
        .goal;
    assert_eq!(revised.plan.completion_conditions, conditions);
    assert_eq!(revised.plan.steps[0], checkpointed.plan.steps[0]);
    assert_eq!(revised.plan_history.total_count, 1);
    assert_eq!(revised.plan_history.revisions[0].plan, edited.plan);
    assert_eq!(revised.plan_history.revisions[0].revision, 3);
    let after = db
        .update_goal_at(
            &owner,
            id,
            4,
            GoalPatch {
                completion_conditions: Some(vec!["Final validation".to_string()]),
                ..Default::default()
            },
            "cross-after-revision",
            T0 + 4,
        )
        .unwrap()
        .goal;
    assert_eq!(after.plan_history, revised.plan_history);
    assert_eq!(after.plan.steps, revised.plan.steps);
    assert_eq!(after.plan.completion_conditions, ["Final validation"]);
    assert_eq!(after.summary.revision, 5);
    assert!(db
        .revise_goal_plan_at(
            &owner,
            id,
            3,
            replacement(&[("inspect", "inspect")], "Stale revision"),
            "cross-stale",
            T0 + 5,
        )
        .is_err());
}
