#!/usr/bin/env python3
"""Live-matrix runner: thin, portable, stdlib-only executor.

Contract: each row JSON in rows/ is one checklist row,
with `checks`. A stdio check is exactly one JSON-RPC `tools/call` against
`rism mcp` over newline-delimited stdio — the row JSONs are the contract;
this Python driver is only the Linux transport. Ports (PowerShell/HTTP)
must reproduce the same request shapes 1:1.

Checks execute in order per row; a row PASSES iff every check passes.
Failures are recorded with the check label and observed text. Results go
to results/<row-id>.json; checklist-update.py rewrites the Status
column from those files (never the row bodies).

Row JSON keys:
  id            checklist row id
  group         execution group (L:1..; lower runs first, chains in order)
  plant         [{name, content:[lines], compile:bool}] run before checks
  unplant       [names] deleted after checks (always, even on failure)
  serial        true for debugger rows (hard global lock, one in flight)
  checks        list of check objects (see below)

Check kinds by door:
  stdio: {door:"stdio", label, tool, args, env?, timeout?, save?, assert?}
  cli:   {door:"cli",  label, argv:[...], env?, unset?, timeout?, save?, assert?}
  http:  {door:"http", label, method, path, body?, headers?, save?, assert?}
  httpd: {door:"httpd_spawn", label, argv:[...], env?, ready_regex} -> saves
         {door:"httpd_kill"}
  diff:  {door:"diff", label, a, b, mode:"eq"|"keys"|"subset"} over saved values
  exec:  {door:"exec", label, argv:[...], save?, assert?}  (local host command)

assert fields (any subset):
  isError: bool                     (stdio tool result)
  exit_code: int | [int,...]        (cli/exec)
  text_regex: "re"                  (stdio result text / cli stdout / http body)
  stdout_regex, stderr_regex, stderr_no_regex: "re"
  json: { "dotted.path": cond }     cond: {regex|eq|ne|exists|in|len_gte|len_lte|
                                           set_eq|type} ("[]" suffix maps a list)
  json_raw: cond                    applied to the whole saved/parsed value
save: name -> stores parsed json (or raw text) in row context; "${name}"
placeholders in args/paths/argv/stdin get replaced (whole-string "${pid}"
keeps the saved type).
"""

from __future__ import annotations

import argparse
import json
import os
import re
import signal
import subprocess
import sys
import threading
import time
import tomllib
import urllib.error
import urllib.request
from pathlib import Path

HERE = Path(__file__).resolve().parent
LOCK = threading.Lock()


# ---------------------------------------------------------------- config ---

def load_config(path: Path) -> dict:
    with open(path, "rb") as f:
        return tomllib.load(f)


