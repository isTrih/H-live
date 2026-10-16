param([switch]$SkipInstaller, [string]$SigningKey = '')
$ErrorActionPreference = 'Stop'
$root = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
Push-Location $root
try {
    . ./scripts/rust-env.ps1
    & npm.cmd run build
    if ($LASTEXITCODE -ne 0) { throw 'Frontend build failed' }
    & ./scripts/collect_licenses.ps1
    & ./scripts/stage_release_resources.ps1
    & ./node_modules/.bin/tauri.cmd build --no-bundle --ci -- --locked --offline
    if ($LASTEXITCODE -ne 0) { throw 'Rust desktop build failed' }
    $output = [IO.Path]::GetFullPath((Join-Path $root 'dist/DouyinLiveMonitor'))
    if (-not $output.StartsWith($root + '\', [StringComparison]::OrdinalIgnoreCase)) { throw 'Distribution target escaped project' }
    if (Test-Path -LiteralPath $output) { Remove-Item -LiteralPath $output -Recurse -Force }
    New-Item -ItemType Directory -Path $output -Force | Out-Null
    Copy-Item -LiteralPath src-tauri/target/release/DouyinLiveMonitor.exe -Destination $output
    Get-ChildItem -LiteralPath src-tauri/bundle-docs | Copy-Item -Destination $output -Recurse
    New-Item -ItemType Directory -Path (Join-Path $output 'licenses') -Force | Out-Null
    Get-ChildItem -LiteralPath packaging/licenses | Copy-Item -Destination (Join-Path $output 'licenses') -Recurse -Force
    $version = (Get-Content package.json -Raw -Encoding UTF8 | ConvertFrom-Json).version
    $report = Join-Path $root "build/smoke-report-$version.json"
    $process = Start-Process -FilePath (Join-Path $output 'DouyinLiveMonitor.exe') -ArgumentList @('--self-test', ('"' + $report + '"')) -WindowStyle Hidden -PassThru
    if (-not $process.WaitForExit(60000)) { throw 'Packaged self-test did not finish' }
    $process.Refresh()
    $result = Get-Content -LiteralPath $report -Raw -Encoding UTF8 | ConvertFrom-Json
    if ($process.ExitCode -ne 0 -or -not $result.ok -or -not $result.checks.no_python_node_v8_sidecar) { throw 'Packaged pure Rust self-test failed' }
    Get-Content -LiteralPath $report
    if (-not $SkipInstaller) { & ./scripts/package_release.ps1 -SigningKey $SigningKey }
} finally { Pop-Location }
