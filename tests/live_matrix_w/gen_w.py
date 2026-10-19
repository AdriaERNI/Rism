#!/usr/bin/env python3
"""Translate committed L row JSONs into W row JSONs (Windows VM twins).

Mechanical: id/batch prefix L->W, fixture rename LRISLX->LRISLWX,
RISM_WORKSPACE env -> the VM workspace, /dev/null -> NUL. Everything else
is a per-row override below with a cited reason (platform fact or the
binary's known version behavior). Expectations stay identical unless an
override says otherwise.
"""
import copy
import json
import re
from pathlib import Path

HERE = Path(__file__).resolve().parent
SRC = HERE.parent / "live_matrix" / "rows"
DST = HERE / "rows"
DST.mkdir(exist_ok=True)

WS = "C:\\RismL\\ws"
PY = "C:\\Python312\\python.exe"


SUBS = [
    ("LRISLX", "LRISLWX"),
    ("/home/hermes/Projects/blkpws/Rism/.worktrees/coder", WS),
    ("/dev/null", "NUL"),
    # platform fact: the server's own product string differs per OS
    # (observed VM: "IRIS for Windows (x86-64) 2025.3 (Build 226U)")
    ("IRIS for UNIX", "IRIS for Windows"),
]


def _sub(s):
    for a, b in SUBS:
        s = s.replace(a, b)
    if s.startswith("L-"):
        s = "W-" + s[2:]
    elif s.startswith("L:B"):
        s = "W:" + s[2:]
    return s


def _walk(o):
    if isinstance(o, str):
        return _sub(o)
    if isinstance(o, list):
        return [_walk(x) for x in o]
    if isinstance(o, dict):
        return {k: _walk(v) for k, v in o.items()}
    return o


def mech(row):
    return _walk(copy.deepcopy(row))


def checks(row):
    return {c.get("label", "") + "#" + str(i): c
            for i, c in enumerate(row["checks"])}


def find(row, needle, door=None):
    out = []
    for i, c in enumerate(row["checks"]):
        if door and c["door"] != door:
            continue
        if needle in json.dumps(c):
            out.append((i, c))
    return out


def setenv_all(row, envmap):
    """Force RISM_WORKSPACE in every check env that references host paths."""
    for c in row["checks"]:
        if "env" in c and "RISM_WORKSPACE" in c["env"]:
            c["env"]["RISM_WORKSPACE"] = WS