class Ctx:
    """Per-run shared state: settings, sessions, saved values."""

    def __init__(self, os_cfg: dict):
        self.os_cfg = os_cfg
        self.binary = str((HERE.parent.parent / os_cfg["binary"]).resolve()
                          if not Path(os_cfg["binary"]).is_absolute()
                          else os_cfg["binary"])
        self.settings_path = Path(
            os.path.expandvars(os_cfg["settings"])).expanduser()
        s = load_config(self.settings_path)
        # The client SKIPS the password field when parsing config (env
        # RISM_IRIS_PASSWORD is the only file-independent door), so a valid
        # connection implies the default auth. Read whatever shape the file
        # actually has for the independent HTTP oracle.
        iris = s.get("iris", s)
        self.base_url = (iris.get("base_url")
                         or iris.get("iris_base_url")
                         or s.get("iris_base_url", "http://localhost:52773")
                         ).rstrip("/")
        self.user = (iris.get("username") or iris.get("iris_username")
                     or s.get("iris_username", "_SYSTEM"))
        self.pw = (os.environ.get("RISM_IRIS_PASSWORD")
                   or os_cfg.get("default_password", "SYS"))
        self.workspace = os_cfg.get("workspace", str(HERE.parent.parent))
        self.scratch = str(Path(self.workspace).resolve() / ".live-matrix")
        Path(self.scratch).mkdir(parents=True, exist_ok=True)
        self.sessions: dict[str, "Mcp"] = {}
        self.g: dict[str, object] = {}  # cross-row saved values (debug chain)
        # env-derived knobs for rows that steer config discovery per-run
        self.g["cfgroot"] = os.environ.get(
            "RISM_LIVE_CFGROOT", str(Path(self.workspace) / "cfgroot"))
        self.httpd: subprocess.Popen | None = None
        self.httpd_url = None

    def env_for(self, check: dict, unset: list) -> dict:
        env = dict(os.environ)
        env.pop("RISM_WORKSPACE", None)
        for k in unset:
            env.pop(k, None)
        for k in check.get("unset", []):
            env.pop(k, None)
        env.update({k: self.resolve(v, {}) for k, v in check.get("env", {}).items()})
        return env

    def restart_server(self, wait_s: int = 0):
        """Crash-round restart (checklist rule): run the configured restart
        command, then poll the Atelier root until it answers 200."""
        cmd = self.os_cfg.get("restart_cmd")
        if not cmd:
            return
        wait_s = self.os_cfg.get("restart_wait_s") or wait_s or 90
        argv = self.os_cfg.get("restart_argv")
        if argv:  # Windows: argv list — no shell quoting traps
            subprocess.run(argv, check=False, capture_output=True)
        else:
            subprocess.run(cmd, shell=True, check=False, capture_output=True)
        deadline = time.time() + wait_s
        ensure = self.os_cfg.get("restart_ensure_argv")
        while time.time() < deadline:
            st, _ = http_call(self, "GET", self.base_url + "/api/atelier/",
                              timeout=5)
            if st == 200:
                time.sleep(2)
                return
            # VM fact: the httpd service can be mid-restart when the poll
            # loop starts — re-assert it's running every ~15s
            if ensure:
                subprocess.run(ensure, check=False, capture_output=True)
            time.sleep(5 if ensure else 2)
        raise RuntimeError("server never returned 200 after restart")

    def resolve(self, obj, saved: dict):
        if isinstance(obj, str):
            if "${" in obj and re.search(r"\$\{(exe|workspace|scratch|python|tools)\}", obj):
                obj = (obj.replace("${exe}", self.binary)
                          .replace("${workspace}", self.workspace)
                          .replace("${scratch}", self.scratch)
                          .replace("${python}", sys.executable)
                          .replace("${tools}", str(HERE / "tools")))
            m = re.fullmatch(r"\$\{(g:)?(\w+)\}", obj)
            if m:
                if m.group(1):
                    if m.group(2) in self.g:
                        return self.g[m.group(2)]
                elif m.group(2) in saved:
                    return saved[m.group(2)]
                elif m.group(2) in self.g:
                    return self.g[m.group(2)]
                return obj
            if re.search(r"\$\{(exe|workspace|scratch|python|tools)\}", obj):
                obj = (obj.replace("${exe}", self.binary)
                          .replace("${workspace}", self.workspace)
                          .replace("${scratch}", self.scratch)
                          .replace("${python}", sys.executable)
                          .replace("${tools}", str(HERE / "tools")))
            return re.sub(
                r"\$\{(g:)?(\w+)\}",
                lambda mm: str(self.g.get(mm.group(2), "") if mm.group(1)
                               else (saved.get(mm.group(2))
                                     if mm.group(2) in saved
                                     else self.g.get(mm.group(2), mm.group(0)))),
                obj)
        if isinstance(obj, dict):
            return {k: self.resolve(v, saved) for k, v in obj.items()}
        if isinstance(obj, list):
            return [self.resolve(v, saved) for v in obj]
        return obj


# ------------------------------------------------------------- stdio door ---

