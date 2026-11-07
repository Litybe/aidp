param([string]$ChildFile)
[System.IO.File]::WriteAllText($ChildFile,[string]$PID)
Start-Sleep -Seconds 120
