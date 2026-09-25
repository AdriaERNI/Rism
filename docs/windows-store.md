# Windows & Microsoft Store

How Rism ships on Windows: the binary, the installer, the CI gate that proves
them, and the current state of Store readiness.

## The binary

`rism.exe` is a pure-Rust MSVC build (`cargo build --release --target
x86_64-pc-windows-msvc`) — no Python runtime, no Visual C++ **redistributable**
requirement beyond what ships in Windows. That removes the biggest support class
Prism had on Windows (frozen-Python AV false positives, vcredist merges).
The build is produced in CI (tag builds: `ci.yml`, Windows matrix leg).

## The installer

`installer/rism.iss` — Inno Setup 6, per-machine install to
`{autopf}\Rism` with an optional PATH task. Built by CI as
`rism-<version>-setup.exe` and attached to the GitHub Release. Config lives at
`%APPDATA%\rism\config.toml` (see [Configuration](configuration.md)) — the
installer never touches it, so upgrades and uninstall preserve user settings.

Layout:

```
installer/
  rism.iss                      # the script
  assets/                       # logo.ico + wizard bitmaps
  static/analyze-install-consistency.ps1
  static/analyze-docs-drift.ps1
```

## The Windows gate (what CI proves before a release)

`.github/workflows/windows-installer.yml` runs on every PR that touches the
installer or the Rust sources:

1. **Consistency (Linux, cheap)** — static scans: `.iss` references exist,
   PATH contract present, no stale vendor strings.
2. **Docs drift (Linux, cheap)** — every tool-count claim in the docs matches
   the `#[tool]` definitions in code; every tool name has a section in
   `docs/mcp-tools.md`; config paths match `settings.rs`; CHANGELOG top ==
   `Cargo.toml`.
3. **Real install contract (windows-latest + live IRIS service)** — silent
   install → `rism --version` → `rism info` + `rism sql` against the fresh
   server → `%APPDATA%` config pickup → **in-place upgrade** → **uninstall** →
   verify absence (exe, PATH entry, Start-menu). The artifact proven here is
   byte-for-byte the artifact the release workflow ships.

Green here is a hard prerequisite for Store submission — the Store certification
report repeats exactly these scenarios.

## Microsoft Store — current status

| Item | Status |
|---|---|
| Installer passes silent install/upgrade/uninstall on real Windows | ✅ automated in CI |
| App has a versioned, reproducible release build | ✅ `ci.yml` tag builds |
| Publisher identity (Partner Center account, `Adria Sanchez`) | ⏳ one-time manual setup |
| **Store signature** (EV cert, or Partner Center submission-time signing) | ⏳ decision pending |
| Win32 packaging (MSIX wrapper) vs **unpackaged** Store listing | ⏳ decision pending |
| Privacy policy URL, age ratings, data declaration | ⏳ needs a live site URL |
| AGPL-3.0 notice in the Store listing | ⏳ copy is ready (`LICENSE`) |

### What's left, concretely

1. **Sign the installer.** Store Desktop-app submissions must be signed.
   Two supported paths: an **EV code-signing certificate** on the build (then
   `msstore` CLI or Partner Center upload), or let **Partner Center sign it**
   at submission (upload the unsigned `.exe`, Store injects the signature).
   CI is already set up so the artifact submitted = the artifact tested.
2. **Choose packaging.** For a CLI tool the pragmatic listing is a
   **Win32 (MSIX) package** generated once with the Desktop App Installer —
   `rism.exe` becomes launchable-as-command from the Store install folder.
   An **unpackaged installer** submission is simpler but loses auto-update;
   CI keeps both options open because the `.iss` and the raw exe ship side
   by side in every release.
3. **Store metadata.** Screenshots (CLI sessions render well), description
   (reuse `docs/index.md`), privacy-policy URL — GitHub Pages is already
   enabled; add a short `docs/privacy.md` with "Rism stores credentials in
   `%APPDATA%`, never phones home" (true: zero telemetry).
4. **Update policy.** Releases come from tags; the Store listing updates per
   release (same artifact). Document the cadence in `docs/development.md`
   Git Flow section — already matches: `vX.Y.Z` tag → CI → GitHub Release →
   Store submission (`msstore upload`).

### Why this is low-risk

The only store-specific failure modes are signing and metadata — the binary
needs no special capability handling (no COM, no install hooks beyond PATH),
which the CI gate has already exercised on the exact runner image Microsoft
uses (`windows-latest`).

## Quick links

- Workflow: [.github/workflows/windows-installer.yml](https://github.com/AdriaERNI/Rism/actions/workflows/windows-installer.yml)
- Installer script: `installer/rism.iss`
- Releases: <https://github.com/AdriaERNI/Rism/releases>
