use super::super::tests::{register_project, test_runtime_with_goal_db};
use super::*;

#[tokio::test]
async fn goal_resources_reuse_is_request_local_and_principal_scoped() {
    let (_tmp, _db, runtime) = test_runtime_with_goal_db();
    let owner = crate::auth::shared_key_context("goal-resource-owner");
    let other = crate::auth::shared_key_context("goal-resource-other");
    let project = "agent:goal-resources:demo";
    register_project(
        &runtime,
        "goal-resources",
        "demo",
        "/private/demo",
        Some(&owner),
    )
    .await;
    let session = runtime
        .sessions
        .start_session(Some(project.into()), Some("Shared Session".into()));
    let task = runtime.create_agent_task(
        Some(&owner),
        "Shared Task".into(),
        "Read-only fixture".into(),
        None,
        None,
        None,
        Some(project.into()),
        "goal-resource-task".into(),
    );
    assert!(task.success, "{:?}", task.output);
    let task_id = task.output["task"]["summary"]["task_id"].as_str().unwrap();
    let correlations = json!({"correlations": [
        {"kind": "workflow_session", "reference_id": session.session_id},
        {"kind": "agent_task", "reference_id": task_id},
    ]});
    let mut resources = GoalResources::new(&runtime, &owner);
    for _ in 0..2 {
        assert_eq!(
            project_ids_for_goal(&mut resources, &correlations).await,
            BTreeSet::from([project.to_string()])
        );
    }
    assert_eq!(resources.sessions.len(), 1);
    assert_eq!(resources.tasks.len(), 1);
    assert_eq!(resources.projects.len(), 1);
    assert_eq!(
        resources
            .session(&session.session_id)
            .await
            .unwrap()
            .session
            .lifecycle,
        "active"
    );

    runtime.sessions.close_session(&session.session_id).unwrap();
    // Repeated reads in this response reuse the authorized observation.
    assert_eq!(
        resources
            .session(&session.session_id)
            .await
            .unwrap()
            .session
            .lifecycle,
        "active"
    );
    // A later poll observes current state, never a process-wide/TTL cache.
    let mut next_request = GoalResources::new(&runtime, &owner);
    assert_eq!(
        next_request
            .session(&session.session_id)
            .await
            .unwrap()
            .session
            .lifecycle,
        "closed"
    );

    let mut foreign_request = GoalResources::new(&runtime, &other);
    assert!(project_ids_for_goal(&mut foreign_request, &correlations)
        .await
        .is_empty());
    assert!(foreign_request.session(&session.session_id).await.is_none());
    assert!(foreign_request.task(task_id).await.is_none());
    assert!(foreign_request.project(project).await.is_none());
}

#[tokio::test]
async fn goal_resources_bound_retained_observations_on_large_lists() {
    let (_tmp, _db, runtime) = test_runtime_with_goal_db();
    let auth = crate::auth::shared_key_context("goal-resource-bounds");
    let mut resources = GoalResources::new(&runtime, &auth);
    let limit = webcodex_store::MAX_GOAL_CORRELATIONS as usize;
    for index in 0..limit * 2 + 1 {
        let id = format!("missing-{index}");
        assert!(resources.session(&id).await.is_none());
        assert!(resources.task(&id).await.is_none());
        assert!(resources.project(&id).await.is_none());
        assert!(resources.sessions.len() <= limit);
        assert!(resources.tasks.len() <= limit);
        assert!(resources.projects.len() <= limit);
    }
}
