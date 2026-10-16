#!/usr/bin/env python3
"""Explicit, bounded experience proposals; approval never grants runtime authority."""

import argparse
import hashlib
import json
from pathlib import Path, PurePosixPath
import re
import sys
import urllib.error
import urllib.request

from external_observation_hook import AdapterError, MAX_BYTES, NoRedirect, load_config, private_file, state_lock
from session_recovery import discover_project, request

MAX_PROPOSAL_BYTES = 7500
MAX_STATE_FILES = 1024
SHA = re.compile(r"[0-9a-f]{64}")
MESSAGE_ID = re.compile(r"wc_msg_(?:[A-Za-z0-9_-]{16}|[0-9a-f]{32})")


def encoded(value):
    return json.dumps(value, ensure_ascii=False, sort_keys=True, separators=(",", ":"))


def fingerprint(value):
    return hashlib.sha256(encoded(value).encode()).hexdigest()


def text(value, limit):
    return isinstance(value, str) and bool(value.strip()) and len(value.encode()) <= limit and "\0" not in value


def path_valid(value):
    return (text(value, 512) and not value.startswith("/") and "\\" not in value
            and ":" not in value and all(p not in {"", ".", ".."} for p in value.split("/")))


def require(condition, error):
    if not condition:
        raise AdapterError(error)


def memory_request(config, tool, params, timeout=6):
    # Memory intentionally requires Stateless MCP 2026 admission, never generic REST.
    authorization = private_file(config["authorization_file"]).decode().strip()
    require(bool(authorization) and "\n" not in authorization and "\r" not in authorization,
            "invalid_authorization_file")
    body = {"jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": {
        "name": tool, "arguments": params, "_meta": {
            "io.modelcontextprotocol/protocolVersion": "2026-07-28",
            "io.modelcontextprotocol/clientCapabilities": {}}}}
    req = urllib.request.Request(config["server_url"].rstrip("/") + "/mcp", data=encoded(body).encode(),
        method="POST", headers={"Authorization": authorization, "Content-Type": "application/json",
        "Accept": "application/json, text/event-stream", "MCP-Protocol-Version": "2026-07-28",
        "MCP-Method": "tools/call", "MCP-Name": tool})
    opener = urllib.request.build_opener(urllib.request.ProxyHandler({}), NoRedirect())
    with opener.open(req, timeout=timeout) as response:
        raw = response.read(MAX_BYTES + 1)
    require(len(raw) <= MAX_BYTES, "oversized_memory_response")
    reply = json.loads(raw)
    require(isinstance(reply, dict) and reply.get("jsonrpc") == "2.0" and type(reply.get("id")) is int and reply.get("id") == 1
            and isinstance(reply.get("result"), dict), "memory_protocol_response_invalid")
    result = reply["result"].get("structuredContent")
    require(isinstance(result, dict), "memory_protocol_response_invalid")
    error_flag = reply["result"].get("isError", False)
    require(type(error_flag) is bool and not (error_flag and result.get("success") is True),
            "memory_protocol_response_invalid")
    return result


