# Docs-vs-code drift gate (Rism). Verifies the documentation makes the same
# promises the code actually keeps. Exit codes: 0 OK | 1 drift detected.
$ErrorActionPreference = 'Stop'
$fail = 0
function Fail($msg) { Write-Host "FAIL: $msg" -ForegroundColor Red; $script:fail = 1 }
function Ok($msg)   { Write-Host "ok:   $msg" }

$root = $PSScriptRoot | Split-Path -Parent | Split-Path -Parent

$mcp = Get-Content (Join-Path $root 'src\mcp\mod.rs') -Raw
# Scan each #[tool( attribute up to the next async fn (descriptions contain
# nested parens, so count attributes and pair them positionally with fns).
$attrIdx = [regex]::Matches($mcp, '#\[tool\(') | ForEach-Object { $_.Index }
$fnIdx = @{}
foreach ($m in [regex]::Matches($mcp, 'async fn (\w+)\s*\(')) {
    if ($m.Groups[1].Value -notin @('new','list_tools','call_tool','get_info')) {
        $fnIdx[$m.Index] = $m.Groups[1].Value
    }
}
$tools = foreach ($i in $attrIdx) {
    $k = ($fnIdx.Keys | Where-Object { $_ -gt $i } | Sort-Object | Select-Object -First 1)
    $fnIdx[$k]
}
$tools = @($tools | Sort-Object -Unique)
$toolCount = $tools.Count
Ok "#[tool] definitions found: $toolCount"

# 1. Every docs page claiming a tool count must match the code
$claims = @()
foreach ($f in Get-ChildItem $root -Recurse -Include *.md -Exclude target) {
    $t = Get-Content $f.FullName -Raw
    # Count TOTAL-tool claims only ("serves 25 tools", "all 25 tools",
    # "**25 tools**") — not section counts like "Debugger (9 tools)".
    foreach ($m in [regex]::Matches($t, '(?<!\()\b(\d+)\s+tools\b')) {
        $claims += [pscustomobject]@{ File = $f.FullName.Substring($root.Length + 1); N = [int]$m.Groups[1].Value }
    }
}
if ($claims.Count -eq 0) { Fail 'no tool-count claim found in docs (did tool docs get deleted?)' }
foreach ($c in $claims) {
    if ($c.N -eq $toolCount) { Ok "$($c.File): claims $($c.N) — matches" }
    else { Fail "$($c.File): claims $($c.N), code defines $toolCount" }
}

# 2. Each registered tool name appears in docs/mcp-tools.md
$docmcp = Get-Content (Join-Path $root 'docs\mcp-tools.md') -Raw
foreach ($name in ($tools | Sort-Object -Unique)) {
    $tool = $name -replace '^tool_', ''
    # either a ### `name` section (single tools) or a `name` token in the
    # debugger/params tables — the contract is: the docs mention it as code.
    if ($docmcp -notmatch "``$tool``") { Fail "docs/mcp-tools.md never mentions $tool" } else { Ok "$tool documented" }
}

# 3. Config path claims agree with settings.rs (BaseDirs + rism/config.toml)
$settings = Get-Content (Join-Path $root 'src\settings.rs') -Raw
if ($settings -match 'BaseDirs::new\(\)[\s\S]{0,120}config_dir\(\)\.join\("rism"\)') {
    $cfg = Get-Content (Join-Path $root 'docs\configuration.md') -Raw
    if ($cfg -match '%APPDATA%') { Ok 'Windows config path documented (%APPDATA%)' } else { Fail 'configuration.md lost the %APPDATA% row' }
    if ($cfg -match '\.config/rism/config\.toml' ) { Ok 'Linux config path documented' } else { Fail 'configuration.md lost the Linux path' }
    # the password-skipped promise must hold in code
    if ($settings -match '#\[serde\(skip\)\][\s\S]{0,80}pub iris_password') { Ok 'password is serde-skipped, as docs promise' } else { Fail 'docs promise password-skipped but code changed' }
} else { Fail 'settings.rs config path scheme changed — recheck docs/configuration.md' }

# 4. Version parity: Cargo.toml vs docs/CHANGELOG.md top entry
$cargoVer = (Select-String -Path (Join-Path $root 'Cargo.toml') -Pattern '^version\s*=\s*"([^"]+)"').Matches[0].Groups[1].Value
$chlogPath = Join-Path $root 'docs\CHANGELOG.md'
if (Test-Path $chlogPath) {
    $head = (Get-Content $chlogPath -TotalCount 8 | Select-String '## \[?v?([0-9][^\] ]*)' | Select-Object -First 1)
    if ($head) {
        $top = $head.Matches[0].Groups[1].Value
        if ($top -eq $cargoVer) { Ok "CHANGELOG top $top == Cargo.toml $cargoVer" }
        else { Fail "CHANGELOG top $top != Cargo.toml $cargoVer" }
    } else { Fail 'CHANGELOG has no version heading' }
} else { Fail 'docs/CHANGELOG.md missing (mkdocs nav references it)' }

exit $fail
