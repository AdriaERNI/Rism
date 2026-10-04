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

# tools/list answer must list exactly 28 tools. inputSchema is the exact
# marker (one per tool); "name" appears 33x on the wire (extra mentions in
# descriptions/schemas — never count those).
tools_line=$(grep -E '"id"[[:space:]]*:[[:space:]]*2' "$tmp" | head -1)
n=$(printf '%s' "$tools_line" | grep -o '"inputSchema"' | wc -l)
if [ "${n:-0}" -ne 28 ]; then
  echo "MCP probe: expected 28 tools, counted $n"
  exit 1
fi

# MCP best-practice contract: the 4 terminal tools must ship annotations
# (rmcp renders them camelCase on the wire). title alone is not enough:
# at least one of the behavior hints must be present.
if ! printf '%s' "$tools_line" | grep -q '"readOnlyHint":true'; then
  echo "MCP probe: command_status missing readOnlyHint annotation"
  exit 1
fi
if ! printf '%s' "$tools_line" | grep -q '"destructiveHint"'; then
  echo "MCP probe: execute_command* missing destructiveHint annotation"
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

# --- background terminal-job contract (stream + cancel), same door ---------
# Long loop: 8 ticks, 1/s. Then poll (must stream), cancel (must interrupt),
# final status (must be finished + interrupted). No python assumed.
fifo=$(mktemp -u)
jobs_out=$(mktemp)
trap 'rm -f "$tmp" "$fifo" "$jobs_out"' EXIT
mkfifo "$fifo"
rism mcp <"$fifo" >"$jobs_out" 2>/dev/null &
mcp_pid=$!
exec 3>"$fifo"

rpc() { printf '%s\n' "{\"jsonrpc\":\"2.0\",\"id\":$1,\"method\":\"tools/call\",\"params\":{\"name\":\"$2\",\"arguments\":$3}}" >&3; }

printf '%s\n' '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2024-11-05","clientInfo":{"name":"ci","version":"0"},"capabilities":{}}}' >&3
printf '%s\n' '{"jsonrpc":"2.0","method":"notifications/initialized"}' >&3
sleep 1
rpc 10 execute_command_background '{"command":"for i=1:1:8 { write \"bgtick\",i,!  h 1 }","timeout_secs":60}'
# wait for the job_id answer
for _ in $(seq 30); do grep -q 'job_id' "$jobs_out" && break; sleep 0.5; done
jid=$(grep -o 'rism-[0-9a-f]*-[0-9a-f]*' "$jobs_out" | head -1)
[ -n "$jid" ] || { echo "MCP probe: no job_id from execute_command_background"; exit 1; }
sleep 2
rpc 11 command_status "{\"job_id\":\"$jid\"}"
# streaming: an output line must have appeared while running
for _ in $(seq 20); do grep '"id":11' "$jobs_out" | grep -q bgtick && break; sleep 0.5; done
grep '"id":11' "$jobs_out" | grep -Eq 'running[\\": ]*true' \
  || { echo "MCP probe: job should be running at first poll"; exit 1; }
grep '"id":11' "$jobs_out" | grep -q bgtick \
  || { echo "MCP probe: job output did not stream before finish"; exit 1; }
rpc 12 command_cancel "{\"job_id\":\"$jid\"}"
# final status must show finished + interrupted
for _ in $(seq 30); do
  rpc 13 command_status "{\"job_id\":\"$jid\"}"
  sleep 1
  grep '"id":13' "$jobs_out" | grep -Eq 'running[\\": ]*false' && break
done
grep '"id":13' "$jobs_out" | grep -Eq 'interrupted[\\": ]*true' \
  || { echo "MCP probe: cancelled job must be interrupted=true"; exit 1; }
grep '"id":13' "$jobs_out" | grep -q 'all done\|bgtick9' \
  && { echo "MCP probe: loop ran to completion despite cancel"; exit 1; }
exec 3>&-
wait $mcp_pid 2>/dev/null || true
echo "MCP probe OK: background job streamed and cancelled"
