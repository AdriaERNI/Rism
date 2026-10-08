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
  # Rail A: a non-Tasks client asking for background execution must get an
  # honest tool-level error, never a blocking call that times out.
  printf '%s\n' '{"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"execute_command","arguments":{"command":"write 1","background":true}}}'
} | timeout 60 rism mcp 2>/dev/null > "$tmp" || { echo "MCP probe: rism mcp failed"; exit 1; }

# tools/list answer must list exactly 25 tools (background execution is the
# SEP-2663 Tasks extension, not extra tools). inputSchema is the exact
# marker (one per tool); "name" appears many times on the wire (extra
# mentions in descriptions/schemas — never count those).
tools_line=$(grep -E '"id"[[:space:]]*:[[:space:]]*2' "$tmp" | head -1)
n=$(printf '%s' "$tools_line" | grep -o '"inputSchema"' | wc -l)
if [ "${n:-0}" -ne 25 ]; then
  echo "MCP probe: expected 25 tools, counted $n"
  exit 1
fi
for gone in execute_command_background command_status command_cancel; do
  if printf '%s' "$tools_line" | grep -q "\"name\":\"$gone\""; then
    echo "MCP probe: $gone must be replaced by MCP Tasks"
    exit 1
  fi
done

# MCP best-practice contract: execute_command ships annotations (rmcp
# renders them camelCase on the wire) and the background param in its schema.
if ! printf '%s' "$tools_line" | grep -q '"destructiveHint"'; then
  echo "MCP probe: execute_command missing destructiveHint annotation"
  exit 1
fi
if ! printf '%s' "$tools_line" | grep -q '"background"'; then
  echo "MCP probe: execute_command schema missing background param"
  exit 1
fi

# execute_sql answer must be a successful result carrying the ok column.
# isError is the authoritative flag — a substring like "ok" in an error
# message ("could not open...") must never pass this gate.
if ! grep -E '"id"[[:space:]]*:[[:space:]]*3' "$tmp" | grep -q '"isError":false'; then
  echo "MCP probe: execute_sql did not answer cleanly"
  exit 1
fi

# Rail A gate: the background call above (id 4) must be a refusal naming
# the Tasks extension — plain text inside the content block, so the
# substring survives JSON escaping.
grep -E '"id"[[:space:]]*:[[:space:]]*4' "$tmp" | grep -q '"isError":true' \
  || { echo "MCP probe: background=true must refuse for non-Tasks clients"; exit 1; }
grep -E '"id"[[:space:]]*:[[:space:]]*4' "$tmp" | grep -q 'requires the Tasks extension' \
  || { echo "MCP probe: Rail-A refusal must name the Tasks extension"; exit 1; }

echo "MCP probe OK: $n tools, SQL answered, Rail-A refusal honest"

# --- background task contract (MCP Tasks, stream + cancel), same door -----
# SEP-2575 INLINE lifecycle (no initialize handshake): every request
# carries protocolVersion 2026-07-28 + the Tasks extension key in its
# _meta. That is >= the 2026-06-30 floor where SEP-2663 is defined, and
# it is the ONLY way to run tasks against this rmcp build over stdio —
# rmcp 3.4.x cannot negotiate a 2026 version through `initialize` (it
# falls back to 2025-11-25, under which Rism MUST treat the extension key
# as inert per the SEP-2663 compat table; proven by the inert check at
# the end of this file). tasks/get must show the streamed byte count GROW
# between polls; tasks/cancel must land cancelled and freeze the count.
fifo=$(mktemp -u)
jobs_out=$(mktemp)
trap 'rm -f "$tmp" "$fifo" "$jobs_out"' EXIT
mkfifo "$fifo"
rism mcp <"$fifo" >"$jobs_out" 2>/dev/null &
mcp_pid=$!
exec 3>"$fifo"

# _meta object reused on every inline request (single quotes; no $ inside)
META='"_meta":{"io.modelcontextprotocol/protocolVersion":"2026-07-28","io.modelcontextprotocol/clientInfo":{"name":"ci","version":"0"},"io.modelcontextprotocol/clientCapabilities":{"extensions":{"io.modelcontextprotocol/tasks":{}}}}'

send() { printf '%s\n' "$1" >&3; }

send "{\"jsonrpc\":\"2.0\",\"id\":10,\"method\":\"tools/call\",\"params\":{\"name\":\"execute_command\",\"arguments\":{\"command\":\"for i=1:1:8 { write \\\"bgtick\\\",i,!  h 1 }\",\"timeout_secs\":60,\"background\":true},$META}}"
# wait for the task handle (top-level resultType is compact JSON, rmcp-
# serialized — NOT the escaped map_json form)
for _ in $(seq 30); do grep -q '"resultType":"task"' "$jobs_out" && break; sleep 0.5; done
grep -q '"resultType":"task"' "$jobs_out" \
  || { echo "MCP probe: background=true did not create a task (inline lifecycle)"; exit 1; }
