"""Windows twin of the L bash probe: RISM_MCP_HOST must NOT exist as an env
override; an http-door server binds loopback unless told otherwise.
Spawn `rism mcp --transport http --port 0` with RISM_MCP_HOST=0.0.0.0 set and
count stderr lines saying 'listening on http://127.0.0.1' (expect 1: the env
var is ignored and the loopback default holds)."""
import os
import re
import subprocess
import time

env = dict(os.environ)
env["RISM_MCP_HOST"] = "0.0.0.0"
p = subprocess.Popen(
    [__import__("sys").argv[1], "mcp", "--transport", "http", "--port", "0"],
    stderr=subprocess.PIPE, stdout=subprocess.DEVNULL, env=env, text=True,
    encoding="utf-8", errors="replace")
out = []
deadline = time.time() + 6
try:
    while time.time() < deadline:
        line = p.stderr.readline()
        if not line:
            break
        out.append(line)
        if "listening" in line:
            break
finally:
    p.kill()
    p.wait(timeout=5)
text = "".join(out)
print(sum(1 for ln in text.splitlines()
          if re.search(r"listening on http://127\.0\.0\.1", ln)))
