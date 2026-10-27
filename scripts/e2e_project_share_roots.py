#!/usr/bin/env python3
"""Loopback-only project-share parent-root acceptance with real Server/Runner.

Build the fork binaries first:
  cargo build --locked --profile dogfood -p webcodex -p webcodex-cli -p webcodex-runner
  python3 scripts/e2e_project_share_roots.py --bin-dir target/dogfood

Requires Unix, Python 3, git. Everything runs on 127.0.0.1: a real
`webcodex share --tunnel none --auth oauth --project-roots ...` process owns a
real Server and WebSocket Runner, the harness completes the real OAuth
authorization-code + PKCE S256 flow with the share's own client, and every
project admission is driven through the MCP `work_on_project` tool surface.
No external service, production state or TLS verification bypass is used.
"""
from __future__ import annotations

import argparse
import base64
import concurrent.futures
import hashlib
import http.client
import json
import os
from pathlib import Path
import re
import secrets
import signal
import socket
import subprocess
import sys
import tempfile
import time
import urllib.parse

from e2e_session_continuity_ws import check

MCP_PROTOCOL = "2026-07-28"


def reserve_port() -> int:
    with socket.socket() as sock:
        sock.bind(("127.0.0.1", 0))
        return sock.getsockname()[1]


def git(repo: Path, *args: str) -> str:
    result = subprocess.run(
        ["git", *args], cwd=repo, capture_output=True, text=True, check=True
    )
    return result.stdout.strip()


def make_git_repo(path: Path, branch: str = "main") -> None:
    path.mkdir(parents=True, exist_ok=True)
    git(path, "init", "-b", branch)
    git(path, "config", "user.email", "fixture@example.invalid")
    git(path, "config", "user.name", "Share Roots Fixture")
    (path / "README.md").write_text("share roots fixture\n")
    git(path, "add", "-A")
    git(path, "commit", "-m", "fixture root commit")


