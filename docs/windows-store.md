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
  assets/                       # logo.ico (Prism's shield, byte-identical —
                                # deliberate sibling branding) + wizard bitmaps
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
   `docs/mcp-tools.md`; config paths match `settings.rs`; `Cargo.toml`
   version is semver (release notes come from git-cliff, `cliff.toml`).
3. **Real install contract (windows-latest)** — silent install → machine PATH
   + `Get-Command` resolution → VERSIONINFO `FileVersion` → dead-port probe
   must fail cleanly → `%APPDATA%` config pickup → **in-place upgrade**
   (exit code 0 **or 100** per [installer exit codes](installer-exit-codes.md))
   → **uninstall** → verify absence (exe + PATH entry gone). Server-free by
   design: hosted Windows runners cannot run Linux containers, so live-IRIS
   correctness is proven on the Linux leg (`ci.yml` live-smoke) while this leg
   proves the *installer*. The artifact tested here is byte-for-byte the one
   the release workflow ships.

Green here is a hard prerequisite for Store submission — the Store certification
report repeats exactly these scenarios.

## Microsoft Store — current status

| Item | Status |
|---|---|
| Installer passes silent install/upgrade/uninstall on real Windows | ✅ automated in CI |
| Silent-mode switch for the Store (`/VERYSILENT /SUPPRESSMSGBOXES /NORESTART /SP- /ALLUSERS`) | ✅ canonical string in [installer exit codes](installer-exit-codes.md), mirrored by CI |
| Exit-code mapping (all 10 Store scenarios, unique values) | ✅ [installer exit codes](installer-exit-codes.md) + wired in `installer/rism.iss` |
| App has a versioned, reproducible release build | ✅ `ci.yml` tag builds |
| Privacy policy page | ✅ [Privacy Policy](privacy.md) → served at <https://adriaerni.github.io/Rism/privacy/> once merged to main (verify it 200s before submission) |
| Publisher identity (Partner Center account, `Adria Sanchez`) | ⏳ one-time manual setup |
| **Code signing** (publisher must sign — see below) | ⏳ blocking: needs a code-signing certificate |
| Package URL hosting + Packages page values | ⏳ manual (values below, copy-pasteable) |
| Listing metadata (description, screenshots, age rating, data declaration) | ⏳ manual (text below) |
| AGPL-3.0 notice in the Store listing | ⏳ copy ready — included in the listing text below |

### Code signing — the truth for the EXE/MSI path

