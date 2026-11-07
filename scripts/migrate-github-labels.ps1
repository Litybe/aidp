<#
.SYNOPSIS
Migrates the labels of a GitHub repository to the aidp vocabulary (plan gamedev section 3.1).
.DESCRIPTION
Runs `aidp gh labels-migrate`. By default it is a DRY RUN: it only reads GitHub (gh label list,
gh issue list, gh pr list) and prints what it would do. Nothing is written without -Apply.

Planned changes:
  - create the missing labels: the eleven aidp:* state labels (triage, ready, queued,
    in-progress, review, fixing, testing, awaiting-merge, done, blocked, skip), the route labels
    code, art, map, doc, and prio:high, prio:low;
  - aidp:need-fix becomes aidp:fixing on every issue that carries it;
  - aidp:done on an open issue whose open pull request closes it becomes aidp:awaiting-merge.
The obsolete label aidp:need-fix itself is left on the repository (delete it by hand).
.PARAMETER Repo
Repository as owner/name.
.PARAMETER Apply
Write the changes on GitHub. Without it nothing is written.
.PARAMETER Aidp
Path of the aidp binary. Default: aidp from the PATH.
.EXAMPLE
.\migrate-github-labels.ps1 -Repo Litybe/PixelHood
.\migrate-github-labels.ps1 -Repo Litybe/PixelHood -Apply
#>
[CmdletBinding()]
param(
    [Parameter(Mandatory)][string]$Repo,
    [switch]$Apply,
    [string]$Aidp = 'aidp'
)

$ErrorActionPreference = 'Stop'

$arguments = @('gh', 'labels-migrate', '--repo', $Repo)
if ($Apply) {
    $arguments += '--apply'
    Write-Host "APPLY on $Repo : labels will be created and issues relabelled."
} else {
    Write-Host "DRY RUN on $Repo : nothing is written. Re-run with -Apply to write."
}

$json = & $Aidp @arguments
if ($LASTEXITCODE -ne 0) {
    throw "aidp gh labels-migrate failed with exit code $LASTEXITCODE"
}

$plan = $json | ConvertFrom-Json
Write-Host ''
Write-Host ("Labels to create : {0}" -f $plan.create.Count)
foreach ($label in $plan.create) { Write-Host ("  + {0}" -f $label.name) }
Write-Host ("Issues to relabel : {0}" -f $plan.relabel.Count)
foreach ($change in $plan.relabel) {
    Write-Host ("  #{0} {1} : {2}" -f $change.number, $change.title, $change.reason)
}
if ($plan.obsolete.Count -gt 0) {
    Write-Host ("Obsolete, left in place : {0}" -f ($plan.obsolete -join ', '))
}
if ($plan.errors.Count -gt 0) {
    Write-Host 'Errors:'
    foreach ($e in $plan.errors) { Write-Host "  $e" }
    exit 1
}
if ($plan.applied) { Write-Host 'Applied.' } else { Write-Host 'Dry run only.' }