class ShareRuntime:
    def __init__(self, bin_dir: Path, root: Path):
        self.bin_dir = bin_dir
        self.root = root
        self.public_port = reserve_port()
        self.origin = f"http://127.0.0.1:{self.public_port}"
        # The Server itself listens on the project's derived local port; the
        # public origin above is only the OAuth issuer/MCP URL identity. It is
        # discovered from the generated Runner config once the share is ready.
        self.port: int | None = None
        self.process: subprocess.Popen | None = None
        self.rpc_id = 0

    def launch(self, project_dir: Path, extra_args: list[str], tag: str = "a") -> None:
        # One log per launch: wait_ready must only ever match the fresh ready
        # message, never a previous phase's.
        log_path = self.root / f"share-{self.public_port}-{tag}.log"
        log_path.write_bytes(b"")
        self.log_path = log_path
        log = open(log_path, "ab")
        command = [
            str(self.bin_dir / "webcodex"), "share",
            "--tunnel", "none",
            "--public-url", self.origin,
            "--auth", "oauth",
            "--oauth-redirect-uri", f"{self.origin}/callback",
            "--no-copy-url",
            "--state-dir", str(self.root / "state"),
            *extra_args,
        ]
        self.process = subprocess.Popen(
            command, cwd=project_dir, stdout=log, stderr=log,
            start_new_session=True,
        )

    def discover_server_port(self) -> None:
        configs = list((self.root / "state").glob("**/runner.toml"))
        check(bool(configs), "generated Runner config not found under state dir")
        match = re.search(r'server_url\s*=\s*"http://127\.0\.0\.1:(\d+)"',
                          configs[0].read_text())
        check(bool(match), f"server_url missing from {configs[0]}")
        self.port = int(match.group(1))

    def wait_ready(self, deadline_secs: float = 90.0) -> dict:
        log_path = getattr(self, "log_path",
                           self.root / f"share-{self.public_port}-a.log")
        deadline = time.monotonic() + deadline_secs
        ready_re = re.compile(
            r"4\. Client ID: (?P<client>\S+)\n.*?5\. Client secret: (?P<secret>\S+)\n"
            r".*?Project share credential \(this share only\): (?P<credential>\S+)",
            re.S,
        )
        while time.monotonic() < deadline:
            if self.process is not None and self.process.poll() is not None:
                raise AssertionError(
                    f"share exited early with {self.process.returncode}; see {log_path}"
                )
            text = log_path.read_text(errors="replace")
            match = ready_re.search(text)
            if match:
                return {
                    "client_id": match.group("client"),
                    "client_secret": match.group("secret"),
                    "credential": match.group("credential"),
                }
            time.sleep(0.3)
        raise AssertionError(f"share readiness deadline exceeded; see {log_path}")

    def stop(self) -> None:
        if self.process is None:
            return
        try:
            os.killpg(self.process.pid, signal.SIGTERM)
        except ProcessLookupError:
            pass
        try:
            self.process.wait(timeout=20)
        except subprocess.TimeoutExpired:
            try:
                os.killpg(self.process.pid, signal.SIGKILL)
            except ProcessLookupError:
                pass
        self.process = None

    # --- plain HTTP -------------------------------------------------------
    def request(self, path: str, method: str = "GET", body: bytes | None = None,
                headers: dict | None = None) -> tuple[int, dict, bytes]:
        headers = dict(headers or {})
        if body is not None and "Content-Type" not in headers:
            headers["Content-Type"] = "application/x-www-form-urlencoded"
        check(self.port is not None, "server port not discovered yet")
        connection = http.client.HTTPConnection("127.0.0.1", self.port, timeout=30)
        try:
            connection.request(method, path, body=body, headers=headers)
            response = connection.getresponse()
            data = response.read(2 * 1024 * 1024 + 1)
            check(len(data) <= 2 * 1024 * 1024, "response exceeded bound")
            return response.status, {k.lower(): v for k, v in response.getheaders()}, data
        finally:
            connection.close()

    # --- OAuth ------------------------------------------------------------
    def authorize(self, client: dict) -> dict:
        verifier = secrets.token_urlsafe(48)
        challenge = base64.urlsafe_b64encode(
            hashlib.sha256(verifier.encode()).digest()
        ).decode().rstrip("=")
        parameters = {
            "response_type": "code",
            "client_id": client["client_id"],
            "redirect_uri": f"{self.origin}/callback",
            "scope": "runtime:read session:collaborate project:read project:write job:run",
            "state": secrets.token_urlsafe(16),
            "resource": self.origin + "/mcp",
            "code_challenge": challenge,
            "code_challenge_method": "S256",
        }
        authorize_path = "/oauth/authorize?" + urllib.parse.urlencode(parameters)
        status, _, _ = self.request(authorize_path)
        check(status == 200, "project share authorize form unavailable")
        form = dict(parameters)
        form["project_credential"] = client["credential"]
        status, headers, _ = self.request(
            "/oauth/authorize/project", "POST", urllib.parse.urlencode(form).encode(),
            headers={"Origin": self.origin},
        )
        check(status == 302, f"project share authorize rejected: {status}")
        query = urllib.parse.parse_qs(urllib.parse.urlsplit(headers.get("location", "")).query)
        check(query.get("state") == [parameters["state"]] and "code" in query,
              "OAuth redirect/code mismatch")
        status, _, raw = self.request("/oauth/token", "POST", urllib.parse.urlencode({
            "grant_type": "authorization_code",
            "code": query["code"][0],
            "client_id": client["client_id"],
            "client_secret": client["client_secret"],
            "redirect_uri": f"{self.origin}/callback",
            "resource": self.origin + "/mcp",
            "code_verifier": verifier,
        }).encode())
        check(status == 200, "S256 code exchange denied")
        tokens = json.loads(raw)
        check(isinstance(tokens.get("access_token"), str)
              and isinstance(tokens.get("refresh_token"), str), "OAuth tokens missing")
        return tokens

    # --- MCP --------------------------------------------------------------
    def mcp(self, token: str, method: str, params: dict | None = None) -> dict:
        self.rpc_id += 1
        params = dict(params or {})
        params["_meta"] = {
            "io.modelcontextprotocol/protocolVersion": MCP_PROTOCOL,
            "io.modelcontextprotocol/clientCapabilities": {},
        }
        headers = {
            "Accept": "application/json, text/event-stream",
            "Content-Type": "application/json",
            "MCP-Protocol-Version": MCP_PROTOCOL,
            "MCP-Method": method,
            "Authorization": "Bearer " + token,
        }
        if "name" in params:
            headers["MCP-Name"] = params["name"]
        status, _, raw = self.request(
            "/mcp", "POST",
            json.dumps({"jsonrpc": "2.0", "id": self.rpc_id, "method": method,
                        "params": params}).encode(),
            headers=headers,
        )
        body = raw.decode(errors="replace")
        if raw.lstrip().startswith(b"event:") or raw.lstrip().startswith(b"data:"):
            data_lines = [line[5:].strip() for line in body.splitlines()
                          if line.startswith("data:")]
            check(bool(data_lines), "empty SSE MCP response")
            body = data_lines[-1]
        reply = json.loads(body)
        check(status == 200, f"MCP {method} failed: {status} {reply}")
        return reply

    def call_tool(self, token: str, name: str, arguments: dict) -> dict:
        reply = self.mcp(token, "tools/call", {"name": name, "arguments": arguments})
        check("error" not in reply, f"tools/call {name} JSON-RPC error: {reply.get('error')}")
        result = reply["result"]
        structured = result.get("structuredContent")
        if isinstance(structured, dict):
            payload = structured
        else:
            content = result.get("content", [])
            text = "".join(part.get("text", "") for part in content
                           if part.get("type") == "text")
            try:
                payload = json.loads(text)
            except json.JSONDecodeError:
                payload = {"raw": text}
        success = result.get("isError", False) is False and payload.get("success") is not False
        return {"success": success, "output": payload}


