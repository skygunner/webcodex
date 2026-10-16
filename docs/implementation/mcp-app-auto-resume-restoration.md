# MCP App continuation restoration acceptance

[PR #1028](https://github.com/yyjeqhc/webcodex/pull/1028) restored
`present_agent_continuation` and `present_job_terminal_continuation` as
model-visible Direct tools with Continuation reason. Durable waits remain
Gateway tools. This procedure is the dogfood/publication gate for future changes
to continuation behavior or Host integration. It does not itself establish Host
readiness or authorize deployment.

The Agent View uses `ui://webcodex/agent-continuation/v21`. A valid ref-only
presentation input waits for the initial Server result to resolve the exact
identity; the ref itself never grants identity or authority. This fixes the
previous initialization teardown when the Host delivered ref-only tool input
before the result. Complete tuple inputs still work, and invalid selectors or
conflicting resolved identities still stop coordination. The View also retains
the presentation-reference fence: changing `~ac1` to `~ac2` is a terminal conflict,
even before a Server projection arrives. Tests cover all six
initialization/input/result orders, including an initial `host_binding.bound=false`.
The [MCP Apps input notification](https://apps.extensions.modelcontextprotocol.io/api/interfaces/app.McpUiToolInputNotification.html)
allows `arguments` to be omitted. Such notifications preserve an already
selected identity or keep waiting for one, rather than tearing down the card.
Explicit malformed or conflicting selectors still fail closed.

## Local and Host evidence

Rust contract, MCP routing/admission and continuation regression tests validate
the local boundary. JavaScript tests validate the card state machines, including
duplicate and uncertain dispatch. They cannot prove ChatGPT scheduling.
`ui/message` acceptance is only dispatch evidence. Agent resumption requires a
fresh model turn and exact Wake consume. The existing Job protocol has no
consume operation: require its exact wait/Job delivery evidence and a fresh turn
that actually observes that Job instead. Neither a delivery ACK nor a displayed
message proves model execution.

Use an explicitly selected disposable test connection on the approved server.
Build the reviewed commit using the `dogfood` profile, record the existing
`git_commit`, `git_dirty` and `built_at` identity, and preserve the previous
binary/configuration before any approved replacement. Isolate state, credentials,
ports and Runner Projects from ordinary service data. Never expose a shared-key
test server without normal authentication. Refresh ChatGPT tool registration
and confirm both dedicated presentation descriptors and their App resources.
Do not route their presentation through `call_runtime_tool`.

Keep `WEBCODEX_MCP_APP_RESUME_MODE=unknown` for an unverified deployment.
Do not set `unattended` merely to make a readiness flag true. The existing App
acquire/prepare/dispatch path can be measured while policy is `unknown`; that
mode keeps the readiness declaration false without blocking this explicit
experiment. Run the complete matrix in that mode first. Only after measured
zero-click fresh-turn behavior may the test deployment declare `unattended` and
verify its readiness projection. Restore its previous policy when the experiment
ends. Historical Host observations are in the
[continuation experiments](../agent/mcp-app-continuation-experiments.md).

## Real ChatGPT matrix

Run each case in a new test conversation with fresh identities. For Agent,
create one identity, rotate one Endpoint and present its exact ref/tuple. Arrange
a delayed Conversation message from an independently authorized sender to that
Agent. For Job, start one harmless delayed Job in a disposable Project, create
its exact durable terminal wait and present that still-waiting wait. Handle a
Job that already terminated in the current turn instead of presenting it.

After the presentation tool result, output a normal assistant final response
and ensure the original turn has finished. Do not click, send another user
message or use a current-turn wait to manufacture the later turn. Leave the
conversation visible for foreground cases; switch away for background cases.

| Card | Delay | Visibility | Required observation |
| --- | --- | --- | --- |
| Agent | about 30 seconds | foreground | zero-click fresh turn, exact Wake consume |
| Agent | about 5 minutes | foreground | zero-click fresh turn, exact Wake consume |
| Agent | about 5 minutes | background | zero-click fresh turn, exact Wake consume |
| Job | about 30 seconds | foreground | zero-click fresh turn, exact wait delivery and Job observation |
| Job | about 5 minutes | foreground | zero-click fresh turn, exact wait delivery and Job observation |
| Job | about 5 minutes | background | zero-click fresh turn, exact wait delivery and Job observation |

For each case record UTC timestamps for original final response, trigger,
prepare, Host dispatch outcome, fresh-turn first tool call and exact Agent
consume or Job observation.
Record the source build, protocol, visibility, policy, correlation IDs and
whether any user action occurred. Verify the Agent turn calls
`consume_agent_wake` with the matching identity. For Job, correlate the exact
wait's one-shot delivery with the resumed turn's `observe_jobs` call for that
Job; do not invent a Job consume API or infer execution from `delivered` alone.
Observe a further polling interval and verify there is no second turn or
duplicated consume/delivery effect for the same logical event.

Retain only bounded, redacted evidence. Omit credentials, binding/claim fences,
consume tokens, raw automatic envelopes and private Conversation content.
`delivery_unknown` must never cause an automatic resend; use the established
explicit recovery path. Stale generations and foreign-owner calls must remain
rejected by their ordinary authority checks.

## Readiness and cleanup

For changes to continuation behavior or Host integration, the matrix is a manual
dogfood/publication gate. Record actual results against the exact candidate build
and deployment; report failures as Host observations without guessing their cause.
Do not declare such a candidate ready while any required case is missing or failed.
Automatic tests do not waive this gate. Test-only and documentation-only changes
use focused local validation and must not claim a new real-Host acceptance result.
Stop the disposable processes and restore the previous test configuration when
done; leave normal service and release state untouched.
