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

# 4. PATH contract. Inno entries may continue across lines with a trailing
#    backslash — flatten (backslash + newline -> space) before matching.
$flat = $iss -replace '\\(\r?\n)', ' '
if ($flat -match '(?m)^Root: HKCU;.*"Path"') { Ok 'HKCU PATH entry' } else { Fail 'HKCU PATH entry missing' }
if ($flat -match '(?m)^Root: HKLM;.*"Path"') { Ok 'HKLM PATH entry' } else { Fail 'HKLM PATH entry missing' }
if ($flat -match '(?m)Tasks: modifypath')     { Ok 'modifypath task wired to entries' } else { Fail 'PATH entries not bound to modifypath task' }
if ($iss  -match '(?m)^Name: "modifypath"')  { Ok 'modifypath task declared' } else { Fail 'modifypath task not declared' }

# 5. Silent-install contract used by windows-installer.yml (postinstall smoke)
if ($flat -match 'Filename: "\{app\}.*--version') { Ok 'postinstall --version verification' } else { Fail 'postinstall smoke run missing' }

# 6. No stray vendor leftovers
if ($iss -match 'vortexis|zyx') { Fail 'stale vendor reference in .iss' } else { Ok 'no stale vendor refs' }

# 7. OutputBaseFilename matches what CI expects to find
if ($iss -match '(?m)^OutputBaseFilename=rism-\{#AppVersion\}-setup') { Ok 'OutputBaseFilename contract' } else { Fail 'OutputBaseFilename drifted from CI expectation' }

# 8. Install layout aligns with docs/windows-store.md promises
if ($iss -match '(?m)^PrivilegesRequired=admin')  { Ok 'per-machine (admin) install' } else { Fail 'PrivilegesRequired changed — update docs/windows-store.md' }
if ($iss -match '(?m)^DefaultDirName=\{autopf\}') { Ok 'installs to Program Files' } else { Fail 'DefaultDirName no longer {autopf} — docs stale' }

exit $fail
