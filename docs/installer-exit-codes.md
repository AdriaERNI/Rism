# Installer Exit Codes

Every scenario the Rism Windows installer (`rism-<version>-setup.exe`) can end
with, the process exit code it returns, and the value to enter for each
scenario in the **Microsoft Store** package submission ("Installer handling"
for EXE apps).

Built with [Inno Setup 6](https://jrsoftware.org/); codes 0–8 are Inno's
documented semantics (<https://jrsoftware.org/ishelp/topic_setupexitcodes.htm>),
100–102 are custom values reported via `GetCustomSetupExitCode` in
`installer/rism.iss`. The Store rejects a submission where two scenarios share
the same value, so every scenario has a distinct code.

## Who consumes these codes

- **Microsoft Store** — the *Installer handling* step of the package
  submission maps its install scenarios to the values below.
- **Deployment scripts** — silent installs (`/VERYSILENT`) branch on the exit
  code; success is **0 or 100** (see note under the table).
- **CI** — `windows-installer.yml` asserts this contract on a real Windows
  runner (the upgrade leg accepts 0 or 100 and fails anything else).

## Exit codes

| Code | Meaning | When Rism's installer returns it |
|---|---|---|
| `0` | Installation successful | Setup ran to completion (first install, or `/HELP` / `/?`). |
| `1` | Installation already in progress | Setup failed to initialize. **Reserved**: no `SetupMutex` is wired, so a second instance currently runs freely; kept as a unique value for the Store scenario and a future single-instance guard. |
| `2` | Installation cancelled by user | **Cancel** clicked in the wizard before installation started, or **No** on the opening "This will install…" box. |
| `3` | Miscellaneous install failure | Fatal error preparing to move to the next installation phase (out of memory / Windows resources — rare). |
| `4` | Miscellaneous install failure | Fatal error during the actual installation. (Abort at an Abort/Retry/Ignore box is **5**, not fatal.) |
| `5` | Installation cancelled by user | **Cancel** during the actual installation, or **Abort** at an Abort/Retry/Ignore box. |
| `6` | Miscellaneous install failure | Process killed by the debugger (`Run \| Terminate` in the Compiler IDE). Not reachable in production. |
| `7` | Disk space is full | *Preparing to Install* found insufficient free space on the target volume. |
| `8` | Reboot required | *Preparing to Install* determined installation cannot continue without a restart. |
| `100` | Application already exists | A previous install is registered on the device (uninstall key for this AppId present in HKLM or HKCU) and Setup still ran to completion — i.e. an **upgrade / re-install**. Snapshot taken in `InitializeSetup`, reported via `GetCustomSetupExitCode`. |
| `101` | Network failure | **Reserved** — the installer is a standalone/offline package and fetches no remote payload, so nothing raises it today. |
| `102` | Package rejected during installation | **Reserved** — for a device-side security-policy hook; none exists today. |

!!! note "Upgrade = 100, not 0"
    A re-install over an existing copy returns **100**, because
    `GetCustomSetupExitCode` fires when Setup would otherwise return 0 and the
    "already installed" snapshot was true. Deployments treating success should
    accept `0` **and** `100`. Any other non-zero code means the install did
    **not** complete (0 and only 0/100 mean "installed").

## Microsoft Store — Package submission values

Store page: **Packages → Add package (EXE)**
(<https://learn.microsoft.com/en-us/windows/apps/publish/publish-your-app/msi/upload-app-packages>).

| Store field | Value |
|---|---|
| Package URL | Versioned secure URL to the **standalone/offline** `rism-<version>-setup.exe` on your CDN (e.g. `https://…/downloads/0.1.0/rism-0.1.0-setup.exe`). A downloader stub is grounds for rejection; the binary at the URL must not change after submission. |
| Architecture | `x64` |
| App type | `EXE` |
| Installer parameters (silent) | `/VERYSILENT /SUPPRESSMSGBOXES /NORESTART` (Inno's silent trio; `/SP-` suppresses the "This will install" box so the Store flow can't return 2) |
| Installation successful | `0` |
| Application already exists | `100` |
| Installation cancelled by user | `2` |
| Installation already in progress | `1` |
| Disk space is full | `7` |
| Reboot required | `8` |
| Network failure | `101` |
| Package rejected during installation | `102` |
| Miscellaneous install failure | `3` (and, per Inno semantics, `4`/`5`/`6` may also occur — the Store lets you register multiple codes per scenario; add them) |
| Documentation URL (misc failures) | point at this page |

## Uninstall

`unins000.exe` accepts the same silent trio. Exit codes follow Inno's
uninstaller semantics; **0** means removed. CI verifies the absence contract
after silent uninstall: no `rism.exe`, no PATH entry, uninstall key gone.

## Detecting "already installed" (deployment hint)

The uninstall registry key that Rism itself uses for the 100 signal:

```
HKLM  (or HKCU) \SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall\
  {D1C0A9E4-2B6F-4A3D-8E57-0C6B9F1A4E3D}_is1
```

The GUID is the installer's `AppId` (never change it once shipped — upgrade
detection keys on it).
