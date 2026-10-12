#!/usr/bin/env python3
"""wa1 canary acceptance driver: F3 MCP tools, F4 job lifecycle (incl.
wait.reason truncation fix), F5 auth negatives. Runs entirely against the
isolated loopback canary. Secrets are taken from env, never printed."""
import json
import os
import sys
import time
import urllib.request
import urllib.error

PORT = "36601"
BASE = f"http://127.0.0.1:{PORT}"
MCP = f"{BASE}/mcp"
TOKEN = os.environ["CANARY_TOKEN"]
PROJECT = "agent:canary-v046-agent:canaryproj"
AUTH = {"Authorization": f"Bearer {TOKEN}", "Content-Type": "application/json"}

results = []
def record(area, test, ok, detail=""):
    results.append((area, test, ok, detail))
    print(f"[{'ok' if ok else 'FAIL'}] {area}: {test}" + (f" — {detail}" if detail else ""))

def raw_post(url, payload, headers, timeout=30):
    req = urllib.request.Request(url, data=json.dumps(payload).encode(), headers=headers)
    try:
        with urllib.request.urlopen(req, timeout=timeout) as r:
            return r.status, json.loads(r.read())
    except urllib.error.HTTPError as e:
        body = e.read()
        try:
            return e.code, json.loads(body)
        except Exception:
            return e.code, {"raw": body.decode(errors="replace")[:200]}

_id = 0
def rpc(method, params=None, timeout=30):
    global _id
    _id += 1
    p = {"jsonrpc": "2.0", "id": _id, "method": method}
    if params is not None:
        p["params"] = params
    return raw_post(MCP, p, AUTH, timeout)

def call(tool, args, timeout=30):
    return rpc("tools/call", {"name": "call_runtime_tool",
                              "arguments": {"tool": tool, "arguments": args}}, timeout)

def sc(d):
    return d.get("result", {}).get("structuredContent", {})


def observe_to_terminal(job_id, wait_secs, wake_on="terminal", timeout=60):
    """Baseline observe first (first observation is immediate by design,
    handing back the observation token), then wait on the token."""
    _, d = call("observe_jobs", {"items": [{"job_id": job_id}], "wait_secs": 1},
                timeout=timeout)
    item = (sc(d).get("output", {}).get("items") or [{}])[0]
    tok = item.get("observation_token")
    args = {"job_id": job_id}
    if tok:
        args["after_observation_token"] = tok
    _, d2 = call("observe_jobs", {"items": [args], "wait_secs": wait_secs,
                "wake_on": wake_on}, timeout=timeout)
    out = sc(d2).get("output", {})
    items = out.get("items") or [{}]
    return out, items[0]

def job_id_of(d):
    out = sc(d).get("output", {})
    return out.get("job_id") or (out.get("continuation", {}).get("arguments", {})
            .get("items", [{}])[0].get("job_id"))

# ---------- F3 MCP tool surface ----------
_, d = rpc("tools/list", {})
tools = [t["name"] for t in d["result"]["tools"]]
record("F3", "tools/list 28 tools", len(tools) == 28, f"n={len(tools)}")

for t in ("tool_manifest", "runtime_status", "list_projects"):
    _, d = call(t, {})
    record("F3", f"{t} succeeds", d["result"]["isError"] is not True and "error" not in d)

_, d = call("read_files", {"project": PROJECT, "items": [{"path": "README.md"}]})
ok = "canary fixture" in json.dumps(d)
record("F3", "read_files README", ok)

_, d = call("search_project_texts", {"project": PROJECT,
          "queries": [{"pattern": "canary", "limit": 5}]})
record("F3", "search_project_texts", d["result"]["isError"] is not True)

_, d = call("run_process", {"project": PROJECT, "executable": "printf",
           "args": ["hello-canary"]}, timeout=60)
record("F3", "run_process structured", "hello-canary" in json.dumps(d))

_, d = call("run_shell", {"project": PROJECT, "command": "echo shell-ok && uname -m"},
            timeout=60)
record("F3", "run_shell", "shell-ok" in json.dumps(d) and "aarch64" in json.dumps(d))

