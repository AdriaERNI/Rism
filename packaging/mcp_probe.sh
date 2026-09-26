#!/usr/bin/env bash
# MCP-door probe for the PACKAGED binary: stdio JSON-RPC handshake + one real
# tool call. Zero deps beyond bash/coreutils and `rism` on PATH (works on
# every distro image — no python assumed). Connection comes from RISM_IRIS_*
# env vars; exits non-zero on any contract violation.
set -euo pipefail

tmp=$(mktemp)
trap 'rm -f "$tmp"' EXIT

{
  printf '%s\n' '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2024-11-05","clientInfo":{"name":"ci","version":"0"},"capabilities":{}}}'
  printf '%s\n' '{"jsonrpc":"2.0","method":"notifications/initialized"}'
  printf '%s\n' '{"jsonrpc":"2.0","id":2,"method":"tools/list"}'
  printf '%s\n' '{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"execute_sql","arguments":{"query":"select 1 as ok"}}}'
} | timeout 60 rism mcp 2>/dev/null > "$tmp" || { echo "MCP probe: rism mcp failed"; exit 1; }

# tools/list answer must list exactly 25 tools. inputSchema is the exact
# marker (one per tool); "name" appears 33x on the wire (extra mentions in
# descriptions/schemas — never count those).
tools_line=$(grep -E '"id"[[:space:]]*:[[:space:]]*2' "$tmp" | head -1)
n=$(printf '%s' "$tools_line" | grep -o '"inputSchema"' | wc -l)
if [ "${n:-0}" -ne 25 ]; then
  echo "MCP probe: expected 25 tools, counted $n"
  exit 1
fi

# execute_sql answer must be a successful result carrying the ok column.
# isError is the authoritative flag — a substring like "ok" in an error
# message ("could not open...") must never pass this gate.
if ! grep -E '"id"[[:space:]]*:[[:space:]]*3' "$tmp" | grep -q '"isError":false'; then
  echo "MCP probe: execute_sql did not answer cleanly"
  exit 1
fi

echo "MCP probe OK: $n tools, SQL answered"
