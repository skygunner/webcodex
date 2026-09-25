# Job Observation Reliability Test Playbook

This playbook covers the Job-observation reliability path:

`Runner JobManager -> RunnerRegistry -> ToolRuntime observe_jobs -> tool contracts/schemas -> runtime metrics`.

It is intended for parallel verification by multiple agents and architectures. It is a
validation guide, not deployment authority: do not deploy, restart production services,
rewrite history, or delete build caches unless the operator explicitly asks.

## What changed

The reliability layer distinguishes three kinds of observable progress:

1. **heartbeat-only revision**: a Runner-owned active Job advances only its update sequence;
   logs, lifecycle, activity, recovery state, and terminal outcome are unchanged;
2. **meaningful change**: logs, lifecycle, activity, recovery state, validation progress,
   command state, or terminal outcome changed;
3. **terminal change**: the Job reached a canonical terminal lifecycle.

Runner-owned active states (`agent_queued`, `running`, and `stop_requested`) emit a
JobManager heartbeat every 15 seconds independently of subprocess stdout/stderr. Heartbeat
scheduling is lightweight, but the current reconciliation-safe wire update reuses the
bounded authoritative Job snapshot and can therefore include retained log tails. The
Server classifies an accepted sequence-only update as `changed=true`,
`heartbeat_changed=true`, `meaningful_changed=false`. The compatible
`wake_on=change` policy may wake on it; `wake_on=meaningful_change` must suppress it;
`terminal` and `all_terminal` continue to coalesce non-terminal revisions.

## Parallel architecture matrix

Run the focused matrix on at least Linux aarch64 and Linux x86_64 when both are available.

| Surface | aarch64 | x86_64 | Why |
|---|---:|---:|---|
| tool contracts/schema | required | required | catches enum/schema/model-surface drift |
| RunnerRegistry Job waits | required | required | pure lifecycle/revision semantics |
| Runner heartbeat/coalescing | required | required | thread/atomic/queue behavior |
| ToolRuntime observe_jobs | required | required | async wait, cursor, compact projection, metrics |
| full Server package suite | required before merge | required when practical | catches cross-test/global-state contamination |
| real-process reconciliation scripts | one architecture minimum | strongly preferred | validates restart/reconnect with real processes |

Architecture differences must not change wire semantics. Record `uname -m`, Rust version,
OS, commit SHA, and whether tests ran with default or constrained test concurrency.

## Deterministic development shards

Start with static compilation, then run these four independent shards. Keep each command
as one durable MCP Job and retain its `job_id`; do not redispatch because an observation
call or model turn was interrupted.

```bash
cargo check -p webcodex -p webcodex-tool-contracts -p webcodex-runner-registry -p webcodex-runner

cargo test -p webcodex-tool-contracts observe_jobs_wake_policy -- --nocapture
cargo test -p webcodex-runner-registry job_log_wait -- --nocapture
cargo test -p webcodex-runner heartbeat -- --nocapture
cargo test -p webcodex observe_jobs -- --nocapture
```

For MCP-driven execution, use a 1-2 second synchronous grace to obtain the persistent
`job_id`. During long compilation or tests, observe in roughly 15-20 second bounded
windows. Use `change` only when proving heartbeat visibility; use
`meaningful_change` when supported and the purpose is to ignore heartbeat/noisy progress.
Do not use one long terminal-only wait as a liveness probe.

If historical testcase timing is available, balance shards largest-first by runtime.
Otherwise keep the deterministic subsystem shards above rather than inventing random
splits. A shard pass is rapid feedback, not final acceptance.

For the heartbeat edge fixes, these focused filters are useful before starting
the broader shards:

```bash
cargo test -p webcodex-runner heartbeat_batch_advances_all_runner_active_jobs_and_skips_terminal_jobs -- --nocapture
cargo test -p webcodex-runner delivery_queue_orders_semantic_truth_and_drops_stale_output_only_updates -- --nocapture
cargo test -p webcodex observe_jobs_meaningful_change_suppresses_heartbeat_then_change_reports_it -- --nocapture
cargo test -p webcodex observe_jobs_terminal_policies_ignore_heartbeat_only_revisions -- --nocapture
```

