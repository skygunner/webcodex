//! Bounded mechanical progress for a durable Goal. The plan is not a
//! scheduler, and natural-language completion intent is never evaluated here.
use super::goal::GoalStoreError;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

pub const MAX_GOAL_STEPS: usize = 32;
pub const MAX_GOAL_STEP_ID_BYTES: usize = 32;
pub const MAX_GOAL_STEP_TITLE_CHARS: usize = 120;
pub const MAX_GOAL_COMPLETION_CONDITIONS: usize = 8;
pub const MAX_GOAL_CONDITION_BYTES: usize = 512;
pub const MAX_GOAL_PROGRESS_SUMMARY_BYTES: usize = 2_048;
pub const MAX_GOAL_PLAN_BYTES: usize = 32_768;
pub const MAX_GOAL_PLAN_REVISIONS: usize = 8;
pub const MAX_GOAL_PLAN_REVISION_REASON_BYTES: usize = 2_048;
pub const MAX_GOAL_PLAN_HISTORY_READ_BYTES: usize = 65_536;

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct GoalPlanRevision {
    pub revision: i64,
    pub revised_at_unix_ms: i64,
    pub reason: String,
    pub plan: GoalPlan,
}

#[derive(Debug, Clone, Default, Serialize, PartialEq, Eq)]
pub struct GoalPlanHistory {
    pub total_count: i64,
    pub returned_count: usize,
    pub truncated: bool,
    pub revisions: Vec<GoalPlanRevision>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct GoalPlanReplacement {
    pub reason: String,
    pub steps: Vec<NewGoalStep>,
}

impl GoalPlanReplacement {
    pub(super) fn normalized(mut self) -> Result<Self, GoalStoreError> {
        self.reason = self.reason.trim().to_string();
        if self.reason.is_empty() || self.reason.len() > MAX_GOAL_PLAN_REVISION_REASON_BYTES {
            return Err(plan_error(
                "Revision reason must contain 1..=2048 UTF-8 bytes",
            ));
        }
        // Reuse the canonical step bounds and normalization before hashing or mutation.
        self.steps = GoalPlan::new(Vec::new(), self.steps, 0)?
            .steps
            .into_iter()
            .map(|step| NewGoalStep {
                id: step.id,
                title: step.title,
            })
            .collect();
        Ok(self)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct NewGoalStep {
    pub id: String,
    pub title: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum GoalStepStatus {
    Pending,
    InProgress,
    Completed,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct GoalStep {
    pub id: String,
    pub title: String,
    pub status: GoalStepStatus,
    pub updated_at_unix_ms: i64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct GoalPlan {
    pub completion_conditions: Vec<String>,
    pub steps: Vec<GoalStep>,
    pub progress_summary: Option<String>,
    pub checkpoint_at_unix_ms: Option<i64>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct GoalCheckpoint {
    pub completed_step_ids: Vec<String>,
    pub current_step_id: Option<String>,
    pub summary: String,
}

fn plan_error(message: &str) -> GoalStoreError {
    GoalStoreError::new("invalid_goal_plan", message)
}

fn checkpoint_error(message: &str) -> GoalStoreError {
    GoalStoreError::new("invalid_goal_checkpoint", message)
}

fn valid_step_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= MAX_GOAL_STEP_ID_BYTES
        && id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
}

impl GoalCheckpoint {
    pub(super) fn normalized(mut self) -> Result<Self, GoalStoreError> {
        if self.completed_step_ids.len() > MAX_GOAL_STEPS {
            return Err(checkpoint_error("Too many completed_step_ids"));
        }
        let mut ids = BTreeSet::new();
        for id in &self.completed_step_ids {
            if !valid_step_id(id) || !ids.insert(id.as_str()) {
                return Err(checkpoint_error(
                    "completed_step_ids must be valid, unique stable step ids",
                ));
            }
        }
        if let Some(id) = self.current_step_id.as_deref() {
            if !valid_step_id(id) || ids.contains(id) {
                return Err(checkpoint_error(
                    "current_step_id must be valid and cannot also be completed in the same checkpoint",
                ));
            }
        }
        self.summary = self.summary.trim().to_string();
        if self.summary.is_empty() || self.summary.len() > MAX_GOAL_PROGRESS_SUMMARY_BYTES {
            return Err(checkpoint_error(
                "summary must contain 1..=2048 UTF-8 bytes",
            ));
        }
        Ok(self)
    }
}

impl GoalPlan {
    pub(super) fn new(
        conditions: Vec<String>,
        steps: Vec<NewGoalStep>,
        now: i64,
    ) -> Result<Self, GoalStoreError> {
        let plan = Self {
            completion_conditions: normalize_completion_conditions(conditions)?,
            steps: steps
                .into_iter()
                .map(|step| GoalStep {
                    id: step.id,
                    title: step.title.trim().to_string(),
                    status: GoalStepStatus::Pending,
                    updated_at_unix_ms: now,
                })
                .collect(),
            progress_summary: None,
            checkpoint_at_unix_ms: None,
        };
        plan.validate_persisted(now, now)?;
        plan.persisted_json()?;
        Ok(plan)
    }

    pub fn complete(&self) -> bool {
        self.steps
            .iter()
            .all(|step| step.status == GoalStepStatus::Completed)
    }

    pub fn current_step(&self) -> Option<&GoalStep> {
        self.steps
            .iter()
            .find(|step| step.status == GoalStepStatus::InProgress)
    }

    pub fn completed_count(&self) -> usize {
        self.steps
            .iter()
            .filter(|step| step.status == GoalStepStatus::Completed)
            .count()
    }

    pub(super) fn checkpoint(
        &self,
        checkpoint: &GoalCheckpoint,
        now: i64,
    ) -> Result<Self, GoalStoreError> {
        // No partial mutation even in memory: validate every selector first.
        for id in checkpoint
            .completed_step_ids
            .iter()
            .chain(checkpoint.current_step_id.iter())
        {
            if !self.steps.iter().any(|step| &step.id == id) {
                return Err(checkpoint_error("Checkpoint names an unknown step id"));
            }
        }
        let mut plan = self.clone();
        for step in &mut plan.steps {
            if checkpoint.completed_step_ids.contains(&step.id)
                && step.status != GoalStepStatus::Completed
            {
                step.status = GoalStepStatus::Completed;
                step.updated_at_unix_ms = now;
            }
            if checkpoint.current_step_id.as_ref() == Some(&step.id) {
                if step.status == GoalStepStatus::Completed {
                    return Err(checkpoint_error("A completed step cannot regress"));
                }
                if step.status != GoalStepStatus::InProgress {
                    step.status = GoalStepStatus::InProgress;
                    step.updated_at_unix_ms = now;
                }
            }
        }
        if plan
            .steps
            .iter()
            .filter(|step| step.status == GoalStepStatus::InProgress)
            .count()
            > 1
        {
            return Err(checkpoint_error(
                "Complete the current step in the same batch before starting another",
            ));
        }
        plan.progress_summary = Some(checkpoint.summary.clone());
        plan.checkpoint_at_unix_ms = Some(now);
        plan.persisted_json()?;
        Ok(plan)
    }

    pub(super) fn replace_steps(
        &self,
        replacement: &GoalPlanReplacement,
        now: i64,
    ) -> Result<Self, GoalStoreError> {
        for completed in self
            .steps
            .iter()
            .filter(|step| step.status == GoalStepStatus::Completed)
        {
            if !replacement
                .steps
                .iter()
                .any(|step| step.id == completed.id && step.title == completed.title)
            {
                return Err(plan_error(
                    "Completed steps must retain their exact id and title",
                ));
            }
        }
        let mut plan = self.clone();
        plan.steps = replacement
            .steps
            .iter()
            .map(|input| {
                if let Some(existing) = self.steps.iter().find(|step| step.id == input.id) {
                    GoalStep {
                        title: input.title.clone(),
                        ..existing.clone()
                    }
                } else {
                    GoalStep {
                        id: input.id.clone(),
                        title: input.title.clone(),
                        status: GoalStepStatus::Pending,
                        updated_at_unix_ms: now,
                    }
                }
            })
            .collect();
        plan.persisted_json()?;
        Ok(plan)
    }

    pub(super) fn persisted_json(&self) -> Result<String, GoalStoreError> {
        let json = serde_json::to_string(self)
            .map_err(|_| plan_error("Goal plan serialization failed"))?;
        if json.len() > MAX_GOAL_PLAN_BYTES {
            return Err(plan_error("Goal plan exceeds the encoded byte bound"));
        }
        Ok(json)
    }

    pub(super) fn from_persisted(raw: &str) -> Result<Self, GoalStoreError> {
        if raw.len() > MAX_GOAL_PLAN_BYTES {
            return Err(plan_error("Persisted Goal plan exceeds its bound"));
        }
        // Option fields are still required in the sole persisted encoding.
        let value: serde_json::Value =
            serde_json::from_str(raw).map_err(|_| plan_error("Malformed persisted Goal plan"))?;
        let object = value
            .as_object()
            .ok_or_else(|| plan_error("Malformed persisted Goal plan"))?;
        if object.len() != 4
            || [
                "completion_conditions",
                "steps",
                "progress_summary",
                "checkpoint_at_unix_ms",
            ]
            .iter()
            .any(|field| !object.contains_key(*field))
        {
            return Err(plan_error("Malformed persisted Goal plan fields"));
        }
        // Deserialize the original text so duplicate fields also fail closed.
        serde_json::from_str(raw).map_err(|_| plan_error("Malformed persisted Goal plan"))
    }

    pub(super) fn validate_persisted(
        &self,
        created_at: i64,
        updated_at: i64,
    ) -> Result<(), GoalStoreError> {
        if self.steps.len() > MAX_GOAL_STEPS {
            return Err(plan_error(
                "Goal plan exceeds step or completion-condition bounds",
            ));
        }
        validate_completion_conditions(&self.completion_conditions)?;
        let mut ids = BTreeSet::new();
        let mut in_progress = 0;
        for step in &self.steps {
            if !valid_step_id(&step.id) || !ids.insert(step.id.as_str()) {
                return Err(plan_error("Goal step ids must be valid and unique"));
            }
            if step.title.is_empty()
                || step.title.chars().count() > MAX_GOAL_STEP_TITLE_CHARS
                || step.title.trim() != step.title
            {
                return Err(plan_error(
                    "Goal step title must contain 1..=120 characters",
                ));
            }
            if step.updated_at_unix_ms < created_at || step.updated_at_unix_ms > updated_at {
                return Err(plan_error(
                    "Goal step timestamp is outside its Goal revision",
                ));
            }
            if step.status != GoalStepStatus::Pending
                && self
                    .checkpoint_at_unix_ms
                    .is_none_or(|checkpoint| step.updated_at_unix_ms > checkpoint)
            {
                return Err(plan_error(
                    "Progress requires a durable checkpoint at or after the step transition",
                ));
            }
            in_progress += usize::from(step.status == GoalStepStatus::InProgress);
        }
        if in_progress > 1 {
            return Err(plan_error("At most one Goal step may be in_progress"));
        }
        match (&self.progress_summary, self.checkpoint_at_unix_ms) {
            (None, None) => {}
            (Some(summary), Some(at))
                if !summary.is_empty()
                    && summary.len() <= MAX_GOAL_PROGRESS_SUMMARY_BYTES
                    && summary.trim() == summary
                    && at >= created_at
                    && at <= updated_at => {}
            _ => return Err(plan_error("Invalid persisted Goal checkpoint")),
        }
        Ok(())
    }
}

// Creation and revision-fenced edits share normalization and UTF-8 byte bounds.
pub(super) fn normalize_completion_conditions(
    conditions: Vec<String>,
) -> Result<Vec<String>, GoalStoreError> {
    let conditions: Vec<_> = conditions
        .into_iter()
        .map(|value| value.trim().to_string())
        .collect();
    validate_completion_conditions(&conditions)?;
    Ok(conditions)
}

fn validate_completion_conditions(conditions: &[String]) -> Result<(), GoalStoreError> {
    if conditions.len() > MAX_GOAL_COMPLETION_CONDITIONS {
        return Err(plan_error("Goal plan exceeds completion-condition bounds"));
    }
    for condition in conditions {
        if condition.is_empty()
            || condition.len() > MAX_GOAL_CONDITION_BYTES
            || condition.trim() != condition
        {
            return Err(plan_error("Invalid bounded completion condition"));
        }
    }
    Ok(())
}
