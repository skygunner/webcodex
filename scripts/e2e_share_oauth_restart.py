#!/usr/bin/env python3
"""Loopback-only OAuth restart-persistence acceptance with a real share process.

Build the binaries first:
  cargo build --locked --profile dogfood -p webcodex -p webcodex-cli -p webcodex-runner
  python3 scripts/e2e_share_oauth_restart.py --bin-dir target/dogfood

Everything runs on 127.0.0.1: a real "webcodex share --tunnel none --auth oauth"
process owns a real Server and WebSocket Runner, the harness completes the real
OAuth authorization-code + PKCE S256 flow with the share's own client, then
stops and restarts only its own process. No external service, production state,
real user token or TLS verification bypass is used.
"""
from __future__ import annotations

import argparse
import base64
import hashlib
import http.client
import json
import os
from pathlib import Path
import re
import secrets
import shutil
import signal
import socket
import subprocess
import sys
import tempfile
import time
import urllib.parse

MCP_PROTOCOL = "2026-07-28"
SCOPES = "runtime:read session:collaborate project:read project:write job:run"


def check(value: object, message: str) -> None:
    if not value:
        raise AssertionError(message)


def reserve_port() -> int:
    with socket.socket() as sock:
        sock.bind(("127.0.0.1", 0))
        return sock.getsockname()[1]


def git(repo: Path, *args: str) -> str:
    result = subprocess.run(
        ["git", *args], cwd=repo, capture_output=True, text=True, check=True
    )
    return result.stdout.strip()


def make_git_repo(path: Path) -> None:
    path.mkdir(parents=True, exist_ok=True)
    git(path, "init", "-b", "main")
    git(path, "config", "user.email", "fixture@example.invalid")
    git(path, "config", "user.name", "Share OAuth Restart Fixture")
    (path / "README.md").write_text("oauth restart fixture\n")
    git(path, "add", "-A")
    git(path, "commit", "-m", "fixture root commit")


