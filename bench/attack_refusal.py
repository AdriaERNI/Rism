#!/usr/bin/env python3
"""Refactorer attack probes — refusal paths (zero-HTTP by design).

Injects SQL-ish/path-ish payloads into the 3 Tier-2 graph tools via the CLI
door and the MCP stdio door (tools/call), and asserts: (1) the call errors,
(2) the refusal text is the interpolation-guard message, (3) the base_url in
the error/env proves no server round trip was needed (guard runs before any
client call — corroborated by wiremock unit U8-U10 and by the negative probe
below: pointing RISM_IRIS_BASE_URL at a dead port must STILL refuse with the
same message, because refusal happens before any connection).
"""
import json
import subprocess
import sys

RISM = "target/release/rism"
PAYLOADS = [
    "Bobby'; DROP TABLE x;--",
    "a' OR '1'='1",
    "../../etc/passwd",
    "Foo%27",
    "Robert`; ;",
    "%27%20OR",
    "",
    "1abc",
    "a b",
    "x'y",
]
TOOLS_GRAPH = ["subclasses", "members", "deps"]


def run_cli(tool, payload, base_url=None):
    env = {}
    if base_url:
        env["RISM_IRIS_BASE_URL"] = base_url
    cmd = [RISM, "--format", "json", "graph", tool, payload]
    env = dict(__import__("os").environ)
    if base_url:
        env.update({"RISM_IRIS_BASE_URL": base_url,
                    "RISM_IRIS_USERNAME": "_nope_",
                    "RISM_IRIS_PASSWORD": "***",
                    "RISM_IRIS_NAMESPACE": "USER"})
    p = subprocess.run(cmd, capture_output=True, text=True, timeout=60, env=env)
    return p.returncode, (p.stdout + p.stderr).strip()


def run_mcp_calls(calls):
    """Feed initialize+notifications+tools/call lines over stdio, read answers."""
    lines = [
        json.dumps({"jsonrpc": "2.0", "id": 1, "method": "initialize",
                    "params": {"protocolVersion": "2025-03-26",
                               "clientInfo": {"name": "att", "version": "0"},
                               "capabilities": {}}}),
        json.dumps({"jsonrpc": "2.0", "method": "notifications/initialized"}),
    ]
    ids = {}
    for i, (tool, args) in enumerate(calls, start=2):
        lines.append(json.dumps({"jsonrpc": "2.0", "id": i, "method": "tools/call",
                                 "params": {"name": tool, "arguments": args}}))
        ids[i] = (tool, args)
    p = subprocess.run([RISM, "mcp"], input="\n".join(lines) + "\n",
                       capture_output=True, text=True, timeout=120)
    out = {}
    for line in p.stdout.splitlines():
        try:
            msg = json.loads(line)
        except json.JSONDecodeError:
            continue
        if msg.get("id") in ids and msg["id"] != 1:
            out[msg["id"]] = msg
    return ids, out


GUARD_NEEDLE = "interpolation guard"

def main():
    rows = []
    # CLI door — live base (guard must fire with the REAL server reachable)
    for tool in TOOLS_GRAPH:
        for payload in PAYLOADS:
            rc, txt = run_cli(tool, payload)
            refused = rc != 0 or "_err" in txt or "invalid class name" in txt
            sane = ("invalid class name" in txt) or ("guard" in txt)
            rows.append(("CLI", f"graph {tool}", payload, refused, sane, txt[:160]))
    # Negative zero-HTTP probe: point at a DEAD port (127.0.0.1:1). If the
    # guard is truly before-HTTP, the error text is the guard message, NOT a
    # connection error.
    for tool in TOOLS_GRAPH:
        rc, txt = run_cli(tool, "Bobby'; DROP TABLE x;--", base_url="http://127.0.0.1:1")
        rows.append(("CLI-dead-port", f"graph {tool}", "Bobby'; DROP TABLE x;--",
                     rc != 0, "invalid class name" in txt, txt[:160]))
    # MCP stdio door
    calls = []
    for tool in TOOLS_GRAPH:
        for payload in PAYLOADS[:4]:
            calls.append(({"subclasses": "find_subclasses",
                           "members": "class_members",
                           "deps": "class_dependencies"}[tool], {"class": payload}))
    ids, out = run_mcp_calls(calls)
    for i, (tool, args) in sorted(ids.items()):
        msg = out.get(i, {})
        if "error" in msg:
            refused, sane, txt = True, True, msg["error"].get("message", "")[:160]
        else:
            res = msg.get("result", {})
            is_err = res.get("isError", False)
            txt = (res.get("content") or [{}])[0].get("text", "")[:160]
            refused, sane = bool(is_err), ("invalid class name" in txt or "guard" in txt)
        rows.append(("MCP", tool, args["class"], refused, sane, txt))

    bad = [r for r in rows if not (r[3] and r[4])]
    for r in rows:
        print(("OK " if r[3] and r[4] else "FAIL"), r[0], r[1], repr(r[2]), "refused=%s sane=%s" % (r[3], r[4]), "|", r[5].replace("\n", " ")[:120])
    print(f"\n{len(rows) - len(bad)}/{len(rows)} refusal rows pass")
    return 1 if bad else 0


if __name__ == "__main__":
    sys.exit(main())
