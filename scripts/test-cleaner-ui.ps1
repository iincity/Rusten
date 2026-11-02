param(
    [switch]$DependencyCheckOnly,
    [switch]$SkipBuild,
    [string]$EvidenceDir = 'target/ui-acceptance'
)

$ErrorActionPreference = 'Stop'
Set-Location (Split-Path $PSScriptRoot -Parent)
$tree = cargo tree -p cleaner-ui --depth 1
if ($LASTEXITCODE -ne 0) { throw 'cargo tree failed' }
if ($tree -match '(?m)^[^\r\n]*(gpui|gpui-component) v') {
    throw 'cleaner-ui still depends on GPUI'
}
if ($DependencyCheckOnly) { Write-Output 'PASS: cleaner-ui has no direct GPUI dependencies'; exit 0 }
New-Item -ItemType Directory -Force $EvidenceDir | Out-Null
function Invoke-CargoCheck([string]$LogName, [string[]]$CargoArgs) {
    & cargo @CargoArgs *>&1 | Tee-Object (Join-Path $EvidenceDir $LogName)
    if ($LASTEXITCODE -ne 0) { throw "cargo $($CargoArgs -join ' ') failed with exit $LASTEXITCODE" }
}
Invoke-CargoCheck 'format.log' @('fmt', '--all', '--', '--check')
Invoke-CargoCheck 'fixture-guard.log' @('test', '-p', 'cleaner-ui', '--test', 'fixture', '-j', '1')
foreach ($suite in @('input', 'navigation', 'access')) {
    Invoke-CargoCheck "platform-$suite.log" @('test', '-p', 'nsh-platform-windows', '--lib', "${suite}::tests", '-j', '1')
}
Invoke-CargoCheck 'workspace-tests.log' @('test', '--workspace', '-j', '1', '--', '--test-threads=1')
# Check owned workspace crates; the patched platform has separate regression suites above.
Invoke-CargoCheck 'clippy.log' @('clippy', '--workspace', '--all-targets', '--no-deps', '-j', '1', '--', '-D', 'warnings')
if (-not $SkipBuild) {
    Invoke-CargoCheck 'debug-build.log' @('build', '-p', 'cleaner-ui', '-j', '1')
    Invoke-CargoCheck 'release-build.log' @('build', '-p', 'cleaner-ui', '--release', '-j', '1')
}
Invoke-CargoCheck 'layout-tests.log' @('test', '-p', 'cleaner-ui', '--test', 'layout', '-j', '1', '--', '--nocapture')
Invoke-CargoCheck 'ui-tests.log' @('test', '-p', 'cleaner-ui', '--test', 'ui', '-j', '1', '--', '--nocapture', '--test-threads=1')
$hashes = @('target/debug/cleaner-ui.exe', 'target/release/cleaner-ui.exe') | ForEach-Object { Get-FileHash -LiteralPath $_ -Algorithm SHA256 }
$hashes | ConvertTo-Json | Set-Content -LiteralPath (Join-Path $EvidenceDir 'executables.json')
$hashes | Format-Table Path, Hash
Write-Output 'Native acceptance remains separate: record screenshots and interactions against these hashes.'
