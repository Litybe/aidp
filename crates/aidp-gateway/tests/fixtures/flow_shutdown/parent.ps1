param([string]$RootFile,[string]$ChildFile,[string]$ChildScript,[switch]$ExitParent)
[System.IO.File]::WriteAllText($RootFile,[string]$PID)
$child=Start-Process powershell -ArgumentList @('-NoProfile','-NonInteractive','-ExecutionPolicy','Bypass','-File',('"'+$ChildScript+'"'),('"'+$ChildFile+'"')) -PassThru -WindowStyle Hidden
while (!(Test-Path -LiteralPath $ChildFile)) { Start-Sleep -Milliseconds 25 }
if ($ExitParent) { exit 0 }
Start-Sleep -Seconds 120
