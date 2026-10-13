# R1 REVIEW_ATTACKS — ADVERSARIAL lane (refactorer), 2026-10-10

Head run: merged coder `14b3a5b` (≡ cb51ecf product + plan/facts notes). Binary:
`cargo build -q` debug in THIS worktree; live probes `env -i HOME=$HOME
PATH=<wt>/target/debug:/usr/bin:/bin`, docker rism-iris (healthy) where needed.
Scripts + raw logs: `/home/hermes/.hermes/cache/scratch/rism-r1-attacks/`
(churn_gc.sh/.log, race.sh, corners.sh, door_b.sh, hostcase.sh, squat_test.sh,
dual2.sh, methods.sh). Every door I spawned killed by PID; end-state `pgrep -x
rism` shows only the coder-worktree strays the FACTS file already flagged.

Verdict legend: CONFIRMED = defect with repro; REFUTED = attack closed by
evidence; NOTE = behavior change vs coder's reading, not a code defect.

## 1. A1 session churn (OPEN-FOR-ATTACK #1) — REFUTED as a leak; CONFIRMED as bounded RSS creep
Repro (`churn_gc.sh`): N=500 initialize on one `--port 0` door, 4 cohorts of 125
(init-only×2, full session init+initialized+tools/list×2); VmRSS + rotating
1-sid POST probe every 30 s; rmcp INFO logs "create new session" (count = 500
in stderr, confirming INFO visibility of the creation side).
MEASURED:
- rss0 25,036 kB → after-create 61,668 kB (~73 B…74 B/session×? no: ~73 kB
  per session *cohort-wide* → ≈ 73 kB/125 ≈ 0.6 MB/sessions is wrong; delta
  36.6 MB / 500 sessions ≈ 73 kB/session).
