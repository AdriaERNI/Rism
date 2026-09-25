# Static installer-script consistency (Rism Windows installer gate).
# Fail-fast, no build needed: the .iss must reference paths that actually
# exist and flags the CI contract relies on. Exit codes:
#   0 OK | 1 a check failed
$ErrorActionPreference = 'Stop'
$fail = 0
function Fail($msg) { Write-Host "FAIL: $msg" -ForegroundColor Red; $script:fail = 1 }
function Ok($msg)   { Write-Host "ok:   $msg" }

$issPath = Join-Path $PSScriptRoot '..\rism.iss'
$iss = Get-Content $issPath -Raw
# Inno entries may continue across lines with a trailing backslash — flatten
# (backslash + newline -> space) so regexes can match a whole logical entry.
$flat = $iss -replace '\[\s]+', ' '

# 1. Required [Setup] keys
foreach ($k in 'AppId','AppName','AppVersion','DefaultDirName','UninstallDisplayName',
               'OutputBaseFilename','ChangesEnvironment','PrivilegesRequired') {
    if ($iss -match "(?m)^$k=") { Ok "[Setup] $k present" } else { Fail "[Setup] $k missing" }
}

# 2. AppId is a stable GUID form
if ($iss -match 'AppId=\{\{[0-9A-F-]+\}') { Ok 'AppId GUID form' } else { Fail 'AppId not a {{GUID} literal' }

# 3. Every referenced asset file exists (.iss paths are relative to installer/)
$inst = Split-Path $PSScriptRoot -Parent
foreach ($m in [regex]::Matches($iss, '(?m)^(?:SetupIconFile|LicenseFile|WizardImageFile|WizardSmallImageFile|Source)\s*=\s*([^;]+?)\s*(?:;|$)')) {
    $raw = $m.Groups[1].Value.Trim()
    if ($raw -match '\{') { Ok "asset $raw is macro-resolved at build"; continue }
    $abs = Join-Path $inst ($raw -replace '\\', '/')
    if (Test-Path $abs) { Ok "asset $raw exists" } else { Fail "asset $raw NOT FOUND ($abs)" }
}

# 4. PATH contract: [Code]-based (add on install, surgical remove on
#    uninstall) — mirrors Prism's Store-tested pattern. Inno [Registry]
#    round-trips with {olddata} corrupt *Path* values; never go back.
if ($iss -match 'procedure EnvAddPath')   { Ok 'EnvAddPath present' } else { Fail 'EnvAddPath missing' }
if ($iss -match 'procedure EnvRemovePath'){ Ok 'EnvRemovePath present' } else { Fail 'PATH removal on uninstall missing' }
if ($iss -match 'CurStepChanged')         { Ok 'install hook wired' } else { Fail 'ssPostInstall hook missing' }
if ($iss -match 'CurUninstallStepChanged'){ Ok 'uninstall hook wired' } else { Fail 'uninstall hook missing' }
if ($iss -match '(?m)Root: HKCU;.*"Path"|Root: HKLM;.*"Path"') { Fail 'PATH via [Registry] regressed (corrupts *Path* on uninstall)' } else { Ok 'no [Registry] PATH hack' }
if ($iss -match '(?m)^Name: "modifypath"')  { Ok 'modifypath task declared' } else { Fail 'modifypath task not declared' }

# 5. Silent-install contract used by windows-installer.yml (postinstall smoke)
if ($flat -match 'Filename: "\{app\}.*--version') { Ok 'postinstall --version verification' } else { Fail 'postinstall smoke run missing' }

# 6. No stray vendor leftovers
if ($iss -match 'vortexis|zyx') { Fail 'stale vendor reference in .iss' } else { Ok 'no stale vendor refs' }

# 6b. Flag-name trap: Inno has no 'checked' flag (only 'unchecked'); ISCC dies
#     with "unknown flag" on it.
if ($iss -match 'Flags:[^\r\n]*\bchecked\b') { Fail "'Flags: checked' is not an Inno flag (use default or 'unchecked')" } else { Ok 'no invalid Flags tokens' }

# 6c. Inside [Code], ';' starts NOTHING — it is Pascal's statement separator,
#     so ';'-prefixed "comment" lines abort ISCC with "'BEGIN' expected".
#     Legal comments there: { } always, // from Inno 6.3. Catch the trap.
$m = [regex]::Match($iss, '(?m)^\[Code\][\s\S]*$')
if ($m.Success) {
    $codeLines = $m.Value -split "\r?\n"
    $bad = $codeLines | Where-Object { $_ -match '^\s*;' }
    if ($bad) { Fail "';' pseudo-comments in [Code] (use braces): $($bad[0].Trim())" } else { Ok 'no ; pseudo-comments in [Code]' }
} else { Fail 'no [Code] section — PATH contract cannot hold' }

# 7. OutputBaseFilename matches what CI expects to find
if ($iss -match '(?m)^OutputBaseFilename=rism-\{#AppVersion\}-setup') { Ok 'OutputBaseFilename contract' } else { Fail 'OutputBaseFilename drifted from CI expectation' }

# 8. Install layout aligns with docs/windows-store.md promises
if ($iss -match '(?m)^PrivilegesRequired=admin')  { Ok 'per-machine (admin) install' } else { Fail 'PrivilegesRequired changed — update docs/windows-store.md' }
if ($iss -match '(?m)^DefaultDirName=\{autopf\}') { Ok 'installs to Program Files' } else { Fail 'DefaultDirName no longer {autopf} — docs stale' }

exit $fail