class Share:
    def __init__(self, bin_dir: Path, root: Path) -> None:
        self.bin_dir = bin_dir
        self.root = root
        self.public_port = reserve_port()
        self.origin = f"http://127.0.0.1:{self.public_port}"
        self.port: int | None = None
        self.process: subprocess.Popen | None = None
        self.log_path = root / "share.log"
        self.state_dir = root / "state"
        self.rpc_id = 0

    def launch(self, project_dir: Path, persist: bool, tag: str, auth: str = "oauth",
               state_dir: Path | None = None, public_url: bool = True) -> None:
        log_path = self.root / f"share-{self.public_port}-{tag}.log"
        log_path.write_bytes(b"")
        self.log_path = log_path
        self.state_dir = state_dir or self.root / "state"
        command = [
            str(self.bin_dir / "webcodex"), "share",
            "--tunnel", "none",
            "--no-copy-url",
            "--state-dir", str(self.state_dir),
        ]
        if public_url:
            command += ["--public-url", self.origin]
        command += ["--auth", auth]
        if auth == "oauth":
            command += ["--oauth-redirect-uri", f"{self.origin}/callback"]
            if persist:
                command.append("--oauth-persist-on-restart")
        log = open(log_path, "ab")
        self.process = subprocess.Popen(
            command, cwd=project_dir, stdout=log, stderr=log, start_new_session=True
        )

    def discover_server_port(self) -> None:
        configs = list(self.state_dir.glob("**/runner.toml"))
        check(bool(configs), "generated Runner config not found under state dir")
        match = re.search(r'server_url\s*=\s*"http://127\.0\.0\.1:(\d+)"', configs[0].read_text())
        check(bool(match), f"server_url missing from {configs[0]}")
        self.port = int(match.group(1))

    def wait_ready(self, deadline_secs: float = 90.0, oauth: bool = True) -> dict:
        deadline = time.monotonic() + deadline_secs
        ready_re = re.compile(
            (
                r"4\. Client ID: (?P<client>\S+)\n.*?5\. Client secret: (?P<secret>\S+)\n"
                r".*?Project share credential \(this share only\): (?P<credential>\S+)"
            )
            if oauth
            else r"Credential \(this share only\): (?P<credential>\S+)",
            re.S,
        )
        while time.monotonic() < deadline:
            if self.process is not None and self.process.poll() is not None:
                raise AssertionError(
                    f"share exited early with {self.process.returncode}; see {self.log_path}"
                )
            match = ready_re.search(self.log_path.read_text(errors="replace"))
            if match:
                self.discover_server_port()
                if not oauth:
                    return {"credential": match.group("credential")}
                return {
                    "client_id": match.group("client"),
                    "client_secret": match.group("secret"),
                    "credential": match.group("credential"),
                }
            time.sleep(0.3)
        raise AssertionError(f"share readiness deadline exceeded; see {self.log_path}")

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
            self.process.wait(timeout=20)
        self.process = None

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

    def authorize(self, client: dict) -> dict:
        verifier = secrets.token_urlsafe(48)
        challenge = base64.urlsafe_b64encode(
            hashlib.sha256(verifier.encode()).digest()
        ).decode().rstrip("=")
        parameters = {
            "response_type": "code",
            "client_id": client["client_id"],
            "redirect_uri": f"{self.origin}/callback",
            "scope": SCOPES,
            "state": secrets.token_urlsafe(16),
            "resource": self.origin + "/mcp",
            "code_challenge": challenge,
            "code_challenge_method": "S256",
        }
        status, _, _ = self.request("/oauth/authorize?" + urllib.parse.urlencode(parameters))
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
        check(status == 200, f"S256 code exchange denied: {status} {raw[:400]!r}")
        tokens = json.loads(raw)
        check(isinstance(tokens.get("access_token"), str)
              and isinstance(tokens.get("refresh_token"), str), "OAuth tokens missing")
        return tokens

    def refresh(self, client: dict, refresh_token: str) -> tuple[int, dict]:
        status, _, raw = self.request("/oauth/token", "POST", urllib.parse.urlencode({
            "grant_type": "refresh_token",
            "refresh_token": refresh_token,
            "client_id": client["client_id"],
            "client_secret": client["client_secret"],
            "resource": self.origin + "/mcp",
        }).encode())
        try:
            return status, json.loads(raw)
        except json.JSONDecodeError:
            return status, {"raw": raw.decode(errors="replace")}

    def revoke(self, client: dict, token: str, hint: str) -> int:
        status, _, _ = self.request("/oauth/revoke", "POST", urllib.parse.urlencode({
            "token": token,
            "token_type_hint": hint,
            "client_id": client["client_id"],
            "client_secret": client["client_secret"],
        }).encode())
        return status

    def runner_transport_status(self, token: str) -> int:
        status, _, _ = self.request(
            "/api/shell/agent/poll", "POST", b"{}",
            headers={
                "Authorization": "Bearer " + token,
                "Content-Type": "application/json",
            },
        )
        return status

    def mcp_status(self, token: str) -> int:
        self.rpc_id += 1
        headers = {
            "Accept": "application/json, text/event-stream",
            "Content-Type": "application/json",
            "MCP-Protocol-Version": MCP_PROTOCOL,
            "MCP-Method": "tools/list",
            "Authorization": "Bearer " + token,
        }
        status, _, _ = self.request(
            "/mcp", "POST",
            json.dumps({"jsonrpc": "2.0", "id": self.rpc_id, "method": "tools/list",
                        "params": {"_meta": {
                            "io.modelcontextprotocol/protocolVersion": MCP_PROTOCOL,
                            "io.modelcontextprotocol/clientCapabilities": {},
                        }}}).encode(),
            headers=headers,
        )
        return status


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--bin-dir", required=True)
    args = parser.parse_args()
    bin_dir = Path(args.bin_dir).resolve()
    check((bin_dir / "webcodex").is_file(), f"webcodex binary missing under {bin_dir}")
    check(sys.platform != "win32", "POSIX-only harness")
    root = Path(tempfile.mkdtemp(prefix="webcodex-oauth-restart-"))
    project = root / "project"
    make_git_repo(project)
    share = Share(bin_dir, root)
    try:
        share.launch(project, persist=True, tag="persist-a")
        first = share.wait_ready()
        tokens = share.authorize(first)
        check(share.mcp_status(tokens["access_token"]) == 200,
              "authorized MCP request failed before restart")
        share.stop()

        share.launch(project, persist=True, tag="persist-b")
        second = share.wait_ready()
        check(second["client_id"] == first["client_id"],
              "restart changed the persisted client id")
        check(second["client_secret"] == first["client_secret"],
              "restart changed the persisted client secret")
        check(share.mcp_status(tokens["access_token"]) == 200,
              "access token stopped working after a persisting restart")
        check(share.runner_transport_status(tokens["access_token"]) in (401, 403),
              "Runner transport accepted a Server OAuth access token")
        status, refreshed = share.refresh(first, tokens["refresh_token"])
        check(status == 200, f"refresh grant denied after restart: {status} {refreshed}")
        check(isinstance(refreshed.get("access_token"), str),
              "refresh response missing access token")
        check(isinstance(refreshed.get("refresh_token"), str),
              "refresh response missing rotated refresh token")
        check(share.mcp_status(refreshed["access_token"]) == 200,
              "refreshed access token was rejected after restart")
        status, replayed = share.refresh(first, tokens["refresh_token"])
        check(status != 200,
              f"rotated refresh token was accepted twice after restart: {replayed}")
        check(share.revoke(first, refreshed["access_token"], "access_token") == 200,
              "revocation of the refreshed access token was not acknowledged")
        check(share.mcp_status(refreshed["access_token"]) == 401,
              "revoked access token still worked")
        share.stop()

        share.launch(project, persist=True, tag="persist-c")
        share.wait_ready()
        check(share.mcp_status(refreshed["access_token"]) == 401,
              "a persisting restart resurrected a revoked access token")
        check(share.mcp_status(tokens["access_token"]) == 200,
              "unrevoked access token did not survive a persisting restart")
        share.stop()

        share.launch(project, persist=False, tag="default-c")
        third = share.wait_ready()
        check(third["client_id"] == first["client_id"],
              "default-mode restart changed the persisted client id")
        check(share.mcp_status(tokens["access_token"]) == 401,
              "default-mode restart kept a pre-restart access token alive")
        status, refused = share.refresh(first, refreshed["refresh_token"])
        check(status != 200,
              f"default-mode restart kept a pre-restart refresh token alive: {refused}")
        share.stop()

        # A non-OAuth share keeps working, and the persist flag stays OAuth-only.
        share.launch(project, persist=False, tag="bearer", auth="bearer", public_url=False,
                     state_dir=root / "state-bearer")
        bearer = share.wait_ready(oauth=False)
        check(share.mcp_status(bearer["credential"]) == 200,
              "non-OAuth share rejected its own project credential")
        share.stop()
        rejected = subprocess.run(
            [str(bin_dir / "webcodex"), "share", "--tunnel", "none", "--no-copy-url",
             "--oauth-persist-on-restart", "--state-dir", str(root / "state-flag")],
            cwd=project, capture_output=True, text=True, timeout=60,
        )
        check(rejected.returncode != 0
              and "requires --auth oauth" in (rejected.stdout + rejected.stderr),
              "persist flag without --auth oauth was not rejected: "
              + rejected.stdout + rejected.stderr)

        print("PASS oauth restart persistence: client continuity, grant continuity, "
              "refresh rotation one-shot, revocation not resurrected, default-mode "
              "revocation, non-OAuth share unaffected, Runner transport rejects OAuth")
        shutil.rmtree(root, ignore_errors=True)
        return 0
    finally:
        share.stop()
        if root.exists():
            print(f"retained failed-run evidence under {root}", file=sys.stderr)


if __name__ == "__main__":
    raise SystemExit(main())
