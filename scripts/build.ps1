param(
    [string]$Python = '',
    [switch]$SkipEngine,
    [switch]$SkipInstaller
)
$ErrorActionPreference = 'Stop'
$projectRoot = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
$oldInstallerCache = $env:PYINSTALLER_CONFIG_DIR
$env:PYINSTALLER_CONFIG_DIR = Join-Path $projectRoot 'build\pyinstaller-cache'
$releaseVersion = (Get-Content -LiteralPath (Join-Path $projectRoot 'package.json') -Raw -Encoding UTF8 | ConvertFrom-Json).version
function Reset-GeneratedDirectory([string]$RelativePath) {
    if ($RelativePath -notin @('src-tauri\engine', 'dist\DouyinLiveMonitor')) { throw 'Unexpected generated directory.' }
    $target = [IO.Path]::GetFullPath((Join-Path $projectRoot $RelativePath))
    if (-not $target.StartsWith($projectRoot + '\', [StringComparison]::OrdinalIgnoreCase)) { throw 'Build target escaped project.' }
    if (Test-Path -LiteralPath $target) { Remove-Item -LiteralPath $target -Recurse -Force }
    New-Item -ItemType Directory -Path $target -Force | Out-Null
    return $target
}
Push-Location $projectRoot
try {
    if (-not $Python) { $Python = Join-Path $projectRoot '.venv\Scripts\python.exe' }
    if (-not (Test-Path -LiteralPath $Python)) { throw 'Create .venv with CPython 3.12 x64 and install requirements-build.txt.' }
    . (Join-Path $PSScriptRoot 'rust-env.ps1')
    & $Python -m pip check
    if ($LASTEXITCODE -ne 0) { throw 'Dependency validation failed.' }
    & $Python -c 'from py_mini_racer import MiniRacer; r=MiniRacer(); assert r.eval("1+2")==3; r.close()'
    if ($LASTEXITCODE -ne 0) { throw 'V8 runtime validation failed.' }
    if (-not $SkipEngine) { & (Join-Path $PSScriptRoot 'build_native.ps1') -Python $Python }
    & $Python packaging/collect_licenses.py
    if ($LASTEXITCODE -ne 0) { throw 'Python license collection failed.' }
    & $Python packaging/collect_desktop_licenses.py
    if ($LASTEXITCODE -ne 0) { throw 'Desktop license collection failed.' }
    if (-not $SkipEngine) {
        & $Python -m PyInstaller --noconfirm --clean packaging/backend.spec
        if ($LASTEXITCODE -ne 0) { throw 'PyInstaller backend build failed.' }
    }
    $engineSource = Join-Path $projectRoot 'dist\DouyinMonitorEngine'
    if (-not (Test-Path -LiteralPath (Join-Path $engineSource 'DouyinMonitorEngine.exe'))) { throw 'Backend executable is missing.' }
    & $Python packaging/check_runtime_assets.py
    if ($LASTEXITCODE -ne 0) { throw 'Packaged V8/signature/TLS resource validation failed.' }
    $resourceEngine = Reset-GeneratedDirectory 'src-tauri\engine'
    Get-ChildItem -LiteralPath $engineSource | Copy-Item -Destination $resourceEngine -Recurse -Force
    & npm.cmd run build
    if ($LASTEXITCODE -ne 0) { throw 'Frontend build failed.' }
    & rustc --edition=2021 --test src-tauri/src/url_policy.rs -o build/url-policy-tests.exe
    if ($LASTEXITCODE -ne 0) { throw 'URL policy test build failed.' }
    & (Join-Path $projectRoot 'build\url-policy-tests.exe')
    if ($LASTEXITCODE -ne 0) { throw 'URL policy validation failed.' }
    & (Join-Path $PSScriptRoot 'stage_release_resources.ps1')
    & .\node_modules\.bin\tauri.cmd build --no-bundle --ci -- --locked --offline
    if ($LASTEXITCODE -ne 0) { throw 'Tauri desktop build failed.' }
    $output = Reset-GeneratedDirectory 'dist\DouyinLiveMonitor'
    Copy-Item -LiteralPath 'src-tauri\target\release\DouyinLiveMonitor.exe' -Destination (Join-Path $output 'DouyinLiveMonitor.exe')
    Copy-Item -LiteralPath $engineSource -Destination (Join-Path $output 'engine') -Recurse
    Copy-Item -LiteralPath 'packaging\licenses' -Destination (Join-Path $output 'licenses') -Recurse
    Get-ChildItem -LiteralPath 'src-tauri\bundle-docs' | Copy-Item -Destination $output -Recurse -Force
    $oldTestHome = $env:DOUYIN_MONITOR_HOME
    try {
        $env:DOUYIN_MONITOR_HOME = Join-Path $projectRoot "build\smoke-$releaseVersion-user-data"
        $smokeReport = Join-Path $projectRoot "build\smoke-report-$releaseVersion.json"
        $process = Start-Process -FilePath (Join-Path $output 'DouyinLiveMonitor.exe') -ArgumentList @('--self-test', ('"' + $smokeReport + '"')) -WindowStyle Hidden -PassThru
        if (-not $process.WaitForExit(60000)) { $process.Kill(); throw 'Packaged self-test timed out.' }
        if ($process.ExitCode -ne 0 -or -not (Test-Path -LiteralPath $smokeReport)) { throw "Packaged self-test failed; see $smokeReport." }
        $report = Get-Content -LiteralPath $smokeReport -Raw -Encoding UTF8 | ConvertFrom-Json
        if (-not $report.ok) { throw 'Packaged self-test report failed.' }
        Get-Content -LiteralPath $smokeReport
    } finally { $env:DOUYIN_MONITOR_HOME = $oldTestHome }
    if (-not $SkipInstaller) {
        & (Join-Path $PSScriptRoot 'package_release.ps1')
    }
} finally {
    $env:PYINSTALLER_CONFIG_DIR = $oldInstallerCache
    Pop-Location
}
