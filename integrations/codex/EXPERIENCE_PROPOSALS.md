# Explicit experience proposals

[中文](EXPERIENCE_PROPOSALS.zh-CN.md)

The optional Python 3.10+ [adapter](experience_proposals.py) accepts a bounded
candidate written by the operator or current Host. It stores that candidate as
an existing Workflow Session `proposal`, never `guidance`. It does not extract
transcripts, call a model, scan history, create a Session, change configuration,
or add a database. Review and approval are deliberate commands. Session ACKs,
proposal text, and saved approval notes do not grant runtime authority.

Use the private configuration and private state-directory preparation from the
[integration README](README.md#optional-adapter-macos--linux--windows-python-310).
Use a dedicated pre-created private state directory outside the project. The
configuration pins the exact server, canonical project, existing Workflow
Session and local conversation. Run commands inside its configured project.
The operator/current Host must select that association explicitly; directory
membership does not establish write authority. HTTPS/loopback, no redirects,
no ambient proxies, private-file ACLs and the existing state lock are reused.
The existing read-only Project inventory verifies the exact registered root
mapping on entry and again before effect dispatch.

The deployment credential must already have native Session collaboration and
Project authority. Memory reads require `project:read` and `memory:read`;
Memory writes require `project:write` and `memory:manage` plus the normal runtime
permission gate. Memory calls use supported Stateless MCP 2026 on `/mcp`;
generic REST intentionally rejects Memory. Session and file calls use the
authenticated Runtime API. Neither transport adds authority. Skill promotion here targets only project content at
`.agents/skills/<name>/SKILL.md` through native `write_project_file`, with its
normal Project-write/path/Runner/permission checks. It does not install, activate,
or modify trusted configured or managed Skills. A denial never redirects Memory
to a Skill or uses a direct filesystem write. See the canonical
[Memory contract](../../docs/agent/session-model.md) and
[Runtime API contract](../../docs/agent/runtime-api-guidelines.md).

Obtain exact source references without exporting their bodies:

```sh
python3 /absolute/path/experience_proposals.py --config /private/operator/observation.json source --file src/example.py
python3 /absolute/path/experience_proposals.py --config /private/operator/observation.json source --message wc_msg_EXACT_SAVED_RECEIPT
```

Each command returns one `{kind, ref, sha256}` object. File SHA-256 covers the
whole authoritative Runner file. Session-message SHA-256 covers canonical JSON
of the exact saved semantic message fields, including lifecycle, resolution and
replacement references; ACK/projection counters are excluded. A saved receipt is evidence only of what that receipt actually
proves. A source hash does not turn a note into native execution evidence.
Sources are restricted to files and exact messages in the configured Session.

Write a private, single-link candidate JSON file outside the project:

```json
{
  "schema": "webcodex.experience-proposal.v1",
  "summary": "Check bounded inspection results",
  "content": "Inspect continuation fields before treating a read as complete.",
  "conditions": ["When using bounded project inspection tools"],
  "sources": [
    {"kind": "file", "ref": "src/example.py", "sha256": "REPLACE_WITH_EXACT_CURRENT_HASH"}
  ],
  "destination": {
    "kind": "memory",
    "memory_key": "bounded-inspection",
    "expected_revision": null
  }
}
```

`null` requests create-only intent; an existing Memory requires its exact current
`wc_memrev_*` revision. Native no-revision Memory writes have desired-state
idempotence for identical concurrent content; such a receipt does not prove who
created it. This adapter never blindly retries that write. Memory uses normal
priority, `bootstrap=false` and the `experience` tag. Review includes the exact
rendered body: candidate content followed by its `Applies when:` conditions.
Do not place credentials, secrets or private transcripts in candidates.

A project Skill destination is instead:

```json
{"kind":"skill","path":".agents/skills/bounded-inspection/SKILL.md","expected_sha256":null}
```

For an existing Skill, replace `null` with its exact current full-file SHA-256.
Skill content must begin with YAML frontmatter containing `name` then
`description`. The same `Applies when:` conditions are appended. This is a
single project Skill definition, without scripts or package installation.

```sh
python3 /absolute/path/experience_proposals.py --config /private/operator/observation.json propose --candidate /private/operator/candidate.json
python3 /absolute/path/experience_proposals.py --config /private/operator/observation.json review --message wc_msg_RETURNED_ID --version RETURNED_VERSION_HASH
python3 /absolute/path/experience_proposals.py --config /private/operator/observation.json approve --message wc_msg_RETURNED_ID --version RETURNED_VERSION_HASH --destination-hash RETURNED_DESTINATION_HASH
```

Review the exact candidate, rendered guidance and destination before issuing
`approve`. That command records an exact approval `note`, rereads its saved
receipt and the current open candidate, revalidates source hashes and target,
then invokes `set_memory` with the approved CAS revision or `write_project_file`
with the fresh native `read_revision` for the approved Skill snapshot. New Skill
files use `overwrite=false`. It returns exact candidate, approval, promotion-note
and Memory-id/revision or file-hash receipt references. The candidate has at most
8 sources, 8 conditions, 4 KiB content and 7,500 encoded bytes. State capacity
fails closed at 1,024 files. These are adapter bounds, not expanded runtime limits.

Before dispatch the adapter fsyncs a private attempt marker. A duplicate returns
the saved receipt without repeating the effect. A rejected, malformed or lost
effect receipt leaves `promotion_outcome_unresolved_do_not_retry`; inspect the
exact target and saved refs through native read tools. Preserve the attempt
journal and do not delete it to manufacture retry permission. This adapter has
no automatic uncertain-effect recovery or replay. If the effect receipt was
saved but its Session promotion note was lost, the next invocation reports
`receipt_saved_promotion_note_missing` with the exact private receipt; it does
not repeat the mutation. Message posting itself uses native retained replay
keys; after message eviction a key is not an archive or proof of earlier effects.

Freshness checks are sequential observations. Existing APIs atomically fence the
destination write, but cannot transact source messages/files, approval notes
and destination together. A source or Project mapping can change after the final read; this adapter
makes no atomic all-source freshness claim. No live deployment or real Host UI
acceptance is established by the fixture tests. Run their real loopback HTTP
transport checks with:

```sh
PYTHONDONTWRITEBYTECODE=1 python3 -m unittest discover -s integrations/codex -p test_experience_proposals.py -q
```

Run the opt-in native Server/WebSocket Runner acceptance without a model:

```sh
python3 scripts/e2e_experience_proposals_ws.py --bin-dir /absolute/path/target/dogfood
```

It creates disposable loopback services and a private project/configuration,
checks native Memory create/CAS and Skill create/guarded update, source/version
rejection, missing Memory-management scope, duplicate handling and exact saved
receipt references, then stops only its own Server/Runner. The JSON report names
the actual binaries (including their dirty flag), receipt references and cleanup
results. Temporary credentials and service state are removed. This native test
also verifies the supported Memory transport; the HTTP fixtures cover receipt
loss and mutation races separately. It does not establish real Host UI acceptance.
