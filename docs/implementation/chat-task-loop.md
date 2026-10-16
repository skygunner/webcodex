# ChatGPT task execution and evidence

[中文](chat-task-loop.zh-CN.md)

`work_on_project` keeps strict selector validation. Invalid combinations explain
the authorized discovery path without echoing rejected selectors or diagnosing
Runner availability: use a known canonical Project/issued `project_ref`, or
`list_projects` when unknown; a path requires its exact `client_id`.
The normal workflow investigates applicable instructions and code, states
observable completion conditions, displays a short plan for substantial work,
and continues authorized execution. The default remains `on_demand`: displaying
a plan does not create a durable Goal. Existing read continuations retain their
exact arguments. Pending/uncertain execution retains the original Job identity;
finish independent work before joining the exact blocking Jobs. Successful
tools, finished Jobs and checked steps do not prove the user's outcome.

## Disposable acceptance and strict benchmark gate

The existing [benchmark](../../scripts/agent_loop_benchmark.py) now accepts all
six `generic_agent_disposable_fixture` cases from its case manifest. Each sample
gets an ordinary fresh temporary directory rather than a repository checkout;
the source revision still pins the manifest and runtime evidence rules. The
shared [fixture implementation](../../scripts/e2e_generic_agent_ws.py) supplies
bounded final-file oracles. The Host driver must apply the supplied fixed
locale/timezone/Python environment to commands executed through WebCodex.

In addition to the existing driver contract, generic samples expose
`WEBCODEX_BENCH_TARGET`, `WEBCODEX_BENCH_WORKSPACE`,
`WEBCODEX_BENCH_FIXTURE_ENVIRONMENT` (JSON child environment),
`WEBCODEX_BENCH_FIXTURE_COMMAND` (the prescribed failing output-pressure Python
script), and `WEBCODEX_BENCH_FIXTURE_COORDINATION` (outside the editable Project).
For the stale-read case the driver signals `first-read` after retaining the
exact read receipt, waits for `external-written`, attempts the stale fenced
edit, signals `stale-rejected` only after rejection, then waits for
`rejection-checked` before rereading and completing. The external writer uses
one verified regular-file descriptor and never follows a swapped symlink.
These signals coordinate the fixture; they are not runtime evidence of tool
selection or permission.

Use `--strict` with explicit `--case-id` selections and `--pairs` (forced lanes)
or `--experiment tool_selection --samples`. Exit zero requires the exact
requested sample matrix, passing task and required validation verdicts,
runtime-backed lane evidence and every available file oracle. Missing cases,
`partial`, `unsupported` and missing required evidence fail this gate. Driver
annotations alone cannot prove execution.

Currently the four temporal generic cases (failure/repair, pending join,
stale revision and missing output) remain `partial` in Host benchmark reports:
the privacy-bounded audit does not prove their full ordered result semantics.
The files/CSV cases can qualify when their runtime and oracle gates pass.
Do not describe an all-six strict run as passing. The deterministic real
Server/Runner smoke separately checks runtime mechanisms, including those
temporal cases; it is not evidence that ChatGPT chose the right actions.

```bash
python3 -m unittest scripts.tests.test_agent_loop_report scripts.tests.test_agent_loop_benchmark scripts.tests.test_generic_agent_acceptance
cargo build --locked --profile dogfood -j 2 -p webcodex --bin webcodex-server -p webcodex-runner --bin webcodex-runner
python3 scripts/e2e_generic_agent_ws.py --profile upgraded --source-version ACTUAL_SOURCE_ID --report /tmp/generic-agent-acceptance.json
```

## ChatGPT Host acceptance

Use only an isolated disposable Project and authorized Runner. Ask ChatGPT:

> Inspect this fixture, explain a short plan, fix `add(a,b)` to return a+b,
> run the supplied failing test, diagnose and repair it, rerun validation,
> review the actual diff and report the result. Continue without routine
> confirmation. Never claim validation you did not observe.

Expected: a visible plan followed by investigation, a revision-fenced edit,
observed failure and repair, passing rerun, review and evidence-based delivery.
Repeat with a truncated file read and one long Job plus independent file work:
the exact read continuation should be followed, the Job started once and joined
after independent work. Resume the exact Session in another chat and verify
recovered facts against current source. For an invalid selector, expect rejection
and authorized discovery, not a fabricated offline diagnosis. Withhold an output
artifact or validation evidence and expect an honest incomplete/unproven result.
These scenarios require actual Host observation and remain separate from local
transport and simulated-driver tests.
