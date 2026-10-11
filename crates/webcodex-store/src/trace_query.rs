//! Read-only bounded diagnostic index over ActionAudit, not another trace store.
use crate::Database;
use rusqlite::params;
use serde::Serialize;

pub struct ToolTraceQueryFilter<'a> {
    pub window_key: Option<&'a str>,
    pub project: Option<&'a str>,
    pub tool_name: Option<&'a str>,
    pub since_ms: i64,
    pub until_ms: i64,
    pub include_nonmeaningful: bool,
    pub offset: usize,
    pub limit: usize,
}

#[derive(Debug, Serialize)]
pub struct ToolTraceCallRecord {
    pub trace_ref: String,
    pub window_key: Option<String>,
    pub window_source: Option<String>,
    pub project: Option<String>,
    pub tool_name: Option<String>,
    pub method: String,
    pub observed_at_ms: i64,
    pub response_handed_at_ms: Option<i64>,
    pub duration_ms: i64,
    pub status: String,
}

impl Database {
    /// The caller must hold administrator diagnostic authority. This index is
    /// not an authority oracle; UI/model adapters share the same authorized query.
    pub fn query_tool_trace_calls(
        &self,
        query: &ToolTraceQueryFilter<'_>,
    ) -> anyhow::Result<Vec<ToolTraceCallRecord>> {
        anyhow::ensure!(
            query.since_ms >= 0
                && query.until_ms >= query.since_ms
                && query.until_ms - query.since_ms <= 31 * 86_400_000,
            "invalid trace time range"
        );
        anyhow::ensure!(
            query.offset <= 10_000 && query.limit <= 65,
            "invalid trace query bounds"
        );
        let conn = self.lock_connection(crate::StoreDomain::Audit);
        let mut statement = conn.prepare(
            "SELECT server_trace_id, client_window_key, client_window_source,
                    project, operation, action_name,
                    COALESCE(request_observed_at_ms, window_started_at_ms, started_at * 1000),
                    response_handed_at_ms, duration_ms, status
             FROM action_events
             WHERE server_trace_id IS NOT NULL
               AND COALESCE(request_observed_at_ms, window_started_at_ms, started_at * 1000) >= ?1
               AND COALESCE(request_observed_at_ms, window_started_at_ms, started_at * 1000) <= ?2
               AND (?3 IS NULL OR client_window_key = ?3)
               AND (?4 IS NULL OR project = ?4)
               AND (?5 IS NULL OR operation = ?5)
               AND (?6 OR window_meaningful = 1)
             ORDER BY COALESCE(request_observed_at_ms, window_started_at_ms, started_at * 1000) DESC, event_id DESC
             LIMIT ?7 OFFSET ?8",
        )?;
        let rows = statement.query_map(
            params![
                query.since_ms,
                query.until_ms,
                query.window_key,
                query.project,
                query.tool_name,
                query.include_nonmeaningful,
                query.limit as i64,
                query.offset as i64
            ],
            |row| {
                Ok(ToolTraceCallRecord {
                    trace_ref: row.get(0)?,
                    window_key: row.get(1)?,
                    window_source: row.get(2)?,
                    project: row.get(3)?,
                    tool_name: row.get(4)?,
                    method: row.get(5)?,
                    observed_at_ms: row.get(6)?,
                    response_handed_at_ms: row.get(7)?,
                    duration_ms: row.get(8)?,
                    status: row.get(9)?,
                })
            },
        )?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(Into::into)
    }
}