Always verify the reported testcase count. A command that exits zero after
`running 0 tests` is not validation evidence. In particular, do not add
`--exact` to a short function-name filter unless the fully-qualified libtest
name is supplied.

## Required semantic cases

Focused tests must cover all of these:

- sequence-only update advances the ordinary observation revision;
- all Runner-owned active states (`agent_queued`, `running`, `stop_requested`) receive
  heartbeat sequence advancement, while terminal states do not;
- heartbeat-only update does not advance the last meaningful revision;
- real stdout/stderr/activity/lifecycle change is meaningful;
- `wake_on=change` wakes on heartbeat-only change;
- `wake_on=meaningful_change` stays pending across heartbeat-only change and wakes on a
  later meaningful update;
- `terminal` ignores heartbeat and ordinary non-terminal output, then wakes on any
  terminal Job;
- `all_terminal` waits for the complete predetermined set;
- stale/replayed Runner sequence does not advance the observation token;
- two observers can wait on the same Job without stealing each other's notification;
- returned observation reports deterministic line/byte counts and preserves cursor/token
  continuity;
- wake diagnostics distinguish heartbeat, meaningful change, terminal, timeout, and item
  error without logging credentials or command bodies;
- Job observation metrics stay fail-open and low-cardinality.
- a heartbeat/output-only update generated earlier but queued after a newer required
  semantic update is discarded rather than delivered out of sequence; this protects
  legacy/no-reconciliation Server mode from state regression as well as sequenced mode.

## Chaos and reconnect matrix

After focused tests pass, run deterministic real-process scenarios where the environment
allows them:

1. **silent command**: a command remains alive and produces no stdout/stderr for at least
   two heartbeat periods; `change` sees heartbeat progress while
   `meaningful_change` does not wake solely for heartbeat;
2. **noisy stdout**: sustained output does not starve terminal delivery and does not extend
   the absolute observer deadline;
3. **observer reconnect**: disconnect the observer and resume with the same
   `job_id + observation_token/cursor`; do not spawn a replacement command;
4. **two observers**: independent callers see the same authoritative Job progression;
5. **server-only restart**: reconciliation-capable Runner keeps the process alive, the same
   `job_id` returns, old Server-epoch token refreshes, and execution occurs once;
6. **runner replacement/loss**: existing recovery tests must still fence stale instances
   and preserve terminal `lost` reasons;
7. **large output**: bounded retention/reset/truncation remains explicit and observation
   result size remains bounded;
8. **timeout**: Job runtime timeout and observer wait timeout remain distinct;
9. **queue pressure**: heartbeat/output-only traffic must remain coalescible and never
   displace required semantic/terminal updates.

Existing real-process restart coverage lives in:

```bash
bash scripts/e2e_job_reconciliation_ws.sh
bash scripts/e2e_job_recovery_failures_ws.sh
```

Use those harnesses rather than creating a second restart state machine.

## Final monolithic acceptance

After all relevant source edits have stabilized and focused shards pass, run formatting
once, then a monolithic package suite to catch global-state leakage, test-order effects,
and interactions hidden by process-level sharding:

```bash
cargo fmt --all -- --check
bash scripts/test_server.sh
```

For cross-cutting Runner changes, also run the ordinary Runner package suite. Run ignored
real-process groups only when their boundary is relevant or the operator explicitly asks.

A final monolithic failure must be investigated even when all shards passed. Do not hide
it by reducing concurrency. Conversely, classify infrastructure failures such as ENOSPC,
OOM, killed compiler processes, or unavailable external dependencies separately from
product regressions and rerun after the environment is repaired.

## Result handoff for another agent

Report at minimum:

- exact commit SHA and branch;
- architecture / OS / Rust version;
- commands run;
- pass/fail/infra-fail per shard;
- failing test names and first causal diagnostic, not just the last cascade error;
- whether monolithic tests were run;
- whether real-process restart/reconnect scenarios were run;
- any observed difference between aarch64 and x86_64.

Do not commit generated logs, credentials, raw authorization headers, tokens, or private
command arguments. If a test exposes a reliability issue, preserve the original
`job_id`, cursor/token evidence, wake policy, elapsed wait, and failure classification
in the report so another agent can reproduce it without redispatching the original Job.