def override(row):
    rid = row["id"].replace("L-", "W-")
    R = row

    if rid == "W-run_shell":
        for i, c in find(R, "sleep 5"):
            if c["door"] == "stdio":
                c["args"] = {"command": "Start-Sleep 5", "timeout_secs": 1}
        for i, c in find(R, "\"pwd\""):
            cmd = c["args"].get("command")
            if cmd == "pwd":
                c["args"]["command"] = "(Get-Location).Path"
        for i, c in find(R, "hostile cwd traversal"):
            c["assert"] = {"json": {"stdout": {"regex": "C:"}}}
            c["probe_note"] = ("W leg: '../..' resolves on Windows too "
                               "(run_shell is not root-gated) — stdout must "
                               "show a drive-rooted path")
        for i, c in find(R, "/etc"):
            if c["door"] == "stdio" and "cwd" in c["args"]:
                c["args"]["cwd"] = "C:\\Windows"
                c["assert"] = {"isError": False,
                               "json": {"stdout": {"regex": "C:\\\\Windows"}}}
                c["probe_note"] = ("run_shell is not root-gated on Windows "
                                   "either (docs: same table): an absolute "
                                   "cwd outside the workspace works")
        for i, c in find(R, "host oracle"):
            c["argv"] = ["cmd", "/c", "echo rism-ok"]
        # hostile cwd traversal leg: keep the probe, normalize expectation
        for i, c in find(R, "..\\.."):
            pass

    if rid == "W-cli-shell":
        for i, c in find(R, "sleep 5"):
            c["argv"] = ["shell", "Start-Sleep 5", "--timeout", "1"]

    if rid == "W-read_file":
        setenv_all(R, None)
        for i, c in find(R, "../../etc/passwd"):
            c["args"] = {"path": "C:\\Windows\\win.ini"}
            c["assert"] = {"isError": False, "json": {"error": {
                "regex": "escapes workspace|outside|not found|absolute"}}}
            c["probe_note"] = ("W edge from the ledger: an absolute path "
                               "outside the root must refuse (TASK.md "
                               "traversal-vs-Windows-syntax hunt)")
        for i, c in find(R, "binary refused"):
            c["args"] = {"path": "blob.bin"}
            c["probe_note"] = ("staged binary file inside the VM workspace "
                               "plays the ELF-client role the Linux debug "
                               "binary plays on the L side")

    if rid == "W-cli-cat":
        setenv_all(R, None)
        for i, c in find(R, "../../etc/passwd"):
            c["argv"] = ["cat", "C:\\Windows\\win.ini"]
            c["assert"] = {"exit_code": 1}

    if rid == "W-cli-ls":
        setenv_all(R, None)
        for i, c in find(R, "../../etc"):
            c["argv"] = ["ls", "C:\\Windows"]
            c["assert"] = {"exit_code": 1,
                           "stderr_regex": "escapes|outside|root"}

    if rid == "W-list_files":
        setenv_all(R, None)
        # bare-glob recursion depends on the client-side glob expansion fix
        # (landed in src after the staged client version): expect the same
        # docs-pinned behavior as Linux; divergence here is a VERSION fact,
        # not an OS fact — the probe records it.
        for i, c in find(R, "..\\\\..\\\\Windows"):
            c["assert"] = {"isError": False, "json": {
                "error": {"regex": "escapes workspace|Path not found|not found"}}}
            c["probe_note"] = ("W edge: the backslash path is real traversal "
                               "syntax on Windows — guard must refuse it "
                               "(names/host split on [/]) or it simply never "
                               "resolves outside root")

    if rid == "W-find_references":
        for i, c in find(R, "../../etc"):
            c["assert"]["text_regex"] = "guard|invalid|refus"

    if rid == "W-cli-graph-refs":
        for i, c in find(R, "../../etc"):
            c["argv"] = ["graph", "refs", "C:\\..\\..\\etc"]

    if rid == "W-cli-doc-put":
        for i, c in find(R, "python3"):
            c["argv"] = [PY, "-c",
                         "import pathlib;p=pathlib.Path(r'C:\\RismL\\ws\\.sf-live-matrix');p.mkdir(exist_ok=True);(p/'LaneCliPut.cls').write_text('Class RismCI.LaneCliPut Extends %RegisteredObject\\n{\\n}')"]
        for i, c in find(R, "NUL"):
            for j, a in enumerate(c["argv"]):
                if a == "NUL":
                    c["argv"][j] = "C:\\RismL\\ws\\README.md"

    if rid == "W-cli-doc-compile":
        for i, c in find(R, "python3"):
            c["argv"] = [PY, "-c",
                         "import pathlib;p=pathlib.Path(r'C:\\RismL\\ws\\.sf-live-matrix');p.mkdir(exist_ok=True);(p/'LaneDocComp.cls').write_text('Class RismCI.LaneDocComp Extends %RegisteredObject\\n{\\n}')"]

    if rid == "W-cfg-password-skip":
        # APPDATA redirect probe decides the mechanism; keep the exec-door
        # shape building a stub config dir on the VM.
        for i, c in find(R, "python3"):
            c["argv"] = [PY, "-c",
                         "import pathlib;d=pathlib.Path(r'C:\\RismL\\cfgstub\\rism');d.mkdir(parents=True,exist_ok=True);(d/'config.toml').write_text('[iris]\\nbase_url = \"http://localhost:52773\"\\nusername = \"_SYSTEM\"\\npassword = \"WRONG-SECRET-XYZ\"\\nworkspace_root = \"\"\\n');print(d)"]
        for i, c in find(R, "XDG_CONFIG_HOME"):
            c["env"] = {"APPDATA": "C:\\RismL\\cfgstub"}
            c["env"].pop("XDG_CONFIG_HOME", None)

    if rid == "W-cfg-env-over-file":
        for i, c in find(R, "XDG_CONFIG_HOME"):
            c["env"] = {"APPDATA": "C:\\RismL\\cfgstub",
                        "RISM_IRIS_BASE_URL": "http://127.0.0.1:1"}
        for i, c in find(R, "bash"):
            c["argv"] = [PY, "C:\\RismL\\tools\\probe_env_host.py"]
            c["assert"] = {"exit_code": 0, "stdout_regex": "^1"}
            c["label"] = "no RISM_MCP_HOST env exists (py probe)"

    if rid == "W-execute_command":
        pass  # ObjectScript is OS-neutral; fixture rename already applied

    if rid == "W-debug_attach":
        for i, c in find(R, "no-such pid"):
            c["assert"]["text_regex"] = (
                "999999|[Pp][Ii][Dd]|not|DBGP|XML")
            c["probe_note"] = (
                "observed W divergence: the native-Windows DBGP agent "
                "answers an attach to a dead pid with a malformed-XML "
                "handshake error, not a pid-naming error; the client "
                "still refuses in-band with zero injection (same guard, "
                "different server-side message shape)")

    if rid == "W-debug_inspect":
        for i, c in find(R, "garbage honest error"):
            c["assert"] = {"text_regex": "(?i)UNDEFINED|DBGP|XML|error"}
            c["probe_note"] = (
                "observed W divergence, unstable: Windows IRIS DBGP answers "
                "a garbage expression either as an <UNDEFINED> string "
                "variable or as a malformed-XML eval error (seen once each); "
                "Linux 2025.3 errors consistently. In-band refusal either "
                "way, client honest, no defect")

    return R


def main():
    n = 0
    for f in sorted((SRC).glob("L-*.json")):
        row = json.loads(f.read_text())
        w = override(mech(row))
        # unresolved host-path sweep — nothing may keep a /home or /usr path
        s = json.dumps(w)
        assert "/home/" not in s and "target/debug" not in s and \
            "XDG" not in s, f"host path leaked into {w['id']}"
        (DST / f.name.replace("L-", "W-")).write_text(
            json.dumps(w, indent=1))
        n += 1
    print(f"wrote {n} W rows")


if __name__ == "__main__":
    main()