class RuntimeAPI:
    def __init__(self, config, caller=request, memory_caller=memory_request):
        self.config = config
        self.caller = caller
        self.memory_caller = memory_caller

    def check_project(self):
        current = discover_project(self.config, self.config["project_root"], self.caller)
        require(current["project"] == self.config["project"], "project_mapping_changed")

    def call(self, tool, params, allow_missing=False):
        if tool in {"read_memory", "set_memory"}:
            result = self.memory_caller(self.config, tool, params)
        else:
            result = self.caller(self.config, "/api/tools/call", {"tool": tool, "params": params})
        require(isinstance(result, dict) and isinstance(result.get("output"), dict), "runtime_response_invalid")
        output = result["output"]
        if result.get("success") is not True:
            if (allow_missing and tool == "read_memory" and output.get("error_kind") == "memory_not_found"
                    and output.get("project") == self.config["project"] and output.get("state_changed") is False):
                return None
            # Error text may contain sensitive input; report only this fixed category.
            raise AdapterError("runtime_call_rejected_or_uncertain")
        return output

    def message(self, message_id):
        require(isinstance(message_id, str) and MESSAGE_ID.fullmatch(message_id), "invalid_message_id")
        session = self.config["workflow_session_id"]
        output = self.call("list_session_messages", {"session_id": session, "message_id": message_id, "limit": 1})
        require(output.get("session_id") == session and isinstance(output.get("messages"), list)
                and len(output["messages"]) == 1, "saved_message_unavailable")
        message = output["messages"][0]
        require(isinstance(message, dict) and message.get("session_id") == session and message.get("message_id") == message_id
                and text(message.get("message"), 32000), "saved_message_identity_mismatch")
        return message

    def post(self, payload, kind, reply_to=None):
        body = encoded(payload)
        require(len(body.encode()) <= MAX_PROPOSAL_BYTES, "proposal_too_large")
        session = self.config["workflow_session_id"]
        params = {"session_id": session, "kind": kind, "message": body, "tags": ["experience-proposal"],
                  "priority": "normal", "requires_ack": False, "reply_to": reply_to,
                  "delivery_key": "experience-" + fingerprint([kind, reply_to, payload])}
        output = self.call("post_session_message", params)
        message = output.get("message", {})
        require(isinstance(message, dict) and output.get("session_id") == session
                and isinstance(output.get("message_id"), str) and MESSAGE_ID.fullmatch(output["message_id"])
                and message.get("message_id") == output["message_id"] and message.get("session_id") == session
                and message.get("message") == body and message.get("kind") == kind
                and message.get("reply_to") == reply_to, "message_receipt_invalid")
        return output["message_id"]

    def file_hash(self, path, allow_missing=False):
        output = self.call("read_project_artifact_metadata", {
            "project": self.config["project"], "path": path, "allow_missing": allow_missing})
        require(output.get("path") == path, "file_identity_mismatch")
        if allow_missing and output.get("exists") is False and output.get("missing") is True:
            return None
        require(output.get("exists") is True and isinstance(output.get("sha256"), str) and SHA.fullmatch(output["sha256"]), "file_hash_missing")
        return output["sha256"]


def message_hash(message):
    # Exclude ACK/projection counters; lifecycle and replacement invalidate the reference.
    return fingerprint({key: message.get(key) for key in (
        "session_id", "message_id", "created_at", "kind", "status", "priority", "message", "tags",
        "reply_to", "requires_ack", "author_session_id", "resolved_at", "resolution", "closure_kind",
        "superseded_by_message_id", "supersedes_message_id", "resolved_by_message_id", "completion_id")})


def validate_candidate(candidate):
    require(isinstance(candidate, dict) and set(candidate) == {
        "schema", "summary", "content", "conditions", "sources", "destination"}
        and candidate.get("schema") == "webcodex.experience-proposal.v1", "candidate_schema_invalid")
    require(text(candidate["summary"], 256) and text(candidate["content"], 4096), "candidate_text_invalid")
    conditions = candidate["conditions"]
    require(isinstance(conditions, list) and 1 <= len(conditions) <= 8
            and all(text(c, 256) for c in conditions), "conditions_required")
    sources = candidate["sources"]
    require(isinstance(sources, list) and 1 <= len(sources) <= 8, "bounded_sources_required")
    for source in sources:
        require(isinstance(source, dict) and set(source) == {"kind", "ref", "sha256"}
                and source.get("kind") in {"file", "session_message"}
                and isinstance(source.get("sha256"), str) and SHA.fullmatch(source["sha256"]), "source_invalid")
        require(path_valid(source["ref"]) if source["kind"] == "file" else
                isinstance(source["ref"], str) and MESSAGE_ID.fullmatch(source["ref"]), "source_ref_invalid")
    dest = candidate["destination"]
    require(isinstance(dest, dict), "destination_invalid")
    if dest.get("kind") == "memory":
        require(set(dest) == {"kind", "memory_key", "expected_revision"} and text(dest["memory_key"], 128)
                and (dest["expected_revision"] is None or
                     isinstance(dest["expected_revision"], str) and
                     re.fullmatch(r"wc_memrev_[A-Za-z0-9_-]{43}", dest["expected_revision"])), "memory_destination_invalid")
    elif dest.get("kind") == "skill":
        require(set(dest) == {"kind", "path", "expected_sha256"} and path_valid(dest["path"])
                and len(PurePosixPath(dest["path"]).parts) == 4
                and dest["path"].startswith(".agents/skills/") and dest["path"].endswith("/SKILL.md")
                and (dest["expected_sha256"] is None or isinstance(dest["expected_sha256"], str)
                     and SHA.fullmatch(dest["expected_sha256"])), "skill_destination_invalid")
        require(re.match(r"\A---\r?\nname: [^\r\n]+\r?\ndescription: [^\r\n]+\r?\n---\r?\n", candidate["content"]),
                "skill_frontmatter_required")
    else:
        raise AdapterError("destination_invalid")
    require(len(encoded(candidate).encode()) <= MAX_PROPOSAL_BYTES, "proposal_too_large")


