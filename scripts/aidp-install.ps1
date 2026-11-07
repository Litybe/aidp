# Library for bootstrap.ps1: installs the aidp binary and migrates the user PATH.
# Dot-source this file. It only defines functions and has no side effects.

function Install-AidpBinary {
    <#
    .SYNOPSIS
    Copies a built aidp binary into a stable install folder, atomically.
    .DESCRIPTION
    Copies Source to "<target>.new" inside InstallDir, then swaps it over the target with
    File.Replace (or File.Move when no target exists yet). When the swap fails, typically
    because the running exe is locked, the staging file is deleted, the original target is
    left untouched and an error containing "in use" is thrown. Returns the target path.
    #>
    param(
        [Parameter(Mandatory)][string]$Source,
        [Parameter(Mandatory)][string]$InstallDir
    )

    if (-not (Test-Path -LiteralPath $InstallDir)) {
        New-Item -ItemType Directory -Force -Path $InstallDir | Out-Null
    }
    $target = Join-Path $InstallDir (Split-Path -Leaf $Source)
    $staging = "$target.new"

    if (Test-Path -LiteralPath $staging) {
        [System.IO.File]::Delete($staging)
    }
    Copy-Item -LiteralPath $Source -Destination $staging

    try {
        if (Test-Path -LiteralPath $target) {
            [System.IO.File]::Replace($staging, $target, [NullString]::Value)
        } else {
            [System.IO.File]::Move($staging, $target)
        }
    } catch {
        if (Test-Path -LiteralPath $staging) {
            [System.IO.File]::Delete($staging)
        }
        throw "aidp is in use at ${target}: close every Claude Code session and terminal running aidp, then re-run /pipeline-setup-v7 ($($_.Exception.Message))"
    }
    return $target
}

function Get-AidpMigratedPath {
    <#
    .SYNOPSIS
    Returns a PATH string with InstallDir first and stale aidp entries removed.
    .DESCRIPTION
    Drops empty entries, InstallDir, every LegacyDirs element, and any entry ending in
    \rust\target\release or \rust\target\debug whose skill root holds a bootstrap.ps1.
    Comparison ignores case and trailing separators. Other entries keep their order and
    spelling. Applying the function to its own output returns the same string.
    #>
    param(
        [string]$UserPath,
        [string[]]$LegacyDirs,
        [Parameter(Mandatory)][string]$InstallDir
    )

    $normalise = { param([string]$Entry) $Entry.TrimEnd('\', '/').ToLowerInvariant() }
    $drop = @(& $normalise $InstallDir)
    foreach ($legacy in $LegacyDirs) {
        if ($legacy) { $drop += (& $normalise $legacy) }
    }

    $kept = @()
    foreach ($entry in ($UserPath -split ';')) {
        if (-not $entry) { continue }
        $normalised = & $normalise $entry
        if ($drop -contains $normalised) { continue }
        if ($normalised.EndsWith('\rust\target\release') -or $normalised.EndsWith('\rust\target\debug')) {
            $skillRoot = Join-Path $entry '..\..\..'
            if (Test-Path -LiteralPath (Join-Path $skillRoot 'bootstrap.ps1') -PathType Leaf) { continue }
        }
        $kept += $entry
    }
    return (@($InstallDir) + $kept) -join ';'
}
