use super::*;
use crate::tool_runtime::closeout_facts::{Observation, WorkspaceFacts};
use webcodex_core::task_outputs::{TaskOutput, TaskOutputStatus};

fn facts(status: Option<ValidationStatus>) -> CloseoutFacts {
    let mut facts = CloseoutFacts::default();
    facts.workspace = Observation::Observed(WorkspaceFacts {
        clean: Some(true),
        conflicts: Some(0),
        non_git: false,
    });
    facts.review_evidence_total = 1;
    facts.validation.current_evidence_status = status;
    facts.validation.status = Some(ValidationStatus::Passed);
    facts.validation.successes = 1;
    facts
}

#[test]
fn successful_execution_and_historical_checks_never_certify_requirements() {
    for (status, reason) in [
        (None, "validation_unknown"),
        (Some(ValidationStatus::NotRun), "validation_not_run"),
        (Some(ValidationStatus::Stale), "validation_stale"),
        (Some(ValidationStatus::Unproven), "source_unproven"),
        (
            Some(ValidationStatus::Inconclusive),
            "validation_inconclusive",
        ),
    ] {
        let output = acceptance_evidence(&facts(status), AcceptanceContext::default());
        assert_eq!(output["state"], "needs_evidence", "{output}");
        assert!(output["reason_codes"]
            .as_array()
            .unwrap()
            .contains(&json!(reason)));
        assert!(!output.to_string().contains("verified"));
    }
}

#[test]
fn current_failures_and_snapshot_races_are_distinct() {
    let mut failed = facts(Some(ValidationStatus::Failed));
    failed.validation.current_unresolved_failure_count = 1;
    assert_eq!(
        acceptance_evidence(&failed, AcceptanceContext::default())["state"],
        "failed"
    );
    let raced = acceptance_evidence(
        &failed,
        AcceptanceContext {
            snapshot_changed: true,
            ..Default::default()
        },
    );
    assert_eq!(raced["state"], "needs_evidence");
    assert!(raced["reason_codes"]
        .as_array()
        .unwrap()
        .contains(&json!("snapshot_changed")));
    // Historical failures resolved by a later execution remain history. The
    // source still needs review; they do not fabricate a current failure.
    failed.validation.current_evidence_status = Some(ValidationStatus::Unproven);
    failed.validation.current_unresolved_failure_count = 0;
    failed.validation.historical_failures_resolved = true;
    assert_eq!(
        acceptance_evidence(&failed, AcceptanceContext::default())["state"],
        "needs_evidence"
    );
}

#[test]
fn missing_outputs_block_but_retained_outputs_do_not_claim_freshness() {
    let outputs = TaskOutputs {
        items: vec![TaskOutput {
            path: "dist/report.txt".into(),
            status: TaskOutputStatus::Missing,
            file_bytes: None,
            sha256: None,
            mime_type: None,
        }],
        verified_count: 0,
        missing_count: 1,
        unavailable_count: 0,
        observed_at: 1,
    };
    let current = acceptance_evidence(
        &facts(Some(ValidationStatus::Unproven)),
        AcceptanceContext {
            outputs: Some(&outputs),
            ..Default::default()
        },
    );
    assert_eq!(current["state"], "failed");
    let restored = acceptance_evidence(
        &facts(Some(ValidationStatus::Unproven)),
        AcceptanceContext {
            outputs: Some(&outputs),
            outputs_historical: true,
            ..Default::default()
        },
    );
    assert_eq!(restored["state"], "needs_evidence");
    assert!(restored["reason_codes"]
        .as_array()
        .unwrap()
        .contains(&json!("outputs_historical")));
}

#[test]
fn zero_tests_incomplete_history_and_read_only_work_remain_explicit() {
    let mut facts = facts(Some(ValidationStatus::NotRun));
    // A read-only review is allowed without tests, but its applicability must
    // be judged by the model/human rather than inferred from a successful read.
    assert_eq!(
        acceptance_evidence(&facts, AcceptanceContext::default())["state"],
        "needs_evidence"
    );
    facts.validation.cargo_test_zero_tests = true;
    facts.validation.evidence_gap_count = 1;
    let output = acceptance_evidence(
        &facts,
        AcceptanceContext {
            history_partial: true,
            ..Default::default()
        },
    );
    for reason in [
        "zero_tests",
        "history_partial",
        "validation_evidence_incomplete",
    ] {
        assert!(output["reason_codes"]
            .as_array()
            .unwrap()
            .contains(&json!(reason)));
    }
    assert_eq!(no_session_acceptance_evidence()["state"], "not_applicable");
    assert!(output.to_string().len() < 1024);
}

#[test]
fn readiness_is_advisory_and_schema_is_closed() {
    let output = acceptance_evidence(
        &facts(Some(ValidationStatus::Passed)),
        AcceptanceContext::default(),
    );
    assert_eq!(output["state"], "ready_for_review");
    assert!(output.get("accepted").is_none());
    for tool in ["finish_coding_task", "read_session_handoff"] {
        let spec = crate::tool_runtime::registered_tool_specs()
            .into_iter()
            .find(|s| s.name == tool)
            .unwrap();
        let schema = &spec.output_schema["properties"]["output"]["properties"]["acceptance"];
        assert_eq!(schema["additionalProperties"], false);
        assert!(
            crate::tool_runtime::startup_brief::validate_schema_instance_for_test(&output, schema)
                .is_ok(),
            "{tool}: {schema}"
        );
        let mut invalid = output.clone();
        invalid["state"] = json!("accepted");
        assert!(
            crate::tool_runtime::startup_brief::validate_schema_instance_for_test(&invalid, schema)
                .is_err()
        );
    }
}