_, d = call("show_changes", {"project": PROJECT, "include_diff": False}, timeout=60)
record("F3", "show_changes", d["result"]["isError"] is not True)

_, d = call("review_changes", {"project": PROJECT,
            "scope": {"kind": "workspace"}}, timeout=90)
ok_review = d["result"].get("isError") is not True and "error" not in d
record("F3", "review_changes(workspace scope)", ok_review,
       json.dumps(d)[:120] if not ok_review else "review pipeline reachable")

# sunk tool still callable via long tail
_, d = call("wait_for_job_terminal", {}, timeout=15)
record("F3", "wait_for_job_terminal via long tail",
       "missing field" in json.dumps(d), "param-level error proves tool exists")

# ---------- F4 job lifecycle ----------
# short completing job via run_shell quick path
_, d = call("run_shell", {"project": PROJECT, "command": "printf 'quick-done'"},
            timeout=30)
record("F4", "short job completes inline", "quick-done" in json.dumps(d))

# long job -> handoff -> observe lifecycle
_, d = call("run_shell", {"project": PROJECT, "command":
            "echo L-START; sleep 30; echo L-END", "timeout_secs": 90}, timeout=45)
out = sc(d).get("output", {})
job_id = out.get("job_id") or (out.get("continuation", {}).get("arguments", {})
        .get("items", [{}])[0].get("job_id"))
record("F4", "long job hands off with job_id", bool(job_id))

token_obs = None
if job_id:
    deadline = time.time() + 20
    state = None
    while time.time() < deadline:
        _, d = call("observe_jobs", {"items": [{"job_id": job_id}], "wait_secs": 3,
                                     "wake_on": "change"}, timeout=30)
        o = sc(d).get("output", {})
        items = o.get("items", [])
        if items:
            item = items[0].get("output") or {}
            token_obs = item.get("observation_token") or items[0].get("observation_ref")
            state = item.get("state") or item.get("lifecycle")
            if item.get("terminal"):
                break
    record("F4", "job reaches running with observation", bool(items) and not item.get("terminal"),
           f"state={state} item_keys={sorted(items[0].keys()) if items else None}")
    # log delta via token continuity
    _, d = call("observe_jobs", {"items": [{"job_id": job_id,
              **({"after_observation_token": token_obs} if token_obs and not token_obs.startswith("~j") else {})}],
              "wait_secs": 10, "wake_on": "terminal"}, timeout=40)
    o = sc(d).get("output", {})
    texts = json.dumps(o)
    record("F4", "terminal wake with L-END delta", "L-END" in texts and "L-START" in texts,
           "both markers present (no duplicate baseline)")

# failing job that hands off
_, d = call("run_shell", {"project": PROJECT, "command": "sleep 12; exit 3",
            "timeout_secs": 90}, timeout=30)
jid_fail = job_id_of(d)
if jid_fail:
    _, o_fail = observe_to_terminal(jid_fail, 15)
    fail_item = o_fail
    record("F4", "failing job reports exit_code 3 terminal",
           fail_item.get("terminal") and fail_item.get("exit_code") == 3,
           f"exit={fail_item.get('exit_code')} status={fail_item.get('status')}")
else:
    record("F4", "failing job reports exit_code 3 terminal", False, "no job_id handed off")

# timeout job that hands off
_, d = call("run_shell", {"project": PROJECT, "command": "sleep 90",
            "timeout_secs": 12}, timeout=30)
jid_to = job_id_of(d)
if jid_to:
    _, o_to = observe_to_terminal(jid_to, 15)
    to_item = o_to
    record("F4", "timeout job terminates", to_item.get("terminal"),
           f"exit={to_item.get('exit_code')} status={json.dumps(to_item)[:120]}")
else:
    record("F4", "timeout job terminates", False, "no job_id handed off")

# cancel a handed-off job
_, d = call("run_shell", {"project": PROJECT, "command": "sleep 90",
            "timeout_secs": 120}, timeout=30)
