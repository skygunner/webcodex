"""HTTP boundary fixtures for explicit proposal approval and effect uncertainty."""

from copy import deepcopy
import hashlib
from http.client import RemoteDisconnected
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import threading
import unittest

import experience_proposals as proposals
from external_observation_hook import AdapterError
from platform_security import secure_created_path


class ExperienceProposalsTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        root = Path(self.temp.name).resolve()
        project = root / "project"
        project.mkdir()
        state = root / "state"
        state.mkdir()
        secure_created_path(state)
        auth = root / "authorization"
        auth.write_text("Bearer fixture-private")
        secure_created_path(auth)
        self.config = {"server_url": "", "authorization_file": str(auth), "project_root": str(project),
                       "project": "agent:fixture:project", "workflow_session_id": "wc_sess_" + "s" * 16,
                       "local_session_id": "local-explicit", "state_dir": str(state)}
        self.messages, self.memories, self.calls, self.files = {}, {}, [], {"src/example.py": "original"}
        self.revisions = {"src/example.py": 5}
        self.hook = lambda tool, params: None
        self.current_project = self.config["project"]
        self.reject = None
        self.lose_receipt = False
        self.invalid_receipt = False
        self.mcp_error_tools = set()
        fixture = self

        class Handler(BaseHTTPRequestHandler):
            def log_message(self, *_):
                pass

            def do_POST(self):
                assert self.path in {"/api/tools/call", "/api/runtime-console/projects", "/mcp"}
                assert self.headers["Authorization"] == "Bearer fixture-private"
                body = json.loads(self.rfile.read(int(self.headers["Content-Length"])))
                if self.path == "/api/runtime-console/projects":
                    result = {"truncated": False, "projects": [{"id": fixture.current_project,
                              "path": fixture.config["project_root"]}]}
                elif self.path == "/mcp":
                    assert self.headers["MCP-Protocol-Version"] == "2026-07-28"
                    assert self.headers["MCP-Method"] == "tools/call"
                    assert self.headers["MCP-Name"] == body["params"]["name"]
                    assert body["params"]["_meta"]["io.modelcontextprotocol/protocolVersion"] == "2026-07-28"
                    native = fixture.dispatch(body["params"]["name"], body["params"]["arguments"])
                    result = ({"jsonrpc": "2.0", "id": body["id"], "result": {"structuredContent": native,
                               "isError": body["params"]["name"] in fixture.mcp_error_tools}}
                              if native is not None else None)
                else:
                    assert body["tool"] not in {"read_memory", "set_memory"}
                    result = fixture.dispatch(body["tool"], body["params"])
                if result is None:
                    self.close_connection = True
                    return
                raw = json.dumps(result).encode()
                self.send_response(200)
                self.send_header("Content-Length", str(len(raw)))
                self.end_headers()
                self.wfile.write(raw)

        self.server = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
        self.server.daemon_threads = True
        thread = threading.Thread(target=self.server.serve_forever, daemon=True)
        thread.start()
        self.addCleanup(self.server.server_close)
        self.addCleanup(self.server.shutdown)
        self.config["server_url"] = "http://127.0.0.1:" + str(self.server.server_port)
        self.api = proposals.RuntimeAPI(self.config)
        self.candidate = {"schema": "webcodex.experience-proposal.v1", "summary": "Fixture experience",
                          "content": "Check the bounded result before continuing.", "conditions": ["When inspecting this project"],
                          "sources": [{"kind": "file", "ref": "src/example.py", "sha256": self.sha("original")}],
                          "destination": {"kind": "memory", "memory_key": "experience-fixture", "expected_revision": None}}

    @staticmethod
    def sha(text):
        return hashlib.sha256(text.encode()).hexdigest()

    def dispatch(self, tool, params):
        self.calls.append((tool, deepcopy(params)))
        if tool not in {"list_session_messages", "post_session_message"}:
            assert params["project"] == self.config["project"]
        else:
            assert params["session_id"] == self.config["workflow_session_id"]
        if tool in {"set_memory", "write_project_file"}:
            assert params["session_id"] == self.config["workflow_session_id"]
        self.hook(tool, params)
        if tool == self.reject:
            return {"success": False, "output": {"error_kind": "permission_denied", "state_changed": False}}
        output = {}
        if tool == "list_session_messages":
            message = self.messages.get(params["message_id"])
            output = {"session_id": params["session_id"], "messages": [message] if message else []}
        elif tool == "post_session_message":
            mid = "wc_msg_" + str(len(self.messages) + 1).zfill(16)
            message = {"session_id": params["session_id"], "message_id": mid, "kind": params["kind"],
                       "message": params["message"], "status": "open", "reply_to": params["reply_to"]}
            self.messages[mid] = message
            output = {"session_id": params["session_id"], "message_id": mid, "message": message,
                      "replayed": False, "state_changed": True}
        elif tool == "read_project_artifact_metadata":
            content = self.files.get(params["path"])
            output = {"path": params["path"], "exists": content is not None, "missing": content is None}
            if content is not None:
                output["sha256"] = self.sha(content)
        elif tool == "read_files":
            path = params["items"][0]["path"]
            output = {"project": params["project"], "items": [{"path": path, "success": True,
                      "output": {"read_revision": self.revisions[path], "text": self.files[path].splitlines()[0]}}]}
        elif tool == "read_memory":
            memory = self.memories.get(params["memory_key"])
            if memory is None:
                return {"success": False, "output": {"project": params["project"],
                        "error_kind": "memory_not_found", "state_changed": False}}
            output = memory
        elif tool == "set_memory":
            old = self.memories.get(params["memory_key"])
            if (old and old["revision"] != params["expected_revision"]) or (not old and params["expected_revision"]):
                return {"success": False, "output": {"error_kind": "memory_changed", "state_changed": False}}
            output = {"project": params["project"], "memory_key": params["memory_key"],
                      "memory_id": "wc_mem_" + "m" * 16, "revision": "wc_memrev_" + "r" * 43,
                      "old_revision": old["revision"] if old else None, "created": old is None, "state_changed": True}
            self.memories[params["memory_key"]] = {**output, "body": params["body"]}
        elif tool == "write_project_file":
            path = params["path"]
            if ((params["overwrite"] and self.revisions[path] != params["expected_read_revision"])
                    or (not params["overwrite"] and path in self.files)):
                return {"success": False, "output": {"error_kind": "stale_file_revision", "state_changed": False}}
            self.files[path] = params["content"]
            self.revisions[path] = self.revisions.get(path, 0) + 1
            output = {"path": path, "sha256": self.sha(params["content"]),
                      "execution_state": "completed", "changed": True, "state_changed": True}
        else:
            raise AssertionError(tool)
        if tool in {"set_memory", "write_project_file"}:
            if self.lose_receipt:
                return None
            if self.invalid_receipt:
                output.pop("state_changed")
        return {"success": True, "output": output}

    def propose(self):
        return proposals.propose(self.api, self.candidate)

    def approve(self, saved):
        return proposals.approve(self.api, saved["message_id"], saved["version"], saved["destination_hash"])

    def mutation_count(self):
        return sum(tool in {"set_memory", "write_project_file"} for tool, _ in self.calls)

    def skill(self, existing=False):
        path = ".agents/skills/fixture/SKILL.md"
        self.candidate["content"] = "---\nname: fixture\ndescription: Explicit fixture\n---\nCheck bounded results."
        self.candidate["destination"] = {"kind": "skill", "path": path,
                                          "expected_sha256": self.sha("old") if existing else None}
        if existing:
            self.files[path] = "old"
            self.revisions[path] = 12
        return path

    def test_proposal_is_session_metadata_never_guidance(self):
        saved = self.propose()
        self.assertEqual(self.messages[saved["message_id"]]["kind"], "proposal")
        self.assertEqual(self.mutation_count(), 0)
        self.assertFalse(any(m["kind"] == "guidance" for m in self.messages.values()))

    def test_memory_create_approval_exact_refs_and_duplicate(self):
        saved = self.propose()
        promoted = self.approve(saved)
        self.assertEqual(promoted["status"], "promoted")
        self.assertEqual(promoted["binding"]["message_id"], saved["message_id"])
        self.assertIn("Applies when:", self.memories["experience-fixture"]["body"])
        self.assertEqual(self.approve(saved)["status"], "already_recorded")
        self.assertEqual(self.mutation_count(), 1)
        self.assertFalse(any(m["kind"] == "guidance" for m in self.messages.values()))

    def test_corrupt_private_promotion_note_cannot_override_duplicate_status(self):
        saved = self.propose()
        self.approve(saved)
        notes = list(Path(self.config["state_dir"]).glob("experience-*-note.json"))
        self.assertEqual(len(notes), 1)
        note = json.loads(notes[0].read_text())
        note["status"] = "promoted"
        notes[0].write_text(json.dumps(note))
        with self.assertRaisesRegex(AdapterError, "approval_state_mismatch"):
            self.approve(saved)
        self.assertEqual(self.mutation_count(), 1)

    def test_corrupt_private_receipt_cannot_override_duplicate_status(self):
        saved = self.propose()
        self.approve(saved)
        receipts = list(Path(self.config["state_dir"]).glob("experience-*-receipt.json"))
        self.assertEqual(len(receipts), 1)
        receipt = json.loads(receipts[0].read_text())
        receipt["status"] = "promoted"
        receipts[0].write_text(json.dumps(receipt))
        with self.assertRaisesRegex(AdapterError, "approval_state_mismatch"):
            self.approve(saved)
        self.assertEqual(self.mutation_count(), 1)

    def test_corrupt_nested_memory_receipt_cannot_report_already_recorded(self):
        saved = self.propose()
        self.approve(saved)
        path = next(Path(self.config["state_dir"]).glob("experience-*-receipt.json"))
        original = json.loads(path.read_text())
        invalid = [{}, dict(original["receipt"], project="agent:foreign:project"),
                   dict(original["receipt"], memory_key="another-key"),
                   dict(original["receipt"], memory_id="invalid"),
                   dict(original["receipt"], revision="invalid"),
                   dict(original["receipt"], state_changed=1),
                   dict(original["receipt"], created=None)]
        for receipt in invalid:
            with self.subTest(receipt=receipt):
                path.write_text(json.dumps(dict(original, receipt=receipt)))
                with self.assertRaisesRegex(AdapterError, "approval_state_mismatch"):
                    self.approve(saved)
        path.write_text(json.dumps(original))
        self.assertEqual(self.approve(saved)["status"], "already_recorded")
        self.assertEqual(self.mutation_count(), 1)

    def test_corrupt_nested_skill_receipt_cannot_report_already_recorded(self):
        self.skill()
        saved = self.propose()
        self.approve(saved)
        path = next(Path(self.config["state_dir"]).glob("experience-*-receipt.json"))
        original = json.loads(path.read_text())
        invalid = [{}, dict(original["receipt"], path=".agents/skills/other/SKILL.md"),
                   dict(original["receipt"], sha256="0" * 64),
                   dict(original["receipt"], execution_state="pending"),
                   dict(original["receipt"], state_changed=1)]
        for receipt in invalid:
            with self.subTest(receipt=receipt):
                path.write_text(json.dumps(dict(original, receipt=receipt)))
                with self.assertRaisesRegex(AdapterError, "approval_state_mismatch"):
                    self.approve(saved)
        path.write_text(json.dumps(original))
        self.assertEqual(self.approve(saved)["status"], "already_recorded")
        self.assertEqual(self.mutation_count(), 1)

    def test_memory_cas_update_passes_exact_revision(self):
        revision = "wc_memrev_" + "o" * 43
        self.memories["experience-fixture"] = {"project": self.config["project"],
                                               "memory_key": "experience-fixture", "revision": revision}
        self.candidate["destination"]["expected_revision"] = revision
        self.approve(self.propose())
        params = next(params for tool, params in self.calls if tool == "set_memory")
        self.assertEqual(params["expected_revision"], revision)

    def test_contradictory_mcp_receipt_keeps_effect_unknown_and_blocks_retry(self):
        saved = self.propose()
        self.mcp_error_tools.add("set_memory")
        with self.assertRaisesRegex(AdapterError, "memory_protocol_response_invalid"):
            self.approve(saved)
        self.assertEqual(self.mutation_count(), 1)
        with self.assertRaisesRegex(AdapterError, "promotion_outcome_unresolved_do_not_retry"):
            self.approve(saved)
        self.assertEqual(self.mutation_count(), 1)

    def test_skill_create_and_guarded_update(self):
        path = self.skill(existing=True)
        self.approve(self.propose())
        self.assertIn("Applies when:", self.files[path])
        params = next(params for tool, params in self.calls if tool == "write_project_file")
        self.assertTrue(params["overwrite"])
        self.assertEqual(params["expected_read_revision"], 12)

    def test_skill_new_file_has_no_overwrite_guard(self):
        self.skill()
        self.approve(self.propose())
        params = next(params for tool, params in self.calls if tool == "write_project_file")
        self.assertFalse(params["overwrite"])
        self.assertNotIn("expected_read_revision", params)

    def test_source_changed_before_approval_never_mutates(self):
        saved = self.propose()
        self.files["src/example.py"] = "new"
        with self.assertRaisesRegex(AdapterError, "source_changed"):
            self.approve(saved)
        self.assertEqual(self.mutation_count(), 0)

    def test_source_changed_after_approval_note_never_mutates(self):
        saved = self.propose()
        def change(tool, params):
            if tool == "post_session_message" and "experience-approval.v1" in params["message"]:
                self.files["src/example.py"] = "new"
        self.hook = change
        with self.assertRaisesRegex(AdapterError, "source_changed"):
            self.approve(saved)
        self.assertEqual(self.mutation_count(), 0)

    def test_candidate_version_and_destination_are_explicit(self):
        saved = self.propose()
        with self.assertRaisesRegex(AdapterError, "approval_destination_mismatch"):
            proposals.approve(self.api, saved["message_id"], saved["version"], "0" * 64)
        self.messages[saved["message_id"]]["message"] = proposals.encoded({**self.candidate, "summary": "changed"})
        with self.assertRaisesRegex(AdapterError, "candidate_version_changed"):
            self.approve(saved)
        self.assertEqual(self.mutation_count(), 0)

    def test_closed_or_replaced_candidate_fails(self):
        saved = self.propose()
        self.messages[saved["message_id"]]["status"] = "resolved"
        with self.assertRaisesRegex(AdapterError, "candidate_not_open_proposal"):
            self.approve(saved)

    def test_session_source_receipt_exact_id_and_hash(self):
        mid = "wc_msg_" + "e" * 16
        self.messages[mid] = {"session_id": self.config["workflow_session_id"], "message_id": mid,
                              "kind": "note", "status": "open", "message": "Explicit saved receipt", "reply_to": None}
        self.candidate["sources"] = [{"kind": "session_message", "ref": mid,
                                      "sha256": proposals.message_hash(self.messages[mid])}]
        saved = self.propose()
        self.messages[mid]["message"] = "Different receipt"
        with self.assertRaisesRegex(AdapterError, "source_changed"):
            self.approve(saved)
        self.assertEqual(self.mutation_count(), 0)

    def test_permission_denial_does_not_fallback_or_repeat(self):
        saved = self.propose()
        self.reject = "set_memory"
        with self.assertRaisesRegex(AdapterError, "runtime_call_rejected"):
            self.approve(saved)
        with self.assertRaisesRegex(AdapterError, "promotion_outcome_unresolved"):
            self.approve(saved)
        self.assertEqual(self.mutation_count(), 1)
        self.assertFalse(any(tool == "write_project_file" for tool, _ in self.calls))

    def test_cas_race_fails_without_skill_fallback(self):
        saved = self.propose()
        def race(tool, params):
            if tool == "set_memory":
                self.memories[params["memory_key"]] = {"revision": "wc_memrev_" + "x" * 43}
        self.hook = race
        with self.assertRaises(AdapterError):
            self.approve(saved)
        self.assertFalse(any(tool == "write_project_file" for tool, _ in self.calls))

    def test_skill_guard_rejects_write_race(self):
        path = self.skill(existing=True)
        saved = self.propose()
        def race(tool, params):
            if tool == "write_project_file":
                self.files[path] = "concurrent"
                self.revisions[path] += 1
        self.hook = race
        with self.assertRaises(AdapterError):
            self.approve(saved)
        self.assertEqual(self.files[path], "concurrent")

    def test_lost_effect_receipt_never_blindly_repeats(self):
        saved = self.propose()
        self.lose_receipt = True
        with self.assertRaises(RemoteDisconnected):
            self.approve(saved)
        self.assertIn("experience-fixture", self.memories)
        with self.assertRaisesRegex(AdapterError, "promotion_outcome_unresolved"):
            self.approve(saved)
        self.assertEqual(self.mutation_count(), 1)

    def test_invalid_effect_receipt_blocks_retry(self):
        saved = self.propose()
        self.invalid_receipt = True
        with self.assertRaisesRegex(AdapterError, "promotion_receipt_invalid"):
            self.approve(saved)
        with self.assertRaisesRegex(AdapterError, "promotion_outcome_unresolved"):
            self.approve(saved)
        self.assertEqual(self.mutation_count(), 1)

    def test_lost_promotion_note_preserves_effect_receipt(self):
        saved = self.propose()
        def reject_note(tool, params):
            if tool == "post_session_message" and "experience-promotion.v1" in params["message"]:
                self.reject = tool
        self.hook = reject_note
        with self.assertRaises(AdapterError):
            self.approve(saved)
        self.reject = None
        result = self.approve(saved)
        self.assertEqual(result["status"], "receipt_saved_promotion_note_missing")
        self.assertEqual(self.mutation_count(), 1)

    def test_changed_local_association_cannot_hide_prior_attempt(self):
        saved = self.propose()
        self.reject = "set_memory"
        with self.assertRaises(AdapterError):
            self.approve(saved)
        self.config["local_session_id"] = "another-local-conversation"
        self.reject = None
        with self.assertRaisesRegex(AdapterError, "approval_state_mismatch"):
            self.approve(saved)
        self.assertEqual(self.mutation_count(), 1)

    def test_candidate_changed_after_approval_receipt_fails(self):
        saved = self.propose()
        def change(tool, params):
            if tool == "post_session_message" and "experience-approval.v1" in params["message"]:
                self.messages[saved["message_id"]]["status"] = "resolved"
        self.hook = change
        with self.assertRaisesRegex(AdapterError, "candidate_not_open_proposal"):
            self.approve(saved)
        self.assertEqual(self.mutation_count(), 0)

    def test_evicted_source_receipt_fails(self):
        mid = "wc_msg_" + "e" * 16
        self.messages[mid] = {"session_id": self.config["workflow_session_id"], "message_id": mid,
                              "kind": "note", "status": "open", "message": "Saved receipt", "reply_to": None}
        self.candidate["sources"] = [{"kind": "session_message", "ref": mid,
                                      "sha256": proposals.message_hash(self.messages[mid])}]
        saved = self.propose()
        del self.messages[mid]
        with self.assertRaisesRegex(AdapterError, "saved_message_unavailable"):
            self.approve(saved)
        self.assertEqual(self.mutation_count(), 0)

    def test_native_session_permission_rejection_never_writes_target(self):
        saved = self.propose()
        self.reject = "post_session_message"
        with self.assertRaises(AdapterError):
            self.approve(saved)
        self.assertEqual(self.mutation_count(), 0)

    def test_current_runtime_project_mapping_is_required(self):
        saved = self.propose()
        self.current_project = "agent:other:project"
        with self.assertRaisesRegex(AdapterError, "project_mapping_changed"):
            self.approve(saved)
        self.assertEqual(self.mutation_count(), 0)

    def test_mapping_changed_after_approval_note_blocks_dispatch(self):
        saved = self.propose()
        def change(tool, params):
            if tool == "post_session_message" and "experience-approval.v1" in params["message"]:
                self.current_project = "agent:other:project"
        self.hook = change
        with self.assertRaisesRegex(AdapterError, "project_mapping_changed"):
            self.approve(saved)
        self.assertEqual(self.mutation_count(), 0)

    def test_cli_source_review_and_explicit_approval(self):
        root = Path(self.temp.name)
        config_path, candidate_path = root / "config.json", root / "candidate.json"
        config_path.write_text(json.dumps(self.config))
        candidate_path.write_text(json.dumps(self.candidate))
        secure_created_path(config_path)
        secure_created_path(candidate_path)
        def run(*args):
            result = subprocess.run([sys.executable, str(Path(proposals.__file__).resolve()),
                                     "--config", str(config_path), *args], input="", capture_output=True,
                                    text=True, cwd=self.config["project_root"], timeout=20)
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertNotIn("Bearer fixture-private", result.stdout + result.stderr)
            return json.loads(result.stdout)
        source = run("source", "--file", "src/example.py")
        self.assertEqual(source, self.candidate["sources"][0])
        saved = run("propose", "--candidate", str(candidate_path))
        reviewed = run("review", "--message", saved["message_id"], "--version", saved["version"])
        self.assertEqual(reviewed["guidance_content"], proposals.guidance_content(self.candidate))
        self.assertEqual(self.mutation_count(), 0)
        promoted = run("approve", "--message", saved["message_id"], "--version", saved["version"],
                       "--destination-hash", saved["destination_hash"])
        self.assertEqual(promoted["status"], "promoted")
        self.assertEqual(self.mutation_count(), 1)

    def test_source_hash_ignores_ack_counters_but_fences_resolution(self):
        message = {"message_id": "wc_msg_" + "e" * 16, "message": "receipt", "status": "resolved",
                   "resolution": "verified", "first_ack_observed_at": None}
        expected = proposals.message_hash(message)
        message["first_ack_observed_at"] = 42
        self.assertEqual(proposals.message_hash(message), expected)
        message["resolution"] = "unknown"
        self.assertNotEqual(proposals.message_hash(message), expected)

    def test_bounded_and_project_only_skill_targets(self):
        for mutate in (lambda c: c.update(content="x" * 5000), lambda c: c.update(conditions=[]),
                       lambda c: c.update(sources=c["sources"] * 9)):
            candidate = deepcopy(self.candidate)
            mutate(candidate)
            with self.assertRaises(AdapterError):
                proposals.propose(self.api, candidate)
        self.skill()
        self.candidate["destination"]["path"] = "../.agents/skills/fixture/SKILL.md"
        with self.assertRaisesRegex(AdapterError, "skill_destination_invalid"):
            self.propose()
        self.assertEqual(self.calls, [])


if __name__ == "__main__":
    unittest.main()