- ALL four cohorts flip to DEAD(404) in the 272 s → 302 s window:
  `t=272s ...=ALIVE(200) / t=302s io_a=DEAD(404) io_b=DEAD(404) f_a=DEAD(404)
  f_b=DEAD(404)` — keep_alive=300 s reaper fires for BOTH shapes, INCLUDING
  init-only sessions that never sent `notifications/initialized` (closes
  coder's caveat) and the never-probed members (final sample: dead=8/8 ×4).
- RSS after reap plateaus at 61,876 kB and does NOT return to the 25 MB
  baseline over the following 300 s. Workers are gone (sessions 404, and
  churn2 threads stayed 8 at all samples); this is allocator arena high-water
  (glibc arenas grow with the 500 short-lived worker threads, don't return to
  the OS), not a session-manager leak. SECOND CYCLE CONFIRMS (churn2.sh,
  fresh door, 250 init-only → idle 340 s → 250 more): rss 26,616 → 30,072
  (cycle A) → 30,072 (idle A, threads 8→7) → 30,112 (cycle B, threads 7 —
  NO re-growth, fd_count 10 at end). Cycle A grew, cycle B at the same
  concurrency did not: allocator warm-up high-water, NOT per-churn growth,
  NOT a leak. Row CLOSED: REFUTED.
⇒ A1 ledger: PROVEN-OK (bounded, reclaimed); severity of the RSS creep: nit
  (DoS amplification is ~73 kB/session × 500 = ~37 MB and FLAT; unbounded
  churn is self-limiting at keep_alive intervals, an attacker's steady-state
  cost is ~37 kB/s per session-second at 500 concurrent, reclaimed every 5 min).
  Recommend docs note at most.

## 2. B1/A6 named-host loopback gate — CONFIRMED (NEW finding; dead code + misleading UX)
Repro: `--host ip6-localhost --port 0` (ip6-localhost = ::1, loopback, in the
stock /etc/hosts) → `Error: configuration error: refusing to bind the HTTP MCP
door to 'ip6-localhost' without --allow-all-interfaces: … every host on the
network could drive all its then-25-tool surface`. And `--host ip6-localhost
--allow-all-interfaces` BINDS FINE on `[::1]:34137` printing the all-network
warning banner. `--host hermes-agent` (127.0.1.1, loopback per /etc/hosts on
this box) → same refusal.
Root cause (source, `src/mcp/http.rs:50-52`): the resolution branch is DEAD
CODE. `str::to_socket_addrs` on a portless name ALWAYS errors with "invalid
socket address" — proved standalone (`rustc -O addrprobe.rs`: `to_socket_addrs`
on "localhost", "127.0.0.1", even "ip6-localhost" → `Err("invalid socket
address")`; std resolves `ToSocketAddrs for str` as "host:service", the bare
name has no service part). So for every host string except the `"localhost"`
literal early-return (http.rs:44) and IPv4/IPv6 literals (`parse::<IpAddr>`),
`is_loopback_host` returns FALSE regardless of DNS. The `is_ok_and` guard can
never see Ok.
⇒ Loopback NAMED hosts are refused and the error text misdirects the user
  toward `--allow-all-interfaces`, which flips a genuine no-auth exposure
  posture ON (and disables rmcp's Host guard) for what is actually a loopback
  bind. Area: rust (dead code) + security-UX. Severity: minor (default and
  documented paths unaffected; unit test http.rs:230 asserts the dead branch's
  FALSE outcome for *.invalid — a coincidence the code never executes as
  written). Fix (non-breaking): pass a service (`format!("{host}:0")`) or use
  `tokio::net::lookup_host` (also closes F3), or drop the branch and document
  "named hosts unsupported".
TOCTOU (B1): attack stands as SOURCE-READ only — the two-resolution window
  requires a mutable resolver; NOT reproducibly attackable read-only, and the
  dead-branch finding above SUBSUMES it in practice (the gate never resolves
  names at all, so it never has two moments to disagree — bind re-resolves,
  gate doesn't). Ledger B1: FINDING(1) = the dead branch.

## 3. A3 Host/Origin corner sweep — CONFIRMED (extends coder rows), NOTE on Host port
Repro (`door_b.sh`, `hostcase.sh`), one loopback door + one `--host 127.0.0.1
--allow-all-interfaces` door + one `::1` door:
- loopback door: `Host: evil.example` → 403; **`Host: LOCALHOST` /
  `LocalHost` → 200** (rmcp lowercases before compare, tower.rs parse_host/
  normalize_host — fine, just documented); **`Host: 127.0.0.1:1` (wrong port)
  → 200** — the allow-list match ignores the port half (tower.rs:819-823), so
  one bound door answers for `Host: 127.0.0.1:<any>`: two doors on
  127.0.0.1:3000/:3001 → cross-Host confusion (a page scripted for :3001 can
  drive :3000 — same-origin policy normally blocks, DNS-rebinding to
  127.0.0.1 with an arbitrary port passes). NOTE-level; defense-in-depth gap,
  spec text does not mandate port pinning.
- `Host: 2130706433` (decimal 127.0.0.1), `Host: 0177.0.0.1` (octal), `Host:
  localhost.` (trailing dot), `Host: ::1` (bare) → ALL 403 ✔ (no parser-diff
  bypass; bare `::1` 403s while `[::1]` 200s — rmcp's bracket strip).
- **`Origin: https://evil.test` → 200 on EVERY door** (loopback, allow-all,
  ::1): Origin is NEVER validated anywhere; with `--allow-all-interfaces` the
  Host guard is ALSO gone (`Host: evil.example` → 200; both headers evil → 200).
  Browser simple-request class (text/plain CT is 415-rejected — see §7 — which
  incidentally blocks cross-origin POST without preflight, so the practical
  browser drive-by is harder than "any page can call it", but the spec MUST
  text is still unmet and a same-IP/XSS-on-localhost pivot needs no Origin).
  Confirms coder A3 → severity call for architect stands: **major** candidate
  (spec Security Warning MUST + no-auth 25-tool surface incl. run_shell).
- `--host ::1` door answers on its own printed URL ✔ and Host `[::1]:<wrong
  port>` → 200 (same port-agnostic NOTE).

## 4. C factory/clone cost — REFUTED (per-request clone is shallow; one negotiate invariant holds)
- Source: tower.rs:1086 `let service = self.clone()` — the SERVICE clones per
  request; factory only at :1890/:1983/:1323. `RismMcp` = {client:
  IrisClient(Arc<Inner>), tool_router: ToolRouter} (mod.rs:42-45): Inner =
  {reqwest Client, base String, Mutex<Settings>} (iris/http.rs:18-22) — clone
  = Arc bump + ToolRouter HashMap of Arc'd entries. settings() takes the
  std::sync::Mutex ONLY to clone the snapshot (iris/http.rs:83-88) — no
  registry-style lock held across .await anywhere in the call path.
- jobs.rs audit (OPEN-FOR-ATTACK #2): every `.await` in jobs.rs (:241 inside
  tokio::spawn AFTER the registry block closes at :229, :265, :276) sits
  OUTSIDE any Mutex scope; `lock_reg()` guards are function-scope only
  (:221,:345,:380,:434,:453,:485); SharedBuf/JobSlot guards all drop at
  statement end. REFUTED: no std::sync guard crosses an await point.
- dbgp state: `src/tools/debugger.rs:149-151` — sessions live in a process-
  wide `OnceLock<Mutex<HashMap>>`, NOT per-RismMcp-clone: coherent across all
  HTTP sessions AND the stdio door of the same process. REFUTED as a fork
  risk. next_id():156 mixes ns-time xor counter — collision-safe enough, nit
  at most (not bearer-grade entropy, but session-scoped handles only).
- tasks.rs: zero locks, zero awaits in shared paths (grep clean).
- C3 dual-door (OPEN via plan): two PROCESSES = two registries, CONFIRMED
  structurally: task ids embed creator pid (`rism-8b312-…` = http door pid,
  `printf %x` match) — a tasks/get for that id on the stdio process cannot
  resolve (measured: stdio answered -32021 capability refusal before reaching
  lookup — its gate fired first; the id-namespace separation is the durable
  evidence). ONE process CANNOT open both doors: main.rs dispatch selects one
  transport arm. NOTE for docs: running two `rism mcp` processes (the natural
  way to serve stdio+http) silently splits the 16-job cap into 2×16 and the
  task-id space; tasks/get is pid-routed. Non-breaking doc row (area: parity).

## 5. D1 cancel/shutdown races — REFUTED
- race.sh: 30 concurrent initializes then immediate `kill -TERM`: exit 0,
  0 panics, session workers logged `quit_reason=Closed` ×5 + `task cancelled`
  — cancellation reaches workers (child_token, tower.rs:1165+).
- race2.sh: SIGTERM while an open GET SSE stream + live session: door exit 0,
  the GET stream terminated cleanly (18 B — priming frames only), no hang.
- FACTS D1's mid-tools/call SIGTERM result (job dies with process, exit 0)
  stands; registry is in-process so no orphaned-running ghost is reachable.

## 6. F5 test vacuity + squat-flake — CONFIRMED (two rows)
- Mutation A: delete the `if all_interfaces { config = config.
  disable_allowed_hosts(); }` branch in src/mcp/http.rs (git checkout
  after, NOT committed) → `cargo test -q --test cli_transport
  --test http_door_stateless`: **7/7 pass** (lib 73 also pass). NOTHING in the
  suite exercises the allow-all HOST-GUARD behavior — the opt-out's central
  security meaning (banner says "guard disabled", A3 measured the effect) is
  unpinned; a regression that keeps the guard ON under allow-all (breaking the
  documented opt-out) or silently removes the branch ships green. FINDING
  (tests, minor→major-adjacent: it is the security-surface pin). Recommended
  non-breaking pin: spawn `serve_http_on(_,_,true)` on :0, POST `Host:
  evil.example` → expect 200; with `false` → expect 403.
- Mutation B: `.with_legacy_session_mode(true)` → false:
  http_door_stateless FAILS loudly (no session id issued) ✔ — that pin is real.
- Squat flake: `python3 -c "bind 127.0.0.1:39999, listen"` while
  `cargo test --test cli_transport` → `stdio_door_with_net_flags_warns_and_
  serves_stdio FAILED` at tests/cli_transport.rs:79 ("stdio transport must not
  open the --port" connect-must-fail on a FIXED port). CONFIRMED: a foreign
  squatter (or a crashed sibling test in shared CI) turns a true contract into
  a red build. Fix: bind an ephemeral port, pass it as --port, assert nobody
  listens there by connect+RST timing OR probe only our known pid namespace —
  simplest honest fix is a port from `:0` listener that we immediately drop.
  (area: tests, minor, CI-located.)
- G4 probe vacuity: reproduced the probe's exact tail shape (`kill -TERM $p;
  wait $p || true`) against a child that traps TERM and exits 3 → prints
  "… SIGTERM exit 0" anyway; control without `|| true` shows exit=4.
  CONFIRMED (tests, minor): the probe's "SIGTERM exit 0" claim is a hardcoded
  echo. The door itself DID exit 0 in all my races — the claim is currently
  true, unasserted.
- DELETE-leg vacuity (coder A4) re-confirmed at read level:
  tests/http_door_stateless.rs:137-142 `.expect("delete answered")` checks
  transport success only; no status assert, no after-DELETE 404 (plan A4).

## 7. E/spec corners not in the FACTS file — new pins (all live door)
- Missing/wrong Content-Type → **415** (rmcp tower.rs content-type check
  BEFORE body parse); body `[{…},{…}]` (2025-03 JSON-RPC batch) → 415
  "fail to deserialize" — spec-correct rejection of batching (2025-11-25+
  removed it). No 4xx matrix row exists in tests for either.
- 5 MB body → **413** "request body exceeds 4194304 bytes" (rmcp default cap;
  we never set `max_request_body_bytes` — fine, cite-only).
- Double initialize on a LIVE sid → **200** with a fresh result (rmcp does
  not 4xx a re-handshake; spec-silent; NOTE).
- initialize body 2025-03-26 + header 2025-06-18 → 400 -32600 (header/body
  coherence enforced); follow-up with NEWER header (2025-06-18) on a
  2025-03-26 session → **200** — rmcp only rejects unknown versions, not
  off-negotiation ones (tower.rs:285-315): spec SHOULD "use the negotiated
  version in the header"; lenient, NOTE-level, rmcp-owned.
- unknown JSON-RPC method over HTTP session → 200/-32601 (same as stdio) ✔.
- OPTIONS/PUT/PATCH → 405 **with `allow: POST` header on EVERY route-miss,
  including on a 406** — discovery: rmcp's error-builder attaches
  `ALLOW: "POST"` (method_not_allowed_response, tower.rs:441-446) to
  non-405 responses too, so my methods.sh/getmatrix.sh GET probe rows printed
  a 405-shaped summary line. One getmatrix GET variant contradicted the
  source reading (expected 406/400 by tower.rs:1578+/1631); the tooling
  artifact above is the most probable explanation and my budget cut before a
  clean re-probe. HONEST STATUS: the GET-stream happy path IS proven by
  race2.sh (real SSE priming frames `data:/id: 0/retry: 3000` captured twice)
    (see ledger). Final budget-cut probes (getrecheck.sh, getfinal.sh,
  settle.sh): every GET-family shape I could build in those runs → **405
  `allow: POST`** (bare, es-accept, both-accept, + explicit
  `MCP-Protocol-Version: 2025-03-26` header; OPTIONS/PUT/PATCH identical;
  body 18 B = method_not_allowed_response, tower.rs:441 static ALLOW "POST").
  CAVEAT that keeps this row NOT-TESTED-CONCLUSIVE: in exactly those runs my
  harness captured sid='' (initialize answered 200 + SSE body, but my header
  grep came back empty), so even the "GET sid" rows were really GET-without
  a validated session; churn_gc/door_b/dual/corners runs captured sids
  correctly with equivalent code and door_b.sh + race2.sh opened REAL SSE
  streams (priming `data: / id: 0 / retry: 3000` frames, twice). FACTS A5
  meanwhile measured "GET without sid → 400" and "GET without es-accept →
  406" against its door. ⇒ DIRECT LANE DISAGREEMENT (400/406 vs my 405) on
  the same binary. Root cause unresolved within budget — most-probable
  candidate is a difference in door STATE between the runs (which door
  binary/args each script actually spawned), not the protocol. ARCHITECT:
  spend ONE curl pair first thing (no-sid GET with/without version header
  against a fresh `--port 0` door; print full response headers) before
  shipping ANY GET-family row; the two FACTS-proven rows (406 wrong-Accept,
  SSE-on-live-session) stand either way.

## 8. Attacked PROVEN-OKs from FACTS that I could NOT break (ledger-closing)
- A2 entropy sweep method is sound (uuid v4 via getrandom; no override path —
  session_id() is a free fn, no config seam). PROVEN-OK stands.
- B4 ready-line-after-negotiate: confirmed the 30 s blackhole window again
  (source unchanged); my churn doors (live IRIS) printed ready before any
  accept I could observe — no change to coder's row.
- D3 banner asymmetry: reproduced (ready line bare eprintln vs tracing-
  timestamped stdio banner). PROVEN as minor parity nit; verdict's to place.
- G1 byte-identical stdio: not re-attacked (coder built both binaries; diff
  was real sha/cmp, not semantic) — accept.
- C1 one-negotiate: my 500-session churn door logged EXACTLY ONE "negotiated
  Atelier API v8" in stderr across 500 initializes + 300+ inlines (log
  grep) — independent confirmation of coder's oracle at 100× scale.

## Ledger (questions A–G, adversarial lane)
A: PROVEN-OK(1,2,4,6 re-confirmed) + FINDING(2: Origin-never + allow-all no-pin
   [§3,§6A]; A1 churn REFUTED with GC proof). B: FINDING(1: dead resolver
   branch, §2). C: PROVEN-OK (§4; dual-door = doc NOTE). D: PROVEN-OK (§5). E:
   PROVEN-OK + FINDING(1: no test pins the 415/413/400-header corners §7 —
   same test-gap family as A5). F: FINDING(1: F5 squat flake §6; mutations
   pin-tested except allow-all). G: FINDING(1: probe vacuity §6C — coder's
   G4 CONFIRMED by execution this time; + doc NOTE C3 dual-process cap).
Re-probe-or-source rows: GET corner matrix (§7 last bullet).

Process hygiene: all doors spawned by this lane killed by PID (churn/corners/
race/methods/hostcase); the only survivors are the coder-worktree strays
already flagged in REVIEW_FACTS §Process hygiene. No repo file left modified
(two mutations reverted via `git checkout --`; `git status` clean at commit).
