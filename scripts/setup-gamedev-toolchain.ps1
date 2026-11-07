# setup-gamedev-toolchain.ps1 - installe la toolchain de build d'aidp sur D: uniquement.
# Rust en toolchain GNU (pas de Visual Studio Build Tools ni de Windows SDK, qui s'installent sur C:),
# WinLibs MinGW-w64 portable (compilateur C de la crate ring) et cargo-nextest. Pose les variables
# d'environnement utilisateur qui gardent rustup, cargo, les builds, le daemon et npm hors de C:.
# Relançable : chaque étape déjà faite est sautée.
# Usage : powershell -NoProfile -ExecutionPolicy Bypass -File scripts\setup-gamedev-toolchain.ps1

param(
    [string]$DevTools = 'D:\DevTools',
    [string]$AidpRoot = 'D:\AIDP'
)

$ErrorActionPreference = 'Stop'

$hostTriple = 'x86_64-pc-windows-gnu'
$rustupUrl = "https://static.rust-lang.org/rustup/dist/$hostTriple/rustup-init.exe"
$winlibsUrl = 'https://github.com/brechtsanders/winlibs_mingw/releases/download/16.2.0posix-23.1.3-14.0.0-msvcrt-r3/winlibs-x86_64-posix-seh-gcc-16.2.0-mingw-w64msvcrt-14.0.0-r3.zip'
$nextestUrl = 'https://get.nexte.st/latest/windows'

$rustupHome = Join-Path $DevTools 'rust\rustup'
$cargoHome = Join-Path $DevTools 'rust\cargo'
$cargoBin = Join-Path $cargoHome 'bin'
$winlibsDir = Join-Path $DevTools 'winlibs'
$winlibsBin = Join-Path $winlibsDir 'mingw64\bin'
$downloads = Join-Path $DevTools '_downloads'
# tar de Windows (bsdtar) : celui de Git Bash, s'il passe avant dans le PATH, lit D: comme un hôte.
$tar = Join-Path $env:SystemRoot 'System32\tar.exe'
$repoRoot = Split-Path -Parent (Split-Path -Parent $MyInvocation.MyCommand.Path)

$userVars = [ordered]@{
    RUSTUP_HOME      = $rustupHome
    CARGO_HOME       = $cargoHome
    CARGO_TARGET_DIR = Join-Path $AidpRoot 'target'
    AIDP_SRC         = Join-Path $AidpRoot 'src\aidp'
    AIDP_INSTALL_DIR = Join-Path $AidpRoot 'bin'
    AIDP_GATEWAY_DIR = Join-Path $AidpRoot 'gateway'
    npm_config_cache = Join-Path $DevTools 'npm-cache'
}

function Write-Section($title) { Write-Host ''; Write-Host "=== $title ===" -ForegroundColor Cyan }
function Write-OK($msg)   { Write-Host "  OK    $msg" -ForegroundColor Green }
function Write-Skip($msg) { Write-Host "  SKIP  $msg" -ForegroundColor DarkGray }