def work_on_project(runtime: ShareRuntime, token: str, client_id: str, path: Path) -> dict:
    return runtime.call_tool(token, "work_on_project", {
        "client_id": client_id,
        "path": str(path),
        "mode": "checkout",
        "instruction": "share roots fixture acceptance",
    })


def error_kind_of(result: dict) -> str | None:
    output = result["output"]
    if isinstance(output, dict):
        if isinstance(output.get("error_kind"), str):
            return output["error_kind"]
        nested = output.get("output")
        if isinstance(nested, dict) and isinstance(nested.get("error_kind"), str):
            return nested["error_kind"]
    return None


def tool_payload(result: dict) -> dict:
    """Unwrap the nested tool-result text payload to its effective output."""
    output = result["output"]
    if isinstance(output, dict) and isinstance(output.get("output"), dict):
        return output["output"]
    return output if isinstance(output, dict) else {}


def runner_client_id(runtime: ShareRuntime, token: str) -> str:
    runners = runtime.call_tool(token, "list_runners", {"summary_only": True})
    check(runners["success"], f"list_runners failed: {runners}")
    ids = tool_payload(runners).get("runners")
    check(isinstance(ids, list) and len(ids) >= 1, f"no visible runners: {runners}")
    first = ids[0]
    client_id = first.get("client_id") if isinstance(first, dict) else None
    check(isinstance(client_id, str) and client_id, f"runner client id missing: {first}")
    return client_id


def list_project_paths(runtime: ShareRuntime, token: str) -> list[str]:
    # list_projects is console-only on Adaptive Runtime: it is reachable only
    # through the canonical gateway tool.
    projects = runtime.call_tool(token, "call_runtime_tool", {
        "tool": "list_projects",
        "arguments": {"summary_only": True},
    })
    check(projects["success"], f"list_projects failed: {projects}")
    rows = tool_payload(projects).get("projects", [])
    return [row.get("path") for row in rows if isinstance(row, dict)]


