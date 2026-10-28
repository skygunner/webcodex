# Issue #994 stage three: the authorized Server Diagnose entry and its CLI consumer

Follow-up to the merged Runner-side primitive (#1129). That PR shipped the
typed Runner fence (`CodingAgentRequest::Diagnose`), the presence-only
`env_from_env` projection, and the bounded ephemeral live ACP probe. This
round adds the missing middle: one canonical Server/ToolRuntime entry that
independently authorizes the exact Runner, provider generation, and
authorized Project before forwarding the typed request, plus the first
presentation surface (a CLI operator command). Desktop can reuse the same
entry later without new authority semantics.

## The entry

`diagnose_coding_agent` is a model-visible runtime tool next to
`observe_coding_agent`, with the same static authority shape:

- OAuth policy `Require(coding_agent:run)` — runner observation authority,
  never project-write;
- effect `Observe`, approval `None`, idempotency `PureRead`;
- Runner capability `CodingAgentRuns` required.

The resolution order inside the entry is fail-closed and ordered so nothing
is observed or launched before its authority is established:

1. provider id validation and principal binding;
2. `include_live_acp=true` gate — `check_runtime_tool_scope` against the
   exact `start_coding_agent` scope policy, then a fresh
   `PermissionEvaluator::evaluate_resolved_required("start_coding_agent", …)`
   decision for the resolved Project. The decision (allow or deny) is
   attached to the result like any permission-bearing execution;
3. Project resolution for the caller (`resolve_project_input_for_auth`);
4. exact Runner semantic view for the caller (offline or unauthorized →
   bounded `coding_agent_runner_unavailable`);
5. provider generation from the Runner's advertised inventory
   (`provider_instance_id` is looked up, never supplied);
6. typed dispatch through the existing `enqueue_coding_agent` fence, which
   independently re-verifies Runner identity, provider generation, online
   window, and access;
7. bounded wait (45 s > the Runner's 30 s probe budget), response identity
   verification, and server-side bound re-verification of the payload.

Project write authority is never required or implied — a diagnostic does not
mutate the Project, and the live probe's launch authority is the explicit
start gate above, not Project mutation.

## Bound re-verification at the Server

The Runner already projects inside fixed budgets. The Server re-verifies the
response before projecting it, so a malformed or oversized Runner answer can
never reach a caller as a diagnostic: option/choice counts, identity byte
bounds, the stable `select|boolean` kind vocabulary, the
"options only on success" invariant, and the environment-presence map bound
(`CODING_AGENT_MAX_ENV_MAPPINGS`, now shared with the Runner's ACP config
validation so the schema can never disagree with what was admitted). All
violations collapse into `invalid_runner_response`.

## Projection

The success payload is `project`, `provider_id`, `env_presence`
(destination name → boolean), and `live_acp` (the bounded
`CodingAgentLiveAcpDiagnostic`, `null` unless the caller authorized the live
probe). The provider instance id never crosses to the caller — it is the
Server-side identity anchor only, matching `safe_provider_inventory`.

## CLI consumer

`webcodex ops diagnose-provider --project <id> --provider <id>
[--include-live-acp]` calls the same authorized entry through the existing
`/api/tools/call` bridge with the operator credential. It renders presence
lines (`present` / `ABSENT`), the live option identities when requested, and
an ops verdict (`pass` / `warn` for absent env sources, `fail` for bounded
failures). Values, provider stderr, instance ids, and credentials never
reach the output.

## Deliberately unchanged

- The Runner fence, probe lifecycle, cleanup, and busy/timeout contracts
  (#1129) are reused verbatim; no second ACP client exists.
- `env_from_env` presence stays spawn-free; nothing about
  `include_live_acp=false` touches a process.
- No Desktop surface, no automatic retry, no configuration writes.