class Mcp:
    def __init__(self, binary: str, env: dict):
        self.p = subprocess.Popen(
            [binary, "mcp"], stdin=subprocess.PIPE, stdout=subprocess.PIPE,
            stderr=subprocess.PIPE, text=True, env=env,
            encoding="utf-8", errors="replace")
        self.nid = 0
        self._init()

    def _send(self, o):
        self.p.stdin.write(json.dumps(o) + "\n")
        self.p.stdin.flush()

    def _read(self, want_id, timeout):
        deadline = time.time() + timeout
        while time.time() < deadline:
            line = self.p.stdout.readline()
            if not line:
                raise RuntimeError("stdio EOF (server died?)")
            try:
                msg = json.loads(line)
            except json.JSONDecodeError:
                continue
            if msg.get("id") == want_id:
                return msg
        raise RuntimeError(f"timeout waiting for id={want_id}")

    def _init(self):
        self.nid += 1
        self._send({"jsonrpc": "2.0", "id": self.nid, "method": "initialize",
                    "params": {"protocolVersion": "2024-11-05", "capabilities": {},
                               "clientInfo": {"name": "live_matrix", "version": "1"}}})
        self._read(self.nid, 30)
        self._send({"jsonrpc": "2.0", "method": "notifications/initialized"})

    def call(self, tool, args, timeout=120):
        self.nid += 1
        rid = self.nid
        self._send({"jsonrpc": "2.0", "id": rid, "method": "tools/call",
                    "params": {"name": tool, "arguments": args}})
        try:
            msg = self._read(rid, timeout)
        except RuntimeError as e:
            # terminal sessions die when IRIS restarts (mid-run slot
            # recovery): reconnect once, resend the exact same request.
            if "EOF" not in str(e):
                raise
            self._init()
            self.nid += 1
            rid = self.nid
            self._send({"jsonrpc": "2.0", "id": rid, "method": "tools/call",
                        "params": {"name": tool, "arguments": args}})
            msg = self._read(rid, timeout)
        if "error" in msg:
            # JSON-RPC-level error (schema rejection etc.): surface as tool error
            return True, json.dumps(msg["error"])
        res = msg.get("result", {})
        text = "\n".join(c.get("text", "") for c in res.get("content", [])
                         if c.get("type") == "text")
        return bool(res.get("isError")), text

    def kill(self):
        try:
            self.p.kill()
            self.p.wait(timeout=5)
        except Exception:
            pass


def get_session(ctx: Ctx, check: dict, scope: str = "") -> Mcp:
    env = ctx.env_for(check, [])
    key = (scope or "") + "|" + json.dumps(
        {k: v for k, v in env.items() if k.startswith("RISM_")
         or k in ("XDG_CONFIG_HOME", "APPDATA")}, sort_keys=True)
    if key not in ctx.sessions:
        ctx.sessions[key] = Mcp(ctx.binary, env)
    return ctx.sessions[key]


# ------------------------------------------------------------- raw http -----

def http_call(ctx: Ctx, method: str, url: str, body=None, headers=None,
              timeout=60, auth=True):
    data = None
    hd = dict(headers or {})
    if body is not None:
        data = json.dumps(body).encode()
        hd.setdefault("Content-Type", "application/json")
    req = urllib.request.Request(url, data=data, method=method, headers=hd)
    if auth:
        import base64
        tok = base64.b64encode(f"{ctx.user}:{ctx.pw}".encode()).decode()
        req.add_header("Authorization", "Basic " + tok)
    try:
        with urllib.request.urlopen(req, timeout=timeout) as r:
            return r.status, r.read().decode("utf-8", "replace")
    except urllib.error.HTTPError as e:
        return e.code, e.read().decode("utf-8", "replace")
    except Exception as e:  # connection-level error is an observable too
        return -1, f"CONNECT_ERROR: {e}"


# ------------------------------------------------------------ asserts -------

def get_path(obj, dotted: str):
    cur = obj
    for part in dotted.split("."):
        if part.endswith("[]"):
            key, mapping = part[:-2], True
            cur = cur.get(key) if isinstance(cur, dict) else None
            if isinstance(cur, list):
                return [ (x[list(x)[0]] if isinstance(x, dict) and len(x) == 1 else x)
                         if mapping else x for x in cur], True
            return None, False
        if part == "":
            continue
        if isinstance(cur, dict):
            cur = cur.get(part)
        elif isinstance(cur, list) and part.isdigit():
            cur = cur[int(part)] if int(part) < len(cur) else None
        else:
            return None, False
    return cur, True


def cond_ok(val, cond):
    if "exists" in cond:
        return (val is not None) == cond["exists"]
    if "eq" in cond:
        return val == cond["eq"]
    if "ne" in cond:
        return val != cond["ne"]
    if "in" in cond:
        return val in cond["in"]
    if "regex" in cond:
        return val is not None and re.search(cond["regex"], str(val)) is not None
    if "len_gte" in cond:
        return val is not None and len(val) >= cond["len_gte"]
    if "len_eq" in cond:
        return val is not None and len(val) == cond["len_eq"]
    if "len_lte" in cond:
        return val is not None and len(val) <= cond["len_lte"]
    if "gte" in cond:
        return isinstance(val, (int, float)) and val >= cond["gte"]
    if "lte" in cond:
        return isinstance(val, (int, float)) and val <= cond["lte"]
    if "sorted_desc" in cond:
        vs = [get_path(x, cond["sorted_desc"])[0] for x in (val or [])]
        return all(vs[i] >= vs[i + 1] for i in range(len(vs) - 1))
    if "set_eq" in cond:
        try:
            return set(map(str, val)) == set(map(str, cond["set_eq"]))
        except TypeError:
            return False
    if "type" in cond:
        t = {"list": list, "dict": dict, "str": str, "int": int,
             "float": float, "bool": bool}[cond["type"]]
        return isinstance(val, t) and not (t is int and isinstance(val, bool))
    return True


