# Development

## Build & test locally

```bash
cargo build                          # debug binary: target/debug/rism
cargo test                           # 47 unit + wiremock API tests — no server needed
cargo clippy --all-targets -- -D warnings
cargo fmt
```

The test suite mocks the Atelier API with `wiremock` (`tests/api.rs`), so unit/CI
tests never need a live IRIS. A couple of tests exercise the real DBGP parser
against captured frames.

## Live smoke against fresh IRIS

The `Live smoke (fresh IRIS)` CI job boots `intersystemsdc/iris-community:2025.3`
as a service container and drives the shipped binary through every door —
`info`, `doc put`/`compile`/`get`/`delete`, `sql`, `exec`, `test run/list/results`,
`debug run` (real DBGP stop), `monitor`. Its purpose: catch anything a dev box
masks (pre-compiled caches, leftover classes). Local equivalent:

```bash
docker compose up -d            # rism-iris on :52773
export RISM_IRIS_BASE_URL=http://localhost:52773
./target/debug/rism info
```

Fresh-server lessons baked into the workflow (keep them):

- **`doc put` does not compile** — test classes need an explicit `rism compile`
  before `test run`, or the suite reports them missing.
- **`%UnitTest` discovery needs compiled classes** — `test list` returns 0 on an
  empty server; filter by class prefix, not `%` LIKE patterns.
- **DBGP WS frames are `\n`-terminated** and `context_get` requires a prior
  `stack_get` — see [Debugger](debugger.md).

## CI

| Workflow | Trigger | Gates |
|---|---|---|
| `ci.yml` → Lint | PR / push | `cargo fmt --check`, `clippy -D warnings` |
| `ci.yml` → Test | PR / push | `cargo test --locked` (Linux + Windows) |
| `ci.yml` → Live smoke (fresh IRIS) | PR / push | end-to-end on clean 2025.3 |
| `ci.yml` → Build + GitHub Release | tag `v*` | signed binaries, winx64/setup.exe, checksums, git-cliff release notes |
| `pages.yml` | push `main` | `mkdocs build --strict` → GitHub Pages |

Note: clippy on GitHub's current stable flags stricter lints than pinned
toolchains locally (`useless_format`, `items_after_statements`, missing
`must_use`) — run `cargo clippy --all-targets -- -D warnings` on your **latest**
stable before pushing.

## Git Flow

- `development` — daily work, always green.
- `release/vX.Y.Z` — cut from `development`; the only commit here is the
  version bump (`Cargo.toml` + `Cargo.lock`). There is no hand-maintained
  changelog: release notes are generated from commit history (see below).
- PR `release/vX.Y.Z` → `main`; merge (web UI) → tag `vX.Y.Z` → CI builds all
  platforms and publishes the GitHub Release with git-cliff notes.
- After release: sync `main` → `development` with a PR merged as a
  **merge commit** ("Create a merge commit" — never rebase/squash: main's
  commits are the pre-rebase twins of what `development` already holds, so
  replaying them applies every change twice and conflicts). Skip it when
  the trees are identical and force-push is off; the diff is normally just
  the bump commit.

Never `gh release create` — the workflow owns releases.

### Release notes (git-cliff)

`cliff.toml` drives note generation: on each `v*` tag, the release job runs
`git-cliff --latest` and uploads the output as the release description.
Notes come from conventional commits — `feat:` and `fix:` are listed
(scope-annotated, deduplicated), everything else (`chore`, `ci`, `docs`,
`test`, …) is filtered out. Two consequences:

1. **Commit messages are user-facing.** Title every PR merge commit
   `feat: …`/`fix: …` with a scope when useful (`fix(installer): …`); the
   history is the changelog, so write it like one.
2. **Version parity is a tag contract.** The release heading comes from the
   tag; `Cargo.toml`'s `version` must equal it (the docs-drift gate checks
   semver shape, CI derives artifacts from the tag).

## Adding a tool

1. Implement in `src/tools/` mirroring the Prism module (parity first);
   register in `src/mcp/mod.rs`.
2. Wiremock test in `tests/api.rs` + a CLI alias if it's a user-facing door
   (`src/cli/`).
3. If it touches the server: add it to the live-smoke step in `ci.yml`.
4. Document in `docs/mcp-tools.md` — read the `JsonSchema` derive, don't guess.
