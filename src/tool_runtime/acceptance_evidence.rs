//! Advisory readiness for task acceptance, derived from existing observations.
//! This is neither an execution state nor a natural-language requirements judge.
use super::closeout_facts::{evaluate_closeout, CloseoutFacts, IntegrityStatus, ValidationStatus};
use serde_json::{json, Value};
use webcodex_core::task_outputs::TaskOutputs;

#[derive(Default)]
pub(crate) struct AcceptanceContext<'a> {
    pub(crate) outputs: Option<&'a TaskOutputs>,
    pub(crate) outputs_historical: bool,
    pub(crate) history_partial: bool,
    pub(crate) snapshot_changed: bool,
}

/// References are names of existing evidence sections, never copied logs or a
/// second persisted ledger. Callers retain the canonical suggested next actions.
pub(crate) fn acceptance_evidence(facts: &CloseoutFacts, context: AcceptanceContext<'_>) -> Value {
    let decision = evaluate_closeout(facts, &[]);
    let mut reasons = Vec::new();
    let mut references = Vec::new();
    if facts.workspace.observed().is_some() {
        references.push("workspace");
    } else {
        reasons.push("workspace_unobserved");
    }
    if facts.validation.successes > 0 || facts.validation.failures > 0 {
        references.push("validation");
    }
    if facts.review_evidence_total > 0 {
        references.push("review_evidence");
    } else {
        reasons.push("review_missing");
    }
    // Never fall back to historical `status=passed`: only an explicit current
    // source result can make evidence ready. Today's source fences deliberately
    // return unproven, even after an otherwise successful validation execution.
    match facts.validation.current_evidence_status {
        Some(ValidationStatus::Passed) => {}
        Some(ValidationStatus::Failed) => reasons.push("validation_failed"),
        Some(ValidationStatus::Stale) => reasons.push("validation_stale"),
        Some(ValidationStatus::Unproven) => reasons.push("source_unproven"),
        Some(ValidationStatus::NotRun) => reasons.push("validation_not_run"),
        Some(ValidationStatus::Inconclusive) => reasons.push("validation_inconclusive"),
        _ => reasons.push("validation_unknown"),
    }
    if facts.validation.evidence_gap_count > 0 {
        reasons.push("validation_evidence_incomplete");
    }
    if facts.validation.cargo_test_zero_tests {
        reasons.push("zero_tests");
    }
    let outputs = context.outputs.filter(|outputs| outputs.valid());
    if outputs.is_some() {
        references.push("task_outputs");
    }
    let outputs_failed = outputs.is_some_and(super::task_outputs::task_outputs_block_closeout);
    if context.outputs.is_some() && outputs.is_none() {
        reasons.push("outputs_invalid");
    } else if context.outputs_historical && outputs.is_some() {
        reasons.push("outputs_historical");
    } else if outputs_failed {
        reasons.push("outputs_unverified");
    }
    if context.history_partial {
        reasons.push("history_partial");
    }
    if context.snapshot_changed {
        reasons.push("snapshot_changed");
    }
    if decision.legacy_blocking() {
        reasons.push("closeout_blocked");
    } else if decision.integrity_status != IntegrityStatus::Clean {
        reasons.push("integrity_warning");
    }
    let state = if context.snapshot_changed {
        "needs_evidence"
    } else if decision.legacy_blocking()
        || (outputs_failed && !context.outputs_historical)
        || (facts.validation.current_evidence_status == Some(ValidationStatus::Failed)
            && facts.validation.current_unresolved_failure_count > 0)
    {
        "failed"
    } else if reasons.is_empty() {
        "ready_for_review"
    } else {
        "needs_evidence"
    };
    json!({"state": state, "reason_codes": reasons, "evidence_refs": references})
}

pub(crate) fn no_session_acceptance_evidence() -> Value {
    json!({"state": "not_applicable", "reason_codes": ["no_task_session"], "evidence_refs": []})
}

#[cfg(test)]
mod tests;
