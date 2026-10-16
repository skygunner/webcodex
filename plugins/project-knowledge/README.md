# Project Knowledge Native Tool Plugin

[English](README.md) | [简体中文](README.zh-CN.md)

An optional, local source-retrieval provider for WebCodex. It indexes Git-tracked
Markdown, UTF-8 text, and common code/configuration formats from the provider's
configured repository root. SQLite FTS5 ranks identifier terms and Chinese CJK
bigrams with BM25. No embeddings, model API, network service, Python packages, or
build step are required.

This is a native `webcodex-plugin-v1` executable, using the existing
[`plugin_tool` gateway](../../docs/PLUGINS.md). It adds no core tool, Memory store,
guidance file, or background indexing. Removing or disabling this provider has no
effect on other WebCodex features. Repository text is untrusted source data;
retrieval never grants permission to follow instructions or write Memory/guidance.

## Requirements and tests

Use Python **3.10 or newer**, Git, and a Python SQLite build with **FTS5**. During
`initialize`, the provider actually creates an FTS5 table, inserts a token, and
queries it. If this fails, Plugin admission fails with a safe FTS5 diagnostic;
there is no silent fallback to a different search engine.

The host must support descriptor-relative `os.open` and `O_NOFOLLOW` file access
(for example, Linux). Unsupported hosts fail initialization rather than weaken
symlink guards. The tests run without Server, Runner, credentials, or network:

```bash
python3 -B -m unittest discover -s plugins/project-knowledge -v
```

Tests include real subprocess protocol initialization, catalog discovery,
index/search/read, interactive flush before EOF, FTS5 failure, stale and deleted
sources, source/root replacement, sensitive and binary content, output bounds,
incremental reuse, resumable refresh, and retention of the last published index
after a failed rebuild or refresh. They do not
claim live Runner admission or `plugin_tool` permission/binding coverage.
Regressions also cover full-size punctuation-heavy sources, malformed cache
metadata, and invalid cached line coordinates. Invalid metadata/ranges return a
known rebuild error without stopping the provider or claiming false source lines.

After building dogfood Server/Runner binaries, run the separate real MCP →
Server → WebSocket Runner → native provider smoke:

```bash
python3 -B scripts/e2e_project_knowledge_ws.py --bin-dir /absolute/path/to/target/dogfood
```

It reuses the disposable service fixture, creates temporary users/credentials,
projects, Runner configuration and cache, and exercises actual admission,
Project-bound bindings, index/refresh/search/read, missing invoke scope, cross-Project
and root-argument denial, stale-source exclusion, and provider removal. It starts
no model server, touches no shared service/configuration, and cleans up afterward.

## Configure the selected Runner

Manually add this optional provider to that Runner's `runner.toml`:

```toml
[[plugins.providers]]
id = "project-knowledge"
name = "Project Knowledge"
command = "/absolute/path/to/python3"
args = ["-B", "/absolute/path/to/webcodex/plugins/project-knowledge/plugin.py"]
cwd = "/absolute/path/to/git-project"
timeout_secs = 30
```

`cwd` must be exactly the ordinary Git working-tree root, including for linked
worktrees. A subdirectory, symlink path, filesystem root, and user home root are
rejected. The process captures this root's filesystem identity and refuses a
replacement. Tool arguments cannot select a root, command, cache, or environment.
Inherited `GIT_*` overrides cannot redirect Git observations to another repository.

The cache lives at
`$XDG_CACHE_HOME/webcodex-project-knowledge/<root-hash>/index.sqlite3`, or
`~/.cache/webcodex-project-knowledge/<root-hash>/index.sqlite3` when
`XDG_CACHE_HOME` is absent. Set `XDG_CACHE_HOME` in the operator's prepared Runner
environment if necessary; callers cannot change it. Cache directories must be
owned by this user, private (mode `0700` on POSIX), outside the Project, and free
of symlink traversal. Database/build files are private (`0600`). A cache inside
the Project is rejected, even if ignored by Git. No cache path is returned to the
model. The derived SQLite file may be removed locally to discard the index;
removing the provider does not automatically erase its private cache.

