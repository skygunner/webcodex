//! Operator diagnostic selection, never a Window/Session authority grant.
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ToolTraceQuery {
    /// Exact observed Window key. Omit to discover candidates by time/Project/tool.
    #[serde(default)]
    #[schemars(length(min = 64, max = 64))]
    pub window_key: Option<String>,
    /// Exact canonical Project id; paths and short Project refs are not query aliases.
    #[serde(default)]
    #[schemars(length(min = 1, max = 512))]
    pub project: Option<String>,
    /// Exact canonical operation name.
    #[serde(default)]
    #[schemars(length(min = 1, max = 128))]
    pub tool_name: Option<String>,
    /// Inclusive Unix milliseconds; omission starts 24 hours before until_ms.
    #[serde(default)]
    #[schemars(range(min = 0))]
    pub since_ms: Option<i64>,
    /// Inclusive Unix milliseconds; omission uses the query's observed current time.
    /// Copy the returned effective range when paging. Maximum span is 31 days.
    #[serde(default)]
    #[schemars(range(min = 0))]
    pub until_ms: Option<i64>,
    /// Include passive UI polling and support calls. Default false.
    #[serde(default)]
    pub include_nonmeaningful: bool,
}