tid=$(grep -o 'rism-[0-9a-f]*-[0-9a-f]*' "$jobs_out" | head -1)
[ -n "$tid" ] || { echo "MCP probe: no taskId on the task handle"; exit 1; }
# G1 contract: id suffix must be a 16-hex random (unguessable, SEP-2663
# Security MUST) — not a small decimal counter.
suffix=${tid##*-}
case "$suffix" in
  [0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f]) ;;
  *) echo "MCP probe: taskId suffix not 16-hex random: $tid"; exit 1;;
esac

# streaming: the streamed-byte count must grow between two polls
sleep 2
send "{\"jsonrpc\":\"2.0\",\"id\":11,\"method\":\"tasks/get\",\"params\":{\"taskId\":\"$tid\",$META}}"
for _ in $(seq 20); do grep '"id":11' "$jobs_out" | grep -q '"status":"working"' && break; sleep 0.5; done
c1=$(grep '"id":11' "$jobs_out" | grep -o 'streamed [0-9]*' | tail -1 | grep -o '[0-9]*')
[ -n "${c1:-}" ] && [ "$c1" -gt 0 ] \
  || { echo "MCP probe: task streamed no output: $(grep '"id":11' "$jobs_out" | tail -1)"; exit 1; }
sleep 2
send "{\"jsonrpc\":\"2.0\",\"id\":12,\"method\":\"tasks/get\",\"params\":{\"taskId\":\"$tid\",$META}}"
for _ in $(seq 20); do
  grep '"id":12' "$jobs_out" | grep -Eq 'streamed ([0-9]+)' && break
  sleep 0.5
done
c2=$(grep '"id":12' "$jobs_out" | grep -o 'streamed [0-9]*' | tail -1 | grep -o '[0-9]*')
[ -n "${c2:-}" ] && [ "$c2" -gt "$c1" ] \
  || { echo "MCP probe: task output did not grow between polls ($c1 -> $c2)"; exit 1; }

send "{\"jsonrpc\":\"2.0\",\"id\":13,\"method\":\"tasks/cancel\",\"params\":{\"taskId\":\"$tid\",$META}}"
# final state must be cancelled, and frozen (count stops growing)
for _ in $(seq 30); do
  send "{\"jsonrpc\":\"2.0\",\"id\":14,\"method\":\"tasks/get\",\"params\":{\"taskId\":\"$tid\",$META}}"
  sleep 1
  grep '"id":14' "$jobs_out" | grep -q '"status":"cancelled"' && break
done
grep '"id":14' "$jobs_out" | grep -q '"status":"cancelled"' \
  || { echo "MCP probe: cancelled task must report status cancelled"; exit 1; }
cf=$(grep '"id":14' "$jobs_out" | grep -o 'streamed [0-9]*' | tail -1 | grep -o '[0-9]*')
sleep 2
send "{\"jsonrpc\":\"2.0\",\"id\":15,\"method\":\"tasks/get\",\"params\":{\"taskId\":\"$tid\",$META}}"
sleep 1
cl=$(grep '"id":15' "$jobs_out" | grep -o 'streamed [0-9]*' | tail -1 | grep -o '[0-9]*')
[ "$cl" = "$cf" ] \
  || { echo "MCP probe: task still streaming after cancel ($cf -> $cl)"; exit 1; }
exec 3>&-
wait $mcp_pid 2>/dev/null || true
echo "MCP probe OK: background task streamed, cancelled, and froze"

# --- G2 inert-key contract: handshake declaring the extension under a
# PRE-extension negotiated version must NOT get a task. rmcp 3.4.x
# negotiates 2025-11-25 even when asked for 2026-06-30 (probed live), so
# this session is pre-floor by construction.
printf '%s\n' \
  '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2026-06-30","clientInfo":{"name":"ci","version":"0"},"capabilities":{"extensions":{"io.modelcontextprotocol/tasks":{}}}}}' \
  '{"jsonrpc":"2.0","method":"notifications/initialized"}' \
  '{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"execute_command","arguments":{"command":"write 1","background":true}}}' \
| timeout 30 rism mcp 2>/dev/null > "$tmp" || { echo "MCP probe: inert-key door failed"; exit 1; }
grep -E '"id"[[:space:]]*:[[:space:]]*2' "$tmp" | grep -q '"isError":true' \
  || { echo "MCP probe: handshake-declared task must stay inert (Rail-A)"; exit 1; }
grep -E '"id"[[:space:]]*:[[:space:]]*2' "$tmp" | grep -q '2026-06-30' \
  || { echo "MCP probe: Rail-A refusal must name the protocol floor"; exit 1; }
echo "MCP probe OK: pre-2026-06-30 extension key treated as absent"