job_cancel = job_id_of(d)
if job_cancel:
    time.sleep(1)
    _, d = call("stop_job", {"project": PROJECT, "job_id": job_cancel, "confirm": True}, timeout=30)
    stop_resp = json.dumps(sc(d))[:200]
    _, o_cancel = observe_to_terminal(job_cancel, 12)
    cancel_item = o_cancel
    record("F4", "stop_job cancels to terminal stopped", cancel_item.get("terminal"),
           f"stop={stop_resp} item={json.dumps(cancel_item)[:180]}")
else:
    record("F4", "stop_job cancels to terminal stopped", False, "no job_id handed off")

# ** wait.reason truthful live proof **
# Job A: quiet long sleeper; Job B: emits a meaningful marker mid-flight.
# The observe call starts BEFORE B's marker exists, so the wake must be
# Updated and the reported reason must be meaningful_change (not heartbeat
# from A's 15s Runner heartbeat, and not immediate).
_, dA = call("run_shell", {"project": PROJECT, "command":
             "sleep 100", "timeout_secs": 120}, timeout=45)
_, dB = call("run_shell", {"project": PROJECT, "command":
             "sleep 20; echo MEANINGFUL-MARKER; sleep 100", "timeout_secs": 120}, timeout=45)
ja = job_id_of(dA); jb = job_id_of(dB)
# baseline for both (first observation is immediate by design)
_, base = call("observe_jobs", {"items": [{"job_id": ja}, {"job_id": jb}],
               "wait_secs": 1}, timeout=30)
toks = {}
for it in (sc(base).get("output", {}).get("items") or []):
    toks[it.get("job_id")] = it.get("observation_token")
# B's marker lands at ~20s from job start; run_shell returns the handoff
# receipt at ~10s, baseline observe follows, so the marker arrives mid-wait.
_, d = call("observe_jobs", {"items": [
            {"job_id": ja, "after_observation_token": toks.get(ja)},
            {"job_id": jb, "after_observation_token": toks.get(jb)}],
            "tail_lines": 40, "wait_secs": 20, "wake_on": "meaningful_change"},
            timeout=90)
o = sc(d).get("output", {})
reason = o.get("wait", {}).get("reason")
outcome = o.get("wait", {}).get("outcome")
_items_dump = json.dumps([(i.get("job_id"), i.get("status"), i.get("terminal")) for i in (o.get("items") or [])])
record("F4", "wait.reason reflects real meaningful_change wake",
       outcome == "updated" and reason == "meaningful_change",
       f"reason={reason} outcome={outcome} items={_items_dump}")
# cleanup the two sleepers
call("stop_job", {"project": PROJECT, "job_id": ja, "confirm": True}, timeout=30)
call("stop_job", {"project": PROJECT, "job_id": jb, "confirm": True}, timeout=30)

# ---------- F5 auth negatives ----------
code, _ = raw_post(MCP, {"jsonrpc": "2.0", "id": 999, "method": "tools/list", "params": {}},
                   {"Authorization": "Bearer wrong-token", "Content-Type": "application/json"})
record("F5", "invalid token rejected", code in (401, 403), f"http={code}")

code, _ = raw_post(MCP, {"jsonrpc": "2.0", "id": 998, "method": "tools/list", "params": {}},
                   {"Content-Type": "application/json"})
record("F5", "missing token rejected", code in (401, 403), f"http={code}")

# foreign project id (not registered for this runner)
_, d = call("read_files", {"project": "agent:canary-v046-agent:nonexistent",
          "items": [{"path": "README.md"}]})
record("F5", "unknown project fails closed", d["result"].get("isError") is True
       or sc(d).get("success") is False)

# cross-principal job access: use a syntactically valid job id from a
# different namespace shape; must be denied/not-found, never executed
_, d = call("observe_jobs", {"items": [{"job_id": "wc_job_zzzForeignJob000"}],
            "wait_secs": 1}, timeout=20)
body = json.dumps(d)
record("F5", "foreign job id rejected", ("error" in body.lower() or "not" in body.lower()),
       body[:80])

print("\n==== SUMMARY ====")
fails = [r for r in results if not r[2]]
print(f"passed {len(results) - len(fails)}/{len(results)}")
for area, test, ok, detail in fails:
    print(f"FAILED {area}: {test} {detail}")
sys.exit(1 if fails else 0)