def validate_sources(api, candidate):
    for source in candidate["sources"]:
        current = api.file_hash(source["ref"]) if source["kind"] == "file" else message_hash(api.message(source["ref"]))
        require(current == source["sha256"], "source_changed")


def guidance_content(candidate):
    return candidate["content"] + "\n\nApplies when:\n" + "\n".join("- " + c for c in candidate["conditions"])


def destination_params(api, candidate):
    dest = candidate["destination"]
    project = api.config["project"]
    if dest["kind"] == "memory":
        current = api.call("read_memory", {"project": project, "memory_key": dest["memory_key"]}, allow_missing=True)
        require(current is None if dest["expected_revision"] is None else
                current is not None and current.get("project") == project
                and current.get("memory_key") == dest["memory_key"]
                and current.get("revision") == dest["expected_revision"], "destination_changed")
        body = guidance_content(candidate)
        return "set_memory", {"project": project, "memory_key": dest["memory_key"], "summary": candidate["summary"],
                              "body": body, "priority": "normal", "bootstrap": False, "tags": ["experience"],
                              "expected_revision": dest["expected_revision"],
                              "session_id": api.config["workflow_session_id"]}
    path = dest["path"]
    require(api.file_hash(path, allow_missing=True) == dest["expected_sha256"], "destination_changed")
    params = {"project": project, "path": path, "content": guidance_content(candidate), "overwrite": False,
              "session_id": api.config["workflow_session_id"]}
    if dest["expected_sha256"] is not None:
        output = api.call("read_files", {"project": project, "items": [{"path": path, "limit": 1}]})
        items = output.get("items")
        require(output.get("project", project) == project and isinstance(items, list) and len(items) == 1
                and isinstance(items[0], dict) and items[0].get("path") == path
                and items[0].get("success", True) is True, "destination_read_invalid")
        item_output = items[0].get("output")
        require(isinstance(item_output, dict) and item_output.get("path", path) == path, "destination_read_invalid")
        revision = item_output.get("read_revision")
        require(type(revision) is int and 1 <= revision <= 9007199254740991, "destination_revision_missing")
        # Confirm the hash after the read; the runtime revision fences subsequent writes.
        require(api.file_hash(path) == dest["expected_sha256"], "destination_changed")
        params.update(overwrite=True, expected_read_revision=revision)
    return "write_project_file", params


def load_saved(api, message_id, version):
    saved = api.message(message_id)
    require(saved.get("kind") == "proposal" and saved.get("status") == "open", "candidate_not_open_proposal")
    try:
        candidate = json.loads(saved["message"])
    except (ValueError, TypeError):
        raise AdapterError("candidate_schema_invalid") from None
    validate_candidate(candidate)
    require(fingerprint(candidate) == version, "candidate_version_changed")
    return candidate


def propose(api, candidate):
    validate_candidate(candidate)
    api.check_project()
    validate_sources(api, candidate)
    # Preflight the exact target without modifying it.
    destination_params(api, candidate)
    message_id = api.post(candidate, "proposal")
    return {"status": "proposed", "message_id": message_id, "version": fingerprint(candidate),
            "destination_hash": fingerprint(candidate["destination"])}