# Refuse tout chemin cible sur C: : ce disque ne reçoit aucune installation.
function Assert-NotOnC([string]$path) {
    $root = [System.IO.Path]::GetPathRoot([System.IO.Path]::GetFullPath($path))
    if ($root -ieq 'C:\') { throw "Refus : $path est sur C:." }
}

# Télécharge une URL dans le dossier de téléchargements de D:, sauf si le fichier y est déjà.
function Get-Download([string]$url, [string]$fileName) {
    $target = Join-Path $downloads $fileName
    if (Test-Path $target) { Write-Skip "$fileName déjà téléchargé"; return $target }
    & curl.exe -fsSL --retry 3 -o $target $url
    if ($LASTEXITCODE -ne 0) { throw "Téléchargement échoué : $url (curl $LASTEXITCODE)" }
    Write-OK "$fileName téléchargé"
    return $target
}

# Ajoute un dossier en tête du PATH utilisateur s'il n'y est pas déjà.
function Add-UserPath([string]$dir) {
    $current = [Environment]::GetEnvironmentVariable('Path', 'User')
    $entries = @($current -split ';' | Where-Object { $_ -ne '' })
    if ($entries -contains $dir) { Write-Skip "$dir déjà dans le PATH utilisateur"; return }
    [Environment]::SetEnvironmentVariable('Path', (@($dir) + $entries) -join ';', 'User')
    Write-OK "$dir ajouté au PATH utilisateur"
}

Write-Section '1/6  Dossiers'
foreach ($dir in @($DevTools, $downloads, $rustupHome, $cargoHome) + @($userVars.Values)) {
    Assert-NotOnC $dir
    New-Item -ItemType Directory -Force -Path $dir | Out-Null
}
Write-OK "dossiers prêts sous $DevTools et $AidpRoot"

Write-Section '2/6  Variables d''environnement utilisateur'
foreach ($name in $userVars.Keys) {
    $value = $userVars[$name]
    if ([Environment]::GetEnvironmentVariable($name, 'User') -eq $value) { Write-Skip "$name = $value" }
    else { [Environment]::SetEnvironmentVariable($name, $value, 'User'); Write-OK "$name = $value" }
    Set-Item -Path "env:$name" -Value $value
}
Add-UserPath $winlibsBin
Add-UserPath $cargoBin
$env:Path = "$cargoBin;$winlibsBin;$env:Path"

Write-Section '3/6  WinLibs MinGW-w64 (gcc pour ring)'
if (Test-Path (Join-Path $winlibsBin 'gcc.exe')) { Write-Skip "gcc déjà dans $winlibsBin" }
else {
    $zip = Get-Download $winlibsUrl 'winlibs-mingw64-msvcrt.zip'
    New-Item -ItemType Directory -Force -Path $winlibsDir | Out-Null
    & $tar -xf $zip -C $winlibsDir
    if ($LASTEXITCODE -ne 0) { throw "Extraction de WinLibs échouée (tar $LASTEXITCODE)" }
    Write-OK "WinLibs extrait dans $winlibsDir"
}
& (Join-Path $winlibsBin 'gcc.exe') --version | Select-Object -First 1

Write-Section '4/6  rustup (hôte GNU)'
$rustup = Join-Path $cargoBin 'rustup.exe'
if (Test-Path $rustup) { Write-Skip "rustup déjà dans $cargoBin" }
else {
    $init = Get-Download $rustupUrl 'rustup-init.exe'
    & $init -y --no-modify-path --profile minimal --default-host $hostTriple --default-toolchain none
    if ($LASTEXITCODE -ne 0) { throw "rustup-init a échoué ($LASTEXITCODE)" }
    Write-OK "rustup installé dans $cargoHome"
}

Write-Section '5/6  Toolchain du dépôt (rust-toolchain.toml)'
Push-Location $repoRoot
try {
    & $rustup toolchain install
    if ($LASTEXITCODE -ne 0) { throw "Installation de la toolchain échouée ($LASTEXITCODE)" }
    # La toolchain du dépôt devient aussi celle par défaut : cargo marche hors du dépôt.
    $active = ((& $rustup show active-toolchain) -split ' ')[0]
    & $rustup default $active
    & rustc -vV | Select-String '^(release|host):'
    & cargo --version
} finally { Pop-Location }

Write-Section '6/6  cargo-nextest'
$nextest = Join-Path $cargoBin 'cargo-nextest.exe'
if (Test-Path $nextest) { Write-Skip "cargo-nextest déjà dans $cargoBin" }
else {
    $zip = Get-Download $nextestUrl 'cargo-nextest-windows.zip'
    & $tar -xf $zip -C $cargoBin
    if ($LASTEXITCODE -ne 0) { throw "Extraction de cargo-nextest échouée (tar $LASTEXITCODE)" }
    Write-OK "cargo-nextest extrait dans $cargoBin"
}
& cargo nextest --version | Select-Object -First 1

Write-Host ''
Write-Host 'Toolchain prête, tout sur D:. Ouvre un nouveau terminal pour que le PATH et les variables soient pris en compte.' -ForegroundColor Green
