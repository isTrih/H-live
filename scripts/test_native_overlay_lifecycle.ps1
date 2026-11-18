param(
    [string]$Executable = '',
    [ValidateRange(1, 100)][int]$Rounds = 12,
    [string]$RunRoot = ''
)
# Real WebView2/IPC lifecycle regression, using an isolated backend and the
# existing explicit --replay-diagnostic app. No real community key is read.
# The placeholder media URL may fail immediately: decoded video and network
# availability are deliberately not prerequisites for creating a DOM overlay.
# Existing diagnostic setup gives its main WebView a custom profile; the child
# follows the production native_replay_open builder and its profile behavior.
$ErrorActionPreference = 'Stop'
$projectRoot = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
$buildRoot = [IO.Path]::GetFullPath((Join-Path $projectRoot 'build'))
if (-not $RunRoot) {
    $RunRoot = Join-Path $buildRoot ('overlay-lifecycle-' + [DateTime]::Now.ToString('yyyyMMdd-HHmmssfff') + '-' + [Guid]::NewGuid().ToString('N').Substring(0, 8))
} elseif (-not [IO.Path]::IsPathRooted($RunRoot)) {
    $RunRoot = Join-Path $projectRoot $RunRoot
}
$RunRoot = [IO.Path]::GetFullPath($RunRoot)
if (-not $RunRoot.StartsWith($buildRoot + [IO.Path]::DirectorySeparatorChar, [StringComparison]::OrdinalIgnoreCase)) {
    throw 'RunRoot must be a dedicated directory inside this project build directory.'
}
$reportPath = Join-Path $RunRoot 'report.json'
if (Test-Path -LiteralPath $RunRoot) {
    throw 'RunRoot already exists; preserving its diagnostic artifacts. Choose a fresh directory.'
}
if (-not $Executable) { $Executable = Join-Path $projectRoot 'dist\DouyinLiveMonitor\DouyinLiveMonitor.exe' }
$Executable = (Resolve-Path -LiteralPath $Executable).Path
if (-not [IO.File]::Exists($Executable)) { throw 'Diagnostic executable not found.' }

$scriptDirectory = Join-Path $RunRoot 'replay-diagnostic'
New-Item -ItemType Directory -Path $scriptDirectory -Force | Out-Null
$script = @'
// first_frame is the existing diagnostic runner's exit-code oracle ONLY.
// This test maps it to overlay_success; it does not claim decoded video.
(() => {
  const invoke = (command, args) => window.__TAURI_INTERNALS__.invoke(command, args);
  const roundLimit = __ROUND_LIMIT__;
  const report = {format:1, scenario:"native-overlay-lifecycle", requested_rounds:roundLimit,
    network_required:false, community_api_called:false, decoder_verified:false,
    first_frame_means:"overlay_lifecycle_passed", first_frame:false, overlay_success:false, rounds:[]};
  const sanitize = error => String(error?.message || error || "unknown_error")
    .replace(/https?:\/\/\S+/g,"<media-url>").replace(/Bearer\s+\S+/gi,"Bearer <redacted>").slice(0,500);
  const visible = {x:0,y:72,width:640,height:360};
  const hidden = {x:0,y:72,width:0,height:0};
  let finished = false;
  const finish = async error => {
    if (finished) return;
    finished = true;
    clearTimeout(deadline);
    if (error) report.error = sanitize(error);
    report.overlay_success = !error && report.rounds.length === roundLimit && report.rounds.every(row => row.passed);
    report.first_frame = report.overlay_success;
    await invoke("replay_diagnostic_finish", {report});
  };
  const deadline = setTimeout(() => void finish("overlay_lifecycle_timeout"), 38000);
  const status = value => ({active:value?.active === true,
    overlay_native_windows:Number(value?.overlay_native_windows || 0),
    layer_order_valid:value?.layer_order_valid === true});
  const assertOverlay = (value, label) => {
    if (!value.active || value.overlay_native_windows < 1) throw Error(label + ": overlay native window missing");
  };
  const run = async () => {
    try {
      for (let round = 0; round < roundLimit; round++) {
        if (finished) return;
        const row = {round, initially_hidden:round % 3 === 1, passed:false};
        report.rounds.push(row);
        try {
          const opened = await invoke("native_replay_open", {
            url:"https://v26-star.xingtu.cn/__qingkan_overlay_fixture__.m3u8",
            bounds:row.initially_hidden ? hidden : visible, startTime:0, paused:true, volume:0, muted:true
          });
          row.open = status(opened);
          assertOverlay(row.open, "open");
          await invoke("native_replay_bounds", {bounds:visible});
          row.restored = status(await invoke("native_replay_status"));
          assertOverlay(row.restored, "restore");
          if (round % 3 === 2) {
            await invoke("native_replay_bounds", {bounds:hidden});
            await invoke("native_replay_bounds", {bounds:visible});
            row.hidden_restored = status(await invoke("native_replay_status"));
            assertOverlay(row.hidden_restored, "hide_restore");
          }
          await invoke("native_replay_close");
          row.closed = (await invoke("native_replay_status"))?.active === false;
          if (!row.closed) throw Error("close: native player remained active");
          row.passed = true;
        } catch (error) {
          row.error = sanitize(error);
          try { await invoke("native_replay_close"); } catch {}
          throw error;
        }
      }
      await finish();
    } catch (error) { await finish(error); }
  };
  if (document.readyState === "loading") window.addEventListener("DOMContentLoaded", () => void run(), {once:true});
  else void run();
})();
'@
$script = $script.Replace('__ROUND_LIMIT__', $Rounds.ToString([Globalization.CultureInfo]::InvariantCulture))
[IO.File]::WriteAllText((Join-Path $scriptDirectory 'diagnostic.js'), $script, (New-Object Text.UTF8Encoding($false)))

$previousDataDirectory = $env:DOUYIN_MONITOR_HOME
$previousReplayToken = $env:INTERNAL_API_TOKEN
$process = $null
try {
    $env:DOUYIN_MONITOR_HOME = Join-Path $RunRoot 'runtime'
    $env:INTERNAL_API_TOKEN = 'diagnostic-placeholder'
    $process = Start-Process -FilePath $Executable -ArgumentList @('--replay-diagnostic', ('"' + $reportPath + '"'), 'overlay-lifecycle') -WindowStyle Hidden -RedirectStandardError (Join-Path $RunRoot 'stderr.log') -PassThru
    if (-not $process.WaitForExit(45000)) { throw 'Overlay lifecycle diagnostic exceeded its 45-second process deadline.' }
    if (-not (Test-Path -LiteralPath $reportPath)) { throw 'The isolated diagnostic exited before writing its report. Inspect its private stderr.log.' }
    $report = Get-Content -LiteralPath $reportPath -Raw | ConvertFrom-Json
    $report | ConvertTo-Json -Depth 8
    if ($process.ExitCode -ne 0 -or -not $report.overlay_success -or $report.rounds.Count -ne $Rounds) {
        throw ('Overlay lifecycle regression failed. Report: ' + $reportPath)
    }
} finally {
    # Only the exact child process created above is ever stopped. Never search
    # for, close, or terminate the user's ordinary client or backend processes.
    if ($null -ne $process -and -not $process.HasExited) { Stop-Process -Id $process.Id -ErrorAction SilentlyContinue }
    $env:DOUYIN_MONITOR_HOME = $previousDataDirectory
    $env:INTERNAL_API_TOKEN = $previousReplayToken
}
