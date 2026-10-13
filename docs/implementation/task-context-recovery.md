# Bounded task context and repeated-failure recovery

[中文](task-context-recovery.zh-CN.md)

Coding startup exposes recovery pointers at `continuation.task`; detailed
continuation feedback exposes the same projection at `attempt.task_context`.
`read_session_handoff` and `finish_coding_task` include these fields in
`handoff_brief.task`. These are read-only projections of retained evidence, not a
new task database, execution loop, progress verdict, or permission grant.

| Field | Meaning and source |
| --- | --- |
| `scope` | Exact Session, Project and retained task-instruction event; `complete=false` when the attempt boundary or safe scope is unavailable. |
| `relevant_files` | Up to six unique project-relative paths from successful canonical events in this attempt, with `event_id` and changed/observed source. |
| `failure_locations` | Up to three safe test/diagnostic names and optional file/line, with references into canonical continuation validation evidence. |
| `decisions`, `recent_progress` | Retained explicitly recorded Session notes with message IDs. Notes may inform later tasks in the same Session; they do not prove execution or completion. |
| `next_steps` | Fixed advisory actions with bounded `source_refs` identifying their event, Session, or validation evidence. |
| `continuation_entry` | Explicit `read_session_handoff` entry for this Session. It never selects a different Session implicitly. |
| `stall` | Advisory bounded repeated-failure diagnostic, separate from progress, validation and closeout. |

On resume, startup still describes the previous attempt captured before the new
accepted instruction. Its explicit boundary identifies that previous task.
Current handoff assembly reuses context only for the same Session, Project and
task boundary. A snapshot race disables the diagnostic and recommends rereading
the exact handoff before dependent work.

Every recovery list includes `total`, `returned`, and `truncated`. Task context
has a 4 KiB serialized JSON envelope within the existing startup budget. Handoff
keeps its existing 8 KiB hard limit, including the new fields. Safety and byte
budget omissions are explicit; omitted paths or notes are not a complete
transcript. Unsafe/oversized scope metadata is omitted rather than shortened into
a different identity. Resolve `event:` and `session:` references against the exact
Session ledger; validation references point into `continuation_feedback` from
explicit detailed handoff recovery (`diagnostic=true`). No file bodies, symbols
inferred from file text, argv, stdout/stderr, or new fingerprints are exposed.

The diagnostic examines at most the latest 20 relevant canonical finished events
within the current attempt and exact Session/Project. Polls, heartbeat calls,
pagination, canonical support/transport activity and the closeout projection are
excluded; recovery reads do not manufacture new evidence. Three distinct unexpected actionable failures can report
`suspected` only when both their canonical execution/target hashes and complete
structured failure identities match, with no intervening relevant observation or
success. Recorder/business duplicates are canonicalized first; replayed event,
call, or invocation identities cannot satisfy the threshold. Expected, resolved
or stale failures do not qualify. A different target or structured failure breaks
the run. Parser identities retain test names/failure locations and exclude
incidental elapsed time and log noise.

Ordinary command previews and redacted/truncated input summaries cannot prove
identical argv. The hash must come from a unique earlier start event paired by
exact call ID, Session/Project/tool and logical metadata inside this task; no
legacy timestamp fallback is used. Missing/ambiguous/cross-boundary correlation,
missing canonical identity, unsupported or incomplete diagnostic
evidence, or a missing attempt boundary returns `unavailable`. An absent or
unchanged lossy log never substitutes for structured failure proof. Some adapters
provide counts without failure identities; those are conservatively unavailable.
`not_observed` means this bounded window did not satisfy the threshold, not proof
that the task is making progress.

An actionable uncertain execution outcome anywhere in that window reports
`reconcile_required`, even after a later file read. Both briefs replace executable
suggestions with reconciliation of the exact Job/execution before considering
another execution. Canonical current-evidence reconciliation can exclude a
resolved prior failure. This diagnostic never retries, dispatches, schedules,
changes permissions, consumes a continuation, or changes at-most-once semantics.
Structured `outcome_unknown` failure kinds require reconciliation even without
execution-state evidence. Successful pending receipts with an exact Job ID remain
owned by the existing Job lifecycle projection; their historical incomplete
command flags do not create a permanent unknown-outcome warning. An explicit
unknown outcome still takes precedence over Job acceptance.

See the [Session contract](../agent/session-model.md#explicit-handoff-recovery-and-internal-snapshot-fencing)
and [startup ownership](../architecture/startup-projection-boundaries.md).