def project_id_of(result: dict) -> str | None:
    output = tool_payload(result)
    for key in ("project", "resolved_project", "id", "agent_project_id"):
        value = output.get(key)
        if isinstance(value, str) and value:
            return value
    return None


def read_file(runtime: ShareRuntime, token: str, project_ref: str, path: str) -> str:
    result = runtime.call_tool(token, "read_files", {
        "project": project_ref,
        "items": [{"path": path}],
    })
    check(result["success"], f"read_files {path} failed: {result}")
    output = tool_payload(result)
    items = output.get("items") or (output.get("output", {}) or {}).get("items")
    check(isinstance(items, list) and items, f"read_files returned no items: {output}")
    first = items[0]
    body = first.get("output", first)
    text = body.get("text") if isinstance(body, dict) else body
    check(isinstance(text, str), f"read_files produced no text: {first}")
    return text


def create_file(runtime: ShareRuntime, token: str, project_ref: str, path: str,
                content: str) -> None:
    result = runtime.call_tool(token, "edit_project_files", {
        "project": project_ref,
        "changes": [{"kind": "create", "path": path, "content": content}],
    })
    check(result["success"], f"edit_project_files create {path} failed: {result}")


def fresh_dir(path: Path) -> Path:
    path.mkdir(parents=True, exist_ok=True)
    return path


def deny_check(runtime: ShareRuntime, token: str, client_id: str, label: str,
               path: Path, token_label: str = "fresh") -> None:
    result = work_on_project(runtime, token, client_id, path)
    check(not result["success"] and error_kind_of(result) == "project_registry_scope_denied",
          f"[{token_label}] {label} was admitted: {result}")


def admit_check(runtime: ShareRuntime, token: str, client_id: str, label: str,
                path: Path, expected_id: str | None = None) -> str:
    result = work_on_project(runtime, token, client_id, path)
    check(result["success"], f"[{label}] was not admitted: {result}")
    identity = project_id_of(result)
    check(identity, f"[{label}] admitted without a project identity: {result}")
    if expected_id is not None:
        check(identity == expected_id,
              f"[{label}] identity drifted: {identity} != {expected_id}")
    return identity


def managed_worktree_check(runtime: ShareRuntime, token: str, client_id: str,
                           label: str, source: Path, source_id: str) -> None:
    before = git(source, "worktree", "list", "--porcelain")
    result = runtime.call_tool(token, "work_on_project", {
        "project": source_id,
        "mode": "worktree",
        "base_ref": "HEAD",
        "instruction": "share roots managed worktree fixture",
    })
    check(result["success"], f"[{label}] managed worktree failed: {result}")
    output = tool_payload(result)
    project = project_id_of(result)
    session_id = output.get("session_id")
    worktree = output.get("worktree", {})
    resolution = output.get("project_resolution", {})
    check(isinstance(project, str) and project.startswith("agent:") and project != source_id,
          f"[{label}] missing distinct managed project: {output}")
    check(isinstance(output.get("project_ref"), str) and output["project_ref"].startswith("~p"),
          f"[{label}] missing managed project ref: {output}")
    check(isinstance(session_id, str) and session_id.startswith("wc_sess_"),
          f"[{label}] missing session: {output}")
    check(worktree.get("managed") is True and worktree.get("base_sha") == git(source, "rev-parse", "HEAD"),
          f"[{label}] incorrect managed base: {worktree}")
    check(resolution.get("source") == "managed_worktree",
          f"[{label}] incorrect project resolution: {resolution}")
    after = git(source, "worktree", "list", "--porcelain")
    check(after != before, f"[{label}] managed worktree was not created")
    check("share roots fixture" in read_file(runtime, token, project, "README.md"),
          f"[{label}] managed project contents missing")

    # A repeated worktree request without a Session starts fresh work. Resume
    # explicitly through the source path to recover this exact managed project.
    resumed = runtime.call_tool(token, "work_on_project", {
        "client_id": client_id,
        "path": str(source),
        "mode": "worktree",
        "session_id": session_id,
        "instruction": "resume share roots managed worktree fixture",
    })
    check(resumed["success"], f"[{label}] managed worktree resume failed: {resumed}")
    resumed_output = tool_payload(resumed)
    check(project_id_of(resumed) == project and resumed_output.get("session_id") == session_id
          and resumed_output.get("continuation") == "resumed_explicitly",
          f"[{label}] managed worktree resume changed identity: {resumed_output}")
    check(git(source, "worktree", "list", "--porcelain") == after,
          f"[{label}] resume created another worktree")


