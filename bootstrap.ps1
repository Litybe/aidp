# bootstrap.ps1 — build and install aidp (Windows)
# Build aidp, install it into a stable folder (~/.aidp/bin, override with AIDP_INSTALL_DIR) and
# put that folder first on the user PATH. The skill's setup script runs this, then installs hooks.
# Idempotent: safe to re-run.

$ErrorActionPreference = 'Stop'
$repoRoot = Split-Path -Parent $MyInvocation.MyCommand.Path
$releaseDir = Join-Path $repoRoot 'target\release'
$builtAidp = Join-Path $releaseDir 'aidp.exe'
$installDir = if ($env:AIDP_INSTALL_DIR) { $env:AIDP_INSTALL_DIR } else { Join-Path $HOME '.aidp\bin' }

. (Join-Path $repoRoot 'scripts\aidp-install.ps1')

function Write-Section($title) {
    Write-Host ''
    Write-Host "=== $title ===" -ForegroundColor Cyan
}

function Write-OK($msg)   { Write-Host "  OK    $msg" -ForegroundColor Green }
function Write-Skip($msg) { Write-Host "  SKIP  $msg" -ForegroundColor DarkGray }
function Write-Fail($msg) { Write-Host "  FAIL  $msg" -ForegroundColor Red }

# 1. Check cargo
Write-Section '1/3  Toolchain check'
$cargo = Get-Command cargo -ErrorAction SilentlyContinue
if (-not $cargo) {
    Write-Fail 'cargo not found. Install Rust from https://rustup.rs and re-run this script.'
    exit 1
}
Write-OK "cargo at $($cargo.Source)"

# 2. Build and install aidp
Write-Section '2/3  Build and install aidp'
Push-Location $repoRoot
try {
    & cargo build --release -p aidp-cli
    if ($LASTEXITCODE -ne 0) {
        Write-Fail "cargo build failed (exit $LASTEXITCODE)"
        exit 1
    }
} finally {
    Pop-Location
}
if (-not (Test-Path $builtAidp)) {
    Write-Fail "build succeeded but $builtAidp not found"
    exit 1
}
Write-OK "binary built at $builtAidp"
try {
    $aidp = Install-AidpBinary -Source $builtAidp -InstallDir $installDir
} catch {
    Write-Fail $_.Exception.Message
    exit 1
}
Write-OK "installed $aidp (copied from $builtAidp)"

# 3. Put the install dir on user PATH
Write-Section '3/3  PATH'
$userPath = [Environment]::GetEnvironmentVariable('PATH', 'User')
if ($null -eq $userPath) { $userPath = '' }
$newPath = Get-AidpMigratedPath -UserPath $userPath -LegacyDirs @($releaseDir) -InstallDir $installDir
if ($newPath -ne $userPath) {
    [Environment]::SetEnvironmentVariable('PATH', $newPath, 'User')
    Write-OK "$installDir is first on user PATH (any old target\release entry was removed)"
} else {
    Write-Skip "$installDir already first on user PATH"
}
$env:PATH = Get-AidpMigratedPath -UserPath $env:PATH -LegacyDirs @($releaseDir) -InstallDir $installDir

Write-Host ''
& $aidp --version
Write-Host 'aidp installed. Set up the skill with /pipeline-setup-v7, or open a new terminal to use aidp.' -ForegroundColor Green
exit 0