def check_asserts(ctx: Ctx, a: dict, saved: dict, *, text=None, parsed=None,
                  status=None, exit_code=None, stdout=None, stderr=None):
    fails = []

    def note(m):
        fails.append(m)

    if "isError" in a and a["isError"] is not None:
        if bool(text_err_flag[0]) != a["isError"]:
            note(f"expected isError={a['isError']} got={bool(text_err_flag[0])}: "
                 f"{(text or '')[:240]}")
    if "text_regex" in a:
        if not re.search(a["text_regex"], text or ""):
            note(f"text_regex {a['text_regex']!r} not found in {(text or '')[:240]}")
    if "status" in a and status is not None:
        want = a["status"]
        want_l = want if isinstance(want, list) else [want]
        if status not in want_l:
            note(f"HTTP status {status} not in {want_l}: {(text or '')[:200]}")
    if "exit_code" in a and exit_code is not None:
        want = a["exit_code"]
        want_l = want if isinstance(want, list) else [want]
        if exit_code not in want_l:
            note(f"exit code {exit_code} not in {want_l}")
    if "stdout_regex" in a:
        if not re.search(a["stdout_regex"], stdout or ""):
            note(f"stdout_regex {a['stdout_regex']!r} missing in {(stdout or '')[:240]}")
    if "stderr_regex" in a:
        if not re.search(a["stderr_regex"], stderr or ""):
            note(f"stderr_regex {a['stderr_regex']!r} missing in {(stderr or '')[:240]}")
    if "stderr_no_regex" in a:
        if re.search(a["stderr_no_regex"], (stderr or "") + (stdout or "")):
            note(f"stderr_no_regex {a['stderr_no_regex']!r} MUST NOT appear")
    if "json" in a and parsed is None:
        fails.append(f"assert.json requested but result is not JSON: {(text or '')[:220]}")
    if "json" in a and parsed is not None:
        for p, cond in a["json"].items():
            if isinstance(cond, dict):
                cond = ctx.resolve(cond, saved)
            val, ok = get_path(parsed, p)
            if not ok or not cond_ok(val, cond):
                note(f"json {p}: {cond} -> {json.dumps(val)[:200] if ok else 'MISSING'}")
    if "json_raw" in a and parsed is not None:
        cond = ctx.resolve(a["json_raw"], saved)
        if not cond_ok(parsed, cond):
            note(f"json_raw {cond} -> {json.dumps(parsed)[:200]}")
    return fails


text_err_flag = [False]  # set per stdio check before asserts run


# -------------------------------------------------------------- plumbing ----

def run_plant(ctx, sess, plant):
    for p in plant:
        err, text = sess.call("put_document",
                              {"name": p["name"], "content": p["content"],
                               "ignore_conflict": True})
        if err:
            raise RuntimeError(f"plant put {p['name']} failed: {text[:200]}")
        if p.get("compile"):
            err, text = sess.call("compile_documents", {"names": [p["name"]]})
            if err:
                raise RuntimeError(f"plant compile {p['name']} failed: {text[:200]}")


def run_unplant(ctx, sess, names):
    for n in names:
        try:
            sess.call("delete_document", {"name": n})
        except Exception:
            pass


# per-row stdio session scope (fresh process per row unless shared via scope tag)
row_scope = [None]


def _flat(obj):
    if isinstance(obj, dict):
        for v in obj.values():
            yield from _flat(v)
    elif isinstance(obj, list):
        for v in obj:
            yield from _flat(v)
    else:
        yield obj