class Lifecycle:
    """One launched share phase: client credentials and fresh OAuth tokens."""

    def __init__(self, tag: str):
        self.tag = tag
        self.client: dict | None = None
        self.tokens: dict | None = None


def revocation_check(runtime: ShareRuntime, token: str, kind: str, client_id: str,
                     label: str, denied_path: Path, valid_path: Path,
                     valid_id: str) -> None:
    """One matrix cell: prove this token class still reaches its registered
    project, THEN prove it cannot admit a real, never-registered directory
    that sits inside the removed/cleared root. A missing directory would be
    denied regardless of authorization, so every denied target is created
    first and asserted to have never been admitted before."""
    admit_check(runtime, token, client_id, f"{label}: valid project stays reachable",
                valid_path, expected_id=valid_id)
    deny_check(runtime, token, client_id,
               f"{label}: new dir under removed root", denied_path,
               token_label=kind)


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--bin-dir", type=Path, required=True)
    args = parser.parse_args()
    bin_dir = args.bin_dir.resolve()
    for binary in ("webcodex", "webcodex-server", "webcodex-runner"):
        check((bin_dir / binary).is_file(), f"missing binary {binary}")

    base = Path(tempfile.mkdtemp(prefix="wc-share-roots-"))
    projects_root = base / "Projects"
    share_proj = projects_root / "share-proj"
    existing_repo = projects_root / "existing-repo"
    root_a = projects_root / "roots-a"
    existing_a = root_a / "existing-a"
    root_b = projects_root / "roots-b"
    sibling = base / "Projects-extra"
    outside = base / "outside"
    make_git_repo(share_proj)
    make_git_repo(existing_repo)
    (existing_repo / "notes.txt").write_text("existing repo notes\n")
    root_a.mkdir(parents=True)
    make_git_repo(existing_a)
    root_b.mkdir(parents=True)                 # exists before its L4 grant
    (outside / "deeper").mkdir(parents=True)   # real outside target for the symlink case
    sibling.mkdir(parents=True)

    runtime = ShareRuntime(bin_dir, base)
    # OAuth transport grants stay fenced to one share launch (this branch
    # intentionally excludes restart-persistent grants), so every phase
    # re-authorizes and only that phase's fresh access token is exercised.
    # Stable share client identity and the persisted parent-root grant are the
    # properties under test across restarts.
    state = {
        "fresh_access": None, "first_client_id": None,
    }
    tokens_log: list[str] = []
    ids: dict[str, str] = {}
    client_id: str | None = None

    def token_for(kind: str) -> str:
        check(kind == "fresh", f"unsupported token class {kind}")
        value = state["fresh_access"]
        check(isinstance(value, str) and value, "fresh access token not minted yet")
        return value  # type: ignore[return-value]

    def restart(tag: str, extra_args: list[str]) -> None:
        phase = Lifecycle(tag)
        runtime.stop()
        time.sleep(1.0)  # let the previous phase's children release the port
        runtime.launch(share_proj, extra_args, tag=tag)
        phase.client = runtime.wait_ready()
        runtime.discover_server_port()
        if state["first_client_id"] is None:
            state["first_client_id"] = phase.client["client_id"]
        check(phase.client["client_id"] == state["first_client_id"],
              f"[{tag}] persisted share OAuth client identity changed: "
              f"{phase.client['client_id']} != {state['first_client_id']}")
        phase.tokens = runtime.authorize(phase.client)
        state["fresh_access"] = phase.tokens["access_token"]
        tokens_log.append(f"{tag}: fresh=minted (fenced to this launch)")

    try:
        # ---- L1: first explicit grant of root A ---------------------------
        restart("l1", ["--project-roots", str(root_a)])
        client_id = runner_client_id(runtime, token_for("fresh"))

        # The share host project is pre-registered; everything else must come
        # through the granted root. existing-repo sits OUTSIDE root A: it stays
        # denied until it would be explicitly granted or registered.
        ids["existing_a"] = admit_check(
            runtime, token_for("fresh"), client_id, "existing dir under A", existing_a)
        admit_check(runtime, token_for("fresh"), client_id,
                    "repeat A dir", existing_a, expected_id=ids["existing_a"])
        deny_check(runtime, token_for("fresh"), client_id,
                   "git repo outside any granted root", existing_repo)

        # Access regression on a project admitted through the parent root:
        # MCP read of known content, MCP create, re-read, Git status.
        marker = read_file(runtime, token_for("fresh"), ids["existing_a"], "README.md")
        check("share roots fixture" in marker, f"known content mismatch: {marker!r}")
        created = "roots-fixture-writes/evidence.txt"
        create_file(runtime, token_for("fresh"), ids["existing_a"], created,
                    "written through MCP\n")
        after = read_file(runtime, token_for("fresh"), ids["existing_a"], created)
        check(after.strip() == "written through MCP", f"created content mismatch: {after!r}")
        reentered = work_on_project(runtime, token_for("fresh"), client_id, existing_a)
        check(reentered["success"], f"re-entry failed: {reentered}")
        workspace = tool_payload(reentered).get("workspace", {})
        changed = [str(p) for p in (workspace.get("changed_paths") or [])]
        # git collapses a wholly-untracked directory to its name; accept both
        # the collapsed directory projection and the file-level one.
        reflected = any(p.rstrip("/") == "roots-fixture-writes"
                        or p.endswith("evidence.txt") for p in changed)
        check(reflected,
              f"git status did not reflect the created file: {workspace}")
        check(workspace.get("branch") == "main", f"unexpected branch: {workspace}")

        # A Git repository cloned/created AFTER the OAuth session exists is
        # admitted on demand and recognized as Git (solidifies the reviewer's
        # independently-verified case into the committed script).
        late_repo = root_a / "late-git-repo"
        make_git_repo(late_repo, branch="trunk")
        late = work_on_project(runtime, token_for("fresh"), client_id, late_repo)
        check(late["success"], f"post-session git repo not admitted: {late}")
        late_ws = tool_payload(late).get("workspace", {})
        check(late_ws.get("branch") == "trunk", f"post-session repo branch wrong: {late_ws}")
        late_id = project_id_of(late)
        admit_check(runtime, token_for("fresh"), client_id,
                    "post-session git repo repeat", late_repo,
                    expected_id=late_id)
        # A plain (non-Git) directory created after the session, already
        # covered by fresh_dir targets below, is asserted explicitly too.
        late_plain = fresh_dir(root_a / "late-plain")
        admit_check(runtime, token_for("fresh"), client_id,
                    "post-session plain dir", late_plain)

        # Four concurrent opens of ONE first-admission directory: one identity.
        fresh_same = root_a / "concurrent-same"
        fresh_same.mkdir()
        with concurrent.futures.ThreadPoolExecutor(max_workers=4) as pool:
            futures = [pool.submit(work_on_project, runtime, token_for("fresh"),
                                   client_id, fresh_same) for _ in range(4)]
            results = [future.result() for future in futures]
        check(all(result["success"] for result in results),
              f"concurrent open failures: {results}")
        identities = [project_id_of(result) for result in results]
        check(all(isinstance(identity, str) and identity for identity in identities),
              f"concurrent opens returned empty identities: {identities}")
        check(len(set(identities)) == 1,
              f"concurrent opens produced identities {set(identities)}")

        # Boundaries: outside root, prefix sibling, real-target symlink escape,
        # missing path (and the missing path stays missing).
        deny_check(runtime, token_for("fresh"), client_id, "outside dir", outside)
        deny_check(runtime, token_for("fresh"), client_id, "prefix sibling", sibling)
        escape_link = root_a / "escape-link"
        os.symlink(outside, escape_link)
        deny_check(runtime, token_for("fresh"), client_id,
                   "symlink escape (real target)", escape_link / "deeper")
        missing = root_a / "never-created"
        deny_check(runtime, token_for("fresh"), client_id, "missing dir", missing)
        check(not missing.exists(), "denied admission created the missing directory")

        # Distinct directories keep distinct identities (regression retained).
        distinct_a = root_a / "distinct-a"
        distinct_b = root_a / "distinct-b"
        distinct_a.mkdir(); distinct_b.mkdir()
        id_a = admit_check(runtime, token_for("fresh"), client_id,
                           "distinct A", distinct_a)
        id_b = admit_check(runtime, token_for("fresh"), client_id,
                           "distinct B", distinct_b)
        check(id_a != id_b, "distinct directories collided on one identity")

        # ---- L2: two more launches carrying the SAME explicit flag --------
        # This is the systemd reality: an unchanging ExecStart re-sends the
        # same --project-roots value on every restart (the P1-01 defect path).
        restart("l2a", ["--project-roots", str(root_a)])
        admit_check(runtime, token_for("fresh"), client_id,
                    "A dir after same-flag restart 1", existing_a,
                    expected_id=ids["existing_a"])
        admit_check(runtime, token_for("fresh"), client_id,
                    "A new dir after same-flag restart 1", fresh_dir(root_a / "l2a-new"))
        restart("l2b", ["--project-roots", str(root_a)])
        admit_check(runtime, token_for("fresh"), client_id,
                    "A dir after same-flag restart 2", existing_a,
                    expected_id=ids["existing_a"])
        admit_check(runtime, token_for("fresh"), client_id,
                    "A new dir after same-flag restart 2", fresh_dir(root_a / "l2b-new"))

        # ---- L3: omitting the flag restores A ------------------------------
        restart("l3", [])
        admit_check(runtime, token_for("fresh"), client_id,
                    "A restored on omission", fresh_dir(root_a / "l3-restored"))

        # ---- L4: replace A with B ------------------------------------------
        restart("l4", ["--project-roots", str(root_b)])
        # The share project is an exact-path grant outside B. A new Git source
        # under B first enters the Runner registry through ordinary checkout.
        share_id = admit_check(runtime, token_for("fresh"), client_id,
                               "exact share project", share_proj)
        managed_worktree_check(runtime, token_for("fresh"), client_id,
                               "exact share project outside B", share_proj, share_id)
        b_source = root_b / "l4-worktree-source"
        make_git_repo(b_source)
        b_source_id = admit_check(runtime, token_for("fresh"), client_id,
                                  "new Git source under B", b_source)
        managed_worktree_check(runtime, token_for("fresh"), client_id,
                               "parent-root Git source B", b_source, b_source_id)

        # An unregistered Git source outside both grants must fail before
        # registration or Git worktree creation.
        paths_before = sorted(list_project_paths(runtime, token_for("fresh")))
        worktrees_before = git(existing_repo, "worktree", "list", "--porcelain")
        denied = runtime.call_tool(token_for("fresh"), "work_on_project", {
            "client_id": client_id,
            "path": str(existing_repo),
            "mode": "worktree",
            "instruction": "outside roots managed worktree fixture",
        })
        check(not denied["success"] and error_kind_of(denied) == "project_registry_scope_denied",
              f"outside roots worktree was admitted: {denied}")
        check(sorted(list_project_paths(runtime, token_for("fresh"))) == paths_before,
              "denied worktree changed project registrations")
        check(git(existing_repo, "worktree", "list", "--porcelain") == worktrees_before,
              "denied worktree changed Git worktrees")
        admit_check(runtime, token_for("fresh"), client_id,
                    "B admitted after replacement", fresh_dir(root_b / "l4-b-new"))
        # Root A was removed: this launch's fresh token keeps registered-project
        # access but cannot admit a REAL, never-registered directory under A.
        revocation_check(runtime, token_for("fresh"), "fresh", client_id,
                         "A→B removed A (fresh)",
                         fresh_dir(root_a / "l4-a-fresh"), existing_a,
                         ids["existing_a"])
        # The top-level Projects directory is outside ANY grant.
        deny_check(runtime, token_for("fresh"), client_id,
                   "top-level outside any grant", fresh_dir(projects_root / "l4-top-new"))
        # The removed-root symlink-escape boundary stays enforced.
        deny_check(runtime, token_for("fresh"), client_id,
                   "removed A symlink escape", escape_link / "deeper")

        # ---- L5: omission restores B, not A --------------------------------
        restart("l5", [])
        admit_check(runtime, token_for("fresh"), client_id,
                    "B restored on omission", fresh_dir(root_b / "l5-b-new"))
        deny_check(runtime, token_for("fresh"), client_id,
                   "A stays removed after omission",
                   fresh_dir(root_a / "l5-a-new"))

        # ---- L6: explicit clear --------------------------------------------
        # The grant in force immediately before the clear is B, so the
        # never-registered revocation target must be a fresh directory under B
        # (a directory under the already-removed root A would be denied
        # regardless of the clear and prove nothing).
        restart("l6", ["--no-project-roots"])
        revocation_check(runtime, token_for("fresh"), "fresh", client_id,
                         "cleared (fresh)",
                         fresh_dir(root_b / "l6-b-fresh"), existing_a,
                         ids["existing_a"])
        # Cleared grant must not resurrect through the old escape link either.
        deny_check(runtime, token_for("fresh"), client_id,
                   "cleared symlink escape", escape_link / "deeper")

        # ---- L7: repeated clear is idempotent -------------------------------
        restart("l7", ["--no-project-roots"])
        deny_check(runtime, token_for("fresh"), client_id,
                   "still cleared after repeat", fresh_dir(root_b / "l7-b-new"))

        # ---- L8: omission after clear never revives old roots ---------------
        restart("l8", [])
        deny_check(runtime, token_for("fresh"), client_id,
                   "A did not revive", fresh_dir(root_a / "l8-a-new"))
        deny_check(runtime, token_for("fresh"), client_id,
                   "B did not revive", fresh_dir(root_b / "l8-b-new"))
        admit_check(runtime, token_for("fresh"), client_id,
                    "registered project access intact at the end", existing_a,
                    expected_id=ids["existing_a"])
        # Registered-project access survives the whole walk even though every
        # new-directory admission it once had is gone.
        admit_check(runtime, token_for("fresh"), client_id,
                    "fresh token registered access at the end", existing_a,
                    expected_id=ids["existing_a"])

        print("token_provenance:")
        for line in tokens_log:
            print(f"  {line}")
        print("grant_revocation=A→B and cleared; fresh-launch grants denied for new "
              "directories with registered-project access proven first")
        print("E2E_PROJECT_SHARE_ROOTS=PASS")
        print(f"fixture_root={base}")
        print("lifecycle_launches=9 client_id_stable=true")
    finally:
        runtime.stop()


if __name__ == "__main__":
    try:
        main()
    except AssertionError as error:
        print(f"E2E_PROJECT_SHARE_ROOTS=FAIL: {error}", file=sys.stderr)
        sys.exit(1)
