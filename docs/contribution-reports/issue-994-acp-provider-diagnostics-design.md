# Issue #994: ACP Provider read-only diagnostics

## Scope and safety contract

Upstream issue: https://github.com/yyjeqhc/webcodex/issues/994
Base: upstream `main` commit `c425f837b990c000b4dec196650d96e410c67594`.

This is an **incomplete implementation branch**, not an upstream-ready PR.
No Runtime/MCP/Runner operation is exposed yet. Do not deploy or claim the
issue acceptance criteria have passed until the diagnostic surface and live
ACP probing are implemented and tested.

### Implemented building block

- `CodingAgentManager::declared_provider_env_presence(provider_id)`
  scopes lookup to an explicitly selected **configured** provider.
- A pure source-presence helper probes only names declared by the selected
  provider's `env_from_env` mapping in the **calling Runner process**.
- Returns only destination names mapped to booleans. Never includes source
  names, values, executable paths, arguments, or undeclared environment.
- Non-Unicode environment values are not stringified.
- Tests cover present/missing mappings, unknown provider, unlisted-source
  exclusion, serialization non-disclosure and no child process execution.
- Caveat: the canonical helper currently lacks a consumption path; Rust may
  report `dead_code` until the diagnostic operation is added.

### Next implementation pieces before PR

1. Add a read-only Runner request/response variant, bound to an explicitly
   selected provider ID and instance, with the existing Runner/provider
   observation authority and no implicit `coding_agent:run` grant.
2. Reuse the same `CodingAgentManager` primitive, executed inside the
   active Runner process. A separately invoked CLI shell environment is not
   an acceptable surrogate for the running service process.
3. Implement a **bounded ephemeral** ACP `initialize` then `session/new`
   probe through the existing managed-child / bounded frame transport.
   No `session/prompt`, `session/set_config_option`, provider tool calls,
   config mutation, or unbounded retries.
4. Project only bounded option IDs/types and allowlisted select choices;
   classify raw ACP parse/transport failures with constant safe codes.
5. Add negative tests: forbidden calls absent on the fake provider's log;
   init/new failure, oversized/malformed output, choices budget, secret values
   absent in *all* results and errors, timeout and process-tree cleanup.
6. Validate schema/authorization/identity in Runner, Server and model-facing
   tool contracts, and run focused ARM64 + E2E tests before proposing PR(s).

Preserve exact warning that an ACP provider's initialize/session/new may
trigger an external login prompt. The implementation must fail closed rather
than silently proceed when provider-side side effects are not understood.

## Version safety

No Gitea `dev`/`prod`, wa1 Live, or QUIC private-security branch changes.
Source worktree: `webcodex-b-acp-diagnostics-20261014` in its independent
upstream contribution branch.

## Verified Runner request and protocol boundary (2026-10-14)

The original internal environment-presence helper is now part of an
explicit, typed `CodingAgentRequest::Diagnose` operation. It is not the
existing Start/Observe/Cancel operation and **never** launches an ACP
process, creates a Run, submits a prompt or mutates Provider configuration.

- Core transport uses a strict request type containing a correlation
  run_id, exact provider and provider-instance IDs, runtime Project ID,
  and canonical Project root. The response has only provider identity and
  configured destination-to-boolean presence.
- `validate_request` fences identifier lengths, Project identity and
  project-root syntax. `validate_response_for_request` rejects mismatched
  provider generations, excess environment entries and bad keys.
- Runner dispatch checks its current provider instance and registered
  Project root before reading environment presence. Read-only Projects
  may be observed; the pre-existing Start operation still requires
  writable Project authority. No arbitrary environment variable names
  are supplied by the caller.
- `webcodex-runner-registry::enqueue_coding_agent` is the established
  dispatch fence: caller RunnerAccess, exact connected Runner identity,
  advertised provider instance and the checked request provider binding.
  The new Diagnose operation rides that existing typed fence.
- A dedicated Runner regression verifies success, missing source, no
  child process/no Run persistence, stale provider rejection, wrong
  Project rejection and read-only Project acceptance.
- A core transport regression verifies typed roundtrip, exact provider
  binding and response size/key rejection.

Verified locally in the isolated ARM64 build target:
- `cargo check --locked -p webcodex-core -p webcodex-runner -j 2`:
  PASS on the intermediate wire patch.
- `cargo test --locked -p webcodex-core --lib
  diagnostic_wire_is_identity_fenced_and_does_not_accept_raw_secrets`:
  1 passed / 0 failed.
- `cargo test --locked -p webcodex-runner --bin webcodex-runner
  acp_diagnose_runner_request_is_identity_fenced_read_only_and_no_spawn`:
  1 passed / 0 failed after adding read-only Project coverage.
- `cargo fmt --all -- --check` and `git diff --check`: PASS.

**Still not externally callable:** no Server/ToolRuntime or UI
presentation invokes the new operation yet. The eventual Server entry
must first independently resolve and authorize the exact requested
Project and Runner; no generic public environment-inspection endpoint
is acceptable. Old Runner protocol generations must fail closed rather
than silently accepting unsupported Diagnose requests. The second
stage live ACP initialize/session-new probe is not implemented. Do not
advertise this internal Runner request branch as complete #994 or open
a completion PR without these pieces.
