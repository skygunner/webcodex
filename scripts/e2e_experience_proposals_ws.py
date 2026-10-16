#!/usr/bin/env python3
"""Opt-in native Server -> WebSocket Runner experience promotion, without a model.

All credentials, configuration, project files, Session notes and Memory are
owned by this disposable loopback fixture. No shared services are changed.
"""
from __future__ import annotations

import argparse
import json
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile

from e2e_session_continuity_ws import ROOT, Smoke, check

sys.path.insert(0, str(ROOT / "integrations/codex"))
import experience_proposals as proposals
from external_observation_hook import AdapterError
from platform_security import secure_created_path
from session_recovery import request

SCOPES = ["runtime:read", "project:read", "project:write", "session:collaborate", "memory:read", "memory:manage"]


class ExperienceSmoke(Smoke):
    def __init__(self, bin_dir, root):
        super().__init__(bin_dir, root)
        # These fixtures only need task-owned configuration and local file I/O.
        self.env = {k: v for k, v in self.env.items() if k in {
            "PATH", "SYSTEMROOT", "WINDIR", "PATHEXT", "TMP", "TEMP", "LANG",
            "XDG_CONFIG_HOME", "XDG_DATA_HOME", "XDG_STATE_HOME", "GIT_CONFIG_GLOBAL",
            "GIT_CONFIG_NOSYSTEM", "RUST_LOG"}}
        self.env["WEBCODEX_AUTHORITY_MODE"] = "trusted_agent"
        self.stage = "setup"
        self.checks = []
        self.mutations = 0
        self.receipts = []
        self.file_guards = []

    def native(self, tool, params, token=None, success=True):
        if tool in {"read_memory", "set_memory"}:
            result = proposals.memory_request(self.config, tool, params)
        else:
            result = self.post("/api/tools/call", {"tool": tool, "params": params}, token or self.owner)
        check(result.get("success") is success,
              f"native {tool} expected success={success}; error_kind={result.get('output', {}).get('error_kind')}")
        return result.get("output", {})

    def adapter_request(self, config, route, body, timeout=6):
        if route == "/api/tools/call" and body["tool"] in {"set_memory", "write_project_file"}:
            self.mutations += 1
            if body["tool"] == "write_project_file":
                params = body["params"]
                self.file_guards.append({"overwrite": params["overwrite"],
                                         "read_revision_present": type(params.get("expected_read_revision")) is int})
        return request(config, route, body, timeout)

    def adapter_memory_request(self, config, tool, params, timeout=6):
        if tool == "set_memory":
            self.mutations += 1
        return proposals.memory_request(config, tool, params, timeout)

    def candidate(self, destination, content="Check bounded native inspection results."):
        source = {"kind": "file", "ref": "README.md", "sha256": self.adapter.file_hash("README.md")}
        return {"schema": "webcodex.experience-proposal.v1", "summary": "Native fixture experience",
                "content": content, "conditions": ["When running the disposable native fixture"],
                "sources": [source], "destination": destination}

    def approve(self, saved, api=None):
        result = proposals.approve(api or self.adapter, saved["message_id"], saved["version"], saved["destination_hash"])
        self.receipts.append({"candidate_message_id": saved["message_id"], "version": saved["version"],
                              "approval_message_id": result["approval_message_id"],
                              "promotion_message_id": result.get("promotion_message_id"), "receipt": result["receipt"]})
        return result

    def expect_adapter_failure(self, action, category=None):
        try:
            action()
        except AdapterError as error:
            if category:
                check(str(error) == category, "unexpected adapter failure category")
            return
        except OSError:
            # HTTP scope admission can reject before Runtime ToolResult creation.
            check(category is None, "unexpected transport rejection")
            return
        raise AssertionError("expected adapter rejection")

    def setup(self):
        self.start_server()
        check(self.post("/api/users/create", {"username": "experience-owner"}, self.bootstrap).get("success") is True,
              "temporary user creation failed")
        self.owner = self.pat("experience-owner", SCOPES)
        self.no_memory_manage = self.pat("experience-owner", [s for s in SCOPES if s != "memory:manage"])
        agent = self.post("/api/agent-tokens/create", {"username": "experience-owner",
                          "client_id": "experience-runner", "name": "experience-native-e2e"}, self.bootstrap)
        check(isinstance(agent.get("token"), str), "temporary Runner token creation failed")
        self.project_dir = self.root / "project"
        registry = self.root / "projects"
        self.project_dir.mkdir()
        registry.mkdir()
        subprocess.run(["git", "init", "-q", "-b", "main", str(self.project_dir)],
                       env=self.env, stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL,
                       stderr=subprocess.PIPE, check=True)
        (self.project_dir / "README.md").write_text("# Native experience fixture\n")
        (registry / "project.toml").write_text(f'id = "fixture"\npath = {json.dumps(str(self.project_dir))}\n')
        runner_config = self.root / "runner.toml"
        runner_config.write_text("\n".join([
            f'server_url = "{self.url}"', f'token = {json.dumps(agent["token"])}',
            'client_id = "experience-runner"', 'owner = "experience-owner"', 'transport = "websocket"',
            f'project_registry_dir = {json.dumps(str(registry))}', '[policy]',
            'allow_cwd_anywhere = false', f'allowed_roots = [{json.dumps(str(self.project_dir))}]']) + "\n")
        secure_created_path(runner_config)
        self.runner = self.launch("webcodex-runner", ["--config", str(runner_config)], self.env)
        self.project = "agent:experience-runner:fixture"
        self.wait_ready(self.project, self.owner)
        session = self.native("work_on_project", {"project": self.project,
                              "instruction": "Verify explicit experience candidates through native runtime"})
        self.session = session["session_id"]
        state = self.root / "approval-state"
        state.mkdir()
        secure_created_path(state)
        auth = self.root / "authorization"
        auth.write_text("Bearer " + self.owner)
        secure_created_path(auth)
        self.config = {"server_url": self.url, "authorization_file": str(auth), "project": self.project,
                       "project_root": str(self.project_dir), "workflow_session_id": self.session,
                       "local_session_id": "native-experience-fixture", "state_dir": str(state)}
        self.adapter = proposals.RuntimeAPI(self.config, caller=self.adapter_request, memory_caller=self.adapter_memory_request)

    def run(self):
        self.setup()
        self.stage = "proposal_and_memory_create"
        candidate = self.candidate({"kind": "memory", "memory_key": "native-fixture", "expected_revision": None})
        saved = proposals.propose(self.adapter, candidate)
        check(self.mutations == 0, "unapproved proposal mutated guidance")
        check(self.adapter.message(saved["message_id"])["kind"] == "proposal", "candidate was not a native proposal")
        result = self.approve(saved)
        check(result["status"] == "promoted", "native Memory create was not recorded")
        memory = self.native("read_memory", {"project": self.project, "memory_key": "native-fixture"})
        check(memory["body"] == proposals.guidance_content(candidate), "native Memory body differs from review")
        check(memory["revision"] == result["receipt"]["revision"], "native Memory receipt revision mismatch")
        mutation_count = self.mutations
        check(self.approve(saved)["status"] == "already_recorded", "duplicate approval did not return receipt")
        check(self.mutations == mutation_count, "duplicate approval repeated guidance write")
        self.checks.append("native_proposal_memory_create_duplicate_receipts")

        self.stage = "memory_revision_cas"
        update = self.candidate({"kind": "memory", "memory_key": "native-fixture", "expected_revision": memory["revision"]},
                                "Inspect exact native receipts before continuing.")
        self.approve(proposals.propose(self.adapter, update))
        current = self.native("read_memory", {"project": self.project, "memory_key": "native-fixture"})
        check(current["revision"] != memory["revision"], "Memory update did not advance CAS revision")
        check(current["body"] == proposals.guidance_content(update), "native Memory CAS body mismatch")
        self.expect_adapter_failure(lambda: proposals.propose(self.adapter, update), "destination_changed")
        self.checks.append("native_memory_cas_update_stale_revision_rejected")

        self.stage = "project_skill_create_update"
        path = ".agents/skills/fixture-experience/SKILL.md"
        skill = self.candidate({"kind": "skill", "path": path, "expected_sha256": None},
                              "---\nname: fixture-experience\ndescription: Native fixture\n---\nInspect exact receipts.")
        self.approve(proposals.propose(self.adapter, skill))
        created_hash = self.adapter.file_hash(path)
        check((self.project_dir / path).read_text() == proposals.guidance_content(skill), "native Skill create mismatch")
        skill["destination"]["expected_sha256"] = created_hash
        skill["content"] += "\nVerify the source revision."
        self.approve(proposals.propose(self.adapter, skill))
        check(self.adapter.file_hash(path) != created_hash, "guarded native Skill write did not change bytes")
        check((self.project_dir / path).read_text() == proposals.guidance_content(skill), "native Skill update mismatch")
        check(self.file_guards == [{"overwrite": False, "read_revision_present": False},
                                   {"overwrite": True, "read_revision_present": True}],
              "native Skill update omitted its runtime read revision")
        self.checks.append("native_project_skill_create_guarded_update")

        self.stage = "source_and_version_guards"
        negative = self.candidate({"kind": "memory", "memory_key": "must-stay-absent", "expected_revision": None})
        saved_negative = proposals.propose(self.adapter, negative)
        before = self.mutations
        self.expect_adapter_failure(lambda: proposals.approve(self.adapter, saved_negative["message_id"], "0" * 64,
                                    saved_negative["destination_hash"]), "candidate_version_changed")
        (self.project_dir / "README.md").write_text("# Changed native source\n")
        self.expect_adapter_failure(lambda: self.approve(saved_negative), "source_changed")
        check(self.mutations == before, "stale version/source dispatched guidance mutation")
        self.checks.append("native_candidate_version_and_source_change_no_write")

        self.stage = "native_permission_denial"
        denied_candidate = self.candidate({"kind": "memory", "memory_key": "denied-fixture", "expected_revision": None})
        denied_saved = proposals.propose(self.adapter, denied_candidate)
        denied_auth = self.root / "denied-authorization"
        denied_auth.write_text("Bearer " + self.no_memory_manage)
        secure_created_path(denied_auth)
        denied_api = proposals.RuntimeAPI(self.config | {"authorization_file": str(denied_auth)}, caller=self.adapter_request, memory_caller=self.adapter_memory_request)
        self.expect_adapter_failure(lambda: self.approve(denied_saved, denied_api))
        absent = self.native("read_memory", {"project": self.project, "memory_key": "denied-fixture"}, success=False)
        check(absent.get("error_kind") == "memory_not_found", "scope-denied Memory unexpectedly exists")
        self.expect_adapter_failure(lambda: self.approve(denied_saved), "promotion_outcome_unresolved_do_not_retry")
        check(not (self.project_dir / ".agents/skills/denied-fixture/SKILL.md").exists(), "Memory denial fell back to Skill")
        self.checks.append("native_memory_scope_denial_no_fallback_no_blind_retry")

        self.stage = "native_saved_receipt_refs"
        for receipt in self.receipts:
            self.adapter.message(receipt["candidate_message_id"])
            self.adapter.message(receipt["approval_message_id"])
            if receipt["promotion_message_id"]:
                self.adapter.message(receipt["promotion_message_id"])
        check(not self.requests, "model requests were unexpectedly made")
        self.checks.append("native_exact_saved_receipt_refs_no_model")
        return {"passed": True, "checks": self.checks, "mutation_dispatches": self.mutations,
                "receipts": self.receipts, "model_requests": 0}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--bin-dir", type=Path, default=ROOT / "target/dogfood")
    args = parser.parse_args()
    check(shutil.which("git") is not None, "git is required")
    for binary in ("webcodex-server", "webcodex-runner"):
        check((args.bin_dir / binary).is_file(), "build dogfood Server/Runner binaries first")
    with tempfile.TemporaryDirectory(prefix="webcodex-experience-e2e-") as temporary:
        smoke = ExperienceSmoke(args.bin_dir.resolve(), Path(temporary))
        try:
            report = smoke.run()
        except Exception:
            # Neither credentials nor rejected runtime payloads enter public diagnostics.
            print(json.dumps({"passed": False, "stage": smoke.stage, "checks": smoke.checks}), file=sys.stderr)
            raise
        finally:
            smoke.close()
        report["cleanup"] = {"server_stopped": smoke.server is None or smoke.server.poll() is not None,
                             "runner_stopped": smoke.runner is None or smoke.runner.poll() is not None}
        check(all(report["cleanup"].values()), "task-owned process cleanup incomplete")
        report["binary_versions"] = {}
        for binary in ("webcodex-server", "webcodex-runner"):
            version = subprocess.run([str(args.bin_dir.resolve() / binary), "--version"], env=smoke.env,
                                     input="", capture_output=True, text=True, check=True, timeout=10)
            report["binary_versions"][binary] = version.stdout.strip()[:512]
        print(json.dumps(report, sort_keys=True))


if __name__ == "__main__":
    main()