Unlike MSIX, **Microsoft does NOT re-sign unpackaged EXE/MSI submissions**
(re-signing is MSIX-only). The submission fails certification unless the
**publisher** has signed both the installer `setup.exe` **and** every PE file
inside it (`rism.exe`, the uninstaller Inno stamps) with a certificate whose
chain anchors in the [Microsoft Trusted Root Program](https://learn.microsoft.com/en-us/windows-hardware/drivers/install/the-root-certificate-program-and-windows). Self-signed certificates are not accepted.

- A **standard OV code-signing certificate** from any Trusted-Root vendor is
  sufficient for certification. An **EV** certificate is not required by the
  Store; it mainly buys immediate SmartScreen reputation.
- With an OV cert, submit the signed binary to
  [SmartScreen submission](https://www.microsoft.com/wdsi/filesubmission) early
  and build reputation before going live, or expect first-run warnings.
- Practical pipeline: sign `rism.exe` at build time, compile the installer,
  sign the `setup.exe` output (`signtool sign /fd SHA256 /tr <timestamp-url>
  /td SHA256`), then upload the signed artifact to the release. CI is already
  set up so the artifact tested here is the artifact you sign.

### Alternative: MSIX

Packaging as **MSIX** instead lets Partner Center sign the package for you
(free). Cost: a packaging layer (Desktop App Installer / `msix pack`) and
reworking the PATH/upgrade contract this repo's CI proves. Kept as the fallback
if buying a certificate stalls; everything below is for the EXE path.

### Manual submission checklist (Partner Center)

1. **Account** — register `Adria Sanchez` as publisher in Partner Center
   (one-time; fee).
2. **Reserve the product name** `Rism` in the new Windows App submission.
3. **Certificate** — obtain the code-signing cert above; sign both the exe and
   the setup output; keep the chain installed on the signing machine.
4. **Host the package at a versioned, immutable HTTPS URL** — a GitHub Release
   per tag works: `https://github.com/AdriaERNI/Rism/releases/download/v<X.Y.Z>/rism-<X.Y.Z>-setup.exe`. The binary at the URL must never change after submission.
5. **Packages → Add package (EXE)** — enter exactly:
   - Package URL: the release asset URL for this version
   - Architecture: `x64`
   - App type: `EXE`
   - Installer parameters: `/VERYSILENT /SUPPRESSMSGBOXES /NORESTART /SP- /ALLUSERS`
   - Language: `English`
   - Installer handling exit codes: the table in
     [installer exit codes](installer-exit-codes.md) (cancelled = `2` and `5`;
     misc = `3`, `4`, `6`; success = `0`, upgrade = `100`).
6. **Listing** — properties, description (below), screenshots: at least one
   desktop PNG ≥1366×768 (≤50 MB, up to 10); recommended set: 300×300 app
   tile, 16:9 1920×1080 promo/super-hero art, 9:16 720×1080 poster — all PNG
   (<https://learn.microsoft.com/en-us/windows/apps/publish/publish-your-app/msix/screenshots-and-images>).
   A CLI renders well: capture `rism --help`, `rism doctor`,
   `rism sql "select …"` sessions.
7. **Privacy** — policy URL <https://adriaerni.github.io/Rism/privacy/>;
   data-declaration: **no data collected** (the app has zero telemetry).
8. **Certification notes** — paste the block below.

### Certification notes (paste text)

```
Rism is a command-line tool and MCP server for InterSystems IRIS. It has no
GUI: after install, run "rism" from any new terminal (the installer registers
C:\Program Files\Rism on the system PATH). No account or sign-in is required;
nothing is locked behind a login. The app works offline for its configuration
and help commands; connecting to a database requires you to point it at an
IRIS server of your own, so reviewers without an IRIS instance can validate
the CLI surface offline. Rism collects no telemetry and communicates only
with the IRIS server configured by the user. A Start Menu folder "Rism" is
created containing a documentation link; the executable itself is a CLI and
intentionally has no tile or launch shortcut. User configuration lives in
%APPDATA%\rism\config.toml and is intentionally preserved across uninstall
(it is user data, not an installer remnant); uninstall removes all installed
files, the PATH entry, and the registry entry. License: AGPL-3.0-or-later
(see https://github.com/AdriaERNI/Rism/blob/main/LICENSE).
```

### Store listing text

- **Short description:** `CLI + MCP server for InterSystems IRIS databases.`
- **Description:** Rism is a Windows command-line tool and MCP (Model Context
  Protocol) server for InterSystems IRIS, built on the Atelier REST API with
  zero server-side code deployment. Browse and edit classes and routines, run
  SQL and ObjectScript, diff and sync, manage jobs and CSP — from a terminal
  or from any AI agent that speaks MCP. Pure Rust binary, no runtime
  dependencies. License: AGPL-3.0-or-later; source:
  <https://github.com/AdriaERNI/Rism>.
- **Release notes:** per release from `git-cliff` (`CHANGELOG.md`).

### Update policy

Releases come from tags: `vX.Y.Z` → CI builds + gates the installer → GitHub
Release (immutable asset URL) → new Store submission pointing at that URL. The
Store does not manage versions for unpackaged apps, so each release is a
submission; the `msstore` CLI can automate the upload once the listing exists.

### Why the remaining risk is small

The install contract itself is already proven on real Windows in CI (silent
install/upgrade/uninstall, PATH, ARP entry, exit codes). The only unmet
requirement is the publisher-side one no repository can close alone: a
code-signing certificate.

## Quick links

- Workflow: [.github/workflows/windows-installer.yml](https://github.com/AdriaERNI/Rism/actions/workflows/windows-installer.yml)
- Installer script: `installer/rism.iss`
- Releases: <https://github.com/AdriaERNI/Rism/releases>
