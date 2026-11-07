pub const TASKKILL: &str = "taskkill";
pub const TASKKILL_FLAGS: [&str; 3] = ["/T", "/F", "/PID"];
pub const POWERSHELL: &str = "powershell";
pub const POWERSHELL_FLAGS: [&str; 3] = ["-NoProfile", "-NonInteractive", "-Command"];
pub const HIDDEN_WINDOW: u32 = 0x0800_0000;
pub const TREE_UNCONFIRMED: &str = "owned process tree termination could not be confirmed";
pub const WINDOWS_DESCENDANTS: &str = r#"& { param([int]$ownedRootPid)
$ErrorActionPreference='Stop'
try {
    $all=@(Get-CimInstance Win32_Process)
    $owned=@()
    $parents=@($ownedRootPid)
    while ($parents.Count -gt 0) {
        $next=@($all | Where-Object { $parents -contains [int]$_.ParentProcessId })
        $owned += $next
        $parents=@($next | ForEach-Object { [int]$_.ProcessId })
        $all=@($all | Where-Object { $parents -notcontains [int]$_.ProcessId })
    }
    foreach ($entry in $owned) {
        $process=$null
        try {
            try { $process=[System.Diagnostics.Process]::GetProcessById([int]$entry.ProcessId) }
            catch [System.ArgumentException] { continue }
            $null=$process.SafeHandle
            if ([Math]::Abs(($process.StartTime.ToUniversalTime()-$entry.CreationDate.ToUniversalTime()).TotalMilliseconds) -gt 1) { continue }
            if (!$process.HasExited) {
                & taskkill /T /F /PID $process.Id *> $null
                if ($LASTEXITCODE -ne 0 -and !$process.HasExited) { throw 'owned descendant could not be terminated' }
                if (!$process.WaitForExit(5000)) { throw 'owned descendant exit was not confirmed' }
            }
        } finally { if ($null -ne $process) { $process.Dispose() } }
    }
    exit 0
} catch { exit 1 }
} "#;
