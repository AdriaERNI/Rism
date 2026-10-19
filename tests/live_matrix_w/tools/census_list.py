"""List LRISL* residue on the VM server + delete it."""
import json
import subprocess
import time

p = subprocess.Popen([r"C:\RismL\bin\rism.exe", "mcp"],
                     stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                     text=True, encoding="utf-8", errors="replace")
nid = [1]


def send(o):
    p.stdin.write(json.dumps(o) + "\n")
    p.stdin.flush()


def rd(w, t=60):
    dl = time.time() + t
    while time.time() < dl:
        l = p.stdout.readline()
        if not l:
            raise RuntimeError("EOF")
        try:
            m = json.loads(l)
        except json.JSONDecodeError:
            continue
        if m.get("id") == w:
            return m


def call(tool, args):
    nid[0] += 1
    send({"jsonrpc": "2.0", "id": nid[0], "method": "tools/call",
          "params": {"name": tool, "arguments": args}})
    m = rd(nid[0])
    res = m.get("result", {})
    return ("\n".join(c.get("text", "") for c in res.get("content", [])
                      if c.get("type") == "text"),
            bool(res.get("isError")))


send({"jsonrpc": "2.0", "id": 1, "method": "initialize",
      "params": {"protocolVersion": "2024-11-05", "capabilities": {},
                 "clientInfo": {"name": "c", "version": "1"}}})
rd(1)
send({"jsonrpc": "2.0", "method": "notifications/initialized"})

t, e = call("execute_sql", {"query":
    "SELECT NAME FROM %Dictionary.ClassDefinition WHERE NAME %STARTSWITH 'LRISL' OR NAME %STARTSWITH 'RismCI'"})
names = [r[0] for r in json.loads(t).get("rows", [])]
print("CLS_RESIDUE", names)
t, e = call("execute_sql", {"query":
    "SELECT NAME FROM %Dictionary.DocumentMapping WHERE NAME %STARTSWITH 'LRISL' OR NAME %STARTSWITH 'RismCI'"})
try:
    d = json.loads(t)
    print("DOCS", d.get("row_count"), d.get("rows"))
except Exception:
    print("DOCS-QUERY-ERR", t[:160])
for nm in list(names):
    t2, e2 = call("delete_document", {"name": nm + ".cls"})
    print("DEL", nm, "err=", e2, t2[:80])
t, e = call("execute_sql", {"query":
    "SELECT COUNT(*) AS C FROM %Dictionary.ClassDefinition WHERE NAME %STARTSWITH 'LRISL' OR NAME %STARTSWITH 'RismCI'"})
print("CLS_AFTER", json.loads(t).get("rows"))
p.kill()
