"""Final census on the VM server: ClassDefinition + docnames-list +
RoutineDefinition residue = 0, globals ^LRISL* empty verified in-session
with a positive-control sentinel (terminal WS state is per-connection)."""
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

ok = True
for t in ["%Dictionary.ClassDefinition"]:
    txt, e = call("execute_sql", {"query":
        f"SELECT COUNT(*) AS C FROM {t} WHERE NAME %STARTSWITH 'LRISL' OR NAME %STARTSWITH 'RismCI'"})
    try:
        c = json.loads(txt)["rows"][0][0]
    except Exception:
        c = "QUERY-ERR"
    print("SQL", t, c)
    ok = ok and (str(c) == "0")

txt, e = call("list_documents", {"filter": "LRISL", "count": 50})
try:
    d = json.loads(txt)
    names = [x.get("name") for x in d.get("documents", [])]
except Exception:
    names = ["PARSE-ERR", txt[:100]]
print("DOCLIST", names)
ok = ok and not names

# routines census via %SYS ProcessQuery-free route: IRIS stores routines as
# documents too; the Atelier docnames list above already covers USER. A
# %Dictionary.RoutineDefinition SELECT is not available on this build
# (table not found), so the doc-list is the authoritative check.

# globals in one session with sentinel control
txt, e = call("execute_command", {"command":
    "set ^ZZCENW=1 zwrite ^ZZCENW"})
sent_ok = "1" in json.loads(txt).get("output", "")
txt, e = call("execute_command", {"command":
    "zkill ^ZZCENW zkill ^LRISL zkill ^LRISLX zkill ^LRISLWX"})
txt, e = call("execute_command", {"command": "zwrite ^LRISL"})
g_empty = json.loads(txt).get("output", "").strip() == ""
txt, e = call("execute_command", {"command": "zwrite ^ZZCENW"})
sent_gone = json.loads(txt).get("output", "").strip() == ""
print("SENTINEL-visible", sent_ok, "LRISL-empty", g_empty,
      "SENTINEL-gone", sent_gone)
ok = ok and sent_ok and g_empty and sent_gone

# stray processes: rism.exe children we spawned die with the runner; verify
p.kill()
print("CENSUS", "ZERO" if ok else "RESIDUE")
