# Security Hardening Regression Acceptance

This playbook is the maintainer/test-agent acceptance plan for security-hardening
changes that cross MCP runtime configuration, authentication, QUIC, tunnel process
ownership, secret-safe diagnostics, temporary executable fixtures, or committed Git
review isolation.

It supplements [Testing Strategy](../TESTING.md),
[Security Hardening Testing](security-hardening-testing.md), and
[Job Observation Reliability Test Playbook](job-observation-reliability-testing.md).
Focused shards are development feedback. They never replace the final monolithic run.

## Reference baseline

The 2026-09-26 hardening follow-up established this known-good local reference on
commit `921d415c78805691456f55c280bf711d6c6ee03e`:

- `cargo fmt --all -- --check`: pass;
- `git diff --check`: pass;
- `cargo check --all-targets`: pass, zero warnings/errors;
- `cargo test --lib`: 2917 passed, 0 failed, 3 ignored.

These counts are historical evidence for that exact SHA, not permanent numeric
thresholds. Future runs must record their own actual testcase counts and treat
`running 0 tests` as no evidence.

## Test-agent execution contract

Long MCP-driven validation must remain recoverable and observable.

1. Start with repository identity and environment capture before running tests.
2. During development/diagnosis, split long validation into deterministic 2/4/8
   shards. If complete historical testcase timing is available, balance shards
   largest-first by runtime. Otherwise split by stable subsystem boundaries, not by
   random testcase counts.
3. Start each long command with only a 1-2 second synchronous grace so the durable
   `job_id` is returned. Preserve that exact `job_id`, observation token/cursor,
   and log cursor.
4. Observe active jobs in roughly 15-20 second bounded windows. Prefer
   `meaningful_change`/debounce when supported so pytest/libtest progress noise or
   heartbeat-only revisions do not cause excessive wakeups. Use `change` only when
   heartbeat visibility itself is under test.
5. Do not redispatch a command merely because an MCP observation call, model turn, or
   client connection was interrupted. Resume the persistent Job.
6. A shard pass is not final acceptance. Once source edits stabilize, run the
   monolithic suite in one process to catch process-global state leakage, test-order
   effects, and contamination hidden by sharding.
7. Do not hide a monolithic failure with retries, reduced concurrency, a longer
   timeout, or `--ignored`. Those are diagnostic controls only. Find the first causal
   failure and classify it.
8. Do not remount `/tmp`, weaken authentication, disable isolation, or mutate
   production services merely to make the suite pass.

## Environment capture

Record at minimum:

```bash
git branch --show-current
git rev-parse HEAD
git status --short
uname -a
rustc -Vv
cargo -V
git --version
python3 --version
findmnt -no TARGET,FSTYPE,OPTIONS /tmp 2>/dev/null || true
findmnt -no TARGET,FSTYPE,OPTIONS /tmp/user/$(id -u) 2>/dev/null || true
```

If the host uses a different temporary directory layout, record the effective temp
path/mount options without changing them. A hardened `noexec` temp mount is a valid
test environment and should remain enabled.

## Required regression matrix

| ID | Boundary | Required evidence | Expected invariant |
| --- | --- | --- | --- |
| SH-01 | MCP runtime configuration snapshot | MCP/model-surface shard plus final monolithic | request handling does not re-read mutable process env for runtime MCP Apps/compact-schema configuration; parallel tests cannot leak temporary env state into unrelated requests |
| SH-02 | QUIC remote shared-key gate | `runner_quic` focused tests; disposable real-network smoke when a multi-interface host is available | loopback quick-start shared-key may be accepted; non-loopback accepted connections reject direct shared-key by default; explicit remote opt-in allows it; managed Runner/Agent credentials keep their own policy |
| SH-03 | tunnel startup/timeout/stop ownership | share-service focused tests | timeout/stop/early-exit supervise and reap the whole Unix process group; descendants do not survive; diagnostic text exposes only safe error kind/state |
| SH-04 | OAuth/Config Debug redaction | config/debug focused tests | bootstrap token, share-session fence, OAuth secret-bearing fields, and credentials never appear in Debug/log text; safe counts/state may appear |
| SH-05 | executable test fixtures on hardened temp hosts | search fake-command shard | tests that execute fake `rg`/`grep`/`head` use the executable fixture root under `target/test-executables`, not an assumed executable system temp mount |
| SH-06 | managed cloudflared fixture on `noexec` temp hosts | cloudflared-service shard | fake/managed cloudflared binaries can execute without remounting `/tmp`; checksum/permission/reuse behavior remains unchanged |
| SH-07 | show_changes external-diff fixtures | `show_changes_` shard | external-diff/wrapper fixtures execute from the explicit executable temp root; sentinel, boundedness, failure propagation, and secret-path behavior remain unchanged |
| SH-08 | git_diff_hunks external-diff fixtures | `git_diff_hunks_` shard | external diff/textconv cannot override the bounded canonical committed/worktree diff path; fixture execution is independent of system-temp exec policy |
| SH-09 | reviewed-HEAD root `.gitattributes` | `git_diff_hunks_committed_exact_range_isolated_targeted_and_head_attributed`; `git_review_summary_uses_reviewed_head_committed_attributes` | exact committed review uses attributes from reviewed HEAD, not dirty worktree, `.git/info/attributes`, global/system config, external diff, or textconv |
| SH-10 | reviewed-HEAD nested `.gitattributes` | `git_review_summary_uses_nested_reviewed_head_attributes_without_mutable_leakage` | nested attribute files from reviewed HEAD are materialized into the isolated temporary view and remain authoritative even when the mutable worktree attempts the opposite classification |
| SH-11 | committed-review isolation mechanics | generated-script assertions in Git tests | temporary index is populated from reviewed HEAD; only `.gitattributes` files are materialized into the isolated worktree; ordinary source files are not checked out; real user worktree is not mutated |
| SH-12 | fail-closed committed range validation | `git_diff_hunks_committed_range_validation_and_merge_base_fail_closed` and related range tests | missing/non-commit base/head preserve specific reason codes before isolated-view setup; no invalid object is disguised as a generic isolated-view failure |
| SH-13 | mutable attribute/config isolation | `git_review_summary_exact_range_ignores_mutable_git_attributes_and_config` | changing worktree/info/global attributes after the reviewed commits exist cannot change the exact committed review result |
| SH-14 | cross-test/global-state contamination | raw monolithic `cargo test --lib` with ordinary concurrency | focused shards and the same tests inside the full libtest process agree; no schema/env/config leakage appears only under monolithic execution |