Use the existing author loop with authorized credentials:

```text
webcodex plugin check --runner <runner> --plugin project-knowledge
webcodex plugin reload --runner <runner>
webcodex plugin list --runner <runner> --plugin project-knowledge
```

Continue after `check` reports `ready=true`. Check starts and disposes a candidate;
reload replaces the **complete** configured provider set and invalidates old
bindings. List/describe need `plugin:inspect`, check/reload need `plugin:manage`,
and invocation needs `plugin:invoke`. All four tools declare `projectBound=true`:
describe additionally needs `project:read`, and the existing gateway policy needs
`project:write` for invocation, including cache/source observations. This Plugin creates no credentials or
registrations and never changes Runner configuration.

## Tools and source provenance

Describe each desired tool with an explicit authorized Project matching the
provider cwd, then call its returned opaque binding through
`plugin_tool(action="call", binding=..., arguments=...)`. There is no
`webcodex plugin call` command and no outer MCP tool for this provider.

```json
{"action":"describe","runner":"<runner>","plugin":"project-knowledge","tool":"index","project":"agent:<runner>:<project-id>"}
{"action":"call","binding":"<binding from describe>","arguments":{}}
```

The current `webcodex plugin describe` CLI has no Project argument; use this
canonical `plugin_tool` describe request. The Runner rejects a binding for a
different Project root before sending tool bytes to the provider. Project binding
is routing and authorization, not a sandbox for an arbitrary native executable.

| Tool | Arguments | Behavior |
| --- | --- | --- |
| `index` | `{}` or `{"mode":"rebuild"}`; `{"mode":"refresh"}` | Default rebuild starts anew. Refresh hashes current tracked bytes, reuses unchanged chunks/FTS rows, updates changed/new sources and removes deleted/excluded sources. Only this tool writes cache data. |
| `status` | `{}` | Observe saved coverage, or `indexed=false`; never scans/rebuilds. |
| `search` | `{"query":"parseHTTPRequest 项目知识","limit":5}` | Rank existing index candidates; verify current tracked source hashes before returning excerpts. |
| `read` | `{"path":"src/model.py","source_hash":"<hash from search>","start_line":1,"end_line":12}` | Recheck eligibility and expected hash; return current source lines or a known error. |

Search hits contain `path`, `start_line`, `end_line`, `text`, `source_hash`,
`index_revision`, and `observed_revision`. Line numbers are one-based and inclusive;
the hash is SHA-256 of the complete current source bytes. Revisions are local
HEAD observations (`null` before the first commit), **not proof the file equals its committed version**: tracked
working-tree edits are indexable. Returned excerpts come from reread current
bytes, not cached text. Verification is an observation during the call, not a
lease against later edits. `read` requires the expected hash precisely to detect
those edits. Refresh or rebuild and search again after a hash mismatch.

Changed, deleted, no-longer-tracked, or newly excluded candidates are omitted;
`stale_candidates_omitted` counts omissions among examined candidates. Search
examines at most 16,384 ranked candidates; candidate/time limits can hide later matches.
`results_truncated` marks candidate, result, time, or byte limits. Zero matches
is not proof of absence. Search does not automatically rebuild the cache.

Coverage describes **one indexing run**, not freshness of the entire corpus:
`revision`, `indexed_at_unix`, `eligible_files`, `indexed_files`, `indexed_bytes`,
`chunks`, `excluded_entries`, `skipped_files`, `oversized_lines_omitted`,
`unprocessed_files`, and `stop_reason`. Format 2 also records `mode`,
`reused_files`, `updated_files`, `deleted_files`, and `remaining_files`.
Reused/updated/deleted counts describe this call's committed work; replacing a
changed source counts as updated, while removing an unavailable/excluded source
counts as deleted. `indexed_files` includes retained hints pending verification;
`remaining_files` counts all candidates still pending in the current verification
cycle, including retained hints and capacity-limited sources. `unprocessed_files`
counts pending candidates without indexed rows. None of these fields certify
current corpus freshness, even when `remaining_files=0`. Excluded entries include
unsupported, sensitive, symlink, submodule, invalid-path, and conflicted index entries; skipped
files include unreadable, hardlinked, binary, sensitive-content, or oversized
sources. Git-tracked status alone does not imply safety or completeness.

