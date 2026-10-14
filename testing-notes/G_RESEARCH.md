# G_RESEARCH — mechanisms for the call-graph + partials gaps (PR #22)

Working notes for mission G. Every candidate: source URL + claim + LIVE probe
verdict (CONFIRMED / REFUTED / UNTESTABLE-HERE) + fit. Container: rism-iris
(IRIS 2025.3 build 226U, Atelier API v8), namespace USER, `_SYSTEM` creds via
`/api/atelier/v1/USER/action/query`. Probes dated 2026-10-14.

## Candidate index

| # | Candidate | Kind | Verdict |
|---|---|---|---|
| 1 | crates.io `objectscript-core` 0.1.9 (tree-sitter parser + DependencyGraph) | third-party Rust crate | (evaluating) |
| 2 | `%Dictionary.CompiledMethod.RtnLabel` / `Rtn` | server mechanism | (pending) |
| 3 | `%Dictionary.CompiledCode` (method bodies as SQL rows?) | server mechanism | (pending) |
| 4 | `%Library.RoutineMgr_StudioOpenDialog` / `%Studio.*` search SQL | server mechanism | (pending) |
| 5 | `%SyntaxColor` (token-encoded CSV of source) | server mechanism | (pending) |
| 6 | vscode-objectscript documentIndex / language-server xref | third-party client | (pending) |
| 7 | Legacy Studio `^XDEF`/`^%CC` xref globals | server artifact | (pending) |
| 8 | Partials: CompiledQuery/Trigger/Index/Procedure census | server mechanism | (pending) |

## 1. objectscript-core (crates.io)

- URL: https://crates.io/crates/objectscript-core (versions API: MIT license,
  no MSRV field; updated 2026-10-02; author Hannah Kimura / InterSystems;
  crate repo field empty).
- URL: https://docs.rs/crate/objectscript-core/latest — deps listed:
  `tree-sitter ^0.26.6`, `tree-sitter-objectscript ^1.10.1`,
  `tree-sitter-objectscript-routine ^1.10.1`, `tree-sitter-xml ^0.7.0`,
  `petgraph`, `rayon`, `tokio` (rt-multi-thread), `tower-lsp 0.20`,
  `parking_lot`, `evalexpr`, `regex`, `serde`, `walkdir`.
- `.cargo_vcs_info.json` in the .crate file: repo path
  `objectscript-lsp/crates/objectscript-core` @ sha d9eb8df — a monorepo
  `objectscript-lsp` not public on GitHub under `intersystems` (search API
  2026-10-14 returns only `intersystems/tree-sitter-objectscript`).
- Grammar repo: https://github.com/intersystems/tree-sitter-objectscript
  (C, MIT, pushed 2026-10-09) — the crates wrap generated C parsers
  (`cc ^1.0` build-dep ⇒ **C compilation**, violates the pure-Rust gate).

(analysis in progress)

## 2-5. Server SQL candidates

(pending probes)

## 6-7. Client index / legacy globals

(pending)

## 8. Partials census

(pending)