## Focused deterministic shards

Run these after the smallest exact regression tests and before the monolithic suite:

```bash
python3 scripts/tests/test_workflow_security.py

cargo test -p webcodex --lib 'mcp::tests::' -- --nocapture
cargo test -p webcodex --lib 'runner_quic' -- --nocapture
cargo test -p webcodex --lib 'project_entry::share_service::tests::' -- --nocapture
cargo test -p webcodex --lib 'config::tests::' -- --nocapture

cargo test -p webcodex --lib 'tool_runtime::tests::files::search_' -- --nocapture
cargo test -p webcodex --lib 'project_entry::cloudflared_service::tests::' -- --nocapture
cargo test -p webcodex --lib 'tool_runtime::tests::git::show_changes_' -- --nocapture
cargo test -p webcodex --lib 'tool_runtime::tests::git::git_diff_hunks_' -- --nocapture
cargo test -p webcodex --lib 'tool_runtime::tests::git::git_review_summary_' -- --nocapture
```

Verify the actual test count for every filter. If one filter selects zero tests, repair the
filter and rerun it; do not report that command as passed validation.

## Static finalization

After relevant source changes stabilize:

```bash
cargo fmt --all -- --check
git diff --check
cargo check --all-targets
```

Do not repeatedly run formatting after each intermediate edit. Run it as finalization,
then rerun only if later Rust edits could affect formatting.

## Monolithic acceptance

The regression that motivated this playbook was visible in a single libtest process,
so preserve the original entry point:

```bash
cargo test --lib
```

Run it with ordinary/default test concurrency unless a separate experiment is explicitly
comparing concurrency. A pass with reduced `--test-threads` does not replace this gate.

For a broader complete Server package lane, also run:

```bash
bash scripts/test_server.sh
```

The script may cap libtest fan-out for resource control. Treat it as broader package
evidence, not as a substitute for the raw monolithic command above when investigating a
console/default-concurrency failure.

## Disposable real-process / network evidence

When the test host and task scope allow it, add the smallest relevant real-process
coverage after the deterministic local suite:

```bash
bash scripts/e2e_shared_key_ws.sh
bash scripts/e2e_hosted_connect.sh
bash scripts/e2e_zero_config_ws.sh
```

Run MCP conformance when external referee dependencies/network are available:

```bash
bash scripts/mcp_conformance.sh
```

For QUIC remote shared-key rejection, prefer a disposable multi-interface host and verify
the accepted connection's actual local address. Do not weaken TLS, bind a production
service, or expose a production shared key merely to create the test.

## Failure classification

Every non-passing command must be classified as one of:

- `PRODUCT REGRESSION`: current product behavior violates the documented invariant.
- `TEST BUG`: the test/fixture makes an invalid assumption or its assertion no longer
  represents the product contract.
- `ENVIRONMENT FAILURE`: ENOSPC, OOM, killed compiler, unavailable dependency, broken
  toolchain, incompatible shell, or another host condition prevents trustworthy execution.
- `FLAKY`: a demonstrated nondeterministic failure whose cause is not yet fixed. One
  subsequent pass does not erase it.
- `NOT RUN`: the lane was intentionally unavailable or out of scope.

A hardened `noexec` system temp mount is not itself an environment failure for tests that
intentionally create executable fixtures. Those fixtures must use
`crate::test_support::executable_tempdir()`. A test that still assumes executable
`tempfile::tempdir()` should be treated as a test-infrastructure defect and fixed at the
fixture boundary.

## Test-agent report

The final report must contain:

- exact branch and SHA;
- whether the worktree was clean before and after testing;
- architecture, OS, Rust/Cargo, Git, Python versions, and temp-mount exec policy;
- every command actually run and actual testcase count;
- pass/fail/infra-fail result per focused shard;
- monolithic `cargo test --lib` result and whether `scripts/test_server.sh` was run;
- any real-process/E2E/conformance lanes run;
- first causal diagnostic for every failure, with cascades separated from causes;
- failure classification;
- whether any source/config/service state was changed by the tester.

Never include plaintext credentials, Authorization headers, generated secret env contents,
OAuth secrets, private keys, or raw sensitive command bodies in the report.
