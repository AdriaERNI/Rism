# Copilot instructions

**[AGENTS.md](../AGENTS.md) at the repo root is canonical** for build, test,
gate commands, architecture rules, and PR policy. Read it first; this file
only adds what agents tend to get wrong here.

- Never rename, add, or remove the 25 MCP tool names or their wire fields —
  they are a frozen contract (`src/mcp/mod.rs`, CI probes count them).
- Any change under `docs/` or `README.md` must keep
  `mkdocs build --strict` green — links and nav are gate-checked.
- Code style is enforced by `[lints]` in `Cargo.toml`
  (`forbid(unsafe_code)`, `deny(unwrap_used/expect_used)`); handle errors,
  don't `unwrap()` them.

Details: [docs/development.md](../docs/development.md).