def execute_check(ctx: Ctx, ch: dict, saved: dict):
    door = ch["door"]
    label = ch.get("label", door)
    if door == "restart":
        ctx.restart_server(ch.get("wait_s", 90))
        return label, [], None, "restarted"
    if door == "stdio":
        scope = row_scope[0] + (ch.get("scope_suffix") or "")
        sess = get_session(ctx, ch, scope)
        args = ctx.resolve(ch.get("args", {}), saved)
        if any(isinstance(v, str) and "${" in v for v in _flat(args)):
            return (label, [f"unresolved placeholder in args: {json.dumps(args)[:120]}"
                            " — a chain row ran out of order"], None, "")
        err, text = sess.call(ch["tool"], args, ch.get("timeout", 120))
        text_err_flag[0] = err
        parsed = None
        try:
            parsed = json.loads(text)
        except Exception:
            pass
        if ch.get("save"):
            saved[ch["save"]] = parsed if parsed is not None else text
        if ch.get("extract") and parsed is not None:
            for gname, pth in ch["extract"].items():
                val, _ = get_path(parsed, pth)
                ctx.g[gname] = val
                saved[gname] = val
        if ch.get("save_g"):
            v = parsed if parsed is not None else text
            ctx.g[ch["save_g"]] = v
            saved[ch["save_g"]] = v
        return label, check_asserts(ctx, ch.get("assert", {}), saved,
                                    text=text, parsed=parsed), parsed, text
    if door == "cli":
        argv = [ctx.binary] + ctx.resolve(ch.get("argv", []), saved)
        env = ctx.env_for(ch, [])
        stdin = ch.get("stdin")
        p = subprocess.run(argv, capture_output=True, text=True,
                           encoding="utf-8", errors="replace",
                           env=env, timeout=ch.get("timeout", 90),
                           input=stdin, cwd=ch.get("cwd", ctx.workspace))
        parsed = None
        try:
            parsed = json.loads(p.stdout)
        except Exception:
            pass
        if ch.get("save"):
            saved[ch["save"]] = parsed if parsed is not None else p.stdout
        return label, check_asserts(ctx, ch.get("assert", {}), saved,
                                    text=p.stdout, parsed=parsed,
                                    exit_code=p.returncode, stdout=p.stdout,
                                    stderr=p.stderr), parsed, p.stdout + "\n--stderr--\n" + p.stderr
    if door == "exec":
        argv = ctx.resolve(ch.get("argv", []), saved)
        p = subprocess.run(argv, capture_output=True, text=True,
                           encoding="utf-8", errors="replace",
                           timeout=ch.get("timeout", 60))
        parsed = None
        try:
            parsed = json.loads(p.stdout)
        except Exception:
            pass
        if ch.get("save"):
            saved[ch["save"]] = parsed if parsed is not None else p.stdout
        return label, check_asserts(ctx, ch.get("assert", {}), saved,
                                    text=p.stdout, parsed=parsed,
                                    exit_code=p.returncode, stdout=p.stdout,
                                    stderr=p.stderr), parsed, p.stdout
    if door == "http":
        path = ctx.resolve(ch["path"], saved)
        url = ctx.resolve(ch.get("base", ctx.base_url), saved) + path
        st, body = http_call(ctx, ch.get("method", "GET"), url,
                            ctx.resolve(ch.get("body"), saved),
                            ctx.resolve(ch.get("headers"), saved),
                            ch.get("timeout", 60), ch.get("auth", True))
        parsed = None
        try:
            parsed = json.loads(body)
        except Exception:
            pass
        if ch.get("save"):
            saved[ch["save"]] = parsed if parsed is not None else body
        return label, check_asserts(ctx, ch.get("assert", {}), saved,
                                    text=body, parsed=parsed, status=st), parsed, body
    if door == "httpd_spawn":
        argv = [ctx.binary] + ch.get("argv", [])
        env = ctx.env_for(ch, [])
        proc = subprocess.Popen(argv, stdout=subprocess.PIPE,
                                stderr=subprocess.PIPE, text=True, env=env)
        deadline = time.time() + ch.get("ready_timeout", 20)
        ready = None
        buf_err = []
        while time.time() < deadline:
            line = proc.stderr.readline()
            if not line:
                if proc.poll() is not None:
                    break
                continue
            buf_err.append(line)
            if re.search(ch.get("ready_regex", r"ready"), line):
                ready = line
                break
        if ready:
            m = re.search(r"http://([\d.]+|localhost):(\d+)", ready)
            if m:
                host, port = m.group(1), m.group(2)
                # 0.0.0.0 is a LISTEN wildcard, not a connect destination:
                # Linux maps such a connect to loopback, Windows refuses it.
                # Probe the same socket via loopback on both OSes.
                probe_host = "127.0.0.1" if host == "0.0.0.0" else host
                proc._mcp_url = f"http://{probe_host}:{port}"
                saved["httpd_url"] = proc._mcp_url
                saved["door_url"] = proc._mcp_url
                ctx.g["door_url"] = proc._mcp_url
            ctx.httpd = proc
            a = ch.get("assert", {})
            fails = []
            if "stderr_regex" in a and not re.search(
                    a["stderr_regex"], ready + "".join(buf_err)):
                fails.append(f"ready/stderr_regex {a['stderr_regex']!r} missing")
            exit_ok = a.get("exit_code")
            if exit_ok is not None:
                # expect refused-at-startup (process exits with error)
                try:
                    rc = proc.wait(timeout=10)
                except subprocess.TimeoutExpired:
                    proc.kill()
                    rc = 0
                if rc not in (exit_ok if isinstance(exit_ok, list) else [exit_ok]):
                    fails.append(f"spawn exit {rc} not in {exit_ok}: "
                                 f"{''.join(buf_err)[:200]}")
                ctx.httpd = None
            if exit_ok is None and not fails:
                # confirm the socket really binds where the ready line says
                url = saved.get("httpd_url")
                if not url:
                    fails.append("ready line has no http://host:port")
                else:
                    st, _body = http_call(ctx, "GET", url + "/mcp", None,
                                          {"Accept": "application/json, text/event-stream"},
                                          10, auth=False)
                    saved["door_url"] = url
                    if st == -1:
                        fails.append(f"ready line claims {url} but connect failed")
            return ch.get("label", "httpd_spawn"), fails, None, \
                ready or "".join(buf_err)
        rc = proc.poll()
        buf = "".join(buf_err)
        if rc is None:
            proc.kill()
        a = ch.get("assert", {})
        fails = []
        if "stderr_regex" in a and not re.search(a["stderr_regex"], buf):
            fails.append(f"stderr_regex {a['stderr_regex']!r} missing in {buf[:240]}")
        exit_ok = a.get("exit_code")
        if exit_ok is not None:
            try:
                rc = proc.wait(timeout=10)
            except Exception:
                rc = -9
            if rc not in (exit_ok if isinstance(exit_ok, list) else [exit_ok]):
                fails.append(f"spawn exit {rc} not in {exit_ok}: {buf[:240]}")
        return ch.get("label", "httpd_spawn"), fails, None, buf
    if door == "httpd_kill":
        if ctx.httpd:
            ctx.httpd.kill()
            ctx.httpd.wait(timeout=5)
            ctx.httpd = None
        return label, [], None, ""
    if door == "diff":
        a, b = saved.get(ch["a"]), saved.get(ch["b"])
        mode = ch.get("mode", "eq")
        fails = []
        if a is None or b is None:
            fails.append(f"diff missing save: {ch['a']}={a is not None} {ch['b']}={b is not None}")
        elif mode == "eq" and a != b:
            fails.append(f"diff {ch['a']} != {ch['b']}: {json.dumps(a)[:160]} vs {json.dumps(b)[:160]}")
        elif mode == "keys":
            ka = set(a.keys()) if isinstance(a, dict) else set()
            kb = set(b.keys()) if isinstance(b, dict) else set()
            only = ka ^ kb
            if only:
                fails.append(f"key sets differ: {sorted(only)}")
        elif mode == "subset":
            def leaves(o, pre=""):
                if isinstance(o, dict):
                    out = set()
                    for k, v in o.items():
                        out |= leaves(v, f"{pre}.{k}")
                    return out
                if isinstance(o, list):
                    s = set()
                    for x in o[:50]:
                        s |= leaves(x, pre + "[]")
                    return s
                return {pre}
            la, lb = leaves(a), leaves(b)
            if la - lb:
                fails.append(f"{ch['a']} keys not in {ch['b']}: {sorted(la - lb)[:12]}")
        return label, fails, None, ""
    if door == "hash_eq":
        import hashlib

        def sha_of(lines, join):
            return hashlib.sha256(join.join(lines).encode()).hexdigest()
        join = ch.get("join", "\n")
        a_lines = ctx.resolve(ch["a"], saved)
        b_val = saved.get(ch["b"])
        b_lines = get_path(b_val, ch.get("path", "content"))[0] if isinstance(b_val, dict) else b_val
        fails = []
        if not isinstance(a_lines, list) or not isinstance(b_lines, list):
            fails.append("hash_eq needs two line lists")
        elif sha_of(a_lines, join) != sha_of(b_lines, join):
            fails.append(f"sha mismatch: put={a_lines[:2]}… get={b_lines[:2]}…")
        return label, fails, None, ""
    raise RuntimeError(f"unknown door {door}")