def receipt_for(tool, params, output):
    if tool == "set_memory":
        require(output.get("project") == params["project"] and output.get("memory_key") == params["memory_key"]
                and isinstance(output.get("memory_id"), str) and re.fullmatch(r"wc_mem_[A-Za-z0-9_-]{16}", output["memory_id"])
                and isinstance(output.get("revision"), str) and re.fullmatch(r"wc_memrev_[A-Za-z0-9_-]{43}", output["revision"])
                and type(output.get("state_changed")) is bool and type(output.get("created")) is bool
                and "old_revision" in output
                and (output.get("old_revision") == params["expected_revision"] or
                     output.get("state_changed") is False and output.get("old_revision") is None
                     and output.get("revision") == params["expected_revision"]), "promotion_receipt_invalid")
        return {k: output[k] for k in ("project", "memory_key", "memory_id", "revision", "created", "state_changed")}
    require(output.get("path") == params["path"] and output.get("execution_state") == "completed"
            and type(output.get("changed")) is bool and output.get("state_changed") == output["changed"]
            and output.get("sha256") == hashlib.sha256(params["content"].encode()).hexdigest(), "promotion_receipt_invalid")
    return {k: output[k] for k in ("path", "sha256", "execution_state", "state_changed")}


def validate_saved_receipt(config, candidate, receipt):
    """Validate retained evidence without replaying effects or requiring a current target."""
    dest, output = candidate["destination"], receipt["receipt"]
    if dest["kind"] == "memory":
        valid = (receipt["tool"] == "set_memory"
                 and set(output) == {"project", "memory_key", "memory_id", "revision", "created", "state_changed"}
                 and output.get("project") == config["project"]
                 and output.get("memory_key") == dest["memory_key"]
                 and isinstance(output.get("memory_id"), str)
                 and re.fullmatch(r"wc_mem_[A-Za-z0-9_-]{16}", output["memory_id"])
                 and isinstance(output.get("revision"), str)
                 and re.fullmatch(r"wc_memrev_[A-Za-z0-9_-]{43}", output["revision"])
                 and type(output.get("created")) is bool
                 and type(output.get("state_changed")) is bool)
    else:
        valid = (receipt["tool"] == "write_project_file"
                 and set(output) == {"path", "sha256", "execution_state", "state_changed"}
                 and output.get("path") == dest["path"]
                 and output.get("sha256") == hashlib.sha256(guidance_content(candidate).encode()).hexdigest()
                 and output.get("execution_state") == "completed"
                 and type(output.get("state_changed")) is bool)
    require(valid, "approval_state_mismatch")


def association(config):
    return fingerprint({k: config[k] for k in (
        "server_url", "project", "project_root", "workflow_session_id", "local_session_id")})