## Bounds and privacy

- At most **4,096 indexed files**, **256 KiB per file**, and **64 MiB of stored
  source bytes** per index, with a **20-second indexing work budget**. Checks occur between
  bounded reads/chunks and SQLite steps; final commit/fsync/replace can add local
  filesystem overhead. Git subprocesses use the remaining deadline and at most
  **8 MiB** of captured output. The private candidate/pending inventory is bounded
  by that tracked-file observation, while the stored source corpus retains the
  stricter 4,096-file/64-MiB caps. Excessive Git output fails without replacing the
  previous index.
- Search defaults to **5 hits**, accepts **1–20**, and has a **5-second local work
  budget**. Serialized tool results are at most **16 KiB**. Read defaults to 48
  lines, caps at 200, and trims complete lines to the same byte bound. Individual
  lines larger than 4 KiB are omitted from indexing and counted in coverage;
  a line too large for read fails explicitly.
- Only stage-zero regular tracked entries in the format allowlist are eligible.
  Hidden paths (including `.env` and `.git`), traversal, control characters,
  sensitive path names, symlinks in any source component, hardlinks, binary data,
  and recognizable key/token/password patterns are excluded. These conservative
  heuristics can omit benign text and **cannot identify every secret**. Review
  tracked content before enabling a source-retrieval provider for a caller.
- Rebuild and refresh write a private, disposable SQLite file and publish it with
  fsync plus atomic replacement. Refresh copies the published database, hashes
  current source bytes before reusing rows, and keeps unchanged chunk/FTS row IDs.
  No stat timestamp or HEAD observation is accepted as file freshness. Every
  refresh rechecks the root identity and current Git-tracked set; deleted and
  path-ineligible entries are pruned before candidate work. Changed sensitive,
  binary, oversized, or unavailable sources are removed when visited. Until then,
  retained rows are only hints: search/read still omit sources that fail current
  eligibility/hash verification.
- **Reaching the 20-second budget publishes bounded partial progress**. Only
  complete files in committed batches are retained; SQLite interruption can roll
  back the in-flight batch. A private pending set and rotating cursor let another
  explicit `index({"mode":"refresh"})` continue unfinished validation, including
  after a provider restart. A slow unfinished source cannot starve later paths.
  Finishing a cycle lets the next refresh begin another validation cycle. Rebuild
  always starts anew. **Repeated refresh cannot exceed the 4,096-file/64-MiB
  corpus caps**; `file_limit`/`corpus_limit` and remaining work may persist until
  sources or scope change. There is no promise that repeated calls index every
  eligible source.
- No-cache refresh builds the index. Format 1 caches remain readable for
  status/search; their first refresh builds format 2 from current sources rather
  than trusting missing incremental bookkeeping. Corrupt caches return a known
  rebuild error when metadata, source/continuation bookkeeping, or chunk/FTS
  integrity checks detect corruption. Git, SQLite, cache, root-identity and other
  non-budget failures discard the temporary build and retain the previously published index. Cache
  preparation that exhausts its budget also retains the previous index.
  `indexed=true` does **not** mean complete or current coverage. No automatic
  Project, Git, Memory, guidance, external model, watcher, or configuration writes.

The Runner remains authoritative for schema admission, permissions, exact
provider bindings, process lifecycle, deadlines, result validation, and uncertain
transport outcomes. After an uncertain index invocation, observe `status` before
deciding whether to rebuild; this provider never retries on the caller's behalf.