# ------------------------------------------------------------------ rows ----

def run_row(ctx: Ctx, row: dict):
    t0 = time.time()
    saved: dict = {}
    failures, notes = [], []
    sess = None
    row_scope[0] = row.get("scope", row["id"])
    try:
        if row.get("plant"):
            sess = get_session(ctx, {"env": row.get("env", {})}, row_scope[0])
            run_plant(ctx, sess, row["plant"])
        if row.get("serial"):
            LOCK.acquire()
        try:
            for ch in row["checks"]:
                label, fails, _, obs = execute_check(ctx, ch, saved)
                for f in fails:
                    failures.append(f"[{label}] {f}")
                if ch.get("probe_note") and not fails:
                    notes.append(f"{label}: {ch['probe_note']}")
        finally:
            if row.get("serial"):
                try:
                    LOCK.release()
                except RuntimeError:
                    pass
    except Exception as e:  # harness-level error in this row
        failures.append(f"HARNESS: {e!r}")
    finally:
        if row.get("unplant"):
            try:
                s = sess or get_session(ctx, {"env": row.get("env", {})}, row_scope[0])
                run_unplant(ctx, s, row["unplant"])
            except Exception as e:
                notes.append(f"unplant-failed: {e!r}")
        # a per-row stdio session ends with the row; a shared-scope session
        # (debug chain) lives until run end
        pref = row["id"] + "|"
        for k in [k for k in ctx.sessions if k.startswith(pref)]:
            ctx.sessions.pop(k).kill()
    return {"id": row["id"], "group": row.get("group", "?"),
            "status": "FAIL" if failures else "PASS",
            "failures": failures, "notes": notes,
            "seconds": round(time.time() - t0, 1)}


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--os-config", default=str(HERE / "os_windows.json"))
    ap.add_argument("--rows", default=str(HERE / "rows"))
    ap.add_argument("--out", default=str(HERE / "results"))
    ap.add_argument("--binary", default=None,
                    help="override the OS-config binary path")
    ap.add_argument("--ids", default=None,
                    help="comma list or file of row ids (default: all L rows)")
    args = ap.parse_args()

    os_cfg = json.loads(Path(args.os_config).read_text())
    if getattr(args, "binary", None):
        os_cfg["binary"] = args.binary
    ctx = Ctx(os_cfg)
    rows_dir = Path(args.rows)
    out_dir = Path(args.out)
    out_dir.mkdir(parents=True, exist_ok=True)

    ids = None
    if args.ids:
        p = Path(args.ids)
        ids = set(p.read_text().split()) if p.exists() else set(args.ids.split(","))

    files = sorted(rows_dir.glob("*.json"))
    selected = []
    for f in files:
        row = json.loads(f.read_text())
        if ids and row["id"] not in ids:
            continue
        selected.append((row["id"], row))
     # chain-safe order: group, then explicit row order (debug chain: start
    # first, attach LAST), then id
    selected.sort(key=lambda t: (t[1].get("group", ""), t[1].get("order", 100), t[0]))

    summary = []
    for rid, row in selected:
        res = run_row(ctx, row)
        (out_dir / f"{rid}.json").write_text(json.dumps(res, indent=1))
        print(f"{res['status']:4s} {rid}  ({res['seconds']}s)"
              + ("  " + "; ".join(res["failures"])[:180] if res["failures"] else ""))
        summary.append(res)

    for s in ctx.sessions.values():
        s.kill()
    if ctx.httpd:
        ctx.httpd.kill()
    n_ok = sum(1 for s in summary if s["status"] == "PASS")
    print(f"== {n_ok}/{len(summary)} rows pass ==")
    # exit 0 always: results files are the book of record, exit signals nothing per-row


if __name__ == "__main__":
    main()