def approve(api, message_id, version, destination_hash):
    require(isinstance(version, str) and SHA.fullmatch(version)
            and isinstance(destination_hash, str) and SHA.fullmatch(destination_hash), "exact_approval_required")
    api.check_project()
    candidate = load_saved(api, message_id, version)
    require(fingerprint(candidate["destination"]) == destination_hash, "approval_destination_mismatch")
    binding = {"association": association(api.config), "message_id": message_id, "version": version,
               "destination_hash": destination_hash}
    # Locate prior intent even if private configuration changes; changed association fails closed.
    key = "experience-" + fingerprint([api.config["project"], api.config["workflow_session_id"],
                                         message_id, version, destination_hash])
    with state_lock(api.config) as state:
        names = set(state.names())
        attempt_name, receipt_name = key + "-attempt.json", key + "-receipt.json"
        if attempt_name in names:
            previous = state.read_json(attempt_name, 32768)
            require(isinstance(previous, dict)
                    and set(previous) == {"binding", "approval_message_id", "tool"}
                    and previous.get("binding") == binding
                    and isinstance(previous.get("approval_message_id"), str)
                    and MESSAGE_ID.fullmatch(previous["approval_message_id"])
                    and previous.get("tool") == ("set_memory" if candidate["destination"]["kind"] == "memory"
                                                  else "write_project_file"), "approval_state_mismatch")
            if receipt_name in names:
                receipt = state.read_json(receipt_name, 32768)
                require(isinstance(receipt, dict)
                        and set(receipt) == {"binding", "approval_message_id", "tool", "receipt"}
                        and receipt.get("binding") == binding
                        and receipt.get("approval_message_id") == previous.get("approval_message_id")
                        and receipt.get("tool") == previous.get("tool")
                        and isinstance(receipt.get("receipt"), dict),
                        "approval_state_mismatch")
                validate_saved_receipt(api.config, candidate, receipt)
                note_name = key + "-note.json"
                if note_name in names:
                    note = state.read_json(note_name, 32768)
                    require(isinstance(note, dict) and set(note) == {"promotion_message_id"}
                            and isinstance(note["promotion_message_id"], str)
                            and MESSAGE_ID.fullmatch(note["promotion_message_id"]),
                            "approval_state_mismatch")
                    return {"status": "already_recorded", **receipt,
                            "promotion_message_id": note["promotion_message_id"]}
                return {"status": "receipt_saved_promotion_note_missing", **receipt}
            raise AdapterError("promotion_outcome_unresolved_do_not_retry")
        require(len(names) + 3 <= MAX_STATE_FILES, "approval_state_capacity")
        validate_sources(api, candidate)
        tool, params = destination_params(api, candidate)
        approval = {"schema": "webcodex.experience-approval.v1", **binding,
                    "destination": candidate["destination"]}
        approval_id = api.post(approval, "note", message_id)
        # Check exact saved receipt refs, candidate and sources again before any guidance mutation.
        approved = api.message(approval_id)
        require(approved.get("kind") == "note" and approved.get("reply_to") == message_id
                and approved.get("message") == encoded(approval) and approved.get("status") == "open",
                "approval_receipt_changed")
        require(load_saved(api, message_id, version) == candidate, "candidate_version_changed")
        validate_sources(api, candidate)
        # Re-observe target after recording approval; exact destination guards remain unchanged.
        tool, params = destination_params(api, candidate)
        api.check_project()
        state.create_json(attempt_name, {"binding": binding, "approval_message_id": approval_id, "tool": tool})
        # Any missing/rejected receipt leaves the durable marker and blocks retries, even on create.
        output = api.call(tool, params)
        receipt = {"binding": binding, "approval_message_id": approval_id,
                   "tool": tool, "receipt": receipt_for(tool, params, output)}
        state.create_json(receipt_name, receipt)
        note_id = api.post({"schema": "webcodex.experience-promotion.v1", **receipt}, "note", message_id)
        state.create_json(key + "-note.json", {"promotion_message_id": note_id})
        return {"status": "promoted", "promotion_message_id": note_id, **receipt}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--config", required=True)
    sub = parser.add_subparsers(dest="command", required=True)
    source = sub.add_parser("source")
    select_source = source.add_mutually_exclusive_group(required=True)
    select_source.add_argument("--file")
    select_source.add_argument("--message")
    create = sub.add_parser("propose")
    create.add_argument("--candidate", required=True, help="Private operator-authored JSON file")
    for name in ("review", "approve"):
        action = sub.add_parser(name)
        action.add_argument("--message", required=True)
        action.add_argument("--version", required=True)
        if name == "approve":
            action.add_argument("--destination-hash", required=True)
    args = parser.parse_args()
    try:
        config = load_config(args.config)
        require(Path.cwd().resolve().is_relative_to(Path(config["project_root"])), "outside_configured_project")
        api = RuntimeAPI(config)
        if args.command == "source":
            api.check_project()
            if args.file:
                require(path_valid(args.file), "source_ref_invalid")
                result = {"kind": "file", "ref": args.file, "sha256": api.file_hash(args.file)}
            else:
                result = {"kind": "session_message", "ref": args.message,
                          "sha256": message_hash(api.message(args.message))}
        elif args.command == "propose":
            require(not Path(args.candidate).resolve().is_relative_to(Path(config["project_root"])),
                    "candidate_must_be_outside_project")
            result = propose(api, json.loads(private_file(args.candidate)))
        elif args.command == "review":
            api.check_project()
            candidate = load_saved(api, args.message, args.version)
            validate_sources(api, candidate)
            destination_params(api, candidate)
            result = {"candidate": candidate, "guidance_content": guidance_content(candidate), "version": args.version,
                      "destination_hash": fingerprint(candidate["destination"])}
        else:
            result = approve(api, args.message, args.version, args.destination_hash)
        print(encoded(result))
        return 0
    except AdapterError as error:
        category = str(error)
        print(category if re.fullmatch(r"[a-z_]{1,128}", category) else "experience_proposal_failed", file=sys.stderr)
        return 1
    except (OSError, ValueError, TypeError, KeyError, urllib.error.URLError):
        # No rejected payload or transport credentials reach output.
        print("experience_proposal_failed_inspect_exact_saved_refs_and_private_attempt_state", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
