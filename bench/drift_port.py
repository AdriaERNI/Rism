#!/usr/bin/env python3
"""Python port of installer/static/analyze-docs-drift.ps1 (the Windows CI
docs-drift gate) so this lane can run the SAME logic without pwsh. Checks
1:1: (1) every *.md tool-count claim == code count; (2) every tool name
appears in docs/mcp-tools.md; (3) config-path promises; (4) semver.
Exit 0 = 0 drift failures."""
import re
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
fail = 0
# the PS gate runs on a CI checkout, where swarmforge/ (gitignored) and
# target/ do not exist — replicate that scope with git ls-files
tracked = set(
    subprocess.run(["git", "-C", str(ROOT), "ls-files"], capture_output=True, text=True).stdout.split()
)


def ok(msg):
    print("ok:  ", msg)


def bad(msg):
    global fail
    print("FAIL:", msg)
    fail = 1


mcp = (ROOT / "src/mcp/mod.rs").read_text()
attr_idx = [m.start() for m in re.finditer(r"#\[tool\(", mcp)]
fn_idx = {}
for m in re.finditer(r"async fn (\w+)\s*\(", mcp):
    if m.group(1) not in ("new", "list_tools", "call_tool", "get_info"):
        fn_idx[m.start()] = m.group(1)
tools = []
for i in attr_idx:
    k = min((j for j in fn_idx if j > i), default=None)
    if k is not None:
        tools.append(fn_idx[k])
tools = sorted(set(tools))
if len(tools) != len(attr_idx):
    bad(f"attr/fn pairing lost names: {len(tools)} vs {len(attr_idx)} attrs")
n = len(tools)
ok(f"#[tool] definitions found: {n}")

# 1. md claims ("serves N tools", "N tools", "**N tools**") but not
#    "(9 tools)" section counts — the PS regex excludes a paren-preceded num.
claims = []
for f in ROOT.rglob("*.md"):
    rel = str(f.relative_to(ROOT))
    if rel not in tracked:  # matches the CI checkout (untracked files absent)
        continue
    t = f.read_text(errors="replace")
    for m in re.finditer(r"(?<!\()\b(\d+)\s+tools\b", t):
        claims.append((f.relative_to(ROOT), int(m.group(1))))
if not claims:
    bad("no tool-count claim found in docs")
for file, c in claims:
    (ok if c == n else bad)(f"{file}: claims {c}" + ("" if c == n else f", code defines {n}"))

# 2. every tool name mentioned in docs/mcp-tools.md
doc = (ROOT / "docs/mcp-tools.md").read_text()
for name in tools:
    short = re.sub(r"^tool_", "", name)
    (ok if f"`{short}`" in doc else bad)(f"{short} documented" if f"`{short}`" in doc else f"docs/mcp-tools.md never mentions {name}")

# 3. config path promises
settings = (ROOT / "src/settings.rs").read_text()
if re.search(r"BaseDirs::new\(\)[\s\S]{0,120}config_dir\(\)\.join\(\"rism\"\)", settings):
    cfg = (ROOT / "docs/configuration.md").read_text()
    (ok if "%APPDATA%" in cfg else bad)("Windows config path documented (%APPDATA%)" if "%APPDATA%" in cfg else "configuration.md lost the %APPDATA% row")
    (ok if ".config/rism/config.toml" in cfg else bad)("Linux config path documented" if ".config/rism/config.toml" in cfg else "configuration.md lost the Linux path")
    (ok if re.search(r"#\[serde\(skip\)\][\s\S]{0,80}pub iris_password", settings) else bad)(
        "password is serde-skipped, as docs promise")
else:
    bad("settings.rs config path scheme changed — recheck docs/configuration.md")

# 4. semver
ver = re.search(r'^version\s*=\s*"([^"]+)"', (ROOT / "Cargo.toml").read_text(), re.M).group(1)
(ok if re.match(r"^\d+\.\d+\.\d+$", ver) else bad)(f"Cargo.toml version {ver}")

print(f"\nDRIFT FAILURES: {fail}")
sys.exit(fail)
