# Start a native development preview against the existing Vite server. The
# executable is copied away from Cargo's output so final builds can continue.
param([switch]$Restart, [switch]$ReusePreviewShell)
$ErrorActionPreference = 'Stop'
$projectRoot = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
$previewDirectory = Join-Path $projectRoot 'build\dev-preview'
$previewExecutable = Join-Path $previewDirectory 'DouyinLiveMonitor.exe'
$existing = @(Get-CimInstance Win32_Process -Filter "Name='DouyinLiveMonitor.exe'" | Where-Object { $_.ExecutablePath -eq $previewExecutable })
if ($existing.Count -gt 0 -and -not $Restart) { throw 'The native development preview is already running; keep its window open.' }
$response = Invoke-WebRequest -Uri 'http://127.0.0.1:1420/' -UseBasicParsing -TimeoutSec 5
if ($response.StatusCode -ne 200) { throw 'Start the Vite server on port 1420 first.' }
Push-Location $projectRoot
try {
    . (Join-Path $PSScriptRoot 'rust-env.ps1')
    # Without custom-protocol, Tauri reads build.devUrl and receives Vite HMR.
    if ($ReusePreviewShell) {
        if (-not (Test-Path -LiteralPath $previewExecutable)) { throw 'No existing preview shell is available to reuse.' }
    } else {
        & cargo build --manifest-path src-tauri/Cargo.toml --release --locked --offline --no-default-features
        if ($LASTEXITCODE -ne 0) { throw 'Native development preview build failed.' }
    }
    if ($Restart) {
        # Restart only this workspace preview, after the new build is ready.
        # Normal WM_CLOSE goes through Rust's graceful shutdown/flush lifecycle.
        $running = @(Get-CimInstance Win32_Process -Filter "Name='DouyinLiveMonitor.exe'" | Where-Object { $_.ExecutablePath -eq $previewExecutable })
        foreach ($entry in $running) {
            $previous = Get-Process -Id $entry.ProcessId -ErrorAction Stop
            if (-not $previous.CloseMainWindow()) { throw 'Cannot request normal preview shutdown; the running preview was left untouched.' }
            if (-not $previous.WaitForExit(45000)) { throw 'Preview shutdown is still pending; refusing to kill the backend or overwrite the application.' }
        }
    }
    New-Item -ItemType Directory -Path $previewDirectory -Force | Out-Null
    if (-not $ReusePreviewShell) { Copy-Item -LiteralPath 'src-tauri\target\release\DouyinLiveMonitor.exe' -Destination $previewExecutable -Force }
    $engineDirectory = Join-Path $previewDirectory 'engine'
    New-Item -ItemType Directory -Path $engineDirectory -Force | Out-Null
    Get-ChildItem -LiteralPath 'dist\DouyinMonitorEngine' | Copy-Item -Destination $engineDirectory -Recurse -Force
    $oldData = $env:DOUYIN_MONITOR_HOME
    $oldWebViewData = $env:WEBVIEW2_USER_DATA_FOLDER
    try {
        $env:DOUYIN_MONITOR_HOME = $null
        $env:WEBVIEW2_USER_DATA_FOLDER = $null
        # The user explicitly requested an interactive, visible native preview.
        $process = Start-Process -FilePath $previewExecutable -WindowStyle Normal -PassThru
    } finally {
        $env:DOUYIN_MONITOR_HOME = $oldData
        $env:WEBVIEW2_USER_DATA_FOLDER = $oldWebViewData
    }
    $deadline = [DateTime]::UtcNow.AddSeconds(25)
    do {
        $process.Refresh()
        if ($process.HasExited) { throw 'Development window exited before it became ready.' }
        $children = @(Get-CimInstance Win32_Process -Filter "ParentProcessId=$($process.Id) AND Name='DouyinMonitorEngine.exe'")
        if ($process.MainWindowHandle -ne 0 -and $children.Count -eq 1) { break }
        Start-Sleep -Milliseconds 200
    } while ([DateTime]::UtcNow -lt $deadline)
    if ($process.MainWindowHandle -eq 0 -or $children.Count -ne 1) { throw 'The development window did not initialize its backend.' }
    $report = [ordered]@{ok=$true; native_process_id=$process.Id; backend_process_id=$children[0].ProcessId; window_handle=$process.MainWindowHandle.ToInt64(); frontend_url='http://127.0.0.1:1420/'; data_directory=(Join-Path $env:LOCALAPPDATA 'DouyinLiveMonitor'); executable=$previewExecutable; left_running_for_user=$true}
    $report | ConvertTo-Json | Set-Content -LiteralPath (Join-Path $previewDirectory 'launch-report.json') -Encoding utf8
    $report | ConvertTo-Json
} finally { Pop-Location }
