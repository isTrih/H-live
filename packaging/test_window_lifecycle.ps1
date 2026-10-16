# Briefly opens only the newly built workspace application. No monitor is started.
param([string]$Executable = '')
$ErrorActionPreference = 'Stop'
$projectRoot = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
if (-not $Executable) { $Executable = Join-Path $projectRoot 'src-tauri\target\release\douyin-live-monitor.exe' }
$Executable = (Resolve-Path -LiteralPath $Executable).Path
if (-not $Executable.StartsWith($projectRoot + '\', [StringComparison]::OrdinalIgnoreCase)) { throw 'Lifecycle test is restricted to workspace builds.' }
$existing = @(Get-CimInstance Win32_Process -Filter "Name='DouyinLiveMonitor.exe' OR Name='douyin-live-monitor.exe'")
if ($existing.Count -gt 0) { throw 'A native application is already running. Lifecycle validation will not disturb the user preview.' }
$testDirectory = Join-Path $projectRoot 'build\window-lifecycle'
New-Item -ItemType Directory -Path $testDirectory -Force | Out-Null
Add-Type @'
using System;
using System.Runtime.InteropServices;
public static class WindowLifecycleNative {
  [DllImport("user32.dll")] public static extern bool ShowWindow(IntPtr hwnd, int command);
  [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
  [DllImport("user32.dll")] public static extern bool IsIconic(IntPtr hwnd);
  [DllImport("user32.dll")] public static extern IntPtr GetWindowDpiAwarenessContext(IntPtr hwnd);
  [DllImport("user32.dll")] public static extern bool AreDpiAwarenessContextsEqual(IntPtr first, IntPtr second);
  [DllImport("user32.dll")] public static extern uint GetDpiForWindow(IntPtr hwnd);
}
'@
$oldTestHome = $env:DOUYIN_MONITOR_HOME
$oldWebViewData = $env:WEBVIEW2_USER_DATA_FOLDER
$first = $null
$second = $null
$third = $null
$report = [ordered]@{ok=$false; desktop_opened=$false; webview_process_started=$false; ui_started_backend=$false; per_monitor_v2_dpi_aware=$false; window_dpi=0; second_process_exited=$false; single_backend=$false; existing_window_restored=$false; existing_window_focused=$false; graceful_shutdown=$false; job_cleanup_on_parent_exit=$false}
function Get-BackendChild([int]$ParentId) {
    @(Get-CimInstance Win32_Process -Filter "ParentProcessId=$ParentId AND Name='DouyinMonitorEngine.exe'")
}
function Wait-Application([Diagnostics.Process]$Process) {
    $deadline = [DateTime]::UtcNow.AddSeconds(20)
    while ([DateTime]::UtcNow -lt $deadline) {
        $Process.Refresh()
        if ($Process.HasExited) { throw 'Desktop exited before loading the UI.' }
        $children = @(Get-BackendChild $Process.Id)
        if ($Process.MainWindowHandle -ne 0 -and $children.Count -eq 1) { return $children[0].ProcessId }
        Start-Sleep -Milliseconds 200
    }
    throw 'WebView did not initialize the backend within 20 seconds.'
}
try {
    $env:DOUYIN_MONITOR_HOME = Join-Path $testDirectory 'data'
    $env:WEBVIEW2_USER_DATA_FOLDER = Join-Path $testDirectory 'webview-cache'
    $first = Start-Process -FilePath $Executable -WindowStyle Hidden -PassThru
    $backendId = Wait-Application $first
    $report.desktop_opened = $true
    $report.ui_started_backend = $true
    $dpiContext = [WindowLifecycleNative]::GetWindowDpiAwarenessContext($first.MainWindowHandle)
    $report.per_monitor_v2_dpi_aware = [WindowLifecycleNative]::AreDpiAwarenessContextsEqual($dpiContext, [IntPtr](-4))
    $report.window_dpi = [WindowLifecycleNative]::GetDpiForWindow($first.MainWindowHandle)
    $report.webview_process_started = @(Get-CimInstance Win32_Process -Filter "ParentProcessId=$($first.Id) AND Name='msedgewebview2.exe'").Count -gt 0
    [WindowLifecycleNative]::ShowWindow($first.MainWindowHandle, 6) | Out-Null
    $second = Start-Process -FilePath $Executable -WindowStyle Hidden -PassThru
    $report.second_process_exited = $second.WaitForExit(5000)
    Start-Sleep -Milliseconds 300
    $report.single_backend = @(Get-BackendChild $first.Id).Count -eq 1 -and @(Get-BackendChild $second.Id).Count -eq 0
    $report.existing_window_restored = -not [WindowLifecycleNative]::IsIconic($first.MainWindowHandle)
    $report.existing_window_focused = [WindowLifecycleNative]::GetForegroundWindow() -eq $first.MainWindowHandle
    if (-not $first.CloseMainWindow()) { throw 'Could not send a normal close to the test window.' }
    $report.graceful_shutdown = $first.WaitForExit(40000) -and -not (Get-Process -Id $backendId -ErrorAction SilentlyContinue)
    $third = Start-Process -FilePath $Executable -WindowStyle Hidden -PassThru
    $crashBackendId = Wait-Application $third
    $third.Kill()
    $third.WaitForExit(5000) | Out-Null
    $deadline = [DateTime]::UtcNow.AddSeconds(5)
    while ((Get-Process -Id $crashBackendId -ErrorAction SilentlyContinue) -and [DateTime]::UtcNow -lt $deadline) { Start-Sleep -Milliseconds 100 }
    $report.job_cleanup_on_parent_exit = -not (Get-Process -Id $crashBackendId -ErrorAction SilentlyContinue)
    $report.ok = @($report.Values | Where-Object { $_ -eq $false }).Count -eq 1
    if (-not $report.ok) { throw 'One or more lifecycle checks failed; inspect report.json.' }
} catch {
    $report.error = $_.Exception.Message
    throw
} finally {
    foreach ($process in @($first,$second,$third)) {
        if ($process -and -not $process.HasExited) { $process.Kill(); $process.WaitForExit(5000) | Out-Null }
    }
    $env:DOUYIN_MONITOR_HOME = $oldTestHome
    $env:WEBVIEW2_USER_DATA_FOLDER = $oldWebViewData
    $report | ConvertTo-Json | Set-Content -LiteralPath (Join-Path $testDirectory 'report.json') -Encoding utf8
    $report | ConvertTo-Json
}
