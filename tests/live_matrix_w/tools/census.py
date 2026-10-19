"""End-of-lane census on the VM IRIS:
- ClassDefinition / DocumentMapping / RoutineDefinition %STARTSWITH 'LRISL'/'RismCI' = 0
- ^LRISL* globals empty, verified IN-SESSION (terminal WS is per-connection),
  with a positive-control sentinel to prove zwrite isn't vacuously empty.
Prints only counts, never data."""
import json
import subprocess
import sys

EXE = r"C:\RismL\bin\rism.exe"

p = subprocess.Popen([EXE, "mcp"], stdin=subprocess.PIPE,
                     stdout=subprocess.PIPE, text=True,
                     encoding="utf-8", errors="replace")
nid = [0]


def send(o):
    p.stdin.write(json.dumps(o) + "\n")
    p.stdin.flush()


def rd(want, t=60):
    import time
    dl = time.time() + t
    while time.time() < dl:
        line = p.stdout.readline()
        if not line:
            raise RuntimeError("EOF")
        try:
            m = json.loads(line)
        except json.JSONDecodeError:
            continue
        if m.get("id") == want:
            return m
    raise RuntimeError("timeout")


def call(tool, args):
    nid[0] += 1
    send({"jsonrpc": "2.0", "id": nid[0], "method": "tools/call",
          "params": {"name": tool, "arguments": args}})
    m = rd(nid[0])
    res = m.get("result", {})
    text = "\n".join(c.get("text", "") for c in res.get("content", [])
                     if c.get("type") == "text")
    return bool(res.get("isError")), text


send({"jsonrpc": "2.0", "id": 1, "method": "initialize",
      "params": {"protocolVersion": "2024-11-05", "capabilities": {},
                 "clientInfo": {"name": "census", "version": "1"}}})
rd(1)
send({"jsonrpc": "2.0", "method": "notifications/initialized"})

TABLES = ["%Dictionary.ClassDefinition", "%Dictionary.DocumentMapping",
          "%Dictionary.RoutineDefinition"]
bad = 0
for t in TABLES:
    err, txt = call("execute_sql", {
        "query": f"SELECT COUNT(*) AS C FROM {t} WHERE NAME %STARTSWITH 'LRISL' OR NAME %STARTSWITH 'RismCI'"})
    d = json.loads(txt)
    c = d["rows"][0][0] if d.get("rows") else "?"
    print("TABLE", t, "->", c)
    bad += int(c)

# globals census: sentinel control + LRISL kill, all in ONE terminal session
err, txt = call("execute_command", {"command": "set ^ZZCENW=1 zwrite ^ZZCENW"})
print("SENTINEL visible:", "1" in json.loads(txt).get("output", ""))
err, txt = call("execute_command", {"command": "zkill ^ZZCENW zkill ^LRISL zkill ^LRISLX zkill ^LRISLWX write \"done\""})
print("KILL:", json.loads(txt).get("output", ""))
err, txt = call("execute_command", {"command": "zwrite ^LRISL zwrite ^LRISLWX write $zconvert(\"\",\"U\",\"\")"})
out = json.loads(txt).get("output", "")
print("POST-KILL EMPTY:", out.strip() == "")
err, txt = call("execute_command", {"command": "zwrite ^ZZCENW"})
print("SENTINEL gone:", json.loads(txt).get("output", "").strip() == "")
p.kill()
sys.exit(1 if bad else 0)
