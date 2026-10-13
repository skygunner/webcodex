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
