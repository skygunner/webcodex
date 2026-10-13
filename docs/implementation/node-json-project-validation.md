# Native Node JSON validation and declared builds

[简体中文](node-json-project-validation.zh-CN.md)

The current user-facing contract is in [MCP](../MCP.md). This extends native
Node TAP tests with recognized direct Vitest/Jest scripts and extends the
project build gateway with manifest-declared Node build scripts.

The validation plan contains a fixed local framework CLI and a report placeholder.
Only Runner preparation replaces the placeholder, after allocating an exclusive
private temporary directory for this Job. The framework creates the report;
the Runner never reads a prior report in the project. After the managed process
finishes, the Runner reads at most 1 MiB, rejects links/special files and checks
aggregate counts against individual assertion statuses and process outcome.
Only passed and failed assertions count as executed; skipped/pending/todo tests
do not. Missing, partial, oversized or inconsistent reports are unproven.
Timeout, stop and lost outcomes cannot supply successful count evidence.

The raw report is removed before terminal delivery. Bounded aggregate counts use
the existing authoritative test-count evidence path through delivery, snapshots,
registry updates, same-instance inventory reconciliation and ToolRuntime results.
Truncated stdout/stderr does not invalidate an independently complete report;
log text itself never supplies framework counts. A new Runner instance or an
unmatched receipt cannot restore detached validation evidence or authorize replay.
Source freshness remains unproven by these reports, as for other project tests.

Builds reuse packageManager/lockfile manager detection and fixed script argv;
Yarn uses `run build` because modern Yarn does not accept `run --silent`.
Corepack network/download/auto-pin overrides prevent adapter-driven bootstrap.
No build output directory is guessed. Explicit `finish_coding_task(outputs=...)`
inspection is separate from a successful script and from functionality proof.
Provenance is rechecked after profile preparation, immediately before native
process handoff. This is a stale-plan fence, not an atomic source snapshot.

## Focused validation

Unit tests cover report bounds, missing/stale/truncated reports, count consistency,
link rejection and cleanup. Recipe/Runner/registry tests cover closed argv,
manager ambiguity, unsupported scope, old-Runner capabilities and queued drift.
The real fixture runs both frameworks through Job admission, execution, terminal
counts and snapshot serialization, for pass, failure, skipped-only zero, todo-only zero, mixed passed/skipped/todo,
timeout and cancellation, plus a same-project failed → repaired → passing Job
sequence. It uses disposable dependencies, never production state:

```bash
fixture_dir=$(mktemp -d)
npm install --prefix "$fixture_dir" --ignore-scripts --no-audit --no-fund vitest@3.2.4 jest@30.2.0
WEBCODEX_NODE_FIXTURE_MODULES="$fixture_dir/node_modules" cargo test -p webcodex-runner -j 2 node_json_real -- --ignored
cargo test -p webcodex-runner -j 2 node_project_build_real -- --ignored
rm -rf "$fixture_dir"
```

These real fixture tests require installed Node/npm and currently exercise Unix
symlink-based dependency placement. Normal report/recipe/contract tests do not
install packages. Windows/macOS process behavior still requires their CI lanes.
