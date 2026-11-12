param([Parameter(Mandatory=$true)][string]$Project, [string]$LogFile = '')
Add-Type -AssemblyName UIAutomationClient, UIAutomationTypes
Add-Type @"
using System;
using System.Runtime.InteropServices;
public class W {
  [DllImport("user32.dll")] public static extern bool ShowWindow(IntPtr h, int c);
  [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);
  [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
  [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr h, out uint p);
  [DllImport("user32.dll")] public static extern bool AttachThreadInput(uint a, uint b, bool f);
  [DllImport("user32.dll")] public static extern bool IsIconic(IntPtr h);
  [DllImport("user32.dll")] public static extern void keybd_event(byte k, byte s, uint f, UIntPtr e);
  [DllImport("kernel32.dll")] public static extern uint GetCurrentThreadId();
}
"@
$log = $LogFile
function Log($line) { if ($log) { Add-Content $log $line } }
if ($log) { New-Item -ItemType Directory -Force (Split-Path $log) | Out-Null }
$ts = Get-Date -Format 'yyyy-MM-dd HH:mm'
$procs = Get-CimInstance Win32_Process -Filter "Name='Unity.exe'" | Where-Object {
  $c = $_.CommandLine; $c -and $c.Contains($Project) -and ($c -notmatch '(?i)-batchmode')
}
if (-not $procs) { "ABSENT"; Log "- $ts | PID - | focus non | Unity absent"; exit }
$procId = [int](@($procs)[0].ProcessId)
$p = Get-Process -Id $procId
$h = $p.MainWindowHandle
"PID=$procId title=$($p.MainWindowTitle)"
if ($h -eq 0) { "NOWINDOW"; Log "- $ts | PID $procId | focus non | pas de fenetre principale"; exit }
# dialogs
$root = [System.Windows.Automation.AutomationElement]::RootElement
$cond = New-Object System.Windows.Automation.PropertyCondition([System.Windows.Automation.AutomationElement]::ProcessIdProperty, $procId)
$wins = $root.FindAll([System.Windows.Automation.TreeScope]::Children, $cond)
$notes = @()
foreach ($w in $wins) {
  if ($w.Current.NativeWindowHandle -eq [int]$h) { continue }
  $name = $w.Current.Name
  $btnCond = New-Object System.Windows.Automation.PropertyCondition([System.Windows.Automation.AutomationElement]::ControlTypeProperty, [System.Windows.Automation.ControlType]::Button)
  $btns = $w.FindAll([System.Windows.Automation.TreeScope]::Descendants, $btnCond)
  $bn = @(); foreach ($b in $btns) { $bn += $b.Current.Name }
  $txtCond = New-Object System.Windows.Automation.PropertyCondition([System.Windows.Automation.AutomationElement]::ControlTypeProperty, [System.Windows.Automation.ControlType]::Text)
  $tx = @(); foreach ($t in $w.FindAll([System.Windows.Automation.TreeScope]::Descendants, $txtCond)) { $tx += $t.Current.Name }
  $act = 'aucune'
  $joined = $bn -join '|'
  $pick = $null
  if ($bn -contains 'Reload') { $pick = 'Reload' } elseif ($bn -contains 'Save' -and $name -match 'Modified') { $pick = 'Save' }
  if ($pick) {
    foreach ($b in $btns) { if ($b.Current.Name -eq $pick) { ($b.GetCurrentPattern([System.Windows.Automation.InvokePattern]::Pattern)).Invoke(); $act = "invoke $pick"; break } }
  }
  "DIALOG title='$name' text='$($tx -join ' / ')' buttons=[$joined] action=$act"
  $notes += "dialogue '$name' [$joined] -> $act"
}
Add-Type @"
using System;
using System.Collections.Generic;
using System.Runtime.InteropServices;
using System.Text;
public class D {
  delegate bool EP(IntPtr h, IntPtr l);
  [DllImport("user32.dll")] static extern bool EnumWindows(EP f, IntPtr l);
  [DllImport("user32.dll")] static extern bool EnumChildWindows(IntPtr p, EP f, IntPtr l);
  [DllImport("user32.dll")] static extern bool IsWindowVisible(IntPtr h);
  [DllImport("user32.dll")] static extern uint GetWindowThreadProcessId(IntPtr h, out uint p);
  [DllImport("user32.dll", CharSet=CharSet.Unicode)] static extern int GetWindowText(IntPtr h, StringBuilder s, int n);
  [DllImport("user32.dll", CharSet=CharSet.Unicode)] static extern int GetClassName(IntPtr h, StringBuilder s, int n);
  [DllImport("user32.dll")] static extern IntPtr SendMessage(IntPtr h, uint m, IntPtr w, IntPtr l);
  static string T(IntPtr h){ var s=new StringBuilder(512); GetWindowText(h,s,512); return s.ToString(); }
  static string C(IntPtr h){ var s=new StringBuilder(256); GetClassName(h,s,256); return s.ToString(); }
  public static List<string> Run(uint pid, IntPtr main) {
    var res=new List<string>(); var tops=new List<IntPtr>();
    EnumWindows((h,l)=>{ uint p; GetWindowThreadProcessId(h,out p); if(p==pid && IsWindowVisible(h) && h!=main && C(h)=="#32770") tops.Add(h); return true;}, IntPtr.Zero);
    foreach(var d in tops){
      var btns=new List<IntPtr>(); var names=new List<string>(); var txt=new List<string>();
      EnumChildWindows(d,(c,l)=>{ var cl=C(c); var t=T(c); if(cl=="Button"){btns.Add(c);names.Add(t);} else if(cl=="Static"&&t!="") txt.Add(t); return true;}, IntPtr.Zero);
      string act="aucune"; IntPtr pick=IntPtr.Zero; string pn="";
      for(int i=0;i<btns.Count;i++){ var n=names[i].Replace("&",""); if(n=="Reload"){pick=btns[i];pn=n;break;} }
      if(pick==IntPtr.Zero && T(d).Contains("Modified")) for(int i=0;i<btns.Count;i++){ if(names[i].Replace("&","")=="Save"){pick=btns[i];pn="Save";break;} }
      if(pick!=IntPtr.Zero){ SendMessage(pick,0x00F5,IntPtr.Zero,IntPtr.Zero); act="BM_CLICK "+pn; }
      res.Add("title='"+T(d)+"' text='"+string.Join(" / ",txt)+"' buttons=["+string.Join("|",names)+"] action="+act);
    }
    return res;
  }
}
"@
foreach ($r in [D]::Run([uint32]$procId, $h)) { "WIN32DIALOG $r"; $notes += "win32 $r" }
Start-Sleep -Milliseconds 500
if ([W]::IsIconic($h)) { [void][W]::ShowWindow($h, 9) } else { [void][W]::ShowWindow($h, 5) }
[void][W]::SetForegroundWindow($h)
if ([W]::GetForegroundWindow() -ne $h) {
  $fg = [W]::GetForegroundWindow(); $d = [uint32]0
  $ft = [W]::GetWindowThreadProcessId($fg, [ref]$d); $me = [W]::GetCurrentThreadId()
  [void][W]::AttachThreadInput($me, $ft, $true)
  [void][W]::SetForegroundWindow($h)
  [void][W]::AttachThreadInput($me, $ft, $false)
}
if ([W]::GetForegroundWindow() -ne $h) {
  [W]::keybd_event(0x12, 0, 0, [UIntPtr]::Zero); [W]::keybd_event(0x12, 0, 2, [UIntPtr]::Zero)
  [void][W]::SetForegroundWindow($h)
}
Start-Sleep -Milliseconds 300
$ok = ([W]::GetForegroundWindow() -eq $h)
$fgd = [uint32]0; [void][W]::GetWindowThreadProcessId([W]::GetForegroundWindow(), [ref]$fgd)
if (-not $ok -and $fgd -eq $procId) { $ok = $true }  # dialog of Unity has focus
"FOCUS=$ok"
$nt = if ($notes) { $notes -join '; ' } else { 'aucun dialogue' }
Log "- $ts | PID $procId | focus $(if($ok){'ok'}else{'NON'}) | $nt"
